# ADR-0343: Version-mismatched daemon gets a user-pressed restart action with a graceful stop

- 상태: 확정 (2026-11-05, 근거: 사용자 위임 2026-11-05 「리서치해서 결정해」 + 조사 `docs/research/daemon-version-mismatch-restart-2026-11-05.md` (medium, 단독 family — cross-family 리뷰 생략은 사용자 합의))
- 관련: Amends ADR-0180 (거부한 대안 데몬 강제 종료 버튼) · Amends ADR-0178 (거부한 대안의 미결 재시작 버튼) · Amends ADR-0342 (결정 1 표시에 재시작 버튼 추가) · ADR-0158 결정 3 (no automatic kill — kept: the user presses it) · `crates/engram-dashboard-protocol/src/handshake.rs` (the "send your own compiled version" rule — exception below) · `crates/engram-dashboard-net/src/ws.rs` (token then version check) · `src-tauri/src/discovery/mod.rs` `daemon_stop` (identity-checked kill) · `src-tauri/src/daemon_client/stop.rs` (tray stop; `NoTarget` on mismatch) · TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` §6-2 U5 · step-log S21

## 맥락

After an update (portable zip over the old folder, no installer — nothing stops the old daemon), the old daemon keeps running with an older protocol version. The shell sees the mismatch in `daemon.json` and does not connect (ADR-0342 shows it on the existing connection-failure surface). The user has no working in-app way out: the tray 「데몬 끄기」 uses the normal handshake and returns `NoTarget` on a mismatch. The first message drafted for ADR-0342 pointed the user at that tray item, which does nothing.

The user rejected both "tell them to kill it in Task Manager" and "make the tray force-kill it" (2026-11-05: 「둘다 별로」) and delegated the decision: 「리서치해서 결정해」.

## 결정

1. **The version-mismatch message on the existing connection-failure surface carries one action: 「데몬 재시작」.** The user presses it; nothing runs automatically (ADR-0158 decision 3 holds). The message says, before the press, that the old daemon's running agents stop and can be resumed afterwards from the agent list. The same action has an LLM path (`__engramCmd`, CLAUDE.md 「LLM-우선 제어」).
2. **The action stops the old daemon gracefully first.** A stop-only client reads `daemon.json`, authenticates with the token and **the daemon's own advertised protocol version**, sends nothing but `StopDaemon`, and waits for the connection to close. Every shipped daemon (protocol v3–v7) accepts this frame shape and runs `shutdown_all`.
   - This is a deliberate, single-purpose exception to the rule "send your own compiled version" (`handshake.rs`). The rule protects normal sessions from talking past a version gap; a connection that sends only `StopDaemon` has nothing to misread. No new privilege: the token comes from the same `daemon.json` every client reads.
3. **Fallback: identity-checked force kill** only if the graceful stop does not close within its deadline — kill exactly the process `daemon.json` names, checked by pid + start time through the same handle (no pid-reuse window).
4. **Then start the new daemon** the normal way (ADR-0021 `ensure` path) and connect.
5. **Lands in 3-2 U5** (the stop client moves onto transport there; the button is wired with U4's notice surface). Until then the message states the cause only.
6. **Follow-up for future daemons:** a version-independent stop frame decoded before the version check, so the echo exception is not needed for daemons shipped after it.

## 거부한 대안

- **Message only: "end `engram-dashboard-daemon` in Task Manager, then restart the app"** — works on every shipped daemon but skips `shutdown_all` and leaves the user to find the process. Rejected by the user 「둘다 별로」. 약함 — 사용자 직접 기각.
- **Tray 「데몬 끄기」 force-kills a mismatched daemon after consent** — the kill path already exists (`daemon_stop`), but it skips the daemon's own shutdown and ADR-0180 rejected exactly this button. Rejected by the user 「둘다 별로」. 약함 — 사용자 직접 기각.
- **Side-by-side generations (start the new daemon in a separate slot, let the old one drain)** — the only option that keeps old agents running, but the lock and the agent roster are per data folder, so the new daemon would start with an empty roster; a large redesign (kept as the long-term direction of ADR-0158). 근거 = 코드(per-data-folder lock and `agents.json`).
- **Explain only** — leaves the user stuck with no in-app way out (the reason this ADR exists). 근거 = 코드 (`stop.rs` `NoTarget`).

## 근거

- Graceful stop works on shipped daemons: the daemon checks the token, then `protocol_version == expected` (`net/src/ws.rs`, same order since v0.1.0); `daemon.json` holds the token and the daemon's version; `StopDaemon {force, kill_agents, request_id}` is unchanged since phase 0 and handled by `shutdown_all` in every shipped tag (research F3 — spot-checked).
- ADR-0180's four reasons against a kill button: (a) "a graceful stop cannot get through" — overturned by decision 2; (b) "all agents die" — still true, but every alternative also ends them, and they are resumable (`agents.json` keeps session ids; re-activation resumes with `claude --resume`; the running turn and in-memory mail are lost); (c) "the app cannot list what is running" — still true, so the message warns generically; (d) "the case is rare" — true, but when it happens the user has no in-app way out.
- Industry (likely — snippets): the side that replaces the binary restarts the daemon; version-agnostic control paths are kept apart from the versioned protocol.

## 영향 / 불변식

- 3-2 U5: build the stop-only client (decision 2) and the fallback (decision 3, handle-pinned identity check — promote the test-only `terminate_if_same` or equivalent into the `platform` crate); wire 「데몬 재시작」 on the notice surface + LLM command; GUI check: mismatched daemon → press → old daemon exits through `shutdown_all`, new daemon starts, app connects.
- The stop-only client must send nothing but `StopDaemon` after `Hello`; any other use of the echoed version is forbidden.
- Until U5, the 4001 / version-mismatch message states the cause only (no instruction that does not work).
- ADR-0342's message guidance ("suggest restarting the daemon") is now backed by an action instead of manual steps.
