# ADR-0245: 터미널 모드(PTY)는 끊기 명령을 지원하지 않는다 — Ctrl-C 를 보내지 않고 끊기는 사람이 터미널에 직접 친다

- 상태: 확정 (2026-09-28, 근거: 사용자 결정 ⑭ (TRD `docs/process/S21-chat-ux/trd.md` §11) · 구현 = TRD §7 B3c)
- 관련: TRD `docs/process/S21-chat-ux/trd.md`(§3-1 · §3-3 · §7 B3c · §9 · §10 0245 · §11 ⑭) · `crates/engram-dashboard-agent/src/transport/pty.rs`(`interrupt()` · `capabilities()`) · `crates/engram-dashboard-agent/src/session.rs`(`interrupt` doc 「PTY=0x03 주입」) · `crates/engram-dashboard-agent/src/commands.rs`(`agent.interrupt` 선언 doc 「≠ kill — 프로세스는 산다」 — 생성 스키마 요약에도 실린다) · ADR-0001(kill 2동사 — 이 결정이 기대는 kill 경로 · 그 ADR 본문에는 interrupt 조항이 없다) · ADR-0030(capability 산출 — 능력은 통로 caps) · ADR-0237(버스 명령 `agent.interrupt`) · ADR-0238(claude JSON 끊기) · ADR-0244(창 명령의 「끊는 중」) · 조사 `docs/research/interrupt-feedback-peers-2026-09-28.md`(codex TUI 행 — Ctrl-C 두 번이면 종료) · CLAUDE.md 「LLM-우선 제어」 · step-log S21 · Amends ADR-0017 (결정 5 interrupt(Ctrl-C) 부분 PTY 는 끊기를 지원하지 않는다) · Amends ADR-0030 (PTY 능력 목록의 interrupt 가 거짓이 된다)

## 맥락
PTY 통로의 `interrupt()` 는 입력 큐로 날 `0x03`(Ctrl-C)을 써 넣고 능력 `control.interrupt: true` 를 알렸다. 원래 설계다 — ADR-0017 결정 5 「kill(프로세스/Job 레벨)과 interrupt(Ctrl-C)는 분리 유지」.

새 버스 명령 `agent.interrupt`(ADR-0237 — LLM · CLI 가 부른다)가 생기면서 이 길은 턴이 돌든 말든 Ctrl-C 를 보낸다. TUI 는 Ctrl-C 를 짧은 창 안에 두 번 받으면 끝난다:
- **codex** — 첫 누름이 끊고 시간 제한 있는 종료 안내를 무장하며, 그 창 안의 둘째 누름이 종료한다(소스 판독 — 조사 문서 codex TUI 행 · `codex-rs/tui/src/chatwidget/interaction.rs` · 가능성 높음).
- **claude** — 알려진 동작 · 여기서 미측정 · 가능성 높음.

그래서 두 번 부르면 터미널 모드 에이전트가 끝날 수 있고, 명령의 계약 「≠ kill — 프로세스는 산다」(`commands.rs` 의 `agent.interrupt` 선언 doc)와 어긋난다.

PTY 끊기를 쓰는 UI 는 없다 — 끊기는 JSON 채팅 칸뿐이고(ADR-0237), 터미널 칸은 사람의 키를 그대로 PTY 에 보낸다.

## 결정
표기 — **[사용자]** = 사용자가 답한 결정 · **[메인]** = 오케스트레이터 판정.

메인이 A 「터미널 모드는 끊기 명령을 거절한다」 · B 「Ctrl-C 를 두고 끝날 위험을 문서화한다」를 올렸다. 사용자 원문(2026-09-28): 「터미널모드는 지가 알아서 하잖아. 아예 기능에서 빼야지」 · 「아니 내말은 ctrl c를 빼라는거지 터미널모드에서는 알아서 하라고해.」 — 둘째 원문이 첫째의 「기능에서 빼」를 **Ctrl-C 주입**으로 좁혔다. 사람이 터미널에 직접 치는 키는 그대로다.

1. **PTY 통로에서 Ctrl-C 주입을 뺀다 — 터미널 모드의 끊기는 터미널이 알아서 한다** [사용자]. 사람이 치는 Esc · Ctrl-C 는 터미널이 스스로 처리하고, 대시보드는 그 키를 가로채거나 대신 보내지 않는다(오늘 터미널 칸 그대로).
2. **그 대응** [메인]: PTY 통로의 `interrupt()` = `Unsupported` → 버스 `agent.interrupt` 는 CONFLICT · 능력 `control.interrupt = false` · WS `Interrupt` 메시지는 오류로 답한다.

## 거부한 대안
- **Ctrl-C 를 그대로 두고 끝날 위험을 문서화한다(B)** [사용자 — 빼기를 골랐다].
- **Ctrl-C 대신 Esc 를 보낸다** [메인] — TUI 의 Esc 뜻은 그 상태에 따라 갈린다(끊기 · 입력 초안 지우기 · Esc 두 번 되감기). 명령은 그 상태를 알 수 없다. 그리고 사람은 이미 키를 직접 보낸다.

## 근거
- **사용자 결정 ⑭(2026-09-28)** — 원문은 「결정」 머리. 기록 = TRD §11 ⑭.
- **codex TUI 의 Ctrl-C 두 번 종료** — 조사 문서 codex TUI 행의 소스 판독(openai/codex master · 조사 시점 · 커밋 미기록 · 가능성 높음). claude = 알려진 동작 · 여기서 미측정 · 가능성 높음.
- ★**검증 상태**★ — 구현 = TRD §7 B3c.

## 영향 / 불변식
- **LLM · CLI 는 이 명령으로 터미널 모드 에이전트를 끊을 수 없다** — `agent.interrupt` 는 CONFLICT 로 답한다. 버스에는 날 입력(raw) 명령도 없어(메인 — `commands.rs` 의 `agent.*` 선언에 없다), 이 결정 뒤 **LLM 이 터미널 모드 에이전트를 끊을 길이 없다**. CLAUDE.md 「LLM-우선 제어」(모든 기능이 LLM 으로 제어 가능해야 한다)의 갭이고, 사용자 결정 ⑭ 로 받아들인다(TRD §9 · CLAUDE.md 는 고치지 않았다).
- **WS `Interrupt` 메시지는 PTY 에이전트에 오류로 답한다**(`connection_core.rs` 의 `Interrupt` 처리 — 통로의 `Unsupported` 를 그대로 옮긴다).
- **능력은 그대로 통로 caps 에서 온다(ADR-0030) — PTY 의 `control.interrupt` 값만 거짓이 된다.** 능력을 산출하는 자리와 뜻은 바뀌지 않는다.
- **kill 은 그대로다** — 프로세스 · Job 레벨 종료(ADR-0001)는 바뀌지 않는다. ADR-0017 결정 5 의 「kill 과 interrupt 분리」 중 **interrupt(Ctrl-C) 부분만** 이 결정이 바꾼다.
- **Ctrl-C 를 되살리지 말 것** — 사용자 결정 ⑭ 가 「터미널 모드는 지가 알아서 한다」로 뺐고, 되살리면 버스 명령 두 번이 터미널 모드 에이전트를 끝낼 수 있다(위 맥락).
