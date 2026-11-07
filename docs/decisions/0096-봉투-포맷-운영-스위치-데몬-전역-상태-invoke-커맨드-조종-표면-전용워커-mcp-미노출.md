# ADR-0096: Envelope format operational switch — daemon-global state + an invoke command (control surface only · not exposed to worker MCP)

- 상태: 확정 (2026-07-21, basis: user decision — resolves ADR-0095 decision 5 (implementation fork))
- 관련: Amends ADR-0095 (where the envelope format switch is stored and how it is exposed (decision 5)) · ADR-0086(single wrap point invariant) · ADR-0029(the daemon owns AgentManager · src-tauri is a client shell) · ADR-0094(no widening of send permission) · CLAUDE.md §5(LLM-first control — backend behavior is controllable by invoke) · `crates/engram-dashboard-daemon/src/control/ingress.rs::wrap_message`

## 맥락

ADR-0095 fixed the envelope format switching structure (colon by default, xml as the alternative, template selection inside `wrap_message`), **carried over where the switch is stored and how it is exposed as an implementation fork** (decision 5), and deferred the formal switch (settings-based) to a "follow-up implementation slice". In this implementation slice the user decides that carried-over item.

Premises: the envelope is not a machine-parsing target but a **surface for the receiving model to read only** (delivery addressing is already done by the tool parameters — ADR-0086), and switching the format is a template selection inside `wrap_message` that touches neither the send nor the receive code path. Haiku (a weak model) has been dropped from deployment, so colon is the normal operating value, and xml is "the alternative card to pull out later when another backend/model shows routing or acceptance anomalies with colon".

## 결정

1. **Switch state = one daemon-global value.** Not per recipient or by presence of metadata; the daemon holds one "current envelope format" as global state (default colon). `wrap_message` reads this state on every assembly.
2. **Exposure = the invoke command `set_envelope_format(format)`** — changed only through the steering control surface (a src-tauri Tauri command → forwarded to the daemon). It is in the CLAUDE.md §5 "backend behavior is LLM-controllable by invoke" category. When a resident orchestrator LLM exists, it calls the same invoke command as is.
3. **Not exposed on the worker MCP channel.** A managed agent (alice, bob, etc.) cannot change the global format over the MCP channel used for `send_message` — a worker is an envelope recipient, not a decider of team policy.
4. **Persistence = memory only.** On a daemon (client shell) restart it resets to the colon default. Surviving a restart (persistence) is backlog.
5. **A human-facing trigger (shortcut · UI button) is a later option** — this slice only opens the control path (the invoke command). For temporary verification/driving, call this invoke through `scripts/cdp.mjs eval` (cdp = a debug/verification tool, not the formal trigger).

## 거부한 대안

- **A per-recipient / by-metadata-presence switch** — priming v3 solved routing (the ADR-0095 basis), so the need for per-recipient branching is not yet demonstrated. Do not carry unproven complexity (per-profile format state management) now; simplify to one global value — extend later if the need is proven.
- **Also exposing the switch on the worker MCP channel** — it is a global setting, so if one worker switches it, the envelopes of the whole team change (a wide side effect) + it widens permission by giving a managed agent control over the team (conflicts with ADR-0094's stance of not widening send permission).
- **Persistence across restarts** — "once xml, always xml" is ideal, but the benefit is uncertain against the storage effort (portfile/settings). Since colon is the normal operating value, a reset is a safe default, so keep it memory-only and leave persistence in the backlog.
- **Promoting the `ENGRAM_WRAP_FORMAT` env to the operational switch** — violates the ADR-0095 invariant (the env is a spike-only seam). The formal switch is implemented as a separate global state + a command, and the env stays as is.

## 근거

- **§5 alignment** — the format switch is daemon (backend) behavior, so it fits exactly the category "backend behavior is already LLM-controllable by invoke". Unlike UI/layout (frontend-only · control surface incomplete), it invents no new control surface and adds just one command to the existing invoke path.
- **Preserves the single wrap point (ADR-0086)** — the global state is just an *input* that `wrap_message` reads; the assembly point is still one place. Switching does not split envelope assembly into two places.
- **Least privilege (in the spirit of ADR-0094)** — if global control is confined to the steering surface and kept away from workers, the stance of minimizing send permission applies consistently to control permission too.

## 영향 / 불변식

- **The single wrap point invariant (ADR-0086) holds** — the global format state is read only inside `wrap_message`, and assembly is that one function. The env (`ENGRAM_WRAP_FORMAT`) spike seam stays as is.
- **State owner = the daemon (ADR-0029)** — the global format state lives in the daemon that owns `AgentManager`. src-tauri is a client shell, so it does not own the state and only forwards the invoke command to the daemon.
- **A single control path** — the format switches only through the invoke command (the steering surface). Adding a worker MCP or agent self-switch path violates this invariant.
- When a new backend/model is deployed, measure colon routing and acceptance with a smoke, and on anomalies switch to xml with this command (the ADR-0095 switching structure lowers that cost to a single command).
