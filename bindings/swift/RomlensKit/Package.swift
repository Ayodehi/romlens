// swift-tools-version: 6.0
// RomlensKit: the generated Swift bindings over the Rust core, wrapped
// around the RomlensFFI XCFramework. Both are produced by
// scripts/build-xcframework.sh (`make swift`) and are git-ignored.
import PackageDescription

let package = Package(
    name: "RomlensKit",
    platforms: [.macOS("27.0")],
    products: [
        .library(name: "RomlensKit", targets: ["RomlensKit"]),
    ],
    targets: [
        .binaryTarget(name: "RomlensFFI", path: "RomlensFFI.xcframework"),
        .target(
            name: "RomlensKit",
            dependencies: ["RomlensFFI"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(name: "RomlensKitTests", dependencies: ["RomlensKit"]),
    ]
)
