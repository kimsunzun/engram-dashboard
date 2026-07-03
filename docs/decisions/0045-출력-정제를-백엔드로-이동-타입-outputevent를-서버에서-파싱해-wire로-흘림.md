# ADR-0045: 출력 정제를 백엔드로 이동 — 타입 OutputEvent를 서버에서 파싱해 wire로 흘림

- 상태: 확정 (2026-07-03, 근거: §5 손발/두뇌 분리 + ADR-0002 출력종류 비가정 + 가상 터미널 요구 + 출력 파이프라인 seam 코드 실측(재-review 3회) + 사용자 결정)
- 관련: Amends ADR-0044 (통로 무정제·프론트 파싱 → 백엔드 서버 정제(타입 OutputEvent)로 전환) · ADR-0002(출력 종류 비가정·터미널 강제 금지 — 이 설계가 그 실현) · ADR-0003(OutputSink/OutputFrame 계약·코어 격리 — payload 확장 대상) · ADR-0004(claude 지식 격리) · ADR-0006(락 순서) · ADR-0030 · ADR-0040(서버 authoritative bounded 버퍼) · ADR-0043(mount-replay)

## 용어 (자립용)
- **`OutputEvent`** — pump/core 경계의 확장 enum. `TerminalBytes(Vec<u8>)` + 신규 구조화 variant(`TextDelta`/`ToolCall`/`Usage`/`MessageDone`/`Error`, optional `turn_id`/`message_id`, + `Structured{kind,json}` 탈출구).
- **decoder** — backend가 소유하는 변환기(버퍼 아님). 바이트→라인 재조립→claude JSON 파싱→`OutputEvent`. claude 스키마 지식은 여기만(ADR-0004).
- **`Ring<StoredOutput>`** — seq·bounded(byte budget)·cursor replay를 담는 단일 히스토리 버퍼. `StoredOutput{ seq, payload, cost_bytes }`. pump 입력 이벤트와 저장 타입을 분리.
- **`OutputPayload<'a>{ Bytes(&[u8]) | Event(&OutputEvent) }`** — 코어 도메인 payload(빌림). **`WirePayload`** — sink/daemon 인코딩 단계 이름(코어에 wire 개념 안 샘).
- **frame tag** — wire 물리 표현. `tag0`=TerminalBytes, `tag1`=StructuredEvent(payload = self-describing 직렬화 이벤트). codec은 이벤트 스키마를 모른다(payload opaque).

## 맥락
ADR-0044가 JSON 모드를 "통로 무정제(바보 파이프) + 프론트 파싱"으로 배선했다. NDJSON이 `OutputEvent::TerminalBytes`로 흘러 프론트 `RichSlot`이 유일 파서다.

문제(코드 실측·재-review 3회 확정):
- **§5 위반** — 구조 이해가 웹뷰에만 존재, 백엔드측 LLM(두뇌)이 자기 출력을 못 봄.
- **바이트 평탄화** — 다형성은 `OutputEvent`(pump/core)에만, sink 아래는 바이트 전용(`OutputFrame{data:&[u8]}`·`OutputSink`·replay 바이트).
- **가상 터미널** — VT 바이트 스트림(터미널)과 discrete 이벤트(구조화 LLM)는 본질이 다른 payload. 단일 구조화 버퍼로 통일하면 터미널 강제 왜곡(ADR-0002 위반).
- **클라 relay가 tag를 검증·거부** — `connection.rs`가 버퍼 저장 전 `decode_frame`을 부르고, `codec.rs`가 `tag≠0`을 `UnknownTag`로 거부한다. 그래서 새 tag를 그냥 흘리면 클라 버퍼에 저장조차 안 돼 리로드 두절 → 구 B 재발. wire·codec 선택이 여기 걸린다(↓ 결정).

## 결정
**payload 다형성을 유지하되, 백엔드가 decoder로 정제하고 타입 이벤트를 binary frame(단일 `tag1`)으로 흘려 클라 공유버퍼의 append/replay 로직을 재사용**한다.

1. **backend 정제 = decoder 레이어(구현 갈림길 = B, 사용자 결정)** — `transport`(pump)는 **바이트 그대로 emit**(순수 파이프 유지, `PtyTransport`와 균일). pump→core 사이 **backend 소유 decoder**가 라인 재조립 + claude 파싱 → `OutputEvent`. transport·core는 claude/라인 지식 없음(ADR-0003/0004). (거부: transport pump가 파싱까지 소유 = transport 책임 불균일. 근거 = decoder만 backend별 교체, transport는 하나 공유.)
2. **단일 `Ring<StoredOutput>` 버퍼(사용자 결정)** — 에이전트당 히스토리 버퍼 1개. 터미널=바이트 payload, JSON=이벤트 payload. **원본/변환본 이중 보관 없음.** 저장 타입(`StoredOutput`)을 pump 이벤트(`OutputEvent`)와 분리. eviction = **byte budget(인코딩 payload 크기) + 이벤트 건수 상한**(큰 `args_json`이 건수 1로 새지 않게).
3. **wire = frame `tag0`/`tag1`(사용자 결정)** — 타입 이벤트를 variant별이 아니라 **`tag1 StructuredEvent` 하나**로(payload self-describing). `codec.decode_frame`을 **known tag는 헤더만 파싱(payload opaque)** 하도록 넓히면 → **클라 공유버퍼 append/replay 로직 재사용**(그 로직 무변경). codex/gemini·API도 같은 seam.
4. **core↔wire 변환 = daemon adapter(ADR-0003)** — 변환을 protocol crate가 아니라 daemon에 둔다(core는 `OutputEvent`만, protocol은 wire 타입만, daemon이 결합). `WsOutputSink`가 `WirePayload`를 frame으로 인코딩.
5. **프론트** — `wsFrame.decodeOutputFrame`이 tag로 분기(`tag0`→`TerminalSlot` xterm / `tag1`→`RichSlot` 타입 렌더). `RichSlot` NDJSON 파서 제거.

