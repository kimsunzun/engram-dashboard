# ADR-0097: Adopt auto mode (bypassPermissions) as the default for spawned agents — making headless worker permissions realistic

- 상태: 확정 (2026-07-22, basis: user decision + CLI send 0/38 measurement) · 부분 폐기 by ADR-0188 (the part that fixed the permission value in backend code)
- 관련: Amends ADR-0094 (pre-authorize sending only · bypassPermissions rejected → adopt auto mode (bypassPermissions) as the spawn default (user decision 2026-07-22). The grant seam stays as a policy surface for a future shared constraint layer) · ADR-0004(backend knowledge isolation) · Amended by ADR-0188 (the part that fixed the permission value in backend code)

## 맥락
ADR-0094 chose "pre-authorize only the send entrances, and reject allow-everything (bypassPermissions)" — the basis being that the send grant alone is enough for the round-trip (message exchange) MVP and it minimizes the injection surface. But two things came to light:

1. **For headless workers, default-deny is a structural wall.** A stream-json (`-p`) spawn has no approver (human). Every tool call that is not pre-approved (Read/Edit/Bash, etc.) falls through to a denial. For an agent to do real work beyond "exchanging messages", the grant plumbing would have to grow per capability, which is effectively a course of reinventing the permission system per tool.
2. **The grant plumbing itself is fragile (measured).** The CLI send (engram-send) grant's absolute-path `Bash(<abs> *)` pattern did not match the string of the agent's real call (`$ENGRAM_SEND_EXE ...`) → the claude permission gate **blocked all 38/38** (roundtrip measurement 2026-07-22). It was fixed by ADR-0098 (bare-name + PATH alignment) and recovered from 0/38 to 10/10, but this showed that "if even one grant string is off, everything silently dies".

The industry direction is also "relax approval + safety is the boundary's (containment) job" (Codex `--sandbox`, Claude Code sandbox, container isolation). Auditing one boundary is simpler and more deterministic than managing N grants.

## 결정
**Launch every spawned claude agent with `--permission-mode bypassPermissions` (auto mode).** Common to both the Terminal PTY and StreamJson modes, unconditionally, regardless of whether there is a control endpoint. (`backend/claude.rs::build_spec` — the base flag at the very front of args.)

- **The send entrance grants (ADR-0094/0096: MCP send_message + Bash/PowerShell engram-send) are kept.** Under bypass they are not a runtime gate, but they are ① a **policy surface** that a future shared constraint layer will revive and ② **documentation** of which channels are open. They are not removed.
- **A transitional regime.** The replacement = a constraint layer shared by all LLMs (claude · codex · gemini) (a shared settings canon → materialized into per-LLM settings files, injected as a combination of sets + inheritance). The step-log backlog "constraint layer shared by all LLMs" + this ADR is the premise when that work starts.

## 거부한 대안
- **Keep default-deny + extend grants per capability (extending ADR-0094)** — with no headless approver every capability is a wall, and a grant string mismatch causes silent annihilation (0/38 measured). Reinventing per-tool permissions = rot.
- **`--dangerously-skip-permissions`** — the effect is practically the same as bypassPermissions, but the name carries a "dangerous" warning and it is entangled with a settings dependency (`allowDangerouslySkipPermissions`). `--permission-mode bypassPermissions` is the formal mode value claude exposes, so use that.
- **`auto` (model-classifier approval) mode** — another mode claude exposes, where a classifier approves/denies probabilistically (the "permission classifier block" in this CLI 0/38 is behavior of this family). A worker being probabilistically unable to use its own tools goes against headless reliability. Choose deterministic bypass.
- **Lay down the Windows boundary first, then bypass** — the direction is right (a boundary must exist for "auto + constraints"), but with no Windows OS sandbox a WSL2/container+PTY spike has to come first. To avoid blocking in the meantime, bypass is adopted first and the boundary is applied in the shared constraint layer (user decision: auto now, boundary later).

## 근거
- **Measured:** CLI send 0/38 (default-deny, grant mismatch · permission classifier block) → 10/10 (cli) after the ADR-0096 alignment. A successful send was measured in a live roundtrip under auto mode. Local measurement (echo passed) confirmed that `bypassPermissions` is claude CLI's formal mode value and that it actually runs non-granted Bash headlessly.
- **Gate:** `/review code full` — 1 doc-aware (Claude) FIX (types.rs stale doc) applied + blind (codex) PASS. All `/qa standard` gates green.

## 영향 / 불변식
- **`--permission-mode bypassPermissions` = the first base flag in args** (a position rule, do not change): the variadic `--allowedTools` group must still be **at the very end** (preventing positional absorption, ADR-0094). The pair is non-variadic, so it goes in front. Regression guards = `claude_auto_permission_mode_precedes_allowed_tools_*` + the existing `claude_allowed_tools_group_is_last_*`.
- **Implication for the measurement seam (important):** `ENGRAM_DISALLOW_MCP_SEND`/`--disallow-mcp` removes only the MCP **grant**. Under bypass there is no permission gate and the mcp-config server connection remains, so send_message is still callable — **under auto mode this seam cannot simulate "MCP absence".** Pure CLI-only/forced-channel measurement is valid only under default-deny (the ADR-0094 regime). Under auto mode, channel choice = a priming decision (no enforcement). (The 0/38→10/10 Phase-3 numbers were measured in the default-deny world = valid as grant plumbing verification.)
- **Injection surface raised:** in a structure where teammate messages are driven in as input, a bypass agent will do anything inside the boundary if persuaded. Envelope verification (blocking sender forgery) and the priming "keep your own judgment" are only probabilistic defenses — **the safety ceiling = the quality of the future boundary (containment)**. Until then, operate it for trusted environments only.
