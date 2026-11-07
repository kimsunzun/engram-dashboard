# ADR-0029: Remove the embedded (single) mode — unify on daemon-only (discards ADR-0027, partly cleans up 0020/0026)

- 상태: 확정 (2026-06-21, dashboard10 session — user decision) · 부분 폐기 by ADR-0134 (replaces the single-instance scope and the release data folder location)
- 관련: ADR-0020 (protocol + transport seam — removes the InProc carrier) · ADR-0026 (2-process topology — narrowed to daemon-only) · ADR-0027 (per-mode instance/data — **this ADR discards it**, Supersedes ADR-0027) · ADR-0028 (event bus) · ADR-0013 (tmux/Docker daemon-always reference) · `docs/process/step-log.md` dashboard10 · Amended by ADR-0134 (replaces the single-instance scope and the release data folder location)
- Scope: whether to keep both modes, embedded (in-process hosting) and daemon (separate-process hosting). **Remove embedded from the codebase and unify on daemon-only.**

## 맥락
In step 3, as we added the mode system (the `--mode` argument, `default_data_dir(mode)`, the tray/X/single-instance/data-location gates), `if mode == Daemon` **comparison branches multiplied across the code**. This breaks the project's rule 1 (abstract interface + swappable implementation, no comparison branches).

Cause analysis: the transport seam swaps the **data plane** cleanly, but embedded and daemon *differ not only in transport* — **the product form differs** (a VS Code-style per-folder in-process app vs a Docker Desktop-style resident service). So branches multiply in lifecycle/topology (instance scope · data location · tray · window close · autostart). A roadmap projection shows **nearly every future feature is daemon-native** (remote · mobile · multi-client · orchestration · agent persistence · layout persistence · updater) — embedded is a negative asset whose "unsupported/degraded" list keeps growing.

The reference architectures (tmux · Docker · ttyd, ADR-0013) are **all daemon-always** (the client attaches whether local or remote). An in-process mode like embedded has no precedent.

Safety check (recon): the daemon is **already a complete agent host** — the `ConnectionCore`+`AgentManager` of `crates/engram-dashboard-daemon` handle every variant of spawn/kill/write/resize/profile CRUD, and `tests/ws_e2e.rs` proves spawn→output→kill with 38 in-proc WS E2E cases + 6 real-process cases. The frontend daemon path (wsTransport) is also complete: discover→connect→spawn→output. The `connection superseded` warning is the intended supersede signal of the connect retrigger at boot (harmless). → **Removing embedded loses no agent feature.**

## 결정
**Remove the embedded mode and unify on daemon-only.**

- **The agent host = always the daemon** (a separate process). The app (Tauri) is the daemon's **resident client**: window · tray · local control + the daemon connection. In-process AgentManager hosting is removed.
- **The mode axis (embedded/daemon) disappears → absorbed into the daemon-location axis (local/remote).** "Local use" = an auto-spawned localhost daemon + a local WS. "Remote/mobile" = a remote daemon + WSS. **This variation is already the transport seam (the wsTransport URL)**, so it is not a new branch. The app always attaches to the daemon by WS.
- **Removal targets:** the frontend `inProcTransport` · `EmbeddedDaemonControl` · the `clientFactory` mode branch. In src-tauri, `embedded_carrier` · the in-proc AgentManager/store/tracker wiring · `AppState.{manager,embedded,mode}` · `shutdown_all`. The whole step-3 mode system (`AppMode` · `parse_mode` · `resolve_mode` · the embedded branch of `default_data_dir(mode)` · `set_mode` · the embedded mutex of `instance.rs` · the `__ENGRAM_MODE__` injection · the tray/X/single-instance/`--hidden` gates).
- **Simplification (branches disappear):** the tray **always on**, X **always hide** (resident), single-instance = the daemon global mutex + the app tauri-plugin registered unconditionally, `default_data_dir` with no argument (release = appdata / debug = repo-root), the `ENGRAM_DATA_DIR` override kept (test isolation — load-bearing).
- **The core (`engram-dashboard-core`) is kept** — the daemon depends on it as the host engine. Only the core re-import in src-tauri is removed.

## 거부한 대안
- **Keep both embedded and daemon + an AppModeRuntime seam (2 implementations)** — it tidies the branches behind a seam but is a permanent tax of maintaining two. By the roadmap only the daemon grows, so the asymmetry deepens. Since embedded is a negative asset, removing it is right rather than wrapping it in a seam.
- **Scattered `if mode ==` (the current step 3)** — violates rule 1. Every mode addition/feature addition means manual branches in N places. Rejected (this was the problem).
- **embedded-only (remove the daemon)** — impossible, because remote · mobile · multi-client · orchestration (the product vision · all of ADR-0013) are daemon-native.
- **Delete immediately now (without verifying the daemon)** — if embedded were the only working mode, the app would die. Avoided by the removal order (confirm the daemon is solid → delete) after the recon proved daemon hosting.

## 근거
- Rule 1 (swappable · no comparison branches) + ADR-0013 (daemon-always reference) + the roadmap's daemon-native bias. User decision.
- recon: the daemon = a complete host (proved by ws_e2e), the frontend daemon path is complete, connection superseded is harmless. Removal is safe.

## 영향 / 불변식
- **ADR-0027 is discarded entirely** — with one mode, "per-mode instance/data" is meaningless. (single-instance = daemon global, data_dir = no argument.)
- **The InProc carrier of ADR-0020 is removed** — the protocol + transport seam concept stays (now swapping local WS vs remote WSS). The single protocol · ProtocolClient · ts-rs contract as is.
- **ADR-0026 is narrowed to daemon-only** — 2 processes (app + daemon) · the local control vs remote data plane · tray merge · X = hide stay (now unconditional). Only the "embedded = an ordinary window app" branch disappears.
- ADR-0028 (event bus) · ADR-0021/0024/0025 (daemon lifecycle) · ADR-0001~0019 (core) remain valid.
- **The daemon is the single owner of data** — the risk of dual ownership by an in-proc store disappears at its source.
- Removal goes in the order **after daemon hosting is verified** (daemon solid → pin the frontend to daemon → delete embedded). The app keeps working at every step.
