/// Built-in tools registered by default.
///
/// Currently a stub - real implementations added as needed.
use crate::registry::{ToolHandler, ToolOutput};
use crate::schema::ToolSchema;
use async_trait::async_trait;
use serde_json::Value;

/// Echo tool - useful for testing the tool pipeline.
pub struct EchoTool;

#[async_trait]
impl ToolHandler for EchoTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema::simple("echo", "Echo the input message back", &["message"])
    }

    async fn call(&self, input: Value) -> ToolOutput {
        let msg = input["message"].as_str().unwrap_or("(empty)");
        ToolOutput::ok(msg)
    }
}

/// Spawn a sub-agent to handle a sub-task.
///
/// The actual execution is handled by the [`Agent`] loop in `harness-cli` —
/// it intercepts calls to this tool name and runs a nested Agent instance.
/// This struct exists only to declare the schema so the LLM sees the tool.
pub struct SpawnSubagentTool;

#[async_trait]
impl ToolHandler for SpawnSubagentTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "spawn_subagent".to_string(),
            description: "Spawn a sub-agent to handle a delegated sub-task. \
                 The sub-agent shares the same provider and tool set as the main agent. \
                 Returns the sub-agent's final response text. \
                 Maximum nesting depth is 4 (MAX_SUBAGENT_DEPTH); calls beyond that depth \
                 are rejected with an error."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "goal": {
                        "type": "string",
                        "description": "The goal or task for the sub-agent to accomplish."
                    },
                    "context": {
                        "type": "string",
                        "description": "Optional background context for the sub-agent."
                    }
                },
                "required": ["goal"]
            }),
        }
    }

    async fn call(&self, _input: Value) -> ToolOutput {
        // The Agent handles spawn_subagent before it reaches the registry.
        ToolOutput::err("spawn_subagent must be handled by the agent loop, not the registry")
    }
}

/// Read the UTF-8 contents of a file at a given path.
///
/// # Security
///
/// To prevent agents from reading arbitrary host files, this tool:
/// - Rejects absolute paths
/// - Rejects any path component that is `..` (parent-directory traversal)
///
/// Only relative paths that stay within the working directory are permitted.
pub struct ReadFileTool;

#[async_trait]
impl ToolHandler for ReadFileTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema::simple("read_file", "Read the UTF-8 contents of a file", &["path"])
    }

    async fn call(&self, input: Value) -> ToolOutput {
        let path = match input["path"].as_str() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return ToolOutput::err("missing required field: path"),
        };

        let p = std::path::Path::new(&path);
        if p.is_absolute() || path.starts_with('/') {
            return ToolOutput::err("absolute paths are not allowed");
        }
        if p.components().any(|c| c == std::path::Component::ParentDir) {
            return ToolOutput::err("path traversal (..) is not allowed");
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => ToolOutput::ok(contents),
            Err(e) => ToolOutput::err(format!("read_file failed for {path}: {e}")),
        }
    }
}

/// Executes a shell command and returns stdout/stderr.
/// Security: only commands whose first token appears in ALLOWED_COMMANDS are permitted.
pub struct BashExecTool;

const ALLOWED_COMMANDS: &[&str] = &[
    "cargo", "rustfmt", "rustc", "git", "ls", "cat", "echo", "pwd", "env", "which",
];

#[async_trait]
impl ToolHandler for BashExecTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "bash_exec".into(),
            description: "Execute a shell command and return stdout+stderr. \
                Only allowlisted commands are permitted: cargo, rustfmt, rustc, git, ls, cat, echo, pwd, env, which. \
                Timeout: 30 seconds."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, input: Value) -> ToolOutput {
        let command = match input["command"].as_str() {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => return ToolOutput::err("command is required"),
        };

        // Allowlist check: only permit commands whose first token is in ALLOWED_COMMANDS
        let first_token = command.split_whitespace().next().unwrap_or("");
        if !ALLOWED_COMMANDS.contains(&first_token) {
            return ToolOutput::err(format!(
                "Command not allowed: only [{}] are permitted",
                ALLOWED_COMMANDS.join(", ")
            ));
        }

        use std::time::Duration;

        let output = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::task::spawn_blocking(move || {
                std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .output()
            }),
        )
        .await;

        match output {
            Ok(Ok(Ok(out))) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let exit_code = out.status.code().unwrap_or(-1);
                let result = if stderr.is_empty() {
                    format!("exit_code: {exit_code}\n{stdout}")
                } else {
                    format!("exit_code: {exit_code}\nstdout:\n{stdout}\nstderr:\n{stderr}")
                };
                if out.status.success() {
                    ToolOutput::ok(result)
                } else {
                    ToolOutput::err(result)
                }
            }
            Ok(Ok(Err(e))) => ToolOutput::err(format!("failed to spawn process: {e}")),
            Ok(Err(e)) => ToolOutput::err(format!("task panic: {e}")),
            Err(_) => ToolOutput::err("command timed out after 30 seconds"),
        }
    }
}

