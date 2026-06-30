# Study Note — 2계층 중계 모델의 멀티뷰 출력 버퍼·진행도 (2026-06-30, medium)

## 쟁점 (왜 조사했나)
직전 light 노트(20260629)가 tmux/Zellij 같은 **1계층** 터미널 멀티플렉서에 갇혀 "모델이 다르다"는 사용자 지적을 받았다. 우리 모델 = **2계층 중계**: 백엔드 데몬이 장수명 세션 호스팅 → 네이티브 클라(src-tauri)가 단일 연결로 **중계 허브** → 그 클라가 같은 세션을 **여러 뷰**에 동시 표시. 이 모델을 쓰는 시스템이 멀티뷰 출력 버퍼·진행도를 어떻게 두는지(특히 "중계 허브가 뷰별 진행도·버퍼를 갖나")를 cross-family로 조사.

## 방법 (medium)
Claude(Sonnet) 3갈래 병렬 BLIND — ① 에디터/데몬(Neovim remote UI·Emacs daemon·VS Code) ② 세션 프로토콜(Jupyter·DAP·LSP·CDP) ③ broadcast/메시징(Kafka·Redis Streams·Yjs/Automerge·Matrix/Slack·Signal·Docker logs). + Codex 1명 전 시스템 독립 BLIND 교차. opus(메인) 교차 대조 + 적대검증.

## 4질문
① 콘텐츠 원본을 서버에 두나 중계 허브에 캐시하나 · ② 뷰별 진행도(렌더 위치) 소유자(서버/중계/뷰) · ③ 새 뷰 늦게 attach 시 채우는 법 · ④ 전송 무손실 시퀀스 ↔ 뷰별 렌더 분리 패턴.

## 시스템별 결론 (출처·확신도)

| 시스템 | ① 콘텐츠 원본 | ② 진행도 소유 | ③ 새 뷰 채우기 | ④ 시퀀스/렌더 분리 | 중계 허브가 뷰별 상태? |
|---|---|---|---|---|---|
| **VS Code** pty host | **pty host**(headless xterm, XtermSerializer) | 각 renderer xterm.js | **pty host의 replay 이벤트 전송**(IPtyHostProcessReplayEvent) | 명확 분리(pty host=시퀀스, renderer=렌더, IPC) | **콘텐츠 replay는 중계(pty host) 소유, per-view 렌더는 각 뷰** — ★우리와 최근접★ |
| Neovim remote UI | 서버 ScreenGrid | 서버 전역(공통 치수로 수렴) | 서버 full resync(ui_attach) | flush 이벤트로 렌더 타이밍 분리 | No(중계 없음, UI가 서버 직접 attach) |
| Emacs daemon | daemon buffer 오브젝트 | 각 window(window-point/window-start) | 서버 buffer를 새 frame에 바인딩 | 스트림 아님(오브젝트 공유) | N/A(중계 없음, per-window는 서버 window 오브젝트) |
| Jupyter 커널 | 커널(iopub broadcast) | 없음(per-client cursor 없음) | broadcast-only — 과거 못 받음, history_request는 partial | msg_id/parent_header | No(frontend/document가 뷰 상태) |
| DAP | 어댑터 | 없음(매번 현재 상태 조회) | replay 없음 | seq=ordering | No(세션마다 독립 연결) |
| LSP | 서버 | 없음 | one-server-one-tool 전제 | 문서 버전 | No(다중 클라 미전제) |
| CDP | 브라우저 타겟 | per-sessionId 독립 | replay 없음(enable 이후만) | sessionId 다중화 | No(타겟+각 프론트) |
| Kafka | 브로커 | **브로커**(__consumer_offsets) | 마지막 committed offset/reset 정책 | 완전 분리(log offset ≠ committed offset) | 서버 소유 |
| Redis Streams | 서버 | **서버**(PEL) | XGROUP `$`=지금/`0`=처음 | 완전 분리(entry ID ≠ PEL) | 서버 소유 |
| Yjs/Automerge | 분산 복제 CRDT | 각 클라(awareness/state vector) | state vector diff sync | 분리(update ≠ awareness) | 클라 소유 |
| Matrix/Slack | 홈서버/서버 | **서버**(read receipt, per-user) | /sync since 토큰 / history cursor | 분리(event ID ≠ read cursor) | 서버 소유 |
| Signal | 디바이스 로컬 | 각 디바이스 | history replay 없음 | 분리 | 클라 소유 |
| Docker logs | daemon log driver | 없음(stateless) | 매 요청 --since/--tail | 개념 없음 | stateless |

