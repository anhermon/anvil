use std::collections::HashMap;

use crate::runner::RunResult;

/// Aggregate scores for one outer benchmark iteration (one cold memory + full task suite).
#[derive(Debug, Clone, PartialEq)]
pub struct IterationRollup {
    pub iteration: usize,
    pub task_count: usize,
    pub pass_count: usize,
    pub pass_rate: f64,
    /// Mean `session.iteration` across tasks (agent loop count; lower usually means faster completion).
    pub mean_session_iterations: f64,
    pub mean_tool_calls: f64,
    /// Count of tasks where the active evolution overlay changed after that task.
    pub evolution_events: usize,
}

pub fn compute_rollups(results: &[RunResult]) -> Vec<IterationRollup> {
    let mut groups: HashMap<usize, Vec<&RunResult>> = HashMap::new();
    for r in results {
        groups.entry(r.iteration).or_default().push(r);
    }

    let mut keys: Vec<usize> = groups.keys().copied().collect();
    keys.sort_unstable();

    keys.into_iter()
        .map(|iteration| {
            let runs = groups
                .get(&iteration)
                .expect("iteration key from groups");
            let task_count = runs.len();
            let pass_count = runs.iter().filter(|r| r.criteria_met).count();
            let pass_rate = if task_count == 0 {
                0.0
            } else {
                pass_count as f64 / task_count as f64
            };
            let mean_session_iterations = if task_count == 0 {
                0.0
            } else {
                runs.iter().map(|r| r.iterations_used as f64).sum::<f64>() / task_count as f64
            };
            let mean_tool_calls = if task_count == 0 {
                0.0
            } else {
                runs.iter().map(|r| r.tool_calls as f64).sum::<f64>() / task_count as f64
            };
            let evolution_events = runs.iter().filter(|r| r.evolution_applied).count();

            IterationRollup {
                iteration,
                task_count,
                pass_count,
                pass_rate,
                mean_session_iterations,
                mean_tool_calls,
                evolution_events,
            }
        })
        .collect()
}

/// Compare first and last outer iterations. Does **not** imply evolution caused the delta
/// (each outer iteration is an independent cold start).
#[derive(Debug, Clone, PartialEq)]
pub struct OuterIterDelta {
    pub pass_rate_delta: f64,
    pub mean_session_iterations_delta: f64,
}

pub fn outer_iteration_delta(rollups: &[IterationRollup]) -> Option<OuterIterDelta> {
    if rollups.len() < 2 {
        return None;
    }
    let first = rollups.first()?;
    let last = rollups.last()?;
    Some(OuterIterDelta {
        pass_rate_delta: last.pass_rate - first.pass_rate,
        mean_session_iterations_delta: last.mean_session_iterations - first.mean_session_iterations,
    })
}

/// Stability of each task across **outer** benchmark iterations (same task name, many runs).
#[derive(Debug, Clone, PartialEq)]
pub struct TaskStabilityRollup {
    pub task_name: String,
    pub runs: usize,
    pub pass_count: usize,
    pub pass_rate: f64,
    pub median_duration_ms: u64,
    pub median_agent_iters: usize,
}

fn median_u64(mut xs: Vec<u64>) -> u64 {
    if xs.is_empty() {
        return 0;
    }
    xs.sort_unstable();
    let mid = xs.len() / 2;
    if xs.len() % 2 == 0 {
        (xs[mid - 1] + xs[mid]) / 2
    } else {
        xs[mid]
    }
}

fn median_usize(mut xs: Vec<usize>) -> usize {
    if xs.is_empty() {
        return 0;
    }
    xs.sort_unstable();
    let mid = xs.len() / 2;
    if xs.len() % 2 == 0 {
        (xs[mid - 1] + xs[mid]) / 2
    } else {
        xs[mid]
    }
}

pub fn compute_task_stability(results: &[RunResult]) -> Vec<TaskStabilityRollup> {
    let mut by_task: HashMap<String, Vec<&RunResult>> = HashMap::new();
    for r in results {
        by_task.entry(r.task_name.clone()).or_default().push(r);
    }
    let mut names: Vec<String> = by_task.keys().cloned().collect();
    names.sort();

    names
        .into_iter()
        .map(|task_name| {
            let runs_ref = by_task
                .get(&task_name)
                .expect("task_name from keys");
            let n = runs_ref.len();
            let pass_count = runs_ref.iter().filter(|r| r.criteria_met).count();
            let pass_rate = if n == 0 {
                0.0
            } else {
                pass_count as f64 / n as f64
            };
            let median_duration_ms = median_u64(runs_ref.iter().map(|r| r.duration_ms).collect());
            let median_agent_iters =
                median_usize(runs_ref.iter().map(|r| r.iterations_used).collect());

            TaskStabilityRollup {
                task_name,
                runs: n,
                pass_count,
                pass_rate,
                median_duration_ms,
                median_agent_iters,
            }
        })
        .collect()
}

