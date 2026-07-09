# 핸드오프: 채팅 대기표시 재설계+dot 정렬 커밋 완료 — 다음 = B(레이아웃) 스코프부터

## 한 줄 상태 · 다음 첫 액션
- **상태:** 채팅 대기표시 재설계(WaitRow) + rail dot 정렬 완료·커밋(2커밋, 로컬·미푸시). 라이브 GUI 실측까지 마침. 컨텍스트 60%에서 깨끗한 지점 승계.
- **다음 첫 액션:** **B(레이아웃) 스코프를 사용자와 확정** — 큰 설계라 PRD/TRD + **사용자 결정** 필수(임의 착수 금지). 어디부터(멀티뷰/split · 레이아웃 영속화 · §5 command 버스) 정하면 `/research`부터.

## 이번 세션 커밋 (master · 로컬만, 미푸시)
- `a185483` fix(chat): rail dot 첫 줄 정중앙 정렬 (`--chat-rail-dot-top` 0.75→0.5625rem)
- `ab54571` feat(chat): 채팅 대기표시 재설계 — 빈 Thought 제거 + Wait tail + 후속전송 flicker 수정 + 하단 여백

## 완료 (커밋됨)
- **채팅 대기표시 재설계** (`/implement standard` → 코더 Opus → `/review code full` 2R → 라이브 GUI 실측):
  - 빈 "Thought(내용 비공개)" 행(opus 암호화 thinking) **렌더 제거** — 매 응답 clutter. `isEmptyThinking` → `rowKindOf` 'skip' ↔ `renderItem` null (ADR-0051 rail parity 유지, 두 함수 동일 판정 필수).
  - tail "Thinking…" → **`WaitRow`(임시/provisional)**: "Wait" + 애니메이션 … + 마운트부터 경과 초(자족 setInterval, 안정 key `__streaming__`로 턴 도중 리셋 방지).
  - `showTail = streaming`(콘텐츠 게이트 제거) → **전송 즉시 표시**.
  - **flicker 수정(Codex 적출):** 후속 전송 시 합성 user 에코가 `turnDone=true`인 채 awaiting 해제 → 인디케이터가 첫 토큰 전까지 깜빡 꺼짐. 근본 수정 = `structuredAccumulator.ts` Structured `kind==='user'` 분기에서 **새 user 메시지 = 새 턴 → `turnDone=false`**(dedup break 뒤에 배치, replay 멱등 유지 — 최종 MessageDone이 다시 true).
  - **대기 tail 하단 여백:** 일반 메시지는 턴 종료 separator(h-3)로 입력창과 12px 간격을 얻지만 awaiting Wait은 없어서 붙음 → tail 뒤에 `<div aria-hidden className="h-3" />` 스페이서 추가(StructuredTextView).
- **rail dot 정렬:** dot이 첫 줄 정중앙보다 3px 아래(선존·af2063b 동일). `--chat-rail-dot-top` 0.75→0.5625rem. **theme.css ↔ chatStyleStore `CHAT_STYLE_DEFAULTS` 둘 다** 고쳐야 함(ADR-0051 drift 가드 테스트가 강제 — 값 권위 = store).

## 검증 상태 (쌍으로)
- **돌린 것 (PASS):** tsc 0 · vitest 282/282(+flicker guard·turnDone 전이 테스트 신규) · 코어 격리(`rg "use tauri" crates/engram-dashboard-core/src`) 0(문서화 false-positive 1건만) · `/review code full` 2R(doc-aware Opus + cross-family Codex) 모두 PASS · 스코프 `cargo test -p engram-dashboard-core -p engram-dashboard-protocol` 203 PASS · `cargo fmt --check` PASS · **라이브 GUI 실측**(실 JSON 에이전트 8e28e25e로 렌더·간격 12px·flicker 없음·dot 정중앙 cdp 측정). 재실행: `npx tsc --noEmit` · `npm test`.
- **미검증/안 됨:** 워크스페이스 `cargo build`/전체 `cargo test`는 **공유 데몬 바이너리 락**(`engram-dashboard-daemon.exe` 실행 중, 강제 종료 정책 거부 — 공유 인프라)으로 미실행. 프론트-only·Rust 무변경이라 회귀 불가, 스코프 -p 테스트로 대체 확인. **데몬 안 죽인 채로는 워크스페이스 cargo build 클린 재실행 불가** — 데몬 재기동 필요 시 이 점 유의.

