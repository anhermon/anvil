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
        BashExecTool, EchoTool, ListSkillsTool, ReadFileTool, ReadSkillTool, RefineSkillTool,
        SaveSkillTool, SpawnSubagentTool, WriteFileTool,
    },
    ToolRegistry,
};
use tracing::{debug, info, warn};

/// Maximum sub-agent nesting depth to prevent infinite recursion.
const MAX_SUBAGENT_DEPTH: usize = 4;

/// Callback interface for terminal UI events emitted by the agent loop.
///
/// The default no-op implementation is used for sub-agents and tests so they
/// stay silent. The root CLI turn installs a coloured implementation.
pub trait UiHook: Send + Sync {
    /// Called just before a tool is dispatched.
    fn on_tool_call(&self, name: &str, input_preview: &str);
    /// Called with the tool output after it returns.
    fn on_tool_result(&self, output: &str);
    /// Called while waiting for the provider to return; receives `[current/max]` label.
    fn on_thinking(&self, iteration: usize, max_iter: usize);
    /// Called when the provider has returned (end of thinking).
    fn on_thinking_done(&self);
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
            .unwrap_or("You are a helpful assistant. Complete the user's goal concisely.")
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
                .recent_by_name(name, self.config.memory.max_context_episodes as i64)
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
        let tool_defs: Vec<_> = self.tools.schemas().iter().map(|s| s.to_def()).collect();

        loop {
            if session.iteration >= max_iter {
                info!("max iterations reached");
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
            let response = self
                .provider
                .complete_with_tools(&messages, &tool_defs)
                .await?;
            self.hook.on_thinking_done();

            let preview = response.message.text().unwrap_or("").to_string();
            info!(
                tokens_out = response.usage.output_tokens,
                stop_reason = ?response.stop_reason,
                depth = self.depth,
                "response: {}",
                &preview[..preview.len().min(120)]
            );

            // Append assistant message to running history and session log.
            messages.push(response.message.clone());
            session.push(response.message.clone());

            match response.stop_reason {
                StopReason::EndTurn | StopReason::StopSequence | StopReason::MaxTokens => {
                    // Persist final assistant turn to memory.
                    let ep = harness_memory::Episode::turn(
                        session.id,
                        "assistant",
                        response.message.text().unwrap_or(""),
                    );
                    let sn = opts.session_name.as_deref();
                    self.memory.insert_named(&ep, sn).await?;

                    // Also persist the goal as a user episode for future recall.
                    let goal_ep = harness_memory::Episode::turn(session.id, "user", goal);
                    self.memory.insert_named(&goal_ep, sn).await?;

                    session.finish(SessionStatus::Done);
                    break;
                }

                StopReason::ToolUse => {
                    // Extract every ToolUse block from the assistant response.
                    let tool_calls: Vec<(String, String, serde_json::Value)> = match &response
                        .message
                        .content
                    {
                        MessageContent::Blocks(blocks) => blocks
                            .iter()
                            .filter_map(|b| {
                                if let ContentBlock::ToolUse { id, name, input } = b {
                                    Some((id.clone(), name.clone(), input.clone()))
                                } else {
                                    None
                                }
                            })
                            .collect(),
                        _ => {
                            warn!("stop_reason=ToolUse but no ToolUse blocks found; treating as EndTurn");
                            session.finish(SessionStatus::Done);
                            break;
                        }
                    };

                    // Execute each tool and collect result blocks.
                    let mut result_blocks: Vec<ContentBlock> = Vec::new();
                    for (tool_use_id, name, input) in tool_calls {
                        info!(tool = %name, depth = self.depth, "calling tool");

                        // Build a compact preview of the input for the UI hook.
                        let input_preview = input
                            .as_object()
                            .and_then(|m| m.values().next())
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .chars()
                            .take(60)
                            .collect::<String>();
                        self.hook.on_tool_call(&name, &input_preview);

                        let output = if name == "spawn_subagent" {
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
                            self.tools.call(&name, input).await
                        };

                        if output.is_error {
                            warn!(tool = %name, "tool returned error: {}", output.content);
                        }
                        self.hook.on_tool_result(&output.content);
                        result_blocks.push(ContentBlock::ToolResult {
                            tool_use_id,
                            content: output.content,
                        });
                    }

                    // Feed results back as a user-role message and continue.
                    let tool_result_msg = Message {
                        role: Role::User,
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
                    header.push_str(&format!("- {ts}: {summary}\n"));
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
            .and_then(|m| m.text())
            .unwrap_or("(sub-agent produced no output)")
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
        fn name(&self) -> &str {
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
            fn name(&self) -> &str {
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
            fn name(&self) -> &str {
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
            .filter_map(|m| m.text().map(|t| t.to_string()))
            .collect();

        assert!(
            all_text.iter().any(|t| t.contains("previous turn content")),
            "expected prior session history to be injected; messages: {all_text:?}"
        );
    }
}
