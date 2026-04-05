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
        assert!(schema
            .validate(&serde_json::json!({"command": "ls"}))
            .is_ok());
        assert!(schema.validate(&serde_json::json!({})).is_err());
    }

    #[test]
    fn simple_schema_builds_correct_json_structure() {
        let schema = ToolSchema::simple("test_tool", "A test tool", &["arg1", "arg2"]);
        assert_eq!(schema.name, "test_tool");
        assert_eq!(schema.description, "A test tool");

        let input_schema = &schema.input_schema;
        assert_eq!(input_schema["type"], "object");

        let props = input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("arg1"));
        assert!(props.contains_key("arg2"));

        let required = input_schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
    }

    #[test]
    fn to_def_converts_correctly() {
        let schema = ToolSchema::simple("bash", "Run command", &["command"]);
        let def = schema.to_def();
        assert_eq!(def.name, "bash");
        assert_eq!(def.description, "Run command");
        assert_eq!(def.input_schema, schema.input_schema);
    }

    #[test]
    fn validate_allows_extra_fields() {
        let schema = ToolSchema::simple("tool", "desc", &["required_field"]);
        let input = serde_json::json!({"required_field": "v", "extra": "ignored"});
        assert!(schema.validate(&input).is_ok());
    }

    #[test]
    fn validate_with_no_required_fields() {
        let schema = ToolSchema::simple("tool", "desc", &[]);
        assert!(schema.validate(&serde_json::json!({})).is_ok());
    }

    #[test]
    fn validate_error_message_includes_field_name() {
        let schema = ToolSchema::simple("tool", "desc", &["missing_field"]);
        let err = schema.validate(&serde_json::json!({})).unwrap_err();
        assert!(err.contains("missing_field"));
    }
}
