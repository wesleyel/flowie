# Flowie — agent guide

Flowie is a keyboard-first outliner (bullets, zoom, journal, math, images, MCP agent access), written in
Rust on GPUI (Zed's UI framework, taken through `gpui-kit`'s `gpui-pre` snapshots). It ships for macOS
(App Store, GitHub releases, Homebrew cask), with Windows and Linux previews (Linux tar.gz and AppImage,
`scripts/bundle-linux.sh`). It's a solo project; release candidates use `X.Y.Z-rc.N` tags.

## Workspace

| Crate | What it holds |
|---|---|
| `crates/flowie-core` | Model and persistence, **no UI**: `tree/` (`NodeId` UUIDs, `Node`, `Tree`, edit ops), `rank.rs` (fractional-index sibling keys: base-62 strings compared as bytes, never renumbered), `store/` (SQLite schema in `store/mod.rs`, background `Writer` batching changes about every 200 ms, Trash, imports, image metadata), `history.rs` (undo), `flatten.rs` (tree → visible `Row`/`DisplayRow`), `markup.rs` (inline formatting), `search.rs`, `io/` (text/Markdown/OPML), `package/` (share packages, three-way merge), `images.rs` (sha256 content-addressed files). Default features `store` (store, package, `ImageDir`) and `writer-thread` (`Writer`); without them it builds for wasm |
| `crates/flowie-doc` | The document layer, **no GPUI**: `Workspace` owns the open outline's `tree`, `history` and `writer` (plus imports and image metadata), opens the database and runs the commit/undo path every edit takes |
| `crates/flowie-sync` | Per-node sync, **no IO, no GPUI**, builds for wasm: `wire` (the JSON `Envelope`/`NodeRecord` devices exchange, golden vectors in `testdata/`), `merge` (field-by-field, last writer wins by `flowie_core::hlc` clock), `engine` (the state machine, `RecordStore`, `sync_once` over a `Backend`), `memory`/`store` (in-memory backend and store for tests) |
| `crates/flowie-server` | The self-hosted sync backend (binary `flowie-server`): axum + SQLite, stores records without reading them, conditional saves, a change feed with long polling, sha256-named image files, device tokens (`token add`). The client side is `flowie-sync`'s `http` feature |
| `crates/flowie-ffi` | Flowie for native apps through UniFFI (iOS, the CloudKit bridge): an open outline, a page's rows, the basic edits and sync, all through `flowie-doc`. `scripts/ffi-swift-smoke.sh` builds its Swift bindings and runs a Swift program over them. `apple/FlowieKit` is the Swift package over it: `FlowieCloudKit` (CKSyncEngine) and `FlowieUI` (the iOS app's SwiftUI layer, which builds for macOS too); `scripts/apple-package-check.sh` builds and checks it with the plain Swift toolchain |
| `crates/flowie-wasm` | Flowie in the browser (preview): `flowie-doc` and the sync engine in wasm with SQLite inside (`sqlite-wasm-rs`); `web/` is the page, its worker and fetch-based sync in TypeScript (`web/src`), built by `scripts/build-web.sh` and served by `flowie-server serve --web web`. `scripts/wasm-check.sh` runs its tests in wasm under Node |
| `crates/flowie-math` | KaTeX-compatible math via RaTeX, plus a GPUI element |
| `crates/flowie-app` | The GPUI app (binary `flowie`) |
| `third_party/gpui-pre-macos` | GPUI's macOS backend with private AppKit calls removed for App Review (`FLOWIE-PATCH.md`). Keep the patch when bumping gpui-kit. |

