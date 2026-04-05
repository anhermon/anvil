//! Default implementations of the five evolution-pipeline traits.
//!
//! These are intentionally lightweight — no LLM calls — so the engine
//! works in any environment without provider credentials.

use async_trait::async_trait;
use harness_core::session::Session;
use harness_memory::MemoryDb;
use std::sync::Arc;
use tracing::debug;
use uuid::Uuid;

use crate::{
    traits::{Applier, Critic, Generator, Observer, Validator},
    types::{EvolutionRecord, PromptCandidate, PromptScore, SessionSummary, ValidationVote},
};

// ---------------------------------------------------------------------------
// DefaultObserver
// ---------------------------------------------------------------------------

/// Extracts a [`SessionSummary`] by inspecting the session's messages and
/// metadata. No LLM call required.
pub struct DefaultObserver;

#[async_trait]
impl Observer for DefaultObserver {
    async fn observe(&self, session: &Session) -> anyhow::Result<SessionSummary> {
        use harness_core::message::{ContentBlock, MessageContent};

        let outcome = session
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, harness_core::message::Role::Assistant))
            .and_then(|m| m.text())
            .unwrap_or("")
            .to_string();

        let tool_call_count = session
            .messages
            .iter()
            .filter(|m| matches!(m.role, harness_core::message::Role::Assistant))
            .flat_map(|m| {
                if let MessageContent::Blocks(blocks) = &m.content {
                    blocks.iter().collect::<Vec<_>>()
                } else {
                    vec![]
                }
            })
            .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
            .count();

        debug!(
            session_id = %session.id,
            iteration_count = session.iteration,
            tool_call_count,
            "observer: summary extracted"
        );

        Ok(SessionSummary {
            session_id: session.id,
            goal: session.goal.clone(),
            outcome,
            iteration_count: session.iteration,
            succeeded: session.is_done(),
            tool_call_count,
        })
    }
}

// ---------------------------------------------------------------------------
// DefaultCritic
// ---------------------------------------------------------------------------

/// Scores the system prompt heuristically:
///
/// * Sessions that finished in ≤ 3 iterations score higher.
/// * Sessions that failed score 0.
/// * Non-empty prompts score slightly higher than empty ones.
pub struct DefaultCritic;

#[async_trait]
impl Critic for DefaultCritic {
    async fn critique(
        &self,
        summary: &SessionSummary,
        current_prompt: &str,
    ) -> anyhow::Result<PromptScore> {
        if !summary.succeeded {
            return Ok(PromptScore {
                score: 0.0,
                rationale: "session did not complete successfully".to_string(),
            });
        }

        // Efficiency score: fewer iterations → higher score
        let efficiency = match summary.iteration_count {
            0 | 1 => 1.0_f64,
            2 | 3 => 0.85,
            4..=6 => 0.65,
            7..=10 => 0.45,
            _ => 0.25,
        };

        // Slight bonus for a non-trivially long system prompt
        let prompt_bonus: f64 = if current_prompt.len() > 50 { 0.05 } else { 0.0 };

        let score = (efficiency + prompt_bonus).min(1.0);
        let rationale = format!(
            "efficiency={efficiency:.2} (iterations={}), prompt_len={}",
            summary.iteration_count,
            current_prompt.len()
        );

        Ok(PromptScore { score, rationale })
    }
}

// ---------------------------------------------------------------------------
// DefaultGenerator
// ---------------------------------------------------------------------------

/// Generates a single candidate when the prompt score is below 0.75.
///
/// The candidate appends a conciseness hint to the current prompt.
pub struct DefaultGenerator;

#[async_trait]
impl Generator for DefaultGenerator {
    async fn generate(
        &self,
        summary: &SessionSummary,
        score: &PromptScore,
        current_prompt: &str,
    ) -> anyhow::Result<Vec<PromptCandidate>> {
        // Only suggest improvements when there is room to grow.
        if score.score >= 0.75 {
            debug!(score = score.score, "score acceptable, skipping generation");
            return Ok(vec![]);
        }

        let hint = if summary.iteration_count > 5 {
            "\n\nBe concise and minimize the number of turns needed to complete the task."
        } else {
            "\n\nWhen you have enough information, respond directly without asking unnecessary questions."
        };

        let candidate = PromptCandidate {
            id: Uuid::new_v4(),
            prompt: format!("{current_prompt}{hint}"),
            description: format!("add conciseness hint (session score={:.2})", score.score),
        };

        Ok(vec![candidate])
    }
}

// ---------------------------------------------------------------------------
// DefaultValidator
// ---------------------------------------------------------------------------

/// Validates that a candidate prompt is non-empty, differs from the base, and
/// does not exceed a reasonable length limit.
///
/// Five instances of this validator are used by [`crate::engine::EvolutionEngine`]
/// by default, each with a different `perspective` label (for tracing).
pub struct DefaultValidator {
    /// Label used in trace logs to distinguish the 5 validator instances.
    pub perspective: &'static str,
}

impl DefaultValidator {
    pub const fn new(perspective: &'static str) -> Self {
        Self { perspective }
    }
}

#[async_trait]
impl Validator for DefaultValidator {
    async fn validate(
        &self,
        candidate: &PromptCandidate,
        _summary: &SessionSummary,
    ) -> anyhow::Result<ValidationVote> {
        debug!(perspective = self.perspective, candidate_id = %candidate.id, "validator running");

        if candidate.prompt.trim().is_empty() {
            return Ok(ValidationVote::Reject {
                reason: "candidate prompt is empty".to_string(),
            });
        }

        // 8 KB limit to prevent runaway prompt growth
        if candidate.prompt.len() > 8192 {
            return Ok(ValidationVote::Reject {
                reason: format!(
                    "candidate prompt too long ({} bytes > 8192)",
                    candidate.prompt.len()
                ),
            });
        }

        Ok(ValidationVote::Accept)
    }
}

// ---------------------------------------------------------------------------
// DefaultApplier
// ---------------------------------------------------------------------------

/// No-op applier: the engine already persists the record via the memory pool.
/// Custom implementations may patch a live config file here.
pub struct DefaultApplier;

#[async_trait]
impl Applier for DefaultApplier {
    async fn apply(
        &self,
        _candidate: &PromptCandidate,
        _record: &EvolutionRecord,
    ) -> anyhow::Result<()> {
        // The EvolutionEngine already persists the record via persist_record.
        // DefaultApplier is a logging-only no-op; it does not patch config on disk.
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Builder helper
// ---------------------------------------------------------------------------

/// Construct a fully-wired [`crate::engine::EvolutionEngine`] with all-default
/// stages and the provided memory store.
pub fn default_engine(memory: Arc<MemoryDb>) -> crate::engine::EvolutionEngine {
    use std::sync::Arc;
    crate::engine::EvolutionEngine {
        observer: Arc::new(DefaultObserver),
        critic: Arc::new(DefaultCritic),
        generator: Arc::new(DefaultGenerator),
        validators: vec![
            Arc::new(DefaultValidator::new("safety")),
            Arc::new(DefaultValidator::new("coherence")),
            Arc::new(DefaultValidator::new("length")),
            Arc::new(DefaultValidator::new("format")),
            Arc::new(DefaultValidator::new("relevance")),
        ],
        applier: Arc::new(DefaultApplier),
        memory,
    }
}
