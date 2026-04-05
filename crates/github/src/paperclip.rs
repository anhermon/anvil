use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;
use tracing::info;

use crate::events::MentionContext;

/// Minimal Paperclip API client for creating mention-triggered tasks.
pub struct PaperclipClient {
    client: Client,
    api_url: String,
    api_key: String,
    company_id: String,
}

impl PaperclipClient {
    pub fn new(api_url: String, api_key: String, company_id: String) -> Self {
        Self {
            client: Client::new(),
            api_url,
            api_key,
            company_id,
        }
    }

    /// Create a Paperclip issue assigned to the given agent, carrying GitHub context.
    pub async fn create_mention_task(
        &self,
        agent_id: &str,
        mention_handle: &str,
        ctx: &MentionContext,
    ) -> Result<String> {
        let title = format!(
            "GitHub mention: @{} in {}/#{} by @{}",
            mention_handle, ctx.repo, ctx.number, ctx.author
        );

        let description = format!(
            "## Objective\n\
            Respond to GitHub @{mention_handle} mention in {repo} {kind} #{number}.\n\
            \n\
            ## Context\n\
            - **Repo:** [{repo}]({repo_url})\n\
            - **{kind_label}:** [#{number} — {title}]({html_url})\n\
            - **Comment:** [{comment_url}]({comment_url})\n\
            - **Author:** @{author}\n\
            - **Trigger:** `@{mention_handle}` detected in comment body\n\
            \n\
            ### Comment body\n\
            \n\
            > {body}\n\
            \n\
            ## Scope\n\
            **Touch:** Respond to this GitHub mention as appropriate (review, comment, create subtask)\n\
            **Do not touch:** Unrelated code or issues\n\
            \n\
            ## Verification\n\
            - [ ] Agent has read the comment and taken appropriate action\n\
            - [ ] Response posted back to GitHub (comment or PR review)",
            mention_handle = mention_handle,
            repo = ctx.repo,
            repo_url = ctx.html_url,
            kind = ctx.kind,
            kind_label = if ctx.kind == "pull_request" { "Pull Request" } else { "Issue" },
            number = ctx.number,
            title = ctx.title,
            html_url = ctx.html_url,
            comment_url = ctx.comment_url,
            author = ctx.author,
            body = ctx.body,
        );

        let payload = json!({
            "title": title,
            "description": description,
            "assigneeAgentId": agent_id,
            "status": "todo",
            "priority": "high",
        });

        let response = self
            .client
            .post(format!(
                "{}/api/companies/{}/issues",
                self.api_url, self.company_id
            ))
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .context("failed to POST issue to Paperclip API")?;

        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse Paperclip API response")?;

        if !status.is_success() {
            anyhow::bail!("Paperclip API returned {}: {}", status, body);
        }

        let issue_id = body["identifier"].as_str().unwrap_or("unknown").to_string();

        info!(
            issue = %issue_id,
            agent = %agent_id,
            handle = %mention_handle,
            "Created mention task"
        );

        Ok(issue_id)
    }
}
