use std::sync::Arc;
use std::time::Instant;

use clap::Args;
use harness_core::config::Config;
use harness_core::session::SessionStatus;
use harness_memory::MemoryDb;
use indicatif::ProgressBar;

use crate::agent::{Agent, RunOptions, UiHook};
use crate::commands::provider;
use crate::ui;

#[derive(Args)]
pub struct RunArgs {
    /// Goal for this agent run
    #[arg(short, long)]
    pub goal: String,

    #[command(flatten)]
    provider: provider::ProviderArgs,

    /// Stream response tokens to stdout as they arrive
    #[arg(long)]
    pub stream: bool,

    /// Named session for continuity. Load prior history from this session
    /// and save new episodes under this name.
    /// Example: anvil run --goal "continue the work" --session myproject
    #[arg(long)]
    pub session: Option<String>,

    /// Override the maximum number of agent iterations.
    /// Defaults to `agent.max_iterations` from the config (50).
    /// Set to 0 for unlimited.
    #[arg(long)]
    pub max_iterations: Option<usize>,

    /// Emit structured NDJSON events to stdout instead of human-readable terminal output.
    ///
    /// Each line is a JSON object with a `type` field:
    ///   {"type":"text",     "part":{"text":"..."}}
    ///   {"`type":"tool_use`", "part":{"tool":"bash","callID":"...","state":{"status":"completed","input":{...},"output":"..."}}}
    ///   {"type":"result",   "part":{"text":"...","isError":false,"outcome":"done","sessionId":"..."}}
    ///
    /// `outcome` is the machine-readable run status: `done` when the agent ended its
    /// own turn, `max_iterations` when the loop hit `--max-iterations` first. `isError`
    /// is true for every outcome other than `done`, and the process then exits 2.
    ///
    /// Use this flag when calling `anvil run` from a machine-readable context (e.g. Paperclip adapter).
    #[arg(long)]
    pub json_output: bool,
}

// ── Terminal (coloured) hook ──────────────────────────────────────────────────

/// CLI UI hook: drives the spinner and prints tool call/result lines.
pub(crate) struct CliHook {
    spinner: std::sync::Mutex<Option<ProgressBar>>,
}

impl CliHook {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            spinner: std::sync::Mutex::new(None),
        })
    }
}

impl UiHook for CliHook {
    fn on_thinking(&self, iteration: usize, max_iter: usize) {
        let label = if max_iter == usize::MAX {
            format!("thinking... [{iteration}]")
        } else {
            format!("thinking... [{iteration}/{max_iter}]")
        };
        let pb = ui::thinking_spinner(&label);
        let mut guard = self
            .spinner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some(pb);
    }

    fn on_thinking_done(&self) {
        let mut guard = self
            .spinner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(pb) = guard.take() {
            pb.finish_and_clear();
        }
    }

    fn on_tool_call(&self, name: &str, input_preview: &str) {
        // Pause spinner output so tool lines print cleanly.
        {
            let guard = self
                .spinner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(pb) = guard.as_ref() {
                pb.suspend(|| {
                    ui::print_tool_call(name, input_preview);
                });
                return;
            }
        }
        ui::print_tool_call(name, input_preview);
    }

    fn on_tool_result(&self, output: &str) {
        {
            let guard = self
                .spinner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(pb) = guard.as_ref() {
                pb.suspend(|| {
                    ui::print_tool_result(output);
                });
                return;
            }
        }
        ui::print_tool_result(output);
    }
}

// ── Structured JSON hook ──────────────────────────────────────────────────────

/// JSON output hook: emits NDJSON events to stdout for machine-readable consumers
/// (e.g. the Paperclip `anvil-local` adapter UI parser).
///
/// Each event is a JSON object on a single line followed by `\n`. The format
/// mirrors the `OpenCode` session event stream so existing UI parsers can reuse logic.
struct JsonHook {
    /// Model identifier (e.g. "claude-sonnet-4-6") included in the result event.
    model: String,
    /// Pending tool call names keyed by callID — used to re-attach the name when
    /// emitting the combined `tool_use` event with both input and output.
    pending: std::sync::Mutex<std::collections::HashMap<String, (String, serde_json::Value)>>,
}

impl JsonHook {
    fn new(model: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            model: model.into(),
            pending: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    fn emit(obj: &serde_json::Value) {
        if let Ok(s) = serde_json::to_string(obj) {
            println!("{s}");
        }
    }
}

impl UiHook for JsonHook {
    // Terminal-mode methods — no-op in JSON mode.
    fn on_thinking(&self, _iteration: usize, _max_iter: usize) {}
    fn on_thinking_done(&self) {}
    fn on_tool_call(&self, _name: &str, _input_preview: &str) {}
    fn on_tool_result(&self, _output: &str) {}

    fn on_tool_call_full(&self, name: &str, tool_use_id: &str, input: &serde_json::Value) {
        // Store the pending call; we emit the full event once we have the output too.
        let mut guard = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.insert(tool_use_id.to_string(), (name.to_string(), input.clone()));
    }

    fn on_tool_result_full(&self, tool_use_id: &str, output: &str, is_error: bool) {
        let entry = {
            let mut guard = self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.remove(tool_use_id)
        };

        let (tool_name, input) = entry.unwrap_or_else(|| {
            (
                "unknown".to_string(),
                serde_json::Value::Object(serde_json::Map::default()),
            )
        });
        let status = if is_error { "error" } else { "completed" };

        Self::emit(&serde_json::json!({
            "type": "tool_use",
            "part": {
                "tool": tool_name,
                "callID": tool_use_id,
                "state": {
                    "status": status,
                    "input": input,
                    "output": output,
                }
            }
        }));
    }

