// swift-tools-version: 6.0
// xtool SwiftPM project for the Dashdown iOS app.
//
// An xtool project declares exactly ONE library product (the app). Here that
// app target depends on `UniFFI` (the generated Swift bindings + the C FFI
// clang module), which in turn depends on the prebuilt Rust core packaged as an
// `.xcframework`.
//
// BOOTSTRAP: `Frameworks/libdashdown_core-rs.xcframework` and the generated
// `Sources/UniFFI/dashdown_core.swift` (+ the FFI header/modulemap) are produced
// by `./build-rust-ios.sh` and are gitignored. Run that script once before
// `xtool dev` / `xtool dev build`, or the package will not resolve.
import PackageDescription

let package = Package(
    name: "Dashdown",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        .library(name: "Dashdown", targets: ["Dashdown"])
    ],
    targets: [
        // Prebuilt Rust core (device slice), assembled by build-rust-ios.sh.
        .binaryTarget(
            name: "DashdownCoreRS",
            path: "Frameworks/libdashdown_core-rs.xcframework"
        ),
        // UniFFI-generated Swift bindings; `dashdown_core.swift` imports the
        // `dashdown_coreFFI` clang module that the xcframework exposes.
        // The uniffi-0.31 generated code targets Swift 5 semantics (its callback
        // vtable globals aren't Sendable), so compile this target in Swift 5
        // language mode to satisfy the package's Swift 6 tools-version.
        .target(
            name: "UniFFI",
            dependencies: [.target(name: "DashdownCoreRS")],
            path: "Sources/UniFFI",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        // The SwiftUI app.
        .target(
            name: "Dashdown",
            dependencies: [.target(name: "UniFFI")],
            path: "Sources/Dashdown"
        ),
        .testTarget(
            name: "DashdownTests",
            dependencies: [.target(name: "Dashdown")],
            path: "Tests/DashdownTests"
        ),
    ]
)
