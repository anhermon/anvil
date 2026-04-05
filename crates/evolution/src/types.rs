//! Shared data types flowing through the evolution pipeline.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Summary of a completed session produced by the Observer stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: Uuid,
    pub goal: String,
    /// Full text of the final assistant message, if any.
    pub outcome: String,
    /// How many iterations the session ran.
    pub iteration_count: usize,
    /// Whether the session completed successfully (status == Done).
    pub succeeded: bool,
    /// Number of tool calls made during the session.
    pub tool_call_count: usize,
}

/// Numeric quality score for the current system prompt produced by the Critic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptScore {
    /// 0.0 (worst) – 1.0 (best).
    pub score: f64,
    /// Human-readable rationale for the score.
    pub rationale: String,
}

/// A candidate replacement / patch for the system prompt produced by the Generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptCandidate {
    pub id: Uuid,
    /// The full proposed system prompt text.
    pub prompt: String,
    /// Short description of the intended improvement.
    pub description: String,
}

/// One validator's verdict on a candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationVote {
    Accept,
    Reject { reason: String },
}

impl ValidationVote {
    pub fn is_reject(&self) -> bool {
        matches!(self, Self::Reject { .. })
    }
}

/// The final outcome of one evolution cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvolutionOutcome {
    /// A candidate survived minority-veto validation and was applied.
    Applied {
        candidate: PromptCandidate,
        rejection_count: usize,
    },
    /// Every candidate was discarded by minority-veto.
    Discarded { reason: String },
    /// No candidates were generated (score already high / session failed).
    Skipped { reason: String },
}

/// Persisted record of one full evolution cycle, stored in the evolution log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionRecord {
    pub id: Uuid,
    pub session_id: Uuid,
    pub prompt_score: f64,
    pub outcome_kind: String, // "applied" | "discarded" | "skipped"
    pub outcome_detail: String,
    pub created_at: DateTime<Utc>,
}

impl EvolutionRecord {
    pub fn from_outcome(session_id: Uuid, score: f64, outcome: &EvolutionOutcome) -> Self {
        let (kind, detail) = match outcome {
            EvolutionOutcome::Applied {
                candidate,
                rejection_count,
            } => (
                "applied".to_string(),
                format!(
                    "Applied candidate '{}' ({} rejections)",
                    candidate.description, rejection_count
                ),
            ),
            EvolutionOutcome::Discarded { reason } => ("discarded".to_string(), reason.clone()),
            EvolutionOutcome::Skipped { reason } => ("skipped".to_string(), reason.clone()),
        };
        Self {
            id: Uuid::new_v4(),
            session_id,
            prompt_score: score,
            outcome_kind: kind,
            outcome_detail: detail,
            created_at: Utc::now(),
        }
    }
}
