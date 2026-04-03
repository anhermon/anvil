use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::{
    auth::AuthMethod,
    error::{HarnessError, Result},
    message::{ContentBlock, Message, MessageContent, Role, StopReason, TurnResponse, Usage},
    provider::{Provider, StreamChunk, TokenStream, ToolDef},
};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct ClaudeProvider {
    client: Client,
    auth: AuthMethod,
    model: String,
    max_tokens: u32,
}

impl ClaudeProvider {
    /// Create a provider with an explicit API key (no credentials-file lookup).
    pub fn new(api_key: impl Into<String>, model: impl Into<String>, max_tokens: u32) -> Self {
        Self {
            client: Client::new(),
            auth: AuthMethod::ApiKey(api_key.into()),
            model: model.into(),
            max_tokens,
        }
    }

    /// Build from the best available auth source:
    /// subscription credentials file -> `ANTHROPIC_API_KEY` env var -> error.
    pub fn from_env(model: impl Into<String>, max_tokens: u32) -> Result<Self> {
        let auth = AuthMethod::resolve().map_err(|e| HarnessError::Config(e.to_string()))?;
        Ok(Self {
            client: Client::new(),
            auth,
            model: model.into(),
            max_tokens,
        })
    }

    /// Add the shared auth + versioning headers to a request builder.
    fn auth_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        self.auth
            .apply(builder)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
    }

    fn build_api_messages(
        &self,
        messages: &[Message],
    ) -> Result<(Option<String>, Vec<ApiMessage>)> {
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

        Ok((system_prompt, api_messages))
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
struct ApiRequestWithTools<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    tools: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct ApiStreamRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    stream: bool,
}

#[derive(Serialize, Clone)]
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

fn parse_stop_reason(s: Option<&str>) -> StopReason {
    match s {
        Some("max_tokens") => StopReason::MaxTokens,
        Some("tool_use") => StopReason::ToolUse,
        Some("stop_sequence") => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    }
}

fn api_resp_to_turn(api_resp: ApiResponse) -> TurnResponse {
    let stop_reason = parse_stop_reason(api_resp.stop_reason.as_deref());

    let mut blocks: Vec<ContentBlock> = Vec::new();
    for item in &api_resp.content {
        match item {
            ApiContent::Text { text } => {
                if !text.is_empty() {
                    blocks.push(ContentBlock::Text { text: text.clone() });
                }
            }
            ApiContent::ToolUse { id, name, input } => {
                blocks.push(ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }
            ApiContent::Unknown => {}
        }
    }

    let message = match blocks.len() {
        0 => Message::assistant(""),
        1 => {
            if let ContentBlock::Text { text } = &blocks[0] {
                Message::assistant(text.clone())
            } else {
                Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(blocks),
                }
            }
        }
        _ => Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(blocks),
        },
    };

    TurnResponse {
        message,
        stop_reason,
        usage: Usage {
            input_tokens: api_resp.usage.input_tokens,
            output_tokens: api_resp.usage.output_tokens,
            cache_read_tokens: api_resp.usage.cache_read_input_tokens,
            cache_write_tokens: api_resp.usage.cache_creation_input_tokens,
        },
        model: api_resp.model,
    }
}

// ── Provider impl ─────────────────────────────────────────────────────────────

#[async_trait]
impl Provider for ClaudeProvider {
    fn name(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse> {
        let (system_prompt, api_messages) = self.build_api_messages(messages)?;

        let body = ApiRequest {
            model: &self.model,
            max_tokens: self.max_tokens,
            messages: api_messages,
            system: system_prompt,
        };

        debug!(model = %self.model, "sending request to Anthropic API");

        let resp = self
            .auth_headers(self.client.post(ANTHROPIC_API_URL))
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

        Ok(api_resp_to_turn(api_resp))
    }

    async fn complete_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<TurnResponse> {
        let (system_prompt, api_messages) = self.build_api_messages(messages)?;

        let tool_defs: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        let body = ApiRequestWithTools {
            model: &self.model,
            max_tokens: self.max_tokens,
            messages: api_messages,
            system: system_prompt,
            tools: tool_defs,
        };

        debug!(model = %self.model, tools = tools.len(), "sending tool-use request to Anthropic API");

        let resp = self
            .auth_headers(self.client.post(ANTHROPIC_API_URL))
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
            warn!(status = %status, error = %msg, "Anthropic API error (tools)");
            return Err(HarnessError::Api {
                status: status.as_u16(),
                body: msg,
            });
        }

        let api_resp: ApiResponse = resp
            .json()
            .await
            .map_err(|e| HarnessError::Provider(e.to_string()))?;

        Ok(api_resp_to_turn(api_resp))
    }

    /// Real SSE streaming — yields text deltas as they arrive from the Anthropic API.
    async fn stream(&self, messages: &[Message]) -> Result<TokenStream> {
        let (system_prompt, api_messages) = self.build_api_messages(messages)?;

        let body = ApiStreamRequest {
            model: &self.model,
            max_tokens: self.max_tokens,
            messages: api_messages,
            system: system_prompt,
            stream: true,
        };

        debug!(model = %self.model, "opening SSE stream to Anthropic API");

        let resp = self
            .auth_headers(self.client.post(ANTHROPIC_API_URL))
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
            warn!(status = %status, error = %msg, "Anthropic SSE error");
            return Err(HarnessError::Api {
                status: status.as_u16(),
                body: msg,
            });
        }

        let (tx, rx) = futures::channel::mpsc::channel::<Result<StreamChunk>>(64);
        let mut byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            let mut tx = tx;
            let mut buf = String::new();

            while let Some(item) = byte_stream.next().await {
                match item {
                    Err(e) => {
                        let _ = tx.try_send(Err(HarnessError::Provider(e.to_string())));
                        return;
                    }
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));

                        // Drain complete lines from the buffer.
                        loop {
                            match buf.find('\n') {
                                None => break,
                                Some(pos) => {
                                    let line: String = buf.drain(..=pos).collect();
                                    let line = line.trim_end();

                                    if let Some(data) = line.strip_prefix("data: ") {
                                        if let Ok(v) =
                                            serde_json::from_str::<serde_json::Value>(data)
                                        {
                                            match v["type"].as_str().unwrap_or("") {
                                                "content_block_delta" => {
                                                    if v["delta"]["type"] == "text_delta" {
                                                        if let Some(text) =
                                                            v["delta"]["text"].as_str()
                                                        {
                                                            if tx
                                                                .try_send(Ok(StreamChunk {
                                                                    delta: text.to_string(),
                                                                    done: false,
                                                                }))
                                                                .is_err()
                                                            {
                                                                return;
                                                            }
                                                        }
                                                    }
                                                }
                                                "message_stop" => {
                                                    let _ = tx.try_send(Ok(StreamChunk {
                                                        delta: String::new(),
                                                        done: true,
                                                    }));
                                                    return;
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Stream ended without explicit message_stop.
            let _ = tx.try_send(Ok(StreamChunk {
                delta: String::new(),
                done: true,
            }));
        });

        Ok(Box::pin(rx))
    }
}
