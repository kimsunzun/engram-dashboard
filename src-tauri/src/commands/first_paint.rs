use tauri::WebviewWindow;

/// Tells that the window webview has drawn the first screen (tab bar + active tab) — `invoke('webview_painted', { generation })`, once per
/// page load (`src/theme/firstPaint.ts`). `generation` = the adoption number that page received (`spare:adopted` body · `window_role` answer —
/// TRD S21-slot-docking §3-9 I26) · `null` or omitted if never received. If the shell built the window hidden and was waiting for it, show that window (if it is a window for which whether to show
/// has not yet been decided, keep the notification · an entrusted prewarmed window accepts only a notification that carries its adoption number), otherwise (reload ·
/// a prewarmed window before entrusting · already visible · a window whose waiting was withdrawn by hiding) do nothing — it cannot fail. It is wiring inside the webview, so it is not a bus
/// command. It is a synchronous command so it runs on the main thread (showing is the main thread's job — `crate::first_paint`).
// ADR-0299
#[tauri::command]
pub fn webview_painted(window: WebviewWindow, generation: Option<u32>) {
    crate::first_paint::reveal_painted(&window, generation);
}
