// Match `lib.rs`: binary crate compiles the same modules; keep clippy policy aligned.
#![allow(
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::redundant_closure_for_method_calls,
    clippy::single_match_else,
    clippy::match_wildcard_for_single_variants,
    clippy::if_not_else,
    clippy::format_push_string,
    clippy::unused_self,
    clippy::unnecessary_literal_bound,
    clippy::map_unwrap_or,
    clippy::uninlined_format_args,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::default_trait_access,
    clippy::unused_async,
    clippy::unwrap_used,
    clippy::expect_used
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
    /// Show current configuration
    Config(commands::config::ConfigArgs),
    /// Manage and inspect memory
    Memory(commands::memory::MemoryArgs),
    /// Batch-evaluate agent against a JSONL test suite
    Eval(commands::eval::EvalArgs),
    /// Inspect and control evolution learning state
    Evolution(commands::evolution::EvolutionArgs),
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
        Commands::Evolution(args) => commands::evolution::execute(args).await,
        Commands::Auth(args) => commands::auth::execute(args).await,
    }
}
