// TabBar — 한 창(label)의 탭 목록 + 활성 탭 표시 + [+] 새 탭 + 탭별 닫기 + 이름 인라인 편집(ADR-0057, §7-2).
//
// ★§5 손발/두뇌 분리★: 사람 조작(탭 전환/추가/닫기/이름변경)은 viewStore 액션(switchTab/createTab/closeTab/
// renameTab)만 부른다 — LLM(window.__engramCmd)이 같은 command 를 흔드는 것과 물리적으로
// 동일 표면. 실제 상태 변경은 백엔드 ViewManager(권위)가 하고 window:tabs-updated emit 으로 반영된다(낙관 갱신 X).
//
// ★탭 끌기(TRD S21-slot-docking §4-2 · §4-4) = 머리가 OS(HTML5) 끌기 출처 · `tabStrip/useTabDragSource`★ — 탭바 하나에 훅
//   하나. 그리기 규칙:
//   - 줄(`tab-strip-row`)에는 머리와 `+` 만 둔다(`measure.ts` 가 줄 폭 − `+` 폭을 잰다). 다른 창 끌기의 세로선은 그 줄 위에
//     겹쳐 그리고 그때만 있다.
//   - 머리 폭 · 숨김 = `stripBounds` · 자리 = 절대 x(`translateX`). 자기 탭바 구역 안에서 끄는 동안의 순서 이동은 transform 만
//     바꾼다 — 끄는 머리는 커서를 따르고 이웃은 확정한 순서의 자리로 비켜난다(`DragSourceSnapshot.motion`). ★끄는 머리는 숨김이어도
//     그린다★ — 출처 노드를 떼면 그 끌기의 dragend 가 오지 않는다.
//   - 머리 DOM 순서(= 탭 키 · 화면 낭독 순서)는 쉴 때 보이는 순서로 맞추고, 누름 · 끌기 · 놓은 뒤 자리 전환 · 이름 편집 동안은
//     이미 그린 순서를 지킨다 — React 가 요소를 옮기면(떼었다 다시 붙임) 진행 중인 CSS 전환이 끊겨 이웃 · 끄는 탭이
//     미끄러지지 않고 튀고(WebView2 실측 아님), 편집 중인 이름 입력은 포커스를 잃으며, 끄는 머리면 OS 끌기가 끊긴다.

import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from 'react'

import { Plus, X } from 'lucide-react'
import type { ViewMeta } from '../../api/layoutTypes'
import { t } from '../../i18n'
import { shownTab, stripHeld } from './dock/dragOverride'
import { useLocalDrag } from './dock/localDrag'
import { TAB_BUTTON_SIZE, TAB_HEAD_GAP, TAB_HEAD_PAD, TAB_MIN_ACTIVE_W, TAB_STANDARD_W } from './tabStrip/layout'
import { measureStrip, stripBounds } from './tabStrip/measure'
import { EASE_OUT_CSS, slideMs, slideTargets } from './tabStrip/reorderGeometry'
import { useTabDragSource } from './tabStrip/useTabDragSource'
import { dragProps } from './titleRow/dragRegion'
import { SURFACE_TOP, TAB_BAR_H } from './titleRow/metrics'

interface TabBarProps {
  /** 이 탭바가 속한 창 label(main·slot-popup-N). */
  label: string
  tabs: ViewMeta[]
  /** 모델 활성 탭 — `null` = 없음(대기 모드의 빈 틀). */
  active: string | null
  onSwitch: (viewId: string) => void
  onCreate: () => void
  onClose: (viewId: string) => void
  onRename: (viewId: string, name: string) => void
  /** 이 창의 탭 캔버스 영역 — 끌기 크기의 크롬(뷰포트 − 캔버스) 재기. 없으면 크롬 0 으로 친다. */
  canvas?: () => Element | null
  /** `UseTabDragSourceOptions.pullFrame`. */
  pullFrame?: boolean
}

/** 그릴 머리 하나 — `x` = 탭바 줄 왼쪽 기준 CSS px. */
interface DrawnHead {
  id: string
  x: number
  w: number
  transition: string
}

// TODO(U7c): 닫기 모드(R26 · `closingMode.layoutAvailable`)를 이으면 이 값이 그 고정 폭이 된다 — 훅과 그리기가 같은 값을 받는다.
const WIDTH_CAP: number | null = null

