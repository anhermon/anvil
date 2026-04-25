#!/usr/bin/env bash
# benchmarks/run.sh — Anvil build-quality benchmark suite
# Measures build reliability, performance, and artifact correctness.
# Outputs benchmark-results.json in the repo root.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

COMMIT="$(git rev-parse HEAD 2>/dev/null || echo "unknown")"
TIMESTAMP="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

###############################################################################
# Helpers
###############################################################################

elapsed() {
  # Portable seconds-level timer using bash SECONDS
  echo "$SECONDS"
}

# normalize: value, perfect_threshold, zero_threshold -> score in [0,1]
normalize() {
  local val="$1" perfect="$2" zero="$3"
  if command -v bc >/dev/null 2>&1; then
    bc -l <<EOF
      v = $val; p = $perfect; z = $zero
      if (v <= p) 1.0 else if (v >= z) 0.0 else (z - v) / (z - p)
EOF
  else
    # Pure-bash fallback (integer approximation)
    if [ "$val" -le "$perfect" ] 2>/dev/null; then
      echo "1.0"
    elif [ "$val" -ge "$zero" ] 2>/dev/null; then
      echo "0.0"
    else
      # linear interpolation with awk
      awk -v v="$val" -v p="$perfect" -v z="$zero" 'BEGIN { printf "%.4f", (z - v) / (z - p) }'
    fi
  fi
}

# Trim whitespace from a value
trim() {
  echo "$1" | tr -d '[:space:]'
}

###############################################################################
# 1. cargo check
###############################################################################

echo "==> cargo check --workspace"
SECONDS=0
if cargo check --workspace 2>&1; then
  CHECK_OK=1
else
  CHECK_OK=0
fi
CHECK_TIME=$SECONDS
echo "    check completed in ${CHECK_TIME}s"

###############################################################################
# 2. cargo build
###############################################################################

echo "==> cargo build --workspace"
SECONDS=0
if cargo build --workspace 2>&1; then
  BUILD_OK=1
else
  BUILD_OK=0
fi
BUILD_TIME=$SECONDS
echo "    build completed in ${BUILD_TIME}s"

###############################################################################
# 3. cargo test
###############################################################################

echo "==> cargo test --workspace"
TEST_OUTPUT="$(cargo test --workspace 2>&1)" || true
echo "$TEST_OUTPUT"

