# Anvil Alignment Feedback Report

## Declared intent

Anvil is a self-bootstrapping Rust agent harness for developers and coding
agents. A user should be able to submit a goal through the CLI, have the
harness plan and execute bounded tool-using turns (including delegated work),
persist the resulting episode history, and inspect the run afterward.

For this dogfood pass, alignment means:

- the documented CLI can complete a CI-safe run without real API calls;
- tool contracts reject invalid inputs instead of reporting false success;
- delegated work reports an honest result when it cannot finish;
- memory preserves a usable chronological history;
- machine-readable output identifies what actually ran; and
- fixes are small, tested, and independently reviewable.

## Iteration 1 — 2026-07-28

### Inventory

Shipping CLI commands:

- `anvil run`
- `anvil config`
- `anvil memory`
- `anvil eval`
- `anvil auth`

Shipping agent surfaces include the bounded turn loop, built-in file/search
and shell tools, `spawn_subagent`, and the skill-library tools
`list_skills`, `read_skill`, `save_skill`, and `refine_skill`.

The `evolution`, `github`, and `paperclip` crates build as workspace members
but are not wired into the shipped binary. PR #88 already covers Ollama
small-model protocol recovery and benchmark-memory contamination, so that
area was excluded from this pass.

### Exercises and wins

- `anvil eval --provider echo` completed a JSONL case without credentials.
- A uniquely named `anvil run --provider echo` session completed and was
  readable through `anvil memory recent`.
- Normal relative-path file reads/writes, file grep, recursive directory grep,
  shell execution, and skill save/list flows worked.
- Existing focused suites passed: 31 `harness-tools` tests, subagent tests,
  max-iteration tests, and skill-library tests.
- The two `harness-paperclip` failures seen in the managed sandbox were
  environmental socket restrictions; all paperclip tests passed when rerun
  outside that restriction.

### Gaps

| # | Gap | Severity | Evidence |
|---|---|---|---|
| 1 | Tool schemas accept values with the wrong JSON type, allowing handlers to silently substitute defaults. | HIGH | A registry call to `write_file` with numeric `content` returned success and created a zero-byte file. `crates/tools/src/schema.rs` checked required-field presence but not declared property types. |
| 2 | A subagent that reaches its iteration cap immediately after a tool call is reported as successful placeholder output. | HIGH | `spawn_subagent` converted a child whose last message was a tool result into `ToolOutput::ok("(sub-agent produced no output)")` in `crates/cli/src/agent.rs`. |
| 3 | Named-session history is persisted in assistant/user order, and a limited query selects the oldest rather than newest window. | HIGH | Two echo runs followed by `anvil memory recent <name> --limit 2` returned the first exchange; inserts in `crates/cli/src/agent.rs` are assistant-first and `crates/memory/src/db.rs` applies `ORDER BY created_at ASC LIMIT`. |
| 4 | `anvil eval` cannot use the default `claude-code` backend. | HIGH | With config reporting `claude-code`, eval fell through to the direct Claude path and requested `ANTHROPIC_API_KEY`; `--provider echo` succeeded. |
| 5 | A typoed `anvil run --provider` value silently falls through to direct Claude. | MEDIUM | `--provider not-a-provider` produced an unrelated Anthropic credential error because the provider match uses direct Claude as its wildcard arm. |
| 6 | JSON result events report the configured model rather than the provider that actually ran. | MEDIUM | An echo run emitted `"model":"claude-sonnet-4-5"` in its result event. |
| 7 | `auth status` contradicts `config --check` for keyless/delegated backends. | MEDIUM | With `claude-code` configured, config check exited successfully while auth status reported no credentials and exited 1. |
| 8 | Directory grep requires callers to discover and set `recursive: true` even though the tool advertises file-or-directory input. | MEDIUM | `grep` on `.` failed with “Is a directory”; the same call with `recursive:true` succeeded. |
| 9 | Skill metadata helpers rewrite body lines that merely look like frontmatter fields. | MEDIUM | `read_skill`/`refine_skill` scan the whole Markdown file for trimmed `uses:` and `version:` lines in `crates/tools/src/builtin.rs`. |

