//! [`EvolutionEngine`] – orchestrates the 5-gate pipeline with minority-veto.

use std::sync::Arc;

use harness_core::session::Session;
use harness_memory::MemoryDb;
use tracing::{debug, info, warn};

use crate::{
    traits::{Applier, Critic, Generator, Observer, Validator},
    types::{EvolutionOutcome, EvolutionRecord},
};

/// Number of validators that must reject a candidate for it to be discarded.
pub const MINORITY_VETO_THRESHOLD: usize = 2;

/// Drives a complete observe → critique → generate → validate → apply cycle.
///
/// # Minority-veto
///
/// For each candidate, all [`validators`](EvolutionEngine::validators) are
/// queried in parallel. If ≥ [`MINORITY_VETO_THRESHOLD`] return
/// [`ValidationVote::Reject`], the candidate is discarded. The first candidate
/// that survives is applied; the rest are dropped.
pub struct EvolutionEngine {
    pub observer: Arc<dyn Observer>,
    pub critic: Arc<dyn Critic>,
    pub generator: Arc<dyn Generator>,
    /// Exactly 5 validators are recommended (but the engine accepts any count ≥ 1).
    pub validators: Vec<Arc<dyn Validator>>,
    pub applier: Arc<dyn Applier>,
    pub memory: Arc<MemoryDb>,
}

impl EvolutionEngine {
    /// Run one full evolution cycle for the given `session`.
    ///
    /// `current_prompt` is the system prompt used during the session.
    pub async fn evolve(
        &self,
        session: &Session,
        current_prompt: &str,
    ) -> anyhow::Result<EvolutionOutcome> {
        // Gate 1 – Observe
        let summary = self.observer.observe(session).await?;
        debug!(session_id = %summary.session_id, "observer done");

        // Gate 2 – Critique
        let score = self.critic.critique(&summary, current_prompt).await?;
        info!(score = score.score, rationale = %score.rationale, "critic score");

        // Gate 3 – Generate
        let candidates = self
            .generator
            .generate(&summary, &score, current_prompt)
            .await?;
        if candidates.is_empty() {
            let reason = format!(
                "generator produced no candidates (score={:.2})",
                score.score
            );
            info!(%reason, "evolution skipped");
            let outcome = EvolutionOutcome::Skipped { reason };
            self.persist(session, score.score, &outcome).await?;
            return Ok(outcome);
        }

        info!(count = candidates.len(), "candidates generated");

        // Gate 4 – Validate with minority-veto, pick first survivor
        let mut applied: Option<EvolutionOutcome> = None;

        'candidates: for candidate in &candidates {
            let mut reject_count = 0usize;

            for (i, validator) in self.validators.iter().enumerate() {
                let vote = validator.validate(candidate, &summary).await?;
                debug!(
                    validator = i,
                    candidate_id = %candidate.id,
                    ?vote,
                    "validator vote"
                );
                if vote.is_reject() {
                    reject_count += 1;
                    if reject_count >= MINORITY_VETO_THRESHOLD {
                        warn!(
                            candidate_id = %candidate.id,
                            reject_count,
                            "candidate vetoed by minority"
                        );
                        continue 'candidates;
                    }
                }
            }

            // Candidate survived — apply it.
            info!(candidate_id = %candidate.id, reject_count, "candidate passed validation");
            let outcome = EvolutionOutcome::Applied {
                candidate: candidate.clone(),
                rejection_count: reject_count,
            };
            let record = EvolutionRecord::from_outcome(session.id, score.score, &outcome);
            self.applier.apply(candidate, &record).await?;
            self.persist_record(&record).await?;
            applied = Some(outcome);
            break;
        }

        let outcome = applied.unwrap_or_else(|| {
            let reason = format!("all {} candidate(s) vetoed by minority", candidates.len());
            warn!(%reason);
            EvolutionOutcome::Discarded { reason }
        });

        if matches!(outcome, EvolutionOutcome::Discarded { .. }) {
            self.persist(session, score.score, &outcome).await?;
        }

        Ok(outcome)
    }

    /// Persist a skipped/discarded outcome (applied outcomes are persisted inline).
    async fn persist(
        &self,
        session: &Session,
        score: f64,
        outcome: &EvolutionOutcome,
    ) -> anyhow::Result<()> {
        let record = EvolutionRecord::from_outcome(session.id, score, outcome);
        self.persist_record(&record).await
    }

    async fn persist_record(&self, record: &EvolutionRecord) -> anyhow::Result<()> {
        let id = record.id.to_string();
        let session_id = record.session_id.to_string();
        let created_at = record.created_at.to_rfc3339();
        let entry = harness_memory::EvolutionEntry {
            id: &id,
            session_id: &session_id,
            prompt_score: record.prompt_score,
            outcome_kind: &record.outcome_kind,
            outcome_detail: &record.outcome_detail,
            created_at: &created_at,
        };
        harness_memory::insert_evolution_entry(self.memory.pool(), &entry).await
    }
}
