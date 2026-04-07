use clap::{Args, Subcommand};
use harness_core::config::Config;
use harness_memory::{self, MemoryDb, ScopeKind};

#[derive(Args)]
pub struct EvolutionArgs {
    #[command(subcommand)]
    command: EvolutionCommands,
}

#[derive(Subcommand)]
enum EvolutionCommands {
    Status,
    Log {
        #[arg(long, default_value_t = 20)]
        limit: i64,
        #[arg(long)]
        scope: Option<String>,
    },
    Diff {
        id: String,
    },
    Rollback {
        #[arg(long)]
        to: String,
    },
    Enable,
    Disable,
    Explain,
}

pub async fn execute(args: EvolutionArgs) -> anyhow::Result<()> {
    let config = Config::load()?;
    let memory = MemoryDb::open(&config.memory.db_path).await?;
    match args.command {
        EvolutionCommands::Status => status(&memory).await,
        EvolutionCommands::Log { limit, scope } => log_cmd(&memory, limit, scope.as_deref()).await,
        EvolutionCommands::Diff { id } => diff_cmd(&memory, &id).await,
        EvolutionCommands::Rollback { to } => rollback_cmd(&memory, &to).await,
        EvolutionCommands::Enable => {
            harness_memory::set_evolution_enabled(memory.pool(), true).await?;
            println!("Evolution enabled.");
            Ok(())
        }
        EvolutionCommands::Disable => {
            harness_memory::set_evolution_enabled(memory.pool(), false).await?;
            println!("Evolution disabled.");
            Ok(())
        }
        EvolutionCommands::Explain => explain_cmd(&memory).await,
    }
}

async fn status(memory: &MemoryDb) -> anyhow::Result<()> {
    let enabled = harness_memory::is_evolution_enabled(memory.pool()).await?;
    let scope = current_scope();
    let resolved = harness_memory::resolve_effective_overlay(memory.pool(), &scope).await?;
    let recent = harness_memory::get_recent_evolution_log(memory.pool(), 1).await?;
    println!("Enabled: {enabled}");
    if let Some(overlay) = resolved {
        println!(
            "Active overlay: {} (scope={} key={})",
            overlay.version.id,
            overlay.selected_scope.as_str(),
            overlay.selected_scope_key.as_deref().unwrap_or("-")
        );
    } else {
        println!("Active overlay: none");
    }
    if let Some(last) = recent.first() {
        println!(
            "Last outcome: {} | {} | {}",
            last.outcome_kind,
            last.prompt_score,
            last.created_at.to_rfc3339()
        );
    } else {
        println!("Last outcome: none");
    }
    Ok(())
}

async fn log_cmd(memory: &MemoryDb, limit: i64, scope: Option<&str>) -> anyhow::Result<()> {
    let scope_parsed = parse_scope(scope);
    let versions = harness_memory::list_prompt_versions(
        memory.pool(),
        scope_parsed.0.as_deref(),
        scope_parsed.1.as_deref(),
        limit,
    )
    .await?;
    if versions.is_empty() {
        println!("No prompt versions found.");
        return Ok(());
    }
    for version in versions {
        println!(
            "{} | scope={} key={} | active={} | score={}",
            version.id,
            version.scope_kind.as_str(),
            version.scope_key.as_deref().unwrap_or("-"),
            version.active,
            version.score_before
        );
    }
    Ok(())
}

async fn diff_cmd(memory: &MemoryDb, id: &str) -> anyhow::Result<()> {
    let version = harness_memory::get_prompt_version_by_id(memory.pool(), id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("prompt version not found: {id}"))?;
    println!("id: {}", version.id);
    println!(
        "scope: {} {}",
        version.scope_kind.as_str(),
        version.scope_key.unwrap_or_default()
    );
    println!("diff:\n{}", version.candidate_diff);
    Ok(())
}

async fn rollback_cmd(memory: &MemoryDb, id: &str) -> anyhow::Result<()> {
    harness_memory::rollback_prompt_version(memory.pool(), id).await?;
    println!("Rolled back active overlay to: {id}");
    Ok(())
}

async fn explain_cmd(memory: &MemoryDb) -> anyhow::Result<()> {
    let requested = current_scope();
    let resolved = harness_memory::resolve_effective_overlay(memory.pool(), &requested).await?;
    println!(
        "Requested scope: {} {}",
        requested.kind.as_str(),
        requested.key.unwrap_or_default()
    );
    if let Some(overlay) = resolved {
        println!(
            "Resolved scope: {} {} (fallback_used={})",
            overlay.selected_scope.as_str(),
            overlay.selected_scope_key.unwrap_or_default(),
            overlay.fallback_used
        );
        println!("Resolved version: {}", overlay.version.id);
    } else {
        println!("Resolved overlay: none");
    }
    Ok(())
}

fn parse_scope(scope: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(raw) = scope else {
        return (None, None);
    };
    if raw == "global" {
        return (Some("global".to_string()), None);
    }
    if let Some(rest) = raw.strip_prefix("workdir:") {
        return (Some("workdir".to_string()), Some(rest.to_string()));
    }
    (None, None)
}

fn current_scope() -> harness_memory::EvolutionScope {
    let key = std::env::current_dir()
        .ok()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .map(|p| p.to_string_lossy().to_string());
    match key {
        Some(scope_key) => harness_memory::EvolutionScope {
            kind: ScopeKind::Workdir,
            key: Some(scope_key),
        },
        None => harness_memory::EvolutionScope::global(),
    }
}
