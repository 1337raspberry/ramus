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

# Media3 uses SPI-style reflection to discover MediaSession callbacks +
# notification provider classes. SimpleBasePlayer also relies on Player
# interface methods being present even when subclasses override only a
# subset. Without these keep rules the release APK builds but lock-screen
# controls go missing.
-keep class androidx.media3.** { *; }
-dontwarn androidx.media3.**

# libmpv calls Kotlin observer + property methods from native code via
# JNI reflection. R8 doesn't see those callsites, so without an explicit
# keep the release APK silently SIGABRTs on the first property observer
# fire after libmpv emits its first time-pos event.
-keep class dev.jdtech.mpv.** { *; }
-keepclassmembers class dev.jdtech.mpv.MPVLib {
    void eventProperty(java.lang.String);
    void eventProperty(java.lang.String, long);
    void eventProperty(java.lang.String, double);
    void eventProperty(java.lang.String, boolean);
    void eventProperty(java.lang.String, java.lang.String);
    void event(int);
    void logMessage(java.lang.String, int, java.lang.String);
}
-dontwarn dev.jdtech.mpv.**