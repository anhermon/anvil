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
- **`--json-output` is NDJSON**, so batch/unattended pipelines can parse every tool call and result. `anvil run` exits non-zero when the run did not finish — see [Exit codes](#exit-codes).

Where it does **not** compete: interactive day-to-day coding. Against a frontier model those tools are far faster and far more capable. Measured on a local 9.6 GB model (`gemma4`, M5, fresh `$HOME` per run so episodic memory cannot leak between them): a one-line fix verified by re-running `cargo test` takes a median of 55s and landed 9 times out of 9. A fix spanning two files and two bugs takes a median of 2m44s and landed 5 times out of 7 — twice it did not, and one of those reported success over a still-failing test suite.

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

Ollama requests time out after 120 seconds by default. For a large model that needs longer to load,
both `run` and `chat` accept `--ollama-timeout-secs 300`, or set `ANVIL_OLLAMA_TIMEOUT_SECS=300`.

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

Want to keep talking without starting a new command for every prompt?

```bash
anvil chat --provider echo --session myproject
```

Chat keeps every prompt in the same named session. Type `/exit` or press Ctrl-D to leave.

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

# Start an interactive chat; omit --session to generate a resumable session name
anvil chat --provider ollama --model qwen2.5:3b-instruct --session myproject

# Stream tokens as they arrive
anvil run --provider echo --goal "hello" --stream

# Machine-readable NDJSON events instead of terminal output
anvil run --provider echo --goal "hello" --json-output

# Named session — loads prior history and saves new episodes under that name
anvil run --provider echo --goal "continue the work" --session myproject

# Batch-evaluate against a JSONL suite of {"goal":"...","expected":"..."} lines
anvil eval --cases cases.jsonl --provider echo

# Raise or remove the iteration cap for this run (0 = unlimited)
anvil run --provider ollama --model gemma4 --goal "fix the failing test" --max-iterations 80

# Memory
anvil memory search "rust async" --limit 10
anvil memory recent myproject --limit 20     # session name or UUID

# Config and auth
anvil config --check
anvil auth status
```

`anvil --help` and `anvil <command> --help` are authoritative.

### Exit codes

`anvil run` reports its outcome so an unattended caller can branch on it:

| Exit code | Meaning |
|-----------|---------|
| `0` | The agent ended its own turn — it believes it is finished. |
| `1` | The run aborted: provider error, bad config, unreachable Ollama, I/O failure. |
| `2` | The run stopped without the agent finishing. Today the only cause is hitting the `--max-iterations` cap. |

Under `--json-output` the same outcome is on the terminal `result` event, so you do not have to shell out to read `$?`:

```json
{"type":"result","part":{"text":"…","isError":true,"outcome":"max_iterations","sessionId":"…","model":"gemma4"}}
```

`outcome` is one of `done`, `max_iterations`, `failed`, `cancelled`; `isError` is true for everything except `done`.

**What exit code 0 does *not* mean.** It means the model stopped and said it was done — not that the goal was achieved. In measured runs against a local model, a session ended with `cargo test` still red while the final message claimed success and rationalised the remaining failure away. Anvil does not attempt to detect that: it has no ground truth for an arbitrary natural-language goal, and guessing from the wording of the final message would produce a confidently wrong exit code, which is worse than an honestly ambiguous `0`. Two related cases are also deliberately left as `0`: an agent that gives up in prose without a tool call, and a provider that reports `stop_reason=tool_use` with no tool-use blocks — both are indistinguishable from a legitimate finish from inside the loop.

**If you need to know whether the goal was met, assert it yourself** — run the check outside anvil and branch on *that*:

```bash
HOME=$(mktemp -d) anvil run --provider ollama --model gemma4 --json-output \
  --goal "fix the failing test in src/lib.rs" || exit 1   # catches cap exhaustion and hard errors
cargo test                                                 # your ground truth for "it actually worked"
```

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

- **`bash` commands time out after 30 seconds** and are restricted to a hardcoded allowlist (`cargo`, `rustc`, `rustfmt`, `git`, `python3`, `python`, `pytest`, `ls`, `cat`, `echo`, `pwd`, `env`, `which`, `grep`, `bash`, `curl`, `jq`). A cold `cargo build` on a non-trivial project will exceed the timeout. The allowlist is a guard rail against accidental damage, **not** a sandbox: `bash` is on it, so `bash -c '<anything>'` passes the check. It is not configurable, because a config knob would advertise a containment property this gate does not have — if you are running untrusted goals, isolate the process.
- **The iteration cap defaults to 50** (`agent.max_iterations` in the config; `--max-iterations` overrides it, `0` means unlimited). On a local model a two-file fix has been measured at up to 10 iterations, so the old cap of 10 was a coin flip. Hitting the cap is a failure and exits `2`.
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
