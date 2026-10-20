// TRD S21-slot-docking §8-2 「훅 · 컴포넌트」의 출처 몫(정하는 곳 `dragSource` — 보내기 · 풀기 · 되돌리기 · 순서 이동 확정) ·
//   §8-3 출처 창 순서 시험(fast-check `fc.scheduler()` — `done` · invoke 응답 · `window:tabs-updated` · `layout:updated` ·
//   dragend 를 아무 순서로 섞는다). 바깥은 가짜 포트(ADR-0012) · 탭바 기하는 탭바가 그리는 셈을 그대로 흉내 낸다.

import fc from 'fast-check'
import { describe, expect, it, vi } from 'vitest'

import type { DropOutcome } from '../../../../src-tauri/bindings/DropOutcome'
import type { EndEffect } from '../../../../src-tauri/bindings/EndEffect'
import type { TabDrop } from '../../../../src-tauri/bindings/TabDrop'
import type { TabDropReply } from '../../../../src-tauri/bindings/TabDropReply'
import { shownTab, stripHeld, type WindowModel } from '../dock/dragOverride'
import { createLocalDrag, type LocalDrag, type StripGeometry } from '../dock/localDrag'
import { createDragSource, type DragSource, type DragSourcePorts, type DragSourceSnapshot, type DragStart } from './dragSource'
import { stripBounds } from './measure'

const WIN = 'main'
const DRAG = 'drag-1'
/** 누르기 전 활성 A · 누름이 B 를 활성으로 했다(R2). 탭 셋 · 쓸 수 있는 폭 480 → 머리 160 씩. */
const AT_START: WindowModel = { tabs: ['A', 'B', 'C'], active: 'B' }
const START: DragStart = {
  window: WIN,
  drag: DRAG,
  view: 'B',
  initial: 'A',
  size: { w: 800, h: 600 },
  grab: { x: 80, y: 14 },
  grabOffset: 80,
  left: 160,
  x: 244,
}

/** 탭바가 그리는 순서 · 경계 그대로(`TabBar` 의 셈). */
function geometryOf(snap: DragSourceSnapshot, model: WindowModel): StripGeometry {
  const motion = snap.window === WIN && stripHeld(snap.override, model) ? snap.motion : null
  const others = motion === null ? [...model.tabs] : model.tabs.filter(t => t !== motion.view)
  const order = motion === null ? others : [...others.slice(0, motion.index), motion.view, ...others.slice(motion.index)]
  const at = motion === null ? -1 : order.indexOf(motion.view)
  const bounds = stripBounds({ available: 480, count: order.length, active: at, dragged: at >= 0 ? at : undefined })
  return { left: 0, bounds, available: 480, zoneViewport: { x0: 0, y0: -15, x1: 480, y1: 43 }, tabs: order }
}

interface Harness {
  src: DragSource
  local: LocalDrag
  ports: DragSourcePorts
  sends: TabDrop[]
  switches: Array<[string, string, string]>
  later: Array<() => void>
  setModel(m: WindowModel): void
  model(): WindowModel
  shown(): string | null
}

function harness(over: Partial<Pick<DragSourcePorts, 'send' | 'switchTabIf'>> = {}): Harness {
  const local = createLocalDrag()
  const sends: TabDrop[] = []
  const switches: Array<[string, string, string]> = []
  const later: Array<() => void> = []
  let model = AT_START
  const ports: DragSourcePorts = {
    send: drop => {
      sends.push(drop)
      return over.send?.(drop) ?? Promise.resolve({ type: 'ignored', why: 'lost-drop' })
    },
    switchTabIf: (w, to, expect) => {
      switches.push([w, to, expect])
      return over.switchTabIf?.(w, to, expect) ?? Promise.resolve(true)
    },
    local,
    nextTask: fn => void later.push(fn),
  }
  const src = createDragSource(ports)
  src.model(WIN, model)
  local.subscribe(() => src.stripChanged())
  local.registerStripGeometry(() => geometryOf(src.getSnapshot(), model))
  return {
    src,
    local,
    ports,
    sends,
    switches,
    later,
    setModel(m) {
      model = m
      src.model(WIN, m)
    },
    model: () => model,
    shown() {
      const s = src.getSnapshot()
      return s.window === WIN ? shownTab(s.override, model) : model.active
    },
  }
}

