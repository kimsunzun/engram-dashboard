# ADR-0044: JSON 모드 배선 — StdioTransport 신설 + 바이트 통로 공용 + 지속 프로세스

- 상태: 확정 (2026-07-02, 근거: claude CLI 실측 스파이크 + 백엔드 seam 매핑 + 사용자 승인)
- 관련: ADR-0002(출력 종류 비가정·capability 렌더러 분기) · ADR-0004(claude 지식 격리) · ADR-0030(transport ⊕ backend caps 합성) · `src/lab/richslot/`(렌더 스파이크) · step-log S? (JSON 렌더 착수) · Amended by ADR-0045 (통로 무정제·프론트 파싱 → 백엔드 서버 정제(타입 OutputEvent)로 전환)

## 맥락
대시보드가 claude 출력을 구조화(JSON)로 렌더하는 모드(RichSlot)를 붙여야 한다. 실측 결과 `--output-format stream-json`·`--input-format stream-json`은 **`-p`(print/헤드리스) 전용**이다(claude 2.1.170 `--help` 명시: "only works with --print"). 즉 현행 PTY 대화형 claude는 JSON을 낼 수 없고, "터미널 렌더러를 JSON 렌더러로 스왑"이 아니라 **프로세스 기동 방식 자체가 다른 별도 경로**가 필요하다. 문제: 이 경로를 기존 파이프라인(OutputCore→codec→데몬→프론트)과 어떻게 공존시키나.

## 결정
**"모드 = transport 교체, 통로는 끝까지 무정제(바보 파이프)"** 전략.

1. **StdioTransport 신설** — PTY 없는 자식 프로세스(stdin/stdout 파이프)로 `AgentTransport` trait을 구현하는 새 variant. 터미널 모드는 기존 `PtyTransport` 그대로. 같은 `AgentSession` 조립에서 transport만 갈아끼운다.
2. **통로 공용(변경 0)** — JSON 출력(NDJSON)은 이미 직렬화된 바이트이므로 와이어에서 특별 취급하지 않는다. `OutputCore`(구독·replay·seq·finalize)·wire codec(`OutputChunk`=seq+bytes, 기존 frame tag)·데몬 `WsOutputSink`·프론트 `transport.ts`/`protocolClient` 전부 그대로 재사용. replay도 그대로 동작(바이트 히스토리 전체 파싱 = 대화 복원).
3. **분기는 양 끝 3지점만** — ① 조립: backend가 mode 보고 `CommandSpec` 인자 구성(`-p --input-format stream-json --output-format stream-json --session-id <sid>`) + `AgentManager`가 StdioTransport 선택 ② capability: transport ⊕ backend 합성으로 output=structured 신고(ADR-0030) ③ 프론트: 슬롯이 caps 보고 xterm(TerminalSlot) vs RichSlot 분기, RichSlot이 바이트→라인→파싱.
4. **멀티턴 = 지속 프로세스(Mechanism A)** — `claude -p` 프로세스 하나를 세션 수명 동안 유지, 유저 턴을 `{"type":"user",...}` JSON 라인으로 stdin에 주입. JSON wrapping은 `backend/claude.rs`에만(ADR-0004 claude 지식 격리) — 프론트·통로는 텍스트/바이트만 안다.
5. **`--replay-user-messages` 사용** — claude가 유저 턴을 출력 스트림에 되울림 → 프론트는 낙관적 로컬 상태 없이 출력 스트림 단일 출처로 유저+어시스턴트 렌더.
6. **MVP 범위 = 텍스트 입출력만** — 백엔드는 JSON을 엿보지 않는다(상태 = 프로세스 생사만). 도구 권한 승인 UI·partial 델타 스트리밍·resume 복원 정합·interrupt(stdio엔 PTY Ctrl-C 없음 — 방법 미확인)는 후속.

