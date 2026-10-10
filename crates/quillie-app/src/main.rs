// Prevents an extra console window on Windows in release builds (§2 targets
// Windows 10/11).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    quillie_app::run();
}