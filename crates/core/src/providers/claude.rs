use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::{
    error::{HarnessError, Result},
    message::{Message, MessageContent, Role, StopReason, TurnResponse, Usage},
    provider::Provider,
};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct ClaudeProvider {
    client: Client,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl ClaudeProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>, max_tokens: u32) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens,
        }
    }

    /// Build from environment variable ANTHROPIC_API_KEY.
    pub fn from_env(model: impl Into<String>, max_tokens: u32) -> Result<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| HarnessError::Config("ANTHROPIC_API_KEY not set".to_string()))?;
        Ok(Self::new(key, model, max_tokens))
    }
}

// ── Anthropic API wire types ──────────────────────────────────────────────────

#[derive(Serialize)]
struct ApiRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
}

#[derive(Serialize)]
struct ApiMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ApiContent>,
    stop_reason: Option<String>,
    usage: ApiUsage,
    model: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ApiContent {
    Text {
        text: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct ApiUsage {
    input_tokens: u32,
    output_tokens: u32,
    cache_read_input_tokens: Option<u32>,
    cache_creation_input_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ApiError {
    error: ApiErrorBody,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    message: String,
}

// ── Provider impl ─────────────────────────────────────────────────────────────

#[async_trait]
impl Provider for ClaudeProvider {
    fn name(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse> {
        let mut system_prompt: Option<String> = None;
        let mut api_messages: Vec<ApiMessage> = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    system_prompt = msg.text().map(|t| t.to_string());
                }
                Role::User | Role::Assistant | Role::Tool => {
                    let role = match msg.role {
                        Role::User | Role::Tool => "user",
                        Role::Assistant => "assistant",
                        Role::System => unreachable!(),
                    };
                    let content = match &msg.content {
                        MessageContent::Text(t) => serde_json::Value::String(t.clone()),
                        MessageContent::Blocks(blocks) => {
                            serde_json::to_value(blocks).map_err(HarnessError::Serialization)?
                        }
                    };
                    api_messages.push(ApiMessage {
                        role: role.to_string(),
                        content,
                    });
                }
            }
        }

        let body = ApiRequest {
            model: &self.model,
            max_tokens: self.max_tokens,
            messages: api_messages,
            system: system_prompt,
        };

        debug!(model = %self.model, "sending request to Anthropic API");

        let resp = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| HarnessError::Provider(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let raw = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<ApiError>(&raw)
                .map(|e| e.error.message)
                .unwrap_or(raw);
            warn!(status = %status, error = %msg, "Anthropic API error");
            return Err(HarnessError::Api {
                status: status.as_u16(),
                body: msg,
            });
        }

        let api_resp: ApiResponse = resp
            .json()
            .await
            .map_err(|e| HarnessError::Provider(e.to_string()))?;

        let text = api_resp
            .content
            .iter()
            .filter_map(|c| {
                if let ApiContent::Text { text } = c {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        let stop_reason = match api_resp.stop_reason.as_deref() {
            Some("max_tokens") => StopReason::MaxTokens,
            Some("tool_use") => StopReason::ToolUse,
            Some("stop_sequence") => StopReason::StopSequence,
            _ => StopReason::EndTurn,
        };

        Ok(TurnResponse {
            message: Message::assistant(text),
            stop_reason,
            usage: Usage {
                input_tokens: api_resp.usage.input_tokens,
                output_tokens: api_resp.usage.output_tokens,
                cache_read_tokens: api_resp.usage.cache_read_input_tokens,
                cache_write_tokens: api_resp.usage.cache_creation_input_tokens,
            },
            model: api_resp.model,
        })
    }
}
