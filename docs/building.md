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
build time. iCloud sync (`crates/flowie-cloudkit`) is Swift, compiled with the Command Line
Tools' `swiftc`; `FLOWIE_CLOUDKIT=0` builds without it. Only the Mac App Store package, signed
with the iCloud entitlements, can use it (see [sync.md](sync.md#icloud)). `third_party/gpui-pre-macos` is GPUI's macOS backend with its private AppKit
calls removed, for the App Store; see `FLOWIE-PATCH.md` there.

### Windows
- The prebuilt Windows x64 package is a preview.
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

### iOS (in development)

The iOS app (`apple/FlowieApp`) is FlowieUI's SwiftUI views over the Rust core through
`flowie-ffi`. It needs Xcode and [Tuist](https://tuist.dev). Simulator builds need no signing:

```bash
scripts/build-ios-ffi.sh                  # build/ios/FlowieFFI.xcframework + Swift bindings
cd apple/FlowieApp && tuist generate --no-open
xcodebuild -workspace Flowie.xcworkspace -scheme Flowie \
    -destination 'platform=iOS Simulator,name=iPhone Air' -derivedDataPath ../../build/ios/DerivedData build
xcrun simctl install booted ../../build/ios/DerivedData/Build/Products/Debug-iphonesimulator/Flowie.app
xcrun simctl launch booted flowie.spokey.cn
```

Run `scripts/build-ios-ffi.sh` again after changing Rust code. The app needs iOS 26 or later. The
outline is kept in the App Group container (`group.flowie.spokey.cn`), in
`Library/Application Support/Flowie/flowie.sqlite` (`xcrun simctl get_app_container booted
flowie.spokey.cn groups` shows where); one left in the app's own Application Support by an
earlier build moves there on launch. To check the launch rule (back to the last page within the
hour, otherwise today's journal), a debug build takes another time: `xcrun simctl launch booted
flowie.spokey.cn -FlowieNow 2026-11-28T09:00:00+08:00`.

`Project.swift` signs automatically with team `5D26F33GNM` (the app `flowie.spokey.cn`, its
Control Center extension `flowie.spokey.cn.JournalControl` and its share extension
`flowie.spokey.cn.Share`, which shares the App Group with the app). The share extension never
opens the outline: it leaves each share as a JSON file in the group container's `inbox/`, which
the app adds to the journal when it starts or comes back (`FlowieInbox`); to try the import
without the share sheet, put a file there by hand. Debug builds use CloudKit's
Development environment and the development push service, Release builds (TestFlight, the App
Store) Production. To check a Release build in the Simulator, which leaves out the self-hosted
sync settings, add `-configuration Release` to the `xcodebuild` line above; the app is then in
`Release-iphonesimulator`. The app icon comes from `swift scripts/make-icon.swift --ios
apple/FlowieApp/Resources/Assets.xcassets/AppIcon.appiconset`.

The App Store screenshots (iPhone 6.9" in portrait and iPad 13" in landscape, English and
Simplified Chinese) come from one command:

```bash
scripts/ios-screenshots.py
```

```bash
scripts/ios-screenshots.py --device iphone --locale zh-Hans --only 03 search --no-build
```

It builds the Debug app for the Simulator, writes a demo outline per language
(`cargo run -p flowie-ffi --example demo_outline`, where the demo text is), and takes each shot
with a fresh copy of it, light appearance and a 9:41 status bar. Each device and language has a
Simulator of its own (`Flowie Screenshots iPhone en-US` and so on, made on first use, set to that
language), and they run at once (`--jobs N` for fewer); an unchanged build isn't reinstalled. simctl can't
turn a Simulator, so the `ScreenshotTests` UI-test target (only that) turns the iPad to
landscape. Launch arguments open each state: the launch rule's keys for the page, and, in debug
builds, `-FlowieSearch <query>` and `-FlowieCapture <text>` (`ScreenshotArguments.swift`). Captures go to
`appstore/fastlane/ios/screenshots-source/<locale>/`. Home also supplies the website’s device images.
The same command frames the captures with localized headlines in `appstore/fastlane/ios/screenshots/<locale>/`.
To change just the artwork, run `scripts/ios-screenshots.py --frame-only` on macOS. The
`fastlane ios screenshots` lane uploads the framed PNGs. [Submission checklist](../appstore/README.md#ios-submission-checklist)

For the App Store:

```bash
scripts/package-ios.sh             # build/ios/appstore/Flowie-<version>-<build>.ipa
scripts/package-ios.sh --upload    # the same, then upload it to App Store Connect (TestFlight)
```

It builds the Rust core, archives the Release configuration with the desktop's version (from
`Cargo.toml`) and build number (the git commit count, or `FLOWIE_BUILD`), exports an App Store
`.ipa` and checks it (production push and iCloud, versions of the app and the extensions, the App
Group, privacy manifest, icon). Signing is automatic: with the App Store Connect API key in
`appstore/fastlane/.env` (see `appstore/README.md`), xcodebuild makes the profiles itself;
otherwise it uses the account signed in to Xcode. `--upload` needs a clean working tree.

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

### Database format upgrades

The database's format is its `PRAGMA user_version`. When a newer Flowie opens a database from
1.0 or later, it first copies it to `flowie.sqlite.backup-v<old format>-<timestamp>` beside it,
then upgrades it in one transaction; it keeps the three newest backups. To go back, quit Flowie
and rename a backup to `flowie.sqlite` (with an older Flowie, the one that wrote that format). If
the backup or the upgrade fails, the database is left as it was and Flowie opens an empty window
that says so. A database from a newer Flowie is never changed: Flowie asks you to update.
A database from a 1.0 release candidate isn't upgraded but kept as
`flowie.sqlite.old-<timestamp>`, with its image folder at `flowie.sqlite.old-<timestamp>.images`.
Flowie starts a new database.

To change the format, change `SCHEMA` and append a migration to `MIGRATIONS` in
`crates/flowie-core/src/store/mod.rs`; the steps and the tests a migration needs are in that
module's docs.

To move an outline from a source build or the release zip into the App Store build, quit Flowie and copy
`flowie.sqlite` (and `settings.json`) into the container folder above. macOS only lets
Terminal.app write into another app's container, so run the copy there.

## Project layout

See [workspace.md](workspace.md) for the full crate breakdown (`crates/*`), desktop module layout
(`crates/flowie-app/src`), Apple (`apple/`) and browser (`web/`) directories, `scripts/`,
`appstore/`, `website/`, and the edit/commit/render data flow (and [wiki.md](wiki.md) for the
local `wiki/` clone).

## Checks

CI (`.github/workflows/ci.yml`) checks pull requests and pushes to `main` with the same macOS, Windows and Linux matrix. It also checks both changelogs and runs `cargo deny check`. It does not create release archives.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# flowie-core's model, without its default `store` and `writer-thread` features, builds for the browser
rustup target add wasm32-unknown-unknown
cargo check -p flowie-core --no-default-features --target wasm32-unknown-unknown
# the browser version: flowie-wasm into web/pkg, then web/dist (Node 22, npm ci, type check, Vite)
rustup target add wasm32-unknown-unknown && rustup component add llvm-tools
scripts/build-web.sh
```

`npm run check` and `npm run build` in `web/` type-check and bundle against an existing `web/pkg`;
`npm run dev` serves it with Vite, passing `/v1/` to the `flowie-server` at `FLOWIE_SERVER`
(default `http://127.0.0.1:8787`).

Handy extras:

```bash
cargo run -p flowie-math --example math_demo      # formula rendering playground
cargo test -p flowie-app i18n -- --nocapture       # translation coverage report
```

## Desktop releases and website

- **Test packages:** manually run **Desktop Packages** (`release.yml`) against a branch or tag. Download the seven-day workflow artifacts. This does not publish a release.
- **Releases:** push a `vX.Y.Z` or `vX.Y.Z-rc.N` tag matching the workspace version and both changelogs. The same package jobs build and smoke-test the macOS universal zip and the Windows x64 and Linux x86_64 preview packages. When all succeed, the workflow uploads the archives and SHA-256 files to a draft, then publishes it. A rerun does not overwrite an already published version. RCs are GitHub prereleases; Homebrew follows the newest published version, including RCs.
- **Website:** changes under `website/` are built and checked on pull requests and deployed to GitHub Pages from `main`. The public [installation guide](https://wesleyel.github.io/flowie/download.html) covers all three platforms.

## Mac App Store build

The App Store package is built and uploaded by the maintainer with fastlane; it needs the
maintainer's signing certificates. See [appstore/README.md](../appstore/README.md).
`scripts/package-appstore.sh --local` makes the same sandboxed app, ad-hoc signed, for
testing the sandbox on your own Mac.
