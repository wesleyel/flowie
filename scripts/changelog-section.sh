#!/bin/sh
# Print one version's section of CHANGELOG.md (without its heading), for release notes.
# Usage: scripts/changelog-section.sh 1.0.0   — exits 1 if the version has no entry.
set -eu
version="$1"
file="$(dirname "$0")/../CHANGELOG.md"
notes=$(awk -v v="$version" '
  /^## \[/ { if (found) exit; if (index($0, "## [" v "]") == 1) { found = 1; next } }
  /^\[[^]]+\]: / { next }
  found { print }
' "$file" | sed -e '/./,$!d')
if [ -z "$(printf '%s' "$notes" | tr -d '[:space:]')" ]; then
  echo "CHANGELOG.md has no entry for $version" >&2
  exit 1
fi
printf '%s\n' "$notes"
