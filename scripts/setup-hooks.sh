#!/usr/bin/env bash
# Install git hooks for paperclip-harness development
set -e
REPO_ROOT="$(git rev-parse --show-toplevel)"
HOOKS_DIR="$REPO_ROOT/.githooks"
GIT_HOOKS="$REPO_ROOT/.git/hooks"

mkdir -p "$HOOKS_DIR"

# Copy hooks from .githooks/ to .git/hooks/
for hook in pre-commit pre-push commit-msg; do
  if [ -f "$HOOKS_DIR/$hook" ]; then
    cp "$HOOKS_DIR/$hook" "$GIT_HOOKS/$hook"
    chmod +x "$GIT_HOOKS/$hook"
    echo "✅ Installed $hook"
  fi
done

echo "🎉 Git hooks installed. Run 'cargo fmt --all' and 'cargo clippy --workspace' before committing."
