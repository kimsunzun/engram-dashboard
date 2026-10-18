// TRD S21-slot-docking §5-6(R26 · 조사 A10 · E5): 닫기 모드 — 마우스로 탭을 닫으면 탭바가 쓰는 폭을 고정해 다음 탭이 커서
//   밑으로 미끄러져 오게 하고, 마우스가 탭바를 떠나야 다시 배치한다. Chromium 데스크톱 탭바(`TabContainerImpl` 의 닫기 모드 ·
//   `views::MouseWatcher`)를 겹침 0 · 고정 탭 없음 · 그룹 없음 · 왼쪽→오른쪽 탭바로 옮겼다. 규칙을 고안해 넣지 말 것(Q23).
//   ★TRD 문장과 Chromium 소스가 다르면 소스를 따른다★(메인 결정 2026-10-18 — 「Chrome 소스 그대로」).
// Chromium 소스는 BSD-style 라이선스다(「Copyright The Chromium Authors」 · 그 LICENSE) — 옮긴 것은 알고리즘이고 코드 문장이
//   아니다. 줄 번호 = chromium/src main(2026-10-18 받음): tci = `chrome/browser/ui/views/tabs/tab_container_impl.cc` ·
//   `tab_strip.cc` = `chrome/browser/ui/views/tabs/` · `horizontal_tab_strip_region_view.cc` = `chrome/browser/ui/views/frame/` ·
//   `mouse_watcher.cc` · `mouse_watcher_view_host.cc` = `ui/views/`.
// 순수 상태 줄이다 — 시계 · 타이머 · DOM 을 모른다. 시각은 부르는 쪽 시계의 ms 이고, 기한(`deadline`)에 `tick` 을 넣는 것도
//   부르는 쪽이다.

import { TAB_MIN_ACTIVE_W, TAB_STANDARD_W } from './layout'

/** 탭바 아래로 넓히는 여유 — 탭을 옮겨 가며 닫다 세로로 조금 흘러도 나가지 않게. Chrome `kTabStripAnimationVSlop`(tci #1576). */
export const CLOSE_SLOP_BELOW_PX = 40

/** 끝(새 탭 단추) 쪽으로 넓히는 여유 — 모드 중 `+` 로 가는 길에 나가지 않게. Chrome `kTabStripAnimationHSlop`(tci #1580). */
export const CLOSE_SLOP_END_PX = 60

/**
 * 경계 밖으로 움직이거나 창 밖으로 나간 뒤 모드를 나가기까지(ms) — 그 안에 돌아오면 취소. Chrome `kNotifyListenerTime`
 * (mouse_watcher.cc#26) — 창 밖으로 나감도 같은 값이다(`notify_on_exit_time_` 기본값 #109 · 닫기 모드는 바꾸지 않는다 — tci 에
 * `set_notify_on_exit_time` 이 없다).
 */
export const CLOSE_LEAVE_DELAY_MS = 300

/** 사각형 `[x0, x1) × [y0, y1)` — CSS px(그 창 뷰포트). */
export interface Rect {
  x0: number
  y0: number
  x1: number
  y1: number
}

/**
 * 모드를 지키는 경계. `bar` = Chrome 이 지켜보는 탭바 뷰(`TabStrip` — tab_strip.cc#1717-1719)의 사각형 =
 * `[첫 머리 왼쪽, + layoutAvailable(mode, 잰 폭)) × 탭바 줄 높이`. 그 뷰의 폭은 `min(선호 폭, 쓸 수 있는 폭)`이고
 * (horizontal_tab_strip_region_view.cc#205-209 · #988-991 — 늘림 상한 = 선호), 닫기 모드 중 선호 폭 = 고정 폭이다
 * (tci #911-927 · tab_strip.cc#2206-2218). ★잰 탭바 줄 전체가 아니다★ — 고정 폭이 줄 폭보다 좁으면 그 뒤 빈 곳은 밖이다.
 * Chrome 은 애니메이션 중 그 폭을 움직이는 마지막 탭 끝에 맞춘다 — 여기서는 멈춘 상태의 값으로 어림한다. 위 · 시작 쪽은 넓히지
 * 않는다.
 */
// Chromium(BSD) tci #1570-1590 `AddMessageLoopObserver` 의 `Insets::TLBR(0, 0, 40, 60)`(왼쪽→오른쪽) · 반열림 판정
//   mouse_watcher_view_host.cc#30-41 을 옮김.
export function closingBounds(bar: Rect): Rect {
  return { x0: bar.x0, x1: bar.x1 + CLOSE_SLOP_END_PX, y0: bar.y0, y1: bar.y1 + CLOSE_SLOP_BELOW_PX }
}

