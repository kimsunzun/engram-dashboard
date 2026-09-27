// followCore 표 시험 — 이 파일이 `docs/process/S21-chat-ux/trd.md` §2-2 규칙 1–8 의 시험 표다(ADR-0242).
//   기하는 한 벌로 고정한다: 내용 1000 · 창 200 → 바닥 = 800. 문턱 = 48.

import { describe, expect, it } from 'vitest'

import {
  FOLLOW_THRESHOLD_PX,
  initialFollowState,
  isScrollable,
  step,
  type FollowInput,
  type FollowState,
  type Metrics,
} from './followCore'

const HEIGHT = 1000
const CLIENT = 200
const BOTTOM = HEIGHT - CLIENT

const at = (top: number, height = HEIGHT, client = CLIENT): Metrics => ({ top, height, client })
const hidden: Metrics = { top: 0, height: 0, client: 0 }

const state = (over: Partial<FollowState> = {}): FollowState => ({ ...initialFollowState(), ...over })

const last = <T,>(xs: T[]): T => xs[xs.length - 1]

/** 입력을 차례로 먹이고 마지막 결과와 중간의 쓰기들을 돌려준다. */
function run(s: FollowState, ...inputs: FollowInput[]): { state: FollowState; writes: (number | null)[] } {
  const writes: (number | null)[] = []
  let cur = s
  for (const i of inputs) {
    const out = step(cur, i)
    cur = out.state
    writes.push(out.scrollTo)
  }
  return { state: cur, writes }
}

describe('규칙 8 — 처음 상태', () => {
  it('붙어 있고 · 얼지 않고 · lastTop 0 · 예고·저장 없음 · unseenGrowth 거짓', () => {
    expect(initialFollowState()).toEqual({
      pinned: true,
      frozen: false,
      lastTop: 0,
      expectTop: null,
      savedTop: null,
      unseenGrowth: false,
    })
  })
})

describe('규칙 1 — 얼음', () => {
  it('client 0 이 오면 언다 — savedTop = expectTop(있으면)', () => {
    const out = step(state({ lastTop: 300, expectTop: 500 }), { type: 'resize', m: hidden })
    expect(out.state.frozen).toBe(true)
    expect(out.state.savedTop).toBe(500)
    expect(out.scrollTo).toBeNull()
  })

  it('예고가 없으면 savedTop = lastTop (그 순간의 m.top 은 쓰지 않는다)', () => {
    const out = step(state({ lastTop: 300 }), { type: 'scroll', m: { ...hidden, top: 7 } })
    expect(out.state.frozen).toBe(true)
    expect(out.state.savedTop).toBe(300)
  })

  it.each<FollowInput['type']>(['attach', 'scroll', 'resize', 'intentUp'])(
    '얼면 어느 잰 입력이든 언다(%s)',
    type => {
      const out = step(state({ lastTop: 300 }), { type, m: hidden } as FollowInput)
      expect(out.state.frozen).toBe(true)
    },
  )

  it('얼어 있는 동안 scroll · client 0 resize/attach · intentUp 은 아무 칸도 안 바꾼다', () => {
    const frozen = state({ frozen: true, savedTop: 300, lastTop: 300, pinned: false })
    for (const i of [
      { type: 'scroll', m: at(600) },
      { type: 'resize', m: hidden },
      { type: 'attach', m: hidden },
      { type: 'intentUp', m: at(600) },
    ] as FollowInput[]) {
      const out = step(frozen, i)
      expect(out.state, i.type).toEqual(frozen)
      expect(out.scrollTo, i.type).toBeNull()
    }
  })

  it('얼어 있어도 pin · unpin 은 붙음만 바꾸고 쓰지 않는다', () => {
    const frozen = state({ frozen: true, savedTop: 300, lastTop: 300, pinned: false })
    const pinned = step(frozen, { type: 'pin' })
    expect(pinned.scrollTo).toBeNull()
    expect(pinned.state).toEqual({ ...frozen, pinned: true })
    const unpinned = step(pinned.state, { type: 'unpin' })
    expect(unpinned.state).toEqual({ ...frozen, pinned: false })
  })
})

