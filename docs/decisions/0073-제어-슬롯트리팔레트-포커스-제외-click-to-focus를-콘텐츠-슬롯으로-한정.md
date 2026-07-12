# ADR-0073: 제어 슬롯(트리·팔레트) 포커스 제외 — click-to-focus를 콘텐츠 슬롯으로 한정

- 상태: 확정 (2026-07-13, 근거: 실 UX cdp 실측 PASS + 2인 적대 리뷰)
- 관련: CLAUDE.md §5(LLM 제어 표면·손발/두뇌) · **ADR-0066(click-to-focus) 정제** · ADR-0067(우클릭 포커스 불변식) · ADR-0060(SlotContent 유니온) · ADR-0035(레이아웃 백엔드 권위) · `src/components/layout/ViewLayoutRenderer.tsx`(onClick 게이트) · `src/components/agent/selectOpenTarget.ts` · `src/components/agent/AgentList.tsx`(openInFocusedSlot) · step-log

## 맥락
트리 행 우클릭 "열기"는 running 에이전트를 활성 뷰의 포커스 슬롯(`focusedSlotId`)에 배정한다(`assignAgent`). 그런데 슬롯 pane의 click-to-focus(ADR-0066)가 **버블 허용**이라, 트리 노드를 좌클릭하면 그 클릭이 트리 슬롯 pane까지 버블해 **트리 슬롯이 focused가 된다.** 이어 우클릭 "열기"(우클릭은 포커스를 안 건드림 — ADR-0067)가 **그 트리 슬롯을 대상으로 잡아 트리를 에이전트 터미널로 덮어썼다**(빈 슬롯이 따로 있어도). 라이브 DOM으로 재현·확인(사용자 보고).

## 결정
제어 슬롯(`agent_list`=트리, `preset_palette`=팔레트)을 **포커스 개념에서 제외**한다(프론트 전용):
1. **click-to-focus 게이트를 allowlist로** — `ViewLayoutRenderer` onClick이 슬롯 content가 콘텐츠 슬롯(`empty`/`agent`)일 때만 `focusSlot`을 호출하고, 제어 슬롯(및 미래 variant)은 스킵한다. 판별기는 `selectOpenTarget`의 `isContentSlot`를 공유(단일 분류기). 버블 자체는 유지 — 내부 상호작용(트리 버튼·팔레트)은 그대로 발화.
2. **"열기" 대상 선택을 순수 함수로** — `selectOpenTarget(layout, focusedSlotId)`: 포커스가 콘텐츠 슬롯이면 그 슬롯 → 아니면(제어이거나 focus=null) 첫 `empty` 슬롯 → 없으면 `null`(실패 토스트). 이 함수가 **구조적으로 제어 슬롯을 대상에서 배제**하므로, 백엔드 포커스 상태와 무관하게 "열기"가 트리/팔레트를 덮는 일은 불가능하다.

## 거부한 대안
- **열기-지점 필터만(포커스 모델 불변)** — `openInFocusedSlot`에서만 제어 슬롯을 걸러 빈 슬롯 선택. 버림: 포커스 링이 여전히 트리에 뜨고, 방향 이동 등 다른 `focusedSlotId` 소비처가 제어 슬롯에 앉는 불일치가 남는다(근본 원인 = 제어 슬롯이 포커스됨 — 을 안 고침).
- **백엔드 `focus_slot`/`fixup_focus`에서 제어 슬롯 강제 거부(권위 소유자 — ADR-0035)** — 가장 근본적(포커스 링까지 정합). **보류(defer, 사용자 결정 2026-07-13 — 이번엔 프론트만).** Rust 변경 회피. 잔존 = 백엔드가 트리를 focused로 emit하면(기본 main 레이아웃 첫 슬롯 = AgentList) 포커스 링이 트리에 뜨는 **순수 시각 잔존**(동작엔 무영향 — `selectOpenTarget`이 클로버 차단). 백엔드 트리/프리셋 구성 재검토 시 함께 다룬다.
- **게이트 denylist(`agent_list || preset_palette` 나열)** — 버림: 미래 제어 variant(FileTree/ControlPanel, ADR-0060) 추가 시 조용히 포커스 대상이 된다. allowlist(콘텐츠 슬롯만)가 미래 안전(리뷰 nit 반영).

## 근거
- **실 UX cdp 실측(PASS):** 트리 실클릭 후 트리 슬롯 `focused: false`(게이트 동작) · 우클릭→"열기" 실클릭 후 트리 `treeStillPresent: true`(미클로버). invoke smoke가 아닌 실제 DOM 클릭·컨텍스트 메뉴·메뉴 클릭으로 검증.
- **2인 적대 리뷰(다른 family):** doc-aware(worker-senior)=FIX·cross-family(Codex effort high)=PASS — production 로직 정확 합의. FIX 반영(allowlist) + 백엔드 강제=defer. `selectOpenTarget` 순수 함수 단위테스트가 전 분기(포커스=콘텐츠/제어/null·stale·중첩 split) 커버, vitest 598 green.

## 영향 / 불변식
- **click-to-focus는 콘텐츠 슬롯(`empty`/`agent`)만 포커스**한다(ADR-0066 정제 — 제어 슬롯 제외). 판별기 = `isContentSlot` 단일 출처(`selectOpenTarget.ts`). 미래 제어 variant는 allowlist라 자동 비포커스.
- **"열기"(`openInFocusedSlot`)는 `selectOpenTarget`을 거쳐 제어 슬롯을 대상으로 삼지 않는다** — 트리/팔레트 클로버 불가(백엔드 포커스 상태 무관).
- ADR-0067(우클릭 포커스 불변식)·ADR-0035(레이아웃 백엔드 권위 — 낙관 갱신 X) 유지. 포커스 자체는 여전히 `focusSlot`→invoke→emit 권위 루프.
- **미해소(deferred):** 백엔드 `focus_slot`/`fixup_focus`는 제어 슬롯 포커스를 여전히 허용 → 백엔드-지정 포커스 시 트리에 포커스 링이 뜨는 시각 잔존(동작 무영향). 완전 정합은 백엔드 강제가 필요(별도 결정 — 백엔드 트리/프리셋 구성 재검토와 함께).
