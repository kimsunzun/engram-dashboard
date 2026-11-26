# A request that can legitimately take long — one shared timeout, a per-request deadline, or not waiting at all

- Status: **decided 2026-11-14 — option B with a derivation chain** (ADR-0369): launch requests get a deadline derived from the daemon's launch budget (base + margin), every other shell request keeps 30 s. User: 「B를 하면 될것같은데」 · 「아니 연속성을 하려면 기준점 + 추가시간? 그런식으로 되야되지 않냐는거지」 · 「어쨋든 ㄱㄱ」. · feeds S25 step 8 TRD (`docs/process/S25-preset-library/trd-daemon-wiring.md` §8 (c) C2)
- Method: `/engram-research` medium (design-decision mode) — two collectors (A: fixed timeouts / deadlines / retry · B: long-running-operation, progress, local IPC), main grounding (✔raw = main re-read the source text), codex review (section 5).
- Question (user, 2026-11-14): the plan is a per-request deadline for agent launches (88 s) while every other shell request keeps 30 s — 「이렇게 하드코딩하는게 맞는 방향인가? 관행 조사해봐.」
- Confidence: 확실 · 가능성 높음 · 불확실 (see `/engram-research` legend).

## 1. Constraints (this repo)
- One WS link shell ↔ daemon with **one shared request timeout** (30 s today, `src-tauri/src/daemon_client/connection.rs` `REQUEST_TIMEOUT`).
- **ADR-0181** (user decision 2026-09-06): one shared value regardless of command — rejected alternative 「a different deadline per command kind」 because 30 hand-picked values would be guesses (user: *"When do we set seconds for each of them, just make it shared."*); no automatic resend after expiry (no reply = 「모른다」, a resend can create a second agent); its 근거 also says *if a measured spawn overflows the cap, raise the cap*.
- **ADR-0360**: the daemon's launch has its own budgets (folder waits 60 + 2 s, reply bound 85 s); an outer wait must be ≥ the inner wait + a hop margin. On the daemon's bus it already gave **launch commands their own longer deadline** and rejected 「Raise the global 7 s default for every command」 ("other commands gain nothing, and a hung command would hold its caller for 88 s"); it also presented 「Answer the launch at once and report folder results later」 — "Presented; not chosen." Both were shown to the user.
- **ADR-0155**: a command's properties are declared beside its producer; the daemon does not statically know shell names.
- The daemon already pushes agent status events to the shell; launch notices and block reasons return in the launch answer (steps 16 / 20), which the CLI and LLM callers read synchronously.

## 2. Findings

