// swift-tools-version: 5.10
import PackageDescription
import Foundation

let nativeArtifactPath = ".artifacts/PiqaeNode.xcframework"
let nativeArtifactExists = FileManager.default.fileExists(
    atPath: URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .appendingPathComponent(nativeArtifactPath).path
)
let requiresNativeArtifact = ProcessInfo.processInfo.environment[
    "PIQAE_REQUIRE_LINKED_RUNTIME_TESTS"
] == "1"
if requiresNativeArtifact && !nativeArtifactExists {
    fatalError("PIQAE_REQUIRE_LINKED_RUNTIME_TESTS requires a built PiqaeNode XCFramework.")
}
let hasNativeArtifact = requiresNativeArtifact && nativeArtifactExists

var abiDependencies: [Target.Dependency] = []
var abiSettings: [CSetting] = []
if hasNativeArtifact {
    abiDependencies.append("PiqaeNodeNative")
    abiSettings.append(.define("PIQAE_NODE_HAS_NATIVE_ARTIFACT"))
}

let package = Package(
    name: "PiqaeNodeKit",
    platforms: [
        .iOS(.v16),
        .macOS(.v13),
    ],
    products: [
        .library(name: "PiqaeNodeKit", targets: ["PiqaeNodeKit"]),
        .library(name: "PiqaeNodeKitAirPrint", targets: ["PiqaeNodeKitAirPrint"]),
        .library(name: "PiqaeNodeKitUI", targets: ["PiqaeNodeKitUI"]),
        .library(name: "PiqaeNodeKitTesting", targets: ["PiqaeNodeKitTesting"]),
    ],
    targets: [
        .target(
            name: "CPiqaeNodeABI",
            dependencies: abiDependencies,
            publicHeadersPath: "include",
            cSettings: abiSettings,
            linkerSettings: [
                .linkedLibrary("bsm", .when(platforms: [.macOS]))
            ]
        ),
        .target(name: "PiqaeNodeKit", dependencies: ["CPiqaeNodeABI"]),
        .target(
            name: "PiqaeNodeKitAirPrint",
            dependencies: ["PiqaeNodeKit"]
        ),
        .target(
            name: "PiqaeNodeKitUI",
            dependencies: ["PiqaeNodeKit"]
        ),
        .target(
            name: "PiqaeNodeKitTesting",
            dependencies: ["PiqaeNodeKit"]
        ),
        .testTarget(
            name: "PiqaeNodeKitTests",
            dependencies: ["PiqaeNodeKit", "PiqaeNodeKitTesting"]
        ),
        .testTarget(
            name: "PiqaeNodeKitAirPrintTests",
            dependencies: ["PiqaeNodeKitAirPrint"]
        ),
    ]
)

if hasNativeArtifact {
    package.targets.append(
        Target.binaryTarget(name: "PiqaeNodeNative", path: nativeArtifactPath)
    )
}
