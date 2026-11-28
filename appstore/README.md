# App Store releases

Everything for the App Store lives in this folder: the Mac app, and the iPhone and iPad app
that ships under the same record. The Mac app's code is the same as the open-source build.

```
appstore/
  fastlane/            Appfile, Deliverfile, Fastfile (lanes), .env (git-ignored)
    macos/             the Mac listing
      metadata/        listing text per locale, categories, copyright, review notes
      screenshots-source/  2400×1500 app captures per locale (scripts/screenshots.py)
      screenshots/     2880×1800 framed screenshots per locale (scripts/appstore-screenshots.py)
    ios/               the iPhone and iPad listing (same App Store record: Universal Purchase)
      metadata/        its own description, keywords, promotional text, release notes and review
                       notes; everything both listings share is a symlink into macos/metadata/
      screenshots-source/  full-resolution Simulator captures (scripts/ios-screenshots.py)
      screenshots/     framed iPhone 6.9" portrait and iPad 13" landscape PNGs for upload
  macos/               sandbox entitlements, privacy manifest, cargo-about notices
                       (+ embedded.provisionprofile, git-ignored)
```

App Store Connect is driven by fastlane: listing text, screenshots, review
information, price, availability, the build upload and the submission. Run lanes from
this folder: `cd appstore && fastlane mac <lane>` (or `fastlane ios <lane>` for the iPhone and
iPad listing).

The two listings sit side by side. App Store Connect keeps the name, subtitle, privacy URL and
categories once for the whole app, so `ios/metadata/` links those (and the support and marketing
URLs, copyright and review contact) to the files in `macos/metadata/`: edit them there. A
fresh clone has no review contact files (below), so those links point at nothing until you
create them, and fastlane skips them.

## What the build does

`scripts/package-appstore.sh` produces `build/appstore/Flowie-<version>-<build>.pkg`:

- Universal binary (arm64 + x86_64), built in its own target dir `target/appstore`.
- Info.plist: version from `Cargo.toml`, build number = git commit count
  (`FLOWIE_BUILD` overrides), category Productivity, minimum macOS 13.0,
  `ITSAppUsesNonExemptEncryption = false`, copyright, en + zh-Hans localizations.
- App Sandbox with read/write access only to files picked in Open / Save
  panels (import, export) — `appstore/macos/Flowie.entitlements`.
