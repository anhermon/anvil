use async_trait::async_trait;
use futures::stream;
use tokio::process::Command;
use tracing::debug;

use crate::{
    error::{HarnessError, Result},
    message::{Message, Role, StopReason, TurnResponse, Usage},
    provider::{Provider, StreamChunk, TokenStream, ToolDef},
};

/// Provider that delegates inference to the `claude` CLI binary via subprocess.
///
/// This inherits the full Claude Max subscription rate limits instead of the
/// more restricted direct-API OAuth pool. The binary must be available on PATH.
pub struct ClaudeCodeProvider {
    model: String,
}

impl ClaudeCodeProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
        }
    }

    pub fn default_model() -> Self {
        Self::new("claude-sonnet-4-5")
    }

    /// Flatten messages into a single text prompt for the subprocess.
    fn build_prompt(messages: &[Message]) -> String {
        let mut parts: Vec<String> = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    if let Some(text) = msg.text() {
                        parts.push(format!("[System]\n{text}"));
                    }
                }
                Role::User => {
                    if let Some(text) = msg.text() {
                        parts.push(format!("[User]\n{text}"));
                    }
                }
                Role::Assistant => {
                    if let Some(text) = msg.text() {
                        parts.push(format!("[Assistant]\n{text}"));
                    }
                }
                Role::Tool => {
                    if let Some(text) = msg.text() {
                        parts.push(format!("[Tool result]\n{text}"));
                    }
                }
            }
        }

        parts.join("\n\n")
    }

    /// Run `claude -p <prompt> --output-format json --model <model>`.
    ///
    /// Returns the text extracted from the `result` field of the JSON response.
    async fn run_subprocess(&self, prompt: &str) -> Result<String> {
        debug!(model = %self.model, "spawning claude subprocess");

        let output = Command::new("claude")
            .args([
                "-p",
                prompt,
                "--output-format",
                "json",
                "--model",
                &self.model,
                "--no-session-persistence",
            ])
            .output()
            .await
            .map_err(|e| {
                HarnessError::Provider(format!(
                    "failed to spawn claude binary: {e}. \
                     Ensure the `claude` CLI is installed and available on PATH."
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            return Err(HarnessError::Provider(format!(
                "claude subprocess exited with {}: stderr={stderr} stdout={stdout}",
                output.status
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // The JSON response shape from `claude --output-format json`:
        // {"type":"result","subtype":"success","result":"<text>","session_id":"...","cost_usd":0.001}
        let v: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|e| {
            HarnessError::Provider(format!(
                "failed to parse claude JSON output: {e}. raw: {stdout}"
            ))
        })?;

        // Extract `result` field (the text response).
        let text = v
            .get("result")
            .and_then(|r| r.as_str())
            .ok_or_else(|| {
                HarnessError::Provider(format!(
                    "claude JSON response missing `result` field. raw: {stdout}"
                ))
            })?
            .to_string();

        Ok(text)
    }
}

// -- Provider impl ------------------------------------------------------------

#[async_trait]
impl Provider for ClaudeCodeProvider {
    fn name(&self) -> &str {
        &self.model
    }

    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse> {
        let prompt = Self::build_prompt(messages);
        let text = self.run_subprocess(&prompt).await?;

        Ok(TurnResponse {
            message: Message::assistant(&text),
            stop_reason: StopReason::EndTurn,
            // Cost/token data not exposed by the subprocess JSON output in a
            // stable way -- report zeros so callers do not have to handle None.
            usage: Usage::default(),
            model: self.model.clone(),
        })
    }

    /// Tool injection into the subprocess is not directly supported via CLI flags;
    /// the claude binary manages its own tool registry. Fall back to `complete`.
    async fn complete_with_tools(
        &self,
        messages: &[Message],
        _tools: &[ToolDef],
    ) -> Result<TurnResponse> {
        self.complete(messages).await
    }

    /// Stream the subprocess output.
    ///
    /// Uses `--output-format stream-json`, parses line-by-line events, and emits
    /// `StreamChunk` deltas. Falls back to the `result` field if no streaming
    /// text events are found.
    async fn stream(&self, messages: &[Message]) -> Result<TokenStream> {
        let prompt = Self::build_prompt(messages);

        debug!(model = %self.model, "spawning claude subprocess (stream-json)");

        let output = Command::new("claude")
            .args([
                "-p",
                &prompt,
                "--output-format",
                "stream-json",
                "--model",
                &self.model,
                "--no-session-persistence",
            ])
            .output()
            .await
            .map_err(|e| HarnessError::Provider(format!("failed to spawn claude binary: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(HarnessError::Provider(format!(
                "claude subprocess (stream) exited with {}: {stderr}",
                output.status
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        // Parse newline-delimited JSON events and collect text chunks.
        let mut chunks: Vec<Result<StreamChunk>> = Vec::new();

        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(text) = extract_stream_text(&v) {
                    if !text.is_empty() {
                        chunks.push(Ok(StreamChunk {
                            delta: text,
                            done: false,
                        }));
                    }
                }
            }
        }

        // If no streaming events yielded text, fall back to the `result` field.
        if chunks.is_empty() {
            for line in stdout.lines() {
                let line = line.trim();
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(result_text) = v.get("result").and_then(|r| r.as_str()) {
                        if !result_text.is_empty() {
                            chunks.push(Ok(StreamChunk {
                                delta: result_text.to_string(),
                                done: false,
                            }));
                            break;
                        }
                    }
                }
            }
        }

        chunks.push(Ok(StreamChunk {
            delta: String::new(),
            done: true,
        }));

        Ok(Box::pin(stream::iter(chunks)))
    }
}

/// Extract displayable text from a stream-json event value.
fn extract_stream_text(v: &serde_json::Value) -> Option<String> {
    let event_type = v.get("type")?.as_str()?;
    match event_type {
        "text" => v.get("text")?.as_str().map(|s| s.to_string()),
        "content_block_delta" => v.get("delta").and_then(|d| {
            if d.get("type")?.as_str()? == "text_delta" {
                d.get("text")?.as_str().map(|s| s.to_string())
            } else {
                None
            }
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_joins_roles() {
        let msgs = vec![
            Message::system("You are helpful."),
            Message::user("Say hello."),
        ];
        let prompt = ClaudeCodeProvider::build_prompt(&msgs);
        assert!(prompt.contains("[System]"));
        assert!(prompt.contains("You are helpful."));
        assert!(prompt.contains("[User]"));
        assert!(prompt.contains("Say hello."));
    }

    #[test]
    fn extract_stream_text_handles_text_event() {
        let v = serde_json::json!({"type": "text", "text": "hello"});
        assert_eq!(extract_stream_text(&v), Some("hello".to_string()));
    }

    #[test]
    fn extract_stream_text_handles_content_block_delta() {
        let v = serde_json::json!({
            "type": "content_block_delta",
            "delta": {"type": "text_delta", "text": "world"}
        });
        assert_eq!(extract_stream_text(&v), Some("world".to_string()));
    }

    #[test]
    fn extract_stream_text_ignores_non_text_events() {
        let v = serde_json::json!({"type": "message_start", "message": {}});
        assert_eq!(extract_stream_text(&v), None);
    }
}
