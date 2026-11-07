# ADR-0098: Align the CLI send grant to bare-name + PATH injection — resolving the absolute-path mismatch

- 상태: 확정 (2026-07-22, basis: CLI send 0/38→10/10 measurement)
- 관련: Amends ADR-0094 (align the CLI send grant translation from the absolute-path Bash({exe} *) to bare-name Bash/PowerShell({exe}:*) + PATH injection (resolves the claude permission matcher's 0/38 non-match · deployment portability)) · ADR-0004(backend knowledge isolation)

## 맥락
ADR-0094 translated the CLI send entrance (engram-send) grant as `Cli{exe = 절대경로}` → claude `Bash({exe} *)` (space-star, absolute path), and stated in a code comment that how "the claude Bash permission matcher normalizes absolute paths, backslashes and space-star is **unverified** (best-effort, to be verified later by a real agent round trip)".

That follow-up verification (roundtrip measurement, 2026-07-22) showed that **all 38/38 CLI sends were blocked**. The cause = a three-string mismatch:
- the grant pattern = `Bash(C:\...\engram-send.exe *)` (absolute path, space-star)
- the actual call = `$ENGRAM_SEND_EXE --to ...` (the env variable string the priming taught)
- the claude permission matcher could not match the two as prefix strings, and automatically blocked it headlessly.

The agent log confirmed it directly: "the automatic permission classifier blocked it — add an engram-send rule to the Bash permissions with /update-config". Contrast with the MCP path (mcp__engram__send_message), which matched exactly and got 10/10.

## 결정
**Align the three strings to the bare call name `engram-send`:**

1. **Grant value = bare `"engram-send"`** (`control/mod.rs::build_grants`) — do not embed the absolute path in the grant (deployment portability: identical on any machine and install path).
2. **Translation = `Bash({exe}:*)` + `PowerShell({exe}:*)`** (`claude.rs::grants_to_allowed_tools`) — colon-star is the prefix wildcard in the Claude Code permission syntax. It covers **both shells** (a response to the measured failure mode where, on Windows, the agent picks the PowerShell tool to attempt sending). Still limited to the send entrance (the ADR-0094 least-privilege invariant).
3. **PATH injection** (`claude.rs::build_spec`) — engram-send is an internal sibling binary that is not on PATH, so the daemon merges the exe parent directory it finds at runtime into the spawn env PATH at the **very front** (shadowing defense). The profile PATH is respected (the last case-equivalent entry wins + dedupe — env application is sequential last-wins). A join failure or non-UTF8 is a **loud skip** (warn, no lossy conversion — keeps today's behavior + makes the mismatch observable). The `ENGRAM_SEND_EXE` env is kept for backward compatibility.
4. **Priming = bare `engram-send --to <name> --body "..."`** (`agent-priming-routing-v3-en-cli.md`) — removes the env variable indirection, the same string as the grant.

## 거부한 대안
- **Keep the absolute-path grant + expect the matcher to normalize (the original ADR-0094 plan)** — refuted by the 0/38 measurement. Absolute-path backslash · space · .exe normalization does not line up with the claude matcher.
- **Allow blanket `Bash(*)`** — violates ADR-0094 least privilege. It opens arbitrary shells beyond sending.
- **Inject the absolute path into claude through priming** — priming is a static file, so it does not know the runtime absolute path, and the path would be embedded in the grant, the priming and the settings, breaking deployment portability.
- **Redesigning delivery, such as injecting an immutable snapshot** — excessive. bare-name + PATH achieves the alignment with the minimum change.

## 근거
- **Measured:** under the default-deny regime CLI send 0/38 (blocked) → after this alignment cli-sonnet 10/10, cli-haiku 8/10, cli-xml 9/10 (entrance=cli). The both-mcp regression stays at 10/10.
- **Gate:** `/review code full` — doc-aware (Claude) PASS + blind (codex) FIX over 2 rounds (respecting profile PATH · loud skip · no lossy · PowerShell coverage · duplicate PATH dedupe), all CLOSED. `/qa standard` green.
- **No wire impact:** `ToolGrant`/`ControlEndpoint` are in-proc (no serde, outside the protocol crate) — no PROTOCOL_VERSION bump needed.

## 영향 / 불변식
- **The single source of grant names = the control channel** (`build_grants`), only the translation syntax is in claude.rs (ADR-0004). The `engram-send` in the priming text is intended cross-artifact duplication, and **the roundtrip harness is the mechanism that enforces that string match** (content-based CLI-instruction decision).
- **PATH injection = a superset** (prepend, existing entries not removed — ADR-0086 env inheritance). The sibling dir must be the first entry (shadowing defense).
- Regression guards: `claude_control_endpoint_injects_path_*` · `claude_duplicate_*_path_*` · `grants_to_allowed_tools_*` (Bash+PowerShell 2 patterns) · `claude_allowed_tools_group_is_last_*`.
- Follow-up (ADR-0097): under auto mode (bypassPermissions) this grant is not a runtime gate but remains as a **policy surface for the future shared constraint layer**.
