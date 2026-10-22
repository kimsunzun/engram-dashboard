// TRD S21-slot-docking §4-2 · §4-6: 탭 머리 = OS(HTML5) 끌기의 출처. 머리 사건(누름 · dragstart · dragend)을 값으로 바꿔
//   `dragSource`(정하는 곳 — 풀기 · 되돌리기 · 보내기)에 넘기고, 이 탭바의 판정 기하를 `localDrag` 에 등록한다. 결과 알림
//   (`tab_drop:done`) · 그 창의 pointerdown · 출처 창 탭바 판정 바뀜을 `dragSource` 에 잇는 구독도 여기가 걸고 뗀다(탭바 수명).
// ★dragstart 안에서 끄는 머리를 누른 점 밑에서 빼지 않는다★ — Chromium 은 dragstart 처리 뒤 누른 점을 다시 짚어 그 머리가
//   거기 없으면 OS 끌기를 열지 않고 곧바로 dragend(none) 을 낸다(옛 HTML5 끌기 실측 15/15 · 그 밖의 DOM 변경은 끌기를 끝내지
//   않았다 25/25). 그래서 dragstart 가 바꾼 상태로 다시 그려도 탭바 DOM 은 그대로다(`DragSourceSnapshot.motion` 이 아직 없다).
// ★우리 끌기의 dragstart 만 끊는다★ — 에이전트 트리(react-arborist → react-dnd)의 HTML5 백엔드가 `window` 거품 리스너로, 자기
//   소스가 받지 않은 dragstart 를 취소한다(`react-dnd-html5-backend` `handleTopDragStart` — TRD §4-2). dragend 는 끊지 않는다.

import { getIdentifier } from '@tauri-apps/api/app'
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type DragEvent as ReactDragEvent,
  type MutableRefObject,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from 'react'

import type { EndEffect } from '../../../../src-tauri/bindings/EndEffect'
import type { Insets } from '../../../../src-tauri/bindings/Insets'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'
import { MAIN_WINDOW_LABEL, useViewStore } from '../../../store/viewStore'
import { stripZone } from '../dock/bands'
import { setTabDragImage } from '../dock/dragImage'
import { shownTab } from '../dock/dragOverride'
import { dragGrab, dragSizes } from '../dock/dragSize'
import { localDrag, type StripGeometry } from '../dock/localDrag'
import { tabDropClient } from '../dock/tabDrop'
import { buildTabDragType, buildTabDragValue, shapeOf } from '../dock/tabDragData'
import { windowFrame } from '../dock/windowFrame'
import { createAppIdentifier } from './appIdentifier'
import { createDragSource, type DragSource, type DragSourcePorts, type DragSourceSnapshot, type DragStart } from './dragSource'
import { measureStrip } from './measure'
import { dragAreaWidth, grabOffset, type StripBounds } from './reorderGeometry'

const ports: DragSourcePorts = {
  send: drop => tabDropClient.send(drop),
  switchTabIf: (window, to, expect) => useViewStore.getState().switchTabIf(window, to, expect),
  local: localDrag,
  nextTask: fn => void setTimeout(fn, 0),
}

let source: DragSource = createDragSource(ports)

/** 웹뷰마다 하나. */
export function dragSource(): DragSource {
  return source
}

/** 테스트 전용 — 모듈 상태 초기화(테스트 간 격리). 프로덕션 코드에서 호출 금지. */
export function __resetDragSourceForTest(): void {
  source = createDragSource(ports)
}

// 웹뷰마다 하나 — 두 함수의 계약은 `AppIdentifier` 의 같은 이름 메서드다.
const appIdentifier = createAppIdentifier(getIdentifier)

export function currentAppIdentifier(): string | null {
  return appIdentifier.current()
}

export function askAppIdentifier(): void {
  appIdentifier.ask()
}

const CAPTURE: AddEventListenerOptions = { capture: true, passive: true }
const NO_INSETS: Insets = { t: 0, r: 0, b: 0, l: 0 }

/** 탭바가 지금 그린 것 — 렌더마다 적는다. */
export interface DrawnStrip {
  /** 그린 순서(끄는 동안 = 확정한 순서 이동을 실은 순서). */
  order: readonly string[]
  bounds: StripBounds
  /** 잰 「머리들이 쓸 수 있는 폭」(`null` = 재지 못함). */
  available: number | null
}

