use crate::schema::ToolSchema;
use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;

/// Result of executing a tool.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            content: msg.into(),
            is_error: true,
        }
    }
}

/// Trait that each tool must implement.
#[async_trait]
pub trait ToolHandler: Send + Sync + 'static {
    fn schema(&self) -> ToolSchema;
    async fn call(&self, input: Value) -> ToolOutput;
}

/// Central registry mapping tool names → handlers.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    handlers: Arc<DashMap<String, Arc<dyn ToolHandler>>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool. Overwrites any existing entry with the same name.
    pub fn register(&self, handler: impl ToolHandler) {
        let name = handler.schema().name.clone();
        self.handlers.insert(name, Arc::new(handler));
    }

    /// Execute a named tool, returning an error output if not found or input is invalid.
    pub async fn call(&self, name: &str, input: Value) -> ToolOutput {
        match self.handlers.get(name) {
            None => ToolOutput::err(format!("tool not found: {name}")),
            Some(handler) => {
                let schema = handler.schema();
                if let Err(e) = schema.validate(&input) {
                    return ToolOutput::err(format!("invalid input for {name}: {e}"));
                }
                handler.call(input).await
            }
        }
    }

    /// List all registered tool schemas (for passing to the LLM).
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.handlers.iter().map(|e| e.value().schema()).collect()
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ToolSchema;

    struct UpperCase;

    #[async_trait]
    impl ToolHandler for UpperCase {
        fn schema(&self) -> ToolSchema {
            ToolSchema::simple("uppercase", "Convert text to uppercase", &["text"])
        }
        async fn call(&self, input: Value) -> ToolOutput {
            let text = input["text"].as_str().unwrap_or("").to_uppercase();
            ToolOutput::ok(text)
        }
    }

    #[tokio::test]
    async fn registry_dispatches_tool() {
        let registry = ToolRegistry::new();
        registry.register(UpperCase);
        let out = registry
            .call("uppercase", serde_json::json!({"text": "hello"}))
            .await;
        assert_eq!(out.content, "HELLO");
        assert!(!out.is_error);
    }

    #[tokio::test]
    async fn registry_unknown_tool() {
        let registry = ToolRegistry::new();
        let out = registry.call("nonexistent", serde_json::json!({})).await;
        assert!(out.is_error);
    }
}