### Actions

- Selected gap 1 for a focused tool-schema validation fix with regression tests.
- Selected gap 2 for a focused subagent-result fix while preserving the
  project-mandated `SessionStatus::Done` behavior at the iteration cap.
- Deferred gaps 3, 4, and 7 because concurrent local work already contains a
  CLI-fixes commit touching the same memory/eval/auth files.
- Deferred gaps 5, 6, 8, and 9 to keep this iteration independently
  reviewable and avoid overlap with active PRs.

### Queued

1. Re-dogfood named-session chronology after the concurrent CLI work lands.
2. Reject unknown provider identifiers and report the effective provider/model
   in structured output.
3. Make directory grep ergonomic by default without changing explicit
   non-recursive behavior.
4. Restrict skill metadata parsing and updates to YAML frontmatter.
5. Decide whether `--max-iterations` is a per-agent or shared delegation-tree
   budget, then encode that contract in tests and CLI help.

## Iteration 2 — 2026-07-28

### User feedback

Direct dogfood feedback superseded the queue:

- `cargo install --path crates/cli` failed because a newly added
  `compact_system_prompt` field was missing from an `AgentConfig` initializer,
  so no `anvil` executable was installed.
- After that initializer landed in concurrent local work, Cargo installed to
  `~/.cargo/bin`, which was not on the interactive shell path.
- A run with `qwen3.6:latest` remained on `thinking` with no visible bound or
  loading guidance.
- The CLI had no interactive chat surface; every interaction required a new
  `anvil run --goal`.

### Evidence and scoring

| # | Gap | Severity | Evidence |
|---|---|---|---|
| 10 | Ollama waits are not configurable from the CLI and timeout failures do not explain cold model loading. | HIGH | `qwen3.6:latest` is a 23 GB local model and was not resident in `ollama ps`; the CLI displayed only `thinking`. Provider construction hid the request timeout and had a fallible path to an unbounded client. |
| 11 | No interactive multi-turn chat command exists. | HIGH | `anvil --help` exposed only one-shot `run --goal`; user feedback explicitly requested a persistent chat mode. |
| 12 | Cargo's default install location is not necessarily on the user's shell path. | MEDIUM | `cargo install` placed the binary in `~/.cargo/bin`, while the interactive shell path included `~/.local/bin` but not `~/.cargo/bin`. |

### Actions

- Installed the current release snapshot successfully and placed `anvil` in
  `~/.local/bin`; an interactive shell resolves `anvil 0.1.0`.
- Opened [PR #89](https://github.com/anhermon/anvil/pull/89) for strict tool
  input type validation.
- Opened [PR #90](https://github.com/anhermon/anvil/pull/90) so unfinished
  delegated runs cannot report false success.
- Opened [PR #91](https://github.com/anhermon/anvil/pull/91) for configurable,
  actionable, bounded Ollama waits.
- Opened stacked [PR #92](https://github.com/anhermon/anvil/pull/92) for
  `anvil chat`, named-session continuity, clean exit behavior, and shared
  provider options.

### Re-exercise

- The installed one-shot CLI completed an Ollama run successfully.
- Clean-main timeout tests stalled before response headers and after streaming
  headers; both terminated within the configured test bound with actionable
  errors.
- Clean-main chat tests completed multiple EchoProvider prompts in one named
  in-memory session and exited cleanly on `/exit` and EOF.
- The combined release-snapshot build passed 92 tests (1 ignored), installed
  to `~/.local/bin`, completed a real EchoProvider terminal chat, and exited
  cleanly on `/exit`.
- A real `qwen3.6:latest` run with `--ollama-timeout-secs 1` stopped within the
  bound and reported the endpoint, likely cold model load, smaller-model
  option, retry path, and timeout override.

### Queued

1. Retarget PR #92 to `main` after PR #91 lands.
2. Decide whether installation docs should recommend `--root ~/.local` or
   instruct users to add Cargo's bin directory to `PATH`.
3. Re-run the 23 GB model after it is warm to separate model startup cost from
   generation behavior.
