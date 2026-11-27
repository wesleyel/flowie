# Workspace layout

Flowie is a Rust Cargo workspace paired with a Swift package (`apple/`) for iOS and macOS CloudKit,
a TypeScript browser client (`web/`) over wasm, and supporting build/release directories.

## Crates

| Crate | What it holds |
|---|---|
| `crates/flowie-core` | Model and persistence, **no UI**: `tree/` (`NodeId` UUIDs, `Node`, `Tree`, edit ops), `rank.rs` (fractional-index sibling keys: base-62 strings compared as bytes, never renumbered), `store/` (SQLite schema in `store/mod.rs`, `formats/v*.sql` snapshots, background `Writer` batching changes ~200 ms, Trash, imports, image metadata), `history.rs` (undo), `flatten.rs` (tree → visible `Row`/`DisplayRow`), `markup.rs` (inline formatting), `search.rs`, `journal.rs`, `hlc.rs`, `short_id.rs`, `io/` (text/Markdown/OPML), `package/` (`.flowie` share packages, three-way merge), `images.rs` (sha256 content-addressed `ImageDir`), `imports.rs`. Default features `store` (store, package, `ImageDir`) and `writer-thread` (`Writer`); without them it compiles for `wasm32-unknown-unknown`. |
| `crates/flowie-doc` | Document layer, **no GPUI**: `Workspace` owns the open outline's `tree`, `history`, `writer`, `imports`, `blobs`, `image_dir`, `prefs`, and `sync` (`SyncState`), opens the database, and runs the commit/undo/tracking path (`Outcome`) every edit takes; also holds `SyncDriver`, `NativeSync`, `capture`, `find`, `keys`, and `utf16`. |
| `crates/flowie-sync` | Per-node sync, **no IO, no GPUI**, builds for wasm: `wire` (JSON `Envelope`/`NodeRecord`/`PrefsRecord`, golden vectors in `testdata/`), `merge` (field-by-field LWW by `flowie_core::hlc`), `engine` (`Engine`, `RecordStore`, `Backend`, `sync_once`), `memory`/`store` (in-memory backend and store for tests), and `http` (`HttpBackend`, behind the `http` feature). |
| `crates/flowie-server` | Self-hosted sync backend (binary `flowie-server`, see [sync.md](sync.md)): axum + SQLite, stores envelopes without reading them, conditional saves (`seq`), long-polling change feed, sha256-named blob files, device tokens (`token add` / `list` / `revoke`); optionally serves `web/` with `--web`. |
| `crates/flowie-ffi` | Native bindings through UniFFI for Apple apps: an open outline, a page's rows, edits, journal capture, search, and self-hosted sync, all routed through `flowie-doc`. `scripts/ffi-swift-smoke.sh` builds Swift bindings and runs a smoke test. |
| `crates/flowie-cloudkit` | iCloud sync for macOS: `build.rs` compiles `apple/FlowieKit/Sources/FlowieCloudKit` and `swift/Bridge.swift` (C API) with `swiftc` into a static library linked into the GPUI binary; `CloudDriver` drives `CKSyncEngine` through `flowie-doc`'s `NativeSync`. Elsewhere, or with `FLOWIE_CLOUDKIT=0`, it builds without Swift and reports iCloud unavailable; only builds signed with Flowie's iCloud entitlements (Mac App Store / TestFlight) can sync. |
| `crates/flowie-wasm` | Browser preview core: `flowie-doc` and `flowie-sync` compiled to wasm with SQLite (`sqlite-wasm-rs` / OPFS). `scripts/wasm-check.sh` runs its tests under Node. |
| `crates/flowie-math` | KaTeX-compatible math layout via RaTeX, plus a GPUI painting element (`element.rs`). |
| `crates/flowie-app` | Desktop GPUI application (binary `flowie`, plus `flowie mcp`). |
| `third_party/gpui-pre-macos` | Excluded workspace crate patched via `[patch.crates-io]`: GPUI's macOS backend with private AppKit selectors removed for App Review (`FLOWIE-PATCH.md`). Keep the patch when bumping `gpui-kit`. |

