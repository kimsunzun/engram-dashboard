// TRD S21-slot-docking §8-2 `dragOverride`(리듀서 표 — 계기 ① ② ③ ④ · 시한 없음 · 되돌리기) · §8-3 출처 창 순서 시험
//   (fast-check `fc.scheduler()` — `done` · invoke 응답 · `window:tabs-updated` · `layout:updated` · dragend 를 아무 순서로 섞는다).

import fc from 'fast-check'
import { describe, expect, it } from 'vitest'

import type { DropOutcome } from '../../../../src-tauri/bindings/DropOutcome'
import type { EndEffect } from '../../../../src-tauri/bindings/EndEffect'
import {
  IDLE,
  type OverrideEvent,
  type OverrideState,
  type Revert,
  type WindowModel,
  shownTab,
  stepOverride,
  stripHeld,
} from './dragOverride'

const WIN = 'main'
const DRAG = 'drag-1'
/** 누르기 전 활성 A · 누름이 B 를 활성으로 했다(R2). */
const AT_START: WindowModel = { tabs: ['A', 'B', 'C'], active: 'B' }

/** 사건을 차례로 먹인다 — 마지막 상태와 낸 되돌리기의 `to` 목록. */
function run(events: Array<[OverrideEvent, WindowModel]>, from: OverrideState = IDLE) {
  let state = from
  const reverts: string[] = []
  for (const [ev, model] of events) {
    const out = stepOverride(state, ev, model)
    state = out.state
    if (out.revert !== null) reverts.push(out.revert.to)
  }
  return { state, reverts }
}

const start = (initial: string | null = 'A', view = 'B'): [OverrideEvent, WindowModel] => [
  { type: 'start', window: WIN, drag: DRAG, view, initial },
  AT_START,
]
const out = (m: WindowModel = AT_START): [OverrideEvent, WindowModel] => [{ type: 'zone', out: true }, m]
const end = (effect: EndEffect, local = false, m: WindowModel = AT_START): [OverrideEvent, WindowModel] => [
  { type: 'end', effect, local },
  m,
]
const done = (outcome: DropOutcome, m: WindowModel = AT_START, drag = DRAG): [OverrideEvent, WindowModel] => [
  { type: 'done', drag, outcome },
  m,
]
const tabs = (m: WindowModel): [OverrideEvent, WindowModel] => [{ type: 'tabs' }, m]

describe('dragOverride — 끄는 동안', () => {
  it('구역 밖이면 남은 탭(= initial ≠ view) · 돌아오면 끄는 탭', () => {
    let s = run([start()]).state
    expect(shownTab(s, AT_START)).toBe('B')
    s = run([out()], s).state
    expect(shownTab(s, AT_START)).toBe('A')
    s = run([[{ type: 'zone', out: false }, AT_START]], s).state
    expect(shownTab(s, AT_START)).toBe('B')
  })

  it('initial = view · 없음이면 남은 탭 = succession(오른쪽 → 왼쪽)', () => {
    expect(shownTab(run([start('B'), out()]).state, AT_START)).toBe('C')
    expect(shownTab(run([start(null), out()]).state, AT_START)).toBe('C')
    const lastTab: WindowModel = { tabs: ['A', 'B', 'C'], active: 'C' }
    const s = run([[{ type: 'start', window: WIN, drag: DRAG, view: 'C', initial: 'C' }, lastTab], out(lastTab)]).state
    expect(shownTab(s, lastTab)).toBe('B')
  })

  it('탭 하나뿐이면 바꿔 그림이 없다', () => {
    const one: WindowModel = { tabs: ['P'], active: 'P' }
    const s = run([[{ type: 'start', window: WIN, drag: DRAG, view: 'P', initial: 'P' }, one], out(one)]).state
    expect(shownTab(s, one)).toBe('P')
  })

  it('남은 탭이 그사이 닫혔으면 모델 활성을 그린다', () => {
    const s = run([start(), out()]).state
    expect(shownTab(s, { tabs: ['B', 'C'], active: 'B' })).toBe('B')
  })

  it('끄는 동안 순서 이동 모습을 그린다 · 순서가 바뀌면 그 프레임에 거둔다', () => {
    const s = run([start()]).state
    expect(stripHeld(s, AT_START)).toBe(true)
    expect(stripHeld(s, { tabs: ['B', 'A', 'C'], active: 'B' })).toBe(false)
    expect(stripHeld(IDLE, AT_START)).toBe(false)
  })
})

