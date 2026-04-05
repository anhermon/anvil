use crate::{
    error::Result,
    message::{ContentBlock, Message, MessageContent, Role, StopReason, TurnResponse, Usage},
};
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Mutex;

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

/// A scripted tool call for deterministic testing.
#[derive(Debug, Clone)]
pub struct ScriptedToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Stub provider for tests — echoes input back.
///
/// In its default mode, `EchoProvider` prefixes the last user message with
/// `"echo: "` and returns `StopReason::EndTurn`.
///
/// In **scripted mode** (via [`EchoProvider::scripted`]), it dequeues
/// pre-loaded tool calls one per turn, returning `StopReason::ToolUse`.
/// Once the script is exhausted it falls back to the normal echo behaviour.
/// This allows deterministic end-to-end testing of the full agent loop
/// including tool dispatch — no real LLM required.
pub struct EchoProvider {
    script: Mutex<VecDeque<ScriptedToolCall>>,
}

impl EchoProvider {
    /// Plain echo provider — no tool calls, just mirrors input.
    pub fn new() -> Self {
        Self {
            script: Mutex::new(VecDeque::new()),
        }
    }

    /// Scripted echo provider — emits pre-loaded tool calls in order,
    /// then falls back to normal echo behaviour.
    pub fn scripted(calls: Vec<ScriptedToolCall>) -> Self {
        Self {
            script: Mutex::new(VecDeque::from(calls)),
        }
    }
}

impl Default for EchoProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for EchoProvider {
    fn name(&self) -> &str {
        "echo"
    }

    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse> {
        // Check for a scripted tool call first.
        {
            let mut script = self.script.lock().unwrap();
            if let Some(call) = script.pop_front() {
                return Ok(TurnResponse {
                    message: Message {
                        role: Role::Assistant,
                        content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                            id: call.id,
                            name: call.name,
                            input: call.input,
                        }]),
                    },
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                    model: "echo".to_string(),
                });
            }
        }

        // Default: echo the last user message.
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
        let p = EchoProvider::new();
        let msgs = vec![Message::user("hello")];
        let resp = p.complete(&msgs).await.unwrap();
        assert_eq!(resp.message.text(), Some("echo: hello"));
        assert_eq!(resp.model, "echo");
    }

    #[tokio::test]
    async fn echo_provider_default_is_plain() {
        let p = EchoProvider::default();
        let msgs = vec![Message::user("test")];
        let resp = p.complete(&msgs).await.unwrap();
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert_eq!(resp.message.text(), Some("echo: test"));
    }

    #[tokio::test]
    async fn scripted_echo_emits_tool_call_then_echoes() {
        let p = EchoProvider::scripted(vec![ScriptedToolCall {
            id: "call-1".to_string(),
            name: "echo".to_string(),
            input: serde_json::json!({"message": "ping"}),
        }]);

        // First call returns the scripted tool use.
        let msgs = vec![Message::user("goal")];
        let r1 = p.complete(&msgs).await.unwrap();
        assert_eq!(r1.stop_reason, StopReason::ToolUse);
        match &r1.message.content {
            MessageContent::Blocks(blocks) => match &blocks[0] {
                ContentBlock::ToolUse { name, .. } => assert_eq!(name, "echo"),
                other => panic!("expected ToolUse, got {other:?}"),
            },
            other => panic!("expected Blocks, got {other:?}"),
        }

        // Second call falls back to echo.
        let r2 = p.complete(&msgs).await.unwrap();
        assert_eq!(r2.stop_reason, StopReason::EndTurn);
        assert_eq!(r2.message.text(), Some("echo: goal"));
    }

    #[tokio::test]
    async fn scripted_echo_drains_multiple_calls() {
        let p = EchoProvider::scripted(vec![
            ScriptedToolCall {
                id: "c1".to_string(),
                name: "read_file".to_string(),
                input: serde_json::json!({"path": "a.txt"}),
            },
            ScriptedToolCall {
                id: "c2".to_string(),
                name: "bash_exec".to_string(),
                input: serde_json::json!({"command": "ls"}),
            },
        ]);

        let msgs = vec![Message::user("do things")];

        let r1 = p.complete(&msgs).await.unwrap();
        assert_eq!(r1.stop_reason, StopReason::ToolUse);

        let r2 = p.complete(&msgs).await.unwrap();
        assert_eq!(r2.stop_reason, StopReason::ToolUse);

        // Script exhausted — falls back to echo.
        let r3 = p.complete(&msgs).await.unwrap();
        assert_eq!(r3.stop_reason, StopReason::EndTurn);
    }
}
