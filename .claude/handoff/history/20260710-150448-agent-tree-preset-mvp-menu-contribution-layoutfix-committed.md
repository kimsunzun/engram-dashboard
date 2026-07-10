# 핸드오프: 에이전트 트리+프리셋 MVP(A~D) + 폴더다이얼로그 + 슬롯메뉴 기여시스템 + 레이아웃 버그수정 — 전부 커밋

## 한 줄 상태 · 다음 첫 액션
- **상태:** 에이전트 트리+프리셋 MVP + 후속 UX/구조 작업까지 **전부 구현·게이트·GUI 실측·커밋 완료**(미커밋 = 이 핸드오프 파일뿐). 앱/데몬 실행 중(재빌드 신규, CDP 9223). ADR 0061~0064 박제.
- **다음 첫 액션:** 새 작업은 **미해결 플래그(아래)** 중 사용자 선택. 사용자는 UI를 하나씩 다듬는 incremental 모드 — 슬롯별 메뉴 curation·프리셋-행 spawn 액션 등이 유력 후보. 급한 것 없으면 대기.

## 무엇이 됨 (커밋 — master 로컬, 푸시 안 함. 이번 세션 15커밋, `30c5303` 위)
- **MVP 3슬라이스:** `570e9b6`/`0311a44` A(백엔드 프리셋 영속 데몬소유 presets.json + SlotContent AgentList/PresetPalette variant · ADR-0061) · `38ba194` B(프론트 프리셋 클라 + PresetPalette) · `03d1733`/`b1715f5` C(AgentTree→AgentList 평면·5-glyph statusGlyph + agent.spawn + 우클릭 + util/basename · ADR-0062).
- **Slice D:** `d4eb05e`/`2661b99` 트리 완전 슬롯화 = set_slot_content 배치 command + 부팅 기본 = 가로분할[AgentList 20% | Empty] + 고정 사이드패널·StatusBar·더미 DiffPanel 제거 · ADR-0063. `8dca0b9` 그 회귀 수정(Allotment CSS import 유실 + ratio→preferredSize %).
- **UX/구조:** `4c17f08` PresetPalette/AgentList 콘텐츠 라벨("프리셋"/"에이전트 트리") · `6a7ac9e` 프리셋 경로추가 = 네이티브 폴더 다이얼로그(@tauri-apps/plugin-dialog + capability dialog:allow-open) · `e2e4309` 그 gen/schemas 정합 · `99c4c53`/`fa3b284` 슬롯 컨텍스트 메뉴 단일 기여 API(ADR-0064) · `aebfa86` 레이아웃 버그2(메뉴 뷰포트 clamp + Allotment pane key 안정화).

## 검증 상태 (쌍으로)
- **돌린 것:** 슬라이스마다 `/review code`(doc-aware Opus + cross-family blind Codex 적대 2인, 각 FIX는 재수정 반영) + `/qa`. 마지막 상태 = `cargo build` 링크 OK · `cargo test -p engram-dashboard-core`/`-p engram-dashboard-protocol` · `cargo fmt --check` · `rg "use tauri" crates/engram-dashboard-core/src/`=0 · `npx tsc --noEmit`=0 · `npm test`=461 · **cdp GUI 스샷 실측**(프리셋 CRUD·spawn→AgentList·부팅 20/80·set_slot_content·통합 메뉴+닫기·메뉴 하단 clamp·분할 시 좌측 ratio 유지). **재실행 = 그대로.**
- **★do-not(재실행 시)★:** bare `cargo test`·`cargo test -p engram-dashboard`/`--lib` = 0xc0000139(WebView2Loader 사망). **member-scoped만.** src-tauri 레이아웃 로직 단위테스트는 이 배리어로 실행 불가 → build + GUI 실측이 정본.
- **검증 안 된 것:** 멀티창(팝아웃) 프리셋/레이아웃 동기화는 단일창만 실측(broadcast 경로는 프로필과 동형·高신뢰). 네이티브 폴더 다이얼로그 실제 선택→createPreset은 OS 창이라 cdp 밖(유닛+사용자 실사용으로 확인, 사용자가 ACProject 프리셋 실제 등록함).

