use serde::Deserialize;

/// A GitHub repository reference inside a webhook payload.
#[derive(Debug, Deserialize)]
pub struct Repository {
    pub full_name: String,
    pub html_url: String,
}

/// A GitHub user/sender reference.
#[derive(Debug, Deserialize)]
pub struct User {
    pub login: String,
}

/// A GitHub comment (issue comment or PR review comment).
#[derive(Debug, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub body: String,
    pub html_url: String,
}

/// A GitHub issue reference inside an `issue_comment` webhook.
#[derive(Debug, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub html_url: String,
}

/// A GitHub pull request reference inside a `pull_request_review_comment` webhook.
#[derive(Debug, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub html_url: String,
}

/// Payload for `X-GitHub-Event: issue_comment`.
#[derive(Debug, Deserialize)]
pub struct IssueCommentEvent {
    pub action: String,
    pub issue: Issue,
    pub comment: Comment,
    pub repository: Repository,
    pub sender: User,
}

/// Payload for `X-GitHub-Event: pull_request_review_comment`.
#[derive(Debug, Deserialize)]
pub struct PullRequestReviewCommentEvent {
    pub action: String,
    pub pull_request: PullRequest,
    pub comment: Comment,
    pub repository: Repository,
    pub sender: User,
}

/// Normalised context extracted from any supported GitHub event.
#[derive(Debug)]
pub struct MentionContext {
    /// GitHub repo slug (e.g. "owner/repo")
    pub repo: String,
    /// PR or issue number
    pub number: u64,
    /// PR or issue title
    pub title: String,
    /// Direct URL to the PR/issue
    pub html_url: String,
    /// URL of the comment itself
    pub comment_url: String,
    /// Full comment body
    pub body: String,
    /// GitHub login of the comment author
    pub author: String,
    /// Kind: "issue" or "pull_request"
    pub kind: String,
}

impl From<IssueCommentEvent> for MentionContext {
    fn from(e: IssueCommentEvent) -> Self {
        Self {
            repo: e.repository.full_name,
            number: e.issue.number,
            title: e.issue.title,
            html_url: e.issue.html_url,
            comment_url: e.comment.html_url,
            body: e.comment.body,
            author: e.sender.login,
            kind: "issue".to_string(),
        }
    }
}

impl From<PullRequestReviewCommentEvent> for MentionContext {
    fn from(e: PullRequestReviewCommentEvent) -> Self {
        Self {
            repo: e.repository.full_name,
            number: e.pull_request.number,
            title: e.pull_request.title,
            html_url: e.pull_request.html_url,
            comment_url: e.comment.html_url,
            body: e.comment.body,
            author: e.sender.login,
            kind: "pull_request".to_string(),
        }
    }
}
