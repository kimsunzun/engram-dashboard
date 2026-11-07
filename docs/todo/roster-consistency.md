# Registry consistency (T-32 + the view · daemon consistency of 「재생·구독」 + T-16) — a large lump, the investigation is done

> **Status:** Ready to start · **Canonical index:** [`../todo.md`](../todo.md)

> **Related:** the body of the 「뷰·데몬 껐다 켜기 정합」 item pointed to below remains in [`replay-subscription.md`](./replay-subscription.md).

- **Scope:** not a single defect but a **lump bundling three requirements** — ① emission order ② frame-loss recovery ③ race between state notifications and snapshots. ★③ is an axis newly found by adversarial review during the investigation, so it is in neither `../tracking.md` nor [`replay-subscription.md`](replay-subscription.md)★.
- **★Read first = [`../research/roster-broadcast-ordering.md`](../research/roster-broadcast-ordering.md)★** — the peer survey · decision rules · repository constraints · adversarial review results are there. **Candidates A/B/C/D/E are laid out and none has been chosen** (the provisional recommendation was rejected in review). Starting without that document means rerunning the same investigation.
- **Canonical:** `../tracking.md` T-32 · T-16 · [`replay-subscription.md`](replay-subscription.md) 「뷰·데몬 껐다 켜기 정합」 (view · daemon off-and-on consistency).
- **★This is something the user once deferred and has reopened★** — that item in [`replay-subscription.md`](replay-subscription.md) carries 「사용자가 나중 이슈로 분류」 (classified by the user as a later issue), but on 2026-08-26 it was reversed to be handled.
- **Prerequisite to start:** the structural decision written in that same item comes first — *the registry query was originally a read but was promoted to attach authority*. If that is not decided, the design of the ordering fix splits.
- **★This is a major design decision, so do not enter implementation without the user's choice★** (`CLAUDE.md` 「개발 순서」 (development order)). The investigation is done, so the next action is to raise the options to the user.