// ADR-0295
export default function TabBar({ label, tabs, active, onSwitch, onCreate, onClose, onRename, canvas, pullFrame }: TabBarProps) {
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  // ★안정 ref★: input 에 매 렌더 새 콜백 ref 를 걸면 React 가 매번 재부착 → select() 가 매 키입력마다
  //   실행돼 방금 친 글자를 다시 전체선택 → 다음 글자가 통째로 덮어써진다("New"→"w"). 안정 ref + effect 로
  //   편집 진입(editingId 변화) 시점에만 정확히 1회 select() 하게 고정한다.
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (editingId !== null) inputRef.current?.select()
  }, [editingId])

  // 편집을 연 때의 이름 — 확정은 지금 이름이 아니라 이것과 견준다. 편집 중 LLM 이 이름을 바꾸거나 자동 이름이 따라 바뀌어도
  //   손대지 않은 칸이 옛 이름을 써 넣지 않는다. 확정 글자(`draft.trim()`)와 견주므로 다듬어 둔다 — 앞뒤 공백이 붙은
  //   이름에서 손대지 않은 칸이 다듬은 글자를 「바뀐 글자」로 보내 자동 이름을 굳히거나 LLM 이 바꾼 이름을 덮지 않게.
  const seed = useRef('')
  const beginEdit = (tab: ViewMeta) => {
    seed.current = tab.name.trim()
    setEditingId(tab.id)
    setDraft(tab.name)
  }
  const cancelEdit = () => setEditingId(null)
  // 마지막 머리 누름이 활성으로 바꾼 탭(바꾸지 않은 누름 = `null`) — 좁은 비활성 머리는 닫기 버튼이 없어, 그 탭의 닫기
  //   버튼은 그 누름 뒤 넓어진 머리에 커서 밑으로 생겼을 수 있다.
  const activatedBy = useRef<string | null>(null)
  // Enter 뒤 input 이 빠지며 나는 blur 는 React 에 닿지 않는다 — React 19 는 commit 동안 이벤트 배달을 끄고
  //   (react-dom `prepareForCommit` 의 `_enabled=false`), 그 뒤의 blur 는 루트 리스너에 닿지 않는 떨어진 노드에 난다.
  //   첫 줄의 `editingId` 확인은 그래도 commitEdit 이 두 번 불릴 때 onRename 이 두 번 나가지 않게 하는 덧막이다.
  // `deliberate` = Enter. 같은 글자는 Enter 로만 보낸다 — 자동 이름 탭(`named` 거짓)을 그 글자 그대로 Enter 로 확정하면 그
  //   이름을 정한 것이고(TRD S26-window-chrome §3-1), 포커스 잃음 · 덮인 이웃 누름은 확정할 뜻이 아니라 바뀐 글자만 보낸다 —
  //   보내면 편집만 열었다 닫은 자동 이름이 굳는다. 정한 이름 탭의 같은 글자는 바꿀 것이 없다.
  const commitEdit = (tab: ViewMeta, deliberate: boolean) => {
    if (editingId !== tab.id) return
    const trimmed = draft.trim()
    if (trimmed.length > 0 && (trimmed !== seed.current || (deliberate && !tab.named))) onRename(tab.id, trimmed)
    setEditingId(null)
  }

  const rowRef = useRef<HTMLDivElement>(null)
  const addRef = useRef<HTMLButtonElement>(null)
  const available = useStripAvailable(rowRef, addRef)
  const ids = useMemo(() => tabs.map(tab => tab.id), [tabs])
  // 편집 중인 탭이 모델에서 빠지면(닫힘 · 다른 창으로 옮김) 편집도 끝낸다 — 남은 `editingId` 는 머리 DOM 순서 맞추기를 막는다.
  useEffect(() => {
    if (editingId !== null && !ids.includes(editingId)) setEditingId(null)
  }, [editingId, ids])
  const metaById = new Map(tabs.map(tab => [tab.id, tab]))
  const source = useTabDragSource({
    label,
    tabs: ids,
    active,
    editing: editingId,
    nameOf: id => metaById.get(id)?.name ?? '',
    row: rowRef,
    add: addRef,
    canvas,
    pullFrame,
  })
  const { snapshot } = source
  const model = { tabs: ids, active }
  const mine = snapshot.window === label
  // 칠하는 활성 = 이 창 캔버스가 보이는 탭(R15 남은 탭 · 되돌리기를 기다리는 탭 포함 — `WindowLayout` 과 같은 값).
  const shown = mine ? shownTab(snapshot.override, model) : active
  const motion = mine && stripHeld(snapshot.override, model) ? snapshot.motion : null
  const dragged =
    mine && (snapshot.override.phase === 'live' || snapshot.override.phase === 'ended') ? snapshot.override.view : null

  const order = motion === null ? ids : placeAt(ids, motion.view, motion.index)
  const at = motion === null ? -1 : order.indexOf(motion.view)
  const draggedAt = dragged === null ? -1 : order.indexOf(dragged)
  const bounds = stripBounds({
    available,
    count: order.length,
    active: motion !== null ? at : shown === null ? -1 : order.indexOf(shown),
    dragged: draggedAt >= 0 ? draggedAt : undefined,
    widthCap: WIDTH_CAP,
  })
  useLayoutEffect(() => {
    source.drawn.current = { order, bounds, available }
  })
  // 끌기 밖에도 자리 전환 속성은 `transform` 그대로 두고 시간만 0 이다 — CSS Transitions 규칙으로는 시간만 바뀌면 진행 중인
  //   전환(놓은 뒤 자리로 가는 끄는 탭 · 이웃)이 끝까지 가고, 속성을 걷으면 그 자리에서 끊긴다(WebView2 실측 — M-run 1부 C2).
  //   끌기가 풀리는 그 그림까지는 시간을 둔다 — 취소 · 거절로 모델 순서로 돌아가는 머리가 미끄러져 간다.
  const dragOwned = mine && snapshot.override.phase !== 'idle'
  const wasDragOwned = useRef(false)
  useLayoutEffect(() => {
    wasDragOwned.current = dragOwned
  })
  const slide = `transform ${dragOwned || wasDragOwned.current ? slideMs(reducedMotion(), false) : 0}ms ${EASE_OUT_CSS}`
  const targets = motion === null ? null : slideTargets(bounds, at)
  const heads: DrawnHead[] = []
  order.forEach((id, i) => {
    let x =
      motion === null || targets === null
        ? bounds.visible[i]
          ? bounds.tabs[i].x0
          : null
        : i === at
          ? motion.settled
            ? bounds.tabs[i].x0
            : motion.left
          : targets[i]
    if (x === null) return
    let w = bounds.tabs[i].x1 - bounds.tabs[i].x0
    // 이름 편집 중인 머리는 표준 폭으로 넓혀 이웃 위에 그린다 — 좁은 머리에서는 머리 내용 칸의 `overflow: hidden` 안에 이름 칸 ·
    //   입력 칸이 모자랐다(실측 — 옛 ≈ 11 px 닫기 버튼 때 머리 42 · 62 · 86 px 에서 0 · 19 · 43 px · 22 px 원으로는 다시 재지
    //   않았다). 오른쪽 끝이 쓸 수 있는 폭을 넘지 않게 왼쪽으로 당기되 줄 왼쪽 끝(0)에서 멈춘다 — 쓸 수 있는 폭이 표준 폭보다
    //   좁으면 머리가 0..160 으로 그 폭을 넘어 `+` 를 덮을 수 있고, 숨은 탭이 있어 `+` 가 쓸 수 있는 폭 안쪽에 서면 `+` 일부를
    //   덮을 수 있다. 덮인 이웃 머리의 누름 = `coveredHead`.
    if (motion === null && id === editingId && w < TAB_STANDARD_W) {
      w = TAB_STANDARD_W
      if (available !== null) x = Math.max(0, Math.min(x, available - w))
    }
    const following = motion !== null && i === at && !motion.settled
    heads.push({ id, x, w, transition: following ? 'none' : slide })
  })
  const end = bounds.lastVisible < 0 ? 0 : bounds.tabs[bounds.lastVisible].x1
  const marker = useLocalDrag(s => s.targetMarker)

  const domOrder = useDomOrder(
    heads.map(h => h.id),
    source.engaged,
    editingId !== null,
    rowRef,
  )
  const headById = new Map(heads.map(h => [h.id, h]))

  const activate = (view: string, button: number) => {
    const switching = button === 0 && metaById.has(view) && (view !== active || source.forceSwitch(view))
    activatedBy.current = switching ? view : null
    if (switching) onSwitch(view)
  }
  // 이름 편집 중 넓힌 머리(`id`)가 덮은 자리의 누름은 그 밑 머리의 누름이다 — 돌리지 않으면 덮인 이웃은 편집을 끝내기
  //   전까지 눌리지 않는다. 넓히기 전 자기 자리 안 · 밑에 머리가 없는 곳(`+` · 빈 줄)이면 `null`.
  const coveredHead = (id: string, clientX: number): string | null => {
    const row = rowRef.current
    if (motion !== null || row === null) return null
    const x = clientX - row.getBoundingClientRect().left
    const under = (i: number) => bounds.visible[i] && x >= bounds.tabs[i].x0 && x < bounds.tabs[i].x1
    if (under(order.indexOf(id))) return null
    const j = order.findIndex((_, i) => under(i))
    return j < 0 ? null : order[j]
  }

  // 순서 이동을 거둔 뒤에도 자리로 가는 동안 이웃 위에 그린다 — 다음 끌기까지 둔다(겹치지 않으면 차이가 없다).
  const lastDragged = useRef<string | null>(null)
  useLayoutEffect(() => {
    if (motion !== null) lastDragged.current = motion.view
  })
  const topId = motion?.view ?? lastDragged.current

  // 순서 이동을 그리는 동안은 맴돎을 칠하지 않는다 — 누른 순간의 맴돎이 비켜나는 머리에 남아 보인다(OS 끌기 동안 머리의 들고
  //   남이 오는지는 미검 — TRD §2-4 · M11).
  const [hovered, setHovered] = useState<string | null>(null)
  const activeId = shown
  const lit = (id: string) => id === activeId || (motion === null && id === hovered)
  // 맴돎이 끝난 지 `SEPARATOR_FADE_MS` 안의 머리 — 그 양옆 구분선만 서서히 되살아난다(`SEPARATOR_FADE_MS` 머리 주석).
  const [hoverEnding, setHoverEnding] = useState<ReadonlySet<string>>(() => new Set())
  const hoverEndTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>())
  useEffect(() => {
    const timers = hoverEndTimers.current
    return () => timers.forEach(clearTimeout)
  }, [])
  const endHover = (id: string) => {
    setHovered(h => (h === id ? null : h))
    clearTimeout(hoverEndTimers.current.get(id))
    setHoverEnding(s => new Set(s).add(id))
    hoverEndTimers.current.set(
      id,
      setTimeout(() => {
        hoverEndTimers.current.delete(id)
        setHoverEnding(s => {
          const next = new Set(s)
          next.delete(id)
          return next
        })
      }, SEPARATOR_FADE_MS),
    )
  }
  // 왼쪽 가에 구분선을 두는 머리 — 그린 이웃과의 사이마다 하나. 보이는 것은 양쪽 어느 머리도 활성 · 맴돎이 아닐 때만이고,
  //   그 밖에는 사라진다(떼지 않는다 — 다시 나타날 때 서서히 나타나는 전환이 보여야 한다)(Chrome).
  const leftNeighbour = new Map(heads.slice(1).map((h, k) => [h.id, heads[k].id]))
  const separatorShown = (id: string) => !lit(leftNeighbour.get(id)!) && !lit(id)
  const separatorFadesIn = (id: string) => hoverEnding.has(id) || hoverEnding.has(leftNeighbour.get(id)!)
  // 마지막 그린 머리의 오른쪽 가에도 하나 — 없으면 마지막 비활성 머리의 × 와 + 가 붙어 보였다(ADR-0298 결정 4).
  //   머리 안쪽에 걸어 줄 폭 · + 자리는 그대로다. 보임 · 서서히 = 그 머리 하나만 본다.
  const trailingId = heads.length > 0 ? heads[heads.length - 1].id : null

  return (
    <div
      data-testid="tab-bar"
      data-window-label={label}
      style={{
        display: 'flex',
        alignItems: 'stretch',
        height: TAB_BAR_H,
        // 제목 줄에서 탭바만 줄어든다 — 줄이 좁아지면 잰 폭(`useStripAvailable`)이 줄어 탭이 먼저 준다(TRD S26-window-chrome §7-1).
        flex: '1 1 0px',
        minWidth: 0,
        // ★면 규칙 = 활성 탭은 내용과 같은 면(`--tab-active-bg`)이라 아래 캔버스로 이어지고, 바(`--tab-bar-bg`)는 내용과
        //   밝기가 다른 면★ — Chrome 꼴(사용자 결정 2026-10-19 · 값은 테마마다 `styles/theme.css`). 라이트 · e-ink = 바가 내용보다
        //   한 칸 눌린 면 · 다크 = 바가 내용보다 밝아 활성 탭이 가장 어둡다(Zed Ayu Dark 꼴 — ADR-0298 결정 1).
        //   ☞ 활성 탭이 바보다 어두운 배치는 「활성 탭이 구멍처럼 읽힌다」로 되돌린 적이 있고(사용자 판단 2026-08-23) 그 판단은
        //   라이트 · e-ink 에 남는다 — 다크는 ADR-0298 이 그것을 뒤집었다(바가 #000 이면 활성이 안 보였다). `--text` 를
        //   섞어 바를 띄우는 안은 바가 트리 패널과 따로 노는 회색 띠가 돼 되돌렸다(사용자 판단 2026-08-23).
        background: 'var(--tab-bar-bg)',
        // 바 아래 1px 선 — 활성 면 · 날개가 덮은 자리만 끊긴다. 선이 바와 내용을 가르는 것은 e-ink 뿐이고 다크 · 라이트는 면
        //   밝기 차가 가르므로 투명이다(선 자리는 남아 줄 높이는 그대로). 테두리를 지우면 줄 높이가 1px 늘어 활성 면의
        //   `bottom: -1` 도 함께 고친다.
        borderBottom: '1px solid var(--tab-bar-edge)',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
        userSelect: 'none',
      }}
    >
      <div
        ref={rowRef}
        data-testid="tab-strip-row"
        // 탭 사이 · 마지막 탭 뒤 빈 곳 = 창 끌 자리(자기 자신만 — 머리 · `+` 는 아니다 · TRD S26-window-chrome §7-2).
        {...dragProps()}
        style={{ position: 'relative', flex: '1 1 0', minWidth: 0 }}
      >
        {domOrder.map(id => {
          const head = headById.get(id)!
          // `placeAt` 은 모델에 없는 끄는 탭도 줄에 넣는다 — 그 머리는 이름 없이 자리 · 폭만 그린다. 순서 이동은 끄기 시작의 탭 목록이
          //   모델과 같을 때만 그려(`stripHeld`) 지금은 닿지 않는 것으로 보인다(미검).
          const tab = metaById.get(id) ?? { id, name: '', named: false }
          const isActive = id === activeId
          const editing = editingId === id
          // 이름 · × 는 한 전경색 — 활성 · 맴돎이면 `--tab-text`(= `--text` · ADR-0298 결정 8). 한 전경색은 Chrome 꼴이고(`tab.cc`
          //   — 탭 제목과 닫기 단추에 같은 전경색), 맴돎 → `--text` 는 우리 선택이다(Chrome 은 맴돎 전경색을 따로 둔다 —
          //   `kColorTabForegroundInactiveHoverFrameActive` · `tab_style.cc`). 두 토큰은 창이 앞 창이 아닐 때 제목 줄이 흐림으로
          //   갈아 끼운다(`styles/theme.css` · TRD S26-window-chrome §7-5).
          const fg = lit(id) ? 'var(--tab-text)' : 'var(--tab-text-inactive)'
          return (
            <div
              key={id}
              data-testid="tab"
              data-view-id={id}
              data-active={isActive ? 'true' : 'false'}
              draggable={source.draggable(id)}
              // 누름은 캡처 단계에서 적는다 — 닫기 버튼이 거품을 끊어도 그 자리에서 시작한 끌기를 가를 수 있고, 전환(아래 거품
              //   처리기)보다 먼저라 누르기 전 활성이 그 끌기의 `initial` 이 된다.
              onPointerDownCapture={e => source.onPointerDown(e, id)}
              // 누르는 순간 활성(TRD §2-3 ①). 편집 중인 머리의 누름은 끌기 후보가 아니고, 덮인 이웃 자리의 누름은 편집을 확정하고
              //   그 이웃으로 전환만 한다.
              onPointerDown={e => {
                if (editing) {
                  const inInput = e.target instanceof Element && e.target.closest('input') !== null
                  const under = e.button === 0 && !inInput ? coveredHead(id, e.clientX) : null
                  if (under === null) return
                  commitEdit(tab, false)
                  activate(under, e.button)
                  return
                }
                activate(id, e.button)
              }}
              onDragStart={e => source.onDragStart(e, id)}
              onDragEnd={e => source.onDragEnd(e, id)}
              // 머리 어디든(이름 칸 폭이 0 인 좁은 활성 머리 포함) — 닫기 버튼 위는 그 버튼이 닫지 않는 클릭일 때만.
              onDoubleClick={e => {
                if (editing) return
                if (e.target instanceof Element && e.target.closest('button') !== null && activatedBy.current !== id) return
                beginEdit(tab)
              }}
              onPointerEnter={() => setHovered(id)}
              onPointerLeave={() => endHover(id)}
              style={{
                position: 'absolute',
                top: 0,
                bottom: 0,
                left: 0,
                width: head.w,
                transform: `translateX(${head.x}px)`,
                transition: head.transition,
                zIndex: editing ? 2 : id === topId ? 1 : undefined,
                boxSizing: 'border-box',
                cursor: 'pointer',
                color: fg,
              }}
            >
              {/* 면 · 날개 · 구분선은 그리기만 한다(`pointer-events: none`) — 머리 상자(누름 · 잡기 판정)는 그대로이고 날개가 이웃
                  머리 위로 삐져도 그 자리의 누름은 이웃 것이다. ★accent 밑줄을 되살리지 말 것★ — 2px 파란 줄이 프레임에서 가장
                  튀어 "고대비"의 주범이었다(사용자 판단 2026-08-23, Orca 대조). */}
              {isActive ? (
                <>
                  <span aria-hidden data-testid="tab-surface" style={ACTIVE_SURFACE} />
                  <span aria-hidden data-testid="tab-flare-left" style={FLARE_LEFT} />
                  <span aria-hidden data-testid="tab-flare-right" style={FLARE_RIGHT} />
                </>
              ) : editing ? (
                // 넓힌 편집 머리가 덮은 이웃 글자가 비쳐 보이지 않게 가린다.
                <span aria-hidden style={EDIT_COVER} />
              ) : (
                lit(id) && <span aria-hidden data-testid="tab-hover" style={HOVER_SURFACE} />
              )}
              {leftNeighbour.has(id) && (
                <span aria-hidden data-testid="tab-separator" style={separatorStyle(separatorShown(id), separatorFadesIn(id))}>
                  <span style={SEPARATOR_LINE} />
                </span>
              )}
              {id === trailingId && (
                <span
                  aria-hidden
                  data-testid="tab-separator-trailing"
                  style={{ ...separatorStyle(!lit(id), hoverEnding.has(id)), left: undefined, right: -SEPARATOR_SLOT / 2 }}
                >
                  <span style={SEPARATOR_LINE} />
                </span>
              )}
              <div style={HEAD_CONTENT}>
                {editing ? (
                  <input
                    data-testid="tab-rename-input"
                    data-view-id={id}
                    value={draft}
                    autoFocus
                    // 여기 인라인 콜백 ref 로 select() 하면 매 렌더 재실행돼 타이핑이 깨진다(FIX 1).
                    ref={inputRef}
                    onChange={e => setDraft(e.target.value)}
                    onKeyDown={e => {
                      // ★버블 차단★: 편집 중 키가 전역 키바인딩으로 새면 안 된다.
                      e.stopPropagation()
                      if (e.key === 'Enter') commitEdit(tab, true)
                      else if (e.key === 'Escape') cancelEdit()
                    }}
                    onBlur={() => commitEdit(tab, false)}
                    style={{
                      // ★내용 폭에 맞춤★: field-sizing:content 로 input 이 draft 텍스트 폭만큼만 차지한다
                      //   (기본 input 은 size 속성 폭 ~160px 고정이라 짧은 이름에도 넓게 벌어짐). 브라우저가
                      //   실제 글자 폭을 측정하므로 CJK·비례폰트도 정확. WebView2(Chromium 123+) 지원.
                      //   minWidth 로 빈/한글자 때 너무 좁아지는 것만 방어, maxWidth 는 상한.
                      fieldSizing: 'content',
                      minWidth: '3ch',
                      maxWidth: '160px',
                      font: 'inherit',
                      color: 'var(--text)',
                      background: 'var(--bg)',
                      border: '1px solid var(--accent)',
                      borderRadius: '2px',
                      padding: '0 2px',
                      outline: 'none',
                    }}
                  />
                ) : (
                  <span
                    data-testid="tab-name"
                    style={{ flex: '1 1 auto', minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}
                  >
                    {tab.name}
                  </span>
                )}
                {/* Chrome 처럼 활성 최소 폭보다 좁은 비활성 머리는 닫기 버튼을 감춘다(`layout.ts` `TAB_MIN_ACTIVE_W`). */}
                {(isActive || head.w >= TAB_MIN_ACTIVE_W) && (
                  <button
                    type="button"
                    aria-label={t('tab.close', { name: tab.name })}
                    data-testid="tab-close"
                    // 닫기 버튼 누름은 머리 누름(전환 · 끌기)이 아니다.
                    onPointerDown={e => e.stopPropagation()}
                    // 이름 편집 중인 탭의 닫기 버튼은 누름에 입력 칸 포커스를 뺏지 않는다 — 뺏으면 blur 의 확정으로 머리가 다시
                    //   그려져(넓힌 머리는 줄고 · 입력 칸 바로 뒤의 버튼은 머리 오른쪽 끝으로 간다) 버튼이 커서 밑에서 빠지고 그
                    //   클릭은 닫지 못한다. 다른 탭의 닫기 버튼 누름은 그대로 편집을 확정한다.
                    onMouseDown={editing ? e => e.preventDefault() : undefined}
                    onClick={e => {
                      // 같은 연속 클릭(detail ≥ 2)에서 앞 누름이 이 탭을 활성으로 만들어 이 버튼이 커서 밑에 막 생겼다 — 닫지 않는다
                      //   (좁은 탭바의 비활성 머리 더블클릭이 탭을 닫았다 4/7 — M-run 1부 이상 3). 연속 클릭으로 닫다가 밀려 온
                      //   다음 탭의 닫기 버튼은 그대로 닫는다.
                      if (e.detail >= 2 && activatedBy.current === id) return
                      activatedBy.current = null
                      onClose(id)
                    }}
                    className={BUTTON_HOVER}
                    style={{ ...CLOSE_STYLE, color: fg }}
                  >
                    <X aria-hidden size={BUTTON_ICON} strokeWidth={2} />
                  </button>
                )}
              </div>
            </div>
          )
        })}
        <button
          ref={addRef}
          type="button"
          aria-label={t('tab.create')}
          data-testid="tab-add"
          onClick={onCreate}
          className="group/add"
          style={{ ...ADD_STYLE, transform: `translateX(${end}px)`, transition: slide }}
        >
          <span aria-hidden data-testid="tab-add-circle" className={ADD_HOVER} style={ADD_CIRCLE}>
            <Plus size={BUTTON_ICON} strokeWidth={2} />
          </span>
        </button>
        {marker !== null && (
          <span
            aria-hidden
            data-testid="tab-drop-marker"
            style={{ ...MARKER_STYLE, transform: `translateX(${marker - MARKER_W / 2}px)` }}
          />
        )}
      </div>
    </div>
  )
}

