//! `harness auth` subcommand -- show the active authentication method.

use clap::{Args, Subcommand};
use harness_core::{auth::AuthMethod, config::Config};

#[derive(Args)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub command: AuthCommand,
}

#[derive(Subcommand)]
pub enum AuthCommand {
    /// Show which auth method is active and its source
    Status,
}

pub async fn execute(args: AuthArgs) -> anyhow::Result<()> {
    match args.command {
        AuthCommand::Status => status().await,
    }
}

async fn status() -> anyhow::Result<()> {
    let config = Config::load()?;

    match AuthMethod::resolve() {
        Ok(auth) => match auth {
            AuthMethod::BearerToken { ref source, .. } => {
                println!("Auth method: subscription (Bearer token)");
                println!("Source:      {}", source.display());
                println!("Model:       {}", config.provider.model);
            }
            AuthMethod::ApiKey(_) => {
                println!("Auth method: API key");
                println!("Source:      ANTHROPIC_API_KEY env var");
                println!("Model:       {}", config.provider.model);
            }
        },
        Err(e) => {
            println!("Auth method: none");
            println!("Error:       {e}");
            anyhow::bail!("no valid auth credentials found");
        }
    }

    Ok(())
}
