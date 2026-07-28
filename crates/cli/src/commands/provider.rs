use std::sync::Arc;
use std::time::Duration;

use clap::Args;
use harness_core::{
    config::Config,
    provider::Provider,
    providers::ollama::DEFAULT_OLLAMA_TIMEOUT_SECS,
    providers::{ClaudeCodeProvider, ClaudeProvider, OllamaProvider},
};

#[derive(Args, Debug, Clone)]
pub(crate) struct ProviderArgs {
    /// Provider backend override (claude, claude-code, cc, ollama, echo)
    #[arg(long, env = "HARNESS_PROVIDER")]
    pub(crate) provider: Option<String>,

    /// Model identifier override (e.g. "gemma4:e2b")
    #[arg(long, env = "HARNESS_MODEL")]
    pub(crate) model: Option<String>,

    /// Maximum seconds to wait for each Ollama request
    #[arg(
        long,
        env = "ANVIL_OLLAMA_TIMEOUT_SECS",
        default_value_t = DEFAULT_OLLAMA_TIMEOUT_SECS,
        value_parser = parse_positive_timeout
    )]
    pub(crate) ollama_timeout_secs: u64,
}

pub(crate) struct ResolvedProvider {
    pub(crate) backend: String,
    pub(crate) model: String,
    pub(crate) provider: Arc<dyn Provider>,
}

pub(crate) fn resolve(config: &Config, args: &ProviderArgs) -> anyhow::Result<ResolvedProvider> {
    let provider_override = args.provider.as_deref();
    let mut backend = provider_override
        .unwrap_or(&config.provider.backend)
        .to_string();
    let model = args
        .model
        .as_deref()
        .unwrap_or(&config.provider.model)
        .to_string();

    // Auto-detect ollama when no provider is explicitly specified and the model
    // doesn't look like a known Claude or OpenAI model.
    if provider_override.is_none()
        && !model.starts_with("claude")
        && !model.starts_with("anthropic/")
        && !model.starts_with("gpt-")
        && !model.starts_with("openai/")
    {
        tracing::info!(model = %model, "auto-detecting ollama provider from model name");
        backend = "ollama".to_string();
    }

    let provider: Arc<dyn Provider> = match backend.as_str() {
        "echo" => {
            tracing::info!("using echo provider (no LLM calls)");
            Arc::new(harness_core::provider::EchoProvider)
        }
        "claude-code" | "cc" => {
            tracing::info!(model = %model, "using ClaudeCodeProvider (subprocess)");
            Arc::new(ClaudeCodeProvider::new(&model))
        }
        "ollama" => build_ollama_provider(config, &model, args.ollama_timeout_secs),
        _ => Arc::new(
            ClaudeProvider::from_env(&model, config.provider.max_tokens)
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        ),
    };

    Ok(ResolvedProvider {
        backend,
        model,
        provider,
    })
}

fn build_ollama_provider(config: &Config, model: &str, timeout_secs: u64) -> Arc<dyn Provider> {
    let base_url = config
        .provider
        .base_url
        .as_deref()
        .unwrap_or("http://localhost:11434");
    tracing::info!(model, base_url, timeout_secs, "using OllamaProvider");
    Arc::new(OllamaProvider::with_timeout(
        base_url,
        model,
        config.provider.max_tokens,
        Duration::from_secs(timeout_secs),
    ))
}

fn parse_positive_timeout(value: &str) -> Result<u64, String> {
    match value.parse::<u64>() {
        Ok(seconds) if seconds > 0 => Ok(seconds),
        _ => Err("timeout must be a positive number of seconds".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_args(provider: Option<&str>, model: Option<&str>) -> ProviderArgs {
        ProviderArgs {
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
            ollama_timeout_secs: DEFAULT_OLLAMA_TIMEOUT_SECS,
        }
    }

    #[test]
    fn echo_provider_can_be_selected_without_credentials() {
        let config = Config::default();
        let resolved = resolve(&config, &provider_args(Some("echo"), Some("test-model"))).unwrap();

        assert_eq!(resolved.backend, "echo");
        assert_eq!(resolved.model, "test-model");
        assert_eq!(resolved.provider.name(), "echo");
    }

    #[test]
    fn local_model_auto_selects_ollama_without_provider_override() {
        let config = Config::default();
        let resolved = resolve(&config, &provider_args(None, Some("qwen2.5:3b"))).unwrap();

        assert_eq!(resolved.backend, "ollama");
        assert_eq!(resolved.model, "qwen2.5:3b");
    }

    #[test]
    fn ollama_timeout_must_be_positive() {
        assert_eq!(parse_positive_timeout("45"), Ok(45));
        assert!(parse_positive_timeout("0").is_err());
        assert!(parse_positive_timeout("not-a-number").is_err());
    }
}
