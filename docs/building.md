# Building Flowie from source

Flowie is written in Rust on [GPUI](https://gpui.rs), Zed's GPU UI framework, with
[RaTeX](https://github.com/erweixin/RaTeX) for KaTeX-compatible math.

## Requirements

- macOS 13 or later
- Xcode Command Line Tools (`xcode-select --install`); full Xcode isn't needed
- Rust stable via [rustup](https://rustup.rs)

GPUI's `runtime_shaders` feature compiles the Metal shaders when the app starts, so no
Metal compiler is needed at build time.

## Build and run

```bash
cargo build --release -p flowie-app
scripts/bundle-macos.sh target/release/flowie Flowie flowie.spokey.cn
open build/Flowie.app
```

`bundle-macos.sh` wraps the binary in an ad-hoc signed `.app` for your own Mac; the version
comes from the workspace `Cargo.toml`. Copy it to `/Applications` if you like.

During development, `cargo run -p flowie-app` starts it without building a bundle.

## Where data lives

| Build | Outline and settings |
|---|---|
| From source, the release zip or Homebrew | `~/Library/Application Support/Flowie/` (`flowie.sqlite`, `settings.json`) |
| Mac App Store / TestFlight | `~/Library/Containers/flowie.spokey.cn/Data/Library/Application Support/Flowie/` |

`FLOWIE_DB=/path/file.sqlite` uses another database. `--stress 50000` opens an in-memory
outline of 50,000 bullets (not saved) and logs timings.

To move an outline from a source build or the release zip into the App Store build, quit Flowie and copy
`flowie.sqlite` (and `settings.json`) into the container folder above. macOS only lets
Terminal.app write into another app's container, so run the copy there.

## Project layout

```
crates/flowie-core   tree, operations, undo, flattening, search, markup parser, SQLite store
crates/flowie-math   RaTeX layout and native GPUI painting of formulas
crates/flowie-app    the GPUI application (UI, settings, i18n, MCP server)
scripts/             app bundle, App Store package, icon
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
