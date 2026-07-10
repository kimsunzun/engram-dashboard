# 핸드오프: Phase 2 탭 스테이지 5(spawn_into) 완료·커밋·푸시(88134e2) — ADR 2건 박제·step-log는 fresh 승계

## 한 줄 상태 · 다음 첫 액션
- **상태:** WezTerm 탭 **스테이지 1~5 전부 완주.** 스테이지 5 = `spawn_into`(D-7 배치 지정 스폰) 구현·적대리뷰 PASS·qa full(GUI 실측 포함) PASS·**커밋 `88134e2`·origin/master 푸시 완료.** 워킹트리 clean.
- **다음 첫 액션:** **ADR 2건 박제(PENDING — 이번 세션 미기록)** + step-log 갱신. 결정 내용은 아래 "미기록 결정"에 보존됨(유실 방지). `/adr new`로 박제 → `docs/decisions/README.md` 인덱스 + `docs/process/step-log.md` 흐름 추가. 그 뒤 로드맵 스테이지 6~ 진입 가능.

## 완료 / repo 상태
- 브랜치 **master**, **origin/master 동기화**(`88134e2`까지 푸시). 워킹트리 clean.
- 이번 세션 커밋(1): `88134e2` feat(layout): spawn_into 배치 지정 스폰 command (D-7·스테이지 5). 6파일 437+/3-.
- **origin = GitHub**(github.com/kimsunzun/engram-dashboard) — 조직의 "코드=GitLab"은 게임 프로젝트 얘기, 이 대시보드는 별도 개인 tooling repo(기존 upstream 그대로).

## 무엇이 됨 (구현 상세 — 재구현 금지)
- **`spawn_into(window, tab?, slot?, backend?, cwd) -> Result<AgentId,String>`** (src-tauri/src/commands/layout.rs, invoke_handler 등록 lib.rs). 합성: 데몬 SpawnByCwd로 스폰(락 밖 await, Spawned reply서 AgentId 캡처) → tab 미지정 시 create_tab → 슬롯 배정. 배치는 단일 임계구역(ADR-0006).
- **`resolve_spawn_slot(view, slot: Option<Uuid>)`** (manager.rs, 순수) + `SpawnSlotError`(SlotOccupied/SlotNotFound/NoEmptySlot). 신규 `tree::first_empty_slot_id`(tree.rs, 전위 a-우선).
- **JS 래퍼** `agent.spawnInto`(src/commands/tabCommands.ts) — UUID 인자 검증(잘못된 tab/slot은 invoke 전 throw).

## 미기록 결정 (★ADR 박제 대상 — 내용 여기 보존★)
- **결정 1a — backend 지연(fail-loud):** spawn_into의 `backend` 인자가 데몬까지 못 감. 데몬 `SpawnByCwd`(crates/engram-dashboard-daemon/src/connection_core.rs:852)는 **무조건 기본 백엔드(현재 셸 `default_shell()`)를 스폰**, backend 선택 wire 없음. → 명시 backend는 **pre-spawn 거부**(claude 포함 — 조용한 오작동 방지). **거부한 대안 = 지금 프로토콜 확장(1b)**. 후속 = 실제 backend 선택 필요 시 protocol crate SpawnByCwd + 데몬 dispatch 확장(별도 ADR).
- **결정 2b — slot=None = 탭의 첫 빈 슬롯:** leftmost/root 한정(a) 아니라 **트리 훑어 첫 빈 슬롯**, 없으면 에러(자동 split/덮어쓰기 X). TRD §6 G9 "빈 root 슬롯" 문구를 이 해석으로 확정. **거부한 대안 = leftmost-root-only(a)**. 신규 `first_empty_slot_id`가 이 결정 구현.
- (두 결정 모두 사용자 승인 — 이번 세션 대화에서 "1-a, 2-b" 확정.)

## 검증 상태 (쌍으로 — 안 된 것 명시)
- **돌린 것 + 재실행:**
  - 적대 리뷰 `/review code full` 2-family(doc-aware reviewer-deep/opus + blind Codex/gpt high). 1R FIX(orphan탭·alive_err·JS검증·테스트) → 반영, 2R Codex PASS·doc-aware FIX(backend 거짓말) → 교정, 폐쇄 **PASS**.
  - qa: member-scoped `cargo test -p engram-dashboard-core -p engram-dashboard-protocol -p engram-dashboard-discovery -p engram-dashboard-daemon`(**327 PASS**) + throwaway verbatim-mount 하네스(**80 PASS** — resolve_spawn_slot/first_empty_slot/spawn_into_assembly 포함) + `cargo build --workspace --lib` + `cargo check -p engram-dashboard --lib` + `cargo fmt --check` + 격리 `rg "^\s*use tauri" crates/engram-dashboard-core/src`(0줄) + `npx tsc --noEmit` + `npm test`(**vitest 352**).
  - **GUI 실측(cdp, PASS):** 앱 띄워 spawn_into 해피패스(탭 1→2→3, 각 탭 root 슬롯에 셸 에이전트 Running·AgentId 반환) + 가드 스모크 2종 fail-loud(backend='claude' 거부·tab없이 slot 지정 거부, 무부작용). 스크린샷 확인. 재실행 = `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev` + `node scripts/cdp.mjs`.
