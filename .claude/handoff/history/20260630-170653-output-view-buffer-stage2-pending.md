# 핸드오프: 출력 평면 재설계 — 설계 완료(PRD/ADR-0040/TRD) + 1단계 core 구현, 2단계 src-tauri 통합 대기

> ⚠️ `.claude/continue` gitignored(로컬). master 브랜치. 직전 20260630(View 독립 방향 확정·정식 PRD/TRD 대기)의 후속 — 이번에 PRD→ADR-0040→TRD 설계 완결 + 사용자 교정 2건 + 1단계 core 모듈 구현까지.

## 한 줄 상태 + 다음 첫 액션
멀티뷰어 출력 재설계(**출력 단위 = View 독립**)의 설계 3단계(PRD→ADR-0040→TRD)를 전부 적대 리뷰 통과·완결 + 사용자 교정 2건 반영 + **1단계 core 순수 모듈 구현**(리뷰 FIX 반영, 게이트 통과)까지 마침.

**다음 첫 액션: 2단계 — src-tauri 통합.** `AgentBufferStore`(content+cursors) 신설 + `main_loop` Binary arm에 공유 버퍼 append 삽입 + fan-out을 cursor 기반으로 재작업 + 기존 min 모델(`output_window_seq`) **삭제** + `subscribe_output` 버퍼 replay + 재연결 `after_seq=latest_seq` + epoch 태깅 + slot 정리. **동시성-치명 → 코더 opus + `/review code deep`(락 순서·epoch race·재연결 무손실) + `/qa` cdp.mjs 멀티뷰 실측.**

## 핵심 설계 (정본 = 문서, 여기는 포인터)
- **ADR-0040(확정):** 출력 단위 = View 독립. 중계 허브(src-tauri) **콘텐츠 공유 버퍼**(에이전트당 1벌) + **per-view 인덱스**(슬롯별 cursor). 데몬 구독 에이전트당 1개.
- **자료구조(사용자 교정 — 핵심):** `content: HashMap<AgentId, BoundedSeqLog>`(콘텐츠 공유 1벌) + `cursors: HashMap<SlotId, {agent_id, cursor}>`(**슬롯 1차 키**). 같은 에이전트 N슬롯 = 콘텐츠 1벌 + cursor N. **버퍼 생명 = 어느 슬롯엔가 배정된 동안만**(cursor 0개면 폐기, 재배정 시 데몬 replay) — 죽은 에이전트 누수 자동 차단.
- **★두 축 분리(TRD §3, 무손실 급소):** 데몬 재구독 `after_seq` = **버퍼 최신 seq**(클라에 없는 것만) ≠ 창 read = **per-view cursor**. 절대 합치지 말 것(이 혼동이 code 리뷰 FIX 핵심이었음).
- **cursor = `Option<u64>`**(None=처음부터 전체, 데몬 `subscribe_from` 동형). u64 underflow 구조적 제거.
- **락:** 버퍼 락 안엔 데이터(append/cursor/snapshot)만, **Channel send는 락 밖**(데몬 C4 패턴, ADR-0006 — fan-out↔mount 락 순서 역전 데드락 차단). **프론트(WebView)는 단일 스레드라 데드락 무관**(기존 `tauriTransport.ts` race 가드는 유지, 추가·삭제 X).

## 완료 + repo 상태
- HEAD = master **e845daf**(설계 문서 커밋 + **origin 푸쉬 완료**). 직전 0723214의 후속.
- **커밋됨(e845daf):** PRD `output-view-buffer-prd.md` · ADR-0040 · TRD `output-view-buffer-trd.md`(사용자 교정 2건 + 데드락 정정 포함 최신본) · `decisions/README`(인덱스) · `step-log` · study-notes 4.
- **미커밋(디스크 영속 — 유실 아님):**
  - **1단계 산출**(이번 세션, 게이트 통과·리뷰 FIX 반영): `crates/engram-dashboard-core/src/output_view_buffer.rs`(신규 — `BoundedSeqLog`+`SlotCursorMap`+`ViewCursor`, 테스트 24) + `lib.rs`에 `pub mod output_view_buffer;`.
  - **직전 세션 T7 코드**(미커밋 유지): src-tauri `connection.rs`·`output_channel.rs`·`protocol_state.rs`·`commands/agent.rs`·`daemon_client/mod.rs`·`lib.rs`·`commands/discovery.rs` · 프론트 `clientFactory.ts`·`transport.ts`·`tauriTransport.ts`(신규)·`tauriTransport.test.ts`(신규) · `module1-transport-spike.md`. **2단계가 이 중 `connection.rs`(fan_out/resubscribe)·`output_channel.rs`를 재작업하며 함께 커밋 예정.**
  - **`output_window_seq.rs`**(직전, min 모델) + `lib.rs`의 그 mod 줄 — **2단계에서 삭제 대상**(cursor 모델로 대체). 1단계를 커밋 안 한 이유 = 이 삭제와 묶으려고(lib.rs mod 얽힘 회피).
  - `CLAUDE.md`(M, 직전 규약 1줄) — `/review doc` 게이트 별도, 이 작업과 무관.

