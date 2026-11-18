# Contributing to Flowie

Bug reports, ideas and pull requests are welcome.

## Before you open a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
```

CI runs the same checks on macOS (see `.github/workflows/ci.yml`), plus
[`cargo deny check`](https://github.com/EmbarkStudios/cargo-deny) for dependency licenses and
advisories (`deny.toml`).

- Keep changes focused; one topic per pull request.
- Add user-visible changes under `[Unreleased]` in `CHANGELOG.md`
  ([Keep a Changelog](https://keepachangelog.com/en/1.1.0/)); it becomes the release notes.
- UI text goes through `t!(…)` with keys in `crates/flowie-app/locales/en.json`
  (and `zh-CN.json` when you can); `cargo test -p flowie-app i18n` checks both.
- New features belong in the first-launch tutorial too
  (`crates/flowie-app/tutorial/<locale>.md`).

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
