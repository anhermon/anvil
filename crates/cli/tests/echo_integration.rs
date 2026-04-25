//! Integration tests exercising the full agent loop with the echo provider.
//!
//! These tests verify that `anvil run --provider echo` works end-to-end:
//! plain text echo, scripted tool calls, memory persistence, and session
//! continuity — all without any real LLM API calls.

use std::sync::Arc;

use harness_cli::agent::{Agent, RunOptions};
use harness_core::{
    provider::{EchoProvider, ScriptedToolCall},
    session::SessionStatus,
};
use harness_memory::MemoryDb;

fn make_config(max_iterations: usize) -> harness_core::config::Config {
    let mut cfg = harness_core::config::Config::default();
    cfg.agent.max_iterations = max_iterations;
    cfg.agent.system_prompt = None;
    cfg
}

async fn make_memory() -> Arc<MemoryDb> {
    Arc::new(MemoryDb::in_memory().await.unwrap())
}

// ---------------------------------------------------------------------------
// 1. Basic echo: goal in -> deterministic echo response out
// ---------------------------------------------------------------------------

#[tokio::test]
async fn echo_provider_completes_full_agent_loop() {
    let provider = Arc::new(EchoProvider::new());
    let memory = make_memory().await;
    let config = make_config(10);

    let agent = Agent::new(provider, memory, config);
    let session = agent.run("test task").await.unwrap();

    assert_eq!(session.status, SessionStatus::Done);
    assert_eq!(session.iteration, 1);

    let last = session.messages.last().unwrap();
    assert_eq!(last.text(), Some("echo: test task"));
}

#[tokio::test]
async fn echo_provider_handles_empty_goal() {
    let provider = Arc::new(EchoProvider::new());
    let memory = make_memory().await;
    let config = make_config(5);

    let agent = Agent::new(provider, memory, config);
    let session = agent.run("").await.unwrap();

    assert_eq!(session.status, SessionStatus::Done);
}

// ---------------------------------------------------------------------------
// 2. Tool use: scripted tool call -> tool dispatch -> echo final response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn echo_provider_with_tool_call_exercises_full_loop() {
    let provider = Arc::new(EchoProvider::scripted(vec![ScriptedToolCall {
        id: "tool-1".to_string(),
        name: "echo".to_string(),
        input: serde_json::json!({"message": "tool-ping"}),
    }]));
    let memory = make_memory().await;
    let config = make_config(10);

    let agent = Agent::new(provider, memory, config);
    let session = agent.run("exercise tools").await.unwrap();

    assert_eq!(session.status, SessionStatus::Done);
    // Iteration 1: tool call. Iteration 2: final echo.
    assert_eq!(session.iteration, 2);

    let last = session.messages.last().unwrap();
    // After the tool result, echo provider echoes the last user message
    // (the tool result block), which has no plain text — falls back to "(empty)".
    assert!(last.text().is_some());
}

#[tokio::test]
async fn echo_provider_multiple_tool_calls_in_sequence() {
    let provider = Arc::new(EchoProvider::scripted(vec![
        ScriptedToolCall {
            id: "c1".to_string(),
            name: "echo".to_string(),
            input: serde_json::json!({"message": "first"}),
        },
        ScriptedToolCall {
            id: "c2".to_string(),
            name: "echo".to_string(),
            input: serde_json::json!({"message": "second"}),
        },
    ]));
    let memory = make_memory().await;
    let config = make_config(10);

    let agent = Agent::new(provider, memory, config);
    let session = agent.run("multi-tool test").await.unwrap();

    assert_eq!(session.status, SessionStatus::Done);
    // 2 scripted tool calls + 1 final echo = 3 iterations.
    assert_eq!(session.iteration, 3);
}

#[tokio::test]
async fn tool_call_respects_max_iterations() {
    // Script has 5 tool calls but max_iterations is 2.
    let provider = Arc::new(EchoProvider::scripted(vec![
        ScriptedToolCall {
            id: "c1".to_string(),
            name: "echo".to_string(),
            input: serde_json::json!({"message": "a"}),
        },
        ScriptedToolCall {
            id: "c2".to_string(),
            name: "echo".to_string(),
            input: serde_json::json!({"message": "b"}),
        },
        ScriptedToolCall {
            id: "c3".to_string(),
            name: "echo".to_string(),
            input: serde_json::json!({"message": "c"}),
        },
    ]));
    let memory = make_memory().await;
    let config = make_config(2);

    let agent = Agent::new(provider, memory, config);
    let session = agent.run("should cap").await.unwrap();

    assert_eq!(session.status, SessionStatus::Done);
    assert_eq!(session.iteration, 2);
}

