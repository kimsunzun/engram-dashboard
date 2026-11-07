# LLD Request Guide — fable (verifier's perspective)

**Author:** fable (pane 8), 2026-06-11
**Audience:** a format guide for requesting a detailed coding plan (LLD) from the engram-dashboard design agent (pane 12)
**Basis:** written after reviewing `backend-architecture-final.md` directly

---

## (2) Format recommendation — two stages (structure verification → code verification), with the code stage split by module

**Recommendation: two stages, with stage 2 split by module internally.** Confidence: certain.

| Approach | Problem |
|---|---|
| One-shot full code | Reviewing thousands of lines at once drops adversarial quality sharply (diluted attention). Structural flaws get hidden behind code detail, and a structural flaw found after the code is written makes rework cost the highest. |
| Stage-by-stage split only (no structure freeze) | Each piece arrives without global context and cannot catch cross-module flaws. This project's real risks (lock order, drain thread lifetime, subscriber cleanup) sit exactly on module boundaries. |
| **Two stages** | Stage 1 verifies the interfaces, state machine and concurrency model and **freezes** them; stage 2 verifies each module's code for "conformance to the frozen spec + local bugs". Structural flaws are caught at the cheapest point, and since the three verifiers all look against the same frozen spec, decision consistency also improves. |

Operating order: ① produce the LLD (structure) → ② three-way verification by Gemini/GPT/fable → ③ incorporate and freeze → ④ produce code module by module (each module notes "which item of the frozen spec it implements") → ⑤ three-way verification per module.

---

## (1) What the LLD must contain (verification-friendly composition)

1. **Module map + dependency direction** — at the level of real file paths (`src-tauri/src/pty/manager.rs` etc.). One diagram.
2. **Public interfaces as real Rust signatures** — no prose descriptions. Function signatures, error types, all struct fields. "Roughly this shape" cannot be verified.
3. **Pinned dependency versions** — a `Cargo.toml` snippet (`tauri = "2.x.y"`, `portable-pty = "0.x"`). Both crates have large API differences between versions, so without a version the existence of an API cannot be verified.
4. **State machine transition table** — for each `AgentStatus` transition: the trigger, the performer of the transition (which thread), and the resources cleaned up on the transition.
5. **Concurrency spec** — lock list + **lock acquisition order rules**, thread list (who spawns and who joins/detaches), channel topology (bounded/unbounded, policy when full).
6. **Resource lifetime table** — for each of child process / drain thread / writer / Channel subscription / Job Object handle: creation point, owner, release trigger, releasing party.
7. **Three error/shutdown path walkthroughs** (as sequences): (a) abnormal child exit (b) cleaning up all PTYs on whole-app exit (c) subscriber cleanup on window close/reload.
8. **First-review feedback traceability table** — which section of the LLD reflects already-decided items such as emit_all→Channel and Job Object. Prevents the waste of verifiers repeating the same finding.
9. **Decision–rationale–rejected alternatives** — gives the adversarial verifier an attack surface.
10. **Non-goals** — state what is out of scope to avoid wasted verification.
11. **Review request questions** — the "review request items" section of final.md is good practice. Keep it in the LLD too.

---

## (3) Items fable as verifier says "I cannot review properly without these"

- **Pseudocode for the drain thread shutdown mechanism.** The `portable-pty` reader does a blocking read, so clean shutdown is the biggest challenge of this design. Unless "when `kill_agent` is called, how does the drain thread blocked in a read wake up and exit" is stated, I cannot pass it.
- **Written-down lock order rules + whether a lock is held at send time.** Does the drain thread hold a session lock while iterating `subscribers` and calling `channel.send`? If it does, a delayed send also blocks `write_stdin`. The current final.md leaves this part empty.
- **Subscriber removal mechanism.** final.md says "channel drop → automatic removal", but in practice removal must be done by detecting send failure. The detect-and-remove code flow must be stated.
- **Integrity of the replay → live switch.** Pseudocode for how `seq` guarantees no gap/duplicate between the replay send and the live stream on a late attach.
- **A Windows-specific section.** The code points for Job Object creation and assignment, and ConPTY particulars. The target is win32, so split it into its own section.
- **A 1–2 line "verification points of this module" at the end of each module** — steers the verifier's attention to the risk points.

---

## Draft request message to send to the design agent

> Write the LLD (detailed coding plan) in two stages based on backend-architecture-final.md.
> **Stage 1 (now):** a structure-finalized version without code — module map (real file paths), public interfaces as real Rust signatures, Cargo.toml version pinning, AgentStatus transition table (trigger/performing thread/resources cleaned up), lock order rules, thread and channel topology, resource lifetime table, three shutdown path walkthroughs (abnormal child exit/app exit/window close), first-review feedback traceability table, rejected alternatives, non-goals, review request questions. In particular, give ① the shutdown mechanism of a drain thread blocked in a read ② whether a lock is held at the time of channel.send ③ subscriber send-failure detection and removal ④ replay→live seq integrity at pseudocode level.
> **Stage 2 (after the three-way verification and finalization of stage 1):** actual code per module. Note on each module the corresponding frozen-spec items and the verification points.
