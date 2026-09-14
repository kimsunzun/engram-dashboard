# ADR-0187: codex Phase 2 통로를 app-server 로 확정한다 — exec 는 글자를 흘리지 않는다

- 상태: 확정 (2026-09-09, 근거: 사용자 결정 + 세 모드 실측)
- 관련: ADR-0004(백엔드 지식 격리) · ADR-0044(입력 인코딩·출력 정제) · ADR-0185(세션 복원 불변식) · `.claude/handoff/attachments/codex-measurements-2026-09-09.md`(실측 정본) · `docs/process/S21-codex-backend/trd-phase2a.md` §10-8 · step-log S21

## 맥락

Phase 1 은 codex 를 **사람용 대화 화면**(인자 없는 `codex`)으로 PTY 에 띄워 기존 xterm 경로에 그렸다. Phase 2 는 그 출력을 우리 챗 표면에 **중립 어휘로** 그리는 것이 목표이고, 그러려면 codex 를 사람 화면이 아닌 방식으로 조종해야 한다. 조종 방법의 후보가 셋이었다 — `codex app-server`(다른 프로그램이 JSON-RPC 로 조종) · `codex exec --json`(한 번 시키고 JSON 줄을 받고 종료) · 공식 SDK.

★**이 선택을 미룬 대가가 실제로 발생했다**★ — 조사 보고서의 적대 리뷰가 「표본이 app-server 쪽으로 편향됐다」(HIGH 5)고 지적했는데도 재지 않고 Phase 2a 설계 문서를 세 판까지 고쳤다. 그 문서의 §10 은 이 항목을 **질문 순서 맨 위**로 표시하고 있었고, 다른 열다섯 항목이 여기 매달려 있었다.

## 결정

**Phase 2 의 통로는 `codex app-server` 다.** 우리 클라이언트가 그 프로세스에 줄단위 JSON-RPC 로 요청을 보내고 알림을 받는다.

- 보낼 요청은 실측 기준 **세 종**이다 — `thread/start` · `turn/start` · `turn/interrupt`.
- **서버가 우리에게 먼저 묻는 요청은 2a 범위에서 다루지 않는다** — 읽기 전용 + 승인 없음 조건에서 실측 0 건이었다(선언은 11 종). 그 조건이 바뀌면 이 면제가 죽는다.
- 대화 id 는 codex 가 발급하고 우리는 응답에서 수령한다(ADR-0185 결정 1 그대로).

## 거부한 대안

- **`codex exec --json`** — ★**글자가 흐르지 않는다(실측)**★. 도착한 이벤트가 `thread.started` · `turn.started` · `item.completed` · `turn.completed` **넷뿐**이고 메시지 본문은 완성된 채 한 번에 온다. 「1부터 30까지 세라」를 4.5 초에 죽였을 때 나온 것은 앞의 두 줄뿐이었고, 같은 시각 app-server 는 이미 조각을 10 개 보내고 있었다. 챗 표면에 얹으면 **턴이 끝날 때까지 화면이 빈다.** 부수 사실 둘: 턴 도중 끼어드는 수단이 없어 캔슬이 프로세스 강제 종료뿐이고, 그렇게 끊은 턴은 기록에 **사용자 메시지만** 남는다(어시스턴트 출력·종료 표식 없음). 그리고 승인 정책 플래그(`-a`)가 **`codex exec` 에는 아예 없다.**
- **공식 SDK** — ★**존재하지 않는다(실측)**★. `codex --help` 에서 `sdk|library|npm|typescript|python` 검색 0 건, 설치된 `@openai/codex@0.153.4` 에도 0 건. 가장 가까운 둘은 라이브러리가 아니라 코드 생성·프로토콜이다(`app-server generate-ts` · `mcp-server`).
- **Phase 1 의 PTY 대화 화면을 그대로 쓰기** — 화면이 터미널이라 챗 표면의 중립 어휘로 옮길 구조화 이벤트가 없다. Phase 1 이 이미 그 길이고, Phase 2 의 목표가 그 위에 있다.

