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

/// Appended to the system prompt when evolution detects sandbox or policy friction.
/// Kept principle-based so the model discovers specifics from tool errors and schemas,
/// not hard-coded task answers.
const SANDBOX_DISCOVERY_HINT: &str = "\n\n\
When tools fail, read the error and your tool descriptions, then adapt—do not repeat the same disallowed pattern. \
Workspace file tools accept relative paths under the project; absolute paths and /tmp-style locations are rejected. \
For bash, the command must begin with an allowlisted program (see the bash tool description); use dedicated tools \
(`read`, `write`, `grep`, etc.) when they cover the job. Prefer narrowing search paths over brute-force repository scans.";

fn count_sandbox_tool_rejections(session: &Session) -> usize {
    use harness_core::message::{ContentBlock, MessageContent, Role};

    let mut n = 0;
    for msg in &session.messages {
        if msg.role != Role::Tool {
            continue;
        }
        let MessageContent::Blocks(blocks) = &msg.content else {
            continue;
        };
        for b in blocks {
            let ContentBlock::ToolResult { content, .. } = b else {
                continue;
            };
            if tool_result_suggests_sandbox_friction(content) {
                n += 1;
            }
        }
    }
    n
}

fn tool_result_suggests_sandbox_friction(content: &str) -> bool {
    let c = content.to_ascii_lowercase();
    [
        "absolute paths are not allowed",
        "path traversal",
        "command not allowed",
        "timed out after",
        "only [cargo,",
        "grep timed out",
    ]
    .into_iter()
    .any(|needle| c.contains(needle))
}

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

        let tool_rejection_count = count_sandbox_tool_rejections(session);

        Ok(SessionSummary {
            session_id: session.id,
            goal: session.goal.clone(),
            outcome,
            iteration_count: session.iteration,
            succeeded: session.status == harness_core::session::SessionStatus::Done,
            tool_call_count,
            tool_rejection_count,
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

        let mut score = (efficiency + prompt_bonus).min(1.0);

        // Any sandbox/policy tool friction should open room for a discovery-oriented overlay,
        // even when the run was short on iterations.
        if summary.tool_rejection_count > 0 {
            score = score.min(0.72);
        }

        let rationale = format!(
            "efficiency={efficiency:.2} (iterations={}), prompt_len={}, tool_rejections={}",
            summary.iteration_count,
            current_prompt.len(),
            summary.tool_rejection_count
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

        let sandbox_block = if summary.tool_rejection_count > 0 {
            SANDBOX_DISCOVERY_HINT
        } else {
            ""
        };

        let general_hint = if summary.iteration_count > 5 {
            "\n\nBe concise and minimize the number of turns needed to complete the task."
        } else {
            "\n\nWhen you have enough information, respond directly without asking unnecessary questions."
        };

        let description = if summary.tool_rejection_count > 0 {
            format!(
                "sandbox discovery + conciseness (score={:.2}, tool_errors={})",
                score.score, summary.tool_rejection_count
            )
        } else {
            format!("add conciseness hint (session score={:.2})", score.score)
        };

        let candidate = PromptCandidate {
            id: Uuid::new_v4(),
            prompt: format!("{current_prompt}{sandbox_block}{general_hint}"),
            description,
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
pub struct DefaultApplier {
    memory: Arc<MemoryDb>,
}

impl DefaultApplier {
    pub fn new(memory: Arc<MemoryDb>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Applier for DefaultApplier {
    async fn apply(
        &self,
        candidate: &PromptCandidate,
        record: &EvolutionRecord,
        scope: &crate::types::EvolutionScope,
        current_prompt: &str,
    ) -> anyhow::Result<()> {
        let base_prompt_hash = stable_prompt_hash(current_prompt);
        let candidate_diff = summarize_diff(current_prompt, &candidate.prompt);
        let version_id = candidate.id.to_string();
        let session_id = record.session_id.to_string();
        let created_at = record.created_at.to_rfc3339();
        let scope_key = scope.key.as_deref();
        let input = harness_memory::PromptVersionInput {
            id: &version_id,
            session_id: &session_id,
            scope_kind: &scope.kind,
            scope_key,
            base_prompt_hash: &base_prompt_hash,
            candidate_prompt: &candidate.prompt,
            candidate_diff: &candidate_diff,
            score_before: record.prompt_score,
            score_after: None,
            created_at: &created_at,
        };
        harness_memory::insert_prompt_version_and_activate(self.memory.pool(), &input).await
    }
}

fn stable_prompt_hash(input: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn summarize_diff(old_prompt: &str, new_prompt: &str) -> String {
    if new_prompt == old_prompt {
        return "no changes".to_string();
    }
    if let Some(suffix) = new_prompt.strip_prefix(old_prompt) {
        return format!("appended:\n{}", suffix.trim());
    }
    format!(
        "old_len={} new_len={}\n--- old ---\n{}\n--- new ---\n{}",
        old_prompt.len(),
        new_prompt.len(),
        old_prompt.lines().take(20).collect::<Vec<_>>().join("\n"),
        new_prompt.lines().take(20).collect::<Vec<_>>().join("\n")
    )
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
        applier: Arc::new(DefaultApplier::new(Arc::clone(&memory))),
        memory,
    }
}
