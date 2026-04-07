use std::path::PathBuf;

use harness_core::session::Session;

/// Task suite: `Hard` appends `crate_dirs_manifest` (strict filesystem check).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BenchTier {
    #[default]
    Default,
    Hard,
}

/// Per–outer-iteration paths for graders (isolated temp dir; see `runner::run_iteration`).
#[derive(Clone, Default)]
pub struct TaskEvalContext {
    /// Output file for `multi_tool_task`; set by the benchmark runner.
    pub multi_tool_output: Option<PathBuf>,
    /// Repo-relative path for `crate_dirs_manifest` (`Hard` tier only).
    pub hard_crate_dirs_manifest: Option<PathBuf>,
}

pub struct BenchmarkTask {
    pub name: &'static str,
    /// Default goal when no dynamic path is required.
    pub goal: &'static str,
    #[allow(dead_code)]
    pub expected_tool_calls_min: usize,
    pub evaluate: fn(&Session, &TaskEvalContext) -> (bool, String),
}

impl BenchmarkTask {
    pub fn goal_for_run(&self, ctx: &TaskEvalContext) -> String {
        if self.name == "multi_tool_task" {
            let p = ctx
                .multi_tool_output
                .as_ref()
                .expect("multi_tool_task requires multi_tool_output in TaskEvalContext");
            format!(
                "Search for all Rust files that contain the word 'Provider' trait. \
Write the complete list of matching file paths to this file (use this exact path): {}",
                p.display()
            )
        } else if self.name == "crate_dirs_manifest" {
            let p = ctx
                .hard_crate_dirs_manifest
                .as_ref()
                .expect("crate_dirs_manifest requires hard_crate_dirs_manifest in TaskEvalContext");
            format!(
                "List every immediate subdirectory of the `crates/` directory at the repository root. \
Sort the directory names lexicographically. Write exactly one name per line (UTF-8), no extra text, \
to this path relative to the repo root: {}",
                p.display()
            )
        } else {
            self.goal.to_string()
        }
    }

    pub fn evaluate(&self, session: &Session, ctx: &TaskEvalContext) -> (bool, String) {
        (self.evaluate)(session, ctx)
    }
}

