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
        // Matches the app's Info.plist deployment target in project.yml.
        // iOS 15 is the first version where Swift concurrency ships in
        // the OS; older targets make Xcode back-deploy concurrency into
        // the app bundle which broke launch on iOS 26 devices.
        .iOS(.v15),
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
        .target(
            name: "tauri-plugin-ramus-ios-bridge",
            dependencies: [
                .byName(name: "Tauri"),
                .product(name: "MPVKit", package: "MPVKit-lavfi"),
            ],
            path: "Sources",
            linkerSettings: [
                .linkedFramework("AVFoundation"),
                .linkedFramework("MediaPlayer"),
                .linkedFramework("Security"),
            ]
        )
    ]
)
