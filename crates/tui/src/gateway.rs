//! WebSocket client that connects to harness-gateway and forwards events.

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::connect_async;
use tracing::{error, info, warn};

use crate::events::{AgentEvent, AppEvent, GatewayStatus};

const INITIAL_BACKOFF_MS: u64 = 500;
const MAX_BACKOFF_MS: u64 = 30_000;

/// Runs the gateway client with automatic reconnection.
/// Never returns (runs until the TUI exits via channel close).
pub async fn run_gateway_client(url: String, tx: mpsc::UnboundedSender<AppEvent>) {
    let mut backoff_ms = INITIAL_BACKOFF_MS;
    let mut attempt: u32 = 0;

    loop {
        // Signal connecting
        let _ = tx.send(AppEvent::GatewayStatus(if attempt == 0 {
            GatewayStatus::Connecting
        } else {
            GatewayStatus::Reconnecting { attempt }
        }));

        match connect_async(&url).await {
            Ok((ws_stream, _)) => {
                info!("Connected to gateway at {url}");
                backoff_ms = INITIAL_BACKOFF_MS;
                attempt = 0;
                let _ = tx.send(AppEvent::GatewayStatus(GatewayStatus::Connected));

                let (_, mut read) = ws_stream.split();

                while let Some(msg_result) = read.next().await {
                    match msg_result {
                        Ok(msg) => {
                            if let Ok(text) = msg.into_text() {
                                match serde_json::from_str::<AgentEvent>(&text) {
                                    Ok(event) => {
                                        if tx.send(AppEvent::Agent(event)).is_err() {
                                            // TUI shut down
                                            return;
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to parse gateway message: {e} — raw: {text}");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("WebSocket error: {e}");
                            break;
                        }
                    }
                }

                let reason = "connection closed".to_string();
                let _ = tx.send(AppEvent::GatewayStatus(GatewayStatus::Disconnected {
                    reason,
                }));
            }
            Err(e) => {
                let reason = e.to_string();
                warn!("Gateway connection failed: {reason}");
                let _ = tx.send(AppEvent::GatewayStatus(GatewayStatus::Disconnected {
                    reason,
                }));
            }
        }

        attempt += 1;
        sleep(Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
    }
}
