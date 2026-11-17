# Mac App Store release

Everything in the repo is ready; the steps marked **(you)** need your Apple
Developer account and can't be scripted from here.

## What the build does

`scripts/package-appstore.sh` produces `build/appstore/Flowie-<version>-<build>.pkg`:

- Universal binary (arm64 + x86_64), built in its own target dir `target/appstore`.
- Info.plist: version from `Cargo.toml`, build number = git commit count
  (`FLOWIE_BUILD` overrides), category Productivity, minimum macOS 13.0,
  `ITSAppUsesNonExemptEncryption = false`, copyright, en + zh-Hans localizations.
- App Sandbox with read/write access only to files picked in Open / Save
  panels (import, export) — `packaging/macos/Flowie.entitlements`.
- Privacy manifest (`PrivacyInfo.xcprivacy`): no tracking, no data collected;
  required-reason APIs: file timestamps (C617.1), user defaults (CA92.1).
- `Contents/Resources/ThirdPartyNotices.txt`: licenses of the bundled open-source
  components (Rust crates via cargo-about, KaTeX fonts OFL, Lucide icons).
  Their licenses (MIT, Apache-2.0, BSD, ISC, OFL…) require shipping these
  notices with the binary; the file isn't shown anywhere in the UI.
- Signed with your **Apple Distribution** identity and the provisioning profile,
  then wrapped in a `.pkg` signed with **3rd Party Mac Developer Installer**.

`scripts/package-appstore.sh --local` makes the same app, sandboxed and ad-hoc
signed, for testing on this Mac (no profile needed). Use another bundle id so
the test container doesn't mix with the real one:

```bash
FLOWIE_BUNDLE_ID=dev.flowie.sandboxtest scripts/package-appstore.sh --local
```

## Sandbox behaviour (tested)

- Data: `~/Library/Containers/<bundle id>/Data/Library/Application Support/Flowie/`
  (the App Store build can't see the old `~/Library/Application Support/Flowie`).
- Agent socket: `<container>/Data/tmp/flowie-<uid>-<hash>.sock` instead of `/tmp`.
- `Flowie mcp` started from a terminal or by Claude finds the same container
  (paths come from `APP_SANDBOX_CONTAINER_ID` + the real home directory),
  reads the outline, and starts the app in the background when it isn't running.
- MCP registration stays the same: `/Applications/Flowie.app/Contents/MacOS/Flowie mcp`.

Moving your current data into the App Store build (once, with Flowie quit,
after the first launch of the App Store version created the container):

```bash
cp ~/Library/Application\ Support/Flowie/flowie.sqlite* ~/Library/Application\ Support/Flowie/settings.json ~/Library/Containers/flowie.spokey.cn/Data/Library/Application\ Support/Flowie/
```

## Steps

1. **(you)** developer.apple.com → Certificates, IDs & Profiles
   - Identifiers → register the App ID `flowie.spokey.cn` (or pick another and
     set `FLOWIE_BUNDLE_ID`). No extra capabilities needed.
   - Profiles → **Mac App Store Connect** distribution profile for that App ID,
     with the Apple Distribution certificate already in your keychain.
     Download it to `packaging/macos/embedded.provisionprofile` (git-ignored).
2. **(you)** App Store Connect → Apps → New App: platform macOS, name Flowie,
   primary language, the bundle id, SKU (e.g. `flowie-mac`).
3. Fill the listing from [`metadata.md`](metadata.md); host
   [`privacy-policy.md`](privacy-policy.md) somewhere public and use its URL.
   App Privacy questionnaire: **Data Not Collected**.
4. Screenshots **(you)**: 1280×800, 1440×900, 2560×1600 or 2880×1800, 1–10
   per localization. Suggested shots: an outline with headings and math, the
   ⌘P palette, the right-click style menu, the calendar / journal, Settings →
   Agent.
5. Build and sign:
   ```bash
   scripts/package-appstore.sh
   ```
6. **(you)** Upload `build/appstore/Flowie-*.pkg` with the **Transporter** app
   (Mac App Store; sign in with your Apple ID). Xcode isn't needed.
7. **(you)** In App Store Connect pick the build, answer export compliance
   (already declared in Info.plist: no non-exempt encryption), paste the review
   notes from `metadata.md`, submit.

Each upload needs a higher build number; the git commit count does that as long
as you commit between uploads (or set `FLOWIE_BUILD`). Bump `version` in the
workspace `Cargo.toml` for a new marketing version (e.g. 1.0.0 for the first
release).
