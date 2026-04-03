//! Terminal UI helpers: spinner, colored tool call display, session header and summary.
//!
//! Style reference: claude-code / hermes-agent terminal output.

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ASCII_BANNER: &str = include_str!("assets/anvil.txt");

/// Print the startup banner with ASCII art when stderr is a TTY.
///
/// When connected to a terminal prints the full ASCII art anvil logo followed
/// by the "anvil vX.Y.Z" byline.  When piped / redirected, prints a compact
/// single-line header so logs stay parseable.
pub fn print_banner() {
    if console::Term::stderr().is_term() {
        // Print each line of the ASCII art dimmed so it doesn't overpower output.
        for line in ASCII_BANNER.lines() {
            eprintln!("  {}", style(line).dim());
        }
        eprintln!(
            "        {} {}  v{VERSION}\n",
            style("anvil").bold(),
            style("—").dim(),
        );
        eprintln!("  {}", style("─".repeat(41)).dim());
    } else {
        eprintln!(
            "\n  {} {}  v{VERSION}",
            style("▲").cyan().bold(),
            style("anvil").bold(),
        );
        eprintln!("  {}", style("─".repeat(41)).dim());
    }
}

/// Print the session header with session ID, model and provider.
///
/// ```text
///   ◆ session: abc12345  model: claude-3-haiku  provider: claude
/// ```
pub fn print_session_header(session_id: &str, model: &str, provider: &str) {
    let short_id = &session_id[..session_id.len().min(8)];
    eprintln!(
        "  {}  session: {}  model: {}  provider: {}",
        style("◆").cyan(),
        style(short_id).dim(),
        style(model).dim(),
        style(provider).dim(),
    );
    eprintln!("  {}\n", style("─".repeat(41)).dim());
}

/// Create and start a braille-dot spinner for "thinking…" while waiting for the API.
///
/// Call `.finish_and_clear()` on the returned bar once the response arrives.
pub fn thinking_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.cyan.dim} {msg:.dim}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Print a tool call line.
///
/// ```text
///   ⯾  read_file  path/to/file.rs
/// ```
pub fn print_tool_call(name: &str, input_preview: &str) {
    eprintln!(
        "  {}  {}  {}",
        style("⎿").cyan().dim(),
        style(name).cyan(),
        style(input_preview).dim(),
    );
}

/// Print a tool result, truncated to 120 chars.
pub fn print_tool_result(output: &str) {
    let truncated = output.len() > 120;
    let preview: String = output
        .chars()
        .take(120)
        .collect::<String>()
        .replace('\n', " ↵ ");
    if truncated {
        eprintln!(
            "       {}  {}",
            style(preview).dim(),
            style("[truncated]").dim()
        );
    } else {
        eprintln!("       {}", style(preview).dim());
    }
}

/// Print the final assistant response.
pub fn print_response(text: &str) {
    println!("\n{}", text);
}

/// Print the session summary line.
///
/// ```text
///   tokens: 312 in / 78 out  |  3 iterations  |  1.4s
/// ```
pub fn print_session_summary(tokens_in: u32, tokens_out: u32, iterations: usize, elapsed_ms: u64) {
    let secs = elapsed_ms as f64 / 1000.0;
    eprintln!(
        "\n  {}",
        style(format!(
            "tokens: {tokens_in} in / {tokens_out} out  |  {iterations} iteration{}  |  {secs:.1}s",
            if iterations == 1 { "" } else { "s" }
        ))
        .dim()
    );
}