// 닫기 · `+` 단추 = `TAB_BUTTON_SIZE`(22) px 원 안의 13 px 아이콘(ADR-0298 결정 5 · 사용자 결정 2026-10-21). `+` 원 = Chrome
//   `tab_strip_control_button.cc` `kButtonSize` 28 을 머리 높이 비율 28/34 로 줄인 자리(셈은 ≈ 23 — 22 는 사용자 결정). × 원 =
//   같은 22 — Chrome 닫기 단추의 보이는 크기(맴돎 모양 = `layout_constants.cc` `kTabCloseButtonSize` 16 dp · `tab.cc`
//   #545-548)보다 크게 둔 사용자 결정이라 Chrome 비율로 줄이지 말 것. 아이콘 둘 = Chrome `tab_strip_control_button.cc`
//   `kIconSize` 16 × 28/34 ≈ 13. 맴돎 원 = Chrome 잉크 드롭 하이라이트 불투명도 0.16(`--tab-button-hover-bg`).
const BUTTON_ICON = 13
// ★클래스 이름은 통째 글자로 둔다★ — Tailwind 는 소스 글자를 훑어 클래스를 만들므로 조각을 이어 붙이면 생기지 않는다. 맴돎
//   색을 인라인 `background` 로 두지 말 것 — 인라인이 클래스를 이겨 맴돎 원이 사라진다.
const BUTTON_HOVER = 'bg-transparent hover:bg-[var(--tab-button-hover-bg)]'
const ADD_HOVER = 'group-hover/add:bg-[var(--tab-button-hover-bg)]'

