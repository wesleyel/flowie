# Flowie

A fast, animated outliner in the spirit of Workflowy, built with [GPUI](https://gpui.rs) (Zed's GPU UI framework) and [RaTeX](https://github.com/erweixin/RaTeX) for KaTeX-compatible math. macOS first; the core is platform-free and GPUI also runs on Windows and Linux.

## Build and run

```bash
cargo build --release -p flowie-app
scripts/bundle-macos.sh target/release/flowie Flowie dev.flowie.app
open build/Flowie.app
```

No Xcode is needed: GPUI's `runtime_shaders` feature compiles Metal shaders at launch.

- Data: `~/Library/Application Support/Flowie/flowie.sqlite` (override with `FLOWIE_DB=/path/file.sqlite`).
- `--stress 50000`: opens an in-memory outline of 50k bullets (not saved) and logs timings.
- Settings (⌘,) are saved in `settings.json` next to the database: theme, accent color and shortcut overrides. Click a shortcut to replace it, use `+` to add another keyboard or mouse side-button binding, and press Backspace while recording to remove that binding.

## Keys

| Key | Action |
|---|---|
| Enter | New bullet (splits at the caret) |
| Shift-Enter | Edit / leave the note |
| Tab / Shift-Tab | Indent / outdent |
| Backspace at start | Merge into the bullet above |
| ⌘Enter | Complete (or check a to-do) |
| ⌘↑ / ⌘↓ | Collapse / expand |
| ⌃1 … ⌃4 / ⌃0 | Show 1–4 levels / expand all |
| ⌘] / ⌘[ / ⌘⇧H | Zoom in / out / home (or click a bullet / breadcrumb) |
| Mouse back / forward, ⌘⌥← / ⌘⌥→ | Previous / next visited page (restores search and scroll position) |
| ⌘P | Go to: starred pages, saved searches, any bullet |
| ⌘⇧S | Star / unstar the page |
| ⌘⇧M | Move to… |
| ⌘⌥M | Mirror a bullet here (live copy: edits show in both places) |
| ⌘⇧K or `[[` | Link to another bullet (backlinks show at the bottom of its page) |
| ⌘⇧↑ / ⌘⇧↓ | Move bullet |
| ⌘⇧Backspace | Delete bullet (kept in Trash for 30 days) |
| Esc | Select whole bullets; Shift-↑/↓ or dragging across rows extends |
| `/` | Command menu: block types, colors, highlight, math, date, link, mirror, move |
| `#` / `@` | Tag / mention completion |
| ⌘⌥0–8 | Bullet, H1, H2, H3, text, to-do, numbered, quote, code block |
| ⌘B ⌘I ⌘E ⌘M ⌘K ⌘⇧Y | Bold, italic, code, math, link, highlight |
| ⌘F, ⌘G / ⌘⇧G | Search, next / previous match |
| ⌘⇧D | Today's journal page |
| ⌘⇧C | Calendar (click a day: its journal page; ⌥-click: bullets dated that day) |
| ⌘O | Show / hide completed |
| ⌘Z / ⌘⇧Z | Undo / redo |
| ⌘⇧I / ⌘⇧E | Import (OPML, Markdown, text — Workflowy exports work) / export Markdown |
| ⌘, | Settings: rebind any shortcut, theme (system / light / dark) and accent color, about |

File menu: export as OPML, Markdown or plain text (selected bullets, or the whole page), and the Trash.

Search terms: words, `"phrases"`, `-exclude`, `#tag`, `@person`, `is:done`, `is:open`, `is:todo`, `is:heading`, `is:quote`, `is:code`, `has:note`, `changed:7d` (`m`/`h`/`d`/`w`), `date:2026-11`, `date:today`, `before:2026-12-01`, `after:today`. Save a search from the header to run it later from ⌘P.

## Markup

`**bold**`, `*italic*`, `~~strike~~`, `` `code` ``, `==highlight==`, `{red}colored{/}` (red, orange, yellow, green, blue, purple, gray), `[label](url)`, `[label](flowie:ID)` (link to a bullet), bare URLs, `#tags`, `@mentions`, dates `2026-11-17` (click to find bullets with that date), inline math `$e^{i\pi}+1=0$` and display math `$$\int_0^1 x\,dx$$` (previewed under the line while editing). Markup characters are hidden except on the bullet being edited. `$5 and $6` stays plain text.

## Agents (MCP)

Flowie includes an MCP server so AI agents can read and edit your outline. Allow it in **Settings → Agent** (Off / Read only / Read & write; default read only), then register it, e.g. for Claude Code:

```bash
claude mcp add flowie -- /path/to/Flowie.app/Contents/MacOS/Flowie mcp
```

`flowie mcp` speaks MCP over stdio and forwards to the running app through an owner-only Unix socket, starting Flowie in the background if needed. Edits go through the app, so they appear live, are saved normally, briefly highlight the changed bullets, and each tool call is one ⌘Z undo step; deletions go to the Trash.

Tools: `search`, `read` (Markdown with `[id:N]` per bullet), `starred`, `create` (Markdown list under a parent / after a bullet), `update`, `complete`, `move`, `delete`, `journal`, `open`.

## Layout

```
crates/flowie-core   tree, operations, undo, flattening, search, markup parser, SQLite store (tested)
crates/flowie-math   RaTeX layout + native GPUI painting of formulas (example: math_demo)
crates/flowie-app    the GPUI application
```

```bash
cargo test -p flowie-core
cargo run -p flowie-math --example math_demo
```

## Credits

Icons: [Lucide](https://lucide.dev) (ISC license, `crates/flowie-app/assets/icons/LICENSE`).
