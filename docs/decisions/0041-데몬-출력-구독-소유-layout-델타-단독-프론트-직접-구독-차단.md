# ADR-0041: Ownership of the daemon output subscription = the layout delta alone (blocking direct frontend subscription)

- 상태: 확정 (2026-07-01, basis: S14 module ① first adversarial review BLOCK-1)
- 관련: CLAUDE.md 「아키텍처 원칙」 §5 · ADR-0035 · ADR-0037 · ADR-0040 · src-tauri/src/commands/agent.rs(forward_daemon_command) · src/api/protocolClient.ts · step-log S14

## 맥락
We had to settle **who owns** the output subscription (the wire `Subscribe`/`Unsubscribe` that tells the daemon "send this agent's output"). In the multi-view structure (ADR-0040) where there are several windows and they view the same agent at the same time, if the frontend, when it attaches an output renderer per window, also forwards `Subscribe{after_seq:null}` (FromOldest) to the daemon, then when N windows view the same agent the daemon retransmits the whole stream N times (a duplicate storm + confusion of progress and order). As an extension of ADR-0035 (layout authority = src-tauri) and ADR-0037 (transport semantics = Rust-only), subscription ownership needed to be pinned down.

## 결정
The daemon output subscription (wire `Subscribe`/`Unsubscribe`) is **owned solely by the src-tauri layout delta**.
- Only when the layout authority (`OutputRouter`, ADR-0035 SSOT) rebuilds the (window, agent) set does it send `Subscribe`/`Unsubscribe` to the daemon (`send_subscription_delta`).
- The frontend `ProtocolClient.subscribeOutput` **only registers a renderer** — it does not send `Subscribe` to the daemon.
- `forward_daemon_command` **explicitly blocks** a `Subscribe`/`Unsubscribe` sent by the frontend (ignored even if received + a warning log).

## 거부한 대안
- **Frontend ownership (each window's `subscribeOutput` forwards the daemon `Subscribe`)** — when N windows view the same agent, N full FromOldest retransmissions (storm) + order/duplicate confusion from each window's different progress. It also makes the daemon need to know "which window is viewing", coupling it to the UI (violating the ADR-0035 daemon UI-agnostic principle).
- **The daemon tracks subscription membership (window placement) itself** — the daemon would need to know the layout, the same coupling problem. The "daemon knows Views" model that ADR-0035 rejected.

## 근거
In the first adversarial review, Codex (blind) found "the revival of direct frontend subscription = an N-window FromOldest storm" as **BLOCK-1** (opus missed it — demonstrating the value of cross-family cross-checking). It is a direct consequence of ADR-0035/0037. We pinned it as a regression assertion in `protocolClient.test.ts`: "the first subscribeOutput subscription does not send Subscribe to the daemon".

## 영향 / 불변식
- `forward_daemon_command` blocks the frontend-originated `Subscribe`/`Unsubscribe` (`src-tauri/src/commands/agent.rs`).
- `ProtocolClient.subscribeOutput` only registers a renderer and does not emit a wire `Subscribe` (`src/api/protocolClient.ts`).
- The daemon wire subscription is emitted by `router.rebuild → send_subscription_delta` (`src-tauri/src/commands/layout.rs`) alone.
- **If violated (direct frontend subscription revived):** the regression of an N-window FromOldest duplicate storm in multi-view + confusion of progress.
