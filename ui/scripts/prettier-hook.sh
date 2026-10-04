#!/bin/bash
FILE=$(jq -r '.tool_response.filePath // .tool_input.file_path')
# Format only files under ui/ in whichever checkout they belong to (the main
# checkout or a git worktree). JSON elsewhere in the repo, such as
# ramus-tauri/data/open.json, keeps its own formatting, and files outside any
# repository are left alone.
REL_DIR=$(git -C "$(dirname "$FILE")" rev-parse --show-prefix 2>/dev/null) || exit 0
case "$REL_DIR" in
  ui/*) ;;
  *) exit 0 ;;
esac
if echo "$FILE" | grep -qE '\.(ts|tsx|css|json)$'; then
  cd "$(dirname "$0")/.." && npx prettier --write "$FILE" 2>/dev/null
fi
exit 0
