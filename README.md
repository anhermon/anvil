# paperclip-harness

> A world-class, Rust-native agent harness. One binary. Any LLM. Full autonomy with comfortable human override.

**Status:** Phase 3 in progress — v0 single-turn loop shipped; sub-agent orchestration and self-evolution next.

---

## Vision

Most agent frameworks bolt autonomy onto a chat loop. `paperclip-harness` is designed from first principles as a **self-bootstrapping agent OS**:

1. You run the binary, configure your LLM provider, and describe your goals.
2. The agent plans its first tasks, builds the skills it needs, and spawns sub-agents to execute them.
3. After each session it reflects, critiques its own outputs, and evolves its prompts and skills.
4. You watch, approve, and redirect via a Paperclip-compatible control plane — intervening only when you want to.

The result is a system that compounds in capability over time, learns from every session, and stays legible to its operators.

---

## Architecture

Eight layers, bottom-up:

```
┌──────────────────────────────────────────────────┐
│  8. Control Plane / UI                           │
│     WebSocket gateway · ratatui TUI              │
│     Paperclip API compatibility                  │
├──────────────────────────────────────────────────┤
│  7. Self-Evolution Engine                        │
│     observe → critique → generate → validate     │
│     5-gate minority-veto · prompt/skill rollback │
├──────────────────────────────────────────────────┤
│  6. Memory System                                │
│     SQLite + FTS5 episodic · SQLite-vec semantic │
│     PARA-structured knowledge base               │
├──────────────────────────────────────────────────┤
│  5. Sub-agent Orchestration                      │
│     tokio tasks + RPC · session-type sandbox     │
│     worktree isolation for coding agents         │
├──────────────────────────────────────────────────┤
│  4. Task Management                              │
│     issue/task lifecycle · planning step         │
│     graph-based DAG with checkpointing           │
├──────────────────────────────────────────────────┤
│  3. Tool & Skill System                          │
│     registry dispatch · YAML/TOML + Rust skills  │
│     dynamic MCP registration · WASM hot-reload   │
├──────────────────────────────────────────────────┤
│  2. Agent Core                                   │
│     provider-agnostic LLM trait                  │
│     async turn loop · capability-gated (type-state)│
├──────────────────────────────────────────────────┤
│  1. Bootstrap Layer                              │
│     provider config at first run                 │
│     context/goal elicitation · self-bootstraps   │
└──────────────────────────────────────────────────┘
```

### Crate layout

```
crates/
├── core/      Provider trait, message types, config, session, turn loop
├── tools/     Tool registry, serde_json schema validation, built-in tools
├── memory/    SQLite episodic memory (sqlx + FTS5), semantic recall (sqlite-vec)
├── cli/       clap derive CLI: anvil run / config / memory / eval
├── task/      (planned) Task DAG, planning step, checkpointing
├── orchestrator/ (planned) Sub-agent spawn/manage via tokio RPC
└── ui/        (planned) ratatui TUI + WebSocket control plane
```

---

## Installation

```bash
# Install from source (requires Rust 1.75+)
cargo install --path crates/cli

# Then run from anywhere:
anvil --help
anvil run --goal "your goal here"
```

Set `ANTHROPIC_API_KEY` or use an existing Claude Code session (see Auth below).

## Usage

```bash
# Run an agent session with a goal
anvil run --goal "Summarise the current directory"

# Run with the echo provider -- no API key, great for testing
anvil run --provider echo --goal "hello"

# Check your config
anvil config --check

# Search episodic memory
anvil memory search "recent goals"
```

## Quick Start

```bash
# Requires Rust 1.75+
git clone https://github.com/anhermon/paperclip-harness
cd paperclip-harness

# Run with Claude (default)
export ANTHROPIC_API_KEY=sk-ant-...
cargo run -- run --goal "Summarise the current directory"

# Run with the echo provider — no API key, great for testing
cargo run -- run --provider echo --goal "hello"

# Check your config
cargo run -- config --check

# Search episodic memory
cargo run -- memory search "yesterday's goal"
```

