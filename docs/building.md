# Building Flowie from source

Flowie is written in Rust on [GPUI](https://gpui.rs), Zed's GPU UI framework, taken through
[gpui-kit](https://crates.io/crates/gpui-kit)'s snapshots of Zed's GPUI, with
[RaTeX](https://github.com/erweixin/RaTeX) for KaTeX-compatible math. Its controls
(`crates/flowie-app/src/ui`) are its own, drawn in the macOS style.

## Requirements

### macOS
- macOS 13 or later
- Xcode Command Line Tools (`xcode-select --install`); full Xcode isn't needed
- Rust via [rustup](https://rustup.rs); `rust-toolchain.toml` picks the version (1.98.1)

GPUI compiles its Metal shaders when the app starts, so no Metal compiler is needed at
build time. `third_party/gpui-pre-macos` is GPUI's macOS backend with its private AppKit
calls removed, for the App Store; see `FLOWIE-PATCH.md` there.

### Windows
- Windows 10 or 11 (64-bit)
- Visual Studio Build Tools with the **Desktop development with C++** workload (MSVC + Windows SDK)
- Rust via [rustup](https://rustup.rs) (`x86_64-pc-windows-msvc`)

### Linux
Released as a preview: an AppImage and a tar.gz for x86_64, built on Ubuntu 22.04, so they need
glibc 2.35 or later. To build it yourself, besides Rust via rustup, GPUI's X11 and Wayland
backends need these development packages (Debian/Ubuntu names):

```bash
sudo apt install pkg-config libfontconfig-dev libfreetype-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev
```

At run time Flowie needs:

- **A Vulkan driver** (Mesa's is fine).
- **fontconfig and fonts.** The interface font and `monospace` are whatever fontconfig picks.
  Chinese, Japanese and Korean text needs a CJK font such as Noto CJK (`fonts-noto-cjk` on
  Debian/Ubuntu, `google-noto-sans-cjk-fonts` on Fedora, `noto-fonts-cjk` on Arch).
- **xdg-desktop-portal** with a backend, for the open and save dialogs (Import, Export, Insert
  Image). GNOME and KDE come with one; on i3, sway and other bare window managers install
  `xdg-desktop-portal` and `xdg-desktop-portal-gtk`. Without it Flowie says it can't open the
  file dialog.
- **An input method**, for typing Chinese and other languages: fcitx5 or IBus reach Flowie
  through Wayland's text-input-v3 or, on X11, XIM (with `XMODIFIERS=@im=fcitx` or
  `@im=ibus` set as usual). Input methods are among what the preview still needs reports on.

Flowie runs on Wayland when the session has it, else on X11. It asks the window manager for
a title bar; GNOME doesn't draw one for Wayland apps, so there Flowie draws its own in the menu
bar, with the window buttons from GNOME's layout. `FLOWIE_X11=1` runs Flowie on X11 instead
(XWayland under a Wayland session), where the window manager always draws the title bar, but
text can look blurry with fractional scaling.

## Build and run

### macOS

```bash
cargo build --release -p flowie-app
scripts/bundle-macos.sh target/release/flowie Flowie flowie.spokey.cn
open build/Flowie.app
```

`bundle-macos.sh` wraps the binary in an ad-hoc signed `.app` for your own Mac; the version
comes from the workspace `Cargo.toml`. Copy it to `/Applications` if you like.

During development, `cargo run -p flowie-app` starts it without building a bundle.

### Windows

```powershell
cargo build --release -p flowie-app
.\target\release\flowie.exe
```

Release builds embed `assets/AppIcon.ico` and use the Windows GUI subsystem (no console window), while still supporting `flowie.exe mcp` over piped stdio.

### Linux

```bash
cargo build --release -p flowie-app
./target/release/flowie
```

To package it the way releases are:

```bash
scripts/bundle-linux.sh target/release/flowie
```

This makes `build/Flowie-<version>-linux-<arch>.tar.gz` and, with
[appimagetool](https://github.com/AppImage/appimagetool) on `PATH` (or in `APPIMAGETOOL`),
`build/Flowie-<version>-linux-<arch>.AppImage`. Smaller icon sizes need ImageMagick. The tar.gz is
an install prefix: extract it into `~/.local` and refresh the desktop's caches so the menu
entry, icon and `.flowie` file type show up (`~/.local/bin` must be on `PATH`, since the menu
entry runs `flowie`):

```bash
tar -xzf build/Flowie-*-linux-*.tar.gz -C ~/.local --strip-components=1
update-desktop-database ~/.local/share/applications
update-mime-database ~/.local/share/mime
```

Both name the desktop entry, icon and window `flowie.spokey.cn`, so Wayland matches the window
to its icon.

### One instance per outline

Launching Flowie again while it is open brings the open window forward instead of opening
the outline twice; a launch with `--background` (as `flowie mcp` starts it) just exits.
The running app owns a local socket per database (a Unix socket in `/tmp` on macOS, in
`$XDG_RUNTIME_DIR` or `/run/user/<uid>` on Linux, or a named pipe on Windows), which is also what
`flowie mcp` talks to.

## Where data lives

| Build | Outline and settings |
|---|---|
| macOS (source, release zip or Homebrew) | `~/Library/Application Support/Flowie/` (`flowie.sqlite`, `settings.json`) |
| Mac App Store / TestFlight | `~/Library/Containers/flowie.spokey.cn/Data/Library/Application Support/Flowie/` |
| Windows | `%LOCALAPPDATA%\Flowie\` (`flowie.sqlite`, `settings.json`) |
| Linux | `$XDG_DATA_HOME/flowie/` or `~/.local/share/flowie/` |

`FLOWIE_DB=/path/file.sqlite` uses another database. `--stress 50000` opens an in-memory
outline of 50,000 bullets (not saved) and logs timings.

To move an outline from a source build or the release zip into the App Store build, quit Flowie and copy
`flowie.sqlite` (and `settings.json`) into the container folder above. macOS only lets
Terminal.app write into another app's container, so run the copy there.

## Project layout

```
crates/flowie-core   tree, operations, undo, flattening, search, markup parser, SQLite store
crates/flowie-doc    Workspace: opens an outline and commits edits (undo history, writer), no GPUI
crates/flowie-math   RaTeX layout and native GPUI painting of formulas
crates/flowie-app    the GPUI application (UI, settings, i18n, MCP server)
scripts/             app bundle, Linux packages, App Store package, icon
appstore/            Mac App Store release: fastlane, listing, screenshots, sandbox files
website/             flowie's GitHub Pages site (home, privacy policy, terms of use)
docs/                these documents
```

## Checks

The same checks run in CI (`.github/workflows/ci.yml`) on every push and pull request:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# flowie-core's model, without its default `store` and `writer-thread` features, builds for the browser
rustup target add wasm32-unknown-unknown
cargo check -p flowie-core --no-default-features --target wasm32-unknown-unknown
```

Handy extras:

```bash
cargo run -p flowie-math --example math_demo      # formula rendering playground
cargo test -p flowie-app i18n -- --nocapture       # translation coverage report
```

## Mac App Store build

The App Store package is built and uploaded by the maintainer with fastlane; it needs the
maintainer's signing certificates. See [appstore/README.md](../appstore/README.md).
`scripts/package-appstore.sh --local` makes the same sandboxed app, ad-hoc signed, for
testing the sandbox on your own Mac.
