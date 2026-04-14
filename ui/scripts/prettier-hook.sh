#!/bin/bash
FILE=$(jq -r '.tool_response.filePath // .tool_input.file_path')
if echo "$FILE" | grep -qE '\.(ts|tsx|css|json)$'; then
  cd "$(dirname "$0")/.." && npx prettier --write "$FILE" 2>/dev/null
fi
exit 0
