// ADR-0242: 스크롤 따라가기의 DOM 배선 — 뷰포트 노드의 입력(scroll · 휠 · 키 · 크기 변화)을 재어 순수 코어
//   (`followCore.ts`)에 먹이고, 코어가 낸 위치를 `scrollTop` 에 쓴다. 두 슬롯(RichSlot · DomSlot)이 손대지 않고
//   같은 훅을 쓴다 — 슬롯은 `viewportRef` 를 ScrollArea 에 꽂고 비우기·보냄에서 `pin()` 을 부를 뿐이다.
//
// ★`viewportRef` · `pin` · `unpin` 은 마운트 수명 동안 같은 함수다★ — 상태·노드를 렌더 밖에 둔다. 그래서
//   슬롯의 구독 효과가 안에서 불러도 옛 함수를 쥘 걱정이 없고, ★그 deps(`[viewId, agentId]`)에 넣지 않는다★
//   (넣으면 손잡이 정체성이 흔들릴 때 재구독 → replay 가 돈다 — CLAUDE.md 「통합 micro-rules」).
// ★청취자는 뷰포트에만 건다 — 전역 청취자를 걸지 않는다★(ADR-0242 — `use-stick-to-bottom` 기각 사유).
// 관측 표면 = 뷰포트의 `data-scroll-follow="pinned" | "free"`(훅이 직접 적는다 — React 상태를 거치지 않는다).

import { useEffect, useRef, useState } from 'react'

import {
  initialFollowState,
  isScrollable,
  step,
  type FollowInput,
  type FollowState,
  type Metrics,
} from './followCore'
import { registerFollow, type FollowHandle } from './followRegistry'

export type JumpButtonMode = 'whenFree' | 'whenUnseen'

/**
 * U3(사용자 결정 2026-09-27): 바닥이 아니면 늘. `'whenUnseen'` = 떨어진 동안 크기가 바뀌었을 때만(거부한 대안 —
 * 이 한 줄로 되돌린다). 어느 쪽이든 스크롤할 것이 없으면 뜨지 않는다(`scrollable`). `as` 는 선언 타입을 합으로
 * 두려는 것이다 — 좁혀지면 다른 갈래의 비교가 tsc 오류다.
 */
export const JUMP_BUTTON_MODE = 'whenFree' as JumpButtonMode

/** U4(사용자 결정 2026-09-27): 보내면 다시 바닥에 붙는다 — RichSlot 이 읽는다. */
export const REPIN_ON_SEND: boolean = true

/** Radix ScrollArea 가 Viewport 에 싣는 표지(`@radix-ui/react-scroll-area` dist 판독). */
const RADIX_VIEWPORT = '[data-radix-scroll-area-viewport]'

const UP_KEYS = new Set(['PageUp', 'Home', 'ArrowUp'])

export interface ScrollFollow extends FollowHandle {
  /** ScrollArea 의 `ref` 에 꽂는다(그 seam 이 Radix Viewport 로 forward 한다 — ADR-0053). */
  viewportRef: (el: HTMLDivElement | null) => void
  unseenGrowth: boolean
  /**
   * 내용이 뷰포트보다 크다 — 마지막으로 유효하게 잰 값(숨은 동안은 그대로 둔다). 붙음과 무관한 버튼 조건이다:
   * 떨어진 뒤 뷰포트가 커지거나 내용이 줄어 다 들어와도 붙음은 다시 재지 않는다(ADR-0242 결정 2).
   */
  scrollable: boolean
}

/** 시험 seam(ADR-0012) — jsdom 엔 ResizeObserver 가 없다. 없으면 전역 것을 쓴다. */
export interface ScrollFollowSeams {
  ResizeObserver?: typeof ResizeObserver
}

interface Published {
  pinned: boolean
  unseenGrowth: boolean
  scrollable: boolean
}

const INITIAL_PUBLISHED: Published = { pinned: true, unseenGrowth: false, scrollable: false }

const measure = (el: HTMLElement): Metrics => ({
  top: el.scrollTop,
  height: el.scrollHeight,
  client: el.clientHeight,
})

/**
 * 안쪽 스크롤러(ThoughtRow 의 자기 ScrollArea)가 먹는 위 입력인가. 안쪽이 이미 맨 위면 입력이 바깥으로
 * 번지므로 바깥의 위 의도다.
 */
function innerScrollerTakes(target: EventTarget | null, viewport: HTMLElement): boolean {
  if (!(target instanceof Element)) return false
  const nearest = target.closest(RADIX_VIEWPORT)
  return nearest !== null && nearest !== viewport && nearest.scrollTop > 0
}

function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)
}

