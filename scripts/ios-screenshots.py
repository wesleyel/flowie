#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["Pillow>=11.0.0"]
# ///
"""Retake the iPhone and iPad App Store screenshots from the current code, in the iOS Simulator.

    scripts/ios-screenshots.py                     # every device, locale and shot
    scripts/ios-screenshots.py --device iphone     # one device (iphone or ipad)
    scripts/ios-screenshots.py --locale zh-Hans    # one locale
    scripts/ios-screenshots.py --only 03 search    # some shots (by number or name)
    scripts/ios-screenshots.py --no-build          # reuse the last Simulator build

Builds the app's Debug configuration for the Simulator (scripts/build-ios-ffi.sh, tuist,
xcodebuild into build/ios/DerivedData, with the ScreenshotTests UI-test target, which turns the
iPad to landscape: simctl can't) and writes a demo outline per locale with
`cargo run -p flowie-ffi --example demo_outline` (crates/flowie-ffi/examples/demo_outline.rs,
the text of every shot). Each shot starts the app on a fresh copy of it, in the app's App Group
container on a Simulator of its own ("Flowie Screenshots iPhone/iPad", made on first use, never
signed in to iCloud), and opens the shot's state with launch arguments: the launch rule's keys
(LaunchMemory) for the page, and in debug builds -FlowieSearch and -FlowieCapture
(apple/FlowieKit/Sources/FlowieUI/ScreenshotArguments.swift). The Simulator speaks the locale's
language, its status bar is fixed at 9:41 with full bars, and the appearance is light.

Writes appstore/fastlane/ios/screenshots/<locale>/iPhone-NN-name.png (1320×2868, iPhone 6.9")
and iPad-NN-name.png (2752×2064, iPad 13" in landscape); `fastlane ios screenshots` uploads them. Dates
follow today. Your own outline and Simulators are never touched.
"""

import argparse
import datetime as dt
import io
import json
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from PIL import Image, ImageChops, ImageStat

ROOT = Path(__file__).resolve().parent.parent
APP_DIR = ROOT / "apple" / "FlowieApp"
DERIVED = ROOT / "build" / "ios" / "DerivedData"
APP = DERIVED / "Build" / "Products" / "Debug-iphonesimulator" / "Flowie.app"
OUT = ROOT / "appstore" / "fastlane" / "ios" / "screenshots"
BUNDLE_ID = "flowie.spokey.cn"
GROUP = "group.flowie.spokey.cn"
SETTLE = 2.0  # seconds after launch before looking whether the screen has settled
STEADY = 6  # frames, half a second apart, that show a settled screen
STEADY_TRIES = 30

# The Simulators used: their own, so nothing of yours is in them. App Store Connect's sizes:
# iPhone 6.9" (1320×2868, portrait) and iPad 13" (2752×2064, landscape).
DEVICES = {
    "iphone": ("Flowie Screenshots iPhone", "com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro-Max", "iPhone", (1320, 2868)),
    "ipad": ("Flowie Screenshots iPad", "com.apple.CoreSimulator.SimDeviceType.iPad-Pro-13-inch-M5-12GB", "iPad", (2752, 2064)),
}

LOCALES = {
    "en-US": {
        "languages": "(en)",
        "language": "en",
        "locale": "en_US",
        "search": "#idea",
        "capture": "Ask Leo about the trail",
    },
    "zh-Hans": {
        "languages": "(zh-Hans)",
        "language": "zh-Hans",
        "locale": "zh_CN",
        "search": "#想法",
        "capture": "问问老王徒步路线",
    },
}

# (number, name, tab, path of demo ids pushed on the tab, extra launch arguments by locale)
SHOTS = [
    ("01", "home", "home", [], lambda l: []),
    ("02", "today", "today", [], lambda l: []),
    ("03", "page", "home", ["web", "milestones"], lambda l: []),
    ("04", "search", "search", [], lambda l: ["-FlowieSearch", l["search"]]),
    ("05", "capture", "today", [], lambda l: ["-FlowieCapture", l["capture"]]),
]


def run(*args, **kw):
    print("+", " ".join(str(a) for a in args), flush=True)
    return subprocess.run([str(a) for a in args], check=True, **kw)


def output(*args):
    return subprocess.run([str(a) for a in args], check=True, capture_output=True, text=True).stdout


def build():
    run(ROOT / "scripts" / "build-ios-ffi.sh", cwd=ROOT)
    run("tuist", "generate", "--no-open", cwd=APP_DIR)
    # The UI-test target, and the app it tests.
    run("xcodebuild", "-quiet", "-workspace", "Flowie.xcworkspace", "-scheme", "ScreenshotTests",
        "-configuration", "Debug", "-destination", "generic/platform=iOS Simulator",
        "-derivedDataPath", DERIVED, "build-for-testing", cwd=APP_DIR)


