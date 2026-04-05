//! The five trait gates of the self-evolution pipeline.
//!
//! Pipeline flow:
//!
//! ```text
//! Session  →  Observer  →  SessionSummary
//!                          ↓
//!                        Critic  →  PromptScore
//!                                   ↓
//!                                 Generator  →  Vec<PromptCandidate>
//!                                               ↓ (per candidate, 5× Validator)
//!                                             [minority-veto]
//!                                               ↓ (survivors)
//!                                             Applier  →  EvolutionRecord
//! ```

use async_trait::async_trait;
use harness_core::session::Session;

use crate::types::{EvolutionRecord, PromptCandidate, PromptScore, SessionSummary, ValidationVote};

/// **Gate 1 – Observer**: examine a completed session and extract a summary.
#[async_trait]
pub trait Observer: Send + Sync + 'static {
    async fn observe(&self, session: &Session) -> anyhow::Result<SessionSummary>;
}

/// **Gate 2 – Critic**: score the current system prompt given a session summary.
#[async_trait]
pub trait Critic: Send + Sync + 'static {
    async fn critique(
        &self,
        summary: &SessionSummary,
        current_prompt: &str,
    ) -> anyhow::Result<PromptScore>;
}

/// **Gate 3 – Generator**: propose candidate prompt improvements.
///
/// Returns an empty `Vec` when no improvements are warranted.
#[async_trait]
pub trait Generator: Send + Sync + 'static {
    async fn generate(
        &self,
        summary: &SessionSummary,
        score: &PromptScore,
        current_prompt: &str,
    ) -> anyhow::Result<Vec<PromptCandidate>>;
}

/// **Gate 4 – Validator**: cast a single accept/reject vote on a candidate.
///
/// The engine calls this trait (possibly multiple instances) and applies the
/// minority-veto rule: if ≥ `MINORITY_VETO_THRESHOLD` (default 2) validators
/// reject a candidate it is discarded.
#[async_trait]
pub trait Validator: Send + Sync + 'static {
    async fn validate(
        &self,
        candidate: &PromptCandidate,
        summary: &SessionSummary,
    ) -> anyhow::Result<ValidationVote>;
}

/// **Gate 5 – Applier**: persist or apply a validated candidate.
///
/// The default implementation writes to the evolution SQLite log.
/// Custom implementations may also patch the live config file.
#[async_trait]
pub trait Applier: Send + Sync + 'static {
    async fn apply(
        &self,
        candidate: &PromptCandidate,
        record: &EvolutionRecord,
    ) -> anyhow::Result<()>;
}
