# ADR-0198: Show codex input on screen at once too — merge the synthetic echo by clientUserMessageId

- 상태: 확정 (2026-09-14, basis: user decision + direct reading of the vendor schema and integration tests) · 부분 폐기 by ADR-0231 (the synthetic echo only when idle · closes the hydration-order open item under impact (waiting → deliver after history) · rejects the original plan of a formal bubble at the sent spot)
- 관련: ADR-0193(the queue-release decision — **not a reversal**, see the last item of 「거부한 대안」 below) · ADR-0192(backend defaults match claude) · `docs/reference/backend-capabilities.md` §2 · `crates/engram-dashboard-agent/src/backend/codex/protocol.rs` · step-log S21 · Amended by ADR-0231 (the synthetic echo only when idle · closes the hydration-order open item under impact (waiting → deliver after history) · rejects the original plan of a formal bubble at the sent spot)

## 맥락

With claude, the text the user types appears in the chat **at once**, but with codex it appears **after a round trip**. The mechanisms differ — the claude path emits a synthetic echo for the screen along with the input when it sends it (`input_echo_event`), and merges that echo with what claude echoes back later under the same uuid. The codex path does not declare a synthetic echo (default `None`), so nothing is on screen until codex hands back a `userMessage` item.

The user noticed it first on the screen (2026-09-14) and decided to fix it. ★**We have not yet measured how many seconds the delay actually is**★ — this decision stands not on a figure for 「느리다」 (“slow”) but on the fact that **behavior diverges from claude** (the axis ADR-0192 set when it said to match defaults to claude).

Why it is possible only now — **it came to light this time that codex has a key to merge on.** `turn/start` accepts `clientUserMessageId`, and the echoed-back `userMessage` item comes with that value as `clientId`. It entered in codex 0.140.0 and is present in the 0.154.0 we use. ★**The public documentation does not have this field, and neither does the `protocol.rs` we wrote by hand, so we missed it twice**★ — so 「합칠 키가 없다」 (“there is no key to merge on”) passed for fact for a long time.

## 결정

**Codex too emits a synthetic echo for the screen when it sends input, and merges it with the echoed-back item by `clientUserMessageId` ↔ `clientId`.** The same shape as claude.

- The sending side makes that id and puts **the same value in both the synthetic echo and `turn/start`**.
- When the echoed-back `userMessage` item arrives carrying that id, we **merge** (we do not make a new line).
- ★**The queue-release decision axis is not touched**★ — 「지금 보낼 수 있나」 (“can we send now?”) is decided by each transport implementation from its own state, as in ADR-0193. This synthetic echo is **for the screen and is used only for the screen.**
- **What codex echoes back may be normalized** — when merging, the display text keeps **our local copy** (the vendor TUI does the same for the same reason).

## 거부한 대안

- **Keep as is (show after the round trip)** — the user felt it on the screen and decided to fix it (2026-09-14). ★The ground for rejection is the user's observation, not a measured figure★ — there is still no delay figure. But since ADR-0192 already decided 「백엔드 기본값은 claude 와 동일하게 맞춘다」 (“match backend defaults to claude”), **the fact of the divergence itself** is the reason to align, regardless of the figure.
- **Render only ours and drop the items codex echoes back** — it looks simpler because no merge key is needed, but **on the reconnect·replay path that echo is the only record**. The vendor TUI has a separate replay branch that skips matching and renders unconditionally, and that is its safeguard. If we drop it, what I sent disappears from the screen after reconnecting.
- **Merge by content comparison (without a key)** — the vendor really uses this as a fallback for old-version servers, but that key **joins the body without boundaries, counts only the number of images, and does not look at audio·mentions at all** (a TODO comment in the vendor source admits this limit). Two sends of the same text, or two sends that differ only in attachments, collide. **We have a key, so there is no reason to use this fallback.**
- **Reverse ADR-0193** — it is not a reversal. The main point of that ADR is **where to make the queue-release decision**, and that remains valid. What that ADR rejected is the proposal to 「합성 에코를 **바쁜지 판정 재료로** 쓰자」 (“use the synthetic echo **as material for deciding whether it is busy**”), and this decision does not do that. Of its two reasons for rejection, only the 「이중 표시·dedup 문제」 (“double display·dedup problem”) is resolved this time, and the reason 「목적이 다른 표면을 판정에 전용한다」 (“it repurposes a surface with a different purpose for the decision”) is untouched.

## 근거

- `turn/start`'s `clientUserMessageId` and the echoed-back item's `clientId` are **in the codex rust-v0.154.0 schema** (read directly). The echo on that path is **asserted by the vendor integration tests** — not only for `turn/steer` but also for `turn/start`.
- Introduction range = absent in 0.135.0 and present in 0.140.0 (schema comparison by tag).
- The vendor TUI merges by the same key, and keeps a content-comparison fallback only for old-version servers that have no key (vendor source).
- A peer implementation (a client that actually uses the codex app-server) does it the same way — optimistic display with a client-made id, then **in-place merge** (not delete then reinsert — to prevent flicker).

## 영향 / 불변식

- ★**`clientUserMessageId` is a correlation key, not an idempotency key**★ — if the same id is sent twice, **it may reach the model twice, yet on screen it looks merged into one line**. Do not use this id on resend·reconnect paths to decide 「이미 보냈나」 (“did we already send it?”). If duplicate prevention is needed, block it **on our side**.
- **A version lower bound arises** — below codex 0.140.0 this field does not exist. Below that there is no key to merge on, so **falling back to today's behavior of not emitting a synthetic echo is correct** (we do not build a content-comparison fallback — the reason for rejection above).
- **The echoed-back item comes twice** (`item/started`·`item/completed`). The place that merges must absorb that duplication too — otherwise it becomes two lines even after merging.
- **The queue-release decision (ADR-0193) and this echo do not know each other.** If code appears that reads this echo as evidence for 「턴이 돈다」 (“a turn is running”), that is the regression.
- **Terminal-mode codex is not applicable** — that mode has no such protocol at all.
- The display-delay figure is still **unmeasured**. This decision does not depend on that figure, but measuring it once during GUI measurement gives a baseline to measure later regressions against.
- ★**Implementing this echo collides in order with screen restore — that spot must be decided together**★ (added 2026-09-16 · the decision of this ADR is unchanged). As ADR-0204 decided, **a resumed codex puts the past conversation into the ring just before it is ready**. But the transport **accepts input even while connecting and queues it**, and the synthetic echo goes in directly by `core.emit` without passing through the door — what the door blocks is *going out*, not *entering the ring*. So if the user types in that window (up to 10 seconds), **the line just typed is drawn on top of the restored old conversation**. Right after a manual re-activation, that slot has focus, so this is a common path.
  - ★**Today this does not happen — and that fact disappears when this ADR is implemented**★: because codex's encoder does not declare an echo, `input_echo_event` short-circuits to `None` (in both modes). The regression net that keeps that property is `neither_mode_makes_a_synthetic_input_echo`(ADR-0193), but **when this ADR is implemented, that test being deleted is normal.** So if this interaction is kept only in that test's comment, it evaporates along with the implementation — which is why that spot is here.
  - **What to decide** = whether the echo goes **before or after** the restored history, and whether to accept input during hydration (accept but defer the echo · block it entirely · leave as is). ★**「아예 막기」 (“Block entirely”) makes typing dead for up to 10 seconds, so it is bad in its own way**★.
  - **To be looked at together, at the same spot** with the last item above (measuring the display delay) — GUI measurement can answer both, and the hydration window is the worst case of that delay.
