// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "RamusMusicCore",
    platforms: [.macOS(.v15)],
    products: [
        .library(name: "PlexMusicCore", targets: [
            "PlexAPI", "Playback", "Cache", "Search", "GenreTree", "Models"
        ]),
    ],
    dependencies: [
        .package(url: "https://github.com/groue/GRDB.swift.git", from: "7.0.0"),
        .package(url: "https://github.com/krisk/fuse-swift.git", from: "1.0.0"),
        .package(url: "https://github.com/mpvkit/MPVKit.git", from: "0.41.0"),
    ],
    targets: [
        .target(name: "Models"),
        .target(name: "PlexAPI", dependencies: [
            "Models",
        ], linkerSettings: [.linkedFramework("Network"), .linkedFramework("IOKit")]),
        .target(
            name: "Playback",
            dependencies: [
                "Models", "PlexAPI",
                .product(name: "MPVKit", package: "MPVKit"),
            ],
            linkerSettings: [.linkedFramework("MediaPlayer")]
        ),
        .target(name: "Cache", dependencies: [
            "Models", "PlexAPI",
            .product(name: "GRDB", package: "GRDB.swift"),
        ]),
        .target(name: "Search", dependencies: [
            "Models", "Cache", "GenreTree",
            .product(name: "GRDB", package: "GRDB.swift"),
            .product(name: "Fuse", package: "fuse-swift"),
        ]),
        .target(name: "GenreTree", dependencies: [
            "Models",
            .product(name: "Fuse", package: "fuse-swift"),
        ]),
        // Tests
        .testTarget(name: "CacheTests", dependencies: ["Cache"]),
        .testTarget(name: "SearchTests", dependencies: ["Search", "Cache", "GenreTree"]),
        .testTarget(name: "GenreTreeTests", dependencies: ["GenreTree"]),
        .testTarget(name: "PlexAPITests", dependencies: ["PlexAPI"]),
        .testTarget(name: "PlaybackTests", dependencies: ["Playback", "Models"]),
    ]
)
