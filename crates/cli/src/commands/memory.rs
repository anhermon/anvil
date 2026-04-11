use clap::{Args, Subcommand};
use harness_core::config::Config;
use harness_memory::MemoryDb;
use uuid::Uuid;

#[derive(Args)]
pub struct MemoryArgs {
    #[command(subcommand)]
    command: MemoryCommands,
}

#[derive(Subcommand)]
enum MemoryCommands {
    /// Search memory for a query
    Search {
        query: String,
        #[arg(short, long, default_value = "10")]
        limit: i64,
    },
    /// Show recent episodes for a session
    Recent {
        session_id: Uuid,
        #[arg(short, long, default_value = "20")]
        limit: i64,
    },
    /// Show recent agent self-evolution records
    Evolution {
        #[arg(short, long, default_value = "20")]
        limit: i64,
    },
}

pub async fn execute(args: MemoryArgs) -> anyhow::Result<()> {
    let config = Config::load()?;
    let memory = MemoryDb::open(&config.memory.db_path).await?;

    match args.command {
        MemoryCommands::Search { query, limit } => {
            let results = memory.search(&query, limit).await?;
            if results.is_empty() {
                println!("No results for: {query}");
            }
            for ep in results {
                println!(
                    "[{}] {}: {}",
                    ep.created_at.format("%Y-%m-%d %H:%M"),
                    ep.role,
                    &ep.content[..ep.content.len().min(120)]
                );
            }
        }
        MemoryCommands::Recent { session_id, limit } => {
            let results = memory.recent(session_id, limit).await?;
            for ep in results {
                println!(
                    "[{}] {}: {}",
                    ep.created_at.format("%Y-%m-%d %H:%M"),
                    ep.role,
                    &ep.content[..ep.content.len().min(120)]
                );
            }
        }
        MemoryCommands::Evolution { limit } => {
            let records = harness_memory::query_evolution_log(memory.pool(), limit).await?;
            if records.is_empty() {
                println!("No evolution records found.");
            }
            for rec in records {
                println!(
                    "[{}] Score: {:.2} | Kind: {} | Details: {}",
                    rec.created_at.format("%Y-%m-%d %H:%M"),
                    rec.prompt_score,
                    rec.outcome_kind,
                    rec.outcome_detail
                );
            }
        }
    }

    Ok(())
}
