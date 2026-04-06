use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    error::{HarnessError, Result},
    message::{Message, MessageContent, Role, StopReason, TurnResponse, Usage},
    provider::{Provider, StreamChunk, TokenStream},
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
            client: Client::new(),
            base_url: base_url.into(),
            model: model.into(),
            max_tokens,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn build_openai_messages(&self, messages: &[Message]) -> Vec<OpenAiMessage> {
        messages
            .iter()
            .map(|msg| {
                let role = match msg.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };

                let content = match &msg.content {
                    MessageContent::Text(t) => serde_json::Value::String(t.clone()),
                    MessageContent::Blocks(blocks) => {
                        serde_json::to_value(blocks).unwrap_or(serde_json::Value::String(String::new()))
                    }
                };

                OpenAiMessage {
                    role: role.to_string(),
                    content,
                }
            })
            .collect()
    }
}

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: serde_json::Value,
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
    role: String,
    content: Option<String>,
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
}

#[async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse> {
        let body = OpenAiRequest {
            model: &self.model,
            messages: self.build_openai_messages(messages),
            max_tokens: Some(self.max_tokens),
            stream: false,
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

        let text = choice.message.content.clone().unwrap_or_default();
        let usage = api_resp.usage.map(|u| Usage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            ..Usage::default()
        }).unwrap_or_default();

        Ok(TurnResponse {
            message: Message::assistant(text),
            stop_reason: StopReason::EndTurn,
            usage,
            model: api_resp.model,
        })
    }

    async fn stream(&self, messages: &[Message]) -> Result<TokenStream> {
        let body = OpenAiRequest {
            model: &self.model,
            messages: self.build_openai_messages(messages),
            max_tokens: Some(self.max_tokens),
            stream: true,
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

                                    if line.is_empty() { continue; }
                                    if line == "data: [DONE]" {
                                        let _ = tx.try_send(Ok(StreamChunk {
                                            delta: String::new(),
                                            done: true,
                                        }));
                                        return;
                                    }

                                    if let Some(data) = line.strip_prefix("data: ") {
                                        if let Ok(v) = serde_json::from_str::<OpenAiStreamResponse>(data) {
                                            if let Some(choice) = v.choices.first() {
                                                if let Some(content) = &choice.delta.content {
                                                    if tx.try_send(Ok(StreamChunk {
                                                        delta: content.clone(),
                                                        done: false,
                                                    })).is_err() {
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
