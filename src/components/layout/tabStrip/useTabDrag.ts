// TRD S21-slot-docking §2-2 · §4 「순서」 · §5-1 · §6-3: 한 탭바의 탭 끌기 — 누름 · 10 px 문턱 · 포인터 캡처 ·
//   `tab_drag_begin` · 탭바 안 순서 이동(끄는 탭 x · 삽입 자리 · 순서 문턱 · 이웃 비켜남) · 탭바 구역을 나가면
//   `tab_drag_leave` · 끝(`tab_drag_end`) · 끄는 중의 키. 창 사이 판정 · 떼기 · 붙기는 셸 세션 몫이고 여기는 답만 따른다.
// 입력 포트는 하나다 — 탭바 안을 모는 점이 `pointer`(이 창이 쥔 포인터 캡처) 또는 `shell`(이 창 탭바에 붙은 뒤 셸이 미는
//   `dock:feed{move}`)에서 오고, 둘 다 같은 몰기(`drive`)를 지난다. 끝 입력 — `pointer` 는 pointerup · 캡처 잃음, 둘 다 키와
//   셸의 놓기 확인 `dock:feed{release}`(붙은 창의 뗌 · 다른 창의 키 놓기).
// 키 — 탭을 쥔 동안(문턱을 넘은 뒤 · 붙은 뒤) 이 창의 keydown 을 `window` capture 에서 먼저 받아 `stopPropagation` 하고 지금
//   자리를 실은 끝을 보낸다. `dockSession` 의 앱 전체 키 감시는 `document` capture 라 그 키를 보지 않는다(그 파일 머리
//   주석). 창째 끌기 · 뗀 뒤에는 탭을 쥐지 않으므로 키를 감시에 넘긴다.
// 좌표 — 사건 · 셸 사각형은 뷰포트 CSS px, 순서 이동 셈은 끌기 영역 좌표(첫 탭 왼쪽 = 0 — `measure.ts`).

import { invoke } from '@tauri-apps/api/core'
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from 'react'

import type { BeginReply } from '../../../../src-tauri/bindings/BeginReply'
import type { FeedStep } from '../../../../src-tauri/bindings/FeedStep'
import type { Grab } from '../../../../src-tauri/bindings/Grab'
import type { LeaveReply } from '../../../../src-tauri/bindings/LeaveReply'
import type { RectCss } from '../../../../src-tauri/bindings/RectCss'
import type { TabDragBegin } from '../../../../src-tauri/bindings/TabDragBegin'
import type { TabDragEnd } from '../../../../src-tauri/bindings/TabDragEnd'
import type { TabDragEvent } from '../../../../src-tauri/bindings/TabDragEvent'
import type { TabDragFeed } from '../../../../src-tauri/bindings/TabDragFeed'
import type { TabDragLeave } from '../../../../src-tauri/bindings/TabDragLeave'
import type { TabDragReorder } from '../../../../src-tauri/bindings/TabDragReorder'
import { keyEnd, sendDragEnd, subscribeDrag, subscribeFeed } from '../dock/dockSession'
import { dragAreaRect } from '../dock/frameReport'
import { TAB_STANDARD_W } from './layout'
import { dragAreaOf, measureStrip, stripBounds, type StripMeasure } from './measure'
import { grabOffset, passedThreshold, reorderMove, slideMs, slideTargets, type Point } from './reorderGeometry'

export const CMD_TAB_DRAG_BEGIN = 'tab_drag_begin'
export const CMD_TAB_DRAG_REORDER = 'tab_drag_reorder'
export const CMD_TAB_DRAG_LEAVE = 'tab_drag_leave'

const REDUCED_MOTION = '(prefers-reduced-motion: reduce)'

/**
 * 탭바가 지금 그릴 끌기 모습 — 이 탭바 안에서 탭을 모는 동안만 있다(창째 끌기 · 뗀 뒤 · 끌기 밖 = `null`).
 * x 는 끌기 영역 좌표(첫 탭 왼쪽 = 0)다.
 */