    fn on_text(&self, text: &str) {
        Self::emit(&serde_json::json!({
            "type": "text",
            "part": { "text": text }
        }));
    }

    fn on_turn_diag(&self, diag: &crate::agent::TurnDiag) {
        Self::emit(&serde_json::json!({
            "type": "turn",
            "part": {
                "iteration": diag.iteration,
                "stopReason": diag.stop_reason,
                "textBlocks": diag.text_blocks,
                "toolBlocks": diag.tool_blocks,
                "empty": diag.empty,
                "recoveredTextCalls": diag.recovered_text_calls,
                "repeatedToolCall": diag.repeated_tool_call,
            }
        }));
    }

    fn on_result(&self, text: &str, session_id: &str, status: SessionStatus) {
        Self::emit(&serde_json::json!({
            "type": "result",
            "part": {
                "text": text,
                "isError": status != SessionStatus::Done,
                "outcome": status.as_str(),
                "sessionId": session_id,
                "model": self.model,
            }
        }));
    }
}

// ── Command entry point ───────────────────────────────────────────────────────

/// Resolve the iteration cap for this run: the `--max-iterations` flag when
/// given, otherwise `agent.max_iterations` from config. `0` means unlimited in
/// both places.
pub(crate) fn resolve_max_iterations(flag: Option<usize>, config_value: usize) -> usize {
    match flag.unwrap_or(config_value) {
        0 => usize::MAX,
        n => n,
    }
}

/// Run the agent and return the terminal session status.
///
/// The caller maps that status to a process exit code via
/// [`SessionStatus::exit_code`] — a run that never finished must not look like
/// a success to an unattended caller.
// Long but linear: a single top-to-bottom flow; splitting it would only scatter state.
#[allow(clippy::too_many_lines)]
pub async fn execute(args: RunArgs) -> anyhow::Result<SessionStatus> {
    let config = Config::load()?;
    let resolved = provider::resolve(&config, &args.provider)?;
    let backend = resolved.backend;
    let model = resolved.model;
    let provider = resolved.provider;

    let memory = Arc::new(MemoryDb::open(&config.memory.db_path).await?);

    let opts = RunOptions {
        session_name: args.session.clone(),
        max_iterations: Some(resolve_max_iterations(
            args.max_iterations,
            config.agent.max_iterations,
        )),
    };

    let status = if args.stream {
        // Streaming mode: run through the Agent loop (with tools) using CliHook.
        let hook = CliHook::new();
        let agent = Agent::new(Arc::clone(&provider), Arc::clone(&memory), config.clone())
            .with_hook(Arc::clone(&hook) as Arc<dyn UiHook>);

        ui::print_banner();
        ui::print_session_header("stream", &model, &backend);

        let session = agent.run_with_options(&args.goal, opts).await?;

        if let Some(msg) = session.messages.last() {
            if let Some(text) = msg.text() {
                if !text.is_empty() {
                    println!("\n{}", "-".repeat(60));
                    ui::print_response(text);
                }
            }
        }

        println!("Streaming complete.");
        session.status
    } else if args.json_output {
        // JSON output mode: emit NDJSON events to stdout, no terminal UI.
        let hook = JsonHook::new(&model);
        let agent = Agent::new(Arc::clone(&provider), Arc::clone(&memory), config.clone())
            .with_hook(Arc::clone(&hook) as Arc<dyn UiHook>);

        agent.run_with_options(&args.goal, opts).await?.status
    } else {
        // Default terminal UI mode.
        let hook = CliHook::new();
        let agent = Agent::new(Arc::clone(&provider), Arc::clone(&memory), config.clone())
            .with_hook(Arc::clone(&hook) as Arc<dyn UiHook>);

        ui::print_banner();
        ui::print_session_header("pending", &model, &backend);

        // Inform user about active session name.
        if let Some(ref sname) = args.session {
            eprintln!("  session name: {sname}\n");
        }

        let t0 = Instant::now();
        let session = agent.run_with_options(&args.goal, opts).await?;
        let elapsed_ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);

        if let Some(msg) = session.messages.last() {
            ui::print_response(msg.text().unwrap_or("(no response)"));
        }

        ui::print_session_summary(0, 0, session.iteration, elapsed_ms);
        eprintln!("  session {} | status {:?}", session.id, session.status);
        session.status
    };

    if status == SessionStatus::MaxIterations {
        eprintln!(
            "error: agent stopped at the --max-iterations cap without finishing its goal \
             (outcome: {}, exit code {})",
            status.as_str(),
            status.exit_code()
        );
    }

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::{resolve_max_iterations, SessionStatus};

    #[test]
    fn flag_overrides_config_and_zero_means_unlimited() {
        assert_eq!(resolve_max_iterations(Some(3), 50), 3);
        assert_eq!(resolve_max_iterations(None, 50), 50);
        assert_eq!(resolve_max_iterations(Some(0), 50), usize::MAX);
        assert_eq!(resolve_max_iterations(None, 0), usize::MAX);
    }

    #[test]
    fn only_a_finished_agent_turn_exits_zero() {
        assert_eq!(SessionStatus::Done.exit_code(), 0);
        assert_eq!(SessionStatus::MaxIterations.exit_code(), 2);
        assert_eq!(SessionStatus::Failed.exit_code(), 2);
        assert_eq!(SessionStatus::Cancelled.exit_code(), 2);
        assert_eq!(SessionStatus::MaxIterations.as_str(), "max_iterations");
    }
}