export interface UseTabDragSourceOptions {
  label: string
  /** 모델 순서의 탭 id — 그 창 스토어 적용마다 새 배열 · 그 밖에는 같은 참조(바뀜 = 탭 알림 — `DragSource.model`). */
  tabs: readonly string[]
  /** 모델 활성 — 누를 때 값이 그 끌기의 `initial` 이다. */
  active: string | null
  /** 이름 편집 중인 탭 — 끌지 않는다. */
  editing: string | null
  /** 끌기 그림 라벨. */
  nameOf: (view: string) => string
  /** 탭바 줄(`measure.ts` `StripDom.row`). */
  row: RefObject<HTMLElement | null>
  add: RefObject<HTMLElement | null>
  /** 탭 캔버스 영역 — 크롬(뷰포트 − 캔버스) 재기. 없거나 크기 0 이면 크롬 0 으로 친다. */
  canvas?: () => Element | null
  /**
   * 창 사실(`window_frame` · `window:frame`)을 받는다 — 기본 `true`. 대기 창은 셸 창 모델 밖이라 `false` 다(셸이 그 창을 몰라
   * 당기기가 `WindowNotFound` 로 실패한다). `true` 로 바뀔 때마다 새로 당긴다.
   */
  pullFrame?: boolean
}

export interface TabDragSourceHandle {
  snapshot: DragSourceSnapshot
  /** 누름부터 그 누름이 끝날 때까지와, 이 창의 끌기(바꿔 그리기 포함)가 풀릴 때까지 `true` — 머리 DOM 순서 맞추기를 미룬다. */
  engaged: boolean
  draggable(view: string): boolean
  /** 탭바가 렌더마다 적는다 — dragstart 셈 · 판정 기하가 읽는다. */
  drawn: MutableRefObject<DrawnStrip | null>
  /**
   * 머리 pointerdown 의 캡처 단계 — 누름 순간 전환(`switch_tab`)보다 먼저(누르기 전 활성을 적는다) · 머리 안 닫기 버튼이 거품을
   * 끊어도 그 누름을 적는다(그 자리에서 시작한 끌기를 막는 재료).
   */
  onPointerDown(e: ReactPointerEvent<HTMLElement>, view: string): void
  onDragStart(e: ReactDragEvent<HTMLElement>, view: string): void
  onDragEnd(e: ReactDragEvent<HTMLElement>, view: string): void
  /** 이 창의 `DragSource.forceSwitch`. */
  forceSwitch(view: string): boolean
}

/** 누름 하나 — 머리 pointerdown 에서 적고 dragstart 에서 쓴다. 좌표 = 뷰포트 CSS px. */
interface Press {
  view: string
  initial: string | null
  target: EventTarget | null
  point: PointCss
  /** 누른 순간 머리의 그려진 가로 범위 — 잡은 폭 비율의 기준(Chrome 은 누를 때 잰다). */
  head: { x0: number; x1: number }
}

