// TRD S21-slot-docking §4-1 · §4-2 · 6.3판 §5-1(C1 · C2 · C3): 탭바 안 순서 이동의 순수 기하 — Chromium 데스크톱 탭바 끌기
//   (`DraggingTabsSession::MoveAttachedImpl` · `TabStrip` 의 `GetInsertionIndexForDraggedBounds` → `CalculateInsertionIndex`)를
//   탭 하나 · 고정 탭 없음 · 그룹 없음 · 분할 없음 · 겹침 0 인 우리 경우로 그대로 옮겼다. 규칙을 고안해 넣지 말 것(Q23).
//   ★TRD 문장과 Chromium 소스가 다르면 소스를 따른다★(메인 결정 2026-10-18 — 「Chrome 소스 그대로」).
// Chromium 소스는 BSD-style 라이선스다(「Copyright The Chromium Authors」 · 그 LICENSE) — 옮긴 것은 알고리즘이고 코드 문장이
//   아니다. 줄 번호 = chromium/src main(2026-10-18 받음): `tab_strip.cc` · `tab_container_impl.cc` = `chrome/browser/ui/views/tabs/` ·
//   `dragging_tabs_session.cc` = `chrome/browser/ui/views/tabs/dragging/` · `tween.cc` · `animation.cc` = `ui/gfx/animation/` ·
//   `bounds_animator.h` = `ui/views/animation/`.
// 좌표 = CSS px · 끌기 영역 왼쪽(= 첫 탭 왼쪽)이 0. 반올림은 잡은 오프셋 · 문턱 두 곳뿐이고 나머지는 부동소수 그대로다(TRD S21-slot-docking 6.3판 §5-1).

import { TAB_STANDARD_W, type TabStripLayout } from './layout'

/** 순서 문턱의 기준값 — 표준 폭 탭에서 이만큼. Chrome `kHorizontalMoveThreshold`(dragging_tabs_session.cc#89). */
export const REORDER_THRESHOLD_PX = 16

/**
 * 순서가 바뀐 뒤 끄는 탭 밖의 탭이 새 자리로 가는 시간(ms). Chrome `TabContainerImpl::AnimateViewTo` 의
 * `RichAnimationDuration(200 ms)` — 풍부한 애니메이션이 꺼지면 0(tab_container_impl.cc#1153-1160 · animation.cc#114-116).
 */
export const SLIDE_MS = 200

/**
 * 비켜남 곡선 = Chrome `BoundsAnimator` 기본 tween `EASE_OUT` `1 − (1 − t)²`(bounds_animator.h#205 · tween.cc#72-73)을 CSS 로
 * 옮긴 것 — 2차 곡선을 3차로 올린 정확한 꼴이다(제어점 1/3 · 2/3 을 넷째 자리에서 끊음).
 */
export const EASE_OUT_CSS = 'cubic-bezier(0.3333, 0.6667, 0.6667, 1)'

/** 탭 하나의 이상적 경계 `[x0, x1)` — 계산 폭의 누적(Chrome `GetIdealBounds` 꼴 · 숨긴 탭도 보이는 끝 너머에 제 폭으로 선다). */
export interface TabBounds {
  x0: number
  x1: number
}

/** 한 탭바의 이상적 경계 — `idealBounds` 가 만든다. */
export interface StripBounds {
  /** 모든 탭(지금 모델 순서 · 끄는 탭 포함). */
  tabs: TabBounds[]
  /** 그려지는 탭 — 레이아웃의 보임 + 끄는 탭. */
  visible: boolean[]
  /** 보이는 마지막 탭의 자리(없으면 −1) — 넣을 자리를 이 안으로 자른다(tab_strip.cc#589-600). */
  lastVisible: number
}

/**
 * `layout.ts` 결과 → 이상적 경계. 경계는 **모든 탭의 계산 폭 누적**이다 — 숨긴 탭도 폭을 지닌 채 보이는 끝 너머에 선다
 * (Chrome 의 이상적 경계 그대로). ★숨긴 탭을 보이는 끝에 폭 0 으로 겹쳐 세우지 말 것★ — 그러면 그 뒤 끄는 탭의 제자리 후보가
 * 숨긴 탭 앞 후보와 같은 x 가 되어 동률에서 앞이 이기고, 끄는 탭이 가만히 있어도 한 칸 앞으로 옮겨진다.
 * `dragged` = 끄는 탭의 자리 — 레이아웃이 숨겨도 보이는 탭으로 센다: Chrome 은 끄는 탭을 끌기 영역이 자르지 않는 한 늘 보인다
 * (tab_container_impl.cc#1688-1697).
 */
