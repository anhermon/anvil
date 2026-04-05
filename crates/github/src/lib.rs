//! GitHub webhook integration for Anvil — agent @mention support.
//!
//! Provides an Axum-based HTTP server that receives GitHub webhooks,
//! verifies HMAC-SHA256 signatures, detects agent @mentions in comments,
//! and creates Paperclip tasks for the mentioned agents.

// Pedantic lints that apply workspace-wide are acknowledged here.
// The github crate is intentionally pragmatic for v0; tighten later.
#![allow(clippy::pedantic)]

pub mod config;
pub mod events;
pub mod mention;
pub mod paperclip;
pub mod server;
pub mod signature;

pub use config::WebhookConfig;
pub use server::WebhookServer;
