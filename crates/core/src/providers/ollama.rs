use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::{
    error::{HarnessError, Result},
    message::{ContentBlock, Message, MessageContent, Role, StopReason, TurnResponse, Usage},
    provider::{Provider, StreamChunk, TokenStream, ToolDef},
};

/// How many times to re-request a turn that came back completely empty.
///
/// Ollama returns a well-formed HTTP 200 with neither content nor tool calls at
/// a measurable rate on small local models. Without a retry the agent loop reads
/// it as an ordinary end-of-turn and abandons the task.
const EMPTY_RESPONSE_RETRIES: usize = 2;

pub struct OllamaProvider {
    client: Client,
    model: String,
    base_url: String,
    max_tokens: u32,
    /// `None` leaves sampling to the server's own default.
    ///
    /// Near-greedy decoding looks right for tool calling on priors, but
    /// measured the opposite on `qwen2.5:3b-instruct`: at 0.2 the model locks
    /// onto a wrong conclusion ("none of the provided functions can be used")
    /// and re-emits identical tool calls instead of varying its retry. Left
    /// unset unless the caller asks.
    temperature: Option<f32>,
    request_timeout: Duration,
}

/// Default per-request wait. Sized for a cold local model load rather than a
/// warm one; override with `--ollama-timeout-secs` / `ANVIL_OLLAMA_TIMEOUT_SECS`.
pub const DEFAULT_OLLAMA_TIMEOUT_SECS: u64 = 120;

fn timeout_label(timeout: Duration) -> String {
    if timeout.as_secs() > 0 {
        format!("{}s", timeout.as_secs())
    } else {
        format!("{}ms", timeout.as_millis())
    }
}

fn ollama_request_error(error: &reqwest::Error, timeout: Duration, endpoint: &str) -> HarnessError {
    if error.is_timeout() {
        HarnessError::Provider(format!(
            "Ollama request to {endpoint} timed out after {}. \
             The model may still be loading; retry, choose a smaller model, or increase \
             --ollama-timeout-secs (ANVIL_OLLAMA_TIMEOUT_SECS).",
            timeout_label(timeout)
        ))
    } else {
        HarnessError::Provider(error.to_string())
    }
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, max_tokens: u32) -> Self {
        Self::with_timeout(
            base_url,
            model,
            max_tokens,
            Duration::from_secs(DEFAULT_OLLAMA_TIMEOUT_SECS),
        )
    }

    pub fn with_timeout(
        base_url: impl Into<String>,
        model: impl Into<String>,
        max_tokens: u32,
        request_timeout: Duration,
    ) -> Self {
        // The timeout is applied per request rather than on the client, so a
        // caller-supplied bound is the single authority on how long a wait runs.
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            model: model.into(),
            max_tokens,
            temperature: None,
            request_timeout,
        }
    }

    /// Pin the sampling temperature instead of using the server default.
    #[must_use]
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        )
    }

    fn request(&self, endpoint: &str) -> reqwest::RequestBuilder {
        self.client.post(endpoint).timeout(self.request_timeout)
    }

    fn build_openai_messages(messages: &[Message]) -> Vec<OpenAiMessage> {
        let mut result = Vec::new();
        for msg in messages {
            let role = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };

            match &msg.content {
                MessageContent::Text(t) => {
                    result.push(OpenAiMessage {
                        role: role.to_string(),
                        content: Some(serde_json::Value::String(t.clone())),
                        tool_call_id: None,
                        tool_calls: None,
                    });
                }
                MessageContent::Blocks(blocks) => {
                    if msg.role == Role::Tool {
                        // For Role::Tool, each ToolResult block must be a separate message in OpenAI
                        for block in blocks {
                            if let ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                            } = block
                            {
                                result.push(OpenAiMessage {
                                    role: "tool".to_string(),
                                    content: Some(serde_json::Value::String(content.clone())),
                                    tool_call_id: Some(tool_use_id.clone()),
                                    tool_calls: None,
                                });
                            }
                        }
                    } else if msg.role == Role::Assistant {
                        let mut text = String::new();
                        let mut tool_calls = Vec::new();
                        for block in blocks {
                            match block {
                                ContentBlock::Text { text: t } => text.push_str(t),
                                ContentBlock::ToolUse { id, name, input } => {
                                    tool_calls.push(OpenAiToolCall {
                                        id: id.clone(),
                                        tool_type: "function".to_string(),
                                        function: OpenAiToolCallFunction {
                                            name: name.clone(),
                                            arguments: input.to_string(),
                                        },
                                    });
                                }
                                ContentBlock::ToolResult { .. } => {}
                            }
                        }
                        result.push(OpenAiMessage {
                            role: "assistant".to_string(),
                            content: if text.is_empty() {
                                None
                            } else {
                                Some(serde_json::Value::String(text))
                            },
                            tool_call_id: None,
                            tool_calls: if tool_calls.is_empty() {
                                None
                            } else {
                                Some(tool_calls)
                            },
                        });
                    } else {
                        // Fallback for System/User with blocks
                        let mut text = String::new();
                        for block in blocks {
                            if let ContentBlock::Text { text: t } = block {
                                text.push_str(t);
                            }
                        }
                        result.push(OpenAiMessage {
                            role: role.to_string(),
                            content: Some(serde_json::Value::String(text)),
                            tool_call_id: None,
                            tool_calls: None,
                        });
                    }
                }
            }
        }
        result
    }
}

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunction,
}

