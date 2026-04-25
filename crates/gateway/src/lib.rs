//! harness-gateway — WebSocket control-plane for Anvil.
#![allow(clippy::module_name_repetitions)]
//!
//! # Overview
//!
//! [`Gateway`] starts a local Axum-based WebSocket server that:
//! - Broadcasts [`AgentEvent`]s to all connected clients in real time.
//! - Accepts [`ControlCommand`]s (interrupt / pause / resume) from any client.
//!
//! # Usage
//!
//! ```rust,no_run
//! use harness_gateway::{Gateway, GatewayConfig, AgentEvent};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let cfg = GatewayConfig { port: 9000, ..Default::default() };
//!     let gateway = Gateway::new(cfg);
//!     let handle = gateway.start().await?;
//!
//!     // Broadcast an event from agent code:
//!     handle.emit(AgentEvent::token("Hello, world!")).await;
//!
//!     handle.shutdown().await;
//!     Ok(())
//! }
//! ```

pub mod config;
pub mod event;
pub mod server;

pub use config::GatewayConfig;
pub use event::{AgentEvent, ControlCommand};
pub use server::{Gateway, GatewayHandle};
