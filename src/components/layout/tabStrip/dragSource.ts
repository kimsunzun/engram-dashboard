// TRD S21-slot-docking §2-4 · §4-2 · §4-6: 출처 웹뷰가 내보낸 탭 끌기 하나를 끝까지 모는 한 곳 — 바꿔 그리기 풀기 · 원래 선택
//   되돌리기 · dragend 의 보내기 · 자기 탭바 순서 이동 모습을 여기서만 정한다. 정하는 근거는 끌기 id 와 모델 상태뿐이다 —
//   시한 · 사건이 닿는 차례에 기대지 않는다(풀기 · 되돌리기 규칙 = 순수 리듀서 `dock/dragOverride.ts` · 여기는 그것을 먹이고 그
//   답을 낸다). DOM 은 모른다 — 탭 머리 사건을 값으로 바꾸는 것은 `useTabDragSource` 다.
// 웹뷰마다 하나다 — 탭바(순서 이동 · 활성 칠)와 창 레이아웃(keep-alive 의 보이는 탭)이 같은 스냅숏을 읽는다.
// ★이 모듈은 끄는 동안(dragstart ~ dragend) 셸을 부르지 않는다★(ADR-0296 · ADR-0297) — 셸 호출은 dragend 의 `tab_drop`
//   하나와 `done` 뒤의 되돌리기(`switch_tab_if`) 하나뿐이고 둘 다 끌기가 끝난 뒤다.

import type { EndEffect } from '../../../../src-tauri/bindings/EndEffect'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'
import type { TabDrop } from '../../../../src-tauri/bindings/TabDrop'
import type { TabDropDone } from '../../../../src-tauri/bindings/TabDropDone'
import type { TabDropReply } from '../../../../src-tauri/bindings/TabDropReply'
import {
  IDLE,
  stepOverride,
  type OverrideEvent,
  type OverrideState,
  type Revert,
  type WindowModel,
} from '../dock/dragOverride'
import type { LocalDrag, SourceStripMove } from '../dock/localDrag'
import { moveDue, reorderThreshold } from './reorderGeometry'

/** 바깥 — 운영은 `useTabDragSource` 가 Tauri · 스토어로 채우고 시험은 가짜를 꽂는다(ADR-0012). */
export interface DragSourcePorts {
  send(drop: TabDrop): Promise<TabDropReply>
  /** 셸이 그 창 활성이 아직 `expect` 일 때만 `to` 로 바꾼다(TRD §4-6). */
  switchTabIf(window: string, to: string, expect: string): Promise<boolean>
  local: LocalDrag
  /** 다음 task 에 부른다 — 마이크로태스크가 아니어야 한다(`DragStart` 의 같은 task 판정). */
  nextTask(fn: () => void): void
}

/** dragstart 에서 출처가 셈한 것. 좌표 = 탭바 좌표(0 = 탭 끌기 영역 왼쪽). */
export interface DragStart {
  window: string
  drag: string
  view: string
  /** 누르기 전 그 창 모델 활성. */
  initial: string | null
  /** `DropTo.new_window` 의 크기 · 커서 자리(`dragSize.ts`). */
  size: SizeCss | null
  grab: PointCss
  grabOffset: number
  /** dragstart 때 끄는 머리 왼쪽 · 커서 x — 자기 탭바 위 첫 판정 값이다(누른 점은 탭바 구역 안). */
  left: number
  x: number
}

/** 자기 탭바 구역 안의 순서 이동 모습. */
export interface StripMotion {
  view: string
  /** 확정한 자리 — 끄는 탭을 뺀 목록 기준(= 놓은 뒤 결과 목록의 자리). 순서 문턱(`moveDue`)을 넘어야 바뀐다. */
  index: number
  /** 끄는 머리 왼쪽(탭바 좌표) — `settled` 면 쓰지 않는다. */
  left: number
  /** dragend 뒤 — 끄는 머리가 커서 대신 `index` 자리로 간다. */
  settled: boolean
}

