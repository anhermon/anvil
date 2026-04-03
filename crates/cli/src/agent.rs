use std::sync::Arc;

use harness_core::{
    config::Config,
    message::Message,
    provider::Provider,
    session::{Session, SessionStatus},
};
use harness_memory::MemoryDb;
use harness_tools::{builtin::EchoTool, ToolRegistry};
use tracing::{debug, info};

/// Drives one agent session: send system prompt + goal, loop until done.
pub struct Agent {
    provider: Arc<dyn Provider>,
    memory: Arc<MemoryDb>,
    #[allow(dead_code)] // will be wired into the tool-call loop in a follow-up
    tools: ToolRegistry,
    config: Config,
}

impl Agent {
    pub fn new(provider: Arc<dyn Provider>, memory: Arc<MemoryDb>, config: Config) -> Self {
        let tools = ToolRegistry::new();
        tools.register(EchoTool);
        Self {
            provider,
            memory,
            tools,
            config,
        }
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

        #[allow(clippy::never_loop)] // v0: single-turn; full loop wired in follow-up
        loop {
            if session.iteration >= max_iter {
                info!("max iterations reached");
                session.finish(SessionStatus::Done);
                break;
            }

            debug!(iteration = session.iteration, "agent turn");
            let response = self.provider.complete(&messages).await?;

            let text = response.message.text().unwrap_or("").to_string();
            info!(
                tokens_out = response.usage.output_tokens,
                "← {}",
                &text[..text.len().min(120)]
            );

            // Record in session
            session.push(response.message.clone());

            // Persist to memory
            let ep = harness_memory::Episode::turn(
                session.id,
                "assistant",
                response.message.text().unwrap_or(""),
            );
            self.memory.insert(&ep).await?;

            // For v0: stop after one assistant turn (no tool loop yet)
            session.finish(SessionStatus::Done);
            break;
        }

        Ok(session)
    }
}
