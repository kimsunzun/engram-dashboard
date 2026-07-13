# 핸드오프: 에이전트 트리 계층(nesting) C1(백엔드)+C2(프론트) 구현·**로컬 커밋**(ahead 3) — 다음 첫 액션 = **앱 재시작 후 reparent 왕복 E2E 실측** + push(승인)

## 한 줄 상태 · 다음 첫 액션
- **상태:** 에이전트 트리에 **계층(A 밑 B·C·D nesting)** 기능 C1(백엔드 데이터)+C2(프론트 표시)를 구현·커밋(로컬 3커밋 미푸시). 코드상 완성·게이트 통과, **단 reparent 왕복 라이브 E2E만 미검증**(실행 중 데몬이 C1 핸들러 이전 stale 빌드라 30s 타임아웃).
- **다음 첫 액션(순서 중요):** ① **앱 재시작**(`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev`) → 데몬이 `ReparentProfile` 핸들러 포함해 새로 빌드됨 → ② cdp(포트 9223)로 프로필 2개 만들고 `reparentProfile`(또는 ReparentProfile invoke)로 자식→부모 설정, 트리 DOM에 중첩 반영·**재시작 넘어 영속** 확인 → ③ 그 후 **push**(사용자 승인 — master 직접, 내 3커밋만).

## 완료 (이번 세션)
- **(세션 시작) rename 기능 커밋+푸시** `a623278`(지난 세션 게이트통과 미커밋분) + **ADR-0070**(표시명 백엔드 override)·**ADR-0071**(persistence 락 규율) 작성 + step-log.
- **세션 id 복원 검증(사용자 "검증 다해"):** 실 claude `--session-id`→`--resume` **대화 지속 CLI spike PASS**(비밀코드 회상) + 세션파일이 우리 통제 sid로 디스크 영속 확인 + rename 라이브 영속(`.engram-data`에 `display_name:"한글"`) 확인. **미검증 = 앱 경로 데몬재부팅 resume E2E**(구성상 보증, 사용자 직접 확인키로).
- **Slice A** `05df951` — rename 경로 refetch 대칭화(방어) + 우클릭 "예약 취소"→**"삭제"**. **증상4(노드 소실)는 현 빌드서 재현 불가** → 방어 수정이며 확정 fix 아님(사용자: 재현 불필요, 재발 시 피드백).
- **Slice C1** `4b8892c` (ADR-0072) — 백엔드 `AgentProfile.parent_id`(serde default) + `ReparentProfile` command + `ProfileRegistry::reparent`(1단 검증) + 삭제 시 orphan-to-root + **`normalize_hierarchy`**(mutate 경계 불변식 — 어느 write든 dangling/cycle/2단 치유) + **`upsert_preserving_hierarchy`**(spawn stale-snapshot이 parent_id/display_name 덮는 것 봉인 — **ADR-0070 display_name lost-update도 함께 수정**) + protocol/domain/bindings/dispatch.
- **Slice C2** `1377f2f` — 프론트 **react-arborist 부활**(평면→중첩 `<Tree>`·들여쓰기·접기/펼치기) + `mergeTreeNodes` 1단 forest + 드래그 재부모화(`onMove→reparentProfile`, §5 사람·LLM 같은 핸들) + **`disableDrop` 1단 UI 가드**(비루트/자식보유 노드 드롭 차단, review FIX) + 상태 글리프. mergeTreeNodes·client·i18n은 이전 세션 선작업, 이번엔 AgentList 부활+테스트.

## repo 상태
- 브랜치 master, `origin/master`(=사용자 커밋 `c639466`까지) 대비 **ahead 3**(`05df951`·`4b8892c`·`1377f2f` 미푸시 — 전부 내 이번 커밋). 워킹트리 clean(`.tauri-dev-qa.log` untracked·무해 QA 로그).
- **앱 실행 중**: 포트 9223, 그러나 **데몬이 stale**(C1 이전 빌드) — reparent 왕복 안 됨. 재시작 필수.
- 동시 세션 주의: 사용자가 세션 중 커밋(a97cc8c 스킬사본·c639466 handoff)·push함. push 전 `git fetch` ff 재확인.

