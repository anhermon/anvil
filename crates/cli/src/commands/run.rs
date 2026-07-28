use std::sync::Arc;
use std::time::Instant;

use clap::Args;
use harness_core::config::Config;
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

    /// Override the maximum number of agent iterations (default: 10).
    /// Set to 0 for unlimited.
    #[arg(long, default_value_t = 10)]
    pub max_iterations: usize,

    /// Emit structured NDJSON events to stdout instead of human-readable terminal output.
    ///
    /// Each line is a JSON object with a `type` field:
    ///   {"type":"text",     "part":{"text":"..."}}
    ///   {"`type":"tool_use`", "part":{"tool":"bash","callID":"...","state":{"status":"completed","input":{...},"output":"..."}}}
    ///   {"type":"result",   "part":{"text":"...","isError":false,"sessionId":"..."}}
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

    fn on_result(&self, text: &str, is_error: bool, session_id: &str) {
        Self::emit(&serde_json::json!({
            "type": "result",
            "part": {
                "text": text,
                "isError": is_error,
                "sessionId": session_id,
                "model": self.model,
            }
        }));
    }
}

// ── Command entry point ───────────────────────────────────────────────────────

// Long but linear: a single top-to-bottom flow; splitting it would only scatter state.
#[allow(clippy::too_many_lines)]
pub async fn execute(args: RunArgs) -> anyhow::Result<()> {
    let config = Config::load()?;
    let resolved = provider::resolve(&config, &args.provider)?;
    let backend = resolved.backend;
    let model = resolved.model;
    let provider = resolved.provider;

    let memory = Arc::new(MemoryDb::open(&config.memory.db_path).await?);

    if args.stream {
        // Streaming mode: run through the Agent loop (with tools) using CliHook.
        let hook = CliHook::new();
        let agent = Agent::new(Arc::clone(&provider), Arc::clone(&memory), config.clone())
            .with_hook(Arc::clone(&hook) as Arc<dyn UiHook>);

        ui::print_banner();
        ui::print_session_header("stream", &model, &backend);

        let opts = RunOptions {
            session_name: args.session.clone(),
            max_iterations: if args.max_iterations == 0 {
                Some(usize::MAX)
            } else {
                Some(args.max_iterations)
            },
        };

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
    } else if args.json_output {
        // JSON output mode: emit NDJSON events to stdout, no terminal UI.
        let hook = JsonHook::new(&model);
        let agent = Agent::new(Arc::clone(&provider), Arc::clone(&memory), config.clone())
            .with_hook(Arc::clone(&hook) as Arc<dyn UiHook>);

        let opts = RunOptions {
            session_name: args.session.clone(),
            max_iterations: if args.max_iterations == 0 {
                Some(usize::MAX)
            } else {
                Some(args.max_iterations)
            },
        };

        agent.run_with_options(&args.goal, opts).await?;
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

        let opts = RunOptions {
            session_name: args.session.clone(),
            max_iterations: if args.max_iterations == 0 {
                Some(usize::MAX)
            } else {
                Some(args.max_iterations)
            },
        };

        let t0 = Instant::now();
        let session = agent.run_with_options(&args.goal, opts).await?;
        let elapsed_ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);

        if let Some(msg) = session.messages.last() {
            ui::print_response(msg.text().unwrap_or("(no response)"));
        }

        ui::print_session_summary(0, 0, session.iteration, elapsed_ms);
        eprintln!("  session {} | status {:?}", session.id, session.status);
    }

    Ok(())
}
