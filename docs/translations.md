# Translating Flowie

UI text lives in `crates/flowie-app/locales/<code>.json` (nested JSON; a key is its dotted path, e.g. `menu.file.title`). `en.json` is the source and fallback. To add a language, drop in a new file with `meta.name` set to the language's own name; build.rs picks it up and it appears in Settings → Appearance → Language. Messages take `{name}` placeholders and plural objects (`{"one": …, "other": …}`, chosen by `count`). In code: `t!("menu.file.title")`, `t!("toast.moved", count = n, target = name)`.

```bash
cargo test -p flowie-app i18n -- --nocapture   # coverage report + key/placeholder checks
```

The iOS app's strings (`NSLocalizedString` with a comment, in `apple/`) are added by hand, in English and Simplified Chinese, to `apple/FlowieApp/Resources/Localizable.xcstrings`; App Shortcuts phrases live in `apple/FlowieApp/Resources/<locale>.lproj/AppShortcuts.strings`. Xcode's string extraction is off for the app's targets (`LOCALIZATION_EXPORT_SUPPORTED` and `SWIFT_EMIT_LOC_STRINGS` in `Project.swift`): most strings live in the FlowieKit package, which it doesn't see, so it marked them stale and rewrote the whole catalog after every build. After editing the catalog, run `swift scripts/format-xcstrings.swift`: it writes it in Xcode's own format, with every string marked `manual`, so an Xcode build leaves it unchanged (`--check` only reports).

Pull requests with new languages or fixes are welcome — see [CONTRIBUTING.md](../CONTRIBUTING.md).
The App Store listing text is translated separately, in `appstore/fastlane/macos/metadata/<locale>/` (Mac) and `appstore/fastlane/ios/metadata/<locale>/` (iPhone and iPad).
