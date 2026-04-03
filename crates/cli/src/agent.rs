use std::sync::Arc;

use harness_core::{
    config::Config,
    message::{ContentBlock, Message, MessageContent, Role, StopReason},
    provider::Provider,
    session::{Session, SessionStatus},
};
use harness_memory::MemoryDb;
use harness_tools::{ToolRegistry, builtin::EchoTool};
use tracing::{debug, info, warn};

/// Drives one agent session: send system prompt + goal, loop until done.
pub struct Agent {
    provider: Arc<dyn Provider>,
    memory: Arc<MemoryDb>,
    tools: ToolRegistry,
    config: Config,
}

impl Agent {
    pub fn new(provider: Arc<dyn Provider>, memory: Arc<MemoryDb>, config: Config) -> Self {
        let tools = ToolRegistry::new();
        tools.register(EchoTool);
        Self { provider, memory, tools, config }
    }

    /// Run until the agent signals completion or max iterations reached.
    pub async fn run(&self, goal: &str) -> anyhow::Result<Session> {
        let mut session = Session::new(goal);
        info!(session_id = %session.id, goal = %goal, "starting session");

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
            debug!(iteration = session.iteration, "agent turn");

            let response = self.provider.complete_with_tools(&messages, &tool_defs).await?;

            let preview = response.message.text().unwrap_or("").to_string();
            info!(
                tokens_out = response.usage.output_tokens,
                stop_reason = ?response.stop_reason,
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
                    let tool_calls: Vec<(String, String, serde_json::Value)> =
                        match &response.message.content {
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
                        info!(tool = %name, "→ calling tool");
                        let output = self.tools.call(&name, input).await;
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
            Self { responses: Mutex::new(r) }
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
    fn tool_use_response(tool_use_id: &str, tool_name: &str, input: serde_json::Value) -> TurnResponse {
        TurnResponse {
            message: Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(vec![
                    ContentBlock::ToolUse {
                        id: tool_use_id.to_string(),
                        name: tool_name.to_string(),
                        input,
                    },
                ]),
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
            .map(|i| tool_use_response(&format!("c-{i}"), "echo", serde_json::json!({"message": "x"})))
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
        };

        let session = agent.run("simple goal").await.unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
        assert_eq!(session.iteration, 1);
        assert_eq!(session.messages.len(), 1); // only the assistant response
    }
}
