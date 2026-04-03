use harness_core::provider::ToolDef;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON Schema definition for a tool's input parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// JSON Schema object describing the `input` parameter.
    pub input_schema: Value,
}

impl ToolSchema {
    /// Build a simple schema with named string properties.
    pub fn simple(name: &str, description: &str, required_strings: &[&str]) -> Self {
        let properties: serde_json::Map<String, Value> = required_strings
            .iter()
            .map(|k| (k.to_string(), serde_json::json!({"type": "string"})))
            .collect();

        Self {
            name: name.to_string(),
            description: description.to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": required_strings,
            }),
        }
    }

    /// Convert to a `ToolDef` for passing to provider methods.
    pub fn to_def(&self) -> ToolDef {
        ToolDef {
            name: self.name.clone(),
            description: self.description.clone(),
            input_schema: self.input_schema.clone(),
        }
    }

    /// Validate an input value against this schema (basic required-field check).
    pub fn validate(&self, input: &Value) -> Result<(), String> {
        let schema = &self.input_schema;
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            for field in required {
                let key = field.as_str().unwrap_or("");
                if input.get(key).is_none() {
                    return Err(format!("missing required field: {key}"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_required_fields() {
        let schema = ToolSchema::simple("bash", "Run a shell command", &["command"]);
        assert!(schema.validate(&serde_json::json!({"command": "ls"})).is_ok());
        assert!(schema.validate(&serde_json::json!({})).is_err());
    }
}
