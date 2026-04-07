use std::sync::Arc;

use clap::Parser;
use harness_core::config::Config;
use harness_core::provider::Provider;
use harness_core::providers::OllamaProvider;
use harness_memory::MemoryDb;

mod report;
mod runner;
mod scoring;
mod tasks;

#[derive(Parser, Debug)]
#[command(name = "anvil-bench", about = "E2E benchmark runner for anvil agents")]
struct Args {
    /// Model identifier (e.g. "glm-4.7-flash")
    #[arg(long, default_value = "glm-4.7-flash")]
    model: String,

    /// Provider backend (ollama, echo)
    #[arg(long, default_value = "ollama")]
    provider: String,

    /// Number of benchmark iterations
    #[arg(long, default_value_t = 5)]
    iterations: usize,

    /// Ollama base URL
    #[arg(long, default_value = "http://localhost:11434")]
    base_url: String,

    /// Max iterations per agent turn
    #[arg(long, default_value_t = 15)]
    max_turns: usize,

    /// Turn off session post-processing (no prompt overlay apply). Compare pass rates vs default runs.
    #[arg(long, default_value_t = false)]
    disable_evolution: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("anvil_bench=info".parse()?)
                .add_directive("harness_core=info".parse()?)
                .add_directive("harness_cli=info".parse()?)
        )
        .init();

    let args = Args::parse();

    let provider: Arc<dyn Provider> = match args.provider.as_str() {
        "echo" => {
            tracing::info!("using echo provider (no LLM calls)");
            Arc::new(harness_core::provider::EchoProvider)
        }
        _ => {
            tracing::info!(
                model = %args.model,
                base_url = %args.base_url,
                "using OllamaProvider"
            );
            Arc::new(OllamaProvider::new(&args.base_url, &args.model, 8192))
        }
    };

    let config = Config::load()?;
    let mut all_results = Vec::new();

    for iteration in 1..=args.iterations {
        tracing::info!(iteration, total = args.iterations, "starting benchmark iteration");

        let memory = Arc::new(MemoryDb::in_memory().await?);
        if args.disable_evolution {
            harness_memory::set_evolution_enabled(memory.pool(), false).await?;
        }

        let iteration_results = runner::run_iteration(
            Arc::clone(&provider),
            Arc::clone(&memory),
            config.clone(),
            iteration,
            args.max_turns,
        )
        .await?;

        all_results.extend(iteration_results);
    }

    if args.disable_evolution {
        println!("Note: --disable-evolution: post-session learning disabled; expect Evo+ = 0.\n");
    }

    let report = report::generate(&all_results, args.iterations);
    println!("{}", report);

    Ok(())
}
