//! Integration tests for the agent loop (separate crate — align with `harness-cli` lib clippy pragmas).
#![allow(
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::unnecessary_literal_bound,
    clippy::unwrap_used
)]

use std::sync::Arc;

use harness_core::{
    message::{ContentBlock, Message, MessageContent, Role, StopReason, TurnResponse, Usage},
    provider::Provider,
};
use harness_memory::MemoryDb;

pub struct ScriptedProvider {
    responses: std::sync::Mutex<Vec<TurnResponse>>,
}

impl ScriptedProvider {
    pub fn new(responses: Vec<TurnResponse>) -> Self {
        let mut r = responses;
        r.reverse();
        Self {
            responses: std::sync::Mutex::new(r),
        }
    }
}

#[async_trait::async_trait]
impl Provider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn complete(
        &self,
        _messages: &[Message],
    ) -> harness_core::error::Result<TurnResponse> {
        let mut guard = self.responses.lock().unwrap();
        Ok(guard.pop().expect("ScriptedProvider ran out of responses"))
    }
}

/// Records the first system message seen by `complete`, then behaves like [`ScriptedProvider`].
#[cfg(feature = "evolution")]
pub struct ScriptedProviderCaptureSystem {
    responses: std::sync::Mutex<Vec<TurnResponse>>,
    /// First system prompt text passed to `complete` (typically the opening turn).
    captured_system: std::sync::Mutex<Option<String>>,
}

#[cfg(feature = "evolution")]
impl ScriptedProviderCaptureSystem {
    pub fn new(responses: Vec<TurnResponse>) -> Self {
        let mut r = responses;
        r.reverse();
        Self {
            responses: std::sync::Mutex::new(r),
            captured_system: std::sync::Mutex::new(None),
        }
    }

    pub fn take_captured_system(&self) -> Option<String> {
        self.captured_system.lock().unwrap().take()
    }
}

#[cfg(feature = "evolution")]
#[async_trait::async_trait]
impl Provider for ScriptedProviderCaptureSystem {
    fn name(&self) -> &str {
        "scripted-capture"
    }

    async fn complete(
        &self,
        messages: &[Message],
    ) -> harness_core::error::Result<TurnResponse> {
        let mut cap = self.captured_system.lock().unwrap();
        if cap.is_none() {
            let sys = messages
                .iter()
                .find(|m| m.role == Role::System)
                .and_then(Message::text)
                .map(ToString::to_string);
            *cap = sys;
        }
        drop(cap);
        let mut guard = self.responses.lock().unwrap();
        Ok(guard.pop().expect("ScriptedProviderCaptureSystem ran out of responses"))
    }
}

pub fn tool_use_response(
    tool_use_id: &str,
    tool_name: &str,
    input: serde_json::Value,
) -> TurnResponse {
    TurnResponse {
        message: Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: tool_use_id.to_string(),
                name: tool_name.to_string(),
                input,
            }]),
        },
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "scripted".to_string(),
    }
}

