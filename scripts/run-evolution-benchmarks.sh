#!/usr/bin/env bash
# Evolution benchmark runner — validates prompt quality through build/test/lint metrics.
#
# Outputs JSON in the format:
# {
#   "score": 0.0-1.0,
#   "metrics": {
#     "build_success": 0.0-1.0,
#     "test_pass_rate": 0.0-1.0,
#     "clippy_clean": 0.0-1.0,
#     "fmt_clean": 0.0-1.0
#   }
# }

set -euo pipefail

# Navigate to project root
cd "$(dirname "$0")/.."

# Metrics (default to 0.0, set to 1.0 on success)
build_success=0.0
test_pass_rate=0.0
clippy_clean=0.0
fmt_clean=0.0

# 1. Build check (25% weight)
if cargo build --workspace --all-targets 2>/dev/null; then
    build_success=1.0
fi

# 2. Test pass rate (35% weight)
if cargo test --workspace --no-fail-fast 2>&1 | tee /tmp/test-output.txt; then
    # Parse test results to get pass rate
    passed=$(grep -oP '\d+(?= passed)' /tmp/test-output.txt | tail -1 || echo "0")
    failed=$(grep -oP '\d+(?= failed)' /tmp/test-output.txt | tail -1 || echo "0")
    total=$((passed + failed))

    if [ "$total" -gt 0 ]; then
        test_pass_rate=$(awk "BEGIN {printf \"%.2f\", $passed / $total}")
    else
        # No tests = assume passing
        test_pass_rate=1.0
    fi
else
    # Tests failed, but still calculate pass rate from output
    passed=$(grep -oP '\d+(?= passed)' /tmp/test-output.txt | tail -1 || echo "0")
    failed=$(grep -oP '\d+(?= failed)' /tmp/test-output.txt | tail -1 || echo "0")
    total=$((passed + failed))

    if [ "$total" -gt 0 ]; then
        test_pass_rate=$(awk "BEGIN {printf \"%.2f\", $passed / $total}")
    fi
fi

# 3. Clippy clean (20% weight)
if cargo clippy --workspace --all-targets -- -D warnings 2>/dev/null; then
    clippy_clean=1.0
fi

# 4. Fmt clean (20% weight)
if cargo fmt --all -- --check 2>/dev/null; then
    fmt_clean=1.0
fi

# Calculate weighted overall score
# Weights: build=0.25, test=0.35, clippy=0.20, fmt=0.20
score=$(awk "BEGIN {
    score = ($build_success * 0.25) + \
            ($test_pass_rate * 0.35) + \
            ($clippy_clean * 0.20) + \
            ($fmt_clean * 0.20);
    printf \"%.3f\", score
}")

# Output JSON
cat <<EOF
{
  "score": $score,
  "metrics": {
    "build_success": $build_success,
    "test_pass_rate": $test_pass_rate,
    "clippy_clean": $clippy_clean,
    "fmt_clean": $fmt_clean
  }
}
EOF
