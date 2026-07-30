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
    #[must_use]
    pub fn simple(name: &str, description: &str, required_strings: &[&str]) -> Self {
        let properties: serde_json::Map<String, Value> = required_strings
            .iter()
            .map(|k| ((*k).to_string(), serde_json::json!({"type": "string"})))
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
    #[must_use]
    pub fn to_def(&self) -> ToolDef {
        ToolDef {
            name: self.name.clone(),
            description: self.description.clone(),
            input_schema: self.input_schema.clone(),
        }
    }

    /// Validate an input value against the schema shape used by built-in tools.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a human-readable message naming the first missing or
    /// incorrectly typed field.
    pub fn validate(&self, input: &Value) -> Result<(), String> {
        let schema = &self.input_schema;
        if let Some(expected) = schema.get("type").and_then(Value::as_str) {
            validate_json_type(input, expected, "input")?;
        }

        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            for field in required {
                let key = field.as_str().unwrap_or("");
                if input.get(key).is_none() {
                    return Err(format!("missing required field: {key}"));
                }
            }
        }

        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (key, property_schema) in properties {
                let Some(value) = input.get(key) else {
                    continue;
                };
                let Some(expected) = property_schema.get("type").and_then(Value::as_str) else {
                    continue;
                };
                validate_json_type(value, expected, &format!("field `{key}`"))?;
            }
        }

        Ok(())
    }
}

fn validate_json_type(value: &Value, expected: &str, location: &str) -> Result<(), String> {
    let matches = match expected {
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "object" => value.is_object(),
        "string" => value.is_string(),
        _ => true,
    };

    if matches {
        Ok(())
    } else {
        Err(format!("{location} must be of type {expected}"))
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
    fn rejects_incorrect_required_field_type() {
        let schema = ToolSchema::simple("echo", "Echo a message", &["message"]);
        assert_eq!(
            schema.validate(&serde_json::json!({"message": 42})),
            Err("field `message` must be of type string".to_string())
        );
    }

    #[test]
    fn rejects_non_object_input() {
        let schema = ToolSchema::simple("echo", "Echo a message", &["message"]);
        assert_eq!(
            schema.validate(&serde_json::json!("hello")),
            Err("input must be of type object".to_string())
        );
    }

    #[test]
    fn validates_present_optional_fields_without_requiring_them() {
        let schema = ToolSchema {
            name: "grep".to_string(),
            description: "Search files".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "recursive": {"type": "boolean"}
                },
                "required": ["pattern"]
            }),
        };

        assert!(schema
            .validate(&serde_json::json!({"pattern": "needle"}))
            .is_ok());
        assert!(schema
            .validate(&serde_json::json!({"pattern": "needle", "recursive": true}))
            .is_ok());
        assert_eq!(
            schema.validate(&serde_json::json!({"pattern": "needle", "recursive": "yes"})),
            Err("field `recursive` must be of type boolean".to_string())
        );
    }
}