- **검증 안 된 것:** **ADR 미기록**(다음 세션 1순위). GUI = smoke 1회(race-free 증명 아님). backend 선택 경로는 기본(셸)만 동작 — 명시 backend 미구현. src-tauri 순수로직은 throwaway-mount로만(정식 test-exe 불가). cold first build 미재검(target/debug 캐시).

## 관찰 (spawn_into 결함 아님 — 후속 추적)
- **부팅 warm-up 레이스:** 부팅 직후 첫 데몬 command가 "데몬에 연결되어 있지 않음"으로 실패 가능(daemon_connection_state=connected인데 current_cmd_tx 아직 준비 안 됨). **daemon_ensure는 재스폰 안 함(ADR-0021)** — 데몬 down이면 daemon_start 필요. 연결 확보 후 재시도하면 정상. 별도 관찰(step-log/debugging-conventions 후보), 이번 범위 밖.

## 실패한 접근 / do-not (재론 금지)
- **★`cargo test -p engram-dashboard --lib` / 전체 workspace `cargo test` = 0xc0000139(STATUS_ENTRYPOINT_NOT_FOUND)★** — WebView2Loader.dll 로드 실패, **선재 환경문제**(컴파일·링크는 성공, launch만 사망). 우회 = **member-scoped + throwaway verbatim-mount**(`#[path="I:/.../src-tauri/src/layout/{types,tree,manager}.rs"]` 실소스 마운트, Tauri 무링크 → `cargo test`). 하네스는 %TEMP%\engram-layout-harness에 ad-hoc 재생성(미커밋 — 다음 세션도 이 방식).
- **assign_agent / tree::assign_in_tree 덮어쓰기 시맨틱 변경 금지** — move_slot_to_window 등이 의존. 점유 검사는 spawn_into가 resolve_spawn_slot으로 자체 수행.
- **backend를 SpawnByCwd로 우겨넣기 금지** — wire 없음. 프로토콜 확장(1b, deferred) 전엔 명시 backend fail-loud 유지(기본 스폰으로 조용히 대체 금지).
- 탭 모델 재설계 금지(스테이지 1~4 확정·커밋). view:closed 되살리지 말 것.

## 정지 조건 (stop conditions)
- **데몬/앱 강제종료 = 사용자 승인 후.** GUI qa로 앱 띄우면 끝나고 dev스택(node tauri.js+vite + engram-dashboard.exe + daemon.exe + 스폰된 셸 에이전트) 정리.
- **dev 로그 프로젝트 폴더 리다이렉트 금지**(vite 무한 reload). bg task output(temp) 안전. cdp 포트 9223 고정.
- **비자명 코드 = `/implement`**(코더→`/review`→`/qa`), 메인 직접편집 금지. 굵은 결정 = ADR + 사용자 결정.

## 블로커/미결 · 참조 (읽을 것만)
- **블로커 없음.** 미결 = ADR 2건 박제(내용은 위 "미기록 결정") + step-log 갱신. 스테이지 6~ 착수는 그 뒤.
- **정본:** `docs/process/B-wezterm-tabs/TRD.md`(§6 command표·spawn_into 슬롯정책 G9·§8 스테이징) · `docs/decisions/0057-*.md`(탭 소유 모델·불변식).
- **코드 포인터:** `src-tauri/src/commands/layout.rs`(spawn_into·backend 가드·alive_err) · `src-tauri/src/layout/manager.rs`(resolve_spawn_slot·SpawnSlotError) · `src-tauri/src/layout/tree.rs`(first_empty_slot_id) · `src/commands/tabCommands.ts`(agent.spawnInto·UUID 검증) · 데몬 `crates/engram-dashboard-daemon/src/connection_core.rs:852`(SpawnByCwd=셸).
- **로드맵(스테이지 6 이후):** 렌더모드 커맨드화(setRenderMode→ADR-0055 레지스트리) · 트리→슬롯(★설계 논의 — 슬롯 콘텐츠 종류 모델 필요) · 트리 정교화 · 우클릭 메뉴 command화 · 메시지 시스템(write_input 재사용).
