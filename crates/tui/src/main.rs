//! harness-tui — interactive ratatui TUI for monitoring an Anvil agent.
//!
//! Connects to a running `harness-gateway` WebSocket endpoint and renders:
//! - Live event feed (turns, tokens, tool calls, tool results, errors)
//! - Selected-event detail panel
//! - Connection status bar
//!
//! # Usage
//!
//! ```bash
//! anvil-tui --gateway ws://127.0.0.1:9000/ws
//! ```
#![forbid(unsafe_code)]
#![allow(clippy::module_name_repetitions)]

mod app;
mod events;
mod gateway;
mod ui;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use tokio::sync::mpsc;

use app::App;
use events::AppEvent;

/// Monitor a live Anvil agent session via the harness-gateway WebSocket.
#[derive(Parser, Debug)]
#[command(name = "anvil-tui", version, about)]
struct Args {
    /// WebSocket URL of harness-gateway
    #[arg(long, default_value = "ws://127.0.0.1:9000/ws")]
    gateway: String,

    /// Maximum number of events to keep in memory
    #[arg(long, default_value_t = 500)]
    max_events: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "warn".into())
                .as_str(),
        )
        .with_writer(std::io::stderr)
        .init();

    // Channel: gateway task → TUI
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AppEvent>();

    // Spawn gateway connection task
    let gw_url = args.gateway.clone();
    let tx = event_tx.clone();
    tokio::spawn(async move {
        gateway::run_gateway_client(gw_url, tx).await;
    });

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the app
    let mut app = App::new(args.max_events, args.gateway, event_rx, event_tx);
    let result = app.run(&mut terminal).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}
