# codex 턴 도중 입력 — 피어 구현은 언제 밀어넣고 무엇을 정산하나 (2026-09-26)

- **상태:** medium (수집 3 갈래 · 메인 grounding 스팟 3건 · cross-family 적대 리뷰 FIX 5건 반영)
- **방법:** 수집자가 각 프로젝트 소스를 아래 표의 커밋에서 직접 읽었다 · 메인이 인용 3건을 원출처와 대조했다(표에 「메인 대조」로 표시) · 테스트는 돌리지 않았다
- **날짜:** 2026-09-26
- **확신도 범례:** 확실(독립 1차 출처 둘 이상, 또는 소스 직독 + 메인 대조) · 가능성 높음(1차 출처 하나 또는 부분 지지) · 불확실(추론·2차 출처) · 모름(읽지 않았거나 소스로 판정 불가)
- **선행 조사:** `docs/research/mid-turn-input-display-2026-09-25.md`(표시 모양 서베이). 이 문서는 그중 **전달 시점·정산·실패 처리**만 소스로 좁혀 판다.

## 0. 결론 (먼저)

1. **모양은 셋이다.** ① 즉시 steer — Codex CLI TUI(Enter) · paseo 기본값. (Zed codex-acp 는 「즉시 제출(core 처리 미확인 · core 0.137.0 고정)」이라 steer 로 분류하지 않는다.) ② 도구 완료 이벤트까지 붙들기 — t3code queue 모드(승인·질문 대기와 일시 전송 게이트가 풀려야 나간다). ③ 턴 끝까지 붙들기 — vibe-kanban · paseo queue 모드(단 권한 요청이 대기 중이면 queue 가 interrupt 로 바뀐다) · Codex TUI(Tab). — 확실
2. **codex 가 기록만 하고 답하지 않은 입력에 빈 후속 턴을 내는 곳은, 확인한 곳(벤더 TUI · t3code · vibe-kanban)에는 없다 — paseo·Zed 는 미확인.** 벤더 TUI 도 그 손실을 받아들인다. — 불확실(paseo·Zed 미조사)
3. **경계를 재거나 타이밍 경합을 거는 곳은 없다.** 벤더는 core 가 루프 머리에서 접어 넣는 것에 기대고, t3code 는 새 `tool.completed` 가 보이는지만 본다. — 확실
4. **이미 넘긴 항목을 취소하는 기능은, 확인한 곳(벤더 TUI · t3code · vibe-kanban)에는 없다 — paseo·Zed 는 미확인.** ✕(되돌리기)는 클라이언트가 항목을 쥐고 있는 동안만 있다. — 불확실(paseo 취소·Zed core 라우팅 미조사)
5. **거절 처리는 두 갈래다** — 새 턴으로 폴백(벤더 TUI · paseo) 또는 앞머리에 붙들고 사용자 조작 대기(t3code). — 확실
6. **우리 결정:** 사용자 결정 2026-09-26 — codex 는 t3code 모양(도구 완료 이벤트에 밀어넣기)으로 줄인다. 옛 설계(경계 판정 · 정산 · 빈 후속 턴 · 답 구간)는 걷는다.

## 1. 비교표

| 항목 | Codex CLI TUI | t3code | paseo | vibe-kanban | Zed codex-acp |
|---|---|---|---|---|---|
| 읽은 커밋 | `openai/codex` `rust-v0.156.1` (`b412ff32`) | `5378f87f9` | `bbe3f17` | `78580443` | `296069e` |
| codex 연결 | app-server | app-server | app-server | CLI 하위 프로세스 | codex-core in-process |
| 턴 중 보내기 | Enter = 즉시 `turn/steer` · Tab = 로컬 대기 | 설정 queue(기본)/steer — 어느 쪽이든 `turn/start` | 설정 interrupt/steer/queue, 기본 steer | 세션당 대기 1건 | 즉시 `Op::UserInput` 제출(core 처리 미확인 · core 0.137.0 고정) |
| 언제 넘기나 | steer = 즉시 · Tab = idle 때 | 최신 `tool.completed` id 가 바뀌거나 턴이 끝나면 만기 — 승인·질문 대기와 일시 전송 게이트가 풀려야 실제로 나간다 | steer = 즉시 · queue = 에이전트 정지 뒤(권한 요청 대기 중이면 interrupt) | 실행 종료 뒤 후속으로 | 즉시(busy 검사 없음) |
| 정산 기준 | `clientId` 일치 `userMessage` 에코(폴백 = 텍스트+이미지 수) | 서버 read-model 에 id 가 나타남 | 조사 안 됨 | 해당 없음 | 모름 |
| 답 없이 기록된 입력 | 후속 턴 없음 | 후속 턴 없음 | 발견 못 함 | 해당 없음 | 모름 |
| 거절 | 턴 id 비우고 `turn/start` · review/compact 는 다음 idle 에 먼저 재전송 | 앞머리에 붙듦, 자동 재시도 없음 | interrupt + 새 턴(진행 중 턴이 없으면 `inactive` · 소유권 확인 실패 시 throw 로 보임 — 미독) · queue 실패는 앞머리 재대기 | Failed/Killed 면 폐기 | 해당 없음 |
| 취소 | Esc = interrupt 후 대기 steer 를 한 턴으로 합쳐 재제출 | 삭제 → 입력창 · Stop = 큐 전체를 입력창으로 | 조사 안 됨 | DELETE → 편집기 | `Op::Interrupt` 1회(스레드 전체) |
| 크기(대략) | 1.3–1.8k 줄 (불확실) | 450 줄 (가능성 높음) | 450 줄 (가능성 높음) | 300 줄 | 150 줄 |

