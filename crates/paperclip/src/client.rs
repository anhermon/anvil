//! Typed HTTP client for the Paperclip REST API.
//!
//! Covers the endpoints used by the heartbeat procedure:
//! identity, inbox, checkout, heartbeat-context, update, comment, create-issue.

use anyhow::{bail, Context, Result};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use tracing::{debug, warn};

use crate::types::{
    AddCommentRequest, AgentIdentity, CheckoutRequest, Comment, CreateIssueRequest,
    HeartbeatContext, InboxItem, Issue, UpdateIssueRequest,
};

/// A client for the Paperclip REST API.
///
/// All mutating requests automatically attach `X-Paperclip-Run-Id` when
/// `run_id` is set.
#[derive(Clone)]
pub struct PaperclipClient {
    http: Client,
    api_url: String,
    api_key: String,
    run_id: Option<String>,
}

impl PaperclipClient {
    /// Create a new client.
    ///
    /// `api_url` — base URL (e.g. `http://127.0.0.1:3100`)
    /// `api_key` — `PAPERCLIP_API_KEY` bearer token
    pub fn new(api_url: String, api_key: String) -> Self {
        Self {
            http: Client::new(),
            api_url,
            api_key,
            run_id: None,
        }
    }

    /// Attach a run ID for audit-trail headers on mutating requests.
    pub fn with_run_id(mut self, run_id: String) -> Self {
        self.run_id = Some(run_id);
        self
    }

    // ── helpers ────────────────────────────────────────────────────────────