export interface TabDragView {
  /** 끄는 탭. */
  view: string
  /**
   * 지금 그릴 탭 순서(끄는 탭 포함) — 모델 순서에 끄는 탭을 보낸 자리로 옮긴 것. 보낸 순서 이동의 모델 메아리가 오기
   * 전에도 이 순서다(TRD §5-1 「모델이 따라오는 동안」). 모델에 아직 없는 끄는 탭(막 붙은 창)도 들어 있다.
   */
  order: string[]
  /** `order` 자리마다 그 탭이 갈 왼쪽 x(`slideTargets`) — 끄는 탭 · 숨긴 탭 = `null`. */
  targets: (number | null)[]
  /** 끄는 탭을 그릴 왼쪽 x — 끄는 동안 커서를 따른다 · `settling` 이면 지금 자리의 이상적 x. */
  left: number
  /** 이웃이 새 자리로 갈 시간(ms · 곡선 `EASE_OUT_CSS`) — 0 = 곧바로 선다(막 붙은 첫 셈 · 줄인 움직임). */
  slideMs: number
  /** 끝을 보냈다 — 입력을 받지 않고 끄는 탭이 자기 자리로 가 있는 채 끌기가 끝나기(`dock:drag{ended}`)를 기다린다. */
  settling: boolean
}

/** 끝 입력 하나 — `tab_drag_end` 의 `src`(키는 그 키 이름까지). */
type EndInput = { src: 'pointer' } | { src: 'capture_lost' } | { src: 'release' } | { src: 'key'; key: string }

/** 이 탭바 안에서 탭을 모는 한 끌기. */
interface Drive {
  session: number
  view: string
  input: 'pointer' | 'shell'
  /** `pointer` 의 포인터(캡처를 쥔 것) · `shell` = −1. */
  pointerId: number
  /** 셸이 준 탭바 구역(뷰포트 · 반열림) — 점이 이 밖이면 떠나기를 청한다. */
  zone: RectCss
  /** `grabOffset` — 이 탭바에서 끌기가 시작될 때(문턱 · 붙음) 한 번 셈한다. */
  offset: number
  /** 마지막으로 자리가 바뀐 점(끌기 영역 x). */
  lastMove: number
  /** 끄는 탭의 지금 자리(결과 목록) — 보낸 자리다. */
  index: number
  /** 메아리를 아직 못 받은 보낸 자리 — 보낸 순서. */
  pending: number[]
  left: number
  slideMs: number
  /** 마지막으로 잰 쓸 수 있는 폭. */
  available: number
  /** 떠나기 답을 기다리는 중 — 그 사이의 처음 끝 입력 하나와 마지막 점(TRD §4 「답을 기다리는 동안」). */
  leaving: { held: EndInput | null; last: Point } | null
  done: boolean
}

type Phase =
  | { kind: 'idle' }
  | {
      kind: 'press'
      view: string
      pointerId: number
      down: Point
      initialActive: string | null
      /** `Grab.x_ratio` — `pressRatio`. */
      ratio: number
    }
  | {
      kind: 'begin'
      view: string
      pointerId: number
      /** 문턱을 넘은 점 — 셸 답이 `strip` 이면 순서 이동을 여기서 연다. */
      start: Point
      last: Point
      held: EndInput | null
      index: number
      offset: number
      /** 문턱 때 잰 끌기 영역 왼쪽. */
      left: number
    }
  | { kind: 'strip'; d: Drive }
  /** 창째 끌기 · 뗀 뒤 — 탭을 쥐지 않는다(포인터 사건 · 키 모두 버림). 그 세션의 `ended` 까지. */
  | { kind: 'away'; session: number }

const IDLE: Phase = { kind: 'idle' }

