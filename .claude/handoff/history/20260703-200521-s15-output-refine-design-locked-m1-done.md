# 핸드오프: S15 "출력 정제를 백엔드로" — 설계(ADR-0045+TRD) 확정·커밋 + 모듈1(타입·wire 계약) 완료·커밋, 다음=모듈2 decoder

## 한 줄 상태 · 다음 첫 액션
S15 = JSON 모드 출력 정제(claude NDJSON→타입 이벤트)를 **프론트→백엔드로 이동**. 이 세션에서 **설계 전부 확정·커밋**(ADR-0045 + S15 TRD, 리뷰 3라운드로 교정) + **구현 모듈1(B1 OutputEvent 구조화 variant + B6 codec tag1) 완료·게이트·커밋(`b716237`)**.
**다음 첫 액션 = 모듈2(B2) decoder:** `/implement "S15 모듈2 B2 — backend/claude.rs에 NDJSON→OutputEvent decoder(라인 재조립 상태머신 + claude stream-json 파싱). transport·core는 claude 스키마 모름(ADR-0004). fixture는 stream-json 캡처로 부분-라인 경계 케이스 포함 TDD." standard` — TRD §3 B2 + §6 순서 2번. 동시성 무관·중간 난이도라 이번 세션(신선)에 하기 적합.

---

## ⭐ 실패한 접근 / do-not — 리뷰가 코드로 반증함. 절대 재현 금지 ⭐
이 세션의 핵심 가치가 아래 오판들을 적대 리뷰(opus+Codex, 3라운드)로 코드 검증해 뒤집은 것이다. 다시 꺼내지 말 것:

1. **"wire가 이미 typed-ready / 잔여 4 seam / 추상화 위에 얹기"** = **거짓.** protocol `OutputChunk` enum은 정의+ts-rs 바인딩만 있고 **producer가 0개**다(`rg "AgentEvent::Output {"` → 소비 매치뿐). `OutputSink`/`OutputFrame`은 바이트 전용(`types.rs:225/297`), replay 버퍼도 바이트. 실제론 파이프라인 여러 seam 개조. (Explore 수집이 confident-wrong으로 나를 오도했고 opus 리뷰어가 `rg`로 반증.)
2. **"타입 이벤트를 JSON `AgentEvent::Output`(Text arm)로 흘린다(frame tag 없이)"** = **틀림 → tag1 binary frame으로 확정.** 클라 공유버퍼(ADR-0040)는 **binary frame(Binary arm)에만** 붙어 있어 JSON Text arm은 seq/cursor/replay를 안 탄다 → 리로드 시 타입 출력 두절(= 구 B 버그 재발). frame tag로 흘려야 기존 frame-opaque replay 로직을 재사용.
3. **"클라 공유버퍼·mount-replay는 frame-opaque라 완전 무변경"** = **부분 거짓.** `connection.rs:1008`이 저장 전 `decode_frame`을 부르고 `codec.rs:64`가 `tag≠0`을 거부 → 그냥은 tag1이 저장조차 안 됨. **정정(모듈1서 완료): `decode_frame`을 known tag(0,1) 헤더-only 파싱(payload opaque)으로 넓힘** → 공유버퍼 append/replay **로직**은 재사용. 단 codec 게이트·`connection.rs` relay·`wsFrame.ts` decode는 변경면(완전 무변경 아님).
4. **"파서 주입 = InputEncoder 대칭"** = **틀림.** 입력은 session 경유지만 **출력 pump는 `StdioTransport` 안에서 `emit` 직접**(`stdio.rs:224`). 그래서 decoder는 pump→core 사이 **backend 소유 레이어**(fork ①=B). 라인 재조립 상태머신 필요(pump가 4096 임의 청크로 주므로 NDJSON 라인 경계와 안 맞음).
5. **원 "B" 버그(터미널 바이트 경로 리로드 두절)** = **S15 범위 밖**(선재 버그, 별도 세션). 타입 경로는 append/replay 재사용이라 재도입 안 함. S15에 끌어들이지 말 것.
6. **`/implement`·`/adr`·`/review`·`/qa` 스킬 전부 디스크에 있고 정상**(세션 중 batch 배포 `b12ecc2`로 들어옴). 옛 핸드오프가 "implement 없음"이라 했으면 무시.
7. **두 버퍼 클래스(TerminalBuffer/StructuredBuffer + trait) 안** — 리뷰·사용자가 기각. 단일 `Ring<StoredOutput>`로 확정(dyn/trait이 락 규율 숨길 위험만 늘림).

---

## 확정된 설계 (정본 = ADR-0045, 아래는 요약)

**사용자 확정 4:** ① 파서=decoder 레이어(transport 순수 바이트) ② 버퍼=단일 `Ring<StoredOutput>`(원본/변환본 안 겹침) ③ wire=`tag1` StructuredEvent 하나(self-describing payload, variant별 tag 지양) ④ 원 B=범위 밖.

