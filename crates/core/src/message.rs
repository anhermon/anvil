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

/// A structured content block (text, `tool_use`, `tool_result`).
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
    #[must_use]
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

impl TurnResponse {
    /// True when the turn carried no usable content at all — no text and no
    /// tool call.
    ///
    /// Distinguishes a provider-level flake (worth retrying) from a model that
    /// deliberately ended its turn with an answer.
    #[must_use]
    pub fn is_empty_turn(&self) -> bool {
        match &self.message.content {
            MessageContent::Text(t) => t.trim().is_empty(),
            MessageContent::Blocks(blocks) => !blocks.iter().any(|b| match b {
                ContentBlock::Text { text } => !text.trim().is_empty(),
                ContentBlock::ToolUse { .. } => true,
                ContentBlock::ToolResult { .. } => false,
            }),
        }
    }
}

#[cfg(test)]
mod turn_response_tests {
    use super::{ContentBlock, Message, MessageContent, Role, StopReason, TurnResponse, Usage};

    fn resp(content: MessageContent) -> TurnResponse {
        TurnResponse {
            message: Message {
                role: Role::Assistant,
                content,
            },
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: "test".to_string(),
        }
    }

    #[test]
    fn no_blocks_is_empty() {
        assert!(resp(MessageContent::Blocks(vec![])).is_empty_turn());
    }

    #[test]
    fn whitespace_only_text_is_empty() {
        assert!(resp(MessageContent::Text("   \n".into())).is_empty_turn());
        assert!(resp(MessageContent::Blocks(vec![ContentBlock::Text {
            text: "  ".into()
        }]))
        .is_empty_turn());
    }

    #[test]
    fn real_text_is_not_empty() {
        assert!(!resp(MessageContent::Text("hello".into())).is_empty_turn());
    }

    #[test]
    fn tool_call_alone_is_not_empty() {
        assert!(!resp(MessageContent::Blocks(vec![ContentBlock::ToolUse {
            id: "1".into(),
            name: "bash".into(),
            input: serde_json::json!({}),
        }]))
        .is_empty_turn());
    }
}