#[derive(Serialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Serialize, Deserialize, Clone)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiToolCallFunction,
}

#[derive(Serialize, Deserialize, Clone)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
    model: String,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Deserialize)]
struct OpenAiStreamResponse {
    choices: Vec<OpenAiStreamChoice>,
}

#[derive(Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiStreamDelta {
    content: Option<String>,
    #[allow(dead_code)]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse> {
        self.complete_with_tools(messages, &[]).await
    }

    async fn complete_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<TurnResponse> {
        // Retry completely empty completions. Ollama returns these at a
        // measurable rate for small local models; they are a transport-level
        // flake, not a decision by the model to stop, so re-asking is correct.
        let mut last = self.complete_once(messages, tools).await?;
        for attempt in 1..=EMPTY_RESPONSE_RETRIES {
            if !last.is_empty_turn() {
                break;
            }
            warn!(
                attempt,
                model = %self.model,
                "Ollama returned an empty completion; retrying"
            );
            last = self.complete_once(messages, tools).await?;
        }
        Ok(last)
    }

    async fn stream(&self, messages: &[Message]) -> Result<TokenStream> {
        self.stream_with_tools(messages, &[]).await
    }

    // Long but linear: a single top-to-bottom flow; splitting it would only scatter state.
    #[allow(clippy::too_many_lines)]
    async fn stream_with_tools(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<TokenStream> {
        self.stream_with_tools_inner(messages, tools).await
    }
}

impl OllamaProvider {
    // Long but linear: a single top-to-bottom flow; splitting it would only scatter state.
    #[allow(clippy::too_many_lines)]
    async fn complete_once(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<TurnResponse> {
        let openai_tools = if tools.is_empty() {
            None
        } else {
            Some(
                tools
                    .iter()
                    .map(|t| OpenAiTool {
                        tool_type: "function".to_string(),
                        function: OpenAiFunction {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            parameters: t.input_schema.clone(),
                        },
                    })
                    .collect(),
            )
        };

        let body = OpenAiRequest {
            model: &self.model,
            messages: Self::build_openai_messages(messages),
            max_tokens: Some(self.max_tokens),
            temperature: self.temperature,
            stream: false,
            tools: openai_tools,
            tool_choice: if tools.is_empty() {
                None
            } else {
                Some("auto".to_string())
            },
        };

        let endpoint = self.endpoint();
        debug!(model = %self.model, url = %endpoint, "sending request to Ollama (OpenAI-compatible)");

        let resp = self
            .request(&endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                ollama_request_error(&error, self.request_timeout, endpoint.as_str())
            })?;

        let status = resp.status();
        if !status.is_success() {
            let raw = resp.text().await.map_err(|error| {
                ollama_request_error(&error, self.request_timeout, endpoint.as_str())
            })?;
            return Err(HarnessError::Api {
                status: status.as_u16(),
                body: raw,
            });
        }

        let api_resp: OpenAiResponse = resp.json().await.map_err(|error| {
            ollama_request_error(&error, self.request_timeout, endpoint.as_str())
        })?;

        let choice = api_resp
            .choices
            .first()
            .ok_or_else(|| HarnessError::Provider("Ollama returned empty choices".to_string()))?;

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("tool_calls" | "function_call") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        let mut blocks = Vec::new();
        if let Some(text) = &choice.message.content {
            if !text.is_empty() {
                blocks.push(ContentBlock::Text { text: text.clone() });
            }
        }

        if let Some(tool_calls) = &choice.message.tool_calls {
            for tc in tool_calls {
                // A native call whose arguments are near-miss JSON (a raw newline
                // inside a shell command, a trailing comma, a truncated tail) used
                // to be dropped on the floor, which read to the agent loop as "the
                // model chose not to call a tool". Repair it instead.
                let input = match serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
                {
                    Ok(v) => Some(v),
                    Err(e) => {
                        let repaired = crate::toolcall_text::repair_json(&tc.function.arguments);
                        if repaired.is_some() {
                            warn!(error = %e, tool = %tc.function.name, "repaired malformed tool arguments from Ollama");
                        } else {
                            warn!(error = %e, tool = %tc.function.name, "failed to parse tool arguments from Ollama");
                        }
                        repaired
                    }
                };
                if let Some(input) = input {
                    blocks.push(ContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        input,
                    });
                }
            }
        }