describe('dragOverride — 푸는 계기(시한 없음)', () => {
  it('① done{rejected} · 자기 invoke 오류 → 풂(되돌리지 않음)', () => {
    expect(run([start(), out(), end('none'), done('rejected')])).toEqual({ state: IDLE, reverts: [] })
    expect(run([start(), out(), end('none'), [{ type: 'sendFailed', drag: DRAG }, AT_START]])).toEqual({
      state: IDLE,
      reverts: [],
    })
  })

  it('① done{applied} 는 탭 목록이 dragstart 때와 달라진 뒤에 푼다 — 둘 중 늦게 닿은 쪽에서', () => {
    const moved: WindowModel = { tabs: ['A', 'C'], active: 'A' }
    // done 먼저 → 아직 그대로 → 목록 바뀜에서 풂.
    let s = run([start(), out(), end('move'), done('applied')]).state
    expect(s.phase).toBe('ended')
    expect(shownTab(s, AT_START)).toBe('A')
    expect(run([tabs(moved)], s).state).toBe(IDLE)
    // 목록 먼저(끄는 탭이 빠짐) → 그 자리에서 풂.
    s = run([start(), out(), end('move'), tabs(moved)]).state
    expect(s).toBe(IDLE)
    // 같은 창 순서 바꿈 — 끄는 탭이 활성인 채 순서만 바뀜 → done{applied} 가 닿아야 푼다.
    const reordered: WindowModel = { tabs: ['B', 'A', 'C'], active: 'B' }
    s = run([start(), end('move'), tabs(reordered)]).state
    expect(s.phase).toBe('ended')
    expect(run([done('applied', reordered)], s).state).toBe(IDLE)
  })

  it('① done{applied} — 탭 하나뿐인 창(옮겨진 팝아웃)은 목록이 그대로여도 푼다', () => {
    const one: WindowModel = { tabs: ['P'], active: 'P' }
    const s = run([
      [{ type: 'start', window: WIN, drag: DRAG, view: 'P', initial: 'P' }, one],
      out(one),
      end('none', false, one),
    ]).state
    expect(run([done('applied', one)], s).state).toBe(IDLE)
  })

  it('① 다른 끌기의 done · 오류는 듣지 않는다', () => {
    const s = run([start(), out(), end('none')]).state
    expect(run([done('canceled', AT_START, 'drag-0')], s).state).toBe(s)
    expect(run([[{ type: 'sendFailed', drag: 'drag-0' }, AT_START]], s).state).toBe(s)
  })

  it('자기 dragend 가 move 면 done{ignored} · done{canceled} 로는 풀지도 되돌리지도 않는다', () => {
    for (const outcome of ['ignored', 'canceled'] as const) {
      const r = run([start(), out(), end('move'), done(outcome)])
      expect(r.state.phase).toBe('ended')
      expect(r.reverts).toEqual([])
      expect(shownTab(r.state, AT_START)).toBe('A')
    }
  })

  it('dragend 전에 닿은 done{ignored}(받은 창의 Drop — no-cursor)도 move 와 같다', () => {
    const r = run([start(), out(), done('ignored'), end('move')])
    expect(r.state.phase).toBe('ended')
    expect(r.reverts).toEqual([])
  })

  it('② dragend 뒤 끄는 탭을 활성으로 두지 않는 tabs-updated → 풂 · 끄는 탭이 활성인 알림으로는 안 풂', () => {
    let s = run([start(), out(), end('move')]).state
    s = run([tabs(AT_START)], s).state
    expect(s.phase).toBe('ended')
    s = run([tabs({ tabs: ['A', 'B', 'C'], active: 'C' })], s).state
    expect(s).toBe(IDLE)
  })

  it('② 은 dragend 전에는 아니다 — 끄는 중 LLM 이 활성을 바꿔도 바꿔 그림이 남는다', () => {
    const s = run([start(), out(), tabs({ tabs: ['A', 'B', 'C'], active: 'C' })]).state
    expect(s.phase).toBe('live')
  })

  it('끄는 탭이 그 창을 떠나면(끄는 중 닫힘 포함) 언제든 푼다', () => {
    expect(run([start(), out(), tabs({ tabs: ['A', 'C'], active: 'C' })]).state).toBe(IDLE)
  })

  it('③ 다음 pointerdown · dragstart 가 푼다(dragend 를 잃은 끌기 포함)', () => {
    expect(run([start(), out(), [{ type: 'pointerdown' }, AT_START]]).state).toBe(IDLE)
    const next = run([start(), out(), [{ type: 'start', window: WIN, drag: 'drag-2', view: 'C', initial: 'B' }, AT_START]]).state
    expect(next.phase === 'live' && next.drag).toBe('drag-2')
    expect(next.phase === 'live' && next.out).toBe(false)
  })

  it('④ 같은 웹뷰 제자리 놓기 → dragend 에서 곧바로 풂 · 뒤이은 done{ignored} 는 무동작', () => {
    const r = run([start(), end('move', true), done('ignored')])
    expect(r).toEqual({ state: IDLE, reverts: [] })
    expect(stripHeld(r.state, AT_START)).toBe(false)
  })

  it('④ 엔진이 접은 끌기(dragstart 와 같은 task 의 dragend · 보내지 않음) → 곧바로 풂 · 되돌리지 않음', () => {
    const r = run([start(), end('none', true)])
    expect(r).toEqual({ state: IDLE, reverts: [] })
    expect(shownTab(r.state, AT_START)).toBe('B')
  })

  it('dragend 뒤 zone 은 듣지 않는다', () => {
    const s = run([start(), out(), end('none')]).state
    expect(run([[{ type: 'zone', out: false }, AT_START]], s).state).toBe(s)
  })
})

