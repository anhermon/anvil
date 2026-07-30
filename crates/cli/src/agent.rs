use std::fmt::Write as _;
use std::sync::Arc;

use futures::future::BoxFuture;
use harness_core::{
    config::Config,
    message::{ContentBlock, Message, MessageContent, Role, StopReason},
    provider::Provider,
    session::{Session, SessionStatus},
};
use harness_memory::MemoryDb;
use harness_tools::{
    builtin::{
        BashExecTool, EchoTool, GrepTool, ListSkillsTool, ReadFileTool, ReadSkillTool,
        RefineSkillTool, SaveSkillTool, SpawnSubagentTool, WriteFileTool,
    },
    ToolCallContext, ToolRegistry,
};
use tracing::{debug, info, warn};

/// Maximum sub-agent nesting depth to prevent infinite recursion.
const MAX_SUBAGENT_DEPTH: usize = 4;

/// How many times per run a tool call written as text may be recovered and
/// executed. Bounded so a model that never uses the native channel still
/// terminates instead of looping.
const MAX_TEXT_CALL_RECOVERIES: usize = 3;

/// Callback interface for terminal UI events emitted by the agent loop.
///
/// The default no-op implementation is used for sub-agents and tests so they
/// stay silent. The root CLI turn installs a coloured implementation.
pub trait UiHook: Send + Sync {
    /// Called just before a tool is dispatched (compact terminal preview).
    fn on_tool_call(&self, name: &str, input_preview: &str);
    /// Called with the full tool invocation details (name, id, complete input JSON).
    /// Default: delegates to `on_tool_call` with a preview derived from the input.
    fn on_tool_call_full(&self, name: &str, tool_use_id: &str, input: &serde_json::Value) {
        let preview = input
            .as_object()
            .and_then(|m| m.values().next())
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(60)
            .collect::<String>();
        self.on_tool_call(name, &preview);
        let _ = tool_use_id; // suppress unused warning in default impl
    }
    /// Called with the tool output after it returns.
    fn on_tool_result(&self, output: &str);
    /// Called with the full tool result including id and error flag.
    /// Default: delegates to `on_tool_result`.
    fn on_tool_result_full(&self, tool_use_id: &str, output: &str, is_error: bool) {
        self.on_tool_result(output);
        let _ = (tool_use_id, is_error); // suppress unused warnings in default impl
    }
    /// Called when the assistant produces a text response (not a tool call).
    /// Default: no-op (terminal rendering is handled separately by `run.rs`).
    fn on_text(&self, text: &str) {
        let _ = text;
    }
    /// Called at session completion with the final result text.
    /// Default: no-op.
    fn on_result(&self, text: &str, is_error: bool, session_id: &str) {
        let _ = (text, is_error, session_id);
    }
    /// Called once per completed provider turn with structural diagnostics.
    ///
    /// Exists so the failure taxonomy of a run is *measured* rather than inferred
    /// from the rendered text. Default: no-op.
    fn on_turn_diag(&self, diag: &TurnDiag) {
        let _ = diag;
    }
    /// Called while waiting for the provider to return; receives `[current/max]` label.
    fn on_thinking(&self, iteration: usize, max_iter: usize);
    /// Called when the provider has returned (end of thinking).
    fn on_thinking_done(&self);
}

/// Structural facts about one completed provider turn.
///
/// Emitted for every turn so a run's failure mode can be classified from ground
/// truth (what the model actually returned) instead of regexing rendered prose.
#[derive(Debug, Clone)]
pub struct TurnDiag {
    /// 1-based turn index within the run.
    pub iteration: usize,
    /// Provider-reported stop reason, as a stable lowercase string.
    pub stop_reason: &'static str,
    /// Number of text blocks in the assistant message.
    pub text_blocks: usize,
    /// Number of native tool-use blocks in the assistant message.
    pub tool_blocks: usize,
    /// True when the turn carried neither text nor a tool call (the Ollama
    /// empty-completion flake).
    pub empty: bool,
    /// Number of tool calls recovered from assistant *text* because the model
    /// wrote them as prose instead of using the native tool channel.
    pub recovered_text_calls: usize,
    /// True when this turn's tool calls exactly repeated the previous turn's.
    pub repeated_tool_call: bool,
}

/// Silent implementation used for sub-agents and unit tests.
pub struct NoopHook;
impl UiHook for NoopHook {
    fn on_tool_call(&self, _name: &str, _input_preview: &str) {}
    fn on_tool_result(&self, _output: &str) {}
    fn on_thinking(&self, _iteration: usize, _max_iter: usize) {}
    fn on_thinking_done(&self) {}
}

/// Options controlling a single agent run.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Optional named session for continuity. When set, previous episodes tagged
    /// with this name are loaded as conversation history, and new episodes are
    /// saved under this name.
    pub session_name: Option<String>,
    /// Override the max-iterations from the config for this run.
    pub max_iterations: Option<usize>,
}

