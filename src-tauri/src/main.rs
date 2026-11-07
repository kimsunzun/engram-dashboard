// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// ADR-0132: this exe does not take a verb through argv — if a verb branch is put before GUI init, the window does not appear when the first argument
// collides with a GUI argument, and that collision would have to be managed every time verbs grow. The control CLI is owned by the `engram` executable.
fn main() {
    engram_dashboard_lib::run()
}
