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
        Commands::Run(args) => commands::run::execute(args).await,
        Commands::Config(args) => commands::config::execute(args).await,
        Commands::Memory(args) => commands::memory::execute(args).await,
        Commands::Eval(args) => commands::eval::execute(args).await,
        Commands::Auth(args) => commands::auth::execute(args).await,
    }
}