describe('규칙 2 — 풀림', () => {
  it('붙은 채 풀리면 바닥으로 쓴다 · lastTop = 쓰기가 닿을 자리 · savedTop = null', () => {
    const out = step(state({ frozen: true, savedTop: 300 }), { type: 'resize', m: at(0) })
    expect(out.scrollTo).toBe(BOTTOM)
    expect(out.state).toMatchObject({ frozen: false, lastTop: BOTTOM, savedTop: null, expectTop: BOTTOM })
  })

  it('떨어진 채 풀리면 savedTop 으로 되살린다', () => {
    const out = step(state({ frozen: true, pinned: false, savedTop: 300 }), { type: 'attach', m: at(0) })
    expect(out.scrollTo).toBe(300)
    expect(out.state).toMatchObject({ frozen: false, savedTop: null, expectTop: 300 })
  })

  it('떨어진 채 풀리는데 이미 그 자리면 쓰지 않는다(display:none 을 지나 보존된 경우)', () => {
    const out = step(state({ frozen: true, pinned: false, savedTop: 300 }), { type: 'resize', m: at(300) })
    expect(out.scrollTo).toBeNull()
    expect(out.state).toMatchObject({ frozen: false, savedTop: null, expectTop: null, lastTop: 300 })
  })

  // ★손으로 만든 상태★(TRD §12 low 2): 규칙 1 이 얼 때마다 savedTop 을 채우므로 step 만으로는 이 갈래에
  //   닿지 않는다.
  it('떨어진 채 풀리는데 savedTop 이 비었으면 쓰지 않는다(손으로 만든 상태)', () => {
    const out = step(state({ frozen: true, pinned: false, savedTop: null }), { type: 'resize', m: at(0) })
    expect(out.scrollTo).toBeNull()
    expect(out.state).toMatchObject({ frozen: false, savedTop: null })
  })

  it('풀림은 resize · attach 만 — client > 0 인 scroll · intentUp 은 여전히 얼어 있다', () => {
    const frozen = state({ frozen: true, savedTop: 300 })
    expect(step(frozen, { type: 'scroll', m: at(0) }).state.frozen).toBe(true)
    expect(step(frozen, { type: 'intentUp', m: at(0) }).state.frozen).toBe(true)
  })

  it('숨기 전의 예고는 풀릴 때 버린다 — 쓰기가 없으면 예고도 없다', () => {
    const { state: s } = run(
      state({ lastTop: 700, expectTop: 800 }),
      { type: 'resize', m: hidden },
      { type: 'resize', m: at(BOTTOM) },
    )
    expect(s.expectTop).toBeNull()
    expect(s.frozen).toBe(false)
  })
})

describe('규칙 3 — 우리 쓰기는 절대 풀지 않는다', () => {
  it('붙은 채 성장 → 바닥 쓰기 → 그 scroll 이벤트는 lastTop 만 옮긴다', () => {
    const grown = step(state({ lastTop: 600 }), { type: 'resize', m: at(600) })
    expect(grown.scrollTo).toBe(BOTTOM)
    const out = step(grown.state, { type: 'scroll', m: at(BOTTOM) })
    expect(out.state).toMatchObject({ pinned: true, lastTop: BOTTOM, expectTop: null })
  })

  it('1px 안이면 우리 것이다', () => {
    const out = step(state({ lastTop: 600, expectTop: 800 }), { type: 'scroll', m: at(799.2) })
    expect(out.state).toMatchObject({ lastTop: 799.2, expectTop: null })
  })

  it('예고한 위치로의 위 움직임도 풀지 않는다(손으로 만든 상태 — 붙음 판정을 건너뛴다)', () => {
    const out = step(state({ lastTop: 800, expectTop: 100 }), { type: 'scroll', m: at(100) })
    expect(out.state.pinned).toBe(true)
  })

  it('떨어진 채 되살린 쓰기(아래로 · 문턱 안)는 되붙이지 않는다', () => {
    const { state: s } = run(
      state({ frozen: true, pinned: false, savedTop: BOTTOM - 10 }),
      { type: 'resize', m: at(0) },
      { type: 'scroll', m: at(BOTTOM - 10) },
    )
    expect(s.pinned).toBe(false)
  })
})

