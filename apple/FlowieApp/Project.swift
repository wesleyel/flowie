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
                "UILaunchScreen": [:],
                "UIApplicationSceneManifest": ["UIApplicationSupportsMultipleScenes": false],
                "ITSAppUsesNonExemptEncryption": false,
            ]),
            sources: ["Sources/**"],
            // The app's own strings, in English and Simplified Chinese.
            resources: ["Resources/**"],
            // The outline lives in the App Group container (wiki: iOS-Cross-Platform §5). The
            // Simulator needs nothing more; device builds need the group registered on the App ID.
            entitlements: .dictionary([
                "com.apple.security.application-groups": .array([.string("group.flowie.spokey.cn")]),
            ]),
            dependencies: [
                .package(product: "FlowieUI"),
                // flowie-ffi's static library, from scripts/build-ios-ffi.sh.
                .xcframework(path: "../../build/ios/FlowieFFI.xcframework"),
                // What the Rust code needs besides libSystem (rustc --print native-static-libs).
                .sdk(name: "iconv", type: .library),
                .sdk(name: "CoreFoundation", type: .framework),
            ]
        ),
    ]
)