// ADR-0296
export function useTabDragSource(o: UseTabDragSourceOptions): TabDragSourceHandle {
  const src = source
  const snapshot = useSyncExternalStore(src.subscribe, src.getSnapshot)
  const live = useRef(o)
  useLayoutEffect(() => {
    live.current = o
  })
  const drawn = useRef<DrawnStrip | null>(null)
  const press = useRef<Press | null>(null)

  // 그 창 스토어가 적용한 탭 상태 — 렌더된 그 값으로 먹인다(창 레이아웃도 같은 값을 그린다). 스토어는 적용마다 탭 목록을 새
  //   배열로 두므로(`applyWindowTabsUpdated`) 그 참조가 바뀐 것이 곧 「알림이 왔다」다.
  useLayoutEffect(() => {
    src.model(o.label, { tabs: o.tabs, active: o.active })
  }, [src, o.label, o.tabs, o.active])

  const pullFrame = o.pullFrame ?? true
  useEffect(() => (pullFrame ? windowFrame.install() : undefined), [pullFrame])

  useEffect(() => {
    askAppIdentifier()
    const offDone = tabDropClient.onDone(d => src.done(d))
    const offStrip = localDrag.subscribe(() => src.stripChanged())
    const offGeometry = localDrag.registerStripGeometry(() => geometryOf(live.current, drawn.current))
    const onDown = (): void => src.pointerDown()
    window.addEventListener('pointerdown', onDown, CAPTURE)
    return () => {
      offDone()
      offStrip()
      offGeometry()
      window.removeEventListener('pointerdown', onDown, CAPTURE)
    }
  }, [src])

  const [pressing, setPressing] = usePressing()

  const onPointerDown = useCallback(
    (e: ReactPointerEvent<HTMLElement>, view: string) => {
      if (e.button !== 0) return
      const r = e.currentTarget.getBoundingClientRect()
      press.current = {
        view,
        initial: live.current.active,
        target: e.target,
        point: { x: e.clientX, y: e.clientY },
        head: { x0: r.left, x1: r.right },
      }
      setPressing(true)
    },
    [setPressing],
  )

  const onDragStart = useCallback(
    (e: ReactDragEvent<HTMLElement>, view: string) => {
      const p = press.current?.view === view ? press.current : null
      press.current = null
      // 끌기가 누름을 이어받는다 — 엔진이 OS 끌기로 가면 그 누름의 pointerup 이 오지 않을 수 있다.
      setPressing(false)
      const opts = live.current
      const head = e.currentTarget
      const dt = e.dataTransfer
      // 이름 입력 · 닫기 버튼에서 시작한 끌기는 탭 끌기가 아니다(글자 끌기 · 버튼 누름) — 엔진은 그 끌기를 draggable 조상(머리)에
      //   내므로 누른 자리를 따로 본다. 우리 끌기가 아니라 끊지 않는다.
      if (opts.editing === view || onControl(head, e.target) || (p !== null && onControl(head, p.target)) || !dt) {
        e.preventDefault()
        return
      }
      const at = Number.isFinite(e.clientX) && Number.isFinite(e.clientY) ? { x: e.clientX, y: e.clientY } : p?.point
      const begin = at === undefined ? null : plan(opts, drawn.current, view, p, at)
      if (begin === null) {
        e.preventDefault()
        return
      }
      e.stopPropagation()
      // ★이 형식 하나만 싣는다★(`tabDragData.ts` 머리) — 엔진 기본 자료를 먼저 비운다.
      dt.clearData()
      dt.setData(begin.type, begin.value)
      dt.effectAllowed = 'move'
      setTabDragImage(dt, opts.nameOf(view))
      src.start(begin.start)
    },
    [src, setPressing],
  )

  const onDragEnd = useCallback(
    (e: ReactDragEvent<HTMLElement>, view: string) => {
      const effect: EndEffect = e.dataTransfer?.dropEffect === 'move' ? 'move' : 'none'
      src.end(view, effect)
    },
    [src],
  )

  const mine = snapshot.window === o.label && snapshot.override.phase !== 'idle'
  const single = o.label === MAIN_WINDOW_LABEL && o.tabs.length === 1
  return {
    snapshot,
    engaged: pressing || mine,
    // Q35 — main 의 마지막 탭은 끌기를 시작하지 않는다(탭 하나뿐인 팝아웃은 끈다 — Q34).
    draggable: view => view !== o.editing && !single,
    drawn,
    onPointerDown,
    onDragStart,
    onDragEnd,
    forceSwitch: view => src.forceSwitch(o.label, view),
  }
}

/**
 * 창 레이아웃 · 탭바가 그 창에 보일 탭 — 출처 창이 바꿔 그리는 동안은 남은 탭(R15) · 되돌리기를 기다리는 동안은 되돌릴 탭,
 * 그 밖은 모델 활성. `tabs` = `null` 이면(그 창 탭을 아직 모름) 모델 활성.
 */
export function useShownTab(label: string, tabs: readonly string[] | null, active: string | null): string | null {
  const src = source
  return useSyncExternalStore(src.subscribe, () => {
    const s = src.getSnapshot()
    return tabs !== null && s.window === label ? shownTab(s.override, { tabs, active }) : active
  })
}

// 누름은 그 뗌 · 취소에서 끝난다 — 듣기는 누를 때 곧바로 단다(effect 로 달면 그 전에 온 뗌을 놓친다).
function usePressing(): [boolean, (on: boolean) => void] {
  const [pressing, setState] = useState(false)
  const off = useRef<(() => void) | null>(null)
  const set = useCallback((on: boolean) => {
    off.current?.()
    off.current = null
    setState(on)
    if (!on) return
    const up = (): void => set(false)
    window.addEventListener('pointerup', up, CAPTURE)
    window.addEventListener('pointercancel', up, CAPTURE)
    off.current = () => {
      window.removeEventListener('pointerup', up, CAPTURE)
      window.removeEventListener('pointercancel', up, CAPTURE)
    }
  }, [])
  useEffect(() => () => off.current?.(), [])
  return [pressing, set]
}

