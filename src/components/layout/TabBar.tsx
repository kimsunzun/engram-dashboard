// TabBar — 한 창(label)의 탭 목록 + 활성 탭 표시 + [+] 새 탭 + 탭별 닫기 + 이름 인라인 편집(ADR-0057, §7-2).
//
// ★§5 손발/두뇌 분리★: 사람 조작(탭 전환/추가/닫기/이름변경)은 viewStore 액션(switchTab/createTab/closeTab/
// renameTab)만 부른다 — LLM(window.__engramCmd)이 같은 command 를 흔드는 것과 물리적으로
// 동일 표면. 실제 상태 변경은 백엔드 ViewManager(권위)가 하고 window:tabs-updated emit 으로 반영된다(낙관 갱신 X).
//
// ★탭 끌기(TRD S21-slot-docking §2-2 · §5-1) = `tabStrip/useTabDrag`★ — 탭바 하나에 훅 하나. 그리기 규칙:
//   - 탭바 줄(`tab-strip-row`)은 끌기 중이든 아니든 같은 DOM 노드다 — 포인터 캡처가 거기 걸려, 다시 마운트되면
//     `lostpointercapture` 로 끌기가 되돌려진다. 줄에는 머리와 `+` 만 둔다(`measure.ts` 가 줄 폭 − `+` 폭을 잰다).
//   - 머리 폭 · 숨김 = `stripBounds`(훅과 같은 입력) · 자리 = 절대 x(끄는 동안 = 훅이 준 `targets` · `left`).
//   - 머리 DOM 순서(= 탭 키 · 화면 낭독 순서)는 쉴 때 보이는 순서로 맞추고, 누름 · 끌기 · 놓은 뒤 자리 전환 · 이름 편집 동안은
//     이미 그린 순서를 지킨다 — React 가 요소를 옮기면(떼었다 다시 붙임) 진행 중인 CSS 전환이 끊겨 이웃 · 끄는 탭이
//     미끄러지지 않고 튀고(WebView2 실측 아님), 편집 중인 이름 입력은 포커스를 잃는다.

import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from 'react'

