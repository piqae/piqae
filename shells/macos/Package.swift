// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "SpoolMenu",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "SpoolMenu", targets: ["SpoolMenu"])],
    targets: [
        .target(name: "SpoolMenuCore"),
        .executableTarget(name: "SpoolMenu", dependencies: ["SpoolMenuCore"]),
        .testTarget(name: "SpoolMenuCoreTests", dependencies: ["SpoolMenuCore"]),
    ]
)
