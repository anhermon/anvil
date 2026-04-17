#!/usr/bin/env bash
# Pre-hook validation for /review-self
# Validates branch state, CI status, and merge conflicts

set -e

echo "🔍 Pre-review validation starting..."

# Get current branch
CURRENT_BRANCH=$(git branch --show-current)
echo "📍 Current branch: $CURRENT_BRANCH"

# Check if we're on a feature branch
if [[ ! "$CURRENT_BRANCH" =~ ^(feature|fix)/ ]]; then
  echo "⚠️  WARNING: Not on a feature/ or fix/ branch"
fi

# Check for uncommitted changes
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "⚠️  WARNING: You have uncommitted changes"
  git status --short
fi

# Fetch latest main to compare
echo "📥 Fetching latest main..."
git fetch origin main --quiet 2>/dev/null || git fetch origin master --quiet 2>/dev/null || echo "Could not fetch origin"

# Check if branch is up-to-date with main
MAIN_BRANCH=$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@' || echo "main")
if git rev-parse --verify "origin/$MAIN_BRANCH" >/dev/null 2>&1; then
  BEHIND=$(git rev-list --count HEAD..origin/$MAIN_BRANCH 2>/dev/null || echo "0")
  AHEAD=$(git rev-list --count origin/$MAIN_BRANCH..HEAD 2>/dev/null || echo "0")
  
  echo "📊 Branch status: $AHEAD ahead, $BEHIND behind origin/$MAIN_BRANCH"
  
  if [ "$BEHIND" -gt 0 ]; then
    echo "⚠️  WARNING: Branch is $BEHIND commits behind origin/$MAIN_BRANCH"
    echo "   Consider rebasing or merging latest changes"
  fi
fi

# Check for merge conflicts
if git ls-files -u | grep -q .; then
  echo "❌ BLOCKING: Merge conflicts detected"
  git ls-files -u
  exit 1
fi

# Check all commits reference a Paperclip issue
echo "🔍 Checking commit messages..."
COMMITS_WITHOUT_ISSUE=$(git log origin/$MAIN_BRANCH..HEAD --oneline | grep -v -E '(ANGA-[0-9]+|Merge)' | wc -l)
if [ "$COMMITS_WITHOUT_ISSUE" -gt 0 ]; then
  echo "⚠️  WARNING: $COMMITS_WITHOUT_ISSUE commit(s) don't reference a Paperclip issue (ANGA-N)"
  git log origin/$MAIN_BRANCH..HEAD --oneline | grep -v -E '(ANGA-[0-9]+|Merge)' | head -5
fi

# Check CI status if GitHub CLI is available
if command -v gh &> /dev/null; then
  echo "🔍 Checking CI status..."
  if gh pr view --json statusCheckRollup -q '.statusCheckRollup[] | select(.conclusion != "SUCCESS") | .name' 2>/dev/null | grep -q .; then
    echo "⚠️  WARNING: Some CI checks are not passing"
    gh pr view --json statusCheckRollup -q '.statusCheckRollup[] | select(.conclusion != "SUCCESS") | "\(.name): \(.conclusion)"'
  else
    echo "✅ CI checks status: OK (or no PR found)"
  fi
fi

echo "✅ Pre-validation complete"
