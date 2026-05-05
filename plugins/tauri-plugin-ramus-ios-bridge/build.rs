// No JS-invokable commands — the plugin is Rust-only. The empty slice
// still generates the permissions manifest so Tauri's capability
// resolver doesn't complain about a missing plugin.
const COMMANDS: &[&str] = &[];

fn main() {
    // `IPHONEOS_DEPLOYMENT_TARGET=17.5` is set in `.cargo/config.toml` —
    // see the comment there for why tauri-utils' iOS 13 default needs to
    // be overridden. Doing it via `[env]` rather than `std::env::set_var`
    // here ensures it propagates to subprocess invocations of swiftc.
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
