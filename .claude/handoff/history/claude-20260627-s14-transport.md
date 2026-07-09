# S14 멀티 페이지 레이아웃 — 핸드오프 (2026-06-27, dashboard2/opus 세션)

## 한 줄 상태
레이아웃 권위·전송 토폴로지 **설계 확정 + ADR 박음 + 모듈②(레이아웃 코어) 구현·리뷰·커밋 완료.** 다음 = 모듈①(전송 재배치) — **설계 spike 완료, D1~D5 결정 후 T1~T8 코딩.**

## 확정된 결정 (ADR)
- **ADR-0035** 레이아웃 권위 = src-tauri 클라(데몬 UI 불가지론, 에디터 모델). 기준: engram은 에이전트/슬롯 디커플링(close_view해도 에이전트 생존·재배정) → 에디터 모델이지 tmux(서버권위) 모델 아님.
- **ADR-0036** 전송 중계 통일 = src-tauri 단일 데몬 클라이언트 + OutputRouter, 창=Tauri IPC. **phasing 없음(레거시 각자-연결 폐기, 사용자 "제대로" 결정).** 전송 재배치는 ★동시성-치명(ADR-0001/0005/0006/0007 보존, TDD+deep 리뷰).
- 근거 리서치: `docs/research/multi-window-layout-authority-topology-research-2026-06-27.md`(deep, Claude+Codex 교차 — 터미널 멀티플렉서=서버권위 / GUI 에디터=클라로컬, engram은 후자).

## 커밋 (master, 3개)
- `66823b2` 설계 docs(research·ADR·TRD·step-log·study-notes) + richslot/terminal lab 스캐폴드(dashboard1)
- `daa2f3f` docs 최신화(TRD rev.5·ADR-0036·step-log — 직전 커밋의 stale 버전 갱신)
- `cb951da` **모듈② ViewManager 레이아웃 코어**(src-tauri) — 55 tests green, opus+Codex 리뷰 FIX 반영

## 완료: 모듈② (커밋됨, 검증 끝)
`src-tauri/src/layout/{types,tree,manager,mod}.rs` + `commands/layout.rs` + lib.rs/Cargo.toml. ViewManager(views·active_view_id·window_bindings·version) + 순수 트리 연산(split/close/assign/focus-fallback/ratio-clamp, headless 테스트) + invoke 7종(create/close/switch_view·split/close_slot·assign_agent·get_view) + emit(layout:updated/view:list-updated). 락→변형→해제→emit(ADR-0006). 데몬/protocol crate 누설 0(ADR-0035). ts-rs 바인딩 `src-tauri/bindings/`.
- 리뷰 FIX 반영: version `#[ts(type="number")]`(FIX-1) · ts-rs export 이중출처 제거(FIX-2). FIX-3(assign_in_tree 이중순회) 보류(저위험).

## 다음: 모듈① 전송 재배치 — ★먼저 D1~D5 결정★
**설계 spike 완료 = `docs/process/S14-multi-page-layout/module1-transport-spike.md` 정독부터.** (현행 TS 불변식 맵 · Rust 재배치 설계 · 불변식 매핑 · T1~T8 분해+격리 테스트 · D1~D5 · 리스크.)

### 코딩 전 사용자 결정 (spike §5)
- **D1(최대)** dedup/epoch 가드 위치: (A)Rust 단독 / (B)Rust 1차+JS 2차 / (C)JS 단독+Rust raw relay. ADR-0011 ProtocolClient 정체성에 영향.
- D2 ProtocolClient 두께(D1 종속) · D3 라우팅 출력 carrier(Tauri Channel vs emit_to) · D4 InProc mock 테스트 처리 · D5 discovery ensure_lock 제거 여부.

### 코딩 순서 (각 코더→`/review code deep`→QA)
T1 deps복원 → T2 connect/handshake → T3 protocol_state → T4 reconnect+resubscribe(★Blocker-1 race) → T5 OutputRouter(arc-swap) → T6 invoke → T7 TauriTransport → T8 React 정리.
- **TS 테스트 2파일(`wsTransport.test.ts`·`protocolClient.test.ts`, 40+케이스)이 Rust 이식 명세서.**
- 최위험: T4 task 교체 zombie/hijack(Blocker-1 이식 단언) · OutputRouter↔ViewManager 락 순서 · resubscribe N창 구독 ref-count.

## dashboard1 조율 (orch)
- SlotPane.tsx(2곳: components/slot·components/layout)·SlotContextMenu.tsx 공통 수정 — **건드리기 전 핑**(합의됨). 모듈③(렌더러)에서 충돌 지점.
- resize Task1(PTY 80x24): 현 경로(protocolClient.resizePty WS직결)에서 고침 → 통일 후 carrier만 교체(인터페이스 불변 carry-forward).
- 멀티뷰 resize 정책(tmux window-size식)은 Phase B/src-tauri 권위. Task1 범위 아님.

## 미커밋 (안 건드림 — dashboard1 소관)
`src/lab/richslot/*`(layouts.tsx/css 신규 + RichSlot/blocks/richslot.css 삭제)·`src/lab/main.tsx`·`richslot.html`·`src/components/slot/TerminalSlot.tsx`(Task1 추정). dashboard1 작업이라 다음 세션도 건드리지 말 것.

## 핸드오프 체크리스트 (CLAUDE.md)
- [x] 새 설계 결정 → ADR-0035·0036 작성·인덱스 갱신(lint error 0)
- [x] step-log 흐름 추가(rev.4/rev.5·리뷰·커밋·orch 교신)
- [x] 모듈② 커밋 + 검증 docs 최신화 커밋
- [ ] 모듈①: D1~D5 결정 → T1~T8 (다음 세션)

## 참조
ADR 인덱스 `docs/decisions/README.md` · TRD `docs/process/S14-multi-page-layout/trd.md`(rev.5) · spike `module1-transport-spike.md` · step-log `docs/process/step-log.md`(S14 섹션).