const CLOSE_STYLE: CSSProperties = {
  flexShrink: 0,
  width: TAB_BUTTON_SIZE,
  height: TAB_BUTTON_SIZE,
  borderRadius: '50%',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  padding: 0,
  border: 'none',
  cursor: 'pointer',
}

/** `+` 원 앞 틈 — 마지막 머리 오른 가에 걸친 구분선(±2.5)에 원이 닿지 않게. */
const TAB_ADD_GAP = 4

// 가로 여백(margin)을 두지 말 것 — 쓸 수 있는 폭은 줄 폭 − `+` 테두리 상자 폭이다(`measure.ts`). 원 앞 틈은 그래서 margin
//   이 아니라 왼 패딩이다(상자 = 틈 4 + 원 22). 맴돎 원은 상자가 아니라 안의 원에만 칠한다(Tailwind `group-hover/add`).
const ADD_STYLE: CSSProperties = {
  position: 'absolute',
  top: SURFACE_TOP,
  bottom: 0,
  left: 0,
  display: 'flex',
  alignItems: 'center',
  background: 'transparent',
  border: 'none',
  // × 의 쉴 때 회색과 같은 토큰.
  color: 'var(--tab-text-inactive)',
  cursor: 'pointer',
  padding: `0 0 0 ${TAB_ADD_GAP}px`,
}

