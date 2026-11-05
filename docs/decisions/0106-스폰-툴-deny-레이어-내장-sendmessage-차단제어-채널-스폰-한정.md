# ADR-0106: Spawn tool deny layer — block the built-in SendMessage (control-channel spawns only)

- 상태: 확정 (2026-07-26, basis: live roundtrip diagnosis + a review finding on scope — see the rationale section below)
- 관련: ADR-0094(sender-entry allowlist pre-authorization — this ADR **adds a deny axis**) · ADR-0097(spawn argv position rule — the deny pair is added to the rule) · ADR-0099(channel switch) · ADR-0092(priming)

## 맥락

When a spawned claude sends a message, it has been reproduced live picking the wrong tool: the **Claude Code built-in `SendMessage` tool** (the harness's own agent-to-agent messaging, PascalCase) instead of our MCP tool `send_message` (lowercase, `engram` server) (2026-07-26 roundtrip diagnosis — the built-in tool does not know engram agents and returns the error "No agent named 'X' is reachable"; without a fallback description, the send ends as a failure). The misfire is non-deterministic and comes from the similar names, so priming text alone cannot stop it.

## 결정

1. **Add a deny layer:** inject `--disallowedTools SendMessage` into the claude spawn argv to block the built-in tool. We confirmed that deny rules are enforced even in bypassPermissions mode (official docs — it skips prompts, it does not ignore deny).
2. **Scope = control-channel (messaging) spawns only.** Inject it only for spawns that have a ControlEndpoint (= agents that receive messaging priming). Ordinary (non-messaging) spawns are unchanged — they keep the built-in feature and avoid the CLI startup warning cost of denying an unregistered tool.
3. **argv position rule (extends ADR-0097):** put the deny pair after `extra_args` and before the `--allowedTools` group (always last). A variadic value list ends at the next flag (`--allowedTools`) or at the end of argv — a golden test pins that it does not swallow the following arguments on any build path.

## 거부한 대안

- **Strengthen the priming text only (no block)** — we observed live that both agents self-corrected with a fallback description after the misfire, but this is non-deterministic (in the QA run the send failed with no fallback). A name collision must be stopped by structure.
- **Block unconditionally for every spawn** — even conversational agents unrelated to messaging lose the built-in feature, a user-visible policy change, and on CLI builds that lack the built-in tool every spawn prints a deny warning (found in review). Blocking only the scope where the collision matters (messaging spawns) is the smallest surface.
- **Leave the built-in SendMessage alone and rename the MCP tool (`engram_send` etc.)** — the three-tool cap, the existing priming/grant/measurements are all aligned to `send_message` (the three strings of ADR-0094 are aligned), and a rename would require re-measuring all of that alignment. Keeping name ownership is cheaper.

## 근거

- Live diagnosis (2026-07-26): in the roundtrip reproduction run, both alice and bob misfired to the built-in SendMessage → "No agent named … is reachable" (zero repo grep hits = an external string) → self-corrected with the engram-send fallback. In the QA run, bob failed with no fallback.
- Effect of deny: Claude Code permissions docs — deny rules are enforced in every mode (including bypassPermissions). Rule matching is exact on the canonical name, so it does not touch `mcp__engram__send_message` (verified in review).
- Live measurement of the effect (implementation QA full): items to confirm — on a blocked spawn the built-in SendMessage is refused, the MCP send_message works normally, and there is no startup warning.

## 영향 / 불변식

- **Deny pair position invariant:** `extra_args` < `--disallowedTools SendMessage` < `--allowedTools` (always last, ADR-0097). Do not reorder — it prevents the variadic swallow.
- **Scope invariant:** a spawn without a ControlEndpoint has no deny pair (pinned by a golden test). Widening the scope is a user decision.
- The priming (ADR-0092) is aligned in wording with the failure mode after the block (permission denial), and the fallback trigger is generalized to "every send failure".
- Code anchor: `// ADR-0106` at the injection point in `backend/claude.rs`.
