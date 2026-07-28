// Test code intentionally uses unwrap/expect/panic: a failed assertion should abort the test.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
pub mod auth;
pub mod config;
pub mod error;
pub mod message;
pub mod provider;
pub mod providers;
pub mod session;