const ADD_CIRCLE: CSSProperties = {
  width: TAB_BUTTON_SIZE,
  height: TAB_BUTTON_SIZE,
  borderRadius: '50%',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
}

const REDUCED_MOTION = '(prefers-reduced-motion: reduce)'

/** Chrome 의 풍부한 애니메이션 꺼짐 — 비켜남 시간 0(`reorderGeometry.slideMs`). */
function reducedMotion(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia(REDUCED_MOTION).matches
}

/** `ids` 에서 `view` 를 빼고 그 목록의 `index` 자리에 끼운 순서(범위 밖은 가까운 끝). */
function placeAt(ids: readonly string[], view: string, index: number): string[] {
  const others = ids.filter(id => id !== view)
  const k = Math.min(Math.max(index, 0), others.length)
  return [...others.slice(0, k), view, ...others.slice(k)]
}

// 활성 탭 모양 = Chrome 그대로(PRD R27 · ADR-0295 결정 1 · Chromium `tab_style.cc` `GetTopCornerRadius` ·
//   `GetBottomCornerRadius`) — 위 모서리 반지름과 아래 바깥 날개 반지름은 다른 값이다.
const CORNER_RADIUS = 10
const FLARE_RADIUS = 12

// 날개(`FLARE_*`)가 머리 밖으로 삐지므로 머리 자신은 `overflow: hidden` 을 두지 않는다 — 자르기는 이 내용 칸이 진다.
const HEAD_CONTENT: CSSProperties = {
  position: 'absolute',
  top: SURFACE_TOP,
  bottom: 0,
  left: 0,
  right: 0,
  boxSizing: 'border-box',
  overflow: 'hidden',
  display: 'flex',
  alignItems: 'center',
  gap: TAB_HEAD_GAP,
  padding: `0 ${TAB_HEAD_PAD}px`,
  whiteSpace: 'nowrap',
}