**리뷰 반영 추가 결정:**
- **core↔wire 변환 = daemon adapter**(protocol crate 아님 — ADR-0003 양방향 격리). core는 `OutputEvent`만, protocol은 wire 타입만, daemon이 결합.
- **네이밍:** core 도메인 = `OutputPayload{Bytes(&[u8])|Event(&OutputEvent)}`, sink/wire 인코딩 단계 = `WirePayload`.
- **`Serialize`는 core `OutputEvent`에 안 붙임** → B7 daemon adapter서 wire 타입에(직렬화 형식 조기확정 회피, §0).
- **eviction = byte budget(인코딩 payload 크기) + 이벤트 건수 상한** 별도(큰 `args_json`이 건수 1로 새서 2MB 상한 깨지 않게).
- **`Structured{kind,json}` 탈출구 + optional `turn_id`/`message_id`** = codex/gemini turn·tool 모델 누수 흡수.

**데이터 흐름(확정):**
```
claude NDJSON ─▶ StdioTransport.pump(바이트 emit, 순수 파이프)
   ─▶ decoder(backend 소유: 라인재조립 + claude 파싱, ADR-0004) ─▶ OutputEvent
   ─▶ OutputCore: Ring<StoredOutput>(seq·byte budget·cursor), emit은 lock 밖 send(ADR-0006 불변)
   ─▶ OutputPayload{Bytes|Event}
   ─▶ daemon adapter(core→wire 변환) ─▶ WsOutputSink(frame: tag0=바이트 / tag1=StructuredEvent 직렬화)
   ─▶ codec.decode_frame(known tag 헤더-only, payload opaque) ─▶ 클라 공유버퍼 append/replay(로직 재사용)
   ─▶ reload resubscribe replay(ADR-0043, 재사용) ─▶ 프론트 wsFrame(tag 분기: tag0→TerminalSlot / tag1→RichSlot 파서 제거)
```

---

## 남은 모듈 맵 + 각 gotcha (TRD §3/§6 정본)
```
✅ ① B1+B6  타입·wire 계약        (커밋 b716237)
   ② B2      decoder             ← 다음. NDJSON→OutputEvent + 라인재조립. 동시성 무관.
   ③ B4+B5   Ring<StoredOutput> + OutputPayload/OutputSink   ← ★동시성-치명(ADR-0006 락)
   ④ B7+B8   daemon adapter + WsOutputSink frame + 클라 relay tag1 + opaque replay 테스트  ← ★핫패스
   ⑤ B3      decoder 주입(조립점)  ← ★B4보다 먼저 금지(아래)
   ⑥ F1~F5   프론트 tag 분기 + streamParse/parse 제거   ← UI 닿음 → /qa full(cdp)
   ⑦ cdp     리로드 후 타입 출력 replay 복원 실측
```
- **② B2:** fixture = claude stream-json 캡처. step-log M0/M1 참조 — `~/.claude/projects/`에 CLI 자체 JSONL 보관 실확인, "1턴 assistant 4줄 반복" 실측. 부분-라인(청크 경계 split)·UTF-8 경계 재조립 테스트 필수.
- **③ B4+B5 (동시성-치명):** 현 `ReplayBuffer`(session.rs)·`output_core.rs` emit을 `Ring<StoredOutput>`로 일반화. **emit "replay lock 짧게→해제→subscribers clone→lock 밖 send" 규율 절대 보존**(ADR-0006). `/review code deep` + `/qa full` 강제. **★신선 컨텍스트에서 시작 권장**(step-log 반복 지침).
- **④ B7+B8:** daemon adapter(core→wire) + `WsOutputSink` Event→tag1 frame + 클라 `connection.rs` relay가 tag1 통과 확인(decode_frame 성공 시 기존 Ok 경로 재사용이라 대개 무변경) + **클라 opaque replay 단위 테스트 신규**(tag1 frame→공유버퍼 append→resubscribe replay 바이트 동일 — cdp만 의존 금지).
- **⑤ B3 (★순서 함정):** decoder 주입. **B4보다 먼저 하면 안 됨** — B4(payload-generic emit) 전엔 구조화 이벤트가 `output_core` `_` arm서 조용히 drop된다. 모듈1이 그 arm에 `debug_assert!`+`tracing::warn` dormant guard를 이미 부착(순서 위반 시 시끄럽게). B4가 그 wildcard를 대체.
- **⑥ 프론트:** wsFrame(tag 분기)·transport/protocolClient/agentClient(union, seq dedup·epoch 가드 유지)·RichSlot(타입 이벤트 직접 소비) + `lab/richslot/streamParse.ts`·`parse.ts` 제거(-189 LOC). 순 ≈ -100 LOC. TerminalSlot·renderMode·ViewLayoutRenderer·pre-subscribe 버퍼·리로드 경로 무변경(tag 분기만).