/** 컨트롤러가 기대는 바깥 — 훅이 DOM 으로 채우고 시험은 가짜로 채운다. */
export interface TabDragDeps {
  /** 포인터 캡처를 걸 탭바 줄 요소. */
  row(): HTMLElement | null
  /** 지금 탭바 재기 — 잴 요소가 없으면 `null`. */
  measure(): StripMeasure | null
  reducedMotion(): boolean
  /** 끌기 모습이 바뀔 때마다(같은 값을 거듭 줄 수 있다). */
  onView(view: TabDragView | null): void
}

/** 누름 하나 — 탭 머리의 `pointerdown`. */
export interface TabPress {
  view: string
  pointerId: number
  button: number
  x: number
  y: number
  /** 사건 대상 — 머리 안 닫기 단추 · 이름 입력에서 시작한 누름은 끌기가 아니다. */
  target: EventTarget | null
  /** 누른 탭 머리 — 잡은 폭 비율을 그 사각형에서 잰다. */
  head: Element
}

/** 한 탭바의 끌기 상태 줄 — 훅(`useTabDrag`)이 하나 쥔다. */
export class TabDragController {
  private phase: Phase = IDLE
  private tabs: readonly string[] = []
  private active: string | null = null
  private widthCap: number | null = null
  private seq = { session: -1, n: 0 }
  private pointerOn = false
  private keysOn = false

  constructor(private readonly deps: TabDragDeps) {}

  /**
   * 지금 모델(탭 id 의 모델 순서 · 활성 · 닫기 모드의 고정 폭) — 그릴 때마다 불러도 된다(목록이 같으면 아무것도 안 한다).
   * 끄는 중이면 메아리를 맞춘다: 보낸 자리의 메아리는 그 자리까지 지우고, 보낸 것이 없을 때 온 다른 자리는 따른다(다른
   * 경로가 바꾼 것 — TRD §5-1).
   */
  setModel(tabs: readonly string[], active: string | null, widthCap: number | null): void {
    const changed = !sameList(this.tabs, tabs) || this.widthCap !== widthCap
    this.tabs = tabs
    this.active = active
    this.widthCap = widthCap
    const ph = this.phase
    if (!changed || ph.kind !== 'strip') return
    const d = ph.d
    const m = tabs.indexOf(d.view)
    if (m >= 0) {
      const k = d.pending.indexOf(m)
      if (k >= 0) d.pending.splice(0, k + 1)
      else if (d.pending.length === 0) d.index = m
    }
    d.index = Math.min(d.index, this.others(d.view).length)
    this.publish()
  }

  // 누름 중의 새 누름은 앞 누름의 뗌을 놓친 것이다 — 새 점에서 다시 누른다.
  press(p: TabPress): void {
    if (p.button !== 0) return
    const ph = this.phase
    if (ph.kind === 'begin' || (ph.kind === 'strip' && !ph.d.done)) return
    if (onControl(p.head, p.target)) return
    this.reset()
    this.phase = {
      kind: 'press',
      view: p.view,
      pointerId: p.pointerId,
      down: { x: p.x, y: p.y },
      initialActive: this.active,
      ratio: pressRatio(p.head, p.x),
    }
    this.listenPointer(true)
  }

  /** `dock:feed` 의 몸 — 이 창이 끄는 탭을 쥔 창일 때만 온다. */
  feed(f: TabDragFeed): void {
    const step = f.step
    if (step.type === 'attach') {
      this.attach(f.session, f.view, step)
      return
    }
    const ph = this.phase
    if (ph.kind !== 'strip' || ph.d.session !== f.session || ph.d.view !== f.view) return
    if (step.type === 'move') {
      if (ph.d.input === 'shell') this.move(ph.d, { x: step.x, y: step.y })
    } else {
      this.endInput(ph.d, { src: 'release' })
    }
  }

  /** `dock:drag` 의 몸 — 이 끌기 세션의 `ended` 면 거둔다. */
  dragEvent(e: TabDragEvent): void {
    if (e.phase !== 'ended') return
    const ph = this.phase
    const session = ph.kind === 'strip' ? ph.d.session : ph.kind === 'away' ? ph.session : null
    if (session === e.session) this.abandon()
  }

  dispose(): void {
    this.reset()
  }

