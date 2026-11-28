// The iOS app, generated with Tuist: `tuist generate` here (after scripts/build-ios-ffi.sh,
// which builds the Rust core it links). Simulator builds need no signing; device builds and
// App Store archives (scripts/package-ios.sh) sign automatically with the team below.
import ProjectDescription

let project = Project(
    name: "Flowie",
    packages: [.package(path: "../FlowieKit")],
    settings: .settings(
        base: [
            // Like the desktop: the version is the workspace's in Cargo.toml and the build number
            // the git commit count. scripts/package-ios.sh sets both when it archives; these are
            // what Simulator builds show.
            "MARKETING_VERSION": "1.0.1",
            "CURRENT_PROJECT_VERSION": "1",
            "DEVELOPMENT_TEAM": "5D26F33GNM",
            "CODE_SIGN_STYLE": "Automatic",
            // The Rust core is built for Apple-silicon Simulators only (scripts/build-ios-ffi.sh),
            // so Release Simulator builds, which build every architecture, leave Intel out.
            "EXCLUDED_ARCHS[sdk=iphonesimulator*]": "x86_64",
        ],
        configurations: [
            // Debug builds talk to CloudKit's Development environment and the sandbox push
            // service; Release builds (TestFlight and the App Store) to Production.
            .debug(name: "Debug", settings: [
                "FLOWIE_APS_ENVIRONMENT": "development",
                "FLOWIE_ICLOUD_ENVIRONMENT": "Development",
            ]),
            .release(name: "Release", settings: [
                "FLOWIE_APS_ENVIRONMENT": "production",
                "FLOWIE_ICLOUD_ENVIRONMENT": "Production",
            ]),
        ]
    ),
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
                // Handoff of the page shown, between iPhone and iPad (FlowieUI's PageActivity).
                "NSUserActivityTypes": ["flowie.spokey.cn.page"],
                // CKSyncEngine's silent pushes wake the app to fetch iCloud changes.
                "UIBackgroundModes": ["remote-notification"],
                "ITSAppUsesNonExemptEncryption": false,
                // arm64 only (iOS 26); Tuist's default asks for armv7.
                "UIRequiredDeviceCapabilities": ["arm64"],
            ]),
            sources: ["Sources/**", "Shared/**"],
            // The app's own strings in English and Simplified Chinese, the app icon
            // (`swift scripts/make-icon.swift --ios …`), and the Mac app's privacy manifest:
            // no tracking, no data collected (wiki: iOS-Cross-Platform §5).
            resources: ["Resources/**", "../../appstore/macos/PrivacyInfo.xcprivacy"],
            // The outline lives in the App Group container (wiki: iOS-Cross-Platform §5), and
            // syncs through iCloud's private database (CloudKit, with pushes for changes; wiki:
            // Per-Node-Sync §5.2). The Simulator needs nothing more; device builds need the group
            // registered on the App ID, and a profile with it, iCloud and push. The environments
            // follow the configuration (above).
            entitlements: .dictionary([
                "com.apple.security.application-groups": .array([.string("group.flowie.spokey.cn")]),
                "com.apple.developer.icloud-container-identifiers": .array([.string("iCloud.flowie.spokey.cn")]),
                "com.apple.developer.icloud-services": .array([.string("CloudKit")]),
                "com.apple.developer.icloud-container-environment": .string("$(FLOWIE_ICLOUD_ENVIRONMENT)"),
                "aps-environment": .string("$(FLOWIE_APS_ENVIRONMENT)"),
            ]),
            dependencies: [
                // Embedded in the app.
                .target(name: "JournalControl"),
                .target(name: "Share"),
                .package(product: "FlowieUI"),
                .package(product: "FlowieInbox"),
                // flowie-ffi's static library, from scripts/build-ios-ffi.sh.
                .xcframework(path: "../../build/ios/FlowieFFI.xcframework"),
                // What the Rust code needs besides libSystem (rustc --print native-static-libs).
                .sdk(name: "iconv", type: .library),
                .sdk(name: "CoreFoundation", type: .framework),
            ],
            settings: .settings(base: ["ASSETCATALOG_COMPILER_APPICON_NAME": "AppIcon"])
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
                // The same version and build number as the app, as the App Store requires.
                "CFBundleShortVersionString": "$(MARKETING_VERSION)",
                "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
            ]),
            sources: ["Control/**", "Shared/**"], resources: ["Resources/Localizable.xcstrings"],
            settings: .settings(base: ["SWIFT_ACTIVE_COMPILATION_CONDITIONS": "$(inherited) FLOWIE_CONTROL"])
        ),
        // The share extension: a sheet that leaves what was shared, text or a web page, in the App
        // Group's inbox (FlowieInbox) for the app to add to today's journal. No outline, no
        // Rust here (wiki: iOS-Cross-Platform §5). Images aren't offered: iOS v1 can't insert
        // them (§2.2).
        .target(
            name: "Share", destinations: [.iPhone, .iPad], product: .appExtension,
            bundleId: "flowie.spokey.cn.Share", deploymentTargets: .iOS("26.0"),
            infoPlist: .extendingDefault(with: [
                "CFBundleDisplayName": "Flowie",
                "NSExtension": [
                    "NSExtensionPointIdentifier": "com.apple.share-services",
                    "NSExtensionPrincipalClass": "$(PRODUCT_MODULE_NAME).ShareViewController",
                    "NSExtensionAttributes": [
                        "NSExtensionActivationRule": [
                            "NSExtensionActivationSupportsWebURLWithMaxCount": 1,
                            "NSExtensionActivationSupportsText": true,
                        ],
                    ],
                ],
                "CFBundleShortVersionString": "$(MARKETING_VERSION)",
                "CFBundleVersion": "$(CURRENT_PROJECT_VERSION)",
            ]),
            sources: ["Share/**"], resources: ["Resources/Localizable.xcstrings"],
            // The inbox is in the App Group container; device builds need the group on this
            // App ID too.
            entitlements: .dictionary([
                "com.apple.security.application-groups": .array([.string("group.flowie.spokey.cn")]),
            ]),
            dependencies: [.package(product: "FlowieInbox")]
        ),
        // Turns the iPad Simulator to landscape for the App Store screenshots
        // (scripts/ios-screenshots.py): only XCTest can turn a Simulator. Not part of the app.
        .target(
            name: "ScreenshotTests", destinations: [.iPhone, .iPad], product: .uiTests,
            bundleId: "flowie.spokey.cn.ScreenshotTests", deploymentTargets: .iOS("26.0"),
            sources: ["ScreenshotTests/**"],
            dependencies: [.target(name: "Flowie")]
        ),
    ]
)
