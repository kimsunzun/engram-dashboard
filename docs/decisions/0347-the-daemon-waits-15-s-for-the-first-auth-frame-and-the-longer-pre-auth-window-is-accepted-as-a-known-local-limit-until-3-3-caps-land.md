# ADR-0347: The daemon waits 15 s for the first auth frame, and the longer pre-auth window is accepted as a known local limit until 3-3 caps land

- 상태: 확정 (2026-11-06, 근거: 사용자 결정 2026-11-06 「ㅇㅇ 15초로 올려」 · 「a로 하고 계속 진행해」 + 조사 `docs/research/first-auth-frame-timeout-2026-11-06.md` (light) + 3-2 U3a review hold 1 + review of this change (deep — lead · risk-axis · codex, must 0))
- 관련: Amends ADR-0310 (결정 11 인증 시한 1초) · ADR-0346 (refusals are terminal; network failures retry) · ADR-0129 (net crate owns the WS server) · `crates/engram-dashboard-net/src/ws.rs` (`AUTH_TIMEOUT`) · `crates/engram-dashboard-daemon/tests/ws_e2e.rs` (`case03_auth_timeout_closes`) · TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` revision 9 ⑧ · §5 rows 3 · 12 · 14 · 17 · R11 · step-log S21

## 맥락

The daemon closes a new connection that sends no auth frame within `AUTH_TIMEOUT`, after an `Error("auth timeout")`. The value was 1 s. Git history shows it entered in the S12 design draft (`b6145f30`, 2026-06-14) with no recorded reason, outside the user Q&A that fixed that design (`a61b315f`); ADR-0310 decision 11 then carried "인증 시한 1 s" into the stage-3 plan.

It started to matter with ADR-0346: the shell now treats any handshake `Error` frame as a terminal refusal (the net daemon closes without a code, so the shell cannot tell an "auth timeout" from a token refusal). A shell stalled for more than 1 s right after the socket opened (heavy load, resume from sleep) would stop retrying until the user presses 「다시 연결」. The user asked who chose 1 s and whether it is conventional.

The survey read seven defaults from source: graphql-ws 3 s, Mattermost 5 s, SignalR `HandshakeTimeout` 15 s, EMQX `idle_timeout` 15 s, Mosquitto about 30 s (derived), socket.io 45 s, PostgreSQL 60 s — mostly 5–15 s. 1 s is stricter than all of them.

Review of the change (risk-axis lens) found the cost: the net daemon has no cap on pre-auth connections (TRD §5 14 "없음(구멍 3)"), and tungstenite reserves a declared frame length up to 16 MiB on a header alone. Held memory ≈ connection rate × window × 16 MiB, so a 15 s window makes exhausting the daemon's memory about 15× cheaper — up to an allocation abort that takes every agent with it. Only a raw-socket process of another OS user on the same PC can do this: a same-user process can read the token, and a browser page is refused at the Origin check or cannot leave a half-sent frame.

## 결정

1. **`AUTH_TIMEOUT` = 15 s** on the net daemon now, and 15 s for the transport daemon's accepted peers in 3-3 (U4 · U5) — replaces "인증 시한 1 s" in ADR-0310 decision 11. A handshake `Error` stays terminal on the shell side (ADR-0346, unchanged).
2. **The longer pre-auth window is accepted as a known local limit** (option A): another OS user on the same PC can hold pre-auth connections 15× longer and can exhaust the daemon's memory. This extends R11 (which recorded only "fill the accept slots"); it is resolved when 3-3 U4's connection caps land. No cap is added to the net daemon now.

## 거부한 대안

- **Keep 1 s** — stricter than every surveyed default, and with ADR-0346 a briefly stalled shell becomes a terminal refusal. 강도: 조사 · 코드.
- **Add a pre-auth connection cap to the net daemon now** (option B) — the only attacker is another OS user on the same PC, already outside the threat model (ADR-0310 decision 7), and 3-3 replaces this server with the transport daemon, whose caps (TRD §5 14) cover it. 강도: 약함 — 사용자 직접 기각.
- **A shorter window (e.g. 5 s) as a middle ground** — not offered; the shell's own handshake timeout (old driver `HANDSHAKE_TIMEOUT` 10 s, transport `connect_timeout` 10 s) is meant to end a stalled attempt as a network failure before the daemon's window, which needs the window above 10 s. 강도: 코드.

## 근거

- Survey `docs/research/first-auth-frame-timeout-2026-11-06.md` (main re-read graphql-ws `connectionInitWaitTimeout = 3_000`, `src/server.ts:613`; SignalR `HubOptionsSetup.cs:15`, EMQX `emqx_schema.erl:4273-4277` per the collector).
- Risk-axis review of this change: per-accept task spawn with no cap (`crates/engram-dashboard-daemon/src/lib.rs:382-400`), `accept_hdr_async` without `WebSocketConfig` (`ws.rs:328`), tungstenite 0.26.2 `max_frame_size` 16 MiB and reserve-on-header (`protocol/frame/mod.rs:181,195,228`); loopback bind, Origin check and constant-time compare unchanged.
- `case03_auth_timeout_closes` now asserts still open at 3 s and closed within 20 s (ran: closed at about 16 s).

## 영향 / 불변식

- **The shell's handshake bound must stay below the daemon's window** (old driver 10 s; transport `connect_timeout` 10 s), so a stalled shell usually ends its attempt as a retryable network failure; a stall after the socket opens and past 15 s can still meet "auth timeout" and stop as a refusal.
- 3-3 U4: size the connection caps (draft 8 · 8 · 64 — TRD §5 14) for a 15 s auth window, not 1 s.
- Do not restate the number in comments — name `AUTH_TIMEOUT` (the definition in `ws.rs` is the one place that holds it).
