use thiserror::Error;

#[derive(Error, Debug)]
pub enum HarnessError {
    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Memory error: {0}")]
    Memory(String),

    #[error("Tool error: {tool} — {message}")]
    Tool { tool: String, message: String },

    #[error("Context limit exceeded: used {used}, limit {limit}")]
    ContextLimit { used: usize, limit: usize },

    #[error("API error {status}: {body}")]
    Api { status: u16, body: String },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, HarnessError>;
