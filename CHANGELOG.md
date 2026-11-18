# Changelog

All notable changes to Flowie are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Each version's section becomes its [GitHub release](https://github.com/wesleyel/flowie/releases)
notes (`scripts/changelog-section.sh`).

## [Unreleased]

## [1.0.0] - 2026-11-18

The first release, on the [Mac App Store](https://apps.apple.com/app/id6835913837) and as
free software under the GPL-3.0-or-later.

### Added

- **Outlining**: every bullet is a page you can zoom into and out of, with smooth animations;
  indent, collapse, move and drag bullets, and act on a multi-bullet selection.
- **Formatting**: headings, to-dos, numbered lists, quotes, code blocks and plain text, from the
  keyboard or the `/` menu; bold, italic, strikethrough, highlight, seven colors and links, with
  markup shown only on the bullet being edited. Notes under any bullet; undo for everything.
- **Math**: inline `$…$` and display `$$…$$` LaTeX, rendered natively with a live preview.
- **Links and mirrors**: `[[` links with backlinks, and mirrors that show one bullet in several
  places.
- **Tags, mentions and dates**: `#tags` and `@mentions` with completion, plus clickable dates.
- **Journal**: a page for every day and a calendar.
- **Quick open and search**: ⌘P jumps to any bullet, starred page or saved search, or creates a
  page. Search filters by phrase, tag, person, to-do state, headings, notes, dates and recent
  changes.
- **Local storage**: one SQLite database on your Mac; no account, no cloud, no tracking.
- **Import and export**: import OPML, Markdown and text (including Workflowy exports), and export
  Markdown, OPML or text. Deleted bullets stay in a 30-day Trash.
- **Keyboard**: a customizable shortcut for every action, including mouse side buttons, and a
  built-in tutorial.
- **Appearance and language**: light and dark themes, accent colors, and English and 简体中文.
- **AI assistants**: a built-in MCP server lets assistants such as Claude read the outline, and
  edit it if allowed in Settings → Agent.
- **Help menu**: Send Feedback, the tutorial, the Privacy Policy and the Terms of Use.

### Security

- Bundled SQLite 3.54.1 (through rusqlite 0.43), which fixes CVE-2026-119375.

[Unreleased]: https://github.com/wesleyel/flowie/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/wesleyel/flowie/releases/tag/v1.0.0
