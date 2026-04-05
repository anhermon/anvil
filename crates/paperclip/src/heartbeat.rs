//! Heartbeat loop — the core execution cycle for a Paperclip-connected agent.
//!
//! Follows the 9-step heartbeat procedure:
//! 1. Identity check
//! 2. (approval follow-up — delegated to executor)
//! 3. Get inbox
//! 4. Pick work (in_progress first, then todo)
//! 5. Checkout
//! 6. Get context
//! 7. Execute (delegated to [`TaskExecutor`])
//! 8. Update status + comment
//! 9. (delegate / create subtasks — executor responsibility)

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::{info, warn};

use crate::{
    client::PaperclipClient,
    types::{HeartbeatContext, InboxItem, IssueStatus},
};

/// Outcome of a single task execution.
#[derive(Debug)]
pub enum ExecutionOutcome {
    /// Task completed successfully.  The string is the final comment body.
    Done(String),
    /// Task is blocked.  The string explains the blocker (who needs to act).
    Blocked(String),
    /// Task needs human review.  String is the comment body.
    InReview(String),
    /// Execution deferred — no changes were made, leave status as-is.
    Deferred,
}

/// Trait implemented by callers to provide task execution logic.
///
/// The heartbeat loop handles all API ceremony (checkout, status update,
/// comment posting).  The executor only needs to perform the actual work
/// and return an [`ExecutionOutcome`].
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    /// Execute a task.
    ///
    /// `item` — the inbox item selected for this heartbeat
    /// `context` — full heartbeat context (issue + ancestors + cursor)
    async fn execute(
        &self,
        item: &InboxItem,
        context: &HeartbeatContext,
    ) -> Result<ExecutionOutcome>;
}

/// Configuration for the heartbeat loop.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Paperclip agent ID this loop runs as.
    pub agent_id: String,
    /// Paperclip company ID.
    pub company_id: String,
    /// How many tasks to process per wake (default: 1).
    pub max_tasks_per_wake: usize,
}

impl HeartbeatConfig {
    pub fn new(agent_id: String, company_id: String) -> Self {
        Self {
            agent_id,
            company_id,
            max_tasks_per_wake: 1,
        }
    }
}

/// The heartbeat loop.
pub struct HeartbeatLoop {
    client: PaperclipClient,
    config: HeartbeatConfig,
    executor: Arc<dyn TaskExecutor>,
}

impl HeartbeatLoop {
    pub fn new(
        client: PaperclipClient,
        config: HeartbeatConfig,
        executor: Arc<dyn TaskExecutor>,
    ) -> Self {
        Self {
            client,
            config,
            executor,
        }
    }

    /// Run one heartbeat cycle.  Returns the number of tasks processed.
    pub async fn run_once(&self) -> Result<usize> {
        // Step 1: agent_id is already in config — no round-trip needed just for logging.
        info!(
            agent_id = %self.config.agent_id,
            "Heartbeat start"
        );

        // Step 3: inbox
        let inbox = self.client.get_inbox().await?;
        if inbox.is_empty() {
            info!("Inbox empty — nothing to do");
            return Ok(0);
        }

        // Step 4: pick work — in_progress first, then todo
        let candidates = Self::prioritise(&inbox);
        if candidates.is_empty() {
            info!("No actionable tasks (all blocked/done)");
            return Ok(0);
        }

        let mut processed = 0;
        for item in candidates.iter().take(self.config.max_tasks_per_wake) {
            if self.process_task(item).await? {
                processed += 1;
            }
        }

        Ok(processed)
    }

