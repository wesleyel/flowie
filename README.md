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
  <img src="https://img.shields.io/badge/Windows-preview-black" alt="Windows preview (10 and 11, x64)">
</p>

<p align="center"><b>English</b> · <a href="README.zh-CN.md">简体中文</a> · <a href="https://wesleyel.github.io/flowie/">Website</a> · <a href="website/images">Screenshots</a></p>

<p align="center"><img src="website/images/01-outline-en-US.jpg" width="820" alt="A project page in Flowie with headings, to-dos, mentions and dates"></p>

## Fast with big outlines

Measured on an Apple M4 with Bike's *Moby-Dick* outline imported five times, 13,505 bullets
([benchmark](crates/flowie-core/benches/large_outline.rs): `scripts/fetch-bench-data.sh`, then
`cargo bench -p flowie-core --bench large_outline`):

| Benchmark | Result |
|---|---:|
| Import all 13,505 bullets | 42 ms |
| Lay out the whole outline, everything expanded | 1.3 ms |
| Save it to the database (in the background) | 99 ms |
| The outline in memory | 9.6 MB |
| Undo for the entire import | 0.4 MB |
| Read a 6.4 MB Markdown file | 6 ms |

Flowie is native Rust drawn on the GPU with [GPUI](https://gpui.rs). It draws only the rows on
screen, and works out every other row's height from its text, so the scrollbar is right before
you scroll.

## Features

**Write and organize**
- Every bullet is a page: zoom in to focus, zoom out to see the big picture, with smooth animations.
- Indent, collapse, move and drag bullets; select several and act on them together.
- Headings, to-dos, numbered lists, quotes, code blocks and plain text, from the keyboard or the `/` menu.
- Bold, italic, strikethrough, highlight, seven colors and links — markup shows only on the bullet you're editing.
- Notes under any bullet, and undo for everything.
- Paste or drop images into the outline and add captions.

**Math**
- Inline `$…$` and display `$$…$$` LaTeX, rendered natively as you type, with a live preview while editing.

**Connect ideas**
- Link to any bullet with `[[`. Hover over an internal link to preview its target, or open the target to see backlinks.
- Mirrors: one bullet in several places — edit it anywhere, it changes everywhere.
- `#tags` and `@mentions` with completion; clickable dates.
- A journal page for every day and a calendar to get around.

**Find anything**
- ⌘P jumps to any bullet, starred page or saved search — or creates a new page.
- Search with filters: phrases, tags, people, to-dos, completed, headings, notes, dates and "changed in the last N days".

**Keep and share your notes**
- Flowie stores the outline in a local SQLite database. No account, no cloud service, no tracking.
- Sync your computers through a Flowie server you run yourself (`flowie-server`, see [docs/sync.md](docs/sync.md)): each bullet syncs on its own, offline edits included.
- Import OPML, Markdown and text (Workflowy exports work); export Markdown, OPML or text.
- Flowie's OPML exports keep internal links and mirrors. To take images with you, use a `.flowie` package or keep the `images` folder with your Markdown export.
- Share a branch as a `.flowie` file. The recipient can preview it, choose where to import it, and merge later updates while keeping their own edits. Use `is:new` to find unread imports.
- Deleted bullets stay in the Trash for 30 days.

**Made for the keyboard**
- Every action has a shortcut, and every shortcut can be changed — including mouse side buttons.
- A built-in tutorial: a quick start, three examples to adapt and a manual covering every feature.
- Light and dark themes, accent colors, English and 简体中文.
- Read release notes in the app under **Help → What's New**.

**Works with AI assistants**
- A built-in [MCP](https://modelcontextprotocol.io) server lets Claude and other assistants read your outline — and edit it, if you allow it. Every change is live and undoable.

<p align="center"><img src="website/images/08-dark-en-US.jpg" width="640" alt="Dark theme with a purple accent"></p>

## Get Flowie

### macOS 13+

- [Mac App Store](https://apps.apple.com/app/id6835913837)
- [GitHub Releases](https://github.com/wesleyel/flowie/releases/latest)
- **Homebrew**: `brew install --cask wesleyel/tap/flowie`

The Homebrew and GitHub builds are not notarized, so macOS may initially prevent them from opening. After moving Flowie to Applications (or installing it with Homebrew), run the following command in Terminal to remove the quarantine flag, then open Flowie. Repeat after each update:

```sh
xattr -dr com.apple.quarantine /Applications/Flowie.app
```

### Windows 10/11 (x64) preview

— [GitHub Releases](https://github.com/wesleyel/flowie/releases/latest)

### Linux x86_64 preview

— [GitHub Releases](https://github.com/wesleyel/flowie/releases/latest)

### Build from source

— [Build guide](docs/building.md).

## Learn more

- [Using Flowie](docs/usage.md) — shortcuts, markup and search syntax
- [AI assistants (MCP)](docs/mcp.md)
- [Translating Flowie](docs/translations.md)
- [Changelog](CHANGELOG.md) · [Roadmap](https://github.com/wesleyel/flowie/wiki) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [Issues](https://github.com/wesleyel/flowie/issues) · [Privacy policy](https://wesleyel.github.io/flowie/privacy.html)

## Credits

Flowie builds on [GPUI](https://gpui.rs), [RaTeX](https://github.com/erweixin/RaTeX), KaTeX fonts and [Lucide](https://lucide.dev) icons.

Thanks to the [LINUX DO](https://linux.do) community for feedback and support.

## License

© 2026 Wesley Yang. Flowie is licensed under the [GNU General Public License v3.0 or later](LICENSE); the app bundle lists every third-party license.
