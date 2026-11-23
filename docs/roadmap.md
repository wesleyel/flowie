# Roadmap

Designs agreed on but not built yet. Each section says what is decided; open questions are marked.

## Authors and global node ids

The base for share packages, and later for collaboration and diff.

### Global ids

Local `NodeId`s are database row numbers and mean nothing outside this database. Every node gets
a `uid` (UUIDv7, time-ordered); the local id stays the internal key, so most code is unchanged.

### Authors: one per person, plus a device id

```sql
CREATE TABLE authors (
    id    TEXT PRIMARY KEY,  -- uuid
    kind  TEXT NOT NULL,     -- 'person' | 'agent'
    name  TEXT NOT NULL,     -- last name seen
    seen  INTEGER NOT NULL
);
-- meta: self_author = <uuid>, self_device = <uuid>
```

- An author is identified by id, never by name: renaming rewrites no history, and two people with
  the same name stay apart.
- One author per person, not per database: the same person on a Mac and a Windows PC is one
  author on two devices. The author id can be exported and imported in Settings (later: carried
  by sync). The device id tells "me on my other machine" from "someone else" in a diff.
- The default name comes from the OS (macOS `NSFullUserName()`, Windows
  `GetUserNameEx(NameDisplay)`, Linux GECOS, else the login name). It can be changed in Settings →
  General → Author name.

### Authorship of nodes

`nodes` gains `created_by` and `modified_by`. Both hold a list of author ids, primary author first,
like a git commit with co-authors. Upgrading an existing database attributes every node to
`self_author`.

### Agent edits: co-authored, like git

- Each MCP client is an author of kind `agent`, named from the MCP `initialize` request's
  `clientInfo` (e.g. "Claude Code"), and recorded the first time it writes.
- By default an agent edit is attributed to **both** the user and the agent (user first, agent as
  co-author), the way a commit carries `Co-authored-by`.
- Configurable in Settings → Agent: *me and the agent* (default) / *the agent only* / *me only*.

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
images/<sha256>    image files (once images exist)
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
  "package_id": "0193…",
  "title": "Reading notes",
  "description": "One line from the sender",
  "exported_at": "2026-11-23T10:00:00Z",
  "exported_by": { "id": "a1…", "name": "Wesley", "device": "d7…" },
  "app": "Flowie 1.1.0",
  "roots": ["0192…"],
  "stats": { "nodes": 128, "images": 3, "bytes": 2048000 },
  "outline_sha256": "…"
}
```

`outline.json` is a flat list, not nested `children`: serde_json stops at a nesting depth of 128,
and deep outlines would fail to parse (or overflow the stack without that limit).

```json
{
  "authors": [{ "id": "a1…", "kind": "person", "name": "Wesley" }],
  "roots": ["0192…"],
  "nodes": [
    { "uid": "0192…", "parent": null,     "text": "…" },
    { "uid": "0193…", "parent": "0192…", "text": "…" }
  ]
}
```

- `authors` lists every author the nodes mention.
- Each node has `uid`, `parent` (null for a root), `text`, `note`, `kind`, `completed`,
  `collapsed`, `created`, `modified`, `created_by`, `modified_by`, `image` and `mirror_of`.
- Siblings keep the order they have in the array; the internal `rank` floats are not exported.
  A reader builds parent → children in one pass, at any depth.

Rules:

- `version` is the major version: a reader refuses a newer one (and says to update Flowie) and
  ignores unknown fields.
- Mirrors whose source is in the export keep the link (`mirror_of` = a uid in the package); others
  are exported as plain copies marked `"was_mirror": true`.
- Not exported: Trash, stars, saved searches, window and scroll state.
- Reading is defensive: only the entry names above, a cap on unpacked size, image hashes checked,
  text treated as data only.
- No signing for now. Outline tools that share through a server (Workflowy, Notion, Roam) trust
  the account, and file-based ones don't check at all; packages carry text and images, nothing
  that runs, and keys (devices, loss, revocation) cost a lot of UI. It becomes necessary only if
  collaboration goes local-first without a server, and then for the operation log, not just
  packages. Readers ignore unknown entries, so a `signature.json` can be added later, and an
  author's key can be bound to their existing uuid the first time it signs.

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
  `Mode::ImportTo`, **default Home, at the end**), "keep original authors" and "keep completion",
  and a note if this package was imported before.
- Import is one undo step, then Flowie zooms to and flashes the imported root.
- Imported nodes get new local uids (importing twice never collides). Their source is recorded in
  `origins` (below). "Keep original authors" off attributes everything to me.

### Prepared for diff and collaboration

Recorded from version 1, so later work needs no second database upgrade:

```sql
CREATE TABLE origins (
    uid         TEXT PRIMARY KEY,  -- local node uid
    package_id  TEXT NOT NULL,
    source_uid  TEXT NOT NULL,     -- uid in the package
    base_hash   TEXT NOT NULL,     -- hash of the node's content when imported
    imported_at INTEGER NOT NULL
);
```

When a newer version of the same package arrives, `source_uid` matches nodes and `base_hash` is
the common base of a three-way merge: changed only there → update, only here → keep, both →
conflict for the user to pick. Authors and times label who changed what.

Live collaboration later needs a persistent operation log with author and device per change. The
undo history's `Change` (node before / after) is already that shape.

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

## Order

1. Authors and uids: database upgrade, Settings → Author name, agent co-authoring setting.
2. Package export (without images).
3. Package import: file type, hand-off to the running instance, import dialog.
4. Images, then images in packages.
5. Re-importing a package: first a "already imported" note, then the three-way update.
6. Signing, only if collaboration ends up local-first without a server.
