# ADR-0128: Hard single channel for mail — split only by capability, and the physical wiring is also equal to the teaching

- 상태: 확정 (2026-08-03, basis: user decision + 2 measurements of the mail channel — see the basis below) · 부분 폐기 by ADR-0132 (the constraint forbidding mail verbs in the control CLI)
- 관련: Amends ADR-0126 (decision 3, keep the wiring) · Amends ADR-0099 (decision 2, physical injection of engram-send) · ADR-0086 (dual entry — the source of the CLI credential env injection) · ADR-0094 (bare-name grant · PATH injection · least privilege) · ADR-0097 (auto mode — grant NO-OP) · `crates/engram-dashboard-core/src/agent/backend/claude.rs` (168~211: env · PATH injection) · `crates/engram-dashboard-daemon/src/control/mod.rs` (grants · provision) · step-log S18.13 · Amended by ADR-0132 (the constraint forbidding mail verbs in the control CLI)

## 맥락

ADR-0126 decision 1 removed the CLI workaround teaching from the priming, but decision 3 **left the physical wiring as it was** — even for MCP-capable spawns `engram-send` is injected through the PATH and environment variables. So an asymmetry of "not taught, but put in the hand" remained, and to manage that asymmetry ADR-0126 had to add two more devices: decision 4's **"teaching ⊆ wiring" one-directional invariant**, and decision 2's escalation teaching (a single priming sentence), the **only** device preventing misuse of the remaining wiring.

ADR-0126 itself left **2 unobserved items**. This session measured both:

- **Send reliability of MCP-only teaching** — ADR-0099's no-regression 10/10 was under the *both-teaching* condition, so there was no basis for the reliability after reducing the teaching to MCP alone.
- **Behavior when MCP is broken** — does it follow the escalation instruction · find and use `engram-send` in the shell · silently give up. Since decision 3 left the wiring, the split among these three hung on a single priming sentence.

The first came out as no regression, and in the second **the manipulation itself failed** so the question could not be raised (basis section). That failure rather revealed a decisive fact — a channel cannot be blocked by permission, and only the physical wiring has effect.

## 결정

1. **The mail send channel is split only by backend capability.** An MCP-capable spawn gets MCP (`send_message`) only, a backend without MCP support gets CLI (`engram-send`) only. **No runtime switching or fallback.**
2. **Remove the physical injection of `engram-send` from MCP-capable spawns** — the PATH prepend + the `ENGRAM_SEND_EXE` · `ENGRAM_TOKEN` · `ENGRAM_CONTROL_URL` environment variables. As a result, **teaching set = physical wiring set (equality)**.
3. Therefore ADR-0126 decision 4's "teaching ⊆ wiring" **one-directional invariant shrinks to an equality**, and a separate device to keep the inequality becomes unnecessary.

## 거부한 대안

- **Keep the wiring (ADR-0126 decision 3 · ADR-0099 decision 2)** — the only stated motive then was *"to leave room to connect with the command family later"* (user decision). That control-family channel was **split off as a separate issue** this time (see Impact below), and it will be a **separate control CLI** that has no mail verbs, so there is no reason to keep this wiring *on the mail axis*. That is, the reason for rejection is not "the motive was wrong" but "that motive does not belong to this axis".
- **Block only by strengthening the priming wording** — measurement (b) showed the absence of enforcement by wording or permission. Furthermore, it is better to dissolve the structure of guarding the remaining wiring with a single sentence than to write that sentence more strongly.
- **Split the mail channel by kind of operation (mixing by frequency)** — one agent would have two physical surfaces and this decision would become meaningless. It collides head-on with measurement (b) that channel control hangs only on the physical wiring.

## 근거

- **User decision (2026-08-03):** mail has a large volume going back and forth, so choose **MCP, which is fast even if it uses a few more tokens**. CLI for backends without MCP support.
- **Measurement (a) — MCP-only teaching send reliability 10/10.** `roundtrip-smoke` (sonnet) 10 runs, in every trial `B_SENT=true` + `ENTRANCE=mcp`, shell workaround 0 · no-send 0 · infrastructure defect 0. **Tied** with ADR-0099's both-teaching 10/10 — reducing the teaching to MCP alone did not cut reliability. (Interpretation caution: the `entrance=cli` rows in the delivery record are seed messages planted by the harness, not agent sends.)
- **Measurement (b) — manipulation failed 6/6, which is itself the basis.** In 6 runs with `--disallow-mcp` (removing **only** the MCP send permission, with the server and config left attached), the agent sent normally every time (`ENTRANCE=mcp`, `delivered=true`). From the agent's point of view the channel was not broken, and the intended behavior question could not be raised. **A channel cannot be blocked by permission** — consistent with ADR-0097's "in auto mode the grant is a NO-OP". These 6 runs are **invalid** as behavior observation, and valid only as a refutation of the means of channel control.

## 영향 / 불변식

- **Code points** — remove `claude.rs:168~211` (ADR-0086 CLI credential env injection + ADR-0094 PATH prepend) from the MCP-capable branch. grants `[Mcp, Cli]` → `[Mcp]`. **Invariant:** if **the daemon provisions** the `engram-send` path (PATH · env) to an MCP-capable spawn, this decision is violated.
  - **★Invariant scope correction (2026-08-04, user decision — the first-edition wording was too broad)★:** the target is **the act of provisioning, not the whole environment**. The first edition wrote *"it is a violation if the path exists **in the environment** of an MCP-capable spawn"*, but then ① even what the user put directly in the profile env becomes a violation and ② if the control plane goes by CLI (the control CLI needs the same credentials · PATH) it becomes impossible to hold at all. **User decision:** the only means of control is **"do not teach it"** (the two priming files do not even mention the other channel by name — sealed by a unit test), and the case where an agent, with extremely low probability, finds and calls the binary by chance is **accepted**. Therefore the MCP branch neither strips the credentials · PATH that the profile put in nor warns about them (it does not set a precedent of the system overriding user settings · a PATH string decision is fragile to case, 8.3 short names, symbolic links and relative paths, and a false positive spreads into breaking the user's PATH).
  - **Implication — the only effective control of this decision is the priming.** Removing the wiring reduces the surface for accidental discovery but is not a guarantee. So the unit test that sealed the mutual exclusivity of the two priming files (zero mentions of the other channel in each file) is **this ADR's real gate** — weakening that test neutralizes this decision.
- **CLI backends unchanged** — the `engram-send` binary and its path are still needed by backends without MCP support. This decision does not remove the binary but **does not lay it on MCP spawns**.
- **Harness impact** — `--disallow-mcp` was already shown meaningless by measurement (b) (removing the permission does not become a manipulation), and after the wiring removal the "two channels coexisting" state that mode presupposed itself disappears. `--cli-only` exercises the CLI backend path, so it stays.
- **Open — the channel for control-plane (spawn etc.) requests.** This ADR does not decide. The user's provisional direction is *"CLI if at all possible"* (the frequency is low and no separate connection is needed, so it is not cumbersome), but **it may change after measurement**, so it was split off as a separate issue. The prerequisite decisions that open with it = **ADR-0086 least-privilege** (a worker has no permission to spawn or kill siblings) and "to whom to give it". **Constraint:** even if a control CLI is adopted, if it **has mail verbs this decision collapses** (the physical surface grows to two).
- **Unverified — behavior under a physical MCP failure.** Whether the agent **reports or silently gives up** when the channel is really cut at the server or wiring level, not the permission level, is still unobserved. After the wiring removal the **shell workaround branch disappears**, so only two branches remain and, with no backstop, the importance of this axis rises. A valid measurement must be designed with **physical separation**, not grant removal.
