// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "SpoolMenu",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "SpoolMenu", targets: ["SpoolMenu"]),
        .executable(name: "SpoolPrintCoreReplay", targets: ["SpoolPrintCoreReplay"]),
    ],
    dependencies: [
        .package(
            url: "https://github.com/sparkle-project/Sparkle",
            exact: "2.9.2"
        ),
    ],
    targets: [
        .target(name: "SpoolMenuCore"),
        .target(name: "SpoolProfileHost", dependencies: ["SpoolMenuCore"]),
        .target(
            name: "SpoolPrintCoreReplayCore",
            dependencies: ["SpoolMenuCore", "SpoolProfileHost"]
        ),
        .executableTarget(
            name: "SpoolMenu",
            dependencies: [
                "SpoolMenuCore",
                "SpoolProfileHost",
                .product(name: "Sparkle", package: "Sparkle"),
            ]
        ),
        .executableTarget(
            name: "SpoolPrintCoreReplay",
            dependencies: ["SpoolPrintCoreReplayCore"]
        ),
        .testTarget(name: "SpoolMenuCoreTests", dependencies: ["SpoolMenuCore"]),
        .testTarget(
            name: "SpoolProfileHostTests",
            dependencies: ["SpoolMenuCore", "SpoolProfileHost"]
        ),
        .testTarget(
            name: "SpoolPrintCoreReplayCoreTests",
            dependencies: ["SpoolMenuCore", "SpoolProfileHost", "SpoolPrintCoreReplayCore"]
        ),
    ]
)
