# ADR-0189: codex app-server 통로는 backend 폴더 안의 AgentTransport 구현체로 둔다

- 상태: 확정 (2026-09-09, 근거: 사용자 판정 + 소유권 추적 검증 + 주석 원문 재독) · 부분 폐기 by ADR-0191 (통로의 거처는 정했으나 생성 경로를 안 정했다)
- 관련: Amends ADR-0044 (바이트 통로 공용 조항) · ADR-0004(백엔드 지식 격리) · ADR-0045(출력 정제를 백엔드로) · ADR-0030(transport ⊕ backend caps 합성) · ADR-0187(app-server 통로 확정) · ADR-0190(입력 큐) · `docs/reference/structure/session-path-ownership.md` · `docs/process/S21-codex-backend/trd-phase2a.md` §5 · step-log S21 · Amended by ADR-0191 (통로의 거처는 정했으나 생성 경로를 안 정했다)

## 맥락

codex app-server 는 **보내고 답을 받아야 다음으로 갈 수 있는** 프로토콜이다 — `initialize` · `thread/start` · `turn/start` · `turn/interrupt` 넷이 왕복이고, 특히 대화 id 는 `thread/start` 응답으로만 알 수 있다(ADR-0187). claude 는 밀어 넣고 흘러나오는 것을 받으면 끝이라 짝지을 것이 없다. **즉 codex 가 이 구조에 없는 것을 처음으로 요구하는 백엔드다.**

★**막힌 자리를 소유권 추적으로 확정했다**★(정본 = 소유권 지도):

- `AgentBackend` 층은 codex 를 아는 유일한 층인데 **세션마다 기억할 자리가 없다** — 크기 0 짜리 unit struct 하나를 전 세션이 공유하는 함수 모음이다(`backend/mod.rs:277-279`).
- `AgentSession` 층은 상태를 갖지만 **codex 를 알면 ADR-0004 위반**이다.
- 그래서 「codex 를 알면서 세션마다 상태를 갖는 것」이 놓일 자리가 없었다.

Phase 2a TRD 의 1·2 판은 이 자리를 층 이름 위에서 추측해 두 번 모두 적대 리뷰 BLOCK 을 받았다.

## 결정

**codex app-server 통로를 `crates/engram-dashboard-agent/src/backend/codex/` 안의 struct 로 만들고 `AgentTransport` 를 구현한다.**

- **공용 계약(`AgentTransport` 여섯 메서드)에 한 줄도 더하지 않는다.** 세션은 지금과 똑같이 그 여섯만 부른다.
- 그 struct 가 소유하는 것: 자식 프로세스 핸들과 Job Object · `Mutex<Option<ChildStdin>>`(단일 writer) · 자기 읽기 스레드 · 보낸 요청의 응답 대기 맵 · 아웃바운드 request id 카운터 · `thread.id` · 준비 상태.
- **인바운드 분기는 그 struct 안에서 끝난다** — 도착한 줄이 `method` 있고 `id` 있으면 서버→클라 요청(2a 실측 0 건, 오면 method-not-found 로 응답한다 — ★버리면 상대가 멈춘다★), `method` 만 있으면 notification(번역 후 `core.emit`), `id` 만 있으면 우리 요청의 응답(대기 맵에서 꺼내 깨움 — **밖으로 나가지 않는다**), JSON 파싱 실패면 로그만 남기고 죽지 않는다.
- 대화 id 는 조립점이 준 콜백으로 프로필에 기록한다 — **backend 가 `ProfileRegistry` 를 직접 부르지 않는다**(선례 = claude sid 기록 경로, `crates/engram-dashboard-daemon/src/lib.rs:284-290`).
- **통로 종류 축은 셋이 된다** — 터미널(`PtyTransport`) · 파이프(`StdioTransport`) · 파이프+양방향 JSON(이번 것). ★**백엔드 이름으로 가르지 않는다**★.

## 거부한 대안

