// The iOS app, generated with Tuist: `tuist generate` here (after scripts/build-ios-ffi.sh,
// which builds the Rust core it links). Simulator builds only for now: no signing set up.
import ProjectDescription

let project = Project(
    name: "Flowie",
    packages: [.package(path: "../FlowieKit")],
    settings: .settings(base: [
        "MARKETING_VERSION": "1.0.1",
        "CURRENT_PROJECT_VERSION": "1",
    ]),
    targets: [
        .target(
            name: "Flowie",
            destinations: [.iPhone, .iPad],
            product: .app,
            // The App Store record the Mac app ships under (Universal Purchase).
            bundleId: "flowie.spokey.cn",
            deploymentTargets: .iOS("26.0"),
            infoPlist: .extendingDefault(with: [
                "CFBundleDisplayName": "Flowie",
                "CFBundleShortVersionString": "$(MARKETING_VERSION)",
                "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
                "UILaunchScreen": [:],
                // So the system (Siri, Shortcuts, intent dialogs) knows the app speaks both.
                "CFBundleLocalizations": ["en", "zh-Hans"],
                // The home-screen quick action: today's journal, ready to type (CaptureShortcuts.swift).
                "UIApplicationShortcutItems": [[
                    "UIApplicationShortcutItemType": "flowie.spokey.cn.capture",
                    "UIApplicationShortcutItemTitle": "Write in Today’s Journal",
                    "UIApplicationShortcutItemIconSymbolName": "square.and.pencil",
                ]],
                "UIApplicationSceneManifest": ["UIApplicationSupportsMultipleScenes": false],
                "ITSAppUsesNonExemptEncryption": false,
            ]),
            sources: ["Sources/**", "Shared/**"],
            // The app's own strings, in English and Simplified Chinese.
            resources: ["Resources/**"],
            // The outline lives in the App Group container (wiki: iOS-Cross-Platform §5). The
            // Simulator needs nothing more; device builds need the group registered on the App ID.
            entitlements: .dictionary([
                "com.apple.security.application-groups": .array([.string("group.flowie.spokey.cn")]),
            ]),
            dependencies: [
                // Embedded in the app.
                .target(name: "JournalControl"),
                .package(product: "FlowieUI"),
                // flowie-ffi's static library, from scripts/build-ios-ffi.sh.
                .xcframework(path: "../../build/ios/FlowieFFI.xcframework"),
                // What the Rust code needs besides libSystem (rustc --print native-static-libs).
                .sdk(name: "iconv", type: .library),
                .sdk(name: "CoreFoundation", type: .framework),
            ]
        ),
        // The Control Center control (also offered for the Action Button). It only runs the
        // shared `WriteInJournal` intent, which the system runs in the app: no outline, no App
        // Group, no Rust here.
        .target(
            name: "JournalControl", destinations: [.iPhone, .iPad], product: .appExtension,
            bundleId: "flowie.spokey.cn.JournalControl", deploymentTargets: .iOS("26.0"),
            infoPlist: .extendingDefault(with: [
                "CFBundleDisplayName": "Flowie",
                "NSExtension": ["NSExtensionPointIdentifier": "com.apple.widgetkit-extension"],
                "CFBundleShortVersionString": "$(MARKETING_VERSION)",
                "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
            ]),
            sources: ["Control/**", "Shared/**"], resources: ["Resources/Localizable.xcstrings"],
            settings: .settings(base: ["SWIFT_ACTIVE_COMPILATION_CONDITIONS": "$(inherited) FLOWIE_CONTROL"])
        ),
    ]
)
