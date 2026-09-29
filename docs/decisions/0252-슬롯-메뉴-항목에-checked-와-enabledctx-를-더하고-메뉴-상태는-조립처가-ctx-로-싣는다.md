# ADR-0252: 슬롯 메뉴 항목에 checked 와 enabled(ctx) 를 더하고 메뉴 상태는 조립처가 ctx 로 싣는다

- 상태: 확정 (2026-09-27/29, 근거: TRD §7 #8 + 리뷰 후속 — 아래 「메인 결정」 절 참조)
- 관련: Amends ADR-0064 (SlotMenuItem 고정 스키마에 checked와 enabled 선택 칸 추가 및 새 항목 두 단계 불변식에 조립처 ctx 구독을 더함) · ADR-0065(hideOn·children additive 개정 선례) · ADR-0055/0022(command registry) · `src/commands/registry.ts` · `src/components/slot/SlotContextMenu.tsx` · step-log S21

## 맥락
사용량 슬롯 메뉴에 "표시 여부" 토글(체크 표시)과 항목별 활성/비활성 판정(예: 새로고침 불가능한 벤더는 ⟳ 항목 비활성화)이 필요했다. ADR-0064는 `SlotMenuItem`을 고정 스키마로 못박고, 메뉴 항목이 뷰 스토어를 직접 읽는 것을 금지한다.

## 결정
`SlotMenuItem`에 **`checked`**와 **`enabled(ctx)`**를 additive로 더한다(ADR-0065가 `hideOn`·`children`을 더한 선례를 그대로 따름). 범위는 이 두 칸 추가뿐이다. 메뉴에 보일 상태(예: 사용량 표시 on/off, 벤더별 새로고침 가능 여부)는 항목 자신이 아니라 **조립처(LayoutLeaf 등)가 구독해 `ctx`로 실어준다**.

### 경위 — 메인 결정(사용자 확인 대기)
7c 1차 구현(`b59a7ac`·`664d011`)은 사용량 상태를 메뉴에 흘리기 위해 기존 `Command.when`을 인자 기반으로 넓히는 방향으로 갔다. 코드 리뷰(codex + Claude 2자)가 이를 지적했고, **`Command.when` 확장을 되돌리고 `enabled(ctx)`를 새로 신설하는 쪽으로 바꾼 것은 메인 오케스트레이터의 결정**이며(2026-09-29 세션), **사용자 확인은 아직 받지 않았다**. 되돌림은 `626279c`가 반영했다.

## 거부한 대안
- **title 에 상태 문자 넣기** — title 은 등록(빌드) 시점에 고정돼 런타임 토글을 못 반영한다.
- **토글마다 메뉴 재등록** — 상태가 바뀔 때마다 등록 API를 다시 태워야 한다.
- **기여가 뷰 스토어를 직접 읽기** — ADR-0064 「메뉴에서 직접 store 호출 금지」 위반.
- **(구현 중 시도했다가 되돌린) `Command.when`을 인자 기반으로 확장** — 메인 결정으로 되돌림, 위 「경위」 참조. 사용자 확인 대기.

## 근거
TRD §7 #8 · main-notes(7c 리뷰 FIX 2회 — `626279c`/`59bb49e` 후속).

## 영향 / 불변식
ADR-0064의 "새 항목 = command + registerSlotMenu 등록만"이라는 불변식이 이번 결정으로 일부 휘어졌다 — **LayoutLeaf가 SlotMenu 상태를 구독해 `ctx`로 주입하는 것**도 신규 항목의 정상 경로가 됐다(`usageRefreshable` 같은 범용 `ctx` 필드가 그 통로 — 재사용 여지는 2번째 소비자가 나올 때 다시 본다). 구현 커밋: `b59a7ac`, `664d011`, `626279c`, `d49a93f`, `59bb49e`.