describe('규칙 4 — 사용자 스크롤', () => {
  it('붙은 채 위로 문턱 밖 → 푼다', () => {
    const out = step(state({ lastTop: BOTTOM }), { type: 'scroll', m: at(BOTTOM - FOLLOW_THRESHOLD_PX - 1) })
    expect(out.state.pinned).toBe(false)
    expect(out.state.lastTop).toBe(BOTTOM - FOLLOW_THRESHOLD_PX - 1)
  })

  it('붙은 채 문턱 안 위 움직임(내용 줄어듦의 클램프)은 안 푼다', () => {
    const out = step(state({ lastTop: BOTTOM }), { type: 'scroll', m: at(BOTTOM - 20) })
    expect(out.state.pinned).toBe(true)
    expect(out.state.lastTop).toBe(BOTTOM - 20)
  })

  it('붙은 채 아래로 움직이면(바닥 거리가 문턱 밖이어도) 안 푼다', () => {
    const out = step(state({ lastTop: 100 }), { type: 'scroll', m: at(300) })
    expect(out.state.pinned).toBe(true)
  })

  it('떨어진 채 아래로 문턱 안에 들면 되붙고 unseenGrowth 를 내린다', () => {
    const out = step(state({ pinned: false, lastTop: 500, unseenGrowth: true }), {
      type: 'scroll',
      m: at(BOTTOM - 10),
    })
    expect(out.state).toMatchObject({ pinned: true, unseenGrowth: false })
  })

  it('떨어진 채 아래로 가도 문턱 밖이면 떨어져 있다', () => {
    const out = step(state({ pinned: false, lastTop: 100 }), { type: 'scroll', m: at(500) })
    expect(out.state.pinned).toBe(false)
  })

  it('떨어진 채 제자리 · 위 scroll 은 문턱 안이어도 되붙이지 않는다(걸쇠)', () => {
    const still = step(state({ pinned: false, lastTop: BOTTOM - 10 }), { type: 'scroll', m: at(BOTTOM - 10) })
    expect(still.state.pinned).toBe(false)
    const up = step(state({ pinned: false, lastTop: BOTTOM }), { type: 'scroll', m: at(BOTTOM - 5) })
    expect(up.state.pinned).toBe(false)
  })

  it('예고와 맞지 않는 scroll 은 예고를 버린다(사용자가 우리 쓰기 뒤에 움직였다)', () => {
    const out = step(state({ lastTop: 600, expectTop: 800 }), { type: 'scroll', m: at(550) })
    expect(out.state.expectTop).toBeNull()
    expect(out.state.lastTop).toBe(550)
  })
})

describe('규칙 5 — 위로 가는 입력', () => {
  it('스크롤할 것이 있으면 문턱 안의 작은 휠도 곧바로 푼다', () => {
    const out = step(state({ lastTop: BOTTOM }), { type: 'intentUp', m: at(BOTTOM) })
    expect(out.state.pinned).toBe(false)
    expect(out.scrollTo).toBeNull()
  })

  it('스크롤할 것이 없으면 안 푼다', () => {
    const out = step(state(), { type: 'intentUp', m: at(0, 150, CLIENT) })
    expect(out.state.pinned).toBe(true)
  })
})

