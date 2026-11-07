# Measurements before the stage 3 transport decisions (2026-10-13)

- Status: measurement record · no decision — of the unconfirmed items in TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` (5th edition), only those used in decisions were measured.
- Method: two lightweight workers (measurement · experiment in an isolated working copy). The app and the daemon were not launched. The contents of the token file were not read.

## M1. Origin header (→ D21)

| Client | Origin | Basis · confidence |
|---|---|---|
| Shell Rust client (connect · shutdown) | not sent | `connect_async` · `tungstenite::client` from a URL string — the only headers tungstenite 0.26.2 attaches are Host · Connection · Upgrade · Version · Key (read from registry source) · certain |
| `scripts/engram.mjs` (Node built-in WebSocket) | not sent | measured with Node v24.14.0 (upgrade header dump) · certain (for this edition only) |
| CLI `engram.exe` | not applicable | it does not use WebSocket (hand-assembled HTTP POST) · certain |
| Direct browser path `src/api/wsTransport.ts` (not production) | sent | a browser always sends it · expected value = deployed `http://tauri.localhost` · development `http://localhost:1420` (no `useHttpsScheme`) · not measured |

→ 「배포판은 Origin 붙은 업그레이드를 거절」 (D21 ①) does not break the production path. What breaks is only the direct browser path, which is not production.

## M2. Data folder permissions (→ D26 · D22)

- ★The development data folder `.engram-dev` (I: drive) and `daemon\run\daemon.json` inside it = by inheritance **Authenticated Users Modify (M) · Users Read (RX)**★ — another logged-in user on the same PC can read the token and change files (measured with `icacls` · certain).
- Cause = the `I:\` root inherits Authenticated Users Modify · Users Read to everything below (confirmed on a new folder). The `C:\` root also inherits Authenticated Users Modify to a new subfolder (the ACL was only read, nothing was written).
- Under `%LOCALAPPDATA%` = SYSTEM · Administrators · the user only (+ two unidentified domain SIDs have Modify — not identified).
- The release data root = `<exe 폴더>\data` → it takes the inheritance of wherever it was extracted (not measured, as there is no installed build).

→ The 「3단계에선 범위 밖」 recommendation in TRD D26 should be looked at again in light of the measurement that 「지금도 실제로 열린 구멍」.

## M3. Time to launch an agent (→ D15) — not newly measured, existing records

- Reactivation is usually about 2.2 s (GUI measurement — step-log · ADR-0206).
- codex first start (the first resume after a daemon restart) `initialize` about 9.7 s (step-log record).
- The upper bound of a stuck activation = 15 s (backstop — not a measured value).

→ The 8 s request deadline is shorter than the codex first start. This fits D15 ① (shared 30 s + measurement).

## M4. Switching the command fields to `Box<RawValue>` (→ 3-1 U1)

- In an isolated working copy, the six command bus fields of protocol (the four places of ADR-0272 decision 3) were changed to `Box<RawValue>` and `cargo test -p engram-dashboard-protocol` → lib 102 passed · 0 failed · all existing goldens byte-identical.
- What to change: one line of the `raw_value` feature of `serde_json` · two pairing-number functions parse the raw text once (a broken envelope = `None`) · mechanical fixes at 19 test places · no change in TS binding content (the existing `#[ts(type)]` suffices).
- ★Caution★: the path that comes in through `serde_json::Value` (`from_value`) does not preserve bytes (key sorting · escape normalization — the meaning is preserved). The shell `forward_daemon_command` receives `Value` — the frontend does not currently produce `Command`, so this is a latent path.
- Not tested: the known failure when RawValue is inside `untagged` · `flatten` · an internally tagged enum — none of the six fields is such a place.
