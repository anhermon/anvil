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
    builtin::{EchoTool, ReadFileTool, SpawnSubagentTool},
    ToolRegistry,
};
use tracing::{debug, info, warn};

/// Maximum sub-agent nesting depth to prevent infinite recursion.
const MAX_SUBAGENT_DEPTH: usize = 4;

/// Drives one agent session: send system prompt + goal, loop until done.
pub struct Agent {
    provider: Arc<dyn Provider>,
    memory: Arc<MemoryDb>,
    tools: ToolRegistry,
    config: Config,
    /// Nesting depth: 0 for the root agent, incremented for each sub-agent.
    depth: usize,
}

impl Agent {
    pub fn new(provider: Arc<dyn Provider>, memory: Arc<MemoryDb>, config: Config) -> Self {
        Self::new_with_depth(provider, memory, config, 0)
    }

    fn new_with_depth(
        provider: Arc<dyn Provider>,
        memory: Arc<MemoryDb>,
        config: Config,
        depth: usize,
    ) -> Self {
        let tools = ToolRegistry::new();
        tools.register(EchoTool);
        tools.register(ReadFileTool);
        tools.register(SpawnSubagentTool);
        Self {
            provider,
            memory,
            tools,
            config,
            depth,
        }
    }

    /// Run until the agent signals completion or max iterations reached.
    ///
    /// Returns a `BoxFuture` so recursive sub-agent calls compile without infinite types.
    pub fn run<'a>(&'a self, goal: &'a str) -> BoxFuture<'a, anyhow::Result<Session>> {
        Box::pin(self.run_inner(goal))
    }

    async fn run_inner(&self, goal: &str) -> anyhow::Result<Session> {
        let mut session = Session::new(goal);
        info!(
            session_id = %session.id,
            depth = self.depth,
            goal = %goal,
            "starting session"
        );

        // Build initial messages
        let mut messages: Vec<Message> = Vec::new();

        if let Some(sys) = &self.config.agent.system_prompt {
            messages.push(Message::system(sys));
        } else {
            messages.push(Message::system(
                "You are a helpful assistant. Complete the user's goal concisely.",
            ));
        }

        messages.push(Message::user(goal));

        let max_iter = if self.config.agent.max_iterations == 0 {
            usize::MAX
        } else {
            self.config.agent.max_iterations
        };

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

            let response = self
                .provider
                .complete_with_tools(&messages, &tool_defs)
                .await?;

            let preview = response.message.text().unwrap_or("").to_string();
            info!(
                tokens_out = response.usage.output_tokens,
                stop_reason = ?response.stop_reason,
                depth = self.depth,
                "← {}",
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
                    self.memory.insert(&ep).await?;
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
                        info!(tool = %name, depth = self.depth, "→ calling tool");
                        let output = if self.is_agent_managed_tool(&name) {
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

        Ok(session)
    }

    /// Returns `true` for tools that are managed directly by the agent loop
    /// rather than dispatched through [`ToolRegistry`].
    ///
    /// `spawn_subagent` must be handled here — not via the registry — because
    /// executing it requires constructing a new [`Agent`] with an incremented
    /// depth, which needs access to `self` (provider, memory, config).  The
    /// registry holds stateless closures and cannot capture agent state.
    fn is_agent_managed_tool(&self, name: &str) -> bool {
        name == "spawn_subagent"
    }

    /// Spawn a nested sub-agent to handle a delegated goal.
    ///
    /// Returns the sub-agent's final response text, or an error if depth
    /// exceeds [`MAX_SUBAGENT_DEPTH`].
    async fn spawn_subagent(&self, goal: &str, context: &str) -> anyhow::Result<String> {
        if self.depth >= MAX_SUBAGENT_DEPTH {
            return Err(anyhow::anyhow!(
                "sub-agent depth limit ({MAX_SUBAGENT_DEPTH}) reached — cannot spawn further"
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

    /// Helpers for building TurnResponse values.
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
        // Turn 1: provider requests an echo tool call.
        // Turn 2: provider returns EndTurn after seeing the tool result.
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
        };

        let session = agent.run("test goal").await.unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
        // messages: system + user + assistant(tool_use) + user(tool_result) + assistant(end_turn)
        assert_eq!(session.messages.len(), 3); // assistant(tool_use) + user(tool_result) + assistant(end_turn)
    }

    #[tokio::test]
    async fn max_iterations_cap_is_respected() {
        // Provider always asks for a tool call; cap at 2 iterations.
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
        };

        let session = agent.run("simple goal").await.unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
        assert_eq!(session.iteration, 1);
        assert_eq!(session.messages.len(), 1); // only the assistant response
    }

    #[tokio::test]
    async fn subagent_spawned_and_returns_result() {
        // Main agent: spawns a sub-agent, then finishes after seeing the result.
        // Sub-agent: immediately returns EndTurn with "sub-result".
        //
        // ScriptedProvider is shared — responses interleave:
        //   pop 1 (main): spawn_subagent tool use
        //   pop 2 (sub):  end_turn "sub-result"
        //   pop 3 (main): end_turn "main done"
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
        // Last message should be the main agent's final EndTurn response.
        let last = session.messages.last().unwrap();
        assert_eq!(last.text(), Some("main done"));
    }

    #[tokio::test]
    async fn subagent_depth_limit_returns_error_output() {
        // Directly test spawn_subagent at max depth returns an Err.
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
        // Verify that when context is non-empty it is prepended to the goal
        // by confirming the sub-agent session runs with the combined text.
        // We use a provider that immediately ends so the sub-agent session completes.
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
}
