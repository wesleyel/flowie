#!/bin/sh
# Print one version's section (without its heading), for release notes.
# Usage: scripts/changelog-section.sh 1.0.0 [CHANGELOG.zh-CN.md]
set -eu
version="$1"
changelog="${2:-CHANGELOG.md}"
file="$(dirname "$0")/../$changelog"
notes=$(awk -v v="$version" '
  /^## \[/ { if (found) exit; if (index($0, "## [" v "]") == 1) { found = 1; next } }
  /^\[[^]]+\]: / { next }
  found { print }
' "$file" | sed -e '/./,$!d')
if [ -z "$(printf '%s' "$notes" | tr -d '[:space:]')" ]; then
  echo "$changelog has no entry for $version" >&2
  exit 1
fi
printf '%s\n' "$notes"