describe('규칙 6 — 성장은 붙음을 다시 재지 않는다', () => {
  it('붙어 있으면 바닥으로 쓴다 — 지금 위치가 바닥에서 멀어도 풀리지 않는다', () => {
    const out = step(state({ lastTop: 100 }), { type: 'resize', m: at(100) })
    expect(out.state.pinned).toBe(true)
    expect(out.scrollTo).toBe(BOTTOM)
  })

  it('내용이 창보다 작으면 바닥 = 0', () => {
    const out = step(state({ lastTop: 50 }), { type: 'resize', m: at(50, 150, CLIENT) })
    expect(out.scrollTo).toBe(0)
  })

  it('떨어져 있으면 쓰지 않고 unseenGrowth 만 세운다', () => {
    const out = step(state({ pinned: false, lastTop: 100 }), { type: 'resize', m: at(100) })
    expect(out.scrollTo).toBeNull()
    expect(out.state).toMatchObject({ pinned: false, unseenGrowth: true, lastTop: 100 })
  })
})

describe('규칙 7 — 쓰기 예고', () => {
  it('이미 바닥인 쓰기는 예고도 쓰기도 없다', () => {
    const out = step(state({ lastTop: BOTTOM }), { type: 'resize', m: at(BOTTOM) })
    expect(out.scrollTo).toBeNull()
    expect(out.state.expectTop).toBeNull()
  })

  it('0.5px 안은 이미 그 자리다 · 그보다 멀면 예고를 남긴다', () => {
    expect(step(state(), { type: 'resize', m: at(BOTTOM - 0.4) }).state.expectTop).toBeNull()
    const far = step(state(), { type: 'resize', m: at(BOTTOM - 0.6) })
    expect(far.scrollTo).toBe(BOTTOM)
    expect(far.state.expectTop).toBe(BOTTOM)
  })

  it('쓰면 lastTop 을 쓰기가 닿을 자리로 옮긴다', () => {
    const out = step(state({ lastTop: 600 }), { type: 'resize', m: at(600) })
    expect(out.state.lastTop).toBe(BOTTOM)
  })

  // 성장 쓰기(800 → 1000)의 scroll 이벤트가 오기 전에 스크롤바를 900 으로 끌었다 — 이벤트는 합쳐져 900 하나만 온다.
  it('쓰기의 이벤트가 오기 전 사용자가 위로 끌면 푼다 — 다음 성장이 끌어내리지 않는다', () => {
    const grow = step(state({ lastTop: BOTTOM }), { type: 'resize', m: at(BOTTOM, HEIGHT + 200) })
    expect(grow.scrollTo).toBe(BOTTOM + 200)
    const dragged = step(grow.state, { type: 'scroll', m: at(BOTTOM + 100, HEIGHT + 200) })
    expect(dragged.state).toMatchObject({ pinned: false, lastTop: BOTTOM + 100, expectTop: null })
    expect(step(dragged.state, { type: 'resize', m: at(BOTTOM + 100, HEIGHT + 400) }).scrollTo).toBeNull()
  })
})

describe('isScrollable — 버튼 조건', () => {
  it('내용이 창보다 0.5px 넘게 크면 참', () => {
    expect(isScrollable(at(0))).toBe(true)
    expect(isScrollable(at(0, CLIENT, CLIENT))).toBe(false)
    expect(isScrollable(at(0, CLIENT + 0.5, CLIENT))).toBe(false)
    expect(isScrollable(at(0, CLIENT + 1, CLIENT))).toBe(true)
  })
})

describe('attach — 새 노드', () => {
  it('붙어 있으면 바닥으로 쓰고, 옛 노드의 예고는 버린다', () => {
    const out = step(state({ lastTop: 400, expectTop: 400 }), { type: 'attach', m: at(0) })
    expect(out.scrollTo).toBe(BOTTOM)
    expect(out.state).toMatchObject({ lastTop: BOTTOM, expectTop: BOTTOM })
  })

  it('떨어져 있으면 쓰지 않는다', () => {
    const out = step(state({ pinned: false, expectTop: 400 }), { type: 'attach', m: at(0) })
    expect(out.scrollTo).toBeNull()
    expect(out.state).toMatchObject({ pinned: false, lastTop: 0, expectTop: null })
  })
})

