# Replay · subscription

> **Status:** Waiting for trigger · **Canonical index:** [`../todo.md`](../todo.md)

> **Related:** the 「뷰·데몬 껐다 켜기 정합」 (view · daemon off-and-on consistency) item below was reopened, tied to the lump in [`roster-consistency.md`](./roster-consistency.md) (2026-08-26) — the item itself stays here as is.

- **The replay head is truncated but nobody is told "truncated"** — even when the ring buffer (2MB · 4096 chunks) discards the front, the client receives it as a complete replay. Drawing from the middle of a frame breaks the TUI. It actually happens in long sessions whose output exceeds 2MB. **Why not now:** a separate matter, and it has not actually been observed yet.
- **View · daemon off-and-on consistency (뷰·데몬 껐다 켜기 정합)** (classified by the user as a later issue) — ⓐ the registry snapshot has no freshness barrier: a late-arriving old snapshot detaches a live agent, and the trigger to revert it hangs on an unrelated event (another agent's creation · exit). ⓑ Even after upgrading, attaching to an old daemon leaves the symptom as is: the admission gate is only the protocol number and there is no build identifier. **Structural clue:** the registry query was originally a read but was promoted to attach authority, so as query callers increase in the future, all of them become detach triggers — settle this structure first during the consistency work.
- **`resume=false` is ignored** — if the profile has a session id, even a fresh request goes as a resume. Unclear whether this is intended. **Why not now:** it needs judgment and is a design decision.
- **「잠자기 모드」 (sleep mode) of the mobile client** — a hidden area drops its subscription and reattaches when shown (reattach + replay). **The PC stays keep-alive as it is now** (ADR-0056 · user decision 2026-09-25). **Trigger:** when building the mobile client. Investigation = `../research/hidden-tab-terminal-subscription-survey-2026-09-25.md`.