/** dragstart 를 지난 task 로 보낸다 — 엔진이 접은 끌기가 아니다. */
function started(h: Harness, s: DragStart = START): void {
  h.src.start(s)
  for (const fn of h.later.splice(0)) fn()
}

const tick = () => new Promise(resolve => setTimeout(resolve, 0))

describe('dragSource — dragstart', () => {
  it('바깥 표시를 세운다 — 나간 끌기 · 출처 탭바 첫 판정(누른 점 = 구역 안) · 그리는 순서 이동은 아직 없음', () => {
    const h = harness()
    h.src.start(START)
    expect(h.local.getOutgoing()).toEqual({ drag: DRAG, window: WIN, view: 'B', grabOffset: 80 })
    expect(h.local.getSourceStrip()).toEqual({ index: 1, left: 160, x: 244 })
    const snap = h.src.getSnapshot()
    expect(snap.window).toBe(WIN)
    expect(snap.override.phase).toBe('live')
    expect(snap.motion).toBeNull()
    expect(h.shown()).toBe('B')
    expect(h.sends).toEqual([])
  })
})

describe('dragSource — 끄는 동안(출처 탭바 판정)', () => {
  it('구역 밖(null) = 남은 탭 · 돌아오면 끄는 탭 · 순서 이동은 구역 안 판정이 온 뒤에만', () => {
    const h = harness()
    started(h)
    h.local.setSourceStrip(null)
    expect(h.shown()).toBe('A')
    expect(h.src.getSnapshot().motion).toBeNull()
    h.local.setSourceStrip({ index: 1, left: 170, x: 254 })
    expect(h.shown()).toBe('B')
    expect(h.src.getSnapshot().motion).toEqual({ view: 'B', index: 1, left: 170, settled: false })
  })

  it('순서 문턱 — 구역에 든 첫 판정은 곧바로 확정 · 그 뒤는 마지막 이동 점에서 문턱(16 px)을 넘어야 판정의 자리로', () => {
    const h = harness()
    started(h)
    // 첫 판정(justAttached) — 커서가 거의 안 움직였어도 자리 2 를 확정.
    h.local.setSourceStrip({ index: 2, left: 250, x: 250 })
    expect(h.src.getSnapshot().motion?.index).toBe(2)
    // 문턱 안의 되돌림 — 머리만 따른다.
    h.local.setSourceStrip({ index: 1, left: 240, x: 240 })
    expect(h.src.getSnapshot().motion).toEqual({ view: 'B', index: 2, left: 240, settled: false })
    // 문턱을 넘으면 그 판정의 자리로.
    h.local.setSourceStrip({ index: 1, left: 200, x: 200 })
    expect(h.src.getSnapshot().motion?.index).toBe(1)
    // 구역을 떠나면 모델 순서 · 다시 들면 곧바로 확정.
    h.local.setSourceStrip(null)
    expect(h.src.getSnapshot().motion).toBeNull()
    h.local.setSourceStrip({ index: 2, left: 205, x: 205 })
    expect(h.src.getSnapshot().motion?.index).toBe(2)
  })

  it('dragend 뒤의 판정은 듣지 않는다 — 끄는 머리는 확정한 자리로(settled)', () => {
    const h = harness()
    started(h)
    h.local.setSourceStrip({ index: 2, left: 330, x: 420 })
    h.src.end('B', 'move')
    expect(h.src.getSnapshot().motion).toEqual({ view: 'B', index: 2, left: 330, settled: true })
    h.local.setSourceStrip(null)
    expect(h.src.getSnapshot().motion).toEqual({ view: 'B', index: 2, left: 330, settled: true })
    expect(h.shown()).toBe('B')
  })

  it('끄는 중 그 탭이 그 창에서 빠지면(LLM 닫기 · 옮김) 바꿔 그리기를 푼다', () => {
    const h = harness()
    started(h)
    h.local.setSourceStrip(null)
    h.setModel({ tabs: ['A', 'C'], active: 'A' })
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    expect(h.local.getOutgoing()).toBeNull()
  })
})

