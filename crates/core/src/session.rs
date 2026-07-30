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
    /// The loop hit its iteration cap before the agent ended its turn. The goal
    /// was *not* reported complete — this is an unambiguous, harness-observable
    /// failure.
    MaxIterations,
    Failed,
    Cancelled,
}

impl SessionStatus {
    /// Stable lowercase name, used as the machine-readable `outcome` field.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::MaxIterations => "max_iterations",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Process exit code for a finished run.
    ///
    /// `0` only when the agent ended its own turn (`Done`). Note this says the
    /// agent *finished*, not that the goal was actually achieved — the harness
    /// cannot verify the latter.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        if *self == Self::Done {
            0
        } else {
            2
        }
    }
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

    #[must_use]
    pub fn is_done(&self) -> bool {
        self.status != SessionStatus::Running
    }
}
