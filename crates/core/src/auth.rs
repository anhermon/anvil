//! Subscription-based auth resolution for the Anthropic API.
//!
//! Priority order:
//!   1. Bearer token from `~/.claude/.credentials.json` (or `CLAUDE_CONFIG_DIR` override)
//!   2. `ANTHROPIC_API_KEY` environment variable
//!   3. Error with a helpful message

use std::path::PathBuf;

use anyhow::Context;

/// How the harness authenticates to the Anthropic API.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// API key read from `ANTHROPIC_API_KEY` env var.
    ApiKey(String),
    /// OAuth Bearer token from `~/.claude/.credentials.json`.
    BearerToken {
        token: String,
        /// Absolute path of the credentials file where the token was found.
        source: PathBuf,
    },
}

impl AuthMethod {
    /// Resolve the best available auth method.
    ///
    /// Order: subscription credentials file -> `ANTHROPIC_API_KEY` env var -> error.
    pub fn resolve() -> anyhow::Result<Self> {
        // Step 1 -- try the Claude credentials file.
        if let Some(result) = Self::try_credentials_file()? {
            return Ok(result);
        }

        // Step 2 -- fall back to ANTHROPIC_API_KEY.
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                return Ok(Self::ApiKey(key));
            }
        }

        anyhow::bail!(
            "No Anthropic credentials found.\n\
             Option A -- Log in with Claude Code: `claude auth login` (writes ~/.claude/.credentials.json)\n\
             Option B -- Set the ANTHROPIC_API_KEY environment variable."
        )
    }

    /// Apply the correct auth header to a `reqwest::RequestBuilder`.
    pub fn apply(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::ApiKey(key) => builder.header("x-api-key", key),
            Self::BearerToken { token, .. } => {
                builder.header("Authorization", format!("Bearer {token}"))
            }
        }
    }

    /// Human-readable one-line description (used by `harness auth status`).
    pub fn describe(&self) -> String {
        match self {
            Self::ApiKey(_) => "API key (from ANTHROPIC_API_KEY env var)".to_string(),
            Self::BearerToken { source, .. } => {
                format!("subscription (Bearer token) from {}", source.display())
            }
        }
    }

    // -- internals -------------------------------------------------------------

    /// Attempt to read a Bearer token from the Claude credentials file.
    ///
    /// Returns `Ok(None)` if the file simply doesn't exist; returns `Err` only
    /// for I/O or parse failures on a file that _does_ exist.
    fn try_credentials_file() -> anyhow::Result<Option<Self>> {
        let config_dir = claude_config_dir();

        // Try `.credentials.json` first, then the bare `credentials.json` fallback.
        let candidates = [
            config_dir.join(".credentials.json"),
            config_dir.join("credentials.json"),
        ];

        for path in &candidates {
            if !path.exists() {
                continue;
            }

            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?;

            let parsed: serde_json::Value = serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse JSON from {}", path.display()))?;

            if let Some(token) = parsed
                .get("claudeAiOauth")
                .and_then(|o| o.get("accessToken"))
                .and_then(|t| t.as_str())
            {
                if !token.is_empty() {
                    return Ok(Some(Self::BearerToken {
                        token: token.to_string(),
                        source: path.clone(),
                    }));
                }
            }
        }

        Ok(None)
    }
}

/// Return the Claude config directory.
///
/// `CLAUDE_CONFIG_DIR` env var overrides the default of `~/.claude`.
fn claude_config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }

    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".claude")
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var("USERPROFILE").ok().map(PathBuf::from);
    #[cfg(not(windows))]
    return std::env::var("HOME").ok().map(PathBuf::from);
}

// -- tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Create a uniquely-named subdirectory under `std::env::temp_dir()`.
    fn make_temp_dir(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("harness-auth-test-{suffix}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// Write a `.credentials.json` file into `dir` with the given access token.
    fn write_creds(dir: &std::path::Path, access_token: &str) {
        let path = dir.join(".credentials.json");
        let json = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": access_token
            }
        });
        let mut f = std::fs::File::create(path).expect("create creds file");
        write!(f, "{json}").expect("write creds file");
    }

    #[test]
    fn auth_resolve_prefers_bearer_when_both_present() {
        let dir = make_temp_dir("bearer-wins");
        write_creds(&dir, "sk-ant-bearer-token");

        std::env::set_var("CLAUDE_CONFIG_DIR", dir.to_str().expect("utf8 path"));
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-apikey-should-be-ignored");

        let auth = AuthMethod::resolve().expect("resolve should succeed");

        std::env::remove_var("CLAUDE_CONFIG_DIR");
        std::env::remove_var("ANTHROPIC_API_KEY");
        let _ = std::fs::remove_dir_all(&dir);

        match auth {
            AuthMethod::BearerToken { token, .. } => {
                assert_eq!(token, "sk-ant-bearer-token");
            }
            AuthMethod::ApiKey(_) => panic!("expected BearerToken, got ApiKey"),
        }
    }

    #[test]
    fn auth_resolve_falls_back_to_api_key_when_no_creds_file() {
        let dir = make_temp_dir("apikey-fallback");
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.to_str().expect("utf8 path"));
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-fallback-key");

        let auth = AuthMethod::resolve().expect("resolve should succeed");

        std::env::remove_var("CLAUDE_CONFIG_DIR");
        std::env::remove_var("ANTHROPIC_API_KEY");
        let _ = std::fs::remove_dir_all(&dir);

        match auth {
            AuthMethod::ApiKey(key) => {
                assert_eq!(key, "sk-ant-fallback-key");
            }
            AuthMethod::BearerToken { .. } => panic!("expected ApiKey, got BearerToken"),
        }
    }
}
