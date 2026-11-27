# Contributing to Flowie

Bug reports, ideas and pull requests are welcome. What's planned, and the design notes behind it, are in the
[roadmap wiki](https://github.com/wesleyel/flowie/wiki) (in Chinese; see [docs/wiki.md](docs/wiki.md) for
its layout and local setup); it's kept by the maintainer, so please bring ideas to
[Discussions](https://github.com/wesleyel/flowie/discussions) or a feature request.

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

## Project layout

See [docs/workspace.md](docs/workspace.md) for the crate map (`flowie-core`, `flowie-doc`,
`flowie-sync`, `flowie-server`, `flowie-ffi`, `flowie-cloudkit`, `flowie-wasm`, `flowie-math`,
`flowie-app`), the `crates/flowie-app/src` module breakdown, `apple/`, `web/`, and the
edit/commit/render data flow. Keep a feature's state, actions, and rendering together, and avoid
GPUI dependencies in core/document logic (`flowie-core`, `flowie-doc`).

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
