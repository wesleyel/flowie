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
cargo check -p flowie-core --no-default-features --target wasm32-unknown-unknown   # the model without store/writer-thread
scripts/bundle-macos.sh target/release/flowie Flowie flowie.spokey.cn   # local .app
```

CI (`.github/workflows/ci.yml`) runs fmt, both changelog checks, cargo-deny, the wasm check above,
and a clippy/test matrix on pull requests and pushes to `main`. Run Desktop Packages manually to get
temporary test artifacts; push a `vX.Y.Z` or `vX.Y.Z-rc.N` tag to publish all three platforms from the
same pipeline after package smoke checks pass. Homebrew tracks the newest release, RCs included. The Website
workflow checks Pages on pull requests and deploys it from `main`. Other env vars: `FLOWIE_THEME`,
`FLOWIE_WINDOW_SIZE`, `FLOWIE_SHOT_*` (screenshots).

## Rules

- **Commit on `main`**, without feature branches. Push only when asked. One change is one commit.
- **Before every commit**: for user-visible changes, add an `## [Unreleased]` entry in **both** changelogs, in the style of the existing entries: `CHANGELOG.md` (Added/Changed/Fixed) and the matching Chinese entry in `CHANGELOG.zh-CN.md` (新增/变更/修复), which What's New shows in the Chinese UI. Then run fmt and clippy as above.
- **Maintain database compatibility**: 1.0 freezes SQLite `BASELINE` at 5. Update `SCHEMA` and append an ordered `MIGRATIONS` step for each new database format; never change the baseline or reorder released steps. Test opening an old database and compare its upgraded schema against a fresh one. RC and development formats below 5 are kept aside, not migrated.
- **UI text** goes through `t!(…)`, with keys in `crates/flowie-app/locales/en.json` and `zh-CN.json`. `cargo test -p flowie-app i18n` checks both files. New features also get a lesson in the tutorial (`crates/flowie-app/tutorial/<locale>.md`).
- **Docs scope**: the README, website, `llms.txt` and the App Store text cover core features only. Never add UX polish there. A big change in one area goes only in that area's doc (e.g. `docs/mcp.md`). Do fix wording that a change has made wrong.
- **Feature requests**: when the user describes a feature, clarify it and record it in the backlog (`docs/roadmap/`). Don't implement it unless they ask you to build it.
- Keep a feature's state, actions and rendering together. New editing logic should not depend on GPUI where that can be avoided (see `docs/roadmap/ios-cross-platform.md`).

## Backlog and direction

The backlog lives in `docs/roadmap/`, which is **not part of this repository**: it's a separate, local-only
git repo (hidden through `.git/info/exclude`), so commit roadmap edits inside `docs/roadmap/` and never add it
here. One Markdown file per item (written in Chinese), indexed in `docs/roadmap/README.md`, which holds each
item's status (长期 / 待办 / 进行中 / 已完成). Discuss and revise designs by editing those files. Long-term designs:
- **Per-node sync**: an LWW-per-field engine with CloudKit and self-hosted backends. It needs fractional-index ranks, soft deletes and deterministic journal IDs first.
- **iOS and cross-platform**: one GPUI UI on every platform (an iOS backend now exists upstream, as `gpui-pre-ios`), with a GPUI-free `flowie-session` crate for commands, selection and undo, and thin native shells. It starts with a small GPUI-on-iOS prototype.

More docs: `docs/building.md` (platforms, data paths), `docs/usage.md`, `docs/mcp.md`, `docs/translations.md`, `CONTRIBUTING.md`.
