use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::{
    error::{HarnessError, Result},
    message::{ContentBlock, Message, MessageContent, Role, StopReason, TurnResponse, Usage},
    provider::{Provider, ToolDef},
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ApiTool<'a>>,
}

#[derive(Serialize)]
struct ApiTool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a serde_json::Value,
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
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
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

// ── Shared request logic ───────────────────────────────────────────────────────

impl ClaudeProvider {
    /// Build messages, send request, parse response.
    ///
    /// When `tools` is empty the request is sent without a `tools` field,
    /// preserving identical wire behavior to the original `complete()`.
    async fn execute_request(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<TurnResponse> {
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

        let api_tools: Vec<ApiTool<'_>> = tools
            .iter()
            .map(|t| ApiTool {
                name: &t.name,
                description: &t.description,
                input_schema: &t.input_schema,
            })
            .collect();

        let body = ApiRequest {
            model: &self.model,
            max_tokens: self.max_tokens,
            messages: api_messages,
            system: system_prompt,
            tools: api_tools,
        };

        debug!(model = %self.model, tools = tools.len(), "sending request to Anthropic API");

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

        let stop_reason = match api_resp.stop_reason.as_deref() {
            Some("max_tokens") => StopReason::MaxTokens,
            Some("tool_use") => StopReason::ToolUse,
            Some("stop_sequence") => StopReason::StopSequence,
            _ => StopReason::EndTurn,
        };

        let usage = Usage {
            input_tokens: api_resp.usage.input_tokens,
            output_tokens: api_resp.usage.output_tokens,
            cache_read_tokens: api_resp.usage.cache_read_input_tokens,
            cache_write_tokens: api_resp.usage.cache_creation_input_tokens,
        };

        // If any tool_use blocks are present return the full structured content
        // so the agent loop can dispatch tool calls.  Otherwise fall back to
        // joining plain text (preserves existing `complete()` behavior).
        let has_tool_use = api_resp
            .content
            .iter()
            .any(|c| matches!(c, ApiContent::ToolUse { .. }));

        let message = if has_tool_use {
            let blocks: Vec<ContentBlock> = api_resp
                .content
                .into_iter()
                .filter_map(|c| match c {
                    ApiContent::Text { text } => Some(ContentBlock::Text { text }),
                    ApiContent::ToolUse { id, name, input } => {
                        Some(ContentBlock::ToolUse { id, name, input })
                    }
                    ApiContent::Unknown => None,
                })
                .collect();
            Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(blocks),
            }
        } else {
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
            Message::assistant(text)
        };

        Ok(TurnResponse {
            message,
            stop_reason,
            usage,
            model: api_resp.model,
        })
    }
}

// ── Provider impl ─────────────────────────────────────────────────────────────

#[async_trait]
impl Provider for ClaudeProvider {
    fn name(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse> {
        self.execute_request(messages, &[]).await
    }

    async fn complete_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<TurnResponse> {
        self.execute_request(messages, tools).await
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Returns the serialised `ApiRequest` body the provider would send.
    fn capture_request_body(messages: &[Message], tools: &[ToolDef]) -> serde_json::Value {
        let provider = ClaudeProvider::new("k", "m", 256);
        // Build the same body the real execute_request builds (minus system extraction).
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
                        MessageContent::Blocks(blocks) => serde_json::to_value(blocks).unwrap(),
                    };
                    api_messages.push(ApiMessage {
                        role: role.to_string(),
                        content,
                    });
                }
            }
        }
        let api_tools: Vec<ApiTool<'_>> = tools
            .iter()
            .map(|t| ApiTool {
                name: &t.name,
                description: &t.description,
                input_schema: &t.input_schema,
            })
            .collect();
        let body = ApiRequest {
            model: &provider.model,
            max_tokens: provider.max_tokens,
            messages: api_messages,
            system: system_prompt,
            tools: api_tools,
        };
        serde_json::to_value(body).unwrap()
    }

    #[test]
    fn tools_serialized_into_request_body() {
        let tool = ToolDef {
            name: "add".to_string(),
            description: "Adds two numbers".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "a": {"type": "number"},
                    "b": {"type": "number"}
                },
                "required": ["a", "b"]
            }),
        };
        let messages = vec![Message::user("what is 2+3?")];
        let body = capture_request_body(&messages, &[tool]);

        let tools = body.get("tools").expect("tools field present");
        assert!(tools.is_array());
        let arr = tools.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "add");
        assert_eq!(arr[0]["description"], "Adds two numbers");
        assert!(arr[0]["input_schema"].is_object());
    }

    #[test]
    fn no_tools_omits_field_from_body() {
        let messages = vec![Message::user("hello")];
        let body = capture_request_body(&messages, &[]);
        assert!(
            body.get("tools").is_none(),
            "tools key must be absent when empty"
        );
    }

    #[test]
    fn tool_use_content_parsed_into_blocks() {
        // Simulate what execute_request does with an ApiResponse that contains tool_use.
        let api_content = vec![
            ApiContent::Text {
                text: "Let me look that up.".to_string(),
            },
            ApiContent::ToolUse {
                id: "tu_abc123".to_string(),
                name: "search".to_string(),
                input: json!({"query": "rust lifetimes"}),
            },
        ];

        let has_tool_use = api_content
            .iter()
            .any(|c| matches!(c, ApiContent::ToolUse { .. }));
        assert!(has_tool_use);

        let blocks: Vec<ContentBlock> = api_content
            .into_iter()
            .filter_map(|c| match c {
                ApiContent::Text { text } => Some(ContentBlock::Text { text }),
                ApiContent::ToolUse { id, name, input } => {
                    Some(ContentBlock::ToolUse { id, name, input })
                }
                ApiContent::Unknown => None,
            })
            .collect();

        assert_eq!(blocks.len(), 2);
        assert!(
            matches!(&blocks[0], ContentBlock::Text { text } if text == "Let me look that up.")
        );
        assert!(
            matches!(&blocks[1], ContentBlock::ToolUse { id, name, .. } if id == "tu_abc123" && name == "search")
        );
    }

    #[test]
    fn text_only_response_returns_plain_message() {
        let api_content = vec![ApiContent::Text {
            text: "hello world".to_string(),
        }];

        let has_tool_use = api_content
            .iter()
            .any(|c| matches!(c, ApiContent::ToolUse { .. }));
        assert!(!has_tool_use);

        let text: String = api_content
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
        assert_eq!(text, "hello world");
    }

    /// Integration test — requires ANTHROPIC_API_KEY in environment.
    /// Gated with #[ignore] so it never runs in CI without explicit opt-in.
    #[tokio::test]
    #[ignore]
    async fn integration_tool_call_round_trip() {
        let provider = ClaudeProvider::from_env("claude-3-haiku-20240307", 1024)
            .expect("ANTHROPIC_API_KEY must be set for integration test");

        let tool = ToolDef {
            name: "get_weather".to_string(),
            description: "Get the current weather in a given location".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "City and state, e.g. San Francisco, CA"
                    }
                },
                "required": ["location"]
            }),
        };

        let messages = vec![Message::user(
            "What's the weather in San Francisco? Use the get_weather tool.",
        )];

        let resp = provider
            .complete_with_tools(&messages, &[tool])
            .await
            .expect("API call should succeed");

        assert_eq!(resp.stop_reason, StopReason::ToolUse);

        let has_tool_block = match &resp.message.content {
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { name, .. } if name == "get_weather")),
            _ => false,
        };
        assert!(
            has_tool_block,
            "expected get_weather tool_use block in response"
        );
    }
}
