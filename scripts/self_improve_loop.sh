#!/usr/bin/env bash
# Self-improve loop: optional anvil agent run → fmt/clippy/test gates → e2e benchmark.
# If overall pass rate strictly improves vs the best seen in this worktree, commit all changes.
#
# Usage (from repo root):  bash scripts/self_improve_loop.sh
# Env: MODEL, PROVIDER, BENCH_PROVIDER (default: same as PROVIDER), BENCH_BASE_URL,
#      LOOPS, BENCH_ITER, TIER (default|hard), MAX_ITERATIONS, REPORT_DIR,
#      SKIP_AGENT (1=skip agent step), SELF_IMPROVE_GOAL
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || {
  echo "not a git repository" >&2
  exit 1
}
cd "$REPO_ROOT"

MODEL="${MODEL:-glm-4.7-lite}"
PROVIDER="${PROVIDER:-ollama}"
BENCH_PROVIDER="${BENCH_PROVIDER:-$PROVIDER}"
BENCH_BASE_URL="${BENCH_BASE_URL:-http://localhost:11434}"
LOOPS="${LOOPS:-3}"
BENCH_ITER="${BENCH_ITER:-3}"
TIER="${TIER:-default}"
MAX_ITERATIONS="${MAX_ITERATIONS:-20}"
REPORT_DIR="${REPORT_DIR:-target/self_improve_reports}"
SKIP_AGENT="${SKIP_AGENT:-0}"

GOAL="${SELF_IMPROVE_GOAL:-You are improving the paperclip-harness (anvil) repository. Requirements: (1) Keep \`cargo test --workspace\`, \`cargo clippy --workspace --all-targets -- -D warnings\`, and \`cargo fmt --check\` passing. (2) Prefer minimal, focused changes that help the e2e benchmark suite pass more often (see Taskfile \`bench:run\` / \`anvil-bench\`). (3) Do not refactor unrelated modules.}"

mkdir -p "$REPORT_DIR"
BEST_FILE="$REPORT_DIR/best_pass_rate.txt"

read_best() {
  if [[ -f "$BEST_FILE" ]]; then
    tr -d ' \n\r' <"$BEST_FILE"
  else
    echo "-1"
  fi
}

run_gates() {
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
}

run_benchmark() {
  local log="$1"
  local md_out="$2"
  # Build argv in one array — under `set -u`, an empty `"${tier_args[@]}"` is unbound on some bash.
  local -a cmd=(
    cargo run --release -p harness-benchmark --bin anvil-bench --
    --iterations "$BENCH_ITER"
    --model "$MODEL"
    --base-url "$BENCH_BASE_URL"
    --provider "$BENCH_PROVIDER"
  )
  if [[ "$TIER" == "hard" ]]; then
    cmd+=(--tier hard)
  fi
  cmd+=(--summary-md "$md_out")
  set +e
  "${cmd[@]}" >"$log" 2>&1
  local rc=$?
  set -e
  return "$rc"
}

extract_pass_rate() {
  local log="$1"
  awk '/^BENCH_OVERALL_PASS_RATE / {print $2; exit}' "$log"
}

BEST="$(read_best)"
echo "Stored best BENCH_OVERALL_PASS_RATE: ${BEST} (file: ${BEST_FILE})"

for i in $(seq 1 "$LOOPS"); do
  echo ""
  echo "==================== self-improve loop $i / $LOOPS ===================="

  if [[ "$SKIP_AGENT" != "1" ]]; then
    echo "--- anvil agent ---"
    cargo run --release -p harness-cli -- run \
      --provider "$PROVIDER" \
      --model "$MODEL" \
      --max_iterations "$MAX_ITERATIONS" \
      --goal "$GOAL"
  else
    echo "--- anvil agent (skipped SKIP_AGENT=1) ---"
  fi

  echo "--- quality gates ---"
  run_gates

  TS="$(date +%Y%m%d-%H%M%S)"
  LOG="$REPORT_DIR/bench_${i}_${TS}.log"
  MD="$REPORT_DIR/capability_loop${i}_${TS}.md"

  echo "--- benchmark (log: $LOG) ---"
  run_benchmark "$LOG" "$MD" || {
    echo "benchmark failed; see $LOG" >&2
    exit 1
  }

  SCORE="$(extract_pass_rate "$LOG")"
  if [[ -z "$SCORE" ]]; then
    echo "could not parse BENCH_OVERALL_PASS_RATE from $LOG" >&2
    exit 1
  fi
  echo "BENCH_OVERALL_PASS_RATE this loop: $SCORE"

  if python3 -c "import sys; b=float(sys.argv[1]); s=float(sys.argv[2]); sys.exit(0 if s>b+1e-9 else 1)" "$BEST" "$SCORE"; then
    echo "--- committing (strict improvement: $SCORE > $BEST) ---"
    git add -A
    git diff --cached --quiet && {
      echo "no staged changes; skipping commit" >&2
    } || git commit -m "bench(self-improve): pass rate ${SCORE} (loop ${i}; model ${MODEL})"
    echo "$SCORE" >"$BEST_FILE"
    BEST="$SCORE"
    cp -f "$MD" "$REPORT_DIR/latest_capability.md"
  else
    echo "No commit (score $SCORE did not beat best $BEST)."
  fi
done

echo ""
echo "Done. Latest capability markdown: $REPORT_DIR/latest_capability.md (if any commit ran)"