// `bottom: -1` = 바 아래 테두리(1px)까지 덮어 내용과 끊김 없이 이어진다.
const ACTIVE_SURFACE: CSSProperties = {
  position: 'absolute',
  top: SURFACE_TOP,
  bottom: -1,
  left: 0,
  right: 0,
  borderRadius: `${CORNER_RADIUS}px ${CORNER_RADIUS}px 0 0`,
  background: 'var(--tab-active-bg)',
  pointerEvents: 'none',
}

// 활성 면 아래 두 모서리의 오목한 날개 — 머리 밖 반지름 폭 정사각형에서 바깥 위 모서리를 중심으로 한 원 밖만 칠한다.
const flare = (side: 'left' | 'right'): CSSProperties => ({
  position: 'absolute',
  bottom: -1,
  left: side === 'left' ? -FLARE_RADIUS : undefined,
  right: side === 'right' ? -FLARE_RADIUS : undefined,
  width: FLARE_RADIUS,
  height: FLARE_RADIUS,
  background: `radial-gradient(circle at ${side === 'left' ? '0 0' : '100% 0'}, transparent ${FLARE_RADIUS - 0.5}px, var(--tab-active-bg) ${FLARE_RADIUS}px)`,
  pointerEvents: 'none',
})
const FLARE_LEFT = flare('left')
const FLARE_RIGHT = flare('right')

