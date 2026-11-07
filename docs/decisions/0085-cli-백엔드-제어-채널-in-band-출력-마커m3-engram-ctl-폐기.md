# ADR-0085: The CLI backend control channel = an in-band output marker (M3) — engram-ctl discarded

- 상태: **폐기 (Superseded by ADR-0086)** — two independent investigations (the main /research fan-out + the user's separate Codex investigation) converged: zero cases that adopted free-text marker stdout scraping as the primary channel + in a follow-up spike compliance was 66% (quoted body broken, nonce misreported). The control channel switches to dual typed entrances (MCP+CLI) + an SQLite mailbox. ~~확정 (2026-07-15, basis: user decision (security, speed) + claude 2.1.170 spike measurement (markers 38/38) + live/replay boundary code check · Supersedes ADR-0080)~~
- 관련: Supersedes ADR-0080 (engram-ctl ingress discarded) · CLAUDE.md §5(LLM-우선 제어)·아키텍처 「백엔드 확장」(capability matrix) · ADR-0002 (capability = transport ⊕ backend)·ADR-0081 (UI opaque-relay survives)·ADR-0079 (resume seed boundary)·ADR-0003 (core isolation) · decision note `docs/process/S17-llm-control-surface/control-channel-deliberation-m3.md` · step-log S17

## 맥락
We must decide the **channel** for agent A→B messages (and future UI and observation control). ADR-0080 set the ingress of this channel as `child claude의 Bash 도구 → 컴파일 Rust engram-ctl → 데몬 WS(토큰 auth)` (that is, the child claude's Bash tool → compiled Rust engram-ctl → daemon WS (token auth)) (proposed state, not confirmed). The user raised two points:

1. **Security** — if the token and port are put in the agent's env, a prompt-injected child can use that credential to manipulate sibling agents (kill, write, UI) (a wider attack surface). With in-band, no credential is given to the agent.
2. **Speed** — operation is **always stream-json mode + high-frequency messaging**, so a process spawn + WS handshake on every call is a decisive loss.

Key observation: **on the CLI (Claude Code) backend the daemon already owns the agent's stdout** (the single consumer of the output pipe). Then having the agent stamp a control marker into its output text and the daemon pick it up is the shortest path — 0 tokens, 0 process spawns.

## 결정
**The CLI (Claude Code) backend control channel = an in-band output marker (M3).** The agent stamps a single-line sentinel marker into the stream-json output → the daemon detects it in the output stream it already owns → the daemon delivers it like mail (a write to the target's stdin). The engram-ctl CLI, the token, and the WS ingress are discarded.

**Per-backend implementation split (capability matrix, ADR-0002)** — the same capability "the agent emits a control signal" is implemented differently per backend:
- **CLI (Claude Code) = the M3 output marker** (the daemon owns stdout → picks up the marker).
- **API/SDK = a direct tool callback** (we own the inference loop → receive tool_use directly = a true in-process callback).
- **M4 (Bash sentinel + hook) = a documented fallback** (a hedge for terminal mode and json format collapse). Not implemented now.

**M3 delivery model = one-way fire-and-forget** — A→daemon only. Delivery and storage are the daemon's responsibility and the sending agent has no return value. Inbox, ACK, and persistence are separate follow-ups (non-goals).

**The detailed design (the marker protocol schema, the fence-skip parser, `OutputEvent::ControlSignal`, daemon routing, display suppression) is passed to a follow-up design and ADR.** This ADR records only the channel mechanism (M3) and the discarding of engram-ctl.

## 거부한 대안
- **The engram-ctl CLI (ADR-0080, M1)** — token exposure in the agent env (an attack surface for an injected child manipulating siblings) + a process spawn on every call + WS handshake (a loss at high frequency). On the CLI backend the daemon already owns stdout, so a separate WS ingress is a detour.
- **MCP over stdio/HTTP (M2)** — the tool schema stays resident in the agent context (~0.5~1.5k tokens/agent) + endpoint authentication and a permission gate. A direction ADR-0080 already deferred as "too much ceremony for a single local app". Reconsider if the command catalog grows or "narrow tool permissions instead of a broad Bash" becomes important (deferred, not permanently rejected).
- **hook-as-primary (M4)** — a hook is not an in-process callback but an **indirect** way in which a script launched by claude relays outward + a new daemon ingress is needed + a tool-call cost per message. Left only as a backup (documented fallback).
- **File drop (M5)** — robust, but a process spawn on every call (a loss at high frequency).
- **Fenced block marker** — in measurement, when the agent *explained* the format, it stamped the marker inside a code fence example (4/5). A fenced block cannot tell a real marker from an example marker, so it is actually worse → adopt a **single-line sentinel + a fence-skip parser**.

## 근거
- **Measurement (claude 2.1.170, throwaway spike `.spike-control/` — git-untracked, deleted):** with a strict single-line instruction the marker lines were **38/38 perfect** (valid JSON, receiver/body, escapes correct). The only pitfall = when *explaining* the format it stamped the marker inside a fence example in 4/5 → defended with the fence-skip parser.
- **Live/replay boundary code check:** the resume seed is loaded by `output_core.seed()` without fanout, and only live goes through `emit()` → the marker parser never sees the replay/transcript seed (no side-effect re-execution). Consistent with ADR-0079.
- **Security and speed (user decision):** a lower attack surface with 0 tokens and 0 spawns + no overhead for high-frequency messaging. The in-band approach is far superior to engram-ctl on both axes.
- The OSS and grounding basis is detailed in ADR-0080 (that investigation is still valid — only the engram-ctl ingress decision is overturned).

## 영향 / 불변식
- **Discard scope = limited to the engram-ctl ingress (beware misreading).** The **UI opaque-relay and the two authority domains (backend = direct to the daemon / UI = the app ViewManager) defined by ADR-0080 survive through ADR-0081 (confirmed)**. M3 changes only the *ingress mechanism* of the control channel — the routing and authority separation after the daemon receives the signal are kept. Do not read "ADR-0080 discarded" as "the UI relay is dead too".
- **core isolation (ADR-0003):** marker detection in core (the decoder) goes **only up to event emission**, and routing is the daemon's. No tauri/daemon import in core.
- **Marker parsing only on live stdout.** Do not apply it to the replay/transcript seed path (preventing side-effect re-execution — the seed does no fanout in the first place, ADR-0079).
- **Fence-skip is mandatory.** Lines inside a code fence are ignored (defense against the measured false-trigger). The marker = a single-line sentinel (fenced blocks forbidden).
- **No recreating engram-ctl.** The crate and spike are deleted. CLI control = M3, API control = a direct callback.
- **Follow-up (undecided, outside this ADR):** the marker protocol schema and limits · adding `OutputEvent::ControlSignal{target, message}` · daemon mail details (delivery failure, absent target, sender labeling) · updating PRD/TRD §6 (the engram-ctl premise) + redesigning the Units.
