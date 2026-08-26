// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "PiqaeNodeKitConsumerFixture",
    platforms: [
        .iOS(.v16),
        .macOS(.v13),
    ],
    products: [
        .library(name: "ConsumerFixture", targets: ["ConsumerFixture"]),
    ],
    dependencies: [
        .package(name: "PiqaeNodeKit", path: "../.."),
    ],
    targets: [
        .target(
            name: "ConsumerFixture",
            dependencies: [
                .product(name: "PiqaeNodeKit", package: "PiqaeNodeKit"),
            ]
        ),
    ]
)
