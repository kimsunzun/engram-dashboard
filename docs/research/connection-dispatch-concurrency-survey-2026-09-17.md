# Per-connection dispatch serialization — how mature OSS solves it (design survey)

| Item | Value |
|---|---|
| **Status** | medium · adversarial review **pending** (while this line remains, this is a pre-review draft) |
| **Method** | three main-family tracks collected in parallel (peer survey · mechanisms · opposing school) → main synthesis → grounding → cross-family adversarial review |
| **Date** | 2026-09-17 |
| **Confidence legend** | **certain** = supported by grounding + independent cross-confirmation · **likely** = supported by grounding, single source · **uncertain** = unsupported/abstained |
| **Why this survey** | User's remark — "where is there software that blocks B's input while agent A is connecting" |

---

## 0. Subject — the current shape of our code

One dashboard app shares **one WS socket** with the daemon (ADR-0036). The socket's receive loop `await`s the handling of one frame before it reads the next.

| Place | File:line |
|---|---|
| Receive loop (the serialization point) | `crates/engram-dashboard-net/src/ws.rs:537`·`:553` |
| Port contract with sequentiality baked in | `crates/engram-dashboard-net/src/frame_port.rs:92` |
| Activation waits inline until its outcome | `crates/engram-dashboard-daemon/src/connection_core.rs:1222` |
| Wait cap (give-up mechanism) | `crates/engram-dashboard-agent/src/manager.rs:70` — 15 seconds |
| Guard that relied on serialization (already bypassed) | `crates/engram-dashboard-agent/src/manager.rs:1107` — ADR-0082 unresolved |

**Symptom:** while A is activating, B's input, kill and other activations all wait. Normal 2.2 seconds (measured), worst case 15 seconds. The output direction is a separate task, so it keeps flowing (`ws.rs:393` read_task / `:402` write_task).

**Material we already have:** every command carries a `request_id`, and the client has a waiter table that matches on that number. Outgoing frames are already narrowed to a **single writer** (`conn_tx` → write_task).

---

## 1. Peers — places that already solved the same shape

★**Even the one case that deliberately kept serialization (tmux) has a 「주차 후 루프 반환」 mechanism. No peer has the plain `await` form.**★

| Peer | What it puts on one socket | How it solved it | Correlation means | License | Confidence |
|---|---|---|---|---|---|
| **rust-analyzer** ★closest★ | All LSP requests of one stdio connection | **Four-way branch by request kind** — read-only goes to the thread pool, latency-sensitive runs inline on main, `&mut` global state runs on main, rustfmt gets a dedicated pool | Request `id` moved into the closure | MIT/Apache-2.0 | certain |
| **wezterm mux server** | All pane/tab PDUs of one connection | The read loop calls `process_one` once and returns. Most PDUs go through `spawn_into_main_thread(...).detach()`. Search uses a **double spawn** | PDU header `serial: u64` | MIT | certain |
| **jsonrpsee** | WS JSON-RPC | `tokio::spawn` as soon as a frame arrives. Responses go through a cloned sink → a **single send_task**. On shutdown it waits for in-flight work to finish | JSON-RPC `id` | MIT | certain |
| **zellij** | Per-client routing | The routing loop **only interprets** and hands off through a channel. Slow work belongs to the pty thread. When a channel is full, a retry queue | None (one-way) | MIT | certain |
| **tmux** ★serial-keeping side★ | Per-client command queue | If the head is `CMDQ_WAITING`, it **returns immediately** to the event loop. An external event wakes it via `cmdq_continue()` | None | ISC | certain |
| **herdr** | — | **Does not multiplex** — one OS thread per connection | Has `request_id` | Apache-2.0 | certain |
| **paseo** | Agent daemon + multiple clients | The socket handler does **not await** the dispatch (`void ...catch()`) | Unverified | ★mixed·unverified★ | technique certain / license uncertain |

**Four things in common:**

