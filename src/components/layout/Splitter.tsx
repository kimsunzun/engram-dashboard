// ADR-0227: 분할 경계 하나의 드래그 손잡이. 셸을 모른다 — 포인터를 비율로 바꿔 콜백으로 알릴 뿐이고, 미리보기
//   상태·`set_split_ratio`·화해는 호출자(`splitPreview.ts` 를 쓰는 렌더러)가 진다.
// 레이아웃 공간을 먹지 않는 오버레이다 — 셸의 칸 px 계산이 구분선 두께를 빼지 않는 것과 짝이다(TRD §2d).

import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from 'react'

import type { SplitDir, SplitRect } from '../../api/layoutTypes'
import { ratioFromPointer } from './splitPreview'

const HIT_PX = 8
const HOVER_DELAY_MS = 300

/**
 * 콜백 계약: `onDragStart` 뒤에는 `onRelease`·`onCancel` 중 정확히 하나가 온다(언마운트 포함).
 * 움직임 없이 뗀 클릭은 아무 콜백도 부르지 않는다. 누르는 동안 다른 포인터의 누름은 무시한다.
 */
export interface SplitterProps {
  /** 이 분할의 지금 사각형 — 미리보기 재사상이 입혀졌을 수 있다. */
  split: SplitRect
  /** 허용 비율 범위(`ratioRange`). null = 잠김 — 드래그·더블클릭 모두 무시한다. */
  range: { lo: number; hi: number } | null
  /** 평평한 루트. 누를 때 한 번 `getBoundingClientRect()` 로 포인터 원점·크기를 잡는다. */
  rootRef: RefObject<HTMLElement | null>
  /**
   * 분할 축으로 처음 실제로 움직였을 때 한 번 — 그 위치의 범위 안 비율을 싣는다(미리보기의 시작값). 누르기만 해서는
   * 부르지 않는다. 이 값은 `onPreview` 로 다시 오지 않는다.
   */
  onDragStart(ratio: number): void
  /** 드래그 중 비율 — 프레임당 최대 한 번, 직전에 알린 값(`onDragStart` 포함)과 다를 때만. */
  onPreview(ratio: number): void
  /** 움직인 뒤 뗐을 때 한 번. 밀린 프레임을 먼저 `onPreview` 로 내보내므로 직전에 알린 값과 같다. */
  onRelease(ratio: number): void
  /**
   * 드래그 중 pointercancel·창 blur·언마운트, 주 버튼이 떨어진 채 온 pointermove, 같은 포인터의 새 pointerdown(어느
   * 구분선이든), 또는 드래그 중 이 분할이 잠기거나 방향이 바뀌거나 다른 split id 로 바뀌었을 때.
   */
  onCancel(): void
  /** 더블클릭(잠김이 아닐 때). 호출자가 미리보기 없이 0.5 를 보낸다. */
  onReset(): void
}

interface Press {
  pointerId: number
  splitId: string
  dir: SplitDir
  el: HTMLElement
  /** 누를 때 잡은 루트의 분할 축 시작 좌표·길이(CSS px). */
  origin: number
  size: number
  start: number
  last: number
  moved: boolean
  frame: number | null
  previewed: number | null
}

const axisOf = (dir: SplitDir, e: { clientX: number; clientY: number }): number =>
  dir === 'left_right' ? e.clientX : e.clientY

// 드래그는 겹칠 수 있다(두 구분선을 두 포인터로). body 스타일은 첫 드래그가 원래 값을 한 번 저장하고 마지막 드래그가
//   끝날 때 되돌린다 — 드래그마다 저장·복원하면 먼저 끝난 쪽이 남은 드래그의 잠금을 풀거나, 나중에 끝난 쪽이 다른
//   드래그가 건 잠금 값을 「원래 값」으로 되살려 남긴다.
const bodyLocks = new Map<object, string>()
let bodyOriginal: { cursor: string; userSelect: string } | null = null

function lockBody(owner: object, cursor: string): void {
  if (bodyLocks.size === 0) {
    bodyOriginal = { cursor: document.body.style.cursor, userSelect: document.body.style.userSelect }
  }
  bodyLocks.set(owner, cursor)
  document.body.style.cursor = cursor
  document.body.style.userSelect = 'none'
}

function unlockBody(owner: object): void {
  if (!bodyLocks.delete(owner)) return
  if (bodyLocks.size > 0) {
    const cursors = [...bodyLocks.values()]
    document.body.style.cursor = cursors[cursors.length - 1]
    return
  }
  if (bodyOriginal !== null) {
    document.body.style.cursor = bodyOriginal.cursor
    document.body.style.userSelect = bodyOriginal.userSelect
    bodyOriginal = null
  }
}

// pointerId → 그 포인터의 누름을 쥔 구분선의 취소 함수. 같은 포인터의 새 누름이 다른 구분선에 와도 앞 누름을 닫을 수
//   있게 구분선끼리 공유한다.
const pointerOwners = new Map<number, () => void>()