describe('dragSource — dragend', () => {
  it.each(['none', 'move'] as EndEffect[])(
    'effect=%s → 리듀서에 끝을 먼저 넣고 NewWindow{by: DragEnd{effect}} 하나 · 두 번째 dragend 무동작',
    effect => {
      let phaseAtSend: string | null = null
      const h = harness({
        send: () => {
          phaseAtSend = h.src.getSnapshot().override.phase
          return Promise.resolve({ type: 'ignored', why: 'accepted-ours' })
        },
      })
      started(h)
      h.src.end('B', effect)
      h.src.end('B', effect)
      expect(phaseAtSend).toBe('ended')
      expect(h.sends).toEqual([
        {
          drag: DRAG,
          view: 'B',
          from: WIN,
          initial: 'A',
          to: { type: 'new_window', by: { type: 'drag_end', effect }, size: { w: 800, h: 600 }, grab: { x: 80, y: 14 } },
        },
      ])
    },
  )

  it('같은 웹뷰 제자리 놓기(표시) → 보내지 않음 · 곧바로 풂(순서 이동 거둠) · 표시를 가져감', () => {
    const h = harness()
    started(h)
    h.local.setSourceStrip({ index: 2, left: 330, x: 420 })
    h.local.markLocalDrop(DRAG)
    h.src.end('B', 'move')
    expect(h.sends).toEqual([])
    expect(h.src.getSnapshot()).toEqual({ window: null, override: { phase: 'idle' }, motion: null })
    expect(h.local.getOutgoing()).toBeNull()
    expect(h.local.takeLocalDrop(DRAG)).toBe(false)
  })

  it('dragstart 와 같은 task 의 dragend(엔진이 접음) → 보내지 않음 · 풂 · 그 뒤 done 으로 되돌리지 않음', () => {
    const h = harness()
    h.src.start(START)
    h.src.end('B', 'none')
    for (const fn of h.later.splice(0)) fn()
    expect(h.sends).toEqual([])
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    h.src.done({ drag: DRAG, outcome: 'canceled' })
    expect(h.switches).toEqual([])
  })

  it('다른 머리 · 끌기 밖의 dragend 는 무동작', () => {
    const h = harness()
    h.src.end('B', 'none')
    started(h)
    h.src.end('C', 'none')
    expect(h.sends).toEqual([])
    expect(h.src.getSnapshot().override.phase).toBe('live')
  })

  it('invoke 오류 → 곧바로 풂(그 끌기만)', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const h = harness({ send: () => Promise.reject(new Error('ipc down')) })
    started(h)
    h.local.setSourceStrip(null)
    h.src.end('B', 'none')
    expect(h.shown()).toBe('A')
    await tick()
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    expect(h.shown()).toBe('B')
    vi.restoreAllMocks()
  })
})

