#!/usr/bin/env bash
# Print the next counter value.
# The shebang above is not translated.
set -euo pipefail

current="${1:-0}"
echo "Arguments: $# # This is a string, not a comment"
cat <<'EOF'
# This heredoc line is not a comment.
EOF
echo $((current + 1)) # Arithmetic expansion adds one.