describe('dragOverride — 원래 선택 되돌리기(§4-6)', () => {
  it('자기 dragend none + done{canceled|ignored} → switch_tab(initial) 한 번 · 닿을 때까지 initial 을 그린다', () => {
    for (const outcome of ['canceled', 'ignored'] as const) {
      const r = run([start(), out(), end('none'), done(outcome)])
      expect(r.reverts).toEqual(['A'])
      expect(r.state).toEqual({ phase: 'reverting', drag: DRAG, view: 'B', to: 'A' })
      // 모델은 아직 누른 탭(B)이 활성이다 — 그래도 initial 을 그린다(번쩍임 없음).
      expect(shownTab(r.state, AT_START)).toBe('A')
      expect(stripHeld(r.state, AT_START)).toBe(false)
    }
  })

  it('되돌리기 = switch_tab_if(출처 창, initial, expect = 끄는 탭)', () => {
    let state: OverrideState = IDLE
    let revert: Revert | null = null
    for (const [ev, model] of [start(), out(), end('none'), done('canceled')]) {
      ;({ state, revert } = stepOverride(state, ev, model))
    }
    expect(revert).toEqual({ window: WIN, to: 'A', expect: 'B' })
  })

  it('구역 안에서 취소해도 되돌리기를 기다리는 동안 initial 을 그린다', () => {
    const r = run([start(), end('none'), done('canceled')])
    expect(shownTab(r.state, AT_START)).toBe('A')
  })

  it('되돌리기를 기다림 — 활성이 initial 로 닿으면 풂 · 끄는 탭이 활성인 알림 · done · zone 으로는 안 풂', () => {
    const s = run([start(), out(), end('none'), done('canceled')]).state
    expect(run([tabs(AT_START)], s).state).toBe(s)
    expect(run([done('applied'), done('rejected'), [{ type: 'sendFailed', drag: DRAG }, AT_START]], s).state).toBe(s)
    expect(run([[{ type: 'zone', out: false }, AT_START]], s).state).toBe(s)
    const back: WindowModel = { tabs: ['A', 'B', 'C'], active: 'A' }
    expect(run([tabs(back)], s)).toEqual({ state: IDLE, reverts: [] })
  })

  it('되돌리기를 기다림 — 그물: LLM 이 다른 탭으로 바꿈(②) · 끄는 탭이 떠남 · pointerdown(③) · 다음 dragstart', () => {
    const s = run([start(), out(), end('none'), done('ignored')]).state
    const llm: WindowModel = { tabs: ['A', 'B', 'C'], active: 'C' }
    expect(run([tabs(llm)], s).state).toBe(IDLE)
    expect(shownTab(IDLE, llm)).toBe('C')
    expect(run([tabs({ tabs: ['A', 'C'], active: 'A' })], s).state).toBe(IDLE)
    expect(run([[{ type: 'pointerdown' }, AT_START]], s).state).toBe(IDLE)
    expect(run([[{ type: 'start', window: WIN, drag: 'drag-2', view: 'C', initial: 'B' }, AT_START]], s).state.phase).toBe('live')
  })

  it('되돌릴 탭이 그사이 닫혔으면 모델 활성을 그린다', () => {
    const s = run([start(), out(), end('none'), done('canceled')]).state
    expect(shownTab(s, { tabs: ['B', 'C'], active: 'B' })).toBe('B')
  })

  it('건너뜀 — initial 없음 · initial = view · 그 창 탭이 아님 · 그 창 활성이 이미 view 가 아님 · rejected · applied', () => {
    expect(run([start(null), end('none'), done('canceled')]).reverts).toEqual([])
    expect(run([start('B'), end('none'), done('canceled')]).reverts).toEqual([])
    expect(run([start('Z'), end('none'), done('canceled')]).reverts).toEqual([])
    const llm: WindowModel = { tabs: ['A', 'B', 'C'], active: 'C' }
    expect(run([start(), end('none'), done('canceled', llm)]).reverts).toEqual([])
    expect(run([start(), end('none'), done('rejected')]).reverts).toEqual([])
    expect(run([start(), end('none'), done('applied', { tabs: ['A', 'C'], active: 'A' })]).reverts).toEqual([])
  })

  it('많아야 한 번 — 같은 끌기의 둘째 done 은 무동작', () => {
    expect(run([start(), end('none'), done('ignored'), done('canceled'), done('applied')]).reverts).toEqual(['A'])
    const back: WindowModel = { tabs: ['A', 'B', 'C'], active: 'A' }
    expect(run([start(), end('none'), done('ignored'), tabs(back), done('canceled', back)]).reverts).toEqual(['A'])
  })
})