function createFollowEngine(
  RO: typeof ResizeObserver | undefined,
  publish: (p: Published) => void,
): { viewportRef: ScrollFollow['viewportRef']; handle: FollowHandle } {
  let state: FollowState = initialFollowState()
  let published: Published = INITIAL_PUBLISHED
  let scrollable = published.scrollable
  let node: HTMLDivElement | null = null
  let detach: (() => void) | null = null

  const apply = (input: FollowInput): void => {
    const out = step(state, input)
    state = out.state
    if ('m' in input && input.m.client > 0) scrollable = isScrollable(input.m)
    if (node) {
      if (out.scrollTo !== null) node.scrollTop = out.scrollTo
      node.dataset.scrollFollow = state.pinned ? 'pinned' : 'free'
    }
    if (
      published.pinned !== state.pinned ||
      published.unseenGrowth !== state.unseenGrowth ||
      published.scrollable !== scrollable
    ) {
      published = { pinned: state.pinned, unseenGrowth: state.unseenGrowth, scrollable }
      publish(published)
    }
  }

  const listen = (el: HTMLDivElement): (() => void) => {
    // ★뷰포트가 클릭으로 포커스를 받게 한다★ — Radix Viewport 엔 tabIndex 가 없어 대화 글을 눌러도 포커스가
    //   body 에 남고, 위 키의 keydown 이 아래 청취자에 오지 않는다(문턱 안의 ArrowUp 이 성장에 끌려 내려온다).
    //   -1 = 클릭 · 코드로만 포커스(탭 순서 밖). keydown 은 그대로 슬롯 루트로 번진다. 테두리는 코드베이스
    //   관례(포커스 받는 목록 컨테이너의 inline `outline: none`)대로 지운다.
    const prevTabIndex = el.getAttribute('tabindex')
    const prevOutline = el.style.outline
    el.tabIndex = -1
    el.style.outline = 'none'

    const onScroll = (): void => apply({ type: 'scroll', m: measure(el) })
    const onWheel = (e: WheelEvent): void => {
      // ctrl + 휠은 확대/축소다 — 스크롤하지 않는다.
      if (e.deltaY >= 0 || e.ctrlKey || innerScrollerTakes(e.target, el)) return
      apply({ type: 'intentUp', m: measure(el) })
    }
    const onKey = (e: KeyboardEvent): void => {
      if (!UP_KEYS.has(e.key) || e.isComposing || isEditable(e.target)) return
      if (innerScrollerTakes(e.target, el)) return
      apply({ type: 'intentUp', m: measure(el) })
    }
    el.addEventListener('scroll', onScroll, { passive: true })
    el.addEventListener('wheel', onWheel, { passive: true })
    el.addEventListener('keydown', onKey)

    // 뷰포트 자신(입력창 위 대기 목록 · 창 크기 · 탭 표시)과 내용(Radix 가 Viewport 안에 두는 첫 자식 래퍼).
    //   RO 콜백은 레이아웃 뒤 · 페인트 전이라 여기서 쓰면 깜빡이지 않는다.
    const ro = RO ? new RO(() => apply({ type: 'resize', m: measure(el) })) : null
    if (ro) {
      ro.observe(el)
      const content = el.firstElementChild
      if (content) {
        ro.observe(content)
      } else {
        // ADR-0242 영향: Radix 가 래퍼 구조를 바꾸면 여기로 온다 — 증상 = 스트리밍이 바닥에 안 붙는다.
        console.warn('[scrollFollow] 뷰포트에 내용 래퍼(첫 자식)가 없다 — 내용 성장을 못 본다')
      }
    }
    return () => {
      el.removeEventListener('scroll', onScroll)
      el.removeEventListener('wheel', onWheel)
      el.removeEventListener('keydown', onKey)
      ro?.disconnect()
      if (prevTabIndex === null) el.removeAttribute('tabindex')
      else el.setAttribute('tabindex', prevTabIndex)
      el.style.outline = prevOutline
    }
  }

  const viewportRef = (el: HTMLDivElement | null): void => {
    if (el === node) return
    detach?.()
    detach = null
    node = el
    if (el) {
      detach = listen(el)
      apply({ type: 'attach', m: measure(el) })
    }
  }

  const handle: FollowHandle = {
    get pinned() {
      return state.pinned
    },
    pin() {
      apply({ type: 'pin' })
      // `pin` 은 잰 값을 싣지 않는다 — 바닥 쓰기는 크기 변화와 같은 한 번 재기가 낸다(얼어 있으면 규칙 1 이 막는다).
      if (node) apply({ type: 'resize', m: measure(node) })
    },
    unpin() {
      apply({ type: 'unpin' })
    },
  }

  return { viewportRef, handle }
}

export function useScrollFollow(slotId: string, seams?: ScrollFollowSeams): ScrollFollow {
  const [published, setPublished] = useState<Published>(INITIAL_PUBLISHED)
  const engineRef = useRef<ReturnType<typeof createFollowEngine> | null>(null)
  if (engineRef.current === null) {
    engineRef.current = createFollowEngine(
      seams?.ResizeObserver ?? (typeof ResizeObserver === 'undefined' ? undefined : ResizeObserver),
      setPublished,
    )
  }
  const { viewportRef, handle } = engineRef.current

  useEffect(() => registerFollow(slotId, handle), [slotId, handle])

  return {
    viewportRef,
    pinned: published.pinned,
    unseenGrowth: published.unseenGrowth,
    scrollable: published.scrollable,
    pin: handle.pin,
    unpin: handle.unpin,
  }
}
