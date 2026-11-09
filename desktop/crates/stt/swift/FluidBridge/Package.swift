// swift-tools-version:5.10
// A tiny C-ABI wrapper around FluidAudio (Parakeet TDT v3 on CoreML / the Neural Engine),
// built as a static library and linked into the Rust `stt` crate (feature `fluid`).
import PackageDescription

let package = Package(
    name: "FluidBridge",
    platforms: [.macOS(.v14)],
    products: [.library(name: "FluidBridge", type: .static, targets: ["FluidBridge"])],
    dependencies: [.package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.18.2")],
    targets: [.target(name: "FluidBridge", dependencies: [.product(name: "FluidAudio", package: "FluidAudio")])]
)