    /// Process a single task through the full heartbeat cycle.
    /// Returns true if the task was worked on.
    async fn process_task(&self, item: &InboxItem) -> Result<bool> {
        info!(
            issue = %item.identifier,
            status = %item.status,
            "Processing task"
        );

        // Step 5: checkout
        let issue = self
            .client
            .checkout(
                &item.id,
                &self.config.agent_id,
                &["todo", "backlog", "blocked", "in_progress"],
            )
            .await?;

        let Some(_issue) = issue else {
            warn!(issue = %item.identifier, "Checkout 409 — skipping");
            return Ok(false);
        };

        // Step 6: heartbeat context
        let context = match self.client.get_heartbeat_context(&item.id).await {
            Ok(ctx) => ctx,
            Err(e) => {
                warn!(issue = %item.identifier, error = %e, "Failed to get context");
                return Ok(false);
            }
        };

        // Step 7: execute
        let outcome = self.executor.execute(item, &context).await;

        // Step 8: update status + comment
        match outcome {
            Ok(ExecutionOutcome::Done(comment)) => {
                self.client.mark_done(&item.id, Some(&comment)).await?;
                info!(issue = %item.identifier, "Marked done");
            }
            Ok(ExecutionOutcome::Blocked(comment)) => {
                self.client.mark_blocked(&item.id, &comment).await?;
                warn!(issue = %item.identifier, "Marked blocked");
            }
            Ok(ExecutionOutcome::InReview(comment)) => {
                use crate::types::UpdateIssueRequest;
                self.client
                    .update_issue(
                        &item.id,
                        UpdateIssueRequest {
                            status: Some("in_review".into()),
                            comment: Some(comment),
                            ..Default::default()
                        },
                    )
                    .await?;
                info!(issue = %item.identifier, "Marked in_review");
            }
            Ok(ExecutionOutcome::Deferred) => {
                info!(issue = %item.identifier, "Execution deferred — no status change");
            }
            Err(e) => {
                let comment = format!(
                    "Execution error: {e}\n\nTask left in current status. Investigate and retry."
                );
                self.client.add_comment(&item.id, &comment).await?;
                warn!(issue = %item.identifier, error = %e, "Execution error");
            }
        }

        Ok(true)
    }

    /// Sort inbox items: in_progress first, then todo by priority.
    /// Skip blocked items (they require explicit new context).
    fn prioritise(inbox: &[InboxItem]) -> Vec<&InboxItem> {
        let mut in_progress: Vec<&InboxItem> = inbox
            .iter()
            .filter(|i| i.status == IssueStatus::InProgress)
            .collect();

        let mut todo: Vec<&InboxItem> = inbox
            .iter()
            .filter(|i| i.status == IssueStatus::Todo || i.status == IssueStatus::Backlog)
            .collect();

        // Sort todo by priority (critical first)
        todo.sort_by(|a, b| a.priority.cmp(&b.priority));

        in_progress.append(&mut todo);
        in_progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::IssuePriority;
    use chrono::Utc;

    fn make_item(status: IssueStatus, priority: IssuePriority) -> InboxItem {
        InboxItem {
            id: uuid::Uuid::new_v4().to_string(),
            identifier: "TEST-1".into(),
            title: "Test task".into(),
            status,
            priority,
            project_id: None,
            goal_id: None,
            parent_id: None,
            updated_at: Utc::now(),
            active_run: None,
        }
    }

    #[test]
    fn prioritise_puts_in_progress_first() {
        let items = vec![
            make_item(IssueStatus::Todo, IssuePriority::High),
            make_item(IssueStatus::InProgress, IssuePriority::Medium),
            make_item(IssueStatus::Blocked, IssuePriority::Critical),
        ];
        let ordered = HeartbeatLoop::prioritise(&items);
        assert_eq!(ordered.len(), 2, "blocked items excluded");
        assert_eq!(ordered[0].status, IssueStatus::InProgress);
        assert_eq!(ordered[1].status, IssueStatus::Todo);
    }

    #[test]
    fn prioritise_sorts_todo_by_priority() {
        let items = vec![
            make_item(IssueStatus::Todo, IssuePriority::Low),
            make_item(IssueStatus::Todo, IssuePriority::Critical),
            make_item(IssueStatus::Todo, IssuePriority::Medium),
        ];
        let ordered = HeartbeatLoop::prioritise(&items);
        assert_eq!(ordered[0].priority, IssuePriority::Critical);
        assert_eq!(ordered[1].priority, IssuePriority::Medium);
        assert_eq!(ordered[2].priority, IssuePriority::Low);
    }

    #[test]
    fn prioritise_excludes_blocked() {
        let items = vec![
            make_item(IssueStatus::Blocked, IssuePriority::Critical),
            make_item(IssueStatus::Blocked, IssuePriority::High),
        ];
        let ordered = HeartbeatLoop::prioritise(&items);
        assert!(ordered.is_empty());
    }

    /// Verify that process_task posts a comment and returns Ok(true) when the
    /// executor returns an error.  Uses a minimal in-process HTTP server to
    /// capture the add_comment request.
    #[tokio::test]
    async fn process_task_posts_comment_on_executor_error() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc as StdArc;

        // ---- minimal inline mock server ----
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let comment_received = StdArc::new(AtomicBool::new(false));
        let comment_received_clone = comment_received.clone();

        // Spawn a bare TCP server that responds to every request with a
        // canned JSON response (good enough for this unit test).
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);

