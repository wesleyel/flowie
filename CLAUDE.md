# Flowie — agent guide

Flowie is a keyboard-first outliner written in Rust on GPUI (`gpui-kit`'s `gpui-pre` snapshots).
It ships on macOS (App Store, GitHub releases, Homebrew cask), with Windows and Linux previews,
an in-development iOS app (`apple/`), and a browser preview (`web/`). It's a solo project; release
candidates use `X.Y.Z-rc.N` tags.

## Workspace

See [docs/workspace.md](docs/workspace.md) for the crate breakdown (`crates/*`), desktop module
layout (`crates/flowie-app/src/`), Apple (`apple/`) and browser (`web/`) layers, `third_party/`,
and the edit/commit/render data flow.

## Commands

```bash
cargo run -p flowie-app                         # dev run; FLOWIE_DB=/tmp/x.sqlite for a scratch outline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked
cargo test --workspace --locked
cargo check -p flowie-core -p flowie-sync --no-default-features --target wasm32-unknown-unknown
scripts/check-formats.sh                        # SQLite format snapshot & upsert checks
scripts/release-notes.sh --check                # verify CHANGELOG.md and CHANGELOG.zh-CN.md match
scripts/setup-wiki.sh                           # bootstrap local wiki/ clone, .to, excludes, and `to` CLI
```

Debug env vars: `FLOWIE_DB`, `FLOWIE_THEME`, `FLOWIE_WINDOW_SIZE`, `FLOWIE_SHOT_*` (screenshots).
See [docs/building.md](docs/building.md) for platform prerequisites, packaging scripts, data paths,
CI workflows, and release pipelines.

## Rules

- **Start new feature**: create task with `to` cli. Tick it when completed.
- **Commit on `main`**, without feature branches. Push only when asked. One change is one commit.
- **Before every commit**: for user-visible changes, add an `## [Unreleased]` entry in **both**
  `CHANGELOG.md` (`Added`/`Changed`/`Fixed`) and `CHANGELOG.zh-CN.md` (`新增`/`变更`/`修复`), which
  What's New shows in Chinese. Then run `fmt` and `clippy` as above.
- **Database compatibility**: follow [docs/versioning.md](docs/versioning.md) — keep `BASELINE = 5`,
  never edit released `formats/v*.sql` snapshots or earlier migrations, use column-preserving upserts
  (`ON CONFLICT … DO UPDATE`), and pass `scripts/check-formats.sh`.
- **UI text & tutorial**: route strings through `t!(…)`, validate `en.json` and `zh-CN.json` with
  `cargo test -p flowie-app i18n`, and add new features to `crates/flowie-app/tutorial/<locale>.md`
  (see [docs/translations.md](docs/translations.md)).
- **Docs scope**: `README.md`, `README.zh-CN.md`, `website/`, `llms.txt`, and App Store copy cover
  core features only (never add UX polish there). Area-specific docs live in `docs/`; fix wording
  that a change makes wrong.
- **Feature requests**: clarify the requirement and record it in the wiki ([docs/wiki.md](docs/wiki.md));
  do not implement it unless asked.
- **Architecture**: keep a feature's state, actions, and rendering together, and avoid GPUI
  dependencies in new editing logic where possible (see [docs/workspace.md](docs/workspace.md) and
  the wiki's `iOS-Cross-Platform`).

## Backlog and wiki

The backlog and long-term design notes live in the [GitHub wiki](https://github.com/wesleyel/flowie/wiki),
the only source of truth for them. `wiki/` is a local clone of that wiki (branch `master`, hidden
from this repo via `.git/info/exclude` along with `.to`): edit, commit, and push inside `wiki/`,
never add it here.

Bootstrap on a fresh clone with `scripts/setup-wiki.sh`, then manage boards via `to` (`to open`,
`to ls`, `to add`, `to edit`, `to done`, etc.). See [docs/wiki.md](docs/wiki.md) for setup details,
`to` CLI usage, directory sharding, `_private/`, page/citation conventions (`wiki: Page-Name §n`),
and long-term design summaries.

## Docs

- [docs/workspace.md](docs/workspace.md) — workspace crates, desktop modules, Apple/web layers, data flow
- [docs/wiki.md](docs/wiki.md) — roadmap wiki setup (`scripts/setup-wiki.sh`), `to` workflow, conventions
- [docs/building.md](docs/building.md) — platform toolchains, building, packaging, checks, data paths, releases
- [docs/usage.md](docs/usage.md) — user guide: shortcuts, import/export, search, markup
- [docs/mcp.md](docs/mcp.md) — MCP server tools and stability guarantees
- [docs/sync.md](docs/sync.md) — self-hosted sync server (`flowie-server`), browser sync, iCloud/CloudKit
- [docs/versioning.md](docs/versioning.md) — SemVer, SQLite migrations, package/wire/MCP compatibility
- [docs/translations.md](docs/translations.md) — UI localization and translation checks
- [CONTRIBUTING.md](CONTRIBUTING.md) — contributor workflow and licensing
