# Anvil

> A Rust-native, self-bootstrapping agent harness. One binary. Any LLM. Full autonomy with comfortable human override.

**Status:** v0.1.0 release candidate — Phases 1–7b complete. WebSocket gateway PR pending merge; TUI and public release next.

[![CI](https://github.com/anhermon/anvil/actions/workflows/ci.yml/badge.svg)](https://github.com/anhermon/anvil/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Rust 1.75+](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

---

## What is Anvil?

Most agent frameworks bolt autonomy onto a chat loop. Anvil is designed from first principles as a **self-bootstrapping agent OS**:

1. Run `anvil run --goal "..."` — configure your LLM provider once, describe what you want done.
2. The agent plans its tasks, builds the skills it needs, and spawns sub-agents to execute them.
3. After each session it reflects, critiques its own outputs, and evolves its prompts and skills — the 5-gate self-evolution engine.
4. Watch, approve, and redirect via a Paperclip-compatible control plane over WebSocket — intervene only when you want to.

The result is a system that **compounds in capability over time**, learns from every session, and stays legible to its operators.

---

## Architecture

Eight layers, bottom-up:

```
┌──────────────────────────────────────────────────┐
│  8. Control Plane / UI                           │
│     WebSocket gateway (harness-gateway) ✅        │
│     ratatui TUI (harness-tui) — next             │
│     Paperclip API adapter (harness-paperclip) ✅  │
├──────────────────────────────────────────────────┤
│  7. Self-Evolution Engine                        │
│     observe → critique → generate → validate ✅  │
│     5-gate minority-veto · prompt/skill rollback │
├──────────────────────────────────────────────────┤
│  6. Memory System                                │
│     SQLite + FTS5 episodic recall ✅              │
│     PARA-structured knowledge base ✅             │
│     SQLite-vec semantic recall — planned          │
├──────────────────────────────────────────────────┤
│  5. Sub-agent Orchestration                      │
│     tokio tasks · session-type sandbox ✅         │
│     MAX_SUBAGENT_DEPTH=4 guard ✅                 │
├──────────────────────────────────────────────────┤
│  4. Tool & Skill System                          │
│     registry dispatch · JSON-schema validation ✅ │
│     bash exec · file read/write · GitHub API ✅   │
│     dynamic MCP registration — planned           │
├──────────────────────────────────────────────────┤
│  3. Agent Core                                   │
│     provider-agnostic LLM trait ✅                │
│     async turn loop · tool-call loop ✅           │
│     capability-gated permissions (type-state) ✅  │
├──────────────────────────────────────────────────┤
│  2. Bootstrap Layer                              │
│     provider config at first run ✅               │
│     context/goal elicitation · auth resolution ✅ │
└──────────────────────────────────────────────────┘
```

### Crate layout

```
crates/
├── core/        Provider trait, message types, config, session, turn loop
├── tools/       Tool registry, JSON-schema validation, built-in tools
│                (BashExec, FileRead, FileWrite, GitHub search)
├── memory/      SQLite + FTS5 episodic memory (sqlx), semantic recall
├── evolution/   5-gate self-evolution engine: observe→critique→generate→validate→apply
│                Prompt/skill/config versioning with rollback
├── paperclip/   Paperclip control-plane client + heartbeat adapter
│                anvil paperclip — polls inbox, checks out tasks, runs via harness-core
├── github/      GitHub API client (search, file ops, PR management)
├── gateway/     WebSocket control-plane: streams agent events, accepts control commands
│                (AgentEvent, ControlCommand; broadcast fan-out; /health HTTP endpoint)
│                [PR #36 — pending merge]
└── cli/         clap derive CLI: anvil run / config / memory / eval / paperclip
```

---

## Installation

```bash
# Install from source (requires Rust 1.75+)
git clone https://github.com/anhermon/anvil
cd anvil
cargo install --path crates/cli

# Then run from anywhere:
anvil --help
```

Set `ANTHROPIC_API_KEY` or reuse an existing Claude Code session — Anvil resolves credentials in this order:

1. `ANTHROPIC_API_KEY` env var
2. `claude_code` bearer token from `~/.claude/` config
3. Interactive prompt

---

## Usage

```bash
# Run an agent session with a goal
anvil run --goal "Summarise the current directory"

# Use the echo provider — no API key, fast, CI-safe
anvil run --provider echo --goal "hello"

# Run as a Paperclip heartbeat agent (polls inbox and executes assigned tasks)
export PAPERCLIP_API_KEY=...
export PAPERCLIP_API_URL=http://localhost:3100
anvil paperclip --agent-id <your-agent-id> --company-id <company-id>

# Search episodic memory
anvil memory search "recent goals"

# Check your config
anvil config --check
```

---

## Providers

| Backend  | Env var             | Notes                                          |
|----------|---------------------|------------------------------------------------|
| `claude` | `ANTHROPIC_API_KEY` | Default. Uses `claude-sonnet-4-5`.             |
| `echo`   | —                   | Mirrors input back. Zero cost, CI-safe.        |
| `openai` | `OPENAI_API_KEY`    | Planned.                                       |
| `local`  | —                   | Ollama / llama.cpp — planned.                  |

Adding a provider: implement one async trait in `crates/core/src/providers/`.

---

## Memory

Episodes are stored in `~/.config/anvil/memory.db` (SQLite + FTS5).

```bash
anvil memory search "rust async"      # full-text search
anvil memory list --limit 20          # recent episodes
anvil memory purge --before 30d       # clean up old entries
```

---

## WebSocket Control Plane

Once `harness-gateway` is merged (PR #36), Anvil exposes a WebSocket endpoint that streams live agent events and accepts control commands:

```
ws://localhost:PORT/ws
GET /health
```

**Events** (server → client):

| Event          | Payload                              |
|----------------|--------------------------------------|
| `TurnStart`    | turn index                           |
| `Token`        | streamed token text                  |
| `ToolCall`     | tool name + input                    |
| `ToolResult`   | tool name + output                   |
| `TurnComplete` | final turn message                   |
| `Error`        | message string                       |

**Commands** (client → server):

| Command    | Effect                      |
|------------|-----------------------------|
| `Interrupt`| abort current turn          |
| `Pause`    | pause after current tool    |
| `Resume`   | resume from pause           |
| `Ping`     | keepalive                   |

---

## Roadmap

| Phase | Status              | Scope |
|-------|---------------------|-------|
| 1     | ✅ done              | Claude Code architecture study; fork and KB seeding |
| 2     | ✅ done              | Cargo workspace, provider trait, tool registry, SQLite memory, CLI |
| 3     | ✅ done              | Full tool-call loop, streaming, `anvil eval`, integration tests |
| 4     | ✅ done              | Sub-agent orchestration (tokio tasks, session-type sandbox, depth guard) |
| 5     | ✅ done              | Self-evolution engine (5-gate validate, prompt/skill versioning, rollback) |
| 6     | ✅ done              | Bash, file, GitHub tools; hooks; workspace lints; rename to Anvil |
| 7a    | ✅ done              | `harness-paperclip` + `anvil paperclip` CLI — Paperclip heartbeat adapter |
| 7b    | 🔄 PR #36 open       | `harness-gateway` — WebSocket control-plane (board merge pending) |
| 7c    | 🗓 next              | `harness-tui` — ratatui interactive TUI (live feed, tool inspector, memory browser) |
| 8     | 🗓 next              | v0.1.0 release tag, demo GIF, public announcement |

---

## Reference Projects

This harness is informed by studying the best open-source agent frameworks:

| Project | Key pattern adopted |
|---------|---------------------|
| [hermes-agent](https://github.com/nousresearch/hermes-agent) | Skills as learnable/portable units; closed RL learning loop |
| [opencode](https://github.com/anomalyco/opencode) | Type-state capability gating; client/server split |
| [deepagents](https://github.com/langchain-ai/deepagents) | Explicit planning step; graph-based task DAG with checkpointing |
| [openclaw](https://github.com/openclaw/openclaw) | WebSocket control plane; enum session type as compile-time policy |
| [codex](https://github.com/openai/codex) | Multi-surface single-backend deployment model |
| [phantom](https://github.com/ghostwright/phantom) | 5-gate self-evolution engine with minority veto; dynamic tool registry |
| Claude Code | Tool registry; skill system; MCP integration; hook system |

---

## Contributing

### Who this is for

Anvil is primarily developed by AI agents operating under [Paperclip](https://paperclip.ing) governance, with human oversight from the project board. External contributors are welcome — read this section first.

### Ground rules

- **Issues before PRs.** Open an issue to discuss intent before implementing. Large PRs without prior discussion will likely be closed.
- **One concern per PR.** A PR that mixes a bug fix, refactor, and new feature will be asked to split.
- **Tests are not optional.** Every new behaviour needs a test. The echo provider exists precisely so tests run without an API key.
- **Unsafe code requires justification.** `unsafe_code = "forbid"` in the workspace — if you need an exception, justify it in the PR body.
- **No breaking changes without a plan.** Open a discussion issue first with a migration path.

### Development workflow

```bash
# Full test suite
cargo test --workspace

# Type-check without building (fast feedback)
cargo check --workspace

# Lint
cargo clippy --workspace -- -D warnings

# Format check
cargo fmt --check

# Security audit
cargo audit
```

### Testing with the echo provider

The `EchoProvider` enables full end-to-end testing without any LLM API key or credits:

```bash
# Run the agent loop with the echo provider (mirrors input back)
anvil run --provider echo --goal "test task"

# Run the full integration test suite (uses echo provider, no API key needed)
cargo test -p harness-cli --test echo_integration
```

**Scripted tool calls:** For tests that need deterministic tool-call behaviour,
use `EchoProvider::scripted()` to queue tool calls that are emitted in order
before falling back to the normal echo response:

```rust
use harness_core::provider::{EchoProvider, ScriptedToolCall};

let provider = EchoProvider::scripted(vec![
    ScriptedToolCall {
        id: "call-1".to_string(),
        name: "echo".to_string(),
        input: serde_json::json!({"message": "ping"}),
    },
]);
// First provider call returns ToolUse; subsequent calls echo normally.
```

Integration tests live in `crates/cli/tests/echo_integration.rs` and cover:
plain echo, scripted tool dispatch, max-iteration caps, memory persistence,
and named-session continuity.

### Commit style

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(gateway): add fan-out broadcast to N WebSocket clients
fix(core): handle missing config file gracefully
docs(readme): update roadmap to reflect Phase 7b complete
chore(deps): bump sqlx to 0.8
```

AI-agent commits must include:
```
Co-Authored-By: Paperclip <noreply@paperclip.ing>
```

### Branching

| Branch pattern   | Purpose |
|------------------|---------|
| `main`           | Stable. Protected. Only merges from reviewed PRs from `dev`. |
| `dev`            | Integration. Feature branches merge here first. |
| `feature/<name>` | Active development. Branch from `dev`. |
| `fix/<name>`     | Bug fixes. |
| `chore/<name>`   | Deps, CI, tooling. |

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE) at your option.
