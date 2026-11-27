# Versioning and compatibility

Flowie follows [Semantic Versioning](https://semver.org/) (`MAJOR.MINOR.PATCH`, with `-rc.N` release candidates) for releases and keeps explicit version numbers for every persisted or network-facing format.

## Version inventory

| Surface | Format | Current | Defined in | Compatibility rule |
|---|---|---|---|---|
| **App release** | SemVer (`X.Y.Z`, `X.Y.Z-rc.N`) | `1.0.1` (next: `1.1.0`) | `Cargo.toml` (`[workspace.package]`), Git tags `v*` | Patch: fixes only. Minor: features, at most one DB format step. Major: breaking changes. |
| **SQLite database** | `PRAGMA user_version` + `meta['min_reader']` | `FORMAT = 6`, `MIN_READER = 6` (`BASELINE = 5`) | `crates/flowie-core/src/store/mod.rs`, `formats/v*.sql` | In-place upgrades from `BASELINE` with pre-upgrade backup; older builds may open newer DBs when `min_reader <= FORMAT`. |
| **Share package (`.flowie`)** | `manifest.json` `format` + integer `version` | `flowie.package`, `VERSION = 1` | `crates/flowie-core/src/package/mod.rs` | Readers refuse a newer major version; unknown optional fields are ignored. |
| **Sync protocol** | JSON `Envelope.v` + HTTP path prefix | `VERSION = 1`, `/v1/…` | `crates/flowie-sync/src/wire.rs`, `crates/flowie-server/src/lib.rs` | Unknown fields and record kinds are preserved verbatim; breaking wire changes bump `VERSION` or add `/v2/…`. |
| **MCP interface** | MCP protocol date + tool schemas | `2025-06-18` | `crates/flowie-app/src/agent/mcp.rs`, [docs/mcp.md](mcp.md) | Additive changes in minor releases; tools are marked **Deprecated** before removal in a later minor release. |

## SemVer and format changes

- **Patch releases (`X.Y.Z`, `Z > 0`)** carry bug fixes and polish only. They must **not** change the database format, share-package version, or sync wire format, and must not break MCP tools.
- **Minor releases (`X.Y.0`)** add features and may advance the database format by **at most one** step (`FORMAT += 1`).
  - **Additive migrations** (`Migration { additive: true, .. }`: new tables, indexes, or columns with defaults) do not raise `meta['min_reader']`. Because every SQL write updates only known columns (`ON CONFLICT … DO UPDATE`), older builds whose `FORMAT >= min_reader` can continue reading and writing the database without wiping newer columns — making rollbacks and mixed-version devices safe.
  - **Non-additive migrations** (`additive: false`) advance `min_reader` to the new `FORMAT`, so older builds refuse to open the database (`OpenError::TooNew`) and leave it untouched.
  - Because `main` currently uses format 6 while `1.0.1` shipped format 5, the next release must be `1.1.0` (not `1.0.2`).
- **Major releases (`X.0.0`)** are reserved for breaking changes that drop prior compatibility promises or bump a major protocol/package version.
- **Release candidates (`X.Y.Z-rc.N`)** freeze the database format at **`rc.1`**:
  - `rc.2` and later candidates of the same version, as well as the final `X.Y.Z` release after candidates, must keep the exact same database format as `rc.1`.
  - Every released SQL snapshot in `crates/flowie-core/src/store/formats/v<N>.sql` is immutable once tagged.
  - Both CI (`.github/workflows/ci.yml`) and releases (`.github/workflows/release.yml`) enforce these rules via `scripts/check-formats.sh`, and `cargo test -p flowie-core` checks that upgrading every `formats/v*.sql` snapshot produces the exact schema of a fresh database.
- **Changelogs**: each release candidate lists only what changed since the previous candidate; the final release summarizes all changes across its candidates.

## Bootstrap phase vs. stable phase

- **Bootstrap phase (pre-1.0: `0.x` and `1.0.0-rc.*`)**: database formats below `BASELINE = 5` were iterative and have no automated migration path. Opening a pre-1.0 database in Flowie 1.0+ folds its WAL, moves it (and its `.images` folder) aside as `<db>.old-<timestamp>`, and creates a fresh outline.
- **Stable phase (`1.0.0` onward, `BASELINE = 5`)**:
  - `BASELINE` is frozen at 5; released entries in `MIGRATIONS` and released `formats/v*.sql` snapshots are never edited or reordered.
  - Before running migrations, Flowie writes a consistent `VACUUM INTO` backup beside the database (`<db>.backup-v<format>-<timestamp>`, keeping the three newest) and applies all pending migrations, `meta['min_reader']`, and `PRAGMA user_version` in a single transaction. If either backup or migration fails, the database is left unchanged.

## Experimental and preview components

Components explicitly marked **preview** or **beta** — currently Windows and Linux desktop builds, self-hosted sync (`flowie-server` / Settings → Sync), the browser preview (`flowie-wasm` / `web/`), and the native Apple FFI/UI layer (`flowie-ffi` / `apple/FlowieKit`) — may adjust UX, configuration, or internal wire details between minor releases, with any manual steps called out in `CHANGELOG.md` and `CHANGELOG.zh-CN.md`.

Experimental components still respect the stable local-data guarantees: they never bypass the database backup/migration rules, sync writes a pre-sync copy (`<db>.before-sync-<timestamp>`) before first sync, and unknown synced fields are preserved. Once a component leaves preview/beta, it follows the full stable compatibility rules above.
