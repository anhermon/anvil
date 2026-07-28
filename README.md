# Anvil

> A coding agent that runs **fully offline as a single binary** — no Node, no Python, no API key.
> Tool-calling loop, sub-agents, a skill library and persistent memory, all against a local Ollama model.

**Status:** v0.1.0, pre-release. Builds and tests clean on `main`; the CLI surface below is what actually ships.

[![CI](https://github.com/anhermon/anvil/actions/workflows/ci.yml/badge.svg)](https://github.com/anhermon/anvil/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Rust 1.86](https://img.shields.io/badge/rust-1.86-orange.svg)](https://www.rust-lang.org)

---

## What is Anvil?

Anvil is a single-binary agent harness:

1. `anvil run --goal "..."` starts an agent turn loop against your configured LLM provider.
2. The agent calls tools (bash, file read/write, grep), can spawn sub-agents up to a depth of 4, and can read, save and refine Markdown skills in `~/.anvil/skills/`.
3. Every turn is written to a SQLite episodic memory you can search from the CLI.

### Why pick it over Claude Code, Codex or aider?

Only for the things those tools structurally can't do:

- **It runs with no network and no account.** One static binary plus Ollama. Air-gapped machines, and zero marginal cost per run.
- **No language runtime to install.** `cargo install` once; there is no `node_modules`, no venv.
- **It is a library, not just a CLI.** The agent loop, tool registry and memory store are ordinary Rust crates you can embed in your own program.
- **`--json-output` is NDJSON**, so batch/unattended pipelines can parse every tool call and result.

Where it does **not** compete: interactive day-to-day coding. Against a frontier model those tools are far faster and far more capable. On a local 9.6 GB model a single edit-build-fix task here takes roughly 3 minutes.

---

## Installation

```bash
# Requires Rust 1.86 (pinned in rust-toolchain.toml)
git clone https://github.com/anhermon/anvil
cd anvil
cargo install --path crates/cli

anvil --help
```

---

## Quick start

The verified end-to-end path is a local [Ollama](https://ollama.com) model — no API key, no network.

```bash
ollama serve
ollama pull qwen2.5:3b-instruct

anvil run --provider ollama --model qwen2.5:3b-instruct --goal "Say hello in exactly three words."
```

### Choosing a local model

Tool-calling ability matters far more than speed here.

| Task | Works on a 3B model? |
|------|----------------------|
| Read files, grep, answer a question about a codebase | Yes |
| Edit a file, run a build, fix the error, re-run | **No** — small models tend to *describe* the edit and end the turn without calling `write_file` |

For anything that edits files, use a larger tool-calling model (`gemma4`, `qwen3.6:27b-q4_K_M` or similar). Pick with `--model`.

No LLM at all? The `echo` provider mirrors input back and is what the test suite uses:

```bash
anvil run --provider echo --goal "hello"
```

---

## Providers

`--provider` selects the backend, `--model` the model. If you pass no `--provider` and the model name doesn't look like a Claude or OpenAI model, Anvil auto-selects `ollama`.

| Backend              | Requires                                    | Notes                                                            |
|----------------------|---------------------------------------------|------------------------------------------------------------------|
| `ollama`             | Ollama on `http://localhost:11434`          | Local models. Override the host with `provider.base_url` in config. |
| `echo`               | —                                           | Mirrors input back. Zero cost, CI-safe.                          |
| `claude-code` / `cc` | The `claude` CLI on your `PATH`             | Config default. Shells out to Claude Code as a subprocess — without the `claude` binary installed, runs fail. |
| `claude`             | `ANTHROPIC_API_KEY`, or a Claude Code login | Direct Anthropic API calls.                                      |

Adding a provider: implement one async trait in `crates/core/src/providers/`.

### Credentials (`claude` backend only)

`anvil auth status` shows what's resolved. Order:

1. OAuth bearer token from `~/.claude/.credentials.json` (override the directory with `CLAUDE_CONFIG_DIR`)
2. `ANTHROPIC_API_KEY` environment variable

---

## Usage

```bash
# Run an agent session
anvil run --provider ollama --model qwen2.5:3b-instruct --goal "Summarise the current directory"

# Stream tokens as they arrive
anvil run --provider echo --goal "hello" --stream

# Machine-readable NDJSON events instead of terminal output
anvil run --provider echo --goal "hello" --json-output

# Named session — loads prior history and saves new episodes under that name
anvil run --provider echo --goal "continue the work" --session myproject

# Batch-evaluate against a JSONL suite of {"goal":"...","expected":"..."} lines
anvil eval --cases cases.jsonl --provider echo

# Memory
anvil memory search "rust async" --limit 10
anvil memory recent myproject --limit 20     # session name or UUID

# Config and auth
anvil config --check
anvil auth status
```

`anvil --help` and `anvil <command> --help` are authoritative.

---

## Memory

Episodes are stored in `~/.paperclip/harness/memory.db` (SQLite + FTS5), alongside config at `~/.paperclip/harness/config.toml`.

There are **two** ways past episodes reach the model, and only the first is opt-in:

1. **Named sessions.** `--session <name>` replays that session's prior episodes as real conversation turns, up to `memory.max_context_episodes` (default 20).
2. **Global recall — always on.** Every run, with or without `--session`, full-text-searches *all* episodes for the goal text and prepends up to 5 matches (200 chars each) to the system prompt.

Because of (2), `--session` is **not** an isolation boundary: a run under one session name can recall facts recorded under another. Everything written to the database is visible to every later run by the same user. Don't put anything in a goal you wouldn't want recalled in an unrelated project.

Both paths are per-user, not per-directory — there is no way to point Anvil at a different database short of changing `$HOME`.

---

## Architecture

```
crates/
├── core/        Provider trait, message types, config, session, auth
│                Providers: claude, claude-code (subprocess), ollama, echo
├── tools/       Tool registry with JSON-schema validation
├── memory/      SQLite + FTS5 episodic memory (sqlx)
└── cli/         clap CLI: anvil run / config / memory / eval / auth
                 Agent turn loop, sub-agent spawning, terminal UI
```

Tools registered in the shipped binary: `echo`, `read_file`, `write_file`, `grep`, `bash`, `spawn_subagent`, `list_skills`, `read_skill`, `save_skill`, `refine_skill`.

Limits worth knowing before you hit them:

- **`bash` commands time out after 30 seconds** and are restricted to an allowlist (`cargo`, `git`, `ls`, `cat`, `grep`, `curl`, `jq`, …). A cold `cargo build` on a non-trivial project will exceed this.
- **`write_file` requires a prior `read_file`** of that path in the same session, and refuses to write if the file changed since that read. Creating a new file is exempt.
- **Paths are relative only** — absolute paths and `..` are rejected. Anvil operates on the current working directory.

### Crates not wired into the binary

These build as part of the workspace, but nothing in `anvil` depends on them yet:

| Crate        | What it is                                                       |
|--------------|------------------------------------------------------------------|
| `evolution/` | 5-gate observe → critique → generate → validate → apply engine. Compiled in only under `--features evolution`, off by default. Per [#65](https://github.com/anhermon/anvil/issues/65) it currently logs outcomes rather than closing a learning loop across sessions. |
| `paperclip/` | Paperclip control-plane client + heartbeat adapter. No CLI entry point on `main`. |
| `github/`    | GitHub API client and @mention webhook server. No CLI entry point on `main`. |

A WebSocket control-plane crate (`gateway/`) and a ratatui TUI exist on the `dev` branch and are not part of `main`.

---

## Contributing

### Ground rules

- **Issues before PRs.** Open an issue to discuss intent before implementing. Large PRs without prior discussion will likely be closed.
- **One concern per PR.** A PR that mixes a bug fix, refactor, and new feature will be asked to split.
- **Tests are not optional.** Every new behaviour needs a test. The echo provider exists precisely so tests run without an API key.
- **Unsafe code requires justification.** `unsafe_code = "forbid"` in the workspace — if you need an exception, justify it in the PR body.

### Development workflow

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

### Commit style

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
fix(core): handle missing config file gracefully
docs(readme): describe the shipped CLI surface
chore(deps): bump sqlx to 0.8
```

### Branching

| Branch pattern   | Purpose |
|------------------|---------|
| `main`           | Stable. Protected. |
| `dev`            | Integration. Feature branches merge here first. |
| `feature/<name>` | Active development. Branch from `dev`. |
| `fix/<name>`     | Bug fixes. |
| `chore/<name>`   | Deps, CI, tooling. |

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE) at your option.
