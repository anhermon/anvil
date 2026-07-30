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
    /// The agent ended its turn believing it was done, but the operator-supplied
    /// `--verify` command exited non-zero. The claim of success was not backed by
    /// ground truth.
    VerificationFailed,
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
            Self::VerificationFailed => "verification_failed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Process exit code for a finished run.
    ///
    /// `0` only when the agent ended its own turn (`Done`) *and* any `--verify`
    /// command passed. Without `--verify`, `Done` says the agent *finished*, not
    /// that the goal was achieved — the harness has no ground truth for that.
    ///
    /// `3` is reserved for `VerificationFailed` so a caller can distinguish
    /// "the agent never finished" (`2`) from "it finished and was wrong" (`3`).
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Done => 0,
            Self::VerificationFailed => 3,
            _ => 2,
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