- **공용 계약에 요청·응답을 더한다** — `AgentTransport` 에 request/response 개념을 넣는 안. 지금 두 구현체가 그 개념을 **전혀 쓰지 않는데** 계약에만 생긴다. `transport_shape` 의 doc 이 같은 모양의 오염을 이미 경고한다(`backend/mod.rs:113-123`: 한 backend 의 축이 전원의 선택을 굴리게 된다). PTY 쪽에 `Unsupported` 가 늘어난다.
- **소유권을 뒤집어 codex 부품이 통로를 소유하고 세션이 빌려 쓴다** — 쓰기 문제가 사라지지만 **미검증**이고, 죽이는 절차(ADR-0001)와 수거 경로가 세션 소유를 전제하므로 그 둘과 다시 부딪힐 위험이 남는다.
- **입력 인코더를 세션마다 상태 갖는 객체로 바꾼다** — 검증에서 기계적으로 가능하나 파장이 가장 넓었다(선언·호출 12 곳 + 다른 crate 의 실행 파일 하나 + 트립와이어 테스트). 통로 구현체가 상태를 갖는 쪽이면 **그 변경이 통째로 불필요해진다** — 구현체는 원래 세션마다 만들어지는 객체다.
- **`transport/` 폴더에 둔다** — ADR-0004 격리 위반. 그 폴더의 것들은 어느 도구에도 안 묶여 있고, 그 성질이 `OutputDecoder` 가 존재하는 이유다.

## 근거

★**결정적 근거 = 주석 원문 재독**★. `transport/mod.rs:19-23` 이 codex 를 이름으로 들어 금지하는 것처럼 읽혀 2 판 리뷰가 이 안을 「TRD 가 정할 게 아니라 별도 결정이 필요한 행위」로 막았다. 원문의 주어는 **`StdioTransport`** 다:

> transport(StdioTransport)는 **바보 파이프**라 자식 stdout 바이트가 무슨 스키마인지(claude stream-json / codex 프로토콜 / 평문) 몰라야 한다.

즉 **범용 파이프 구현체 하나**에 대한 서술이고, codex 는 스키마 예시로 나열됐을 뿐이다. 같은 취지가 `stdio.rs:7-9` 에도 있고 역시 그 파일 자신을 가리킨다. **범용 파이프는 그대로 바보로 남고, 스키마 지식은 backend 폴더에 있다 — 두 규칙이 동시에 지켜진다.** ★이 재해석은 리뷰 판정을 뒤집는 것이라 메인이 임의 확정하지 않고 사용자 판정을 받았다(2026-09-09).★

**검증이 확인한 것**(소유권 지도 + 8 문항 검증): 이 모양은 `AgentTransport` 계약 변경을 **강제하지 않는다.** 그리고 앞서 막혔던 셋이 전부 해소된다 — ① 쓰기 경로(구현체가 stdin 을 직접 소유) ② 실패 표현(`send_input` 이 이미 `Result`) ③ 취소(`interrupt()` 를 그 구현체가 구현한다. 공용 인터페이스에 이미 있는 메서드다).

**불변식 점검:** kill 인과 2 동사 · finalize 1 회 · 턴 관측 정리 두 지점 · 백엔드 이름 격리 — 전부 안 건드린다.

## 영향 / 불변식

- ★**`interrupt()` 가 처음으로 실제 동작하는 구현체가 생긴다**★ — 지금 `StdioTransport::interrupt` 는 `Unsupported` 를 돌려주고(`transport/stdio.rs:316-321`) `ControlCaps.interrupt` 는 리터럴 `false` 다(`:363-384`). 그 신고 값이 조립점 주입으로 정직해져야 한다(그 레버 결정 = TRD §10, 메인 판정 = 생성자 인자 확장).
- **`transport_shape` 에 셋째 변형이 생긴다.** 그 값을 보고 caps 를 주입하는 조립점 규칙(`manager.rs:79-99`)이 한 줄 늘고, ★그 줄이 「챗 표면에 그리나」의 답을 자동으로 정한다★(`structured: true` → 프론트가 `renderMode.ts:24` 에서 `rich` 를 고른다).
- **codex 백엔드가 `transport_shape` 를 선언해야 한다** — 지금은 기본값 `Pty` 를 물려받는다(`backend/mod.rs:126`). 선언 표 트립와이어(`backend/mod.rs:779`)를 같은 커밋에서 갱신하지 않으면 그 테스트가 빨개진다.
- **읽기 스레드 수명** — 그 struct 가 자기 스레드를 소유하므로 `shutdown()` 이 그것까지 정리한다. ★kill 인과의 2 동사 안에서 끝나야 한다 — 셋째 동사를 만들지 않는다.★
- **Phase 1 의 PTY codex 경로는 그대로 산다** — 이 결정은 통로를 **더하는** 것이고 기존 것을 걷지 않는다. 같은 백엔드가 통로 둘을 갖게 되며, 그 둘의 권한 정책이 다른 문제는 ADR-0188 이 다룬다.
- **이 결정을 다시 열 트리거** — 다른 도구가 같은 프로토콜(app-server 계열 JSON-RPC)을 쓰게 되면, 그 구현체는 특정 도구에 안 묶이므로 `transport/` 로 올린다.
