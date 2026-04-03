# AGENTS.md — crates/core

This is the `harness-core` library crate. It owns the foundational types shared by every other
crate in the workspace. Read this file before making any changes here.

---

## Responsibilities

| Module          | What it owns                                                         |
|-----------------|----------------------------------------------------------------------|
| `provider.rs`   | `Provider` async trait, `EchoProvider` stub, `StreamChunk`, `TokenStream` |
| `providers/`    | Concrete provider implementations (`claude.rs`, …)                  |
| `message.rs`    | `Message`, `Role`, `MessageContent`, `ContentBlock`, `TurnResponse`, `Usage`, `StopReason` |
| `config.rs`     | `Config`, `ProviderConfig`, `MemoryConfig`, `AgentConfig`            |
| `session.rs`    | `Session`, `SessionStatus`                                           |
| `error.rs`      | `HarnessError` (thiserror), `Result<T>` alias                        |

---

## Strict constraints

- **This is a library crate.** Do not add a `main.rs`. Do not add `clap`, `tokio::main`, or any
  CLI dependency. The only runtime dependency allowed is `tokio` (as a dev-dependency for tests
  and via `async-trait` for the trait definition).
- Do not add dependencies that pull in a TLS stack unless they are already in the workspace (e.g.
  `reqwest` is already present for `ClaudeProvider`).
- Do not expose `ClaudeProvider` in a way that requires an API key at compile time. Construction
  must be deferred to runtime.

---

## Adding a new provider

1. Create `crates/core/src/providers/<name>.rs`.
2. Implement the `Provider` trait. Minimum required methods: `name()` and `complete()`.
3. `complete()` must always populate `TurnResponse.usage` — even if the backend does not report
   token counts, set them to 0 rather than leaving them unset.
4. Re-export the new provider from `crates/core/src/providers/mod.rs`.
5. Add at least one unit test in the same file. The test must not make a real network call.
   Use conditional compilation (`#[cfg(test)]`) and mock the HTTP layer, or write a test that
   instantiates the provider with a fake key and asserts construction succeeds without panicking.

---

## The Provider trait

```rust
#[async_trait]
pub trait Provider: Send + Sync + 'static {
    fn name(&self) -> &str;
    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse>;
    async fn stream(&self, messages: &[Message]) -> Result<TokenStream>;  // default impl provided
    fn context_limit(&self) -> usize;                                     // default: 200_000
}
```

`TurnResponse` always carries:
- `message: Message` — the assistant reply
- `stop_reason: StopReason` — `EndTurn | MaxTokens | ToolUse | StopSequence`
- `usage: Usage` — **required** (`input_tokens`, `output_tokens`, optional cache fields)
- `model: String` — the model identifier string as returned by the API

---

## EchoProvider

`EchoProvider` is the canonical test double. It mirrors the last user message back with an
`"echo: "` prefix. It never touches the network.

**Rule: all tests in this crate and in any other crate that exercises the provider pipeline must
use `EchoProvider`. Never use `ClaudeProvider` in tests. There is no API key in CI.**

```rust
use harness_core::provider::EchoProvider;
let p = EchoProvider;
let resp = p.complete(&[Message::user("ping")]).await.unwrap();
assert_eq!(resp.message.text(), Some("echo: ping"));
```

---

## Message types

- `Role::System` messages are sent as the top-level `system` field in the Anthropic API, not as
  a conversation turn. Providers other than Claude must handle this appropriately.
- `MessageContent::Blocks` carries structured content (text blocks, tool use, tool results). Prefer
  `MessageContent::Text` for simple string content to avoid unnecessary allocations.
- `TurnResponse.usage` is always present. Tests must assert on usage fields when verifying
  provider behaviour.

---

## Config

- `Config::load()` reads `~/.paperclip/harness/config.toml` and falls back to `Config::default()`
  if the file does not exist.
- API keys are never stored in `Config` in source or tests. `Config::resolved_api_key()` reads
  from the config file first, then from environment variables.
- Do not add new top-level config sections without updating `Config::default()`.

---

## Testing

- Use `#[tokio::test]` for async tests.
- Use `EchoProvider` for all provider-pipeline tests.
- Do not create temp files. If a test needs a config, construct `Config::default()` directly.
- Test module convention: `#[cfg(test)] mod tests { ... }` at the bottom of each source file.
