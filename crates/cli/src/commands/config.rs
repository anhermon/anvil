use clap::Args;
use harness_core::config::Config;

#[derive(Args)]
pub struct ConfigArgs {
    /// Also report whether the configured backend has the credentials it needs
    #[arg(long)]
    pub check: bool,
}

// Kept async for a uniform signature across command handlers.
#[allow(clippy::unused_async)]
pub async fn execute(args: ConfigArgs) -> anyhow::Result<()> {
    let config = Config::load()?;
    println!("Provider:   {}", config.provider.backend);
    println!("Model:      {}", config.provider.model);
    println!("Max tokens: {}", config.provider.max_tokens);
    println!("Memory DB:  {}", config.memory.db_path.display());
    println!("Agent name: {}", config.agent.name);

    if args.check {
        println!("Credentials: {}", credential_status(&config));
    }
    Ok(())
}

/// Only the `claude` backend reads `ANTHROPIC_API_KEY`; `claude-code` shells out
/// to the `claude` CLI and `ollama`/`echo` need no credentials at all. Reporting
/// a missing key for those backends sends new users hunting for a key they will
/// never need.
fn credential_status(config: &Config) -> String {
    match config.provider.backend.as_str() {
        "claude" => match config.resolved_api_key() {
            Some(_) => "[set]".to_string(),
            None => "[NOT SET] ← set ANTHROPIC_API_KEY, or run `claude auth login`".to_string(),
        },
        "claude-code" | "cc" => {
            "not required — this backend runs the `claude` CLI (must be on PATH)".to_string()
        }
        other => format!("not required by the `{other}` backend"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_backend(backend: &str) -> Config {
        let mut c = Config::default();
        c.provider.backend = backend.to_string();
        c
    }

    #[test]
    fn keyless_backends_do_not_demand_an_api_key() {
        // The regression: these used to print "[NOT SET] <- set ANTHROPIC_API_KEY".
        for backend in ["ollama", "echo", "claude-code", "cc"] {
            let status = credential_status(&config_with_backend(backend));
            assert!(
                !status.contains("ANTHROPIC_API_KEY"),
                "{backend} should not ask for ANTHROPIC_API_KEY, got: {status}"
            );
        }
    }
}
