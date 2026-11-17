- Flowie tutorial — start here {{#tutorial}} {{star}}
  Everything Flowie does, one lesson per heading. Tick each box as you try it. To open a lesson, hover it and click the arrow on its left, or press ⌘↓. When you're done, delete this page — Help → Flowie Tutorial brings it back.
  - ## 1 · Writing
    - [ ] Click at the end of this line and press Enter to add a bullet
    - [ ] Press Tab to indent a bullet, Shift-Tab to outdent it
    - [ ] Press Backspace at the start of a bullet to merge it into the one above
    - [ ] Press Shift-Enter to write a note under a bullet
      Like this one. Notes hold longer text; Shift-Enter again goes back to the bullet.
    - [ ] Press ⌘Enter to complete a bullet, ⌘O to show or hide completed ones
      - ~~This bullet is already done~~
    - [ ] Move a bullet with ⌘⇧↑ / ⌘⇧↓, or drag it by its dot
    - [ ] Press Esc to select whole bullets, then Shift-↑ / Shift-↓ to select more
      Selected bullets can be indented, moved, copied, cut, completed or deleted together. You can also drag across rows to select them.
    - [ ] Undo anything with ⌘Z, redo with ⌘⇧Z
    - Everything is saved automatically, on your Mac only
  - ## 2 · Pages and zooming {{#zoom}} {{collapsed}}
    - [ ] Every bullet is a page: click a bullet's dot (or press ⌘]) to zoom into it
    - [ ] Press ⌘[ to zoom out, ⌘⇧H to go Home; the breadcrumbs at the top lead back too
    - [ ] Use the mouse back / forward buttons or ⌘⌥← / ⌘⌥→ to step through pages you visited
    - [ ] Press ⌘↑ / ⌘↓ to collapse and expand a bullet
    - [ ] Press ⌃1 – ⌃4 to show 1–4 levels, ⌃0 to expand everything
    - A collapsed bullet — expand me {{collapsed}}
      - Hidden until you expanded it
        - Bullets nest as deep as you like
  - ## 3 · Block types {{collapsed}}
    - Type / at the start of a bullet for the command menu, or press ⌘⌥0–8
    - # Heading 1
    - ## Heading 2
    - ### Heading 3
    - Plain text, without a bullet dot {{text}}
    - [ ] A to-do — click the box or press ⌘Enter
    - [x] A finished to-do
    1. Numbered lists
    2. count by themselves
    - > A quote
    - fn main() { println!("A code block"); } {{code}}
  - ## 4 · Text styles {{collapsed}}
    - **Bold** ⌘B · *italic* ⌘I · ~~strikethrough~~ · `code` ⌘E · ==highlight== ⌘⇧Y
    - Colors: {red}red{/} {orange}orange{/} {yellow}yellow{/} {green}green{/} {blue}blue{/} {purple}purple{/} {gray}gray{/}
    - [ ] Select some text and right-click it for a style menu
    - [ ] Click this line: markup characters appear only on the bullet you're editing
    - Links: [label](https://en.wikipedia.org/wiki/Outliner) with ⌘K, or paste a bare URL like https://example.com
    - Dates like {{today}} are clickable: they find every bullet with that date
  - ## 5 · Math {{collapsed}}
    - Inline math with `$…$` or ⌘M: $e^{i\pi}+1=0$
    - Display math with `$$…$$`
      - $$\int_0^1 x^2\,dx=\frac{1}{3}$$
    - [ ] Click a formula to edit it — a live preview shows under the line
    - Prices like $5 and $6 stay plain text
  - ## 6 · Connecting ideas {{collapsed}}
    - Tags #idea and mentions @alex: type # or @ for completion; click one to search for it
    - [ ] Type `[[` or press ⌘⇧K to link to any bullet, like [Pages and zooming](flowie:@zoom)
      Open that lesson: this link shows at the bottom of its page as a backlink.
    - Mirrors (⌘⌥M) show one bullet in several places
      - I'm the original — edit me and watch the copy below {{#mirror}}
      - {{mirror mirror}}
    - [ ] Press ⌘⇧M to move bullets to another page, or to a new page on Home
    - [ ] Press ⌘⇧S to star a page; this tutorial is starred
  - ## 7 · Finding things {{collapsed}}
    - [ ] Press ⌘P to jump to starred pages, saved searches or any bullet
      Type a name that doesn't exist yet to create a new page on Home.
    - [ ] Press ⌘F to search, ⌘G / ⌘⇧G for the next / previous match
    - Search filters: "exact phrase", -exclude, #tag, @person, is:todo, is:open, is:done, is:heading, has:note, changed:7d, date:today, before:2026-12-01, after:today
    - [ ] Save a search from its header; "is:todo is:open" is already saved — find it in ⌘P
  - ## 8 · Journal and calendar {{collapsed}}
    - [ ] Press ⌘⇧D for today's journal page
    - [ ] Press ⌘⇧C for the calendar: click a day for its journal page, ⌥-click for bullets dated that day
  - ## 9 · Import, export and Trash {{collapsed}}
    - [ ] Import OPML, Markdown or text with ⌘⇧I — Workflowy exports work
    - Export the page or the selected bullets as Markdown (⌘⇧E), OPML or plain text from the File menu
    - Deleted bullets (⌘⇧Backspace) stay in the Trash for 30 days: File → Trash
  - ## 10 · Make it yours {{collapsed}}
    - [ ] Press ⌘, for Settings: theme, accent color, language
    - [ ] Click any shortcut in Settings to change it; + adds another key or a mouse side button
    - AI assistants can read your outline — and edit it if you allow it — through Flowie's MCP server: Settings → Agent
      For Claude Code, run this in a terminal:
      - claude mcp add flowie -- /Applications/Flowie.app/Contents/MacOS/Flowie mcp {{code}}
    - Ideas or problems? Help → Send Feedback…
    - [ ] Done? Select this page's bullet with Esc and press ⌘⇧Backspace to delete it
