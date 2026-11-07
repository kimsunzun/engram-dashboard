# ADR-0086: Control channel = dual typed entrances (MCP+CLI) + SQLite mailbox-first — marker (M3) discarded

- 상태: 확정 (2026-07-16, basis: convergence of two independent investigations (the main /research fan-out + the user's separate Codex investigation — cross-family confirmation) + four spike measurements + approval of the user's full briefing) · 부분 폐기 by ADR-0087 (the order of splitting step ladder ②③④ → the 2-min minimal transmission batch first + the SQLite mailbox deferred (resumed after the user learns)) · 부분 폐기 by ADR-0103 (the SQLite mailbox-first clause replaced by an in-memory mailbox)
- 관련: Supersedes ADR-0085 (marker = primary channel — discarded entirely) · ADR-0080 (engram-ctl ingress — the typed CLI idea is revived, the name must not be reused) · ADR-0081 (UI opaque-relay and the two authority domains survive — valid regardless of this reversal) · ADR-0007 (epoch — linked to token rotation) · ADR-0029 (daemon = agent host) · ADR-0002/0030 (capability matrix) · CLAUDE.md §5(LLM-우선 제어) · decision note `docs/process/S17-llm-control-surface/control-channel-deliberation-m3.md` · original Codex investigation `I:\claude-code-multiagent-messaging-research.md` (outside the repo) · Amended by ADR-0087 (the order of splitting step ladder ②③④ → the 2-min minimal transmission batch first + the SQLite mailbox deferred (resumed after the user learns)) · Amended by ADR-0103 (the SQLite mailbox-first clause replaced by an in-memory mailbox)

## 맥락
We must decide the channel for agent A→B messages (and future control commands). ADR-0085 adopted "since the daemon already owns stdout, an in-band output marker (M3) is the shortest path", but the 38/38 measurement that was its basis was under strictly controlled conditions. Afterwards two independent investigations (the main /research 3-branch fan-out + the user's separate Codex investigation) converged on the same conclusion:

- **Zero OSS cases that adopted free-text marker stdout scraping as the primary channel**. Zero cases of a PreToolUse hook primary channel. Raw tmux send-keys is an anti-pattern.
- The dominant ecosystem pattern = **a typed dedicated interface + a durable mailbox**. Orca (20.1k⭐) demonstrates a Bash carrier + a typed CLI (not a magic string).
- In the follow-up Bash sentinel spike (haiku) compliance was 66% (4 clean parses out of 6 runs — quoted body broken, nonce misreported, asking back about an ambiguous receiver).
- Codex's key insight: MCP vs Bash is a secondary question. The real fork = **"does the orchestrator own the typed ingress + the durable mailbox"**.

## 결정
**Control channel = dual typed entrances + a common pipeline + SQLite mailbox-first.** The marker (M3) is discarded.

```
[스폰 시 1회] 데몬: (AgentId, epoch)별 토큰 발급
   ├→ MCP용: 에이전트별 mcp-config 생성 (127.0.0.1:PORT + Bearer 토큰 헤더)
   └→ CLI용: PTY env 주입 (ENGRAM_TOKEN)   ※epoch 회전=구토큰 폐기, kill=즉시 폐기

[런타임] 에이전트
   ├─[입구① McpIngress] send_message tool_use → 웜 연결(시작 시 1회 바인딩) — 기본 후보
   └─[입구② CliIngress] Bash("engram send --to B ...") → 콜마다 접속+env토큰 (스텝5)
        ▼ ControlIngress seam — 둘 다 정규화
   ControlCommand { from*, cmd, args }   *from = 페이로드 아님, 토큰에서 파생(사칭 차단)
        ▼ Validator (수신자 실존·스키마·상한 · 후일 authz)
        ▼ Mailbox: SQLite WAL append = ★source of truth★
          (id·from·to·sequence·body·created_at·delivered_at·read_at)
        ▼ Dispatcher: 온라인→ write_stdin push / 오프라인→ 장부 대기, 복귀 시 배달
        ▼ 동기 enqueue-ACK (입구 무관 동일 JSON)
```

- **two-level ACK:** the synchronous ACK = only "it was put in the ledger" (there is no "read" ACK). A semantic reply = a separate message/thread. (The ecosystem's dominant pattern)
- **Commands = split into dedicated tools per intent** (zero cases adopting a generic mega-tool). This scope = send only, and spawn/window move and so on follow as additive. **Workers are exposed only send_message** (least-privilege = solves authz and the MCP context tax at once).
- **Authentication:** at spawn, a token per (AgentId, epoch) → the registry. MCP = baked into the mcp-config and bound once at connection (the token is outside the model's shell — the claude.exe transport layer handles it automatically) / CLI = presented on every call via env (a call = a process, so identity cannot be pinned). `from` is always derived from the token — the payload from is ignored.
- **TUI mode = excluded from the control channel** (user decision — with no separate handling, "naturally excluded because it cannot be parsed").
- **Step ladder (one step = one verification gate):** ① token + MCP entrance connection ② send_message tool + Validator + enqueue-ACK ③ SQLite mailbox ④ Dispatcher push + delivered_at + offline recovery ⑤ CLI entrance ⑥ A/B measurement → **the default entrance decision = the user**.
- Dependency added: **rmcp (the official Rust MCP SDK)**.

## 거부한 대안
- **In-band output marker (M3, ADR-0085)** — zero cases of primary channel adoption in two independent investigations + spike compliance 66% (broken at quotes and nonce even with strict instructions). A channel that "presupposes perfect compliance" collapses with weak models and long sessions.
- **PreToolUse hook primary channel** — effectively a hand-made reinvention of MCP (a process spawn per message + an intermittent idle bug on deny #24327 + too many parts). Zero primary channel adoptions in the investigation.
- **Raw tmux send-keys relay** — judged an anti-pattern in the investigation (no typed contract, misdelivery).
- **MCP Channels** — not meant for agent-to-agent messaging (ephemeral, delivery reliability issue #45563).
- **Depending on the Agent Teams file mailbox (`~/.claude/teams/...`) contract** — an unofficial reverse-engineering (observed in v2.1.45), so it must not be treated as an official contract (Codex correction).
- **Referencing claude-flow (ruflo)** — even with 64k⭐, many placeholders and a presumed retry logic defect (Codex code check).

## 근거
- **Convergence of two independent investigations = cross-family confirmation** — the main /research 3-branch fan-out and the user's separate Codex investigation reached the same conclusion without knowing each other (typed ingress + durable mailbox dominates). Codex corrected two of the main investigation's findings (Orca = a typed CLI, the Agent Teams mailbox = unofficial), and by the user's instruction Codex was adopted first.
- **Orca (20.1k⭐) demonstration** — Bash carrier + typed CLI + a mailbox that separates sequence/delivered_at/read_at. But Orca is an app-embedded runtime (Electron, dies with the app) — our separate daemon (ADR-0029) has a structural advantage in recovery semantics. "The daemon owns the PTY + clients detach" is a classic pattern demonstrated by the tmux server.
- **Four spike measurements (this session)** — ① the stream-json envelope: in the complete envelope the tool_use input is fully-formed (no delta reassembly needed), envelope types vary → defensive parsing is mandatory ② Bash sentinel 66% → confirms discarding the marker approach ③ hook: the deny idle bug #24327 ④ MCP cannot be loaded dynamically (the server must be declared at startup) → "connection succeeded" is a milestone verifiable right after spawn.
- **User approval** — approved after the full architecture briefing (a two-layer briefing), the Step 1 scope approved.
- **Bearer header measured and confirmed (2026-07-16, claude 2.1.170)** — the Authorization in the mcp-config `headers` is actually sent on all initialize, tools/list, and tools/call requests + the `Mcp-Session-Id` echo works (spike ⑤, `control-channel-research-consolidated.md` §6). rmcp 2.2.0 officially supports a Streamable HTTP server + the axum middleware authentication pattern. But the daemon MCP server must not advertise OAuth metadata (#59467 — when advertised, a static Bearer is ignored).

## 영향 / 불변식
- **Mailbox SQLite append = source of truth.** The Dispatcher push is only a means of delivery — delivered_at prevents re-injection, and an offline receiver gets delivery on return.
- **`from` is derived only from the token** — the payload from is ignored (blocking impersonation by a prompt-injected/malfunctioning agent). Since it is the same OS user, hard isolation is impossible to begin with — the final defense is the Validator.
- **Token lifetime = (AgentId, epoch)** — on epoch rotation (restart/fresh fallback) the old token is revoked, and on kill it is revoked immediately (linked to ADR-0007).
- **Worker exposure = send_message only** — dedicated tools per intent are added as additive, but least-privilege is the default.
- **Do not reuse the engram-ctl *name*** — the typed CLI entrance itself is revived (step 5), but to prevent confusion with the discarded crate name it is named `engram-send` or a daemon subcommand (at the main agent's discretion + reported).
- **ADR-0081 (UI opaque-relay and the two authority domains) survives** — do not read the 0085 discard as "the UI relay is dead too" (inheriting the same warning from 0085). Check consistency with 0081 when carving out UI commands.
- **The default entrance (MCP vs CLI) decision = the user after the step 6 A/B measurement** — until then MCP is only the default candidate, not confirmed.
- Code anchors: on entering implementation, plant `// ADR-0086` anchors in the token registry, the ControlIngress seam, and the Mailbox.
