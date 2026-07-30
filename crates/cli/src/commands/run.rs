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

    /// Shell command that must exit 0 for the run to report success.
    ///
    /// Run in the working directory after the agent ends its turn believing it is
    /// done. On a non-zero exit the run reports `verification_failed` (exit code 3)
    /// instead of `done`, and the command's exit code and output are printed to
    /// stderr and attached to the `--json-output` result event.
    ///
    /// This is *your* shell on *your* machine, not something the model chose, so it
    /// is executed directly via `sh -c` and is deliberately **not** subject to the
    /// bash tool's command allowlist (which exists to constrain model-generated
    /// commands).
    ///
    /// Example: anvil run --goal "fix the failing test" --verify "cargo test"
    #[arg(long, value_name = "COMMAND")]
    pub verify: Option<String>,

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
    /// Final `(text, session_id)` captured from the agent loop. The terminal
    /// `result` event is held back until [`JsonHook::finish`] so `--verify` can
    /// still change the outcome — a single result event, emitted once, with the
    /// gated status.
    result: std::sync::Mutex<Option<(String, String)>>,
}

impl JsonHook {
    fn new(model: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            model: model.into(),
            pending: std::sync::Mutex::new(std::collections::HashMap::new()),
            result: std::sync::Mutex::new(None),
        })
    }

    /// Emit the terminal `result` event with the final (post-verification) status.
    fn finish(&self, status: &SessionStatus, verification: Option<&Verification>) {
        let Some((text, session_id)) = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return; // the loop errored out before completing; nothing to report
        };
        let mut part = serde_json::json!({
            "text": text,
            "isError": *status != SessionStatus::Done,
            "outcome": status.as_str(),
            "sessionId": session_id,
            "model": self.model,
        });
        if let (Some(v), Some(obj)) = (verification, part.as_object_mut()) {
            obj.insert(
                "verification".into(),
                serde_json::json!({ "exitCode": v.code, "output": v.output }),
            );
        }
        Self::emit(&serde_json::json!({ "type": "result", "part": part }));
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
        // Held back until `finish`; `status` here is the agent loop's view, which
        // `--verify` may still override.
        let _ = status;
        *self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((text.to_string(), session_id.to_string()));
    }
}

// ── Verification gate ─────────────────────────────────────────────────────────

/// Outcome of the operator-supplied `--verify` command.
struct Verification {
    code: i32,
    /// Combined stdout+stderr, tail-truncated — the useful part of a test run's
    /// output is at the end.
    output: String,
}

/// How much verification output to keep and show.
const VERIFY_OUTPUT_TAIL: usize = 4096;

/// Run the verification command and gate `status` on it.
///
/// Only a run the agent itself considered finished is gated: `MaxIterations` and
/// friends already carry a more specific reason and are passed through untouched.
///
/// The command is operator-supplied — it comes from this machine's own command
/// line, not from the model — so it is executed directly via `sh -c` and is not
/// filtered through the bash tool allowlist, which exists to constrain
/// model-generated commands. It inherits the process working directory, the same
/// one the bash tool runs the agent's own commands in.
fn gate_on_verification(
    status: SessionStatus,
    verify: Option<&str>,
) -> anyhow::Result<(SessionStatus, Option<Verification>)> {
    let (Some(cmd), SessionStatus::Done) = (verify, &status) else {
        return Ok((status, None));
    };

    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()?;
    let code = out.status.code().unwrap_or(-1);
    let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
    output.push_str(&String::from_utf8_lossy(&out.stderr));
    if output.len() > VERIFY_OUTPUT_TAIL {
        let start = output.len() - VERIFY_OUTPUT_TAIL;
        let start = (start..output.len())
            .find(|i| output.is_char_boundary(*i))
            .unwrap_or(output.len());
        output = format!("[…truncated…]\n{}", &output[start..]);
    }

    if code == 0 {
        eprintln!("verification passed: `{cmd}` exited 0");
        return Ok((status, Some(Verification { code, output })));
    }

    eprintln!(
        "error: the agent reported it was finished, but verification failed.\n\
         command: {cmd}\nexit code: {code}\noutcome: {} (exit code {})\n--- verification output ---\n{}",
        SessionStatus::VerificationFailed.as_str(),
        SessionStatus::VerificationFailed.exit_code(),
        output.trim_end(),
    );
    Ok((
        SessionStatus::VerificationFailed,
        Some(Verification { code, output }),
    ))
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

    // Bound outside the branch: the JSON result event is emitted only after the
    // verification gate has had its say.
    let mut json_hook: Option<Arc<JsonHook>> = None;

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
        json_hook = Some(hook);

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

    let (status, verification) = gate_on_verification(status, args.verify.as_deref())?;

    if let Some(hook) = json_hook {
        hook.finish(&status, verification.as_ref());
    }

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::{gate_on_verification, resolve_max_iterations, SessionStatus, VERIFY_OUTPUT_TAIL};

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
        // A finished-but-unverified run is distinguishable from one that never finished.
        assert_eq!(SessionStatus::VerificationFailed.exit_code(), 3);
        assert_eq!(
            SessionStatus::VerificationFailed.as_str(),
            "verification_failed"
        );
    }

    /// The case this feature exists for: the agent ends its turn claiming success,
    /// the operator's ground truth disagrees, and the run must not report `done`.
    #[test]
    fn a_claimed_success_that_fails_verification_is_not_done() {
        let (status, v) = gate_on_verification(
            SessionStatus::Done,
            Some("echo 'test result: FAILED'; exit 7"),
        )
        .unwrap();

        assert_eq!(status, SessionStatus::VerificationFailed);
        assert_eq!(status.as_str(), "verification_failed");
        assert_ne!(status.exit_code(), 0);

        let v = v.expect("verification result is reported");
        assert_eq!(v.code, 7);
        assert!(v.output.contains("FAILED"), "output was {:?}", v.output);
    }

    #[test]
    fn a_verified_success_stays_done_and_no_verify_is_a_no_op() {
        let (status, v) = gate_on_verification(SessionStatus::Done, Some("exit 0")).unwrap();
        assert_eq!(status, SessionStatus::Done);
        assert_eq!(v.expect("reported").code, 0);

        let (status, v) = gate_on_verification(SessionStatus::Done, None).unwrap();
        assert_eq!(status, SessionStatus::Done);
        assert!(v.is_none());
    }

    /// A run that never finished keeps its own, more specific outcome — we do not
    /// bury `max_iterations` under a verification failure.
    #[test]
    fn an_unfinished_run_is_not_gated() {
        let (status, v) =
            gate_on_verification(SessionStatus::MaxIterations, Some("exit 1")).unwrap();
        assert_eq!(status, SessionStatus::MaxIterations);
        assert!(v.is_none());
    }

    #[test]
    fn long_output_keeps_the_tail() {
        let (_, v) = gate_on_verification(
            SessionStatus::Done,
            Some("head -c 20000 /dev/zero | tr '\\0' 'x'; echo THE-SUMMARY; exit 1"),
        )
        .unwrap();
        let out = v.expect("reported").output;
        assert!(out.contains("THE-SUMMARY"));
        assert!(out.len() < VERIFY_OUTPUT_TAIL + 64, "len {}", out.len());
    }

    /// `--verify` runs in the process working directory — the same one the bash
    /// tool gives the agent, so the agent's edits are what gets checked.
    #[test]
    fn verification_runs_in_the_working_directory() {
        let (_, v) = gate_on_verification(SessionStatus::Done, Some("pwd")).unwrap();
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            v.expect("reported").output.trim(),
            cwd.to_string_lossy().trim()
        );
    }
}
