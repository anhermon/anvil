# AGENTS.md — crates/cli

This is the `harness` binary crate. It owns the `Agent` loop, all CLI subcommands, and the
user-facing harness entry point. Changes here are user-visible and directly affect agent behaviour.
Read this file carefully before modifying the agent loop or adding a subcommand.

---

## Responsibilities

| File / Module          | What it owns                                                      |
|------------------------|-------------------------------------------------------------------|
| `main.rs`              | Clap derive root, tracing setup, async entry point                |
| `agent.rs`             | `Agent` struct and `Agent::run()` — the core agentic loop         |
| `commands/run.rs`      | `harness run --goal` — provider selection, session execution      |
| `commands/config.rs`   | `harness config` — inspect/validate config                        |
| `commands/memory.rs`   | `harness memory search/list/purge` — memory introspection         |
| `commands/mod.rs`      | Subcommand enum; registers all commands                           |

---

## The Agent loop (`Agent::run`)

`Agent::run()` is the **core agentic loop**. Every change to this function affects all user-facing
agent behaviour. Treat it with corresponding care.

Current loop (Phase 2 / v0 single-turn):
1. Build the message list: system prompt + user goal.
2. Loop up to `config.agent.max_iterations` times.
3. Call `provider.complete(&messages)`.
4. Append the assistant reply to the session and persist to memory.
5. (v0) Stop after the first turn. Tool call loop is planned for Phase 3.

Rules for modifying the loop:
- **Always respect `max_iterations`.** This is the safety bound that prevents runaway agents. Do
  not add any code path that allows the loop to run longer than `max_iterations` turns.
- When `max_iterations` is reached, set `session.finish(SessionStatus::Done)` and break. Do not
  silently continue.
- If `config.agent.max_iterations == 0`, treat it as unlimited (`usize::MAX`) — this is the
  current behaviour and must be preserved.
- Every assistant turn must be persisted to `MemoryDb` as an `EpisodeKind::Turn` episode before
  the loop iterates or breaks.
- Log iteration number and output token count at `debug` / `info` level on every turn.

---

## Adding a new subcommand

1. Create `crates/cli/src/commands/<name>.rs`.
2. Define an `Args` struct with `#[derive(clap::Args)]` and an `async fn execute(args: Args)`.
3. Add the new command to the `Commands` enum in `crates/cli/src/commands/mod.rs`.
4. Add a match arm in `main.rs` (or wherever dispatch lives) to call `execute`.
5. Write integration tests using `ScriptedProvider` (see Testing below).

Keep subcommands small. Business logic belongs in `Agent`, `MemoryDb`, or library crates — not in
command handlers. Command handlers should be thin wires.

---

## Provider selection (run command)

The `run` command selects a provider at startup:

```rust
match backend.as_str() {
    "echo" => Arc::new(EchoProvider),
    "claude" | _ => Arc::new(ClaudeProvider::new(api_key, model, max_tokens)),
}
```

Rules:
- The `echo` provider must always be selectable without any environment variables.
- When `claude` (or default) is selected and `ANTHROPIC_API_KEY` is not set, return a clear error
  message — do not panic.
- New providers are added here as additional match arms after implementing the `Provider` trait in
  `crates/core/src/providers/`.

---

## ScriptedProvider — test fixture for the agent loop

`ScriptedProvider` is the test double for agent loop tests. It returns pre-configured responses
in sequence, allowing deterministic testing of multi-turn behaviour without any API calls.

Use it for all tests that exercise `Agent::run()` or command handlers that invoke the agent loop:

```rust
// Conceptual usage (implement ScriptedProvider if not yet present):
let provider = ScriptedProvider::new(vec![
    "first response",
    "second response",
]);
let agent = Agent::new(Arc::new(provider), Arc::new(MemoryDb::in_memory().await?), config);
let session = agent.run("test goal").await?;
assert_eq!(session.messages.len(), 2); // user + assistant
```

Rules:
- **Never use `ClaudeProvider` or any real API provider in tests.**
- **Never use `EchoProvider` for multi-turn tests** where you need specific response content.
  Use `ScriptedProvider` instead.
- Use `MemoryDb::in_memory()` for all agent tests.
- Do not write to the file system in tests.

---

## Config in tests

Do not read from `~/.paperclip/harness/config.toml` in tests. Construct config directly:

```rust
let config = Config {
    agent: AgentConfig { max_iterations: 3, ..Default::default() },
    ..Config::default()
};
```

This ensures tests are hermetic and do not depend on the developer's local config file.

---

## Tracing / logging

- Set up tracing in `main.rs` using `tracing-subscriber` with `EnvFilter` (respect `RUST_LOG`).
- Use structured fields: `info!(session_id = %id, goal = %goal, "starting session")`.
- Do not log full message content at INFO level. Use DEBUG.
- Tests must not emit tracing output unless `RUST_LOG` is set. Do not call `tracing_subscriber::init()`
  in test code.

---

## Constraints summary

| Rule | Detail |
|------|--------|
| Respect `max_iterations` | Never add a code path that exceeds the configured iteration bound |
| Persist every turn | Every assistant message must be written to `MemoryDb` before loop continues |
| No real API calls in tests | Use `ScriptedProvider` or `EchoProvider` only |
| No file system writes in tests | Use `MemoryDb::in_memory()` and construct `Config` directly |
| Thin command handlers | Business logic belongs in library crates, not in command files |
| New commands in own file | `commands/<name>.rs`, registered in `commands/mod.rs` |
| `echo` always works without env vars | Must be selectable with zero environment setup |
