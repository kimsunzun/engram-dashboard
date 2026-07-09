# 핸드오프: B(레이아웃) 착수 — Brick 1(레거시 퍼지+메뉴=커맨드) 커밋 완료, 다음 = Brick 2 탭바

## 한 줄 상태 · 다음 첫 액션
- **상태:** 레이아웃 재설계 **B** 착수. **Brick 1 완주·커밋**(`297796e`, 로컬·미푸시). `/implement standard` 파이프라인 완료(코더 Opus → `/review code full` 2R → `/qa full` PASS + GUI 실측). 앱 dev 실행 중.
- **다음 첫 액션:** **Brick 2 = 최소 탭 바 UI**(View 가시화). `createView`/`switchView`는 이미 동작, UI만 없음. `/implement`로. 착수 전 아래 "결정거리" 중 **"새 창"=새 View 가정** 사용자 재확인 권장.

## 1차 목표 (수용 시나리오 — 이 밀스톤의 종착)
튜토리얼 클로드 생성(우클릭) → 채팅 지시로 **C드라이브 포커싱 A/B/C/D 스폰** → **A는 현재 창, C/D는 새 창(=새 View 가정)** → **A→C 메시지 전송**. 현 구조 유지한 채 이 기능 위주. LLM(튜토리얼 클로드)이 커맨드로 전부 구동하는 게 핵심(§5).

## Brick 분해 (사용자 승인 로드맵)
- **Brick 1 ✅(커밋됨):** 레거시 퍼지 + 우클릭 메뉴=커맨드 마운트 + slot-popup 폐기.
- **Brick 2(다음):** 최소 탭 바 UI(View 가시화 — `+`=createView, 클릭=switchView). "새 창" 결정 지점.
- **Brick 3:** 튜토리얼 클로드(메뉴 항목, **cwd=실행파일 위치** — 런타임 exe dir 해석) + 스폰.
- **Brick 4:** 오케스트레이션 시나리오(A/B/C/D 스폰·배치·A→C 메시지). 대부분 LLM이 커맨드로 구동 — 커맨드 완비 + cdp/chat 실증.

## Brick 1 완료 내역 (커밋 297796e)
- **삭제:** 레거시 `slotStore`(numeric id·dispatch(LayoutCommand)) · 죽은 `layout/LayoutRenderer`·`layout/SlotPane`·`slot/SlotPane` · `PopupPage`+`/popup` 라우트 · `src/lab/richslot/` 전체 · M0 스파이크(`richSlots`/`mountRich`/`__richslot`, RichSlot FixtureRichSlot 절반) · 죽은 `StructuredItemStream`.
- **재배선(§5):** `ViewLayoutRenderer` 슬롯 래퍼 우클릭 → `SlotContextMenu` 인라인 마운트 → `window.__engramLayout`(split/closeSlot/assignAgent, activeViewId 키, 문자열 UUID; spawn/kill은 agentClient ADR-0011). `// ADR-0035` 앵커.
- **slot-popup 창 폐기:** tauri.conf.json + capabilities/default.json(+regen gen/schemas → `["main","agent-tree"]`) + output_router 테스트 픽스처(slot-popup→agent-tree)·주석. **일반 `window_bindings`/`OutputRouter::rebuild`/main·agent-tree 보존.** CLAUDE.md "창 3개→2개".

## 검증 상태 (쌍으로)
- **돌린 것 (PASS):** tsc 0 · vitest 265 · cargo build(전체 링크) · 멤버별 cargo test green(protocol/core/discovery/daemon) · fmt · 코어 격리 0 · `/review code full` 2R(doc-aware Opus + cross-family Codex) PASS · **GUI 실측(cdp): 우클릭 메뉴 5항목 마운트 + 가로분할 슬롯 1→2 + 닫기 2→1 엔드투엔드**. 재실행: `npx tsc --noEmit` · `npm test` · `cargo build` · `cargo test -p engram-dashboard-protocol -p engram-dashboard-core -p engram-dashboard-discovery -p engram-dashboard-daemon`.
- **미검증/제약:** **src-tauri lib 테스트(`cargo test` 전체)는 선재 `0xC0000139 STATUS_ENTRYPOINT_NOT_FOUND`(WebView2Loader, build.rs 문서화)로 실행 불가** — output_router 라우팅 테스트는 컴파일 + 리뷰 정독으로만 검증(실행 미검, 변경 무관 환경 이슈). 전체 `cargo test`(루트) 대신 멤버별 `-p`로 돌릴 것.

