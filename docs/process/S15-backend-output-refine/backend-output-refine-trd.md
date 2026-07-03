# S15 TRD — 출력 정제를 백엔드로 (payload-generic 파이프라인 + tag1 frame)

> **왜/무엇 = ADR-0045**. 이 문서 = **어떻게**(seam·시그니처·볼륨·테스트·순서). 불변식 = ADR-0002/0003/0004/0006/0040/0043.
> **확정 결정(사용자):** ① 파서 = decoder 레이어(transport 바이트만, backend 소유 decoder가 재조립+파싱) · ② 단일 `Ring<StoredOutput>` · ③ wire = `tag0`바이트/`tag1`StructuredEvent(self-describing) · ④ 원 "B" 리로드 두절은 범위 밖.

## 1. 목적·범위
claude `stream-json`(NDJSON) 정제를 프론트→백엔드 decoder로 옮기고, 타입 이벤트를 `tag1` frame으로 흘려 클라 공유버퍼 append/replay(ADR-0040/0043) 로직을 재사용. 프론트는 파서 버리고 tag로 분기.
**범위 밖:** 원 "B"(터미널 바이트 경로 리로드 두절) = 선재 버그, 별도. ToolCall/Usage/MessageDone/Error 렌더·권한 UX = DEFER.

## 2. 데이터 흐름
```
claude stdout(NDJSON bytes)
   ▼  StdioTransport.pump: read → emit(bytes)         ← 순수 파이프(변경 최소)
   ▼  decoder(backend 소유): 라인재조립 + claude 파싱 → OutputEvent   ← claude 지식 여기(ADR-0004)
   ▼  OutputCore: Ring<StoredOutput>(seq·byte budget·cursor) 저장, emit lock 밖 send(ADR-0006)
   ▼  OutputPayload{Bytes|Event}(코어 도메인, 빌림)
   ▼  daemon adapter: core OutputEvent → wire 타입 변환(ADR-0003)
   ▼  WsOutputSink: WirePayload → frame(tag0=bytes / tag1=StructuredEvent 직렬화)
   ▼  codec.decode_frame: known tag 헤더-only 파싱(payload opaque)
   ▼  클라 공유버퍼 append/replay ── ★로직 재사용(무변경)★, decode 게이트만 넓힘
   ▼  reload resubscribe replay(ADR-0043) ── 재사용
   ▼  프론트 wsFrame decode(tag 분기): tag0→TerminalSlot / tag1→RichSlot(파서 제거)
```

## 3. Seam별 설계

### 백엔드 (Rust)
| # | 파일 | 변경 | 내용 |
|---|---|---|---|
| B1 | `core/agent/types.rs` | `OutputEvent` 확장 + `Serialize` | `TextDelta`/`ToolCall{name,args_json,optional id}`/`Usage`/`MessageDone`/`Error`/`Structured{kind,json}`(탈출구) + optional `turn_id`/`message_id`. `#[derive(Clone,Serialize)]` |
| B2 | `core/agent/backend/claude.rs` | **decoder** 신설 | `LineDecoder`(라인 재조립 상태 + claude JSON→OutputEvent). backend 소유. transport·core는 이 타입을 dyn으로만 봄 |
| B3 | `core/agent/`(조립점: session/manager) | decoder 주입 | pump→core 사이에 decoder를 꽂는다. transport는 바이트 emit 유지. json 모드면 decoder 있음, 터미널이면 없음(bytes 직통) |
| B4 | `core/agent/session.rs`(ReplayBuffer)·`output_core.rs`(emit) | `Ring<StoredOutput>` 일반화 | 저장단위 `StoredOutput{seq,payload,cost_bytes}`. `cost()`=바이트 len / 이벤트 인코딩 크기. **emit 락 규율 불변(ADR-0006)** |
| B5 | `core/agent/types.rs` | `OutputPayload{Bytes\|Event}` | `OutputFrame.data:&[u8]`→`OutputPayload`(Copy/빌림 유지). `OutputSink::send` 시그니처 갱신 |
| B6 | `protocol/codec.rs` | `tag1` + 헤더-only 게이트 | `tag1` encode/decode(payload=직렬화 이벤트, codec은 내용 모름). `decode_frame`이 known tag는 헤더만 파싱 payload opaque. golden |
| B7 | `daemon/ws.rs`(+adapter) | core→wire 변환 + frame 인코딩 | daemon adapter가 `OutputEvent`→wire 타입. `WsOutputSink`가 `OutputPayload::Event`→`tag1` frame |
| B8 | `src-tauri/daemon_client/connection.rs` | relay(대개 무변경 확인) | decode_frame이 tag1 성공하면 기존 Ok 경로가 프레임 저장·replay(로직 재사용). tag1 통과만 확인 |

