use clap::Args;
use harness_core::config::Config;

#[derive(Args)]
pub struct ConfigArgs {
    /// Verify provider connectivity (not just static config)
    #[arg(long)]
    pub check: bool,
}

pub async fn execute(args: ConfigArgs) -> anyhow::Result<()> {
    let config = Config::load()?;
    println!("Provider:   {}", config.provider.backend);
    println!("Model:      {}", config.provider.model);
    println!("Max tokens: {}", config.provider.max_tokens);
    println!("Memory DB:  {}", config.memory.db_path.display());
    println!("Agent name: {}", config.agent.name);

    if args.check {
        check_api_key(&config);
        check_connectivity(&config).await;
    }
    Ok(())
}

fn check_api_key(config: &Config) {
    match config.resolved_api_key() {
        Some(_) => println!("API key:    [set]"),
        None => println!("API key:    [NOT SET] <- set ANTHROPIC_API_KEY"),
    }
}

async fn check_connectivity(config: &Config) {
    use harness_core::{
        message::Message,
        provider::{EchoProvider, Provider},
        providers::{ClaudeCodeProvider, ClaudeProvider},
    };
    use std::sync::Arc;
    use std::time::Instant;

    let backend = &config.provider.backend;

    let provider: Result<Arc<dyn Provider>, String> = match backend.as_str() {
        "echo" => Ok(Arc::new(EchoProvider)),
        "claude-code" | "cc" => Ok(Arc::new(ClaudeCodeProvider::new(&config.provider.model))),
        _ => ClaudeProvider::from_env(&config.provider.model, config.provider.max_tokens)
            .map(|p| Arc::new(p) as Arc<dyn Provider>)
            .map_err(|e| e.to_string()),
    };

    let provider = match provider {
        Ok(p) => p,
        Err(e) => {
            println!("Connectivity: FAILED (cannot create provider: {e})");
            return;
        }
    };

    print!("Connectivity: checking...");
    let _ = std::io::Write::flush(&mut std::io::stdout());

    let start = Instant::now();
    let ping = vec![Message::user("ping")];
    match provider.complete(&ping).await {
        Ok(resp) => {
            let elapsed = start.elapsed();
            print!("\r");
            println!("Connectivity: OK ({} -- {:.0?})", resp.model, elapsed,);
        }
        Err(e) => {
            print!("\r");
            println!("Connectivity: FAILED ({e})");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_displays_static_config() {
        let args = ConfigArgs { check: false };
        let result = execute(args).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn check_with_echo_provider_reports_ok() {
        let mut config = Config::default();
        config.provider.backend = "echo".to_string();
        check_connectivity(&config).await;
    }
}
