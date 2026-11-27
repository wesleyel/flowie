# Contributing to Flowie

Bug reports, ideas and pull requests are welcome. What's planned, and the design notes behind it, are in the
[roadmap wiki](https://github.com/wesleyel/flowie/wiki) (in Chinese); it's kept by the maintainer, so please
bring ideas to [Discussions](https://github.com/wesleyel/flowie/discussions) or a feature request.

## Before you open a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
```

CI runs the same checks on macOS (see `.github/workflows/ci.yml`), plus
[`cargo deny check`](https://github.com/EmbarkStudios/cargo-deny) for dependency licenses and
advisories (`deny.toml`), which also limits which crates may depend on `gpui-kit` and `rusqlite`.

- Keep changes focused; one topic per pull request.
- Add user-visible changes under `[Unreleased]` in `CHANGELOG.md`
  ([Keep a Changelog](https://keepachangelog.com/en/1.1.0/)); it becomes the release notes.
  Add the same entry in Chinese to `CHANGELOG.zh-CN.md` (新增/变更/修复), which What's New
  shows when the app is in Chinese; if you can't write Chinese, say so in the pull request.
- UI text goes through `t!(…)` with keys in `crates/flowie-app/locales/en.json`
  (and `zh-CN.json` when you can); `cargo test -p flowie-app i18n` checks both.
- New features belong in the first-launch tutorial too
  (`crates/flowie-app/tutorial/<locale>.md`).

## App source layout

The desktop app lives under `crates/flowie-app/src`. Keep a feature's state,
actions, and rendering together. `outline/` owns the main editor, `pages/`
owns full-page replacements such as Trash and What's New, and `overlays/`
owns the palette, popovers, and menus. `settings/` is organized by tab.
`app/` holds platform integration, while `text/` and `ui/` provide shared
editing and UI components.

`share/` owns the package dialogs and unread indicators. The package format
and image storage live in `flowie-core`. Screenshot tooling lives in `dev/`
and builds with the `screenshots` feature.

## Core source layout

In `crates/flowie-core/src`, `tree/` holds node IDs, the model, and editing
operations. `io/` handles text, Markdown, and OPML. `package/` handles share
package encoding, export, import tracking, and three-way updates. `store/`
holds SQLite persistence for nodes, Trash, imports, and image metadata, plus
the background writer. `images.rs` provides the content-addressed file store.

`crates/flowie-doc` holds `Workspace`, which owns the tree, undo history and
writer of an open outline: it opens the database and records and saves each
edit. It doesn't depend on GPUI; `Outline` holds one as `ws` and does the UI
part (rows, animation, caret) after each commit.

## Licensing of contributions

Flowie is licensed under the GNU General Public License v3.0 or later (`LICENSE`).
The same code is also sold on the Mac App Store, whose terms don't sit well with the
GPL, so the project needs one extra permission from contributors:

> By submitting a contribution, you license it under the GPL-3.0-or-later **and**
> grant Wesley Yang a perpetual, worldwide, non-exclusive, royalty-free licence to
> distribute it, as part of Flowie, through the Mac App Store and other app stores
> under their terms.

Add a `Signed-off-by:` line to your commits (`git commit -s`) to confirm that you wrote
the change, or have the right to submit it, and agree to the above.
