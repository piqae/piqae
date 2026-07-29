// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "SpoolMenu",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "SpoolMenu", targets: ["SpoolMenu"])],
    targets: [
        .target(name: "SpoolMenuCore"),
        .target(name: "SpoolProfileHost", dependencies: ["SpoolMenuCore"]),
        .executableTarget(
            name: "SpoolMenu",
            dependencies: ["SpoolMenuCore", "SpoolProfileHost"]
        ),
        .testTarget(name: "SpoolMenuCoreTests", dependencies: ["SpoolMenuCore"]),
        .testTarget(
            name: "SpoolProfileHostTests",
            dependencies: ["SpoolMenuCore", "SpoolProfileHost"]
        ),
    ]
)