/** dragstart 의 실음 · 크기 · 출발값. 실을 수 없으면(탭바를 모름 · identifier 없음) `null` — 끌기를 열지 않는다. */
function plan(
  o: UseTabDragSourceOptions,
  d: DrawnStrip | null,
  view: string,
  p: Press | null,
  at: PointCss,
): { type: string; value: string; start: DragStart } | null {
  const app = appIdentifier.current()
  const row = o.row.current
  const i = d === null ? -1 : d.order.indexOf(view)
  if (app === null || row === null || d === null || i < 0) {
    console.warn('[useTabDragSource] 끌기 자료를 지을 수 없다 — 끌기를 열지 않는다:', { app, row: row !== null, view })
    return null
  }
  const b = d.bounds.tabs[i]
  const stripLeft = row.getBoundingClientRect().left
  const pressed = p?.point ?? at
  const head = p?.head ?? { x0: stripLeft + b.x0, x1: stripLeft + b.x1 }
  const initial = p?.initial ?? o.active
  const w = head.x1 - head.x0
  const ratio = w > 0 ? Math.min(1, Math.max(0, (pressed.x - head.x0) / w)) : 0

  const frame = windowFrame.current()
  const restored = frame?.restored ?? null
  const maximized = frame?.maximized ?? false
  const single = o.label !== MAIN_WINDOW_LABEL && o.tabs.length === 1
  const viewport: SizeCss = { w: window.innerWidth, h: window.innerHeight }
  const sizes = dragSizes({
    restored,
    maximized,
    insets: frame?.insets ?? NO_INSETS,
    viewport,
    work: { w: window.screen.availWidth, h: window.screen.availHeight },
    chrome: chromeOf(o.canvas?.() ?? null, viewport),
    single,
  })
  // `dragGrab` 의 W1 = 새 창 탭바의 탭 하나 머리 폭 — 새 창(팝아웃)의 탭바 줄이 그 창 안쪽 폭 전체이고 `+` 는 이 창의 것과 같은
  //   폭이라고 친다(팝아웃은 탭바 좌우에 크롬이 없다 — `PopoutPage` · [미검] TRD §9-2 · M2 가 잰다). 최대화된 탭 하나뿐인 팝아웃은
  //   복원한 창 폭으로 센다(TRD §3-5).
  const newWidth = sizes.size !== null ? sizes.size.w : maximized && restored !== null ? restored.w : null
  const addW = o.add.current?.getBoundingClientRect().width ?? 0
  const headRoom = newWidth === null ? null : Math.max(0, newWidth - addW)
  const grab = dragGrab({ pressed, head, stripLeft, headRoom, single, maximized })

  const drag = crypto.randomUUID()
  const layout = useViewStore.getState().layouts[view]?.layout
  return {
    type: buildTabDragType({ app, drag, original: sizes.original, shape: layout === undefined ? null : shapeOf(layout) }),
    value: buildTabDragValue({ drag, view, from: o.label, initial, size: sizes.size, grab }),
    start: {
      window: o.label,
      drag,
      view,
      initial,
      size: sizes.size,
      grab,
      grabOffset: grabOffset(ratio, b.x1 - b.x0),
      left: b.x0,
      x: at.x - stripLeft,
    },
  }
}

/** 크롬 = 뷰포트 − 탭 캔버스(음수는 0). */
function chromeOf(canvas: Element | null, viewport: SizeCss): SizeCss {
  const c = canvas?.getBoundingClientRect()
  if (c === undefined || !(c.width > 0 && c.height > 0)) return { w: 0, h: 0 }
  return { w: Math.max(0, viewport.w - c.width), h: Math.max(0, viewport.h - c.height) }
}

/** 이 탭바의 판정 재료 — 줄을 그때 잰다(창 크기가 바뀌어도 맞게). */
function geometryOf(o: UseTabDragSourceOptions, d: DrawnStrip | null): StripGeometry | null {
  const row = o.row.current
  if (row === null || d === null) return null
  const m = measureStrip({ row, add: o.add.current })
  const area = { x0: m.left, y0: m.row.y0, x1: m.left + dragAreaWidth(d.available, d.bounds), y1: m.row.y1 }
  return { left: m.left, bounds: d.bounds, available: d.available, zoneViewport: stripZone(area), tabs: d.order }
}

function onControl(head: Element, target: EventTarget | null): boolean {
  const el = target instanceof Element ? target : target instanceof Node ? target.parentElement : null
  const control = el?.closest('input, button')
  return control != null && head.contains(control)
}