    fn auth_get(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.api_url, path);
        debug!(url, "GET");
        self.http.get(url).bearer_auth(&self.api_key)
    }

    fn auth_post(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.api_url, path);
        debug!(url, "POST");
        let mut req = self.http.post(url).bearer_auth(&self.api_key);
        if let Some(rid) = &self.run_id {
            req = req.header("X-Paperclip-Run-Id", rid);
        }
        req
    }

    fn auth_patch(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.api_url, path);
        debug!(url, "PATCH");
        let mut req = self.http.patch(url).bearer_auth(&self.api_key);
        if let Some(rid) = &self.run_id {
            req = req.header("X-Paperclip-Run-Id", rid);
        }
        req
    }

    async fn require_ok(resp: reqwest::Response, ctx: &str) -> Result<Value> {
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .with_context(|| format!("{ctx}: failed to parse response JSON"))?;

        if !status.is_success() {
            bail!("{ctx}: HTTP {status} — {body}");
        }
        Ok(body)
    }

    // ── identity ───────────────────────────────────────────────────────────

    /// Step 1 — GET /api/agents/me
    pub async fn get_identity(&self) -> Result<AgentIdentity> {
        let resp = self
            .auth_get("/api/agents/me")
            .send()
            .await
            .context("GET /api/agents/me")?;
        let body = Self::require_ok(resp, "get_identity").await?;
        serde_json::from_value(body).context("deserialise AgentIdentity")
    }

    // ── inbox ──────────────────────────────────────────────────────────────

    /// Step 3 — GET /api/agents/me/inbox-lite
    pub async fn get_inbox(&self) -> Result<Vec<InboxItem>> {
        let resp = self
            .auth_get("/api/agents/me/inbox-lite")
            .send()
            .await
            .context("GET /api/agents/me/inbox-lite")?;
        let body = Self::require_ok(resp, "get_inbox").await?;
        serde_json::from_value(body).context("deserialise inbox")
    }

    // ── checkout ───────────────────────────────────────────────────────────

    /// Step 5 — POST /api/issues/{id}/checkout
    ///
    /// Returns `Ok(Some(issue))` on success, `Ok(None)` on 409 Conflict
    /// (owned by another agent — caller should skip this task).
    pub async fn checkout(
        &self,
        issue_id: &str,
        agent_id: &str,
        expected_statuses: &[&str],
    ) -> Result<Option<Issue>> {
        let payload = CheckoutRequest {
            agent_id: agent_id.to_string(),
            expected_statuses: expected_statuses.iter().map(|s| s.to_string()).collect(),
        };

        let resp = self
            .auth_post(&format!("/api/issues/{issue_id}/checkout"))
            .json(&payload)
            .send()
            .await
            .context("POST checkout")?;

        if resp.status() == StatusCode::CONFLICT {
            warn!(issue_id, "Checkout 409 — owned by another agent, skipping");
            return Ok(None);
        }

        let body = Self::require_ok(resp, "checkout").await?;
        let issue: Issue = serde_json::from_value(body).context("deserialise checkout response")?;
        Ok(Some(issue))
    }

    // ── heartbeat context ──────────────────────────────────────────────────

    /// Step 6 — GET /api/issues/{id}/heartbeat-context
    pub async fn get_heartbeat_context(&self, issue_id: &str) -> Result<HeartbeatContext> {
        let resp = self
            .auth_get(&format!("/api/issues/{issue_id}/heartbeat-context"))
            .send()
            .await
            .context("GET heartbeat-context")?;
        let body = Self::require_ok(resp, "get_heartbeat_context").await?;
        serde_json::from_value(body).context("deserialise HeartbeatContext")
    }

    // ── comments ───────────────────────────────────────────────────────────

    /// GET /api/issues/{id}/comments  (full thread)
    pub async fn get_comments(&self, issue_id: &str) -> Result<Vec<Comment>> {
        let resp = self
            .auth_get(&format!("/api/issues/{issue_id}/comments"))
            .send()
            .await
            .context("GET comments")?;
        let body = Self::require_ok(resp, "get_comments").await?;
        serde_json::from_value(body).context("deserialise comments")
    }

    /// GET /api/issues/{id}/comments?after={comment_id}&order=asc  (incremental)
    pub async fn get_comments_after(
        &self,
        issue_id: &str,
        after_comment_id: &str,
    ) -> Result<Vec<Comment>> {
        let resp = self
            .auth_get(&format!(
                "/api/issues/{issue_id}/comments?after={after_comment_id}&order=asc"
            ))
            .send()
            .await
            .context("GET comments (after)")?;
        let body = Self::require_ok(resp, "get_comments_after").await?;
        serde_json::from_value(body).context("deserialise comments (after)")
    }

    /// POST /api/issues/{id}/comments
    pub async fn add_comment(&self, issue_id: &str, body: &str) -> Result<Comment> {
        let payload = AddCommentRequest {
            body: body.to_string(),
        };
        let resp = self
            .auth_post(&format!("/api/issues/{issue_id}/comments"))
            .json(&payload)
            .send()
            .await
            .context("POST comment")?;
        let val = Self::require_ok(resp, "add_comment").await?;
        serde_json::from_value(val).context("deserialise comment")
    }

    // ── update issue ───────────────────────────────────────────────────────

    /// PATCH /api/issues/{id}
    pub async fn update_issue(&self, issue_id: &str, req: UpdateIssueRequest) -> Result<Issue> {
        let resp = self
            .auth_patch(&format!("/api/issues/{issue_id}"))
            .json(&req)
            .send()
            .await
            .context("PATCH issue")?;
        let body = Self::require_ok(resp, "update_issue").await?;
        serde_json::from_value(body).context("deserialise updated issue")
    }

    // ── convenience: set status + optional comment ─────────────────────────

    /// Mark issue done with an optional comment.
    pub async fn mark_done(&self, issue_id: &str, comment: Option<&str>) -> Result<Issue> {
        self.update_issue(
            issue_id,
            UpdateIssueRequest {
                status: Some("done".into()),
                comment: comment.map(str::to_string),
                ..Default::default()
            },
        )
        .await
    }

    /// Mark issue blocked with a required comment explaining the blocker.
    pub async fn mark_blocked(&self, issue_id: &str, comment: &str) -> Result<Issue> {
        self.update_issue(
            issue_id,
            UpdateIssueRequest {
                status: Some("blocked".into()),
                comment: Some(comment.to_string()),
                ..Default::default()
            },
        )
        .await
    }

    // ── create issue ───────────────────────────────────────────────────────

    /// POST /api/companies/{companyId}/issues
    pub async fn create_issue(&self, company_id: &str, req: CreateIssueRequest) -> Result<Issue> {
        let resp = self
            .auth_post(&format!("/api/companies/{company_id}/issues"))
            .json(&req)
            .send()
            .await
            .context("POST create issue")?;
        let body = Self::require_ok(resp, "create_issue").await?;
        serde_json::from_value(body).context("deserialise created issue")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_builds_without_run_id() {
        let c = PaperclipClient::new("http://localhost:3100".into(), "test-key".into());
        assert!(c.run_id.is_none());
    }

    #[test]
    fn client_with_run_id() {
        let c = PaperclipClient::new("http://localhost:3100".into(), "test-key".into())
            .with_run_id("run-abc".into());
        assert_eq!(c.run_id.as_deref(), Some("run-abc"));
    }

    /// checkout() must return Ok(None) on a 409 Conflict response so the
    /// heartbeat loop skips the task rather than propagating an error.
    #[tokio::test]
    async fn checkout_returns_none_on_409() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let body = r#"{"error":"conflict"}"#;
            let response = format!(
                "HTTP/1.1 409 Conflict\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });

        let client = PaperclipClient::new(format!("http://{addr}"), "key".into());
        let result = client.checkout("issue-1", "agent-1", &["todo"]).await;

        assert!(result.is_ok(), "checkout should not error on 409");
        assert!(
            result.unwrap().is_none(),
            "checkout should return None on 409"
        );
    }
}