// 아래 2px 은 비워 둔다 — 맴돎 면이 옆 활성 탭의 날개를 덮지 않는다.
const HOVER_SURFACE: CSSProperties = {
  position: 'absolute',
  top: SURFACE_TOP,
  bottom: 2,
  left: 2,
  right: 2,
  borderRadius: '6px',
  background: 'var(--tab-hover-bg)',
  pointerEvents: 'none',
}

const EDIT_COVER: CSSProperties = {
  position: 'absolute',
  inset: 0,
  background: 'var(--tab-bar-bg)',
  pointerEvents: 'none',
}

// 다른 창 끌기의 끼울 자리 — VS Code 의 2 px 세로선(TRD §4-3 · W13 · U2 ② 양쪽 머리 끝에 1 px 씩 걸친다). x = 탭바 좌표
//   (`localDrag` `targetMarker` — 그 창 뿌리 dragover 가 적는다).
const MARKER_W = 2
const MARKER_STYLE: CSSProperties = {
  position: 'absolute',
  top: SURFACE_TOP,
  bottom: 0,
  left: 0,
  width: MARKER_W,
  background: 'var(--accent)',
  pointerEvents: 'none',
  zIndex: 3,
}

// 구분선 = Chrome 꼴(PRD R27 · Chromium `tab_style.cc` `kSeparator*` · `horizontal_tab_style_views.cc`
//   `GetSeparatorBounds`) — 좌우 여백 2 · 둥글기 = 두께의 반 · 위아래 = 머리 높이 가운데는 그대로이고, ★두께 1 · 높이 13 은
//   ADR-0298 결정 3★(두께 = Chrome 의 2 보다 얇게 · 높이 = Chrome 탭 34 대 구분선 16 의 비율을 우리 머리 28 에). 여백 ·
//   두께 · 여백 칸을 이웃 머리와의 경계 가운데에 건다 — Chrome 은 그 칸을 탭끼리 겹친 자리(18)에 두는데 우리 머리는 겹치지
//   않는다(`tabStrip/layout.ts` 머리).
const SEPARATOR_W = 1
const SEPARATOR_H = 13
const SEPARATOR_MARGIN = 2
const SEPARATOR_SLOT = SEPARATOR_MARGIN + SEPARATOR_W + SEPARATOR_MARGIN
// 사라지기 · 나타나기 = Chrome 소스 그대로(PRD R27 · Q23) — 배경이 보이는 탭(활성 ·
//   맴돎) 옆 구분선은 곧바로 사라진다(맴돎 값을 보간하기 전에 0 을 낸다 — `horizontal_tab_style_views.cc`
//   `GetSeparatorOpacity`). 서서히는 되살아나기만이고 그것도 맴돎이 끝난 머리 양옆뿐이다 — 구분선 불투명도 = 1 − 그 머리의
//   맴돎 값이라(`GetHoverInterpolatedSeparatorOpacity`) 맴돎 값이 내려가는 동안(`glow_hover_controller.h` 기본 200 ms ·
//   꺼질 때 `Tween::EASE_IN`) 나타난다. 활성이 옮아 되살아나는 구분선은 곧바로 나타난다.
const SEPARATOR_FADE_MS = 200
// `Tween::EASE_IN` `t²`(tween.cc#37-38)을 CSS 로 — `EASE_OUT_CSS` 와 같은 꼴.
const EASE_IN_CSS = 'cubic-bezier(0.3333, 0, 0.6667, 0.3333)'
const SEPARATOR_SLOT_STYLE: CSSProperties = {
  position: 'absolute',
  top: 0,
  bottom: 0,
  left: -SEPARATOR_SLOT / 2,
  width: SEPARATOR_SLOT,
  boxSizing: 'border-box',
  padding: `0 ${SEPARATOR_MARGIN}px`,
  display: 'flex',
  alignItems: 'center',
  pointerEvents: 'none',
}
const SEPARATOR_LINE: CSSProperties = {
  width: SEPARATOR_W,
  height: SEPARATOR_H,
  flexShrink: 0,
  borderRadius: SEPARATOR_W / 2,
  background: 'var(--tab-separator)',
}
const separatorStyle = (shown: boolean, fadesIn: boolean): CSSProperties => ({
  ...SEPARATOR_SLOT_STYLE,
  opacity: shown ? 1 : 0,
  transition: shown && fadesIn ? `opacity ${SEPARATOR_FADE_MS}ms ${EASE_IN_CSS}` : undefined,
})

