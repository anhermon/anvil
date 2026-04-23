#!/usr/bin/env bash
# check-commit-cadence.sh - Enforce commit cadence for workflow changes
#
# Prevents long-lived uncommitted changes beyond a defined threshold.
# Default threshold: 24 hours (86400 seconds)
# Override with: COMMIT_CADENCE_THRESHOLD_HOURS=48 ./check-commit-cadence.sh

set -euo pipefail

# Configuration
THRESHOLD_HOURS="${COMMIT_CADENCE_THRESHOLD_HOURS:-24}"
THRESHOLD_SECONDS=$((THRESHOLD_HOURS * 3600))
NOW=$(date +%s)

# Color codes
RED='\033[0;31m'
YELLOW='\033[1;33m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

echo "🕐 Checking commit cadence (threshold: ${THRESHOLD_HOURS}h)..."

# Check if we're in a git repository
if ! git rev-parse --git-dir > /dev/null 2>&1; then
  echo -e "${YELLOW}⚠️  Not a git repository - skipping cadence check${NC}"
  exit 0
fi

# Get list of modified tracked files
MODIFIED_FILES=$(git diff --name-only 2>/dev/null || echo "")
STAGED_FILES=$(git diff --cached --name-only 2>/dev/null || echo "")

# Combine and deduplicate
ALL_CHANGED_FILES=$(echo -e "${MODIFIED_FILES}\n${STAGED_FILES}" | sort -u | grep -v '^$' || echo "")

if [ -z "$ALL_CHANGED_FILES" ]; then
  echo -e "${GREEN}✅ No uncommitted changes - cadence check passed${NC}"
  exit 0
fi

# Track violations
VIOLATIONS=()
MAX_AGE=0

# Check each changed file's modification time
while IFS= read -r file; do
  if [ -f "$file" ]; then
    # Get file modification time (platform-compatible)
    if [ "$(uname)" = "Darwin" ]; then
      # macOS
      FILE_MTIME=$(stat -f %m "$file" 2>/dev/null || echo "$NOW")
    else
      # Linux
      FILE_MTIME=$(stat -c %Y "$file" 2>/dev/null || echo "$NOW")
    fi

    AGE=$((NOW - FILE_MTIME))

    if [ "$AGE" -gt "$THRESHOLD_SECONDS" ]; then
      AGE_HOURS=$((AGE / 3600))
      VIOLATIONS+=("$file (${AGE_HOURS}h old)")

      if [ "$AGE" -gt "$MAX_AGE" ]; then
        MAX_AGE=$AGE
      fi
    fi
  fi
done <<< "$ALL_CHANGED_FILES"

# Report results
if [ ${#VIOLATIONS[@]} -eq 0 ]; then
  CHANGED_COUNT=$(echo "$ALL_CHANGED_FILES" | wc -l | tr -d ' ')
  echo -e "${GREEN}✅ Cadence check passed - $CHANGED_COUNT file(s) within threshold${NC}"
  exit 0
else
  echo -e "${RED}❌ Commit cadence violation detected!${NC}"
  echo ""
  echo -e "${RED}The following files have been uncommitted for longer than ${THRESHOLD_HOURS}h:${NC}"
  for violation in "${VIOLATIONS[@]}"; do
    echo -e "  ${RED}→${NC} $violation"
  done
  echo ""
  echo -e "${YELLOW}Action required:${NC}"
  echo "  1. Commit your changes with a descriptive message and rationale"
  echo "  2. If not ready to commit, stash changes: git stash save 'WIP: description'"
  echo "  3. To bypass this check (not recommended): COMMIT_CADENCE_THRESHOLD_HOURS=999 git commit"
  echo ""
  echo -e "${YELLOW}Rationale:${NC} Frequent commits enable reliable branching, rollback, and traceable evolution."
  echo ""

  # Exit with error code
  exit 1
fi
