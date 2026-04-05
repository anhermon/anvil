# Anvil Benchmark Suite

Measures build reliability, performance, and artifact correctness for every commit.

## Metrics

| Metric | What it measures | Perfect | Zero | Weight |
|---|---|---|---|---|
| `check_time` | `cargo check --workspace` duration | <= 30s | >= 120s | 0.10 |
| `build_time` | `cargo build --workspace` duration | <= 120s | >= 600s | 0.20 |
| `test_pass_rate` | Fraction of tests passing | 1.0 | 0.0 | 0.40 |
| `clippy_clean` | `cargo clippy -- -D warnings` passes | clean | any warning | 0.15 |
| `fmt_clean` | `cargo fmt --check` passes | clean | any diff | 0.10 |
| `binary_size` | Size of compiled binary | <= 50 MB | >= 200 MB | 0.05 |

**Overall** = weighted average of all scores. The benchmark **passes** if overall >= 0.7.

## Running locally

```bash
bash benchmarks/run.sh
```

Results are written to `benchmark-results.json` in the repo root.

The script requires a Rust toolchain (`cargo`, `rustfmt`, `clippy`) and optionally `jq` for
prettier JSON output. Exit code is 0 on pass, 1 on fail.

## CI integration

The benchmark runs automatically in the GitHub Actions CI workflow as a `benchmark` job
after tests pass. The `benchmark-results.json` file is uploaded as a workflow artifact.
