// TEACHING MAP — src-tauri/src/main.rs
// Tauri desktop process entrypoint. Starts the Rust application shell;
// src-tauri/src/lib.rs registers commands and state consumed by the TypeScript UI.
// See READING_ORDER.md for the UI -> Tauri -> Kairo executable workflow.

fn main() {
    kairo_hub_lib::run();
}
