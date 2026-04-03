#!/usr/bin/env bash
# Install git hooks for paperclip-harness development.
# Uses core.hooksPath so hooks stay in the repo and update automatically.
set -e

REPO_ROOT="$(git rev-parse --show-toplevel)"

git config core.hooksPath "$REPO_ROOT/.githooks"

echo "Git hooks installed. Hooks path set to .githooks/"
echo "Run 'bash scripts/setup-hooks.sh' once per clone to activate."