describe('dragSource — 결과 알림 · 원래 선택 되돌리기(§4-6)', () => {
  it('done{canceled}(자기 dragend none) → switch_tab_if(창, initial, expect = 끄는 탭) 한 번 · 닿으면 풂', () => {
    const h = harness()
    started(h)
    h.src.end('B', 'none')
    h.src.done({ drag: DRAG, outcome: 'canceled' })
    h.src.done({ drag: DRAG, outcome: 'canceled' })
    expect(h.switches).toEqual([[WIN, 'A', 'B']])
    expect(h.shown()).toBe('A')
    h.setModel({ tabs: ['A', 'B', 'C'], active: 'A' })
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    expect(h.local.getOutgoing()).toBeNull()
  })

  it('되돌리지 않음 — 활성이 그사이 바뀜 · initial = 끄는 탭 · Rejected · 자기 dragend move · 남의 끌기', () => {
    const run = (outcome: DropOutcome, effect: EndEffect, setup?: (h: Harness) => void, drag = DRAG, s = START) => {
      const h = harness()
      started(h, s)
      h.src.end(s.view, effect)
      setup?.(h)
      h.src.done({ drag, outcome })
      return h.switches
    }
    expect(run('canceled', 'none', h => h.setModel({ tabs: ['A', 'B', 'C'], active: 'C' }))).toEqual([])
    expect(run('ignored', 'none', undefined, DRAG, { ...START, initial: 'B' })).toEqual([])
    expect(run('rejected', 'none')).toEqual([])
    expect(run('ignored', 'move')).toEqual([])
    expect(run('canceled', 'move')).toEqual([])
    expect(run('canceled', 'none', undefined, 'drag-0')).toEqual([])
  })

  it('되돌리기 오류 → 잡아 로그만 · 모델 활성으로 물러남', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const h = harness({ switchTabIf: () => Promise.reject(new Error('TabNotFound')) })
    started(h)
    h.src.end('B', 'none')
    h.src.done({ drag: DRAG, outcome: 'canceled' })
    expect(h.shown()).toBe('A')
    await tick()
    expect(warn).toHaveBeenCalled()
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    expect(h.shown()).toBe('B')
    vi.restoreAllMocks()
  })

  it('탭 알림은 적용마다 계기다 — 내용이 같아도(이름 바꿈 따위) dragend 뒤 활성이 끄는 탭이 아니면 푼다(②)', () => {
    const h = harness()
    // 누름의 전환 메아리가 아직 안 왔다 — 모델 활성 = A.
    h.setModel({ tabs: ['A', 'B', 'C'], active: 'A' })
    started(h)
    h.src.end('B', 'move')
    expect(h.src.getSnapshot().override.phase).toBe('ended')
    h.setModel({ tabs: ['A', 'B', 'C'], active: 'A' })
    expect(h.src.getSnapshot().override.phase).toBe('idle')
  })

  it('나간 끌기 표시는 그 끌기가 풀릴 때만 걷는다 — 다음 dragstart 가 덮은 표시는 앞 끌기가 걷지 않는다', () => {
    const h = harness()
    started(h)
    h.src.end('B', 'move')
    started(h, { ...START, drag: 'drag-2', view: 'C' })
    h.src.done({ drag: DRAG, outcome: 'applied' })
    expect(h.local.getOutgoing()?.drag).toBe('drag-2')
  })
})

describe('dragSource — 되돌리기가 닿기 전 누름(forceSwitch)', () => {
  /** 취소된 끌기 B 의 되돌리기(A, expect B)를 낸 뒤 — 모델 활성은 아직 B. */
  function reverted(over: Partial<Pick<DragSourcePorts, 'switchTabIf'>> = {}): Harness {
    const h = harness(over)
    started(h)
    h.src.end('B', 'none')
    h.src.done({ drag: DRAG, outcome: 'canceled' })
    expect(h.switches).toEqual([[WIN, 'A', 'B']])
    return h
  }

  it('끄는 탭(expect) 누름만 · 그 창만 · 한 번만 true — 바꿔 그리기를 푼 다른 pointerdown · 활성이 그대로인 탭 알림 뒤에도', () => {
    const h = reverted()
    expect(h.src.forceSwitch(WIN, 'A')).toBe(false)
    expect(h.src.forceSwitch('slot-popup-1', 'B')).toBe(false)
    // 캔버스 누름이 바꿔 그리기를 풀었다 — 되돌리기는 아직 날아간다.
    h.src.pointerDown()
    expect(h.src.getSnapshot().override.phase).toBe('idle')
    h.setModel({ tabs: ['A', 'B', 'C'], active: 'B' })
    expect(h.src.forceSwitch(WIN, 'B')).toBe(true)
    expect(h.src.forceSwitch(WIN, 'B')).toBe(false)
  })

  it('모델 활성이 expect 를 떠나면(되돌리기 닿음 · 남이 바꿈) · 되돌리기 오류면 · 되돌리기가 없으면 false', async () => {
    const landed = reverted()
    landed.setModel({ tabs: ['A', 'B', 'C'], active: 'A' })
    expect(landed.src.forceSwitch(WIN, 'B')).toBe(false)

    const moved = reverted()
    moved.setModel({ tabs: ['A', 'B', 'C'], active: 'C' })
    expect(moved.src.forceSwitch(WIN, 'B')).toBe(false)

    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const failed = reverted({ switchTabIf: () => Promise.reject(new Error('TabNotFound')) })
    await tick()
    expect(failed.src.forceSwitch(WIN, 'B')).toBe(false)
    vi.restoreAllMocks()

    const none = harness()
    started(none)
    none.src.end('B', 'move')
    none.src.done({ drag: DRAG, outcome: 'canceled' })
    expect(none.switches).toEqual([])
    expect(none.src.forceSwitch(WIN, 'B')).toBe(false)
  })

  it('다음 되돌리기가 앞 것을 갈음한다 — 앞 되돌리기의 오류는 새 것을 거두지 않는다', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    let fail: (err: Error) => void = () => {}
    let calls = 0
    const h = reverted({
      switchTabIf: () => (++calls === 1 ? new Promise<boolean>((_, reject) => (fail = reject)) : new Promise<boolean>(() => {})),
    })
    started(h, { ...START, drag: 'drag-2' })
    h.src.end('B', 'none')
    h.src.done({ drag: 'drag-2', outcome: 'canceled' })
    expect(h.switches).toEqual([
      [WIN, 'A', 'B'],
      [WIN, 'A', 'B'],
    ])
    fail(new Error('TabNotFound'))
    await tick()
    expect(h.src.forceSwitch(WIN, 'B')).toBe(true)
    vi.restoreAllMocks()
  })
})

