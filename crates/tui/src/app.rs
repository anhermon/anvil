//! Application state and main event loop.

use std::collections::VecDeque;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{backend::Backend, Terminal};
use tokio::sync::mpsc;

use crate::events::{AgentEvent, AppEvent, GatewayStatus};
use crate::ui;

/// Application state.
pub struct App {
    /// Capped event history
    pub events: VecDeque<AgentEvent>,
    pub max_events: usize,
    /// Scroll offset in the event list
    pub list_offset: usize,
    /// Currently selected event index
    pub selected: Option<usize>,
    /// Current gateway URL
    pub gateway_url: String,
    /// Current gateway connection status
    pub gateway_status: GatewayStatus,
    /// Scroll offset in the detail panel
    pub detail_offset: usize,

    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
}

impl App {
    pub fn new(
        max_events: usize,
        gateway_url: String,
        event_rx: mpsc::UnboundedReceiver<AppEvent>,
        event_tx: mpsc::UnboundedSender<AppEvent>,
    ) -> Self {
        Self {
            events: VecDeque::with_capacity(max_events),
            max_events,
            list_offset: 0,
            selected: None,
            gateway_url,
            gateway_status: GatewayStatus::Connecting,
            detail_offset: 0,
            event_rx,
            event_tx,
        }
    }

    /// Main event loop. Drives both terminal input and gateway events.
    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        loop {
            // Draw
            terminal.draw(|f| ui::draw(f, self))?;

            // Poll for events — give terminal input priority, but don't block long
            // so gateway events are still processed promptly.
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if let Some(ev) = self.handle_key(key.code, key.modifiers) {
                        let _ = self.event_tx.send(ev);
                    }
                }
            }

            // Drain channel (non-blocking)
            loop {
                match self.event_rx.try_recv() {
                    Ok(AppEvent::Quit) => return Ok(()),
                    Ok(AppEvent::Key(k)) => {
                        if let Some(ev) = self.handle_key(k.code, k.modifiers) {
                            if matches!(ev, AppEvent::Quit) {
                                return Ok(());
                            }
                        }
                    }
                    Ok(AppEvent::Agent(agent_event)) => {
                        self.push_event(agent_event);
                    }
                    Ok(AppEvent::GatewayStatus(status)) => {
                        self.gateway_status = status;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => return Ok(()),
                }
            }
        }
    }

    /// Push a new agent event, capping history at `max_events`.
    fn push_event(&mut self, event: AgentEvent) {
        if self.events.len() >= self.max_events {
            self.events.pop_front();
            // Adjust selection/offset if needed
            if let Some(sel) = self.selected {
                self.selected = sel.checked_sub(1);
            }
            if self.list_offset > 0 {
                self.list_offset -= 1;
            }
        }
        self.events.push_back(event);
    }

    /// Handle a key press. Returns an `AppEvent` to enqueue, or None.
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<AppEvent> {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return Some(AppEvent::Quit),
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(AppEvent::Quit);
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            KeyCode::PageDown => {
                for _ in 0..10 {
                    self.select_next();
                }
            }
            KeyCode::PageUp => {
                for _ in 0..10 {
                    self.select_prev();
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = if self.events.is_empty() {
                    None
                } else {
                    Some(0)
                };
                self.list_offset = 0;
                self.detail_offset = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                if !self.events.is_empty() {
                    self.selected = Some(self.events.len() - 1);
                    self.list_offset = self.events.len().saturating_sub(1);
                }
                self.detail_offset = 0;
            }
            KeyCode::Char('d') => {
                self.detail_offset = self.detail_offset.saturating_add(5);
            }
            KeyCode::Char('u') => {
                self.detail_offset = self.detail_offset.saturating_sub(5);
            }
            _ => {}
        }
        None
    }

    fn select_next(&mut self) {
        if self.events.is_empty() {
            self.selected = None;
            return;
        }
        self.selected = Some(match self.selected {
            None => 0,
            Some(i) => (i + 1).min(self.events.len() - 1),
        });
        self.detail_offset = 0;
    }

    fn select_prev(&mut self) {
        if self.events.is_empty() {
            self.selected = None;
            return;
        }
        self.selected = Some(match self.selected {
            None => 0,
            Some(i) => i.saturating_sub(1),
        });
        self.detail_offset = 0;
    }
}
