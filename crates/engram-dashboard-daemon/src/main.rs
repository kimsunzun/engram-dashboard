//! engram-dashboard-daemon — daemon entry point (thin).

// ★Console visibility — depends on build mode (why cfg_attr)★:
//   Only release uses the windows subsystem (no console window). Debug attaches no attribute and stays a **console app** →
//   when discovery launches the daemon via WMI a console window comes up with it so daemon logs (RUST_LOG) can be seen in that window (dev convenience).
//   discovery WMI (Win32_Process.Create) rejects CREATE_NO_WINDOW with RV=21 (measured in ADR-0021) so it **cannot make a console app
//   windowless**. So release's "no window" is achieved only by this subsystem attribute (not a WMI flag).
//   User decision (2026-06-19): debug = with window (logs), release = without window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tokio::main]
async fn main() {
    if let Err(code) = engram_dashboard_daemon::run().await {
        std::process::exit(code);
    }
}
