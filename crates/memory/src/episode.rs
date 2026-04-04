use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Kind of memory episode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum EpisodeKind {
    /// A turn in the conversation (user or assistant message)
    Turn,
    /// A tool call and its result
    ToolCall,
    /// A distilled fact extracted from conversation
    Fact,
    /// A session summary
    Summary,
}

/// A single recorded memory episode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: Uuid,
    pub session_id: Uuid,
    pub kind: EpisodeKind,
    /// Role: "user", "assistant", "tool", "system"
    pub role: String,
    /// Content text
    pub content: String,
    /// Optional JSON metadata
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

impl Episode {
    pub fn turn(session_id: Uuid, role: &str, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            kind: EpisodeKind::Turn,
            role: role.to_string(),
            content: content.into(),
            metadata: None,
            created_at: Utc::now(),
        }
    }
}
