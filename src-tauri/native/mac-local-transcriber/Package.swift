// swift-tools-version: 6.1
import PackageDescription

#if compiler(>=6.2)
let fluidAudio: Package.Dependency = .package(url: "https://github.com/FluidInference/FluidAudio", exact: "0.18.0", traits: [])
#else
let fluidAudio: Package.Dependency = .package(url: "https://github.com/FluidInference/FluidAudio", exact: "0.18.0")
#endif

let package = Package(
    name: "ClovyLocalTranscriber",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "clovy-local-transcriber", targets: ["ClovyLocalTranscriber"])],
    dependencies: [fluidAudio],
    targets: [.executableTarget(name: "ClovyLocalTranscriber", dependencies: [.product(name: "FluidAudio", package: "FluidAudio")], path: "Sources")]
)
