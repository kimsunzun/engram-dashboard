# ADR-0253: When a usage value changes it is pushed immediately — UsageLimitsUpdated and a per-cell revision

- 상태: 확정 (2026-09-27, basis: user decision + TRD §7 #9, amendments 3 · 4 are in §6 #22)
- 관련: TRD §7 #9 · §6 #22(assumption on the order of resetting revision) · §6 #23(the seventh DaemonEvents notification) · `src-tauri/src/daemon_client/events.rs` · `connection.rs` · step-log S21

## 맥락
When a usage value changes, we had to decide when to reflect it on the screen — pile it up in the pending map until the next request, or push it out as soon as it changes.

## 결정
When a value changes, **push it immediately** — a `UsageLimitsUpdated` event + a **`revision` per cell (company)**. The targets are **only the connections that subscribed to that company** (one outlet per connection — not an all-connection fan-out). The trigger is every time `revision` rises (picking is merged and sent at one per second per company, and one emit at the end is guaranteed). The frontend discards an update whose `revision` is smaller, and **forgets `revision` only when the shell socket marker (`socketEpoch`) grows** — it does not forget on a connection-state transition alone (leaving `connected`) (amendments 3 · 4 — TRD §6 #22 records those two holes).

## 거부한 대안
- **Apply on the next request (the old pending map)** — it takes until before the next request (up to 60 seconds) for a picked result to reach the screen.

## 근거
Reference precedents that merge a push source the moment it arrives (t3code · Orca · Codex TUI · CodexMeter — `docs/research/usage-limit-display-ui-survey-2026-09-26.md:89-108`) · user decision 2026-09-27 (「이왕 빨리 전달할 수 있으면 즉각즉각이 맞지싶어」) · TRD §7 #9.

## 영향 / 불변식
The seventh notification of `DaemonEvents` (`usage_limits_updated`) is the only exception to the head comment 「여섯 가지 전부 — 넓히지 말 것」 (`daemon_client/events.rs:68-70`) — we judged it is not the target of that warning because it is a pure 1:1 pass-through (it holds no interest state · socket marker · sink) (TRD §6 #23). Breaking the `revision` comparison · `socketEpoch` forgetting rule lets a stale value overwrite a new one in a reconnect race. Implementation commits: `7aad8f7`, `00e6683`, `088266e`.
