// TRD S21-slot-docking §4-3 · §2-4: 한 창(웹뷰)의 끌기 받는 쪽 — 뿌리에 온 우리 탭 끌기를 판정(`judge`)해 안내(미리보기 · 구역
//   선 · 레이아웃 틀) · 다른 창 탭바 세로선 · 출처 창 자기 탭바 판정을 적고, drop 에서 놓기 명령 하나를 낸다. 정하는 것은 여기뿐이고
//   DOM 은 모른다 — 문서 사건을 이 값으로 잇는 것은 `useWindowDrop` 이다.
// ★끌기 명령은 drop 에서만 보낸다★(ADR-0296 · ADR-0297) — 이 모듈의 셸 호출은 drop 의 `tab_drop` 하나뿐이고, 같은 창 같은 자리면
//   그것도 없다. 끄는 동안 바깥(창 크기 · 탭 · 레이아웃)은 막지 않고 흐르므로 판정은 늘 지금 값을 쓴다 — 셸이 놓을 때 대상을 다시
//   찾는다.
// ★안내는 끌기 회차 하나에만 선다★ — 회차는 dragenter 가 열고 drop · 창 떠남(dragleave · 관련 대상 없음) · dragover 없는
//   `GUIDE_IDLE_MS` · mousemove 뒤 `GUIDE_IDLE_MS` 가 닫는다. dragover 는 열린 회차의 것만 그린다 — 닫힌 뒤 늦게 온 dragover 가 새
//   dragenter 없이 안내 · 세로선을 되살리지 않는다(TRD §8-3). 닫는 시한은 건 회차를 쥐고 울려 「지금 회차가 그 회차인가」 하나로
//   가른다 — 사건 · 시한이 닿는 차례에 기대지 않는다. 받기(`preventDefault`)는 회차와 무관하게 늘 한다 — 그래야 drop 이 온다.
// 값싸게(S1 처리기 ≤ 0.5 ms) — 띠 · 유효성은 보이는 탭 · 그 캐시 · 캔버스 크기 · 탭바 구역이 바뀔 때만 다시 세고, dragover 마다는
//   탭바 기하 읽기와 점 고르기만 한다. 캔버스는 dragenter 와 회차 동안의 크기 바뀜(`watchCanvas`)에만 다시 잰다 — 바뀌면 마지막
//   점으로 다시 판정해 dragover 를 기다리지 않고 미리보기가 새 크기를 따른다.
// ★끌기는 LLM 명령이 아니다★(TRD §3-6) — 사람 손의 입력과 그 측정이라 전역 핸들 · 버스에 올리지 않는다.

import type { LayoutNode } from '../../../api/layoutTypes'
import type { DropTo } from '../../../../src-tauri/bindings/DropTo'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { TabDrop } from '../../../../src-tauri/bindings/TabDrop'
import type { TabDropReply } from '../../../../src-tauri/bindings/TabDropReply'
import { windowBands, zoneInCanvas, type WindowBands } from './bands'
import type { CanvasPx } from './dockedTree'
import { judge, type JudgeStrip, type Judgement } from './judge'
import type { LocalDrag, OutgoingDrag } from './localDrag'
import type { OverlayView, RectCss } from './overlayTypes'
import { readTabDragType, readTabDragValue, type TabDragHead } from './tabDragData'

/** 안내 · 세로선을 지우는 시한 — dragover 없이 이만큼 · mousemove 뒤 이만큼(VS Code 꼴 · 웹에만 있는 값 — TRD §4-5). */
export const GUIDE_IDLE_MS = 300

/** 보이는 탭의 캐시 — 띠 셈의 재료(`viewStore.CachedView` 의 칸). 같은 캐시면 같은 객체여야 띠를 다시 세지 않는다. */
export interface DropView {
  layout: LayoutNode
  ratioMin: number
  ratioMax: number
}

/** 이 창의 모델 탭 상태(스토어) — `tabs` = 모델 순서 탭 id. */
export interface DropModel {
  tabs: readonly string[]
  active: string | null
}

