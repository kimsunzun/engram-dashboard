# 핸드오프: Phase 2 탭 — 스테이지 1~4 완주(백엔드+프론트, 앱 실행가능·GUI검증·push됨), 스테이지 5(spawn_into)·로드맵은 fresh 승계

## 한 줄 상태 · 다음 첫 액션
- **상태:** WezTerm 탭 기능 = **백엔드 모델(스테이지 1~3, `bd8dfb2`) + 프론트 UI(스테이지 4, `9a6f5b8`) 완료·origin push됨.** 앱 실행가능, cdp GUI E2E 통과. 사용자 결정 D-1~D-8 전부 구현. **ADR-0057** 박제.
- **다음 첫 액션:** **스테이지 5 = `spawn_into`(D-7 배치 지정 스폰)** — 얇은 합성 command(스폰(데몬 agentClient) → tab 미지정이면 create_tab → 슬롯 배정). 정본 = TRD B-tabs **§6 spawn_into 슬롯 정책**(slot 미지정→그 탭 빈 root 슬롯 / 점유→에러 / 실패 시 에이전트 데몬 생존·`list_agents` 재부착). 두 서브시스템(데몬 스폰 + src-tauri 레이아웃) 걸침 → command는 src-tauri. `/implement`(standard~critical — 조립 래퍼라 국소, 단 두 시스템 걸침).

## 완료 / repo 상태
- 브랜치 **master**, **origin 동기화**(4149254까지 push). 워킹트리 **clean**.
- 이번 세션 커밋(4): `9e1bf80`(TRD + ADR-0057) · `bd8dfb2`(백엔드 1~3) · `9a6f5b8`(프론트 4) · `4149254`(step-log).
- **ADR-0057** = 탭 소유 모델(창별 탭 + 유니크 소유, owner-index 하이브리드). **ADR-0035 부분 개정**(active_view_id=main-전용 절 → 창별 active. 핵심=레이아웃 권위 src-tauri는 불변). 인덱스·lint clean.

## 무엇이 됨 (구현 상세 — 재구현 금지)
- **백엔드:** `ViewManager` = `views: HashMap<ViewId,View>` + `view_owner: HashMap<ViewId,WindowLabel>`(유니크) + `windows: HashMap<Label,WindowTabs{tabs,active}>` + version. 전역 active_view_id·window_bindings **제거**. `OutputRouter.rebuild` = 창의 **모든 탭** 라우팅(keep-alive, active분기 제거). command 개명(create/switch/close_tab·create_window·close_window·move_slot_to_window·list_tabs/list_windows). `move_slot_to_window` 2-phase(기존창 삽입 phase C 이연·재검증 롤백). `cleanup_popup_window` 멀티탭 순회(G1 누수 수정, 코어=`cleanup_window_core` Tauri-free). `view:closed` 엔드투엔드 은퇴. 불변식 1~5 + `assert_invariants` 게이트.
- **프론트:** 단일 `WindowLayout(label)`(main·팝업 통일, D-2) + `TabBar`([+]·탭닫기) + `PopoutPage` `?window=` 껍데기 + `viewStore` 창별·`useCurrentViewId`(main/팝업/agent-tree→main 폴백) + keep-alive(숨은탭 마운트·display:none, 안정 key, 슬롯 no-remount) + Ctrl+Tab(D-8) + `active_view_id` 소비처 이관(AgentTree/SlotContextMenu/resolveDefaultViewId) + `tabCommands.ts`(§5 LLM 경로).

## 검증 상태 (쌍으로 — 안 된 것 명시)
- **돌린 것 + 재실행 명령:**
  - 백엔드: `cargo build --workspace --lib`(clean) · 멤버 회귀 `cargo test -p engram-dashboard-core -p engram-dashboard-protocol -p engram-dashboard-discovery -p engram-dashboard-daemon`(PASS) · `cargo fmt --check` · 격리 `rg "^\s*use tauri" crates/engram-dashboard-core/src`(0줄).
  - 프론트: `npx tsc --noEmit`(clean) · `npm test`(vitest **352**).
  - GUI: qa 바인딩 §full cdp — create/switch/close_tab·create_window(slot-popup-1·`?window=`)·close_window E2E PASS.
