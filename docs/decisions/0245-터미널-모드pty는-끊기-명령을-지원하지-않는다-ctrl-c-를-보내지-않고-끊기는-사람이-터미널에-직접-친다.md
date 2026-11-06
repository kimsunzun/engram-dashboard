# ADR-0245: Terminal mode (PTY) does not support the interrupt command — it does not send Ctrl-C, and the person types the interrupt directly into the terminal

- 상태: 확정 (2026-09-28, basis: user decision ⑭ (TRD `docs/process/S21-chat-ux/trd.md` §11) · implementation = TRD §7 B3c)
- 관련: TRD `docs/process/S21-chat-ux/trd.md`(§3-1 · §3-3 · §7 B3c · §9 · §10 0245 · §11 ⑭) · `crates/engram-dashboard-agent/src/transport/pty.rs`(`interrupt()` · `capabilities()`) · `crates/engram-dashboard-agent/src/session.rs`(`interrupt` doc 「PTY=0x03 주입」) · `crates/engram-dashboard-agent/src/commands.rs`(`agent.interrupt` declaration doc 「≠ kill — 프로세스는 산다」 — it is also carried in the generated schema summary) · ADR-0001(kill, 2 verbs — the kill path this decision relies on · that ADR's body has no interrupt clause) · ADR-0030(capability derivation — capabilities are the channel's caps) · ADR-0237(bus command `agent.interrupt`) · ADR-0238(claude JSON interrupt) · ADR-0244(the window command's 「끊는 중」) · research `docs/research/interrupt-feedback-peers-2026-09-28.md`(codex TUI row — two Ctrl-C quit) · CLAUDE.md 「LLM-우선 제어」 · step-log S21 · Amends ADR-0017 (decision 5, the interrupt (Ctrl-C) part: PTY does not support interrupt) · Amends ADR-0030 (the interrupt in the PTY capability list becomes false)

## 맥락
The PTY channel's `interrupt()` wrote a raw `0x03` (Ctrl-C) into the input queue and advertised the capability `control.interrupt: true`. That is the original design — ADR-0017 decision 5 「kill(프로세스/Job 레벨)과 interrupt(Ctrl-C)는 분리 유지」.

With the new bus command `agent.interrupt` (ADR-0237 — called by the LLM · CLI), this path sends Ctrl-C whether or not a turn is running. A TUI quits when it receives Ctrl-C twice within a short window:
- **codex** — the first press interrupts and arms a time-limited quit hint, and the second press within that window quits (source reading — research document codex TUI row · `codex-rs/tui/src/chatwidget/interaction.rs` · likely).
- **claude** — known behavior · not measured here · likely.

So calling it twice can end a terminal-mode agent, which conflicts with the command's contract 「≠ kill — 프로세스는 산다」 (the `agent.interrupt` declaration doc in `commands.rs`).

No UI uses the PTY interrupt — interrupting exists only in the JSON chat pane (ADR-0237), and the terminal pane sends a person's keys to the PTY as they are.

## 결정
Notation — **[사용자]** = a decision the user answered · **[메인]** = an orchestrator decision.

The main agent put up A 「터미널 모드는 끊기 명령을 거절한다」 and B 「Ctrl-C 를 두고 끝날 위험을 문서화한다」. The user's original words (2026-09-28): 「터미널모드는 지가 알아서 하잖아. 아예 기능에서 빼야지」 · 「아니 내말은 ctrl c를 빼라는거지 터미널모드에서는 알아서 하라고해.」 — the second statement narrowed the first's 「기능에서 빼」 to **Ctrl-C injection**. The keys a person types directly into the terminal are unchanged.

1. **Remove Ctrl-C injection from the PTY channel — interrupting in terminal mode is left to the terminal** [사용자]. The Esc · Ctrl-C a person types are handled by the terminal itself, and the dashboard neither intercepts those keys nor sends them on the person's behalf (the terminal pane as it is today).
2. **The corresponding handling** [메인]: the PTY channel's `interrupt()` = `Unsupported` → the bus `agent.interrupt` answers CONFLICT · capability `control.interrupt = false` · the WS `Interrupt` message answers with an error.

## 거부한 대안
- **Keep Ctrl-C and document the risk of quitting (B)** [사용자 — chose removal].
- **Send Esc instead of Ctrl-C** [메인] — what Esc means in a TUI depends on its state (interrupt · clear the input draft · double-Esc rewind). The command cannot know that state. And a person already sends the key directly.

## 근거
- **User decision ⑭ (2026-09-28)** — the original words are at the head of 「결정」. Record = TRD §11 ⑭.
- **codex TUI quits on two Ctrl-C** — the source reading in the codex TUI row of the research document (openai/codex master · at research time · commit not recorded · likely). claude = known behavior · not measured here · likely.
- ★**Verification status**★ — implementation = TRD §7 B3c.

## 영향 / 불변식
- **The LLM · CLI cannot interrupt a terminal-mode agent with this command** — `agent.interrupt` answers CONFLICT. The bus also has no raw-input command (main — it is not in the `agent.*` declarations in `commands.rs`), so after this decision **there is no way for the LLM to interrupt a terminal-mode agent**. This is a gap in CLAUDE.md 「LLM-우선 제어」 (every feature must be controllable by the LLM), accepted by user decision ⑭ (TRD §9 · CLAUDE.md was not changed).
- **The WS `Interrupt` message answers an error for a PTY agent** (the `Interrupt` handling in `connection_core.rs` — it passes the channel's `Unsupported` through as is).
- **Capabilities still come from the channel caps (ADR-0030) — only the PTY's `control.interrupt` value becomes false.** The place that derives capabilities and its meaning do not change.
- **kill is unchanged** — process · Job-level termination (ADR-0001) does not change. This decision changes **only the interrupt (Ctrl-C) part** of ADR-0017 decision 5's 「kill 과 interrupt 분리」.
- **Do not bring Ctrl-C back** — user decision ⑭ removed it as 「터미널 모드는 지가 알아서 한다」, and bringing it back means two bus commands can end a terminal-mode agent (context above).