/** 바깥 — 운영은 `useWindowDrop` 이 스토어 · 탭바 기하 · Tauri 로 채우고 시험은 가짜를 꽂는다(ADR-0012). */
export interface WindowDropPorts {
  /** Tauri identifier(소문자) — `null` = 아직 모름(어떤 끌기도 우리 것으로 읽지 않는다). */
  app(): string | null
  local: LocalDrag
  /** 탭 캔버스 영역(뷰포트 CSS px) — `null` = 없음. */
  canvasRect(): { left: number; top: number; width: number; height: number } | null
  /** 이 창 캔버스가 지금 보이는 탭(출처 창의 R15 남은 탭 포함 — `WindowLayout` 이 그리는 그 값). */
  shownTab(): string | null
  view(id: string): DropView | null
  model(): DropModel
  send(drop: TabDrop): Promise<TabDropReply>
  setTimer(fn: () => void, ms: number): unknown
  clearTimer(handle: unknown): void
  /**
   * 캔버스 크기 바뀜을 지켜본다 — `onResize` 는 크기가 바뀔 때 부른다(첫 관측 등 같은 크기로도 부를 수 있다). 돌려준 함수가
   * 멈춘다. 캔버스가 없으면 아무것도 지켜보지 않는다.
   */
  watchCanvas(onResize: () => void): () => void
}

/** 뿌리 사건 — DOM `DragEvent` 가 그대로 맞는다. 좌표 = 뷰포트 CSS px. */
export interface RootDragEvent {
  readonly clientX: number
  readonly clientY: number
  readonly relatedTarget: EventTarget | null
  readonly dataTransfer: Pick<DataTransfer, 'types' | 'getData' | 'dropEffect'> | null
  preventDefault(): void
  stopPropagation(): void
}

export interface WindowDrop {
  dragEnter(e: RootDragEvent): void
  dragOver(e: RootDragEvent): void
  dragLeave(e: RootDragEvent): void
  drop(e: RootDragEvent): void
  /** 이 문서의 mousemove — `buttons` 가 0 이 아니면 보지 않는다. */
  mouseMove(buttons: number): void
  /** 지금 그릴 안내 — 바뀔 때만 새 객체다. */
  getGuide(): OverlayView | null
  subscribe(cb: () => void): () => void
  /** 열린 회차를 닫는다(떼기) — 그 뒤에도 쓸 수 있다. */
  reset(): void
}

/** 캔버스 — `size` = 반올림한 크기(`null` = 캔버스가 없거나 크기 0). 다시 잰 값이 같으면 객체를 바꾸지 않는다(띠 캐시 열쇠). */
interface Canvas {
  origin: PointCss
  size: CanvasPx | null
}

/** 보이는 탭의 띠 셈 — 열쇠(보이는 탭 · 그 캐시 · 끄는 탭 자신인가 · 탭바 구역 · 캔버스)가 같으면 다시 쓴다. */
interface Bands {
  shown: string | null
  view: DropView | null
  self: boolean
  zone: RectCss | null
  canvas: Canvas
  /** `null` = 띠 없음(끄는 탭 자신이 보이는 캔버스 — `SelfTarget` · 보이는 탭의 캐시 · 캔버스가 없음). */
  bands: WindowBands | null
}

/** 판정 하나와 그때 쓴 상태 — drop 이 이것으로 명령을 짓는다(미리보기 = 결과 — R4 · TRD §4-3). */
interface Decision {
  j: Judgement
  /** 이 웹뷰에서 나간 그 끌기(이 창 = 출처) · 아니면 `null`. */
  source: OutgoingDrag | null
  /** 판정한 탭바가 끄는 탭을 담은 출처 자기 탭바였다. */
  ownStrip: boolean
  /** 탭바 좌표 0 의 뷰포트 x. */
  stripLeft: number
  bands: Bands
}

interface Session {
  head: TabDragHead
  canvas: Canvas
  bands: Bands | null
  /** 마지막 판정 — 회차는 dragenter 의 판정으로 열리므로 열린 회차에서는 늘 있다. */
  last: Decision | null
  /** 마지막 판정의 점 — 캔버스 크기가 바뀌면 이 점으로 다시 판정한다. */
  point: PointCss | null
  idle: unknown | null
  mouse: unknown | null
  unwatch: (() => void) | null
}

