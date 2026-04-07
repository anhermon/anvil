use std::path::PathBuf;

use harness_core::session::Session;

/// Per–outer-iteration paths for graders (isolated temp dir; see `runner::run_iteration`).
#[derive(Clone, Default)]
pub struct TaskEvalContext {
    /// Output file for `multi_tool_task`; set by the benchmark runner.
    pub multi_tool_output: Option<PathBuf>,
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
        } else {
            self.goal.to_string()
        }
    }

    pub fn evaluate(&self, session: &Session, ctx: &TaskEvalContext) -> (bool, String) {
        (self.evaluate)(session, ctx)
    }
}

pub static ALL_TASKS: &[BenchmarkTask] = &[TOOL_CALL_BASIC, SUMMARIZE_TEXT, MULTI_TOOL_TASK];

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
