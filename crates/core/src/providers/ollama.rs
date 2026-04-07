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

pub struct OllamaProvider {
    client: Client,
    model: String,
    base_url: String,
    max_tokens: u32,
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, max_tokens: u32) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base_url: base_url.into(),
            model: model.into(),
            max_tokens,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn build_openai_messages(&self, messages: &[Message]) -> Vec<OpenAiMessage> {
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
                            if let ContentBlock::ToolResult { tool_use_id, content } = block {
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
                                _ => {}
                            }
                        }
                        result.push(OpenAiMessage {
                            role: "assistant".to_string(),
                            content: if text.is_empty() { None } else { Some(serde_json::Value::String(text)) },
                            tool_call_id: None,
                            tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
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
            messages: self.build_openai_messages(messages),
            max_tokens: Some(self.max_tokens),
            stream: false,
            tools: openai_tools,
            tool_choice: if tools.is_empty() { None } else { Some("auto".to_string()) },
        };

        debug!(model = %self.model, url = %self.endpoint(), "sending request to Ollama (OpenAI-compatible)");

        let resp = self
            .client
            .post(self.endpoint())
            .json(&body)
            .send()
            .await
            .map_err(|e| HarnessError::Provider(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let raw = resp.text().await.unwrap_or_default();
            return Err(HarnessError::Api {
                status: status.as_u16(),
                body: raw,
            });
        }

        let api_resp: OpenAiResponse = resp
            .json()
            .await
            .map_err(|e| HarnessError::Provider(e.to_string()))?;

        let choice = api_resp.choices.first().ok_or_else(|| {
            HarnessError::Provider("Ollama returned empty choices".to_string())
        })?;

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
            Some("stop") => StopReason::EndTurn,
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
                match serde_json::from_str::<serde_json::Value>(&tc.function.arguments) {
                    Ok(input) => {
                        blocks.push(ContentBlock::ToolUse {
                            id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            input,
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to parse tool arguments from Ollama");
                    }
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

    async fn stream(&self, messages: &[Message]) -> Result<TokenStream> {
        self.stream_with_tools(messages, &[]).await
    }

    async fn stream_with_tools(
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
            messages: self.build_openai_messages(messages),
            max_tokens: Some(self.max_tokens),
            stream: true,
            tools: openai_tools,
            tool_choice: if tools.is_empty() { None } else { Some("auto".to_string()) },
        };

        let resp = self
            .client
            .post(self.endpoint())
            .json(&body)
            .send()
            .await
            .map_err(|e| HarnessError::Provider(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let raw = resp.text().await.unwrap_or_default();
            return Err(HarnessError::Api {
                status: status.as_u16(),
                body: raw,
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
