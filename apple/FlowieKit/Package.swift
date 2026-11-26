// swift-tools-version:5.9
// FlowieKit: what Flowie's Apple apps share, over flowie-ffi (the Rust core through UniFFI).
// - FlowieCloudKit: CloudKit sync through CKSyncEngine (docs/roadmap/per-node-sync.md §5.2).
// - FlowieUI: the iOS app's SwiftUI layer (ios-cross-platform.md §4): the outline as pages of
//   bullets, editing, and sync with a self-hosted server. It builds for macOS too, so it can be
//   compiled and checked without Xcode; the iOS app target adds only its shell.
// The generated bindings (FlowieFFI and its C module) come from scripts/apple-package-check.sh,
// which builds and checks the package.
import PackageDescription

let package = Package(
    name: "FlowieKit",
    platforms: [.macOS(.v14), .iOS(.v17)],
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
        .target(name: "FlowieCloudKit", dependencies: ["FlowieFFI"], path: "Sources/FlowieCloudKit"),
        .target(name: "FlowieUI", dependencies: ["FlowieFFI"], path: "Sources/FlowieUI"),
        // A window with FlowieUI on the Mac, for trying the views without an iOS build.
        .executableTarget(name: "FlowiePreview", dependencies: ["FlowieUI"], path: "Sources/FlowiePreview"),
        .executableTarget(name: "cloudkit-check", dependencies: ["FlowieCloudKit"], path: "Sources/cloudkit-check"),
        .executableTarget(name: "ui-check", dependencies: ["FlowieUI"], path: "Sources/ui-check"),
    ]
)