export function idealBounds(layout: TabStripLayout, dragged?: number): StripBounds {
  let x = 0
  const tabs = layout.widths.map(w => {
    const b = { x0: x, x1: x + w }
    x += w
    return b
  })
  const visible = layout.widths.map((_, i) => layout.visible[i] || i === dragged)
  return { tabs, visible, lastVisible: visible.lastIndexOf(true) }
}

/**
 * 탭 끌기 영역 폭 `A` — 끄는 탭은 탭들이 차지한 범위가 아니라 이 폭 안 어디로든 간다. `available` = `layout.ts` 가 받는
 * 「머리들이 쓸 수 있는 폭」의 **잰 값** — ★닫기 모드의 고정 폭을 넣지 말 것★(Chrome 주석 「needs to ignore tab closing mode」).
 * 잰 값이면 그대로다 — Chrome 의 `max(쓸 수 있는 폭, 탭바 폭)` 에서 탭바 폭은 늘림 상한이 선호 폭이라 쓸 수 있는 폭을 넘지
 * 않는다(horizontal_tab_strip_region_view.cc#205-209). `null` · 유한하지 않은 수 = 재지 못함 → 보이는 탭 폭 합.
 */
// Chromium(BSD) `TabStrip::GetTabDragAreaWidth`(tab_strip.cc#530-544)를 옮김.
export function dragAreaWidth(available: number | null, B: StripBounds): number {
  if (available !== null && Number.isFinite(available)) return available
  return B.tabs.reduce((s, b, i) => s + (B.visible[i] ? b.x1 - b.x0 : 0), 0)
}

/**
 * 자기 탭바 순서 이동에서 끄는 탭이 갈 수 있는 폭 — `dragAreaWidth` 를 보이는 탭 폭 합(마지막 자리의 오른쪽 끝)으로 자른다.
 * `draggedLeft` 의 `A` 로 넘긴다. 마지막 자리의 이상적 x(= 합 − 끄는 탭 폭)에 그대로 닿으므로 넣을 자리는 Chrome 그대로다.
 */
// 사용자 결정 2026-10-22: Chrome 은 끌기 영역(`dragAreaWidth`) 끝까지 보내지만 우리는 끄는 탭이 마지막 탭 자리를 넘어 `+` 위로
//   가지 않게 한다(실측 — 탭 둘에서 오른쪽으로 끌면 `+` 너머로 갔다).
export function reorderAreaWidth(available: number | null, B: StripBounds): number {
  return Math.min(dragAreaWidth(available, B), dragAreaWidth(null, B))
}

/**
 * 잡은 오프셋 = `round(r · w0)` — `r` = 잡은 폭 비율(누른 점의 머리 안 자리 ÷ 머리 폭) · `w0` = **끌기를 시작할 때**
 * (dragstart) 끄는 탭의 폭. ★끌기마다 한 번만 셈한다★ — 끄는 중 탭 폭이 바뀌어도 오프셋은 그대로다(Chrome 은 세션
 * 생성자에서 `mouse_offset_` 을 한 번 정한다 — dragging_tabs_session.cc#50).
 */
// Chromium(BSD) `CalculateMouseOffset`(dragging_tabs_session.cc#25-36)을 탭 하나로 옮김.
export function grabOffset(r: number, w0: number): number {
  return roundHalfAway(r * w0)
}

/**
 * 끄는 탭을 그릴 왼쪽 `L`. `p` = 커서 x · `offset` = `grabOffset` · `w` = 끄는 탭의 지금 폭 · `A` = `dragAreaWidth`.
 * 끄는 탭이 끌기 영역보다 넓으면 늘 0 이다(Chrome 주석 · crbug.com/40791711).
 */
