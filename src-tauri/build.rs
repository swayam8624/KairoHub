// TEACHING MAP — src-tauri/build.rs
// Rust cargo/Tauri build script invoked during compilation to generate host
// bindings/assets. It does not perform project/asset conversion in the user's working
// directory.
// See READING_ORDER.md for the Tauri/TypeScript and runtime ownership flow.

fn main() {
    tauri_build::build()
}