/** 점이 경계 안인가 — 반열림이라 아래 39 · 끝 쪽 59 는 안 · 40 · 60 은 밖(AC40). */
export function inClosingBounds(bar: Rect, x: number, y: number): boolean {
  const b = closingBounds(bar)
  return b.x0 <= x && x < b.x1 && b.y0 <= y && y < b.y1
}

/**
 * 탭 하나가 빠진 뒤 고정할 폭 = `current` − 빠지는 폭 + 겹침(0). `current` = 지금 고정 폭(모드 중) 또는 들어가는 때의 탭바 폭 ·
 * `widths` = 빠지기 **전** 모든 탭의 계산 폭(`layout.ts` `widths` — 숨긴 탭 포함) · `closed` = 빠진 탭의 자리(마지막 탭이
 * 아니어야 한다 — 마지막이면 Chrome 은 폭을 고치지 않는다 · `stepClosingMode` 가 거른다) · `nextActive` = 빠진 탭이 활성이었으면
 * 다음에 활성이 될 탭의 자리(빠지기 전 자리 · `closed` 와 다르다). 자리가 범위 밖이거나 둘이 같으면 `RangeError`.
 */
// Chromium(BSD) tci #1441-1473 을 옮김. ★비활성이 활성보다 좁은 배치(`kInactiveWidthBelowActiveWidth`)에서 활성을 빼면 빠지는
//   폭 = 다음 활성 탭의 지금 폭★ — 그 탭이 활성 최소 폭으로 늘어나므로 실제로 비는 것은 그 폭이다. 닫힌 폭을 빼면 남은 탭이 더
//   줄어 다음 탭이 커서 밑에 오지 않는다. 그 배치 = 계산 폭 하나라도 활성 최소 폭보다 좁다(`layout.ts` 단계 B · 단계 A 는 모두
//   그 이상).
export function frozenAvailable(current: number, widths: number[], closed: number, nextActive?: number): number {
  const n = widths.length
  if (!Number.isInteger(closed) || closed < 0 || closed >= n - 1) {
    throw new RangeError(`frozenAvailable: 빠진 탭 자리 ${closed} 가 0..${n - 2} 밖이다`)
  }
  const badNext = nextActive !== undefined && (!Number.isInteger(nextActive) || nextActive < 0 || nextActive >= n)
  if (badNext || nextActive === closed) {
    throw new RangeError(`frozenAvailable: 다음 활성 자리 ${nextActive} 가 0..${n - 1} 밖이거나 빠진 자리 ${closed} 와 같다`)
  }
  const inactiveBelowActive = widths.some(w => w < TAB_MIN_ACTIVE_W)
  return current - (nextActive !== undefined && inactiveBelowActive ? widths[nextActive] : widths[closed])
}

/**
 * 닫기 모드. `available` = 고정 폭(`layout.ts` 의 폭 상한 — `layoutAvailable` 로 꺼낸다) · `deadline` = 이 시각(부르는 쪽 시계
 * ms)에 나간다 · `null` = 마우스가 경계 안이라 기한 없음(시간만으로는 끝나지 않는다).
 */
export type ClosingMode = { on: false } | { on: true; available: number; deadline: number | null }

export const CLOSING_OFF: ClosingMode = { on: false }

export type ClosingInput =
  /**
   * 탭 하나가 모델에서 빠졌다. `mouse` = 그 탭의 닫기 단추를 마우스로 눌러 닫았다 — 아니면(키보드 · 명령 · 다른 길) 모드에
   * 들어가지 않고, 모드 중이면 폭만 고친다(Chrome 은 빠지는 길과 무관하게 고친다 — tci #1419-1474). `measured` = 잰 「머리들이
   * 쓸 수 있는 폭」(`null` = 재지 못함) — 들어갈 때 탭바 폭을 셈한다. 나머지 칸 = `frozenAvailable`.
   */
  | { kind: 'closed'; mouse: boolean; widths: number[]; closed: number; measured: number | null; nextActive?: number }
  | { kind: 'move'; x: number; y: number; bar: Rect; now: number }
  | { kind: 'press'; x: number; y: number; bar: Rect }
  /** 마우스가 창 밖으로 나갔다. */
  | { kind: 'leaveWindow'; now: number }
  /** 부르는 쪽 타이머 — `deadline` 에 맞춰 넣는다. */
  | { kind: 'tick'; now: number }
  /** 탭이 더해졌다(`+` · 붙기 · 명령) — 곧바로 나간다(Chrome `StartInsertTabAnimation` · tci #1229-1230). */
  | { kind: 'inserted' }