def seed(locale, today, folder):
    """A demo outline for `locale` in `folder`, and the ids the shots open."""
    db = folder / f"{locale}.sqlite"
    ids = output("cargo", "run", "-q", "--locked", "-p", "flowie-ffi", "--example", "demo_outline",
                 "--", db, locale, today.isoformat())
    return db, json.loads(ids)


def device(kind):
    """The Simulator for `kind`, made on first use and booted."""
    name, device_type, _, _ = DEVICES[kind]
    devices = json.loads(output("xcrun", "simctl", "list", "devices", "--json"))["devices"]
    udid = next((d["udid"] for runtime in devices.values() for d in runtime
                 if d["name"] == name and d.get("isAvailable", True)), None)
    if udid is None:
        runtimes = json.loads(output("xcrun", "simctl", "list", "runtimes", "--json"))["runtimes"]
        ios = [r for r in runtimes if r.get("platform") == "iOS" and r.get("isAvailable")]
        if not ios:
            sys.exit("No iOS Simulator runtime: install one in Xcode → Settings → Components")
        runtime = max(ios, key=lambda r: [int(p) for p in r["version"].split(".")])
        udid = output("xcrun", "simctl", "create", name, device_type, runtime["identifier"]).strip()
    run("xcrun", "simctl", "bootstatus", udid, "-b", stdout=subprocess.DEVNULL)
    return udid


def speak(udid, locale):
    """The Simulator in `locale`'s language and region, as a phone set up in it: the status bar
    (the iPad's date) and the keyboard follow the system's, not the app's. Restarts it if they
    change."""
    settings = LOCALES[locale]
    current = output("xcrun", "simctl", "spawn", udid, "defaults", "read", "-g", "AppleLanguages")
    languages = current.translate(str.maketrans("()\",", "    ")).split()
    if languages[:1] == [settings["language"]]:
        return
    run("xcrun", "simctl", "spawn", udid, "defaults", "write", "-g", "AppleLanguages", "-array", settings["language"])
    run("xcrun", "simctl", "spawn", udid, "defaults", "write", "-g", "AppleLocale", settings["locale"])
    run("xcrun", "simctl", "shutdown", udid)
    run("xcrun", "simctl", "bootstatus", udid, "-b", stdout=subprocess.DEVNULL)


def landscape(udid):
    """Turn the Simulator to landscape, with XCTest (ScreenshotTests/Landscape.swift)."""
    xctestrun = sorted((DERIVED / "Build" / "Products").glob("ScreenshotTests_*.xctestrun"))
    if not xctestrun:
        sys.exit("No ScreenshotTests build: run without --no-build")
    run("xcodebuild", "test-without-building", "-quiet", "-xctestrun", xctestrun[-1],
        "-destination", f"id={udid}", "-only-testing:ScreenshotTests/Landscape/testTurnToLandscape",
        stdout=subprocess.DEVNULL)


def prepare(udid, wide):
    run("xcrun", "simctl", "ui", udid, "appearance", "light")
    run("xcrun", "simctl", "status_bar", udid, "override", "--time", "9:41",
        "--dataNetwork", "wifi", "--wifiMode", "active", "--wifiBars", "3",
        "--cellularMode", "active", "--cellularBars", "4", "--operatorName", "",
        "--batteryState", "charged", "--batteryLevel", "100")
    # The keyboard's first-run tip (slide to type) would cover the capture shot.
    for key in ("DidShowContinuousPathIntroduction", "DidShowGestureKeyboardIntroduction"):
        run("xcrun", "simctl", "spawn", udid, "defaults", "write", "com.apple.Preferences", key, "-bool", "YES")
    run("xcrun", "simctl", "install", udid, APP)
    if wide:
        landscape(udid)
    # The first launch after installing is slow: get it over with before the shots.
    run("xcrun", "simctl", "launch", udid, BUNDLE_ID, stdout=subprocess.DEVNULL)
    time.sleep(2 * SETTLE)


