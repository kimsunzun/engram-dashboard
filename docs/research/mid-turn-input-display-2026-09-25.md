# 턴 도중 보낸 입력을 어떻게 띄우고 어떻게 전달하나 — 채팅·에이전트 클라이언트 서베이 (2026-09-25)

- **상태:** 적대 리뷰 반영(판정 BLOCK → 지적 4건 전부 수용·정정 — §8) · 결정 대기
- **방법:** `/research` medium · 설계-결정 모드 · 수집 갈래 4(직접 피어 GUI · 터미널 에이전트 CLI · 일반 채팅 UI · 우리 두 백엔드 프로토콜) · 메인 grounding(load-bearing 10건 원출처 대조) · cross-family 적대 리뷰 1회(codex)
- **확신도 범례:** 확실(독립 1차 출처 둘 이상) · 가능성 높음(1차 출처 하나 또는 부분 지지) · 불확실(추론·2차 출처) · contested(근거 있는 반박)
- **계기:** 사용자 요청(2026-09-25) — codex 채팅 모드에서 보낸 글이 늦게 뜬다 → 「먼저 띄우고 진짜가 오면 지운다」, 그리고 claude 도 같이 한 번에.

## 0. 결론 (먼저)

1. **턴 도중 보내기는 막지 않는 것이 현재 관행이다.** 에이전트 도구는 거의 전부 허용한다(Claude Code · Codex CLI/앱 · t3code · paseo · Cline · Roo · Zed · Cursor · OpenCode). 막는 쪽은 옛 기본값(일반 ChatGPT 채팅 · Vercel AI SDK 예제)이다. — 가능성 높음
2. **보낸 즉시 보이되, 「아직 안 보냄/안 받음」 상태로 따로 보인다 — 그리고 정식 메시지의 대화 안 위치는 보낸 시점이 아니라 전달 시점이다.** 사람 메신저는 보낸 자리에 대기 표시(시계·회색)로 띄우고 서버 확인 뒤 정식이 된다(Matrix 규격은 즉시 local echo 를 MUST 로 요구). 에이전트 도구는 대기분을 **입력창 위 목록이나 대화 맨 아래**에 회색·「queued」로 둔다. 정식이 되는 시점은 갈린다:
   - **Codex TUI** — 턴 중 steer 는 보낼 때 이력에 넣지 않고 입력창 위 패널에 두었다가, 서버가 `clientId` 를 단 `userMessage` 를 되돌리는(= 코어가 **소비한**) 순간 이력에 넣는다. — 가능성 높음(소스)
   - **t3code** — 대기 행은 타임라인 끝에 있다가 **큐에서 꺼내 보내는 순간**(도구 경계·턴 끝) 지워지고, 그 자리에 새 id 의 낙관 사용자 행이 생기며, 서버 스레드에 그 id 가 나타나면 서버 행으로 바뀐다. 전환 시점은 소비가 아니라 **우리가 보낸 시점**이다. — 가능성 높음(소스)
   - **Claude Code** — 대기분을 입력창 위(2.1.275 부터 「대화 안, 스피너 위」)에 두고 「Claude 가 응답을 시작할 때까지」 회색으로 둔다. 시작한 뒤 **어디에 놓이는지는 문서에 없다**. — 가능성 높음(문서·changelog) · 최종 위치는 불확실
   - 공통으로 확인되는 것 = **보낸 자리에 정식으로 고정하지 않는다**(Codex TUI · t3code 두 소스). 예외는 Orca(깜빡임 회피). — 가능성 높음
3. **전달은 「다음 도구 경계에서 끼워 넣기(steer)」가 기본인 쪽이 다수다.** Claude Code(Enter) · Codex(0.98.0 부터 Enter=steer, Tab=턴 끝까지 대기) · paseo · Cursor. 대기(턴 끝) 기본은 t3code · Zed · Cline. 끊기(interrupt)는 늘 별도 키다. — 가능성 높음
4. **합치는 열쇠는 클라이언트가 만든 id 다.** 내용 비교는 옛 서버 폴백(Codex TUI)이나 id 가 없는 PTY 경로(Orca)에서만 쓴다. — 가능성 높음
5. **우리 두 백엔드는 둘 다 이 모양을 받칠 수 있다.** claude CLI 는 턴 도중 stdin 입력을 **스스로 다음 도구 경계에 접어 넣고** 그 진행을 `command_lifecycle`(queued → started → completed/cancelled …)로 우리 uuid 를 달고 알린다. codex app-server 는 `turn/steer`(또는 0.148.0↑에서 진행 중 `turn/start`)로 끼워 넣고, 소비되는 순간 `clientId` 를 단 `userMessage` 를 되돌려준다. — 가능성 높음