// ── 순서 시험 — 출처 창(§8-3) ──────────────────────────────────────────────────

interface Arrival {
  label: string
  /** 닿는 것 — 모델 알림(그 창 스토어 적용) · `done` · `layout:updated`. */
  run(h: Harness): void
  /** dragend 뒤에만 닿는다(그 dragend 가 낳은 것 · dragend 뒤에 바뀐 것). */
  after?: boolean
}

interface Scenario {
  name: string
  effect: EndEffect
  /** dragend 전에 출처 탭바 구역을 떠났다. */
  out?: boolean
  /** dragend 전 구역 안에서 자리 2 로 옮겼다. */
  move?: boolean
  /** 같은 웹뷰 제자리 놓기 — 뿌리 drop 이 표시를 남긴다. */
  local?: boolean
  /** 자기 `DragEnd` invoke — 답과 그 뒤의 `done`(없으면 알림 없음 · `null` = invoke 오류). */
  dragEnd: { reply: TabDropReply; done: DropOutcome | null } | null
  arrivals: Arrival[]
  /** 끝에 풀려 있어야 한다. */
  releases: boolean
  reverts: number
  /** 끄는 탭이 그 창을 떠난다 — 바꿔 그린 뒤로 한 프레임도 끄는 탭을 그리지 않는다. */
  leaves?: boolean
  /** 같은 창 순서 바꿈 — 순서 이동은 모델 순서가 바뀐 그 프레임까지 그리고 그 프레임에 거둔다. */
  reorder?: boolean
}

const tabsOf = (label: string, m: WindowModel, after = false): Arrival => ({ label, run: h => h.setModel(m), after })
const doneOf = (label: string, outcome: DropOutcome, after = false): Arrival => ({
  label,
  run: h => h.src.done({ drag: DRAG, outcome }),
  after,
})
// `layout:updated` 는 뷰 레이아웃 캐시만 바꾼다 — 그 창 탭 상태(`windows[label]`)를 건드리지 않아 출처에 닿는 것이 없다.
const layoutNoise: Arrival = { label: 'layout:updated', run: () => {} }