def shoot(udid, db, ids, today, locale, shot, path):
    number, name, tab, pushed, extra = shot
    subprocess.run(["xcrun", "simctl", "terminate", udid, BUNDLE_ID], capture_output=True)
    container = Path(output("xcrun", "simctl", "get_app_container", udid, BUNDLE_ID, GROUP).strip())
    data = container / "Library" / "Application Support" / "Flowie"
    data.mkdir(parents=True, exist_ok=True)
    for old in data.glob("flowie.sqlite*"):
        old.unlink()
    (data / "icloud-sync-state.json").unlink(missing_ok=True)
    shutil.copy(db, data / "flowie.sqlite")

    settings = LOCALES[locale]
    route = ",".join(f'"{ids[key]}"' for key in pushed)
    run("xcrun", "simctl", "launch", udid, BUNDLE_ID,
        "-AppleLanguages", settings["languages"], "-AppleLocale", settings["locale"],
        # Back where the app was a moment ago (the launch rule, LaunchMemory).
        "-launch.tab", tab, "-launch.path", f"({route})", "-launch.scroll", "()",
        "-launch.active", str(time.time()), "-launch.day", today.isoformat(),
        "-hint.zoomed", "YES", "-appearance", "light",
        *extra(settings), stdout=subprocess.DEVNULL)
    time.sleep(SETTLE)
    # Done once the screen stops changing: the page has loaded and the keyboard is up. A caret
    # blinks, so frames that differ only in a caret-sized spot count as the same screen; of
    # those, the one with the caret drawn most solid is kept.
    frames = []
    for _ in range(STEADY_TRIES):
        frames.append(capture(udid, path))
        recent = frames[-STEADY:]
        if len(recent) == STEADY and caret_only(recent):
            # The caret fades in and out: a few quick frames more, for one with it solid.
            for _ in range(8):
                time.sleep(0.12)
                recent.append(capture(udid, path))
            path.write_bytes(with_caret([f for f in recent if caret_only([recent[0], f])]))
            return
        time.sleep(0.5)
    sys.exit(f"{path}: the screen didn't settle")


def capture(udid, path):
    run("xcrun", "simctl", "io", udid, "screenshot", "--type=png", path, stderr=subprocess.DEVNULL)
    return path.read_bytes()


def gray(frame):
    return Image.open(io.BytesIO(frame)).convert("L")


def caret_box(frames):
    """Where `frames` differ (None: nowhere)."""
    images = [gray(f) for f in dict.fromkeys(frames)]
    boxes = [b for b in (ImageChops.difference(images[0], im).getbbox() for im in images[1:]) if b]
    if not boxes:
        return None
    return (min(b[0] for b in boxes), min(b[1] for b in boxes), max(b[2] for b in boxes), max(b[3] for b in boxes))


def caret_only(frames):
    """Whether `frames` show one settled screen: alike but for a blinking caret, and not the
    empty launch screen (nothing below the status bar). A flash of the Dynamic Island, a page
    still loading or the keyboard coming in all differ by more."""
    image = gray(frames[0])
    low, high = image.crop((0, image.height // 8, image.width, image.height)).getextrema()
    if high - low < 32:
        return False
    box = caret_box(frames)
    return box is None or (box[2] - box[0] <= 24 and box[3] - box[1] <= 160)


def with_caret(frames):
    """The frame with the caret drawn most solid: darkest where they differ."""
    box = caret_box(frames)
    if box is None:
        return frames[-1]
    return min(frames, key=lambda f: ImageStat.Stat(gray(f).crop(box)).mean[0])


def size(path):
    out = output("sips", "-g", "pixelWidth", "-g", "pixelHeight", path).split()
    return int(out[out.index("pixelWidth:") + 1]), int(out[out.index("pixelHeight:") + 1])


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--device", choices=sorted(DEVICES), action="append")
    parser.add_argument("--locale", choices=sorted(LOCALES), action="append")
    parser.add_argument("--only", nargs="+", help="shot numbers or names")
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()

    shots = [s for s in SHOTS if not args.only or s[0] in args.only or s[1] in args.only]
    if not shots:
        sys.exit(f"No such shot: {' '.join(args.only)}")
    if not args.no_build:
        build()
    if not APP.exists():
        sys.exit(f"No Simulator build at {APP}: run without --no-build")

    today = dt.date.today()
    with tempfile.TemporaryDirectory(prefix="flowie-ios-shots-") as tmp:
        outlines = {locale: seed(locale, today, Path(tmp)) for locale in args.locale or LOCALES}
        for kind in args.device or DEVICES:
            _, _, prefix, expected = DEVICES[kind]
            udid = device(kind)
            for locale, (db, ids) in outlines.items():
                speak(udid, locale)
                prepare(udid, wide=expected[0] > expected[1])
                folder = OUT / locale
                folder.mkdir(parents=True, exist_ok=True)
                for shot in shots:
                    path = folder / f"{prefix}-{shot[0]}-{shot[1]}.png"
                    shoot(udid, db, ids, today, locale, shot, path)
                    if size(path) != expected:
                        sys.exit(f"{path} is {size(path)}, App Store Connect wants {expected}")
            subprocess.run(["xcrun", "simctl", "terminate", udid, BUNDLE_ID], capture_output=True)
            run("xcrun", "simctl", "shutdown", udid)
    print(f"Screenshots in {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
