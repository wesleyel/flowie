# AI assistants (MCP)

Flowie includes an MCP server so AI agents can read and edit your outline. Allow it in **Settings → Agent** (Off / Read only / Read & write; default read only), then register it, e.g. for Claude Code:

```bash
claude mcp add flowie -- /Applications/Flowie.app/Contents/MacOS/Flowie mcp
```

`flowie mcp` speaks MCP over stdio and forwards to the running app through an owner-only Unix socket, starting Flowie in the background if needed. Edits go through the app, so they appear live, are saved normally, briefly highlight the changed bullets, optionally focus the edited location (**Settings → Agent → Focus edited bullets**), and each tool call is one ⌘Z undo step; deletions go to the Trash. To change many bullets at once, `batch` applies a list of `create` / `journal` / `update` / `complete` / `move` / `delete` ops all or nothing, as one undo step; a `create` op can carry a `key` that later ops use as `"$key"` to refer to the bullet it created.

Tools: `current` (the page, active search filter, and focused or selected bullet(s) currently open in Flowie), `search`, `read` (Markdown with a short id like `[id:3f9a1c]` per bullet; tools accept any unique prefix of 4 or more hex digits, as git does for commits, plus `"home"`, `"current"`, `"today"` or a `YYYY-MM-DD` date), `starred`, `create` (Markdown list under a parent / after a bullet), `update`, `complete`, `move`, `delete`, `attach_image`, `journal`, `batch`, `open`. `read` and `move` also take `ids` for several bullets in one call.

For a copy built from source, use the path of your bundle, e.g. `build/Flowie.app/Contents/MacOS/Flowie mcp`.
The Mac App Store build is sandboxed; `Flowie mcp` started from a terminal still finds its
container and talks to the running app.

## Stability and deprecation

Tool names, required parameters, and response shapes stay backward-compatible across minor releases within a major version; new tools and optional parameters may be added in minor releases. When a tool or parameter needs to be retired or replaced, it is first marked **Deprecated** (in its tool description and in this document, noting the replacement) and removed no earlier than the next minor release. See [docs/versioning.md](versioning.md).