## 실패한 접근 (do-not — 다시 시도 말 것)
- **WaitRow(또는 마지막 요소)에 하단 패딩(pb-4/pb-8)** = 안 보임. **★Radix ScrollArea의 `display:table` 내부 래퍼가 마지막 자식의 하단 *패딩*을 scrollHeight에 안 넣음 + 하단고정 auto-scroll(`scrollTop=scrollHeight`)★** → 패딩이 뷰포트 밖으로 밀려 갭 0. 실측 확인(pb-4·pb-8 둘 다 gap 0). **하단 여백은 패딩 말고 실제 높이 블록(h-3 스페이서)으로.** StructuredTextView의 컨테이너 `pb-3`도 같은 이유로 이미 먹히고 있음.
- **ChatRow `rowPt` 프롭 오버라이드**(tail 패딩 좁히기) = 이 문제엔 무관해서 원복. (uniform row-shift라 dot 정렬은 보존됐으나, 진짜 문제는 위 스페이서였음.)
- **dot 정렬을 근사 측정으로** = 첫 줄 center를 `text.top + lineHeight/2`로 근사해도 되지만, 정밀은 **Range.getClientRects()[0]**(첫 줄 실제 line box)로. 둘 다 0.5625rem에서 dot=정중앙.

## repo 상태
- 브랜치 master. 이번 2커밋 **로컬만, 미푸시**(원하면 push).
- **미커밋 이월(이 작업 밖 — 커밋 말 것):** `run-dashboard-clean.bat`(M) · `docs/reference/architecture-overview.md`(untracked). 세션 시작부터 존재.
- `.claude/handoff/`·`.claude/skill-bindings/` = 미추적(gitignore 추정).

## 앱 상태 (실행 중)
- dev 앱 실행 중(background 태스크 `bdswe99dp`, CDP 9223, 빌드 = 커밋 전이지만 HMR로 최신 반영). **라이브 JSON 에이전트 `8e28e25e`** 슬롯 배정됨(모델 fable-5). 재사용 or 재기동. cdp 튜닝 중 그 에이전트 대화에 테스트 메시지 여러 개 주입됨(간격측정·진단2·스샷용·pb8확인·여백확인 등 — 무해, 정리 불필요).
- ★주의: 앱 재기동 시 포트 1420 스테일 Vite / 공유 데몬 락 유의. cdp 실측용 send = 텍스트에어리어에 native setter로 값 주입 + Enter keydown 디스패치(실 send 경로).

## 정지 조건 (다음 세션)
- **B(레이아웃)는 큰 설계 = 사용자 결정.** 스코프·옵션을 사용자가 고르기 전 설계 확정·구현 진입 금지(CLAUDE.md 개발 스텝 순서 불변).

## B(레이아웃) — 다음 큰 주제 (backlog)
- 큰 설계 영역 → dev-step(PRD/TRD·**사용자 결정**) 밟아야. backlog(step-log "다음" 절 정본):
  - 멀티뷰/split · **창 레이아웃 영속화**(저장위치=프론트 localStorage 결정됨, tracking D-7 — 데몬화 뒤로 보류였음)
  - **§5 LLM 제어 표면(command 버스)** — 새 UI 기능마다 LLM 호출 경로 동반(ADR-0022 제안)
  - ADR-0014(오케스트레이션 제안)
- 착수 전 사용자와 스코프 확정 → `/research`로 OSS 서베이.

## 참조 (읽을 것만)
- step-log 2026-07-08 "채팅 대기표시 재설계 + rail dot 정렬" 절 (방금 추가) · "다음 (미진행)" 절 = B backlog 정본
- 코드 포인터: `WaitRow.tsx`(임시) · `StructuredTextView.tsx`(isEmptyThinking·showTail·tail+스페이서) · `structuredAccumulator.ts`(turnDone=false on user) · `theme.css`+`chatStyleStore.ts`(--chat-rail-dot-top 동기) · `ChatScrollArea.tsx`(Radix display:table 스크롤 seam)