/// Drives one agent session: send system prompt + goal, loop until done.
pub struct Agent {
    provider: Arc<dyn Provider>,
    memory: Arc<MemoryDb>,
    tools: ToolRegistry,
    config: Config,
    /// Nesting depth: 0 for the root agent, incremented for each sub-agent.
    depth: usize,
    /// UI hook -- silent by default; replaced by the CLI for the root agent.
    hook: Arc<dyn UiHook>,
}

impl Agent {
    pub fn new(provider: Arc<dyn Provider>, memory: Arc<MemoryDb>, config: Config) -> Self {
        Self::new_with_depth(provider, memory, config, 0)
    }

    pub fn with_hook(mut self, hook: Arc<dyn UiHook>) -> Self {
        self.hook = hook;
        self
    }

    pub fn new_with_depth(
        provider: Arc<dyn Provider>,
        memory: Arc<MemoryDb>,
        config: Config,
        depth: usize,
    ) -> Self {
        let tools = ToolRegistry::new();
        tools.register(EchoTool);
        tools.register(ReadFileTool);
        tools.register(GrepTool);
        tools.register(SpawnSubagentTool);
        tools.register(BashExecTool);
        tools.register(WriteFileTool);
        tools.register(ListSkillsTool);
        tools.register(ReadSkillTool);
        tools.register(SaveSkillTool);
        tools.register(RefineSkillTool);
        Self {
            provider,
            memory,
            tools,
            config,
            depth,
            hook: Arc::new(NoopHook),
        }
    }

    /// Run until the agent signals completion or max iterations reached.
    ///
    /// Returns a `BoxFuture` so recursive sub-agent calls compile without infinite types.
    pub fn run<'a>(&'a self, goal: &'a str) -> BoxFuture<'a, anyhow::Result<Session>> {
        Box::pin(self.run_inner(goal, RunOptions::default()))
    }

