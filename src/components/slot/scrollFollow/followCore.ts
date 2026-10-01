// ADR-0242: 스크롤 따라가기의 판정 — 입력 하나와 지금 상태를 받아 다음 상태와 쓸 위치를 내는 순수 함수(DOM 0).
//   규칙표의 정본 = `docs/process/S21-chat-ux/trd.md` §2-2(번호가 그 표의 번호다 — `followCore.test.ts` 가 그 표다).
//
// ★붙음(`pinned`)은 scroll 이벤트와 위로 가는 입력으로만 바뀐다 — 크기가 바뀌어도(`resize`) 다시 재지 않는다★:
//   재면 스트리밍 성장이 「위로 올렸다」로 읽힌다(ADR-0242 거부한 대안). 명시 `pin`·`unpin` 은 예외다.
// ★붙어 있으면 `unseenGrowth` 는 늘 false 다★ — 붙게 하는 길(규칙 4 되붙기 · `pin`)이 함께 내린다.

/** ADR-0242: 피어 범위 40–64 의 가운데(t3code 40 · paseo 64). 바닥 거리가 이 안이면 「바닥」이다. */
export const FOLLOW_THRESHOLD_PX = 48

/** 우리 쓰기의 scroll 이벤트로 알아보는 폭(규칙 3). */
const OWN_WRITE_TOLERANCE_PX = 1

/**
 * 이 폭 안이면 이미 그 자리로 친다(규칙 7) — 쓰지도 예고하지도 않는다. 브라우저는 제자리 쓰기에 scroll
 * 이벤트를 안 내므로, 예고를 남기면 뒤에 오는 사용자 스크롤을 우리 것으로 오인한다.
 */
const SAME_POSITION_PX = 0.5

/** 뷰포트를 한 번 잰 값(px) — `scrollTop` · `scrollHeight` · `clientHeight`. */
export interface Metrics {
  top: number
  height: number
  client: number
}

export interface FollowState {
  pinned: boolean
  /** 뷰포트가 `display:none` 안이다(`client === 0` — ADR-0056 keep-alive 탭). 이 동안 잰 값은 무효다. */
  frozen: boolean
  lastTop: number
  /** 우리가 쓰고 아직 scroll 이벤트로 못 본 위치. `null` = 기다리는 쓰기 없음. */
  expectTop: number | null
  /** 얼 때 적어 둔 위치 — 떨어진 채 풀리면 되살린다. `null` = 되살릴 것 없음(처음 · 풀린 뒤). */
  savedTop: number | null
  /** 떨어진 동안 크기가 바뀌었다. 버튼 조건 `'whenUnseen'`(U3 대안)만 읽는다. */
  unseenGrowth: boolean
}

export type FollowInput =
  | { type: 'attach'; m: Metrics }
  | { type: 'scroll'; m: Metrics }
  | { type: 'resize'; m: Metrics }
  | { type: 'intentUp'; m: Metrics }
  | { type: 'pin' }
  | { type: 'unpin' }

/** `scrollTo` 가 `null` 이 아니면 호출자가 그 값을 `scrollTop` 에 쓴다. */
export interface FollowStep {
  state: FollowState
  scrollTo: number | null
}

/** 규칙 8. */
export function initialFollowState(): FollowState {
  return {
    pinned: true,
    frozen: false,
    lastTop: 0,
    expectTop: null,
    savedTop: null,
    unseenGrowth: false,
  }
}

const stay = (state: FollowState): FollowStep => ({ state, scrollTo: null })

const bottomOf = (m: Metrics): number => Math.max(0, m.height - m.client)

/**
 * 규칙 7. ★`lastTop` 은 쓰기가 닿을 자리로 옮긴다★ — 쓰기의 scroll 이벤트가 오기 전에 사용자가 위로 끌면
 * 그 이벤트는 우리 쓰기와 합쳐져 한 번만 온다. 옛 자리를 기준으로 두면 끈 자리가 그보다 아래라 「위로」가
 * 안 읽혀 붙은 채 남고, 다음 성장이 도로 끌어내린다. 쓰기 자신의 이벤트는 `expectTop` 이 따로 알아본다.
 */
function writeTo(s: FollowState, m: Metrics, target: number): FollowStep {
  if (Math.abs(m.top - target) <= SAME_POSITION_PX) return stay(s)
  return { state: { ...s, lastTop: target, expectTop: target }, scrollTo: target }
}