1. **If id correlation already exists, you can turn this on without changing the protocol** — all it takes is capturing the id in the read loop and moving it into the work closure.
2. ★**Not 「전부 spawn」 — they split per request**★ — rust-analyzer is the model. The code states explicitly the judgment that throwing even instant-answer commands onto tasks makes latency worse.
3. **Narrow writes to a single writer, always** — the only way not to pay for concurrent handling with frame interleaving. **We already have it.**
4. **Include shutdown handling of in-flight tasks in the design** — jsonrpsee's wait for completion, rust-analyzer's cancellation and retry.

---

## 2. Mechanisms and the price of each

| # | Pattern | What it buys | Invariant given up | New failure mode |
|---|---|---|---|---|
| M1 | Spawn per message (unbounded) | Loop returns immediately | Arrival-order handling + **natural backpressure** | Unbounded task growth · side-effect reordering · cancellation loses its definition |
| M2 | Bounded concurrency (semaphore · N workers) | Concurrency + bounded tasks | Arrival-order completion | When permits run out, **where to wait** becomes the problem again |
| M3 | Separate read/handle (bounded mpsc → actor) | Decode separated from execution · backpressure propagates to the socket | (With a single worker) none | With a single worker, **HOL just moves into the queue** |
| M4 | Control-only lane (kill·cancel bypass the queue) | Cancellation does not queue behind its target | Global arrival order | Causal inversion from overtaking · starvation between lanes |
| M5 | Ticket/deferred completion (accept immediately, result as a later event) | HOL disappears while the loop stays serial | **「응답 = 완료」 semantics** | If the result event is lost, permanent pending · tickets orphaned on reconnect |
| M6 | ★**Command classification** — state mutations stay serial, only the rest goes off-loop★ | Confines ordering guarantees to **only the commands that need them** | None | A misclassification becomes a silent race · **the default for a new command decides safety** |

**Three tokio facts (source = official docs):**

- If you do not call `next()`, data piles up in the OS receive buffer, and once it is full TCP zero-window stops the sender — backpressure is free, but it arrives **late by one buffer's worth**.
- ★**tungstenite puts Ping and Close replies into an internal queue and sends them out on the "next call to `read`, `write` or `flush`"**★ — if reading is stopped and the Pong does not go out, the peer may kill the connection. **Whether our write_task rescues this is unverified** (→ §6 gap).
- **A `spawn_blocking` task cannot be aborted** — "If you call `abort` on a `spawn_blocking` task, then this will not have any effect". Activation currently runs on it.

★**`spawn_blocking(...).await` itself is not an anti-pattern**★ — it is the canonical form tokio recommends. The problem is the position, **inside a serial dispatch loop**, and the established name for that phenomenon is head-of-line blocking.

---

## 3. The opposing school — what is lost

★**The strongest counter-argument lands not on 「동시화하지 마라」 but on 「명령 분류 없이 루프만 풀지 마라」**★. In that form it is strong.

- ★**request_id licenses out-of-order *responses*, not out-of-order *execution***★ (certain)
  **RFC 9112 §9.3.2** — the condition for processing pipelined requests in parallel is not a correlation means but whether the **method is safe (non-mutating)**. And responses must be sent in the order received (MUST).
- **LSP 3.17 spelled out the criterion** (certain · **two tracks independently brought back the same sentence = cross-confirmation**)
  "The server may decide to use a parallel execution strategy… **as long as this reordering doesn't affect the correctness of the responses**." It gives counterexamples directly — completion and signatureHelp can be reordered, **definition and rename cannot** (the latter changes the result of the former).
- **Making the app layer concurrent may move the blocking rather than remove it** (evidence certain / application to us likely)
  A documented example: QUIC removed transport HOL, yet QPACK created a new blocking axis (RFC 9204). Our counterpart = if the handlers all end up meeting again at **one manager lock**, the queue has merely moved from the loop to the lock. ★We did not measure this on our code★.
- **Today's serialization is itself the intake cap** (principle certain / our numbers unknown)
  Freeing it removes that cap too — **the cap must go in the same commit**.
