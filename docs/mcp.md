# AI assistants (MCP)

Flowie includes an MCP server so AI agents can read and edit your outline. Allow it in **Settings → Agent** (Off / Read only / Read & write; default read only), then register it, e.g. for Claude Code:

```bash
claude mcp add flowie -- /Applications/Flowie.app/Contents/MacOS/Flowie mcp
```

`flowie mcp` speaks MCP over stdio and forwards to the running app through an owner-only Unix socket, starting Flowie in the background if needed. Edits go through the app, so they appear live, are saved normally, briefly highlight the changed bullets, and each tool call is one ⌘Z undo step; deletions go to the Trash.

Tools: `search`, `read` (Markdown with `[id:N]` per bullet), `starred`, `create` (Markdown list under a parent / after a bullet), `update`, `complete`, `move`, `delete`, `journal`, `open`.

For a copy built from source, use the path of your bundle, e.g. `build/Flowie.app/Contents/MacOS/Flowie mcp`.
The Mac App Store build is sandboxed; `Flowie mcp` started from a terminal still finds its
container and talks to the running app.