## 1. 사용자 가설 대조

| 가설 | 판정 | 근거 |
|---|---|---|
| 「일반 채팅은 그냥 뿌린다」 | **부분 지지** — 사람 메신저는 즉시 뿌리되 **대기 상태로** 구분한다. AI 채팅은 옛 기본이 「보내기 막기」였고, 에이전트 도구는 즉시 보이되 **대화 밖(맨 아래·입력창 위)에 대기로** 둔다 | §2-3 |
| 「터미널은 그 상황에서 치면 대기가 있다」 | **지지** — Claude Code 는 입력창 위에 회색으로 줄 세우고, Codex 는 입력창 위 패널에 「다음 도구 호출 뒤 제출」·「턴 끝에 제출」로 나눠 보인다 | §2-2 |

## 2. 발견

### 2-1. 직접 피어 (GUI 클라이언트)

| 피어 | 턴 중 보내기 | 표시 | 전달 | 합치기 | 취소·실패 | 확신도 |
|---|---|---|---|---|---|---|
| **t3code** | 허용 · 설정 `followUpBehavior` = queue(기본)/steer | 타임라인 **끝**에 점선 말풍선 + 상태줄(「Sends after the next tool call or when the turn ends」) | 대기 항목을 **도구 호출 경계마다 하나씩** 또는 턴 끝에 보냄. 서버 쪽 claude 어댑터는 진행 중 전송을 steer 로, codex 는 진행 중 `turn/start` | **두 단계** — ① 보내는 순간 대기 행을 큐에서 지우고(`queuedMessageStore.ts:84-106`) 새 id(`messageIdForSend`)의 낙관 사용자 행을 만든다(`ChatView.tsx:7785-7803`) ② 서버 스레드에 그 id 가 생기면 낙관 행 **제거**(`ChatView.tsx:5769-5795`) | Send now · 제거(입력창으로 복귀) · Stop 이 대기열 전체를 입력창으로 · 실패는 맨 앞 유지 | 가능성 높음(소스 대조 — 초안의 「대기 행이 서버 id 도착 때 정식이 된다」는 오독이었다, §8) |
| **paseo** | 허용 · `sendBehavior` = steer(기본)/interrupt/queue | steer·interrupt = 스트림에 즉시 `isPending` 행 · queue = 입력창 안 대기 트랙 | steer = 진행 중 `turnId` 로(codex `turn/steer`) | `clientMessageId` 를 `messageId` 로 싣고 되돌아온 항목과 제자리 병합 · RPC 거절이면 낙관 행 제거 | 편집 = 입력창으로 · 실패 시 맨 앞 재삽입 | 가능성 높음(`storage.ts:121,194` · `stream.ts:89-119` 대조) |
| **orca**(PTY 위 채팅 뷰) | Stop 버튼으로 바뀌나 Enter 로는 보냄(추정) | **정식 사용자 턴과 똑같이** 그림 — 「queued」 스타일이 깜빡여서 뺐다는 주석 | TUI 에 타이핑 | **내용 비교**(보낸 시점 경계 이후의 정규화 본문 + 횟수) | 끊기 시 에코 제거 | 불확실(수집자 코드 독해만) |
| **vibe-kanban** | 「Queue」 버튼 | 입력창 안에 대기(대화엔 없음) | 세션당 한 칸, 성공 시에만 후속 턴 | 불필요 | 타이핑하면 취소 | 불확실 |
| **Cline** | 허용 | 대화엔 안 넣고 **입력창 위 패널** | queue(VS Code) · SDK 는 steer 도 | 패널이 서버 목록 미러 | 항목별 취소 | 불확실 |
| **Claude Code VS Code 확장** | 허용 | 2.1.274 부터 「Claude 가 시작할 때까지 대화 맨 아래에서 기다린다」 | CLI 의 queue/steer | 모름 | 모름 | 가능성 높음(changelog) |
| **Codex 앱 / VS Code 확장** | 허용 · Follow-up behavior 설정(Steer/Queue) | 대기 목록(위치 모름) | steer = `turn/steer` | 모름 | 편집 시 맨 뒤로(버그 #22895) · 대기 항목 증발 버그(#31128) | 가능성 높음(문서) |
| **Zed** | 허용 · 기본 queue | 패널의 대기 항목 | 턴 끝 · 항목별 Steer 는 **자체 에이전트만**(외부 ACP 에이전트는 턴 경계를 못 봐서) | 모름 | 편집·삭제 | 가능성 높음(문서) |
| **Cursor** | 허용 · Enter = 다음 도구 호출에 전달 · Alt+Enter 대기 · Ctrl+Enter 끊기 | 모름 | steer 기본 | 모름 | Send now | 불확실(changelog) |
| **Roo Code** | 허용(3.25↑) | 「Queued Messages」 카드 | 준비되면 FIFO | 모름 | 편집·삭제 · 재정렬 없음 | 가능성 높음(문서) |
| **OpenCode** | 허용 | 대화 안에 **QUEUED 배지**(보낸 자리) | 다음 모델 요청 | 모름 | 배지가 안 풀리는 버그(#16856) | 불확실 |

**요약:** 표시 위치는 두 갈래 — ① 대화 **맨 아래** 대기 행(t3code · Claude Code · OpenCode · paseo steer) ② **입력창 위/안** 별도 목록(Cline · Roo · Zed · vibe-kanban · paseo queue). **보낸 자리에 정식처럼 고정**하는 곳은 Orca 하나(깜빡임 회피가 사유).

### 2-2. 터미널 에이전트 CLI

- **Claude Code** — Enter 는 끊지 않고 대기에 넣고 **입력창 위에 목록**으로 보인다. 「Sent and queued messages show in gray until Claude starts responding to them」. 전달: 도구 호출 중에 넣은 메시지는 **그 도구 호출이 끝나는 즉시, 같은 턴 안에서** 넘어간다. 턴이 끝났는데 남아 있으면 친 순서대로 자동 전송. 명령·셸은 턴 끝까지 보류. Esc = 끊고 대기분을 다음에 보냄 · Ctrl+Enter = 끊고 지금 보냄(2.1.275↑) · Up = 대기분을 입력창으로 되가져오기. — 표시·전달 시점·키는 **확실**(공식 문서 interactive-mode 「Queue messages while Claude works」 원문 대조 + changelog 2.1.274/2.1.275 + 설치본 2.1.280 바이너리의 `command_lifecycle` 설명 문자열). ★**응답이 시작된 뒤 그 메시지가 대화 어디에 놓이는지는 어느 출처에도 없다**★ — 불확실(§8 지적 2)
- **Codex CLI TUI** — Enter(턴 진행 중) = `turn/steer` 즉시 · Tab = 로컬 대기, 한가해지면 새 턴(0.98.0 에서 steer 기본화). 입력창 위 패널에 세 구획: 「다음 도구 호출 뒤 제출(Esc 로 끊고 즉시)」 · 「턴 끝에 제출」(거절된 steer) · 「대기 중 후속 입력」. **steer 는 보낼 때 이력에 넣지 않는다**(`render_in_history = !agent_turn_running`) — 서버가 되돌린 `userMessage` 가 오면 그때 이력에 넣으므로 **자리는 전달 시점**이다. 합치기 = 대기열 **맨 앞** 하나와 `clientId` 일치(옛 서버는 본문+이미지 개수 비교 폴백). 그냥 Esc(끊기)는 대기분을 **입력창으로 복원**한다. — 가능성 높음(소스 `rust-v0.154.0` `input_submission.rs:177` · `chatwidget.rs:1287-1336` 대조 · 0.98.0 릴리스 노트는 수집자 인용)
- **Gemini CLI**(기본 턴 끝 · 실험적 steering) · **Crush**(턴 끝) · **Amp**(기본 끊기, `/queue` 로 대기) · **OpenCode**(대화 안 QUEUED 배지). — 불확실

### 2-3. 일반 채팅 UI

- **사람 메신저:** 즉시 local echo + 대기 표시 + 클라이언트 id 로 서버 사본과 합치기가 규범이다 — Matrix 규격은 즉시 echo 를 MUST, 서버 응답 전 다른 모양을 MAY 로 두고 `transaction_id` 로 합친다 · Discord `nonce` · WhatsApp 시계 아이콘(2차 출처). 실패 표시·대기 중 도착한 메시지와의 순서 규칙은 공식 출처를 못 찾았다(Matrix SDK 의 `chronological`/`detached` 옵션만 있음). — 가능성 높음
- **AI 채팅:** 옛 기본 = 스트리밍 중 보내기 막기(Vercel AI SDK 예제 `disabled={status !== 'ready'}` · 일반 ChatGPT 채팅). 새 OSS UI(Open WebUI — 기본 켜짐, **입력창 바로 위** 대기 영역, 응답이 끝나면 대기분을 빈 줄로 이어 **한 번에** 보냄 · assistant-ui 대기 목록 + Steer)는 대기를 대화 밖에 둔다. — 가능성 높음(Open WebUI 문서 원문 대조)

### 2-4. 우리 두 백엔드가 할 수 있는 것

**claude (stream-json 채팅 모드)**
- 턴 도중 stdin 의 `user` 줄은 CLI 가 자기 명령 큐에 넣고, 진행 중 턴이 있으면 **다음 도구 경계에 접어 넣는다**. 진행 상황을 `{"type":"command_lifecycle","command_uuid":<우리 uuid>,"state":…}` 로 낸다 — `queued` → `started` → `completed`/`cancelled`/`discarded`/`refused`. 「a command folded into an already-in-flight turn emits 'completed' BEFORE that turn's result frame」. uuid 없는 메시지엔 이벤트가 없다. — 가능성 높음(설치본 2.1.280 바이너리 원문 대조 · 「@internal」 표식이 있어 안정 계약인지는 모름 · 소비 시점은 제3자 실측)
- ★**lifecycle 상태의 뜻을 「받음」으로 뭉뚱그리면 안 된다**★(§8 지적 3 · 바이너리 설명문 원문): `started` = 큐에서 턴으로 **배출됨**, `completed` = 그것을 소비한 **턴이 끝남**(그래도 답은 후속 실행에서야 나올 수 있다). 예외 종료 시 `started` 에 종결 상태가 안 오거나 `queued` 만 남을 수 있고, 호스트가 프로세스 종료 때 미종결 uuid 를 `discarded` 로 **합성하라**는 지침이 있다. `cancelled` 에 무턱대고 재전송하지 말라는 경고도 있다. 그러니 설계는 **「전달됨(started)」 · 「처리된 턴 끝남(completed)」 · 「답이 나옴」**을 따로 다루고, 종결이 안 오는 경우의 복구·재전송·중복 방지 규칙을 가져야 한다. — 가능성 높음
- `--replay-user-messages` 되울림이 **쓸 때** 나오나 **소비될 때** 나오나는 미측정(축소 코드 추론으로는 소비 시점). — 불확실
- **우리 코드 오늘:** 입력은 곧장 stdin(턴 관문 없음) · 쓰는 즉시 합성 에코를 **보낸 자리**에 정식 모양으로 띄움 · `command_lifecycle` 은 디코더가 모르는 타입이라 버림 · 누적기는 uuid 당 첫 항목을 남겨 자리가 보낸 곳에 고정된다.

**codex (app-server 채팅 모드)**
- `turn/steer{threadId, clientUserMessageId?, input, expectedTurnId(필수)}` — 활성 턴이 없거나 끝났으면 −32600 「no active turn to steer」, id 불일치면 오류, 리뷰·압축 턴은 steer 불가. 0.148.0↑ 에선 **진행 중 `turn/start` 도 steer 로 처리**되어 같은 턴 id 를 돌려준다(`start_or_steer_turn` → `Steered`) — 이 경로면 「턴이 막 끝났는데 steer」 경합이 코어 안에서 원자적으로 풀린다. — 가능성 높음(소스 `v2/turn.rs:277-299` · `turn_processor.rs:636-668` 대조 · 0.148.0 하한은 수집자 태그 대조만)
- 끼워 넣은 입력은 현재 샘플링과 그 도구 호출이 끝난 뒤 소비되고, **소비되는 순간** `clientId` 를 단 `userMessage`(`item/started`·`item/completed`)가 나온다. — 가능성 높음(수집자 코드 독해 + 벤더 테스트)
- **끊기(`turn/interrupt`) 시 소비 전 대기 입력은 에코 없이 버려진다**(`clear_pending`). — 가능성 높음(`tasks/mod.rs:520-534` 대조 — 함수 본문은 미독)
- **우리 코드 오늘:** 턴 진행 중 입력은 우리 큐에 붙잡아 두었다가 턴이 끝나면 새 `turn/start` 로 보냄 · `clientUserMessageId` 안 실음 · 합성 에코 없음(ADR-0198 결정·미구현).

## 3. 제약 (engram — `CLAUDE.md` · ADR)

| 제약 | 출처 | 설계에 주는 요구 |
|---|---|---|
| LLM-우선 제어 · 프론트는 렌더링만 | CLAUDE.md 「LLM-우선 제어」 | 대기/정식 상태는 **백엔드 링 이벤트**로 흘러야 한다 — 프론트만의 낙관 상태는 두 번째 제어 경로 |
| 백엔드 확장 | CLAUDE.md 「백엔드 확장」 · ADR-0004 | `command_lifecycle`·`clientId`·steer 지식은 `backend/claude`·`backend/codex` 안에만 |
| replay→live · seq dedup | 핵심 불변식 | 재접속 재생 뒤에도 대기/정식 상태가 같게 수렴해야 한다 |
| 큐 해제 판정은 각 통로가 | ADR-0193 | 에코를 「턴이 돈다」 근거로 쓰지 않는다 · steer 는 「한 번에 한 턴 시작」 관문을 다시 연다 |
| 합성 에코 + `clientId` 병합 · 버전 하한 0.140.0 | ADR-0198 | 결정 그대로 쓰되 「어디에·어떤 모양으로」가 이 조사의 몫 |
| 하이드레이션 중 입력 순서 | ADR-0198 영향 절 · ADR-0204 · ADR-0226 | 복원 중 친 글이 옛 대화 **위**에 그려지는 문제 — 「맨 아래 대기」 모양이면 자연히 풀린다(대기는 늘 꼬리) |
| claude 는 기본값 기준 | ADR-0192 | 두 백엔드가 같은 모양이어야 한다 |

## 4. 선택지 (제약 적합도)

A 와 C 는 **같은 원칙의 두 표시 위치**다 — 둘 다 「대기는 따로, 정식 위치는 전달 시점」이고 갈리는 것은 대기분을 어디에 두느냐뿐이다. B 만 원칙이 다르다.

| | A. 대화 맨 아래 대기 행 (추천) | B. 보낸 자리에 정식 고정 | C. 입력창 위 대기 목록 |
|---|---|---|---|
| 모양 | 보내면 즉시 **대화 맨 아래(흐르는 답 아래)에 회색 「대기」** · 전달되면 정식 | 보내면 즉시 **보낸 자리에 정식 모양**(ADR-0198 원안 · claude 오늘 동작) | 대화엔 안 넣고 **입력창 위 목록** · 전달되면 대화에 등장 |
| 피어 | t3code(타임라인 끝) · Claude Code 2.1.275↑(대화 안 스피너 위) · OpenCode(배지) | Orca | Codex TUI · Claude Code 문서 서술 · Cline · Roo · Zed · Open WebUI · vibe-kanban |
| 순서 정확성 | ○ 정식 위치 = 전달 시점 | ✕ 턴 중이면 앞 답의 나머지가 그 아래로 이어진다 | ○ |
| 「바로 뿌린다」 체감 | ○ 대화 흐름 안에 즉시 | ○ | △ 대화 밖(입력창 바로 위) |
| 행 이동 | 대기 행이 늘 꼬리라 정식이 돼도 자리가 거의 안 바뀐다 | 없음 | 목록에서 사라지고 대화에 새로 나타난다 |
| 하이드레이션 순서 문제 | ○ 대기는 늘 꼬리라 옛 대화 위로 안 간다 | ✕ ADR-0198 이 미결로 남긴 그대로 | ○ |
| 끊기·실패 처리 | 전달 안 된 대기분을 입력창으로 복원(Codex TUI·t3code) 또는 실패 표시 | 이미 정식처럼 보여 「안 간 말」이 남는다 | 목록에서 복원 |
| 구현 비용 | 큼 — 대기/전달 상태를 링 이벤트로 · claude `command_lifecycle` 해독 · 누적기의 「첫 항목 고정」을 「전달 시점 배치」로 | 작음(codex 만 ADR-0198 그대로) | A 와 같은 상태 기계 + 입력창 위 표면 하나 |
| LLM-우선 제어 | ○ 링 이벤트면 | ○ | ○ 같은 링 이벤트를 다른 자리에 그릴 뿐(초안의 △ 는 근거가 없었다 — §8 지적 4) |

**두 백엔드 공통 상태 기계(A·C 어느 쪽이든 필요):** 대기(보냈으나 전달 전) → 전달됨(claude `started` · codex `clientId` 단 `userMessage` · 또는 우리가 큐에서 꺼내 보낸 시점) → 정식. 종결이 안 오는 경우(claude 예외 종료·`discarded`/`cancelled` · codex 끊기로 대기 입력 소실)는 **입력창 복원 또는 「안 보내짐」 표시**로 끝낸다 — 자동 재전송은 하지 않는다(claude 설명문의 경고 · codex `clientUserMessageId` 는 멱등 키가 아니다 — ADR-0198).

**함께 정할 것(묶임):** 전달 방식 — ① 두 백엔드 모두 steer(다음 도구 경계에 끼워 넣기 · claude 는 이미 CLI 가 그렇게 한다 · codex 는 진행 중 `turn/start` 또는 `turn/steer`) ② codex 만 턴 끝까지 대기(오늘). ①이면 A 의 「받는 순간」이 도구 경계마다 오고, ②면 턴 끝에 온다. 끊기 전용 동작(Codex Esc · Claude Ctrl+Enter)은 별건.

## 5. 거부 후보 (→ ADR 거부 대안 후보)

- **보내기 막기(스트리밍 중 비활성)** — 옛 기본이지만 현재 에이전트 도구 관행과 반대이고, claude 경로는 이미 허용 중(ADR-0044 메커니즘 A)이라 퇴행이다.
- **내용 비교로 합치기** — ADR-0198 이 이미 기각. 두 백엔드 모두 id 가 있다(claude uuid · codex clientId). Orca 가 내용 비교를 쓰는 것은 PTY 라 id 가 없어서다.
- **대기분을 한 프롬프트로 이어 붙여 보내기(Open WebUI)** — 두 백엔드 모두 메시지별 id 로 합치므로 이어 붙이면 id 가 하나로 뭉개져 합치기가 깨진다.

## 6. 쟁점·한계

- Claude Code 의 대기 항목이 정식이 될 때 **어디에** 놓이는지(전달 자리 vs 보낸 자리)는 changelog 문구 추론이다 — 닫힌 소스.
- claude `command_lifecycle` 은 「@internal」 표식이다 — 안정 계약인지 모른다. 최소 버전도 모른다.
- claude 되울림(`isReplay`)의 시점(쓸 때/소비할 때) 미측정 — 실측 한 번이면 가려진다(도구를 쓰는 긴 턴 중에 입력).
- codex 0.140–0.145 에서 진행 중 `turn/start` 의 동작 · `turn/steer` 도입 버전 · 재개 이력에서 `clientId` 가 살아남는지 — 미확인.
- 사람 메신저의 실패 표시·순서 규칙은 공식 출처가 없다.
- 조사하지 못한 피어: Claude 데스크톱 앱 · Conductor · Warp · Continue · Sculptor · Kilo Code.

## 7. grounding 결과 (메인 대조)

| 클레임 | 판정 | 대조한 것 |
|---|---|---|
| Codex TUI 는 턴 중 steer 를 보낼 때 이력에 안 넣는다 | 지지 | `input_submission.rs:177`@v0.154.0 |
| Codex TUI 는 대기열 맨 앞과 `clientId` 로 합친다 | 지지 | `chatwidget.rs:1287-1336`@v0.154.0 |
| `TurnSteerParams` 에 `clientUserMessageId`·필수 `expectedTurnId` | 지지 | `v2/turn.rs:277-299`@v0.154.0 |
| 진행 중 `turn/start` 가 steer 로 처리된다 | 지지(0.154.0) · 하한 0.148 은 미대조 | `turn_processor.rs:636-668` |
| 끊기 시 대기 입력을 버린다 | 부분 지지 | `tasks/mod.rs:520-534`(abort 경로의 `clear_pending` 호출 — 본문 미독) |
| claude 가 턴 중 입력을 진행 중 턴에 접어 넣고 lifecycle 을 낸다 | 지지 | 설치본 2.1.280 바이너리 문자열 원문 |
| Claude Code: 회색 대기 · 도구 호출 끝나면 같은 턴에 전달 | 지지 | 공식 문서 interactive-mode 원문 |
| t3code: 타임라인 끝 대기 행 · 서버에 id 가 생기면 낙관 행 제거 | **부분 지지로 강등**(리뷰 반증 · 메인 재대조) — 서버 id 대조가 지우는 것은 대기 행이 아니라 **보낼 때 새로 만든 낙관 행**이다 | `MessagesTimeline.tsx:1770-1772` · `ChatView.tsx:5769-5795` · 재대조 `queuedMessageStore.ts:84-106` · `ChatView.tsx:7785-7803` |
| paseo: 기본 steer · `clientMessageId` 로 키잉 | 부분 지지 | `storage.ts:121,194` · `stream.ts:89-119`(병합 함수 본문 미독) |
| Open WebUI: 입력창 바로 위 · 끝나면 이어 붙여 한 번에 | 지지 | 공식 문서 원문 |

## 8. 적대 리뷰 결과

- **리뷰어:** codex(cross-family) · effort high · 웹 검색 켬 · 레벨 2(검산) · 2026-09-25
- **판정: BLOCK** — 결론 하나(§0.2 「받는 순간 정식」과 그 확신도)가 근거를 넘었다. 지적 4건 모두 반증 출처를 달았고 **전부 수용해 본문을 정정했다**(contested 없음).

| # | 지적 | 유형 | 심각도 | 처리 |
|---|---|---|---|---|
| 1 | t3code 는 대기 행을 **보낼 때** 지우고 새 id 의 낙관 행을 만든다 — 인용한 서버 id 대조는 그 낙관 행을 지우는 코드다. 「받는 순간 정식」의 근거가 못 된다 | 오귀속·과장 | high | 메인이 `queuedMessageStore.ts:84-106`·`ChatView.tsx:7785-7803` 재대조로 확인 → §0.2·§2-1·§7 정정 |
| 2 | Claude Code 문서는 회색 표시만 말하고, 응답 시작 뒤 **어디에 놓이는지는 말하지 않는다** — 「확실」은 과장 | 과장·논리 공백 | high | §0.2·§2-2 확신도 강등, A 의 피어 근거 재계산(Codex TUI·Claude Code 문서 서술은 입력창 위 → C 쪽) |
| 3 | `command_lifecycle` 의 `started`/`completed` 는 「받음」이 아니다 — 예외 종료 시 종결이 안 올 수 있고 `discarded` 합성 지침이 있다. 설계에 복구·재전송·중복 방지 규칙이 없다 | 근거 없음·누락 | high | §2-4 에 상태 뜻 명시, §4 에 공통 상태 기계와 「자동 재전송 안 함」 추가 |
| 4 | C 의 LLM-우선 제어 △ 는 근거가 없다 — 같은 링 이벤트를 입력창 위에 그리면 된다 | 논리 공백 | medium | §4 표 정정, A·C 를 가시성·행 이동·비용으로 비교 |

- **grounding 스팟 재검증(리뷰어):** Codex TUI 턴 중 이력 미삽입 · 맨 앞 `clientId` 대조 · `TurnSteerParams` 필드 · Claude Code 회색/같은 턴 전달 · Open WebUI — 지지. t3code 행 — 불지지(위 1).
- **정정 뒤 남는 결론:** 「대기는 따로 보이고, 정식 위치는 보낸 시점이 아니라 전달 시점」은 Codex TUI·t3code 두 소스로 선다. A(대화 맨 아래)와 C(입력창 위)는 그 원칙의 표시 위치 선택이고, B(보낸 자리 고정)만 원칙이 다르다. **재리뷰는 돌리지 않았다**(medium 단일 패스) — 정정분은 리뷰어가 본 적 없는 표면이다.
