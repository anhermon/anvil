use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level harness configuration (loaded from ~/.paperclip/harness/config.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub provider: ProviderConfig,
    pub memory: MemoryConfig,
    pub agent: AgentConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Active provider: "claude", "openai", "ollama", "echo"
    pub backend: String,
    /// Model identifier (e.g. "claude-sonnet-4-5")
    pub model: String,
    /// Max tokens per response
    pub max_tokens: u32,
    /// API key — prefer reading from env var ANTHROPIC_API_KEY / OPENAI_API_KEY
    pub api_key: Option<String>,
    /// Base URL override (useful for Ollama or proxies)
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// SQLite database path
    pub db_path: PathBuf,
    /// Max episodes to retain in context window
    pub max_context_episodes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent name (shown in prompts)
    pub name: String,
    /// System prompt prefix
    pub system_prompt: Option<String>,
    /// Max iterations per run (0 = unlimited)
    pub max_iterations: usize,
}

impl Default for Config {
    fn default() -> Self {
        let mut db_path = dirs_home().unwrap_or_else(|| PathBuf::from("."));
        db_path.push(".paperclip/harness/memory.db");

        Self {
            provider: ProviderConfig {
                backend: "claude-code".to_string(),
                model: "claude-sonnet-4-5".to_string(),
                max_tokens: 8192,
                api_key: None,
                base_url: None,
            },
            memory: MemoryConfig {
                db_path,
                max_context_episodes: 20,
            },
            agent: AgentConfig {
                name: "anvil".to_string(),
                system_prompt: None,
                max_iterations: 50,
            },
        }
    }
}

impl Config {
    /// Load config from disk, falling back to defaults for missing values.
    pub fn load() -> anyhow::Result<Self> {
        let path = config_path();
        if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            Ok(toml::from_str(&raw)?)
        } else {
            Ok(Self::default())
        }
    }

    /// Resolve API key: config file → environment variable.
    pub fn resolved_api_key(&self) -> Option<String> {
        self.provider
            .api_key
            .clone()
            .or_else(|| match self.provider.backend.as_str() {
                "claude" => std::env::var("ANTHROPIC_API_KEY").ok(),
                "openai" => std::env::var("OPENAI_API_KEY").ok(),
                _ => None,
            })
    }
}

fn config_path() -> PathBuf {
    let mut p = dirs_home().unwrap_or_else(|| PathBuf::from("."));
    p.push(".paperclip/harness/config.toml");
    p
}

fn dirs_home() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var("USERPROFILE").ok().map(PathBuf::from);
    #[cfg(not(windows))]
    return std::env::var("HOME").ok().map(PathBuf::from);
}
