# ADR-0121: Mail delivery rule adjustment — splitting @all and @here, order guarantee for the class without a turn signal

- 상태: 확정 (2026-07-31, basis: user decision — "@all은 다 보내는 거고 … 방송 명단만 들어오는 건 다른 네이밍", "새로 쏜 편지는 묵은 것하고 같이 보내야지 순서가 있는데") · 부분 폐기 by ADR-0124 (rejected alternative, strict FIFO, adopted instead)
- 관련: Amends ADR-0116 (definition of @all and the delivery order of the class without a turn signal) · **Amends ADR-0111 (@all list source)** — the `@all` of 0111 decision 4 ("everyone alive at the moment of sending") is renamed `@here` in this ADR, and `@all` is redefined on the registry basis. **The sender exclusion is canonical in spec §4, not in 0111** (the same 2026-07-29 user decision), and applies unchanged to both vocabularies · **Amends ADR-0112 (decision 1, `@all` list source)** — it restates the same definition, so a partial discard is stamped on its header too (the *direction* of the resolution seam survives; only the vocabulary grew to two) · ADR-0104 (idle gate) · ADR-0120 (name uniqueness) · ADR-0119 (registry — the source of `@all`) · `crates/engram-dashboard-messaging/src/groups.rs`·`service.rs` (`// ADR-0121` anchor at implementation) · Amended by ADR-0124 (rejected alternative, strict FIFO, adopted instead)

## 맥락

ADR-0116 diverges from reality in two places.

**One — `@all` is not everyone.** Decision 1 specifies that the `@all` expansion uses the roster (= everyone alive right now). A sleeping agent does not enter the list. The name says "all" but the behavior is "only what is on right now", so **the name does not match the facts.** The user's remark: "@all 방송이 명단에 안 들어오면 그건 @all이 아니라 네이밍을 변경해야 될 듯."

**Two — the order guarantee was thrown away along with it, without a basis.** Decision 7 settled the class of backends without a turn signal as "inject immediately without a gate (**no waiting, no retry**, just send and never look back)". **The only basis for that decision was the user's observation that the CLI has its own input queue**, and what that justifies is **skipping the idle gate**. But the sentence "never look back" also dragged along **skipping the wait for earlier parked items** (FIFO not applied). These are different problems — the first is "is the other side ready to receive", the second is "in what order do the letters arrive".

As a result, today a new letter is delivered **by itself, jumping ahead**, even when older letters are parked. The user's remark: "새로 쏜 편지는 묵은 것하고 같이 보내야지 순서가 있는데 앞지른다고?"

## 결정

**1. Split `@all` and `@here`.**
- **`@all`** = the whole registry (alive + sleeping) **− the sender**. The share for a sleeping recipient is parked and delivered when that agent appears.
- **`@here`** = everyone alive right now **− the sender**. The current `@all` behavior corresponds to this.

**The sender exclusion applies to both vocabularies. The canonical source of that clause is spec §4, not an ADR** ("exclude the sender from the expansion" — 2026-07-29 user decision). ADR-0111 decision 4 does not have that wording, so **implementing from 0111 alone misses it** (the confusion arose because the current `groups.rs` comment jointly attributes it to `spec §4 · ADR-0111 결정 4` — the canonical source is the spec). The implementation work is "rename the current behavior to `@here` + add a new `@all` on the registry basis".

**2. The class without a turn signal also keeps the order.** Narrow decision 7's "no waiting, no retry" to **"no idle gate, but the order is kept"**. If there are parked items ahead of that recipient, a new send does not jump ahead, and **older items go out together with it, oldest first.** Skipping the idle gate (not asking about busy) remains valid.

**If an injection fails midway through a batch, stop at that point and leave the remaining items parked** — the front part already sent stays `delivered`, and the failed item and all subsequent items stay `pending`. A partial success is not turned back into a total failure (in the ledger transition graph, `delivered → failed` is illegal). Decision 7's "no retry" assumed a single item, so in a batch this rule takes its place.

**3. The ledger notation does not change.** Terminal mode is also `delivered`. The evidence is the same — whether json or terminal, what `delivered` means is "written to stdin and fully accepted", and there is no evidence in either that the agent read it.

## 거부한 대안

