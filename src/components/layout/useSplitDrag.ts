// ADR-0227: 구분선 드래그 배선 — 미리보기 상태(`splitPreview.ts`)에 제스처 token 발급·`set_split_ratio` 확정·
//   확정 뒤 당김 타이머·허용 범위를 붙여 `Splitter` props 로 내준다. 화해 규칙의 정본은 `reducePreview` 다(TRD §2f).
//   여기서 더하는 판정은 둘이다 — 떼는 순간 미리보기를 이미 다른 드래그가 가져갔으면 확정하지 않는다, 미리보기와
//   확정의 응답·당김을 그것을 시작한 뷰에 묶는다.
// 프론트 명령 레지스트리(`window.__engramCmd`)엔 올리지 않는다 — LLM 경로는 버스 `split.setRatio` 하나다.

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  useSyncExternalStore,
  type RefObject,
} from 'react'
import { invoke } from '@tauri-apps/api/core'

import type { SlotRect, SplitRect, ViewSnapshot } from '../../api/layoutTypes'
import { useViewStore } from '../../store/viewStore'
import { retryAsync, RetryCancelledError } from '../../util/retryInvoke'
import type { SplitterProps } from './Splitter'
import {
  COMMIT_PULL_DELAY_MS,
  MIN_PANE_PX,
  needsPull,
  ratioRange,
  reducePreview,
  remapRects,
  type PreviewEvent,
  type PreviewState,
} from './splitPreview'
import { getCanvasReport, subscribeCanvasReport, type CanvasPx } from './windowCanvasReport'

/**
 * 입력은 전부 이 뷰의 캐시(`CachedView`) 그대로다 — 셸 스냅샷이 권위다.
 * ★`slotRects`·`splitRects` 는 캐시의 배열을 그대로 넘겨야 한다(MUST) — 정렬·복사한 배열을 넘기지 말 것.★
 *   배열 참조가 바뀌면 새 스냅샷으로 보고 화해하고, 결과의 「바뀌지 않은 사각형·배열은 입력과 같은 참조」 보장도
 *   입력 참조가 안정해야 선다. 그리는 순서(id 순)가 필요하면 이 훅의 *결과*를 정렬한다.
 */
export interface UseSplitDragArgs {
  viewId: string
  slotRects: SlotRect[]
  splitRects: SplitRect[]
  ratioMin: number
  ratioMax: number
  /** 캐시 version — 바뀌면 새 스냅샷으로 보고 화해한다. */
  version: number
  /** 평평한 루트. 캔버스 보고 전에는 허용 범위의 축 길이를 여기서 잰다. */
  rootRef: RefObject<HTMLElement | null>
}

export interface UseSplitDragResult {
  /** 그릴 사각형 — 미리보기 재사상을 입힌 값. 바뀌지 않은 사각형·배열은 입력과 같은 참조다. */
  slotRects: SlotRect[]
  splitRects: SplitRect[]
  /**
   * `split` = 위 `splitRects` 의 한 항목(재사상된 값). `<Splitter key={split.split_id} …/>` 에 그대로 펼친다.
   * `range` 는 스냅샷에 그 split id 가 없으면 null(잠김)이다.
   */
  splitterProps(split: SplitRect): SplitterProps
}

type Range = ReturnType<typeof ratioRange>

const readCanvas = (): CanvasPx | null => getCanvasReport().lastSent

/** 미리보기와 그것이 속한 뷰. 이벤트는 자기를 낸 뷰를 달고 오고, 지금 뷰가 아닌 것은 버린다. */
interface Scoped {
  viewId: string
  preview: PreviewState
}

type ScopedAction = { type: 'event'; viewId: string; ev: PreviewEvent } | { type: 'reset'; viewId: string }

// 바뀌는 것이 없으면 같은 객체를 돌려준다 — `send` 가 그것으로 헛 디스패치를 거른다.
function reduceScoped(s: Scoped, a: ScopedAction): Scoped {
  if (a.type === 'reset') {
    return s.viewId === a.viewId && s.preview === null ? s : { viewId: a.viewId, preview: null }
  }
  if (a.viewId !== s.viewId) return s
  const preview = reducePreview(s.preview, a.ev)
  return preview === s.preview ? s : { viewId: s.viewId, preview }
}

const initScoped = (viewId: string): Scoped => ({ viewId, preview: null })

