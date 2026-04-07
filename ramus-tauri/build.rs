fn main() {
    tauri_build::build();

    // Link libmpv via pkg-config
    let lib = pkg_config::probe_library("mpv").expect(
        "libmpv not found. Install mpv: brew install mpv (macOS), apt install libmpv-dev (Linux)",
    );
    for path in &lib.link_paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
}
