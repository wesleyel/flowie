# Roadmap wiki

Flowie's backlog and design notes live in the [GitHub wiki](https://github.com/wesleyel/flowie/wiki)
(`wesleyel/flowie.wiki.git`, branch `master`), written in Chinese and maintained by the developer.
It is the only source of truth for planned work and long-term designs; shipped changes live in
[CHANGELOG.md](../CHANGELOG.md) and [CHANGELOG.zh-CN.md](../CHANGELOG.zh-CN.md). Contributors should
bring ideas to [Discussions](https://github.com/wesleyel/flowie/discussions) or an issue rather
than editing the wiki directly (see [CONTRIBUTING.md](../CONTRIBUTING.md)).

## Local setup

Run the idempotent bootstrap script from any clone:

```bash
scripts/setup-wiki.sh
```

This does four things (and is safe to re-run):

1. If `wiki/.git` does not exist, clones `wesleyel/flowie.wiki.git` into `wiki/` using the same
   protocol (SSH or HTTPS) as the main repository's `origin` remote.
2. Appends `/wiki/` and `/.to` to `.git/info/exclude`, and `/_private/` to
   `wiki/.git/info/exclude` (deduplicated).
3. Creates `.to` at the repository root containing `wiki` if it does not exist.
4. Checks whether `to` is installed and supports nested scopes (`to -C wiki scopes`), installing
   or upgrading it with `cargo install --git https://github.com/wesleyel/to` when missing or
   outdated.

`wiki/` and `.to` remain local-only and are never committed to the main repository. Edit, commit,
and push inside `wiki/`; pushing the wiki runs no CI and adds no commits to the main repository.

## Directory layout

The wiki is sharded into nested domain directories, each holding a `TODO-<scope>.md` board and
related design notes:

- `release/` — milestone schedule (`Release-Plan.md`, `TODO-release.md`)
- `sync/` — per-node sync design and tasks (`Per-Node-Sync.md`, `TODO-sync.md`)
- `ios/` — iOS app and cross-platform architecture (`iOS-Cross-Platform.md`, `TODO-ios.md`)
- `web/` — browser preview design and tasks (`Web-Client.md`, `TODO-web.md`)
- `desktop/` — desktop backlog, feedback, and Linux preview (`Backlink-Entries.md`, `Feedback.md`, `Linux.md`, `TODO-desktop.md`)
- `archive/` — archived design pages for shipped features (`Large-Title-Collapse.md`, `MCP-Batch.md`, `System-Tray.md`)
- `_private/` — local-only notes, excluded via `wiki/.git/info/exclude` and never pushed

`Home.md` and `_Sidebar.md` act as the top-level index and sidebar; `to` updates their generated
sections (`<!-- to:home:start -->` / `<!-- to:home:end -->` and
`<!-- to:sidebar:start -->` / `<!-- to:sidebar:end -->`) automatically.

## Managing tasks with `to`

With `.to` pointing to `wiki`, `to` commands work from the repository root (or pass `-C wiki`):

- Inspect:
  - `to scopes` — list domain boards with open/done counts
  - `to open` — list unfinished tasks across scopes (hides completed tasks)
  - `to ls --scope <scope> [--json]` — list tasks with optional `--section`, `--priority`, `--label`, `--all`, `--notes`
  - `to show <ref>`, `to tree <ref>`, `to next [--scope <scope>]` — inspect details (`<ref>` is a global number or `<scope>:<n>`)
- Mutate:
  - `to add "title" --scope <scope> --section <sec> --priority <high|medium|low> --label <l> --note "..."` (use `--note -` for stdin)
  - `to edit <ref> ...`, `to mv <ref> ...`, `to done <ref> [--archive]`, `to uncheck <ref>`, `to archive`

Every write via `to` refreshes `Home.md` and `_Sidebar.md` automatically (pass `--no-sync` to
skip, or run `to home` manually).

## Page conventions and citations

- Design pages stay in English Title-Case (for example `Per-Node-Sync.md`). On GitHub Wiki, the
  file basename is the page URL regardless of subdirectory, so **never rename existing pages**
  and keep basenames unique across scopes.
- Link wiki pages as `[text](Page-Name)` without `.md` or directory prefixes.
- Code comments and commit messages cite wiki sections as `wiki: Page-Name §n` (for example
  `wiki: Per-Node-Sync §5.2` or `wiki: iOS-Cross-Platform §3.3`).
