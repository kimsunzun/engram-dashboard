# ADR-0099: Channel choice = a backend capability switch + two static priming files (MCP-capable = both-teaching / non-MCP = CLI-only)

- 상태: 확정 (2026-07-22, basis: user decision + measurements of channel latency and priming's deciding power — see basis below) · 부분 폐기 by ADR-0126 (dropping the engram-send fallback teaching and making the channel-consistency invariant one-directional) · 부분 폐기 by ADR-0128 (decision 2, physically injecting engram-send) · 부분 폐기 by ADR-0211 (non-MCP priming and the two static files) · 부분 폐기 by ADR-0215 (codex carries the content instead of a path)
- 관련: ADR-0030/0002(capability matrix) · ADR-0004(backend knowledge isolation) · ADR-0086(dual entrance · single wrap) · ADR-0097(auto mode — channel choice = priming) · ADR-0098(bare-name grant) · ADR-0093(experiment case matrix — foreshadowed "clean up when the wording is fixed") · `crates/engram-dashboard-daemon/src/control/mod.rs::build_grants` · `crates/engram-dashboard-daemon/src/control/priming.rs` · `crates/engram-dashboard-core/src/agent/backend/claude.rs` · step-log S17 · Amended by ADR-0126 (dropping the engram-send fallback teaching and making the channel-consistency invariant one-directional) · Amended by ADR-0128 (decision 2, physically injecting engram-send) · Amended by ADR-0211 (non-MCP priming and the two static files) · Amended by ADR-0215 (codex carries the content instead of a path)

## 맥락
The channel policy was settled ("MCP if available, otherwise CLI fallback" — the user, 2026-07-22), but **how to enforce it** was undecided. Measurement of the current plumbing:
- Under auto mode (ADR-0097, bypassPermissions) the grant is a NO-OP gate — the only real enforcing power is **physical provisioning** (a channel that is not laid down cannot be used), and the choice among the ones laid down is **decided by priming** (measured: even when MCP is visible, CLI priming leads to CLI use).
- MCP attachment (mcp-config) was unconditional in prod, and priming was a single fixed file (`FilePrimingProvider` → `prompts/agent-priming.md`) with **no plumbing to select a variant**. The experiment variants (v3-en-cli/-both) existed only as files, not wired.
- For non-MCP backends (codex/gemini etc. in the future) the MCP entrance cannot even exist, yet the single priming teaches send_message — a mismatch between instruction and tool is measured to cause a send freeze.

## 결정
**One backend capability drives the physical wiring, priming and grants all together:**

1. **The capability declaration = backend knowledge** (ADR-0004): "is this a backend that can receive an MCP config" is declared by `backend/` (claude=true), and consumed by `DaemonControlChannel::provision`, `build_grants` and priming selection.
2. **MCP-capable spawn**: attach mcp-config + keep the engram-send injection (PATH · env) + priming **A (both-teaching)** + grants `[Mcp, Cli]`.
3. **Non-MCP spawn**: **do not attach mcp-config (physical)** + engram-send only + priming **B (CLI-only)** + grants `[Cli]`.
4. **Priming = 2 static files** — `prompts/agent-priming.md` (A = v3-en-both promoted: send_message as the main path + engram-send fallback) · `prompts/agent-priming-cli.md` (B = v3-en-cli promoted: engram-send only, the word send_message entirely absent). No runtime assembly or substitution — at spawn, one of the two paths is passed through `--append-system-prompt-file`.
5. **Clean up the experiment variants**: delete the `prompts/experiments/` priming variants (git history preserved, carrying out the cleanup ADR-0093 foreshadowed) + remove the C1~C3 file aliases from roundtrip_smoke (`--priming <직접 경로>` is kept).

## 거부한 대안
- **Hard single channel (MCP-capable gets MCP only — no engram-send PATH · env injection)** — it gains airtight determinism but loses the in-spawn fallback. Auto mode itself is a transitional regime premised on a trusted environment (ADR-0097), so airtight has no real benefit, and both-teaching was measured not to hurt MCP compliance (10/10), so soft is enough.
- **Dynamic glossary/template substitution (a single file + swapping only the channel words)** — at a scale where the file is 11~14 lines and only one section "Replying to teammates" differs, it only adds a render→temp-file materialize step (duplicating the lifetime, cleanup and failure-mode complexity of the mcp-config write path). **Revisit trigger**: when the variant axes grow to 2 or more (language × channel × backend) or a third variant becomes necessary.
- **Remove the switch and universalize the CLI (discard the MCP entrance)** — the two entrances converge in one place in the daemon, so delivery is identical, but it gives up the standing-connection advantage of MCP (zero process spawns per message) and goes against the settled policy ("MCP if available").

## 근거
All measured (2026-07-22, the step-log S17 "channel switch design kickoff" entry):
- **CLI latency is not a deciding variable**: `engram-send.exe` spawn+start warm ~33ms (direct PowerShell) / ~110–130ms (inside bash), with the whole chain including the Bash tool's shell spawn estimated at ~200–500ms. Tool execution is serial with (adds to) inference, but is a few percent against the several seconds of inference per send.
- **Priming's deciding power over the channel (auto mode)**: auto-cli 8/8 cli · auto-mcp 3/3 mcp · auto-both 3/3 mcp — even with MCP exposed, it followed the channel the priming pointed to.
- **both-teaching has no regression**: both+MCP 10/10 (default-deny regime).
- **Instruction-tool mismatch freeze**: MCP exposed + CLI-only instruction = ~6/7 unsent → a channel that is not laid down must be deleted from the prompt entirely.
- **Converged plumbing**: both entrances go through the daemon's single `/control/send` handler · wrap once (ADR-0086 reconfirmed) — channel choice is only a matter of availability and unrelated to delivery correctness, so the failure cost of a soft switch is low.

## 영향 / 불변식
- **Priming-wiring consistency invariant**: the set of channels physically laid down in a spawn = the set of channels the priming teaches. Breaking it brings back the send freeze (6/7 unsent measured). Move this pair together when adding a new backend or channel.
- **Grants are emitted per channel only** — a NO-OP now because of bypass, but when the future shared constraint layer (step-log backlog, the formal replacement of auto mode) is attached it is reactivated as a hard gate. Keep this alignment so that nothing but the priming files needs changing then.
- Code anchor targets: `backend/claude.rs` (capability declaration · mcp-config conditional branch) · `control/mod.rs::build_grants` (per-channel emission) · `control/priming.rs` (variant selection) — put `// ADR-0099` anchors at implementation.
