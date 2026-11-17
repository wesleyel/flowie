fastlane documentation
----

# Installation

Make sure you have the latest version of the Xcode command line tools installed:

```sh
xcode-select --install
```

For _fastlane_ installation instructions, see [Installing _fastlane_](https://docs.fastlane.tools/#installing-fastlane)

# Available Actions

## Mac

### mac build

```sh
[bundle exec] fastlane mac build
```

Build and sign build/appstore/Flowie-<version>-<build>.pkg (build = git commit count)

### mac upload

```sh
[bundle exec] fastlane mac upload
```

Upload the newest package in build/appstore to App Store Connect

### mac metadata

```sh
[bundle exec] fastlane mac metadata
```

Push listing text, categories, copyright and review information (no screenshots, no binary)

### mac screenshots

```sh
[bundle exec] fastlane mac screenshots
```

Make the remote screenshots match fastlane/screenshots exactly

### mac pricing

```sh
[bundle exec] fastlane mac pricing
```

Set the price (US$4.99, equalized) and sell everywhere except CHN

### mac check

```sh
[bundle exec] fastlane mac check
```

Check the listing for anything App Review flags, without uploading

### mac submit

```sh
[bundle exec] fastlane mac submit
```

Attach the newest uploaded build to the version and submit it for review

----

This README.md is auto-generated and will be re-generated every time [_fastlane_](https://fastlane.tools) is run.

More information about _fastlane_ can be found on [fastlane.tools](https://fastlane.tools).

The documentation of _fastlane_ can be found on [docs.fastlane.tools](https://docs.fastlane.tools).
