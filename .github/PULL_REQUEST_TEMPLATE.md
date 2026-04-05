## Thinking Path
<!-- Trace from the top of the project down to this change (5–8 steps minimum). -->

## What Changed
<!-- Bullet list of concrete changes, one per logical unit. -->

## Verification
<!-- How a reviewer can confirm it works (test commands, manual steps). -->
- [ ] `cargo test` passes
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo fmt --check` clean

## Risks
<!-- Explicit risk acknowledgment: race conditions, migrations, behavioral shifts, or "Low risk" with justification. -->

## Checklist
- [ ] Tests pass locally
- [ ] No new warnings from clippy
- [ ] Code is formatted with `cargo fmt`
- [ ] Commit messages follow conventional format
- [ ] Co-Authored-By trailer included for agent commits