  // ── 포인터 · 키 ──

  private readonly onPointerMove = (e: PointerEvent): void => {
    const ph = this.phase
    const p = { x: e.clientX, y: e.clientY }
    if (ph.kind === 'press') {
      if (e.pointerId !== ph.pointerId) return
      // 주 단추가 놓여 있다 = 뗌을 놓친 누름 — 끌기를 열지 않고 버린다.
      if ((e.buttons & 1) === 0) this.reset()
      else if (passedThreshold(ph.down, p)) this.begin(ph, p)
    } else if (ph.kind === 'begin') {
      if (e.pointerId === ph.pointerId) ph.last = p
    } else if (ph.kind === 'strip' && ph.d.input === 'pointer' && e.pointerId === ph.d.pointerId) {
      this.move(ph.d, p)
    }
  }

  private readonly onPointerUp = (e: PointerEvent): void => {
    const ph = this.phase
    if (ph.kind === 'press') {
      if (e.pointerId === ph.pointerId) this.reset()
    } else if (ph.kind === 'begin') {
      if (e.pointerId === ph.pointerId) ph.held ??= { src: 'pointer' }
    } else if (ph.kind === 'strip' && ph.d.input === 'pointer' && e.pointerId === ph.d.pointerId) {
      this.endInput(ph.d, { src: 'pointer' })
    }
  }

  // 문턱을 넘은 뒤의 취소는 뒤따르는 `lostpointercapture` 로 받는다.
  private readonly onPointerCancel = (e: PointerEvent): void => {
    const ph = this.phase
    if (ph.kind === 'press' && e.pointerId === ph.pointerId) this.reset()
  }

  // 누름 중 어디서든 새 `pointerdown` · 창 포커스 잃음 = 앞 누름의 뗌이 이 창에 온다는 보장이 없다 — 누름을 버린다. 머리의
  //   새 누름은 이 뒤 React 의 `pointerdown` 이 `press` 로 다시 연다.
  private readonly onPressBroken = (): void => {
    if (this.phase.kind === 'press') this.reset()
  }

  private readonly onLostCapture = (e: PointerEvent): void => {
    const ph = this.phase
    if (ph.kind === 'begin') {
      if (e.pointerId === ph.pointerId) ph.held ??= { src: 'capture_lost' }
    } else if (ph.kind === 'strip' && ph.d.input === 'pointer' && e.pointerId === ph.d.pointerId) {
      this.endInput(ph.d, { src: 'capture_lost' })
    }
  }

  // 키 반복 · 수정키도 끝낸다(Chrome `EventTracker` — TRD §6-3) · 끝낸 키는 어느 슬롯에도 닿지 않는다.
  private readonly onKey = (e: KeyboardEvent): void => {
    const ph = this.phase
    if (ph.kind !== 'begin' && ph.kind !== 'strip') return
    e.preventDefault()
    e.stopPropagation()
    const input: EndInput = { src: 'key', key: e.key }
    if (ph.kind === 'begin') ph.held ??= input
    else this.endInput(ph.d, input)
  }

  private listenPointer(on: boolean): void {
    if (on === this.pointerOn) return
    this.pointerOn = on
    // `window` capture — 캡처 전에는 머리 밖으로 나간 움직임도, 캡처 뒤에는 탭바 줄로 가는 사건도 여기서 먼저 본다.
    //   `blur` 만 capture 가 아니다 — capture 면 요소의 blur(문턱 때 포커스 놓기 포함)까지 창 포커스 잃음으로 받는다.
    if (on) {
      window.addEventListener('pointermove', this.onPointerMove, true)
      window.addEventListener('pointerup', this.onPointerUp, true)
      window.addEventListener('pointercancel', this.onPointerCancel, true)
      window.addEventListener('lostpointercapture', this.onLostCapture, true)
      window.addEventListener('pointerdown', this.onPressBroken, true)
      window.addEventListener('blur', this.onPressBroken)
    } else {
      window.removeEventListener('pointermove', this.onPointerMove, true)
      window.removeEventListener('pointerup', this.onPointerUp, true)
      window.removeEventListener('pointercancel', this.onPointerCancel, true)
      window.removeEventListener('lostpointercapture', this.onLostCapture, true)
      window.removeEventListener('pointerdown', this.onPressBroken, true)
      window.removeEventListener('blur', this.onPressBroken)
    }
  }

