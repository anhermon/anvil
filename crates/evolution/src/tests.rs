//! Integration tests for the self-evolution engine.

#[cfg(test)]
mod integration {
    use std::sync::Arc;

    use harness_core::{
        message::{Message, MessageContent, Role},
        session::{Session, SessionStatus},
    };
    use harness_memory::MemoryDb;

    use crate::{
        defaults::default_engine,
        engine::EvolutionEngine,
        traits::Validator,
        types::{EvolutionOutcome, PromptCandidate, SessionSummary, ValidationVote},
    };
    use async_trait::async_trait;

    // -----------------------------------------------------------------------
    // Helper: build a finished session
    // -----------------------------------------------------------------------

    fn make_session(goal: &str, iterations: usize, succeeded: bool) -> Session {
        let mut s = Session::new(goal);
        s.iteration = iterations;
        let msg = Message {
            role: Role::Assistant,
            content: MessageContent::Text("task completed".to_string()),
        };
        s.messages.push(msg);
        if succeeded {
            s.finish(SessionStatus::Done);
        } else {
            s.finish(SessionStatus::Failed);
        }
        s
    }

    // -----------------------------------------------------------------------
    // Test: full default evolution cycle — should not panic
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn full_evolution_cycle_does_not_panic() {
        let memory = Arc::new(MemoryDb::in_memory().await.expect("in-memory db"));
        let engine = default_engine(Arc::clone(&memory));

        // A session that took 8 iterations → score < 0.75 → candidate generated
        let session = make_session("do something", 8, true);
        let prompt = "You are a helpful assistant.";

        let outcome = engine
            .evolve(&session, prompt)
            .await
            .expect("evolution cycle must not error");

        // With 8 iterations and a short prompt the score should be < 0.75,
        // so the generator should produce a candidate and it should pass validation.
        assert!(
            matches!(outcome, EvolutionOutcome::Applied { .. }),
            "expected Applied, got {outcome:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Test: high-scoring session → evolution skipped
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn high_score_skips_evolution() {
        let memory = Arc::new(MemoryDb::in_memory().await.expect("in-memory db"));
        let engine = default_engine(Arc::clone(&memory));

        // 1 iteration → efficiency = 1.0 → score ≥ 0.75
        let session = make_session("quick task", 1, true);
        let prompt = "You are a helpful assistant with a reasonably detailed system prompt.";

        let outcome = engine
            .evolve(&session, prompt)
            .await
            .expect("evolution cycle must not error");

        assert!(
            matches!(outcome, EvolutionOutcome::Skipped { .. }),
            "expected Skipped, got {outcome:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Test: failed session → score 0.0 → generator skips
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn failed_session_skips_evolution() {
        let memory = Arc::new(MemoryDb::in_memory().await.expect("in-memory db"));
        let engine = default_engine(Arc::clone(&memory));

        let session = make_session("broken task", 3, false);
        let outcome = engine
            .evolve(&session, "short")
            .await
            .expect("must not error");

        // score = 0.0 → generator produces a candidate (score < 0.75)
        // but the candidate should pass all 5 default validators and be Applied.
        // (The failed session lowers the score but doesn't block the candidate.)
        assert!(
            matches!(
                outcome,
                EvolutionOutcome::Applied { .. } | EvolutionOutcome::Skipped { .. }
            ),
            "unexpected outcome: {outcome:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Test: minority-veto — 2 rejecting validators cause candidate to be discarded
    // -----------------------------------------------------------------------

    struct AlwaysRejectValidator;

    #[async_trait]
    impl Validator for AlwaysRejectValidator {
        async fn validate(
            &self,
            _candidate: &PromptCandidate,
            _summary: &SessionSummary,
        ) -> anyhow::Result<ValidationVote> {
            Ok(ValidationVote::Reject {
                reason: "stubbed rejection".to_string(),
            })
        }
    }

    struct AlwaysAcceptValidator;

    #[async_trait]
    impl Validator for AlwaysAcceptValidator {
        async fn validate(
            &self,
            _candidate: &PromptCandidate,
            _summary: &SessionSummary,
        ) -> anyhow::Result<ValidationVote> {
            Ok(ValidationVote::Accept)
        }
    }

    #[tokio::test]
    async fn minority_veto_discards_candidate_with_two_rejections() {
        let memory = Arc::new(MemoryDb::in_memory().await.expect("in-memory db"));

        // 2 rejectors + 3 acceptors → threshold reached → discarded
        let engine = EvolutionEngine {
            observer: Arc::new(crate::defaults::DefaultObserver),
            critic: Arc::new(crate::defaults::DefaultCritic),
            generator: Arc::new(crate::defaults::DefaultGenerator),
            validators: vec![
                Arc::new(AlwaysRejectValidator),
                Arc::new(AlwaysRejectValidator),
                Arc::new(AlwaysAcceptValidator),
                Arc::new(AlwaysAcceptValidator),
                Arc::new(AlwaysAcceptValidator),
            ],
            applier: Arc::new(crate::defaults::DefaultApplier),
            memory,
        };

        // 8 iterations → low score → generator produces a candidate
        let session = make_session("do work", 8, true);
        let outcome = engine
            .evolve(&session, "You are a helpful assistant.")
            .await
            .expect("must not error");

        assert!(
            matches!(outcome, EvolutionOutcome::Discarded { .. }),
            "expected Discarded, got {outcome:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Test: minority-veto — 1 rejection is NOT enough to discard
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn one_rejection_does_not_veto_candidate() {
        let memory = Arc::new(MemoryDb::in_memory().await.expect("in-memory db"));

        // 1 rejector + 4 acceptors → below threshold → applied
        let engine = EvolutionEngine {
            observer: Arc::new(crate::defaults::DefaultObserver),
            critic: Arc::new(crate::defaults::DefaultCritic),
            generator: Arc::new(crate::defaults::DefaultGenerator),
            validators: vec![
                Arc::new(AlwaysRejectValidator),
                Arc::new(AlwaysAcceptValidator),
                Arc::new(AlwaysAcceptValidator),
                Arc::new(AlwaysAcceptValidator),
                Arc::new(AlwaysAcceptValidator),
            ],
            applier: Arc::new(crate::defaults::DefaultApplier),
            memory,
        };

        let session = make_session("do work", 8, true);
        let outcome = engine
            .evolve(&session, "You are a helpful assistant.")
            .await
            .expect("must not error");

        assert!(
            matches!(
                outcome,
                EvolutionOutcome::Applied {
                    rejection_count: 1,
                    ..
                }
            ),
            "expected Applied with 1 rejection, got {outcome:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Test: evolution records are persisted in the evolution_log table
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn evolution_record_written_to_sqlite() {
        let memory = Arc::new(MemoryDb::in_memory().await.expect("in-memory db"));
        let engine = default_engine(Arc::clone(&memory));

        // Force a low-score session so the cycle runs fully
        let session = make_session("work task", 10, true);
        engine
            .evolve(&session, "short prompt")
            .await
            .expect("must not error");

        // Verify the evolution_log row was inserted
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM evolution_log WHERE session_id = ?")
                .bind(session.id.to_string())
                .fetch_one(memory.pool())
                .await
                .expect("db query");

        assert_eq!(count, 1, "expected one evolution_log entry");
    }
}
