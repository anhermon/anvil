use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::path::PathBuf;

use harness_github::{WebhookConfig, WebhookServer};

#[derive(Args)]
pub struct WebhookArgs {
    #[command(subcommand)]
    pub command: WebhookCommand,
}

#[derive(Subcommand)]
pub enum WebhookCommand {
    /// Start the GitHub webhook receiver server
    Serve(ServeArgs),
}

#[derive(Args)]
pub struct ServeArgs {
    /// Path to webhook config TOML file.
    /// See `anvil webhook serve --help` for the expected format.
    #[arg(long, short)]
    pub config: PathBuf,
}

pub async fn execute(args: WebhookArgs) -> Result<()> {
    match args.command {
        WebhookCommand::Serve(a) => serve(a).await,
    }
}

async fn serve(args: ServeArgs) -> Result<()> {
    let raw = std::fs::read_to_string(&args.config)
        .with_context(|| format!("reading webhook config from {}", args.config.display()))?;

    let config: WebhookConfig = toml::from_str(&raw).context("parsing webhook config TOML")?;

    WebhookServer::new(config).run().await
}