- **A detached task loses error, completion and cleanup together** (certain)
  At the moment the connection drops, a handler that has changed shared state halfway either (a) runs to completion without a requester or (b) is cut off at an await point and left half-applied. `spawn_blocking` cannot even be aborted.
- **A cancellation racing its own target becomes a new spec obligation** (certain)
  MCP spelled out that convention — the receiver may ignore the cancellation of an already finished request (MAY), and the sender should ignore a response that arrives after cancellation (SHOULD). The decision that was self-evident under serialization **has to be written by hand as a state machine**.

★**The biggest weakness of this opposing side — recorded honestly**★: we **found no public incident report** of "made per-connection dispatch concurrent and regretted it." All the counter-arguments we obtained are spec- and principle-based, not incident records.

---

## 4. Things quietly riding on the serialization (to be set up explicitly before freeing it)

1. **Prior-registration guarantee** — the order between a command that creates a session and an input that targets it. If the input lands first, a "no such target" error.
2. **Duplicate requests for the same id** — two concurrent activations of the same agent entering → double spawn. ★**Already bypassed between connections**★ (`manager.rs:1107`, ADR-0082 unresolved). So this guard has to be set up explicitly anyway.
3. **Causality of kill** — the path where a cancellation overtakes the creation of its target.
4. **Side-effect order** — reversed order of the on-disk registry writes.
5. **Consumers that read arrival order** — a client that matches by id is safe, but replay/seq and test harnesses are not. That RFC 9112 pins in-order responses as a MUST even while having its own correlation is corroboration.

---

## 5. Constraint fit — what suits our conditions

> ★**This section's recommendation was superseded by §5-2 — it picked the wrong axis**★. It is kept below as a record (not overwritten). What was wrong = the first paragraph of §5-2.

**Our constraints:** Rust · tokio · tokio-tungstenite · single WS · `request_id` correlation **present** · single writer **present** · core isolation and swappability (CLAUDE.md) · the double-spawn guard currently depends on serialization · ADR-0201 **deliberately** introduced waiting for the activation outcome · the transport crate work decided not to open up `net` (ADR-0177 decision 1).

| Track | Peer precedent | Fit to our constraints | Remaining work |
|---|---|---|---|
| **A. Command classification + off-loop** ★recommended★ | rust-analyzer · LSP spec | **High** — id correlation and single writer already exist. No command loses its ordering guarantee | Classification table · default to serial · a cap on the off-loop side |
| B. Spawn everything | jsonrpsee · paseo | Medium — it works, but takes on all five of §4 at once | Cap · every guard · cancellation state machine |
| C. Ticket/deferred completion (activation only) | (M5) | Medium — leaves the loop serial and only removes the symptom. **But it head-on changes ADR-0201's 「결말을 응답으로」** | Two-stage client state machine · revisit ADR-0201 |
| D. Keep serial + park | tmux | Medium — the most conservative. The wait itself remains and only the loop is freed | Park/resume mechanism |
| E. Leave it as is | None | **Low** — no peer has this form | — |

**Recommendation = A.** Three grounds: ① the closest peer (rust-analyzer) and the only formal specification (LSP) have the same shape ② the two things we already have (id correlation · single writer) are the premises of that shape ③ the strongest argument of the opposing school **does not oppose A, it opposes B**.

★**If we pick A, 「무엇을 직렬로 남길지」 is the design itself**★ — draft: activation, kill and profile changes stay serial; input, queries and subscriptions go off-loop. **The default for a new command is serial** (so a missed classification does not become a silent race).

---

## 5-2. Axis correction + final recommendation (second survey, three tracks, same day 2026-09-17)

★**Where §5 was wrong**★: "classify commands and leave some serial" was right, but it **did not say serial with respect to what.** What is needed is **serial per same resource**, and it was lumped together as **serial per whole connection**. Those two are entirely different costs — this surfaced through the user's remark (2026-09-17), and the second survey supported that remark.

### ★Strongest ground — this question has a named principle★ (certain · main checked against the original)

