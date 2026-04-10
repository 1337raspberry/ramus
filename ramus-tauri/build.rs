fn main() {
    tauri_build::build();
    // libmpv is loaded at runtime via libloading (see src/mpv_ffi.rs),
    // so there's nothing for build.rs to link against.
}
