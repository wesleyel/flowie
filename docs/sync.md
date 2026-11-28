# Sync

Flowie syncs an outline between your devices in one of two ways, chosen in **Settings → Sync**:

- **iCloud**: the Mac App Store version and the iPhone app sync through your iCloud account,
  with nothing to set up (see [iCloud](#icloud) below).
- **Your own server**: every desktop version (and the browser version) syncs through
  `flowie-server`, a small server you run yourself: one binary over a SQLite file.

Either way an outline syncs with one of them at a time.

## Sync with your own server

Flowie syncs an outline between your computers through `flowie-server`, a small server you run
yourself: one binary over a SQLite file. It stores what devices send without reading it, and
every device merges the same way, field by field, so edits made on different devices at the
same time — or offline — all come through.

### Run the server

Build it from this repository (see [building.md](building.md) for the toolchain):

```bash
cargo build --release -p flowie-server
```

Issue a token for each device, and keep it: it is shown only once.

```bash
./target/release/flowie-server token add laptop
```

Then serve:

```bash
./target/release/flowie-server serve --listen 127.0.0.1:8787
```

| Option | Default | |
|---|---|---|
| `--db PATH` | `flowie-server.sqlite` | the records and tokens |
| `--blobs DIR` | `blobs` | image files |
| `--listen ADDR` | `127.0.0.1:8787` | where to listen |

`flowie-server token list` shows the devices, and `flowie-server token revoke NAME` locks one
out. The server speaks plain HTTP: put it behind a reverse proxy that terminates TLS (Caddy,
nginx), and give devices the `https://` address. Every token can read and write the whole
outline, so issue them only to your own devices.

### In a browser (preview)

The server can also serve Flowie's browser version, which syncs with it like any device:

```bash
scripts/build-web.sh
./target/release/flowie-server serve --web web
```

Open the server's address and paste a device token. The browser version shows and edits the
outline (open bullets as pages, fold, add, edit, tick off, delete; Enter, Tab and Shift-Tab work as on the desktop; search, stars, today's journal page, images and undo) and keeps it in the browser's own storage (OPFS); a service worker keeps the page's own files, so
it opens without the server, can be installed as an app, and sends what changed meanwhile
once it can reach the server. A browser
without OPFS (an old one, some private windows) keeps it in memory and fetches it again.

### Turn it on in Flowie

In **Settings → Sync**, choose **My own server**, enter the server's address and the
device's token. The icon in the top bar shows whether sync is up to date, syncing or offline
(click it for details). Edits made while the server can't be reached are kept and sent later.

- Before the first sync with a server (or iCloud), Flowie copies the database beside itself as
  `flowie.sqlite.before-sync-<time>`.
- A server (or iCloud) this outline didn't sync with last gets all of it, bullets written before
  sync was turned on included.
- A new device joining an outline that is already on the server leaves out its own tutorial if
  you haven't touched it.
- The bullet you are typing in takes what changed on other devices when you leave it.
- When two devices change the same text at the same time, the version that loses is listed
  under **Conflict copies** in Settings → Sync, to restore as a bullet or dismiss.
- **Start over from the server** replaces this device's outline with the server's instead of
  merging them; the old one is kept beside the database as `flowie.sqlite.replaced-<time>`
  (with its images), which Flowie can open.
- Collapsed bullets, the page you are on and scroll positions stay per device.

## iCloud

On the Mac, choose **iCloud** in Settings → Sync; the iPhone app syncs through iCloud by itself.
Both need the device to be signed in to iCloud (System Settings → Apple Account on the Mac, the
Settings app on the iPhone), and sync the outline through that account's private iCloud
database, so only you can read it; its contents are stored in CloudKit's encrypted fields. Only
builds signed for Flowie's iCloud container can use it: the Mac App Store version (and
TestFlight) on the Mac, where it needs macOS 14 or later. A build from source, Homebrew or a
GitHub release says *iCloud is unavailable in this build* and can sync with your own server.

Everything in the list above holds for iCloud too (the snapshot before the first sync, the
tutorial, the bullet you are typing in, conflict copies), except Start over, which is only for a
server. Signing out of iCloud stops sync and keeps the outline on the device; signing in to
another account sends the whole outline to that account.

### CloudKit schema

What Flowie keeps in CloudKit, in the container `iCloud.flowie.spokey.cn` (deployed to
Production once from the CloudKit Dashboard; after that, record types and fields can only be
added):

| | |
|---|---|
| Database | the user's private database |
| Zone | `Outline` (a custom zone; CKSyncEngine creates it, and makes it again if the user deletes Flowie's iCloud data) |
| Record type | `Record`, one per Flowie record: a bullet (record name: its id, 32 hex digits), its tombstone once deleted for good, and the synced preferences (record name `prefs`) |
| Field | `envelope`: String, encrypted (`encryptedValues`). The record as JSON (the envelope in `flowie-sync::wire`: version, kind, id, payload); each record's kind is inside it, so new kinds need no schema change |
| Record type | `Blob`, one per image file (record name: `flowie_sync::blob_name`, today the file's own name, `<sha256>.<ext>`) |
| Fields | `file`: Asset (a `CKAsset`; assets can't be encrypted fields, CloudKit encrypts them at rest anyway); `width`, `height`: Int(64), the size in pixels (0 when not known); `mime`: String. All plain fields, none indexed |
| Indexes | none needed: CKSyncEngine fetches by zone changes, not by query. (`recordName` queryable only helps browsing records in the Dashboard; encrypted fields can't be indexed) |
| Subscriptions | CKSyncEngine's own database subscription (silent pushes) |

Records are saved with their last-seen system fields, so a save over a newer version fails with
`serverRecordChanged` and the two versions merge field by field before the next save.

Image files travel as `Blob` records. The engine (`NativeSync::next_saves`) puts a bullet's file
just before the bullet the first time it is sent, unless it knows the file is in iCloud already;
a `Blob` is always saved as a new record, so `serverRecordChanged` on one means the same file is
there (its name is its contents). A fetched file is kept only if it is an image whose name
matches the record's, and noted with its size; a bullet whose file comes later shows a
placeholder (on iOS: *Image not downloaded from iCloud yet*) until it does. After each fetch,
the files that rows show but this device doesn't have (`NativeSync::missing_files`) are asked
for by name, each once while the engine runs (again if iCloud had it but sent no file): a
`Blob` that came without its download is behind CKSyncEngine's place in the zone and wouldn't
come again otherwise. Each `Blob` fetched or sent is logged (subsystem `flowie.spokey.cn`,
category `icloud`) with what became of it; debug builds also write those lines to
`sync-debug.log` beside `icloud-sync-state.json` (256 KB, then `sync-debug.old.log`). Nothing is ever
deleted from the zone: Flowie ignores every record deletion it fetches, and unused files aren't
collected yet. Versions before `Blob` ignore those records.

The `Blob` type and its fields are created in the Development schema by the first save from a
development-signed build, and must then be deployed to Production from the CloudKit Dashboard
before a release build can save one.
