// Android library module for the ramus iOS bridge plugin.
//
// Despite the "ios-bridge" name, this module also hosts the Android Kotlin
// `MpvBridgePlugin` — Android plays audio via Media3/ExoPlayer (not libmpv)
// behind the same Rust IPC surface that the iOS Swift bridge exposes. The
// `mpv*` IPC names in `RamusIosBridge` are kept for cross-platform parity
// with the Rust trait; nothing on Android actually invokes libmpv.

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.ramus.iosbridge"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    implementation(project(":tauri-android"))
    implementation("androidx.core:core-ktx:1.13.1")

    // Media3 / ExoPlayer — Android playback engine.
    // `media3-exoplayer` is the core player; `media3-session` gives us
    // a `MediaSession` for free, which the OS uses for lock-screen,
    // Bluetooth, and Android Auto controls without further glue.
    val media3 = "1.5.1"
    implementation("androidx.media3:media3-exoplayer:$media3")
    implementation("androidx.media3:media3-session:$media3")
}
