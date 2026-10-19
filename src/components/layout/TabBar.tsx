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
import { TAB_MIN_ACTIVE_W } from './tabStrip/layout'
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
  //   전환(놓은 뒤 자리로 가는 끄는 탭 · 이웃)이 끝까지 가고, 속성을 걷으면 그 자리에서 끊긴다. WebView2 에서 그대로인지
  //   미검 — 아니면 끌기가 끝나는 순간(`dock:drag{ended}`) 끄는 탭이 자리로 튄다.
  const slide = `transform ${drag?.slideMs ?? 0}ms ${EASE_OUT_CSS}`
  const heads: DrawnHead[] = []
  order.forEach((id, i) => {
    const x = drag === null ? (bounds.visible[i] ? bounds.tabs[i].x0 : null) : i === dragged ? drag.left : drag.targets[i]
    if (x === null) return
    const following = drag !== null && i === dragged && !drag.settling
    heads.push({ id, x, w: bounds.tabs[i].x1 - bounds.tabs[i].x0, transition: following ? 'none' : slide })
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

  // 놓은 뒤(`drag` = null)에도 자리로 가는 동안 이웃 위에 그린다 — 다음 끌기까지 둔다(겹치지 않으면 차이가 없다).
  const lastDragged = useRef<string | null>(null)
  useLayoutEffect(() => {
    if (drag !== null) lastDragged.current = drag.view
  })
  const topId = drag?.view ?? lastDragged.current

  return (
    <div
      data-testid="tab-bar"
      data-window-label={label}
      style={{
        display: 'flex',
        alignItems: 'stretch',
        height: '28px',
        flexShrink: 0,
        // ★규칙 = "활성 탭이 깨끗한 면(`--bg-secondary`), 바는 한 칸 눌린 면(`--bg`)"★ — Chrome 방식
        //   (사용자 지정 참조·결정 2026-08-23). 다크(#0a0a0a 바 < #141414 탭)·라이트(#f5f5f5 바 < #fff 탭)
        //   양쪽에서 **활성 탭이 더 밝다**로 일관되게 성립한다.
        //   ☞ 반대 배치(바=`--bg-secondary` · 활성=`--bg`)와 `--text` 를 섞어 바를 띄우는 안, 둘 다 실제로
        //   적용해 봤다가 되돌렸다 — 전자는 활성 탭이 프레임에서 가장 어두워 구멍처럼 읽혔고, 후자는
        //   바가 트리 패널과 따로 노는 회색 띠가 됐다(사용자 판단).
        //   ★e-ink 만 이 관계가 성립하지 않는다★ — 그 테마는 `--bg`(#ffffff) 가 `--bg-secondary`(#f0f0f0)
        //   보다 *밝아* 방향이 뒤집힌다. 거기선 글자색과 비활성 구분선이 활성 판정을 진다(미해소 잔여).
        background: 'var(--bg)',
        borderBottom: '1px solid var(--border)',
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
          const isActive = drag !== null ? id === drag.view : id === active
          const editing = editingId === id
          return (
            <div
              key={id}
              data-testid="tab"
              data-view-id={id}
              data-active={isActive ? 'true' : 'false'}
              // 누르는 순간 활성(TRD §2-2) — 훅이 누르기 전 활성(`initial_active`)을 먼저 적게 전환은 그 뒤에 부른다.
              onPointerDown={e => {
                if (editing) return
                onPointerDown(e, id)
                if (e.button === 0 && id !== active && metaById.has(id)) onSwitch(id)
              }}
              style={{
                position: 'absolute',
                top: 0,
                bottom: 0,
                left: 0,
                width: head.w,
                transform: `translateX(${head.x}px)`,
                transition: head.transition,
                zIndex: id === topId ? 1 : undefined,
                boxSizing: 'border-box',
                overflow: 'hidden',
                display: 'flex',
                alignItems: 'center',
                gap: '6px',
                padding: '0 12px',
                cursor: 'pointer',
                // 활성 탭 = 깨끗한 면(트리 패널과 같은 `--bg-secondary`). 근거·거부한 대안은 위 바 주석.
                //   ★accent 밑줄을 되살리지 말 것★ — 2px 파란 줄이 프레임에서 가장 튀어 "고대비"의
                //   주범이었다(사용자 판단 2026-08-23, Orca 대조).
                background: isActive ? 'var(--bg-secondary)' : 'var(--tab-inactive-bg)',
                color: isActive ? 'var(--text)' : 'var(--text-muted)',
                // Chrome 처럼 비활성 탭 사이에만 얇은 세로 구분선 — 활성 탭 양옆은 떠오른 조각이라 선이 없다.
                borderRight: isActive ? '1px solid transparent' : '1px solid var(--border)',
                borderTopLeftRadius: '6px',
                borderTopRightRadius: '6px',
                whiteSpace: 'nowrap',
              }}
            >
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
                  onDoubleClick={() => beginEdit(tab)}
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
                  onClick={() => onClose(id)}
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