## 거부한 대안
- **PTY 대화형에서 JSON 출력(렌더러만 스왑)** — 불가능. `stream-json`이 `-p` 전용임이 실측으로 확정(claude `--help` 명시). 이 ADR의 출발점.
- **턴별 재기동(Mechanism B, `claude -p --resume <sid>` 매 턴)** — 매 턴 프로세스 시동비용 + 처리 중 개입(guidance 주입) 불가. 기존 통제-sid/resume 인프라(ADR-0008) 재사용 이점은 있으나 대화형 UX에 부자연. A의 미확인 지점(도구 권한 wire)은 MVP가 텍스트 챗만이라 안 밟는다.
- **와이어에 typed 구조화 variant 신설(새 frame tag + OutputEvent/OutputChunk/codec/데몬/프론트 확장)** — 백엔드 spawn→프론트 렌더까지 seam 10곳을 전부 손대야 하는데 MVP 이득 0(내용 해석은 어차피 프론트 몫). 백엔드가 제어 이벤트(권한 요청 등)를 타입으로 다뤄야 하는 시점에 재검토 — 그때도 transport pump에 라인 tap을 넣으면 와이어는 불변으로 갈 수 있다.

## 근거
- **실측 스파이크(2026-07-02):** claude 2.1.170 `--help` — `--output-format`/`--input-format` 둘 다 "only works with --print" 명시. `--input-format stream-json` = 지속 프로세스에 stdin JSON 라인으로 멀티턴(공식 headless docs: "without relaunching the claude binary"). `--replay-user-messages` = 입력 되울림 플래그 실존.
- **백엔드 seam 매핑:** 통로를 바보로 유지하면 변경이 조립(SEAM 1)·caps(SEAM 5)·렌더러 분기(SEAM 10) + StdioTransport 신설로 축소 — typed wire안(SEAM 1~10 전부)과 대비.
- **랩 검증:** `src/lab/richslot/` 파서(`parse.ts`)·5레이아웃이 실측 fixture(stream-json 캡처)로 이미 동작 — 프론트가 파싱 소유 가능함을 입증.
- 사용자 승인: "일단 이 배선으로 ㄱ" (2026-07-02 세션).

## 영향 / 불변식
- **통로 무정제 불변** — `OutputCore`·codec·데몬·프론트 transport는 JSON 모드에서도 바이트 내용을 해석하지 않는다. JSON 지식은 만드는 쪽(claude)·감싸는 쪽(`backend/claude.rs`)·그리는 쪽(RichSlot 파서)에만 존재. 어기면(통로에 JSON 파싱 삽입) swappable 원칙(ADR-0002)과 seam 격리가 깨진다. **(→ ADR-0045로 개정: '통로 무정제'는 이제 transport 층에만 한한다. OutputCore/버퍼/codec/데몬은 payload-generic으로 타입 `OutputEvent`를 나르고, 파싱은 `backend/claude.rs`가 소유(프론트 RichSlot 파서는 제거). swappable 원칙은 payload 다형성으로 유지.)**
- **입력 wrapping = backend 단독** — `{"type":"user",...}` 스키마가 `backend/claude.rs` 밖으로 새면 ADR-0004 위반.
- **StdioTransport caps** — resize 불가·terminal_bytes 아님을 정직 신고(터미널 개념 없음). 프론트 렌더러 분기는 이 caps가 유일한 판단 근거(ADR-0002).
- **MVP 한계 명시** — interrupt 미지원(kill만)·권한 승인 없음(텍스트 챗 전용)은 *의도된 미구현*이다. 후속 작업이 이를 "결함"으로 오독해 통로에 땜질하지 말 것 — 확장 시 pump 라인 tap + 필요 시 typed variant 재검토 경로를 따른다. **(→ ADR-0045가 이 확장 경로를 실행: backend 파싱 + typed variant를 binary frame tag로 wire. "통로 땜질 금지"는 transport 층 한정으로 유효 — 정제는 backend가, wire는 codec tag가 담당.)**