In Saltzer/Reed/Clark, *End-to-End Arguments in System Design* (1984), the subsection 「**Guaranteeing FIFO message delivery**」 is entirely about this topic:

> "A distributed application in which one node can originate requests that initiate actions at several sites cannot take advantage of the FIFO ordering property to guarantee that the actions requested occur in the correct order. **Instead, an independent mechanism at a higher level than the communication subsystem must control the ordering of actions.**"

The same section pins low-level ordering guarantees as "**justified only as performance enhancements**".

★**Stating exactly the scope of the conclusion we can draw here**★ — what the original says is **「통신 서브시스템의 FIFO 만으로는 행위 순서를 보장할 수 없다」**, not 「dispatch 계층 직렬화는 정확성 근거가 못 된다」. The app-side dispatcher or manager **can qualify as** the 「더 높은 층의 독립 메커니즘」 the original speaks of. The paper itself limits the end-to-end argument as "not an absolute rule". (★A stronger assertion once stood here; adversarial review 2026-09-17 removed it as an overstatement.★)

**What still remains for our case:** the place where we keep order today is not the app layer but the **socket receive loop**, and that is the low level the original speaks of. Serialization at that place is justified only by performance, and **in our case it is cutting performance.**

### What RFC 9112 says to us (★the line once written here, 「우리는 면제」 was wrong★)

RFC 9112 §9.2 says HTTP/1.1 depends on **response arrival order** because it has no request identifier:

> "HTTP/1.1 **does not include a request identifier** … Hence, **it relies on the order of response arrival**…"

★**But that does not exempt us from §9.3.2's 「safe method 일 때만 병렬」**★ — they are separate clauses. What an id solves is only the **response correspondence**; the **execution semantics** of state-changing operations remain. That is, what §3 wrote — 「request_id 는 out-of-order **실행**의 면허가 아니다」 — is right, and the 「ID 가 있으니 면제」 once written here **flatly contradicted** that sentence (removed by adversarial review 2026-09-17).

**There is one thing to actually take from that clause** — the axis the constraint applies to is **response delivery order**, not **processing**. Whether processing may be parallelized is decided by whether the operation is safe.

**LSP is also not a counterexample but a codification of this position** — ordering is the exception, and what decides that exception is **semantics** (which operation touches which state), not the connection.

### How peers actually built it (measured on local clones)

★**At our scale, the industry answer is not 「엔티티별 큐」 but 「전송은 즉시 넘기고, 앱 쪽 단일 지점이 순서를 진다」.**★ Per-entity queues appear when there are thousands of entities and the system is distributed (Orleans · Akka).

| Peer | Transport layer | Where the ordering is carried |
|---|---|---|
| **wezterm mux** ★closest★ | `dispatch.rs:82` — calls `process_one()` and goes to the **next frame immediately** | The mux main thread. State is the `RwLock<HashMap<..>>` at `mux/src/lib.rs:103` |
| **zellij** ★partial counterexample★ | Forwards over subsystem channels — **but some actions synchronously call `wait_for_action_completion()`** (usually a 1 second timeout, some unbounded). Putting it in an 「즉시 handoff」 column would be wrong | A dedicated thread per subsystem + an MPSC receive loop (`lib.rs:2130~`) |
| **herdr** ★not a precedent★ | Spawns **an OS thread per connection** and waits for the response inside it — the `timeout=None` path can block indefinitely. **It is not a non-blocking precedent for a single multiplexed socket; it is a per-connection isolation precedent** | One app event loop |

wezterm correlates responses by `serial`, and the client receives them in `HashMap<serial, promise>` (`wezterm-client/src/client.rs:387`) — **the same as our `request_id` + waiter table**.

### Final recommendation — separate three layers

| Layer | Who | What it does |
|---|---|---|
| **Transport** | `ws.rs` receive loop | Decodes and **hands off immediately**. ★Never blocks★ |
| **Ordering** | The side that owns the resource (the manager) | In its own queue or lock, **only where semantics require it** |
| **Response** | Single writer (already exists) | Replies matched by `request_id`, **order-independent** |