// Chromium(BSD) mouse_watcher.cc#63-86 `HandleMouseEvent` · mouse_watcher_view_host.cc#19-26 `Contains` 를 옮김.
export function stepClosingMode(mode: ClosingMode, input: ClosingInput): ClosingMode {
  switch (input.kind) {
    case 'closed':
      return onClosed(mode, input)
    case 'move':
      if (!mode.on) return mode
      if (inClosingBounds(input.bar, input.x, input.y)) return mode.deadline === null ? mode : { ...mode, deadline: null }
      // 기한이 걸려 있으면 다시 걸지 않는다 — 300 ms 는 처음 나간 때부터다(mouse_watcher.cc#70 `!HasWeakPtrs()`).
      return mode.deadline !== null ? mode : { ...mode, deadline: input.now + CLOSE_LEAVE_DELAY_MS }
    case 'press':
      if (!mode.on) return mode
      if (!inClosingBounds(input.bar, input.x, input.y)) return CLOSING_OFF
      return mode.deadline === null ? mode : { ...mode, deadline: null }
    case 'leaveWindow':
      if (!mode.on || mode.deadline !== null) return mode
      return { ...mode, deadline: input.now + CLOSE_LEAVE_DELAY_MS }
    case 'tick':
      return mode.on && mode.deadline !== null && input.now >= mode.deadline ? CLOSING_OFF : mode
    case 'inserted':
      return CLOSING_OFF
  }
}

/**
 * 탭 폭 셈의 상한 = 모드 중이면 `min(고정 폭, 잰 폭)` · 아니면 잰 폭(`measured` · `null` = 재지 못함). `layout.ts` 의
 * `widthCap` 으로 넣고(보임은 잰 폭으로 따로 잰다) · `closingBounds` 의 탭바 폭으로도 쓴다.
 */
// Chromium(BSD) tci #1220-1227 `CalculateAvailableWidthForTabs` 를 옮김.
export function layoutAvailable(mode: ClosingMode, measured: number | null): number | null {
  if (!mode.on) return measured
  return measured !== null && Number.isFinite(measured) ? Math.min(mode.available, measured) : mode.available
}

// Chromium(BSD) tci #716-741 `EnterTabClosingMode` · #1419-1474 `UpdateClosingModeOnRemovedTab` · #1258-1272
//   `StartRemoveTabAnimation` 을 옮김. ★Chrome 의 「보이는 탭」은 넘침 숨김이 아니다★ — `IsTabVisible`(tci #1476-1512)은 고정 탭 ·
//   그룹만 보므로 우리에겐 모든 탭이 보인다. 그래서 탭 수 · 마지막 자리 · 빠지는 폭은 숨긴 탭까지 센다.
function onClosed(mode: ClosingMode, input: Extract<ClosingInput, { kind: 'closed' }>): ClosingMode {
  if (!input.mouse && !mode.on) return mode
  const n = input.widths.length
  if (!Number.isInteger(input.closed) || input.closed < 0 || input.closed >= n) {
    throw new RangeError(`stepClosingMode: 빠진 탭 자리 ${input.closed} 가 0..${n - 1} 밖이다`)
  }
  // 탭이 하나 이하면 고정할 뜻이 없다(tci #1423-1430).
  if (n <= 1) return CLOSING_OFF
  // 모드 중이면 지금 고정 폭(tci #1470-1473) · 들어가면 그때 탭바 폭(tci #730-733 `width()`).
  const current = mode.on ? mode.available : entryWidth(n, input.measured)
  // 마지막 탭이면 폭을 고치지 않는다 — 남은 탭이 그 폭 안에서 넓어진다(tci #1432-1437). ★모드를 나가지 않는다★.
  const available =
    input.closed === n - 1 ? current : frozenAvailable(current, input.widths, input.closed, input.nextActive)
  // 고정 폭이 남은 탭의 표준 폭 합 이상이면 더 이상 폭을 묶고 있지 않다(tci #1263-1270).
  if (available >= (n - 1) * TAB_STANDARD_W) return CLOSING_OFF
  return { on: true, available, deadline: mode.on ? mode.deadline : null }
}

/**
 * 모드에 들어가는 때의 탭바 폭(Chrome `width()`) = `min(선호 폭, 잰 폭)` — 모드 밖 선호 폭 = 모든 탭 표준 폭 합
 * (tab_strip_layout_helper.cc `CalculatePreferredWidth`). 재지 못하면 선호 폭.
 */
function entryWidth(n: number, measured: number | null): number {
  const preferred = n * TAB_STANDARD_W
  return measured !== null && Number.isFinite(measured) ? Math.min(preferred, Math.floor(measured)) : preferred
}