## 미해결 / 플래그 (다음 — 대부분 사용자 결정·incremental)
- **[TaskList #5, 사용자 결정] 영속·ProtocolClient 하드닝(프로필+프리셋 양 경로)** — Codex 지적 공유 잠재이슈: Registry::mutate lock밖 save 순서역전 · Store::save Result없이 log-only(실패해도 Ack) · ws broadcast try_send drop · sendCommand pending 타임아웃 부재 · PROTOCOL_VERSION 미bump. 프리셋만 고치면 미러 비대칭이라 양경로 동시. 근거 = Codex threads 019f482f·019f499b.
- **슬롯별 메뉴 curation(incremental):** 사용자가 슬롯 종류별 메뉴 항목을 하나씩 스펙 예정. 기여 방법 = ADR-0064 `registerSlotMenu(target, [{commandId,group,order}])`(공통='*', 콘텐츠=각 모듈 co-location, 매니페스트 `src/commands/contributions.ts`에 로딩 일원화).
- **AgentList "에이전트 생성" 프리셋-리스트 spawn 이관:** Slice D에서 폴더다이얼로그로 일관화하며 프리셋 선택 spawn 드롭 → "이 프리셋으로 에이전트 생성"을 프리셋-행 액션으로 추가 예정.
- **ctx.viewId null 엣지(저빈도):** 활성탭 없는 순간 공통 slot-ops 무반응(fireAndForget이 로깅). 필요시 disabled 처리.
- **when-DSL(복합 가시성 조건) 미도입** · **Split 드래그 리사이즈→백엔드 ratio 되쓰기 미도입**(현재 ratio는 초기 사이징만) · **monaco 의존성 정리**(DiffPanel 삭제로 미사용) · **CLAUDE.md 의존성 절에 plugin-dialog 미반영**.

## 정지 조건 (stop conditions)
- **실행 중 데몬(PID 40364)+앱(33204) = 재빌드된 신규**(프리셋·set_slot_content·dialog handling 有, CDP 9223). Rust 재빌드하려면 이 프로세스가 exe 락 → **종료·재시작은 사용자 승인 후**(persist 모델). 이번 세션 재시작은 사용자가 승인했음(과거 승인이 미래로 확장되지 않음 — 다시 확인).
- 비자명 코드 = `/implement`(코더→`/review code`→`/qa`), 메인 직접 구현 금지. 굵은 결정 = ADR. 설계 서베이 = `/research`.
- **★레이아웃/시각 변경은 eval 치수 아닌 스크린샷으로 검증★**(이번 세션 교훈 — eval 폭만 재고 스샷 안 봐 split 높이 붕괴 놓침). qa 바인딩이 "eval > 스샷"이라 하나 그건 콘텐츠/동작용, 레이아웃엔 스샷 필수.

## 참조 (읽을 것만)
- **ADR:** 0061(프리셋 데몬소유)·0062(5-glyph 상태)·0063(set_slot_content 배치+부팅분할)·0064(슬롯메뉴 단일기여 API) · 배경 0060(SlotContent 유니온)·0055/0022(command registry)·0035(레이아웃 권위=src-tauri)·0011(agentClient 표면)·0016/0017(에이전트 수명). CLAUDE.md §5.
- **step-log:** `docs/process/step-log.md` — Slice A / B·C / D / 폴더다이얼로그 / 슬롯메뉴 / 레이아웃버그 항목.
- **코드 포인터:** 백엔드 `crates/engram-dashboard-core/src/{agent/preset.rs,persistence/presets.rs}` · protocol 프리셋 wire(`messages.rs`/`domain.rs`) · daemon `connection_core.rs`(preset arms+broadcast) · `src-tauri/src/layout/{tree.rs,manager.rs}`(set_slot_content·부팅분할)·`commands/layout.rs`·`lib.rs`(dialog plugin)·`capabilities/{default,popup}.json`. 프론트 `src/commands/{slotMenu,slotCommands,slotContentCommands,contributions,presetCommands,agentCommands}.ts` · `src/components/slot/{SlotContextMenu,PresetPalette}.tsx` · `src/components/agent/AgentList.tsx` · `src/components/layout/{ViewLayoutRenderer,WindowLayout,AppLayout}.tsx` · `src/api/{agentClient,protocolClient,tauriTransport}.ts` · `src/store/{viewStore,eventBus,agentStore}.ts` · `src/util/basename.ts`.
- **GUI 검증:** `scripts/cdp.mjs`(포트 9223) — eval(DOM/invoke) + shot(스샷). 앱 기동 = `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev`(백그라운드), 콜드스타트 ~1분.
