//! `anvil paperclip` — run Anvil as a first-class Paperclip heartbeat adapter.
//!
//! # Subcommands
//!
//! - `anvil paperclip heartbeat` — run one heartbeat cycle (inbox → work → report)
//! - `anvil paperclip whoami`    — print agent identity and exit
//!
//! # Environment variables
//!
//! | Variable                | Description                                         |
//! |-------------------------|-----------------------------------------------------|
//! | `PAPERCLIP_API_URL`     | Paperclip base URL (default: http://127.0.0.1:3100) |
//! | `PAPERCLIP_API_KEY`     | Bearer token                                        |
//! | `PAPERCLIP_AGENT_ID`    | Agent UUID                                          |
//! | `PAPERCLIP_COMPANY_ID`  | Company UUID                                        |
//! | `PAPERCLIP_RUN_ID`      | Current run ID (attached to mutating requests)      |
//! | `ANTHROPIC_API_KEY`     | Anthropic API key for the claude provider           |

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use clap::{Parser, Subcommand};
use tracing::info;

use harness_core::{
    config::Config,
    message::{ContentBlock, MessageContent, Role},
    providers::claude::ClaudeProvider,
};
use harness_memory::MemoryDb;
use harness_paperclip::types::{HeartbeatContext, InboxItem};
use harness_paperclip::{
    heartbeat::ExecutionOutcome, HeartbeatConfig, HeartbeatLoop, PaperclipClient, TaskExecutor,
};

use crate::agent::Agent;

// ── CLI args ───────────────────────────────────────────────────────────────────

/// Paperclip control-plane integration (heartbeat, whoami)
#[derive(Parser)]
pub struct PaperclipArgs {
    /// Paperclip API base URL
    #[arg(
        long,
        env = "PAPERCLIP_API_URL",
        default_value = "http://127.0.0.1:3100"
    )]
    api_url: String,

    /// Paperclip API key (bearer token)
    #[arg(long, env = "PAPERCLIP_API_KEY")]
    api_key: Option<String>,

    /// Agent UUID
    #[arg(long, env = "PAPERCLIP_AGENT_ID")]
    agent_id: Option<String>,

    /// Company UUID
    #[arg(long, env = "PAPERCLIP_COMPANY_ID")]
    company_id: Option<String>,

    /// Current run ID (for audit trail headers on mutating requests)
    #[arg(long, env = "PAPERCLIP_RUN_ID")]
    run_id: Option<String>,

    #[command(subcommand)]
    command: PaperclipCommand,
}

#[derive(Subcommand)]
enum PaperclipCommand {
    /// Print agent identity and exit
    Whoami,
    /// Run one heartbeat cycle (poll inbox → checkout → run → report)
    Heartbeat(HeartbeatCmd),
}

#[derive(Parser)]
struct HeartbeatCmd {
    /// Maximum tasks to process in this heartbeat
    #[arg(long, default_value = "1")]
    max_tasks: usize,

    /// Anthropic API key for the claude provider
    #[arg(long, env = "ANTHROPIC_API_KEY")]
    anthropic_key: Option<String>,

    /// Model to use
    #[arg(long, env = "ANTHROPIC_MODEL", default_value = "claude-sonnet-4-6")]
    model: String,
}

// ── command entry point ────────────────────────────────────────────────────────

pub async fn execute(args: PaperclipArgs) -> Result<()> {
    let api_key = args
        .api_key
        .context("PAPERCLIP_API_KEY is required (set env var or --api-key)")?;

    let mut client = PaperclipClient::new(args.api_url.clone(), api_key);
    if let Some(run_id) = args.run_id {
        client = client.with_run_id(run_id);
    }

    match args.command {
        PaperclipCommand::Whoami => {
            let identity = client.get_identity().await?;
            println!("Agent:   {} ({})", identity.name, identity.id);
            println!("Company: {}", identity.company_id);
            println!("Role:    {}", identity.role);
            println!("Status:  {}", identity.status);
            Ok(())
        }

        PaperclipCommand::Heartbeat(cmd) => {
            let agent_id = args
                .agent_id
                .context("PAPERCLIP_AGENT_ID required for heartbeat")?;
            let company_id = args
                .company_id
                .context("PAPERCLIP_COMPANY_ID required for heartbeat")?;

            let anthropic_key = cmd
                .anthropic_key
                .context("ANTHROPIC_API_KEY required for heartbeat")?;

            // Build provider and memory once at the CLI level so every task
            // reuses the same DB connection and provider instance.
            let executor: Arc<dyn TaskExecutor> =
                Arc::new(AnvilExecutor::build(anthropic_key, cmd.model).await?);

            let config = HeartbeatConfig {
                agent_id,
                company_id,
                max_tasks_per_wake: cmd.max_tasks,
            };

            let hb = HeartbeatLoop::new(client, config, executor);
            let processed = hb.run_once().await?;
            info!(processed, "Heartbeat complete");
            println!("Heartbeat complete — {processed} task(s) processed");
            Ok(())
        }
    }
}

