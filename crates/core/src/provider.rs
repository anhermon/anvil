use crate::{
    error::Result,
    message::{Message, TurnResponse},
};
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// Lightweight tool definition passed to providers alongside messages.
/// Mirrors the JSON schema shape expected by Claude / OpenAI tool-calling APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A streaming token chunk from the provider.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub delta: String,
    pub done: bool,
}

pub type TokenStream = Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>;

/// Core provider trait — implemented per LLM backend.
///
/// Implementors: ClaudeProvider, OpenAIProvider, OllamaProvider, etc.
#[async_trait]
pub trait Provider: Send + Sync + 'static {
    /// Human-readable provider name (e.g. "claude-3-5-sonnet-20241022").
    fn name(&self) -> &str;

    /// Single non-streaming turn: send messages, get back a complete response.
    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse>;

    /// Turn with tool definitions made available to the LLM.
    /// Defaults to `complete` (tools ignored) for providers that don't support them yet.
    async fn complete_with_tools(
        &self,
        messages: &[Message],
        _tools: &[ToolDef],
    ) -> Result<TurnResponse> {
        self.complete(messages).await
    }

    /// Streaming turn: yields token chunks as they arrive.
    /// Default falls back to `complete` and emits one chunk.
    async fn stream(&self, messages: &[Message]) -> Result<TokenStream> {
        use futures::stream;
        let response = self.complete(messages).await?;
        let text = response.message.text().unwrap_or("").to_string();
        let chunk = StreamChunk {
            delta: text,
            done: true,
        };
        Ok(Box::pin(stream::once(async move { Ok(chunk) })))
    }

    /// Maximum context window in tokens (informational).
    fn context_limit(&self) -> usize {
        200_000
    }
}

/// Stub provider for tests — echoes input back.
pub struct EchoProvider;

#[async_trait]
impl Provider for EchoProvider {
    fn name(&self) -> &str {
        "echo"
    }

    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse> {
        use crate::message::{MessageContent, Role, StopReason, Usage};
        let last = messages
            .last()
            .and_then(|m| m.text())
            .unwrap_or("(empty)")
            .to_string();
        Ok(TurnResponse {
            message: Message {
                role: Role::Assistant,
                content: MessageContent::Text(format!("echo: {last}")),
            },
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: "echo".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    #[tokio::test]
    async fn echo_provider_round_trips() {
        let p = EchoProvider;
        let msgs = vec![Message::user("hello")];
        let resp = p.complete(&msgs).await.unwrap();
        assert_eq!(resp.message.text(), Some("echo: hello"));
        assert_eq!(resp.model, "echo");
    }
}
