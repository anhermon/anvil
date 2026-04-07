use std::sync::Arc;
use std::time::Instant;

use harness_cli::agent::{Agent, RunOptions};
use harness_core::config::Config;
use harness_core::message::{ContentBlock, MessageContent, Role};
use harness_core::provider::Provider;
use harness_core::session::Session;
use harness_memory::{EvolutionScope, MemoryDb, ScopeKind};

use crate::scoring::EloRating;
use crate::tasks::{TaskEvalContext, ALL_TASKS};

/// Match `Agent::resolve_learning_scope` so overlay telemetry uses the same DB keys
/// as evolution apply/activate (workdir when cwd resolves, else global).
fn bench_evolution_scope() -> EvolutionScope {
    let key = std::env::current_dir()
        .ok()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .map(|p| p.to_string_lossy().to_string());
    match key {
        Some(scope_key) => EvolutionScope {
            kind: ScopeKind::Workdir,
            key: Some(scope_key),
        },
        None => EvolutionScope::global(),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunResult {
    pub iteration: usize,
    pub task_name: String,
    pub duration_ms: u64,
    pub iterations_used: usize,
    pub tool_calls: usize,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub criteria_met: bool,
    pub criteria_details: String,
    pub final_text: String,
    pub evolution_applied: bool,
    pub elo_before: f64,
    pub elo_after: f64,
}

pub async fn run_iteration(
    provider: Arc<dyn Provider>,
    memory: Arc<MemoryDb>,
    config: Config,
    iteration: usize,
    max_turns: usize,
) -> anyhow::Result<Vec<RunResult>> {
    let mut results = Vec::new();
    let mut elo = EloRating::new();
    let scope = bench_evolution_scope();

    let artifact_dir = std::env::temp_dir().join(format!(
        "anvil-bench-iter{iteration}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&artifact_dir)?;
    let eval_ctx = TaskEvalContext {
        multi_tool_output: Some(artifact_dir.join("bench_provider_files.txt")),
    };

    for task in ALL_TASKS {
        let overlay_before = harness_memory::resolve_effective_overlay(memory.pool(), &scope)
            .await
            .ok()
            .flatten();

        let t0 = Instant::now();

        let mut run_config = config.clone();
        run_config.agent.max_iterations = max_turns;
        run_config.agent.system_prompt = None;

        let agent = Agent::new(
            Arc::clone(&provider),
            Arc::clone(&memory),
            run_config,
        );

        let goal = task.goal_for_run(&eval_ctx);
        let session = agent
            .run_with_options(
                goal.as_str(),
                RunOptions {
                    session_name: Some(format!("bench-iter{}", iteration)),
                    max_iterations: Some(max_turns),
                },
            )
            .await?;

        let elapsed_ms = t0.elapsed().as_millis() as u64;

        let (criteria_met, criteria_details) = task.evaluate(&session, &eval_ctx);

        let tool_calls = count_tool_calls(&session);

        let (input_tokens, output_tokens) = estimate_tokens(&session);

        let overlay_after =
            harness_memory::resolve_effective_overlay(memory.pool(), &scope)
                .await
                .ok()
                .flatten();
        let evolution_applied = match (&overlay_before, &overlay_after) {
            (None, None) => false,
            (None, Some(_)) => true,
            (Some(a), Some(b)) => a.version.id != b.version.id,
            (Some(_), None) => true,
        };

        let elo_before = elo.get_rating(task.name);
        elo.update(task.name, criteria_met);
        let elo_after = elo.get_rating(task.name);

        let final_text = session
            .messages
            .last()
            .and_then(|m| m.text())
            .unwrap_or("")
            .to_string();

        results.push(RunResult {
            iteration,
            task_name: task.name.to_string(),
            duration_ms: elapsed_ms,
            iterations_used: session.iteration,
            tool_calls,
            input_tokens,
            output_tokens,
            criteria_met,
            criteria_details,
            final_text,
            evolution_applied,
            elo_before,
            elo_after,
        });
    }

    Ok(results)
}

fn count_tool_calls(session: &Session) -> usize {
    session
        .messages
        .iter()
        .filter(|m| matches!(m.role, Role::Assistant))
        .flat_map(|m| match &m.content {
            MessageContent::Blocks(blocks) => blocks.iter().collect::<Vec<_>>(),
            _ => vec![],
        })
        .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
        .count()
}

fn estimate_tokens(session: &Session) -> (u32, u32) {
    let mut input: u32 = 0;
    let mut output: u32 = 0;
    for msg in &session.messages {
        if let Some(text) = msg.text() {
            let words = text.split_whitespace().count() as u32;
            let tokens = words * 4 / 3;
            match msg.role {
                Role::User | Role::System => input += tokens,
                Role::Assistant => output += tokens,
                Role::Tool => input += tokens,
            }
        }
    }
    (input, output)
}
