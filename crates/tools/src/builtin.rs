/// Built-in tools registered by default.
///
/// Currently a stub — real implementations added as needed.
use crate::registry::{ToolHandler, ToolOutput};
use crate::schema::ToolSchema;
use async_trait::async_trait;
use serde_json::Value;

/// Echo tool — useful for testing the tool pipeline.
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
        let raw = input["path"].as_str().unwrap_or("").to_string();
        if raw.is_empty() {
            return ToolOutput::err("path is required");
        }
        // Reject absolute paths and traversal attempts
        let p = std::path::Path::new(&raw);
        if p.is_absolute() {
            return ToolOutput::err("absolute paths are not allowed");
        }
        if p.components().any(|c| c == std::path::Component::ParentDir) {
            return ToolOutput::err("path traversal (..) is not allowed");
        }
        match std::fs::read_to_string(&raw) {
            Ok(contents) => ToolOutput::ok(contents),
            Err(e) => ToolOutput::err(format!("read_file failed for {raw}: {e}")),
        }
    }
}