### 프론트 (TS)
| # | 파일 | 변경 | LOC |
|---|---|---|---|
| F1 | `api/wsFrame.ts` | tag 분기(tag1→헤더+opaque payload→StructEvent 역직렬화) | +~20 |
| F2 | `api/transport.ts`·`wsTransport.ts` | `InboundMessage` union + onmessage 분기 | +~15 |
| F3 | `api/protocolClient.ts`·`agentClient.ts` | `handleOutput` tag 분기 + `OutputChunk` union(seq dedup·epoch 가드 유지) | +~35 |
| F4 | `components/slot/RichSlot.tsx` | 타입 이벤트 직접 소비 | +~15 |
| F5 | `lab/richslot/streamParse.ts`·`parse.ts` | NDJSON 파서 제거 | -189 (+대체 ~40) |
**무변경 확인:** `TerminalSlot`·`renderMode`·`ViewLayoutRenderer`·pre-subscribe 버퍼·리로드 경로(tag 분기만). ts-rs 바인딩 자동생성.

## 4. 볼륨
- **백엔드 ~500–650 LOC** — decoder+라인재조립(B2, ~150 최대)·버퍼 일반화(B4, ~90)·codec tag1+게이트+golden(B6, ~90)·나머지(B1/B3/B5/B7/B8) 소규모 + 모듈 테스트.
- **프론트 순 ≈ -100 LOC** — tag decode·union·RichSlot(+~85), 파서 제거(-189+40).
- **범위:** core·protocol·daemon + backend(claude decoder) + src-tauri relay 확인 + 프론트 5파일. **고위험 seam(ADR-0003 계약·ADR-0006 락)** → 코더 high · `/review code` deep · `/qa` full.
- **체감:** 중간~큰 단일 기능, 수일. 모듈별 커밋.

## 5. 테스트 (TDD + 모듈 격리, ADR-0012)
- **codec(B6):** golden — tag0/tag1 encode↔decode round-trip, 헤더-only 파싱, unknown tag 거부. `cargo test -p ...-protocol`.
- **decoder(B2):** claude stream-json fixture(실측 캡처) → OutputEvent 단언 + **부분 라인(청크 경계 split) 재조립** 케이스. 순수 함수 격리.
- **버퍼(B4):** `Ring<StoredOutput>` push/replay/eviction(byte budget + 건수 경계)·ReplayKind. headless.
- **emit 락(B4/B5):** 기존 output_core 회귀 + `OutputPayload` fanout.
- **★클라 opaque replay(신규):** `tag1` frame 수신 → 공유버퍼 append → resubscribe replay가 **원본 frame bytes 동일** 반환. (cdp 실측만 의존 금지 — 회귀 조기 포착.)
- **프론트(F):** vitest — wsFrame tag 분기·protocolClient seq dedup/epoch 가드 회귀.
- **통합/실측:** `cargo test`(워크스페이스) + `scripts/cdp.mjs` — JSON 에이전트 출력→RichSlot 렌더 + **웹뷰 리로드 후 타입 출력 replay 복원**.

## 6. 구현 순서 (의존순)
1. **B1 OutputEvent + B6 codec tag1 계약 + golden** — 타입·wire 계약 먼저 고정.
2. **B2 decoder(+라인재조립 테스트, fixture).**
3. **B4 Ring<StoredOutput> 일반화**(락 규율 회귀 주의) + **B5 OutputPayload/OutputSink.**
4. **B7 daemon adapter + WsOutputSink frame 인코딩** + **B8 클라 relay tag1 통과 확인** + 클라 opaque replay 테스트.
5. **B3 decoder 주입**(조립점) — E2E 배선 완성.
6. **F1~F4 프론트 tag 분기** → **F5 파서 제거.**
7. 통합·cdp 실측(리로드 replay 정합).

## 7. 불변식·리스크
- **ADR-0006** — emit lock 밖 send. 단일 ring이 기존 구조 유지(새 락 0).
- **ADR-0003** — core `OutputEvent`≠wire 타입, 변환 daemon adapter, core tauri-import 0.
- **ADR-0004** — claude 스키마 = decoder(backend/claude.rs)만. transport·core는 dyn decoder만 봄.
- **decoder 주입점(B3)** — transport는 바이트 emit 유지, decoder는 pump 출력을 소비해 core로. 주입 위치(session vs manager 조립점)는 코딩 시 확정, transport/core 순수성 유지가 제약.
- **codec 게이트 파급(B6)** — `decode_frame`은 데몬·클라 공용. tag1 허용이 양쪽 통과(의도된 것). unknown(≥2) tag는 계속 거부.
- **epoch·seq dedup(ADR-0007)** — 프론트 tag 분기가 우회 안 함(F3).
- **원 "B" 버그** — 범위 밖. 타입 경로는 append/replay 재사용이라 재도입 안 함.