## 검증 상태 (게이트)
- **돌림(PASS):** `cargo test -p engram-dashboard-core --lib`(**182**) · `-p engram-dashboard-protocol` golden(38) · daemon `--lib`(39) · `cargo fmt --check` · `cargo check --workspace` · 코어격리(`use tauri` 0) · `npx tsc --noEmit` · `npm test`(vitest **568**, 드롭가드 후 agent-dir 60 재확인) · GUI cdp: 중첩 렌더(부모 토글+자식 20px 들여쓰기)·접기/펼치기·글리프 실측.
- **재실행:** Rust는 **member-scoped만**(`-p ...-core`/`-p ...-protocol`/`--lib`) · 프론트 `npx tsc --noEmit`+`npm test` · GUI `node scripts/cdp.mjs eval`(포트 9223 라이브).
- **검증 안 된 것(중요):** ① **reparent 왕복 라이브 E2E** — 데몬 stale, 재시작 후 실측 필요(백엔드 처리는 C1 유닛테스트가 커버) ② **C2 cross-family(Codex) 리뷰 미완**(사용자 중단) — doc-aware 단독 family만(FIX=disableDrop 반영) ③ 실제 마우스 드래그 cdp 시뮬 안 함(onMove 배선·렌더만).

## do-not / 실패한 접근
- **bare `cargo test`·`-p engram-dashboard` = WebView2 0xc0000139 크래시.** member-scoped만.
- **증상4 재현 실패** — 현 빌드 구조상 불가(데몬 전체리스트 broadcast + 프론트 전체교체 setter). 추측 fix 금지 — 재발 시 실 재현 스텝 확보 후.
- **stale 데몬으로 reparent 왕복 검증 시도 = 30s 타임아웃.** 데몬 재빌드(앱 재시작) 먼저.
- **SendMessage 툴 없음** → Claude 코더/리뷰어 이어가기 = fresh 스폰 + 이전 산출·FIX 주입. **Codex만 `codex-reply`(threadId)**. cross-family blind = `mcp__codex__codex`(sandbox read-only·approval never·`config:{model_reasoning_effort:"high"}`).

## Codex-deferred (오케스트레이션 라우팅 단계에서 — 사용자 스코프 결정)
C1 리뷰 잔여 3건, **전부 "두 연결 동시 조작"이라 데이터+표시 트리엔 무관**: (a) 동시 spawn이 삭제된 프로필 되살림 (b) 로드시점 normalize 미적용(새 필드라 실 유입 경로 없음) (c) 데몬 rejected-reparent no-broadcast 테스트 관측성(mock sink가 ConnRegistry 미등록). 실제 동시 오케스트레이션 붙일 때 처리.

## 미결 결정 (사용자)
- **push**(ahead 3 = 내 커밋만, master 직접 = 승인 필요, fetch ff 후).
- **Spawning 글리프 ◐** — 전용 신호(백엔드 상태 or in-flight-spawn 추적) 생기면. 현재 busyIds 다액션이라 미추가(fabrication 회피).
- **다음 큰 단계 = "아주 간단한 오케스트레이션"**(A→B 메시지 보낼 정도만 → 이후 영상촬영용 요청) — 사용자 로드맵. 라우팅 실제 동작은 PRD 미착수(순서 불변 — 구현 전 사용자 결정).

## 정지 조건 (다음 세션)
- 앱 재시작 후 reparent 왕복이 안 되면 → 데몬 로그·`connection_core.rs` ReparentProfile dispatch 확인 후 사용자 보고(임의 대수술 금지).
- push는 사용자 승인 후만(auto push 차단 실측됨).
- 오케스트레이션 라우팅은 PRD/설계 미확정 — 구현 진입 전 사용자 결정.

## 참조 (읽을 것만)
- **설계:** `docs/decisions/0072-*.md`(트리 계층 — 1단·orphan-to-root·글리프·거부 대안) · ADR-0070/0071(동형 additive·락 패턴) · ADR-0018(reserved 머지) · ADR-0008(세션 복원).
- **코드(`파일:심볼`):** `src/components/agent/mergeTreeNodes.ts`(1단 forest) · `AgentList.tsx`(react-arborist `<Tree>`·`onMove`·`disableDrop`·NodeRenderer) · `crates/engram-dashboard-core/src/agent/profile.rs`(`parent_id`·`reparent`·`normalize_hierarchy`·`upsert_preserving_hierarchy`) · `manager.rs`(spawn upsert_preserving) · `crates/engram-dashboard-daemon/src/connection_core.rs`(`ReparentProfile` dispatch).
- **흐름:** `docs/process/step-log.md` 최근 4항목(Slice A·C1·C2·세션id는 rename 항목).
- 미커밋 없음 — `git log`가 정본.