// Chromium(BSD) `DraggingTabsSession::GetAttachedDragPoint`(dragging_tabs_session.cc#329-343)를 탭 하나로 옮김.
export function draggedLeft(p: number, offset: number, w: number, A: number): number {
  return Math.min(Math.max(p - offset, 0), Math.max(0, A - w))
}

/**
 * 끄는 탭을 넣을 자리 — **끄는 탭을 뺀 목록 기준**(그 자리로 옮긴 뒤 끄는 탭의 자리 · 같으면 제자리). `B` = 지금 순서의
 * 이상적 경계(끄는 탭 `d` 포함 — `idealBounds(layout, d)`) · `L` = `draggedLeft`. `B.lastVisible` 을 넘지 않는다.
 * 탭이 없으면 0(Chrome) · 아니면 `d` 가 `B` 밖일 때 `RangeError`.
 */
// Chromium(BSD) `CalculateInsertionIndex` · `IsValidInsertionIndex` · `CalculateIdealX` · `GetInsertionIndexForDraggedBounds`
//   (tab_strip.cc#980-1119 · #554-609)를 옮김.
export function insertionIndex(B: StripBounds, d: number, L: number): number {
  const T = B.tabs
  const n = T.length
  if (n === 0) return 0
  if (!Number.isInteger(d) || d < 0 || d >= n) throw new RangeError(`insertionIndex: 끄는 탭 자리 ${d} 가 0..${n - 1} 밖이다`)
  const w = T[d].x1 - T[d].x0

  let best = -1
  let bestDistance = Infinity
  for (let k = 0; k <= n; k += 1) {
    if (k === d + 1) continue
    const idealX = k === 0 ? 0 : T[k - 1].x1 - (k - 1 > d ? w : 0)
    const distance = Math.abs(L - idealX)
    if (distance < bestDistance) {
      bestDistance = distance
      best = k
    }
  }
  if (best > d) best -= 1

  // Chrome: 마지막 보이는 탭 자리 + 1(없으면 0) − 끄는 탭 수(1) 를 0 아래로 내리지 않는다.
  return Math.min(best, Math.max(0, B.lastVisible + 1 - 1))
}

/**
 * 순서를 바꾸기 전에 커서가 움직여야 하는 거리 — **지금 자리 `to` 에 있는 탭**(끄는 탭이 아닐 수 있다)의 폭에 비례한다.
 * `to` 가 `B` 밖이면 `RangeError`.
 */
// Chromium(BSD) `DraggingTabsSession::MoveAttachedImpl` 의 문턱(dragging_tabs_session.cc#89-94)을 옮김.
export function reorderThreshold(B: StripBounds, to: number): number {
  const T = B.tabs
  if (!Number.isInteger(to) || to < 0 || to >= T.length) {
    throw new RangeError(`reorderThreshold: 자리 ${to} 가 0..${T.length - 1} 밖이다`)
  }
  return roundHalfAway(((T[to].x1 - T[to].x0) / TAB_STANDARD_W) * REORDER_THRESHOLD_PX)
}

/**
 * 순서 이동을 셈할 때인가 — 붙은 직후는 문턱 없이 · 아니면 마지막으로 자리가 바뀐 점 `pLast` 에서 문턱을 **넘었을** 때.
 * ★부르는 쪽은 `pLast` 를 자리가 실제로 바뀔 때만 옮긴다★ — Chrome 「Make sure the tabstrip actually changed before updating
 * `last_move_attached_context_loc_`」(dragging_tabs_session.cc#143-149). 문턱을 넘을 때마다 적으면 자리가 바뀌는 순간 다시
 * 문턱을 넘어야 해 순서 바뀜이 최대 문턱만큼 늦어진다.
 */
// Chromium(BSD) dragging_tabs_session.cc#101-104 를 탭 하나로 옮김(연속 아님 갈래는 탭 하나에 없다).
export function moveDue(pLast: number, p: number, thr: number, justAttached: boolean): boolean {
  return justAttached || Math.abs(p - pLast) > thr
}

/**
 * 순서가 바뀐 뒤 각 탭이 갈 왼쪽 x(`B′` = 새 순서의 이상적 경계 · `d` = 새 순서에서 끄는 탭 자리). `null` = 비켜 가지 않는
 * 탭 — 끄는 탭(늘 `L` 에 그리고 겹치는 동안 위에 둔다) · 그려지지 않는 숨긴 탭.
 */
