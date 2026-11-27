#!/usr/bin/env bash
# Check SQLite format snapshots and SemVer format-freeze rules.
#
# Usage:
#   bash scripts/check-formats.sh            # CI mode (PRs / pushes to main)
#   bash scripts/check-formats.sh 1.1.0-rc.1 # Release mode (tag v1.1.0-rc.1)
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - "${1:-}" <<'PY'
import os
import re
import subprocess
import sys
from pathlib import Path

STORE_MOD = "crates/flowie-core/src/store/mod.rs"
FORMATS_DIR = "crates/flowie-core/src/store/formats"
SEMVER_RE = re.compile(r"^v?([0-9]+)\.([0-9]+)\.([0-9]+)(?:-rc\.([0-9]+))?$")
SNAP_RE = re.compile(r"^v([0-9]+)\.sql$")


def fail(msg: str) -> None:
    print(f"check-formats: {msg}", file=sys.stderr)
    sys.exit(1)


def git(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", *args],
        capture_output=True,
        check=check,
    )


def parse_semver(text: str):
    m = SEMVER_RE.match(text.strip())
    if not m:
        fail(f"invalid release version '{text}' (expected X.Y.Z or X.Y.Z-rc.N)")
    major, minor, patch = int(m.group(1)), int(m.group(2)), int(m.group(3))
    rc = int(m.group(4)) if m.group(4) is not None else None
    return (major, minor, patch, rc)


def semver_key(v):
    major, minor, patch, rc = v
    # Final releases sort after RCs of the same major.minor.patch.
    return (major, minor, patch, 1 if rc is None else 0, rc or 0)


def fmt_version(v) -> str:
    major, minor, patch, rc = v
    base = f"{major}.{minor}.{patch}"
    return f"{base}-rc.{rc}" if rc is not None else base


def strip_rust_comments(src: str) -> str:
    src = re.sub(r"/\*.*?\*/", "", src, flags=re.S)
    src = re.sub(r"//[^\n]*", "", src)
    return src


def count_top_level_items(body: str) -> int:
    body = strip_rust_comments(body).strip()
    if not body:
        return 0
    depth = 0
    items = 0
    has_token = False
    for ch in body:
        if ch in "{([":
            depth += 1
            has_token = True
        elif ch in "})]":
            depth -= 1
            has_token = True
        elif ch == "," and depth == 0:
            if has_token:
                items += 1
                has_token = False
        elif not ch.isspace():
            has_token = True
    if has_token:
        items += 1
    return items


def parse_store_mod(src: str, origin: str):
    m_base = re.search(r"const\s+BASELINE\s*:\s*i64\s*=\s*([0-9]+)\s*;", src)
    if not m_base:
        fail(f"could not find BASELINE in {origin}")
    baseline = int(m_base.group(1))

    m_mig = re.search(r"const\s+MIGRATIONS\s*:\s*&\[Migration\]\s*=\s*&\[(.*?)\]\s*;", src, flags=re.S)
    if not m_mig:
        fail(f"could not find MIGRATIONS in {origin}")
    migrations = count_top_level_items(m_mig.group(1))
    return baseline, baseline + migrations


def parse_cargo_marketing() -> tuple[int, int, int]:
    text = Path("Cargo.toml").read_text(encoding="utf-8")
    m_sec = re.search(r"^\[workspace\.package\]\s*$(.*?)(?:^\[|\Z)", text, flags=re.M | re.S)
    if not m_sec:
        fail("could not find [workspace.package] in Cargo.toml")
    m_ver = re.search(r'^version\s*=\s*"([0-9]+)\.([0-9]+)\.([0-9]+)"\s*$', m_sec.group(1), flags=re.M)
    if not m_ver:
        fail("could not find version in [workspace.package] of Cargo.toml")
    return (int(m_ver.group(1)), int(m_ver.group(2)), int(m_ver.group(3)))


def released_tags():
    out = git("tag", "-l", "v*").stdout.decode("utf-8")
    tags = []
    for line in out.splitlines():
        name = line.strip()
        if not name or not SEMVER_RE.match(name):
            continue
        v = parse_semver(name)
        # Format rules begin at 1.0.0 (BASELINE = 5); earlier 0.x and 1.0.0-rc.* predated it.
        if v[0] < 1 or (v[:3] == (1, 0, 0) and v[3] is not None):
            continue
        tags.append((semver_key(v), name, v))
    tags.sort(key=lambda item: item[0])
    return tags


def tag_snapshot_files(tag: str) -> dict[int, str]:
    res = git("ls-tree", "--name-only", tag, f"{FORMATS_DIR}/", check=False)
    if res.returncode != 0:
        return {}
    snaps = {}
    for line in res.stdout.decode("utf-8").splitlines():
        rel = line.strip()
        if not rel:
            continue
        name = os.path.basename(rel)
        m = SNAP_RE.match(name)
        if m:
            snaps[int(m.group(1))] = rel
    return snaps


