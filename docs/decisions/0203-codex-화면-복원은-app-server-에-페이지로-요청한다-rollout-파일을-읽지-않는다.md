# ADR-0203: Restore the codex screen by requesting pages from the app-server — do not read rollout files

- 상태: 확정 (2026-09-15, basis: user decision + measured against a real codex app-server 0.154.0 + a full scan of 901 rollouts) · 부분 폐기 by ADR-0204 (the spot where the restore events attach)
- 관련: ADR-0079(the daemon reads claude `.jsonl` and seeds the buffer) · ADR-0008(no extending features through tracking files) · ADR-0172(reading scrollback and locking features are different grades) · ADR-0004(isolating backend knowledge) · `backend/mod.rs::AgentBackend::resume_transcript_events` · Amended by ADR-0204 (the spot where the restore events attach)

## 맥락

Even when resume succeeds, a codex slot **starts with an empty screen.** codex does not give us restore events, and we read nothing either.

claude solved the same problem with ADR-0079 — we read `~/.claude/projects/<슬러그>/<sid>.jsonl` ourselves and seed the buffer. The user's first-line policy was 「유지보수 비용이 없으면 claude 와 맞춘다」 (“if there is no maintenance cost, match claude”), so codex too was leaning toward reading rollout files directly.

**When we re-measured that premise, it broke.** The reason claude is cheap is that the file's line shape is **the same as** the live stream's line shape, so no new parser was written. codex is not like that.

## 결정

**We receive codex history by requesting it from the app-server.** We do not read rollout files.

- `thread/resume` is sent with `excludeTurns: true`, and the history is received separately **page by page** (`thread/items/list` — cursor·limit·direction). The backward cursor that the resume response returns is the entry point.
- The received items are **exactly the types we already parse.** If we only strip the envelope, the existing translation is reused.
- The spot where it attaches is the same as claude's — the backend hands over the restore events and the manager seeds **before** launching the child (the order invariant of ADR-0079 is kept).

## 거부한 대안

- **We read the rollout file directly (the same shape as claude)** — the user's first choice, and **rejected by measurement**. The rollout lines have **nothing at all in common** with our codex decoder's input: the envelope is not JSON-RPC (there is no top-level `method` at all, so the classifier drops all of it), notification names are snake_case vs slash/camel, and item kinds are PascalCase vs camelCase. The three layers each go off independently. What is reused is only the leaf string helpers and the output types, so a **new parser + a new vocabulary table** comes into being wholesale. Moreover, that file is not a contracted wire but a **vendor-internal storage format**, so it breaks silently when it changes.
- **Receive the whole history at once (hydration)** — the vendor itself deprecates it by putting a `deprecationNotice` in the response: "Full-history hydration is deprecated for paginated threads; use `excludeTurns: true`, then page". And a single line is **up to 1.96MB** (measured), so there is only a factor of two of margin against our line cap of 4MiB.
- **Leave it (an empty screen)** — half the value of resume is on the screen. Explicitly rejected by the user.

## 근거

We brought up a real `codex app-server` 0.154.0 and measured directly over JSON-RPC (2026-09-15):

- **`excludeTurns` is honored.** Omitting it or `false` gives a byte-identical response (1,259,774B, 120 items), and `true` gives 6,486B with `turns: []`. Our code was right to be built trusting it.
- ★**The item types are the same as live**★ — in the vendor schema, the item-definition hashes of the `item/completed` notification, the resume response and the page response are **all identical** (19 variants). Only the envelope differs.
- **There are two paging means** (`thread/items/list`·`thread/turns/list`, opaque cursor, normal termination confirmed with `nextCursor: null`).
- The history is **entirely within the response** — after resume there are 0 `item/*` notifications.
- Size: the resume payload is 0.25~0.4 times the on-disk rollout. ★**Paging does not bound the line size either**★ — `limit` cuts the count, not individual bytes, so even at `limit:1` the largest page is 437KB (a single `commandExecution.aggregatedOutput` is that big).

We also confirmed from records whether ADR-0008 is violated — the target of that prohibition is the one file `sessions/<pid>.json`, and its purpose is 「복원 정확성을 벤더 파일에 걸지 말라」 (“do not hang restore correctness on a vendor file”). ADR-0079 adopted transcript reading head-on, and ADR-0172 wrote the difference in grade in plain words ("reading for scrollback revival and locking features are different grades"). That is, **direct reading was not prohibited either**, and this decision was split not by prohibition but by **cost**.

## 영향 / 불변식

- Backend knowledge stays inside the implementation (ADR-0004) — the manager only calls and does not know pages·cursors·item shapes.
- ★**The screen buffer is the real ceiling**★ — the ring is 2MB/4096 entries, so anything older than that is not carried even if received. We only need to fetch that much from the end, and it is not a reason to raise the line cap.
- **Resume itself is rejected for subagent threads** (measured: 93 of 902). A history request is bound by the same constraint.
- That `docs/process/S21-codex-backend/trd-phase2a.md` wrote the three vendor-file reads (`state_5.sqlite`·rollout JSONL·`session_index.jsonl`) bundled in one line as if prohibited is **a description that did not separate the grades.** This ADR is the canon for that spot.
- This decision is **not yet implemented** — the number was fixed first (multiple worktrees in operation), and implementation is a separate step.
