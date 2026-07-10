# ADR-0067: 슬롯 콘텐츠 배치 = 우클릭 컨텍스트 메뉴 2경로 + 검색 팝업 (focus-then-place 대체)

- 상태: 확정 (2026-07-10, 근거: 사용자 결정(우클릭 2경로 + 검색 팝업 · 스폰=트리 소관) + `/research medium` OSS 서베이(VS Code·JetBrains·tmux·wezterm·i3/sway·듀얼패널 mc/TC·WCAG 2.5.7) + Codex 적대 리뷰 FIX 4건)
- 관련: Amends ADR-0066 (결정 2(focus-then-place 배치) + 결정 5(크로스-윈도우 place 타깃) → 우클릭 컨텍스트 메뉴 배치로 대체) · CLAUDE.md §5(LLM-우선 제어) · ADR-0011(assign_agent) · ADR-0060(SlotContent 유니온) · ADR-0063(set_slot_content) · ADR-0064/0065(슬롯 메뉴 단일 기여 API · descriptor hideOn/children) · ADR-0035(레이아웃 권위) · `src/commands/slotContentCommands.ts` · `src/components/slot/SlotContextMenu.tsx` · `src/components/agent/AgentList.tsx`(openInFocusedSlot) · step-log

## 맥락

ADR-0066 결정 2는 배치를 **focus-then-place**("슬롯 클릭=포커스 → 트리 에이전트 더블클릭 → 포커스 슬롯에 assign")로 정했다. 결정 1(click-to-focus)을 구현·검증한 뒤 실제 배치 흐름을 짜보니 근본 결함이 드러났다:

**에이전트 트리도 한 slot 안에 있어서, 트리를 클릭해 에이전트를 고르는 순간 그 클릭이 트리 slot으로 포커스를 뺏어간다**(click-to-focus = slot 아무 데나 클릭하면 그 slot이 포커스). 그래서 "포커스 슬롯에 배치"가 트리 slot 자신을 가리켜 배치가 깨진다(focus-steal). 이건 focus-then-place의 전제(타깃 포커스가 트리 조작 중에도 보존됨)를 무너뜨린다 — 우클릭("열기")만 포커스를 안 뺏어 유일하게 동작했다.

굵은 §5 배치 제어 표면 재설계라 `/research medium`(OSS 서베이 → 옵션셋)으로 근거를 만들고 사용자가 방향을 골랐다.

## 결정

배치를 **우클릭 컨텍스트 메뉴 2경로 + 검색 팝업**으로 한다. 우클릭은 포커스를 이동시키지 않아 focus-steal이 원천 차단된다.

1. **경로 1 (슬롯 → 에이전트):** slot 우클릭 → "에이전트 모니터링" → **검색 팝업**(command-palette식 — 검색창 + 실행 중 에이전트 필터 목록) → 고르면 **우클릭한 그 slot**에 `assign_agent`. 타깃 = 우클릭한 slot(명시적).
2. **경로 2 (에이전트 → 슬롯):** 트리 에이전트 우클릭 → "열기"(기존 `openInFocusedSlot`) 유지 — 우클릭이라 직전 좌클릭으로 잡아둔 포커스 slot이 보존된다.
3. **스폰 = 트리 소관:** slot 콘텐츠-채움 메뉴에서 "생성"(`slot.createAgentHere`)을 제거한다. 새 에이전트 생성은 트리에서만(reserved 프로필 더블클릭 + agent_list slot 메뉴의 `agentlist.createAgent`).
4. **드래그앤드롭 = 후속.** 같은 assign 위에 마우스 sugar로 나중에 얹는다.
5. **§5 단일 제어 표면:** 배치 코어 = `assign_agent`(view/slot/agent) 하나 — 사람 우클릭·팝업·LLM이 모두 같은 커맨드를 흔든다. 팝업 열기·선택도 command registry에 등록.

## 거부한 대안

