# Translating Flowie

UI text lives in `crates/flowie-app/locales/<code>.json` (nested JSON; a key is its dotted path, e.g. `menu.file.title`). `en.json` is the source and fallback. To add a language, drop in a new file with `meta.name` set to the language's own name; build.rs picks it up and it appears in Settings → Appearance → Language. Messages take `{name}` placeholders and plural objects (`{"one": …, "other": …}`, chosen by `count`). In code: `t!("menu.file.title")`, `t!("toast.moved", count = n, target = name)`.

```bash
cargo test -p flowie-app i18n -- --nocapture   # coverage report + key/placeholder checks
```

Pull requests with new languages or fixes are welcome — see [CONTRIBUTING.md](../CONTRIBUTING.md).
The App Store listing text is translated separately, in `appstore/fastlane/metadata/<locale>/`.
