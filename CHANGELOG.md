# Changelog

All notable changes to Flowie are documented in this file.

Each version's section becomes its [GitHub release](https://github.com/wesleyel/flowie/releases)
notes (`scripts/changelog-section.sh`). A version first ships as release candidates
(`X.Y.Z-rc.N`, as GitHub releases and in the `flowie` Homebrew cask), then as `X.Y.Z`.

## [Unreleased]

### Added

- **Tutorial**: rewritten in three parts: a short Getting started, three examples to adapt (a project, a to-do list and reading notes), and a Manual with every feature, its lessons collapsed. The Images lesson shows an example image.
- **Pages**: the page title now scrolls with the page and collapses into the top bar. The path there then switches from the page's parents to a Home icon followed by the page and the parents of the topmost visible bullet, and the page's reference count moves next to the star. A title left half collapsed snaps open or shut when you stop scrolling.
- **MCP**: add the `batch` tool, which applies many `create`, `journal`, `update`, `complete`, `move` and `delete` operations in one call, all or nothing and as a single undo step. A `create` can carry a `key` so later operations in the batch can refer to the bullet it created.
- **MCP**: `read` and `move` accept `ids` to read several subtrees, or move several bullets in order, in one call.
- **macOS**: add a Window menu with Minimize, Zoom and Bring All to Front. macOS adds its own window items to it, including Fill, Center, Move & Resize, and moving the window to another display or an iPad.

### Changed

- **Search**: the search box rests as a search icon in the top bar and widens into the full box when you click it or press Find; it narrows back once you leave it empty.
- **Pages**: the top bar's path lines up with the page title and uses the same text color as the page.
- **macOS**: What's New is now only in the Help menu, not in the Flowie menu.

### Fixed

- **MCP**: a write that fails partway no longer leaves part of its changes behind, such as a journal page created for a `journal` call whose Markdown was invalid.

## [1.0.0-rc.5] - 2026-11-24

### Added

- **MCP**: add the `current` tool to report the active page, search filter, and focused or selected bullets, and accept `"current"` as a node ID.
- **MCP**: automatically focus bullets edited by an agent (jumping to their page, expanding collapsed ancestors and scrolling them into view), configurable under **Settings → Agent**.

### Fixed

- **Storage**: keep pending changes and retry after a database write fails. Flowie shows a save warning and asks for confirmation if you try to quit or close the window while changes still can't be saved.
- **Editing**: Backspace at the start of a bullet preserves its note when merging and leaves bullets with images or mirror links in place.
- **Images**: startup cleanup preserves old image files that are reused when you paste the same image again.
- **Trash**: entries with damaged or empty data are marked as unrestorable instead of showing a Restore button that does nothing.
- **Trash**: undoing a restore puts the bullets back in Trash instead of losing them. Delete Forever removes every entry for that bullet.
- **MCP**: `complete` on Home returns an error instead of crashing Flowie.
- **MCP**: `delete` checks every ID before deleting anything, so one bad ID no longer leaves earlier deletions half-applied.
- **MCP**: `create` removes the parent's only child as a blank placeholder only if it is an empty plain bullet. Images, mirrors and bullets with a note are kept.
- **MCP**: `read` and `open` with `today` or a date no longer create journal pages. They return an error when the page does not exist, so read-only mode never changes the outline.

## [1.0.0-rc.4] - 2026-11-24

### Breaking changes

- **MCP**: node IDs are now UUID strings or short hexadecimal prefixes; clients sending numeric IDs must update their requests.
- **Storage**: Flowie uses UUIDs for bullets. If the app opens an older database, it keeps the old file beside the new database as `<database>.old-<timestamp>`. Database initialization stops if moving the old file fails.

### Upgrade a database from RC3 to RC4

Quit Flowie and run the one-time migration tool before opening your RC3 database in RC4. The tool supports only `1.0.0-rc.3` → `1.0.0-rc.4` and is available only as an attachment to this release.

- **macOS (Apple silicon and Intel)** — unzip the tool, open Terminal in the extracted folder and run `./migrate-rc3-to-rc4 --replace "$HOME/Library/Application Support/Flowie/flowie.sqlite"`. [Download the macOS tool](https://github.com/wesleyel/flowie/releases/download/v1.0.0-rc.4/Flowie-1.0.0-rc.4-migration-macos.zip).
- **Windows (64-bit)** — unzip the tool, open PowerShell in the extracted folder and run `.\migrate-rc3-to-rc4.exe --replace "$env:LOCALAPPDATA\Flowie\flowie.sqlite"`. [Download the Windows tool](https://github.com/wesleyel/flowie/releases/download/v1.0.0-rc.4/Flowie-1.0.0-rc.4-migration-windows-x64.zip).
- If Flowie uses a different database, substitute its path. With `--replace`, the tool saves a backup next to it as `<database>.rc3-backup-<timestamp>`, including committed WAL changes. Keep the backup until you check your outline in RC4. To make a migrated copy instead, run the tool with the old and new database paths as its two arguments, without `--replace`.
- You can also export OPML from Home with nothing selected in the old app, then import it at Home in the new app. OPML from RC3 and earlier keeps text, order, hierarchy, notes, completion and block types. Recreate internal links and mirrors after importing. Stars, saved searches, Trash, fold state and image files are left out.

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

[Unreleased]: https://github.com/wesleyel/flowie/compare/v1.0.0-rc.4...HEAD
[1.0.0-rc.4]: https://github.com/wesleyel/flowie/compare/v1.0.0-rc.3...v1.0.0-rc.4
[1.0.0-rc.3]: https://github.com/wesleyel/flowie/compare/v1.0.0-rc.2...v1.0.0-rc.3
[1.0.0-rc.2]: https://github.com/wesleyel/flowie/compare/v1.0.0-rc.1...v1.0.0-rc.2
[1.0.0-rc.1]: https://github.com/wesleyel/flowie/releases/tag/v1.0.0-rc.1
