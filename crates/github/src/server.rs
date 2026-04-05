use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

use crate::{
    config::WebhookConfig,
    events::{IssueCommentEvent, MentionContext, PullRequestReviewCommentEvent},
    mention::extract_mentions,
    paperclip::PaperclipClient,
    signature,
};

/// GitHub webhook server.
pub struct WebhookServer {
    config: Arc<WebhookConfig>,
}

impl WebhookServer {
    pub fn new(config: WebhookConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    /// Start the Axum HTTP server and block until shutdown.
    pub async fn run(self) -> anyhow::Result<()> {
        let bind = self.config.bind.clone();
        let state = Arc::new(AppState {
            config: Arc::clone(&self.config),
            paperclip: PaperclipClient::new(
                self.config.paperclip_api_url.clone(),
                self.config.paperclip_api_key.clone(),
                self.config.paperclip_company_id.clone(),
            ),
        });

        let app = Router::new()
            .route("/webhook/github", post(handle_webhook))
            .layer(TraceLayer::new_for_http())
            .with_state(state);

        info!("Webhook server listening on {}", bind);
        let listener = tokio::net::TcpListener::bind(&bind).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}

struct AppState {
    config: Arc<WebhookConfig>,
    paperclip: PaperclipClient,
}

async fn handle_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    // 1. Verify signature
    let sig = match headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s.to_string(),
        None => {
            warn!("Webhook received without X-Hub-Signature-256");
            return StatusCode::UNAUTHORIZED;
        }
    };

    if let Err(e) = signature::verify(&state.config.webhook_secret, &body, &sig) {
        warn!(error = %e, "Webhook signature verification failed");
        return StatusCode::UNAUTHORIZED;
    }

    // 2. Identify event type
    let event = match headers.get("x-github-event").and_then(|v| v.to_str().ok()) {
        Some(e) => e.to_string(),
        None => {
            warn!("Webhook missing X-GitHub-Event header");
            return StatusCode::BAD_REQUEST;
        }
    };

    // 3. Parse into a normalised MentionContext (only handle `created` actions)
    let ctx: Option<MentionContext> = match event.as_str() {
        "issue_comment" => {
            match serde_json::from_slice::<IssueCommentEvent>(&body) {
                Ok(e) if e.action == "created" => Some(e.into()),
                Ok(_) => None, // ignore edited/deleted
                Err(err) => {
                    error!(error = %err, "Failed to parse issue_comment payload");
                    return StatusCode::BAD_REQUEST;
                }
            }
        }
        "pull_request_review_comment" => {
            match serde_json::from_slice::<PullRequestReviewCommentEvent>(&body) {
                Ok(e) if e.action == "created" => Some(e.into()),
                Ok(_) => None,
                Err(err) => {
                    error!(error = %err, "Failed to parse pull_request_review_comment payload");
                    return StatusCode::BAD_REQUEST;
                }
            }
        }
        other => {
            // Silently acknowledge unsupported events (GitHub sends ping, push, etc.)
            info!(event = %other, "Ignoring unsupported event");
            return StatusCode::OK;
        }
    };

    let ctx = match ctx {
        Some(c) => c,
        None => return StatusCode::OK,
    };

    // 4. Extract @mentions and match against configured agents
    let mentions = extract_mentions(&ctx.body);
    if mentions.is_empty() {
        return StatusCode::OK;
    }

    for handle in &mentions {
        let agent_id = match state.config.agents.get(handle.as_str()) {
            Some(id) => id.clone(),
            None => {
                info!(handle = %handle, "No agent configured for mention");
                continue;
            }
        };

        match state
            .paperclip
            .create_mention_task(&agent_id, handle, &ctx)
            .await
        {
            Ok(issue_id) => {
                info!(
                    issue = %issue_id,
                    handle = %handle,
                    repo = %ctx.repo,
                    number = ctx.number,
                    "Created task for mention"
                );
            }
            Err(e) => {
                error!(
                    handle = %handle,
                    error = %e,
                    "Failed to create Paperclip task for mention"
                );
            }
        }
    }

    StatusCode::OK
}
