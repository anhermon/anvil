use std::io::Write as IoWrite;
use std::sync::Arc;

use clap::Args;
use futures::StreamExt;
use harness_core::{
    config::Config,
    provider::Provider,
    providers::{ClaudeCodeProvider, ClaudeProvider},
};
use harness_memory::MemoryDb;

use crate::agent::Agent;

#[derive(Args)]
pub struct RunArgs {
    /// Goal for this agent run
    #[arg(short, long)]
    pub goal: String,

    /// Provider backend override (claude, claude-code, cc, echo)
    #[arg(long, env = "HARNESS_PROVIDER")]
    pub provider: Option<String>,

    /// Stream response tokens to stdout as they arrive
    #[arg(long)]
    pub stream: bool,
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
        "claude-code" | "cc" => {
            // The --provider flag selects the backend; model comes from config.
            let model = &config.provider.model;
            tracing::info!(model = %model, "using ClaudeCodeProvider (subprocess)");
            Arc::new(ClaudeCodeProvider::new(model))
        }
        _ => Arc::new(
            ClaudeProvider::from_env(&config.provider.model, config.provider.max_tokens)
                .map_err(|e| anyhow::anyhow!("{}", e))?,
        ),
    };

    let memory = Arc::new(MemoryDb::open(&config.memory.db_path).await?);

    if args.stream {
        // Streaming mode: print tokens as they arrive, then persist to memory.
        let msgs = vec![
            harness_core::message::Message::system(
                config.agent.system_prompt.as_deref().unwrap_or(
                    "You are a helpful assistant. Complete the user's goal concisely.",
                ),
            ),
            harness_core::message::Message::user(&args.goal),
        ];

        println!("\n{}", "─".repeat(60));
        let mut token_stream = provider.stream(&msgs).await?;
        let mut full_text = String::new();
        let stdout = std::io::stdout();
        let mut out = stdout.lock();

        while let Some(chunk) = token_stream.next().await {
            let chunk = chunk?;
            if !chunk.delta.is_empty() {
                full_text.push_str(&chunk.delta);
                write!(out, "{}", chunk.delta)?;
                out.flush()?;
            }
            if chunk.done {
                break;
            }
        }

        writeln!(out)?;
        println!("{}", "─".repeat(60));

        // Persist the streamed response to memory.
        let ep = harness_memory::Episode::turn(uuid::Uuid::new_v4(), "assistant", &full_text);
        memory.insert(&ep).await?;

        println!("Streaming complete.");
    } else {
        let agent = Agent::new(provider, memory, config);
        let session = agent.run(&args.goal).await?;

        println!("\n{}", "─".repeat(60));
        if let Some(msg) = session.messages.last() {
            println!("{}", msg.text().unwrap_or("(no response)"));
        }
        println!("{}", "─".repeat(60));
        println!("Session: {} | Status: {:?}", session.id, session.status);
    }

    Ok(())
}
