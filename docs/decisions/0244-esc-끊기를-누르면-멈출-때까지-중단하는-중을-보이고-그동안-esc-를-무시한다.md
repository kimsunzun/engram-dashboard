# ADR-0244: Pressing the Esc interrupt shows 「중단하는 중…」 until it stops and ignores Esc in the meantime

- 상태: 확정 (2026-09-28, basis: user decision ⑬ (TRD `docs/process/S21-chat-ux/trd.md` §11) + peer research `docs/research/interrupt-feedback-peers-2026-09-28.md` + implementation details = main decision (delegated · can be reversed) · implementation = TRD §7 FE-1.4) · 부분 폐기 by ADR-0257 (decision 2) · 부분 폐기 by ADR-0262 (decision 2) · 부분 폐기 by ADR-0263 (the chat-tail Wait row place of implementation detail b)
- 관련: TRD `docs/process/S21-chat-ux/trd.md`(§3-2 · §3-4 residual race · §3-6 · §7 FE-1.4 · §8-2 F2 · §9 · §10 0244 · §11 ⑬) · research `docs/research/interrupt-feedback-peers-2026-09-28.md` · research `docs/research/claude-interrupt-spike-2026-09-28.md`(§0 conclusion table S1 row · §2 S1 — 9 ms – 1.0 s from interrupt to `result` · §5 S4 — an interrupt lands on whatever was pending at that moment (latch)) · `src/store/interruptStore.ts` · ADR-0237(decisions 1 · 4 — Esc interrupt · predicate) · ADR-0238(claude turn-open gate · residual race) · ADR-0243(interrupt marker row — the fact of being interrupted is still announced by the turn end) · CLAUDE.md 「LLM-우선 제어」 · step-log S21 · Amends ADR-0237 (decision 1's no-change to the optimistic state, and decision 4's predicate gains an interrupting indicator and an ignore state) · Amended by ADR-0257 (decision 2) · Amended by ADR-0262 (decision 2) · Amended by ADR-0263 (the chat-tail Wait row place of implementation detail b)

## 맥락
TRD §3-2 (ADR-0237 decision 1) decided: after Esc calls `fireAndForget('agent.interrupt', …)`, 「★낙관 상태를 바꾸지 않는다★ — 끊겼다는 것은 턴 끝 사건이 알린다」. So after Esc the screen stays as it is until the turn-end event arrives — in the B2 spike that gap was 9 ms – 1.0 s (research §0 S1 row · §2).

The B3 concurrency review raised the risk of that gap (not measured). If the user presses Esc again in the meantime, a second interrupt line also goes out (the claude turn-open gate is still open). Per the spike §5 (S4) observation — an interrupt lands on whatever was pending at that moment — that line could land on the queued next message B, and B could be interrupted as soon as it starts. ★Confidence = uncertain★ — a second interrupt received while stopping has never been tested.

Peer research (light — the research document above): 4 of 5 GUI peers change the screen the moment it is pressed, and among them paseo · vibe-kanban · cline ignore a re-press or block the button while stopping. The terminal UIs (codex TUI · Claude Code) have no immediate indicator. t3code attaches a turn id to every interrupt, but our claude interrupt line has no turn-id field, so that method cannot be used (TRD §3-4 residual race).

## 결정
Notation — **[사용자]** = a decision the user answered · **[메인]** = an orchestrator decision (delegated · can be reversed).

The user's original words (2026-09-28, after the main agent put up A 「즉시 표시 + 무시」 · B 「표시 없이 연타만 무시」 · C 「그대로 둔다」 together with the peer research): 「ㅇㅇ 중단하는중 나오고 그동안 esc 무시.」