- **focus-then-place (ADR-0066 결정 2, 이 ADR가 대체):** 트리 클릭이 포커스를 트리 slot으로 뺏어 "포커스 슬롯에 배치"가 트리 자신을 가리킴(focus-steal). 성숙 툴은 소스(사이드바)를 타깃(에디터 그룹)에서 아키텍처로 분리해 이걸 피하지만, 우리는 균일 slot이라 그 분리가 없다. 우클릭이 focus-steal을 원천 차단하고 더 단순. focus-then-place 폐기.
- **arm-then-drop(집었다 놓기):** 트리 클릭=집기, slot 클릭=놓기. focus-steal은 피하나 데스크톱 표준 idiom이 아니고(모달 armed 상태 + 시각 어포던스·Esc 필요) 우클릭 컨텍스트 메뉴가 더 표준·단순. 거부.
- **크로스-윈도우 last_focused_window 백엔드 추적 (ADR-0066 결정 5):** 우클릭이 타깃을 명시(우클릭한 slot)하므로 "마지막 포커스 = 타깃" 해소가 불필요. Codex: 우연 포커스로 엉뚱한 배치 위험 + 백엔드 상태 추가 비용. 컨텍스트 메뉴가 target-explicit이라 `last_focused_window` 상태 자체를 안 만든다. 불필요로 거부.
- **콘텐츠 종류 역할 태깅(트리를 "소스 = 타깃 아님"으로 특수취급):** VS Code식 소스/타깃 분리를 흉내내려면 slot을 콘텐츠 종류로 특수취급해야 하는데, 이는 "모든 slot 균일" 원칙을 깬다(Codex 지적). 우클릭은 콘텐츠 종류 무관이라 균일성 유지. 거부.
- **드래그-only:** WCAG 2.2 SC 2.5.7(드래그 기능엔 단일 포인터 대안 필수) + ADR-0066에서 이미 후속으로 미룸. 컨텍스트 메뉴(단일 포인터 클릭)가 접근성 경로. 드래그는 나중 sugar. 드래그-only 거부.

## 근거

- **리서치 medium (OSS 서베이 + Codex FIX 4건):** 모든 성숙 툴이 **소스≠타깃 분리**로 소스 클릭이 타깃을 안 덮게 한다. 비-드래그 표준 배치 경로 = **컨텍스트 메뉴 "open in…/send to"**(VS Code "Open to the Side" `Ctrl+Enter`, Windows "Send to", mc `Alt-O`). Codex 보정: ① 포커스와 배치-타깃은 별개 개념 ② WCAG 2.5.7은 키보드 아닌 **단일 포인터** 대안 요구(우클릭 충족) ③ 콘텐츠 종류 특수취급 지양 ④ 크로스-윈도우 last-focus는 우연 배치 위험. **우클릭 컨텍스트 메뉴가 네 지적을 전부 회피**한다.
- **기존 인프라 재사용:** 슬롯 메뉴 단일 기여 API(ADR-0064/0065) + SlotContent/assign(ADR-0060/0011) 위에 메뉴 항목 + 검색 팝업만 얹는다 — 백엔드 신규 없음(`assign_agent` 그대로).
- **click-to-focus(ADR-0066 결정 1) 재해석:** 이미 구현·커밋된 click-to-focus는 폐기가 아니다 — 시각 선택 지시자로 남되 **배치 메커니즘 역할은 벗는다**(배치 타깃은 우클릭한 slot). 원래 정당화(focus-then-place의 토대)만 이 ADR가 대체한다.

## 영향 / 불변식

- **배치 타깃 = 우클릭한 slot(명시)** — 포커스에 의존하지 않는다. `focused_slot_id`는 배치와 분리(Codex: 포커스 ≠ 배치 타깃). `last_focused_window` 백엔드 상태를 만들지 않는다.
- **§5 단일 제어 표면:** 배치 = `assign_agent`(ADR-0011) 하나 — 사람·팝업·LLM 동일. 팝업·메뉴가 이 커맨드를 우회해 별도 배치 상태를 만들면 §5 위반(리뷰 reject).
- **우클릭은 포커스 불변** — 컨텍스트 메뉴/팝업 열기가 `focused_slot_id`를 바꾸면 focus-steal 재발(리뷰 reject).
- **스폰은 트리에만** — slot 콘텐츠-채움 메뉴에 스폰 항목 재추가 금지(생성 = 트리 소관).
- **드래그는 후속** — 얹을 때 같은 assign 경유(별도 상태 금지, §5).
- load-bearing 경로(슬롯 메뉴 기여·팝업·on-select assign)에 `// ADR-0067` 앵커.