        let usage = api_resp
            .usage
            .map(|u| Usage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
                ..Usage::default()
            })
            .unwrap_or_default();

        Ok(TurnResponse {
            message: Message {
                role: Role::Assistant,
                content: MessageContent::Blocks(blocks),
            },
            stop_reason,
            usage,
            model: api_resp.model,
        })
    }

    // Long but linear: a single top-to-bottom flow; splitting it would only scatter state.
    #[allow(clippy::too_many_lines)]
    async fn stream_with_tools_inner(
        &self,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<TokenStream> {
        let openai_tools = if tools.is_empty() {
            None
        } else {
            Some(
                tools
                    .iter()
                    .map(|t| OpenAiTool {
                        tool_type: "function".to_string(),
                        function: OpenAiFunction {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            parameters: t.input_schema.clone(),
                        },
                    })
                    .collect(),
            )
        };

        let body = OpenAiRequest {
            model: &self.model,
            messages: Self::build_openai_messages(messages),
            max_tokens: Some(self.max_tokens),
            temperature: self.temperature,
            stream: true,
            tools: openai_tools,
            tool_choice: if tools.is_empty() {
                None
            } else {
                Some("auto".to_string())
            },
        };

        let endpoint = self.endpoint();
        let resp = self
            .request(&endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                ollama_request_error(&error, self.request_timeout, endpoint.as_str())
            })?;

        let status = resp.status();
        if !status.is_success() {
            let raw = resp.text().await.map_err(|error| {
                ollama_request_error(&error, self.request_timeout, endpoint.as_str())
            })?;
            return Err(HarnessError::Api {
                status: status.as_u16(),
                body: raw,
            });
        }

        let (tx, rx) = futures::channel::mpsc::channel::<Result<StreamChunk>>(64);
        let mut byte_stream = resp.bytes_stream();
        let request_timeout = self.request_timeout;

        tokio::spawn(async move {
            let mut tx = tx;
            let mut buf = String::new();

            while let Some(item) = byte_stream.next().await {
                match item {
                    Err(e) => {
                        let _ = tx.try_send(Err(ollama_request_error(
                            &e,
                            request_timeout,
                            endpoint.as_str(),
                        )));
                        return;
                    }
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));

                        loop {
                            match buf.find('\n') {
                                None => break,
                                Some(pos) => {
                                    let line: String = buf.drain(..=pos).collect();
                                    let line = line.trim();

                                    if line.is_empty() {
                                        continue;
                                    }
                                    if line == "data: [DONE]" {
                                        let _ = tx.try_send(Ok(StreamChunk {
                                            delta: String::new(),
                                            done: true,
                                        }));
                                        return;
                                    }

                                    if let Some(data) = line.strip_prefix("data: ") {
                                        if let Ok(v) =
                                            serde_json::from_str::<OpenAiStreamResponse>(data)
                                        {
                                            if let Some(choice) = v.choices.first() {
                                                if let Some(content) = &choice.delta.content {
                                                    if tx
                                                        .try_send(Ok(StreamChunk {
                                                            delta: content.clone(),
                                                            done: false,
                                                        }))
                                                        .is_err()
                                                    {
                                                        return;
                                                    }
                                                }
                                                if choice.finish_reason.is_some() {
                                                    let _ = tx.try_send(Ok(StreamChunk {
                                                        delta: String::new(),
                                                        done: true,
                                                    }));
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok(Box::pin(rx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn spawn_stalled_server(send_headers: bool) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = socket.read(&mut request).await;
            if send_headers {
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\n\
                          Content-Type: text/event-stream\r\n\
                          Transfer-Encoding: chunked\r\n\r\n",
                    )
                    .await
                    .unwrap();
            }
            std::future::pending::<()>().await;
        });
        (format!("http://{address}"), task)
    }

    #[tokio::test]
    async fn non_streaming_request_timeout_is_bounded_and_actionable() {
        let (base_url, server) = spawn_stalled_server(false).await;
        let provider =
            OllamaProvider::with_timeout(base_url, "test-model", 128, Duration::from_millis(100));

        let error = provider
            .complete(&[Message::user("hello")])
            .await
            .unwrap_err()
            .to_string();
        server.abort();

        assert!(error.contains("timed out after 100ms"), "{error}");
        assert!(error.contains("--ollama-timeout-secs"), "{error}");
        assert!(error.contains("model may still be loading"), "{error}");
    }

    #[tokio::test]
    async fn streaming_body_timeout_is_bounded_and_actionable() {
        let (base_url, server) = spawn_stalled_server(true).await;
        let provider =
            OllamaProvider::with_timeout(base_url, "test-model", 128, Duration::from_millis(100));

        let mut stream = provider
            .stream(&[Message::user("hello")])
            .await
            .expect("response headers should arrive before the timeout");
        let error = stream
            .next()
            .await
            .expect("stream should report the timeout")
            .unwrap_err()
            .to_string();
        server.abort();

        assert!(error.contains("timed out after 100ms"), "{error}");
        assert!(error.contains("--ollama-timeout-secs"), "{error}");
    }
}