export interface DragSourceSnapshot {
  /** 바꿔 그리기를 가진 창 — `override` 가 `idle` 이면 `null`. */
  window: string | null
  override: OverrideState
  /**
   * 그릴 순서 이동 — 자기 탭바 구역 안에서 그 끌기의 첫 판정(`setSourceStrip`)이 온 뒤에만 있다. dragstart 그 자리에서는
   * `null` 이라 탭바가 쉴 때와 같은 DOM 을 그린다(dragstart 안에서 끄는 머리가 누른 점 밑을 떠나면 엔진이 끌기를 접는다).
   * 그릴지는 부르는 쪽이 `stripHeld` 로 한 번 더 가른다(모델 순서가 dragstart 때와 다르면 거둔다).
   */
  motion: StripMotion | null
}

export interface DragSource {
  getSnapshot(): DragSourceSnapshot
  subscribe(cb: () => void): () => void
  /**
   * 그 창 스토어에 `window:tabs-updated`(· `list_tabs` 당기기)를 적용한 뒤의 탭 상태 — 적용마다 한 번(바뀐 것이 없어도 그 알림이
   * 왔다는 사실이 풀기 계기다 — TRD §2-4 ②). 그 창의 첫 값은 알림으로 치지 않는다.
   */
  model(window: string, m: WindowModel): void
  /** 그 창의 pointerdown(머리 밖 포함). */
  pointerDown(): void
  start(s: DragStart): void
  /** `localDrag` 의 칸이 바뀜 — 출처 창 탭바 판정(`sourceStrip`)을 읽는다. */
  stripChanged(): void
  /** 출처 머리의 dragend. `effect` = 그 `dropEffect`(`move` 밖은 `none`). */
  end(view: string, effect: EndEffect): void
  /** 이 창으로 온 `tab_drop:done`. */
  done(d: TabDropDone): void
  /**
   * 그 창 머리 누름이 모델 활성 탭(`view`)을 눌렀을 때도 `switch_tab` 을 보내야 하나 — 그 창에 `expect = view` 인 되돌리기를 냈고
   * 그 창 모델 활성이 아직 `view` 면 `true` 다. 보내지 않으면 뒤에 닿는 되돌리기가 이 누름을 덮는다(셸은 받은 차례대로 적용한다 —
   * 뒤에 보낸 `switch_tab` 이 되돌리기를 넘는다). `true` 를 한 번 돌려주면 그 되돌리기는 잊는다.
   */
  forceSwitch(window: string, view: string): boolean
}

const EMPTY: WindowModel = { tabs: [], active: null }

/** 끌기 하나 — dragend 의 보내기를 정하는 표다. */
interface Flight {
  s: DragStart
  /** dragstart 와 같은 task — 이 안의 dragend 는 엔진이 OS 끌기를 열지 못하고 접은 것이다. */
  sameTask: boolean
  ended: boolean
}

/** 자기 탭바 위 순서 이동의 셈(Chrome `DraggingTabsSession` 의 문턱 · 마지막 이동 점). */
interface Track {
  /** 확정한 자리 — `null` = 모델 순서(구역 밖이었다). */
  index: number | null
  left: number
  /** 마지막으로 자리가 실제로 바뀐 커서 x. */
  lastMove: number
  /** 구역에 (다시) 들어온 첫 판정 — 문턱 없이 셈한다. */
  justAttached: boolean
  inZone: boolean
  /** 뿌리의 첫 판정이 왔다. */
  tracking: boolean
}