## 검증 상태 (쌍으로)
### 돌린 것 (green)
- 1단계: `cargo test -p engram-dashboard-core` = lib 105 passed(output_view_buffer 24 포함) + 통합(reaper 6/session_smoke 1/transport_smoke 1) 전부. 재실행: 동일 명령. `cargo build -p engram-dashboard-core` OK. `rg "use tauri" crates/engram-dashboard-core/src/` = 실제 import 0(lib.rs 1매치는 docstring). `cargo fmt --check` OK.
- 리뷰: `/review prd full`·`/review trd full`·`/review code full` 전부 통과(FIX 반영). code 리뷰에서 Codex(blind)가 무손실 경계 버그 2건 적출→FIX(cursor Option화·clamp off-by-one)→테스트로 박제.
### 검증 안 됨 (오신뢰 금지)
- **2단계 전부 미착수** — src-tauri 통합·fan-out 재작업·재연결·epoch·slot 정리 코드 0.
- **전체 `cargo test`(워크스페이스)·`cargo build`(루트) 미실행**(1단계는 core crate만 돌림).
- **GUI 실측 0** — cdp.mjs 멀티뷰(같은 에이전트 2·3창·새 창 늦게·재연결 중 새 창) 미실행. 화면 동작 미확인.
- **src-tauri lib test = `0xc0000139`(DLL 엔트리포인트) 여전**(직전부터 미해결). 그래서 회귀를 core headless로 뺀 것(1단계 설계 근거).
- 프론트 cutover(wsTransport→TauriTransport)는 직전 T7c 일부 진행, 출력 평면과의 결합 미실측.

## 2단계 구현 시 주의 (TRD에서 꺼내 쓸 것 — 리뷰가 짚은 함정)
- **epoch ∩ 새창:** `main_loop` 단일 actor라 직렬 — 락 아니라 **frame.epoch 태깅**으로(SubscribeAck 안 기다림). 데몬 wire FIFO(Ack→replay→frame) 보장은 spike로 확인.
- **재연결 `after_seq = latest_seq()`**(버퍼 최신), min 합산 폐기. 창 read는 cursor로 별도(두 축).
- **다중 락 순서:** 버퍼 락 ⊃ registry 락 금지 → send 락 밖. `subscribe_output`(registry+버퍼)도 순서 일관.
- **gap(Truncated):** 버퍼 최신 < 데몬 oldest면 데몬 oldest부터 재구성 + 모든 cursor clamp. 잘림 미표시(사용자 결정).
- **공통 추출 = 데이터 구조만** — 데몬 `ReplayBuffer` struct/동기화/C4 비공유(핫패스 회귀 차단). 1단계 `BoundedSeqLog`는 이미 별도 신설.
- **resize 위험(미해결):** 단일 공유 버퍼 raw bytes ∩ 크기 다른 창 → escape 충돌 가능. 현재도 동일이라 악화 아님. viewport 분기 필요 시 retrofit(ADR-0040 §영향).

## do-not (실패/금지)
- **두 축(데몬 재구독 ↔ 창 read) 합치지 말 것** — cursor 하나로 합치면 미렌더 창 유실/중복(code 리뷰 FIX 핵심).
- **버퍼 락 보유 중 Channel send 금지**(데드락) — 데몬 C4처럼 락 밖 send.
- **데몬 `output_core.rs`/`ReplayBuffer` struct·src-tauri 동시성 영역 함부로 건드리지 말 것** — 공통은 데이터 구조만.
- **min 모델(`output_window_seq`) 부분 보존 금지** — 통째 삭제·cursor 대체(부분 잔존 = 두 모델 혼재).
- 직전 do-not 유효: (b)안 프론트 직접 구독 금지(ADR-0035), DLL `0xc0000139` 재빌드/clean으로 풀려 말 것(기각됨).

## 미결정·블로커
- 없음(설계·정책 다 확정). 2단계는 순수 구현 + 검증.
- 직전 후속 항목 그대로 살아있음: app:None ADR 채번, CLAUDE.md 규약 `/review doc`, 주석 중복(tauriTransport.ts).

## 이번 세션 서브에이전트 이력
- Explore: 현재 출력 평면 정밀 맵(데몬 ReplayBuffer·main_loop·fan_out·resubscribe).
- `/review prd full`(Codex User-blind + opus Tester-blind): 둘 다 FIX — "처음부터 전부↔버퍼 상한" 모순 cross-family 수렴.
- `/review trd full`(Codex Designer + opus Architect-breaker): 둘 다 FIX — 두 축 분리·락·epoch·생명주기·공통추출 5건.
- 코더 general-purpose(opus) ×2: 1단계 core 구현 + 리뷰 FIX(cursor Option화·clamp·거대 chunk·테스트 보강).
- `/review code full`(Codex blind breaker + opus correctness): **불일치**(Codex FIX vs opus PASS) → 메인 검증 = Codex 옳음 → 무손실 경계 버그 2건 FIX. (cross-family 가치 실증.)

## 참조 (읽어야 할 것만)
- `docs/process/S14-multi-page-layout/output-view-buffer-trd.md` — 2단계 구현 명세 정본(§1 자료구조·§3 두 축·§4 생명주기·§4b epoch·모듈경계·동시성·결정점 표).
- `docs/decisions/0040-*.md` — 결정·거부 대안.
- `crates/engram-dashboard-core/src/output_view_buffer.rs` — 1단계 산출(2단계가 배선할 대상).
- `crates/engram-dashboard-core/src/agent/output_core.rs` — 데몬 ReplayBuffer/subscribe_from(미러 참조, 건드리지 말 것).
- `src-tauri/src/daemon_client/connection.rs`(main_loop Binary arm·fan_out_per_window·resubscribe) · `src-tauri/src/output_channel.rs`(registry) — 2단계 재작업 지점.
- `docs/process/step-log.md` 최신 항목.
