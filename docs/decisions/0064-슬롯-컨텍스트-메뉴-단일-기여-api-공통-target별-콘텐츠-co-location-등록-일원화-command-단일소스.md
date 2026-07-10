# ADR-0064: 슬롯 컨텍스트 메뉴 = 단일 기여 API (공통 target=별 + 콘텐츠 co-location + 등록 일원화, command 단일소스)

- 상태: 확정 (2026-07-10, 근거: 사용자 결정 + `/research light`(VS Code/JetBrains/Zellij/Obsidian 프리아트 서베이))
- 관련: CLAUDE.md §5(LLM-우선 제어) · ADR-0055/0022(command registry) · ADR-0060(SlotContent 유니온) · ADR-0035(레이아웃 권위=src-tauri) · `src/commands/registry.ts` · `src/components/slot/SlotContextMenu.tsx` · `src/components/layout/ViewLayoutRenderer.tsx` · step-log "슬롯 메뉴 기여" · Amended by ADR-0065 (descriptor 스키마 확장: hideOn 제외조건 + children 1단 서브메뉴 (when-DSL 연기를 hideOn으로 부분 실현))

## 맥락

슬롯 우클릭 메뉴가 두 방식으로 갈려 있었다: ① 제네릭 `SlotContextMenu`(9개 항목 하드코딩, viewStore 직접 호출 — registry 미경유) ② 콘텐츠 컴포넌트(PresetPalette·AgentList)가 자기 메뉴를 따로 만들고 `stopPropagation` 으로 제네릭 메뉴를 **억제**. 결과: 프리셋/트리 슬롯은 공통 슬롯 조작(닫기·분할·팝업·비우기)이 **전부 사라져 닫히지도 않는** 버그. 또 command 등록이 4개 모듈(`themeCommands`·`tabCommands`·`presetCommands`·`agentCommands`)에 흩어져 App.tsx가 각각 side-effect import(로딩 산발).

프리아트(`/research`): VS Code가 정답 모델 — command는 `contributes.commands`에 **한 번만** 정의, 메뉴는 `contributes.menus`에서 **command id 참조만**(재선언 X), 코어 공통 항목과 확장 항목이 같은 contribution point에 공존, `contextValue`(콘텐츠 판별자)+`when`으로 가시성. 우리 command registry(ADR-0055)와 1:1.

## 결정

**슬롯 메뉴 = 단일 기여 API로 조립. 공통도 콘텐츠도 같은 경로로 등록(일관성).**

1. **단일 기여 API** — `registerSlotMenu(target: SlotContentType | '*', items: SlotMenuItem[])`. `SlotMenuItem = { commandId, group, order }`(고정 스키마). `target='*'` = 모든 슬롯(공통). `target=<SlotContentType>` = 그 콘텐츠 타입 슬롯에만.
2. **공통 슬롯 ops = command 등록 + `'*'` 기여(중앙 1파일):** `slot.split.h/slot.split.v/slot.popout/slot.empty/slot.close` 를 registry command 로 승격(현 SlotContextMenu 하드코딩 로직을 command 로 이동) 후 `registerSlotMenu('*', [...])`. 이 command 들은 실행 컨텍스트(viewId·slotId·agentId)를 **run(args)** 로 받아 viewStore 액션을 호출(팔레트·키바인딩·LLM 도 같은 command 를 컨텍스트 인자로 실행).
3. **콘텐츠 기여 = 각 콘텐츠 모듈에서 command 등록 + 메뉴 기여 co-location:** 예) `presetCommands.ts` 가 `preset.add` command 등록 + `registerSlotMenu('preset_palette', [{commandId:'preset.add', ...}])`. agent_list 는 "에이전트 생성"(picker→agent.spawn), empty 슬롯은 fill-ops("에이전트 생성"→spawn+assign / "트리 열기" / "팔레트 열기"), agent 슬롯은 "에이전트 종료"(kill).
4. **등록 일원화(2층):** ㉮ 콘텐츠 내부 = command+메뉴 기여 한 모듈. ㉯ 로딩 = 모든 기여 모듈을 **단일 매니페스트**(`src/commands/contributions.ts` 류)에서 import, 부팅이 그 하나만 로드(App.tsx 산발 import 제거). "새 콘텐츠 = 그 모듈 + 매니페스트 한 줄".
5. **빌더 + 단일 렌더:** `buildSlotMenu(contentType)` 가 `contributionsFor(contentType)` + `'*'` 기여를 **group·order 결정적 정렬**(등록 순서 무관) → command id 를 registry 에서 resolve(title/run) → 단일 메뉴 컴포넌트가 렌더. ViewLayoutRenderer 가 슬롯 pane 우클릭에서 이걸 연다.
6. **콘텐츠 컴포넌트는 pane 메뉴를 소유하지 않음:** PresetPalette·AgentList 의 자체 pane 메뉴 + `stopPropagation` 제거 → pane 우클릭이 통합 메뉴로 버블. **단 AgentList 의 행(row)-레벨 메뉴는 item-targeted 라 별개 유지**(VS Code `view/item/context` vs `view/context` 구분) — 행 우클릭만 stopPropagation.
7. **fail-loud + 타입 안전:** 기여한 `commandId` 가 registry 에 없으면 dev 에러(오타·미등록 즉시 발각). `target` 은 `SlotContentType | '*'` 타입이라 없는 타입 기여 = 컴파일 에러.

