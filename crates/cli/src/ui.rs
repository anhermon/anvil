//! Terminal UI helpers: banner, spinner, tool call display, session header/summary.
//!
//! Style reference: claude-code / hermes-agent terminal output.
//!
//! Color palette:
//!   brand   — cyan  (`style(...).cyan()`)
//!   meta    — dim gray (`style(...).dim()`)
//!   success — green
//!   warning — yellow
//!   tool    — cyan bold

use console::{style, Term};
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ASCII_BANNER: &str = include_str!("assets/anvil.txt");

// ─────────────────────────────────────────────────────────────────────────────────
// Banner
// ─────────────────────────────────────────────────────────────────────────────────

/// Print the startup banner.
///
/// In a TTY: dimmed cyan block-art logo, bold name, italic tagline.
/// When piped/redirected: compact one-liner so logs stay parseable.
pub fn print_banner() {
    if Term::stderr().is_term() {
        eprintln!();
        for line in ASCII_BANNER.lines() {
            eprintln!("  {}", style(line).cyan().dim());
        }
        eprintln!();
        eprintln!(
            "  {}  {}",
            style("anvil").bold().cyan(),
            style(format!("v{VERSION}")).dim(),
        );
        eprintln!("  {}", style("forge your agents").dim().italic());
        eprintln!();
    } else {
        eprintln!(
            "  {} {}  v{VERSION}",
            style("▲").cyan().bold(),
            style("anvil").bold(),
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────────
// Session header
// ─────────────────────────────────────────────────────────────────────────────────

/// Print the compact session header.
///
/// ```text
///   ◆ session:abc12345  claude-3-haiku  via:claude
///   ──────────────────────────────────────────────────
/// ```
pub fn print_session_header(session_id: &str, model: &str, provider: &str) {
    let short_id = &session_id[..session_id.len().min(8)];
    eprintln!(
        "  {}  {}  {}  {}",
        style("◆").cyan(),
        style(format!("session:{short_id}")).dim(),
        style(model).dim(),
        style(format!("via:{provider}")).dim(),
    );
    eprintln!("  {}", style("─".repeat(50)).dim());
    eprintln!();
}

// ─────────────────────────────────────────────────────────────────────────────────
// Spinner
// ─────────────────────────────────────────────────────────────────────────────────

/// Create and start a braille-dot spinner.
///
/// ```text
///   ⠋  thinking [2/5]
/// ```
///
/// Pass a pre-formatted label (e.g. `"thinking [1/5]"`).
/// Call `.finish_and_clear()` on the returned `ProgressBar` when done.
pub fn thinking_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.cyan} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_message(format!("{}", style(msg).dim()));
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

// ─────────────────────────────────────────────────────────────────────────────────
// Tool call / result
// ─────────────────────────────────────────────────────────────────────────────────

/// Print a tool call line.
///
/// ```text
///   ⎿  read_file  "path/to/file.rs"
/// ```
pub fn print_tool_call(name: &str, input_preview: &str) {
    eprintln!(
        "  {}  {}  {}",
        style("⎿").cyan().dim(),
        style(name).cyan().bold(),
        style(input_preview).dim(),
    );
}

/// Print a tool result, truncated to 200 chars with newlines collapsed.
pub fn print_tool_result(output: &str) {
    let preview: String = output.chars().take(200).collect();
    let preview = preview.replace('\n', " ↵ ");
    eprintln!("    {}", style(&preview).dim());
    if output.len() > 200 {
        eprintln!("    {}", style(format!("… ({} chars)", output.len())).dim());
    }
}

// ─────────────────────────────────────────────────────────────────────────────────
// Response
// ─────────────────────────────────────────────────────────────────────────────────

/// Print the final assistant response with consistent left padding.
pub fn print_response(text: &str) {
    eprintln!();
    for line in text.lines() {
        eprintln!("  {line}");
    }
    eprintln!();
}

// ─────────────────────────────────────────────────────────────────────────────────
// Session summary
// ─────────────────────────────────────────────────────────────────────────────────

/// Print the closing session summary footer.
///
/// ```text
///   ╰─  3 iterations  ·  1.4s
/// ```
///
/// `tokens_in` and `tokens_out` are best-effort; pass 0 when unavailable.
pub fn print_session_summary(tokens_in: u32, tokens_out: u32, iterations: usize, elapsed_ms: u64) {
    let secs = elapsed_ms as f64 / 1000.0;
    let token_str = if tokens_in > 0 || tokens_out > 0 {
        format!("tokens {tokens_in}/{tokens_out}  ·  ")
    } else {
        String::new()
    };
    eprintln!(
        "  {}  {}{}  ·  {:.1}s",
        style("╰─").dim(),
        style(&token_str).dim(),
        style(format!(
            "{iterations} iteration{}",
            if iterations == 1 { "" } else { "s" }
        ))
        .dim(),
        secs,
    );
}
