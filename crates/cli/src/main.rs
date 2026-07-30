// Test code intentionally uses unwrap/expect/panic (a failed assertion should abort the
// test) and declares test-local items next to their use.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::items_after_statements
    )
)]
mod agent;
mod commands;
mod ui;

use clap::{Parser, Subcommand};
use tracing_subscriber::{fmt, EnvFilter};

/// anvil — forge your agents.
#[derive(Parser)]
#[command(name = "anvil", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, env = "RUST_LOG", default_value = "warn", global = true)]
    log_level: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Run an agent turn toward a goal
    Run(commands::run::RunArgs),
    /// Start an interactive multi-turn chat session
    Chat(commands::chat::ChatArgs),
    /// Show current configuration
    Config(commands::config::ConfigArgs),
    /// Manage and inspect memory
    Memory(commands::memory::MemoryArgs),
    /// Batch-evaluate agent against a JSONL test suite
    Eval(commands::eval::EvalArgs),
    /// Manage authentication credentials
    Auth(commands::auth::AuthArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Init tracing -- default to warn so UI output is not drowned by logs.
    fmt()
        .with_env_filter(EnvFilter::new(&cli.log_level))
        .with_target(false)
        .compact()
        .init();

    match cli.command {
        Commands::Run(args) => {
            // A run that never finished must not report success to an unattended
            // caller, so the terminal session status picks the exit code.
            let status = commands::run::execute(args).await?;
            let code = status.exit_code();
            if code != 0 {
                std::io::Write::flush(&mut std::io::stdout())?;
                std::process::exit(code);
            }
            Ok(())
        }
        Commands::Chat(args) => commands::chat::execute(args).await,
        Commands::Config(args) => commands::config::execute(args).await,
        Commands::Memory(args) => commands::memory::execute(args).await,
        Commands::Eval(args) => commands::eval::execute(args).await,
        Commands::Auth(args) => commands::auth::execute(args).await,
    }
}
