# ADR-0346: Network failures retry without limit on a capped backoff; token and version refusals stop at the first refusal

- 상태: 확정 (2026-11-05, 근거: 사용자 결정 2026-11-05 「그냥 계속 재시도하는게 구현적으로 편한거지?」 · 「토큰 거절을 재시도하는건 그냥 버그 수정때문에 땜빵채운다는거잖아」 · 「ㅇㅇ 쭉 진행해」 + 조사 `docs/research/disconnect-banner-dismiss-reconnect-2026-11-05.md` (medium, codex FIX applied) + daemon code read)
- 관련: Amends ADR-0310 (결정 12 토큰 불일치 재시도와 결정 20 재연결 횟수 주입) · Amends ADR-0345 (결정 4 재시도 3회로 내림) · Amends ADR-0180 (결정 4 재연결 횟수 3회) · ADR-0287 (band stays closable) · ADR-0135 (daemon.json carries port and token) · `crates/engram-dashboard-transport/src/policy.rs` (`max_attempts`) · `crates/engram-dashboard-daemon/src/lib.rs` (`generate_token`, `bind("127.0.0.1:0")`) · `src-tauri/src/daemon_client/link/route.rs` (`refusal_message`) · TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` D14 · D20 · §6-2 U3–U5 · step-log S21

## 맥락

ADR-0345 put 「다시 연결」 on the disconnected band but kept the bounded budget (3 tries, ADR-0180 decision 4 · ADR-0310 decision 20). The band must stay closable (ADR-0287) and remembers a dismissal while the reason is unchanged, so closing it after the give-up left no visible way back.

The survey (medium) read six apps from source: all retry automatically; the four chat clients (Mattermost, Element, Zulip, Rocket.Chat) never give up and cap the backoff between 5 s and 300 s; none leaves the user with no way back after closing the notice; none gives up after a few seconds. The user chose to keep retrying (「그냥 계속 재시도하는게 구현적으로 편한거지? 이것저것 안따지니깐」).

That exposed the token case. ADR-0310 decision 12 made a token mismatch `Retryable` (spend budget, re-read the connection info) to cover a stale credential after an in-place `daemon.json` rewrite. With unlimited retries a genuinely wrong token would loop forever. The user rejected retrying a refusal to cover a race (「토큰 거절을 재시도하는건 그냥 버그 수정때문에 땜빵채운다는거잖아」). Reading the daemon: the token is generated once per daemon start (`generate_token`) and the port is a fresh ephemeral port (`bind("127.0.0.1:0")`); both are written together to `daemon.json`. A client holding the previous daemon's pair therefore hits a dead port — a connection failure, retried as network — not a token refusal.

## 결정

1. **Network-class failures retry without limit on the existing capped backoff** — cannot connect, handshake timeout, link loss, and connected-but-does-not-hold (flaps). Production no longer reaches `GaveUp(BudgetExhausted)`. Mechanism (main's call): the shell injects `max_attempts = u32::MAX`; the transport crate is unchanged.
2. **A token refusal is terminal, like a version mismatch** — stop at the first refusal, no retry, the band shows the cause. No separate fix for the stale-credential race: the code read above shows it surfaces as a connection failure. The rare leftover (the new daemon happens to receive the old port) is covered by the user pressing 「다시 연결」.
3. **「다시 연결」 is "try now"** (ADR-0345): while retrying it skips the current wait and resets the backoff; after a terminal refusal it starts a new connection. Because network failures no longer reach a give-up state, the button shows on the band while retrying too (main's call — follows Zulip "Try now", Rocket.Chat "Connect", VS Code "Reconnect Now"; this settles ADR-0345 decision 6).
4. **The 5 / 3 retry counts retire** — ADR-0180 decision 4, ADR-0310 decision 20 and ADR-0345 decision 4 no longer apply.
5. **Repeated identical failures are logged throttled**, not once per attempt (main's call; the rule is the TRD's).
6. **Not decided here:** whether a closed band shows again on the next failure (Zulip does).

## 거부한 대안

- **Bounded retries plus 「다시 연결」 in the tray menu as well** (VS Code / JupyterLab shape — bounded retries with a persistent menu action) — keeps a give-up state and adds a second surface; the user chose unbounded retries. 강도: 약함 — 사용자 직접 기각.
- **Retry a token refusal once after re-reading `daemon.json`** — a workaround for a race; the user rejected it, and the daemon code shows the race yields a connection failure, not a refusal. 강도: 코드.
- **Keep the token refusal `Retryable` on the budget (ADR-0310 decision 12 as written)** — with unlimited retries a wrong token loops forever; same race argument as above. 강도: 코드.

## 근거

- Survey `disconnect-banner-dismiss-reconnect-2026-11-05.md` (medium; source lines re-read by main; codex review FIX applied): Zulip `server_events.js:226` "Retry indefinitely on timeout."; Rocket.Chat `DDPSDK.ts:201` `retryCount: Infinity` (and its CHANGELOG moving away from "giving up after one retry"); Element keepalive forever; Mattermost `3000*n²` capped 300 s.
- Daemon: `crates/engram-dashboard-daemon/src/lib.rs` — step 3 `TcpListener::bind("127.0.0.1:0")`, step 4 `generate_token()`, step 8 writes `daemon.json` with both.
- Transport: `policy.rs:43` `pub max_attempts: u32` (consumer-injected, ADR-0177 decision 4); `machine.rs` `schedule_retry` gives up when `retries` or `flaps` reach it.

## 영향 / 불변식

- **Nothing on the local path retries a refusal.** If a future refusal kind needs retrying, it needs its own ADR — do not route it through the network backoff.
- **Premise verified (code read 2026-11-05):** every dial goes `LocalDaemonDialer::dial` → `resolve` → `EndpointSource::current()`, which reads `daemon.json` afresh (`endpoint.rs:150-153`) and sets the credential from that same read (`dialer.rs:153`); the `Busy` path reuses the last-known endpoint together with its own credential. If a later change caches the port or the token apart, decision 2's argument fails.
- The transport `Retryable` kind (close code 4006) loses its local producer; the daemon side (stage 3-3) sends a token mismatch as terminal. Close code 4001 is already shared with 3-3 U7's identity refusal — the refusal text must tell token, version and identity apart.
- Shell text (`link::route::refusal_message`) gains a token-refusal cause; the count constants (`connection.rs` `MAX_RECONNECT_ATTEMPTS`, the 「5회」 prose in `src/api/agentClient.ts` and `src/api/daemonControl.ts`) go away with the old driver (3-2 U4 / U5).
- Landing: U3 (driver: unlimited policy, terminal token refusal) · U5 (band 「다시 연결」 with ADR-0344's button) — the TRD records the final units.
