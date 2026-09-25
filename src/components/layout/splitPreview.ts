// ADR-0227: 구분선 드래그의 순수 계산 — 미리보기 상태와 화해 규칙, 허용 비율 범위, 셸 사각형의 선형 재사상.
//   React·invoke 를 모른다. 토큰 발급·`set_split_ratio` 호출·당김 타이머는 렌더러 쪽 훅이 진다.
// ★프론트에 두 번째 트리 기하를 두지 않는다★ — 미리보기는 셸이 준 사각형을 옮길 뿐이고 트리를 다시 풀지 않는다.
//   끝자리 차이는 확정 스냅샷이 덮는다(TRD §2f).

import type { SlotRect, SplitRatioOutcome, SplitRect } from '../../api/layoutTypes'

/** 칸 최소 크기(CSS px). 화면의 정책 상수이고 유일한 정본이다 — 셸은 `report_ui_metrics` 로 받아 쓴다. */
export const MIN_PANE_PX = 30

/** `Applied` 응답 뒤 그 version 의 스냅샷을 기다리는 시간. 넘기면 `get_view` 를 한 번 당긴다(이벤트 유실 대비). */
export const COMMIT_PULL_DELAY_MS = 500

export type PreviewPhase = 'drag' | 'commit'

/**
 * 뷰당 하나인 미리보기. `ratio` = a 쪽(왼쪽/위) 칸의 몫(ADR-0140).
 * `awaitVersion` 은 `commit` 에서 `Applied` 응답을 받은 뒤에만 있다 — 그 쓰기가 통지된 스냅샷의 version 이다.
 */
export interface SplitPreview {
  token: number
  splitId: string
  ratio: number
  phase: PreviewPhase
  awaitVersion?: number
}

export type PreviewState = SplitPreview | null

/**
 * `token` = 호출자가 드래그마다 새로 뽑는 값. 응답·실패는 그 드래그의 token 을 싣는다.
 * `response.cachedVersion` = 응답을 받은 순간 이 뷰 캐시의 version.
 * `snapshot` = 이 뷰의 캐시가 새 스냅샷을 채택할 때마다 — `splitIds` 는 그 스냅샷의 분할 id 전부.
 */
export type PreviewEvent =
  | { type: 'dragStart'; token: number; splitId: string; ratio: number }
  | { type: 'dragMove'; token: number; ratio: number }
  | { type: 'dragCancel'; token: number }
  | { type: 'release'; token: number }
  | { type: 'response'; token: number; outcome: SplitRatioOutcome; version: number; cachedVersion: number }
  | { type: 'failure'; token: number }
  | { type: 'snapshot'; version: number; splitIds: ReadonlySet<string> }

/**
 * 화해 규칙(TRD §2f 1~5). null = 미리보기 없음(화면 = 캐시 = 셸 값).
 * - `dragStart` 는 언제나 새 드래그로 갈아 끼운다. 나머지 드래그 이벤트는 token 이 지금 것과 다르면 무시한다(규칙 5).
 * - `drag` 중 `snapshot`: 끌리는 split id 가 사라졌으면 null(드래그 취소), 아니면 유지(규칙 1).
 * - `release` → `commit`. `dragMove`·`dragCancel` 은 `drag` 에서만, `response`·`failure` 는 `commit` 에서만 먹는다.
 * - `response`: `Unchanged`·`TooSmall` → 즉시 null. `Applied` → `awaitVersion = version`, 캐시가 이미 그 이상이면 null(규칙 2).
 * - `commit` 중 `snapshot`: `version >= awaitVersion` 이면 null, 응답 전이거나 미만이면 유지(규칙 2·3).
 * - `failure` → null(규칙 4 — 로그는 호출자가 남긴다).
 */
export function reducePreview(state: PreviewState, ev: PreviewEvent): PreviewState {
  if (ev.type === 'dragStart') {
    return { token: ev.token, splitId: ev.splitId, ratio: ev.ratio, phase: 'drag' }
  }
  if (state === null) return null
  if (ev.type === 'snapshot') {
    if (state.phase === 'drag') return ev.splitIds.has(state.splitId) ? state : null
    return state.awaitVersion !== undefined && ev.version >= state.awaitVersion ? null : state
  }
  if (ev.token !== state.token) return state
  switch (ev.type) {
    case 'dragMove':
      return state.phase === 'drag' ? { ...state, ratio: ev.ratio } : state
    case 'dragCancel':
      return state.phase === 'drag' ? null : state
    case 'release':
      return state.phase === 'drag' ? { ...state, phase: 'commit' } : state
    case 'response':
      if (state.phase !== 'commit') return state
      if (ev.outcome !== 'Applied') return null
      if (ev.cachedVersion >= ev.version) return null
      return { ...state, awaitVersion: ev.version }
    case 'failure':
      return state.phase === 'commit' ? null : state
  }
}

/**
 * 확정 뒤 당김 타이머를 걸어 둘 상태인가 — `commit` 이고 기다리는 version 이 캐시보다 위일 때만 참이다.
 * 응답 전(`awaitVersion` 없음)·무변경 응답은 스냅샷을 기다리지 않으므로 거짓이다.
 */
export function needsPull(state: PreviewState, cachedVersion: number): boolean {
  return (
    state !== null &&
    state.phase === 'commit' &&
    state.awaitVersion !== undefined &&
    state.awaitVersion > cachedVersion
  )
}