---

## Development

Install [Task](https://taskfile.dev/#/installation) then use these commands:

```bash
task build    # cargo build --release
task test     # cargo test --workspace
task lint     # cargo clippy --workspace --all-targets -- -D warnings
task fmt      # cargo fmt --all
task check    # fmt + lint + test (full CI gate)
task install  # cargo install --path crates/cli
task          # list all tasks
```

---

## Providers

| Backend  | Env var             | Notes                                    |
|----------|---------------------|------------------------------------------|
| `claude` | `ANTHROPIC_API_KEY` | Default. Uses `claude-sonnet-4-5`.       |
| `echo`   | —                   | Mirrors input back. Zero cost, CI-safe.  |
| `openai` | `OPENAI_API_KEY`    | Planned — Phase 4.                       |
| `local`  | —                   | Ollama / llama.cpp. Planned — Phase 4.   |

Adding a provider means implementing one async trait in `crates/core/src/providers/`.

---

## Memory

Episodes are stored in `~/.paperclip/harness/memory.db` (SQLite + FTS5).

```bash
anvil memory search "rust async"    # full-text search
anvil memory list --limit 20        # recent episodes
anvil memory purge --before 30d     # clean up old entries
```

Semantic recall via `sqlite-vec` is planned for Phase 3.

---

## Roadmap

| Phase | Status       | Scope |
|-------|-------------|-------|
| 1     | ✅ done      | Claude Code fork & architecture study |
| 2     | ✅ done      | Rust dev skills, Cargo workspace, provider trait, tool registry, SQLite memory, CLI |
| 3     | 🔄 in progress | Tool call loop, streaming, `anvil eval`, task DAG, planning step |
| 4     | planned     | Sub-agent orchestration (tokio RPC, session-type sandbox, worktree isolation) |
| 5     | planned     | Self-evolution engine (5-gate validate, prompt/skill versioning, rollback) |
| 6     | planned     | Control plane: WebSocket gateway, ratatui TUI, Paperclip API adapter, open-source release |

Detailed per-phase breakdown lives in the [ANGA-70 plan](http://localhost:3100/ANGA/issues/ANGA-70#document-plan).

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

This project is primarily developed by AI agents operating under [Paperclip](https://paperclip.ing) governance, with human oversight from the project board. External contributors are welcome, but should read this section carefully.

### Ground rules

- **Issues before PRs.** Open an issue to discuss intent before investing in an implementation. Large PRs without prior discussion will likely be closed.
- **One concern per PR.** Keep changes focused. A PR that mixes a bug fix with a refactor and a new feature will be asked to split.
- **Tests are not optional.** Every new behaviour needs a test. The echo provider exists precisely so tests run without an API key.
- **Unsafe code requires justification.** If you reach for `unsafe`, explain why in the PR body. Most use cases don't need it.
- **No breaking changes without a plan.** If you need to change a public API, open a discussion issue first with a migration path.

### Development workflow

```bash
# Run the full test suite
cargo test

# Type-check without building (fast feedback)
cargo check

# Lint
cargo clippy -- -D warnings

# Format
cargo fmt --check
```

### Commit style

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(memory): add FTS5 phrase search support
fix(cli): handle missing config file gracefully
docs(readme): expand architecture section
chore(deps): bump sqlx to 0.8
```

AI-agent commits must include:
```
Co-Authored-By: Paperclip <noreply@paperclip.ing>
```

### Branching

| Branch pattern | Purpose |
|----------------|---------|
| `master`       | Stable. Protected. Only merges from reviewed PRs. |
| `feat/*`       | New features. Branch from `master`. |
| `fix/*`        | Bug fixes. |
| `docs/*`       | Documentation only. |
| `chore/*`      | Deps, CI, tooling. |

### Code of conduct

Be direct, be kind, be useful. We don't have a lengthy CoC — just don't be a jerk, and focus on the work.

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE) at your option.
