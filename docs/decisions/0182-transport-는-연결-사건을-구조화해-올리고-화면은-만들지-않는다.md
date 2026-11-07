# ADR-0182: transport raises connection events as structure and does not build a screen

- 상태: 확정 (2026-09-06, basis: user decision + static-read measurement in this session. ★**Not one line of code was changed**★)
- 관련: ADR-0177 decision 9 (a loss signal is carried on **a stream the upper layer reads** — this event stream is that stream's real form. ★**The measurement that sequence-gap checking is absent is also canonical in that ADR**★) · ADR-0177 decision 3 (what goes inside the crate — generation boundary · origin marker · reconnect · ★**the N-peer registry**★) · the inside/outside split in ADR-0177 「거부한 대안/근거」 (★the rejection ground of decision 3★) · ADR-0177 decision 8 (verification harness = in-memory + time injection — this decision gives that harness **what to assert**) · ADR-0180 (notification policy overall — what goes out as a strip and what is covered by a popup. ★**This ADR does not reopen it**★) · ADR-0179 (several daemons, several clients — ★meaning events happen **per peer**, and how to view that bundle is an **open question**★, see 「영향」 below) · ADR-0129 (net vocabulary isolation — the same wall stands at `transport`) · ADR-0046 (replay single-flight — where rewind truncation happens) · step-log S21

## 맥락

Building the `transport` crate (ADR-0177) splits out the question of **who knows 「무슨 일이 있었나」 (what happened)**. Today those facts **arise inside the place that will become the crate and die there** — the list in 「근거」 below is that, and none of them **comes out in a form the upper layer reads.**

And deciding this scope late is expensive. The event points sit **inside the connection lifetime, the reconnect loop and the rewind boundary**, so once the crate is fully built, opening them up means **tearing into that interior again and changing the interface one more time.**

## 결정

1. ★**`transport` raises events to the upper layer as structure**★ — **connected/disconnected · why it was refused · which retry attempt this is · how many were lost and where.** It is **structure**, not log strings.
2. ★**The screen (a log window) is out of scope this time.**★ The view builds it later. User statement: *"올리는 것까지 해야지. 화면 만드는 건 나중에 view에서 얼마든지 할 수 있으니."*
3. ★**`transport` raises only what it knows itself.**★ **Agent errors and command failures are things the upper layer knows, so they are not raised.** Later the screen **merges several sources.**

## 거부한 대안

- **Defer even the events out of this scope** — if the event points are opened late, ★**`transport` is torn into again and the interface changes one more time**★. The place where events arise is inside the connection lifetime and the reconnect loop, so they cannot be patched on from outside.
- **Build the screen this time too** — the scope grows a lot, and ★**the focus of the first step, building the crate and verifying it alone, gets blurred**★ (the working method of ADR-0177 decision 1 = build new → verify independently → attach in a suitable scope).
- **`transport` also collects and raises agent errors and command failures** — ★**a crate boundary violation**★ (the inside/outside split table of ADR-0177 — what stays outside is the packet definition and the policy values, and the crate never knows the agent vocabulary in the first place). To collect and raise them it would have to know that vocabulary, and at that moment the same wall that ADR-0129 set up for `net` collapses.

## 근거

★**Things that disappear now (measured — static reading)**★:

- **Refusal reason during reconnect** → left only as a `debug!` log and thrown away. ★**On the first connection it reaches the screen, but during a reconnect it does not**★ — the same fact is sometimes visible and sometimes not, depending on the path.
- **Rewind truncation** → **only in the browser console** (the release WebView2 has no developer tools, so in practice nobody can see it).
- **Output range loss** → ★**nobody checks it**★ — there is no sequence **gap check**, only monotonic dedup. So a missing range cannot be told apart from a 「지나간 번호」 (number that has passed). ★**The canonical source of this measurement is ADR-0177 「근거」 and the search expression is there too — we do not copy only the number here.**★
- **A file log exists but there is no way to view it inside the app** — the user has to open the file directly.
- ★**Command outcome discard** → only as a warning log. **But only this item has a different standing**★ — it is an example of 「**지금 앱 안에서 사라지는 것**」 (**what disappears inside the app now**), not ★**an event `transport` raises**★ (decision 3 takes the command family outside). The reason it is written here is that **later, when the screen merges several sources, this too is material for it**, and that merging is done by the upper layer.

★**Added value — this is also a verification tool**★: an in-memory test can **assert 「그때 무슨 일이 있었나」 (what happened at that time) as an event**, so it **measures the reconnect and loss logic without a real socket.** ADR-0177 decision 8 nailed the harness down as 「로직은 전부 인메모리 + 시간 주입」 (all logic in-memory + time injection), but ★**that ADR did not give what the harness looks at to decide**★ — this event stream is that observation point.

## 영향 / 불변식

- ★**The event vocabulary is the crate boundary**★ (decision 3). If agent or command vocabulary comes in even by one slot on the grounds that 「이것도 같이 올리면 편하다」 (raising this too would be convenient), the inside/outside split of ADR-0177 and the isolation of ADR-0129 break right there. **The screen does the merging.**
- **The 「위층이 읽는 스트림」 (stream the upper layer reads) of ADR-0177 decision 9 is this event stream** — the loss signal is carried **here, not on the console**. Truncation is given not as true/false but as ★**「지금 유효한 가장 오래된 번호」 (the oldest number valid right now)**★ (this ADR does not change the form of that decision).
- ★**Events are only raised and there is no screen, so there is no change visible to the user for now — that is the intent**★ (decision 2). **The first consumer is a test**, and the screen is attached later. ★Do not read this state as 「미완성」 (unfinished) and hurry to build a log window★ — removing that scope is this decision.
- **Events do not automatically gain a notification surface** — the canonical source of that policy is **ADR-0180** (not rewritten here).
- ★**Open question — events happen per peer, so how do we view them bundled**★. ADR-0177 decision 3 is **an N-peer registry + supervision per peer**, so events carry an **origin marker**, and with ADR-0179 those peers really do become several. Then both 「몇 번째 재시도인가」 (which retry attempt this is) and 「어디서 몇 개를 잃었나」 (how many were lost and where) are counted **separately per connection**. ★**This ADR does not decide whether to fold that into the whole app or keep it per connection — and does not make one up.**★ The user is still deciding.
- ★**Not verified**★ — the 「사라지는 것들」 (things that disappear) above are **the conclusions of static reading**. We did not reproduce each point at runtime to confirm it, and **this ADR does not fix the final list of event kinds either** (decision 1 names four branches, it is not a schema). The schema is decided in the phase where the crate is built.
