# gpui-pre-macos 0.3.12, patched for the Mac App Store

This is [gpui-pre-macos 0.3.12](https://crates.io/crates/gpui-pre-macos/0.3.12) from crates.io
(Apache-2.0, see `LICENSE-APACHE`), Zed's `gpui_macos` crate as published for gpui-kit 0.10.0.
It is used through `[patch.crates-io]` in the workspace `Cargo.toml`.

App Review rejects binaries that reference non-public APIs (guideline 2.5.1; build 39 was
rejected for `_CGSSetWindowBackgroundBlurRadius`, which this version no longer has). The changes,
all in `src/window.rs` and each marked `Flowie patch`, remove the private Objective-C names that
remained:

- `_opaqueRectForWindowMoveWhenInTitlebar`: override not registered. It only matters with
  `WindowOptions::app_owns_titlebar_drag`, which Flowie doesn't set.
- `_windowRestorationOptions`: override not registered. On macOS 15+ the window may reopen on
  the current Space instead of its original one.
- `_zoomFill:`: the "Fill" title-bar double-click action uses the public `zoom:`.
- `NSCursor _windowResizeNorthWestSouthEastCursor` / `_windowResizeNorthEastSouthWestCursor`:
  diagonal resize cursor styles fall back to the arrow. Flowie never sets them.
- `CAChameleonLayer`: the blurred-background view no longer hides it. Flowie doesn't use
  blurred windows.

The top of `src/window.rs` allows the dead code this leaves. Reapply when upgrading gpui-kit, or drop this
copy once upstream no longer needs it. To check a build, both of these must print nothing:

    nm -u <binary> | grep -E 'CGS|SLS'
    strings <binary> | grep -E '^_(opaqueRect|windowRestoration|zoomFill|windowResize)|CAChameleon'