  private listenKeys(on: boolean): void {
    if (on === this.keysOn) return
    this.keysOn = on
    if (on) window.addEventListener('keydown', this.onKey, true)
    else window.removeEventListener('keydown', this.onKey, true)
  }

  // ── 단계 ──

  // 문턱을 넘었다 — 캡처는 탭바 줄에 건다(머리가 아니다: 순서가 바뀌면 React 가 머리 DOM 을 옮기고, 옮겨지며 잠깐
  //   빠진 요소는 캡처를 잃을 수 있다 · 문턱 전에 잡으면 click · dblclick 대상이 바뀌어 이름 바꾸기가 깨진다 — TRD §2-2).
  private begin(ph: Extract<Phase, { kind: 'press' }>, p: Point): void {
    const m = this.deps.measure()
    const row = this.deps.row()
    const index = this.tabs.indexOf(ph.view)
    if (m === null || row === null || index < 0) {
      this.reset()
      return
    }
    const B = this.bounds(this.tabs.length, index, m.available)
    const w0 = B.tabs[index].x1 - B.tabs[index].x0
    const grab: Grab = { x_ratio: ph.ratio, y: ph.down.y - m.row.y0, single_x: grabOffset(ph.ratio, TAB_STANDARD_W) }
    row.setPointerCapture(ph.pointerId)
    // Chrome `SaveFocus`(TRD §6-3) — 끄는 중의 키가 포커스 받은 슬롯에 들어가지 않게.
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
    const next: Phase = {
      kind: 'begin',
      view: ph.view,
      pointerId: ph.pointerId,
      start: p,
      last: p,
      held: null,
      index,
      offset: grabOffset(ph.ratio, w0),
      left: m.left,
    }
    this.phase = next
    this.listenKeys(true)
    const args: TabDragBegin = {
      view: ph.view,
      down: ph.down,
      area: dragAreaRect(dragAreaOf(m, B)),
      grab,
      initial_active: ph.initialActive,
    }
    invoke<BeginReply>(CMD_TAB_DRAG_BEGIN, { args }).then(
      reply => {
        if (this.phase === next) this.answeredBegin(next, reply)
      },
      err => {
        if (this.phase !== next) return
        console.warn(`[useTabDrag] ${CMD_TAB_DRAG_BEGIN} 실패 — 끌기를 버린다:`, err)
        this.abandon()
      },
    )
  }

  private answeredBegin(b: Extract<Phase, { kind: 'begin' }>, reply: BeginReply): void {
    switch (reply.type) {
      case 'strip': {
        const d: Drive = {
          session: reply.session,
          view: b.view,
          input: 'pointer',
          pointerId: b.pointerId,
          zone: reply.zone,
          offset: b.offset,
          lastMove: b.start.x - b.left,
          index: b.index,
          pending: [],
          left: 0,
          slideMs: 0,
          available: 0,
          leaving: null,
          done: false,
        }
        this.phase = { kind: 'strip', d }
        // Chrome 은 문턱을 넘은 점에서 세션을 열며 막 붙은 셈을 한 번 한다(dragging_tabs_session.cc#56 — TRD §5-1).
        this.drive(d, b.start, true)
        if (b.held === null) {
          this.move(d, b.last)
          return
        }
        if (inZone(d.zone, b.last)) this.drive(d, b.last, false)
        this.sendEnd(d, b.held)
        return
      }
      case 'whole_window':
        this.goAway(reply.session, b.held)
        return
      case 'rejected':
        console.warn(`[useTabDrag] ${CMD_TAB_DRAG_BEGIN} 거절 — ${reply.reason}`)
        this.abandon()
        return
      case 'busy':
        this.abandon()
    }
  }