// ── Anvil executor ─────────────────────────────────────────────────────────────

/// Task executor that drives a full Anvil agent session for a Paperclip task.
///
/// Provider, memory DB, and base config are initialised once (via [`build`])
/// and reused across every task so that cross-task state is preserved and
/// resources are not wasted.
struct AnvilExecutor {
    provider: Arc<ClaudeProvider>,
    memory: Arc<MemoryDb>,
    config: Config,
}

impl AnvilExecutor {
    /// Build an executor, initialising the provider and memory DB exactly once.
    async fn build(anthropic_key: String, model: String) -> Result<Self> {
        let mut config = Config::load().unwrap_or_default();
        config.provider.api_key = Some(anthropic_key.clone());
        config.provider.model = model.clone();
        config.provider.backend = "claude".to_string();

        let provider = Arc::new(ClaudeProvider::new(
            anthropic_key,
            model,
            config.provider.max_tokens,
        ));
        let memory = Arc::new(
            MemoryDb::open(&config.memory.db_path)
                .await
                .context("open memory db for heartbeat executor")?,
        );

        Ok(Self {
            provider,
            memory,
            config,
        })
    }

    /// Extract goal from issue description.  Uses the ## Objective section
    /// from the harness spec format, falling back to the title.
    fn extract_goal(title: &str, description: &str) -> String {
        if let Some(idx) = description.find("## Objective") {
            let after = &description[idx + "## Objective".len()..];
            let paragraph: String = after
                .lines()
                .skip(1)
                .take_while(|l| !l.starts_with("##"))
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if !paragraph.is_empty() {
                return format!("{title}\n\n{paragraph}");
            }
        }
        title.to_string()
    }
}

#[async_trait]
impl TaskExecutor for AnvilExecutor {
    async fn execute(
        &self,
        item: &InboxItem,
        context: &HeartbeatContext,
    ) -> Result<ExecutionOutcome> {
        let description = context
            .issue
            .description
            .as_deref()
            .unwrap_or(item.title.as_str());

        let goal = Self::extract_goal(&item.title, description);

        info!(
            issue = %item.identifier,
            goal_preview = %goal.chars().take(120).collect::<String>(),
            "Running Anvil agent for Paperclip task"
        );

        let agent = Agent::new(
            self.provider.clone(),
            self.memory.clone(),
            self.config.clone(),
        );
        let session = agent.run(&goal).await?;

        // Extract last assistant message text as the completion comment
        let last_assistant_text = session
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .and_then(|m| match &m.content {
                MessageContent::Text(t) => Some(t.clone()),
                MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| {
                    if let ContentBlock::Text { text } = b {
                        Some(text.clone())
                    } else {
                        None
                    }
                }),
            })
            .unwrap_or_else(|| "Task complete — no assistant response recorded.".to_string());

        let comment = format!(
            "## Anvil Agent Output\n\n{last_assistant_text}\n\n---\n_Executed by `anvil paperclip heartbeat` (rust-harness)._"
        );

        Ok(ExecutionOutcome::Done(comment))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_goal_uses_objective_section() {
        let desc = "## Objective\nDo the thing.\n\n## Scope\nFoo bar.";
        let goal = AnvilExecutor::extract_goal("My Task", desc);
        assert_eq!(goal, "My Task\n\nDo the thing.");
    }

    #[test]
    fn extract_goal_falls_back_to_title_when_no_objective() {
        let desc = "## Scope\nFoo bar.\n\n## Verification\n- [ ] thing";
        let goal = AnvilExecutor::extract_goal("My Task", desc);
        assert_eq!(goal, "My Task");
    }

    #[test]
    fn extract_goal_falls_back_to_title_when_objective_is_empty() {
        // Objective section exists but has no text before the next heading
        let desc = "## Objective\n## Scope\nFoo bar.";
        let goal = AnvilExecutor::extract_goal("My Task", desc);
        assert_eq!(goal, "My Task");
    }

    #[test]
    fn extract_goal_objective_at_end_of_string() {
        // Objective is the last section — no subsequent ## heading
        let desc = "## Scope\nFoo.\n\n## Objective\nDo the thing at the end.";
        let goal = AnvilExecutor::extract_goal("My Task", desc);
        assert_eq!(goal, "My Task\n\nDo the thing at the end.");
    }
}
