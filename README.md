<p align="center"><img src="assets/icon.png" width="128" alt="Flowie icon"></p>

<h1 align="center">Flowie</h1>

<p align="center"><b>A fast, keyboard-first outliner for thinking in lists.</b><br>
Zoom into any bullet, link and mirror ideas, write math, and search your outline. Your notes are stored on your device.</p>

<p align="center">
  <a href="https://apps.apple.com/app/id6835913837"><img src="assets/mac-app-store-badge.svg" height="48" alt="Download on the Mac App Store"></a>
</p>

<p align="center">
  <a href="https://github.com/wesleyel/flowie/actions/workflows/ci.yml"><img src="https://github.com/wesleyel/flowie/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue" alt="License: GPL-3.0-or-later"></a>
  <img src="https://img.shields.io/badge/macOS-13%2B-black" alt="macOS 13+">
  <img src="https://img.shields.io/badge/Windows-10%2F11-black" alt="Windows 10 and 11 (64-bit)">
</p>

<p align="center"><b>English</b> · <a href="README.zh-CN.md">简体中文</a> · <a href="https://wesleyel.github.io/flowie/">Website</a></p>

<p align="center"><img src="website/images/01-outline-en-US.jpg" width="820" alt="An outline in Flowie with headings, to-dos and math"></p>

## Features

**Write and organize**
- Every bullet is a page: zoom in to focus, zoom out to see the big picture, with smooth animations.
- Indent, collapse, move and drag bullets; select several and act on them together.
- Headings, to-dos, numbered lists, quotes, code blocks and plain text, from the keyboard or the `/` menu.
- Bold, italic, strikethrough, highlight, seven colors and links — markup shows only on the bullet you're editing.
- Notes under any bullet, and undo for everything.
- Paste or drop images into the outline and add captions.

<p align="center">
  <img src="website/images/03-commands-en-US.jpg" width="400" alt="The / command menu">
  <img src="website/images/06-bullet-menu-en-US.jpg" width="400" alt="The bullet menu">
</p>

**Math**
- Inline `$…$` and display `$$…$$` LaTeX, rendered natively as you type, with a live preview while editing.

**Connect ideas**
- Link to any bullet with `[[`. Hover over an internal link to preview its target, or open the target to see backlinks.
- Mirrors: one bullet in several places — edit it anywhere, it changes everywhere.
- `#tags` and `@mentions` with completion; clickable dates.
- A journal page for every day and a calendar to get around.

<p align="center">
  <img src="website/images/07-links-en-US.jpg" width="400" alt="Linking to a page with [[">
  <img src="website/images/04-journal-calendar-en-US.jpg" width="400" alt="Journal and calendar">
</p>

**Find anything**
- ⌘P jumps to any bullet, starred page or saved search — or creates a new page.
- Search with filters: phrases, tags, people, to-dos, completed, headings, notes, dates and "changed in the last N days".

<p align="center">
  <img src="website/images/02-quick-open-en-US.jpg" width="400" alt="Quick open">
  <img src="website/images/05-search-en-US.jpg" width="400" alt="Search with filters">
</p>

**Keep and share your notes**
- Flowie stores the outline in a local SQLite database. No account, no cloud, no tracking.
- Import OPML, Markdown and text (Workflowy exports work); export Markdown, OPML or text.
- Flowie's OPML exports keep internal links and mirrors. To take images with you, use a `.flowie` package or keep the `images` folder with your Markdown export.
- Share a branch as a `.flowie` file. The recipient can preview it, choose where to import it, and merge later updates while keeping their own edits. Use `is:new` to find unread imports.
- Deleted bullets stay in the Trash for 30 days.

<p align="center">
  <img src="website/images/10-images-en-US.jpg" width="400" alt="An image and caption in a Kyoto trip outline">
  <img src="website/images/11-share-package-en-US.jpg" width="400" alt="Previewing a Flowie share package before importing it">
</p>

**Made for the keyboard**
- Every action has a shortcut, and every shortcut can be changed — including mouse side buttons.
- A built-in tutorial that covers every feature.
- Light and dark themes, accent colors, English and 简体中文.
- Read release notes in the app under **Help → What's New**.

<p align="center"><img src="website/images/08-dark-en-US.jpg" width="640" alt="Dark theme with a purple accent"></p>

**Works with AI assistants**
- A built-in [MCP](https://modelcontextprotocol.io) server lets Claude and other assistants read your outline — and edit it, if you allow it. Every change is live and undoable.

<p align="center"><img src="website/images/09-agent-en-US.jpg" width="640" alt="Agent settings: connecting Claude through MCP"></p>

## Get Flowie

### macOS

Requires macOS 13 or later on Apple silicon or Intel.

- **Mac App Store** — a signed, sandboxed build with automatic updates. Buying it supports development. [Get it on the Mac App Store](https://apps.apple.com/app/id6835913837).
- **Homebrew** — install with `brew install --cask wesleyel/tap/flowie` and update with `brew upgrade --cask wesleyel/tap/flowie`.
- **GitHub** — download `Flowie-<version>-macos.zip`, unzip it and move `Flowie.app` to Applications. [Download from GitHub Releases](https://github.com/wesleyel/flowie/releases/latest).

The Homebrew and GitHub builds are not notarized. On macOS 15 or later, try opening `Flowie.app` first, then go to **System Settings → Privacy & Security → Open Anyway** if macOS blocks it. On older macOS versions, right-click the app and choose **Open**.

### Windows

Requires Windows 10 or 11 (64-bit). Download `Flowie-<version>-windows-x64.zip`, unzip it and run `Flowie.exe`. [Download from GitHub Releases](https://github.com/wesleyel/flowie/releases/latest).

GitHub Releases and Homebrew include release candidates (`-rc.N`).

### Build from source

You can build Flowie on macOS, Windows or Linux. [Build guide](docs/building.md).

## Learn more

- [Using Flowie](docs/usage.md) — shortcuts, markup and search syntax
- [AI assistants (MCP)](docs/mcp.md)
- [Translating Flowie](docs/translations.md)
- [Changelog](CHANGELOG.md) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [Issues](https://github.com/wesleyel/flowie/issues) · [Privacy policy](https://wesleyel.github.io/flowie/privacy.html)

## License

© 2026 Wesley Yang. Flowie is licensed under the [GNU General Public License v3.0 or later](LICENSE).
It builds on [GPUI](https://gpui.rs), [RaTeX](https://github.com/erweixin/RaTeX), KaTeX fonts and
[Lucide](https://lucide.dev) icons; the app bundle lists every third-party license.
