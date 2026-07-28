//! Paperclip control-plane client and heartbeat adapter for Anvil.
//!
//! This crate provides:
//! - [`PaperclipClient`] — typed HTTP client for the Paperclip REST API
//! - [`HeartbeatLoop`] — full heartbeat cycle: poll inbox → checkout → run → report
//! - [`types`] — shared API types (Agent, Issue, `InboxItem`, …)
//!
//! # Quick start
//!
//! ```no_run
//! use harness_paperclip::{PaperclipClient, HeartbeatLoop, HeartbeatConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = PaperclipClient::new(
//!         "http://127.0.0.1:3100".into(),
//!         std::env::var("PAPERCLIP_API_KEY")?,
//!     );
//!     let identity = client.get_identity().await?;
//!     println!("Running as: {} ({})", identity.name, identity.id);
//!     Ok(())
//! }
//! ```

// Test code intentionally uses unwrap/expect/panic (a failed assertion should abort the
// test) and declares test-local items next to their use.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::items_after_statements
    )
)]
pub mod client;
pub mod heartbeat;
pub mod types;

pub use client::PaperclipClient;
pub use heartbeat::{HeartbeatConfig, HeartbeatLoop, TaskExecutor};