## 2. 프로젝트별 메모

### 2-1. Codex CLI TUI (`openai/codex`@`b412ff32`, 태그 `rust-v0.156.1`)

- 라우터는 활성 턴 id 가 캐시돼 있으면 `turn/steer`, 없으면 `turn/start` 를 고른다(`codex@b412ff32 tui/src/app/thread_routing.rs:736-760`). Tab 은 idle 까지 로컬 대기다(`tui/src/bottom_pane/chat_composer.rs:3510-3518`). — 확실
- **TUI 는 아무것도 재지 않는다.** core 가 샘플링 요청과 그 도구들이 끝난 뒤 각 루프 반복의 머리에서만 대기 입력을 비운다(`core/src/session/turn.rs:423-445, 552-567`). — 확실
- `PendingSteer` 목록을 두고, steer 응답이 아니라 **커밋된 `userMessage` 에코**가 `clientId` 로 맞을 때 앞 항목을 지운다(폴백 = 텍스트 + 이미지 수 비교)(`tui/src/chatwidget.rs:1374-1400`). — 확실
- 태스크가 끝날 때 남은 입력은 샘플링 없이 기록만 된다(`core/src/tasks/mod.rs:643-683`). TUI 는 그 입력을 위한 후속 턴을 띄우지 않는다. — 가능성 높음
- `no active turn to steer` 를 받으면 캐시된 턴 id 를 비우고 `turn/start` 로 보낸다(`thread_routing.rs:783-792` — 메인 대조). review/compact 의 `ActiveTurnNotSteerable` 은 거절 큐에 넣었다가 다음 idle 에 가장 먼저 재전송한다(`tui/src/chatwidget/input_restore.rs:184-210, 288-305` · idle 자동 전송 = `tui/src/chatwidget/input_flow.rs:234-265`). — 확실
- 대기 steer 가 있을 때 Esc = interrupt 뒤, 서버가 그것들을 버렸으므로 **하나로 합쳐 새 턴으로 재제출**한다(`input_restore.rs:311-378`). — 확실 · 이미 steer 된 항목은 일반 모드에서 취소할 수 없다. — 가능성 높음

### 2-2. t3code (`t3code@5378f87f9`)

- 설정 `followUpBehavior` = queue(기본)/steer. codex 에서는 `turn/steer` 를 **쓰지 않고** 턴 도중이라도 늘 `turn/start` 를 보낸다 — 코드 주석 "Codex accepts follow-ups while the current turn is still running"(`CodexSessionRuntime.ts:2470-2487` — 메인 대조). — 확실
- queue 모드는 클라이언트가 붙든다. 항목마다 넣을 때의 최신 `tool.completed` 활동 id 를 적어 두고, 최신 id 가 달라지거나 턴이 더는 running 이 아니면 만기가 된다(`queuedMessageStore.ts:196-205` — 메인 대조). 경계 하나에 하나씩 보낸다(`ChatView.tsx:8603-8644`). **만기는 전송 조건의 하나일 뿐이다** — 승인·질문 대기, 그리고 일시 전송 게이트(환경 오프라인 · 설정 미적재 · 체크포인트 되감기 · 메시지 적재 · 머신 미선택)가 남아 있으면 만기가 된 항목도 나가지 않는다(`ChatView.tsx:8614-8634`). — 확실
- 항목은 보내기 직전 `take()` 에서 빠지고, 낙관 행은 서버 read-model 에 그 id 가 보이면 지워진다 — codex 에코 대조는 없다. — 가능성 높음
- 답 없이 기록된 입력에 대한 후속 처리는 없다. — 가능성 높음
- 실패 = 사용자 조작까지 앞머리에 붙듦, 자동 재시도 없음. 삭제 = 입력창으로 되돌림, Stop = interrupt 전에 큐 전체를 입력창으로 쏟는다. — 확실

### 2-3. paseo (`paseo@bbe3f17`)