  // 막 붙은 첫 셈 — 셸은 `TabBar{W, 0}` 으로 넣었고(Chrome 의 첫 non-pinned 자리) 여기서 문턱 없이 자리를 고친다. 이웃은
  //   미끄러지지 않고 곧바로 선다(TRD §5-1 · dragging_tabs_session.cc#157-162).
  private attach(session: number, view: string, step: Extract<FeedStep, { type: 'attach' }>): void {
    this.reset()
    const m = this.deps.measure()
    if (m === null) return
    const B = this.bounds(this.others(view).length + 1, 0, m.available)
    const d: Drive = {
      session,
      view,
      input: 'shell',
      pointerId: -1,
      zone: step.zone,
      offset: grabOffset(step.grab.x_ratio, B.tabs[0].x1 - B.tabs[0].x0),
      lastMove: step.x - m.left,
      index: 0,
      pending: [],
      left: 0,
      slideMs: 0,
      available: m.available,
      leaving: null,
      done: false,
    }
    this.phase = { kind: 'strip', d }
    this.listenKeys(true)
    this.drive(d, { x: step.x, y: step.y }, true, true)
  }

  private move(d: Drive, p: Point): void {
    if (d.done) return
    if (d.leaving !== null) {
      d.leaving.last = p
      return
    }
    if (inZone(d.zone, p)) this.drive(d, p, false)
    else this.leave(d, p)
  }

  private drive(d: Drive, p: Point, justAttached: boolean, snap = false): void {
    const m = this.deps.measure()
    if (m === null) return
    const B = this.bounds(this.others(d.view).length + 1, d.index, m.available)
    const r = reorderMove({
      bounds: B,
      dragged: d.index,
      grabOffset: d.offset,
      pointerX: p.x - m.left,
      available: m.available,
      lastMovePoint: d.lastMove,
      justAttached,
    })
    d.left = r.left
    d.lastMove = r.lastMovePoint
    d.available = m.available
    d.slideMs = slideMs(this.deps.reducedMotion(), snap)
    if (r.send) {
      d.index = r.to
      d.pending.push(r.to)
      const args: TabDragReorder = { session: d.session, seq: this.nextSeq(d.session), index: r.to }
      invoke<void>(CMD_TAB_DRAG_REORDER, { args }).catch(err => {
        console.warn(`[useTabDrag] ${CMD_TAB_DRAG_REORDER} 실패:`, err)
      })
    }
    this.publish()
  }

  // 답을 기다리는 동안 움직임은 점만 적는다 — `not_ready` 면 구역 밖 다음 움직임에서 다시 청한다(답마다 곧바로 다시 청하면
  //   예비 창이 설 때까지 사람 움직임 없이 청이 돈다). 청하기 전에는 몰지 않는다 — Chrome 도 떠나는 움직임에서는 떼기만 하고
  //   탭바 안 몰기(`MoveAttached`)를 하지 않는다(tab_drag_controller.cc#1104-1109 · #1141-1144).
  private leave(d: Drive, p: Point): void {
    const wait: NonNullable<Drive['leaving']> = { held: null, last: p }
    d.leaving = wait
    const args: TabDragLeave = { session: d.session }
    const live = (): boolean => this.phase.kind === 'strip' && this.phase.d === d && d.leaving === wait
    invoke<LeaveReply>(CMD_TAB_DRAG_LEAVE, { args }).then(
      reply => {
        if (live()) this.answeredLeave(d, reply)
      },
      err => {
        if (!live()) return
        console.warn(`[useTabDrag] ${CMD_TAB_DRAG_LEAVE} 실패 — 끌기를 버린다:`, err)
        this.abandon()
      },
    )
  }

