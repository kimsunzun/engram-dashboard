# S13 — tray/process topology TRD (draft for review)

Written 2026-06-18. Synthesis of PRD opinions (user conversation) + `/consult` 3-model cross-validation (job `20260618-151957-consult-tray-daemon-topology`). **Awaiting user review before implementation.** Major decisions are recorded as ADRs once finalized (no ADR written yet).

## 0. Decision summary (what/why)

**3-process topology — but tray-host is a pure Rust tray utility with no WebView.**

```
[tray-host]  순수 Rust(tray-icon + tao, WebView 없음). 시작프로그램 등록. 런처+제어 유틸.
   ├─ ensure/launch ─▶ [daemon]  detached, self-owned. 에이전트 보유. WebView 없음.
   └─ launch/show/hide/kill ─▶ [UI 앱]  Tauri(React). 누를 때만 WebView 뜸.
```

- **Why 3 processes (separating tray-host) is justified:** the purpose of tray-host is not "crash survival" but a **login-resident launcher/control utility** (bring the daemon and UI up, hide them, remove them + observe status). This matches exactly the "justification condition for tray-host" stated by the cross-validation (GPT, Gemini): "a pure Rust tray that does not use a WebView / avoiding the WebView cost at login". The heavy WebView2 is not started at login, and the UI is spawned only when needed.
- **What the cross-validation opposed** was the plan of "running one more Tauri app (WebView) as tray-host" → that is not adopted (pure Rust tray instead).

## 1. Mandatory fixes raised by the cross-validation (mode-independent, judge-verified)