- `sendBehavior` interrupt/steer/queue, 기본 steer(`app/src/hooks/use-settings/storage.ts:29,121,194`). — 확실
- steer = `clientUserMessageId` 를 단 즉시 `turn/steer`(`codex-app-server-agent.ts:4174-4218`). 확정 거절(`no active turn to steer` · not steerable · 턴 id 불일치)은 interrupt + 새 턴으로 폴백한다(`agent-manager.ts:2435-2474`). 단 그 대체 경로는 진행 중 턴이 없고 기대한 턴 id 가 활성 턴과 같으면 `inactive` 를 돌려주고, 턴 소유권 확인에 걸리면 throw 하는 것으로 보인다(`agent-manager.ts:2462-2477` — 확인 함수 본문은 미독, 불확실). — 확실
- queue 모드 = 에이전트가 멈출 때까지 붙들고 머리 하나만 보낸다(`host-runtime.ts:2164-2210`). 실패한 대기 전송은 앞머리로 재대기. — 확실
- **예외:** 권한 요청이 대기 중이면 queue 동작이 interrupt 로 바뀐다 — 권한 응답 전까지 턴이 멈춰 있어 대기시키면 메시지가 고립되기 때문이다(`app/src/composer/index.tsx:1479-1493` · `composer/input/state.ts:8-12`). — 확실
- 답 없이 기록된 입력 처리는 찾지 못했다. — 불확실

### 2-4. vibe-kanban (`vibe-kanban@78580443`)

- `turn/steer`·`turn/interrupt` 를 아예 쓰지 않는다. — 확실
- 세션당 대기 1건(다시 넣으면 교체), 실행이 끝난 뒤 후속으로 보내고 실행이 Failed/Killed 면 버린다(`container.rs:619-666`, `queued_message.rs:33-71`). 취소 = DELETE → 편집기로. — 확실

### 2-5. Zed codex-acp (`codex-acp@296069e`)

- app-server 가 아니라 codex-core 를 in-process 로 링크한다. 두 번째 `session/prompt` 는 busy 검사 없이 즉시 `Op::UserInput` 으로 제출된다(`thread.rs:3173-3281` · 제출 지점 `3263-3279`). — 확실 · 그 줄은 **제출**만 보여 주고 core 가 그것을 어떻게 라우팅하는지는 모름. 그래서 「즉시 steer」가 아니라 「즉시 제출(core 처리 미확인)」로 분류한다.
- codex core 를 `rust-v0.137.0` 태그로 고정한다(`Cargo.toml:24-35`) — TUI 근거(`rust-v0.156.1`)와 버전이 다르므로 벤더 core 의 동작을 여기에 그대로 옮겨 읽지 않는다. — 확실
- 취소 = 스레드 전체에 `Op::Interrupt` 1회. — 확실

## 3. 한계

- codex VS Code 확장·앱은 비공개 소스라 읽지 않았다.
- 추적하지 않은 경로: paseo 스토어 reconcile · t3code 가 `item/completed` 를 `tool.completed` 로 들이는 경로 · codex abort 경로.
- 테스트는 하나도 돌리지 않았다 — 모든 판정은 소스 정독이다.
- 메인 대조는 3건(`thread_routing.rs:783-792` · `CodexSessionRuntime.ts:2470-2487` · `queuedMessageStore.ts:196-205`)뿐이다. 적대 리뷰의 재대조는 아래 4절.
- paseo 의 답 없이 기록된 입력 처리·취소, Zed 의 core 라우팅은 조사하지 않았다 — 결론 2·4 가 그 범위로 좁혀져 있다.

## 4. 적대 리뷰 결과

cross-family 적대 리뷰가 인용 10건을 원출처와 다시 대조했다 — 8건 지지 · 2건 부분 지지. FIX 5건을 반영했다.

1. paseo queue 모드는 권한 요청 대기 중이면 interrupt 로 바뀐다 — 「멈출 때까지 붙든다」에 예외를 달았다(결론 1 · 비교표 · 2-3).
2. t3code 의 새 `tool.completed`(또는 턴 끝)는 대기 항목을 **만기**로 만들 뿐이고, 승인·질문 대기와 일시 전송 게이트가 여전히 전송을 막는다 — 시점 서술을 좁혔다(결론 1 · 비교표 · 2-2).
3. paseo steer 거절의 대체 경로에는 `inactive` 반환과 턴 소유권 확인 분기가 있다 — 단서를 달았다(비교표 · 2-3 · 확인 함수 본문 미독).
4. 결론 2·4 를 확인한 범위(벤더 TUI · t3code · vibe-kanban)로 좁히고 확신도를 불확실로 내렸다 — paseo·Zed 는 미확인.
5. Zed codex-acp 는 제출만 확인됐고 core 를 `rust-v0.137.0` 으로 고정한다 — 「즉시 steer」 분류를 「즉시 제출(core 처리 미확인 · core 0.137.0 고정)」로 바꿨다(결론 1 · 비교표 · 2-5).

덧붙여 벤더 TUI 의 거절 steer 재전송 근거에 idle 자동 전송 인용(`input_flow.rs:234-265`)을 더했다.
