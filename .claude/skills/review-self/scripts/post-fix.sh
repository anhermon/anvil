#!/usr/bin/env bash
# Post-hook automation for /review-self
# Runs fmt, clippy, and checks for TODOs

set -e

echo "🔧 Post-review fixes starting..."

# Run cargo fmt
echo "📝 Running cargo fmt..."
if cargo fmt --all --check 2>&1 | grep -q "Diff"; then
  echo "⚠️  Code needs formatting, applying fixes..."
  cargo fmt --all
  echo "✅ Code formatted"
else
  echo "✅ Code already formatted"
fi

# Run cargo clippy
echo "📎 Running cargo clippy..."
if ! cargo clippy --workspace --all-targets --all-features -- -D warnings 2>&1; then
  echo "⚠️  Clippy warnings/errors found - please fix before requesting review"
  exit 1
else
  echo "✅ Clippy checks passed"
fi

# Check for TODO comments
echo "🔍 Checking for TODO comments..."
TODO_COUNT=$(grep -r "TODO\|FIXME\|XXX" --include="*.rs" --exclude-dir=target . 2>/dev/null | wc -l || echo "0")
if [ "$TODO_COUNT" -gt 0 ]; then
  echo "⚠️  Found $TODO_COUNT TODO/FIXME/XXX comments:"
  grep -r "TODO\|FIXME\|XXX" --include="*.rs" --exclude-dir=target -n . 2>/dev/null | head -10
  echo "   Consider addressing these before requesting review"
else
  echo "✅ No TODO comments found"
fi

# Check commit message format
echo "🔍 Validating commit messages..."
MAIN_BRANCH=$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "main")
INVALID_COMMITS=$(git log origin/$MAIN_BRANCH..HEAD --format="%s" | grep -v -E '^(feat|fix|docs|style|refactor|perf|test|chore|build|ci|revert)(\(.+\))?: .+ \(ANGA-[0-9]+\)$' | wc -l || echo "0")

if [ "$INVALID_COMMITS" -gt 0 ]; then
  echo "⚠️  Found $INVALID_COMMITS commit(s) with invalid format"
  echo "   Expected: type(scope): description (ANGA-N)"
  git log origin/$MAIN_BRANCH..HEAD --format="%s" | grep -v -E '^(feat|fix|docs|style|refactor|perf|test|chore|build|ci|revert)(\(.+\))?: .+ \(ANGA-[0-9]+\)$' | head -5
fi

echo "✅ Post-validation complete"
