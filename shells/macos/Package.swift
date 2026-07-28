// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "SpoolMenu",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "SpoolMenu", targets: ["SpoolMenu"])],
    targets: [.executableTarget(name: "SpoolMenu")]
)

