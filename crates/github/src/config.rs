use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for the GitHub webhook server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Address to bind the HTTP server (e.g. "0.0.0.0:3000")
    #[serde(default = "default_bind")]
    pub bind: String,

    /// GitHub webhook secret used to verify HMAC-SHA256 signatures.
    /// Set this to the same value as configured in the GitHub webhook settings.
    pub webhook_secret: String,

    /// Paperclip API base URL (e.g. "http://localhost:4000")
    pub paperclip_api_url: String,

    /// Paperclip API key (agent JWT or personal access token)
    pub paperclip_api_key: String,

    /// Paperclip company ID
    pub paperclip_company_id: String,

    /// Mapping of GitHub @mention handles to Paperclip agent IDs.
    /// Example: { "build-agent" => "uuid-of-agent" }
    ///
    /// A comment containing `@build-agent` will create a task assigned to the
    /// agent whose ID is listed here. Handles are matched case-insensitively.
    #[serde(default)]
    pub agents: HashMap<String, String>,
}

fn default_bind() -> String {
    "0.0.0.0:3000".to_string()
}
