// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "SpoolMenu",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "SpoolMenu", targets: ["SpoolMenu"])],
    targets: [.executableTarget(name: "SpoolMenu")]
)
