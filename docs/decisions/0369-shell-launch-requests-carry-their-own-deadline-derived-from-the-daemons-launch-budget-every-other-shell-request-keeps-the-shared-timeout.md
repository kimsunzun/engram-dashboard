# ADR-0369: Shell launch requests carry their own deadline derived from the daemon's launch budget; every other shell request keeps the shared timeout

- 상태: 확정 (2026-11-14, 근거: user decision 2026-11-14 after the convention research `docs/research/long-request-timeout-conventions-2026-11-14.md` · TRD S25 step 8 `trd-daemon-wiring.md` §4 · §8 C2)
- 관련: Amends ADR-0181 (decision 1 one shared value for launch requests) · Amends ADR-0310 (decision 21 shared deadline for launch requests) · ADR-0360 (launch budgets; launch-only deadline on the daemon bus) · ADR-0155 (properties declared with the verb) · ADR-0270 (the shell does not depend on agent)

## 맥락
From S25 step 8 an agent launch may legitimately take up to the daemon's launch reply bound (ADR-0360: 85 s — folder waits 60 + 2 s, resume window 3 s, codex link backstop 15 s, slack 5 s). The shell talks to the daemon over one link whose request timeout is one shared value for every command (ADR-0181 decision 1; 30 s injected by ADR-0310 decision 21). If the shell gives up on a launch at 30 s while the daemon is still checking folders, the launch can still finish, and a retry creates a second agent. The plan in the step-8 TRD draft raised the shared value to 88 s for every request; review round 1 held it as a user decision because ADR-0181 records the user's choice of one shared value and judgment that 30 s was already large.

## 결정
1. **Launch requests get their own deadline; every other shell request keeps the shared 30 s.** Launch requests = the shell link's `Spawn` · `SpawnByCwd` · `SpawnProfile`, and a command whose declaration carries the launch deadline class (declared with the verb — ADR-0155; TRD B5). The transport's request call takes an optional per-request deadline; none = the shared policy value.
2. **Every launch wait is derived from one base and one margin — no independent literals.** Base = the daemon's launch reply bound (`LAUNCH_REPLY_BOUND_SECS`, itself the sum of the ADR-0360 budgets); margin = one hop margin (3 s). Shell launch wait = base + margin (88 s); relay seat = shell launch wait + margin (91 s); CLI launch wait = relay seat + margin (94 s). Changing the base moves every derived wait.
3. **The shell restates the base and the margin as its own named constants** (it cannot depend on agent — ADR-0270), derives its waits by the same sums, and a test pins the restated values equal to the daemon's (ADR-0360 decision 6). A base change is one constant per crate; the pin fails until the shell follows.
4. **A test pins outer ≥ inner + margin for every pair** in the chain.
5. ADR-0181 decisions 2–5 stay (expiry fails only that request, no automatic resend, a late reply is discarded, the waiting indicator is separate from giving up). ADR-0181's objection to per-command values — hand-picked guesses for every command — does not apply: there are two classes, and the launch value is computed.

## 거부한 대안
- **One shared timeout raised to 88 s for every request** (the TRD draft's plan) — every silent non-launch request (e.g. renaming a tab while the daemon is stuck) would fail only after 88 s instead of 30 s; ADR-0360 already rejected raising the global default on the daemon bus for the same reason ("a hung command would hold its caller for 88 s"). 사용자 직접 기각 (chose B, 2026-11-14).
- **Asynchronous launch** (answer at once, report the outcome by event — Google AIP-151, Azure async request-reply, systemd `StartUnit` + `JobRemoved`) — fits the conventions for an operation this long, but changes the launch-answer contract of S25 steps 16 / 20 that the CLI and LLM callers read synchronously; presented in ADR-0360 and not chosen; kept as the later direction.
- **Progress-extended timeout with a maximum** (MCP lifecycle 「Timeouts」, systemd `EXTEND_TIMEOUT_USEC`) — useful when the server's total time is unbounded; here the daemon already answers within its fixed reply bound, so the maximum would equal the same derived wait, and it would add a progress message to the wire for no gain now.
- **Each wait as its own measured or hand-set number** — the user asked for continuity: 「아니 연속성을 하려면 기준점 + 추가시간? 그런식으로 되야되지 않냐는거지」; independent numbers drift apart when the budget changes.

## 근거
User decisions 2026-11-14: 「B를 하면 될것같은데」 → after the convention research 「아니 연속성을 하려면 기준점 + 추가시간? 그런식으로 되야되지 않냐는거지 관행이 이게 아니면 그냥 하나하나 계산해서 하고」 → 「어쨋든 ㄱㄱ」. Convention research (`docs/research/long-request-timeout-conventions-2026-11-14.md`, codex review FIX applied): per-call / per-method deadlines are the norm in RPC frameworks (gRPC, Envoy) and one default with per-call overrides exists in local IPC (sd-bus); gRPC derives inner deadlines from the outer one rather than setting each independently. ADR-0360 had already chosen a launch-only longer deadline on the daemon bus.

## 영향 / 불변식
- The derivation chain is the only source of launch waits: base and margin are named constants in agent `types.rs` (re-exported by the daemon for the shell's tests); the shell holds its restatement pinned by test. A literal 88 / 91 / 94 in code is a defect.
- The ordinary shared timeout (30 s), the daemon bus's default seat (7 s) and the CLI's default wait (10 s) are separate values outside the chain.
- The shell does not classify a launch-class envelope arriving on the socket in step 8 (no sender exists — TRD B14 deferred); the first such caller must add that classification, or it waits only the shared 30 s.
- The residual "the shell gave up but the launch still finished" (TRD K9) is not closed by this decision; it needs a client request id design of its own.
