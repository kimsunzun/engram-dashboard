# 학습 노트 — 단일 세션 다중 소비자 출력 처리 패턴 (2026-06-30)

## 주제
Jupyter/DAP/LSP/CDP 4개 프로토콜에서 단일 백엔드 세션 출력을 다중 뷰/클라이언트가 어떻게 소비하는가.

## 이번 deep tier에서 다룬 핵심 쟁점

### 쟁점 1: Jupyter iopub에 per-client cursor가 있는가
- **결론 도달 과정:** JEP-65 직접 fetch로 "Welcome messages do not and cannot identify the client whose subscription is being received" 인용 → cursor 개념 자체가 없는 설계 확인. Codex도 독립 조사에서 동일 결론.
- **왜 중요한가:** 우리 engram 대시보드의 replay/fan-out 설계에서 "서버 측 per-client offset"을 두면 Jupyter 모델과 다른 선택이다. Jupyter는 broadcast 후 클라이언트가 알아서 필터링하는 모델.

### 쟁점 2: jupyter_server의 버퍼가 "신규 클라이언트용 이력 캐시"인가
- **초기 가설:** jupyter_server가 출력을 캐싱해 새 클라이언트에 replay할 수 있을 것.
- **반증:** PR #2871, issue #4105 확인 → 버퍼는 *일시단절 클라이언트가 재연결할 때*만 replay. 새 세션 ID로 연결하면 버퍼 miss. jupyter_server issue #1274는 이 갭을 "제안" 단계로만 다룸 — 현재 미구현.
- **핵심 구분:** "끊겼다 돌아온 같은 클라이언트" vs "처음 붙는 새 클라이언트" — 전자만 지원.

### 쟁점 3: DAP multi-session = 여러 클라이언트가 하나의 디버그 세션 공유?
- **혼동 포인트:** "multi-session mode"라는 이름이 여러 클라이언트가 같은 세션을 공유한다고 오해하게 만든다.
- **실제:** 하나의 어댑터 프로세스가 *여러 독립 디버그 세션*(각각 별도 TCP 연결)을 받는 것. 같은 디버기를 여러 클라이언트가 동시에 보는 건 명세 범위 밖.
- **실제 구현체:** LLDB DDS, Dart DDS가 프로토콜 바깥에서 중간 레이어로 이걸 구현. 명세는 다루지 않음.

### 쟁점 4: CDP Network.enable이 per-session인가 broadcast인가
- **공식 명세 prose에 명시 없음** → "가능성 높음"으로 처리.
- **근거:** `maxTotalBufferSize` 파라미터 설명이 "this DevTools session"으로 세션 단위 기술 + Codex 교차 합의. 확정하려면 Chromium 소스 `InspectorNetworkAgent` 확인 필요.

## deep vs medium 차이 체감
- deep에서 WebFetch로 JEP-65 원문, jupyter_server PR, DAP issue #329까지 직접 인용 확보.
- medium이었다면 "Jupyter iopub = broadcast"만 얻고 jupyter_server 버퍼의 미묘한 "재연결용/신규용" 구분을 놓쳤을 가능성 높음.
- 적대 검증에서 "jupyter_server가 신규 클라이언트용 캐시를 이미 갖는다"는 초기 가설을 반증 → deep tier 가치 실증.

## engram 설계 함의
- iopub 모델(broadcast + per-client cursor 없음)을 따르면: 서버는 단순 broadcast, 클라이언트가 자체 seq dedup.
- "뒤늦게 붙는 뷰"용 replay가 필요하면: **중간 호스트(데몬)가 replay 버퍼를 소유**하는 구조가 필요 (Jupyter 방식이 아니라 jupyter_server issue #1274 제안 방향).
- 현재 engram은 `OutputCore.replay` + subscribers lock 보유 중 replay 전송(C4) 구조로 이미 "중간 호스트가 버퍼 소유" 방향을 선택했다 → Jupyter 커널 레벨이 아닌 jupyter_server 레벨과 유사.
