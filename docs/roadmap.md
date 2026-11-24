# Roadmap

Designs agreed on but not built yet. Each section says what is decided; open questions are marked.

## Global node ids (done)

Done (Unreleased). Node ids are random UUIDs (v4) everywhere: in memory, in the database, in `flowie:` links, over
MCP and in share packages. There is no separate local id. Not compatible with existing databases
(nothing has shipped yet).

- `NodeId` becomes a `Copy` newtype over `u128`, shown as 32 lowercase hex digits; `ROOT` is the
  nil UUID. JSON carries ids as strings (integers above 2^53 lose precision in JS clients).
- v4, not v7: v7 starts with a 48-bit millisecond timestamp, so nodes made within a few hours share
  their first six hex digits and short ids would grow to a dozen characters. Creation order is in
  `created` already.
- `alloc_id` / `next_id` go away. Mirror rows' virtual ids, marked today by `VIRTUAL_BIT`, are
  marked by a reserved UUID version instead. UI element ids hash the id instead of `id as usize`.
  The database stores ids as 16-byte blobs.

### Short ids, like git

- Shown as the shortest prefix no other node shares, at least 6 hex digits. Computed by sorting all
  ids and comparing each with its two neighbours; cached and updated as nodes come and go. About 6
  digits at 10,000 nodes, 6 to 8 at 100,000.
- MCP prints short ids (`[id:3f9a1c]`) and accepts any unique prefix of at least 4 digits; an
  ambiguous prefix is an error that lists the candidates.
- Everything stored uses the full id: links in text, the database, packages. A prefix unique today
  may not be tomorrow.

## Identity: author and device

Only what share packages need. Nodes carry no author fields: in a one-person outline they would all
say the same thing, and where authorship matters (what an agent changed, diff, collaboration) the
last modifier per node is not enough; that needs the change history below.

- Author: `{ id: uuid, name }`, one per person, kept in settings. The id can be exported and
  imported in Settings, for a new or second device. An author is identified by id, never by name:
  renaming rewrites nothing, and two people with the same name stay apart.
- The default name comes from the OS (macOS `NSFullUserName()`, Windows
  `GetUserNameEx(NameDisplay)`, Linux GECOS, else the login name). It can be changed in Settings →
  General → Author name.
- Device: `device_id` in the database's meta, one per device.

## Change history (later)

Persist the undo history's `Change` records (node before / after) with time, device and authors.
This is the base for "what did Claude change today", undoing only an agent's edits, blame, diff
and collaboration.

- Authors are a list, like a git commit with co-authors. Each MCP client is an agent author, named
  from the MCP `initialize` request's `clientInfo` (e.g. "Claude Code").
- By default an agent edit is attributed to both the user and the agent (user first). Configurable
  in Settings → Agent: *me and the agent* (default) / *the agent only* / *me only*.

## Share packages (`.flowie`)

Export one or more bullets with their children into a file; the receiver double-clicks it to
import.

### Format

A zip file, not a macOS folder package: folder packages get split up or re-wrapped when sent by
mail, chat or cloud drives.

```
mimetype           first entry, stored uncompressed: application/x-flowie-package
manifest.json      who, when, what
outline.json       the node tree
images/<sha256>.<ext>  image files, checked against the hash in their name
preview.md         optional: for Quick Look and people without Flowie
```

- `mimetype` comes first and uncompressed (as in EPUB and ODF), so `file` and other programs
  recognize a package even without its extension.
- JSON entries are deflated; images are stored as they are (already compressed).
- The data is JSON, not SQLite. A package is read whole, so SQLite's random access buys nothing,
  while it would tie the package format to the internal schema (which changes freely), need a
  temp file or `sqlite3_deserialize` to open, and make every import open an untrusted database
  file. JSON is readable, diffable, can be written by scripts and agents, and older readers skip
  fields they don't know.

`manifest.json`:

```json
{
  "format": "flowie.package",
  "version": 1,
  "package_id": "c41d…",
  "title": "Reading notes",
  "description": "One line from the sender",
  "exported_at": "2026-11-24T10:00:00Z",
  "exported_by": { "id": "a1…", "name": "Wesley", "device": "d7…" },
  "app": "Flowie 1.1.0",
  "roots": ["3f9a1c…"],
  "stats": { "nodes": 128, "images": 3, "bytes": 2048000 },
  "outline_sha256": "…"
}
```

`outline.json` is a flat list, not nested `children`: serde_json stops at a nesting depth of 128,
and deep outlines would fail to parse (or overflow the stack without that limit).

