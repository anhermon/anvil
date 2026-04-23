# Platform Scripts

This directory contains platform-level tooling for commit-cadence enforcement and change-history tracking.

## Scripts

### check-commit-cadence.sh

Enforces commit cadence by preventing long-lived uncommitted changes beyond a defined threshold.

**Default threshold:** 24 hours

**Usage:**
```bash
# Run manually
./scripts/check-commit-cadence.sh

# Override threshold (48 hours)
COMMIT_CADENCE_THRESHOLD_HOURS=48 ./scripts/check-commit-cadence.sh

# Bypass during commit (not recommended)
COMMIT_CADENCE_THRESHOLD_HOURS=999 git commit
```

**Integration:**
- Automatically runs as part of `pre-commit` hook
- Blocks commits when uncommitted changes exceed threshold
- Encourages frequent commits for traceable workflow evolution

**Rationale:** Frequent commits enable reliable branching, rollback, and auditable organizational evolution.

---

### generate-change-history.sh

Generates machine-readable change history from git commit log.

**Output format:** JSON (extensible to YAML, CSV)

**Usage:**
```bash
# Generate history for last 50 commits
./scripts/generate-change-history.sh --limit 50

# Generate history since specific date
./scripts/generate-change-history.sh --since "2026-04-01"

# Write to file
./scripts/generate-change-history.sh --since "1 week ago" --output history.json

# Show help
./scripts/generate-change-history.sh --help
```

**Options:**
- `--since DATE` - Only commits since this date (e.g., "2026-04-01", "1 week ago")
- `--limit N` - Maximum number of commits to include
- `--output FILE` - Write to file instead of stdout
- `--format TYPE` - Output format: json (default), yaml, csv
- `--help` - Show help message

**Output structure:**
```json
[
  {
    "hash": "abc123...",
    "author": "Author Name",
    "email": "author@example.com",
    "date": "2026-04-23T12:00:00Z",
    "subject": "feat(scope): description",
    "type": "feat",
    "scope": "scope",
    "description": "description",
    "rationale": "Why this change was needed...",
    "body": "Full commit message body..."
  }
]
```

**Use cases:**
- Audit trail generation for compliance
- Workflow evolution tracking
- Release notes automation
- Change impact analysis

---

## Git Hooks Integration

The platform enforces commit quality through git hooks located in `.githooks/`:

### pre-commit
- **Commit cadence check** - Prevents long-lived uncommitted changes
- **Format check** - Ensures code formatting (cargo fmt)
- **Lint check** - Validates code quality (cargo clippy)

### commit-msg
- **Conventional Commits validation** - Enforces format: `<type>(<scope>): <description>`
- **Rationale requirement** - For certain commit types (feat, fix, refactor), requires a "Rationale:" field explaining why the change is needed

### pre-push
- **Test suite** - Runs full test suite before push

## Commit Message Template

A git commit message template is available at `.gitmessage`. To configure it globally:

```bash
git config commit.template .gitmessage
```

The template includes:
- Format guidance for Conventional Commits
- Rationale field requirements
- Examples of well-formed commit messages
- Co-Authored-By trailer for agent commits

## Rationale Requirements

Rationale is **required** for:
- `feat` (all scopes) - New features
- `fix` (all scopes) - Bug fixes
- `refactor` (all scopes) - Code refactoring
- `chore`, `docs` (when scope is workflow/governance/agents/paperclip/platform)

Rationale is **optional** for:
- `style` - Formatting changes
- `test` - Test additions/updates
- `ci` - CI/CD configuration

**Rationale should explain:**
- **WHY** this change is needed
- What problem it solves
- How it fits into broader workflow/organizational evolution
- Any trade-offs or alternatives considered

**Minimum length:** 10 characters for meaningful explanation

## Examples

### Good commit with rationale:
```
feat(platform): add commit-cadence enforcement guardrail

Implements a pre-commit check that blocks commits when uncommitted
changes exceed 24h threshold.

Rationale: Frequent commits enable reliable branching, rollback, and
traceable organizational evolution. This addresses the gap identified
in ANGA-652 where workflow changes accumulated without version control,
making rollback and auditing difficult.

Co-Authored-By: Paperclip <noreply@paperclip.ing>
```

### Good fix with rationale:
```
fix(memory): prevent FTS5 corruption on concurrent writes

Adds SQLite WAL mode and IMMEDIATE transaction for episode inserts.

Rationale: Production agents experienced episodic memory corruption
when multiple runs wrote simultaneously. WAL mode allows concurrent
reads during writes, and IMMEDIATE prevents write-write conflicts.
Alternative (serialized queue) would add latency to turn completion.

Co-Authored-By: Paperclip <noreply@paperclip.ing>
```

## Testing

To test the platform scripts locally:

```bash
# Test cadence check (should pass if changes are recent)
./scripts/check-commit-cadence.sh

# Test change history generation
./scripts/generate-change-history.sh --limit 10

# Test commit message validation
echo "test: invalid message" | .githooks/commit-msg /dev/stdin
```

## Troubleshooting

**Cadence check failing?**
- Commit your changes: `git commit -m "..."`
- Or stash them: `git stash save "WIP: description"`
- Emergency bypass (not recommended): `COMMIT_CADENCE_THRESHOLD_HOURS=999 git commit`

**Commit message validation failing?**
- Follow Conventional Commits format: `<type>(<scope>): <description>`
- Add rationale for feat/fix/refactor: `Rationale: explanation...`
- See `.gitmessage` for template and examples

**Change history script not working?**
- Ensure you're in a git repository
- Check git log has commits: `git log --oneline | head`
