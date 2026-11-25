# Using Flowie

The first launch opens a tutorial outline in three parts: Getting started (a few minutes), Examples
(a project, a to-do list and reading notes to adapt) and a Manual covering every feature, its
lessons collapsed. Help → Flowie Tutorial adds a fresh copy. Its text lives in
`crates/flowie-app/tutorial/<locale>.md`.

Settings (⌘,) hold the theme (system / light / dark), accent color, language and shortcuts.
Click a shortcut to replace it, use `+` to add another keyboard or mouse side-button binding,
and press Backspace while recording to remove that binding.

## Keyboard shortcuts

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
| ⌘P | Go to: starred pages, saved searches, any bullet; a name that matches no bullet offers a new page on Home |
| ⌘⇧S | Star / unstar the page |
| ⌘⇧M | Move to… (an existing bullet or a new page on Home) |
| ⌘⌥M | Mirror a bullet here (live copy: edits show in both places) |
| ⌘⇧K or `[[` | Link to another bullet, or to a new page on Home (backlinks show at the bottom of its page) |
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
| ⌘, | Settings: rebind any shortcut, theme (system / light / dark), accent color, language, about |

## Import, export and Trash

File menu: export as OPML, Markdown or plain text (selected bullets, or the whole page), and the Trash.

Select a `.flowie` share package on its own to preview it; OPML, Markdown and text files can be
imported together. **Import a Copy** creates independent bullets with new IDs. **Update** merges
changes into a previous import. If the outline changes while the preview is open, review the
refreshed comparison before confirming. If an attachment cannot be saved, the import stays open
and no bullets are changed.

Flowie writes [OPML 2.0](https://opml.org/spec2.opml). Every `outline` has a standard `text`
attribute. A mirror's `text` is a snapshot of its source text, so any OPML reader can display it.
The `flowie` namespace (`https://github.com/wesleyel/flowie/ns/opml/1`) adds `id` for the bullet's
UUID and `mirrorOf` for its source UUID. A mirror is a leaf in the file. Flowie resolves its
source in the imported document first, then in the destination outline. If the source is missing,
the snapshot becomes a regular item. On an ID collision, Flowie gives the imported item a fresh ID
and redirects its document's internal links and mirrors to the imported copy.

## Search

Search terms: words, `"phrases"`, `-exclude`, `#tag`, `@person`, `is:done`, `is:open`, `is:todo`, `is:heading`, `is:quote`, `is:code`, `has:note`, `changed:7d` (`m`/`h`/`d`/`w`), `date:2026-11`, `date:today`, `before:2026-12-01`, `after:today`. Save a search from the header to run it later from ⌘P.

## Markup

`**bold**`, `*italic*`, `~~strike~~`, `` `code` ``, `==highlight==`, `{red}colored{/}` (red, orange, yellow, green, blue, purple, gray), `[label](url)`, `[label](flowie:ID)` (link to a bullet), bare URLs, `#tags`, `@mentions`, dates `2026-11-17` (click to find bullets with that date), inline math `$e^{i\pi}+1=0$` and display math `$$\int_0^1 x\,dx$$` (previewed under the line while editing). Markup characters are hidden except on the bullet being edited. `$5 and $6` stays plain text.
