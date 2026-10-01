# 끊기(interrupt) 누른 뒤의 피드백 · 연타 처리 — 피어 선례 (light)

- 상태: 조사 완료(light · 적대 리뷰 없음) · 결정 = ADR-0244(2026-09-28 · A 채택) — 이 조사가 받친 선택은 TRD `docs/process/S21-chat-ux/trd.md` §3-2 「낙관 상태를 바꾸지 않는다」의 재검토였다(§11 ⑬).
- 방법: 수집자 1명(로컬 클론 소스 판독 + 공개 문서) · 메인이 핵심 셋(paseo · vibe-kanban · t3code)을 소스에서 스팟 대조.
- 날짜: 2026-09-28
- 확신도 범례: 확실 · 가능성 높음 · 불확실

## 질문

1. 끊기를 누른 직후, 멈춤이 확인되기 전까지 「중단 중…」 같은 즉시 표시를 보이나?
2. 그 사이 다시 누르면 어떻게 되나?

## 결론 (먼저)

- **GUI 피어 5개 중 4개가 누르는 즉시 화면을 바꾼다.** 그중 셋(paseo · vibe-kanban · cline)은 그동안 버튼을 막거나 다시 누름을 무시한다(가능성 높음).
- **터미널 UI(codex TUI · Claude Code)는 즉시 표시가 없다.** codex TUI 는 다시 누르면 끊기를 또 보낸다(가능성 높음).
- **턴 번호로 끊기를 묶는 곳이 있다** — t3code 는 끊기마다 도는 턴의 id 를 싣고, codex app-server 규약도 `turn_id` 를 싣는다. 우리 claude 끊기 줄에는 턴 id 칸이 없어 이 방법은 못 쓴다(TRD §3-4 잔여 경합).

## 피어별

| 피어 | 즉시 표시 | 다시 누르면 | 근거 | 확신도 |
|---|---|---|---|---|
| paseo | 있음 — 버튼이 스피너 + 「canceling」 라벨. 끊기 요청이 끝나거나 다른 턴이 열리면 풀린다 | 버튼 비활성 · 단축키도 무시 | `packages/app/src/composer/index.tsx:1012-1025`(메인 대조 — `disabled={!isConnected \|\| isCancellingAgent}`) · `timeline/turn-liveness.ts:52-115` | 확실 |
| vibe-kanban | 있음 — 「Stopping」 비활성 버튼 + 스피너. 단 요청 응답이 오면 풀린다(턴 끝이 아니다) | 무시(`if (isStopping) return`) | `packages/web-core/src/shared/hooks/useWorkspaceExecution.ts:77-86`(메인 대조) · `packages/ui/src/components/SessionChatBox.tsx:557-563` | 확실 |
| cline | 부분 — 보내기·버튼을 막는다. 「취소 중」 글자는 못 찾았다 | 무시(진행 중 표시 ref) | `webview-ui/src/components/chat/chat-view/hooks/useMessageHandlers.ts:384-404` | 가능성 높음 |
| orca(PTY 위 채팅) | 있음 — 「작업 중」을 바로 숨긴다 | 막지 않는다 — 두 번째 누름은 ESC 바이트를 그대로 PTY 에 쓴다(우리와 같은 위험) | `src/renderer/src/components/native-chat/NativeChatView.tsx:343-352` · `native-chat-working-suppression.ts` | Q1 확실 · Q2 가능성 높음 |
| t3code | 본 채팅 정지 버튼은 없음(메인 대조 — 비활성 prop 없음). 백그라운드 작업 정지는 「Stopping...」 | 막지 않지만 끊기마다 도는 턴 id 를 실어 늦은 두 번째 요청이 다음 턴을 겨누지 않는다 | `apps/web/src/components/chat/ComposerPrimaryActions.tsx:85-104` · `ChatView.logic.ts:546-555` | 확실(서버가 낡은 턴 id 를 어떻게 다루는지는 모름) |
| codex TUI | 못 찾음(부재 검색이라 불확실) | 끊기를 또 보낸다 · 대상 없이 「도는 것」을 끊는다. **Ctrl-C** 는 첫 누름이 끊고 시간 제한 있는 종료 안내를 무장하며, 그 창 안의 둘째 누름이 **종료한다** | `codex-rs/tui/src/bottom_pane/mod.rs:1700-1715` · `core/src/session/handlers.rs:436-439` · Ctrl-C = `codex-rs/tui/src/chatwidget/interaction.rs:99-114` · `:204-217` · `:503-510`(openai/codex master · 조사 시점 · 커밋 미기록) | 가능성 높음 |
| Claude Code CLI | 문서에 없음 — 「Esc 로 멈추고, 대기 메시지가 있으면 이어 보낸다」 | 문서에 없음(Esc 두 번은 다른 기능 — 되감기) | https://code.claude.com/docs/en/interactive-mode | 문서 서술은 확실 · 실제 화면은 모름 |

## 공백

- Zed 에이전트 패널은 보지 않았다(light 예산).
- Claude Code · codex TUI 는 실행해 보지 않았다.
- paseo · cline 서버가 새 턴이 열린 뒤 도착한 취소를 어떻게 다루는지는 보지 않았다.
- 우리 쪽 핵심 미확인 — claude 가 「멈추는 중」에 받은 두 번째 끊기를 무시하는지, 대기 중인 다음 메시지에 거는지는 실험하지 않았다(B2 스파이크 범위 밖).