/**
 * 끌리는 분할의 허용 비율 범위. null = 잠김(두 쪽 모두 `minPanePx` 를 줄 수 없다 — `L < 2·minPanePx`).
 * `axisPx` = 이 창이 셸에 보고한 정수 캔버스의 그 축 길이, `boxExtent` = 스냅샷 분할 상자의 그 축 폭(`x1 - x0` 또는 `y1 - y0`).
 * ★셸 `ViewManager::set_split_ratio`(`src-tauri/src/layout/manager.rs`) ④ 의 px 클램프와 같은 식·같은 연산 순서다★
 * — 입력이 같으면 비트 단위로 같은 범위가 나와 확정값이 셸에서 다시 잘리지 않는다(뗄 때 튀지 않는다).
 * 식을 바꾸려면 셸과 함께 바꾼다.
 */
export function ratioRange(p: {
  ratioMin: number
  ratioMax: number
  minPanePx: number
  axisPx: number
  boxExtent: number
}): { lo: number; hi: number } | null {
  const edge = p.minPanePx / (p.axisPx * p.boxExtent)
  const lo = Math.max(p.ratioMin, edge)
  const hi = Math.min(p.ratioMax, 1 - edge)
  // 셸은 `lo > hi` 로 빈 범위를 가른다. NaN 도 잠금으로 보내려고 부정형으로 쓴다.
  if (!(lo <= hi)) return null
  return { lo, hi }
}

/**
 * 포인터 위치(루트 기준 정규화, 분할 축)를 분할 상자 안의 비율로 바꿔 `range` 안으로 자른다.
 * 계산이 NaN 이면(상자 폭 0 등) `range.lo` 를 돌려준다 — NaN 이 미리보기·명령으로 새지 않게 한다.
 */
export function ratioFromPointer(p: {
  pointerNorm: number
  box0: number
  box1: number
  range: { lo: number; hi: number }
}): number {
  const r = (p.pointerNorm - p.box0) / (p.box1 - p.box0)
  if (Number.isNaN(r)) return p.range.lo
  return Math.min(p.range.hi, Math.max(p.range.lo, r))
}

type Axis = 'x' | 'y'

/**
 * 미리보기를 셸 사각형에 입힌다 — 끌리는 분할의 상자 안(그 서브트리의 칸·분할)에서 그 축 좌표만 선형 재사상한다.
 * 상자 밖 사각형은 같은 객체를 그대로 돌려준다 — 참조가 같으므로 호출자가 그 칸의 재렌더를 건너뛸 수 있다.
 * 입력 배열은 바꾸지 않는다.
 * 재사상은 좌표 값의 함수라 입력에서 비트 단위로 같던 경계는 결과에서도 같다. 상자 가장자리는 제자리다
 * (경계가 상자 끝에 붙은 퇴화 입력은 예외 — 셸의 쓰기 경로는 그런 분할을 만들지 않는다).
 * `preview` 가 null 이거나 그 split id 가 없거나 비율이 유한하지 않으면 입력을 그대로 돌려준다.
 */
export function remapRects(
  slotRects: SlotRect[],
  splitRects: SplitRect[],
  preview: SplitPreview | null,
): { slotRects: SlotRect[]; splitRects: SplitRect[] } {
  if (preview === null || !Number.isFinite(preview.ratio)) return { slotRects, splitRects }
  const target = splitRects.find(s => s.split_id === preview.splitId)
  if (target === undefined) return { slotRects, splitRects }

  const axis: Axis = target.dir === 'left_right' ? 'x' : 'y'
  const X0 = axis === 'x' ? target.x0 : target.y0
  const X1 = axis === 'x' ? target.x1 : target.y1
  const B = target.at
  // 셸 geometry 의 경계식과 같은 모양(`x0 + (x1 - x0) * ratio`). 상자 밖으로 나가면 이웃을 덮으므로 가둔다.
  const B2 = Math.min(X1, Math.max(X0, X0 + (X1 - X0) * preview.ratio))
  if (B2 === B) return { slotRects, splitRects }

  const map = (v: number): number => {
    if (v === B) return B2
    // 셸이 분할 경계를 상자 안에 엄격히 두므로(`SplitTooDeep`) 두 분모는 0 이 아니다 — 들어오면 움직이지 않는다.
    if (v < B) return B === X0 ? v : X0 + ((v - X0) * (B2 - X0)) / (B - X0)
    return B === X1 ? v : X1 - ((X1 - v) * (X1 - B2)) / (X1 - B)
  }
  const inBox = (r: { x0: number; y0: number; x1: number; y1: number }): boolean =>
    r.x0 >= target.x0 && r.x1 <= target.x1 && r.y0 >= target.y0 && r.y1 <= target.y1

  const nextSlots = slotRects.map(r => {
    if (!inBox(r)) return r
    return axis === 'x' ? { ...r, x0: map(r.x0), x1: map(r.x1) } : { ...r, y0: map(r.y0), y1: map(r.y1) }
  })
  const nextSplits = splitRects.map(s => {
    if (!inBox(s)) return s
    const moved = axis === 'x' ? { ...s, x0: map(s.x0), x1: map(s.x1) } : { ...s, y0: map(s.y0), y1: map(s.y1) }
    // `at` 은 그 분할 자신의 축 좌표다 — 같은 축으로 나뉜 분할의 경계만 움직인다.
    if (s.dir === target.dir) moved.at = map(s.at)
    return moved
  })
  return { slotRects: nextSlots, splitRects: nextSplits }
}
