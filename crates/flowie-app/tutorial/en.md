- Flowie tutorial — start here {{#tutorial}} {{star}}
  Three parts: "Getting started" takes five minutes; "Examples" are three common setups you can turn into your own; "Manual" covers everything, to look up when you need it. Bullets with an arrow are collapsed: click the arrow, or put the caret on the bullet and press {{key:expand}}. When you're done, delete this page — Help → Flowie Tutorial brings it back.
  - # Getting started
    - [ ] Click at the end of this line and press Enter to add a bullet; Tab indents, Shift-Tab outdents
    - [ ] Press Shift-Enter to write a note under a bullet
      Like this one. Notes hold longer text; Shift-Enter again goes back to the bullet.
    - [ ] Press {{key:complete}} to tick off a to-do, like this one
    - [ ] Click a bullet's dot to zoom into it: every bullet is a page. Press {{key:zoom-out}} or click the path at the top to go back
    - [ ] Press {{key:collapse}} / {{key:expand}} to collapse and expand — "Examples" below keeps three pages folded away
    - [ ] Press {{key:jump}} to jump to any page, {{key:find}} to search
    - Everything is saved automatically, on this computer; undo anything with {{key:undo}}
  - # Examples
    Click a dot to open one, then make it your own.
    - Project: website redesign {{collapsed}} {{#project}}
      Goal: ship the new home page by the end of the month. Tasks, meetings and open questions on one page.
      - ## Tasks
        - [x] Pick colors and fonts #design
        - [ ] Rebuild the home page @sam {{today}} {{#task}}
          - [ ] Hero image and tagline
          - [ ] Mobile layout
        - [ ] Add analytics #dev
      - ## Meeting notes
        - {{today}} weekly sync
          - Use a product screenshot as the hero, see [Rebuild the home page](flowie:@task)
          - ==Final design by next Wednesday==
      - ## Questions
        - Should old links redirect? #to-discuss
    - To-do list {{collapsed}}
      Search `is:todo is:open` to list every open to-do on every page; that search is already saved, so you'll find it in {{key:jump}}.
      - ## Today
        - [ ] Reply to the client #work
        - [ ] Pick up the parcel #home
      - ## This week
        - [ ] Draft the quarterly report #work {{today}}
        - [ ] Book a checkup #home
      - ## Someday {{collapsed}}
        - [ ] Learn a new language
      - [x] Finished to-dos fade; press {{key:toggle-completed}} to hide them
    - Reading notes: Principles {{collapsed}}
      By Ray Dalio. Notes as you read: chapters are bullets, quotes and thoughts each get a level.
      - ## Quotes
        - > Pain + Reflection = Progress
        - > Distinguish your goals from your desires
      - ## My thoughts
        - Write a short review after every mistake #idea
          A mirror can put it in the To-do list too; edits show up in both places.
        - Reviews could go in the [website redesign](flowie:@project) meeting notes
      - ## Highlights {{collapsed}}
        - **Principles** are fundamental truths that guide how you act
  - # Manual
    Everything Flowie does, one lesson per heading. Tick each box as you try it.
    - ## Writing {{collapsed}}
      - [ ] Press Backspace at the start of a bullet to merge it into the one above
      - [ ] Press {{key:toggle-completed}} to show or hide completed bullets
        - ~~This bullet is already done~~
      - [ ] Move a bullet with {{key:move-up}} / {{key:move-down}}, or drag it by its dot
      - [ ] Press Esc to select whole bullets, then Shift-↑ / Shift-↓ to select more
        Selected bullets can be indented, moved, copied, cut, completed or deleted together. You can also drag across rows to select them.
      - [ ] Press {{key:redo}} to redo what you undid
    - ## Pages and zooming {{#zoom}} {{collapsed}}
      - [ ] Press {{key:zoom-in}} to zoom into the bullet with the caret, {{key:home-page}} to go Home
      - [ ] Use the mouse back / forward buttons or {{key:page-back}} / {{key:page-forward}} to step through pages you visited
      - [ ] Press {{key:level-1}} – {{key:level-4}} to show 1–4 levels, {{key:expand-all}} to expand everything
      - As you scroll down, the page title tucks into the top bar and the path shows where you are
      - A collapsed bullet — expand me {{collapsed}}
        - Hidden until you expanded it
          - Bullets nest as deep as you like
    - ## Block types {{collapsed}}
      - Type / at the start of a bullet for the command menu, or press {{key:kind-bullet}}–{{key:kind-code}}
      - # Heading 1
      - ## Heading 2
      - ### Heading 3
      - Plain text, without a bullet dot {{text}}
      - [ ] A to-do — click the box or press {{key:complete}}
      1. Numbered lists
      2. count by themselves
      - > A quote
      - fn main() { println!("A code block"); } {{code}}
    - ## Text styles {{collapsed}}
      - **Bold** {{key:bold}} · *italic* {{key:italic}} · ~~strikethrough~~ · `code` {{key:code}} · ==highlight== {{key:highlight}}
      - Colors: {red}red{/} {orange}orange{/} {yellow}yellow{/} {green}green{/} {blue}blue{/} {purple}purple{/} {gray}gray{/}
      - [ ] Select some text and right-click it for a style menu
      - [ ] Click this line: markup characters appear only on the bullet you're editing
      - Links: [label](https://en.wikipedia.org/wiki/Outliner) with {{key:link}}, or paste a bare URL like https://example.com
      - Dates like {{today}} are clickable: they find every bullet with that date
    - ## Math {{collapsed}}
      - Inline math with `$…$` or {{key:math}}: $e^{i\pi}+1=0$
      - Display math with `$$…$$`
        - $$\int_0^1 x^2\,dx=\frac{1}{3}$$
      - [ ] Click a formula to edit it — a live preview shows under the line
      - Prices like $5 and $6 stay plain text
    - ## Images {{collapsed}}
      - Paste an image, drop one on a bullet or use {{key:insert-image}}; the bullet's text is its caption
      - Flowie's icon {{image icon}}
      - [ ] Select the image bullet above and press Space to see it full size
    - ## Connecting ideas {{collapsed}}
      - Tags #idea and mentions @alex: type # or @ for completion; click one to search for it
      - [ ] Type `[[` or press {{key:link-to}} to link to any bullet, like [Pages and zooming](flowie:@zoom)
        Open that lesson: this link shows at the bottom of its page as a backlink.
      - Mirrors ({{key:mirror}}) show one bullet in several places
        - I'm the original — edit me and watch the copy below {{#mirror}}
        - {{mirror mirror}}
      - [ ] Press {{key:move-to}} to move bullets to another page, or to a new page on Home
      - [ ] Press {{key:star}} to star a page; this tutorial is starred
    - ## Finding things {{collapsed}}
      - [ ] In {{key:jump}}, type a name that doesn't exist yet to create a new page on Home
      - [ ] While searching, press {{key:next-match}} / {{key:prev-match}} for the next / previous match
      - Search filters: "exact phrase", -exclude, #tag, @person, is:todo, is:open, is:done, is:heading, has:note, changed:7d, date:today, before:2026-12-01, after:today
      - [ ] Save a search from its header; saved searches show up in {{key:jump}}
    - ## Journal and calendar {{collapsed}}
      - [ ] Press {{key:today}} for today's journal page
      - [ ] Press {{key:calendar}} for the calendar: click a day for its journal page, {{mod:alt}}-click for bullets dated that day
    - ## Import, export, sharing and Trash {{collapsed}}
      - [ ] Import OPML, Markdown or text with {{key:import}} — Workflowy exports work
      - Export the page or the selected bullets as Markdown ({{key:export-md}}), OPML or plain text from the File menu
      - To send bullets to another Flowie user, choose Export as Package… in a bullet's menu (or type /export): you get one `.flowie` file with the bullets and everything under them
      - Open a `.flowie` file someone sent you to import it; new arrivals show a dot until you open them (search `is:new` to find them)
      - Deleted bullets ({{key:delete-bullet}}) stay in the Trash for 30 days: File → Trash
    - ## Make it yours, and AI {{collapsed}}
      - [ ] Press {{key:settings}} for Settings: theme, accent color, content width, language
      - [ ] In a wide window, press {{key:content-width}} to switch the text column between narrow, medium and full width
      - [ ] Click any shortcut in Settings to change it; + adds another key or a mouse side button
      - AI assistants can read your outline — and edit it if you allow it — through Flowie's MCP server: Settings → Agent
        For Claude Code, run this in a terminal:
        - {{mcp-command}} {{code}}
      - Help → What's New lists what changed in each version; ideas or problems? Help → Send Feedback…
