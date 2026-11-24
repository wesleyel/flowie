# Changelog

All notable changes to Flowie are documented in this file.

Each version's section becomes its [GitHub release](https://github.com/wesleyel/flowie/releases)
notes (`scripts/changelog-section.sh`). A version first ships as release candidates
(`X.Y.Z-rc.N`, as GitHub releases and in the `flowie` Homebrew cask), then as `X.Y.Z`.

## [Unreleased]

### Breaking changes

- **MCP**: node IDs are now UUID strings or short hexadecimal prefixes; clients sending numeric IDs must update their requests.
- **Storage**: Flowie uses UUIDs for bullets and cannot upgrade an older database in place. On first open, it keeps the old file beside the new database as `<database>.old-<timestamp>`. Database initialization stops if moving the old file fails.
- **Migration**: before upgrading, go to Home in the old version, clear any bullet selection and choose File → Export as OPML…. In the new version, return to Home and choose File → Import… to add the saved outline.
- **Import and export**: OPML from rc.3 and earlier carries text, order, hierarchy, notes, completion and block types. Recreate internal links and mirrors after migrating. Stars, saved searches, Trash, fold state and image files are omitted from those exports.

### Added

- **OPML**: preserve bullet IDs, mirrors and internal links in new exports. Readers that only use the standard `text` attribute display the mirror's source text. Importing again creates an independent copy, with links and mirrors pointing to the corresponding copied bullets.
- **Settings**: add author name and author ID in General, and a device ID per outline database.
- **Sharing**: add Export as Package to save bullets and descendants as a `.flowie` file, with an option to leave completed bullets out.
- **Sharing**: open `.flowie` packages for preview and import into a chosen location; imported bullets keep their IDs, and importing again creates a copy.
- **Sharing**: mark imported package bullets unread and find them with `is:new`.
- **Sharing**: offer to update an imported package while preserving local changes; copied imports retain their update mapping.
- **Images**: attach one image to each bullet by pasting, dropping files or using Insert Image.
- **Images**: transfer image files with a `.flowie` share package or with Markdown and its accompanying `images` folder. OPML carries only the image type and caption.
- **What's New**: browse release notes as nested bullets. Edits last until the page closes. The page can open after an update and marks new versions.

### Changed

- **Identity**: give bullets random UUIDs and show short IDs over MCP.
- **Links**: list backlinks and mirrors in outline order.
- **Editing**: show internal links by label and move or delete the caret over a whole link.
- **Links**: preview a linked bullet's path, text, note and first children on hover.

### Fixed

- **OPML**: when duplicate IDs make a link or mirror ambiguous, import its text rather than pointing it at the wrong bullet.
- **Editing**: moving the caret between bullets keeps its visible column even when a link hides its destination.
- **Navigation**: closing Trash or What's New restores the previous outline selection; outline commands no longer act on the hidden page.
- **Images**: concurrent writes of the same attachment use separate temporary files.
- **Images**: store attachments in a separate folder per database so cleaning up one outline preserves the others' images.
- **Dependencies**: update `url` to 2.5.9 for the stack overflow in deeply nested `blob:` URLs reported as RUSTSEC-2026-0592.
- **Search**: restore the previous scroll position when clearing a search.

## [1.0.0-rc.3] - 2026-11-22

### Added

- **Formatting**: add text color shortcuts, ⌘⌃1 through ⌘⌃7 for red, orange, yellow, green, blue, purple and gray, ⌘⌃0 to clear and ⌘⌃C to cycle.
- **Window**: restore size, position, display and full-screen state when reopening, adjusting the window if its display is gone.
- **Platforms**: add Windows 10 and 11 (64-bit) release archives and support building Linux from source.
- **Window**: focus an existing Flowie window on a second launch, and keep MCP-started windows from taking focus.

### Changed

- **Shortcuts**: show configured shortcuts in hints, toasts and the tutorial using system notation.
- **Agents**: include `FLOWIE_DB` in MCP client configuration when using a non-default database.
- **Scrolling**: animate mouse-wheel scrolling smoothly.
- **Appearance**: improve the contrast between orange and yellow text in the light theme.
- **Releases**: publish GitHub release candidates as the latest version and offer them in the Homebrew cask.
- **Lists**: Enter continues a to-do or numbered list, while Enter on an empty item returns it to a plain bullet.

### Fixed

- **Navigation**: restore a bullet's previous scroll position when reopening it, including after restarting Flowie.

## [1.0.0-rc.2] - 2026-11-19

### Added

- **Releases**: include a universal macOS 13+ build archive with each GitHub release.
- **Installation**: add the Homebrew cask `brew install --cask wesleyel/tap/flowie`.
- **Outline**: add a bullet context menu for opening, folding, completing, starring, moving, mirroring, exporting and deleting bullets.
- **Scrolling**: add macOS overlay scrollbars to the outline, Trash, Settings, popovers, palette and menus.
- **Editing**: add Cut, Copy and Paste to the text context menu.

### Changed

- **Appearance**: update system and accent colors, menus, popovers, buttons, checkboxes, search field and toasts.
- **Dependencies**: use the current Zed GPUI snapshot through gpui-kit and skip unrecognized shortcut settings.

### Fixed

- **Editing**: support macOS Emacs-style editing shortcuts and allow changing them in Settings.
- **Editing**: snap mouse text selection to character boundaries.

## [1.0.0-rc.1] - 2026-11-18

Flowie 1.0.0's first release candidate, free software under GPL-3.0-or-later. The 1.0.0 release will also be available on the [Mac App Store](https://apps.apple.com/app/id6835913837).

### Added

- **Outlining**: zoom into each bullet as a page, with indentation, folding, moving, dragging and multi-selection.
- **Formatting**: add headings, to-dos, numbered lists, quotes, code blocks, plain text, bold, italic, strikethrough, highlights, text colors and links.
- **Math**: render inline `$…$` and display `$$…$$` LaTeX formulas natively with a live preview.
- **Links**: add internal links, backlinks and mirrors.
- **Tags**: add autocomplete for `#tags` and `@mentions`, plus clickable dates.
- **Journal**: create a page for each day and browse days in a calendar.
- **Search**: quickly open bullets, starred pages and saved searches, and filter by phrase, tag, person, to-do state, heading, note, date and recent changes.
- **Storage**: keep data in a local SQLite database without an account or cloud service.
- **Import and export**: support OPML, Markdown and text, including Workflowy exports, and keep deleted bullets in Trash for 30 days.
- **Shortcuts**: make every action customizable and include a built-in tutorial.
- **Appearance**: add light and dark themes, accent colors, English and Simplified Chinese.
- **Agents**: include an MCP server for assistants to read the outline and, when allowed in Settings, edit it.
- **Help**: add Send Feedback, the tutorial, Privacy Policy and Terms of Use.

### Security

- **Dependencies**: bundle SQLite 3.54.1 through rusqlite 0.43 to fix CVE-2026-119375.

[Unreleased]: https://github.com/wesleyel/flowie/compare/v1.0.0-rc.3...HEAD
[1.0.0-rc.3]: https://github.com/wesleyel/flowie/compare/v1.0.0-rc.2...v1.0.0-rc.3
[1.0.0-rc.2]: https://github.com/wesleyel/flowie/compare/v1.0.0-rc.1...v1.0.0-rc.2
[1.0.0-rc.1]: https://github.com/wesleyel/flowie/releases/tag/v1.0.0-rc.1