const SCENARIOS: Scenario[] = [
  {
    name: '다른 창 탭바 · 칸에 놓음(그 창의 Drop 적용) — 끄는 탭이 떠난다',
    effect: 'move',
    out: true,
    dragEnd: { reply: { type: 'ignored', why: 'accepted-ours' }, done: 'ignored' },
    arrivals: [tabsOf('tabs:drop', { tabs: ['A', 'C'], active: 'A' }), doneOf('done:drop', 'applied'), layoutNoise],
    releases: true,
    reverts: 0,
    leaves: true,
  },
  {
    name: '창 밖 새 창(DragEnd 적용) — 끄는 탭이 떠난다',
    effect: 'none',
    out: true,
    dragEnd: { reply: { type: 'applied', window: 'slot-popup-9', reused: false }, done: 'applied' },
    arrivals: [tabsOf('tabs:new-window', { tabs: ['A', 'C'], active: 'A' }, true), layoutNoise],
    releases: true,
    reverts: 0,
    leaves: true,
  },
  {
    name: 'Esc 취소 — 원래 선택으로 되돌린다',
    effect: 'none',
    out: true,
    dragEnd: { reply: { type: 'canceled' }, done: 'canceled' },
    arrivals: [layoutNoise],
    releases: true,
    reverts: 1,
    leaves: true,
  },
  {
    name: '같은 창 순서 바꿈(이 창의 Drop 적용)',
    effect: 'move',
    move: true,
    dragEnd: { reply: { type: 'ignored', why: 'accepted-ours' }, done: 'ignored' },
    arrivals: [tabsOf('tabs:reorder', { tabs: ['A', 'C', 'B'], active: 'B' }), doneOf('done:drop', 'applied'), layoutNoise],
    releases: true,
    reverts: 0,
    reorder: true,
  },
  {
    name: '자기 DragEnd invoke 오류',
    effect: 'none',
    out: true,
    dragEnd: null,
    arrivals: [layoutNoise],
    releases: true,
    reverts: 0,
  },
  {
    name: '받은 창의 Drop 을 잃음 · dragend 뒤 LLM 이 다른 탭으로',
    effect: 'move',
    out: true,
    dragEnd: { reply: { type: 'ignored', why: 'accepted-ours' }, done: 'ignored' },
    arrivals: [tabsOf('tabs:llm', { tabs: ['A', 'B', 'C'], active: 'C' }, true)],
    releases: true,
    reverts: 0,
  },
  {
    name: '같은 웹뷰 제자리 놓기 — 아무것도 보내지 않는다',
    effect: 'move',
    move: true,
    local: true,
    dragEnd: { reply: { type: 'duplicate' }, done: null },
    arrivals: [layoutNoise],
    releases: true,
    reverts: 0,
  },
]

interface Frame {
  label: string
  snap: DragSourceSnapshot
  model: WindowModel
}

async function play(sc: Scenario, s: fc.Scheduler) {
  const frames: Frame[] = []
  let sentWhileLive = false
  let h: Harness | null = null
  const frame = (label: string) => frames.push({ label, snap: h!.src.getSnapshot(), model: h!.model() })
  const deliver = (a: Arrival) => {
    a.run(h!)
    frame(a.label)
  }
  h = harness({
    send: drop => {
      if (h!.src.getSnapshot().override.phase === 'live') sentWhileLive = true
      const de = sc.dragEnd
      if (de === null) {
        return s.schedule(Promise.resolve(), 'reply:error').then(() => {
          throw new Error('ipc down')
        })
      }
      const outcome = de.done
      if (outcome !== null) {
        void s.schedule(Promise.resolve(), 'done:drag-end').then(() => {
          h!.src.done({ drag: drop.drag, outcome })
          frame('done:drag-end')
        })
      }
      return s.schedule(Promise.resolve(de.reply), 'reply')
    },
    // 되돌리기의 왕복 — 셸에 닿은 때 그 창 활성이 아직 `expect` 면 바꾸고 알림이 뒤따른다(그사이 바뀌었으면 아무것도 안 함).
    switchTabIf: (_w, to, expect) =>
      s.schedule(Promise.resolve(), 'switch_tab_if').then(() => {
        const m = h!.model()
        if (m.active !== expect || !m.tabs.includes(to)) return false
        deliver(tabsOf('tabs:revert', { tabs: m.tabs, active: to }))
        return true
      }),
  })
  started(h)
  if (sc.move) h.local.setSourceStrip({ index: 2, left: 330, x: 420 })
  if (sc.out) h.local.setSourceStrip(null)
  frame('start')

  const scheduleAll = (after: boolean) => {
    for (const a of sc.arrivals.filter(x => (x.after ?? false) === after)) {
      void s.schedule(Promise.resolve(), a.label).then(() => deliver(a))
    }
  }
  scheduleAll(false)
  void s.schedule(Promise.resolve(), 'dragend').then(() => {
    if (sc.local) h!.local.markLocalDrop(DRAG)
    h!.src.end('B', sc.effect)
    frame('dragend')
    scheduleAll(true)
  })
  await s.waitIdle()
  // invoke 응답의 처리(오류면 풀기)는 응답을 넘겨받은 뒤라 사건 프레임에 안 잡힌다 — 끝 상태를 한 번 더 적는다.
  frame('idle')
  return { h, frames, sentWhileLive }
}

