fn main() {
    tauri_build::build();
    // libmpv is loaded at runtime via libloading (see src/mpv_ffi.rs); no linking needed here.
}
