// ADR-0242: DOM wiring for scroll-follow — measures inputs on the viewport node (scroll · wheel · key · size change), feeds the pure core
//   (`followCore.ts`), and writes the position the core returns to `scrollTop`. Both slots (RichSlot · DomSlot) use the same hook without
//   touching it — a slot only plugs `viewportRef` into ScrollArea and calls `pin()` on clear · send.
//
// ★`viewportRef` · `pin` · `unpin` are the same function for the mount lifetime★ — state and node live outside render. So
//   there is no risk of a slot's subscription effect holding an old function when it calls them inside, and ★do not put them in its deps (`[viewId, agentId]`)★
//   (doing so resubscribes when handle identity shifts → replay runs — `docs/reference/architecture.md` 「프론트 규칙」).
// ★Listeners attach to the viewport only — no global listeners★ (ADR-0242 — reason `use-stick-to-bottom` was rejected).
// Observation surface = `data-scroll-follow="pinned" | "free"` on the viewport (the hook writes it directly — not through React state).

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
 * U3 (user decision 2026-09-27): show whenever not at the bottom. `'whenUnseen'` = only when the size changed while detached (rejected alternative —
 * revert with this one line). Either way it does not show when there is nothing to scroll (`scrollable`). `as` is meant to keep the declared type a union —
 * if narrowed, comparing the other branch is a tsc error.
 */
export const JUMP_BUTTON_MODE = 'whenFree' as JumpButtonMode

/** U4 (user decision 2026-09-27): sending re-pins to the bottom — RichSlot reads it. */
export const REPIN_ON_SEND: boolean = true

/** Marker Radix ScrollArea puts on the Viewport (read from `@radix-ui/react-scroll-area` dist). */
const RADIX_VIEWPORT = '[data-radix-scroll-area-viewport]'

const UP_KEYS = new Set(['PageUp', 'Home', 'ArrowUp'])

export interface ScrollFollow extends FollowHandle {
  /** Plug into ScrollArea's `ref` (that seam forwards it to the Radix Viewport — ADR-0053). */
  viewportRef: (el: HTMLDivElement | null) => void
  unseenGrowth: boolean
  /**
   * Content is larger than the viewport — the last validly measured value (left as is while hidden). A button condition unrelated to pinned:
   * even if the viewport grows or the content shrinks to fit after detaching, pinned is not re-measured (ADR-0242 decision 2).
   */
  scrollable: boolean
}

/** Test seam (ADR-0012) — jsdom has no ResizeObserver. If none is injected, use the global one. */
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
 * Is this an upward input absorbed by an inner scroller (ThoughtRow's own ScrollArea). If the inner one is already at the top, the input
 * spills outward, so it is the outer one's upward intent.
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
    // ★Make the viewport take focus on click★ — Radix Viewport has no tabIndex, so clicking conversation text leaves focus on
    //   body, and the up key's keydown does not reach the listener below (an ArrowUp within the threshold gets dragged down by growth).
    //   -1 = focus only by click · code (outside tab order). keydown still bubbles to the slot root. The outline is removed per the codebase
    //   convention (inline `outline: none` on focusable list containers).
    const prevTabIndex = el.getAttribute('tabindex')
    const prevOutline = el.style.outline
    el.tabIndex = -1
    el.style.outline = 'none'

    const onScroll = (): void => apply({ type: 'scroll', m: measure(el) })
    const onWheel = (e: WheelEvent): void => {
      // ctrl + wheel is zoom — do not scroll.
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

    // The viewport itself (the waiting list above the input · window size · tab display) and the content (the first child wrapper Radix puts inside the Viewport).
    //   The RO callback runs after layout · before paint, so writing here does not flicker.
    const ro = RO ? new RO(() => apply({ type: 'resize', m: measure(el) })) : null
    if (ro) {
      ro.observe(el)
      const content = el.firstElementChild
      if (content) {
        ro.observe(content)
      } else {
        // ADR-0242 impact: if Radix changes the wrapper structure it ends up here — symptom = streaming does not stick to the bottom.
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
      // `pin` carries no measurement — the bottom write comes from the same single measurement as a size change (when frozen, rule 1 blocks it).
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