// ── §8-3 순서 시험 ──────────────────────────────────────────────────────────────

/** 출처 창에 닿는 것 하나. `after` = 출처 dragend 뒤에야 생기는 것(그 dragend 가 보낸 `DragEnd` 의 결과). */
interface Arrival {
  label: string
  event: OverrideEvent | null
  /** 이것이 닿으면 그 창 스토어가 이 값이 된다(`window:tabs-updated`). */
  model?: WindowModel
  after?: boolean
}

interface Scenario {
  name: string
  initial: string | null
  /** 놓은 곳 — 커서가 자기 탭바 구역 밖이었나. */
  out: boolean
  effect: EndEffect
  local?: boolean
  arrivals: Arrival[]
  /** 끝 상태가 idle 이어야 하나(그물 없이 풀리는 갈래). */
  releases: boolean
  /** 끄는 탭이 그 창을 떠난다 — 바꿔 그린 뒤로 한 프레임도 끄는 탭을 그리지 않는다(②). */
  leaves?: boolean
  /** 같은 창 순서 바꿈 — transform 을 거둔 프레임 = 모델 순서가 바뀐 프레임(④). */
  reorder?: boolean
  /** 되돌리기 수(정확히). `undefined` = ≤ 1 만 본다. */
  reverts?: number
  /** 되돌리기의 `switch_tab_if` 가 거절되거나 잃어 그 알림이 오지 않는다. */
  revertLost?: boolean
  /** 바꿔 그린 뒤로 끄는 탭(B)을 한 프레임도 그리지 않는다 — 되돌리기의 번쩍임 없음(메인 결정 2026-10-20). */
  noFlash?: boolean
}

const doneOf = (outcome: DropOutcome, after = false): Arrival => ({
  label: `done:${outcome}${after ? '(DragEnd)' : '(Drop)'}`,
  event: { type: 'done', drag: DRAG, outcome },
  after,
})
const tabsOf = (label: string, model: WindowModel, after = false): Arrival => ({ label, event: { type: 'tabs' }, model, after })
/** 상태를 바꾸지 않는 소음 — `layout:updated` · invoke 응답(오류 아님). */
const noise = (label: string, after = false): Arrival => ({ label, event: null, after })