```json
{
  "roots": ["3f9a1c…"],
  "nodes": [
    { "id": "3f9a1c…", "parent": null,      "text": "…" },
    { "id": "b70e42…", "parent": "3f9a1c…", "text": "…" }
  ]
}
```

- Each node has `id` (its full UUID), `parent` (null for a root), `text`, `note`, `kind`,
  `completed`, `collapsed`, `created`, `modified`, `image` and `mirror_of`. No authors: who made
  the package is in the manifest.
- Siblings keep the order they have in the array; the internal `rank` floats are not exported.
  A reader builds parent → children in one pass, at any depth.

Rules:

- `version` is the major version: a reader refuses a newer one (and says to update Flowie) and
  ignores unknown fields.
- Mirrors whose source is in the export keep the link (`mirror_of` = an id in the package); others
  are exported as plain copies marked `"was_mirror": true`.
- `flowie:` links to nodes in the export are kept; links to nodes outside it become plain text
  (their label), since the receiver doesn't have those nodes.
- Not exported: Trash, stars, saved searches, window and scroll state.
- Reading is defensive: only the entry names above, a cap on unpacked size, image hashes checked,
  text treated as data only.
- No signing for now. Outline tools that share through a server (Workflowy, Notion, Roam) trust
  the account, and file-based ones don't check at all; packages carry text and images, nothing
  that runs, and keys (devices, loss, revocation) cost a lot of UI. It becomes necessary only if
  collaboration goes local-first without a server, and then for the operation log, not just
  packages. Readers ignore unknown entries, so a `signature.json` can be added later, and an
  author's key can be bound to their existing id the first time it signs.

### Export

- From a bullet's context menu, File → Export as Package…, and the command palette; with several
  bullets selected, each is a root.
- The dialog has a title (default: the bullet's text), an optional description, an "include
  completed" switch, the estimated size, and the line **"The package includes the author name:
  Wesley · Change"**. The default name is the OS full name, often a real name, so the first export
  must show it.
- Then the save panel, `<title>.flowie`.

### Import

- macOS: Info.plist exports the UTI `cn.spokey.flowie.package` (conforming to
  `public.zip-archive`) and declares the `.flowie` document type. A double-click is delivered to
  the running instance through gpui's `on_open_urls`; the sandbox grants read access to a file
  opened this way.
- Windows and Linux: the file association passes the path as an argument. A second launch hands it
  to the running instance with a new `app.open {path}` request on the existing socket, so the
  one-instance-per-database rule stays as it is.
- The import dialog shows: title, author, export time, app version, counts and size, the
  description, a read-only preview of the first levels, **Put in:** (the move-to picker as a new
  `Mode::ImportTo`, **default Home, at the end**), "keep completion", and a note if this package
  was imported before.
- Import is one undo step, then Flowie zooms to and flashes the imported root.
- Imported nodes keep the ids they have in the package: ids are global, so the same node is the
  same id in both outlines, and a later version of the package matches up without any mapping.
- If a root's id already exists here (in the outline or in Trash), the package was imported
  before. The dialog offers **Update** (the three-way merge below; at first only the note) or
  **Import a copy**, which gives every node a new id and rewrites the package's own links and
  mirrors to match.

### Newly imported roots

An imported root is marked until the user has looked at it; no other node shows where it came
from.

- An unread dot (accent color) on the root's bullet, like an unread mail. Hovering it shows "From
  Wesley · imported Nov 24".
- The dot goes away when the user zooms into the root or edits inside it, or with Mark as Seen in
  the bullet's menu.
- `is:new` finds imported roots not seen yet.

### Import records

```sql
CREATE TABLE imports (           -- one row per imported root
    root         BLOB PRIMARY KEY,
    package_id   TEXT NOT NULL,   -- shared by a package's roots, and by a copy imported later
    title        TEXT NOT NULL,
    author_id    TEXT NOT NULL,
    author_name  TEXT NOT NULL,
    exported_at  INTEGER NOT NULL,
    imported_at  INTEGER NOT NULL,
    seen_at      INTEGER          -- NULL while the root shows the unread dot
);
CREATE TABLE import_bases (      -- the base of a three-way merge
    node        BLOB PRIMARY KEY,
    package_id  TEXT NOT NULL,
    base_hash   TEXT NOT NULL     -- hash of the node's content when imported
);
```

When a newer version of the same package arrives, ids match nodes directly and `base_hash` is the
common base: changed only there → update, only here → keep, both → conflict for the user to pick.

## Images

- One image per bullet: a new `BlockKind::Image`. The bullet's text is the caption (editable and
  searchable). No images inside running text; images in notes may come later.