- **Keep the `@all` name and supplement the behavior with an explanation** — if the name stays false, the sending LLM believes "I told everyone" and misses the sleeping recipients. This is not a problem to cover with documentation.
- **Drop `@all` and keep only `@here`** — the user judged there are cases that need "really everyone" ("그러라고 만든 기능임"). Piling items up for a sleeping recipient already works with existing machinery (sleep parking), so it is no new cost.
- **Keep the order but queue behind (strict FIFO — put the new letter at the end of the queue and stop)** — the delivery of the new letter becomes **tied to the parking retry loop.** The class without a turn signal has no turn-end doorbell, so the occasions on which the queue opens are limited to appearance (respawn) and the service's internal self-healing (re-check right after parking, deferral recovery). The user's intent is "send it all at once", not "it goes out sometime by retry". *(The exact conditions under which self-healing fires are to be confirmed at implementation — unverified)*
- **Keep the current behavior (jumping ahead)** — the order breaks. There is no reason to keep a rule whose basis does not cover it (§맥락).
- **For the class without a turn signal, do not park on a write failure but fail immediately** (a proposal that removes the ordering problem at the root) — rejected after review. A write failure is **a missing session or a stdin write/flush error**, and while the pipe is alive it does not fail even if the CLI is busy. So a failure means "cannot be put into this recipient right now", and that recipient's profile remains — waiting and putting it in again is **the same treatment** as ADR-0116 decision 1 ("if the profile remains, wait"). If only here we split off immediate failure, then in the same situation a letter sometimes waits and sometimes dies, depending on when it arrived.
- **Mark terminal-mode delivery separately in the ledger** (e.g. unverified-complete) — the delivery evidence is literally identical to json mode, so there is no basis for splitting the notation. The actual risk difference (at the moment of injection it may be swallowed by a modal widget) was already recorded by ADR-0116 decision 7 as "known and accepted", and is hung as a spec §7 measured item. **If it is observed, we revisit it then, with evidence** — adding a notation now would make every consumer that reads the ledger handle two kinds.

## 근거

- User decision (2026-07-31): the `@all` naming — "어쨌든 @all은 다 보내는 거고 @all-일부분? 어쨌든 네이밍 다른 걸로 해서 2개 두자고" · the order — "ㅇㅇ 새로 오면 기존 거 뒤에 담아서 한꺼번에 보내는 거고" · the ledger — "어쨌든 별도로 구분하는 것보다 Delivered가 좋다는 거지? 그냥 이대로 진행하고 추후 확인".
- **Basis for choosing the `@here` word:** Slack uses the same distinction (`@channel` = everyone / `@here` = currently active). The LLM that drives an agent is likely to already know that convention.
- **Checking the scope of decision 7's basis (measured in code):** the current comment marks the FIFO skip as "★귀결(알고 수용)★" (★consequence (known and accepted)★) — the code itself states that it is derived from "just send and never look back", not an independent decision. This is the core basis of this revision.
- **Checking the doorbell path (measured in code, corrected by cross-verification):** besides the turn-end signal (`flush_for_agent`) and the appearance roster diff, **there are points inside the service that call the same flush path** (self-healing right after parking · deferred-flush recovery · notification delivery). So the statement "the only occasion to wake it is appearance" is not accurate — this is the basis for narrowing the FIFO rejection reason above to "tied to the retry loop" rather than "can never go out".
- **Checking the write-failure conditions (measured in code):** `write_stdin_observed` → session lookup → `transport.send_input`. **Ok = both `write_all` and `flush` succeed** = the requested bytes were fully accepted. Err includes ① a missing session ② **the stdin handle already closed so that not even a write was attempted** ③ write/flush I/O errors, and **does not prove process death** (that inference went too far — withdrawn after a cross-verification remark).

## 영향 / 불변식

- **`@all` looks at the registry, `@here` looks at the roster.** If the two vocabularies look at the same source, this decision is violated. **Both exclude the sender** (canonical = spec §4. ADR-0111 decision 4 does not have this wording, so do not look for it there — if it is missed, the sender receives its own letter).
- **The price of `@all` (explicitly accepted):** as many parked items arise at once as there are sleeping entries. It is bounded by the 24h TTL, but it can hit the mailbox cap.
- **Define the order rule in one place** — extract the predicate "this class does not ask the idle gate" as one, and have the decision points call it. If the same condition is written separately in two places, only one gets fixed, and the send side and the flush side see different worlds (a class of defect the 2026-07-30 review caught elsewhere).
- **Skipping the idle gate stays** — not asking the class without a turn signal about busy is as in decision 7. This ADR reverts only the order. Reverting to "cannot observe, so cannot deliver" is still forbidden (ADR-0116 rejected alternative).
- **Tests that flip:** the kernel test that asserts a new send jumping ahead of parked items in the class without a turn signal changes to "older ones go out together". That is normal, since the decision changed.
- Unverified (to confirm at implementation): the exact conditions under which the service-internal self-healing flush fires · whether the ledger rows keep to the transition graph on a partial batch failure.