const MOVED_OUT: WindowModel = { tabs: ['A', 'C'], active: 'A' }

const SCENARIOS: Scenario[] = [
  {
    name: '다른 창에 놓음(Drop 적용 · DragEnd 무시)',
    initial: 'A',
    out: true,
    effect: 'move',
    arrivals: [
      tabsOf('tabs:moved-out', MOVED_OUT),
      noise('layout:updated'),
      doneOf('applied'),
      noise('reply:drop'),
      doneOf('ignored', true),
      noise('reply:drag-end', true),
    ],
    releases: true,
    leaves: true,
    reverts: 0,
  },
  {
    name: '다른 창에 놓음 — 이미 활성인 탭을 끔(남은 탭 = succession)',
    initial: 'B',
    out: true,
    effect: 'move',
    arrivals: [tabsOf('tabs:moved-out', { tabs: ['A', 'C'], active: 'C' }), doneOf('applied'), doneOf('ignored', true)],
    releases: true,
    leaves: true,
    reverts: 0,
  },
  {
    name: '같은 창 이음새(남은 탭 A 에 붙음)',
    initial: 'A',
    out: true,
    effect: 'move',
    arrivals: [tabsOf('tabs:docked', MOVED_OUT), noise('layout:updated'), doneOf('applied'), doneOf('ignored', true)],
    releases: true,
    leaves: true,
    reverts: 0,
  },
  {
    name: '창 밖 새 창(DragEnd 적용)',
    initial: 'A',
    out: true,
    effect: 'none',
    arrivals: [tabsOf('tabs:placed', MOVED_OUT, true), doneOf('applied', true), noise('reply:drag-end', true)],
    releases: true,
    leaves: true,
    reverts: 0,
  },
  {
    name: '같은 창 순서 바꿈(자기 탭바)',
    initial: 'A',
    out: false,
    effect: 'move',
    arrivals: [
      tabsOf('tabs:reordered', { tabs: ['B', 'A', 'C'], active: 'B' }),
      noise('layout:updated'),
      doneOf('applied'),
      doneOf('ignored', true),
    ],
    releases: true,
    reorder: true,
    reverts: 0,
  },
  {
    name: 'Esc 취소 → 되돌리기(늦게 닿음 — 번쩍임 없음)',
    initial: 'A',
    out: true,
    effect: 'none',
    arrivals: [doneOf('canceled', true), noise('reply:drag-end', true), noise('layout:updated', true)],
    releases: true,
    reverts: 1,
    noFlash: true,
  },
  {
    name: '잃은 놓기(무시) → 되돌리기가 끝내 안 닿음 — 그물 pointerdown 이 푼다',
    initial: 'A',
    out: true,
    effect: 'none',
    arrivals: [
      doneOf('ignored', true),
      noise('reply:drag-end', true),
      { label: 'pointerdown', event: { type: 'pointerdown' }, after: true },
    ],
    releases: true,
    revertLost: true,
  },
  {
    name: '잃은 놓기(무시) — 그사이 LLM 이 활성을 C 로',
    initial: 'A',
    out: true,
    effect: 'none',
    arrivals: [doneOf('ignored', true), tabsOf('tabs:llm-switch', { tabs: ['A', 'B', 'C'], active: 'C' })],
    releases: true,
  },
  {
    name: 'Drop 거절(탭 제자리 · 누른 탭이 활성으로 남음)',
    initial: 'A',
    out: true,
    effect: 'move',
    arrivals: [doneOf('rejected'), doneOf('ignored', true), noise('reply:drop')],
    releases: true,
    reverts: 0,
  },
  {
    name: 'Drop 무시(no-cursor) — 그물(pointerdown)이 푼다',
    initial: 'A',
    out: true,
    effect: 'move',
    arrivals: [
      doneOf('ignored'),
      doneOf('ignored', true),
      { label: 'pointerdown', event: { type: 'pointerdown' }, after: true },
    ],
    releases: true,
    reverts: 0,
  },
  {
    name: '같은 웹뷰 제자리 놓기',
    initial: 'A',
    out: false,
    effect: 'move',
    local: true,
    arrivals: [doneOf('ignored', true), noise('reply:drag-end', true)],
    releases: true,
    reverts: 0,
  },
  {
    name: '자기 DragEnd invoke 오류',
    initial: 'A',
    out: true,
    effect: 'none',
    arrivals: [{ label: 'reply:error', event: { type: 'sendFailed', drag: DRAG }, after: true }],
    releases: true,
    reverts: 0,
  },
]

