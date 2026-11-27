#!/usr/bin/env bash
# Check SQLite format snapshots, SemVer format-freeze rules, and that store
# writes keep columns they don't know about.
#
# Usage:
#   bash scripts/check-formats.sh            # CI mode (PRs / pushes to main)
#   bash scripts/check-formats.sh 1.1.0-rc.1 # Release mode (tag v1.1.0-rc.1)
#
# The snapshot files are the source of truth for formats here: the newest
# formats/v<N>.sql is the current format. `cargo test -p flowie-core` checks that
# they match BASELINE, MIGRATIONS and SCHEMA in store/mod.rs, so this script
# never reads Rust source.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 - "${1:-}" <<'PY'
import re
import subprocess
import sys
import tomllib
from pathlib import Path

FORMATS_DIR = "crates/flowie-core/src/store/formats"
# Frozen at 1.0 (docs/versioning.md); the oldest snapshot must be this one.
BASELINE = 5
# Releases from 1.0 on that predate formats/ all shipped the baseline format.
FORMAT_BEFORE_SNAPSHOTS = BASELINE

# Code under these folders writes the outline database. `crates/flowie-server`
# keeps its own database and isn't checked.
STORE_WRITERS = [
    "crates/flowie-core/src",
    "crates/flowie-doc/src",
    "crates/flowie-sync/src",
    "crates/flowie-ffi/src",
    "crates/flowie-wasm/src",
    "crates/flowie-app/src",
]
# These delete the old row and insert a new one, wiping columns from a newer
# format that this build doesn't know: additive formats need
# `INSERT … ON CONFLICT … DO UPDATE` instead. SQL here is written in capitals,
# so prose like "insert or replace a node" doesn't match.
ROW_REPLACING = re.compile(r"\bINSERT\s+OR\s+REPLACE\b|\bREPLACE\s+INTO\b")

SEMVER_RE = re.compile(r"^v?([0-9]+)\.([0-9]+)\.([0-9]+)(?:-rc\.([0-9]+))?$")
SNAPSHOT_RE = re.compile(r"^v([0-9]+)\.sql$")


class Version:
    def __init__(self, major: int, minor: int, patch: int, rc):
        self.major = major
        self.minor = minor
        self.patch = patch
        self.rc = rc

    @staticmethod
    def parse(text: str) -> "Version":
        m = SEMVER_RE.match(text.strip())
        if not m:
            fail(f"invalid release version '{text}' (expected X.Y.Z or X.Y.Z-rc.N)")
        rc = int(m.group(4)) if m.group(4) is not None else None
        return Version(int(m.group(1)), int(m.group(2)), int(m.group(3)), rc)

    def base(self) -> tuple:
        return (self.major, self.minor, self.patch)

    def sort_key(self) -> tuple:
        # A final release sorts after the release candidates of the same version.
        if self.rc is None:
            return (self.major, self.minor, self.patch, 1, 0)
        return (self.major, self.minor, self.patch, 0, self.rc)

    def __str__(self) -> str:
        base = f"{self.major}.{self.minor}.{self.patch}"
        if self.rc is None:
            return base
        return f"{base}-rc.{self.rc}"


class Tag:
    def __init__(self, name: str, version: Version):
        self.name = name
        self.version = version


def fail(msg: str) -> None:
    print(f"check-formats: {msg}", file=sys.stderr)
    sys.exit(1)


def git(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], capture_output=True, check=check)


def snapshot_version(file_name: str):
    m = SNAPSHOT_RE.match(file_name)
    if not m:
        return None
    return int(m.group(1))


def working_snapshots() -> dict:
    """Format number -> path of each snapshot in the working tree."""
    formats_dir = Path(FORMATS_DIR)
    if not formats_dir.is_dir():
        fail(f"missing snapshot directory {FORMATS_DIR}")
    snapshots = {}
    for entry in sorted(formats_dir.iterdir()):
        version = snapshot_version(entry.name)
        if version is None or not entry.is_file():
            fail(f"unexpected entry in {FORMATS_DIR}: {entry.name}")
        snapshots[version] = entry
    return snapshots


def check_contiguous(snapshots: dict) -> int:
    """The snapshots run from BASELINE without gaps; returns the current format."""
    if not snapshots:
        fail(f"{FORMATS_DIR} holds no snapshots")
    current = max(snapshots)
    for version in range(BASELINE, current + 1):
        if version not in snapshots:
            fail(f"{FORMATS_DIR} must contain v{BASELINE}.sql..v{current}.sql (v{version}.sql is missing)")
    for version in snapshots:
        if version < BASELINE:
            fail(f"{FORMATS_DIR}/v{version}.sql is older than BASELINE {BASELINE}")
    return current


def check_no_row_replacing_writes() -> None:
    found = []
    for folder in STORE_WRITERS:
        root = Path(folder)
        if not root.is_dir():
            continue
        for path in sorted(root.rglob("*")):
            if path.suffix not in (".rs", ".sql") or not path.is_file():
                continue
            lines = path.read_text(encoding="utf-8").splitlines()
            for number, line in enumerate(lines, start=1):
                # Comments may name these statements, as this check's own docs do.
                if line.lstrip().startswith(("//", "--")):
                    continue
                if ROW_REPLACING.search(line):
                    found.append(f"  {path}:{number}: {line.strip()}")
    if found:
        fail(
            "these writes replace whole rows and would wipe columns a newer format added; "
            "use INSERT … ON CONFLICT … DO UPDATE SET <known columns>:\n" + "\n".join(found)
        )


