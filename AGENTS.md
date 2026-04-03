# AGENTS.md — paperclip-harness

This file is read by AI coding agents (Claude Code, Codex, Cursor, etc.) at the start of every
session. Follow these instructions precisely. They override any default model behaviour.

---

## What this project is

`paperclip-harness` is a **self-bootstrapping agent harness written in Rust**. It is not a chatbot
framework. It is an opinionated runtime that:

1. Accepts a goal from the user.
2. Plans, selects tools, and executes turns in a loop.
3. Stores every turn in episodic memory (SQLite + FTS5).
4. Stays under a configurable iteration budget (safety bound).
5. Exposes hooks for human oversight via a Paperclip-compatible control plane (planned Phase 6).

The binary is `harness`. Configuration lives at `~/.paperclip/harness/config.toml`.

---

## Workspace architecture (4 crates)

```
crates/
├── core/      Provider trait, Message types, Config, Session, turn loop primitives
├── tools/     ToolRegistry, ToolHandler trait, JSON Schema validation, built-in tools
├── memory/    SQLite + FTS5 episodic memory (MemoryDb), Episode types
└── cli/       clap CLI binary: Agent loop, subcommands (run, config, memory)
```

Planned crates (do not add without a tracking issue): `task`, `orchestrator`, `ui`.

Dependency direction is strict: `cli` → `core + tools + memory`. `tools` and `memory` must not
depend on each other. `core` has no workspace-crate dependencies.

---

## Development commands

```bash
# Build all crates
cargo build

# Build the release binary
cargo build --release

# Run the full test suite (no API key required — uses EchoProvider)
cargo test

# Type-check without full compilation (fast)
cargo check

# Lint — must be clean before any commit
cargo clippy -- -D warnings

# Format check — must pass before any commit
cargo fmt --check

# Apply formatting
cargo fmt

# Run with the echo provider (CI-safe, no API key)
cargo run -- run --provider echo --goal "hello"

# Run with Claude (requires ANTHROPIC_API_KEY)
export ANTHROPIC_API_KEY=sk-ant-...
cargo run -- run --goal "summarise the current directory"
```

All CI checks must pass locally before pushing: `cargo test`, `cargo clippy -- -D warnings`,
`cargo fmt --check`.

---

## Testing philosophy

- **Never make real API calls in tests.** Use `EchoProvider` from `harness-core` for all unit and
  integration tests that exercise the agent pipeline.
- **Never write to the real filesystem in tests.** Use `MemoryDb::in_memory()` for all tests that
  touch the memory layer.
- Unit tests live as `#[cfg(test)]` modules inside the source file they test.
- Integration tests go in `tests/` at the crate root.
- Test function names should describe the behaviour being asserted, not the implementation.

---

## Provider trait

Every LLM backend implements the `Provider` trait defined in `crates/core/src/provider.rs`:

```rust
pub trait Provider: Send + Sync + 'static {
    fn name(&self) -> &str;
    async fn complete(&self, messages: &[Message]) -> Result<TurnResponse>;
    async fn stream(&self, messages: &[Message]) -> Result<TokenStream> { ... } // optional override
    fn context_limit(&self) -> usize { 200_000 } // optional override
}
```

Rules:
- `complete()` must always populate `TurnResponse.usage` (input + output tokens).
- `name()` should return the model identifier string, not a display name.
- New providers go in `crates/core/src/providers/<name>.rs` and are re-exported from
  `crates/core/src/providers/mod.rs`.
- CI must not require any API key; new providers must have a test that uses `EchoProvider` or a
  scripted mock.

---

## Tool implementation

New tools implement `ToolHandler` from `crates/tools/src/registry.rs`:

```rust
pub trait ToolHandler: Send + Sync + 'static {
    fn schema(&self) -> ToolSchema;
    async fn call(&self, input: Value) -> ToolOutput;
}
```

Rules:
- `schema()` must return a valid `ToolSchema` with `name`, `description`, and `parameters`.
- Input validation via `schema.validate()` is called automatically by `ToolRegistry::call()` before
  dispatching. Do not duplicate validation inside `call()`.
- Tools that read or write files **must sandbox paths** to `~/.paperclip/workspace/` or the process
  CWD. Reject any path that resolves outside these roots.
- Do not import an async runtime inside a tool. Tools receive their tokio context from the caller.
- Register new built-in tools in `crates/tools/src/builtin.rs` and add them to `Agent::new()` in
  `crates/cli/src/agent.rs`.

---

## Security policy

- File-reading and file-writing tools must sandbox to `~/.paperclip/workspace/` or CWD.
  Canonicalize paths and reject any traversal (`..`) that escapes the sandbox root.
- Do not store API keys in source files or tests. Read from environment variables only.
- Do not log full message content at INFO level or below. Use DEBUG with appropriate guards.
- `unsafe` code requires a written justification in the PR body. Most use cases do not need it.

---

## Coding standards

- Minimum Rust edition: **2021**. Minimum toolchain: **1.75**.
- `cargo clippy -- -D warnings` must be clean — zero warnings allowed.
- `cargo fmt` must produce no diff.
- Use `anyhow` for application-level errors in `cli`. Use `thiserror` for typed errors in library
  crates (`core`, `tools`, `memory`).
- Avoid `unwrap()` in non-test code. Propagate errors with `?`.
- Prefer `tracing::{debug, info, warn, error}` over `println!` for runtime output.

---

## Commit format

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <short description>

[optional body]

Co-Authored-By: Paperclip <noreply@paperclip.ing>
```

Types: `feat`, `fix`, `docs`, `chore`, `refactor`, `test`, `perf`
Scopes: `core`, `tools`, `memory`, `cli`, `task`, `orchestrator`, `ui`, `deps`

AI-agent commits **must** include the `Co-Authored-By` trailer.

---

## Branching policy

| Pattern   | Purpose                                   |
|-----------|-------------------------------------------|
| `master`  | Stable. Protected. Merge via reviewed PR. |
| `feat/*`  | New features. Branch from `master`.       |
| `fix/*`   | Bug fixes.                                |
| `docs/*`  | Documentation only.                       |
| `chore/*` | Deps, CI, tooling.                        |

**Never push directly to `master`.** Always open a PR, even for documentation changes.

---

## Roadmap context

The project is in **Phase 3** (tool call loop, streaming, eval harness, task DAG). Phases 4–6
(sub-agent orchestration, self-evolution, control plane) are planned but not yet implemented.
Do not add stubs or placeholder crates for unimplemented phases without a tracking issue.

---

## Authentication

The harness resolves Anthropic credentials in this priority order:

1. **Subscription token** — reads `~/.claude/.credentials.json` (or `~/.claude/credentials.json`)
   looking for `claudeAiOauth.accessToken`. The directory can be overridden with `CLAUDE_CONFIG_DIR`.
   This is the same credentials file written by `claude auth login` (Claude Code) and compatible
   tools such as opencode and hermes-agent.
2. **API key** — `ANTHROPIC_API_KEY` environment variable, sent as `x-api-key` header.
3. **Error** — if neither source yields a non-empty credential, the harness exits with a helpful
   message pointing to both options.

Use `harness auth status` to inspect which method is active in the current environment.
