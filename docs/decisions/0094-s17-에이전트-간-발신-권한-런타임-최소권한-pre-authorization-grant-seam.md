# ADR-0094: S17 agent-to-agent sending permission — runtime least-privilege pre-authorization (grant seam)

- 상태: 확정 (2026-07-20, basis: user decision + /research medium (cross-family review) + C0~C3 measurement)
- 관련: ADR-0093(round-trip experiment · C0~C3) · ADR-0092(receive contract · priming) · ADR-0086(dual entrance · token-derived from) · ADR-0004(backend knowledge isolation) · `crates/engram-dashboard-core/src/agent/backend/claude.rs`(translation) · `crates/engram-dashboard-core/src/agent/types.rs`(ControlEndpoint) · daemon control channel (owns the grant) · step-log S17 · Amended by ADR-0097 (pre-authorize sending only · bypassPermissions rejected → adopt auto mode (bypassPermissions) as the spawn default (user decision 2026-07-22). The grant seam stays as a policy surface for a future shared constraint layer) · Amended by ADR-0098 (align the CLI send grant translation from the absolute-path Bash({exe} *) to bare-name Bash/PowerShell({exe}:*) + PATH injection (resolves the claude permission matcher's 0/38 non-match · deployment portability))

## 맥락
In the round-trip experiment (ADR-0093, C0~C3), **B_SENT=false in every case**. Diagnosis (capturing B's turn after the seed) pinned down the cause: the claude runtime's tool-permission gate blocks the spawned agent's send-tool call (`send_message` MCP / `engram-send` CLI) (headless, no approver). B receives the message normally and has both the intent to reply and the knowledge of how to send, but stops at "human approval required". The `backend/claude.rs` spawn args had no permission flag at all. `/research` (medium, details corrected by the cross-family codex review VERDICT FIX) confirmed: **Claude Code permissions are enforced at runtime — an agent cannot self-grant through a prompt or CLAUDE.md** (official docs). Opening up sending requires runtime pre-authorization.

## 결정
1. **Sending permission = runtime pre-authorization** (not a prompt). At spawn, the send entrance tools are listed explicitly in the allowlist. Prompt pre-authorization is only a supplement (so that B attempts to send), not enforcement.
2. **Least privilege** — pre-authorize only the send entrances (`send_message` MCP + `engram-send` CLI). All other tools keep the default gate. Not allow-everything (`--dangerously-skip-permissions`/bypassPermissions).
3. **No policy file** — sending is a constant of the messaging system (always on), so there is no variable to change at release. It is baked into the code (externalizing is YAGNI — add a seam when the capability becomes variable).
4. **The grant has a single source: the control channel** — the control channel fills `ControlEndpoint.grants: Vec<ToolGrant>` (abstract: `Mcp{server,tool}` / `Cli{exe}`) next to its own entrance definition. The backend **only translates the format**: claude → `--allowedTools mcp__{server}__{tool}` + `Bash({exe} *)`. codex/gemini use the same grants in their own dialects (currently stub · TODO). Tool names and flag syntax live only in code (claude knowledge = claude.rs, ADR-0004).

## 거부한 대안
- **Prompt-only pre-authorization** — permissions are enforced at runtime, so the model cannot self-grant. Even when priming taught the sending method in C1~C3, B was stopped by the gate with "permission needed" (measured). A prompt is an input to decisions, not enforcement.
- **Allow everything (bypassPermissions / --dangerously-skip-permissions)** — defenseless against prompt injection (the research warning — for isolated environments only). Sending alone is enough for the round trip, so it is excessive.
- **An external policy file** — sending is a constant (always allowed), so there is no variable to externalize. Unlike priming (variable prompt text), there is no demand for editing at release, so YAGNI.
- **Hardcoding tool names in claude.rs** — physically separate from the entrance definition (the control channel), so a rename would need touching two places and rot. The grant seam makes it a single source (names from the control channel, only the format in the backend).

## 근거
- `/research` medium (3 main-family branches + grounding + cross-family codex adversarial review): Claude Code permissions are enforced at runtime (official docs: "permissions are not changed by a prompt/CLAUDE.md") · the MCP tool naming `mcp__<server>__<tool>` · the `Bash(exe *)` syntax (not all of Bash) · `--allowedTools` confirmed to pre-approve that call. What the review corrected = an over-broad framework classification · one miscited statistic (the directional conclusion unchanged).
- **Our measurement (ADR-0093 C0~C3)**: B was stopped by the runtime gate regardless of priming or channel — independently cross-confirming the research conclusion that runtime pre-auth is the only fix.

## 영향 / 불변식
- Add `grants` to `ControlEndpoint` (core type) · the daemon control channel fills it next to its own entrance definition · each backend translates into its own CLI syntax. **Only claude is implemented now, codex/gemini TODO** (stub — after a CLI spike).
- **Least-privilege invariant** — open only the send entrances and keep the gate on the other tools. Adding a tool here requires an explicit decision (an amendment of this ADR). Do not widen to allow-everything.
- **Single-source invariant** — the canonical source of the send tool names (`send_message` · `engram` · `engram-send`) is the control channel's entrance definition. claude.rs knows only the format rules (`mcp__{s}__{t}`/`Bash({e} *)`) — it does not retype the names.
- Verification = rerun the round trip (measure B_SENT=true on the send-priming cases after pre-auth). Whether the prompt pre-auth supplement is additionally needed is also decided by measurement then.
