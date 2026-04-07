use harness_core::session::Session;

pub struct BenchmarkTask {
    pub name: &'static str,
    pub goal: &'static str,
    #[allow(dead_code)]
    pub expected_tool_calls_min: usize,
    pub evaluate: fn(&Session) -> (bool, String),
}

impl BenchmarkTask {
    pub fn evaluate(&self, session: &Session) -> (bool, String) {
        (self.evaluate)(session)
    }
}

pub static ALL_TASKS: &[BenchmarkTask] = &[TOOL_CALL_BASIC, SUMMARIZE_TEXT, MULTI_TOOL_TASK];

pub const TOOL_CALL_BASIC: BenchmarkTask = BenchmarkTask {
    name: "tool_call_basic",
    goal: "List the files in the current directory. Just output the filenames, one per line.",
    expected_tool_calls_min: 1,
    evaluate: |session: &Session| {
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
    evaluate: |session: &Session| {
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
    goal: "Search for all Rust files that contain the word 'Provider' trait. Write the list of matching file paths to a file called /tmp/bench_provider_files.txt",
    expected_tool_calls_min: 2,
    evaluate: |session: &Session| {
        let final_text = session
            .messages
            .last()
            .and_then(|m| m.text())
            .unwrap_or("");

        let file_written = std::path::Path::new("/tmp/bench_provider_files.txt").exists();

        let mentions_result =
            final_text.to_lowercase().contains("provider") || final_text.to_lowercase().contains("file");

        let met = file_written && mentions_result;
        let detail = if met {
            let content = std::fs::read_to_string("/tmp/bench_provider_files.txt").unwrap_or_default();
            let line_count = content.lines().count();
            format!("File written with {} entries", line_count)
        } else {
            format!(
                "Task incomplete: file_written={}, mentions_result={}",
                file_written, mentions_result
            )
        };
        (met, detail)
    },
};