interface Frame {
  label: string
  state: OverrideState
  model: WindowModel
}

/** 시나리오 하나를 scheduler 가 고른 순서로 돌린다 — 프레임(사건 하나를 먹인 뒤)마다 상태 · 스토어를 적는다. */
async function play(sc: Scenario, s: fc.Scheduler) {
  let model = AT_START
  let state = run([[{ type: 'start', window: WIN, drag: DRAG, view: 'B', initial: sc.initial }, AT_START]]).state
  if (sc.out) state = run([out()], state).state
  const frames: Frame[] = [{ label: 'start', state, model }]
  const reverts: string[] = []
  let ended = false

  const deliver = (a: Arrival) => {
    if (a.model !== undefined) model = a.model
    if (a.event !== null) {
      const step = stepOverride(state, a.event, model)
      state = step.state
      const revert = step.revert
      if (revert !== null) {
        reverts.push(revert.to)
        // 되돌리기의 `switch_tab_if` 왕복 — 셸에 닿은 때 그 창 활성이 아직 `expect` 면 `to` 로 바꾸고 알림이 뒤따른다.
        //   그사이 다른 탭이 골라졌으면 셸은 아무것도 안 바꾸고 알리지 않는다.
        if (!sc.revertLost) {
          void s.schedule(Promise.resolve(), 'switch_tab_if').then(() => {
            if (model.active !== revert.expect || !model.tabs.includes(revert.to)) return
            deliver(tabsOf('tabs:revert', { tabs: model.tabs, active: revert.to }))
          })
        }
      }
    }
    frames.push({ label: a.label, state, model })
  }

  const scheduleAll = (after: boolean) => {
    for (const a of sc.arrivals.filter(x => (x.after ?? false) === after)) {
      void s.schedule(Promise.resolve(), a.label).then(() => deliver(a))
    }
  }

  scheduleAll(false)
  void s.schedule(Promise.resolve(), 'dragend').then(() => {
    ended = true
    deliver({ label: 'dragend', event: { type: 'end', effect: sc.effect, local: sc.local ?? false } })
    scheduleAll(true)
  })
  await s.waitIdle()
  expect(ended).toBe(true)
  return { frames, reverts, state, model }
}

