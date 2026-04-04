use clap::Args;
use harness_core::config::Config;

#[derive(Args)]
pub struct ConfigArgs {
    /// Show resolved API key presence
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
        match config.resolved_api_key() {
            Some(_) => println!("API key:    [set]"),
            None => println!("API key:    [NOT SET] ← set ANTHROPIC_API_KEY"),
        }
    }
    Ok(())
}