export function createDragSource(ports: DragSourcePorts): DragSource {
  const models = new Map<string, WindowModel>()
  let owner: string | null = null
  let state: OverrideState = IDLE
  let flight: Flight | null = null
  let track: Track | null = null
  let lastStrip: SourceStripMove | null = null
  let snap: DragSourceSnapshot = { window: null, override: IDLE, motion: null }
  const listeners = new Set<() => void>()
  // 창마다 마지막으로 낸 되돌리기(`forceSwitch`) — 그 창 모델 활성이 `expect` 를 떠나거나(닿음 · 남이 바꿈) 그 invoke 가 오류면
  //   거둔다. 바꿔 그리기(`reverting`)와 따로 쥔다 — 그것은 그 창의 아무 pointerdown 에나 풀리지만 되돌리기는 여전히 날아간다.
  const reverts = new Map<string, Revert>()

  const modelOf = (w: string | null): WindowModel => (w === null ? EMPTY : (models.get(w) ?? EMPTY))

  function motionOf(): StripMotion | null {
    if (track === null || !track.inZone || !track.tracking) return null
    if (state.phase !== 'live' && state.phase !== 'ended') return null
    const index = track.index ?? modelOf(owner).tabs.indexOf(state.view)
    if (index < 0) return null
    return { view: state.view, index, left: track.left, settled: state.phase === 'ended' }
  }

  function publish(): void {
    const motion = motionOf()
    if (snap.window === owner && snap.override === state && sameMotion(snap.motion, motion)) return
    snap = { window: owner, override: state, motion }
    for (const cb of [...listeners]) cb()
  }

  function dispatch(event: OverrideEvent): void {
    const window = event.type === 'start' ? event.window : owner
    const before = state
    const step = stepOverride(state, event, modelOf(window))
    owner = window
    state = step.state
    if (state.phase === 'idle') {
      owner = null
      track = null
      // 걷는 것은 풀린 그 끌기의 것뿐이다 — 새 끌기의 dragstart 가 앞 끌기를 버리는 길은 여기를 지나지 않는다(`start` 가 덮는다).
      const out = ports.local.getOutgoing()
      if (out !== null && before.phase !== 'idle' && out.drag === before.drag) ports.local.setOutgoing(null)
    }
    if (step.revert !== null) revert(step.revert)
  }

  function revert(r: Revert): void {
    const drag = state.phase === 'reverting' ? state.drag : null
    reverts.set(r.window, r)
    ports.switchTabIf(r.window, r.to, r.expect).then(
      changed => {
        // 바꾸지 않았다 = 그사이 활성이 바뀌었다 — 그 알림(`tabs`)이 푼다.
        if (!changed) console.debug('[dragSource] switch_tab_if: 활성이 바뀌어 되돌리지 않았다', r)
      },
      err => {
        // 정상 경주다(되돌릴 탭이 그사이 닫힘 따위) — 로그만 남기고 모델 활성으로 물러난다.
        console.warn('[dragSource] switch_tab_if 실패 — 모델 활성으로 물러난다:', err)
        if (reverts.get(r.window) === r) reverts.delete(r.window)
        if (drag === null) return
        dispatch({ type: 'revertFailed', drag })
        publish()
      },
    )
  }

  function send(f: Flight, effect: EndEffect): void {
    const { s } = f
    const drop: TabDrop = {
      drag: s.drag,
      view: s.view,
      from: s.window,
      initial: s.initial,
      to: { type: 'new_window', by: { type: 'drag_end', effect }, size: s.size, grab: s.grab },
    }
    ports.send(drop).then(
      reply => console.debug('[dragSource] tab_drop(drag_end) 답:', reply),
      err => {
        console.warn('[dragSource] tab_drop(drag_end) 실패 — 바꿔 그리기를 곧바로 푼다:', err)
        dispatch({ type: 'sendFailed', drag: s.drag })
        publish()
      },
    )
  }

  // Chrome 순서 문턱 — 확정한 자리는 `moveDue` 가 참일 때만 판정의 자리로 바뀐다(`judge` 는 문턱을 보지 않는다 · TRD §7 U3).
  //   탭바 기하는 지금 그리는 순서(확정한 자리를 실은 순서)라 판정의 자리와 같은 기준이다.
  function follow(t: Track, view: string, m: SourceStripMove): void {
    const g = ports.local.readStripGeometry()
    const d = g === null ? -1 : g.tabs.indexOf(view)
    if (g !== null && d >= 0 && Number.isInteger(m.index) && m.index >= 0 && m.index < g.bounds.tabs.length) {
      const due = moveDue(t.lastMove, m.x, reorderThreshold(g.bounds, m.index), t.justAttached)
      if (due && m.index !== d) {
        t.index = m.index
        t.lastMove = m.x
      }
    }
    t.justAttached = false
  }

  return {
    getSnapshot: () => snap,

    subscribe(cb) {
      const entry = (): void => cb()
      listeners.add(entry)
      return () => {
        listeners.delete(entry)
      }
    },

    model(window, m) {
      const prev = models.get(window)
      models.set(window, m)
      if (m.active !== reverts.get(window)?.expect) reverts.delete(window)
      if (prev === undefined) return
      if (owner !== window) return
      dispatch({ type: 'tabs' })
      publish()
    },

    pointerDown() {
      if (state.phase === 'idle') return
      dispatch({ type: 'pointerdown' })
      publish()
    },

    start(s) {
      const f: Flight = { s, sameTask: true, ended: false }
      flight = f
      ports.nextTask(() => {
        f.sameTask = false
      })
      dispatch({ type: 'start', window: s.window, drag: s.drag, view: s.view, initial: s.initial })
      track = { index: null, left: s.left, lastMove: s.x, justAttached: true, inZone: true, tracking: false }
      // ★첫 판정을 나간 끌기 표시보다 먼저 둔다★ — 그 표시의 알림이 `stripChanged` 를 부르는데, 그때 앞 끌기의 마지막 판정
      //   (`null` 따위)이 남아 있으면 새 끌기를 「구역 밖」으로 읽는다.
      const first: SourceStripMove = { index: modelOf(s.window).tabs.indexOf(s.view), left: s.left, x: s.x }
      lastStrip = first
      ports.local.setSourceStrip(first)
      ports.local.setOutgoing({ drag: s.drag, window: s.window, view: s.view, grabOffset: s.grabOffset })
      publish()
    },

    stripChanged() {
      const m = ports.local.getSourceStrip()
      if (m === lastStrip) return
      lastStrip = m
      const f = flight
      const t = track
      if (f === null || f.ended || t === null || state.phase !== 'live' || state.drag !== f.s.drag) return
      if (m === null) {
        t.inZone = false
        t.index = null
        t.justAttached = true
      } else {
        follow(t, f.s.view, m)
        t.inZone = true
        t.left = m.left
        t.tracking = true
      }
      const out = m === null
      if (out !== state.out) dispatch({ type: 'zone', out })
      publish()
    },

    end(view, effect) {
      const f = flight
      if (f === null || f.ended || f.s.view !== view) return
      f.ended = true
      // 둘 다 끝을 셸에 보내지 않는다(TRD §2-4 ④ · U2 ⑨): 이 웹뷰의 drop 이 같은 창 같은 자리라 보내지 않았다 · 엔진이 접은
      //   끌기(옛 HTML5 끌기 실측 — dragstart 와 같은 task 의 dragend(none) · 그것을 보내면 셸은 마지막 점에 놓기로 읽어 새 창이
      //   뜬다). 표시는 늘 가져간다 — 남기면 다음 끌기가 그것을 제 것으로 읽는다.
      const marked = ports.local.takeLocalDrop(f.s.drag)
      const local = marked || f.sameTask
      // ★리듀서에 먼저 넣고 보낸다★(U2 ⑦) — dragend 전에 닿은 `done` 은 받은 창의 `Drop` 몫으로 갈린다.
      dispatch({ type: 'end', effect, local })
      publish()
      if (!local) send(f, effect)
    },

    done(d) {
      if (state.phase === 'idle' || state.drag !== d.drag) return
      dispatch({ type: 'done', drag: d.drag, outcome: d.outcome })
      publish()
    },

    forceSwitch(window, view) {
      if (reverts.get(window)?.expect !== view) return false
      reverts.delete(window)
      return true
    },
  }
}

function sameMotion(a: StripMotion | null, b: StripMotion | null): boolean {
  if (a === null || b === null) return a === b
  return a.view === b.view && a.index === b.index && a.left === b.left && a.settled === b.settled
}
