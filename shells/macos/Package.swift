// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "PiqaeMenu",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "PiqaeMenu", targets: ["PiqaeMenu"]),
        .executable(name: "PiqaePrintCoreReplay", targets: ["PiqaePrintCoreReplay"]),
    ],
    dependencies: [
        .package(name: "PiqaeNodeKit", path: "../../sdk/apple"),
        .package(
            url: "https://github.com/sparkle-project/Sparkle",
            exact: "2.9.2"
        ),
    ],
    targets: [
        .target(
            name: "PiqaeMenuCore",
            dependencies: [
                .product(name: "PiqaeNodeKit", package: "PiqaeNodeKit"),
            ]
        ),
        .target(name: "PiqaeProfileHost", dependencies: ["PiqaeMenuCore"]),
        .target(
            name: "PiqaePrintCoreReplayCore",
            dependencies: ["PiqaeMenuCore", "PiqaeProfileHost"]
        ),
        .executableTarget(
            name: "PiqaeMenu",
            dependencies: [
                "PiqaeMenuCore",
                "PiqaeProfileHost",
                .product(name: "Sparkle", package: "Sparkle"),
            ]
        ),
        .executableTarget(
            name: "PiqaePrintCoreReplay",
            dependencies: ["PiqaePrintCoreReplayCore"]
        ),
        .testTarget(name: "PiqaeMenuCoreTests", dependencies: ["PiqaeMenuCore"]),
        .testTarget(
            name: "PiqaeProfileHostTests",
            dependencies: ["PiqaeMenuCore", "PiqaeProfileHost"]
        ),
        .testTarget(
            name: "PiqaePrintCoreReplayCoreTests",
            dependencies: ["PiqaeMenuCore", "PiqaeProfileHost", "PiqaePrintCoreReplayCore"]
        ),
    ]
)