## 거부한 대안

- **콘텐츠-소유 메뉴 유지(현 상태)** — 각 콘텐츠가 자기 메뉴+stopPropagation → 공통 ops 소실(바로 이 버그). 공통을 각 콘텐츠에 복붙하면 중복·drift. 거부.
- **중앙 Record `Record<SlotContentType, id[]>` (B안)** — 한 파일에 전부(조망·exhaustive 체크 장점)지만 새 콘텐츠 추가 시 중앙 편집 필요 → 콘텐츠 응집도 저하. 사용자 결정 = 분산 기여(A안, "각 콘텐츠는 각 콘텐츠에서"). 거부.
- **JetBrains식 명령형 `update()`(항목마다 가시성 코드)** — 선언 아님·테스트 난해, 현 "각자 메뉴 빌드→공통 소실" 문제와 동형(재현 위험). 거부.
- **when 표현식 DSL 지금 도입** — 가시성은 `target`(=content.type 판별자) 하나로 MVP 충분(VS Code `contextValue` 결). 복합 조건 필요 시 descriptor 에 `when?` **추가만** 하게 열어두되 지금은 미도입(ADR-0060 유니온 확장 철학 = over-engineering 회피). 거부(연기).

## 근거

- **§5·ADR-0055 정합:** 메뉴 항목 = command 단일소스 참조 → 사람 클릭·키바인딩·LLM(`__engramCmd.run`)이 같은 id·핸들러. "UI 먼저 제어 나중" 갭 해소.
- **프리아트 검증(VS Code):** command 한 번 정의 + 메뉴는 참조, 공통/확장 공존, contextValue 판별 = 검증된 정석. 우리 registry 에 그대로 이식.
- **일관성(사용자 핵심 요구):** 공통이든 콘텐츠든 **같은 `registerSlotMenu` API·같은 descriptor·같은 타이밍(import 부수효과)·단일 매니페스트 로딩** → 등록부 완전 일원화.

## 영향 / 불변식

- **슬롯 메뉴 항목은 반드시 command id 참조** — 메뉴에서 직접 store 호출 금지(현 SlotContextMenu 직접 호출 패턴 폐기). 새 항목 = command 등록 + `registerSlotMenu` 기여 두 단계, 공통(`'*'`)·빌더·메뉴 컴포넌트는 안 건드림.
- **공통 ops 는 `'*'` 기여 단일소스** — 콘텐츠가 공통을 재선언하지 않는다(재선언 = drift, 리뷰 reject).
- **등록 일원화** — 콘텐츠 기여 모듈은 단일 매니페스트에서만 로드(산발 import 금지). 새 콘텐츠 추가 = 매니페스트 한 줄.
- **row-level 메뉴는 예외** — AgentList 행 메뉴는 item-targeted 라 pane 통합 메뉴와 별개(이 분리는 의도 — 혼동 금지).
- **fail-loud** — 미등록 commandId 기여 = dev 에러. target 오타 = 컴파일 에러.