# Parse test summary line: "test result: ok. X passed; Y failed; Z ignored; ..."
TOTAL_PASSED=0
TOTAL_FAILED=0
while IFS= read -r line; do
  if echo "$line" | grep -qE '^test result:'; then
    p=$(echo "$line" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' || echo 0)
    f=$(echo "$line" | grep -oE '[0-9]+ failed' | grep -oE '[0-9]+' || echo 0)
    TOTAL_PASSED=$((TOTAL_PASSED + p))
    TOTAL_FAILED=$((TOTAL_FAILED + f))
  fi
done <<< "$TEST_OUTPUT"

TOTAL_TESTS=$((TOTAL_PASSED + TOTAL_FAILED))
if [ "$TOTAL_TESTS" -gt 0 ]; then
  TEST_PASS_RATE=$(awk -v p="$TOTAL_PASSED" -v t="$TOTAL_TESTS" 'BEGIN { printf "%.4f", p / t }')
else
  # No tests found; treat as 0
  TEST_PASS_RATE="0.0"
fi
echo "    tests: $TOTAL_PASSED passed, $TOTAL_FAILED failed (rate: $TEST_PASS_RATE)"

###############################################################################
# 4. cargo clippy
###############################################################################

echo "==> cargo clippy --workspace -- -D warnings"
if cargo clippy --workspace --all-targets -- -D warnings 2>&1; then
  CLIPPY_CLEAN="1.0"
else
  CLIPPY_CLEAN="0.0"
fi
echo "    clippy_clean: $CLIPPY_CLEAN"

###############################################################################
# 5. cargo fmt --check
###############################################################################

echo "==> cargo fmt --check"
if cargo fmt --all -- --check 2>&1; then
  FMT_CLEAN="1.0"
else
  FMT_CLEAN="0.0"
fi
echo "    fmt_clean: $FMT_CLEAN"

###############################################################################
# 6. Binary size
###############################################################################

echo "==> measuring binary size"
# Look for release binary first, then debug
BIN_PATH=""
if [ -d "target/release" ]; then
  BIN_PATH="$(find target/release -maxdepth 1 -type f -executable ! -name '*.d' ! -name '*.so' | head -1 || true)"
fi
if [ -z "$BIN_PATH" ] && [ -d "target/debug" ]; then
  BIN_PATH="$(find target/debug -maxdepth 1 -type f -executable ! -name '*.d' ! -name '*.so' ! -name 'build-script-build' | head -1 || true)"
fi

BINARY_SIZE_BYTES=0
if [ -n "$BIN_PATH" ] && [ -f "$BIN_PATH" ]; then
  BINARY_SIZE_BYTES=$(stat -c%s "$BIN_PATH" 2>/dev/null || stat -f%z "$BIN_PATH" 2>/dev/null || echo 0)
  echo "    binary: $BIN_PATH ($BINARY_SIZE_BYTES bytes)"
else
  echo "    no binary found; scoring binary_size as 1.0"
fi

###############################################################################
# 7. Compute scores
###############################################################################

# Normalize times
CHECK_SCORE=$(trim "$(normalize "$CHECK_TIME" 30 120)")
BUILD_SCORE=$(trim "$(normalize "$BUILD_TIME" 120 600)")

# Binary size score: under 50MB = 1.0, linear decay to 0 at 200MB
if [ "$BINARY_SIZE_BYTES" -eq 0 ] 2>/dev/null; then
  BINARY_SCORE="1.0"
else
  BINARY_MB=$(awk -v b="$BINARY_SIZE_BYTES" 'BEGIN { printf "%.2f", b / 1048576 }')
  BINARY_SCORE=$(trim "$(normalize "$BINARY_MB" 50 200)")
fi

# Weighted overall: test_pass_rate: 0.4, build_time: 0.2, clippy: 0.15,
#                   check_time: 0.1, fmt: 0.1, binary_size: 0.05
OVERALL=$(awk -v tr="$TEST_PASS_RATE" -v cl="$CLIPPY_CLEAN" -v fm="$FMT_CLEAN" \
  -v bs="$BUILD_SCORE" -v cs="$CHECK_SCORE" -v bn="$BINARY_SCORE" \
  'BEGIN { printf "%.4f", tr*0.4 + cl*0.15 + fm*0.1 + bs*0.2 + cs*0.1 + bn*0.05 }')

# Pass if overall >= 0.7
PASS=false
if awk -v ov="$OVERALL" 'BEGIN { exit (ov >= 0.7) ? 0 : 1 }'; then
  PASS=true
fi

echo ""
echo "==> Scores"
echo "    check_time:     $CHECK_SCORE  (${CHECK_TIME}s)"
echo "    build_time:     $BUILD_SCORE  (${BUILD_TIME}s)"
echo "    test_pass_rate: $TEST_PASS_RATE  ($TOTAL_PASSED/$TOTAL_TESTS)"
echo "    clippy_clean:   $CLIPPY_CLEAN"
echo "    fmt_clean:      $FMT_CLEAN"
echo "    binary_size:    $BINARY_SCORE  (${BINARY_SIZE_BYTES} bytes)"
echo "    overall:        $OVERALL"
echo "    pass:           $PASS"

###############################################################################
# 8. Write JSON
###############################################################################

OUTPUT="$REPO_ROOT/benchmark-results.json"

if command -v jq >/dev/null 2>&1; then
  jq -n \
    --arg repo "anvil" \
    --arg commit "$COMMIT" \
    --arg ts "$TIMESTAMP" \
    --argjson check_score "$CHECK_SCORE" \
    --argjson build_score "$BUILD_SCORE" \
    --argjson test_rate "$TEST_PASS_RATE" \
    --argjson clippy "$CLIPPY_CLEAN" \
    --argjson fmt "$FMT_CLEAN" \
    --argjson binsize "$BINARY_SCORE" \
    --argjson overall "$OVERALL" \
    --argjson pass "$PASS" \
    --argjson check_time "$CHECK_TIME" \
    --argjson build_time "$BUILD_TIME" \
    --argjson binary_bytes "$BINARY_SIZE_BYTES" \
    '{
      repo: $repo,
      commit: $commit,
      timestamp: $ts,
      scores: [
        { name: "check_time",     value: $check_score, unit: "ratio", raw_seconds: $check_time },
        { name: "build_time",     value: $build_score,  unit: "ratio", raw_seconds: $build_time },
        { name: "test_pass_rate", value: $test_rate,    unit: "ratio" },
        { name: "clippy_clean",   value: $clippy,       unit: "ratio" },
        { name: "fmt_clean",      value: $fmt,          unit: "ratio" },
        { name: "binary_size",    value: $binsize,      unit: "ratio", raw_bytes: $binary_bytes }
      ],
      overall: $overall,
      pass: $pass
    }' > "$OUTPUT"
else
  printf '{\n' > "$OUTPUT"
  printf '  "repo": "anvil",\n' >> "$OUTPUT"
  printf '  "commit": "%s",\n' "$COMMIT" >> "$OUTPUT"
  printf '  "timestamp": "%s",\n' "$TIMESTAMP" >> "$OUTPUT"
  printf '  "scores": [\n' >> "$OUTPUT"
  printf '    { "name": "check_time",     "value": %s, "unit": "ratio" },\n' "$CHECK_SCORE" >> "$OUTPUT"
  printf '    { "name": "build_time",     "value": %s, "unit": "ratio" },\n' "$BUILD_SCORE" >> "$OUTPUT"
  printf '    { "name": "test_pass_rate", "value": %s, "unit": "ratio" },\n' "$TEST_PASS_RATE" >> "$OUTPUT"
  printf '    { "name": "clippy_clean",   "value": %s, "unit": "ratio" },\n' "$CLIPPY_CLEAN" >> "$OUTPUT"
  printf '    { "name": "fmt_clean",      "value": %s, "unit": "ratio" },\n' "$FMT_CLEAN" >> "$OUTPUT"
  printf '    { "name": "binary_size",    "value": %s, "unit": "ratio" }\n' "$BINARY_SCORE" >> "$OUTPUT"
  printf '  ],\n' >> "$OUTPUT"
  printf '  "overall": %s,\n' "$OVERALL" >> "$OUTPUT"
  printf '  "pass": %s\n' "$PASS" >> "$OUTPUT"
  printf '}\n' >> "$OUTPUT"
fi

echo ""
echo "==> Results written to $OUTPUT"

if [ "$PASS" = "true" ]; then
  echo "BENCHMARK PASSED (overall: $OVERALL)"
  exit 0
else
  echo "BENCHMARK FAILED (overall: $OVERALL < 0.7)"
  exit 1
fi