// 창 수준 리스너는 떼려면 같은 함수여야 해서 인스턴스당 한 번 만들고, 최신 props 는 ref 로 읽는다.
function createDrag(latest: { current: SplitterProps }, setDragging: (v: boolean) => void) {
  let press: Press | null = null

  const ratioNow = (p: Press): number | null => {
    const { split, range } = latest.current
    if (range === null || split.split_id !== p.splitId || split.dir !== p.dir) return null
    const box0 = p.dir === 'left_right' ? split.x0 : split.y0
    const box1 = p.dir === 'left_right' ? split.x1 : split.y1
    return ratioFromPointer({ pointerNorm: (p.last - p.origin) / p.size, box0, box1, range })
  }

  const flush = (p: Press): void => {
    const r = ratioNow(p)
    if (r === null || r === p.previewed) return
    p.previewed = r
    latest.current.onPreview(r)
  }

  const end = (p: Press): void => {
    press = null
    window.removeEventListener('pointermove', onMove, true)
    window.removeEventListener('pointerup', onUp, true)
    window.removeEventListener('pointercancel', onPointerCancel, true)
    window.removeEventListener('blur', abort)
    if (p.frame !== null) cancelAnimationFrame(p.frame)
    try {
      if (p.el.hasPointerCapture?.(p.pointerId)) p.el.releasePointerCapture(p.pointerId)
    } catch {
      // 캡처는 이미 풀렸을 수 있다(웹뷰 경계를 넘나든 뒤 등) — 풀려 있으면 할 일이 없다.
    }
    unlockBody(p)
    if (pointerOwners.get(p.pointerId) === abort) pointerOwners.delete(p.pointerId)
    if (p.moved) setDragging(false)
  }

  function abort(): void {
    const p = press
    if (p === null) return
    end(p)
    if (p.moved) latest.current.onCancel()
  }

  function onPointerCancel(e: PointerEvent): void {
    if (press !== null && e.pointerId === press.pointerId) abort()
  }

  function onMove(e: PointerEvent): void {
    const p = press
    if (p === null || e.pointerId !== p.pointerId) return
    // 주 버튼 없이 온 이동은 pointerup 을 놓쳤거나 다른 버튼을 쥔 채 주 버튼을 뗀 것이다 — 어느 쪽이든 확정 근거가
    //   없고, 따라가면 버튼을 뗀 채 끌리다가 다음 pointerup 에서 아무 비율로나 확정되므로 취소로 닫는다.
    //   Pointer Events 규격상 터치 접촉·펜 팁 접촉도 bit 0 을 켠다.
    if ((e.buttons & 1) === 0) {
      abort()
      return
    }
    // pointerdown 을 막지 않으므로(아래 onPointerDown) 누른 채 끄는 동안의 기본 동작은 여기서 막는다(allotment 와 같다).
    e.preventDefault()
    p.last = axisOf(p.dir, e)
    if (!p.moved) {
      if (p.last === p.start) return
      const r = ratioNow(p)
      if (r === null) {
        end(p)
        return
      }
      p.moved = true
      p.previewed = r
      lockBody(p, p.dir === 'left_right' ? 'col-resize' : 'row-resize')
      setDragging(true)
      latest.current.onDragStart(r)
      return
    }
    if (ratioNow(p) === null) {
      abort()
      return
    }
    if (p.frame === null) {
      p.frame = requestAnimationFrame(() => {
        p.frame = null
        if (press === p) flush(p)
      })
    }
  }

  function onUp(e: PointerEvent): void {
    const p = press
    if (p === null || e.pointerId !== p.pointerId) return
    // 뗀 값은 pointerup 좌표가 아니라 마지막 pointermove 위치로 정한다 — 그래야 확정값이 직전 미리보기와 같아 뗄 때 튀지 않는다.
    const r = p.moved ? ratioNow(p) : null
    end(p)
    if (!p.moved) return
    if (r === null) {
      latest.current.onCancel()
      return
    }
    if (r !== p.previewed) {
      p.previewed = r
      latest.current.onPreview(r)
    }
    latest.current.onRelease(r)
  }

  const onPointerDown = (e: ReactPointerEvent<HTMLElement>): void => {
    if (e.button !== 0) return
    // 같은 포인터는 떼지 않고 다시 누를 수 없다(버튼을 더 눌러도 pointermove 다) — 그 포인터의 앞 누름은 pointerup 을
    //   놓친 것이니 어느 구분선 것이든 취소로 닫는다. 안 닫으면 다른 구분선이면 한 포인터가 두 분할을 함께 끌고, 같은
    //   구분선이면 새 누름이 무시돼 움직이지 않은 클릭도 앞 드래그의 마지막 값으로 확정된다.
    pointerOwners.get(e.pointerId)?.()
    if (press !== null) return
    const { split, range, rootRef } = latest.current
    const root = rootRef.current
    if (range === null || root === null) return
    const rect = root.getBoundingClientRect()
    const dir = split.dir
    const size = dir === 'left_right' ? rect.width : rect.height
    if (!(size > 0)) return
    // preventDefault 하지 않는다 — 막으면 호환 mousedown 이 사라져, 문서 mousedown 으로 닫히는 열린 메뉴·팝업
    //   (SlotContextMenu·AgentList·PresetPalette)이 구분선을 눌러도 안 닫힌다. 텍스트 선택은 구분선의
    //   `user-select: none` 과 드래그 중 body 잠금·pointermove 의 preventDefault 로 막는다.
    const start = axisOf(dir, e)
    const el = e.currentTarget
    press = {
      pointerId: e.pointerId,
      splitId: split.split_id,
      dir,
      el,
      origin: dir === 'left_right' ? rect.left : rect.top,
      size,
      start,
      last: start,
      moved: false,
      frame: null,
      previewed: null,
    }
    pointerOwners.set(e.pointerId, abort)
    // jsdom 등 포인터 캡처가 없는 환경이 있어 선택적으로 부른다 — 창 수준 리스너만으로도 드래그는 돈다.
    try {
      el.setPointerCapture?.(e.pointerId)
    } catch {
      // 캡처 실패는 치명적이지 않다 — 아래 창 수준 리스너가 떼기·취소까지 받는다.
    }
    // Chromium 은 버튼을 쥔 채로도 캡처를 잠깐 잃는다(orca `pane-divider-drag.ts`). 그래서 캡처와 별도로 창 수준에서
    //   pointerup·pointercancel·blur 까지 드래그를 쥐고, `lostpointercapture` 는 취소로 치지 않는다.
    window.addEventListener('pointermove', onMove, true)
    window.addEventListener('pointerup', onUp, true)
    window.addEventListener('pointercancel', onPointerCancel, true)
    // blur 는 캡처 단계로 걸지 않는다 — 걸면 창 안 요소의 blur 까지 받아 드래그가 끊긴다. 창 자신의 blur 만 필요하다.
    window.addEventListener('blur', abort)
  }

  /** 렌더 뒤 호출 — 누르는 동안 잠기거나 방향·split id 가 바뀌었으면 드래그를 거둔다. */
  const revalidate = (): void => {
    if (press !== null && ratioNow(press) === null) abort()
  }

  return { onPointerDown, revalidate, dispose: abort }
}

