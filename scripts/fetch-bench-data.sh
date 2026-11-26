#!/usr/bin/env bash
# Download the documents the large-outline benchmark can run on (not kept in git).
# usage: scripts/fetch-bench-data.sh [outdir]   (default: target/bench-data)
#
# - moby_dick.opml: Moby-Dick as a Bike outline, from jessegrosjean/mobydickworkout, the file
#   the Discussion #7 report imported five times.
#
# Then: cargo bench -p flowie-core --bench large_outline
# The benchmark reads target/bench-data/moby_dick.opml when it is there (or the file in
# FLOWIE_BENCH_OPML) and imports it FLOWIE_BENCH_SCALE times (default 5).
set -euo pipefail
OUT="${1:-target/bench-data}"
URL=https://github.com/jessegrosjean/mobydickworkout/raw/main/moby_dick.opml.zip
mkdir -p "$OUT"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
curl -fsSL "$URL" -o "$TMP/moby_dick.opml.zip"
unzip -q -o "$TMP/moby_dick.opml.zip" -d "$TMP/unzipped"
OPML="$(find "$TMP/unzipped" -name '*.opml' -not -path '*/__MACOSX/*' | head -n 1)"
[ -n "$OPML" ] || { echo "no .opml in $URL" >&2; exit 1; }
mv "$OPML" "$OUT/moby_dick.opml"
echo "$OUT/moby_dick.opml"
