# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile

# Tauri plugin IPC argument classes are deserialised by reflection via
# Plugin.parseArgs. R8/ProGuard will otherwise strip the fields and every
# @Command handler will fail at runtime with empty / defaulted args.
-keep @app.tauri.annotation.InvokeArg class * { *; }
-keepclassmembers @app.tauri.annotation.InvokeArg class * { *; }

# Media3/ExoPlayer uses SPI-style reflection to discover Renderers and
# Extractors. Without these keep rules the release APK builds but audio
# playback fails silently with "no suitable media source".
-keep class androidx.media3.** { *; }
-dontwarn androidx.media3.**