export default function Splitter(props: SplitterProps) {
  const { split, range } = props
  const latest = useRef(props)
  useLayoutEffect(() => {
    latest.current = props
  })
  const [dragging, setDragging] = useState(false)
  const [hovered, setHovered] = useState(false)
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [drag] = useState(() => createDrag(latest, setDragging))

  const locked = range === null
  useEffect(() => {
    drag.revalidate()
  }, [drag, split.split_id, split.dir, locked])

  useEffect(
    () => () => {
      drag.dispose()
      if (hoverTimer.current !== null) clearTimeout(hoverTimer.current)
    },
    [drag],
  )

  const leftRight = split.dir === 'left_right'
  const active = !locked && (dragging || hovered)
  const style: CSSProperties & Record<`--${string}`, number> = {
    '--x0': split.x0,
    '--y0': split.y0,
    '--x1': split.x1,
    '--y1': split.y1,
    '--at': split.at,
    position: 'absolute',
    zIndex: 20,
    touchAction: 'none',
    userSelect: 'none',
    cursor: locked ? 'default' : leftRight ? 'col-resize' : 'row-resize',
    ...(leftRight
      ? {
          left: `calc(var(--at) * 100% - ${HIT_PX / 2}px)`,
          width: HIT_PX,
          top: 'calc(var(--y0) * 100%)',
          height: 'calc((var(--y1) - var(--y0)) * 100%)',
        }
      : {
          top: `calc(var(--at) * 100% - ${HIT_PX / 2}px)`,
          height: HIT_PX,
          left: 'calc(var(--x0) * 100%)',
          width: 'calc((var(--x1) - var(--x0)) * 100%)',
        }),
  }

  return (
    <div
      role="separator"
      aria-orientation={leftRight ? 'vertical' : 'horizontal'}
      data-split-id={split.split_id}
      data-dir={split.dir}
      style={style}
      onPointerDown={drag.onPointerDown}
      onDoubleClick={() => {
        if (latest.current.range !== null) latest.current.onReset()
      }}
      onPointerEnter={() => {
        if (hoverTimer.current !== null) clearTimeout(hoverTimer.current)
        hoverTimer.current = setTimeout(() => {
          hoverTimer.current = null
          setHovered(true)
        }, HOVER_DELAY_MS)
      }}
      onPointerLeave={() => {
        if (hoverTimer.current !== null) clearTimeout(hoverTimer.current)
        hoverTimer.current = null
        setHovered(false)
      }}
    >
      <div
        aria-hidden
        style={{
          position: 'absolute',
          pointerEvents: 'none',
          background: active ? 'var(--accent)' : 'var(--border)',
          ...(leftRight
            ? { top: 0, bottom: 0, left: 'calc(50% - 0.5px)', width: 1 }
            : { left: 0, right: 0, top: 'calc(50% - 0.5px)', height: 1 }),
        }}
      />
    </div>
  )
}