    /// Run with explicit options (named session, max-iterations override).
    pub fn run_with_options<'a>(
        &'a self,
        goal: &'a str,
        opts: RunOptions,
    ) -> BoxFuture<'a, anyhow::Result<Session>> {
        Box::pin(self.run_inner(goal, opts))
    }

    // Long but linear: a single top-to-bottom flow; splitting it would only scatter state.
    #[allow(clippy::too_many_lines)]
    async fn run_inner(&self, goal: &str, opts: RunOptions) -> anyhow::Result<Session> {
        let mut session = Session::new(goal);
        info!(
            session_id = %session.id,
            depth = self.depth,
            goal = %goal,
            session_name = ?opts.session_name,
            "starting session"
        );

        let max_iter = {
            let cfg_max = if self.config.agent.max_iterations == 0 {
                usize::MAX
            } else {
                self.config.agent.max_iterations
            };
            opts.max_iterations.unwrap_or(cfg_max)
        };

        // -- 1. Build system prompt, optionally prefixed with memory recall ------
        let base_system = self
            .config
            .agent
            .system_prompt
            .as_deref()
            .unwrap_or(
                "You are anvil, a highly capable software agent with access to tools. \
                 Your goal is to accomplish tasks autonomously. \
                 You are currently in the root of the 'anvil' repository. \
                 Always prefer using tools (read, write, bash, grep) to explore the environment and execute actions. \
                 Note: To use 'ls', you must use the 'bash' tool (e.g. bash(command=\"ls\")). \
                 \n\n\
                 ## Skills\n\n\
                 You have a skill library stored at ~/.anvil/skills/. Skills are Markdown files \
                 containing instructions, workflows, and domain knowledge that guide your behaviour. \
                 At the start of every session, call list_skills() to see what skills are available. \
                 When a user's request matches a skill's domain, call read_skill(name) to load the \
                 full instructions and follow them. \n\n\
                 Available skill tools:\n\
                 - list_skills() — returns all skills with name and description\n\
                 - read_skill(name) — loads full skill content and increments usage counter\n\
                 - save_skill(name, description, content) — creates or updates a skill\n\
                 - refine_skill(name, feedback) — appends refinement notes to an existing skill\n\n\
                 Be concise and direct.\n\n\
                 ## Before you finish\n\n\
                 Before producing your final answer (ending your turn without a tool call), check:\n\
                 1. Does my answer address every part of the stated goal, not just the first step? \
                 If the goal asks for a comparison, ranking, or derived fact (e.g. \"which is largest\", \
                 \"how many\", \"what changed\"), you must have actually run a tool that computes that \
                 fact — do not guess or infer it from a partial listing.\n\
                 2. Is every specific fact in my answer — file names, numbers, paths — copied verbatim \
                 from the most recent tool output, not recalled from memory or paraphrased? If you are not \
                 quoting a tool result directly, re-check it against the raw output before writing it down.\n\
                 If either check fails, call another tool instead of ending your turn."
            )
            .to_string();

        let system_with_memory = self
            .build_system_prompt_with_memory(&base_system, goal)
            .await;

        let mut messages: Vec<Message> = Vec::new();
        messages.push(Message::system(&system_with_memory));

        // -- 2. Session continuity: inject prior named-session history ------------
        if let Some(ref name) = opts.session_name {
            let history = self
                .memory
                .recent_by_name(
                    name,
                    i64::try_from(self.config.memory.max_context_episodes).unwrap_or(i64::MAX),
                )
                .await
                .unwrap_or_default();

            if !history.is_empty() {
                info!(
                    session_name = %name,
                    episodes = history.len(),
                    "injecting named-session history"
                );
                for ep in &history {
                    let role = match ep.role.as_str() {
                        "assistant" => Role::Assistant,
                        _ => Role::User,
                    };
                    messages.push(Message {
                        role,
                        content: MessageContent::Text(ep.content.clone()),
                    });
                }
            }
        }

        messages.push(Message::user(goal));

        // Convert registered tool schemas to ToolDefs for the provider.
        let tool_defs: Vec<_> = self
            .tools
            .schemas()
            .iter()
            .map(harness_tools::ToolSchema::to_def)
            .collect();

        // Previous turn's (name, input) tool-call signature, for loop detection.
        let mut prev_call_sig: Vec<(String, serde_json::Value)> = Vec::new();
        // Bounded so a model that only ever writes prose cannot spin here.
        let mut recoveries_used = 0usize;

        loop {
            if session.iteration >= max_iter {
                info!("max iterations reached");
                let last_text = session.messages.last().and_then(|m| m.text()).unwrap_or("");
                self.hook
                    .on_result(last_text, false, &session.id.to_string());
                session.finish(SessionStatus::Done);
                break;
            }

            session.iteration += 1;
            debug!(
                iteration = session.iteration,
                depth = self.depth,
                "agent turn"
            );

            self.hook.on_thinking(session.iteration, max_iter);
            let mut response = self
                .provider
                .complete_with_tools(&messages, &tool_defs)
                .await?;
            self.hook.on_thinking_done();

            // -- Recover tool calls the model wrote as text ------------------------
            // Small local models regularly drop out of the native tool-call
            // channel and write the call into the message body instead. Without
            // this the loop reads that as an ordinary end-of-turn and abandons
            // the task one step from success. Bounded per run so a model that
            // only ever emits text cannot spin here.
            let mut recovered_text_calls = 0usize;
            if response.stop_reason != StopReason::ToolUse
                && recoveries_used < MAX_TEXT_CALL_RECOVERIES
            {
                if let Some(text) = response.message.text() {
                    let known: Vec<String> = self
                        .tools
                        .schemas()
                        .iter()
                        .map(|s| s.name.clone())
                        .collect();
                    let calls: Vec<_> = harness_core::toolcall_text::parse_text_tool_calls(text)
                        .into_iter()
                        .filter(|c| known.iter().any(|k| k == &c.name))
                        .collect();
                    if !calls.is_empty() {
                        recovered_text_calls = calls.len();
                        recoveries_used += 1;
                        warn!(
                            count = calls.len(),
                            "recovered tool call(s) written as text; executing them"
                        );
                        let blocks = calls
                            .into_iter()
                            .enumerate()
                            .map(|(i, c)| ContentBlock::ToolUse {
                                id: format!("recovered_{}_{i}", session.iteration),
                                name: c.name,
                                input: c.input,
                            })
                            .collect();
                        response.message.content = MessageContent::Blocks(blocks);
                        response.stop_reason = StopReason::ToolUse;
                    }
                }
            }

            let preview = response.message.text().unwrap_or("").to_string();
            info!(
                tokens_out = response.usage.output_tokens,
                stop_reason = ?response.stop_reason,
                depth = self.depth,
                "response: {}",
                &preview[..preview.len().min(120)]
            );

            // -- Turn diagnostics -------------------------------------------------
            // Emitted before any behavioural branch so the taxonomy reflects what
            // the model actually returned, not what the loop did about it.
            let (text_blocks, tool_blocks, call_sig) = match &response.message.content {
                MessageContent::Blocks(blocks) => {
                    let mut texts = 0usize;
                    let mut sig = Vec::new();
                    for b in blocks {
                        match b {
                            ContentBlock::Text { text } if !text.is_empty() => texts += 1,
                            ContentBlock::ToolUse { name, input, .. } => {
                                sig.push((name.clone(), input.clone()));
                            }
                            _ => {}
                        }
                    }
                    (texts, sig.len(), sig)
                }
                MessageContent::Text(t) => (usize::from(!t.is_empty()), 0, Vec::new()),
            };
            let repeated_tool_call = !call_sig.is_empty() && call_sig == prev_call_sig;
            prev_call_sig = call_sig;

            self.hook.on_turn_diag(&TurnDiag {
                iteration: session.iteration,
                stop_reason: match response.stop_reason {
                    StopReason::EndTurn => "end_turn",
                    StopReason::ToolUse => "tool_use",
                    StopReason::MaxTokens => "max_tokens",
                    StopReason::StopSequence => "stop_sequence",
                },
                text_blocks,
                tool_blocks,
                empty: text_blocks == 0 && tool_blocks == 0,
                recovered_text_calls,
                repeated_tool_call,
            });

            // Append assistant message to running history and session log.
            messages.push(response.message.clone());
            session.push(response.message.clone());

            match response.stop_reason {
                StopReason::EndTurn | StopReason::StopSequence | StopReason::MaxTokens => {
                    let final_text = response.message.text().unwrap_or("").to_string();

                    // Notify hook of the final assistant text so JSON output mode can emit it.
                    if !final_text.is_empty() {
                        self.hook.on_text(&final_text);
                    }

                    // Persist final assistant turn to memory.
                    let ep = harness_memory::Episode::turn(session.id, "assistant", &final_text);
                    let sn = opts.session_name.as_deref();
                    self.memory.insert_named(&ep, sn).await?;

                    // Also persist the goal as a user episode for future recall.
                    let goal_ep = harness_memory::Episode::turn(session.id, "user", goal);
                    self.memory.insert_named(&goal_ep, sn).await?;

                    // Notify hook that the session is complete.
                    self.hook
                        .on_result(&final_text, false, &session.id.to_string());

                    session.finish(SessionStatus::Done);
                    break;
                }

                StopReason::ToolUse => {
                    // Extract every ToolUse block from the assistant response.
                    // Also emit any text blocks that appear alongside tool calls.
                    let (tool_calls, text_blocks): (
                        Vec<(String, String, serde_json::Value)>,
                        Vec<String>,
                    ) = if let MessageContent::Blocks(blocks) = &response.message.content {
                        let tools = blocks
                            .iter()
                            .filter_map(|b| {
                                if let ContentBlock::ToolUse { id, name, input } = b {
                                    Some((id.clone(), name.clone(), input.clone()))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        let texts = blocks
                            .iter()
                            .filter_map(|b| {
                                if let ContentBlock::Text { text } = b {
                                    if text.is_empty() {
                                        None
                                    } else {
                                        Some(text.clone())
                                    }
                                } else {
                                    None
                                }
                            })
                            .collect();
                        (tools, texts)
                    } else {
                        warn!(
                            "stop_reason=ToolUse but no ToolUse blocks found; treating as EndTurn"
                        );
                        let last_text =
                            session.messages.last().and_then(|m| m.text()).unwrap_or("");
                        self.hook
                            .on_result(last_text, false, &session.id.to_string());
                        session.finish(SessionStatus::Done);
                        break;
                    };

                    // Emit any text blocks that appear before or alongside the tool calls.
                    for text in &text_blocks {
                        self.hook.on_text(text);
                    }

                    // Execute each tool and collect result blocks.
                    let mut result_blocks: Vec<ContentBlock> = Vec::new();
                    let tool_context = ToolCallContext::for_session(session.id.to_string());
                    for (tool_use_id, name, input) in tool_calls {
                        info!(tool = %name, depth = self.depth, "calling tool");

                        // Notify hooks with both compact preview (terminal) and full data (JSON).
                        // Only call on_tool_call_full; it will delegate to on_tool_call if needed
                        self.hook.on_tool_call_full(&name, &tool_use_id, &input);

                        let mut output = if name == "spawn_subagent" {
                            let sub_goal = input["goal"].as_str().unwrap_or("").to_string();
                            let context = input
                                .get("context")
                                .and_then(|c| c.as_str())
                                .unwrap_or("")
                                .to_string();
                            info!(sub_goal = %sub_goal, depth = self.depth, "spawning sub-agent");
                            match self.spawn_subagent(&sub_goal, &context).await {
                                Ok(result) => harness_tools::ToolOutput::ok(result),
                                Err(e) => {
                                    harness_tools::ToolOutput::err(format!("sub-agent error: {e}"))
                                }
                            }
                        } else {
                            self.tools
                                .call_with_context(&name, input, &tool_context)
                                .await
                        };

                        // Truncate extremely large tool outputs to prevent context bloat.
                        if output.content.len() > 10000 {
                            warn!(
                                tool = %name,
                                len = output.content.len(),
                                "truncating large tool output"
                            );
                            // Find the last UTF-8 character boundary before 10000 bytes
                            let truncate_at = output
                                .content
                                .char_indices()
                                .take_while(|(idx, _)| *idx < 10000)
                                .last()
                                .map_or(0, |(idx, ch)| idx + ch.len_utf8());
                            let truncated_chars = output.content[truncate_at..].chars().count();
                            output.content = format!(
                                "{}... [TRUNCATED {} characters]",
                                &output.content[..truncate_at],
                                truncated_chars
                            );
                        }

                        if output.is_error {
                            warn!(tool = %name, "tool returned error: {}", output.content);
                        }
                        // Only call on_tool_result_full; it will delegate to on_tool_result if needed
                        self.hook.on_tool_result_full(
                            &tool_use_id,
                            &output.content,
                            output.is_error,
                        );
                        result_blocks.push(ContentBlock::ToolResult {
                            tool_use_id,
                            content: output.content,
                        });
                    }

                    // A model that re-issues the identical call will keep doing
                    // so until the iteration cap: the tool result it just got is
                    // the same one it already ignored, so nothing in the context
                    // pushes it elsewhere. Say so explicitly on the channel it is
                    // already reading.
                    if repeated_tool_call {
                        if let Some(ContentBlock::ToolResult { content, .. }) =
                            result_blocks.first_mut()
                        {
                            content.push_str(
                                "\n\n[harness] This is the same tool call, with the same \
                                 arguments, as your previous turn — so this is the same result. \
                                 Repeating it again will not help. Either use this output to \
                                 answer, or try a materially different command.",
                            );
                        }
                    }

                    // Feed results back as a tool-role message and continue.
                    let tool_result_msg = Message {
                        role: Role::Tool,
                        content: MessageContent::Blocks(result_blocks),
                    };
                    messages.push(tool_result_msg.clone());
                    session.push(tool_result_msg);
                }
            }
        }

        // Post-session evolution hook (compiled only when the `evolution` feature is enabled).
        #[cfg(feature = "evolution")]
        if self.depth == 0 {
            let prompt = self
                .config
                .agent
                .system_prompt
                .as_deref()
                .unwrap_or("You are a helpful assistant. Complete the user's goal concisely.");
            let engine = harness_evolution::defaults::default_engine(Arc::clone(&self.memory));
            match engine.evolve(&session, prompt).await {
                Ok(outcome) => {
                    info!(?outcome, "evolution cycle complete");
                }
                Err(e) => {
                    // Non-fatal: log and continue so the session result is not affected.
                    warn!(error = %e, "evolution cycle failed (non-fatal)");
                }
            }
        }

        Ok(session)
    }

    /// Search memory for relevant past episodes and prepend them to the system
    /// prompt as a `[Memory: N relevant episodes]` block.
    async fn build_system_prompt_with_memory(&self, base_system: &str, goal: &str) -> String {
        match self.memory.search(goal, 5).await {
            Ok(episodes) if !episodes.is_empty() => {
                let mut header = format!(
                    "[Memory: {} relevant episode{}]\n",
                    episodes.len(),
                    if episodes.len() == 1 { "" } else { "s" }
                );
                for ep in &episodes {
                    let ts = ep.created_at.format("%Y-%m-%dT%H:%M:%SZ");
                    // Use first 200 chars of content as the summary.
                    let summary: String = ep.content.chars().take(200).collect();
                    let _ = writeln!(header, "- {ts}: {summary}");
                }
                header.push('\n');
                header.push_str(base_system);
                header
            }
            Ok(_) => base_system.to_string(),
            Err(e) => {
                warn!("memory search failed: {e}; proceeding without recall");
                base_system.to_string()
            }
        }
    }

    /// Spawn a nested sub-agent to handle a delegated goal.
    ///
    /// Returns the sub-agent final response text, or an error if depth
    /// exceeds [`MAX_SUBAGENT_DEPTH`].
    async fn spawn_subagent(&self, goal: &str, context: &str) -> anyhow::Result<String> {
        if self.depth >= MAX_SUBAGENT_DEPTH {
            return Err(anyhow::anyhow!(
                "sub-agent depth limit ({MAX_SUBAGENT_DEPTH}) reached -- cannot spawn further"
            ));
        }

        let full_goal = if context.is_empty() {
            goal.to_string()
        } else {
            format!("{context}\n\n{goal}")
        };

        let sub_agent = Agent::new_with_depth(
            Arc::clone(&self.provider),
            Arc::clone(&self.memory),
            self.config.clone(),
            self.depth + 1,
        );

        let session = sub_agent.run(&full_goal).await?;

        let result = session
            .messages
            .last()
            .filter(|message| message.role == Role::Assistant)
            .and_then(|m| m.text())
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "sub-agent ended without a final assistant response \
                     (it may have reached its iteration limit)"
                )
            })?
            .to_string();

        info!(
            depth = self.depth,
            result_len = result.len(),
            "sub-agent completed"
        );

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use harness_core::{
        message::{ContentBlock, MessageContent, Role, StopReason, TurnResponse, Usage},
        provider::Provider,
    };
    use harness_tools::builtin::EchoTool;
    use std::sync::{Arc, Mutex};

    /// Provider that pops responses from a pre-loaded queue.
    struct ScriptedProvider {
        responses: Mutex<Vec<TurnResponse>>,
    }

    impl ScriptedProvider {
        fn new(responses: Vec<TurnResponse>) -> Self {
            // Reverse so we can pop from the back in FIFO order.
            let mut r = responses;
            r.reverse();
            Self {
                responses: Mutex::new(r),
            }
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn name(&self) -> &'static str {
            "scripted"
        }

        async fn complete(
            &self,
            _messages: &[harness_core::message::Message],
        ) -> harness_core::error::Result<TurnResponse> {
            let mut guard = self.responses.lock().unwrap();
            Ok(guard.pop().expect("ScriptedProvider ran out of responses"))
        }
    }

    fn make_config(max_iterations: usize) -> harness_core::config::Config {
        let mut cfg = harness_core::config::Config::default();
        cfg.agent.max_iterations = max_iterations;
        cfg.agent.system_prompt = None;
        cfg
    }

    async fn make_memory() -> Arc<MemoryDb> {
        Arc::new(MemoryDb::in_memory().await.unwrap())
    }

    fn tool_use_response(
        tool_use_id: &str,
        tool_name: &str,
        input: serde_json::Value,
    ) -> TurnResponse {
        TurnResponse {
            message: Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    id: tool_use_id.to_string(),
                    name: tool_name.to_string(),
                    input,
                }]),
            },
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
            model: "scripted".to_string(),
        }
    }

    fn end_turn_response(text: &str) -> TurnResponse {
        TurnResponse {
            message: Message {
                role: Role::Assistant,
                content: MessageContent::Text(text.to_string()),
            },
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: "scripted".to_string(),
        }
    }

    #[tokio::test]
    async fn tool_loop_calls_tool_and_continues() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            tool_use_response("call-1", "echo", serde_json::json!({"message": "ping"})),
            end_turn_response("done"),
        ]));

        let memory = make_memory().await;
        let config = make_config(10);

        let agent = Agent {
            provider: provider.clone(),
            memory,
            tools: {
                let r = ToolRegistry::new();
                r.register(EchoTool);
                r
            },
            config,
            depth: 0,
            hook: Arc::new(NoopHook),
        };

        let session = agent.run("test goal").await.unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
        assert_eq!(session.messages.len(), 3);
    }

    #[tokio::test]
    async fn max_iterations_cap_is_respected() {
        let responses: Vec<TurnResponse> = (0..10)
            .map(|i| {
                tool_use_response(
                    &format!("c-{i}"),
                    "echo",
                    serde_json::json!({"message": "x"}),
                )
            })
            .collect();

        let provider = Arc::new(ScriptedProvider::new(responses));
        let memory = make_memory().await;
        let config = make_config(2);

        let agent = Agent {
            provider,
            memory,
            tools: {
                let r = ToolRegistry::new();
                r.register(EchoTool);
                r
            },
            config,
            depth: 0,
            hook: Arc::new(NoopHook),
        };

        let session = agent.run("loop forever").await.unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
        assert_eq!(session.iteration, 2);
    }

    #[tokio::test]
    async fn end_turn_stops_without_tool_calls() {
        let provider = Arc::new(ScriptedProvider::new(vec![end_turn_response("hello")]));
        let memory = make_memory().await;
        let config = make_config(5);

        let agent = Agent {
            provider,
            memory,
            tools: ToolRegistry::new(),
            config,
            depth: 0,
            hook: Arc::new(NoopHook),
        };

        let session = agent.run("simple goal").await.unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
        assert_eq!(session.iteration, 1);
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn subagent_spawned_and_returns_result() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            tool_use_response(
                "sa-1",
                "spawn_subagent",
                serde_json::json!({"goal": "compute something"}),
            ),
            end_turn_response("sub-result"),
            end_turn_response("main done"),
        ]));

        let memory = make_memory().await;
        let config = make_config(10);

        let agent = Agent::new(provider, memory, config);
        let session = agent.run("delegate work").await.unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
        let last = session.messages.last().unwrap();
        assert_eq!(last.text(), Some("main done"));
    }

    #[tokio::test]
    async fn subagent_depth_limit_returns_error_output() {
        let provider = Arc::new(ScriptedProvider::new(vec![]));
        let memory = make_memory().await;
        let config = make_config(10);

        let deep_agent = Agent::new_with_depth(provider, memory, config, MAX_SUBAGENT_DEPTH);
        let result = deep_agent.spawn_subagent("unreachable", "").await;

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("depth limit"),
            "expected 'depth limit' in: {msg}"
        );
    }

    #[tokio::test]
    async fn subagent_with_context_prepends_to_goal() {
        let provider = Arc::new(ScriptedProvider::new(vec![end_turn_response(
            "context-aware result",
        )]));
        let memory = make_memory().await;
        let config = make_config(5);

        let provider: Arc<dyn Provider> = provider;
        let agent = Agent::new_with_depth(Arc::clone(&provider), memory, config, 0);
        let result = agent
            .spawn_subagent("do the thing", "background: xyz")
            .await
            .unwrap();

        assert_eq!(result, "context-aware result");
    }

    #[tokio::test]
    async fn subagent_without_final_assistant_text_returns_error() {
        let provider = Arc::new(ScriptedProvider::new(vec![tool_use_response(
            "child-echo",
            "echo",
            serde_json::json!({"message": "unfinished"}),
        )]));
        let memory = make_memory().await;
        let config = make_config(1);

        let agent = Agent::new(provider, memory, config);
        let result = agent.spawn_subagent("keep working", "").await;

        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("without a final assistant response"),
            "unexpected error: {message}"
        );
    }

    // -- New tests for memory recall and session continuity -------------------

    #[tokio::test]
    async fn memory_is_searched_at_start_of_run() {
        // Pre-populate memory with a past episode whose content matches the goal.
        let memory = make_memory().await;
        let past_session_id = uuid::Uuid::new_v4();
        let past_ep = harness_memory::Episode::turn(
            past_session_id,
            "assistant",
            "rust ownership means values have a single owner",
        );
        memory.insert(&past_ep).await.unwrap();

        // Provider that captures the messages it receives.
        let captured: Arc<Mutex<Vec<Message>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = Arc::clone(&captured);

        struct CapturingProvider {
            captured: Arc<Mutex<Vec<Message>>>,
        }
        #[async_trait]
        impl Provider for CapturingProvider {
            fn name(&self) -> &'static str {
                "capturing"
            }
            async fn complete(
                &self,
                messages: &[Message],
            ) -> harness_core::error::Result<TurnResponse> {
                let mut g = self.captured.lock().unwrap();
                *g = messages.to_vec();
                Ok(TurnResponse {
                    message: Message {
                        role: Role::Assistant,
                        content: MessageContent::Text("done".to_string()),
                    },
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    model: "capturing".to_string(),
                })
            }
        }

        let provider: Arc<dyn Provider> = Arc::new(CapturingProvider {
            captured: captured_clone,
        });
        let config = make_config(5);
        let agent = Agent::new(provider, memory, config);

        // Goal contains "rust ownership" which should match the past episode.
        agent.run("explain rust ownership").await.unwrap();

        let msgs = captured.lock().unwrap();
        let system_text = msgs
            .iter()
            .find(|m| matches!(m.role, Role::System))
            .and_then(|m| m.text())
            .unwrap_or("");

        assert!(
            system_text.contains("[Memory:"),
            "expected [Memory: ...] block in system prompt, got: {system_text}"
        );
        assert!(
            system_text.contains("rust ownership"),
            "expected past episode content in system prompt, got: {system_text}"
        );
    }

    #[tokio::test]
    async fn episodes_are_saved_after_run() {
        let memory = make_memory().await;
        let provider = Arc::new(ScriptedProvider::new(vec![end_turn_response(
            "the final answer",
        )]));
        let config = make_config(5);
        let agent = Agent::new(provider as Arc<dyn Provider>, Arc::clone(&memory), config);

        let session = agent.run("what is 2+2?").await.unwrap();

        // Both the goal (user) and the final response (assistant) should be saved.
        let episodes = memory.recent(session.id, 10).await.unwrap();
        assert!(
            episodes.len() >= 2,
            "expected at least 2 saved episodes, got {}",
            episodes.len()
        );
        let roles: Vec<&str> = episodes.iter().map(|e| e.role.as_str()).collect();
        assert!(
            roles.contains(&"user"),
            "expected a 'user' episode to be saved"
        );
        assert!(
            roles.contains(&"assistant"),
            "expected an 'assistant' episode to be saved"
        );
    }

    #[tokio::test]
    async fn run_with_options_overrides_max_iterations() {
        // Config says max=5; options say max=2 -- should cap at 2.
        let responses: Vec<TurnResponse> = (0..10)
            .map(|i| {
                tool_use_response(
                    &format!("c-{i}"),
                    "echo",
                    serde_json::json!({"message": "x"}),
                )
            })
            .collect();

        let provider = Arc::new(ScriptedProvider::new(responses));
        let memory = make_memory().await;
        let config = make_config(5);

        let agent = Agent {
            provider: provider as Arc<dyn Provider>,
            memory,
            tools: {
                let r = ToolRegistry::new();
                r.register(EchoTool);
                r
            },
            config,
            depth: 0,
            hook: Arc::new(NoopHook),
        };

        let opts = RunOptions {
            max_iterations: Some(2),
            ..Default::default()
        };
        let session = agent.run_with_options("loop", opts).await.unwrap();
        assert_eq!(session.iteration, 2);
    }

    #[tokio::test]
    async fn session_continuity_injects_named_history() {
        let memory = make_memory().await;

        // Pre-populate memory with a named-session episode.
        let past_id = uuid::Uuid::new_v4();
        let ep = harness_memory::Episode::turn(past_id, "user", "previous turn content");
        memory.insert_named(&ep, Some("myproject")).await.unwrap();

        let captured: Arc<Mutex<Vec<Message>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = Arc::clone(&captured);

        struct CapturingProvider {
            captured: Arc<Mutex<Vec<Message>>>,
        }
        #[async_trait]
        impl Provider for CapturingProvider {
            fn name(&self) -> &'static str {
                "capturing"
            }
            async fn complete(
                &self,
                messages: &[Message],
            ) -> harness_core::error::Result<TurnResponse> {
                let mut g = self.captured.lock().unwrap();
                *g = messages.to_vec();
                Ok(TurnResponse {
                    message: Message {
                        role: Role::Assistant,
                        content: MessageContent::Text("ok".to_string()),
                    },
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    model: "capturing".to_string(),
                })
            }
        }

        let provider: Arc<dyn Provider> = Arc::new(CapturingProvider {
            captured: captured_clone,
        });
        let config = make_config(5);
        let agent = Agent::new(provider, memory, config);

        let opts = RunOptions {
            session_name: Some("myproject".to_string()),
            ..Default::default()
        };
        agent
            .run_with_options("continue the work", opts)
            .await
            .unwrap();

        let msgs = captured.lock().unwrap();
        let all_text: Vec<String> = msgs
            .iter()
            .filter_map(|m| m.text().map(std::string::ToString::to_string))
            .collect();

        assert!(
            all_text.iter().any(|t| t.contains("previous turn content")),
            "expected prior session history to be injected; messages: {all_text:?}"
        );
    }
    /// The failure that motivated the recovery path: the model writes the tool
    /// call into the message body and ends its turn. Before, the loop read that
    /// as a final answer and stopped one step from success.
    #[tokio::test]
    async fn tool_call_written_as_text_is_executed() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            end_turn_response(
                "I'll check that.\n```json\n{\"name\": \"echo\", \"input\": {\"message\": \"ping\"}}\n```",
            ),
            end_turn_response("done"),
        ]));

        let agent = Agent {
            provider,
            memory: make_memory().await,
            tools: {
                let r = ToolRegistry::new();
                r.register(EchoTool);
                r
            },
            config: make_config(10),
            depth: 0,
            hook: Arc::new(NoopHook),
        };

        let session = agent.run("test goal").await.unwrap();

        // The recovered call must have actually run: a tool-result message is
        // present and carries the echoed payload.
        let echoed = session.messages.iter().any(|m| {
            m.role == Role::Tool
                && match &m.content {
                    MessageContent::Blocks(b) => b.iter().any(|blk| {
                        matches!(blk, ContentBlock::ToolResult { content, .. } if content.contains("ping"))
                    }),
                    MessageContent::Text(t) => t.contains("ping"),
                }
        });
        assert!(
            echoed,
            "expected the text-written tool call to execute, got: {:?}",
            session.messages
        );
    }

    /// A truncated fence — the model stopped mid-JSON — is still recoverable.
    #[tokio::test]
    async fn truncated_text_tool_call_is_recovered() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            end_turn_response(
                "Here:\n```json\n{\"name\": \"echo\", \"input\": {\"message\": \"ping\"",
            ),
            end_turn_response("done"),
        ]));

        let agent = Agent {
            provider,
            memory: make_memory().await,
            tools: {
                let r = ToolRegistry::new();
                r.register(EchoTool);
                r
            },
            config: make_config(10),
            depth: 0,
            hook: Arc::new(NoopHook),
        };

        let session = agent.run("test goal").await.unwrap();
        assert!(session.messages.iter().any(|m| m.role == Role::Tool));
    }

    /// Ordinary prose must not be mistaken for a tool call — otherwise every
    /// final answer mentioning JSON would restart the loop.
    #[tokio::test]
    async fn plain_final_answer_still_ends_the_run() {
        let provider = Arc::new(ScriptedProvider::new(vec![end_turn_response(
            "There are no TODO comments in crates/.",
        )]));

        let agent = Agent {
            provider,
            memory: make_memory().await,
            tools: {
                let r = ToolRegistry::new();
                r.register(EchoTool);
                r
            },
            config: make_config(10),
            depth: 0,
            hook: Arc::new(NoopHook),
        };

        let session = agent.run("test goal").await.unwrap();
        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
        assert!(
            !session.messages.iter().any(|m| m.role == Role::Tool),
            "no tool should have run for a plain prose answer"
        );
    }

    /// An unknown tool name must not be executed just because it parsed.
    #[tokio::test]
    async fn text_call_to_unknown_tool_is_not_executed() {
        let provider = Arc::new(ScriptedProvider::new(vec![end_turn_response(
            "```json\n{\"name\": \"rm_rf\", \"input\": {\"path\": \"/\"}}\n```",
        )]));

        let agent = Agent {
            provider,
            memory: make_memory().await,
            tools: {
                let r = ToolRegistry::new();
                r.register(EchoTool);
                r
            },
            config: make_config(10),
            depth: 0,
            hook: Arc::new(NoopHook),
        };

        let session = agent.run("test goal").await.unwrap();
        assert!(!session.messages.iter().any(|m| m.role == Role::Tool));
    }
}