1. **Pressing the Esc interrupt immediately shows 「중단하는 중…」 and ignores Esc until that agent stops** [사용자 — A · paseo's shape].
2. **The backend does not change — the place that blocks is the frontend** [메인]. The earlier-proposed 「턴마다 끊기 한 번(`TurnGate` 의 `sent` 표식)」 is replaced by this decision. B3 stays as is.
3. **This indicator means 「멈춰 달라고 했다」, not 「멈췄다」** [메인]. TRD §3-2's 「낙관 상태를 바꾸지 않는다」 is reversed **only for the indicator** — that the turn actually stopped is still announced by the turn-end event (interrupt row · interrupt marker row — ADR-0243).

Implementation details [메인]:

- **a. One small 「끊는 중」 state per agent** (`src/store/interruptStore.ts` — one pending request per agent + the number of that agent's chat views mounted in this window). ★Each window has its own★ — a pop-out window does not see another window's state.
  - **Set** — set **synchronously in the same tick** in which the window command `agent.interrupt` sends (before the reply · frames). So a human's Esc and an LLM calling the window command share one state (CLAUDE.md 「LLM-우선 제어」).
  - **Not set if there is no view** — if not a single chat view of that agent is mounted in this window, the command does not set the state (nobody would clear it).
  - **Cleared** — on any one of the following:
    - **The window command ends with any error** — input lease rejection (`CONFLICT:`) · the channel's 「unsupported」 (there is no turn to interrupt) · connection error, all of them.
    - **RichSlot — the live-frame turn-boundary count** — the accumulator counts turn boundaries (`MessageDone` · `TurnEnd` — regardless of outcome) (it never decreases). It is cleared when a frame the view received after its first `'live'` raises that count. So it is cleared even if the end of A and the start of B arrive at once. ★It is not seen as a turn-end state transition (false → true)★ — seen as a transition, it would miss 「보냄 → 여는 프레임 전 Esc → [`TurnEnd`, 다음 턴의 `TextDelta`] 한 묶음」. ★Frames before this incarnation's first `'live'` are not counted★ — an old turn end in a new view's history · a new incarnation's history must not clear a live interrupt.
    - **`streaming` true → false** (backup) — covers only a send that entered the queue without opening a turn (`Queued` → Wait off).
    - **When the view first catches up to `'live'` and there is no open turn** — the interrupted turn already ended inside the pre-live flush.
    - **The incarnation changes** (RichSlot onReset).
    - **The agent is absent (Killed · Exited · Failed · dropped from the registry) or the connection is lost — even briefly.** An Esc after reconnecting interrupts again.
    - **When no healthy view is left** — a view whose own subscription dropped or errored leaves the count (it releases its own `watch`). When the last view is released, it is cleared. ★A second view's subscription error does not clear the state of other healthy views★.
  - **There is no time limit** — if the turn end never arrives, the state stays (the same as how Wait stays stuck today).
- **b. While interrupting**, the Wait row at the chat tail shows 「중단하는 중…」 with a stop icon, and the Esc predicate returns false (Esc ignored — `ctx.interrupting`). New i18n key `chat.interrupting: '중단하는 중…'`. **Tone = dim text + blinking** (not the accent color · bold of the U8 end row) — because it is not yet a finished outcome.
- **c. Calling the window command again while interrupting does not send again** — it returns `{ outcome: 'requested' }`, or waits for the pending request's result (including a rejection) and returns it as is.

## 거부한 대안
**Rejected by the user** (the reason is only the choice itself and the peer research the main agent put up):
- **B — no indicator, just ignore the mashing.**
- **C — leave it** (today's shape · no screen change until the turn end · mashing goes out as is).

**Rejected by the main agent:**
- **One interrupt per turn in the backend (the `sent` marker on `TurnGate`)** — the indicator (user's A) needs frontend state anyway, and that blocks mashing on the window path **only in a window that has a view**. Even with that limit the reason stands (main). The second interrupt on the direct bus path and in a window with no view is accepted as TRD §9's known limitation. If measurement shows the risk is real, reopen it.

## 근거
- **User decision ⑬ (2026-09-28)** — the original words are at the head of 「결정」. Record = TRD §11 ⑬.
- **Peer research** — `docs/research/interrupt-feedback-peers-2026-09-28.md` (paseo · vibe-kanban were checked by the main agent against the source · certain).
- **The risk itself is unverified** — whether a second interrupt while stopping lands on B was not measured (uncertain). This decision closes the entrance of the window path without measuring that risk.
- **Main decisions 2 · 3 · a · b · c** — delegated by the user. Can be reversed.
- ★**Verification status**★ — GUI measurement plan = TRD §8-2 F2 ⑩. Test list = TRD §3-6.

## 영향 / 불변식
- **An interrupt sent directly over the daemon bus (`engram agent.interrupt …`) does not go through the window command** — that path has neither the indicator nor the guard. The same holds for a window command called from a window that has no chat view of that agent (known limitation · TRD §9).
- **The residual race of a single press (TRD §3-4 · ADR-0238) stays as is** — a late first Esc right after the turn end can interrupt the next turn that the CLI opened by itself. This decision blocks only the second press.
- **An Esc after B has visibly started is B's ordinary interrupt** — 「끊는 중」 is cleared at the end frame of the previous turn, so that Esc is not ignored.
- **An interrupt that was accepted but not obeyed — known limitation** — `requested` means 「줄에 실었다」. If an accepted interrupt is in fact not obeyed (e.g. codex queues `turn/interrupt` and returns `Ok` without the peer's reply), until the turn ends by itself that window's Esc and window command do nothing. No time limit is set (intentional — a above). Ways out = close that view · call the bus directly (TRD §9).
- **If killed while interrupting**, it is cleared the moment the agent becomes absent (and on onReset if respawned) — it does not stay even if the turn end never comes.
- **Early clearing of a newly attached view — known limitation · accepted** — if a newly attached view's first catch-up ends at the end of the previous turn (another view just sent and the new turn's frames do not exist yet), it is cleared early as 「열린 턴 없음」. It just goes back to today's behavior (Esc can send again). The window is short (TRD §9).
- **The indicator is not the source of truth** — the turn end is not decided from the 「끊는 중」 state. Turn end · interrupt are decided by the turn-end event (ADR-0238 · ADR-0243), and this state only watches it and is cleared.
- **History replay is prevented from clearing it** — turn boundaries are counted only from frames after this incarnation's first `'live'` (a above). If it were cleared by a rewound old turn end, Esc would come alive again while stopping.
- **ADR-0237 decision 1's 「낙관 상태를 바꾸지 않는다」 · decision 4's predicate** are partially amended by this decision — the indicator and ignore state grow by one.
