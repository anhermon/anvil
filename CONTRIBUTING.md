# Contributing to Anvil

Thanks for your interest. This document covers everything you need to contribute effectively.

## Before you start

This project is developed primarily by AI agents under [Paperclip](https://paperclip.ing) governance, with a human board providing direction and review. External contributions are welcome — but the bar is high. We prefer fewer, better PRs over volume.

**Always open an issue before writing code for a non-trivial change.** It avoids wasted effort and lets us tell you whether the change fits the roadmap.

## Setting up

```bash
git clone https://github.com/anhermon/anvil
cd anvil
cargo build          # verify dependencies resolve
cargo test           # should be green — uses echo provider, no API key needed
```

Minimum toolchain: **Rust 1.75** (see `Cargo.toml` `rust-version`).

## What to work on

Good first targets:

- Items marked `good first issue` in the [issue tracker](https://github.com/anhermon/anvil/issues)
- Test coverage gaps
- Documentation improvements

Things to avoid proposing unless you've discussed first:

- New LLM provider integrations (we have a defined provider trait; new backends are welcome but need discussion)
- Changes to the core `Provider` or `ToolRegistry` traits (these affect everything)
- New crates / workspace members
- Anything touching `unsafe`

## Pull request checklist

Before opening a PR, confirm:

- [ ] `cargo test` passes locally
- [ ] `cargo clippy -- -D warnings` is clean
- [ ] `cargo fmt` has been run
- [ ] New behaviour has a test (using `EchoProvider` where possible)
- [ ] Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/)
- [ ] The PR description explains *why*, not just *what*

## Commit format

```
<type>(<scope>): <short description>

[optional body]

[optional footer]
Co-Authored-By: <name> <email>   ← required for AI-agent commits
```

Types: `feat`, `fix`, `docs`, `chore`, `refactor`, `test`, `perf`

Scopes: `core`, `tools`, `memory`, `cli`, `gateway`, `tui`, `paperclip`, `github`, `evolution`, `deps`

## Branching model

```
main              ← stable, protected — PRs from dev only, requires human approval
  dev             ← integration — PRs from feature/* only (no direct commits)
    feature/*     ← new features
    fix/*         ← bug fixes
    docs/*        ← docs only
    chore/*       ← deps / CI / tooling
```

All feature branches cut from `dev`. Open a PR to `dev`; `dev` → `main` requires board approval.
Squash-merge preferred for feature/* and fix/*; merge commit for larger milestones.

## Testing philosophy

- The `EchoProvider` exists so the full tool/memory/CLI pipeline can be tested without hitting an API.
- Unit tests live next to the code they test (`#[cfg(test)]` modules).
- Integration tests go in `tests/` at the crate root.
- Do not mock the database. Use an in-memory SQLite URL (`sqlite::memory:`) instead.

## AI-agent contributors

Agents committing to this repo via Paperclip must:

1. Include `Co-Authored-By: Paperclip <noreply@paperclip.ing>` in every commit.
2. Reference the Paperclip issue ID in the PR description (e.g. `Closes ANGA-70`).
3. Not push directly to `main` or `dev` — always via a reviewed PR.

## Questions

Open a [GitHub Discussion](https://github.com/anhermon/anvil/discussions) for open-ended questions. Use issues for concrete bugs or proposals.

## Setting Up Dev Hooks

After cloning, install git hooks:

```bash
bash scripts/setup-hooks.sh
```

This installs:
- **pre-commit**: `cargo fmt --check` + `cargo clippy -D warnings`
- **pre-push**: `cargo test --workspace`
- **commit-msg**: Enforces Conventional Commits format
