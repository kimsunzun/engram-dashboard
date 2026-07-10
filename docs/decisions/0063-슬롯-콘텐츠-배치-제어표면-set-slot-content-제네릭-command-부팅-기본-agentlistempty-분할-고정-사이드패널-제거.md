# ADR-0063: 슬롯 콘텐츠 배치 제어표면 = set_slot_content 제네릭 command + 부팅 기본 = AgentList·Empty 분할 (고정 사이드패널 제거)

- 상태: 확정 (2026-07-10, 근거: 사용자 결정("완전 슬롯화") + 레이아웃 권위 grounding)
- 관련: CLAUDE.md §5(LLM-우선 제어)·아키텍처 §5 · ADR-0060(SlotContent 유니온) · ADR-0035(레이아웃 권위=src-tauri) · ADR-0055/0022(command registry) · `src-tauri/src/layout/{manager.rs,tree.rs,types.rs}` · `src-tauri/src/commands/layout.rs` · `src/store/viewStore.ts` · `src/components/layout/AppLayout.tsx`

## 맥락

에이전트 트리(`AgentList`)·`PresetPalette`는 ADR-0060에서 SlotContent variant로 정의됐지만(Slice A~C), 화면엔 여전히 **고정 좌측 사이드패널**로 마운트돼 있었다 — variant는 있으나 **빈 슬롯을 AgentList/PresetPalette로 바꾸는 배치 수단이 없었다**. 기존 레이아웃 command(`assign_agent`)는 슬롯에 *에이전트*만 꽂고, 비-에이전트 콘텐츠를 슬롯에 배치하는 경로가 없었다. 그래서 "사이드패널 = 고정 크롬"이 남아, 트리를 이동·분할·닫기 하거나 LLM이 배치를 제어할 수 없었다(§5 위반: UI가 프론트 고정, 제어 표면 없음).

사용자 결정: **에이전트 트리를 진짜 슬롯으로 완전 전환**(고정 사이드패널 제거). + 별건으로 하단 StatusBar/더미 DiffPanel(S0 뷰-단계 잔재) 제거.

## 결정

1. **`set_slot_content(view_id, slot_id, content: SlotContent)` 제네릭 command 신설** — 슬롯의 콘텐츠를 Empty/Agent/AgentList/PresetPalette 어느 것으로도 바꾸는 단일 command. `assign_agent`(에이전트 전용)의 배치 패턴을 미러하되 content를 직접 받는다. `tree::set_in_tree`(신규, `assign_in_tree` 미러) → `ViewManager::set_slot_content` → `commands/layout.rs` wrapper(lock → mutate → bump_version → router.rebuild → send_subscription_delta → emit `layout:updated`+`window:tabs-updated`) → `generate_handler!` 등록 → 프론트 `viewStore.setSlotContent` → `invoke('set_slot_content')`. **이것이 §5 슬롯 콘텐츠 배치의 LLM/사람 공용 제어 표면.**
2. **부팅 기본 레이아웃 = 가로 분할 [AgentList(좌, 소) · Empty(우)]** — `ViewManager::new()`의 단일 Empty 슬롯 대신, main 창 첫 뷰를 좌측 `AgentList` 슬롯 + 우측 `Empty` 슬롯의 Split로 만든다. 사이드패널이 하던 "트리 좌측 상시 노출"을 슬롯으로 재현하되, 이제 트리 슬롯도 이동·분할·닫기 가능하다.
3. **고정 사이드패널(`Sidebar` in `AppLayout`) 제거** — 좌측 Allotment pane 삭제. AgentList는 부팅 레이아웃의 슬롯으로만 존재(2). `/tree` 전용 창(TreePage)은 유지(별도 창).
4. **StatusBar + 더미 DiffPanel 제거** — 둘 다 S0 뷰-단계 잔재(‘Ready’ 고정 문자열 + `function hello()` 목업, 실기능 0, AppLayout에서만 사용). AppLayout 배선·컴포넌트 파일 제거. 진짜 diff 기능은 필요 시 재구현.

## 거부한 대안

- **variant별 전용 command**(`set_slot_agent_list`, `set_slot_preset_palette` …) — variant가 늘 때마다 command가 폭발(ADR-0060 유니온 확장 철학과 배치). 제네릭 `set_slot_content(content)` 하나가 유니온 전체를 커버하고 새 variant는 자동 흡수.
- **고정 사이드패널 유지 + variant 병행**(Slice C 현상) — 트리가 슬롯이 아니라 프론트 고정 크롬으로 남아 §5(모든 UI가 LLM 제어) 위반 + 이동·분할·팝업 불가. 사용자가 완전 전환 결정.
- **부팅 기본 = 단일 AgentList 슬롯**(분할 없이) — 그러면 터미널/에이전트를 띄울 슬롯이 없어 매번 먼저 분할해야 함(첫 사용 clunky). 좌 트리 + 우 빈 슬롯 분할이 사이드패널 UX를 무손실 대체.
- **프론트 부팅 시 set_slot_content 호출로 트리 주입**(백엔드 기본 대신) — 레이아웃 권위는 src-tauri(ADR-0035)라 기본 레이아웃도 백엔드가 소유해야 일관. 프론트 주입은 권위 이원화 + 부팅 레이스.

## 근거

- **§5 제어 표면 완성:** 슬롯 콘텐츠 배치가 이제 command 하나로 LLM·사람 공용(배경 우클릭 메뉴·`__engramCmd`·invoke 모두 같은 핸들). "UI 먼저 제어 나중" 갭 해소.
- **레이아웃 권위 일관(ADR-0035):** 기본 레이아웃·배치 변경 전부 src-tauri ViewManager 소유 → 프론트는 미러·렌더만.
- **grounding 확인:** `assign_agent`→`assign_in_tree` 패턴이 이미 슬롯 콘텐츠를 mutate + emit → `set_slot_content`는 그 검증된 경로를 제네릭화만.

## 영향 / 불변식

- **슬롯 콘텐츠 변경은 `set_slot_content` 단일 경로**(에이전트 배치는 기존 `assign_agent` 유지 — spawn_into가 씀). 프론트가 슬롯 content를 직접 로컬 변조하지 않는다(백엔드 emit → 미러, ADR-0035).
- **부팅 기본 레이아웃 변경 = 마이그레이션** — 기존 저장된 레이아웃(있다면)과 새 기본의 상호작용 확인 필요(현재 레이아웃 영속은 미구현이라 매 부팅 기본 생성 → 안전).
- **AgentList/PresetPalette는 이제 슬롯 전용** — 고정 사이드패널 마운트 제거. 트리를 닫으면 배경 우클릭 or set_slot_content로 재배치.
- **StatusBar/DiffPanel 제거** — 진짜 diff/상태바가 필요하면 재구현(더미 잔재 부활 아님).
