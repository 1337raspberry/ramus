// swift-tools-version:5.9
//
// MPVKit ships pre-built libmpv + FFmpeg + libass xcframeworks via a Swift
// Package, so the Rust side doesn't need to cross-compile anything. The
// pin is exact: each MPVKit-lavfi release republishes one upstream MPVKit
// release, so moving it moves mpv and FFmpeg together with it.

import PackageDescription

let package = Package(
    name: "tauri-plugin-ramus-ios-bridge",
    platforms: [
        // Tauri's swift-rs build step compiles the package for macOS
        // to generate Rust bindings — even on an iOS-only target — so
        // the minimum here has to satisfy MPVKit's (v12) too, otherwise
        // SPM rejects the dependency resolution.
        .macOS(.v12),
        // The app's deployment target is 17.5 (project.yml); the visualiser
        // views use iOS 17 trait-change registration.
        .iOS(.v17),
    ],
    products: [
        .library(
            name: "tauri-plugin-ramus-ios-bridge",
            type: .static,
            targets: ["tauri-plugin-ramus-ios-bridge"]
        ),
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api"),
        // MPVKit rebuilt with the extra FFmpeg audio filters the spectrum
        // tap's filter graph needs; otherwise identical to upstream.
        .package(url: "https://github.com/1337raspberry/MPVKit-lavfi.git", exact: "1.0.0"),
    ],
    targets: [
        // The native full-screen visualiser: ridge and backdrop renderers,
        // the frame ring they read, and the view the plugin shows over the
        // web view. No Tauri or MPVKit dependency, so it tests on its own.
        .target(
            name: "RamusVisualiser",
            path: "RamusVisualiser",
            linkerSettings: [
                .linkedFramework("Metal"),
                .linkedFramework("QuartzCore"),
            ]
        ),
        .target(
            name: "tauri-plugin-ramus-ios-bridge",
            dependencies: [
                .byName(name: "Tauri"),
                .product(name: "MPVKit", package: "MPVKit-lavfi"),
                "RamusVisualiser",
            ],
            path: "Sources",
            linkerSettings: [
                .linkedFramework("AVFoundation"),
                .linkedFramework("MediaPlayer"),
                .linkedFramework("Security"),
            ]
        ),
        .testTarget(
            name: "RamusVisualiserTests",
            dependencies: ["RamusVisualiser"],
            path: "RamusVisualiserTests",
            resources: [.copy("Fixtures")]
        ),
    ]
)
