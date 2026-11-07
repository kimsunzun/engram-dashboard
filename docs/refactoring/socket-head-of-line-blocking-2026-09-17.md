# Socket receive waits for the activation outcome — an observation handed to the network refactoring

**Written = 2026-09-17 · source = the S21 codex backend session (branch `v0.3.0/feat/codex-backend`)**
**Nature = structural-debt measurement. Not a decision** — this session decided only up to 「점수정하지 않고 소켓 리팩토링에 편입한다」 (user decision 2026-09-17).

---

## In one line

**While one agent is being activated, the rest of that client's commands are not even read from the socket.** Input sent to another agent, a kill, another activation — all of them queue behind it.

---

## Symptom

Every agent in the dashboard window shares **one socket** with the daemon — "however many windows there are, the daemon has 1 connection" (`src-tauri/src/daemon_client/mod.rs:3`, ADR-0036).

| Time | User | Actual |
|---|---|---|
| 0.0s | Activates agent A | Handshake starts |
| 0.3s | Types input + Enter in the slot of agent **B** | The frame reached the socket but was **not read** |
| 2.2s | — | A's activation completes |
| 2.2s | B's input only now goes in | The backed-up frame is processed |

Process A and process B are unrelated to each other. What is blocked is **the one client ↔ daemon segment**.

```
대시보드 앱 ──[WS 소켓 1개]── 데몬 ──┬── A 프로세스 (codex)
                                      └── B 프로세스 (claude)
```

---

## Causation — three places in the code

1. **`crates/engram-dashboard-net/src/ws.rs:553`** (loop start `:537`) — the receive loop reads the next frame only **after** it has `await`ed the processing of one frame.
   ```rust
   while let Some(item) = incoming.next().await {
       ...
       Message::Text(text) => {
           if handler.on_text(conn_id, &text, &frames).await == ConnFlow::Close {
   ```
2. **`crates/engram-dashboard-net/src/frame_port.rs:93`** — that sequentiality is written down as the port **contract**: "`on_connect` → (`on_text` | `on_binary`)\* → `on_disconnect`".
3. **`crates/engram-dashboard-daemon/src/connection_core.rs:1222`** — handling of the activation command waits **until the outcome** of the agent handshake.
   ```rust
   let started = tokio::task::spawn_blocking(move || {
       mgr.activate_profile(&profile_for_task, mode)
   }).await
   ```
   The reason in the comment: "on the path that declares a connection, we wait for the outcome regardless of mode (**ADR-0201** — Fresh used to return immediately)".

★**Only when all three multiply together does it become the symptom**★ — sequential receive was there from the start, and it showed up when codex activation became the first guest to occupy that spot for a long time.

### The actual upper bound of the wait = 15 seconds (not infinite — it nearly became infinite)

★**The answer to 「응답이 영영 안 오면 영영 대기인가」 is 「15초」, and the reason that value exists is exactly 「없으면 무한」**★.

- `crates/engram-dashboard-agent/src/manager.rs:70` — `LINK_RESOLUTION_BACKSTOP = 15s`. Its comment: "codex's handshake limit applies only to **waiting for the response** and not to **writing** — if the peer does not read our stdin, `write_all` hangs forever on pipe backpressure (…) and without this backstop `activate_profile` blocks **forever** (**one async dispatch worker of the daemon is tied up entirely**)."
- The comment calls that value not 「판정」 (a decision) but ★「포기」 (giving up)★ — when it is reached, it settles the outcome as failure and starts teardown, and that kill also frees the write that was hanging.
- **So the two wait limits must be read apart**: the path where the response is late ≈ **11 seconds** (handshake budget 10 seconds + two overruns) · the path where no response comes at all and the write also hangs = **15 seconds** (this backstop). ★**「최악 11초」 (worst case 11 seconds) is a wrong summary**★.
- ★**The comment already knows this document's symptom**★ — "one async dispatch worker of the daemon is tied up entirely" is it. That is, the cost of the inline wait came in with full awareness, and the backstop only **bounds it, it does not remove it**.

---

## The material for the fix is already there

- **Every command carries a `request_id`** (`crates/engram-dashboard-protocol/src/messages.rs` — each `AgentCommand` variant).
- **The client has a waiter table that pairs responses by that number** (regression net = the shell package integration target `daemon_client_pending`. It measures that an overlapping `request_id` does not panic the connection task, that the old waiter wakes with an error, and that the new request inherits the number).

★**So there is no structural reason the response has to be made synchronous inside frame processing**★ — correlation already stands on the number, not on position.

---

## ★Serial processing is not a safety device★

The quick fix (leave activation as it is and process only the kill frame ahead of it) was considered, and the assumption that propped it up was **「직렬이라 이중 스폰이 안 난다」 (it is serial, so no double spawn happens)**. That assumption is false — the guard's comment denies it directly (`crates/engram-dashboard-agent/src/manager.rs:1107`):

> "If **different connections** SpawnProfile the same id at the same time, both can pass this check and the pre-check of `activate_profile`, and a double-spawn can happen (daemon command handling is **serial only per connection, not across connections** — each connection awaits dispatch in its own read_task). This window is a **pre-existing race** from before ADR-0082, and this change neither introduces nor closes it (flagged as a follow-up task)."

