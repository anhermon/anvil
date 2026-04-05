pub mod auth;
pub mod config;
pub mod eval;
pub mod memory;
pub mod run;

/// Return a contextual fallback suggestion when a provider fails.
///
/// The hint tells the user which alternative provider or env var to try,
/// based on which backend just failed.
pub(crate) fn provider_fallback_hint(backend: &str) -> Option<&'static str> {
    match backend {
        "claude-code" | "cc" => Some(
            "Hint: Try --provider claude with ANTHROPIC_API_KEY set",
        ),
        "echo" => None, // echo never needs a fallback
        // "claude" or any direct-API backend
        _ => Some(
            "Hint: Set ANTHROPIC_API_KEY or try --provider echo for testing",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_hint_cc_suggests_claude() {
        let hint = provider_fallback_hint("cc").unwrap();
        assert!(hint.contains("--provider claude"));
        assert!(hint.contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn fallback_hint_claude_code_suggests_claude() {
        let hint = provider_fallback_hint("claude-code").unwrap();
        assert!(hint.contains("--provider claude"));
    }

    #[test]
    fn fallback_hint_claude_suggests_api_key_or_echo() {
        let hint = provider_fallback_hint("claude").unwrap();
        assert!(hint.contains("ANTHROPIC_API_KEY"));
        assert!(hint.contains("--provider echo"));
    }

    #[test]
    fn fallback_hint_echo_returns_none() {
        assert!(provider_fallback_hint("echo").is_none());
    }
}
