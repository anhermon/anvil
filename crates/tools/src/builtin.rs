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

/// Read the UTF-8 contents of a file at a given path.
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
/// Security: commands run in the process working directory.
/// Blocked: no network commands (curl, wget, nc), no sudo.
pub struct BashExecTool;

#[async_trait]
impl ToolHandler for BashExecTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "bash_exec".into(),
            description: "Execute a bash command and return stdout+stderr. \
                Blocked: sudo, curl, wget, nc, nmap. \
                Timeout: 30 seconds. \
                Use for: cargo build/test, git commands, file listing."
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

        // Block dangerous patterns
        let blocked = [
            "sudo",
            "curl ",
            "wget ",
            " nc ",
            "nmap",
            "rm -rf /",
            ":(){ :|:& };:",
        ];
        for pattern in &blocked {
            if command.contains(pattern) {
                return ToolOutput::err(format!("command contains blocked pattern: {pattern}"));
            }
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
    async fn bash_exec_blocks_sudo() {
        let tool = BashExecTool;
        let out = tool.call(json!({"command": "sudo ls"})).await;
        assert!(out.is_error);
        assert!(
            out.content.contains("blocked"),
            "expected blocked in: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn bash_exec_nonzero_exit_is_error() {
        let tool = BashExecTool;
        let out = tool.call(json!({"command": "exit 1"})).await;
        assert!(out.is_error, "expected error for non-zero exit");
        assert!(out.content.contains("exit_code: 1"), "got: {}", out.content);
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
