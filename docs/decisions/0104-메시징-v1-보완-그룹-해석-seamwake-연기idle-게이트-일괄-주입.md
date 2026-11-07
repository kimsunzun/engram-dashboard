# ADR-0104: Messaging v1 addendum — group resolution seam · deferring wake · idle gate with batch injection

- 상태: 확정 (2026-07-24, basis: user decision — 3 issues discussed and settled in a re-review session right after the S18 design was fixed (ADR-0103)) · 부분 폐기 by ADR-0112 (decision 1 the source of the runtime-registered roster, decision 2 parking for a sleeping recipient) · 부분 폐기 by ADR-0231 (decision 3 adds to the idle of the idle gate that the user list is empty and that it is not stopped after an error (no upper bound))
- 관련: ADR-0103(the messaging v1 main body — this ADR is its addendum) · `docs/process/S18-messaging-v1/spec/messaging-v1-spec.md` §4·§5·§7·§8 · ADR-0096(envelope wrap seam) · ADR-0101(WYSIWYA name) · ADR-0012(seam isolation) · step-log S18 · Amended by ADR-0112 (decision 1 the source of the runtime-registered roster, decision 2 parking for a sleeping recipient) · Amended by ADR-0231 (decision 3 adds to the idle of the idle gate that the user list is empty and that it is not stopped after an error (no upper bound))

## 맥락

After the messaging v1 design was recorded in ADR-0103, the user's re-review before entering implementation raised 3 issues: ① the unit of group definition — folders (tree organization) and a sub-agent hierarchy will appear in the future, so where is the group anchored ② for a sleeping agent (the process is closed but the session can be restored) as a recipient, do we wake it to deliver ③ the message injection timing — do we inject while the receiving LLM is in the middle of a turn, and when several pile up do we inject one at a time. All three are behavior and policy (felt by the user), not internal implementation, so a user decision was needed.

## 결정

1. **Group resolution = a seam (GroupRegistry: a "group name → member list" resolver, with a pluggable source structure).** v1 sources = the runtime-registered roster + the built-in `@all` (ADR-0103 decision 4 kept). **When folders come to exist owned by the daemon, a folder = an additional source** (`@폴더명`) and is planned as the canonical unit of organizational broadcast. **A sub-agent hierarchy is not adopted as an address unit.**
2. **wake not adopted (v1) — a sleeping recipient is parked the same as an absent one.** Both general wake (every send wakes) and the request-limited variant are excluded from v1 — the leading v2 candidate = **wake-on-request** (same as the term in spec §8). Add it when the need is demonstrated (the user: "나중으로, 필요할 때 추가").
3. **Injection timing = an idle gate + a batch flush.** If the recipient is in the middle of a turn (busy), hold the message in the daemon mailbox, and on entering idle (turn end) or appearance (spawn/epoch) inject all the accumulated messages in a batch, oldest first (each message in its own envelope). The `delivered` ledger is stamped only at the actual injection time. busy/idle observation is a backend capability (stream-json turn events) — a backend that cannot be observed falls back to immediate injection. Batch handling is verified with emphasis in the acceptance criteria (strengthened at the user's instruction).

## 거부한 대안

- **Adopting a sub-agent hierarchy as the group address unit** — the only real demander of hierarchical broadcast ("everyone under me") is the orchestrator (the parent), and the parent gets the same effect by registering a runtime group as it spawns (duplicate feature). Also, a hierarchy roster is dynamic (changes at every spawn), so the blast radius of a send is hard to predict and it clashes with the "snapshot at the moment of sending" principle. Revisit by adding a seam source once the orchestration model is established + real demand is shown.
- **Doing the folder system work first (group = folder, unified)** — messaging v1 would wait for the daemon migration of the tree/folders (a separate step) + a folder is exclusive membership, so it cannot express overlapping groups (a task force spanning several folders). With the seam structure, when folders appear they are absorbed non-destructively by adding a source, so there is no reason to wait.
- **wake-on-send (included in v1)** — waking = spawning a claude process + token cost, a risk of mass spawning on a broadcast, and the scope of building a new "name → sleeping restorable session" registry. For plain notification there is no benefit against the cost, and request-limited wake is the leading candidate but is excluded from the v1 scope (user decision). Parking already provides the delivery guarantee (flush at natural wake-up).
- **Immediate injection (ignoring busy, keeping the current way)** — pushing to stdin in the middle of a turn piles up in the CLI's internal queue and leaves the daemon's hands: the ledger `delivered` diverges from "actually saw it" and control of batching and order is lost.
- **Dribble injection one at a time** — one turn per message = N times the tokens and time, and a contradiction within a batch ("#3: ignore #1") is found by the recipient only after it has already acted on #1. With a batch, the receiving LLM sees the whole and sorts out priorities and contradictions itself (isomorphic to how a person opens a mailbox).

## 근거

- User decision (2026-07-24, this session): group address unit = the folder direction (hierarchy not adopted) · wake deferred · idle gate + batch adopted + an instruction to strengthen batch verification.
- The idle injection precedent = the Claude Code team's file inbox + idle injection (confirmed in the research in the 근거 section of ADR-0103) — isomorphic to our mailbox wait and flush.
- Unifying on the XML envelope (ADR-0103 decision 1) supports the premise of batch injection — message boundaries within a batch are structurally clear (with the colon format the boundaries would be blurred).
- Unverified (implementation QA item, spec §7): the real queuing behavior of the CLI (stream-json) when injecting to stdin in the middle of a turn · the accuracy of busy/idle observation (turn events).

## 영향 / 불변식

- **GroupRegistry is a resolution seam** — adding a group source (roster/@all, a future folder or hierarchy) must leave the messaging pipeline (ingress · MessagingService) unchanged. If source knowledge leaks into the pipeline, this decision is violated.
- **`delivered` = the actual injection time** — pre-injecting to the CLI stdin during busy and stamping delivered breaks trust in the ledger (no bypassing the idle gate).
- **flush = batch · oldest first** — the appearance flush (ADR-0103 decision 5) and the idle flush share the same rule (no two paths).
- **A sleeping recipient = parking** (v1) — do not slip in a wake path. Only the user may reopen it (v2 candidate: request-limited wake).
- busy/idle observation is declared as a capability — forcing the idle gate on a backend that cannot be observed makes delivery wait forever (the immediate-injection fallback is mandatory).
