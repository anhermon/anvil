# Evolution Safety Gates (APT Pattern)

This document describes the APT (Agentic Prompt Template) pattern safety gates implemented in the self-evolution engine to prevent performance degradation.

## Overview

The `SafeApplier` implements a three-stage safety gate system that wraps around prompt evolution to ensure new prompts don't degrade system performance:

```
┌─────────────────────────────────────────────────────┐
│  Evolution Pipeline with Safety Gates              │
├─────────────────────────────────────────────────────┤
│  Observer → Critic → Generator → Validators (5×)   │
│                                    ↓                │
│                          ┌─────────────────┐       │
│                          │  SafeApplier    │       │
│                          ├─────────────────┤       │
│                          │ 1. Pre-hook     │       │
│                          │    - Backup     │       │
│                          │    - Baseline   │       │
│                          ├─────────────────┤       │
│                          │ 2. Apply        │       │
│                          │    - Write cfg  │       │
│                          ├─────────────────┤       │
│                          │ 3. Post-hook    │       │
│                          │    - Benchmark  │       │
│                          │    - Compare    │       │
│                          │    - Rollback?  │       │
│                          └─────────────────┘       │
└─────────────────────────────────────────────────────┘
```

## Three Safety Gates

### Gate 1: Pre-Hook (Backup)

**Location**: `SafeApplier::backup_current_state()`

**Actions**:
1. Creates `~/.paperclip/harness/prompt-backups/` directory
2. Backs up current system prompt to timestamped file
3. Runs baseline benchmarks to capture current performance
4. Stores backup metadata (timestamp, candidate ID, baseline score) as JSON

**Files created**:
- `prompt-YYYYMMDD-HHMMSS.txt` — backed-up prompt
- `prompt-YYYYMMDD-HHMMSS.meta.json` — backup metadata

### Gate 2: Apply

**Location**: `SafeApplier::apply_prompt()`

**Actions**:
1. Loads `~/.paperclip/harness/config.toml`
2. Updates `agent.system_prompt` field
3. Writes modified config back to disk

**Failure behavior**: If apply fails, post-hook is skipped and error is returned immediately.

### Gate 3: Post-Hook (Benchmark + Auto-Rollback)

**Location**: `SafeApplier::run_benchmarks()` + rollback logic

**Actions**:
1. Executes `./scripts/run-evolution-benchmarks.sh`
2. Parses JSON output with benchmark scores
3. Compares new score to baseline score
4. **Auto-rollback** if regression > 10%

**Rollback trigger**:
```rust
let regression_pct = (baseline_score - new_score) / baseline_score;
if regression_pct > 0.10 {
    // Restore prompt from backup
    // Return error to mark evolution as failed
}
```

## Benchmark Format

The benchmark runner script must output JSON to stdout in this format:

```json
{
  "score": 0.85,
  "metrics": {
    "build_success": 1.0,
    "test_pass_rate": 0.85,
    "clippy_clean": 0.9,
    "fmt_clean": 1.0
  }
}
```

**Score range**: 0.0 (worst) to 1.0 (best)

**Default weights** (in `run-evolution-benchmarks.sh`):
- Build success: 25%
- Test pass rate: 35%
- Clippy clean: 20%
- Fmt clean: 20%

## Benchmark Runner Script

**Path**: `./scripts/run-evolution-benchmarks.sh`

**Metrics measured**:
1. **Build success**: Does `cargo build --workspace --all-targets` succeed?
2. **Test pass rate**: Percentage of tests passing in `cargo test --workspace`
3. **Clippy clean**: Does `cargo clippy -- -D warnings` pass?
4. **Fmt clean**: Does `cargo fmt -- --check` pass?

**Fallback behavior**: If the script doesn't exist, SafeApplier returns a perfect score (1.0) and skips validation. This allows the system to function before benchmarks are fully integrated.

## Usage

The `SafeApplier` is automatically used when calling `harness_evolution::defaults::default_engine()`:

```rust
use harness_evolution::defaults::default_engine;
use harness_memory::MemoryDb;

let memory = Arc::new(MemoryDb::in_memory().await?);
let engine = default_engine(Arc::clone(&memory)); // Uses SafeApplier
let outcome = engine.evolve(&session, &current_prompt).await?;
```

To bypass safety gates (for testing), use `unsafe_engine()`:

```rust
use harness_evolution::defaults::unsafe_engine;

let engine = unsafe_engine(Arc::clone(&memory)); // No-op applier
```

## Backup Management

- **Location**: `~/.paperclip/harness/prompt-backups/`
- **Retention**: Last 10 backups are kept
- **Cleanup**: Automatic after successful evolution
- **Naming**: `prompt-YYYYMMDD-HHMMSS.txt` + `.meta.json`

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Backup creation fails | Error returned, evolution aborted |
| Apply fails | Error returned, post-hook skipped |
| Benchmark script missing | Skip validation, accept prompt |
| Benchmark script fails | Rollback, return error |
| Regression > 10% | Rollback, return error with details |
| Rollback fails | Error returned (system may be in inconsistent state) |

## Testing

Unit tests are in `crates/evolution/src/safe_applier.rs`:

- `test_safe_applier_backup_and_rollback`: Verifies backup creation
- `test_benchmark_parsing`: Validates JSON parsing

Run tests:
```bash
cargo test --package harness-evolution
```

## Configuration

Default values (can be customized via `SafeApplier::with_paths()`):

- **Backup directory**: `~/.paperclip/harness/prompt-backups/`
- **Benchmark script**: `./scripts/run-evolution-benchmarks.sh`
- **Regression threshold**: 10% (0.10)

## Related

- **ANGA-900**: Self-Evolution Safety Gates with APT Hooks
- **ANGA-865**: APT Pattern Brainstorm (parent task)
- **ANGA-901**: `/review-self` command (APT pattern reference implementation)
