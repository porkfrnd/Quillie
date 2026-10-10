// Tauri build script. Only runs when the `tauri-host` feature is enabled, so
// a plain `cargo build` on a machine with no webview development packages
// still works.
fn main() {
    #[cfg(feature = "tauri-host")]
    tauri_build::build();
}