const NO_CANVAS: Canvas = { origin: { x: 0, y: 0 }, size: null }

// ADR-0296
export function createWindowDrop(ports: WindowDropPorts): WindowDrop {
  const { local } = ports
  let session: Session | null = null
  let guide: OverlayView | null = null
  let guideKey: { bands: WindowBands; type: Judgement['type']; band: number | null; zones: number | null } | null = null
  const listeners = new Set<() => void>()

  function publish(next: OverlayView | null): void {
    if (next === guide) return
    guide = next
    for (const cb of [...listeners]) cb()
  }

  // 우리 끌기인가 — 열린 회차와 같은 타입 문자열이면 다시 읽지 않는다(앱 열쇠는 그 회차를 열 때 맞췄다).
  function ours(e: RootDragEvent): TabDragHead | null {
    const dt = e.dataTransfer
    const app = ports.app()
    if (dt === null || app === null) return null
    if (session !== null && dt.types.includes(session.head.mime)) return session.head
    return readTabDragType(dt.types, app)
  }

  function accept(e: RootDragEvent): void {
    e.preventDefault()
    if (e.dataTransfer !== null) e.dataTransfer.dropEffect = 'move'
    e.stopPropagation()
  }

  function measure(): Canvas {
    const r = ports.canvasRect()
    if (r === null) return NO_CANVAS
    const size = { w: Math.round(r.width), h: Math.round(r.height) }
    return { origin: { x: r.left, y: r.top }, size: size.w > 0 && size.h > 0 ? size : null }
  }

  function open(head: TabDragHead): Session {
    return { head, canvas: measure(), bands: null, last: null, point: null, idle: null, mouse: null, unwatch: null }
  }

  // 다시 잰 캔버스가 다르면 바꿔 두고 `true`.
  function remeasure(s: Session): boolean {
    const c = measure()
    const a = s.canvas
    const same =
      c.origin.x === a.origin.x &&
      c.origin.y === a.origin.y &&
      (c.size === null || a.size === null ? c.size === a.size : c.size.w === a.size.w && c.size.h === a.size.h)
    if (!same) s.canvas = c
    return !same
  }

  function resized(s: Session): void {
    if (session !== s || !remeasure(s) || s.point === null) return
    step(s, s.point)
  }

  function bandsOf(s: Session, source: OutgoingDrag | null, zone: RectCss | null): Bands {
    const shown = ports.shownTab()
    const view = shown === null ? null : ports.view(shown)
    const self = source !== null && shown === source.view
    const c = s.bands
    if (c !== null && c.shown === shown && c.view === view && c.self === self && c.canvas === s.canvas && sameRect(c.zone, zone)) {
      return c
    }
    const size = s.canvas.size
    const bands =
      view === null || self || size === null
        ? null
        : windowBands({
            canvas: size,
            layout: view.layout,
            ratio: { min: view.ratioMin, max: view.ratioMax },
            incoming: s.head.shape,
            original: s.head.original,
            zone,
          })
    const next = { shown, view, self, zone, canvas: s.canvas, bands }
    s.bands = next
    return next
  }

  function decide(s: Session, point: PointCss): Decision {
    // ★끌기 id 를 견준 뒤에만 출처로 본다★ — 나간 끌기 표시는 dragend 뒤 바꿔 그리기가 풀릴 때까지 남는다.
    const out = local.getOutgoing()
    const source = out !== null && s.head.drag !== null && out.drag === s.head.drag ? out : null
    const g = local.readStripGeometry()
    const zone = g === null ? null : zoneInCanvas(g.zoneViewport, s.canvas.origin)
    const b = bandsOf(s, source, zone)
    let strip: JudgeStrip | null = null
    let dragged = -1
    if (g !== null) {
      if (source !== null) {
        dragged = g.tabs.indexOf(source.view)
        strip = {
          role: 'source',
          left: g.left,
          bounds: g.bounds,
          dragged,
          grabOffset: source.grabOffset,
          available: g.available,
        }
      } else {
        strip = { role: 'other', left: g.left, bounds: g.bounds }
      }
    }
    const j = judge(
      { canvasOrigin: s.canvas.origin, tabBar: strip === null || zone === null ? null : { zone, strip }, bands: b.bands },
      point,
    )
    return { j, source, ownStrip: dragged >= 0, stripLeft: g?.left ?? 0, bands: b }
  }

  function step(s: Session, point: PointCss): void {
    let d = decide(s, point)
    const { j } = d
    if (d.source !== null) {
      // 출처 창 뿌리 판정 — 늘 새 객체로 적는다(같은 객체는 바뀐 것으로 치지 않는다) · `null` = 자기 탭바 구역을 떠남.
      local.setSourceStrip(
        j.type === 'strip' && j.left !== null ? { index: j.index, left: j.left, x: point.x - d.stripLeft } : null,
      )
    }
    local.setTargetMarker(j.type === 'strip' ? j.marker : null)
    // 위 적기가 출처 창의 보이는 탭을 바꿨으면(R15 — 구역을 떠나 남은 탭 · 돌아와 끄는 탭) 그 탭의 띠로 다시 판정한다 — 한
    //   dragover 늦게 그리지 않게. 탭바 구역 판정은 보이는 탭과 무관해 다시 볼 것이 없다.
    if (j.type !== 'strip' && ports.shownTab() !== d.bands.shown) d = decide(s, point)
    s.last = d
    s.point = point
    publish(guideOf(d))
  }

  function guideOf(d: Decision): OverlayView | null {
    const wb = d.bands.bands
    const { j } = d
    if (wb === null || j.type === 'strip') {
      guideKey = null
      return null
    }
    const band = j.type === 'band' ? j.band : null
    const k = guideKey
    if (k !== null && k.bands === wb && k.type === j.type && k.band === band && k.zones === j.zones) return guide
    guideKey = { bands: wb, type: j.type, band, zones: j.zones }
    const slot = j.zones === null ? null : wb.zones[j.zones]
    if (band === null) return slot === null ? null : { bands: [], outer: [], preview: null, slot }
    const b = wb.bands[band]
    // 레이아웃 틀은 바깥 띠를 고른 때만 — 틀이 서는 것은 이 목록이 비지 않음이다(사용자 결정 2026-10-19).
    const outer = b.anchor.type === 'outer' ? wb.bands.filter(x => x.anchor.type === 'outer').flatMap(x => x.areas) : []
    return { bands: [], outer, preview: b.landing, slot }
  }

  function arm(s: Session, slot: 'idle' | 'mouse'): void {
    const prev = s[slot]
    if (prev !== null) ports.clearTimer(prev)
    s[slot] = ports.setTimer(() => {
      if (session === s) end(s)
    }, GUIDE_IDLE_MS)
  }

  function end(s: Session): void {
    if (session !== s) return
    session = null
    if (s.idle !== null) ports.clearTimer(s.idle)
    if (s.mouse !== null) ports.clearTimer(s.mouse)
    s.unwatch?.()
    s.idle = null
    s.mouse = null
    s.unwatch = null
    local.setTargetMarker(null)
    guideKey = null
    publish(null)
  }

  function deliver(head: TabDragHead, d: Decision, dt: Pick<DataTransfer, 'getData'>): void {
    const { j } = d
    let to: DropTo | null = null
    if (j.type === 'strip') {
      let index = j.index
      if (d.ownStrip && d.source !== null) {
        // 출처 자기 탭바 — 놓을 자리 = 지금 그린 순서의 끄는 탭 자리(순서 문턱을 넘어 확정한 자리 · 판정의 날 자리가 아니다).
        //   drop 때 다시 읽는다 — 마지막 dragover 가 확정한 자리는 그 뒤 그림에서야 기하에 실린다.
        const view = d.source.view
        const drawn = local.readStripGeometry()?.tabs.indexOf(view) ?? -1
        if (drawn >= 0) index = drawn
        // ★제자리인가는 지금 스토어로 가른다★(ADR-0297) — 마지막 판정 뒤 끄는 탭이 다른 창으로 갔으면(LLM) 그때 본 모델은 여전히
        //   「제자리」라, 보내지 않으면 탭이 그 창에 남는다. 서지 않으면 보내고 셸이 id 로 다시 찾는다.
        if (index === ports.model().tabs.indexOf(view)) {
          // 같은 창 같은 자리 — 보내지 않고 출처 dragend 가 곧바로 풀게 적어 둔다(TRD §2-4 ④).
          local.markLocalDrop(d.source.drag)
          return
        }
      }
      to = { type: 'tab_bar', index }
    } else if (j.type === 'band') {
      const b = d.bands.bands?.bands[j.band]
      const target = d.bands.shown
      if (b === undefined || target === null) {
        console.warn('[windowDrop] 띠 놓기의 대상을 모른다 — 보내지 않는다:', { target })
        return
      }
      to = { type: 'seam', target, anchor: b.anchor, side: b.side, share: b.share }
    }
    const value = readTabDragValue(dt.getData(head.mime))
    if (value === null) {
      console.warn('[windowDrop] 끌기 자료의 값을 못 읽어 놓기를 보내지 않는다:', head.mime)
      return
    }
    to ??= { type: 'new_window', by: { type: 'drop' }, size: value.size, grab: value.grab }
    ports.send({ drag: value.drag, view: value.view, from: value.from, initial: value.initial, to }).then(
      // 결과는 출처가 `tab_drop:done` 으로 받는다(TRD §4-6) — 여기는 남기기만 한다.
      reply => console.debug('[windowDrop] tab_drop(drop) 답:', reply),
      err => console.warn('[windowDrop] tab_drop(drop) 실패:', err),
    )
  }

  return {
    dragEnter(e) {
      const head = ours(e)
      if (head === null) return
      accept(e)
      let s = session
      if (s !== null && s.head.mime !== head.mime) {
        end(s)
        s = null
      }
      if (s === null) {
        const opened = open(head)
        s = opened
        session = s
        opened.unwatch = ports.watchCanvas(() => resized(opened))
      } else {
        remeasure(s)
      }
      step(s, { x: e.clientX, y: e.clientY })
      arm(s, 'idle')
    },

    dragOver(e) {
      const head = ours(e)
      if (head === null) return
      accept(e)
      const s = session
      if (s === null || s.head.mime !== head.mime) return
      step(s, { x: e.clientX, y: e.clientY })
      arm(s, 'idle')
    },

    dragLeave(e) {
      const head = ours(e)
      if (head === null) return
      e.stopPropagation()
      // 관련 대상이 있으면 문서 안 요소 사이를 건넌 것이다 — Chromium 은 새 요소의 dragenter 를 먼저 낸다.
      if (e.relatedTarget !== null) return
      const out = local.getOutgoing()
      if (out !== null && head.drag !== null && out.drag === head.drag) local.setSourceStrip(null)
      if (session !== null) end(session)
    },

    drop(e) {
      const head = ours(e)
      if (head === null) return
      // ★우리 끌기면 첫 일이 `preventDefault` 다★ — 그래야 출처 dragend 가 `move` 다(TRD §4-3). 그 앞의 일은 타입 문자열 읽기뿐이다.
      e.preventDefault()
      e.stopPropagation()
      const s = session
      // 열린 회차가 없으면(시한 · mousemove 가 닫았다) 그 자리를 지금 판정한다 — 그 drop 을 버리지 않는다.
      const d = s !== null && s.head.mime === head.mime && s.last !== null ? s.last : decide(open(head), { x: e.clientX, y: e.clientY })
      if (s !== null) end(s)
      if (e.dataTransfer !== null) deliver(head, d, e.dataTransfer)
    },

    mouseMove(buttons) {
      const s = session
      // 주 버튼을 쥔 이동은 거른다 — 끄는 동안의 합성 · 밀린 이동이다(M11).
      if (s === null || buttons !== 0 || s.mouse !== null) return
      arm(s, 'mouse')
    },

    getGuide: () => guide,

    subscribe(cb) {
      const entry = (): void => cb()
      listeners.add(entry)
      return () => {
        listeners.delete(entry)
      }
    },

    reset() {
      if (session !== null) end(session)
    },
  }
}

function sameRect(a: RectCss | null, b: RectCss | null): boolean {
  if (a === null || b === null) return a === b
  return a.x0 === b.x0 && a.y0 === b.y0 && a.x1 === b.x1 && a.y1 === b.y1
}
