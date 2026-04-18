//! `anvil gateway` - local WebSocket control-plane gateway.

use anyhow::Result;
use clap::{Args, Subcommand};
use harness_gateway::{AgentEvent, Gateway, GatewayConfig};
use std::time::Duration;
use tokio::{signal, sync::mpsc};
use uuid::Uuid;

#[derive(Debug, Args)]
pub struct GatewayArgs {
    #[command(subcommand)]
    command: GatewayCommand,
}

#[derive(Debug, Subcommand)]
enum GatewayCommand {
    /// Start the local WebSocket gateway
    Serve(ServeArgs),
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// Port to listen on. Use 0 to bind an OS-selected free port.
    #[arg(long, default_value_t = 9000)]
    port: u16,

    /// Number of events buffered for each connected client.
    #[arg(long, default_value_t = 256, value_parser = parse_event_buffer)]
    event_buffer: usize,

    /// Emit a startup token event after the first WebSocket client connects.
    #[arg(long)]
    emit_hello: bool,
}

pub async fn execute(args: GatewayArgs) -> Result<()> {
    match args.command {
        GatewayCommand::Serve(args) => serve(args).await,
    }
}

async fn serve(args: ServeArgs) -> Result<()> {
    let config = GatewayConfig {
        port: args.port,
        event_buffer: args.event_buffer,
    };
    let mut handle = Gateway::new(config).start().await?;

    println!("gateway listening");
    println!("health: http://{}/health", handle.addr);
    println!("websocket: ws://{}/ws", handle.addr);

    let (_placeholder_tx, placeholder_rx) = mpsc::channel(1);
    let mut cmd_rx = std::mem::replace(&mut handle.cmd_rx, placeholder_rx);
    let command_drain = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            eprintln!("control command received: {cmd:?}");
        }
    });

    let hello_task = if args.emit_hello {
        Some(tokio::spawn({
            let handle = handle.clone_event_handle();
            async move {
                while handle.connected_clients() == 0 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                handle.emit(AgentEvent::turn_start(Uuid::new_v4())).await;
                handle.emit(AgentEvent::token("anvil gateway online")).await;
            }
        }))
    } else {
        None
    };

    signal::ctrl_c().await?;
    handle.shutdown().await;
    if let Some(task) = hello_task {
        task.abort();
    }
    command_drain.abort();
    Ok(())
}

fn parse_event_buffer(value: &str) -> std::result::Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| "event buffer must be a positive integer".to_string())?;
    if parsed == 0 {
        return Err("event buffer must be greater than zero".to_string());
    }
    Ok(parsed)
}
