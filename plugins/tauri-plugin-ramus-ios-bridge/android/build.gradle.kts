// Android library module for the ramus iOS bridge plugin.
//
// Despite the "ios-bridge" name, this module also hosts the Android Kotlin
// `MpvBridgePlugin`. Audio playback runs on libmpv via the `dev.jdtech.mpv`
// AAR, wrapped behind a Media3 `SimpleBasePlayer` so the existing
// `MediaSession` / `MediaSessionService` / lock-screen controls keep
// working without changes. The `mpv*` IPC names in `RamusIosBridge` now
// describe the real engine on every platform.

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.ramus.iosbridge"
    compileSdk = 36

    defaultConfig {
        // dev.jdtech.mpv:libmpv 1.0.0 requires API 26.
        minSdk = 26
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

    // Media3 — `media3-common` carries `SimpleBasePlayer` + the `Player`
    // interface; `media3-session` provides `MediaSession` +
    // `MediaSessionService` (hosted by `MpvForegroundService`) which drive
    // the lock-screen / Bluetooth / Android Auto controls. ExoPlayer is no
    // longer a dependency — audio runs on libmpv via the `dev.jdtech.mpv`
    // AAR, exposed through `LibmpvSimplePlayer`.
    val media3 = "1.5.1"
    implementation("androidx.media3:media3-common:$media3")
    implementation("androidx.media3:media3-session:$media3")

    // libmpv for Android — universal AAR (arm64-v8a, armeabi-v7a, x86,
    // x86_64). Bundles mpv 0.41 + FFmpeg 8.1 + libass + libplacebo +
    // mbedtls. LGPL — dynamically linked, source available upstream.
    implementation("dev.jdtech.mpv:libmpv:1.0.0")
}
