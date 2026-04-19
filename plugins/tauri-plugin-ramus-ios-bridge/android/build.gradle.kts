// Minimal Android library stub for the ramus iOS bridge plugin.
//
// The plugin's Rust side already uses the desktop no-op path on Android
// (see src/lib.rs `#[cfg(any(desktop, target_os = "android"))]`), so there's
// no Kotlin `MpvBridgePlugin` here yet — just an empty library that Gradle
// can resolve so the root app's `include ':tauri-plugin-ramus-ios-bridge'`
// succeeds. Replace this with a real Media3/MediaSession bridge when
// Android playback lands.

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
}
