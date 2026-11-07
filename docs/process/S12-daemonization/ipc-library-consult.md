# S12 Daemonization — real IPC library survey + architecture decision (consult cross-verification)

Date: 2026-06-14. Method: `/consult` — GPT·Gemini·Claude-Opus three-way blind cross-verification + a dedicated judge's decision. Raw material: `I:\Engram\agents\web-runner\shared\20260614-042121-consult-daemon-ipc-lib\`.

## TL;DR — decision to put implementation on hold

**There is no turnkey library. No mature library can take over the core invariants (replay→live order, seq dedup, epoch resubscribe), so a custom implementation is unavoidable, and the scale is MVP 1~3 weeks + production-ready 3~6+ weeks.** Following the user's instruction ("적당한 성숙 라이브러리가 없어 구현이 커지면 하지 말 것" — if there is no suitably mature library and the implementation grows large, do not do it), **starting is put on hold** — proceed after the user decides the decision points below.

The three models + the judge are **unanimous** on this: a library supplies only the "transport"; correctness is the responsibility of the daemon's session core. (Verified by limits that each library's official documentation states — not speculation.)

## Agreed conclusions (the judge ruled "correct" after critical verification)

1. **Replay location = held by the daemon itself** (the current OutputCore/ring buffer is the source of truth). Do not delegate to a broker (JetStream/Centrifugo) — PTY bytes are volatile and limited to the session lifetime, so an external persistent broker fits poorly. Broker recovery fails when size/TTL/limit is exceeded (Centrifugo recovered:false) → our own fallback is needed anyway. (If durability is needed later, the daemon *mirrors* the seq-stamped frames into JetStream, while authoritative ordering stays with the daemon.)
2. **Path = B-contract-first** (JS↔daemon direct WS). From the start the daemon opens a WS server and fixes the envelope on the B (mobile direct) basis. Initially start with the Tauri app attaching over localhost (A's stability + a single WS absorbs A/B). **Do not implement A separately with tarpc/jsonrpsee** (JS cannot reach it → discarded on switching to B). Consistent with §5 (LLM-first single control surface) too.
3. **Need for custom implementation = YES (unavoidable).** However OutputCore already has the seq/replay/epoch/subscribe logic, so it is not from zero — "exposing it over the wire" is the main work.

## Contested issues — judge's decision

| Issue | Decision | Rationale |
|---|---|---|
| **First-choice library** | **raw `tokio-tungstenite` + our own thin protocol** (conditional) — Claude's position adopted | The core invariants must be implemented ourselves anyway, and Socket.IO is protocol lock-in, which clashes with the "swappable" seam principle. "A thin WS + exposing the existing OutputCore" is cleaner than reverse-mapping a library model. **However a user decision point** (below). |
| **socketioxide CSR** | Claude most accurate / Gemini wrong | Connection State Recovery is **absent** from the socketioxide GitHub feature list (the judge confirmed it by WebSearch). JS Socket.IO has CSR from 4.6.0~, but socketioxide (a separate Rust implementation) does not. Even if present it is best-effort and cannot guarantee invariant ① (zero loss) → **meaningless** for this project. |
| **Effort estimate** | GPT's range estimate is realistic / Gemini underestimates | Gemini's "2~3 weeks" omits the daemon process lifecycle and backpressure. Realistic: **MVP 1~3 weeks, production-ready 3~6+ weeks** (hardening is the bulk). Daemon management (single instance·zombies·ports·moving the Job Object to the daemon) is separate effort. |

## Eliminated (three models + judge agree)
- **tarpc** — cannot reach JS/browser (Rust↔Rust only). Only the path-A segment is possible, and it is discarded on switching to B.
- **tonic/gRPC + grpc-web** — no client/bidi streaming in the browser, so it cannot carry PTY input.
- **jsonrpsee reconnect wrapper** — the official docs state "subscription messages are lost and you cannot tell which ones" → directly conflicts with invariants ①②.
- **NATS / nats.ws** — async-nats (the Rust server) is excellent, but **the browser client nats.ws was archived on 2026-05-08** → fatal for the primary target of mobile WS attach (JS).

## ★ User decision points (before starting)

1. **Library: raw WS vs Socket.IO+socketioxide.** The judge decided raw WS (avoids lock-in·minimal dependencies·seam swappability). But if "automatic reconnect·binary framing·ready-made namespace conveniences" are judged more valuable, Socket.IO+socketioxide is also reasonable. **If both are hidden behind the transport seam, the regret cost is low** — swappable later.
2. **TLS·authentication trust boundary for remote mobile direct attach** (all three treated it weakly). A loopback token is localhost-only. Whether to reflect the remote-stage design in the envelope in advance.
3. **backpressure policy** (only GPT stated it explicitly). The ring buffer cap and drop/block policy when a slow/disconnected client is attached — the key to memory safety under high-throughput PTY.

## Decisive errors per model (reference)
- **Gemini:** confused CSR/reconnect (Q1 "no manual state machine needed" ↔ Q5 "100% custom" is self-contradictory), underestimated effort, unclear first choice (Socket.IO·Axum listed as equals).
- **GPT:** zero factual errors, honest with "don't know" on the Centrifugo Windows binary. Weakness: recommended Socket.IO first while admitting lock-in → subtly conflicts with the seam principle.
- **Claude:** zero factual errors (every claim checked matched measurement), most accurate. Weakness: shallow on mobile auth/backpressure, avoided effort numbers.

## Second OSS precedent survey (2026-06-14, separate research) — verifying "is there anything shared?"

The user's question ("데몬 예제는 비슷한 부분 많을 텐데 왜 공수가 높나, 오픈소스 데몬 확인해봐" — daemon examples must have a lot in common, so why is the effort high; check open-source daemons). Conclusions of the primary-source survey:
- **No turnkey Rust crate (certain).** WezTerm `codec`/`mux`=`publish=false` (internal code), `zellij-server`=Zellij-specific, Eternal Terminal=C++. There is no all-in-one library anywhere that you can plug the daemon body into.
- **Only reusable "plumbing":** `portable-pty` (already used), `tokio-tungstenite` (WS), `interprocess` (cross-platform IPC, used by zellij). JS: `@xterm/addon-attach` (WS↔terminal pipe, **no reconnect logic**), `@xterm/addon-serialize` (client snapshot; optional for us since we do server replay).
- **The replay→live algorithm = a universal pattern, which we already have.** Eternal Terminal `BackedWriter`: `재전송량 = serverSeq − clientLastSeq` (resend amount), take the latest N in reverse → resend in reverse again → structurally prevents gaps·dups. **OutputCore is an isomorphic implementation.** ET/VS Code cross-verify that our pattern is the standard. → at the level of copying rather than inventing, de-risked.
- **The real effort = infrastructure, not the algorithm (no library to plug in):** ① daemon lifecycle (boot·crash restart·orphan PTY/Job Object cleanup, redesigning the `KILL_ON_JOB_CLOSE` coupling) ② process-boundary serialization + backpressure (in-process Arc/RwLock→WS framing, so a slow client does not block the daemon) ③ buffering policy during disconnection ④ Windows daemonization (most precedents assume Unix domain sockets → named pipe/localhost TCP directly).
- **Effort estimate updated:** replay concurrency is de-risked so variables decrease. ~5~8 focused sessions; the biggest variable is the Windows daemon lifecycle (not replay).
- Borrow only the architecture: the VS Code model "out-of-process pty host + push the server-cached scrollback on reconnect" (1:1 with Engram).

## Design direction finalized (2026-06-14)

### Proposed architecture ↔ Engram reality (the external LLM proposal assumed "from scratch", but it was actually completed in S10)
| Proposed stage | Engram reality |
|---|---|
| Split out a Tauri-free core | **Done** — `pty/` tauri import 0 (invariant); `examples/{headless,transport_smoke,session_smoke}.rs` drive AgentManager directly without Tauri = standalone proof |
| Separate the Command/Event protocol | **Done (neutral seam)** — `OutputEvent`/`InputEvent`/`CommandSpec`/`Capabilities` (types.rs), `AgentCommand` (profile.rs). But not a separate crate |
| Isolate Tauri IPC as a transport adapter | **Done** — `OutputSink`/`StatusSink` traits (= the proposal's EventSink) + the `AgentTransport` trait + `ChannelOutputSink`/`TauriStatusSink` (lib.rs) |
| Add a daemon transport | **The real remaining work** |
| embedded/daemon two modes | Cheap because sinks are injected |

Note (where the proposal oversimplified and ours is more correct): (1) the proposal's single unified `EventSink` vs our split `OutputSink` (high-throughput)/`StatusSink` (low-frequency) — the throughput characteristics differ, so keep ours. (2) The proposal's `AgentEvent::Output{Vec<u8>}` = fixed to bytes vs our kind-agnostic `OutputEvent` (§2 "터미널 강제 금지" — no forcing terminal) — accepting the proposal would be a regression. To accept: **ts-rs automatic TS type generation** (currently api/types.ts is a manual mirror → less drift; an improvement, not required for the daemon).

### Two-mode toggle (confirmed direction)
An `AgentClient` interface on the frontend (canonical `AgentCommand`/`AgentEvent` protocol). The toggle swaps only the implementation:
- **Embedded:** an in-process adapter → the AgentManager in the same process (zero serialization). = the known-good fallback.
- **Daemon:** a WebSocket client → socket → the daemon's AgentManager. **B-direct (JS↔WS direct)**, no relay (discarded for being unreachable from mobile + double serialization).
- The core (AgentManager) is identical in both modes. The toggle is chosen at startup/config (**not a live hot-swap** — the user confirmed "실시간 변경 불요" (no live change needed). Agents restart on a mode switch).
- Benefit: embedded is always a fallback → a daemon bug does not block us (no babysitting needed). However the correctness of the daemon path (lifecycle·reconnect) must be built and verified once — the toggle does not exempt that.

### Implementation phasing (user's design — boundary-based isolated tests)
1. **Encapsulate the seam + attach the frontend, confirm zero regression of the existing (embedded)** — extract the workspace lib crate `engram-core` (the code is Tauri-free so it is *moved*) + `engram-protocol` (AgentCommand/AgentEvent) + the frontend `AgentClient` interface. Confirm embedded still works through that interface.
2. **Daemon alone + isolated data-transfer tests** — a daemon binary + WS server + a standalone test client (verifies spawn/output/replay/reconnect/epoch without the frontend). = extending the `transport_smoke.rs` philosophy across the socket boundary.
3. **Keep that harness permanently** — when a daemon bug appears, incubate only that part with a test (nasty cases like reconnect·slow consumer·daemon kill/respawn must be included for phase 4 to be clean).
4. **Connect only the seam** — swap the frontend `AgentClient` from the embedded-impl to the daemon (WS)-impl. If the phase 2 harness verified the protocol behavior enough, the join is safe.

**linchpin:** what every isolated test verifies = the `engram-protocol` contract. Pinning this contract down exactly first is the precondition for "경계 두면 알아서 깨끗" (put a boundary and it stays clean on its own) to hold. Envelope draft: `subscribe{sessionId,epoch?,afterSeq?}` / `unsubscribe` / `input{sessionId,epoch,data}` / `list` / `spawn{spec}` / `kill{sessionId}` + events `output`/`status`/`exited`/`error`.

## Next
Re-verify the S12 design document (engram-protocol contract + AgentClient interface + ET recover pattern + daemon lifecycle + harness contract) by consult → start phase 1 (low-risk·mechanical, zero-regression gate).