pub fn end_turn_response(text: &str) -> TurnResponse {
    TurnResponse {
        message: Message {
            role: Role::Assistant,
            content: MessageContent::Text(text.to_string()),
        },
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "scripted".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_cli::agent::{Agent, RunOptions};
    use harness_core::config::Config;

    fn make_config(max_iterations: usize) -> Config {
        let mut cfg = Config::default();
        cfg.agent.max_iterations = max_iterations;
        cfg.agent.system_prompt = None;
        cfg
    }

    async fn make_memory() -> Arc<MemoryDb> {
        Arc::new(MemoryDb::in_memory().await.unwrap())
    }

    #[tokio::test]
    async fn e2e_tool_call_basic_completes() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            tool_use_response("t1", "echo", serde_json::json!({"message": "Cargo.toml\nREADME.md\nTaskfile.yml\nAGENTS.md\ncrates"})),
            end_turn_response("Here are the files:\nCargo.toml\nREADME.md\nTaskfile.yml\nAGENTS.md\ncrates"),
        ]));

        let memory = make_memory().await;
        let config = make_config(10);

        let agent = Agent::new(provider, memory, config);

        let session = agent.run_with_options(
            "List the files in the current directory",
            RunOptions {
                session_name: Some("e2e-tool-basic".to_string()),
                max_iterations: Some(10),
            },
        ).await.unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
        let final_text = session.messages.last().and_then(|m| m.text()).unwrap_or("");
        assert!(final_text.contains("Cargo.toml"), "expected Cargo.toml in output");
        assert!(final_text.contains("README.md"), "expected README.md in output");
    }

    #[tokio::test]
    async fn e2e_summarize_text_completes() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            tool_use_response("r1", "echo", serde_json::json!({"message": "# anvil\n\nA self-bootstrapping agent harness written in Rust."})),
            end_turn_response("Anvil is a Rust-based agent harness that bootstraps itself. It uses SQLite for episodic memory and supports tool calling."),
        ]));

        let memory = make_memory().await;
        let config = make_config(10);

        let agent = Agent::new(provider, memory, config);

        let session = agent.run_with_options(
            "Read the README.md and give me a 2-sentence summary",
            RunOptions {
                session_name: Some("e2e-summarize".to_string()),
                max_iterations: Some(10),
            },
        ).await.unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
        let final_text = session.messages.last().and_then(|m| m.text()).unwrap_or("");
        assert!(
            final_text.to_lowercase().contains("anvil") || final_text.to_lowercase().contains("agent"),
            "summary should mention project concepts"
        );
    }

    #[tokio::test]
    async fn e2e_multi_tool_task_completes() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            tool_use_response("b1", "echo", serde_json::json!({"message": "crates/core/src/provider.rs\ncrates/core/src/providers/claude.rs"})),
            end_turn_response("Found Provider trait in these files:\ncrates/core/src/provider.rs\ncrates/core/src/providers/claude.rs\n\nWriting to /tmp/bench_provider_files.txt"),
        ]));

        let memory = make_memory().await;
        let config = make_config(10);

        let agent = Agent::new(provider, memory, config);

        let session = agent.run_with_options(
            "Search for Provider trait and write results to /tmp/bench_provider_files.txt",
            RunOptions {
                session_name: Some("e2e-multi-tool".to_string()),
                max_iterations: Some(10),
            },
        ).await.unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);
    }

    /// With `evolution`, the post-session engine persists at least one `evolution_log` row per session
    /// (applied / skipped / discarded). Without this feature there is no hook and the table stays empty for the run.
    #[cfg(feature = "evolution")]
    #[tokio::test]
    async fn e2e_evolution_hook_writes_evolution_log() {
        use harness_memory::get_recent_evolution_log;

        let provider = Arc::new(ScriptedProvider::new(vec![
            tool_use_response("t1", "echo", serde_json::json!({"message": "x"})),
            end_turn_response("done"),
        ]));

        let memory = make_memory().await;
        let config = make_config(10);
        let agent = Agent::new(provider, Arc::clone(&memory), config);

        let session = agent
            .run_with_options(
                "e2e evolution log probe",
                RunOptions {
                    session_name: Some("e2e-evolution-log".to_string()),
                    max_iterations: Some(10),
                },
            )
            .await
            .unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);

        let sid = session.id.to_string();
        let logs = get_recent_evolution_log(memory.pool(), 50)
            .await
            .expect("read evolution_log");
        let mine: Vec<_> = logs.iter().filter(|r| r.session_id == sid).collect();
        assert!(
            !mine.is_empty(),
            "expected evolution_log row for session {}; got {} recent rows",
            sid,
            logs.len()
        );
        let kind = mine[0].outcome_kind.as_str();
        assert!(
            matches!(kind, "applied" | "discarded" | "skipped"),
            "unexpected outcome_kind: {kind}"
        );
    }

    #[cfg(not(feature = "evolution"))]
    #[tokio::test]
    async fn e2e_without_evolution_feature_no_evolution_log() {
        use harness_memory::get_recent_evolution_log;

        let provider = Arc::new(ScriptedProvider::new(vec![
            tool_use_response("t1", "echo", serde_json::json!({"message": "x"})),
            end_turn_response("done"),
        ]));

        let memory = make_memory().await;
        let config = make_config(10);
        let agent = Agent::new(provider, Arc::clone(&memory), config);

        let session = agent
            .run_with_options(
                "e2e no evolution feature",
                RunOptions {
                    session_name: Some("e2e-no-evo".to_string()),
                    max_iterations: Some(10),
                },
            )
            .await
            .unwrap();

        assert_eq!(session.status, harness_core::session::SessionStatus::Done);

        let sid = session.id.to_string();
        let logs = get_recent_evolution_log(memory.pool(), 50)
            .await
            .expect("read evolution_log");
        assert!(
            !logs.iter().any(|r| r.session_id == sid),
            "without evolution feature, expected no evolution_log rows for session {sid}"
        );
    }

    #[tokio::test]
    async fn e2e_session_persists_episodes_to_memory() {
        let provider = Arc::new(ScriptedProvider::new(vec![
            end_turn_response("the answer is 42"),
        ]));

        let memory = make_memory().await;
        let config = make_config(5);

        let agent = Agent::new(provider, Arc::clone(&memory), config);

        agent.run_with_options(
            "what is the meaning of life",
            RunOptions::default(),
        ).await.unwrap();

        let episodes = memory.search("meaning of life", 10).await.unwrap();
        assert!(
            !episodes.is_empty(),
            "session goal should be searchable in memory"
        );
    }

    /// Two-session flow: a multi-iteration first run triggers heuristic evolution (critic score < 0.75),
    /// persisting an active prompt version; the second run's opening system prompt includes
    /// `[LearnedOverlay]` whose body matches that version row in SQLite (same `session_id` as session 1).
    #[cfg(feature = "evolution")]
    #[tokio::test]
    async fn e2e_evolution_overlay_carried_into_second_session() {
        use super::ScriptedProviderCaptureSystem;
        use harness_memory::{get_prompt_version_by_id, resolve_effective_overlay, EvolutionScope, ScopeKind};

        fn learning_scope_for_test() -> EvolutionScope {
            let key = std::env::current_dir()
                .ok()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .map(|p| p.to_string_lossy().to_string());
            match key {
                Some(scope_key) => EvolutionScope {
                    kind: ScopeKind::Workdir,
                    key: Some(scope_key),
                },
                None => EvolutionScope::global(),
            }
        }

        // Three tool rounds + one end_turn → iteration_count == 4 → DefaultCritic efficiency 0.65 → generator runs.
        let provider1 = Arc::new(ScriptedProvider::new(vec![
            tool_use_response("t1", "echo", serde_json::json!({"message": "a"})),
            tool_use_response("t2", "echo", serde_json::json!({"message": "b"})),
            tool_use_response("t3", "echo", serde_json::json!({"message": "c"})),
            end_turn_response("done"),
        ]));

        let memory = make_memory().await;
        let config = make_config(10);

        let agent1 = Agent::new(provider1, Arc::clone(&memory), config.clone());
        let session1 = agent1
            .run_with_options(
                "alpha beta gamma delta evolution first session",
                RunOptions {
                    session_name: Some("e2e-evo-overlay-a".to_string()),
                    max_iterations: Some(10),
                },
            )
            .await
            .expect("first session");

        assert_eq!(session1.status, harness_core::session::SessionStatus::Done);

        let scope = learning_scope_for_test();
        let overlay = resolve_effective_overlay(memory.pool(), &scope)
            .await
            .expect("resolve overlay query");
        let effective = overlay.expect(
            "first session should persist an active learned overlay (4 iterations → critic below threshold)",
        );
        assert_eq!(
            effective.version.session_id,
            session1.id.to_string(),
            "active prompt version should reference the first session id"
        );

        let version = get_prompt_version_by_id(memory.pool(), &effective.version.id)
            .await
            .expect("get version by id")
            .expect("version row should exist");

        let capture = Arc::new(ScriptedProviderCaptureSystem::new(vec![end_turn_response(
            "second done",
        )]));
        let agent2 = Agent::new(Arc::clone(&capture) as Arc<dyn Provider>, memory, config);
        agent2
            .run_with_options(
                "epsilon zeta eta theta iota kappa distinct second goal",
                RunOptions {
                    session_name: Some("e2e-evo-overlay-b".to_string()),
                    max_iterations: Some(5),
                },
            )
            .await
            .expect("second session");

        let sys = capture
            .take_captured_system()
            .expect("provider should have seen a system message");

        assert!(
            sys.contains("[LearnedOverlay]"),
            "system prompt should include LearnedOverlay block; got len {}",
            sys.len()
        );
        assert!(
            sys.contains(version.candidate_prompt.as_str()),
            "overlay body should match DB candidate_prompt for version {}",
            version.id
        );
    }
}
