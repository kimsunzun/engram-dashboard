# ADR-0352: Stop leaves the shell link open so the daemon's shutdown broadcasts reach the app; stopping mode removed

- 상태: 확정 (2026-11-10, 근거: 사용자 결정 2026-11-10 「일단 땜질은 버리고」 · 「연결 안되는건 그냥 다시 연결하면 되잖아. 데이터적으로 중요한 문제 없으면 상관이없을것같은데」 + 회귀 관찰(트레이 데몬 끄기 뒤 에이전트 트리가 실행 중으로 남음))
- 관련: Amends ADR-0350 (결정 3의 stopping mode) · ADR-0021 (reconnect loop stays attach-only) · ADR-0026 (app exit stop) · `src-tauri/src/daemon_client/shutdown.rs` · `src-tauri/src/daemon_client/connection.rs` · `crates/engram-dashboard-daemon/src/link/server.rs`

## 맥락
ADR-0350 decision 3 made the shell's stop close its own link first and hold a "stopping mode" for the flight: no redials, attach-only `ensure` answering not-connected, explicit connect and tray 「데몬 켜기」 waiting the flight out, and routing holding the shown band. Landed in 3-2 U5a (`b85c0aaa`).

On a release build the user saw the agent tree keep agents as running (green dot) after tray 「데몬 끄기」. Before U5a the shell's main link stayed open during a stop, so the daemon's own shutdown broadcasts — each agent's exiting / terminal status, then the reaper's empty agent list — reached the frontend and the rows turned stopped. Closing the link first cut exactly those broadcasts. The stopping-mode machinery that close-first required had also grown large (≈ 500 lines with tests) for rare orderings.

## 결정
1. **A stop leaves the shell's main link open.** The daemon's shutdown broadcasts reach the frontend as before U5a; when the daemon exits, the link sees its normal disconnect and reconnects as it always does (the first failed reconnect shows band ①, ADR-0350 decision 1 / D1).
2. **Stopping mode is removed**: no `ensure` refusal during a stop, no connect / tray Start wait, no routing band hold during a stop, no separate stop-close kind. Tray 「데몬 켜기」 = launch then attach.
3. **Kept from ADR-0350 decision 3**: single flight with joiners; one absolute clock and the flight deadline / expiry; the target resolved once; the version-echo stop client; the "gone" check; the identity-checked kill for `User`; `Exit` waits ≤ 3 s and never kills; stop reasons and the `{outcome, reason}` reply; the attach-only re-add after a `User` stop (a link parked after a terminal version refusal does not redial by itself, so without it band ② would stay after the old daemon is gone).
4. **Accepted**: connection churn around a stop (an extra redial, a band a moment later, the state word reconnecting vs down). Only data loss, leftover processes and hangs count (user, 2026-11-10).

## 거부한 대안
- **Keep close-first and let the shell emit an empty agent list itself after a confirmed stop** — the user discarded it (「일단 땜질은 버리고」); review found it races real events: a delayed pre-stop roster could overwrite the reset, and a new daemon's roster could arrive first and be cleared.
- **Keep close-first unchanged** — the agent tree stays wrong after every stop (observed on a release build).

## 근거
- Investigation: at `v0.3.2` and before U5a the tray stop used a separate connection while the main link stayed open; the daemon's `shutdown_all` → `kill_agent` → status broadcasts and the reaper's `agent_list_updated` reached the shell's `emit_broadcast`. U5a's `Stopper` closed the link first, so they were dropped.
- Code review (Claude lead + codex, 2026-11-10, bar = data loss / leftover processes / hangs), all PASS:
  - the daemon drops its listener before `close_all` (`link/server.rs`), so the shell's redial after the drop is refused — no re-attach to an exiting daemon;
  - a command sent on the open link during shutdown cannot leave an orphan agent: `run()` runs `shutdown_all` a second time, and each agent's Job ends its processes when the daemon exits; profile writes are atomic;
  - a tray 「데몬 켜기」 during a stop cannot put two daemons on one data folder (the single-instance guard holds `daemon.json` for the daemon's life);
  - app exit stays bounded (≤ 4 s) and never kills.
- New test: during a `User` stop the link stays open and the daemon's shutdown broadcasts reach the frontend; it fails if the close at flight start comes back.

## 영향 / 불변식
- The stop must not close the shell's main link before the daemon is gone — otherwise the agent tree misses the daemon's shutdown broadcasts again (`shutdown.rs` module doc; the test above).
- The daemon must stop accepting before it closes connections (listener dropped before `close_all`).
- ADR-0350's other decisions hold: two bands, routing the single `ConnectionStatus` emitter, 「다시 연결」 starts the daemon, the one stop path, the daemon's `StopDaemon` fix, boot start in the main window. The TRD `trd-3-2-u5-stop-and-band.md` §3 R1 step 1 / 7, R2 stopping mode, R11, R16 and §4 R8 describe the removed mechanism.
