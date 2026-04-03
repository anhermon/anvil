use crate::message::Message;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single agent run session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub goal: String,
    pub messages: Vec<Message>,
    pub iteration: usize,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl Session {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            goal: goal.into(),
            messages: Vec::new(),
            iteration: 0,
            started_at: Utc::now(),
            finished_at: None,
            status: SessionStatus::Running,
        }
    }

    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
    }

    pub fn finish(&mut self, status: SessionStatus) {
        self.finished_at = Some(Utc::now());
        self.status = status;
    }

    pub fn is_done(&self) -> bool {
        self.status != SessionStatus::Running
    }
}
