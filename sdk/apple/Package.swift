// swift-tools-version: 5.10
import PackageDescription
import Foundation

let nativeArtifactPath = ".artifacts/PiqaeNode.xcframework"
let hasNativeArtifact = FileManager.default.fileExists(
    atPath: URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .appendingPathComponent(nativeArtifactPath).path
)

var nodeKitDependencies: [Target.Dependency] = ["CPiqaeNodeABI"]
if hasNativeArtifact { nodeKitDependencies.append("PiqaeNodeNative") }

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
            publicHeadersPath: "include"
        ),
        .target(name: "PiqaeNodeKit", dependencies: nodeKitDependencies),
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
        .binaryTarget(name: "PiqaeNodeNative", path: nativeArtifactPath)
    )
}
