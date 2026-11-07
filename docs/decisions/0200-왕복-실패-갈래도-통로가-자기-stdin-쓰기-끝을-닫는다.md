# ADR-0200: The round-trip-failure branch too — the transport closes its own stdin write end

- 상태: 확정 (2026-09-15, basis: independent findings by 3 review lenses + mutation verification — if the gate is revived the rejection test fails after 20 seconds, if reverted it passes within 1 second)
- 관련: Amends ADR-0199 (the scope of branches that close stdin) · Supersedes none · ADR-0001(kill causality, 2 verbs) · ADR-0005(finalize once) · ADR-0019(reaper single consumer) · ADR-0082(disposition of resume failure) · ADR-0201 · `backend/codex/transport.rs::writer_loop`

## 맥락

ADR-0199 decided that **only on the record-failure branch** the transport closes its own stdin. The ground for drawing that scope narrowly was 「그 갈래에서만 상대의 생존이 **확정**된다」 (the peer's liveness is **confirmed** only on that branch) — since it tripped after the round trip succeeded, it is certain that the peer is alive and reading our stdin.

In the meantime, with the codex resume wiring coming in, **the branch where the round trip itself fails became reachable in release.** When codex rejects `thread/resume` (an unknown thread · a configuration error · a subagent thread), a JSON-RPC error comes back, and it is classified as a 「왕복 실패」 (“round-trip failure”). But on that branch **the peer is alive and well.** A rejection is not a death but a response.

So there are now cases where the premise of ADR-0199 (「왕복 실패 = 상대 생존 미확정」 (“round-trip failure = peer liveness unconfirmed”)) is false, and the narrowly drawn scope became a defect as it stands:

stdin is not closed → the child, reader and writer survive → the reader does not see EOF → `OutputCore::finish` does not run → `ReapMsg` is not emitted → **the reaper is the single consumer of that message, so the reap paths are 0** (ADR-0019). An agent stands on screen as Running, but all input is rejected and nobody reaps it. Boot restore multiplies this by the number of resumable profiles.

## 결정

**We close regardless of branch.** When the handshake fails — whether it tripped on the round trip or on the record — the transport releases its own stdin write end.

★**Confirmation of liveness is not a precondition for closing**★ — if the peer is already dead, the action is a no-op, and if it is alive, it is EOF. Both are safe. On the other hand, **there is no safe outcome on the not-closing side**: if the peer is alive, it is the deadlock above, and if it is dead, it is the same whether we close or not.

The `while_recording` field stays but **only the wording is split.** A record failure says 「이 화신을 쓰지 않는다」 (“this incarnation is not used”), and a round-trip failure says 「연결이 서지 못했다」 (“the connection could not be established”) — the two have to tell a person different facts. We do not use that field as a behavior branch again.

## 거부한 대안

- **Keep ADR-0199's scope and handle rejection separately** — one more decision that recognizes a rejection arises inside the transport, and that decision is tied to the vendor error code. As measured, `-32600` is not exclusive to 「모르는 스레드」 (“unknown thread”); configuration errors and subagent threads use the same code too (measured against a real codex app-server, 2026-09-15). Splitting by code is wrong.
- **On rejection, fall back to a new thread** — a direction ADR-0082 already discarded, and there is one more ground on the codex side: the same `-32600` also comes for configuration errors, so the fallback would **overwrite a perfectly good handle**. Reconfirmed by user decision (2026-09-15).
- **The transport waits in a bounded way and then reaps directly** — we actually implemented it for one round and reverted it. It crosses head-on the line ADR-0199 drew (「통로가 `kill`·`TerminateJobObject`·`core.finish` 중 하나라도 부르면 위반」 (“the transport violates it if it calls any one of kill·TerminateJobObject·core.finish”)), and the reason ADR-0001 ties the terminating party to one gets blurred. The reaping is done by the side that judged it a failure (the manager) with the existing 2 verbs → ADR-0201.

## 근거

- **3 review lenses pointed at the same spot independently** (invariants · concurrency · cross-family blind). The blind lens reached the same defect without receiving any of the decision grounds.
- **Mutation verification**: if the `while_recording` gate is revived, the rejection test fails after 20 seconds with `관측된 상태: []`, and if reverted it passes within 1 second. The green is not sitting on top of the implementation.
- The causality that ADR-0199 wrote for the record-failure branch (close → peer exit → reader EOF → finish → reaper) is **reused as is.** There is no new machinery.

## 영향 / 불변식

- The 2 verbs of ADR-0001 are as they are — this decision is **releasing the write end**, not kill. ADR-0199's line that the transport does not call kill is **alive** (the implementation that crossed that line was removed in ADR-0201).
- The production points that take the stdin lock in a blocking way are still **two**, and `tests::the_production_blocking_stdin_locks_are_counted` keeps that count. If a third appears, the lock-safety argument of this decision must be redone.
- How many ms the peer actually takes to finish after closing is **unmeasured on the rejection branch.** We do not borrow the 41~51ms of the record-failure branch.