  private answeredLeave(d: Drive, reply: LeaveReply): void {
    const wait = d.leaving
    if (wait === null) return
    d.leaving = null
    if (reply === 'detached') {
      this.goAway(d.session, wait.held)
    } else if (reply === 'ended') {
      this.abandon()
    } else {
      // `not_ready` = 탭바에 머문다 — 구역 밖 점도 가로로 몬다(끄는 탭은 끌기 영역 끝에 잘려 선다 — TRD §2-2 · §2-5).
      this.drive(d, wait.last, false)
      if (wait.held !== null) this.sendEnd(d, wait.held)
    }
  }

  private endInput(d: Drive, input: EndInput): void {
    if (d.done) return
    if (d.leaving !== null) d.leaving.held ??= input
    else this.sendEnd(d, input)
  }

  private sendEnd(d: Drive, input: EndInput): void {
    const at = d.index
    const end: TabDragEnd =
      input.src === 'key'
        ? keyEnd(d.session, input.key, at)
        : input.src === 'capture_lost'
          ? { session: d.session, how: 'cancel', src: 'capture_lost', at }
          : { session: d.session, how: 'drop', src: input.src, at }
    sendDragEnd(end)
    d.done = true
    d.slideMs = slideMs(this.deps.reducedMotion(), false)
    this.listenPointer(false)
    this.listenKeys(false)
    if (d.input === 'pointer') this.releaseCapture(d.pointerId)
    this.publish()
  }

  // 창째 끌기 · 뗀 뒤 — 루프가 뗌을 보고(들어갈 때 이미 떨어졌으면 놓기), 키는 앱 전체 키 감시가 받는다. 붙든 포인터 끝은
  //   버리고 키 끝만 보낸다(TRD §4). 포인터 캡처는 셸 이동 루프가 가져가므로 놓지 않는다.
  private goAway(session: number, held: EndInput | null): void {
    if (held?.src === 'key') sendDragEnd(keyEnd(session, held.key, null))
    this.listenPointer(false)
    this.listenKeys(false)
    this.phase = { kind: 'away', session }
    this.publish()
  }

  private abandon(): void {
    const ph = this.phase
    const pointerId =
      ph.kind === 'begin' ? ph.pointerId : ph.kind === 'strip' && ph.d.input === 'pointer' ? ph.d.pointerId : null
    this.reset()
    if (pointerId !== null) this.releaseCapture(pointerId)
  }

  private reset(): void {
    this.listenPointer(false)
    this.listenKeys(false)
    const shown = this.phase.kind === 'strip'
    this.phase = IDLE
    if (shown) this.publish()
  }

  // ── 셈 ──

  private publish(): void {
    const ph = this.phase
    if (ph.kind !== 'strip') {
      this.deps.onView(null)
      return
    }
    const d = ph.d
    const others = this.others(d.view)
    const index = Math.min(d.index, others.length)
    const order = [...others.slice(0, index), d.view, ...others.slice(index)]
    const B = this.bounds(order.length, index, d.available)
    this.deps.onView({
      view: d.view,
      order,
      targets: slideTargets(B, index),
      left: d.done ? B.tabs[index].x0 : d.left,
      slideMs: d.slideMs,
      settling: d.done,
    })
  }

  // 끄는 탭이 활성이다 — 누름이 활성으로 만들고(TRD §2-2) 붙은 탭도 그 창의 활성이다.
  private bounds(count: number, index: number, available: number) {
    return stripBounds({ available, count, active: index, dragged: index, widthCap: this.widthCap })
  }

  private others(view: string): string[] {
    return this.tabs.filter(t => t !== view)
  }

  private nextSeq(session: number): number {
    if (this.seq.session !== session) this.seq = { session, n: 0 }
    this.seq.n += 1
    return this.seq.n
  }

  private releaseCapture(pointerId: number): void {
    const row = this.deps.row()
    if (row?.hasPointerCapture(pointerId)) row.releasePointerCapture(pointerId)
  }
}

/** 반열림 `[x0, x1) × [y0, y1)` — 셸 판정과 같은 읽기. */
function inZone(z: RectCss, p: Point): boolean {
  return p.x >= z.x0 && p.x < z.x1 && p.y >= z.y0 && p.y < z.y1
}