**볼륨:** 백엔드 ~500-650 LOC(decoder+라인재조립이 최대 ~150), 프론트 순 ≈ -100. 수일. 모듈별 커밋.

---

## repo 상태
- **HEAD = `b716237`(master), 미푸쉬**(관행상 owner 승인 대기). 작업트리 **clean**(미커밋 0).
- 이 세션 커밋 2개: `273a4cb`(설계 docs — ADR-0045 신규·ADR-0044 §31/§34 부분폐기 인라인·S15 TRD·step-log S15) + `b716237`(모듈1 코드 — core+protocol 7파일, 211+/20-).
- 세션 중 HEAD가 여러 번 이동(다른 세션/사용자가 skills batch 배포 `b12ecc2`·qa fix `b80ec76` 등 커밋) — 내 작업은 그 위에 얹힘. 정상.

## 검증 상태 (쌍 — 오신뢰 금지)
**돌린 것(모듈1만):** `/review code full` PASS(opus reviewer-deep doc-aware + Codex blind) · `/qa standard` PASS — `cargo test` core 150+·protocol 11·daemon 36·discovery 0 failed / core tauri import 0 / `npx tsc --noEmit` 0 / vitest 194 green. **재실행:** `cargo test -p engram-dashboard-protocol -p engram-dashboard-core -p engram-dashboard-daemon -p engram-dashboard-discovery` · `cargo build` · `npx tsc --noEmit` · `npm test`.
**검증 안 된 것:** 모듈2~⑦ 전부 미구현. decoder 실제 claude 파싱은 fixture 실측 전 미검증(B2). **리로드 replay 정합은 ④~⑦ cdp 실측 전까지 미검증.** B4 락 규율은 deep 리뷰 게이트로만 담보(1회 통과≠race-free).

## 선재 이슈 (내 변경 아님 — "내가 깼나" 오해 금지)
- `cargo fmt --check`(workspace)가 **`crates/engram-dashboard-daemon/tests/ws_e2e.rs:23`** 에서만 FAIL — CRLF 아티팩트 의심, 세션 전부터 존재. 내 core/protocol 파일은 clean. (원하면 별도 1줄 정리.)
- **src-tauri 테스트 exe `STATUS_ENTRYPOINT_NOT_FOUND(0xc0000139)`** — WebView2Loader.dll 부재, 선재 환경 이슈(step-log M1 기록). workspace 루트 `cargo test`가 src-tauri에서 깨짐 → **멤버별로 돌릴 것**.

## 참조 (읽을 것만)
- **정본:** `docs/decisions/0045-출력-정제를-백엔드로-이동-*.md`(결정·거부대안·불변식) + `docs/process/S15-backend-output-refine/backend-output-refine-trd.md`(seam별 설계·볼륨·테스트·순서). 관련 ADR: 0044(부분폐기됨)·0002(출력종류 비가정)·0003(코어 격리)·0004(claude 격리)·0006(락)·0040(서버 버퍼)·0043(mount-replay).
- **모듈1 코드(참고):** `crates/engram-dashboard-core/src/agent/types.rs`(OutputEvent) · `output_core.rs:104-158`(emit `_` guard·락 규율) · `transport/stdio.rs:7-11,224`(pump·정정 주석) · `crates/engram-dashboard-protocol/src/codec.rs`(tag0/tag1·decode_frame 헤더-only) · `src/lib.rs` · `tests/codec_golden.rs`.
- **다음 모듈이 만질 코드 refs(리뷰가 짚음):** `connection.rs:1008/1064`(클라 relay decode) · `output_view_buffer.rs:19/34`(프레임-opaque 저장) · `session.rs`(ReplayBuffer) · `backend/mod.rs:124`(InputEncoder=입력측, 출력 decoder는 대칭 아님) · `messages.rs:286`(OutputChunk wire enum+ts-rs, B7이 tag1 payload로 재사용 후보) · `daemon/ws.rs`(WsOutputSink).
- step-log `docs/process/step-log.md` S15 항목 = 이 흐름의 언제/무엇.

## 협업 메모
- 사용자는 **개념 이해 후 결정**을 원함 — 구조 갈림길은 그림(ASCII)으로 천천히 설명하면 잘 받음. AskUserQuestion 피커는 거부 경향(대화형 prose + 추천 선호).
- 위임 강함("알아서 해"·"쭉쭉 진행") — 단 **전체 구조 결정은 사용자에게 올림**. 구현 갈림길도 비자명하면 올릴 것.
- 구현 규약 준수: 코더(opus)→`/review code`(opus+Codex)→`/qa`→모듈별 커밋. 메인 직접 구현 금지.
