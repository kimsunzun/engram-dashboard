# TODO — To-Do List

**This file is an index** — one line per item; the detail lives in the individual documents under `todo/`. Decision records (`decisions/`) hold *why we decided that*, git log (`S<n>:` commit titles) + each work folder's `README.md` hold *when we did what*, and this file holds **what has not been done yet**.

**Four rules**

1. One to three lines per item. When it grows longer, promote it to `todo/<이름>.md` and leave only one line plus a pointer here.
2. **Always write "왜 지금 안 하나" (why not now).** Without it, the next reader cannot judge and the item just piles up.
3. ★When resolved, **delete it**★. Decision records are append-only, but this is a living list, and it stays short only if resolved items are removed.
   - **When deleting, delete the detail file under `todo/` too.** If you delete only the index line, orphans pile up in the folder.
   - ★**Before deleting, verify that the content was actually moved into its canonical home**★ — the destination differs per item (a decision goes to `decisions/`, when/what goes to the commit message (`S<n>:` title) + the work folder's `README.md`, a code-sync convention goes to `reference/`). Delete without moving it and the item survives nowhere and evaporates. 「확인만 됨」 (observed only) items are not *resolved*; a **judgment is made** on them, so the time to delete is when that judgment is recorded as an ADR.
4. Do not write rationale or rejected alternatives here — keep them in decision records and point to the number only.

**Boundary with `tracking.md`**

Both are deleted when resolved — `tracking.md` keeps only open T- items, and of the resolved ones moves to `tracking-archive.md` in a single line only those that other documents or code point to by number (the record of resolution is borne by ADRs, commit messages and work-folder `README.md`s). So *deferred matters that must carry rationale, investigation and re-introduction triggers* go to T- items in `tracking.md`, and *work that disappears once done* comes here. When a matter spans both, keep the T- number as canonical and leave only one line plus a pointer here.

**Four status values**

- **Ready to start (착수 가능)** — anyone with free hands may pick it up.
- **Waiting for trigger (트리거 대기)** — opened only when a condition arrives.
- **Owned (소유됨)** — someone holds it right now.
- **Observed only (확인만 됨)** — only observed; judgment deferred.

**Work size — there is no fixed standard; whoever picks it up judges again**

The value is only there to let you gauge size at a glance, not a contract. **If the written size differs from the real one, correct it in place** — do not grow or shrink the work to match the number here. Roughly: **Small** = finishes within one session · **Medium** = one or two design judgments are involved · **Large** = a user decision or a major design comes first · **Unknown** = investigation is needed before a size emerges.

---

| Item | Status | Size | Detail |
|---|---|---|---|
| Registry consistency | Ready to start | Large | [`todo/roster-consistency.md`](todo/roster-consistency.md) |
| Doc drift bundle | Ready to start | Small~Medium | [`todo/doc-drift.md`](todo/doc-drift.md) |
| Gate scope-escape check — whether to add it to the conventions | Ready to start | Small | [`todo/gate-scope-check.md`](todo/gate-scope-check.md) |
| Verification procedure (decisions · deferred cleanup) | Waiting for trigger | Unknown | [`todo/verification.md`](todo/verification.md) |
| Command surface — owners are not declared | Waiting for trigger | Medium~Large | [`todo/command-ownership.md`](todo/command-ownership.md) |
| Display policy is scattered across surfaces | Waiting for trigger | Large | [`todo/display-policy.md`](todo/display-policy.md) |
| Terminal dimension propagation | Waiting for trigger | Medium | [`todo/terminal-sizing.md`](todo/terminal-sizing.md) |
| Replay · subscription | Waiting for trigger | Medium | [`todo/replay-subscription.md`](todo/replay-subscription.md) |
| Naming · records | Waiting for trigger | Large | [`todo/naming-and-records.md`](todo/naming-and-records.md) |
| The QA procedure doc is a CI test input — decoupling | Waiting for trigger | Medium | [`todo/qa-doc-test-coupling.md`](todo/qa-doc-test-coupling.md) |
| Frontend module split | Owned | Unknown | [`todo/frontend-module-split.md`](todo/frontend-module-split.md) |
| Observed only (judgment deferred) | Observed only | — | [`todo/observed-only.md`](todo/observed-only.md) |

**Why each size was assigned** (one line each — the detail document is canonical):

- **Registry consistency = Large** — The investigation is done, but choosing among five candidates is a user decision, and the requirements have grown to three (ordering · lost-frame recovery · race with state notifications).
- **Doc drift = Small~Medium** — Of the two that remain, the unused dependency touches the manifest, so a build check follows, and the ADR-0175 errors require building the list first.
- **Verification procedure = Unknown** — Local reproduction failed on every run, so what to treat as the thing to fix is not settled. There is no means to measure the size.
- **Command surface = Medium~Large** — Changing daemon command arguments + deleting frontend commands + cleaning up ownership are one body, and it is tied to the agent-tree work.
- **Display policy = Large** — Phase 1 (terminal only) is small, but phase 2 replaces an app-wide seam and an ADR reversal follows.
- **Naming · records = Large** — The `epoch` rename alone is 885 code identifiers (measured). That is also the reason for deferring.
- **Frontend module split = Unknown** — The scope is not decided. There is no size pressure (11,285 non-test lines, largest file 992 lines — measured 2026-08-26)
- **QA procedure doc coupling = Medium** — Extracting the script itself is small, but the QA procedure, the test, the CI path filter and ADR-0232 move together.

---

## Common — for any item

- Commit-title step number = a new number for each new topic — the canonical source for when to get one and how to pick it is `handbook/branch-commit.md`.
- Code changes go through `/engram-implement` (including review and QA).
- Branch from master. **Do not keep master checked out permanently** — the canonical source for the integration procedure is `handbook/branch-commit.md`.