- **검증 안 된 것:** src-tauri 순수로직 85 테스트는 **throwaway-mount로만** 검증(아래 do-not). **GUI = smoke 1회(race-free 증명 아님).** 스테이지 5 미착수.

## 실패한 접근 / do-not (재론 금지)
- **★`cargo test -p engram-dashboard --lib` = STATUS_ENTRYPOINT_NOT_FOUND(0xc0000139)★** — Tauri test-exe가 WebView2Loader.dll 로드 실패로 **프로세스 로드 시점 사망**(테스트 본문 미실행 → 결함 은폐 불가). **선재 환경문제**(데몬 죽여도·stash 원본서도 재현). **우회 = throwaway 크레이트에 실소스 `#[path="I:/.../src-tauri/src/layout/manager.rs"]` 등 verbatim 마운트(Tauri 무링크) → `cargo test`.** src-tauri 순수 로직(manager/tree/output_router/popout) 테스트는 다음 세션도 이 방식(리뷰어 2인이 독립 재현). CI/타 PC선 정상 가능.
- **탭 모델 재설계 금지** — D-1 C·D-2·keep-alive·유니크소유 확정·구현·리뷰 완료. `view:closed` 되살리지 말 것. 라우팅=모든탭(active분기 제거됨). `view_owner` 별도맵 유지(스캔파생 거부됨, ADR-0057).

## 정지 조건 (stop conditions)
- **데몬/앱 강제종료 = 사용자 승인 후.** GUI qa로 앱 띄우면 끝나고 dev스택(node tauri.js+vite + engram-dashboard(.exe)+daemon.exe) 정리.
- **dev 로그 프로젝트 폴더 리다이렉트 금지**(vite 무한 reload). bg task output(temp) 안전. cdp 포트 9223 고정.
- **전체 `cargo build`(app bin)/`cargo test`(workspace)** = src-tauri test-exe 0xc0000139. 멤버별 스코프 + throwaway-mount.
- **비자명 코드 = `/implement`**(코더→`/review`→`/qa`), 메인 직접편집 금지. 굵은 결정=ADR.

## 블로커/미결 · 참조 (읽을 것만)
- **미결 없음**(스테이지 1~4 게이트 통과·커밋·push). 스테이지 5는 D-7 확정(Phase 1 편입)이라 바로 착수 가능.
- **정본:** `docs/process/B-wezterm-tabs/TRD.md`(§6 spawn_into·§8 스테이징) · `docs/decisions/0057-*.md`(모델·불변식) · PRD B-tabs(§7 후속 Phase 스케치).
- **코드 포인터:** `src-tauri/src/layout/manager.rs`(모델·불변식·assert_invariants) · `output_router.rs`(라우팅·`cleanup_window_core`) · `commands/layout.rs`·`commands/popout.rs`(move_slot 2-phase·cleanup_popup_window) · `lib.rs`(invoke_handler·Destroyed arm) · `src/components/layout/WindowLayout.tsx`·`TabBar.tsx` · `src/commands/tabCommands.ts` · `src/store/viewStore.ts`(창별·useCurrentViewId).
- **로드맵 순서(스테이지 5 이후):** 3=렌더모드 커맨드화(`setRenderMode`→ADR-0055 레지스트리) · 4=**트리→슬롯(★설계 논의★ 슬롯 콘텐츠 종류 모델 필요)** · 5=트리 정교화 · 6=우클릭 메뉴 command화 · 7=메시지 시스템(`write_input` 재사용).
- **마이너(qa 바인딩 rot):** 격리 게이트 `rg "use tauri" crates/engram-dashboard-core/src/`가 core `lib.rs`의 self-documenting `//!` 주석("...rg \"use tauri\"...")을 false-match → `^\s*use tauri`로 좁혀야 0줄 정확. qa.md 바인딩 갱신 후보(사용자 승인 하).
