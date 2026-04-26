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
    /// When `true`, pass `--dangerously-skip-permissions` to the subprocess so
    /// that file-write operations are not blocked by the claude CLI permission
    /// guard.  Only set this when the caller has explicitly opted in via
    /// `--allow-writes` (or equivalent).
    allow_writes: bool,
}

impl ClaudeCodeProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            allow_writes: false,
        }
    }

    /// Enable file-write operations by passing `--dangerously-skip-permissions`
    /// to the claude subprocess.
    pub fn with_allow_writes(mut self, allow_writes: bool) -> Self {
        self.allow_writes = allow_writes;
        self
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
        debug!(model = %self.model, allow_writes = %self.allow_writes, "spawning claude subprocess");

        let mut cmd = Command::new("claude");
        cmd.args([
            "-p",
            prompt,
            "--output-format",
            "json",
            "--model",
            &self.model,
            "--no-session-persistence",
        ]);
        if self.allow_writes {
            cmd.arg("--dangerously-skip-permissions");
        }

        let output = cmd.output().await.map_err(|e| {
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

        // If the response contains a permission-denial pattern, surface a hard
        // error with a clear remediation step rather than silently returning the
        // confusing "requires approval" message with no next step.
        if !self.allow_writes && is_permission_denial(&text) {
            return Err(HarnessError::Provider(
                "The claude CLI blocked a write operation — interactive permission approval \
                 is not available in subprocess mode.\n\n\
                 To enable file writes, re-run with the --allow-writes flag:\n\n  \
                 anvil run --allow-writes --goal \"...\"\n\n\
                 This passes --dangerously-skip-permissions to the claude subprocess."
                    .to_string(),
            ));
        }

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

        debug!(model = %self.model, allow_writes = %self.allow_writes, "spawning claude subprocess (stream-json)");

        let mut cmd = Command::new("claude");
        cmd.args([
            "-p",
            &prompt,
            "--output-format",
            "stream-json",
            "--model",
            &self.model,
            "--no-session-persistence",
        ]);
        if self.allow_writes {
            cmd.arg("--dangerously-skip-permissions");
        }

        let output = cmd
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

        // If the accumulated text looks like a permission denial, return an error
        // instead of streaming the confusing "requires approval" message.
        if !self.allow_writes {
            let accumulated: String = chunks
                .iter()
                .filter_map(|c| c.as_ref().ok())
                .map(|c| c.delta.as_str())
                .collect();
            if is_permission_denial(&accumulated) {
                return Err(HarnessError::Provider(
                    "The claude CLI blocked a write operation — interactive permission \
                     approval is not available in subprocess mode.\n\n\
                     To enable file writes, re-run with the --allow-writes flag:\n\n  \
                     anvil run --allow-writes --goal \"...\"\n\n\
                     This passes --dangerously-skip-permissions to the claude subprocess."
                        .to_string(),
                ));
            }
        }

        chunks.push(Ok(StreamChunk {
            delta: String::new(),
            done: true,
        }));

        Ok(Box::pin(stream::iter(chunks)))
    }
}

/// Return `true` when the response text looks like a permission-denial message
/// emitted by the claude CLI's interactive permission guard.
///
/// These messages are soft responses (subprocess exits 0) but have no
/// actionable next step for the user when running in non-interactive mode.
fn is_permission_denial(text: &str) -> bool {
    let lower = text.to_lowercase();
    // Match phrases the claude CLI typically produces when write permission is blocked.
    lower.contains("requires your permission")
        || lower.contains("permission approval")
        || lower.contains("requires approval to")
        || lower.contains("needs your permission")
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

    // -- is_permission_denial tests -------------------------------------------

    #[test]
    fn permission_denial_detects_requires_your_permission() {
        assert!(is_permission_denial(
            "I've attempted to create the file. The operation requires your permission \
             approval to proceed."
        ));
    }

    #[test]
    fn permission_denial_detects_permission_approval() {
        assert!(is_permission_denial(
            "This action requires permission approval before it can continue."
        ));
    }

    #[test]
    fn permission_denial_detects_requires_approval_to() {
        assert!(is_permission_denial(
            "The write tool requires approval to execute on this system."
        ));
    }

    #[test]
    fn permission_denial_detects_needs_your_permission() {
        assert!(is_permission_denial(
            "Writing to /tmp/test.txt needs your permission."
        ));
    }

    #[test]
    fn permission_denial_is_case_insensitive() {
        assert!(is_permission_denial(
            "THIS OPERATION REQUIRES YOUR PERMISSION APPROVAL."
        ));
    }

    #[test]
    fn permission_denial_does_not_match_normal_response() {
        assert!(!is_permission_denial(
            "I have created the file /tmp/test.txt with the content 'hello'."
        ));
    }

    #[test]
    fn permission_denial_does_not_match_empty_string() {
        assert!(!is_permission_denial(""));
    }

    // -- with_allow_writes builder test ---------------------------------------

    #[test]
    fn with_allow_writes_sets_flag() {
        let p = ClaudeCodeProvider::new("claude-sonnet-4-5").with_allow_writes(true);
        assert!(p.allow_writes);
    }

    #[test]
    fn new_defaults_allow_writes_false() {
        let p = ClaudeCodeProvider::new("claude-sonnet-4-5");
        assert!(!p.allow_writes);
    }
}