That is, sequential receive **only narrows the window within one connection**, and double spawn is already open. Even if receive is made asynchronous, **no hole that did not exist is created — an existing hole just gets wider** — and that guard has to be built explicitly anyway.

---

## Points to look at in the refactoring

1. `ws.rs:553` — the receive loop that waits for processing to finish before reading the next frame. Loosening this is the main body.
2. `frame_port.rs:93` — the sequentiality is baked into the port contract. Whether to fix the contract or punch an exception through it is the fork.
3. `connection_core.rs:1222` — the dispatch that waits for the activation outcome. ADR-0201 made it so, so **to reverse it, that ADR has to be reopened** (it would not be a revert to immediate return, but a shape that moves the outcome report onto the `request_id` path — this sentence is inference).
4. `manager.rs:1107` — the double-spawn guard. Build explicitly the part that relied on serial processing (the unresolved item of ADR-0082).

---

## Verification status

| Item | Status |
|---|---|
| One shared socket · sequential receive · inline wait on activation | **Confirmed in code** (file:line above) |
| The double-spawn race is open across connections | **Stated by a code comment** (pre-existing, ADR-0082) |
| codex re-activation ~2.2 seconds | **Measured in the GUI** (recorded 2026-09-16) |
| Path where the response is late ~11 seconds | ★**Derived by reading the code — never measured**★. There is no zero-budget check between the two requests, so the deadline overruns twice (the 10.5 seconds that was once written is a wrong value) |
| **Wait upper bound 15 seconds** | **Confirmed in code** (`manager.rs:70` backstop). ★This is the real upper bound, not 11 seconds★ — the path where the write hangs is not covered by the handshake limit |
| Other commands actually being delayed during activation | ★**Not measured in the GUI**★ — derived only from the code path |
| Whether the claude backend's activation creates the same wait | **Not looked at** |

---

## ★It is not in the transport crate's work scope right now (confirmed 2026-09-17)★

「어차피 소켓 리팩토링에서 같이 처리된다」 (it gets handled together in the socket refactoring anyway) **does not hold as of today.** The in-progress `S21-transport-crate` work not only does not cover this item, it **explicitly left it out of scope.**

- **The charter nails down that the existing `net` is not torn apart** — ADR-0177 decision 1: "★**This work does not tear apart the existing `net`**★". But the receive loop (`ws.rs:553`) and the port contract (`frame_port.rs:93`) are inside exactly that `net`.
- **Applying it on the daemon side is a later stage** — `docs/process/S21-transport-crate/trd.md` §10 「안 하는 것」: "Applying it on the daemon side | attach order ④". The 「정지 조건」 of that branch's handoff also writes "**when about to modify the `net` crate** → out of scope".
- **Activation · `connection_core` · ADR-0201 appear in none of that work's documents** (grep 0 hits). ADR-0177~0183 are from 2026-09-04~06, so they predate ADR-0201.
- **`request_id` is in that work too, but on a different axis** — it is treated only as the client-side waiter table (ADR-0177 decisions 3/4 · 0181 · 0183), and is not used for the argument 「데몬 핸들러가 동기로 답하지 않아도 된다」.
- **The closest item is a rejected alternative, but on a different axis** — in ADR-0177 「거부한 대안」, "control-only queue / control-only socket — ★guarantee is impossible on a single socket★" discusses the case where **writes are blocked**. It does not cover this case, where our reader waits for our handler.

**Inference (label)** — once stage ④ is actually attached, the receive shape of that crate could absorb the symptom **as a side effect**. `crates/engram-dashboard-transport/src/lib.rs:26` makes `Inbound::recv` the "only receive surface" and receives into a bounded queue, so reading and processing are separated. **But no document says so**, and the blocking `await` is in the daemon dispatch, not in the transport.

**Where to register it (by that side's convention):** ① `docs/process/S21-transport-crate/trd.md` §12 「열린 질문」 — the section that already holds the stage ④ item in the same way. ② `docs/tracking.md` — the closest neighbor is T-32 (the daemon-side serialization gap).

---

## ★The same repository has already avoided this pattern twice★

This shape did not first appear in this repository. **It has been named and avoided twice, and only the activation path is still doing it.**

- **ADR-0081:29** — "preventing per-connection sequential dispatch head-of-line blocking". On the relay path it was loosened by spawning **outside** the actor.
- **ADR-0142:16** — "do not await the `spawn_blocking` handle inline … **because awaiting it right there is itself head-of-line blocking**". The mail delivery path.

What `connection_core.rs:1222` does is exactly what the latter forbids — `spawn_blocking(...).await` inline. ★**So this item can be read not as a new design judgment but as a judgment already made that was not applied to just one path**★ (this sentence is interpretation — ADR-0201 introduced that wait deliberately, so whether this is a place where the two decisions collide or they are on different axes needs checking).

---

## Links

- Decision-list item 2 of the source session, 「활성화 중 취소가 뒤에 줄 선다(~11초)」, was moved over to this document.
- Related ADRs = **0201** (the decision that made activation wait for the outcome) · **0082** (double-spawn race, unresolved) · **0036** (one connection).
