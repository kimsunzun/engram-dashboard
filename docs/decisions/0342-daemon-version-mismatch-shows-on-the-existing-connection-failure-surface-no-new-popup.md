# ADR-0342: Daemon version mismatch shows on the existing connection-failure surface, no new popup

- 상태: 확정 (2026-11-04, 근거: 사용자 결정 2026-11-04 + 3-2 U2 landing survey (no popup surface exists, no TRD unit builds one))
- 관련: Amends ADR-0310 (결정 12 버전 불일치 즉시 팝업) · Amends ADR-0180 (결정 2 버전 거절 팝업 자격과 결정 5 즉시 팝업과 결정 6 버전 불일치 팝업 모양) · Amends ADR-0178 (결정 5 클라 팝업) · TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` §6-2 U2 (B landing note ①) · `src/components/layout/ConnectionNotice.tsx` (the existing surface) · ADR-0287 (notice bands on the overlay layer) · `src-tauri/src/daemon_client/link/dialer.rs` (local mismatch = Dialer `Refused` 4001, no dial) · `src-tauri/src/discovery/mod.rs` `DiscoveryError::VersionMismatch` · step-log S21

## 맥락

ADR-0178 decision 5 left the client one new thing to build: a popup for a daemon that rejects on version. ADR-0180 decisions 2, 5 and 6 made the version rejection a popup case (immediate, text + close, no action button), and ADR-0310 decision 12 (D20 ①) kept "Terminal, no budget spent, immediate popup" with a fixed local text per close code.

The 3-2 U2 landing survey (2026-11-04) found that the popup was never built: the generic popup component of ADR-0180 decision 3 does not exist, and no unit of the stage-3 TRD builds it, while 3-2 U4's GUI check ③ expected one.

A version mismatch happens in practice only after an update: the app is replaced while the old daemon keeps running (the daemon outlives the app because it owns the agents). Today the shell already detects it from `daemon.json` before connecting and the error text `protocol 버전 불일치: 데몬=X, 기대=Y` reaches the frontend's connection-failure display.

## 결정

1. **A daemon version mismatch is shown on the existing UI connection-failure surface. No new popup is built for it** (if the generic popup of ADR-0180 decision 3 lands one day, moving this case onto it is a new decision). The user's words (2026-11-04): 「그냥 UI 화면에 띄우면 안됨? 데몬에는 로그가 남겠지」 → 「클라에서는 UI가 있으니 표시되고」.
2. **The text rule of ADR-0310 decision 12 still holds on that surface:** the UI shows a fixed local message chosen by the close code (4001 = version mismatch); the peer's text never reaches the UI — it goes to the log only, masked.
3. **The classification and budget rules of ADR-0310 decision 12 do not change:** version mismatch = `Terminal`, no reconnect budget spent. Locally the shell's dialer refuses before dialing (`daemon.json` comparison → Dialer `Refused` 4001 → `GaveUp(Rejected)`), so no Auth frame is sent to a mismatched daemon.
4. **The shell logs the mismatch at warn** (daemon and expected versions, no secrets). On the local path the daemon never sees the connection, so it logs nothing; the daemon logs only the post-handshake 4001 path.

Only the popup parts of the amended clauses die: the "popup" in ADR-0310 decision 12, version rejection as a popup case in ADR-0180 decision 2, "immediate popup" for version rejection in ADR-0180 decision 5, the popup shape of ADR-0180 decision 6, and "the client shows a popup" in ADR-0178 decision 5. ADR-0180 decision 2's other popup case (reconnect budget exhausted) and decision 7 (its "다시 연결" button) are not touched by this ADR — what to show after budget exhaustion is still open with the user.

## 거부한 대안

- **Build the popup in 3-2 U4 (fixed-text modal, checked in GUI ③)** — the decision as recorded. Rejected by the user in favour of the existing surface: it adds a new UI component to a unit whose goal is replacing the connection layer, for a case that happens only after an update, while the existing display already names the cause. 약함 — 사용자 직접 기각.

## 근거

- Survey facts (3-2 U2, 2026-11-04): no popup/modal component in `src/` (ADR-0180 context measured the same), no frontend code handles `VersionMismatch` (`grep` over `src/` = 0), the shell's discovery already returns `DiscoveryError::VersionMismatch` whose text reaches the connection-failure display (`src-tauri/src/discovery/mod.rs:167`).
- Release daemon runs windowless in the background (`crates/engram-dashboard-daemon/src/main.rs:9`), so the app UI is the only place a user sees it; logs are under the data folder `logs\`.

## 영향 / 불변식

- 3-2 U2 C / U4: map the 4001 / `GaveUp(Rejected)` path to the existing connection-failure surface with the fixed local message; never put the peer's text in the UI.
- 3-2 U4 GUI check ③ checks the existing surface, not a popup.
- TRD passages that still say "popup" for version mismatch (§3-0 ⑤, §3-3, §3-6 D25 bullet, §5 6 and 19, §6-2 U2 body, §6-4 ③) are superseded by this ADR and the B landing note.
