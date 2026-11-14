# ADR-0189: The codex app-server channel is placed as an AgentTransport implementation inside the backend folder

- 상태: 확정 (2026-09-09, basis: user decision + ownership-trace verification + re-reading the original comment text) · 부분 폐기 by ADR-0191 (it decided where the channel lives but did not decide the creation path)
- 관련: Amends ADR-0044 (the shared clause on the byte channel) · ADR-0004 (backend knowledge isolation) · ADR-0045 (output cleaning moves to the backend) · ADR-0030 (transport ⊕ backend caps composition) · ADR-0187 (app-server channel settled) · ADR-0190 (input queue) · `docs/research/session-path-ownership.md` · `docs/process/S21-codex-backend/trd-phase2a.md` §5 · step-log S21 · Amended by ADR-0191 (it decided where the channel lives but did not decide the creation path)

## 맥락

The codex app-server is a protocol where **you must send and receive the answer before you can go on** — `initialize` · `thread/start` · `turn/start` · `turn/interrupt` are four round trips, and in particular the conversation id can be learned only from the `thread/start` response (ADR-0187). claude is done once you push in and receive what flows out, so there is nothing to pair. **That is, codex is the first backend to demand something this structure does not have.**

★**We settled the blocked spot by ownership tracing**★ (canonical = the ownership map):

- The `AgentBackend` layer is the only layer that knows codex, but it **has no place to remember things per session** — it is a collection of functions, one zero-size unit struct shared by all sessions (`backend/mod.rs:277-279`).
- The `AgentSession` layer holds state, but **knowing codex there violates ADR-0004**.
- So there was no place for 「codex 를 알면서 세션마다 상태를 갖는 것」 (something that knows codex and holds state per session).

Editions 1 and 2 of the Phase 2a TRD guessed at this place on top of layer names and both received an adversarial review BLOCK.

## 결정

**Make the codex app-server channel a struct inside `crates/engram-dashboard-agent/src/backend/codex/` and have it implement `AgentTransport`.**

- **Add not a single line to the shared contract (the six methods of `AgentTransport`).** The session calls only those six, exactly as now.
- What that struct owns: the child process handle and Job Object · `Mutex<Option<ChildStdin>>` (single writer) · its own read thread · the wait map of responses for sent requests · the outbound request id counter · `thread.id` · the ready state.
- **Inbound branching ends inside that struct** — if an arriving line has `method` and has `id`, it is a server→client request (measured 0 in 2a, and if one comes we answer with method-not-found — ★if it is dropped the peer stalls★), if it has only `method` it is a notification (translate, then `core.emit`), if it has only `id` it is the response to our request (taken out of the wait map and woken — **it does not go outside**), and on a JSON parse failure it only logs and does not die.
- The conversation id is recorded into the profile through a callback given by the assembly point — **the backend does not call `ProfileRegistry` directly** (precedent = the claude sid recording path, `crates/engram-dashboard-daemon/src/lib.rs:284-290`).
- **The channel-kind axis becomes three** — terminal (`PtyTransport`) · pipe (`StdioTransport`) · pipe + bidirectional JSON (this one). ★**Do not split by backend name**★.

## 거부한 대안

- **Add request/response to the shared contract** — the option of putting a request/response concept into `AgentTransport`. The two current implementations **do not use** that concept **at all**, yet it would appear only in the contract. The doc of `transport_shape` already warns about the same kind of contamination (`backend/mod.rs:113-123`: one backend's axis ends up driving everyone's choice). `Unsupported` increases on the PTY side.
- **Flip ownership so that the codex component owns the channel and the session borrows it** — it removes the write problem but is **unverified**, and since the kill procedure (ADR-0001) and the reaping path presuppose session ownership, a risk of colliding with those two again remains.
- **Turn the input encoder into an object that holds state per session** — mechanically possible by the verification but with the widest ripple (12 declaration and call sites + one executable in another crate + a tripwire test). If the channel implementation is the one that holds state, **that change becomes entirely unnecessary** — an implementation is an object created per session to begin with.
- **Put it in the `transport/` folder** — a violation of ADR-0004 isolation. The things in that folder are tied to no tool, and that nature is the reason `OutputDecoder` exists.

## 근거

★**The decisive ground = re-reading the original comment text**★. `transport/mod.rs:19-23` read as if it named codex and forbade it, so the edition-2 review blocked this option as 「TRD 가 정할 게 아니라 별도 결정이 필요한 행위」 (an act that needs a separate decision, not something a TRD can decide). The subject of the original sentence is **`StdioTransport`** (the comment is kept verbatim in Korean; in English: the transport (StdioTransport) is a **dumb pipe**, so it must not know what schema the child's stdout bytes follow (claude stream-json / codex protocol / plain text)):

> transport(StdioTransport)는 **바보 파이프**라 자식 stdout 바이트가 무슨 스키마인지(claude stream-json / codex 프로토콜 / 평문) 몰라야 한다.

That is, it describes **one general-purpose pipe implementation**, and codex was merely listed as a schema example. The same intent is in `stdio.rs:7-9` and likewise points at that file itself. **The general-purpose pipe stays dumb as it is, and the schema knowledge lives in the backend folder — both rules are kept at once.** ★Because this reinterpretation overturns a review verdict, the main agent did not settle it on its own and took the user's decision (2026-09-09).★

**What the verification confirmed** (ownership map + 8-question verification): this shape does **not force** a change to the `AgentTransport` contract. And the three things that were blocked earlier are all resolved — ① the write path (the implementation owns stdin directly) ② the failure representation (`send_input` is already a `Result`) ③ cancel (that implementation implements `interrupt()`, a method already in the shared interface).

**Invariant check:** kill causality 2 verbs · finalize once · the two turn-observation cleanup points · backend name isolation — none is touched.

## 영향 / 불변식

- ★**For the first time there is an implementation where `interrupt()` really works**★ — today `StdioTransport::interrupt` returns `Unsupported` (`transport/stdio.rs:316-321`) and `ControlCaps.interrupt` is a literal `false` (`:363-384`). That reported value has to become honest through injection at the assembly point (the decision on that lever = TRD §10, the main agent's decision = extend the constructor arguments).
- **`transport_shape` gets a third variant.** The assembly-point rule that looks at that value and injects caps (`manager.rs:79-99`) grows by one line, and ★that line automatically decides the answer to 「챗 표면에 그리나」 (do we draw on the chat surface)★ (`structured: true` → the frontend picks `rich` at `renderMode.ts:24`).
- **The codex backend must declare `transport_shape`** — now it inherits the default `Pty` (`backend/mod.rs:126`). If the declaration-table tripwire (`backend/mod.rs:779`) is not updated in the same commit, that test turns red.
- **Read thread lifetime** — that struct owns its own thread, so `shutdown()` cleans that up too. ★It must finish within the 2 verbs of kill causality — do not create a third verb.★
- **The Phase 1 PTY codex path stays alive** — this decision **adds** a channel and does not remove the existing one. The same backend ends up with two channels, and the problem that their permission policies differ is handled by ADR-0188.
- **Trigger to reopen this decision** — if another tool starts using the same protocol (an app-server-family JSON-RPC), that implementation is tied to no particular tool, so it moves up to `transport/`.
