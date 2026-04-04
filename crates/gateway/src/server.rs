#![allow(clippy::module_name_repetitions)]

use crate::{
    config::GatewayConfig,
    event::{AgentEvent, ControlCommand},
};
use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{broadcast, mpsc},
    task::JoinHandle,
};
use tower_http::cors::CorsLayer;
use tracing::{debug, error, info, warn};

/// Shared application state threaded through Axum handlers.
#[derive(Clone)]
struct AppState {
    /// Sender for the broadcast channel; cloned into each WebSocket session.
    event_tx: broadcast::Sender<AgentEvent>,
    /// Sender to forward inbound control commands from any client.
    cmd_tx: mpsc::Sender<ControlCommand>,
}

/// The WebSocket gateway.
pub struct Gateway {
    config: GatewayConfig,
}

impl Gateway {
    /// Create a new gateway with the given configuration.
    pub fn new(config: GatewayConfig) -> Self {
        Self { config }
    }

    /// Start the gateway and return a [`GatewayHandle`] for interaction.
    ///
    /// The underlying server runs as a background `tokio` task.
    ///
    /// # Errors
    ///
    /// Returns an error if the TCP listener cannot bind to the configured port.
    pub async fn start(self) -> anyhow::Result<GatewayHandle> {
        let (event_tx, _) = broadcast::channel::<AgentEvent>(self.config.event_buffer);
        let (cmd_tx, cmd_rx) = mpsc::channel::<ControlCommand>(64);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let state = AppState {
            event_tx: event_tx.clone(),
            cmd_tx: cmd_tx.clone(),
        };

        let app = Router::new()
            .route("/ws", get(ws_handler))
            .route("/health", get(health_handler))
            .layer(CorsLayer::permissive())
            .with_state(Arc::new(state));

        let addr = SocketAddr::from(([127, 0, 0, 1], self.config.port));
        let listener = TcpListener::bind(addr).await?;
        let bound_addr = listener.local_addr()?;
        info!("harness-gateway listening on ws://{}/ws", bound_addr);

        let server_task: JoinHandle<()> = tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            if let Err(e) = server.await {
                error!("gateway server error: {e}");
            }
            info!("harness-gateway shut down");
        });

        Ok(GatewayHandle {
            event_tx,
            cmd_rx,
            shutdown_tx: Some(shutdown_tx),
            server_task,
            addr: bound_addr,
        })
    }
}

/// A handle returned by [`Gateway::start`].
///
/// Use this to emit events, receive commands, and shut down the server.
pub struct GatewayHandle {
    event_tx: broadcast::Sender<AgentEvent>,
    /// Inbound commands from connected clients.
    pub cmd_rx: mpsc::Receiver<ControlCommand>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    server_task: JoinHandle<()>,
    /// The actual bound address (useful when port 0 was requested in tests).
    pub addr: SocketAddr,
}

impl GatewayHandle {
    /// Broadcast an event to all connected WebSocket clients.
    ///
    /// If no clients are connected the event is silently dropped.
    pub async fn emit(&self, event: AgentEvent) {
        match self.event_tx.send(event) {
            Ok(n) => debug!("event broadcast to {n} clients"),
            Err(_) => debug!("no clients connected; event dropped"),
        }
    }

    /// Gracefully shut down the gateway server.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.server_task.await;
    }
}

// ─── Axum handlers ───────────────────────────────────────────────────────────

