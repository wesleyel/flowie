// swift-tools-version:5.9
// FlowieKit: what Flowie's Apple apps share, over flowie-ffi (the Rust core through UniFFI).
// - FlowieCloudKit: CloudKit sync through CKSyncEngine (wiki: Per-Node-Sync §5.2), over a host
//   protocol: flowie-ffi's outline on iOS (FlowieUI), flowie-cloudkit's bridge on the Mac.
// - FlowieUI: the iOS app's SwiftUI layer (ios-cross-platform.md §4): the outline as pages of
//   bullets, editing, and sync with a self-hosted server. It builds for macOS too, so it can be
//   compiled and checked without Xcode; the iOS app target adds only its shell.
// The generated bindings (FlowieFFI and its C module) come from scripts/apple-package-check.sh,
// which builds and checks the package.
import PackageDescription

let package = Package(
    name: "FlowieKit",
    platforms: [.macOS(.v14), .iOS("26.0")],
    products: [
        .library(name: "FlowieCloudKit", targets: ["FlowieCloudKit"]),
        .library(name: "FlowieUI", targets: ["FlowieUI"]),
    ],
    targets: [
        .target(name: "flowie_ffiFFI", path: "Sources/flowie_ffiFFI"),
        .target(
            name: "FlowieFFI",
            dependencies: ["flowie_ffiFFI"],
            path: "Sources/FlowieFFI",
            linkerSettings: [.linkedLibrary("flowie_ffi")]
        ),
        // No Rust here: the Mac app links these sources into the GPUI binary through
        // crates/flowie-cloudkit, with its own bridge to the outline.
        .target(name: "FlowieCloudKit", path: "Sources/FlowieCloudKit"),
        .target(name: "FlowieUI", dependencies: ["FlowieFFI", "FlowieCloudKit"], path: "Sources/FlowieUI"),
        // A window with FlowieUI on the Mac, for trying the views without an iOS build.
        .executableTarget(name: "FlowiePreview", dependencies: ["FlowieUI"], path: "Sources/FlowiePreview"),
        .executableTarget(name: "cloudkit-check", dependencies: ["FlowieCloudKit", "FlowieUI"], path: "Sources/cloudkit-check"),
        .executableTarget(name: "ui-check", dependencies: ["FlowieUI"], path: "Sources/ui-check"),
    ]
)
