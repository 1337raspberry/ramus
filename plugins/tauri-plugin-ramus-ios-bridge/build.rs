// No JS-invokable commands — the plugin is Rust-only. The empty slice
// still generates the permissions manifest so Tauri's capability
// resolver doesn't complain about a missing plugin.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