## 근거

정본 = `.claude/handoff/attachments/codex-measurements-2026-09-09.md`(codex-cli 0.153.4 · Windows 11 · 2026-09-09). 요지:

- **app-server 는 조각을 흘린다** — 턴 중 `item/agentMessage/delta` 가 `{threadId, turnId, itemId, delta}` 로 연달아 오고, 관측된 알림이 11 종이다.
- ★**`turn/interrupt` 가 18ms 에 성공하고 대화가 살아남는다**★ — 조각이 즉시 멈추고 `turn/completed` 가 `status:"interrupted"` 로 닫히며, **같은 `threadId` 로 다음 턴이 정상 동작했다.** 즉 캔슬이 프로세스 종료를 요구하지 않는다.
- **대화 id 가 `Uuid` 로 파싱된다** — `thread.id == thread.sessionId`, UUIDv7 확인. 프로필의 기존 `Option<Uuid>` 칸에 그대로 들어간다.
- **stdout 에 비-JSON 줄이 0**, stderr 0 바이트. teardown = stdin 닫기, 종료코드 0 을 46ms 에.
- **스키마를 로컬에서 생성할 수 있다**(`generate-json-schema`, 네트워크·모델 호출 없음) — 번역 표를 추측 없이 채울 경로가 있다.

두 기각 모두 실측에 걸린다(서술만인 기각 없음).

## 영향 / 불변식

- ★**PATH 의 `codex` 는 shim 이라 프로그램에서 직접 띄우면 `ENOENT` 다**★ — 우리 코드가 이미 `cmd.exe /c` 로 감싸는 것이 같은 이유의 처리이고, 그 감싸기를 걷으면 이 통로가 죽는다(`crates/engram-dashboard-agent/src/backend/mod.rs` 의 `console_command`).
- **서버 응답은 `jsonrpc` 필드를 생략한다.** 응답·알림을 가르는 축은 방향과 모양(`method` 유무 · `result`/`error`)이고 **id 값이 아니다** — 서버 요청 id 와 우리 요청 id 는 값 공간이 겹칠 수 있다.
- **턴 id 는 `result.turn.id`** 다(`result.turnId` 가 아니다). `turn/interrupt` 는 그 값을 문자열로 요구하고 `null` 을 `-32600` 으로 거부한다.
- **중단된 메시지의 `item/completed` 는 오지 않는다** — 그 `item/started` 가 닫히지 않은 채 남으므로, 「시작한 아이템은 반드시 닫힌다」를 전제한 번역기는 깨진다.
- **선언 ≠ 관측.** 스키마는 클라 요청 155 종 · 서버 알림 81 종 · 서버 요청 11 종을 선언하지만 관측된 것은 알림 11 종뿐이다. ★**스키마에 있다는 이유로 「온다」고 쓰지 말 것**★ — 이 저장소의 출처 등급이 `[스키마]` 와 `[실측]` 을 가르는 이유가 그것이다.
- **이 결정이 여는 값** — Phase 2a 설계 문서의 §10 항목 다섯(요청·응답 계약 · 죽인 것 구별 · 정상 종료 · 인바운드 경로 · 드라이버의 프로필 접근)이 app-server 를 전제로만 성립한다. `exec` 를 골랐다면 그 다섯이 소멸했을 것이고, 그 크기 차이가 이 ADR 의 실질이다.
- **이 결정을 다시 열 트리거** — ① 승인을 실제로 받기로 하면(ADR-0188 의 값 설계) 서버 요청 11 종이 살아나 「0 건」 면제가 죽는다 ② 상주 프로세스가 대화마다 하나씩 사는 비용·인증 만료로 서버가 죽어 기록이 잘리는 사고(상류 이슈로 보고됨)가 실제로 발생하면 `exec` 쪽 단발 실행이 다시 후보가 된다.