pub fn generate(results: &[RunResult], total_iterations: usize) -> String {
    let mut buf = String::new();

    buf.push_str("=== ANVIL E2E BENCHMARK REPORT ===\n\n");
    buf.push_str(&format!(
        "Outer benchmark iterations: {total_iterations} (each uses a fresh in-memory DB)\n\n"
    ));

    buf.push_str("## Per-Task Results\n\n");
    buf.push_str(&format!(
        "{:<22} {:>6} {:>10} {:>10} {:>10} {:>8} {:>6}\n",
        "Task", "Iter", "Time(ms)", "ToolCalls", "AgentIters", "ELO", "Pass"
    ));
    buf.push_str(&"-".repeat(86));
    buf.push('\n');

    for r in results {
        buf.push_str(&format!(
            "{:<22} {:>6} {:>10} {:>10} {:>10} {:>8} {:>6}\n",
            r.task_name,
            r.iteration,
            r.duration_ms,
            r.tool_calls,
            r.iterations_used,
            r.elo_after.round() as i64,
            if r.criteria_met { "PASS" } else { "FAIL" }
        ));
    }

    buf.push('\n');

    let rollups = compute_rollups(results);
    buf.push_str("## Scores by outer iteration\n\n");
    buf.push_str(
        "Pass rate and mean agent loop count are comparable within a row. \
Outer iterations are independent cold starts — differences between rows mainly reflect provider variance, \
not accumulated learning across those iterations.\n\n",
    );
    buf.push_str(&format!(
        "{:<6} {:>10} {:>18} {:>16} {:>10}\n",
        "Iter", "Pass rate", "Mean agent loops", "Mean tool calls", "Evo+"
    ));
    buf.push_str(&"-".repeat(64));
    buf.push('\n');

    for s in &rollups {
        buf.push_str(&format!(
            "{:<6} {:>9.0}% {:>18.2} {:>16.2} {:>10}\n",
            s.iteration,
            s.pass_rate * 100.0,
            s.mean_session_iterations,
            s.mean_tool_calls,
            s.evolution_events
        ));
    }

    buf.push('\n');

    let evolution_runs: Vec<_> = results.iter().filter(|r| r.evolution_applied).collect();
    buf.push_str("## Evolution (within each outer iteration)\n\n");
    if evolution_runs.is_empty() {
        buf.push_str("No evolution overlay changes were recorded after any task.\n");
        buf.push_str("Typical reasons:\n");
        buf.push_str("- Critic score ≥ 0.75 (generator skips)\n");
        buf.push_str("- Validators vetoed candidates\n");
        buf.push_str("- `evolution_settings` disabled in DB\n\n");
    } else {
        buf.push_str(&format!(
            "Recorded {} post-task overlay change(s) (new or replaced active prompt version).\n\n",
            evolution_runs.len()
        ));
    }

    if let Some(delta) = outer_iteration_delta(&rollups) {
        buf.push_str("## Outer iter 1 vs last — variance check only\n\n");
        buf.push_str(&format!(
            "Δ pass rate: {:+.0}% (last − first)\n",
            delta.pass_rate_delta * 100.0
        ));
        buf.push_str(&format!(
            "Δ mean agent loops / task: {:+.2} (positive means slower on last outer iter)\n\n",
            delta.mean_session_iterations_delta
        ));
        buf.push_str(
            "Do not treat the above as proof that evolution helped or hurt: \
each outer iteration starts from an empty overlay state. \
To measure learning impact, compare controlled A/B runs (same tasks, overlay on vs off) with many seeds.\n\n",
        );
    }

    let task_stab = compute_task_stability(results);
    if !task_stab.is_empty() && total_iterations >= 2 {
        buf.push_str("## Per-task stability (across outer iterations)\n\n");
        buf.push_str(
            "Median duration and median agent loops summarize run-to-run variance for the same task. \
Raise `--iterations` for more reliable medians.\n\n",
        );
        buf.push_str(&format!(
            "{:<22} {:>5} {:>10} {:>14} {:>18}\n",
            "Task", "Runs", "Pass %", "Med time(ms)", "Med agent loops"
        ));
        buf.push_str(&"-".repeat(74));
        buf.push('\n');
        for t in &task_stab {
            buf.push_str(&format!(
                "{:<22} {:>5} {:>9.0}% {:>14} {:>18}\n",
                t.task_name,
                t.runs,
                t.pass_rate * 100.0,
                t.median_duration_ms,
                t.median_agent_iters
            ));
        }
        buf.push('\n');
    }

    buf.push_str("## Criteria Details\n\n");
    for r in results {
        buf.push_str(&format!(
            "**{}** (iter {}): {}\n",
            r.task_name, r.iteration, r.criteria_details
        ));
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::RunResult;

    fn sample_run(
        iteration: usize,
        criteria_met: bool,
        iterations_used: usize,
        tool_calls: usize,
        evolution_applied: bool,
    ) -> RunResult {
        RunResult {
            iteration,
            task_name: "dummy".to_string(),
            duration_ms: 1,
            iterations_used,
            tool_calls,
            input_tokens: 0,
            output_tokens: 0,
            criteria_met,
            criteria_details: String::new(),
            final_text: String::new(),
            evolution_applied,
            elo_before: 1200.0,
            elo_after: 1200.0,
        }
    }

    #[test]
    fn rollup_pass_rate_and_means() {
        let results = vec![
            sample_run(1, true, 2, 1, false),
            sample_run(1, false, 4, 2, false),
            sample_run(1, true, 2, 0, true),
        ];
        let roll = compute_rollups(&results);
        assert_eq!(roll.len(), 1);
        assert!((roll[0].pass_rate - 2.0 / 3.0).abs() < 1e-9);
        assert!((roll[0].mean_session_iterations - 8.0 / 3.0).abs() < 1e-9);
        assert!((roll[0].mean_tool_calls - 1.0).abs() < 1e-9);
        assert_eq!(roll[0].evolution_events, 1);
    }

    #[test]
    fn task_stability_median_and_pass_rate() {
        let results = vec![
            RunResult {
                iteration: 1,
                task_name: "tool_call_basic".to_string(),
                duration_ms: 100,
                iterations_used: 2,
                tool_calls: 1,
                input_tokens: 0,
                output_tokens: 0,
                criteria_met: true,
                criteria_details: String::new(),
                final_text: String::new(),
                evolution_applied: false,
                elo_before: 1200.0,
                elo_after: 1200.0,
            },
            RunResult {
                iteration: 2,
                task_name: "tool_call_basic".to_string(),
                duration_ms: 300,
                iterations_used: 4,
                tool_calls: 2,
                input_tokens: 0,
                output_tokens: 0,
                criteria_met: false,
                criteria_details: String::new(),
                final_text: String::new(),
                evolution_applied: false,
                elo_before: 1200.0,
                elo_after: 1200.0,
            },
            RunResult {
                iteration: 3,
                task_name: "tool_call_basic".to_string(),
                duration_ms: 200,
                iterations_used: 3,
                tool_calls: 1,
                input_tokens: 0,
                output_tokens: 0,
                criteria_met: true,
                criteria_details: String::new(),
                final_text: String::new(),
                evolution_applied: false,
                elo_before: 1200.0,
                elo_after: 1200.0,
            },
        ];
        let st = compute_task_stability(&results);
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].runs, 3);
        assert!((st[0].pass_rate - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(st[0].median_duration_ms, 200);
        assert_eq!(st[0].median_agent_iters, 3);
    }

    #[test]
    fn outer_iteration_delta_requires_two_iters() {
        assert!(outer_iteration_delta(&compute_rollups(&[
            sample_run(1, true, 1, 0, false),
        ]))
        .is_none());

        let two = vec![
            sample_run(1, true, 2, 1, false),
            sample_run(2, false, 3, 2, false),
        ];
        let roll = compute_rollups(&two);
        let d = outer_iteration_delta(&roll).unwrap();
        assert!((d.pass_rate_delta - (-1.0)).abs() < 1e-9);
        assert!((d.mean_session_iterations_delta - 1.0).abs() < 1e-9);
    }
}
