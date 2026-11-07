# ADR-0199: On the record-failure branch the transport closes its own stdin write end

- 상태: 확정 (2026-09-15, basis: review round 3 · 2 coder mutation verifications) · 부분 폐기 by ADR-0200 (the scope of branches that close stdin) · 부분 폐기 by ADR-0292 (the abort premise of the ground that release is unreachable)
- 관련: CLAUDE.md 「핵심 불변식」 kill causality · ADR-0001 · ADR-0005 · ADR-0185 decision 2 · `crates/engram-dashboard-agent/src/backend/codex/transport.rs` `record_session_id`·`writer_loop` · Amended by ADR-0200 (the scope of branches that close stdin) · Amended by ADR-0292 (the abort premise of the ground that release is unreachable)

## 맥락

While laying down the wiring that writes the thread id issued by the codex app-server into the profile (ADR-0185 Phase 2 ①), a new question arose: **where does a failure of the record call go?** The first implementation merged that failure into the **existing handshake-failure branch**.

That branch was originally made for the case where **the handshake itself failed**. Under that premise the child was already dead or silent, so it was enough for the branch to set `Link::Down`, empty the queue, and produce one failure outcome. That branch does not set the `closed` marker and does not touch the child — there was no need to.

**On the new entry path, that premise breaks.** A record failure happens **after the handshake has succeeded**, so the child is alive and reading stdin. Then:

- `closed` is not set, so `next_job` does not return `None`, and the writer cycles `Job::Sweep` forever
- the child is alive, so the reader does not see EOF, and therefore `OutputCore::finish` does not run
- there is no terminal transition, so the reaper does not take that session

Settled state = **on screen it looks like it is running, but all input is rejected, and one child, two threads and a Job Object stay held until a person kills them.** That is, the first implementation, in trying to fix 「조용히 굳는다」 (“freezing quietly”), **swapped it for a freeze of a different shape.**

## 결정

**Only on the record-failure branch** the transport **closes its own stdin write end.** Then the child exits by itself, and after that **the existing causality handles it as is** — reader EOF → `ReaderExit::drop` sets `closed`·`Link::Down` → the pump runs `finish` → terminal transition → reap.

We split the failure reason into `HandshakeFailure { reason, while_recording }`, and the existing branch where the protocol round trip failed is **left untouched.** For that branch the existing decision that 「상대가 살아 있을 수 있으니 루프는 돈다」 (“the peer may be alive, so the loop keeps running”) stays valid (if we do not answer, the peer stops permanently).

**The line = it is a violation the moment the writer calls any one of `child.kill()`·`TerminateJobObject`·`core.finish`.** Right now it calls none of them.

## 거부한 대안

- **Leave it (the first implementation state)** — the freeze above is real. When the coder turned stdin closing off, the new regression net failed with **zero state transitions** (mutation verification). The concurrency lens constructed that interleaving first and the mutation verification reproduced it. Rejected by measurement.
- **The transport drives the normal termination path directly (including kill)** — the two verbs of ADR-0001 (`transport.shutdown()` → `core.join_pump(5s)`) are **owned by the kill handler**, and the transport not ending its own lifetime is the invariant in that file's header. If the transport calls kill, that boundary collapses, and the single owner of the terminal transition (the pump) is shaken too. Rejected by ADR and code.
- **Set only `closed` / set only the shutdown atom** — the writer stops, but **the child is orphaned** (pointed out by the concurrency lens). The process and the Job Object remain as they are, and only half of the original problem is solved. Rejected by code.

## 근거

Two review lenses **independently** judged that this path is not a violation of ADR-0001:

- This path does not send a signal to the child, does not touch handles the child holds, and does not call `core.finish`. What we close is **one write end on our side**.
- The terminal transition is still **the pump's alone** (ADR-0005 undamaged). What this branch does is **make the input** that wakes that pump, not substitute for that transition.
- There is a precedent in the same file — `shutdown()` itself treats the stdin take as a **separate best-effort step** apart from kill.

The body of ADR-0001 prescribes the **order** and **owner** of the two verbs, and does not claim that 「자식은 우리 kill 로만 죽을 수 있다」 (“the child can die only by our kill”).

## 영향 / 불변식

- ★**This branch is unreachable in a release build**★ — the only source of a record failure is a **caught panic**, and the root `Cargo.toml`'s `[profile.release]` has `panic = "abort"`, so there is no unwinding. So the behavior this decision creates is **observed only in unwind builds (debug·test).** In release the same panic is an abort of the whole daemon.
- ★**The first link of the new causality is 「상대의 EOF 행동」 (“the peer's EOF behavior”), and there is no bounded backstop**★ — since the transport cannot call kill, if the peer does not die on stdin EOF, the freeze above comes back **with no signal**. codex updates itself and this protocol has no version field. **Who owns the bounded wait and its disposition is undecided**, and deciding that is the precondition for closing this limit.
- ★**「stdin 은 kill 보다 먼저 닫지 않는다」 (“Do not close stdin before kill”) in the file header has been narrowed to the teardown path only**★ — do not remove this branch on the basis of that sentence. **And conversely, do not reverse the order of `shutdown()` on the basis of this branch** — the latter is the real regression and `shutdown_completes_even_if_a_write_blocks_on_a_full_pipe` guards it.
- **The safety of closing rests on the property that 「운영 호출 그래프에서는 막는 stdin 락 자리가 둘뿐」 (“in the production call graph there are only two places that block on the stdin lock”).** For the file as a whole it is false — the test module already spawns a thread that breaks that property. A gate that measures that property now exists (the number of blocking stdin lock acquisitions in the production section = 2). Before raising that number, decide the calling thread of the new spot.