describe('pin · unpin', () => {
  it('pin 은 붙이고 unseenGrowth 를 내린다 · 잰 값이 없어 쓰지 않는다', () => {
    const out = step(state({ pinned: false, unseenGrowth: true }), { type: 'pin' })
    expect(out.state).toMatchObject({ pinned: true, unseenGrowth: false })
    expect(out.scrollTo).toBeNull()
  })

  it('unpin 은 붙음만 내린다', () => {
    const out = step(state({ lastTop: 300 }), { type: 'unpin' })
    expect(out.state).toEqual(state({ lastTop: 300, pinned: false }))
    expect(out.scrollTo).toBeNull()
  })
})

describe('조합', () => {
  // TRD §2-7 ★걸쇠★ — 문턱 안 작은 위 휠 → 그 휠의 위 scroll(문턱 안) → 여전히 떨어짐 → 아래로 문턱 안 → 붙음.
  it('걸쇠: 작은 위 휠이 푼 뒤 그 휠의 scroll 은 되붙이지 않고, 아래로 돌아오면 되붙는다', () => {
    const start = state({ lastTop: BOTTOM })
    const wheel = run(start, { type: 'intentUp', m: at(BOTTOM) }, { type: 'scroll', m: at(BOTTOM - 10) })
    expect(wheel.state.pinned).toBe(false)
    const back = step(wheel.state, { type: 'scroll', m: at(BOTTOM) })
    expect(back.state.pinned).toBe(true)
  })

  it('작은 위 휠 뒤 성장은 끌어내리지 않는다', () => {
    const { state: s, writes } = run(
      state({ lastTop: BOTTOM }),
      { type: 'intentUp', m: at(BOTTOM) },
      { type: 'scroll', m: at(BOTTOM - 10) },
      { type: 'resize', m: at(BOTTOM - 10, HEIGHT + 300) },
    )
    expect(s.pinned).toBe(false)
    expect(last(writes)).toBeNull()
  })

  it('명시 pin 은 걸쇠를 푼다 — 뒤이은 성장이 바닥으로 쓴다', () => {
    const { state: s, writes } = run(
      state({ pinned: false, lastTop: BOTTOM - 10 }),
      { type: 'pin' },
      { type: 'resize', m: at(BOTTOM - 10, HEIGHT + 300) },
    )
    expect(s.pinned).toBe(true)
    expect(last(writes)).toBe(BOTTOM + 300)
  })

  it('스트리밍 성장 내내 붙어 있다 — 쓰기와 그 이벤트가 번갈아 와도', () => {
    let s = state({ lastTop: BOTTOM })
    for (let grow = 1; grow <= 5; grow++) {
      const h = HEIGHT + grow * 40
      const out = step(s, { type: 'resize', m: at(h - 40 - CLIENT, h) })
      expect(out.scrollTo).toBe(h - CLIENT)
      s = step(out.state, { type: 'scroll', m: at(h - CLIENT, h) }).state
    }
    expect(s).toMatchObject({ pinned: true, expectTop: null })
  })

  it('떨어진 채 숨었다 돌아오면 읽던 자리로, 붙은 채 숨었다 돌아오면 바닥으로', () => {
    const free = run(
      state({ pinned: false, lastTop: 300 }),
      { type: 'resize', m: hidden },
      { type: 'resize', m: at(0, HEIGHT + 500) },
    )
    expect(last(free.writes)).toBe(300)
    const pinned = run(
      state({ lastTop: BOTTOM }),
      { type: 'resize', m: hidden },
      { type: 'resize', m: at(0, HEIGHT + 500) },
    )
    expect(last(pinned.writes)).toBe(BOTTOM + 500)
  })

  it('숨은 동안 pin → 쓰기 없음 → 보이면 바닥', () => {
    const { writes, state: s } = run(
      state({ pinned: false, lastTop: 300 }),
      { type: 'resize', m: hidden },
      { type: 'pin' },
      { type: 'resize', m: hidden },
      { type: 'resize', m: at(300) },
    )
    expect(writes).toEqual([null, null, null, BOTTOM])
    expect(s.pinned).toBe(true)
  })
})
