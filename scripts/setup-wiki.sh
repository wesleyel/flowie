#!/bin/sh
# Bootstrap the local roadmap wiki (`wiki/`) and the `to` CLI on a fresh clone.
# Safe to run repeatedly (idempotent): clones `wesleyel/flowie.wiki.git` using
# the same protocol (SSH/HTTPS) as `origin`, updates git excludes, writes `.to`,
# and installs `to` when missing or lacking `to scopes`.
set -eu

cd "$(dirname "$0")/.."

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "error: must be run inside the flowie git repository" >&2
    exit 1
fi

ensure_exclude_line() {
    file=$1
    line=$2
    mkdir -p "$(dirname "$file")"
    [ -f "$file" ] || : > "$file"
    if ! grep -Fxq -- "$line" "$file"; then
        if [ -s "$file" ] && [ "$(tail -c 1 "$file" | wc -l)" -eq 0 ]; then
            printf '\n' >> "$file"
        fi
        printf '%s\n' "$line" >> "$file"
    fi
}

wiki_url() {
    origin_url=$(git remote get-url origin 2>/dev/null || true)
    case "$origin_url" in
        http://* | https://*)
            printf 'https://github.com/wesleyel/flowie.wiki.git\n'
            ;;
        ssh://*)
            authority=${origin_url#ssh://}
            authority=${authority%%/*}
            printf 'ssh://%s/wesleyel/flowie.wiki.git\n' "$authority"
            ;;
        *@*:*)
            prefix=${origin_url%%:*}
            printf '%s:wesleyel/flowie.wiki.git\n' "$prefix"
            ;;
        *)
            printf 'https://github.com/wesleyel/flowie.wiki.git\n'
            ;;
    esac
}

if [ ! -e wiki/.git ]; then
    if [ -d wiki ] && [ -n "$(ls -A wiki)" ]; then
        echo "error: wiki/ exists and is not empty, without wiki/.git" >&2
        exit 1
    fi
    url=$(wiki_url)
    echo "Cloning $url into wiki/..."
    git clone "$url" wiki
fi

main_exclude=$(git rev-parse --git-path info/exclude)
ensure_exclude_line "$main_exclude" "/wiki/"
ensure_exclude_line "$main_exclude" "/.to"

wiki_exclude=$(git -C wiki rev-parse --git-path info/exclude)
case "$wiki_exclude" in
    /*) ;;
    *) wiki_exclude="wiki/$wiki_exclude" ;;
esac
ensure_exclude_line "$wiki_exclude" "/_private/"

if [ ! -e .to ]; then
    printf 'wiki\n' > .to
else
    to_target=$(tr -d '\r\n' < .to)
    if [ "$to_target" != "wiki" ] && [ ! -e "$to_target" ]; then
        echo "warning: .to points to '$to_target' (not found); expected 'wiki'" >&2
    fi
fi

if ! command -v to >/dev/null 2>&1 || ! to -C wiki scopes >/dev/null 2>&1; then
    if ! command -v cargo >/dev/null 2>&1; then
        echo "error: 'to' is missing or outdated and 'cargo' is not on PATH" >&2
        exit 1
    fi
    echo "Installing to from https://github.com/wesleyel/to..."
    if command -v to >/dev/null 2>&1; then
        cargo install --force --git https://github.com/wesleyel/to
    else
        cargo install --git https://github.com/wesleyel/to
    fi
fi

if command -v to >/dev/null 2>&1 && to scopes >/dev/null 2>&1; then
    echo "Wiki setup ready (wiki/, .to, and to CLI)."
else
    echo "error: wiki/ is ready, but 'to scopes' failed; check .to and Cargo's bin PATH" >&2
    exit 1
fi
