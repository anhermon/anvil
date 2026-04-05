//! Rendering logic for the TUI.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{app::App, events::AgentEvent};

/// Top-level draw function.
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Outer layout: title bar + content + status bar
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(0),    // content
            Constraint::Length(1), // status
        ])
        .split(area);

    // Title bar
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " anvil-tui ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw("  "),
        Span::styled(&app.gateway_url, Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(
            "[j/k] navigate  [d/u] detail scroll  [G] end  [g] top  [q] quit",
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    f.render_widget(title, outer[0]);

    // Content: event list (left) + detail panel (right)
    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(outer[1]);

    draw_event_list(f, app, content[0]);
    draw_detail_panel(f, app, content[1]);

    // Status bar
    let status_text = format!(" {} events | {} ", app.events.len(), app.gateway_status);
    let status_color = match &app.gateway_status {
        crate::events::GatewayStatus::Connected => Color::Green,
        crate::events::GatewayStatus::Disconnected { .. } => Color::Red,
        _ => Color::Yellow,
    };
    let status = Paragraph::new(Line::from(Span::styled(
        status_text,
        Style::default().fg(status_color),
    )));
    f.render_widget(status, outer[2]);
}

fn draw_event_list(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    let items: Vec<ListItem> = app
        .events
        .iter()
        .enumerate()
        .map(|(i, event)| {
            let selected = app.selected == Some(i);
            let (label_color, label) = event_label_style(event);

            let ts = event.timestamp().format("%H:%M:%S").to_string();
            let summary = event.summary();

            let style = if selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };

            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{ts} "),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(if selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!("{label:<12} "),
                    Style::default()
                        .fg(label_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(summary, style),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    if let Some(sel) = app.selected {
        list_state.select(Some(sel));
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Events "))
        .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_stateful_widget(list, area, &mut list_state);
}

fn draw_detail_panel(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let content = match app.selected.and_then(|i| app.events.get(i)) {
        None => Text::from(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No event selected.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Use j/k to navigate the event list.",
                Style::default().fg(Color::DarkGray),
            )),
        ]),
        Some(event) => {
            let (color, label) = event_label_style(event);
            let detail = event.detail();
            let mut lines = vec![
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        label,
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        event
                            .timestamp()
                            .format("%Y-%m-%d %H:%M:%S UTC")
                            .to_string(),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::from(""),
            ];
            for raw_line in detail.lines() {
                lines.push(Line::from(format!(" {raw_line}")));
            }
            Text::from(lines)
        }
    };

    // Scroll: skip `detail_offset` lines
    let scrolled_content: Vec<Line> = content.lines.into_iter().skip(app.detail_offset).collect();

    let paragraph = Paragraph::new(Text::from(scrolled_content))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Detail  [d/u scroll] "),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

fn event_label_style(event: &AgentEvent) -> (Color, &'static str) {
    match event {
        AgentEvent::TurnStart { .. } => (Color::Cyan, "TURN_START"),
        AgentEvent::Token { .. } => (Color::White, "TOKEN"),
        AgentEvent::ToolCall { .. } => (Color::Yellow, "TOOL_CALL"),
        AgentEvent::ToolResult { .. } => (Color::Green, "TOOL_RESULT"),
        AgentEvent::TurnComplete { .. } => (Color::Blue, "TURN_COMPLETE"),
        AgentEvent::Error { .. } => (Color::Red, "ERROR"),
    }
}
