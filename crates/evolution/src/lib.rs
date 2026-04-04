//! `harness-evolution` — Phantom-pattern 5-gate self-evolution engine.
//!
//! # Pipeline
//!
//! ```text
//! Session → Observer → Critic → Generator → [Validator×5 minority-veto] → Applier
//! ```
//!
//! # Quick start
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use harness_evolution::defaults::default_engine;
//! use harness_memory::MemoryDb;
//!
//! let memory = Arc::new(MemoryDb::in_memory().await?);
//! let engine = default_engine(Arc::clone(&memory));
//! let outcome = engine.evolve(&session, &current_prompt).await?;
//! ```

pub mod defaults;
pub mod engine;
pub mod traits;
pub mod types;

#[cfg(test)]
mod tests;

pub use engine::EvolutionEngine;
pub use types::{
    EvolutionOutcome, EvolutionRecord, PromptCandidate, PromptScore, SessionSummary, ValidationVote,
};
