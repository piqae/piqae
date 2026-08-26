// swift-tools-version: 5.10
import PackageDescription

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
        .target(name: "PiqaeNodeKit"),
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