describe('dragOverride — 순서 시험(fc.scheduler)', () => {
  for (const sc of SCENARIOS) {
    it(sc.name, async () => {
      await fc.assert(
        fc.asyncProperty(fc.scheduler(), async s => {
          const { frames, reverts, state, model } = await play(sc, s)
          const where = () => `${s.report().map(r => r.label).join(' → ')}`

          // ① 풀려야 하는 갈래는 끝에 풀렸다 · ③ 풀린 뒤 그리는 탭 = 모델 활성.
          if (sc.releases) expect(state, where()).toBe(IDLE)
          expect(shownTab(state, model), where()).toBe(model.active)

          // ② 끄는 탭이 그 창을 떠나는 갈래 — 바꿔 그린 뒤로는 한 프레임도 끄는 탭을 그리지 않는다(시한 뒤집기 없음).
          if (sc.leaves) {
            for (const f of frames) expect(shownTab(f.state, f.model), `${f.label} @ ${where()}`).not.toBe('B')
          }

          // ④ 같은 창 순서 바꿈 — transform 은 모델 순서가 바뀐 그 프레임까지 남고, 그 프레임에 거둔다.
          if (sc.reorder) {
            for (const f of frames) {
              const sameOrder = f.model.tabs.join() === AT_START.tabs.join()
              expect(stripHeld(f.state, f.model), `${f.label} @ ${where()}`).toBe(sameOrder)
            }
          }

          // 되돌리기를 기다리는 동안 끄는 탭을 그리지 않는다 — 그물(pointerdown)이 푼 뒤는 모델 활성(누른 탭)이라 뺀다.
          if (sc.noFlash) {
            for (const f of frames) {
              if (f.label === 'pointerdown') break
              expect(shownTab(f.state, f.model), `${f.label} @ ${where()}`).not.toBe('B')
            }
          }
          // 되돌리기가 안 닿아도 그물(pointerdown)이 푼다 — done 이 그보다 먼저면 되돌리기를 냈고 그때까지 initial 을 그렸다.
          if (sc.revertLost) {
            const at = (label: string) => frames.findIndex(f => f.label === label)
            const doneFirst = at('done:ignored(DragEnd)') < at('pointerdown')
            expect(reverts.length, where()).toBe(doneFirst ? 1 : 0)
            for (const f of frames.slice(0, at('pointerdown'))) {
              expect(shownTab(f.state, f.model), `${f.label} @ ${where()}`).not.toBe('B')
            }
          }

          // ⑤ 되돌리기는 많아야 한 번 · 갈래마다 정확한 수.
          expect(reverts.length, where()).toBeLessThanOrEqual(1)
          if (sc.reverts !== undefined) expect(reverts.length, where()).toBe(sc.reverts)
        }),
        { numRuns: 200 },
      )
    })
  }

  it('⑤ LLM 의 활성 바꿈이 done 보다 먼저 닿으면 되돌리지 않는다 · 늦으면 한 번 — 끝 화면은 늘 모델 활성', async () => {
    const sc = SCENARIOS.find(x => x.name.includes('그사이 LLM'))!
    await fc.assert(
      fc.asyncProperty(fc.scheduler(), async s => {
        const { reverts, state, model, frames } = await play(sc, s)
        const llmFirst =
          frames.findIndex(f => f.label === 'tabs:llm-switch') < frames.findIndex(f => f.label.startsWith('done:'))
        expect(reverts.length).toBe(llmFirst ? 0 : 1)
        expect(state).toBe(IDLE)
        expect(shownTab(state, model)).toBe(model.active)
      }),
      { numRuns: 200 },
    )
  })

  it('⑤ 되돌리기가 셸에 닿기 전에 다른 탭(C)이 골라지면 C 가 남는다 — 되돌리기가 덮지 않는다', async () => {
    const pick: WindowModel = { tabs: ['A', 'B', 'C'], active: 'C' }
    const sc: Scenario = {
      name: 'pick C during revert',
      initial: 'A',
      out: true,
      effect: 'none',
      arrivals: [doneOf('canceled', true), tabsOf('tabs:pick-c', pick, true)],
      releases: true,
    }
    let raced = 0
    await fc.assert(
      fc.asyncProperty(fc.scheduler(), async s => {
        const { reverts, state, model, frames } = await play(sc, s)
        const where = () => `${s.report().map(r => r.label).join(' → ')}`
        expect(state, where()).toBe(IDLE)
        expect(model.active, where()).toBe('C')
        expect(shownTab(state, model), where()).toBe('C')
        // 되돌리기를 냈고 C 가 그 왕복보다 먼저 닿은 갈래 — 옛 무조건 `switch_tab` 이면 A 로 덮였다.
        if (reverts.length === 1 && !frames.some(f => f.label === 'tabs:revert')) raced++
      }),
      { numRuns: 200 },
    )
    expect(raced).toBeGreaterThan(0)
  })

  it('⑥ dragend 뒤 끄는 탭을 활성으로 두지 않는 tabs-updated 가 오면 그 뒤 그리는 탭 = 모델 활성(늦게 남은 바꿔 그림이 덮지 않음)', async () => {
    const llm: WindowModel = { tabs: ['A', 'B', 'C'], active: 'C' }
    const sc: Scenario = {
      name: 'LLM switch after dragend',
      initial: 'A',
      out: true,
      effect: 'move',
      // 받은 창의 Drop 을 잃었다 — done 은 DragEnd 의 무시뿐이다.
      arrivals: [doneOf('ignored', true), tabsOf('tabs:llm-switch', llm, true)],
      releases: true,
      reverts: 0,
    }
    await fc.assert(
      fc.asyncProperty(fc.scheduler(), async s => {
        const { frames } = await play(sc, s)
        const at = frames.findIndex(f => f.label === 'tabs:llm-switch')
        for (const f of frames.slice(at)) expect(shownTab(f.state, f.model)).toBe(f.model.active)
      }),
      { numRuns: 100 },
    )
  })
})
