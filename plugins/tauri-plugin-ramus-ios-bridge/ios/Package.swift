// swift-tools-version:5.9
//
// MPVKit ships pre-built libmpv + FFmpeg + libass xcframeworks via a Swift
// Package, so the Rust side doesn't need to cross-compile anything. The
// version pin should move forward in tandem with Rust-side mpv API
// changes; 0.41 was the latest stable at the time we adopted this.

import PackageDescription

let package = Package(
    name: "tauri-plugin-ramus-ios-bridge",
    platforms: [
        // Tauri's swift-rs build step compiles the package for macOS
        // to generate Rust bindings — even on an iOS-only target — so
        // the minimum here has to satisfy MPVKit's (v11) too, otherwise
        // SPM rejects the dependency resolution.
        .macOS(.v11),
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
        .package(url: "https://github.com/mpvkit/MPVKit.git", from: "0.41.0"),
    ],
    targets: [
        .target(
            name: "tauri-plugin-ramus-ios-bridge",
            dependencies: [
                .byName(name: "Tauri"),
                .product(name: "MPVKit", package: "MPVKit"),
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