fn expected_sorted_crate_dir_names() -> Result<Vec<String>, String> {
    let crates = std::path::Path::new("crates");
    if !crates.is_dir() {
        return Err(format!("{} is not a directory (run anvil-bench from repo root)", crates.display()));
    }
    let mut names: Vec<String> = std::fs::read_dir(crates)
        .map_err(|e| format!("read_dir crates: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    Ok(names)
}

pub const TOOL_CALL_BASIC: BenchmarkTask = BenchmarkTask {
    name: "tool_call_basic",
    goal: "List the files in the current directory. Just output the filenames, one per line.",
    expected_tool_calls_min: 1,
    evaluate: |session: &Session, _ctx: &TaskEvalContext| {
        let final_text = session.messages.last().and_then(|m| m.text()).unwrap_or("");

        let known_files = [
            "Cargo.toml",
            "README.md",
            "Taskfile.yml",
            "AGENTS.md",
            "crates",
        ];
        let found: Vec<&str> = known_files
            .iter()
            .filter(|f| final_text.contains(*f))
            .copied()
            .collect();

        let met = !found.is_empty();
        let detail = if met {
            format!("Found {} known files: {}", found.len(), found.join(", "))
        } else {
            format!(
                "No known anvil files found in output ({} chars)",
                final_text.len()
            )
        };
        (met, detail)
    },
};

pub const SUMMARIZE_TEXT: BenchmarkTask = BenchmarkTask {
    name: "summarize_text",
    goal: "Read the README.md file and give me a 2-sentence summary of what this project is.",
    expected_tool_calls_min: 1,
    evaluate: |session: &Session, _ctx: &TaskEvalContext| {
        let final_text = session.messages.last().and_then(|m| m.text()).unwrap_or("");

        let text_lower = final_text.to_lowercase();
        let mentions_project = text_lower.contains("anvil")
            || text_lower.contains("harness")
            || text_lower.contains("agent");
        let concise = final_text.split('.').count() <= 5;

        let met = mentions_project && concise;
        let detail = if met {
            format!(
                "Summary mentions project concepts and is concise ({} sentences)",
                final_text.split('.').count()
            )
        } else {
            format!(
                "Summary quality issue: mentions_project={}, concise={}",
                mentions_project, concise
            )
        };
        (met, detail)
    },
};

pub const MULTI_TOOL_TASK: BenchmarkTask = BenchmarkTask {
    name: "multi_tool_task",
    goal: "(legacy; use goal_for_run)",
    expected_tool_calls_min: 2,
    evaluate: |session: &Session, ctx: &TaskEvalContext| {
        let final_text = session
            .messages
            .last()
            .and_then(|m| m.text())
            .unwrap_or("");

        let path = ctx
            .multi_tool_output
            .as_ref()
            .expect("multi_tool_output required");
        let file_written = path.exists();

        let mentions_result = final_text.to_lowercase().contains("provider")
            || final_text.to_lowercase().contains("file");

        let met = file_written && mentions_result;
        let detail = if met {
            let content = std::fs::read_to_string(path).unwrap_or_default();
            let line_count = content.lines().count();
            format!("File written with {} entries", line_count)
        } else {
            format!(
                "Task incomplete: path={} file_written={}, mentions_result={}",
                path.display(),
                file_written,
                mentions_result
            )
        };
        (met, detail)
    },
};

pub const CRATE_DIRS_MANIFEST: BenchmarkTask = BenchmarkTask {
    name: "crate_dirs_manifest",
    goal: "(use goal_for_run)",
    expected_tool_calls_min: 2,
    evaluate: |session: &Session, ctx: &TaskEvalContext| {
        let path = ctx
            .hard_crate_dirs_manifest
            .as_ref()
            .expect("hard_crate_dirs_manifest required");

        let expected = match expected_sorted_crate_dir_names() {
            Ok(n) => n,
            Err(e) => return (false, e),
        };
        let file_ok = path.is_file();
        let got = if file_ok {
            std::fs::read_to_string(path).unwrap_or_default()
        } else {
            String::new()
        };
        let got_lines: Vec<String> = got
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(std::string::ToString::to_string)
            .collect();

        let content_ok = file_ok && got_lines == expected;

        let tool_calls = session
            .messages
            .iter()
            .filter(|m| matches!(m.role, harness_core::message::Role::Assistant))
            .flat_map(|m| match &m.content {
                harness_core::message::MessageContent::Blocks(blocks) => blocks.iter().collect::<Vec<_>>(),
                _ => vec![],
            })
            .filter(|b| matches!(b, harness_core::message::ContentBlock::ToolUse { .. }))
            .count();
        let tools_ok = tool_calls >= 2;

        let detail = if content_ok && tools_ok {
            format!(
                "Exact match: {} crate dirs, {} tool calls",
                expected.len(),
                tool_calls
            )
        } else if content_ok && !tools_ok {
            format!(
                "Content correct but expected at least 2 tool calls, got {}",
                tool_calls
            )
        } else if !file_ok {
            format!(
                "Missing or not a file: {}; assistant last text hint: {} chars",
                path.display(),
                session
                    .messages
                    .last()
                    .and_then(|m| m.text())
                    .map(str::len)
                    .unwrap_or(0)
            )
        } else {
            format!(
                "Lines mismatch: want {} lines (first: {:?}), got {} lines",
                expected.len(),
                expected.first(),
                got_lines.len()
            )
        };

        (content_ok && tools_ok, detail)
    },
};

/// Default suite (same as historically `ALL_TASKS`).
pub static DEFAULT_TASKS: &[BenchmarkTask] = &[TOOL_CALL_BASIC, SUMMARIZE_TEXT, MULTI_TOOL_TASK];

/// Backwards-compatible name for [`DEFAULT_TASKS`].
#[allow(dead_code)]
pub static ALL_TASKS: &[BenchmarkTask] = DEFAULT_TASKS;

pub static HARD_TASKS: &[BenchmarkTask] = &[
    TOOL_CALL_BASIC,
    SUMMARIZE_TEXT,
    MULTI_TOOL_TASK,
    CRATE_DIRS_MANIFEST,
];

pub fn tasks_for_tier(tier: BenchTier) -> &'static [BenchmarkTask] {
    match tier {
        BenchTier::Default => DEFAULT_TASKS,
        BenchTier::Hard => HARD_TASKS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_task_counts() {
        assert_eq!(tasks_for_tier(BenchTier::Default).len(), 3);
        assert_eq!(tasks_for_tier(BenchTier::Hard).len(), 4);
    }
}