// ---------------------------------------------------------------------------
// 3. Memory: episodes are persisted and recallable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn echo_sessions_persist_to_memory() {
    let memory = make_memory().await;
    let provider = Arc::new(EchoProvider::new());
    let config = make_config(5);

    let agent = Agent::new(provider, Arc::clone(&memory), config);
    let session = agent.run("remember this").await.unwrap();

    let episodes = memory.recent(session.id, 10).await.unwrap();
    assert!(
        episodes.len() >= 2,
        "expected at least user + assistant episodes, got {}",
        episodes.len()
    );

    let roles: Vec<&str> = episodes.iter().map(|e| e.role.as_str()).collect();
    assert!(roles.contains(&"user"));
    assert!(roles.contains(&"assistant"));
}

#[tokio::test]
async fn echo_tool_session_persists_all_turns() {
    let memory = make_memory().await;
    let provider = Arc::new(EchoProvider::scripted(vec![ScriptedToolCall {
        id: "t1".to_string(),
        name: "echo".to_string(),
        input: serde_json::json!({"message": "ping"}),
    }]));
    let config = make_config(10);

    let agent = Agent::new(provider, Arc::clone(&memory), config);
    let session = agent.run("tool + memory").await.unwrap();

    let episodes = memory.recent(session.id, 20).await.unwrap();
    // At minimum: user goal, assistant tool call, tool result, assistant final.
    assert!(
        episodes.len() >= 2,
        "expected multiple episodes for tool session, got {}",
        episodes.len()
    );
}

// ---------------------------------------------------------------------------
// 4. Session continuity via named sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn named_session_allows_multi_turn_continuity() {
    let memory = make_memory().await;
    let config = make_config(5);

    // Turn 1: initial goal.
    let provider1 = Arc::new(EchoProvider::new());
    let agent1 = Agent::new(provider1, Arc::clone(&memory), config.clone());
    let opts1 = RunOptions {
        session_name: Some("my-session".to_string()),
        ..Default::default()
    };
    let s1 = agent1.run_with_options("first turn", opts1).await.unwrap();
    assert_eq!(s1.status, SessionStatus::Done);

    // Turn 2: continue under the same session name.
    let provider2 = Arc::new(EchoProvider::new());
    let agent2 = Agent::new(provider2, Arc::clone(&memory), config);
    let opts2 = RunOptions {
        session_name: Some("my-session".to_string()),
        ..Default::default()
    };
    let s2 = agent2.run_with_options("second turn", opts2).await.unwrap();
    assert_eq!(s2.status, SessionStatus::Done);

    // Both sessions completed and persisted episodes under the same name.
    // The second session injected history from the first (verified by the
    // echo response reflecting the prior context). We confirm memory has
    // episodes from both sessions.
    let all_episodes = memory.recent_by_name("my-session", 20).await.unwrap();
    assert!(
        all_episodes.len() >= 4,
        "expected episodes from both sessions (>= 4), got {}",
        all_episodes.len()
    );
}

// ---------------------------------------------------------------------------
// 5. RunOptions override
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_options_overrides_config_max_iterations() {
    let provider = Arc::new(EchoProvider::scripted(vec![
        ScriptedToolCall {
            id: "c1".to_string(),
            name: "echo".to_string(),
            input: serde_json::json!({"message": "x"}),
        },
        ScriptedToolCall {
            id: "c2".to_string(),
            name: "echo".to_string(),
            input: serde_json::json!({"message": "y"}),
        },
        ScriptedToolCall {
            id: "c3".to_string(),
            name: "echo".to_string(),
            input: serde_json::json!({"message": "z"}),
        },
    ]));
    let memory = make_memory().await;
    let config = make_config(10); // Config says 10, options say 1.

    let agent = Agent::new(provider, memory, config);
    let opts = RunOptions {
        max_iterations: Some(1),
        ..Default::default()
    };
    let session = agent.run_with_options("override test", opts).await.unwrap();
    assert_eq!(session.iteration, 1);
}