| # | Fix | Discarded wrong answer |
|---|---|---|
| C1 | **The daemon is not a child of anyone's Job (detached, self-owned).** Whoever starts it, tray-host or the UI, its lifetime is independent. | "daemon = child of tray-host + KILL_ON_JOB_CLOSE" → if tray-host dies the daemon dies too, violating survival (Gemini's wrong answer) |
| C2 | **Liveness detection = WS connection liveness (primary) + lockfile{PID, port, generation/nonce} (rediscovery · stale decision).** On rediscovery, **check PID alive · port responding · generation match, and if the lockfile is stale clean it up and do a fresh spawn** (prevents mistaking a zombie lock, consult race 5). | "no polling needed with only parent-child exit signals" → on a tray-host restart the handle is lost and rediscovery is impossible (Gemini's exaggeration) |
| C3 | **The daemon desired-state is owned by the owner (tray-host). The UI must not ensure directly, only connect.** The current UI bootstrap ensures the daemon → refactor. | "the UI spawns the daemon directly" → multiple-ensure race · the orphan-deciding subject is lost |
| C4 | **Full-quit order: block ensure/spawn first → daemon graceful drain (ack, timeout + Job forced fallback) → UI exit → tray-host exit.** | "UI exits first" → a re-spawn race in between (the reason automatic restart was discarded recurs) |

Crash automatic re-launch (supervision) is **deferred** — a manual "UI 열기" is already the recovery path. An ADR if it becomes frequent.

## 2. Process responsibilities

### tray-host (new, pure Rust)
- Startup-program entry point. Owns the tray icon.
- **lifecycle owner**: daemon ensure (detached spawn) · stop, UI launch/show/hide/kill, full-quit coordinator.
- Holds the daemon desired-state (Running/Stopped/Stopping).
- Status observation: lockfile + port-ping (v1) / WS control (future). Reflected in the menu as "데몬: 실행 중/꺼짐".
- **Spawns the daemon and UI detached** (not put in its own Job) → both survive if tray-host dies.

### daemon (existing crate, minimal changes)
- Holds the agent core. detached, self-owned.
- Singleton lock (lockfile: PID · port · generation/nonce). No-op if already alive, no matter who ensures.
- Orphan prevention: idle self-shutdown (0 connections ∧ 0 agents lasting N seconds). tmux model (ADR-0021).
- graceful stop: reject new commands → clean up PTY children (ADR-0001 causality) → remove lock → exit. After an upper-bound timeout, force with the Job.

### UI app (existing Tauri, changed)
- React window. Pure I/O.
- At boot **does not ensure the daemon directly** (C3) — it only **connects** to the daemon the owner started. On connection failure, it asks the owner to ensure (or the owner has already guaranteed it).
- X button = minimize (hide, keep the process). Single instance.
- On receiving the "full quit" intent (reason=full_shutdown), stops reconnect/ensure.

## 3. Tray menu (v1)
- **데몬 시작** (start daemon) — the owner ensures a detached daemon (in the background without the UI).
- **UI 열기** (open UI) — show/focus if the UI is alive; spawn if not (first run / after the UI exited).
- **UI 종료** (quit UI) — terminates only the UI process (daemon survives). ※ Different from X=hide.
- **전체 종료** (full quit) — brings the daemon + UI + tray-host all down in the order of §1 C4.
- (Finer-grained items are added later — the tray works like `add(command)`.)

## 4. Command surface (§5 hands/brain)
- Execution logic = **backend-side handles**. Lifecycle handles (launch UI · ensure daemon · full quit) are **owned by tray-host**, agent handles (spawn/kill/write) are **owned by the daemon**.
- Callers: tray menu · UI button · LLM · (future) shortcuts — all call the same handles.
- **[Open item]** The IPC path by which the UI/LLM calls tray-host's lifecycle handles (does tray-host open a control endpoint, or via the daemon?). To be settled in the detailed TRD.

## 5. Lifecycle sequences

**Boot (login):** tray-host starts → only the tray is shown (WebView 0). (Optional) whether to auto-ensure the daemon = a setting. v1 default: the daemon is ensured at "데몬 시작"/"UI 열기".

**Open UI:** tray-host → is the UI process alive? → if so show/focus, if not spawn → the UI connects to the daemon (if absent the owner ensures first).

**Window X:** UI hide (process kept). Unrelated to tray-host.

**Quit UI (tray):** the owner terminates the UI process. Daemon survives.

**Full quit (tray):** owner state=Stopping → block ensure/open/start → full_shutdown (reconnect=false) to the UI → daemon graceful (ack, timeout) → confirm daemon exit → UI exit → tray-host exit.

## 5b. Forced-termination/orphan coverage matrix
tray-host is not the OS parent but a third party (sends commands). So on forced termination:

| What was killed | Result | Coverage |
|---|---|---|
| tray-host force-kill (Task Manager) | daemon and UI survive (detached). The graceful command does not go out = intended | on relaunch, **re-adoption** via the lockfile (C2). If the daemon is idle it self-shuts down (C1) |
| UI force-kill | daemon survives. Unrelated to tray-host | respawn via the tray "UI 열기" → reconnect |
| daemon force-kill | the UI connection is cut ('down'). No automatic restart | revived only by an explicit "데몬 시작" (ADR-0021). The lockfile is stale → the next ensure cleans it up |
| everything force-killed, only the lockfile remains | — | the next launch decides stale by the PID/port/generation check → clean up → fresh (C2) |

**Principle:** a daemon remaining after a forced termination is not a bug but the "work survives" design. But with (a) re-adoption catching it again, (b) self-clean if idle, (c) a stale lock reclaimed by the check — these 3 keep it from becoming a Discord-style "unmanageable zombie". A daemon holding live agents is intentionally persistent (reconnect by relaunching; to really turn it off, "전체 종료"/Task Manager).

## 6. Module/file outline (expected)
- **New crate/bin** `engram-tray-host` (or a separate bin outside src-tauri): tray-icon + tao, launcher, lockfile observation, lifecycle handles.
- **daemon**: add generation/nonce to the lockfile (C2), idle self-shutdown (C1 reinforcement), graceful drain cleanup (C4).
- **UI (src-tauri/lib.rs)**: CloseRequested → hide + prevent_close (reverses the current `app.exit(0)`, adjusts 664d629). Remove the daemon ensure from bootstrap (C3) → connect only.
- **Frontend (clientFactory/App.tsx)**: remove/move `bootstrapDaemonIfNeeded`, block reconnect on receiving full_shutdown.

## 7. Risks / cost (cross-validation (c))
The cost of changing the entry point from the app to tray-host is real and must not be underestimated:
- Autostart registration (path/arguments), single-instance guard (two layers: tray-host + UI), deep link (`engram://`) receipt · forwarding, updater (the always-on tray-host replacing itself), code signing (+1 binary).
- → Specify as migration items in the ADR.

## 8. Unresolved (decided at implementation start)
- **[Decided] Daemon auto ensure at boot:** default = **(b) on-demand** (ensure only at "데몬 시작"/"UI 열기", in the spirit of ADR-0021). **Auto-start (ensure the daemon right at login) is left only as a settings option** (default off).
- **[Half resolved] lifecycle handle IPC path:** **daemon control = the daemon's existing WS server + lockfile token** (tray-host sends StopDaemon etc. as an authenticated client; force is a taskkill fallback = same as the current `DaemonControl.stop`). **UI control (launch/hide/kill)** = handled separately by tray-host with an OS spawn + a small signal (path not settled).
- tray-host = pure Rust (tray-icon+tao) vs Tauri-tray-only (no window). The former is recommended (lightweight).
- Single instance / deep link / updater details (§7).

### Data location (data_dir) — decided
- **Default = the engram internal folder** (instead of the current `%APPDATA%\com.engram.dashboard`). Both embedded and daemon look at the same folder so the profile (agents.json) stays shared. **Persistence kept (not volatile).**
- **Location swappable:** promote the `resolve_data_dir()` override, currently marked "for testing", to an **official settings knob** → later (especially when security requires) it can be moved to an external folder with one line ("taking it out"). An installed-type is not planned but no path assumption is baked in.
- **Folder = `.engram-data/`** (repo root, registered in `.gitignore`). All runtime data such as daemon.json (token) and agents.json. Blocks the risk of committing the token to git.
- **Path resolution:** the daemon is a separate process (`target/...`) so finding the repo root by itself is awkward → **the launcher (tray-host) decides the absolute path of data_dir and injects it into the daemon and UI through the override knob** (linked to promoting the override). Single source for the path + easy later move outside.
- **embedded = persistent (not volatile)** — but the tray and daemon.json are created only in daemon mode (embedded has no daemon).

### Control mechanism (daemon "permission" model)
- **Cooperative control (graceful):** the daemon writes {port, token} into the lockfile at start = prior permission. tray-host authenticates with the token and sends commands, and the daemon handles them itself (clean up children → self-exit). Possible because the daemon exposes a control surface.
- **Forced control (force):** OS taskkill (no permission needed for the same user), but with an orphan risk → fallback only.

## 9. Next
User review → once finalized, **2 ADRs**: ① tray/process topology (3 processes, pure-Rust tray-host, adjusts 664d629) ② daemon ownership · liveness · termination model (C1~C4). Then the detailed TRD → coder.
