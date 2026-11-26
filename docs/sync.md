# Sync with your own server

Flowie syncs an outline between your computers through `flowie-server`, a small server you run
yourself: one binary over a SQLite file. It stores what devices send without reading it, and
every device merges the same way, field by field, so edits made on different devices at the
same time — or offline — all come through.

## Run the server

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

## In a browser (preview)

The server can also serve Flowie's browser version, which syncs with it like any device:

```bash
scripts/build-web.sh
./target/release/flowie-server serve --web web
```

Open the server's address and paste a device token. The browser version shows and edits the
outline (open bullets as pages, fold, add, edit, tick off, delete; Enter, Tab and Shift-Tab work as on the desktop; search, stars, today's journal page, images and undo) and keeps it in the browser's own storage (OPFS), so it
opens without the server and sends what changed meanwhile once it can reach it. A browser
without OPFS (an old one, some private windows) keeps it in memory and fetches it again.

## Turn it on in Flowie

In **Settings → Sync**, turn on **Sync with a server**, enter the server's address and the
device's token. The icon in the top bar shows whether sync is up to date, syncing or offline
(click it for details). Edits made while the server can't be reached are kept and sent later.

- Before the first sync with a server, Flowie copies the database beside itself as
  `flowie.sqlite.before-sync-<time>`.
- A new device joining an outline that is already on the server leaves out its own tutorial if
  you haven't touched it.
- The bullet you are typing in takes what changed on other devices when you leave it.
- When two devices change the same text at the same time, the version that loses is listed
  under **Conflict copies** in Settings → Sync, to restore as a bullet or dismiss.
- **Start over from the server** replaces this device's outline with the server's instead of
  merging them; the old one is kept beside the database as `flowie.sqlite.replaced-<time>`
  (with its images), which Flowie can open.
- Collapsed bullets, the page you are on and scroll positions stay per device.
