#!/usr/bin/env bash
# generate-change-history.sh - Generate machine-readable change history
#
# Outputs structured JSON containing git commit history with parsed metadata.
# Usage: ./generate-change-history.sh [--since DATE] [--limit N] [--output FILE]
#
# Examples:
#   ./generate-change-history.sh --since "2026-04-01"
#   ./generate-change-history.sh --limit 50 --output history.json
#   ./generate-change-history.sh --since "1 week ago" --output weekly.json

set -euo pipefail

# Defaults
SINCE=""
LIMIT=""
OUTPUT=""
FORMAT="json"

# Parse arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --since)
      SINCE="$2"
      shift 2
      ;;
    --limit)
      LIMIT="$2"
      shift 2
      ;;
    --output)
      OUTPUT="$2"
      shift 2
      ;;
    --format)
      FORMAT="$2"
      shift 2
      ;;
    --help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Options:"
      echo "  --since DATE    Only commits since this date (e.g., '2026-04-01', '1 week ago')"
      echo "  --limit N       Maximum number of commits to include"
      echo "  --output FILE   Write to file instead of stdout"
      echo "  --format TYPE   Output format: json (default), yaml, csv"
      echo "  --help          Show this help message"
      echo ""
      echo "Examples:"
      echo "  $0 --since '2026-04-01'"
      echo "  $0 --limit 50 --output history.json"
      echo "  $0 --since '1 week ago' --format yaml"
      exit 0
      ;;
    *)
      echo "Unknown option: $1"
      echo "Use --help for usage information"
      exit 1
      ;;
  esac
done

# Check if we're in a git repository
if ! git rev-parse --git-dir > /dev/null 2>&1; then
  echo "Error: Not a git repository" >&2
  exit 1
fi

# Build git log command
GIT_CMD="git log"

if [ -n "$SINCE" ]; then
  GIT_CMD="$GIT_CMD --since='$SINCE'"
fi

if [ -n "$LIMIT" ]; then
  GIT_CMD="$GIT_CMD -n $LIMIT"
fi

# Custom format to extract all needed fields
# Format: COMMIT_HASH|AUTHOR_NAME|AUTHOR_EMAIL|DATE_ISO|SUBJECT|BODY
GIT_CMD="$GIT_CMD --pretty=format:'COMMIT_START%nHASH:%H%nAUTHOR:%an%nEMAIL:%ae%nDATE:%aI%nSUBJECT:%s%nBODY_START%n%b%nBODY_END%nCOMMIT_END'"

# Execute git log and capture output
GIT_OUTPUT=$(eval "$GIT_CMD" || echo "")

if [ -z "$GIT_OUTPUT" ]; then
  echo "[]" # Empty JSON array if no commits
  exit 0
fi

# Parse git output and generate JSON
generate_json() {
  local output="["
  local first=true
  local in_commit=false
  local in_body=false
  local hash="" author="" email="" date="" subject="" body=""

  while IFS= read -r line; do
    case "$line" in
      COMMIT_START)
        in_commit=true
        hash="" author="" email="" date="" subject="" body=""
        ;;
      HASH:*)
        hash="${line#HASH:}"
        ;;
      AUTHOR:*)
        author="${line#AUTHOR:}"
        ;;
      EMAIL:*)
        email="${line#EMAIL:}"
        ;;
      DATE:*)
        date="${line#DATE:}"
        ;;
      SUBJECT:*)
        subject="${line#SUBJECT:}"
        ;;
      BODY_START)
        in_body=true
        body=""
        ;;
      BODY_END)
        in_body=false
        ;;
      COMMIT_END)
        if [ "$in_commit" = true ]; then
          # Parse commit type and scope from subject
          commit_type=$(echo "$subject" | sed -E 's/^([a-z]+)(\(.+\))?: .*/\1/' || echo "unknown")
          commit_scope=$(echo "$subject" | sed -E 's/^[a-z]+\(([^)]+)\): .*/\1/' || echo "")
          short_desc=$(echo "$subject" | sed -E 's/^[a-z]+(\([^)]+\))?: //' || echo "$subject")

          # Extract rationale from body
          rationale=$(echo "$body" | grep -iE "^(Rationale|Why):" | sed -E 's/^(Rationale|Why):[[:space:]]*//' || echo "")

          # Escape JSON special characters
          hash_escaped=$(echo "$hash" | sed 's/"/\\"/g')
          author_escaped=$(echo "$author" | sed 's/"/\\"/g')
          email_escaped=$(echo "$email" | sed 's/"/\\"/g')
          subject_escaped=$(echo "$subject" | sed 's/"/\\"/g; s/\\/\\\\/g')
          short_desc_escaped=$(echo "$short_desc" | sed 's/"/\\"/g; s/\\/\\\\/g')
          rationale_escaped=$(echo "$rationale" | sed 's/"/\\"/g; s/\\/\\\\/g')
          body_escaped=$(echo "$body" | sed 's/"/\\"/g; s/\\/\\\\/g; :a;N;$!ba;s/\n/\\n/g')

          # Add comma separator
          if [ "$first" = true ]; then
            first=false
          else
            output="$output,"
          fi

          # Build JSON object
          output="$output
  {
    \"hash\": \"$hash_escaped\",
    \"author\": \"$author_escaped\",
    \"email\": \"$email_escaped\",
    \"date\": \"$date\",
    \"subject\": \"$subject_escaped\",
    \"type\": \"$commit_type\",
    \"scope\": \"$commit_scope\",
    \"description\": \"$short_desc_escaped\",
    \"rationale\": \"$rationale_escaped\",
    \"body\": \"$body_escaped\"
  }"

          in_commit=false
        fi
        ;;
      *)
        if [ "$in_body" = true ]; then
          if [ -z "$body" ]; then
            body="$line"
          else
            body="$body"$'\n'"$line"
          fi
        fi
        ;;
    esac
  done <<< "$GIT_OUTPUT"

  output="$output
]"

  echo "$output"
}

# Generate output based on format
case "$FORMAT" in
  json)
    RESULT=$(generate_json)
    ;;
  *)
    echo "Error: Unsupported format '$FORMAT'. Only 'json' is currently supported." >&2
    exit 1
    ;;
esac

# Write to output file or stdout
if [ -n "$OUTPUT" ]; then
  echo "$RESULT" > "$OUTPUT"
  echo "Change history written to: $OUTPUT" >&2
else
  echo "$RESULT"
fi
