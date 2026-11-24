# Changelog

All notable changes to Flowie are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Each version's section becomes its [GitHub release](https://github.com/wesleyel/flowie/releases)
notes (`scripts/changelog-section.sh`). A version first ships as release candidates
(`X.Y.Z-rc.N`, as GitHub releases and in the `flowie` Homebrew cask), then as `X.Y.Z`.

## [Unreleased]

### Changed

- Bullets now have random UUIDs as ids, the same in every outline a bullet reaches, ready for
  sharing outlines later. An outline database from an earlier version is not converted: Flowie
  renames it to `flowie.sqlite.old-<time>` beside the new one and starts afresh. Export to OPML
  first to keep its contents.
- The MCP server shows bullets with short ids, like git's abbreviated commits: at least 6 hex
  digits, more when another bullet starts the same way. Tools accept any unique prefix of 4 or more
  digits, and links agents write with a short id are saved with the full one.
- Backlinks and mirrors are listed in outline order.

### Fixed

- Clearing a search returns the page to where it was scrolled before the search, instead of to the
  top of the zoomed-in bullet.

## [1.0.0-rc.3] - 2026-11-22

### Added

- Text color shortcuts: ⌘⌃1 to ⌘⌃7 for red, orange, yellow, green, blue, purple and gray, ⌘⌃0 to
  remove the color, and ⌘⌃C to step to the next color in that order. They color the selected text,
  or with nothing selected the whole bullet (which is then selected, so the color shows and ⌘⌃C
  keeps stepping), or every selected bullet. Pressing a color again on text that has it removes
  it. They are also in the Format menu and can be changed in Settings.
- Flowie reopens its window at the size and place you left it, on the same display, and in full
  screen if it was. If that display is no longer attached it opens centered as before, and a window
  that would now be partly off the display is moved and shrunk to fit.
- Windows 10 and 11 (64-bit): each GitHub release includes `Flowie-<version>-windows-x64.zip`. The
  window has its own menu bar and shortcuts use Ctrl where macOS uses ⌘. Because Ctrl+Alt is AltGr
  on many keyboards, no default uses it: block types are Ctrl+Shift+0 to Ctrl+Shift+8, back and
  forward are Alt+← and Alt+→, mirror is Alt+M, and text colors are Alt+0 to Alt+7 and Alt+C. The
  outline lives in `%LOCALAPPDATA%\Flowie`. Linux builds from source with the same shortcuts, and CI
  checks it.
- Opening Flowie while it is already open brings the open window forward instead of opening the
  same outline a second time. When `flowie mcp` starts Flowie for an agent, its window no longer
  takes focus.

### Changed

- Shortcut hints, toasts and the tutorial name the shortcuts as you have set them in Settings,
  written the way your system writes them (⇧⌘P on macOS, Ctrl+Shift+P elsewhere).
- The `claude mcp add` command and MCP client config in Settings → Agent pass `FLOWIE_DB` when
  Flowie runs on a database other than the default one.
- Scrolling the outline with a mouse wheel is now smooth: each step of the wheel glides instead of
  jumping, as a trackpad already did.
- In the light theme, orange and yellow text are easier to tell apart from red and from each
  other.
- Release candidates on GitHub are no longer marked as pre-releases, and the newest version, a
  candidate included, is marked as the latest release.
- The Homebrew cask `wesleyel/tap/flowie` now gets release candidates too, and `flowie@beta` is
  gone: `brew upgrade` moves an installed `flowie@beta` over to `flowie`.
- Enter on a to-do or numbered item starts the next row as the same kind (a new to-do starts
  unchecked), and a new first child takes the kind of the child below it. Enter on an empty to-do
  or numbered item turns it back into a plain bullet, ending the list.

### Fixed

- Reopening a bullet returns to where you had scrolled in it, instead of its top, also after
  quitting and relaunching Flowie.

## [1.0.0-rc.2] - 2026-11-19

### Added

- Each GitHub release includes the open-source version, `Flowie-<version>-macos.zip`: a universal build
  for macOS 13 or later, ad-hoc signed and not notarized.
- Homebrew cask: `brew install --cask wesleyel/tap/flowie`, updated by each release.
- Right-click a bullet's dot for a bullet menu: open, expand or collapse everything below, complete,
  star, change the block type, copy the bullet or its link, duplicate, move, mirror to another
  bullet, export and delete, plus when it was created and edited and what links to it. Inside a
  selection it acts on every selected bullet.
- Scrollbars in the macOS overlay style on the outline, the trash, settings pages, popovers, the
  palette, the `/` menu and right-click menus; menus taller than the window scroll.
- The right-click menu on text now starts with Cut, Copy and Paste, and shows Paste alone when
  nothing is selected.

### Changed

- A more Mac-like look: the macOS system colors and accent colors, menus whose highlighted item
  fills with the accent, rounder menus, popovers and sheets with softer shadows, Mac-style
  segmented controls, buttons, checkboxes, key caps and search field, and a toast that matches
  the window instead of being inverted. Selected rows in Quick Open, the `/` menu and Settings
  are filled with the accent.
- Flowie now runs on a current snapshot of Zed's GPUI (through gpui-kit) instead of the 2025
  release. A shortcut entered in Settings that can't be read is now skipped instead of stopping
  the app.

### Fixed

- The Mac's Emacs-style editing keys now work while editing a bullet: ⌃A / ⌃E for the start or end
  of the line, ⌃B / ⌃F / ⌃P / ⌃N to move, ⌃H / ⌃D to delete, and ⌃K to delete to the end of the
  line, with ⌃Y putting that text back. They can be changed in Settings like any other shortcut.
- Selecting text with the mouse now snaps to the nearest character boundary, as in other Mac apps:
  a drag selects a character once it passes the character's middle, instead of only after crossing
  all of it.

## [1.0.0-rc.1] - 2026-11-18

The first release candidate of Flowie 1.0.0, free software under the GPL-3.0-or-later. 1.0.0
itself comes to the [Mac App Store](https://apps.apple.com/app/id6835913837) and here.

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

[Unreleased]: https://github.com/wesleyel/flowie/compare/v1.0.0-rc.3...HEAD
[1.0.0-rc.3]: https://github.com/wesleyel/flowie/compare/v1.0.0-rc.2...v1.0.0-rc.3
[1.0.0-rc.2]: https://github.com/wesleyel/flowie/compare/v1.0.0-rc.1...v1.0.0-rc.2
[1.0.0-rc.1]: https://github.com/wesleyel/flowie/releases/tag/v1.0.0-rc.1
