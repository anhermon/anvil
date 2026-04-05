//! Paperclip API response and request types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Agent identity returned by GET /api/agents/me
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentIdentity {
    pub id: String,
    pub company_id: String,
    pub name: String,
    pub role: String,
    pub status: String,
    pub budget_monthly_cents: i64,
    pub spent_monthly_cents: i64,
    pub url_key: String,
    #[serde(default)]
    pub chain_of_command: Vec<ChainMember>,
}

/// A single entry in the agent chain of command.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainMember {
    pub id: String,
    pub name: String,
    pub role: String,
}

/// Compact inbox item returned by GET /api/agents/me/inbox-lite
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub status: IssueStatus,
    pub priority: IssuePriority,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub active_run: Option<ActiveRun>,
}

/// Minimal active-run info embedded in an InboxItem.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveRun {
    pub id: String,
    pub status: String,
    pub agent_id: String,
}

/// Full issue object.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub status: IssueStatus,
    pub priority: IssuePriority,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub assignee_agent_id: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Issue comment.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub issue_id: String,
    pub body: String,
    #[serde(default)]
    pub author_agent_id: Option<String>,
    #[serde(default)]
    pub author_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Compact heartbeat context returned by GET /api/issues/{id}/heartbeat-context
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatContext {
    pub issue: Issue,
    #[serde(default)]
    pub ancestors: Vec<Issue>,
    pub comment_cursor: CommentCursor,
}

/// Comment cursor metadata in heartbeat context.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentCursor {
    pub total_comments: u32,
    #[serde(default)]
    pub latest_comment_id: Option<String>,
    #[serde(default)]
    pub latest_comment_at: Option<DateTime<Utc>>,
}

/// Issue status values.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Backlog,
    Todo,
    InProgress,
    InReview,
    Done,
    Blocked,
    Cancelled,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for IssueStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueStatus::Backlog => write!(f, "backlog"),
            IssueStatus::Todo => write!(f, "todo"),
            IssueStatus::InProgress => write!(f, "in_progress"),
            IssueStatus::InReview => write!(f, "in_review"),
            IssueStatus::Done => write!(f, "done"),
            IssueStatus::Blocked => write!(f, "blocked"),
            IssueStatus::Cancelled => write!(f, "cancelled"),
            IssueStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// Issue priority values.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssuePriority {
    Critical,
    High,
    Medium,
    Low,
    #[serde(other)]
    Unknown,
}

/// Request body for checkout.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutRequest {
    pub agent_id: String,
    pub expected_statuses: Vec<String>,
}

/// Request body for PATCH /api/issues/{id}.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIssueRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee_agent_id: Option<String>,
}

/// Request body for POST /api/issues/{id}/comments.
#[derive(Debug, Serialize)]
pub struct AddCommentRequest {
    pub body: String,
}

/// Request body for creating a new issue.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIssueRequest {
    pub title: String,
    pub description: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}
