// swift-tools-version:5.9
//
// Logic shared by the ramus app and its home-screen widget extension:
// decoding the playback snapshot the app writes, planning the widget's
// timeline from it, and the waveform geometry. No UIKit, so the tests run
// on the Mac with `swift test`.

import PackageDescription

let package = Package(
    name: "RamusWidgetCore",
    platforms: [.iOS(.v17), .macOS(.v13)],
    products: [
        .library(name: "RamusWidgetCore", targets: ["RamusWidgetCore"]),
    ],
    targets: [
        .target(name: "RamusWidgetCore"),
        .testTarget(name: "RamusWidgetCoreTests", dependencies: ["RamusWidgetCore"]),
    ]
)
