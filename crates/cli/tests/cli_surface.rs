//! CLI surface regression tests.
//!
//! These tests ensure built artifacts expose expected top-level subcommands,
//! so dogfood workflows do not depend on stale binaries.

use std::process::Command;

fn run_anvil(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_anvil"))
        .args(args)
        .output()
        .expect("failed to run anvil binary")
}

#[test]
fn top_level_help_lists_paperclip_subcommand() {
    let output = run_anvil(&["--help"]);
    assert!(
        output.status.success(),
        "anvil --help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("paperclip"),
        "expected `paperclip` in anvil --help output, got:\n{stdout}"
    );
}

#[test]
fn top_level_help_lists_gateway_subcommand() {
    let output = run_anvil(&["--help"]);
    assert!(
        output.status.success(),
        "anvil --help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("gateway"),
        "expected `gateway` in anvil --help output, got:\n{stdout}"
    );
}

#[test]
fn paperclip_subcommand_help_is_available() {
    let output = run_anvil(&["paperclip", "--help"]);
    assert!(
        output.status.success(),
        "anvil paperclip --help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn gateway_subcommand_help_is_available() {
    let output = run_anvil(&["gateway", "--help"]);
    assert!(
        output.status.success(),
        "anvil gateway --help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn gateway_serve_rejects_zero_event_buffer() {
    let output = run_anvil(&["gateway", "serve", "--event-buffer", "0"]);
    assert!(
        !output.status.success(),
        "anvil gateway serve accepted zero event buffer"
    );
}