## Desktop app (`crates/flowie-app/src`)

- `outline/`: the main editor. `Outline` (in `outline/mod.rs`) owns a `Workspace` (`self.ws`), zoom, rows, and edit state. Split by concern: `editing`, `structure` (indent/move/drag), `blocks`, `clipboard`, `display`, `documents`, `heights`, `history`, `journal`, `navigation`, `rows`/`render`, `selection`, `sync`, `window`.
- `text/`: rich-text editor and text/inline layout (`editor`, `fields`, `inline`, `node`, `rich`, `selectable`).
- `ui/`: Flowie's own controls, icons, scrollbar, animations, and theme, drawn in the macOS style (no component library).
- `overlays/`: command palette (`palette`), `popover`, `context_menu`, `format_menu`, `calendar`.
- `pages/`: full-page views (`trash`, `whats_new`).
- `settings/`: one file per tab (`general`, `appearance`, `shortcuts`, `sync`, `agent`, `about`).
- `share/`: `.flowie` package `export`, `import`, and `unread` indicators.
- `agent/`: MCP stdio server (`mcp.rs`), local IPC socket (`ipc.rs`), and tool handlers on `Outline` (`tools.rs`). See [mcp.md](mcp.md).
- `app/`: platform glue (`platform`, `menu_bar`, `keymap`, `i18n`, `tutorial`, `tray`, `file_dialog`, `window_frame`, `window_place`).
- `dev/`: screenshot tooling (`shots.rs`), behind the `screenshots` feature (`scripts/screenshots.py`).
- `image.rs`, `main.rs`: image decoding/caching and entry point (`flowie` GUI and `flowie mcp`).

### Data flow

An edit runs in `Outline`, mutates `ws.tree`, and commits through `Workspace` (records `History`,
queues the write on `Writer`, and marks sync state). `flatten` then rebuilds the visible rows and
GPUI re-renders.

## Non-Rust directories

- `apple/`:
  - `apple/FlowieKit`: Swift package (`Package.swift`) wrapping generated `FlowieFFI` bindings:
    `FlowieCloudKit` (`CKSyncEngine` over a host protocol, shared with the Mac via
    `flowie-cloudkit`) and `FlowieUI` (the iOS SwiftUI/UIKit layer, which also builds on macOS).
    `scripts/apple-package-check.sh` builds and runs `cloudkit-check` and `ui-check` with the
    plain Swift toolchain.
  - `apple/FlowieApp`: iOS app (Tuist `Project.swift`, plus `Control` Control Center extension)
    over `FlowieUI` and `build/ios/FlowieFFI.xcframework` (`scripts/build-ios-ffi.sh`,
    `scripts/package-ios.sh`; see [building.md](building.md)).
- `web/`: browser preview TypeScript app (`web/src`, worker + `fetch` sync, `sw.js`) built against
  `web/pkg` (`flowie-wasm`) by `scripts/build-web.sh` and served by `flowie-server serve --web web`.
- `scripts/`: build, packaging, CI verification, icon, screenshot, and setup scripts (including
  `setup-wiki.sh`, `check-formats.sh`, `release-notes.sh`).
- `appstore/`: Mac App Store release config: fastlane metadata, screenshots, privacy manifest,
  and sandbox build files.
- `website/`: Flowie's Jekyll GitHub Pages site (home, downloads, privacy policy, terms, `llms.txt`).
- `docs/`: user and developer documentation (`building.md`, `usage.md`, `mcp.md`, `sync.md`,
  `versioning.md`, `translations.md`, `wiki.md`, `workspace.md`).
- `wiki/`: local clone of the roadmap wiki (`wesleyel/flowie.wiki.git`), excluded from main git
  via `.git/info/exclude` and pointed to by `.to` (see [wiki.md](wiki.md)).
