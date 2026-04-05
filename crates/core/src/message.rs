use serde::{Deserialize, Serialize};

/// Role of a conversation participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A single message in the conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

/// Message content — either plain text or structured blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// A structured content block (text, tool_use, tool_result).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: MessageContent::Text(text.into()),
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: MessageContent::Text(text.into()),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: MessageContent::Text(text.into()),
        }
    }

    /// Extract plain text from any content variant.
    pub fn text(&self) -> Option<&str> {
        match &self.content {
            MessageContent::Text(s) => Some(s.as_str()),
            MessageContent::Blocks(blocks) => blocks.iter().find_map(|b| {
                if let ContentBlock::Text { text } = b {
                    Some(text.as_str())
                } else {
                    None
                }
            }),
        }
    }
}

/// Token usage reported by the provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: Option<u32>,
    pub cache_write_tokens: Option<u32>,
}

/// Stop reason returned by the provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    StopSequence,
}

/// Complete response from a provider turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResponse {
    pub message: Message,
    pub stop_reason: StopReason,
    pub usage: Usage,
    pub model: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_message_constructors() {
        let sys = Message::system("hello");
        assert_eq!(sys.role, Role::System);
        assert_eq!(sys.text(), Some("hello"));

        let usr = Message::user("question");
        assert_eq!(usr.role, Role::User);
        assert_eq!(usr.text(), Some("question"));

        let asst = Message::assistant("answer");
        assert_eq!(asst.role, Role::Assistant);
        assert_eq!(asst.text(), Some("answer"));
    }

    #[test]
    fn text_extracts_from_blocks() {
        let msg = Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({}),
                },
                ContentBlock::Text {
                    text: "result text".into(),
                },
            ]),
        };
        assert_eq!(msg.text(), Some("result text"));
    }

    #[test]
    fn text_returns_none_for_blocks_without_text() {
        let msg = Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: "output".into(),
            }]),
        };
        assert_eq!(msg.text(), None);
    }

    #[test]
    fn role_serde_roundtrip() {
        let json = serde_json::to_string(&Role::System).unwrap();
        assert_eq!(json, "\"system\"");
        let back: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Role::System);
    }

    #[test]
    fn message_serde_roundtrip() {
        let msg = Message::user("hello");
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.text(), Some("hello"));
    }
}
