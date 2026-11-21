# Mac App Store release

Everything for the Mac App Store lives in this folder; the app's code is the same as
the open-source build.

```
appstore/
  fastlane/            Appfile, Deliverfile, Fastfile (lanes), .env (git-ignored)
    metadata/          listing text per locale, categories, copyright, review notes
    screenshots/       2880×1800 screenshots per locale (scripts/screenshots.py)
  macos/               sandbox entitlements, privacy manifest, cargo-about notices
                       (+ embedded.provisionprofile, git-ignored)
```

App Store Connect is driven by fastlane: listing text, screenshots, review
information, price, availability, the build upload and the submission. Run lanes from
this folder: `cd appstore && fastlane mac <lane>`.

## What the build does

`scripts/package-appstore.sh` produces `build/appstore/Flowie-<version>-<build>.pkg`:

- Universal binary (arm64 + x86_64), built in its own target dir `target/appstore`.
- Info.plist: version from `Cargo.toml`, build number = git commit count
  (`FLOWIE_BUILD` overrides), category Productivity, minimum macOS 13.0,
  `ITSAppUsesNonExemptEncryption = false`, copyright, en + zh-Hans localizations.
- App Sandbox with read/write access only to files picked in Open / Save
  panels (import, export) — `appstore/macos/Flowie.entitlements`.
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
after the first launch of the App Store version created the container). macOS only lets
Terminal.app write into another app's container, so run it there:

```bash
cp ~/Library/Application\ Support/Flowie/flowie.sqlite* ~/Library/Application\ Support/Flowie/settings.json ~/Library/Containers/flowie.spokey.cn/Data/Library/Application\ Support/Flowie/
```

## Screenshots

`scripts/screenshots.py` retakes every screenshot from the current code: the App Store
PNGs in `fastlane/screenshots/<locale>/` and the website's 1600×1000 JPEGs in
`website/images/`. It builds Flowie with the `screenshots` feature (into
`target/screenshots`), runs it on a throwaway database with a 1440×900 window, fills it
with demo pages through the agent socket and captures each state with `screencapture`.
The demo text and the list of shots are at the top of the script.

```bash
scripts/screenshots.py
```

```bash
scripts/screenshots.py --locale zh-Hans --only 03 bullet-menu
```

It needs a Retina display and Screen Recording permission for the terminal, and the
mouse kept off the Flowie window while it runs. Dates follow today, so the journal and
calendar show the current month. Then `fastlane mac screenshots` uploads them.

## Releasing with fastlane

App: **Flowie Outliner** (Apple ID 6835913837, bundle id `flowie.spokey.cn`,
SKU `flowie-mac`). Marketing, support, privacy and terms pages are the GitHub Pages
site https://wesleyel.github.io/flowie/ (source in `website/`, deployed by
`.github/workflows/pages.yml`): `/`, `/#support`, `/privacy.html`, `/terms.html`.

Authentication is an App Store Connect API key, set in `appstore/fastlane/.env`
(git-ignored):

```
ASC_KEY_ID=…
ASC_ISSUER_ID=…
ASC_KEY_PATH=~/.appstoreconnect/private_keys/AuthKey_….p8
```

The App Review contact (`first_name.txt`, `last_name.txt`, `phone_number.txt`,
`email_address.txt` in `appstore/fastlane/metadata/review_information/`) is git-ignored so the
public repository doesn't carry a phone number; create those four files locally
before running `fastlane mac metadata`.

| Lane | What it does |
|---|---|
| `fastlane mac metadata` | Push `metadata/` (name, subtitle, description, keywords, URLs, categories, copyright, review contact and notes) |
| `fastlane mac screenshots` | Make the remote screenshots match `screenshots/<locale>/` (2880×1800) |
| `fastlane mac pricing` | US$4.99, other storefronts equalized by Apple; every storefront except mainland China |
| `fastlane mac check` | precheck the listing (placeholder text, broken URLs, …) |
| `fastlane mac build` | `scripts/package-appstore.sh` on a clean tree → `build/appstore/Flowie-<version>-<build>.pkg` |
| `fastlane mac upload` | Upload the newest pkg with the App Store Connect Build Upload API (no Xcode or Transporter) |
| `fastlane mac submit` | Attach that build to the version and submit for review; stops unless Cargo.toml, the pkg and the App Store version agree (release type comes from the metadata lane: manual) |
| `fastlane mac beta` | build + upload, then wait until TestFlight's Internal group (all builds) can install it |
| `fastlane mac cancel` | Withdraw the submission waiting for review, to fix the version and submit again |

## Releasing a new version

1. Bump `version` in the workspace `Cargo.toml` (the app, the package and App Store
   Connect must all agree; `submit` checks), move the `[Unreleased]` notes in
   `CHANGELOG.md` under the new version (CI fails without an entry), update the listing
   text if needed, commit.
2. Create the version in App Store Connect (or let `metadata` do it), then
   `fastlane mac metadata` for "What's New" and URLs.
3. `fastlane mac beta` → try it from TestFlight.
4. `fastlane mac submit`.
5. Release candidates first: put the notes under `## [X.Y.Z-rc.N]` in `CHANGELOG.md`, tag
   the commit `vX.Y.Z-rc.N` and push the tag. `.github/workflows/release.yml` checks the
   tag against `Cargo.toml` (still `X.Y.Z`, which is what the App Store gets), publishes a
   GitHub release (not a pre-release) with that section as its notes and `Flowie-X.Y.Z-rc.N-macos.zip`,
   and updates the `flowie@beta` cask.
6. Once a candidate holds up (and App Review has passed), gather the candidates' notes
   under `## [X.Y.Z]`, tag `vX.Y.Z` and push: the stable release and the `flowie` cask.
   Whichever tag is the newest version is marked latest, a candidate included. To fix notes later, edit `CHANGELOG.md` on main and run the Release
   workflow by hand with the tag.

The build number is the git commit count, so commit between uploads.

Set in App Store Connect once and not managed by fastlane: age rating (4+),
App Privacy (Data Not Collected), content rights (no third-party content).

Homebrew's fastlane on Ruby 4 needs `bigdecimal`, `digest-crc` and `nkf`
installed into `~/.local/share/fastlane/4.0.0`, and `LC_ALL=en_US.UTF-8`.