def released_tags() -> list:
    """Release tags from 1.0.0 on, oldest first. Format rules start at 1.0.0;
    0.x and the 1.0.0 release candidates predate them."""
    tags = []
    out = git("tag", "-l", "v*").stdout.decode("utf-8")
    for line in out.splitlines():
        name = line.strip()
        if not SEMVER_RE.match(name):
            continue
        version = Version.parse(name)
        if version.major < 1:
            continue
        if version.base() == (1, 0, 0) and version.rc is not None:
            continue
        tags.append(Tag(name, version))
    tags.sort(key=lambda tag: tag.version.sort_key())
    return tags


def tag_snapshots(tag: str) -> dict:
    """Format number -> repository path of each snapshot at `tag`."""
    result = git("ls-tree", "--name-only", tag, f"{FORMATS_DIR}/", check=False)
    snapshots = {}
    if result.returncode != 0:
        return snapshots
    for line in result.stdout.decode("utf-8").splitlines():
        path = line.strip()
        version = snapshot_version(Path(path).name)
        if version is not None:
            snapshots[version] = path
    return snapshots


def format_at(tag: str) -> int:
    snapshots = tag_snapshots(tag)
    if not snapshots:
        return FORMAT_BEFORE_SNAPSHOTS
    return max(snapshots)


def check_released_unchanged(tags: list, snapshots: dict) -> None:
    for tag in tags:
        for version, path in sorted(tag_snapshots(tag.name).items()):
            if version not in snapshots:
                fail(f"released snapshot {path} (from {tag.name}) was deleted")
            released = git("show", f"{tag.name}:{path}").stdout
            if snapshots[version].read_bytes() != released:
                fail(f"released snapshot {path} (from {tag.name}) was modified; never edit published formats")


def cargo_version() -> Version:
    with open("Cargo.toml", "rb") as f:
        cargo = tomllib.load(f)
    try:
        text = cargo["workspace"]["package"]["version"]
    except KeyError:
        fail("could not find version in [workspace.package] of Cargo.toml")
    version = Version.parse(text)
    if version.rc is not None:
        fail(f"Cargo.toml version {text} should not carry -rc.N; release candidates are tags")
    return version


def check_release(target: Version, previous: Tag, current: int, previous_format: int) -> None:
    changed = current != previous_format
    if target.patch > 0 and changed:
        fail(
            f"patch release {target} must keep database format {previous_format} "
            f"(from {previous.name}), found {current}; bump the minor version instead"
        )
    if target.rc is not None and target.rc >= 2 and changed:
        fail(
            f"release candidate {target} cannot change database format after rc.1 "
            f"({previous.name} is format {previous_format}, current is {current})"
        )
    finishes_candidates = previous.version.rc is not None and previous.version.base() == target.base()
    if target.rc is None and finishes_candidates and changed:
        fail(
            f"final release {target} must keep the database format frozen by "
            f"{previous.name} ({previous_format}), found {current}"
        )


def check_main(cargo: Version, previous: Tag, current: int, previous_format: int) -> None:
    changed = current != previous_format
    in_candidates = previous.version.rc is not None and previous.version.base() == cargo.base()
    if in_candidates and changed:
        fail(
            f"database format is frozen for {cargo} after {previous.name} "
            f"(format {previous_format}), found {current}"
        )
    if cargo.patch > 0 and cargo.base() > previous.version.base() and changed:
        fail(
            f"Cargo.toml patch version {cargo} must keep database format {previous_format} "
            f"(from {previous.name}), found {current}; bump the minor version instead"
        )


def main() -> None:
    target_arg = sys.argv[1].strip()
    target = Version.parse(target_arg) if target_arg else None

    # 1. The snapshots in the working tree.
    snapshots = working_snapshots()
    current = check_contiguous(snapshots)

    # 2. Writes keep columns from newer formats.
    check_no_row_replacing_writes()

    # 3. Released snapshots are never modified or removed.
    prior = []
    for tag in released_tags():
        if target is None or tag.version.sort_key() < target.sort_key():
            prior.append(tag)
    check_released_unchanged(prior, snapshots)

    # 4. SemVer format progression and the release-candidate freeze.
    if not prior:
        print(f"formats ok: format {current} (no prior 1.0+ tag)")
        return
    previous = prior[-1]
    previous_format = format_at(previous.name)
    if current < previous_format:
        fail(f"database format decreased from {previous_format} ({previous.name}) to {current}")
    if current - previous_format > 1:
        fail(
            f"database format jumped from {previous_format} ({previous.name}) to {current}; "
            f"at most one new format is allowed per minor version"
        )
    if target is not None:
        check_release(target, previous, current, previous_format)
        mode = f"release {target}"
    else:
        check_main(cargo_version(), previous, current, previous_format)
        mode = "CI"
    print(f"formats ok ({mode}): format {current} (previous {previous.name} was {previous_format})")


main()
PY
