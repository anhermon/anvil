use std::io::Write as IoWrite;
use std::sync::Arc;
use std::time::Instant;

use clap::Args;
use futures::StreamExt;
use harness_core::{
    config::Config,
    provider::Provider,
    providers::{ClaudeCodeProvider, ClaudeProvider},
};
use harness_memory::MemoryDb;
use indicatif::ProgressBar;

use crate::agent::{Agent, RunOptions, UiHook};
use crate::ui;

#[derive(Args)]
pub struct RunArgs {
    /// Goal for this agent run
    #[arg(short, long)]
    pub goal: String,

    /// Provider backend override (claude, claude-code, cc, echo)
    #[arg(long, env = "HARNESS_PROVIDER")]
    pub provider: Option<String>,

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
}

/// CLI UI hook: drives the spinner and prints tool call/result lines.
struct CliHook {
    spinner: std::sync::Mutex<Option<ProgressBar>>,
}

impl CliHook {
    fn new() -> Arc<Self> {
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
        let mut guard = self.spinner.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(pb);
    }

    fn on_thinking_done(&self) {
        let mut guard = self.spinner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pb) = guard.take() {
            pb.finish_and_clear();
        }
    }

    fn on_tool_call(&self, name: &str, input_preview: &str) {
        // Pause spinner output so tool lines print cleanly.
        {
            let guard = self.spinner.lock().unwrap_or_else(|e| e.into_inner());
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
            let guard = self.spinner.lock().unwrap_or_else(|e| e.into_inner());
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

pub async fn execute(args: RunArgs) -> anyhow::Result<()> {
    let config = Config::load()?;

    let backend = args
        .provider
        .as_deref()
        .unwrap_or(&config.provider.backend)
        .to_string();
    let provider: Arc<dyn Provider> = match backend.as_str() {
        "echo" => {
            tracing::info!("using echo provider (no LLM calls)");
            Arc::new(harness_core::provider::EchoProvider::new())
        }
        "claude-code" | "cc" => {
            let model = &config.provider.model;
            tracing::info!(model = %model, "using ClaudeCodeProvider (subprocess)");
            Arc::new(ClaudeCodeProvider::new(model))
        }
        _ => Arc::new(
            ClaudeProvider::from_env(&config.provider.model, config.provider.max_tokens)
                .map_err(|e| anyhow::anyhow!("{}", e))?,
        ),
    };

    let memory = Arc::new(MemoryDb::open(&config.memory.db_path).await?);

    if args.stream {
        // Streaming mode: print tokens as they arrive, then persist to memory.
        let msgs =
            vec![
                harness_core::message::Message::system(
                    config.agent.system_prompt.as_deref().unwrap_or(
                        "You are a helpful assistant. Complete the user's goal concisely.",
                    ),
                ),
                harness_core::message::Message::user(&args.goal),
            ];

        ui::print_banner();
        ui::print_session_header("stream", &config.provider.model, &backend);

        println!("\n{}", "-".repeat(60));
        let mut token_stream = provider.stream(&msgs).await?;
        let mut full_text = String::new();
        let stdout = std::io::stdout();
        let mut out = stdout.lock();

        while let Some(chunk) = token_stream.next().await {
            let chunk = chunk?;
            if !chunk.delta.is_empty() {
                full_text.push_str(&chunk.delta);
                write!(out, "{}", chunk.delta)?;
                out.flush()?;
            }
            if chunk.done {
                break;
            }
        }

        writeln!(out)?;
        println!("{}", "-".repeat(60));

        // Persist the streamed response to memory.
        let ep = harness_memory::Episode::turn(uuid::Uuid::new_v4(), "assistant", &full_text);
        let sn = args.session.as_deref();
        memory.insert_named(&ep, sn).await?;

        println!("Streaming complete.");
    } else {
        let hook = CliHook::new();
        let agent = Agent::new(Arc::clone(&provider), Arc::clone(&memory), config.clone())
            .with_hook(Arc::clone(&hook) as Arc<dyn UiHook>);

        ui::print_banner();
        ui::print_session_header("pending", &config.provider.model, &backend);

        // Inform user about active session name.
        if let Some(ref sname) = args.session {
            eprintln!("  session name: {}\n", sname);
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
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        if let Some(msg) = session.messages.last() {
            ui::print_response(msg.text().unwrap_or("(no response)"));
        }

        ui::print_session_summary(0, 0, session.iteration, elapsed_ms);
        eprintln!(
            "  session {} | status {:?}",
            &session.id.to_string()[..8],
            session.status,
        );
    }

    Ok(())
}