## 남은 결정거리 (백엔드 권위 공백 — Brick 2+ 사용자 결정)
- **슬롯 "콘텐츠 종류"(tree/terminal swap) 백엔드 모델** — 지금 프론트 전용. item 1(트리 슬롯화)·item 3(트리보기 메뉴)이 여기 걸림. **가장 핵심.** (이번 브릭에 트리보기/터미널보기 메뉴 항목은 `// gap:` 드롭.)
- **슬롯 포커스 권위** — 프론트 전용, 백엔드 커맨드 없음. click-focus도 `// gap:` 드롭(SlotPane은 focusedSlotId 읽기만).
- **동적 창 생성** — 지금 정적 창(main/agent-tree만). "새 창"=새 View로 가정 중. 진짜 OS 창 팝아웃은 나중(DOM `window.open('#/view-popup?viewId=X')` 재활용 가능).

## repo 상태
- 브랜치 master. **미푸시 커밋 = Brick 1(`297796e`) + 이전 정리 3 + 그 전 5 = 총 9개**(origin/master 대비). push는 **하네스가 master 직접 push 차단** → 사용자 `! git push origin master` 또는 승인 필요.
- 워킹트리 클린. `.claude/handoff/`·`.claude/skill-bindings/`는 gitignore(추적 밖).

## 앱 상태 (실행 중)
- dev 앱 실행 중(background 태스크 `bopmfqe39`, CDP 9223, **새 빌드**). 데몬은 이번에 재기동(fresh — 이전 데몬 PID 33116 kill, 사용자 승인). View 1개·슬롯 1개(`cf9b8a59…`) 기본 상태.
- cdp 실측용 우클릭: 슬롯 `[data-slot-id]`에 `contextmenu` MouseEvent 디스패치 → **fire와 read를 분리**(React commit 갭 ~500ms)해야 메뉴 DOM 관측됨. 메뉴 항목은 leaf `div/span` textContent로 찾아 `.click()`.

## 실패한 접근 / 정지 조건 (do-not)
- **앱 재기동 시 포트 1420 스테일 Vite** — `beforeDevCommand`(vite) "Port 1420 in use"로 실패. `Get-NetTCPConnection -LocalPort 1420` → `Stop-Process` 먼저. (Rust는 정상 컴파일됐어도 vite에서 죽음.)
- **공유 데몬 exe 락** — 데몬 실행 중이면 `cargo build` 링크가 os error 5. 데몬 kill = 공유 인프라(에이전트 죽음) → **사용자 승인 후에만.**
- **B는 큰 설계 = 사용자 결정.** 위 "결정거리"를 사용자가 고르기 전 설계 확정·구현 진입 금지(CLAUDE.md 개발 스텝 순서 불변).

## 참조 (읽을 것만)
- step-log 2026-07-08 "B(레이아웃) 착수 — Brick 1" 절(방금 추가) · "다음 (미진행)" 절에 §5 command 버스·data-driven 우클릭 메뉴·D-7 창영속화 backlog.
- 코드 포인터: `ViewLayoutRenderer.tsx`(슬롯 우클릭 마운트·`// ADR-0035`) · `SlotContextMenu.tsx`(메뉴→__engramLayout, enabled 가드, `// gap:` 트리/포커스) · `eventBus.ts`(`window.__engramLayout` 표면) · `viewStore.ts`(activeViewId·layouts·views) · `output_router.rs`(일반 window_bindings — slot-popup 제거해도 보존).
- 아키텍처 조감도: `docs/reference/architecture-overview.md`.