def format_at_tag(tag: str) -> int:
    snaps = tag_snapshot_files(tag)
    if snaps:
        return max(snaps)
    res = git("show", f"{tag}:{STORE_MOD}", check=False)
    if res.returncode != 0:
        fail(f"could not read {STORE_MOD} at tag {tag}")
    _, fmt = parse_store_mod(res.stdout.decode("utf-8"), f"{tag}:{STORE_MOD}")
    return fmt


# 1. Check working-tree snapshots and consistency with store/mod.rs.
mod_src = Path(STORE_MOD).read_text(encoding="utf-8")
baseline, current_fmt = parse_store_mod(mod_src, STORE_MOD)

formats_path = Path(FORMATS_DIR)
if not formats_path.is_dir():
    fail(f"missing snapshot directory {FORMATS_DIR}")

working_snaps: dict[int, Path] = {}
for entry in sorted(formats_path.iterdir()):
    m = SNAP_RE.match(entry.name)
    if not m or not entry.is_file():
        fail(f"unexpected entry in {FORMATS_DIR}: {entry.name}")
    working_snaps[int(m.group(1))] = entry

expected_versions = list(range(baseline, current_fmt + 1))
if sorted(working_snaps) != expected_versions:
    fail(
        f"{FORMATS_DIR} must contain v{baseline}.sql..v{current_fmt}.sql "
        f"(found: {', '.join(f'v{n}.sql' for n in sorted(working_snaps)) or 'none'})"
    )

expected_include = f'include_str!("formats/v{current_fmt}.sql")'
if expected_include not in mod_src:
    fail(f"SCHEMA in {STORE_MOD} must use {expected_include}")

# 2. Check released snapshots have never been modified or removed.
target_arg = sys.argv[1].strip()
target_ver = parse_semver(target_arg) if target_arg else None
all_tags = released_tags()
prior_tags = (
    [t for t in all_tags if t[0] < semver_key(target_ver)]
    if target_ver is not None
    else all_tags
)

for _, tag, _ in prior_tags:
    for ver, rel in sorted(tag_snapshot_files(tag).items()):
        if ver not in working_snaps:
            fail(f"released snapshot {rel} (from {tag}) was deleted")
        old_bytes = git("show", f"{tag}:{rel}").stdout
        cur_bytes = working_snaps[ver].read_bytes()
        if cur_bytes != old_bytes:
            fail(f"released snapshot {rel} (from {tag}) was modified; never edit published formats")

# 3. Check SemVer format progression and RC freeze rules against the previous tag.
if not prior_tags:
    print(f"formats ok: format {current_fmt} (no prior 1.0+ tag)")
    sys.exit(0)

_, prev_tag, prev_ver = prior_tags[-1]
prev_fmt = format_at_tag(prev_tag)

if current_fmt < prev_fmt:
    fail(f"database format decreased from {prev_fmt} ({prev_tag}) to {current_fmt}")
if current_fmt - prev_fmt > 1:
    fail(
        f"database format jumped from {prev_fmt} ({prev_tag}) to {current_fmt}; "
        f"at most one new format is allowed per minor version"
    )

if target_ver is not None:
    tm, tmin, tpatch, trc = target_ver
    pm, pmin, ppatch, prc = prev_ver
    if tpatch > 0 and current_fmt != prev_fmt:
        fail(
            f"patch release {fmt_version(target_ver)} must keep database format {prev_fmt} "
            f"(from {prev_tag}), found {current_fmt}; bump the minor version instead"
        )
    if trc is not None and trc >= 2 and current_fmt != prev_fmt:
        fail(
            f"release candidate {fmt_version(target_ver)} cannot change database format after rc.1 "
            f"({prev_tag} is format {prev_fmt}, current is {current_fmt})"
        )
    if trc is None and (pm, pmin, ppatch) == (tm, tmin, tpatch) and prc is not None and current_fmt != prev_fmt:
        fail(
            f"final release {fmt_version(target_ver)} must keep the database format frozen by "
            f"{prev_tag} ({prev_fmt}), found {current_fmt}"
        )
else:
    cm, cmin, cpatch = parse_cargo_marketing()
    pm, pmin, ppatch, prc = prev_ver
    if prc is not None and (cm, cmin, cpatch) == (pm, pmin, ppatch) and current_fmt != prev_fmt:
        fail(
            f"database format is frozen for {cm}.{cmin}.{cpatch} after {prev_tag} "
            f"(format {prev_fmt}), found {current_fmt}"
        )
    if cpatch > 0 and (cm, cmin, cpatch) > (pm, pmin, ppatch) and current_fmt != prev_fmt:
        fail(
            f"Cargo.toml patch version {cm}.{cmin}.{cpatch} must keep database format {prev_fmt} "
            f"(from {prev_tag}), found {current_fmt}; bump the minor version instead"
        )

mode = f"release {fmt_version(target_ver)}" if target_ver is not None else "CI"
print(f"formats ok ({mode}): format {current_fmt} (previous {prev_tag} was {prev_fmt})")
PY
