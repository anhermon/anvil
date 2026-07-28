use std::io::{self, BufRead, Write};
use std::sync::Arc;

use clap::Args;
use harness_core::config::Config;
use harness_memory::MemoryDb;
use uuid::Uuid;

use crate::agent::{Agent, RunOptions, UiHook};
use crate::commands::{provider, run::CliHook};
use crate::ui;

#[derive(Args)]
pub struct ChatArgs {
    #[command(flatten)]
    provider: provider::ProviderArgs,

    /// Named session to resume. A new chat session name is generated when omitted
    #[arg(long)]
    pub session: Option<String>,

    /// Maximum agent iterations for each prompt (default: 10). Set to 0 for unlimited
    #[arg(long, default_value_t = 10)]
    pub max_iterations: usize,
}

pub async fn execute(args: ChatArgs) -> anyhow::Result<()> {
    let config = Config::load()?;
    let resolved = provider::resolve(&config, &args.provider)?;
    let memory = Arc::new(MemoryDb::open(&config.memory.db_path).await?);
    let session_name = args
        .session
        .unwrap_or_else(|| format!("chat-{}", Uuid::new_v4()));
    let hook = CliHook::new();
    let agent = Agent::new(Arc::clone(&resolved.provider), Arc::clone(&memory), config)
        .with_hook(Arc::clone(&hook) as Arc<dyn UiHook>);

    ui::print_banner();
    ui::print_session_header(&session_name, &resolved.model, &resolved.backend);
    eprintln!("  Interactive chat session: {session_name}");
    eprintln!("  Type /exit or press Ctrl-D to leave.\n");

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();
    run_chat_loop(
        &agent,
        &session_name,
        args.max_iterations,
        &mut reader,
        &mut writer,
    )
    .await
}

async fn run_chat_loop<R: BufRead, W: Write>(
    agent: &Agent,
    session_name: &str,
    max_iterations: usize,
    reader: &mut R,
    writer: &mut W,
) -> anyhow::Result<()> {
    let max_iterations = if max_iterations == 0 {
        usize::MAX
    } else {
        max_iterations
    };
    let options = RunOptions {
        session_name: Some(session_name.to_string()),
        max_iterations: Some(max_iterations),
    };

    loop {
        write!(writer, "you> ")?;
        writer.flush()?;

        let mut input = String::new();
        if reader.read_line(&mut input)? == 0 {
            writeln!(writer, "\nGoodbye.")?;
            break;
        }

        let prompt = input.trim();
        if prompt == "/exit" {
            writeln!(writer, "Goodbye.")?;
            break;
        }
        if prompt.is_empty() {
            continue;
        }

        let session = agent.run_with_options(prompt, options.clone()).await?;
        let response = session
            .messages
            .last()
            .and_then(|message| message.text())
            .unwrap_or("(no response)");
        writeln!(writer, "anvil> {response}")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use harness_core::provider::EchoProvider;

    use super::*;

    async fn test_agent() -> (Agent, Arc<MemoryDb>) {
        let memory = Arc::new(MemoryDb::in_memory().await.unwrap());
        let agent = Agent::new(
            Arc::new(EchoProvider),
            Arc::clone(&memory),
            Config::default(),
        );
        (agent, memory)
    }

    #[tokio::test]
    async fn processes_multiple_prompts_in_one_named_session() {
        let (agent, memory) = test_agent().await;
        let mut input = Cursor::new(b"first prompt\nsecond prompt\n/exit\n");
        let mut output = Vec::new();

        run_chat_loop(&agent, "test-chat", 10, &mut input, &mut output)
            .await
            .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("anvil> echo: first prompt"));
        assert!(output.contains("anvil> echo: second prompt"));
        assert!(output.ends_with("Goodbye.\n"));

        let history = memory.recent_by_name("test-chat", 10).await.unwrap();
        assert_eq!(history.len(), 4);
    }

    #[tokio::test]
    async fn eof_exits_cleanly_without_running_the_agent() {
        let (agent, memory) = test_agent().await;
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();

        run_chat_loop(&agent, "empty-chat", 10, &mut input, &mut output)
            .await
            .unwrap();

        assert_eq!(String::from_utf8(output).unwrap(), "you> \nGoodbye.\n");
        assert!(memory
            .recent_by_name("empty-chat", 10)
            .await
            .unwrap()
            .is_empty());
    }
}