- Files, not database blobs: `Images/<ab>/<sha256>.<ext>` beside `flowie.sqlite`, written once
  (temp file, fsync, rename) before the database refers to them. Backups then copy each image
  once, and deleting frees space without `VACUUM`. The database keeps a `blobs` table of metadata
  (hash, mime, width, height, size). Display thumbnails live in the Caches folder.
- Garbage collection removes files referenced by neither `nodes` nor Trash payloads.
- In: paste, drop a file on a row, Insert Image…, and MCP `attach_image` (base64 data, since the
  sandboxed `flowie mcp` can't read arbitrary paths).
- Shown scaled to the column width with a height cap. The row height is known from the stored size
  before decoding; decoding is off the UI thread with an LRU cache. Space or a double-click opens
  a preview; ⌘C copies PNG and Markdown.
- `has:image` search filter. Markdown export writes `images/` and `![caption](images/…)`; Markdown
  import reads them (needs the user to pick the folder, for the sandbox).
- Share packages carry the images in `images/`.

## What's New page

A page like Trash: outside the outline (not a node under Home), read-only, generated from the
release notes built into the app.

### Content

- The versioned sections of `CHANGELOG.md`, embedded at build time (`include_str!`) and parsed by
  version; `[Unreleased]` only in development builds. Newest first.
- Versions newer than the last one the user saw are marked **New**; older ones are folded and
  open on a click. Each version links to its GitHub release.
- Shown with Flowie's own markup renderer (bold, code, links); links open in the browser.
- Translated, with English as the fallback, per version:
  - `CHANGELOG.md` stays the English source (and the GitHub release notes).
  - Translations live in `crates/flowie-app/locales/whats-new/<code>.md`, in the same
    `## [x.y.z] - date` sections, embedded at build time next to the UI locales.
  - Each version shows in the UI language when that file has its section, otherwise in English;
    a page can mix the two.
  - The i18n coverage test lists released versions missing from each translation (a report, not a
    failure). Translating a version's notes joins the release steps, before tagging.

### Opening

- Menu: Help → What's New (and Flowie → What's New… on macOS beside About), and the command
  palette. No default shortcut.
- Automatically, once per update:
  - Settings keep `seen_version`. At launch, if the running version is newer (semver, so
    `1.1.0-rc.2` → `1.1.0` counts), the page opens after the window appears.
  - Not on a first install: no `seen_version` yet means the tutorial is what the user sees; the
    current version is recorded without opening the page.
  - Not after a downgrade: the lower version is recorded silently.
  - Not on a `--background` launch (started by `flowie mcp`, nobody looking). It opens the first
    time the window is shown or activated instead.
  - Not if the changelog has no section for any version in between (nothing to show); the version
    is still recorded.
  - Settings → General: "Show What's New after updates" (on by default) turns the automatic
    opening off; the menu item still works.
- `seen_version` is written when the page is shown, not when it closes, so a crash or quit while
  it is open doesn't show it again.

### Closing

- Esc, the page's ← button, ⌘[ / mouse back, or going anywhere else (palette, a star, journal,
  calendar, Home, Trash, an agent's `open`). It and Trash replace each other: one overlay at a time
  (`Outline::trash` becomes an `overlay` enum).
- Closing returns to exactly where the user was: the same page, scroll position and caret
  (the page location saved when it opened). It is not a step in back / forward history.
- Edits keep working behind it: agent changes still land (and flash once the page closes);
  ⌘Z closes the page first, then undoes in the outline.
- The window title shows "What's New — Flowie" while it is open.

## Order

1. ~~UUID node ids and short ids~~ (done).
2. ~~Identity: author id and name in settings, device id, Settings → Author name~~ (done).
3. ~~Package export (without images)~~ (done).
4. ~~Package import: file type, hand-off to the running instance, import dialog, unread dot~~ (done;
   Windows and Linux take the path as an argument, but no installer registers the file type yet).
5. ~~Images, then images in packages~~ (done; display thumbnails are not cached on disk yet:
   GPUI decodes and caches each image in memory).
6. Re-importing a package: first the "already imported" note, then the three-way update.
7. Change history with authors, and the agent co-authoring setting.
8. What's New page.
9. Signing, only if collaboration ends up local-first without a server.