async fn health_handler() -> impl IntoResponse {
    axum::Json(serde_json::json!({ "status": "ok" }))
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut event_rx = state.event_tx.subscribe();
    info!("WebSocket client connected");

    loop {
        tokio::select! {
            // Forward agent events to this client.
            event = event_rx.recv() => {
                match event {
                    Ok(ev) => {
                        let json = match serde_json::to_string(&ev) {
                            Ok(j) => j,
                            Err(e) => {
                                error!("failed to serialise event: {e}");
                                continue;
                            }
                        };
                        if socket.send(WsMessage::Text(json.into())).await.is_err() {
                            break; // client disconnected
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("client lagged by {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // Receive control commands from the client.
            msg = socket.recv() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        match serde_json::from_str::<ControlCommand>(&text) {
                            Ok(cmd) => {
                                debug!("received command: {cmd:?}");
                                if cmd == ControlCommand::Ping {
                                    let pong = serde_json::json!({ "kind": "pong" }).to_string();
                                    let _ = socket.send(WsMessage::Text(pong.into())).await;
                                } else {
                                    let _ = state.cmd_tx.send(cmd).await;
                                }
                            }
                            Err(e) => warn!("invalid command payload: {e}"),
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(_)) => {} // ignore binary / ping frames
                    Some(Err(e)) => {
                        error!("WebSocket error: {e}");
                        break;
                    }
                }
            }
        }
    }

    info!("WebSocket client disconnected");
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::wildcard_imports
)]
mod tests {
    use super::*;
    use crate::{AgentEvent, GatewayConfig};
    use uuid::Uuid;

    async fn start_test_gateway() -> GatewayHandle {
        // Port 0 → OS picks a free port.
        let cfg = GatewayConfig {
            port: 0,
            event_buffer: 16,
        };
        Gateway::new(cfg).start().await.expect("gateway start")
    }

    #[tokio::test]
    async fn test_gateway_starts_and_health_ok() {
        let handle = start_test_gateway().await;
        let url = format!("http://{}/health", handle.addr);
        let resp = reqwest::get(&url).await.expect("http get");
        assert!(resp.status().is_success());
        let body: serde_json::Value = resp.json().await.expect("json");
        assert_eq!(body["status"], "ok");
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn test_emit_with_no_clients_does_not_panic() {
        let handle = start_test_gateway().await;
        // No client subscribed — send should not panic.
        handle.emit(AgentEvent::token("hello")).await;
        handle.emit(AgentEvent::error("oops")).await;
        handle.emit(AgentEvent::turn_start(Uuid::new_v4())).await;
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn test_ws_event_delivery() {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;

        let handle = start_test_gateway().await;
        let ws_url = format!("ws://{}/ws", handle.addr);

        let (mut ws, _) = connect_async(&ws_url).await.expect("ws connect");

        // Yield so the server-side handle_socket task has time to subscribe
        // to the broadcast channel before we emit.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Emit an event from the server side.
        handle
            .emit(AgentEvent::Token {
                turn_id: Uuid::nil(),
                delta: "streaming!".into(),
                ts: chrono::Utc::now(),
            })
            .await;

        // Receive on the client side.
        let msg = ws.next().await.expect("no msg").expect("ws err");
        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
            let ev: serde_json::Value = serde_json::from_str(&text).expect("json");
            assert_eq!(ev["kind"], "token");
            assert_eq!(ev["delta"], "streaming!");
        } else {
            panic!("unexpected message type");
        }

        ws.close(None).await.ok();
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn test_ws_ping_pong() {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;

        let handle = start_test_gateway().await;
        let ws_url = format!("ws://{}/ws", handle.addr);

        let (mut ws, _) = connect_async(&ws_url).await.expect("ws connect");

        let ping_payload = serde_json::json!({ "cmd": "ping" }).to_string();
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            ping_payload.into(),
        ))
        .await
        .expect("send");

        let msg = ws.next().await.expect("no msg").expect("ws err");
        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
            let v: serde_json::Value = serde_json::from_str(&text).expect("json");
            assert_eq!(v["kind"], "pong");
        } else {
            panic!("expected text message");
        }

        ws.close(None).await.ok();
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn test_ws_control_command_forwarded() {
        use futures::{SinkExt, StreamExt as _};
        use tokio_tungstenite::connect_async;

        let mut handle = start_test_gateway().await;
        let ws_url = format!("ws://{}/ws", handle.addr);

        let (mut ws, _) = connect_async(&ws_url).await.expect("ws connect");

        let cmd_payload = serde_json::json!({ "cmd": "interrupt" }).to_string();
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            cmd_payload.into(),
        ))
        .await
        .expect("send");

        // Give the server a moment to forward the command.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let cmd = handle.cmd_rx.try_recv().expect("no command received");
        assert_eq!(cmd, ControlCommand::Interrupt);

        ws.close(None).await.ok();
        handle.shutdown().await;
    }
}