## 거부한 대안
- **프론트 파싱 유지(ADR-0044 MVP·현행)** — 구조가 웹뷰에 갇혀 두뇌가 못 봄(§5). → *Amends 대상.*
- **타입을 JSON `AgentEvent::Output`(Text arm, frame tag 없이)** — 클라 공유버퍼가 frame 경로(Binary arm)에만 붙어 있어 Text arm은 seq/cursor/replay를 새로 신설해야 함(더 비쌈 + 리로드 두절 위험). frame tag가 append/replay 재사용으로 더 쌈.
- **frame tag를 variant별(`tag1`=TextDelta, `tag2`=ToolCall…)로** — codec이 이벤트 스키마 변화마다 바뀜. `tag1 StructuredEvent` 하나 + self-describing payload가 codec을 스키마-무지로 유지(교체성↑).
- **단일 구조화(JSON) 버퍼로 통일** — 가상 터미널 raw VT를 discrete 이벤트로 강제 = 손실 + ADR-0002 위반.
- **두 버퍼 클래스(`TerminalBuffer`/`StructuredBuffer` + trait)** — 에이전트당 버퍼 1개뿐이라 dyn/trait이 락 규율(ADR-0006)을 숨길 위험만 늘림. 단일 `Ring<StoredOutput>` + `cost()`가 eviction 차이 흡수.
- **바이트 + typed 사이드카(병렬 채널)** / **backend-only typed view** — 각각 replay 이중부담 / 프론트 재파싱 잔존(§5 "프론트 순수 렌더" 미달).
- **데몬 재요청/재-hydration(ADR-0040 기각분)** — 무관, 여전히 거부.

## 근거
- **§5 손발/두뇌 분리(불변)** · **ADR-0002**(다형성 실현·터미널 강제 금지) · **가상 터미널 요구**(바이트 payload 유지 강제).
- **클라 relay가 tag를 저장 전 검증(재-review 코드검증)** — 그래서 "클라 완전 무변경"은 거짓. 정정: `codec.decode_frame` tag 게이트를 넓히면 **append/replay 로직만 재사용**(무변경), codec 게이트·프론트 decode는 변경면. 그래도 JSON 경로보다 쌈(JSON은 replay 신설).
- **사용자 결정(2026-07-03)** — 서버 정제 + payload 각각 추상화(파이프 공유, decoder만 backend별) + ①B/②단일ring/③tag1 확정. §0(저위험 seam·장기 → 지금 제대로).

## 영향 / 불변식
- **변경면(정직):** ① `OutputEvent` 구조화 variant + `Serialize` ② decoder(backend/claude.rs, 라인재조립+파싱) ③ `Ring<StoredOutput>` 버퍼 일반화(session/output_core) ④ `codec` `tag1` + decode_frame 헤더-only 게이트 넓힘 + golden ⑤ daemon adapter(core→wire 변환) + `WsOutputSink` frame 인코딩 ⑥ `OutputSink`/`OutputFrame`→`OutputPayload`(ADR-0003, Copy/빌림) ⑦ 프론트 `wsFrame`(tag 분기) + `protocolClient`/`agentClient`(union) + `RichSlot`(파서 제거). **재사용(무변경): 클라 공유버퍼 append/replay·mount-replay 로직**(decode 게이트만 넓힘).
- **ADR-0006 락** — 버퍼 일반화가 emit "replay lock 짧게→해제→lock 밖 send" 규율 유지(단일 ring은 기존 구조 그대로, 새 락 0).
- **ADR-0003 격리** — core `OutputEvent`≠protocol wire 타입, 변환은 daemon adapter. `OutputPayload`는 코어 도메인 타입(빌림), wire 인코딩은 sink. core tauri-import 0 유지.
- **ADR-0004** — claude 스키마 지식 = decoder(backend/claude.rs)만.
- **출력종류 = 세션 수명 내 불변** — spawn 시 capability로 고정, 전환=epoch 교체. 런타임 스위칭 금지.
- **교체성** — `Structured{kind,json}` 탈출구 + optional `turn_id`/`message_id`로 codex/gemini·API 이벤트 모델 누수 흡수.
- **MVP** — 파이프라인 축(decoder·tag1·버퍼·decode = TextDelta 흘리기)=MUST / 렌더 축(TextDelta 렌더 MUST, ToolCall·Usage·MessageDone·Error 렌더·권한 UX DEFER).
- **범위 밖** — 원 "B"(터미널 바이트 경로 리로드 두절)는 선재 버그, 별도 후속. 타입 경로는 append/replay 재사용이라 재도입 안 함.
- **ADR-0044 부분폐기 경계** — 0044의 "통로 무정제/프론트 파싱/typed 거부"만 개정. "통로 무정제"는 transport 층 한정으로 재정의(OutputCore/버퍼/codec/데몬은 payload 다형). StdioTransport·지속프로세스·입력 wrapping 격리는 유효. (0044 §31/§34 인라인 포인터 박음.)