/** 잡은 폭 비율 — 누른 순간 머리의 그려진 사각형에서 잰다(0..1 로 자름 · 폭 0 이면 0). */
// Chromium(BSD) `TabDragController::Init`(누를 때 불림 — tab_strip.cc#2004)의 `mouse_offset_to_size_ratios.x`
//   (tab_drag_controller.cc#578-583)를 옮김 · 줄 번호 = `reorderGeometry.ts` 머리의 판.
function pressRatio(head: Element, x: number): number {
  const r = head.getBoundingClientRect()
  return r.width > 0 ? Math.min(1, Math.max(0, (x - r.left) / r.width)) : 0
}

function onControl(head: Element, target: EventTarget | null): boolean {
  const el = target instanceof Element ? target : target instanceof Node ? target.parentElement : null
  const control = el?.closest('input, button')
  return control != null && head.contains(control)
}

function sameList(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i])
}

export interface UseTabDragOptions {
  /** 이 탭바의 탭 id — 모델 순서. 그릴 때마다 새 배열이어도 된다. */
  tabs: readonly string[]
  /** 지금 활성 탭 — 누를 때 값이 `initial_active` 다. */
  active: string | null
  /** 탭바 줄 — 포인터 캡처 대상이고 그 사각형이 탭바 줄 · 끌기 영역 왼쪽이다(`measure.ts` `StripDom.row`). */
  row: RefObject<HTMLElement | null>
  /** `+` 단추 — 그 폭을 쓸 수 있는 폭에서 뺀다. */
  add: RefObject<HTMLElement | null>
  /** 닫기 모드의 고정 폭(`closingMode.layoutAvailable`) — 없으면 `null`. */
  widthCap?: number | null
}

export interface TabDrag {
  /** 이 탭바 안에서 탭을 모는 동안의 모습 — 그 밖 = `null`. */
  drag: TabDragView | null
  /**
   * 탭 머리의 `pointerdown` 에 단다 — 누름 순간의 활성(`switch_tab`)은 부르는 쪽 몫이다. 왼쪽 단추 · 닫기 단추와 이름
   * 입력 밖의 누름만 끌기 후보이고, 10 px 을 넘기 전에 떼면 아무것도 하지 않는다(click · dblclick 은 그대로 간다).
   */
  onPointerDown: (e: ReactPointerEvent<HTMLElement>, view: string) => void
}

/** 한 탭바의 탭 끌기(머리 주석). 끌기 세션 사건은 `dockSession` 의 `watchDockSession` 이 그 창에 걸려 있어야 온다. */
// ADR-0295
export function useTabDrag(options: UseTabDragOptions): TabDrag {
  const [drag, setDrag] = useState<TabDragView | null>(null)
  const live = useRef(options)
  useLayoutEffect(() => {
    live.current = options
  })
  const [ctl] = useState(
    () =>
      new TabDragController({
        row: () => live.current.row.current,
        measure: () => {
          const row = live.current.row.current
          return row === null ? null : measureStrip({ row, add: live.current.add.current })
        },
        reducedMotion: () => window.matchMedia(REDUCED_MOTION).matches,
        onView: setDrag,
      }),
  )
  const { tabs, active, widthCap = null } = options
  useLayoutEffect(() => {
    ctl.setModel(tabs, active, widthCap)
  }, [ctl, tabs, active, widthCap])
  useEffect(() => {
    const offFeed = subscribeFeed(f => ctl.feed(f))
    const offDrag = subscribeDrag(e => ctl.dragEvent(e))
    return () => {
      offFeed()
      offDrag()
      ctl.dispose()
    }
  }, [ctl])
  const onPointerDown = useCallback(
    (e: ReactPointerEvent<HTMLElement>, view: string) =>
      ctl.press({
        view,
        pointerId: e.pointerId,
        button: e.button,
        x: e.clientX,
        y: e.clientY,
        target: e.target,
        head: e.currentTarget,
      }),
    [ctl],
  )
  return { drag, onPointerDown }
}