import type { ViewMeta } from '../../api/layoutTypes'
import { t } from '../../i18n'
import { TAB_MIN_ACTIVE_W, TAB_STANDARD_W } from './tabStrip/layout'
import { measureStrip, stripBounds } from './tabStrip/measure'
import { EASE_OUT_CSS } from './tabStrip/reorderGeometry'
import { useTabDrag } from './tabStrip/useTabDrag'

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
export default function TabBar({ label, tabs, active, onSwitch, onCreate, onClose, onRename }: TabBarProps) {
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  // ★안정 ref★: input 에 매 렌더 새 콜백 ref 를 걸면 React 가 매번 재부착 → select() 가 매 키입력마다
  //   실행돼 방금 친 글자를 다시 전체선택 → 다음 글자가 통째로 덮어써진다("New"→"w"). 안정 ref + effect 로
  //   편집 진입(editingId 변화) 시점에만 정확히 1회 select() 하게 고정한다.
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (editingId !== null) inputRef.current?.select()
  }, [editingId])

  const beginEdit = (tab: ViewMeta) => {
    setEditingId(tab.id)
    setDraft(tab.name)
  }
  const cancelEdit = () => setEditingId(null)
  // 마지막 머리 누름이 활성으로 바꾼 탭(바꾸지 않은 누름 = `null`) — 좁은 비활성 머리는 닫기 버튼이 없어, 그 탭의 닫기
  //   버튼은 그 누름 뒤 넓어진 머리에 커서 밑으로 생겼을 수 있다.
  const activatedBy = useRef<string | null>(null)
  // ★멱등★: editingId 가 이 탭이 아니면 즉시 return — Enter 가 setEditingId(null) 로 input 을 언마운트하면
  //   브라우저가 blur 를 쏴 onBlur→commitEdit 이 한 번 더 돈다. Enter 후 editingId 는 이미 null 이므로
  //   blur 의 commitEdit 은 여기서 no-op 이 돼 onRename 이중 호출(중복 rename_tab invoke)을 막는다.
  const commitEdit = (tab: ViewMeta) => {
    if (editingId !== tab.id) return
    const trimmed = draft.trim()
    if (trimmed.length > 0 && trimmed !== tab.name) onRename(tab.id, trimmed)
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
  const { drag, engaged, onPointerDown } = useTabDrag({ tabs: ids, active, row: rowRef, add: addRef, widthCap: WIDTH_CAP })

  const order = drag?.order ?? ids
  const dragged = drag === null ? -1 : order.indexOf(drag.view)
  const bounds = stripBounds({
    available,
    count: order.length,
    active: drag !== null ? dragged : active === null ? -1 : ids.indexOf(active),
    dragged: drag !== null ? dragged : undefined,
    widthCap: WIDTH_CAP,
  })
  // 끌기 밖에도 자리 전환 속성은 `transform` 그대로 두고 시간만 0 이다 — CSS Transitions 규칙으로는 시간만 바뀌면 진행 중인
  //   전환(놓은 뒤 자리로 가는 끄는 탭 · 이웃)이 끝까지 가고, 속성을 걷으면 그 자리에서 끊긴다(WebView2 실측 — M-run 1부 C2).
  const slide = `transform ${drag?.slideMs ?? 0}ms ${EASE_OUT_CSS}`
  const heads: DrawnHead[] = []
  order.forEach((id, i) => {
    let x = drag === null ? (bounds.visible[i] ? bounds.tabs[i].x0 : null) : i === dragged ? drag.left : drag.targets[i]
    if (x === null) return
    let w = bounds.tabs[i].x1 - bounds.tabs[i].x0
    // 이름 편집 중인 머리는 표준 폭으로 넓혀 이웃 위에 그린다 — 좁은 머리에서는 머리 내용 칸의 `overflow: hidden` 안에 이름 칸 ·
    //   입력 칸이 모자랐다(실측 — 머리 42 · 62 · 86 px 에서 0 · 19 · 43 px). 오른쪽 끝이 쓸 수 있는 폭을 넘지 않게 왼쪽으로
    //   당기되 줄 왼쪽 끝(0)에서 멈춘다 — 쓸 수 있는 폭이 표준 폭보다 좁으면 머리가 0..160 으로 그 폭을 넘어 `+` 를 덮을 수
    //   있고, 숨은 탭이 있어 `+` 가 쓸 수 있는 폭 안쪽에 서면 `+` 일부를 덮을 수 있다. 덮인 이웃 머리의 누름 = `coveredHead`.
    if (drag === null && id === editingId && w < TAB_STANDARD_W) {
      w = TAB_STANDARD_W
      if (available !== null) x = Math.max(0, Math.min(x, available - w))
    }
    const following = drag !== null && i === dragged && !drag.settling
    heads.push({ id, x, w, transition: following ? 'none' : slide })
  })
  const end = bounds.lastVisible < 0 ? 0 : bounds.tabs[bounds.lastVisible].x1

  const domOrder = useDomOrder(
    heads.map(h => h.id),
    engaged,
    editingId !== null,
    rowRef,
  )
  const headById = new Map(heads.map(h => [h.id, h]))
  const metaById = new Map(tabs.map(tab => [tab.id, tab]))

  const activate = (view: string, button: number) => {
    const switching = button === 0 && view !== active && metaById.has(view)
    activatedBy.current = switching ? view : null
    if (switching) onSwitch(view)
  }
  // 이름 편집 중 넓힌 머리(`id`)가 덮은 자리의 누름은 그 밑 머리의 누름이다 — 돌리지 않으면 덮인 이웃은 편집을 끝내기
  //   전까지 눌리지 않는다. 넓히기 전 자기 자리 안 · 밑에 머리가 없는 곳(`+` · 빈 줄)이면 `null`.
  const coveredHead = (id: string, clientX: number): string | null => {
    const row = rowRef.current
    if (drag !== null || row === null) return null
    const x = clientX - row.getBoundingClientRect().left
    const under = (i: number) => bounds.visible[i] && x >= bounds.tabs[i].x0 && x < bounds.tabs[i].x1
    if (under(order.indexOf(id))) return null
    const j = order.findIndex((_, i) => under(i))
    return j < 0 ? null : order[j]
  }

  // 놓은 뒤(`drag` = null)에도 자리로 가는 동안 이웃 위에 그린다 — 다음 끌기까지 둔다(겹치지 않으면 차이가 없다).
  const lastDragged = useRef<string | null>(null)
  useLayoutEffect(() => {
    if (drag !== null) lastDragged.current = drag.view
  })
  const topId = drag?.view ?? lastDragged.current

  // 끄는 동안은 머리 위 맴돎을 보지 않는다 — 포인터 캡처가 줄에 걸려 머리의 들고 남이 끄는 손을 따르지 않는다.
  const [hovered, setHovered] = useState<string | null>(null)
  const activeId = drag !== null ? drag.view : active
  const lit = (id: string) => id === activeId || (drag === null && id === hovered)
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

  return (
    <div
      data-testid="tab-bar"
      data-window-label={label}
      style={{
        display: 'flex',
        alignItems: 'stretch',
        height: '28px',
        flexShrink: 0,
        // ★면 규칙 = 바는 내용 면보다 한 칸 눌린 면(`--tab-bar-bg`), 활성 탭은 내용과 같은 면(`--tab-active-bg`)★ —
        //   Chrome 처럼 활성 탭이 아래 캔버스로 이어진다(사용자 결정 2026-10-19 · 값은 테마마다 `styles/theme.css`).
        //   ☞ 활성 탭이 바보다 어두운 배치와 `--text` 를 섞어 바를 띄우는 안은 적용해 봤다가 되돌렸다 — 전자는 활성 탭이
        //   구멍처럼 읽혔고, 후자는 바가 트리 패널과 따로 노는 회색 띠가 됐다(사용자 판단 2026-08-23).
        background: 'var(--tab-bar-bg)',
        // 다크는 내용(#0a0a0a)이 거의 검정이라 바를 더 누를 여지가 없다 — 바와 내용의 경계는 이 선이 긋고, 활성 면 · 날개가
        //   덮은 자리만 끊긴다. 지우면 줄 높이가 1px 늘어 활성 면의 `bottom: -1` 도 함께 고친다.
        borderBottom: '1px solid var(--tab-bar-edge)',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
        userSelect: 'none',
      }}
    >
      <div
        ref={rowRef}
        data-testid="tab-strip-row"
        style={{ position: 'relative', flex: '1 1 0', minWidth: 0 }}
      >
        {domOrder.map(id => {
          const head = headById.get(id)!
          // 막 붙은 창은 끄는 탭을 모델 메아리보다 먼저 받는다 — 그때는 이름 없이 자리 · 폭만 그린다(TRD §5-1 「붙은 직후」).
          const tab = metaById.get(id) ?? { id, name: '' }
          const isActive = id === activeId
          const editing = editingId === id
          return (
            <div
              key={id}
              data-testid="tab"
              data-view-id={id}
              data-active={isActive ? 'true' : 'false'}
              // 누르는 순간 활성(TRD §2-2) — 훅이 누르기 전 활성(`initial_active`)을 먼저 적게 전환은 그 뒤에 부른다.
              //   편집 중인 머리의 누름은 끌기 후보가 아니고, 덮인 이웃 자리의 누름은 편집을 확정하고 그 이웃으로 전환만 한다.
              onPointerDown={e => {
                if (editing) {
                  const inInput = e.target instanceof Element && e.target.closest('input') !== null
                  const under = e.button === 0 && !inInput ? coveredHead(id, e.clientX) : null
                  if (under === null) return
                  commitEdit(tab)
                  activate(under, e.button)
                  return
                }
                onPointerDown(e, id)
                activate(id, e.button)
              }}
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
                color: isActive ? 'var(--text)' : 'var(--text-muted)',
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
                      if (e.key === 'Enter') commitEdit(tab)
                      else if (e.key === 'Escape') cancelEdit()
                    }}
                    onBlur={() => commitEdit(tab)}
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
                    style={{
                      flexShrink: 0,
                      background: 'transparent',
                      border: 'none',
                      color: 'var(--text-muted)',
                      cursor: 'pointer',
                      padding: '0 2px',
                      fontSize: '12px',
                      lineHeight: 1,
                    }}
                  >
                    ×
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
          style={{ ...ADD_STYLE, transform: `translateX(${end}px)`, transition: slide }}
        >
          +
        </button>
      </div>
    </div>
  )
}

// 가로 여백(margin)을 두지 말 것 — 쓸 수 있는 폭은 줄 폭 − `+` 테두리 상자 폭이다(`measure.ts`).
const ADD_STYLE: CSSProperties = {
  position: 'absolute',
  top: 0,
  bottom: 0,
  left: 0,
  background: 'transparent',
  border: 'none',
  color: 'var(--text-muted)',
  cursor: 'pointer',
  padding: '0 12px',
  fontSize: '14px',
}

// 활성 · 맴돎 면은 머리 위 이만큼 내려앉는다 — 그 위로 바가 보여야 탭이 바에서 솟은 조각으로 읽힌다(Chrome). 머리 상자는 줄
//   높이 그대로다.
const SURFACE_TOP = 4
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
  gap: '6px',
  padding: '0 12px',
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

// 구분선 = Chrome 그대로(PRD R27 · Chromium `tab_style.cc` `kSeparator*` · `horizontal_tab_style_views.cc`
//   `GetSeparatorBounds`) — 두께 2 · 높이 16 · 좌우 여백 2 · 둥글기 = 두께의 반 · 위아래 = 머리 높이 가운데. 여백 · 두께 ·
//   여백 칸을 이웃 머리와의 경계 가운데에 건다 — Chrome 은 그 칸을 탭끼리 겹친 자리(18)에 두는데 우리 머리는 겹치지 않는다
//   (`tabStrip/layout.ts` 머리).
const SEPARATOR_W = 2
const SEPARATOR_H = 16
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
