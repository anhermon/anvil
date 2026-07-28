//! GitHub webhook integration for Anvil — agent @mention support.
//!
//! Provides an Axum-based HTTP server that receives GitHub webhooks,
//! verifies HMAC-SHA256 signatures, detects agent @mentions in comments,
//! and creates Paperclip tasks for the mentioned agents.

// Test code intentionally uses unwrap/expect/panic: a failed assertion should abort the test.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
pub mod config;
pub mod events;
pub mod mention;
pub mod paperclip;
pub mod server;
pub mod signature;

pub use config::WebhookConfig;
pub use server::WebhookServer;
