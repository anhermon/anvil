//! Internal event types that flow through the TUI's mpsc channel.

use chrono::{DateTime, Utc};
use crossterm::event::KeyEvent;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Events from the gateway WebSocket, matching harness-gateway's wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    TurnStart {
        id: Uuid,
        session_id: Uuid,
        ts: DateTime<Utc>,
    },
    Token {
        turn_id: Uuid,
        delta: String,
        ts: DateTime<Utc>,
    },
    ToolCall {
        turn_id: Uuid,
        tool_use_id: String,
        name: String,
        input: serde_json::Value,
        ts: DateTime<Utc>,
    },
    ToolResult {
        turn_id: Uuid,
        tool_use_id: String,
        content: String,
        ts: DateTime<Utc>,
    },
    TurnComplete {
        turn_id: Uuid,
        stop_reason: String,
        input_tokens: u32,
        output_tokens: u32,
        ts: DateTime<Utc>,
    },
    Error {
        turn_id: Option<Uuid>,
        message: String,
        ts: DateTime<Utc>,
    },
}

impl AgentEvent {
    /// Short label for display in the event list.
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            Self::TurnStart { .. } => "TURN_START",
            Self::Token { .. } => "TOKEN",
            Self::ToolCall { .. } => "TOOL_CALL",
            Self::ToolResult { .. } => "TOOL_RESULT",
            Self::TurnComplete { .. } => "TURN_COMPLETE",
            Self::Error { .. } => "ERROR",
        }
    }

    /// Human-readable one-line summary.
    pub fn summary(&self) -> String {
        match self {
            Self::TurnStart { session_id, .. } => {
                format!("session {}", &session_id.to_string()[..8])
            }
            Self::Token { delta, .. } => {
                let truncated = delta.chars().take(60).collect::<String>();
                if delta.len() > 60 {
                    format!("{}…", truncated)
                } else {
                    truncated
                }
            }
            Self::ToolCall { name, .. } => name.clone(),
            Self::ToolResult { tool_use_id, .. } => {
                format!("id={}", &tool_use_id[..tool_use_id.len().min(8)])
            }
            Self::TurnComplete {
                stop_reason,
                input_tokens,
                output_tokens,
                ..
            } => {
                format!(
                    "{} ({} in / {} out tokens)",
                    stop_reason, input_tokens, output_tokens
                )
            }
            Self::Error { message, .. } => message.clone(),
        }
    }

    /// Full detail text for the detail panel.
    pub fn detail(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "<serialisation error>".into())
    }

    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::TurnStart { ts, .. }
            | Self::Token { ts, .. }
            | Self::ToolCall { ts, .. }
            | Self::ToolResult { ts, .. }
            | Self::TurnComplete { ts, .. }
            | Self::Error { ts, .. } => *ts,
        }
    }
}

/// Top-level event type for the app's main loop.
pub enum AppEvent {
    /// Key press from the terminal (reserved for future channel-based input)
    #[allow(dead_code)]
    Key(KeyEvent),
    /// New agent event received from gateway
    Agent(AgentEvent),
    /// Gateway connection status changed
    GatewayStatus(GatewayStatus),
    /// Request to quit
    Quit,
}

/// Connection status for the status bar.
#[derive(Debug, Clone)]
pub enum GatewayStatus {
    Connecting,
    Connected,
    Disconnected { reason: String },
    Reconnecting { attempt: u32 },
}

impl std::fmt::Display for GatewayStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connecting => write!(f, "connecting…"),
            Self::Connected => write!(f, "connected"),
            Self::Disconnected { reason } => write!(f, "disconnected: {reason}"),
            Self::Reconnecting { attempt } => write!(f, "reconnecting (attempt {attempt})…"),
        }
    }
}