/**
 * 머리들이 쓸 수 있는 폭(줄 폭 − `+` 폭 — 훅이 끌기 때 재는 것과 같은 식) — 줄 크기가 바뀔 때마다 다시 잰다. 줄 폭이 0 이면
 * (재기 전 · 잴 수 없는 환경) `null` = 제한 없음.
 */
function useStripAvailable(
  row: RefObject<HTMLElement | null>,
  add: RefObject<HTMLElement | null>,
): number | null {
  const [available, setAvailable] = useState<number | null>(null)
  useLayoutEffect(() => {
    const el = row.current
    if (el === null) return
    const measure = () => {
      const m = measureStrip({ row: el, add: add.current })
      setAvailable(m.row.x1 > m.row.x0 ? m.available : null)
    }
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [row, add])
  return available
}

/**
 * 머리 DOM 순서(머리 주석 「DOM 순서」) — 쉴 때는 보이는 순서(`visual`), 그 밖에는 이미 그린 머리를 제자리에 두고 새 머리를
 * 끝에 붙인다. 쉰다 = 이름 편집이 없고, 끌기 훅이 누름을 놓았고(`engaged`), 그 뒤로 탭바의 전환이 다 끝났다.
 */
function useDomOrder(
  visual: string[],
  engaged: boolean,
  editing: boolean,
  row: RefObject<HTMLElement | null>,
): string[] {
  // 끌기 훅이 누름을 쥐면 `false` · 놓고 전환까지 끝나 맞출 때가 되면 `true`.
  const [synced, setSynced] = useState(true)
  const committed = useRef<string[]>([])

  useEffect(() => {
    if (engaged) {
      setSynced(false)
      return
    }
    if (synced || editing) return
    let live = true
    const step = (): void => {
      if (!live) return
      // `getAnimations` 는 스타일을 먼저 갱신해 방금 그린 자리 전환까지 센다(Web Animations 규정 · WebView2 실측 아님). 끝나고
      //   채움으로 남은 것은 `finished` 가 이미 풀려 있어 세면 기다림이 제자리를 돈다. jsdom 에는 없다(= 전환 없음).
      const running = (row.current?.getAnimations?.({ subtree: true }) ?? []).filter(
        a => a.playState === 'running' || a.pending,
      )
      if (running.length === 0) setSynced(true)
      else void Promise.allSettled(running.map(a => a.finished)).then(step)
    }
    // 뗌에 이은 click 이 간 뒤에 맞춘다 — 그 사이 머리를 옮기면(떼었다 다시 붙임) click 이 머리에 안 가 더블클릭 이름 바꾸기가
    //   깨질 수 있다(WebView2 실측 아님).
    const timer = setTimeout(step, 0)
    return () => {
      live = false
      clearTimeout(timer)
    }
  }, [engaged, editing, synced, row])

  let order: string[]
  if (synced && !engaged && !editing) {
    order = visual
  } else {
    const drawn = new Set(visual)
    order = committed.current.filter(id => drawn.has(id))
    const kept = new Set(order)
    for (const id of visual) if (!kept.has(id)) order.push(id)
  }
  useLayoutEffect(() => {
    committed.current = order
  })
  return order
}