/// Writes content to a file. Rejects absolute paths and traversal.
pub struct WriteFileTool;

#[async_trait]
impl ToolHandler for WriteFileTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_file".into(),
            description: "Write content to a file at the given relative path. \
                Creates parent directories as needed. \
                Rejects absolute paths and \'..\' traversal."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative file path to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn call(&self, input: Value) -> ToolOutput {
        let raw = match input["path"].as_str() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return ToolOutput::err("path is required"),
        };
        let content = input["content"].as_str().unwrap_or("").to_string();

        let p = std::path::Path::new(&raw);
        if p.is_absolute() || raw.starts_with('/') {
            return ToolOutput::err("absolute paths are not allowed");
        }
        if p.components().any(|c| c == std::path::Component::ParentDir) {
            return ToolOutput::err("path traversal (..) is not allowed");
        }

        // Create parent directories if needed
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return ToolOutput::err(format!("failed to create directories: {e}"));
                }
            }
        }

        match std::fs::write(&raw, &content) {
            Ok(()) => ToolOutput::ok(format!("wrote {} bytes to {raw}", content.len())),
            Err(e) => ToolOutput::err(format!("write_file failed for {raw}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn bash_exec_echo() {
        let tool = BashExecTool;
        let out = tool.call(json!({"command": "echo hello"})).await;
        assert!(!out.is_error, "unexpected error: {}", out.content);
        assert!(
            out.content.contains("hello"),
            "expected hello in: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn bash_exec_sudo_not_allowed() {
        let tool = BashExecTool;
        let out = tool.call(json!({"command": "sudo rm -rf /"})).await;
        assert!(out.is_error);
        assert!(
            out.content.contains("not allowed"),
            "expected 'not allowed' in: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn bash_exec_cargo_version_allowed() {
        let tool = BashExecTool;
        let out = tool.call(json!({"command": "cargo --version"})).await;
        assert!(
            !out.is_error,
            "expected cargo --version to succeed: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn bash_exec_nonzero_exit_is_error() {
        let tool = BashExecTool;
        // ls on a non-existent path exits non-zero; ls is in the allowlist
        let out = tool
            .call(json!({"command": "ls /this_path_does_not_exist_xyz_12345"}))
            .await;
        assert!(out.is_error, "expected error for non-zero exit");
    }

    #[tokio::test]
    async fn bash_exec_missing_command_is_error() {
        let tool = BashExecTool;
        let out = tool.call(json!({})).await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn write_file_creates_file() {
        let tool = WriteFileTool;
        let out = tool
            .call(json!({"path": "test_write_output.txt", "content": "hello world"}))
            .await;
        let _ = std::fs::remove_file("test_write_output.txt");
        assert!(!out.is_error, "write failed: {}", out.content);
        assert!(out.content.contains("11 bytes"), "got: {}", out.content);
    }

    #[tokio::test]
    async fn write_file_rejects_traversal() {
        let tool = WriteFileTool;
        let out = tool
            .call(json!({"path": "../../evil.txt", "content": "bad"}))
            .await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn write_file_rejects_absolute() {
        let tool = WriteFileTool;
        let out = tool
            .call(json!({"path": "/tmp/evil.txt", "content": "bad"}))
            .await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn read_file_rejects_traversal() {
        let tool = ReadFileTool;
        let out = tool.call(json!({"path": "../../.env"})).await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn read_file_rejects_absolute() {
        let tool = ReadFileTool;
        let out = tool.call(json!({"path": "/etc/passwd"})).await;
        assert!(out.is_error);
    }
}
