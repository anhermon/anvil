#!/usr/bin/env bash
# Stop hook for /review-self
# Generates PR checklist and suggests labels

set -e

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📋 PR Self-Review Checklist"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Get current branch and commit info
CURRENT_BRANCH=$(git branch --show-current)
MAIN_BRANCH=$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@')
[ -z "$MAIN_BRANCH" ] && MAIN_BRANCH="main"
COMMIT_COUNT=$(git rev-list --count origin/$MAIN_BRANCH..HEAD 2>/dev/null || echo "0")

# Generate the checklist
cat <<'CHECKLIST'
## Pre-Merge Checklist

Copy this checklist into your PR description:

```markdown
### Code Quality
- [ ] Branch created from latest main
- [ ] All tests pass locally (`cargo test --workspace`)
- [ ] Code formatted (`cargo fmt --all --check`)
- [ ] No clippy warnings (`cargo clippy --workspace -- -D warnings`)
- [ ] No temporary debugging code left in

### Documentation
- [ ] PR description links to Paperclip issue
- [ ] All public APIs have doc comments
- [ ] CHANGELOG updated (if applicable)

### Review Process
- [ ] All Greptile review comments addressed
- [ ] All Codex review comments addressed
- [ ] All CI checks green
- [ ] All human/manager review comments addressed

### Final Validation
- [ ] User Agent LGTM received
- [ ] No merge conflicts with target branch
- [ ] Commit messages follow format: `type(scope): description (ANGA-N)`
```

CHECKLIST

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🏷️  Suggested PR Labels"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Analyze changes to suggest labels
CHANGED_FILES=$(git diff --name-only origin/$MAIN_BRANCH..HEAD 2>/dev/null || echo "")

# Suggest labels based on file changes
if echo "$CHANGED_FILES" | grep -q "\.rs$"; then
  echo "  • rust"
fi

if echo "$CHANGED_FILES" | grep -q "^\.claude/skills/"; then
  echo "  • skill"
fi

if echo "$CHANGED_FILES" | grep -q "^\.github/workflows/"; then
  echo "  • ci"
fi

if echo "$CHANGED_FILES" | grep -q "test"; then
  echo "  • testing"
fi

if echo "$CHANGED_FILES" | grep -q "README\|\.md$"; then
  echo "  • documentation"
fi

# Determine type from commits
COMMIT_TYPES=$(git log origin/$MAIN_BRANCH..HEAD --format="%s" | grep -o "^[a-z]*" | sort -u)

if echo "$COMMIT_TYPES" | grep -q "feat"; then
  echo "  • enhancement"
fi

if echo "$COMMIT_TYPES" | grep -q "fix"; then
  echo "  • bug"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 PR Summary"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "Branch: $CURRENT_BRANCH"
echo "Commits: $COMMIT_COUNT"
echo "Files changed: $(echo "$CHANGED_FILES" | wc -l)"
echo ""
echo "✅ Self-review validation complete!"
echo "   Next: Create/update PR and request User Agent review"
echo ""
