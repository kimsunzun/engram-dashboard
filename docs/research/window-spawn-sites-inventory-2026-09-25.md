# Inventory of every process-launch site that can open a window (2026-09-25)

- **Status:** survey complete · **unifying is on hold** (user decision 2026-09-25 — 「일단 스크립트에서 각각 하는걸로 만족하자. 나중에 어찌어찌」 (let's be content for now with each script doing it by itself; we'll work it out later somehow))
- **Method:** 1 light survey (code reading) + 3 main-session spot checks (the omitted startup info in `wmi_create_raw` · `CREATE_NO_WINDOW` on the two agent paths · `windows_subsystem` on the daemon and shell) — no adversarial review
- **Trigger:** while fixing the problem of the detached-run script's console window stealing focus (branch `v0.3.2/fix/hide-detached-console`), a request came: 「앱이 직접 띄우는 것도 전부 최소화 기본으로 — 한 곳에서 공용화는 안 되나」 (make everything the app launches directly minimized by default too — can't it be unified in one place?).

## Conclusion

- **In production code there are two Rust sites that can open a window, and one of them can steal focus.** Both are in `crates/engram-dashboard-discovery/src/lib.rs`.
  1. **Daemon WMI launch** (`wmi_spawn` → `wmi_create_raw`, ~l.1110-1275) — the debug daemon is a console subsystem (`daemon/src/main.rs:9` applies `windows_subsystem` only in release), so a console window normally appears. With `console=false`, the startup info is **not passed at all**, so there is no window-show value (l.1127·1152). Intent per the comment (「로그용」 (for logs) — user decision 2026-06-19). The focus theft is inferred from the script-side measurement (`scripts/run-detached.ps1` comment) — the daemon path itself was not measured. — likely
  2. **`taskkill` of the release app** (`TaskKiller` ~l.911, `daemon_stop`) — the release shell has no console (`src-tauri/src/main.rs:2`), so a console tool briefly opens a new console. An unobserved inference. — uncertain
- **The rest already have no window** — the PTY path (ConPTY) · the stdio path and the codex app-server path (`CREATE_NO_WINDOW` — `agent/src/transport/stdio.rs:95-96` · `agent/src/backend/codex/transport.rs:611`, each inlines the same constant). — likely
- **Scripts:** the wrapper consoles of the two detached-run scripts (`run-detached.ps1` · `launch-detached.ps1`) change to a minimized, inactive launch on this branch (the app window opens normally — see the "To unify" section below). vite uses `start /MIN` (`run-debug.bat` · `rebuild-run-debug.bat`). The other scripts inherit the caller's console, so there is no window.
- **Tests:** about 25 places launch real processes, but they inherit the cargo console so no window opens. The exception = the `#[ignore]` manual WMI tests (`real_wmi_spawn` · `real_wmi_spawn_flag_matrix`).

## To unify (shelved proposal — start here when reopening)

- **There is no complete single place** — PowerShell and Rust are two languages, so code cannot be shared. One place per language is possible. As far as we know there is no OS-wide setting (「새 콘솔은 늘 최소화」 (a new console is always minimized)). — likely
- **Rust daemon launch:** `wmi_create_raw` is already the only bottleneck — it always creates `Win32_ProcessStartup` and carries `ShowWindow = 7` (SW_SHOWMINNOACTIVE), and `CreateFlags` only when `console=true`. Zero call-site changes. Since WMI has a single consumer it does not meet `base` tenancy condition ①, so it goes in discovery.
- **Rust std `Command` sites (stdio · codex app-server · taskkill):** a helper of the `background_command()` kind in `base::platform` (Windows = `CREATE_NO_WINDOW`, elsewhere = no-op — ADR-0230). With two consumers (agent · discovery) it satisfies the tenancy condition, and since it is inside `platform` it is not a new tenant. ★**Stable std cannot pass `STARTUPINFO.wShowWindow`**★ — 「최소화·비활성」 (minimized · inactive) needs a direct `CreateProcessW` call, and for headless processes hiding (`CREATE_NO_WINDOW`) is the right meaning (inference).
- **Scripts:** put the WMI launch blocks of the two detached-run scripts into a single dot-source helper in `scripts/lib/`. The first display state of the Engram app window cannot be decided by the launcher (measured — `scripts/launch-detached.ps1` comment).

## What we do not know

- Whether the debug daemon console launched through WMI actually takes the foreground (no measurement of the daemon path) · whether the release app's `taskkill` flicker is actually observed · whether `start /MIN` can be delegated · activated when Windows Terminal is the default terminal · whether a ConPTY child ever opens a window.
