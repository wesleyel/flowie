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
- `FLOWIE_THEME=dark|light` forces a theme; otherwise it follows the system.

## Keys

| Key | Action |
|---|---|
| Enter | New bullet (splits at the caret) |
| Shift-Enter | Edit / leave the note |
| Tab / Shift-Tab | Indent / outdent |
| Backspace at start | Merge into the bullet above |
| ⌘Enter | Complete |
| ⌘↑ / ⌘↓ | Collapse / expand |
| ⌘] / ⌘[ / ⌘⇧H | Zoom in / out / home (or click a bullet / breadcrumb) |
| ⌘⇧↑ / ⌘⇧↓ | Move bullet |
| ⌘⇧Backspace | Delete bullet |
| Esc | Select whole bullets; Shift-↑/↓ extends; Tab, ⌘C, ⌘X, Backspace act on them |
| ⌘B ⌘I ⌘E ⌘M ⌘K | Bold, italic, code, math, link |
| ⌘F | Search (`#tag`, `@person`, `"phrase"`, `-exclude`, `is:done`, `is:todo`) |
| ⌘O | Show / hide completed |
| ⌘Z / ⌘⇧Z | Undo / redo |

Paste indented text (tabs or spaces, optional `- ` bullets) to create a subtree; ⌘C on selected bullets copies them the same way. Drag a bullet to move it; the horizontal position picks the depth.

## Markup

`**bold**`, `*italic*`, `~~strike~~`, `` `code` ``, `[label](url)`, bare URLs, `#tags`, `@mentions`, inline math `$e^{i\pi}+1=0$` and display math `$$\int_0^1 x\,dx$$`. Markup characters are hidden except on the bullet being edited. `$5 and $6` stays plain text.

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