★**In our code, layers 1 and 2 are stuck together in the same place, and pulling them apart is the whole fix.**★ We **will not build** a per-entity queue map — a zellij comment records its cost (when entries are put in and taken out of the key map, "**everything is halted, that's life**").

### Two honest costs of this direction (physical evidence)

- **It turns into a silent race** — a system-wide halt is safe but visible. Once we push things down, the one place we miss becomes a silent race. zellij's resize-race fix commit (`a725935`) is the physical evidence, and ★**most of what that commit fixed was 「테스트 하네스를 실물처럼 만드는 일」**★ — this class of bug is invisible when the harness differs from the real thing.
- **The opposite-direction cost is also on record** — zellij `route.rs:194` comment: waiting inside a routing function parks while holding a read guard, so **a concurrent write deadlocks.** Our current shape is the same kind of thing as that warning.
- **The form for bringing an exception back** — instead of full serialization, zellij uses **per-operation opt-in** (a completion oneshot) and **fails open with a 1 second timeout** (`route.rs:38,247`). The shape to copy when we later meet "only this command must wait for completion."

### Remaining risks (unchanged even if we do not use the entity axis)

1. **Creation race** — the thing to be queued is created by that very command. ★Already bypassed between connections (ADR-0082)★. Akka eliminates it with a single coordinator, and **Orleans allows the race and forwards the loser to the winner** — when the duplicate-activation logs got too frequent, they **closed it by lowering the log level** (`dotnet/orleans#6240`). Preventing is not the only answer.
2. **The next incarnation of the same id** — in Erlang a signal sent to a dead target **silently vanishes**, and Akka **delivers the buffer held during stopping to the new incarnation**. Our incarnation marker is a third choice (discard by fence), so we now have two control groups.
3. **Commands whose target is not a single one** (registry operations · full shutdown) — peers do not give them a separate lane; they put them into the same single point.

---

## 6. Open issues · gaps · things not verified

- ~~Unverified whether our write_task rescues tungstenite's Pong queue~~ → ★**resolved = not applicable (code checked 2026-09-17)**★. Severity **does not go up**. Grounds: the stall cap is **15 seconds** (`manager.rs:70`) while the keepalive idle_timeout is **50 seconds** (`ws.rs:53` · 2.5 times the 20-second ping_interval). During the stall the client's Pong cannot be read and `last_recv` is not updated, but **35 seconds of margin remain**, so write_task's exit branch (`ws.rs:486`) is not reached. The client side sets no idle timeout at all (`daemon_client/connection.rs` — it only ignores Ping/Pong and leaves it to tungstenite's automatic reply).
  ★**But this is safety that leans on the 15-second cap**★ — if the activation cap grows or another inline command appears that reaches 50 seconds, **the connection drops.** A change that raises the cap must look at this margin too.
- **We did not read the body of ADR-0201** — the decision that made activation wait for its outcome. Whether that decision only wanted "return the outcome as the response" or also intended blocking other commands in between is the fork between A and C.
- **Not measured whether the handlers end up meeting again at one manager lock** — if they do, A's gain shrinks.
- **jsonrpsee's per-connection in-flight cap** unverified (only the connection-count cap was checked).
- **paseo's license** started from a mixed wording — borrowing the pattern is fine, but copying code needs checking.
- **DAP · Docker/containerd/libvirt** not surveyed (abstained).
- **Not verified in code whether the tmux server actually keeps handling other clients' input while processing a command** (inferred from structure, likely).
- **Adversarial review not done** — while this line remains, this report is in a **degraded** state.

## 7. Links

- Canonical source for symptom and code points = `docs/refactoring/socket-head-of-line-blocking-2026-09-17.md` (the document handed to the network refactoring). This report carries **how to fix it**.
- Related ADRs = **0201** (activation outcome wait) · **0082** (double spawn, unresolved) · **0036** (one connection) · **0177** (transport crate charter — `net` is inviolable) · **0081·0142** (two precedents where this repo named the same pattern head-of-line blocking and avoided it).