export function slideTargets(B: StripBounds, d: number): (number | null)[] {
  return B.tabs.map((b, i) => (i === d || !B.visible[i] ? null : b.x0))
}

/**
 * 비켜남 시간(ms). `snap` = 다른 창 탭바에 막 붙은 첫 셈 — 이웃 탭이 미끄러지지 않고 곧바로 새 자리에 선다(Chrome
 * 「otherwise those tabs will animate to those bounds after attach, which looks flickery/bad」 · dragging_tabs_session.cc#157-162 ·
 * 출처 창의 첫 셈은 `initial_move_` 라 해당 없다). `reducedMotion` = `prefers-reduced-motion: reduce`(Chrome 의 풍부한 애니메이션 꺼짐).
 */
export function slideMs(reducedMotion: boolean, snap: boolean): number {
  return reducedMotion || snap ? 0 : SLIDE_MS
}

/** Chrome `Tween::EASE_OUT`(tween.cc#72-73) — `EASE_OUT_CSS` 가 그리는 곡선. `t` ∈ [0, 1]. */
export function easeOut(t: number): number {
  return 1 - (1 - t) ** 2
}

// TRD S21-slot-docking §4-3 「다른 창」 · W13: 다른 창 탭바 위의 놓을 자리 · 2 px 세로선(VS Code 꼴) — 그 탭바의 탭은 움직이지
//   않는다. 출처 창 자기 탭바는 위 Chrome `insertionIndex` 를 쓴다.

/** 세는 머리 — 보이고 `dragged` 가 아닌 것(지금 순서). 숨긴 머리는 보이는 끝 너머에 서므로 빼면 보이는 끝에서 잘린다(R24). */
function dropHeads(B: StripBounds, dragged: number | null): TabBounds[] {
  return B.tabs.filter((_, i) => B.visible[i] && i !== dragged)
}

/**
 * 다른 창 탭바에 놓을 자리 — 보이는 머리 중 가운데가 `x` 보다 왼쪽인 것의 수(가운데와 같으면 그 머리의 왼쪽 = 세지 않는다).
 * 빈 곳 · 보이는 끝 너머 = 보이는 머리 수(보이는 마지막 머리 다음 자리 — 숨긴 머리 앞)를 넘지 않는다. 결과는 `DropTo.tab_bar`
 * 의 `index`(그 창 탭 목록에 끄는 탭을 끼운 자리)다. `x` = 끌기 영역 왼쪽 0 기준 CSS px. `dragged` = 이 탭바에 든 끄는 탭의
 * 자리 — 세지 않는다(다른 창은 `null`).
 */
export function dropIndex(B: StripBounds, x: number, dragged: number | null): number {
  return dropHeads(B, dragged).filter(b => (b.x0 + b.x1) / 2 < x).length
}

/**
 * `dropIndex` 자리의 세로선 x — 그 자리 왼쪽 머리의 오른쪽 끝(= 오른쪽 머리의 왼쪽 끝 · 머리는 맞붙는다). 맨 앞이면 첫 머리의
 * 왼쪽 끝, 머리가 없으면 0. 선(2 px)은 이 x 를 사이에 두고 왼쪽 머리 끝 1 px + 오른쪽 머리 끝 1 px 로 그린다(VS Code
 * `::after` · `::before`). `index` 는 `dropIndex` 의 답 — 범위 밖이면 가까운 끝으로 자른다.
 */
export function dropMarkerX(B: StripBounds, index: number, dragged: number | null): number {
  const heads = dropHeads(B, dragged)
  if (heads.length === 0) return 0
  const k = Math.min(Math.max(index, 0), heads.length)
  return k === 0 ? heads[0].x0 : heads[k - 1].x1
}

/** Chromium `base::ClampRound` 의 반올림 — .5 는 0 에서 먼 쪽(`Math.round` 는 음수 .5 를 0 쪽으로 보낸다). */
function roundHalfAway(v: number): number {
  return Math.sign(v) * Math.round(Math.abs(v))
}
