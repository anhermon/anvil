use std::sync::Arc;

use clap::Args;
use harness_core::{config::Config, provider::Provider, providers::ClaudeProvider};
use harness_memory::MemoryDb;
use serde::Deserialize;

use crate::agent::Agent;

/// A single evaluation test case (one line in the JSONL file).
#[derive(Debug, Deserialize)]
struct EvalCase {
    /// The goal / prompt to send to the agent.
    goal: String,
    /// Expected substring that must appear in the agent's final response.
    expected: String,
    /// Optional human-readable label for this case.
    #[serde(default)]
    label: Option<String>,
}

#[derive(Args)]
pub struct EvalArgs {
    /// Path to a JSONL file where each line is {"goal":"...","expected":"..."}
    #[arg(short, long)]
    pub cases: String,

    /// Provider backend (claude, echo). Defaults to config value.
    #[arg(long, env = "HARNESS_PROVIDER")]
    pub provider: Option<String>,

    /// Fail fast: stop after the first failing case
    #[arg(long)]
    pub fail_fast: bool,
}

pub async fn execute(args: EvalArgs) -> anyhow::Result<()> {
    let config = Config::load()?;

    // Load and parse the JSONL cases file.
    let content = std::fs::read_to_string(&args.cases)
        .map_err(|e| anyhow::anyhow!("failed to read cases file '{}': {}", args.cases, e))?;

    let cases: Vec<EvalCase> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, line)| {
            serde_json::from_str::<EvalCase>(line)
                .map_err(|e| anyhow::anyhow!("line {}: invalid JSON: {}", i + 1, e))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    if cases.is_empty() {
        anyhow::bail!("no test cases found in '{}'", args.cases);
    }

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
        _ => {
            let api_key = config.resolved_api_key().ok_or_else(|| {
                anyhow::anyhow!("ANTHROPIC_API_KEY not set — pass via env or config")
            })?;
            Arc::new(ClaudeProvider::new(
                api_key,
                &config.provider.model,
                config.provider.max_tokens,
            ))
        }
    };

    let memory = Arc::new(MemoryDb::in_memory().await?);

    println!(
        "Running {} eval case(s) with provider '{}'",
        cases.len(),
        backend
    );
    println!("{}", "─".repeat(60));

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut failures: Vec<(usize, String, String, String)> = Vec::new(); // (idx, label, got, expected)

    for (idx, case) in cases.iter().enumerate() {
        let label = case
            .label
            .as_deref()
            .map(|l| l.to_string())
            .unwrap_or_else(|| format!("case {}", idx + 1));

        let agent = Agent::new(Arc::clone(&provider), Arc::clone(&memory), config.clone());
        let session = agent.run(&case.goal).await?;

        let response = session
            .messages
            .last()
            .and_then(|m| m.text())
            .unwrap_or("")
            .to_string();

        let pass = response
            .to_lowercase()
            .contains(&case.expected.to_lowercase());

        if pass {
            passed += 1;
            println!("[PASS] {}", label);
        } else {
            failed += 1;
            println!("[FAIL] {}", label);
            println!("       expected to contain: {:?}", case.expected);
            println!(
                "       got:                 {:?}",
                &response[..response.len().min(120)]
            );
            failures.push((idx + 1, label.clone(), response, case.expected.clone()));

            if args.fail_fast {
                println!("\nStopping early (--fail-fast).");
                break;
            }
        }
    }

    println!("{}", "─".repeat(60));
    println!("Results: {}/{} passed", passed, passed + failed);

    if failed > 0 {
        anyhow::bail!("{} case(s) failed", failed);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::EvalCase;

    #[test]
    fn parses_eval_case_json() {
        let line = r#"{"goal":"What is 2+2?","expected":"4"}"#;
        let case: EvalCase = serde_json::from_str(line).unwrap();
        assert_eq!(case.goal, "What is 2+2?");
        assert_eq!(case.expected, "4");
        assert!(case.label.is_none());
    }

    #[test]
    fn parses_eval_case_with_label() {
        let line = r#"{"goal":"hello","expected":"world","label":"greeting"}"#;
        let case: EvalCase = serde_json::from_str(line).unwrap();
        assert_eq!(case.label.as_deref(), Some("greeting"));
    }
}