`crates/flowie-app/src`:
- `outline/`: the main editor. `Outline` (in `outline/mod.rs`) owns a `Workspace` (`self.ws`), zoom, rows and the edit state. It's split by concern: `editing`, `structure` (indent/move/drag), `blocks`, `selection`, `navigation`, `history`, `journal`, `rows`/`render`.
- `text/`: the text editor and rich-text layout. `ui/`: Flowie's own controls and theme, drawn in the macOS style (no component library).
- `overlays/`: palette, popovers, menus. `pages/`: full-page views (Trash, What's New). `settings/`: one file per tab. `share/`: package dialogs.
- `agent/`: MCP. `flowie mcp` is a stdio server that talks over a local socket (`ipc.rs`) to the running app, which runs the tools (`tools.rs`) on the `Outline`. Its tool docs are in `docs/mcp.md`.
- `app/`: platform glue (`platform.rs`, `menu_bar.rs`, `keymap.rs`, `i18n.rs`, `tutorial.rs`, window placement).
- `dev/`: screenshot tooling, behind the `screenshots` feature (`scripts/screenshots.py`).

Data flow: an edit runs in `Outline`, which changes `ws.tree`, then commits through the `Workspace` (records
`History`, queues the change on the `Writer`). Then `flatten` rebuilds the visible rows and GPUI re-renders.

## Commands

```bash
cargo run -p flowie-app                         # dev run; FLOWIE_DB=/tmp/x.sqlite to use a scratch outline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked
cargo test --workspace --locked
cargo check -p flowie-core -p flowie-sync --no-default-features --target wasm32-unknown-unknown   # the model and sync without store/writer-thread
scripts/bundle-macos.sh target/release/flowie Flowie flowie.spokey.cn   # local .app
```

CI (`.github/workflows/ci.yml`) runs fmt, both changelog checks, `scripts/check-formats.sh`, cargo-deny, the wasm check above,
and a clippy/test matrix on pull requests and pushes to `main`. Run Desktop Packages manually to get
temporary test artifacts; push a `vX.Y.Z` or `vX.Y.Z-rc.N` tag to publish all three platforms from the
same pipeline after package smoke checks pass. Homebrew tracks the newest release, RCs included. The Website
workflow checks Pages on pull requests and deploys it from `main`. Other env vars: `FLOWIE_THEME`,
`FLOWIE_WINDOW_SIZE`, `FLOWIE_SHOT_*` (screenshots).

## Rules

- **Commit on `main`**, without feature branches. Push only when asked. One change is one commit.
- **Before every commit**: for user-visible changes, add an `## [Unreleased]` entry in **both** changelogs, in the style of the existing entries: `CHANGELOG.md` (Added/Changed/Fixed) and the matching Chinese entry in `CHANGELOG.zh-CN.md` (新增/变更/修复), which What's New shows in the Chinese UI. Then run fmt and clippy as above.
- **Maintain database compatibility**: follow [docs/versioning.md](docs/versioning.md) — keep `BASELINE = 5`, add a new `crates/flowie-core/src/store/formats/v<N>.sql` snapshot and append a `Migration` (marking `additive` accurately; never edit released snapshots or earlier steps), use column-preserving upserts (`ON CONFLICT … DO UPDATE`), and pass `scripts/check-formats.sh`.
- **UI text** goes through `t!(…)`, with keys in `crates/flowie-app/locales/en.json` and `zh-CN.json`. `cargo test -p flowie-app i18n` checks both files. New features also get a lesson in the tutorial (`crates/flowie-app/tutorial/<locale>.md`).
- **Docs scope**: the README, website, `llms.txt` and the App Store text cover core features only. Never add UX polish there. A big change in one area goes only in that area's doc (e.g. `docs/mcp.md`). Do fix wording that a change has made wrong.
- **Feature requests**: when the user describes a feature, clarify it and record it in the backlog (the wiki, see below). Don't implement it unless they ask you to build it.
- Keep a feature's state, actions and rendering together. New editing logic should not depend on GPUI where that can be avoided (see the wiki's `iOS-Cross-Platform` page).

## Backlog and direction

The backlog and design notes live in the **GitHub wiki** (<https://github.com/wesleyel/flowie/wiki>), the
only source of truth for them. `docs/roadmap/` is a local clone of the wiki (branch `master`, hidden from this
repo through `.git/info/exclude`): edit, commit and push there, never add it here. Pushing the wiki runs no CI and
leaves no commits in this repo. One page per item, written in Chinese, named in English Title-Case
(`Per-Node-Sync.md`; the name is the page's URL, so don't rename pages); link pages as `[text](Page-Name)`
without `.md`. `Home.md` holds each item's status as its Now/Next/Later/Done group (进行中 / 待办 / 长期 / 已完成);
keep `_Sidebar.md` in step. Code comments cite pages as `wiki: Page-Name §n`. Long-term designs:
- **Per-node sync**: an LWW-per-field engine (`flowie-sync`) with CloudKit and self-hosted backends. The format it needs (fractional-index ranks, soft deletes, deterministic journal IDs, per-field clocks, outbox and tombstones) is in place since database format 6.
- **iOS and cross-platform**: a native iOS app (SwiftUI + UIKit, a reduced feature set) over the Rust core through UniFFI (`flowie-ffi` over `flowie-doc`); the desktop stays on GPUI; the browser comes last as a TypeScript UI over a wasm core. See the wiki's `Release-Plan` for the order.

More docs: `docs/building.md` (platforms, data paths), `docs/usage.md`, `docs/mcp.md`, `docs/sync.md` (the self-hosted sync server), `docs/versioning.md` (versions and compatibility), `docs/translations.md`, `CONTRIBUTING.md`.