/**
 * 스크롤할 것이 있나 — 「맨 아래로」 버튼 조건(훅이 싣는다). 붙음 판정은 이것을 쓰지 않는다(규칙 5 는
 * `height > client` 그대로 — 정수 px 에서 둘은 같다).
 */
export function isScrollable(m: Metrics): boolean {
  return m.height - m.client > SAME_POSITION_PX
}

/**
 * 규칙 2. ★`expectTop` 도 비운다★ — 숨기 전에 쓴 위치의 scroll 이벤트는 얼어 있는 동안 버려졌고, 여기서
 * 위치를 다시 쟀으므로 남은 예고는 뒤의 사용자 스크롤을 우리 것으로 오인하게 할 뿐이다.
 */
function thaw(s: FollowState, m: Metrics): FollowStep {
  const base: FollowState = { ...s, frozen: false, lastTop: m.top, expectTop: null, savedTop: null }
  if (s.pinned) return writeTo(base, m, bottomOf(m))
  if (s.savedTop !== null && m.top !== s.savedTop) return writeTo(base, m, s.savedTop)
  return stay(base)
}

/** 규칙 3 · 4. */
function onScroll(s: FollowState, m: Metrics): FollowStep {
  if (s.expectTop !== null && Math.abs(m.top - s.expectTop) <= OWN_WRITE_TOLERANCE_PX) {
    return stay({ ...s, lastTop: m.top, expectTop: null })
  }
  // ★맞지 않는 scroll 이 오면 예고도 버린다★: 브라우저는 한 프레임의 위치 변화를 이벤트 하나로 합친다 —
  //   우리 쓰기 뒤 사용자가 움직였으면 그 쓰기의 이벤트는 따로 오지 않는다.
  const next: FollowState = { ...s, lastTop: m.top, expectTop: null }
  const distance = m.height - m.top - m.client
  if (s.pinned) {
    // 문턱 안의 위 움직임은 내용이 줄어 생긴 클램프일 수 있다 — 풀지 않는다.
    if (distance > FOLLOW_THRESHOLD_PX && m.top < s.lastTop) next.pinned = false
  } else if (m.top > s.lastTop && distance <= FOLLOW_THRESHOLD_PX) {
    // ★위 의도 걸쇠★: 되붙기는 아래로 움직여 문턱에 든 scroll 에서만이다 — 작은 위 휠이 규칙 5 로 푼 뒤,
    //   그 휠이 만든 위 scroll(문턱 안)이 곧바로 되붙이지 않게 한다.
    next.pinned = true
    next.unseenGrowth = false
  }
  return stay(next)
}

export function step(s: FollowState, i: FollowInput): FollowStep {
  // 규칙 10 — 얼어 있어도 붙음만 바꾼다(규칙 1). 쓰기는 훅이 뒤이어 먹이는 잰 값이나 풀림(규칙 2)이 낸다.
  if (i.type === 'pin') return stay({ ...s, pinned: true, unseenGrowth: false })
  if (i.type === 'unpin') return stay({ ...s, pinned: false })

  const { m } = i
  if (s.frozen) {
    if (m.client > 0 && (i.type === 'resize' || i.type === 'attach')) return thaw(s, m)
    return stay(s)
  }
  if (m.client === 0) {
    // 규칙 1 — 이 순간의 `m.top` 은 이미 무효일 수 있어 마지막으로 유효하게 본 값을 적는다.
    return stay({ ...s, frozen: true, savedTop: s.expectTop ?? s.lastTop })
  }

  switch (i.type) {
    case 'attach': {
      // 규칙 9 — 새 노드다. 옛 노드에 쓴 위치의 이벤트는 여기로 오지 않는다.
      const base: FollowState = { ...s, lastTop: m.top, expectTop: null }
      return s.pinned ? writeTo(base, m, bottomOf(m)) : stay(base)
    }
    case 'scroll':
      return onScroll(s, m)
    case 'resize':
      if (s.pinned) return writeTo(s, m, bottomOf(m))
      return stay(s.unseenGrowth ? s : { ...s, unseenGrowth: true })
    case 'intentUp':
      return stay(s.pinned && m.height > m.client ? { ...s, pinned: false } : s)
  }
}