                // Detect the add_comment POST
                if req.contains("POST") && req.contains("/comments") {
                    comment_received_clone.store(true, Ordering::SeqCst);
                }

                // Respond with a valid Comment JSON for add_comment,
                // and a valid Issue JSON for checkout + heartbeat-context.
                let body = if req.contains("/checkout") {
                    r#"{"id":"issue-1","companyId":"co","projectId":null,"projectWorkspaceId":null,"goalId":null,"parentId":null,"title":"T","description":null,"status":"todo","priority":"medium","assigneeAgentId":null,"assigneeUserId":null,"checkoutRunId":null,"executionRunId":null,"executionAgentNameKey":null,"executionLockedAt":null,"createdByAgentId":null,"createdByUserId":null,"issueNumber":1,"identifier":"TEST-1","originKind":"manual","originId":null,"originRunId":null,"requestDepth":0,"billingCode":null,"assigneeAdapterOverrides":null,"executionWorkspaceId":null,"executionWorkspacePreference":null,"executionWorkspaceSettings":null,"startedAt":null,"completedAt":null,"cancelledAt":null,"hiddenAt":null,"createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T00:00:00Z","labels":[],"labelIds":[]}"#
                } else if req.contains("/heartbeat-context") {
                    r#"{"issue":{"id":"issue-1","identifier":"TEST-1","title":"T","description":null,"status":"todo","priority":"medium","projectId":null,"goalId":null,"parentId":null,"assigneeAgentId":null,"assigneeUserId":null,"updatedAt":"2024-01-01T00:00:00Z","createdAt":"2024-01-01T00:00:00Z"},"ancestors":[],"project":null,"goal":null,"commentCursor":{"totalComments":0,"latestCommentId":null,"latestCommentAt":null},"wakeComment":null}"#
                } else {
                    // add_comment response
                    r#"{"id":"comment-1","issueId":"issue-1","body":"ok","authorAgentId":null,"authorUserId":null,"createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T00:00:00Z"}"#
                };

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

        // ---- executor that always errors ----
        struct FailExecutor;
        #[async_trait]
        impl TaskExecutor for FailExecutor {
            async fn execute(
                &self,
                _item: &InboxItem,
                _context: &HeartbeatContext,
            ) -> Result<ExecutionOutcome> {
                Err(anyhow::anyhow!("simulated executor failure"))
            }
        }

        let client = PaperclipClient::new(format!("http://{addr}"), "test-key".into());
        let config = HeartbeatConfig {
            agent_id: "agent-1".into(),
            company_id: "co-1".into(),
            max_tasks_per_wake: 1,
        };
        let loop_ = HeartbeatLoop::new(client, config, StdArc::new(FailExecutor));

        let item = make_item(IssueStatus::Todo, IssuePriority::Medium);
        // Override id to match mock
        let mut item = item;
        item.id = "issue-1".into();

        let result = loop_.process_task(&item).await;
        assert!(
            result.is_ok(),
            "process_task should not propagate executor error"
        );
        assert!(
            result.unwrap(),
            "process_task should return true even on executor error"
        );
        assert!(
            comment_received.load(Ordering::SeqCst),
            "should have posted a comment"
        );
    }
}