/**
 * 뷰 하나의 구분선 드래그 — 미리보기는 뷰당 하나다.
 * - 떼면 `set_split_ratio` 를 한 번 부른다. 단 그 사이 다른 구분선의 새 드래그가 미리보기를 가져갔으면(두 포인터)
 *   떼어도 확정하지 않는다 — 화면에 보이던 값은 이미 그 드래그의 것이라, 보내면 보이지 않던 값이 확정된다.
 * - 더블클릭은 미리보기 없이 0.5 를 보낸다.
 * - `Applied` 뒤 `COMMIT_PULL_DELAY_MS` 안에 그 version 의 스냅샷이 오지 않으면 `get_view` 를 한 번 당긴다.
 * - 허용 범위의 축 길이 = 이 창이 셸에 보고한 정수 캔버스(보고 전이면 루트 크기를 반올림) × 스냅샷 분할 상자 폭.
 *   셸 px 클램프와 같은 입력이라 확정값이 셸에서 다시 잘리지 않는다.
 * - 같은 인스턴스에서 `viewId` 가 바뀌면 미리보기와 진행 중 제스처를 버린다. 옛 뷰에서 시작한 확정의 응답·당김은
 *   새 뷰에 닿지 않는다.
 */
export function useSplitDrag(args: UseSplitDragArgs): UseSplitDragResult {
  const { viewId, slotRects, splitRects, ratioMin, ratioMax, version, rootRef } = args

  const [scoped, dispatch] = useReducer(reduceScoped, viewId, initScoped)
  // 뷰가 막 바뀐 렌더(아래 초기화 effect 전)에도 옛 뷰의 미리보기를 그리지 않는다.
  const preview = scoped.viewId === viewId ? scoped.preview : null
  // 보낸 액션을 곧바로 접은 값 = React 가 결국 그릴 상태다(같은 순수 함수·같은 순서). 렌더보다 앞서 있는 것이
  //   목적이다 — 떼는 순간 「미리보기가 아직 이 제스처의 것인가」를 렌더를 기다리지 않고 판정한다.
  // ★`dispatch` 를 직접 부르지 말고 늘 `send` 를 지날 것★ — 하나라도 우회하면 그 소유 판정이 조용히 어긋난다.
  const latest = useRef<Scoped>(scoped)
  const send = useCallback((a: ScopedAction) => {
    const next = reduceScoped(latest.current, a)
    // 바꾸는 것이 없는 액션은 보내지 않는다 — 순수 함수의 순서 접기에서 무변화 단계를 빼도 뒤 상태는 같다.
    if (next === latest.current) return
    latest.current = next
    dispatch(a)
  }, [])

  const nextToken = useRef(0)
  // split id → 그 구분선의 진행 중 제스처 token. 구분선마다 따로 쥐어야 두 포인터의 겹친 드래그나 언마운트 취소가
  //   남의 미리보기를 건드리지 않는다.
  const gestures = useRef(new Map<string, number>())

  // 당김의 취소 판정에만 쓴다. 확정 응답은 이것으로 막지 않는다 — 효과만 끊기고 상태는 사는 경우
  //   (`<Activity mode="hidden">`) 응답을 버리면 미리보기가 `commit` 에 갇힌다.
  const mounted = useRef(false)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  // 스냅샷 화해보다 먼저 돈다(선언 순서) — 바뀐 뷰의 스냅샷을 옛 미리보기에 먹이지 않게.
  useLayoutEffect(() => {
    if (latest.current.viewId === viewId) return
    gestures.current.clear()
    send({ type: 'reset', viewId })
  }, [viewId, send])

  // 응답이 올 때 읽는 캐시 version — 떼던 순간의 값이 아니라 그 뒤 채택된 스냅샷까지 반영한 값이어야 한다.
  const cachedVersion = useRef(version)
  // layout effect 인 이유: 새 스냅샷 위에 낡은 미리보기가 한 프레임 그려지지 않게 페인트 전에 화해한다.
  useLayoutEffect(() => {
    cachedVersion.current = version
    const cur = latest.current
    // 미리보기가 없으면 스냅샷은 상태를 바꾸지 않는다 — split id 집합을 만들 것도 없다.
    if (cur.viewId !== viewId || cur.preview === null) return
    send({
      type: 'event',
      viewId,
      ev: { type: 'snapshot', version, splitIds: new Set(splitRects.map(s => s.split_id)) },
    })
  }, [viewId, version, splitRects, send])

  const pullToken = preview !== null && needsPull(preview, version) ? preview.token : null
  useEffect(() => {
    if (pullToken === null) return
    const timer = setTimeout(
      () => pullView(viewId, () => !mounted.current || latest.current.viewId !== viewId),
      COMMIT_PULL_DELAY_MS,
    )
    return () => clearTimeout(timer)
  }, [pullToken, viewId])

  const canvas = useSyncExternalStore(subscribeCanvasReport, readCanvas)
  const [measured, setMeasured] = useState<CanvasPx | null>(null)
  // 캔버스 보고 전에만 잰다. 렌더마다 다시 재지만 크기가 같으면 상태를 갈지 않는다.
  // 알려진 한계: 이 훅의 호스트가 다시 그려지지 않는 사이의 루트 크기 변화는 못 본다 — 보고가 오면(디바운스 100ms) 그쪽을 쓴다.
  useLayoutEffect(() => {
    if (canvas !== null) return
    const el = rootRef.current
    if (el === null) return
    const r = el.getBoundingClientRect()
    const next = { w: Math.round(r.width), h: Math.round(r.height) }
    setMeasured(prev => (prev !== null && prev.w === next.w && prev.h === next.h ? prev : next))
  })
  const axis = canvas ?? measured
  const axisW = axis?.w
  const axisH = axis?.h

  const display = useMemo(
    () => remapRects(slotRects, splitRects, preview),
    [slotRects, splitRects, preview],
  )

  const ranges = useMemo(() => {
    const m = new Map<string, Range>()
    for (const s of splitRects) {
      const leftRight = s.dir === 'left_right'
      const axisPx = leftRight ? axisW : axisH
      m.set(
        s.split_id,
        axisPx === undefined
          ? null
          : ratioRange({
              ratioMin,
              ratioMax,
              minPanePx: MIN_PANE_PX,
              axisPx,
              // 재사상 전 스냅샷 상자 — 셸과 비트 단위로 같은 입력이어야 한다.
              boxExtent: leftRight ? s.x1 - s.x0 : s.y1 - s.y0,
            }),
      )
    }
    return m
  }, [splitRects, ratioMin, ratioMax, axisW, axisH])

  const splitterProps = useCallback(
    (split: SplitRect): SplitterProps => {
      const splitId = split.split_id
      // 이 구분선을 그린 뷰를 단다 — 뷰가 바뀐 뒤 도착한 것(옛 확정의 응답 등)은 `reduceScoped` 가 버린다.
      const toView = (ev: PreviewEvent): void => send({ type: 'event', viewId, ev })
      return {
        split,
        range: ranges.get(splitId) ?? null,
        rootRef,
        onDragStart(ratio) {
          nextToken.current += 1
          const token = nextToken.current
          gestures.current.set(splitId, token)
          toView({ type: 'dragStart', token, splitId, ratio })
        },
        onPreview(ratio) {
          const token = gestures.current.get(splitId)
          if (token !== undefined) toView({ type: 'dragMove', token, ratio })
        },
        onCancel() {
          const token = gestures.current.get(splitId)
          if (token === undefined) return
          gestures.current.delete(splitId)
          toView({ type: 'dragCancel', token })
        },
        onRelease(ratio) {
          const token = gestures.current.get(splitId)
          if (token === undefined) return
          gestures.current.delete(splitId)
          const cur = latest.current
          const owns =
            cur.viewId === viewId &&
            cur.preview !== null &&
            cur.preview.token === token &&
            cur.preview.phase === 'drag'
          toView({ type: 'release', token })
          if (!owns) return
          // 언마운트 여부로 응답을 막지 않는다(위 `mounted` 주석). 진짜 언마운트 뒤의 디스패치는 React 가 무시한다.
          useViewStore
            .getState()
            .setSplitRatio(viewId, splitId, ratio)
            .then(
              res =>
                toView({
                  type: 'response',
                  token,
                  outcome: res.outcome,
                  version: res.version,
                  cachedVersion: cachedVersion.current,
                }),
              err => {
                console.error(`[useSplitDrag] set_split_ratio(${splitId}, ${ratio}) 실패:`, err)
                toView({ type: 'failure', token })
              },
            )
        },
        onReset() {
          useViewStore
            .getState()
            .setSplitRatio(viewId, splitId, 0.5)
            .catch(err => console.error(`[useSplitDrag] set_split_ratio(${splitId}, 0.5) 실패:`, err))
        },
      }
    },
    [ranges, rootRef, send, viewId],
  )

  return { slotRects: display.slotRects, splitRects: display.splitRects, splitterProps }
}

// `WindowLayout.tsx` 의 탭 캐시 당김과 같은 모양 — 유계 재시도 뒤 캐시에 채택한다. 취소되면(언마운트·뷰 전환)
//   재시도를 멈추고, 이미 날아간 응답도 캐시에 넣지 않는다. 채택된 것은 캐시의 version 가드가 한 번 더 거른다.
function pullView(viewId: string, isCancelled: () => boolean): void {
  void retryAsync(() => invoke<ViewSnapshot>('get_view', { viewId }), {
    isCancelled,
    onRetry: (err, attempt) => console.warn(`[useSplitDrag] get_view(${viewId}) 재시도 ${attempt}:`, err),
  })
    .then(snap => {
      if (!isCancelled()) useViewStore.getState().applyLayoutUpdated(snap)
    })
    .catch(err => {
      if (err instanceof RetryCancelledError) return
      console.error(`[useSplitDrag] get_view(${viewId}) 최종 실패(재시도 소진):`, err)
    })
}
