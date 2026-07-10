# 핸드오프: 슬롯 콘텐츠 모델(ADR-0060) 설계+seam 구현 완료·푸시(1ec0a79) — follow-up variant은 UX 사용자 결정 대기

## 한 줄 상태 · 다음 첫 액션
- **상태:** 자율 세션 완주. ① 지난 스테이지 5 미기록 ADR-0058/0059 박제(`e7d92c0`) ② **슬롯 콘텐츠 모델 = 타입드 유니온 확정(ADR-0060) + seam 리팩터 구현**(`1d5be9c`→`d694bae`→`1ec0a79`) — review deep 2-family + qa full(cdp 실측) PASS, **origin/master 푸시 완료**. 워킹트리 clean(handoff 파일 제외).
- **다음 첫 액션:** follow-up 착수 전 **사용자 결정 2건** 필요(아래 "미결/사용자 결정"). 그 전까지 코드 진입 금지. 착수 가능한 무결정 항목 = 렌더모드 결정 기록(경미) + capability 표 설계.

## 무엇이 됨 (이번 세션 — 재작업 금지)
- **ADR-0058/0059**(스테이지 5 spawn_into 결정 박제): backend fail-loud · slot=None 첫 빈 슬롯. 코드 앵커·인덱스·step-log 완료.
- **ADR-0060(확정):** 슬롯 콘텐츠 = 타입드 유니온 `SlotContent`(내부태깅 `#[serde(tag="type")]`, LayoutNode와 동일). 거부 P2(view-type 레지스트리=플러그인 앱 관행이나 불투명 state·타입안전 포기)·P3(URI). **관행(P2) 전제(플러그인 생태계)가 우리(백엔드 통제·타입안전)와 달라 소수파 P1.** 근거 = `/research` OSS 서베이(VS Code·Lumino/JupyterLab·Theia·Obsidian·Tabby·tmux·Zellij) + Codex 적대 리뷰.
- **seam 구현(behavior-identical):** `LayoutNode::Slot{agent_id:Option<String>}` → `{content: SlotContent(Empty|Agent{agent_id})}`. 백엔드 `types.rs`(SlotContent+is_empty/agent_id)·`tree.rs`·`manager.rs`·`mod.rs`·`output_router.rs` + ts-rs 재생성(`LayoutNode.ts`+신규 `SlotContent.ts`) + 프론트 `ViewLayoutRenderer.tsx` `switch(content.type)`. resolve_spawn_slot 3-way(ADR-0059)·collect_agents 라우팅(ADR-0041/42/46)·assign 덮어쓰기(ADR-0058) 전부 불변.

## 검증 상태 (쌍으로)
- **돌린 것:** `/review code deep` 2-family — doc-aware(reviewer-deep) **PASS**(라우팅·점유·slot_agent None-vs-missing·ts-rs byte-identity 매크로소스 대조 확인), Codex blind **FIX**(신규 SlotContent.ts git 미추적 — commit에 add로 해소). `/qa full` **PASS**: `cargo build`·member-scoped test(core160·discovery44·protocol42)·`cargo fmt --check`·코어격리 `rg "^\s*use tauri" core`=0·`npx tsc --noEmit`·`npm test`(352) + **throwaway verbatim-mount harness 79**(SlotContent serde golden·resolve_spawn_slot 3-way·first_empty_slot·assign overwrite) + **cdp 실측**(spawn_into→`{type:agent,agent_id}`·빈슬롯→`{type:empty}`+"— empty —" 렌더·tab/split/create_window 무회귀).
- **검증 안 된 것:** cdp 실측 1회 = smoke(race-free 증명 아님). 실제 비-에이전트 variant는 미구현(seam만). 영속화 경로 없음(마이그레이션 미검증 — 애초에 영속화 부재).

## 실패한 접근 / do-not (재론 금지)
- **★`cargo test` bare·`-p engram-dashboard --lib` = 0xc0000139(WebView2Loader launch 사망)★** — 선재 환경배리어(컴파일·링크 OK). 우회 = **member-scoped + throwaway verbatim-mount**(`#[path=".../layout/{types,tree,manager}.rs"]` 실소스 마운트, Tauri 무링크 temp crate). daemon exe 잠금(기존 데몬 프로세스)으로 full re-link 막힘 — daemon 소스 미변경이라 무관.
- **SlotContent 라우팅/점유/덮어쓰기 시맨틱 변경 금지** — collect_agents는 Agent만 라우팅(Empty 무시)·resolve_spawn_slot 3-way·assign 무조건 치환. 리뷰가 이 불변으로 PASS.
- **SlotContent = leaf-only(현재)** — 중첩 레이아웃(콘텐츠가 자체 탭/분할)은 미결. variant 추가 시 재검토.
- ts-rs 바인딩 수동편집 원칙이나, 0xc0000139로 export 테스트 실행 불가 → hand-sync + throwaway harness(Tauri 무링크)로 실제 export 돌려 byte-identity 검증하는 게 현 방식.

## 미결 / 사용자 결정 (follow-up 착수 전 필요)
- **① 실제 비-에이전트 variant의 UX** — `FileTree`/`ControlPanel` variant를 실제로 넣으려면 "슬롯 안 트리/버튼셋이 어떻게 보이고 동작하나"가 **사용자 결정(동작·정책)**. AgentTree는 현재 고정 사이드패널(슬롯 콘텐츠 아님) — 슬롯화 여부·범위 결정 필요.
- **② 렌더모드 결정 기록(경미·미기록):** 사용자가 이번 세션서 "렌더모드 = 콘텐츠 종류 기준 내부 디폴트(Agent=xterm/rich·비-에이전트=dom), 사용자 선택권 미노출, 내부 command만 여지" 확정 → **아직 ADR/step-log 미기록.** ADR-0056(렌더모드 command 레버) 부분 amend 또는 step-log 노트로 남길 것(굵기 = 사용자 판단).

## 정지 조건 (stop conditions)
- **데몬/앱 강제종료 = 사용자 승인 후.** 이번 qa 실측서 띄운 dev 앱(PID 34140)은 정리했으나, **기존 데몬(PID 22832) + qa spawn_into로 생긴 테스트 셸 에이전트는 존속**(persist 모델 ADR-0015/0021, 강제종료 미승인). 정리 원하면 사용자 확인.
- 비자명 코드 = `/implement`, 굵은 결정 = ADR + 사용자 결정. dev 로그 프로젝트 폴더 리다이렉트 금지, cdp 포트 9223.

## 참조 (읽을 것만)
- **정본:** `docs/decisions/0060-*.md`(SlotContent 결정·불변식) · `docs/process/C-slot-content/TRD.md`(seam 계획·follow-up "Out" 목록·수용기준).
- **코드 포인터:** `src-tauri/src/layout/types.rs`(SlotContent) · `output_router.rs`(collect_agents 라우팅 불변) · `manager.rs`(resolve_spawn_slot) · `src/components/layout/ViewLayoutRenderer.tsx`(content.type switch) · `src-tauri/bindings/SlotContent.ts`.
- **후속 요건(ADR-0060 불변식):** capability 표 · 영속화 시 version/Unknown/migration · 중첩 레이아웃.
