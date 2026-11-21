#!/usr/bin/env bash
# Write a new version and sha256 into a cask in wesleyel/homebrew-tap and push it.
# usage: scripts/update-tap-cask.sh VERSION SHA256_FILE [CASK]
#   SHA256_FILE: the .sha256 next to the release zip
#   CASK: defaults to flowie, which takes every release, release candidates included
# Needs GH_TOKEN with contents:write on wesleyel/homebrew-tap (secret HOMEBREW_TAP_GITHUB_TOKEN).
set -euo pipefail

version=$1
cask=${3:-flowie}
sha=$(awk 'NR == 1 { print $1 }' "$2")
[[ $sha =~ ^[0-9a-f]{64}$ ]] || { echo "::error::no sha256 in $2"; exit 1; }
[ -n "${GH_TOKEN:-}" ] || { echo "::error::HOMEBREW_TAP_GITHUB_TOKEN is not set"; exit 1; }

dir=$(mktemp -d)
git clone --quiet --depth 1 "https://x-access-token:${GH_TOKEN}@github.com/wesleyel/homebrew-tap.git" "$dir"
file="$dir/Casks/$cask.rb"
[ -f "$file" ] || { echo "::error::wesleyel/homebrew-tap has no Casks/$cask.rb"; exit 1; }
sed -i.bak -E \
  -e "s/^  version \"[^\"]*\"/  version \"$version\"/" \
  -e "s/^  sha256 \"[0-9a-f]{64}\"/  sha256 \"$sha\"/" "$file"
rm -f "$file.bak"

cd "$dir"
if git diff --quiet; then
  echo "$cask is already at $version ($sha)"
  exit 0
fi
git -c user.name="github-actions[bot]" \
    -c user.email="41898282+github-actions[bot]@users.noreply.github.com" \
    commit --quiet -am "$cask $version"
# Other repositories' release workflows push to the tap too: rebase and retry.
for _ in 1 2 3; do
  git push --quiet origin HEAD:main && { echo "pushed $cask $version"; exit 0; }
  git pull --quiet --rebase origin main
done
echo "::error::could not push $cask"
exit 1