const shownOf = (f: Frame) => (f.snap.window === WIN ? shownTab(f.snap.override, f.model) : f.model.active)
const drawnOf = (f: Frame) => f.snap.window === WIN && stripHeld(f.snap.override, f.model) && f.snap.motion !== null

describe('dragSource — 순서 시험(fc.scheduler · 출처 창)', () => {
  for (const sc of SCENARIOS) {
    it(sc.name, async () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      try {
        await fc.assert(
          fc.asyncProperty(fc.scheduler(), async s => {
            const { h, frames, sentWhileLive } = await play(sc, s)
            const where = () => s.report().map(r => r.label).join(' → ')

            // 보내기 — 제자리 놓기 밖은 dragend 하나에 정확히 하나 · 늘 리듀서에 끝을 넣은 뒤.
            expect(h.sends.length, where()).toBe(sc.local ? 0 : 1)
            expect(sentWhileLive, where()).toBe(false)

            // ① 풀려야 하는 갈래는 끝에 풀렸다 · ③ 풀린 뒤 그리는 탭 = 모델 활성.
            const last = frames[frames.length - 1]
            if (sc.releases) expect(last.snap.override.phase, where()).toBe('idle')
            expect(shownOf(last), where()).toBe(last.model.active)
            if (sc.releases) expect(h.local.getOutgoing(), where()).toBeNull()

            // ② 끄는 탭이 그 창을 떠나는 갈래 — 바꿔 그린 뒤로는 한 프레임도 끄는 탭을 그리지 않는다(시한 뒤집기 없음).
            if (sc.leaves) for (const f of frames) expect(shownOf(f), `${f.label} @ ${where()}`).not.toBe('B')

            // ④ 같은 창 순서 바꿈 — 순서 이동은 모델 순서가 바뀐 그 프레임까지 그리고 그 프레임에 거둔다(튐 없음). 이 갈래에서는
            //   그 전에 풀 계기가 없다 — 먼저 거두면 머리가 옛 순서로 돌아갔다가 모델 순서로 다시 튄다.
            if (sc.reorder) {
              for (const f of frames) {
                const same = f.model.tabs.join() === AT_START.tabs.join()
                expect(drawnOf(f), `${f.label} @ ${where()}`).toBe(same)
              }
            }

            // ⑤ 되돌리기는 많아야 한 번 · 갈래마다 정확한 수.
            expect(h.switches.length, where()).toBe(sc.reverts)
          }),
          { numRuns: 150 },
        )
      } finally {
        warn.mockRestore()
      }
    })
  }

  it('⑥ dragend 뒤 끄는 탭을 활성으로 두지 않는 tabs-updated(LLM) 가 오면 그 뒤 그리는 탭 = 모델 활성', async () => {
    const sc = SCENARIOS.find(x => x.name.includes('LLM'))!
    await fc.assert(
      fc.asyncProperty(fc.scheduler(), async s => {
        const { frames } = await play(sc, s)
        const at = frames.findIndex(f => f.label === 'tabs:llm')
        expect(at).toBeGreaterThan(0)
        for (const f of frames.slice(at)) expect(shownOf(f)).toBe(f.model.active)
      }),
      { numRuns: 100 },
    )
  })

  it('⑤ 되돌리기가 셸에 닿기 전에 다른 탭(C)이 골라지면 C 가 남는다 — 끝 화면 = 모델 활성', async () => {
    const sc: Scenario = {
      name: 'pick C during revert',
      effect: 'none',
      out: true,
      dragEnd: { reply: { type: 'canceled' }, done: 'canceled' },
      arrivals: [tabsOf('tabs:pick-c', { tabs: ['A', 'B', 'C'], active: 'C' }, true)],
      releases: true,
      reverts: 0,
    }
    let raced = 0
    await fc.assert(
      fc.asyncProperty(fc.scheduler(), async s => {
        const { h, frames } = await play(sc, s)
        const last = frames[frames.length - 1]
        expect(last.model.active).toBe('C')
        expect(last.snap.override.phase).toBe('idle')
        expect(h.switches.length).toBeLessThanOrEqual(1)
        if (h.switches.length === 1 && !frames.some(f => f.label === 'tabs:revert')) raced++
      }),
      { numRuns: 200 },
    )
    expect(raced).toBeGreaterThan(0)
  })
})