- Privacy manifest (`PrivacyInfo.xcprivacy`): no tracking, no data collected;
  required-reason APIs: file timestamps (C617.1), user defaults (CA92.1), disk
  space (E174.1, SQLite's statfs). The iOS app and both extensions ship it too.
- `Contents/Resources/ThirdPartyNotices.txt`: licenses of the bundled open-source
  components (Rust crates via cargo-about, KaTeX fonts OFL, Lucide icons).
  Their licenses (MIT, Apache-2.0, BSD, ISC, OFL…) require shipping these
  notices with the binary; the file isn't shown anywhere in the UI. The iOS app
  ships the same notices for flowie-ffi (`scripts/package-ios.sh`) and shows them in
  Settings → About → Open-Source Notices.
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

Ten shots per language, in two steps. `scripts/screenshots.py` retakes the app captures
from the current code: 2400×1500 PNGs in `fastlane/macos/screenshots-source/<locale>/` and the
website's 1600×1000 JPEGs in `website/images/`. It builds Flowie with the `screenshots`
feature (into `target/screenshots`), runs it on a throwaway database with a 1200×750
window, fills it with demo pages through the agent socket and captures each state with
`screencapture`. The demo text and the list of shots are at the top of the script.
`scripts/appstore-screenshots.py` then frames each capture under its headline as the
2880×1800 App Store screenshots in `fastlane/macos/screenshots/<locale>/`; the headlines are at
the top of that script.

```bash
scripts/screenshots.py
```

```bash
scripts/screenshots.py --locale zh-Hans --only 03 focus
```

```bash
scripts/appstore-screenshots.py
```

It needs a Retina display and Screen Recording permission for the terminal, and the
mouse kept off the Flowie window while it runs. Dates follow today, so the journal and
calendar show the current month. Then `fastlane mac screenshots` uploads them.

The iOS listing has five shots per device for both English and Simplified Chinese. They show
Home, Today, a bullet opened as a page (two pages on iPad), tag search and quick capture.
From the repository root, run one of these commands on macOS:

```bash
scripts/ios-screenshots.py
scripts/ios-screenshots.py --frame-only
```

The first command takes fresh captures in the Simulator, updates the website’s four unframed
Home images and frames the store set. The second reads the saved captures. Both accept
`--device`, `--locale` and `--only`. Edit the headlines in `scripts/ios_store_artwork.py`.
The upload directory is `fastlane/ios/screenshots/<locale>/`. [Simulator setup](../docs/building.md#ios-in-development)

## iOS submission checklist

- Review the five PNGs in order for each device and locale. iPhone files are 1320×2868 and iPad files are 2752×2064. Match the two-pane view and keyboard layout to the build selected for review.
- Check the description, promotional text, keywords and release notes in `fastlane/ios/metadata/<locale>/`. Confirm the shared support, marketing and privacy links, and the local App Review contact. The reviewer walkthrough is in [review notes](fastlane/ios/metadata/review_information/notes.txt). Sign-in is optional and the reviewer can try the outline offline.
- Test the candidate on a physical iPhone and iPad. Try editing offline, sharing text or a web link to Today, and syncing through production iCloud. Check that the same journal day appears once on iPhone, iPad and Mac. The iOS launch is scheduled after the iCloud-capable Mac version goes live.
- Before the first iOS submission, confirm in the CloudKit Console that the `Record` and `Blob` record types are deployed to Production, and answer any new age rating questions in App Store Connect (Flowie has no social or user-generated content features, so the answers stay No and the rating stays 4+).
- When ready to update App Store Connect, run `fastlane ios metadata`, `fastlane ios screenshots` and `fastlane ios check` from this folder. Verify App Privacy (Data Not Collected), content rights, availability and release timing, then `fastlane ios submit` the tested build. [Build and TestFlight instructions](../docs/building.md#ios-in-development)

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
`email_address.txt` in `appstore/fastlane/macos/metadata/review_information/`) is git-ignored so the
public repository doesn't carry a phone number; create those four files locally
before running either platform’s metadata lane.

| Lane | What it does |
|---|---|
| `fastlane mac metadata` | Push `macos/metadata/` (name, subtitle, description, keywords, URLs, categories, copyright, review contact and notes) |
| `fastlane mac screenshots` | Make the remote screenshots match `macos/screenshots/<locale>/` (2880×1800) |
| `fastlane mac pricing` | US$4.99, other storefronts equalized by Apple; every storefront except mainland China |
| `fastlane mac check` | precheck the listing (placeholder text, broken URLs, …) |
| `fastlane mac build` | `scripts/package-appstore.sh` on a clean tree → `build/appstore/Flowie-<version>-<build>.pkg` |
| `fastlane mac upload` | Upload the newest pkg with the App Store Connect Build Upload API (no Xcode or Transporter) |
| `fastlane ios metadata` | Push `ios/metadata/` (the iPhone and iPad listing; the shared fields are the same files as the Mac's) |
| `fastlane ios screenshots` | Make the remote iPhone and iPad screenshots match `ios/screenshots/<locale>/` |
| `fastlane mac submit` | Attach that build to the version and submit for review; stops unless Cargo.toml, the pkg and the App Store version agree (release type comes from the metadata lane: automatic, so it goes live once approved) |
| `fastlane mac beta` | build + upload, then wait until TestFlight's Internal group (all builds) can install it |
| `fastlane mac cancel` | Withdraw the submission waiting for review, to fix the version and submit again |
| `fastlane ios check` | precheck the iPhone and iPad listing |
| `fastlane ios submit` | Attach the newest `build/ios/appstore/Flowie-<version>-<build>.ipa` build (uploaded with `scripts/package-ios.sh --upload`) to the iOS version and submit it for review; the same version checks as `mac submit`, released automatically once approved |
| `fastlane ios cancel` | Withdraw the iOS submission waiting for review |

## Releasing a new version

1. Set the workspace version in `Cargo.toml` (the app, the package and App Store Connect
   must all agree; `submit` checks). Move `[Unreleased]` under the new version in both
   changelogs, and update the two locales' `macos/metadata/<locale>/release_notes.txt` before committing.
2. Create the version in App Store Connect (or let `metadata` do it), then
   `fastlane mac metadata` for "What's New" and URLs.
3. `fastlane mac beta` → try it from TestFlight.
4. `fastlane mac submit`.

GitHub desktop distribution is independent of App Review and does not use App Store
credentials. Push `vX.Y.Z` for a stable release or `vX.Y.Z-rc.N` for a GitHub prerelease
after the checks pass. **Desktop Packages** builds the macOS universal zip alongside the
Windows x64 and Linux x86_64 previews. The `flowie` Homebrew cask follows the newest
published version, including release candidates. See [desktop releases](../docs/building.md#desktop-releases-and-website).

The build number is the git commit count, so commit between uploads.

Set in App Store Connect once and not managed by fastlane: age rating (4+),
App Privacy (Data Not Collected), content rights (no third-party content).

Homebrew's fastlane on Ruby 4 needs `bigdecimal`, `digest-crc` and `nkf`
installed into `~/.local/share/fastlane/4.0.0`, and `LC_ALL=en_US.UTF-8`.
