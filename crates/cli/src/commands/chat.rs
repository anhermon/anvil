use std::sync::Arc;
use std::time::Instant;

use clap::Args;
use harness_core::{
    config::Config,
    provider::Provider,
    providers::{ClaudeCodeProvider, ClaudeProvider, OpenAIProvider},
};
use harness_memory::MemoryDb;
use indicatif::ProgressBar;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::agent::{Agent, RunOptions, UiHook};
use crate::ui;

#[derive(Args)]
pub struct ChatArgs {
    /// Provider backend override (claude, openai, custom, claude-code, cc, echo, ollama)
    #[arg(long, env = "HARNESS_PROVIDER")]
    pub provider: Option<String>,

    /// Model identifier override (e.g. "gpt-4o")
    #[arg(long)]
    pub model: Option<String>,

    /// Base URL override (useful for Ollama, vLLM, LM Studio)
    #[arg(long)]
    pub base_url: Option<String>,

    /// Named session for continuity. Load prior history from this session.
    #[arg(long, default_value = "chat")]
    pub session: String,

    /// Override the maximum number of agent iterations (default: 10).
    #[arg(long, default_value_t = 10)]
    pub max_iterations: usize,
}

struct ChatHook {
    spinner: std::sync::Mutex<Option<ProgressBar>>,
}

impl ChatHook {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            spinner: std::sync::Mutex::new(None),
        })
    }
}

impl UiHook for ChatHook {
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

    fn on_thought(&self, thought: &str) {
        {
            let guard = self.spinner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pb) = guard.as_ref() {
                pb.suspend(|| {
                    ui::print_thought(thought);
                });
                return;
            }
        }
        ui::print_thought(thought);
    }

    fn on_tool_call(&self, name: &str, input_preview: &str) {
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

    fn on_assistant_message(&self, message: &harness_core::message::Message) {
        if let Some(text) = message.text() {
            if !text.trim().is_empty() {
                ui::print_response(text);
            }
        }
    }
}

pub async fn execute(args: ChatArgs) -> anyhow::Result<()> {
    let config = Config::load()?;

    let backend = args
        .provider
        .as_deref()
        .unwrap_or(&config.provider.backend)
        .to_string();
    let model = args
        .model
        .as_deref()
        .unwrap_or(&config.provider.model)
        .to_string();
    let base_url = args.base_url.clone().or(config.provider.base_url.clone());

    let provider: Arc<dyn Provider> = match backend.as_str() {
        "echo" => Arc::new(harness_core::provider::EchoProvider),
        "claude-code" | "cc" => Arc::new(ClaudeCodeProvider::new(&model)),
        "openai" | "custom" | "ollama" => {
            let api_key = config.resolved_api_key().unwrap_or_default();
            Arc::new(OpenAIProvider::new(
                api_key,
                &model,
                config.provider.max_tokens,
                base_url,
                config.provider.tool_format.clone(),
            ))
        }
        _ => Arc::new(
            ClaudeProvider::from_env(&model, config.provider.max_tokens)
                .map_err(|e| anyhow::anyhow!("{}", e))?,
        ),
    };

    let memory = Arc::new(MemoryDb::open(&config.memory.db_path).await?);
    let hook = ChatHook::new();
    let agent = Agent::new(Arc::clone(&provider), Arc::clone(&memory), config.clone())
        .with_hook(Arc::clone(&hook) as Arc<dyn UiHook>);

    ui::print_banner();
    ui::print_session_header(&args.session, &model, &backend);
    println!("  Type your message and press Enter. /help for commands. Ctrl+D or /exit to quit.\n");

    let mut rl = DefaultEditor::new()?;
    let prompt = format!("{} ❯ ", console::style(&model).cyan());

    loop {
        let readline = rl.readline(&prompt);
        match readline {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                if line == "/exit" || line == "/quit" {
                    break;
                }

                if line == "/help" {
                    println!("\n  Commands:");
                    println!("    /help  - Show this help message");
                    println!("    /exit  - Quit the chat session");
                    println!("    /clear - (TODO) Clear current session history\n");
                    continue;
                }

                let _ = rl.add_history_entry(line);

                let opts = RunOptions {
                    session_name: Some(args.session.clone()),
                    max_iterations: if args.max_iterations == 0 {
                        Some(usize::MAX)
                    } else {
                        Some(args.max_iterations)
                    },
                };

                let t0 = Instant::now();
                match agent.run_with_options(line, opts).await {
                    Ok(session) => {
                        let elapsed_ms = t0.elapsed().as_millis() as u64;
                        ui::print_session_summary(0, 0, session.iteration, elapsed_ms);
                        println!();
                    }
                    Err(e) => {
                        eprintln!("  {} Error: {}", console::style("✗").red(), e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }

    Ok(())
}