## 교차검증 (Claude 3갈래 ↔ Codex)
- **수렴(확실):** (a) 콘텐츠 원본은 **중앙 단일 보관**(뷰가 복제 안 함) — 전 시스템 일치. (b) per-view 진행도(렌더 위치)는 **뷰별 독립**이거나 아예 없음 — "단일 consumed cursor 없음"이 표준. (c) **전송 시퀀스 ≠ 소비 진행도 분리**가 지배적(Kafka/Redis/Matrix/Slack 명확). Codex도 동일 결론.
- **②진행도 소유 분포(발산이 아니라 도메인별 차이):** 서버 소유=Kafka/Redis/Matrix/Slack(영속 스트림/브로커) · 뷰/클라 소유=Emacs/VS Code-renderer/Yjs/Signal · 없음=Jupyter/DAP/LSP/CDP/Docker.
- **만장일치 경계:** 공통 편향이 아니라 공식 소스 직접 인용(소스 코드·프로토콜 spec) 기반이라 신뢰도 높음.

## 적대검증 (반증 시도)
- "jupyter_server가 신규 클라용 history 캐시 보유" 가설 → **반증**: 그 버퍼는 *일시 단절 재연결*용이지 신규 클라 history 캐시 아님(notebook PR #2871, jupyter_server issue #1274 미해결).
- "DAP multi-session = 한 세션 공유" 오해 → **반증**: 세션마다 독립 TCP 연결(DAP issue #329).
- "Matrix read cursor가 per-device" → **반증**: per-user다(m.read.private는 알림 동기화용).

## engram 시사 (핵심)
- **2계층 중계 선례는 VS Code pty host가 사실상 유일.** 패턴 = **중계 허브가 콘텐츠 replay 버퍼를 단일 소유 + 각 뷰가 독립 렌더 상태(cursor/scroll).** 우리 src-tauri = pty host 역할.
- 브로커 계열(Kafka/Redis)은 **공유 로그 + per-consumer offset**을 서버가 보관 — "콘텐츠 한 벌 + 진행도는 구독자별 인덱스"의 정석. 진행도 소유가 서버냐 중계냐는 영속성 요구로 갈릴 뿐, **"콘텐츠 단일 + 진행도 인덱스 분리"는 보편**.
- 현재 engram 결함의 근원 = "가장 뒤처진 뷰 기준 min"을 **데몬 재구독 지점**으로 써서 새 뷰가 앞부분 유실. 위 선례는 모두 **데몬 재구독과 뷰 채우기를 분리**한다.

## 사용자 설계 아이디어 (2026-06-30) — 평가
사용자 안: **"클라(중계 허브)가 버퍼를 갖되, 멀티뷰(중첩)면 가장 큰 버퍼 하나만 보관하고 각 뷰는 인덱스만 다르게."**
- = **VS Code pty host 모델 + Kafka/Redis 공유로그-per-offset 모델의 결합** — 리서치가 검증한 표준 패턴과 정합(가능성 높음).
- 콘텐츠 단일 저장(메모리 N배 회피) + per-view 인덱스(독립 진행도) = ②의 "뷰별 독립" + ①의 "단일 보관" 동시 충족.
- **핵심 전환:** "가장 뒤처진 뷰 min"의 역할을 *데몬 재구독 지점*(현재·결함) → *클라 버퍼 보관 하한(eviction)*으로 바꾼다. 새 뷰는 데몬 재요청 없이 클라 버퍼에서 자기 인덱스로 채움 → 현재 유실 결함 해소.
- **정식 설계 때 풀 결정점:** (1) 클라 버퍼 보관 범위 = 붙은 뷰 최대 필요분, 그보다 과거는 데몬 ReplayBuffer 2-tier fallback. (2) 데몬 ReplayBuffer(전송 무손실·장기 원본) ↔ 클라 버퍼(뷰 채우기 캐시) 역할 분담. (3) 인덱스 단위=seq, 버퍼 내 위치 매핑. (4) 각 뷰 xterm scrollback(렌더 결과) ↔ 클라 버퍼(raw bytes) 관계.

## 공백·한계
- 2계층 "중계 허브" 직접 선례가 VS Code 하나(표본 적음). 나머지는 1계층이라 외삽.
- 클라 버퍼 ↔ 데몬 ReplayBuffer 2-tier 동기화·eviction의 구체 OSS 선례는 미조사(필요시 deep).
- medium이라 적대검증은 핵심 3건만(전수 아님).

## 출처 (주요)
- VS Code ptyService.ts(XtermSerializer/PersistentTerminalProcess/replay) — github.com/microsoft/vscode src/vs/platform/terminal/node/ptyService.ts
- Neovim ui.c / api-ui-events — neovim.io/doc/user/api-ui-events.html
- Emacs Window-Point/Window-Start — gnu.org/software/emacs/manual/html_node/elisp/
- Jupyter JEP-65 / messaging spec — jupyter-client.readthedocs.io/en/latest/messaging.html
- DAP overview / issue #329 — microsoft.github.io/debug-adapter-protocol
- CDP Target/Network domain — chromedevtools.github.io/devtools-protocol
- Kafka consumer design — docs.confluent.io/kafka/design/consumer-design.html
- Redis XREADGROUP/XPENDING — redis.io/docs/latest/commands/
- Yjs awareness/document-updates — docs.yjs.dev
- Matrix read receipts / Slack conversations.mark — api.slack.com/methods/conversations.mark
