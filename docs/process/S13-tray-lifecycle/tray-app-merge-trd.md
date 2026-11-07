# TRD — merging the tray into the Tauri app (ADR-0026, 2 processes)

Written 2026-06-19 dashboard9. Implementation design for ADR-0026. The previous `tray-topology-trd.md` (3 processes) was discarded together with ADR-0023 — this document replaces it.

## 0. Scope / non-scope
**Scope (this time):** remove the separate tray process (`engram-tray-host`) → bring the tray inside the Tauri app. Local control plane only:
- Tauri v2 native tray (icon · menu · status color).
- Tray menu → daemon ensure/stop (existing discovery), window show/hide (inside the process), full quit.
- X = hide (prevent_close). autostart (`--hidden`). single-instance (hidden raise).

**Non-scope (next):** data-plane flip (UI↔daemon WS attach), remote · multiple-daemon profile/context, updater. Agents stay embedded this time too (the daemon is only a lifecycle target, not yet an agent host — before the flip).

## 1. Target structure (2 processes)
```
로컬 앱 (engram-dashboard.exe — Tauri v2, 1 프로세스, autostart 상주)
├─ 트레이(네이티브)  ─┐
├─ main 창(WebView) ─┤  같은 프로세스 → 창 제어 = window.show()/hide() (IPC 없음)
├─ agent-tree/slot-popup 창(hidden) ─┘
├─ Rust: tray 모듈(의도→command) · commands(daemon_*) · embedded_carrier(에이전트)
└─ discovery(ensure/send_stop/status, WMI/WS) ── 로컬 제어 평면

데몬 (engram-dashboard-daemon.exe — detached, WMI spawn, 앱과 독립 생존)
  └─ 향후 에이전트 호스트(flip). 지금은 lifecycle 대상.
```

## 2. Module placement (src-tauri)
New `src-tauri/src/tray/` (or `tray.rs`):
- `mod.rs` — TrayIcon creation / menu wiring / event handler (Tauri `tray::TrayIconBuilder`, `menu::{Menu,MenuItem,PredefinedMenuItem}`). Created once in setup().
- `core.rs` — **pure logic kept from engram-tray-host's core.rs and moved over**: `MenuAction` (intent enum) · `menu_id`/`label` · `action_for_menu_id` · `IconState` · `icon_state_for` · `to_grayscale_rgba`. The unit tests move with it. (The Launcher/DaemonProbe traits and dispatch are unnecessary in the app — the tray handler calls the command directly. Simplification.)
- The two icon sets (color/gray) are embedded with `include_bytes!` and included in the app build (existing method kept).

## 3. Core behavior design
- **Menu 6→simplified:** 데몬 켜기 (turn daemon on) / 데몬 끄기 (turn daemon off) / UI 보이기 (show UI) / UI 숨기기 (hide UI) / ── / 완전 종료 (full quit). ("트레이 종료" (quit tray) is meaningless after the merge — the tray is the app, so the tray alone cannot be turned off. Only "완전 종료".)
- **Window show/hide:** `app.get_webview_window("main")` → `show()`+`unminimize()`+`set_focus()` / `hide()`. **In-process call, no IPC.**
- **X = hide:** in the main `CloseRequested` of `on_window_event`, `api.prevent_close()` + `window.hide()`. (Replaces the current `app.exit(0)` — ADR-0026.) The only real quit is the `app.exit(0)` of the tray's "완전 종료".
- **Daemon on/off:** call the existing `discovery::ensure_daemon` (WMI) · `send_stop` (WS). **Because it is a WMI spawn, the daemon is a child of WmiPrvSE = does not inherit the app Job = detached/breakaway is satisfied automatically (ADR-0024 C1 kept).** Never spawn the daemon directly with `std::process::Command`.
- **Avoiding blocking:** ensure/send_stop block for several seconds (WMI polling/WS). Calling them directly from the tray event handler (main thread) freezes the tray and window → send them off with `tauri::async_runtime::spawn` (or spawn_blocking), and update the status color with `tray.set_icon()` after completion. Mind that set_icon has a main-thread constraint (post to main via AppHandle). (Replaces tray-host's worker + collect pattern with Tauri async — simpler.)
- **Status color:** daemon alive = color / dead = gray. Updated with `discovery::daemon_status().alive` right after an action. Periodic detection of external/crash death is non-scope (absorbed by the persistent connection at the flip).

## 4. Plugins (new dependencies)
- `tauri-plugin-single-instance` v2 — **register first of all**. Callback `|app, argv, cwd|`: if argv has `--hide` (= another trigger wants to hide), hide; otherwise main window `show()+unminimize()+set_focus()`. **Known pitfall of raising a hidden window:** set_focus alone is blocked by Windows focus-stealing prevention and may only flash the taskbar → the show→unminimize→set_focus order is mandatory, and if that is not enough, temporarily toggle `always_on_top`.
- `tauri-plugin-autostart` v2 — start at login. A `--hidden` argument on boot launch → detected in setup() so the main window is not shown (tray only). A user's direct launch shows the window.
- `tauri = { features=["tray-icon", "image-ico"] }` — native tray.

## 5. Rollback (removal) procedure
1. Move the `core.rs` pure logic → `src-tauri/src/tray/core.rs` (including tests).
2. Delete `crates/engram-tray-host/`.
3. Remove `"crates/engram-tray-host"` from the root `Cargo.toml` members.
4. `cargo build` (root) → regenerate Cargo.lock, confirm green.
5. `rg engram[-_]tray[-_]host` → confirm 0 code references outside docs/handoff.

## 6. Invariants (kept during implementation)
- Daemon spawn = discovery (WMI) only. The daemon is not put in the app Job (detached/breakaway, ADR-0024 C1).
- A tray action = the Tauri command path (§5 — the LLM calls the same command surface). The native tray popup is not a cdp DOM → verify with cdp through commands/state (do not expect direct clicks).
- Do not route local control (window/tray/daemon lifecycle) through the remote data channel (ADR-0026 plane separation).
- Keep core.rs pure (no OS/Tauri dependency — the target of unit tests).

## 7. Tests / verification
- **Unit (moved):** MenuAction id mapping · icon state · grayscale (the existing tray-host tests as is).
- **build/test:** `cargo build` green, `cargo test` green, `rg "use tauri" crates/engram-dashboard-core/src/`=0.
- **GUI measurement (cdp):** window show/hide invoke; after X=hide the window disappears and the process survives; return with "UI 보이기". Tray menu clicks are manual (native popup) — instead verify by invoking the tray commands directly.
- **spike (ADR-0026, measured before/during implementation):** ① WebView2 hidden memory, ② hidden raise focus-stealing, ③ daemon reconnect during an updater restart (non-scope but a memo), ④ daemon rediscovery after an app crash.

## 8. Implementation execution (CLAUDE.md conventions)
Separate subagents: coder (opus) → reviewer-deep (or fable) → QA (build/test + cdp measurement). Rollback removal + rebuild in staged commits (① rollback removal green, ② tray + X=hide, ③ single-instance + autostart).
