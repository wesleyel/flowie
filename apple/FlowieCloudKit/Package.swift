// swift-tools-version:5.9
// FlowieCloudKit: CloudKit sync for Flowie's Apple apps (docs/roadmap/per-node-sync.md §5.2).
// It only moves records: CKSyncEngine's callbacks go to flowie-ffi's engine, and a CKRecord
// carries an envelope in one encrypted field. The generated bindings (FlowieFFI and its C
// module) come from scripts/cloudkit-package-check.sh, which builds and checks the package.
import PackageDescription

let package = Package(
    name: "FlowieCloudKit",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "FlowieCloudKit", targets: ["FlowieCloudKit"])],
    targets: [
        .target(name: "flowie_ffiFFI", path: "Sources/flowie_ffiFFI"),
        .target(
            name: "FlowieFFI",
            dependencies: ["flowie_ffiFFI"],
            path: "Sources/FlowieFFI",
            linkerSettings: [.linkedLibrary("flowie_ffi")]
        ),
        .target(name: "FlowieCloudKit", dependencies: ["FlowieFFI"], path: "Sources/FlowieCloudKit"),
        .executableTarget(name: "cloudkit-check", dependencies: ["FlowieCloudKit"], path: "Sources/cloudkit-check"),
    ]
)
