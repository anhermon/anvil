use std::sync::Arc;

use clap::Args;
use harness_core::{config::Config, provider::Provider, providers::ClaudeProvider};
use harness_memory::MemoryDb;

use crate::agent::Agent;

#[derive(Args)]
pub struct RunArgs {
    /// Goal for this agent run
    #[arg(short, long)]
    pub goal: String,

    /// Provider backend override (claude, echo)
    #[arg(long, env = "HARNESS_PROVIDER")]
    pub provider: Option<String>,
}

pub async fn execute(args: RunArgs) -> anyhow::Result<()> {
    let config = Config::load()?;

    let backend = args
        .provider
        .as_deref()
        .unwrap_or(&config.provider.backend)
        .to_string();
    let provider: Arc<dyn Provider> = match backend.as_str() {
        "echo" => {
            tracing::info!("using echo provider (no LLM calls)");
            Arc::new(harness_core::provider::EchoProvider)
        }
        _ => {
            let api_key = config.resolved_api_key().ok_or_else(|| {
                anyhow::anyhow!("ANTHROPIC_API_KEY not set — pass via env or config")
            })?;
            Arc::new(ClaudeProvider::new(
                api_key,
                &config.provider.model,
                config.provider.max_tokens,
            ))
        }
    };

    let memory = Arc::new(MemoryDb::open(&config.memory.db_path).await?);

    let agent = Agent::new(provider, memory, config);
    let session = agent.run(&args.goal).await?;

    println!("\n{}", "─".repeat(60));
    if let Some(msg) = session.messages.last() {
        println!("{}", msg.text().unwrap_or("(no response)"));
    }
    println!("{}", "─".repeat(60));
    println!("Session: {} | Status: {:?}", session.id, session.status);

    Ok(())
}