### 2-1. Frameworks set the deadline per call, often per method in config — 확실
- **gRPC** ✔raw (Microsoft Learn, ASP.NET Core gRPC): "There is no default deadline value. gRPC calls aren't time limited unless a deadline is specified." grpc.io (collector-relayed): "you should always explicitly set a realistic deadline in your clients".
- **gRPC service config** (collector-relayed): `methodConfig` carries a per-method `timeout`; **Envoy** routes carry a per-route `timeout` (default 15 s, `0` disables). So the per-method deadline is commonly *declared* (config), not a client-wide constant.
- **Choosing the value** (AWS Builders' Library, collector-relayed): pick an acceptable false-timeout rate and use the matching latency percentile *of that operation* — i.e. derive each operation's timeout from that operation, not one number for all.

### 2-2. A client giving up does not stop the server, and the outcome is unknown — 확실
- ✔raw (Microsoft Learn): "Although the HTTP request is aborted, the gRPC call continues to run on the server until the method completes."
- **No automatic retry** of a timed-out side-effecting call: Google AIP-194 (DEADLINE_EXCEEDED not auto-retried — collector-relayed); AWS: "APIs with side effects aren't safe to retry unless they provide idempotency" (collector-relayed). Idempotency keys (Stripe `Idempotency-Key`, Google AIP-155 `request_id`, Azure repeatability headers) make a retry return the earlier result. — matches ADR-0181 decision 3.

### 2-3. Long operations are often made asynchronous — 확실 (for the cited APIs)
- **Google AIP-151** ✔raw: "Individual API methods that might take a significant amount of time to complete should return a google.longrunning.Operation object instead of the ultimate response message", with ~10 seconds as the rule of thumb; failure is `Operation.error`, progress in `metadata`.
- **Azure** (Architecture Center async request-reply, collector read in full): respond at once with 202 + a status location; status `Pending/Running/Succeeded/Failed/Canceled` with `lastUpdatedAt` ("Helps clients distinguish between stalled and in-progress operations"); an idempotency key so a duplicate returns the existing status "because the client can't distinguish between a lost response and a request that was never received".
- A launch of up to ~85 s is well past AIP-151's rule of thumb (which is qualified, not a hard limit).
- **A local daemon precedent** (codex-found): systemd's D-Bus `StartUnit` returns a job handle at once and reports completion with a `JobRemoved` signal; its documentation tells clients to subscribe before sending the request to avoid missing the completion — an asynchronous contract needs that ordering rule.

### 2-4. Local IPC: practice varies — 가능성 높음
- **D-Bus (sd-bus)** (codex-found, not re-read by main): a default method-call timeout of 25 s, overridable per call and configurable through the `SYSTEMD_BUS_TIMEOUT` environment variable — one default plus per-call override.
- LSP 3.17 (raw-grepped main spec) and JSON-RPC 2.0 define no request timeout; LSP requires every request to get a response and cancellation is client-driven (`$/cancelRequest`); progress goes on a separate channel (`$/progress`, DAP `progressStart/Update/End`).
- Tauri `invoke` and Electron `ipcRenderer.invoke` document no timeout on the pages opened (an absence on those pages, not a survey of the ecosystem).
### 2-5. Progress can extend a timeout, bounded by a maximum — 확실 (MCP) · 가능성 높음 (systemd)
- **MCP** ✔raw (specification 2025-06-18, lifecycle 「Timeouts」): "Implementations MAY choose to reset the timeout clock when receiving a progress notification…" and should still "enforce a maximum timeout, regardless of progress notifications, to limit the impact of a misbehaving client or server".
- **systemd** (codex-found): a service can extend its start-up timeout by repeated notifications (`sd_notify` `EXTEND_TIMEOUT_USEC`).
- Ordinary connection heartbeats are not request progress — only progress tied to that request may extend it.

## 3. Options for the shell's launch requests

| | What | Precedent | Fit to constraints | Cost |
|---|---|---|---|---|
| **A. One shared timeout raised to 88 s** | every shell request waits up to 88 s | one client default (Envoy 15 s, sd-bus 25 s); ADR-0181's 근거 「raise the cap」 | Keeps ADR-0181 d1 as the user decided it. Every silent non-launch request errors after 88 s instead of 30 s. The same move was rejected on the daemon's bus in ADR-0360 ("a hung command would hold its caller for 88 s"). K7 stays. | one constant |
| **B. Normal 30 s + a launch deadline derived from the daemon's budget**, the launch class declared with the verb (TRD B5) | per-request deadline for launches only: the three shell launch requests at the daemon's reply bound + hop margin (88 s); on the socket, a launch-class envelope gets its seat + margin (91 s seat → 94 s caller), which is what closes K7 | per-call deadline / per-method config (gRPC, Envoy — 2-1); default + per-call override (sd-bus — 2-4) | **An amendment of ADR-0181 decision 1** (one value regardless of command), not an exemption from it — but the same principle the user chose for the daemon's bus in ADR-0360 (launch commands only). It keeps the user's objection to ADR-0181's rejected alternative (no hand-picked values per command): two classes, and the launch value is computed from constants. | transport `request_in` deadline parameter, ADR-0181 / ADR-0310 amendments |
| **C. Asynchronous launch** — answer at once ("accepted", agent id), outcome (started / blocked + reasons / notices) as a pushed event | no long wait | Google AIP-151, Azure async request-reply (2-3); systemd `StartUnit` + `JobRemoved` (local daemon) | Strongest fit to the conventions for an ~85 s operation, but ADR-0360 already presented it and the user did not choose it; it reopens the launch-answer contract of steps 16 / 20 (CLI and LLM callers read block reasons synchronously) and needs subscribe-before-request ordering. | a topic of its own |
| **D. Progress-extended timeout with a maximum** — the daemon sends launch progress (e.g. "checking folders"); each progress tied to that request restarts the shell's timer, up to an absolute maximum | measures silence, not total time | MCP lifecycle 「Timeouts」, systemd `EXTEND_TIMEOUT_USEC` (2-5) | Useful when the server's total time is unbounded or unknown. Here the daemon already guarantees an answer within its reply bound (85 s), so the maximum D needs is the same 88 s as B; D adds a progress message type on the wire and a timer-reset rule for little gain now. | new wire message + timer logic |

On "hard-coding": the precedents keep deadlines as code/config values chosen per operation or as one default with per-call overrides (2-1, 2-4); sd-bus is one case of a default exposed as an environment variable; none found exposes them as an end-user setting in a desktop UI. B's launch value is computed from the daemon's own constants (`LAUNCH_REPLY_BOUND_SECS` + hop margin), so it moves when the budget moves. (main's inference)

## 4. Recommendation (main's inference — one confidence level lower)
**B now.** The sources show that per-call deadlines exist and are common in RPC frameworks; they do not prove "exactly two classes" is best. B is a repository-specific compromise: it extends to the shell link the launch-only rule the user already chose for the daemon's bus (ADR-0360), keeps every other request at today's 30 s, and avoids per-command guessing (the user's ADR-0181 objection). It amends ADR-0181 decision 1, so it is the user's call.
**Not now:** C (already presented in ADR-0360 and not chosen; reopens the step 16/20 contract) — keep it as the direction if launches get slower or the screen needs launch progress; D (needs a progress wire for no gain while the daemon's bound is fixed). Under every option keep ADR-0181 decision 3 (no automatic resend). The residual "shell gave up, launch still finished" (TRD K9) is not closed by any option here; a client request id (AIP-155-style) would need key propagation, in-flight de-duplication, retention and a way to recover the outcome — a separate design, distinct from the TRD's same-envelope-id seat retention.

## 5. Review
codex (gpt-6.1-sol, effort high), verdict **FIX**, seven findings — all applied:
1. **Refuted** — "B is not ADR-0181's rejected alternative" → reworded: B amends ADR-0181 d1; added that ADR-0360 already chose launch-only on the daemon bus.
2. **Refuted** — B's 88 s alone does not close K7 → the socket launch-class wait (91 s seat → 94 s caller) is stated as what closes it.
3. **Refuted** (omission) — MCP progress-reset with a maximum, systemd timeout extension → new 2-5 and option D (main raw-read the MCP text).
4. "Local IPC mostly has no timeout" unsupported; sd-bus 25 s default + per-call override + env → 2-4 reworded.
5. Findings show mechanisms exist, not that two classes are best → section 4 wording.
6. C: systemd `StartUnit`/`JobRemoved` local precedent and subscribe-before-request; ADR-0360 already presented C → added.
7. An idempotency key alone does not close K9 → section 4 states what it would need.
codex independently verified gRPC deadline/cancellation, Envoy's 15 s default, AIP-151's qualified 10 s guideline and AIP-194's non-retryable `DEADLINE_EXCEEDED`.

## 6. Gaps
- Not opened: Go `context` docs, per-operation default timeouts in AWS / Google client libraries (gapic configs), Envoy per-try timeout, D-Bus and MCP request-timeout conventions, VS Code extension-host timeouts, Kubernetes name-conflict text (paraphrase only).
- No measurement of how long real launches take on this machine (ADR-0181's own unverified note still stands).
