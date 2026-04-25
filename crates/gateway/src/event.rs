#![allow(clippy::module_name_repetitions)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Events emitted by the agent and broadcast to WebSocket clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A new agent turn has started.
    TurnStart {
        id: Uuid,
        session_id: Uuid,
        ts: DateTime<Utc>,
    },
    /// A streaming text token from the LLM.
    Token {
        turn_id: Uuid,
        delta: String,
        ts: DateTime<Utc>,
    },
    /// The agent is calling a tool.
    ToolCall {
        turn_id: Uuid,
        tool_use_id: String,
        name: String,
        input: serde_json::Value,
        ts: DateTime<Utc>,
    },
    /// A tool has returned its result.
    ToolResult {
        turn_id: Uuid,
        tool_use_id: String,
        content: String,
        ts: DateTime<Utc>,
    },
    /// The agent turn completed.
    TurnComplete {
        turn_id: Uuid,
        stop_reason: String,
        input_tokens: u32,
        output_tokens: u32,
        ts: DateTime<Utc>,
    },
    /// An unrecoverable error occurred.
    Error {
        turn_id: Option<Uuid>,
        message: String,
        ts: DateTime<Utc>,
    },
}

impl AgentEvent {
    /// Convenience constructor: streaming text token.
    pub fn token(delta: impl Into<String>) -> Self {
        Self::Token {
            turn_id: Uuid::nil(),
            delta: delta.into(),
            ts: Utc::now(),
        }
    }

    /// Convenience constructor: turn started.
    pub fn turn_start(session_id: Uuid) -> Self {
        Self::TurnStart {
            id: Uuid::new_v4(),
            session_id,
            ts: Utc::now(),
        }
    }

    /// Convenience constructor: error event.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            turn_id: None,
            message: message.into(),
            ts: Utc::now(),
        }
    }
}

/// Commands that remote clients can send to control the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum ControlCommand {
    /// Ask the agent to stop cleanly after the current turn.
    Interrupt,
    /// Pause execution after the current tool call completes.
    Pause,
    /// Resume a paused agent.
    Resume,
    /// No-op ping.
    Ping,
}
