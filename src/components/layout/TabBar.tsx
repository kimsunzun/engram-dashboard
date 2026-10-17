// TabBar — 한 창(label)의 탭 목록 + 활성 탭 표시 + [+] 새 탭 + 탭별 닫기 + 이름 인라인 편집(ADR-0057, §7-2).
//
// ★§5 손발/두뇌 분리★: 사람 클릭(탭 전환/추가/닫기/이름변경)은 viewStore 액션(switchTab/createTab/closeTab/
// renameTab)만 부른다 — LLM(window.__engramCmd)이 같은 command 를 흔드는 것과 물리적으로
// 동일 표면. 실제 상태 변경은 백엔드 ViewManager(권위)가 하고 window:tabs-updated emit 으로 반영된다(낙관 갱신 X).
//
// ★탭 머리 = 도킹 끌기의 출처(TRD S21-slot-docking §1-4 「출처」)★ — HTML5 끌어 놓기라 문턱 · 캡처 · 창 밖 추적은
// 엔진 몫이다. `dragstart` 는 끌기 데이터 · 그림만 싣고, 셸 세션(`dock_drag_start`)은 그 사건 디스패치가 끝난 뒤
// 연다. 대상 쪽 · 세션 구독은 `dock/dockDrag.ts` 이고, 어느 머리를 접을지(떼어진 표시)는 부르는 쪽이 `detached` 로 준다.

import { useEffect, useRef, useState, type CSSProperties, type DragEvent } from 'react'
import { invoke } from '@tauri-apps/api/core'

import type { DragEffect } from '../../../src-tauri/bindings/DragEffect'
import type { PointCss } from '../../../src-tauri/bindings/PointCss'
import type { ViewMeta } from '../../api/layoutTypes'
import { t } from '../../i18n'
import { MAIN_WINDOW_LABEL } from '../../store/viewStore'
import { DOCK_MIME } from './dock/dockDrag'
import { newWindowSize, setDockDragImage } from './dock/dragImage'

interface TabBarProps {
  /** 이 탭바가 속한 창 label(main·slot-popup-N). */
  label: string
  tabs: ViewMeta[]
  /** 활성으로 칠할 탭 — 이 창 캔버스가 보이는 탭이다. `null` = 없음(떼어진 표시의 메인 마지막 탭). */
  active: string | null
  onSwitch: (viewId: string) => void
  onCreate: () => void
  onClose: (viewId: string) => void
  onRename: (viewId: string, name: string) => void
  /** 떼어진 표시로 접을 머리(이 창에서 끄는 탭 — TRD §4-1). 접힌 머리도 DOM 에 남는다(`dragend` 가 거기서 온다). */
  detached?: string | null
  /** 끌기 그림의 축소 그림 재료 — 그 탭이 지금 이 창에 보이는 탭이면 그 탭 캔버스 요소, 아니면 `null`(제목만). */
  dragCanvas?: (viewId: string) => HTMLElement | null
  /** 탭바가 가로로 스크롤됨 — 머리 사각형이 바뀐다(크롬 보고). */
  onScroll?: () => void
}

/** 머리 하나의 끌기 — `dragstart` 에서 만들고 `dragend` 에서 버린다. */
interface HeadDrag {
  id: string
  viewId: string
  hotspot: PointCss
  /** 원 사건 — 디스패치가 끝난 뒤 다른 리스너가 시작을 취소했나(`defaultPrevented`)를 본다. */
  native: Event
  /** 세션을 열 차례를 기다리는 시한. `null` = 이미 정했다(`started` 가 그 답). */
  timer: ReturnType<typeof setTimeout> | null
  started: boolean
}

// ★디스패치가 끝난 뒤에만, 취소되지 않았을 때만 연다(TRD §1-4 · 5.1 F2)★ — 뒤 리스너가 `dragstart` 를 취소하면 OS
//   끌기가 없어 끝 신호도 오지 않는다. 그때 세션을 열면 떼어진 탭 · 표식 · 숨긴 팝아웃이 `pointer_seen` 까지 남는다.
//   ★마이크로태스크로 바꾸지 말 것★ — 리스너 하나가 끝날 때마다 돌아 뒤 리스너보다 먼저다. 시한(task 하나 뒤)이어야
//   엔진이 접은 끌기의 끝(같은 task 안 · `onHeadDragEnd`)도 이보다 먼저 와 걸러진다.
function openSession(drag: HeadDrag): void {
  drag.timer = null
  if (drag.native.defaultPrevented) return
  drag.started = true
  invoke<void>('dock_drag_start', { drag_id: drag.id, view_id: drag.viewId, hotspot: drag.hotspot }).catch(
    err => console.warn('[TabBar] dock_drag_start 거절 — 이 끌기의 놓기는 아무것도 바꾸지 않는다:', err),
  )
}

function onControl(head: Element, target: EventTarget | null): boolean {
  const el = target instanceof Element ? target : target instanceof Node ? target.parentElement : null
  const control = el?.closest('input, button')
  return control != null && head.contains(control)
}

function effectOf(dropEffect: string | undefined): DragEffect {
  if (dropEffect === 'move') return 'move'
  if (dropEffect === 'copy' || dropEffect === 'link') return 'other'
  return 'none'
}

export default function TabBar({
  label,
  tabs,
  active,
  onSwitch,
  onCreate,
  onClose,
  onRename,
  detached = null,
  dragCanvas,
  onScroll,
}: TabBarProps) {
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
  //   (두 이벤트가 서로 다른 렌더 사이클·다른 클로저라 editingId 최신값을 본다.)
  const commitEdit = (tab: ViewMeta) => {
    if (editingId !== tab.id) return
    const trimmed = draft.trim()
    if (trimmed.length > 0 && trimmed !== tab.name) onRename(tab.id, trimmed)
    setEditingId(null)
  }

  // ★`dragstart` 안에서 끌린 머리를 누른 자리 밑에서 빼지 않는다★ — Chromium 은 `dragstart` 처리 뒤 누른 자리를 다시
  //   짚어 그 머리가 거기 없으면(폭 0 · `visibility: hidden` · 앞에 형제가 끼어 밀림) OS 끌기를 열지 않고 곧바로
  //   `dragend(none)` 을 낸다(실측 15/15). 그 밖의 DOM 변경은 끌기를 끝내지 않는다(실측 25/25). 그래서 머리 접기는
  //   `Started` 뒤이고, 끌기 기록은 다시 그리기를 부르지 않는 ref 다.
  const dragRef = useRef<HeadDrag | null>(null)

  // 마지막 누름 자리 — 엔진은 `dragstart` 를 누른 요소가 아니라 그 위의 `draggable` 조상(머리)에 낸다. 닫기 버튼에서
  //   누른 끌기도 사건 대상은 머리라, 누른 자리를 따로 기억해야 가른다.
  const pressRef = useRef<EventTarget | null>(null)

  const onHeadDragStart = (e: DragEvent<HTMLDivElement>, tab: ViewMeta) => {
    const pressed = pressRef.current
    pressRef.current = null
    const head = e.currentTarget
    const dt = e.dataTransfer
    // 이름 입력 · 닫기 버튼에서 시작한 끌기는 탭 끌기가 아니다(글자 끌기 · 버튼 누름). 우리 MIME 이 아니라 끊지 않는다.
    if (editingId === tab.id || onControl(head, e.target) || onControl(head, pressed) || !dt) {
      e.preventDefault()
      return
    }
    // ★우리 MIME 사건이 window 에 닿지 않게 끊는다(TRD §1-4 「다른 끌기 리스너」)★ — 에이전트 트리의 react-dnd 가 뿌리
    //   `window` 버블 리스너로 자기 원천도 브라우저 기본형도 아닌 `dragstart` 를 취소한다. React 는 뿌리 컨테이너에서
    //   처리기를 부르므로 원 사건이 거기서 멈춘다.
    e.stopPropagation()
    const dragId = crypto.randomUUID()
    // ★이 형식 하나만 싣는다★ — `text/plain` · URL 을 실으면 다른 앱이 받아 `dropEffect` 가 none 이 아니게 되고 그 앱에
    //   글자가 들어간다.
    dt.clearData()
    dt.setData(DOCK_MIME, JSON.stringify({ drag_id: dragId, view_id: tab.id }))
    dt.effectAllowed = 'move'
    const box = head.getBoundingClientRect()
    const image = setDockDragImage(
      dt,
      {
        title: tab.name,
        newWindow: newWindowSize(label === MAIN_WINDOW_LABEL, { w: window.innerWidth, h: window.innerHeight }),
        canvas: dragCanvas?.(tab.id) ?? null,
      },
      { x: e.clientX - box.left, y: e.clientY - box.top },
    )
    const prev = dragRef.current
    if (prev !== null && prev.timer !== null) clearTimeout(prev.timer)
    const drag: HeadDrag = {
      id: dragId,
      viewId: tab.id,
      hotspot: image.hotspot,
      native: e.nativeEvent,
      timer: null,
      started: false,
    }
    drag.timer = setTimeout(() => openSession(drag), 0)
    dragRef.current = drag
  }

  // 끊지 않는다 — window 쪽 react-dnd 는 캡처 리스너 하나뿐이고 자기 끌기가 아니면 아무것도 안 한다(TRD §1-4).
  const onHeadDragEnd = (e: DragEvent<HTMLDivElement>) => {
    const drag = dragRef.current
    if (drag === null) return
    dragRef.current = null
    // ★시작 시한 전에 온 끝 = 엔진이 접은 끌기 — 세션을 열지도 끝내지도 않는다★: Chromium(`MouseEventManager::
    //   TryStartDrag` 「Drag was canned at the last minute」)은 OS 끌기를 열지 못하면 `dragstart` 와 같은 task 안에서
    //   `dragend(none)` 을 낸다(실측 = `dragstart` 뒤 0–1 ms). 여기서 시작 · 끝을 보내면 셸에는 영역 보고 없는 「마지막 점에
    //   놓기」로 보여 새 창이 뜬다(실측 15/15). 실제 OS 끌기의 끝은 이 시한보다 늦다(가장 짧은 실측 = `dragstart` 뒤 +28 ms).
    if (drag.timer !== null) {
      clearTimeout(drag.timer)
      drag.timer = null
      return
    }
    if (!drag.started) return
    invoke<void>('dock_drag_end', { drag_id: drag.id, effect: effectOf(e.dataTransfer?.dropEffect) }).catch(err =>
      console.warn('[TabBar] dock_drag_end 실패:', err),
    )
  }

  return (
    <div
      data-testid="tab-bar"
      data-window-label={label}
      onScroll={onScroll}
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
        overflowX: 'auto',
      }}
    >
      {tabs.map(tab => {
        const isActive = tab.id === active
        const collapsed = tab.id === detached
        return (
          <div
            key={tab.id}
            data-testid="tab"
            data-view-id={tab.id}
            data-active={isActive ? 'true' : 'false'}
            data-dock-detached={collapsed ? 'true' : undefined}
            // 이름 편집 중인 머리는 끌지 않는다 — 입력 안에서 누른 채 움직이면 글자 고르기가 아니라 머리 끌기가 된다.
            draggable={editingId !== tab.id}
            onPointerDown={e => {
              pressRef.current = e.target
            }}
            onDragStart={e => onHeadDragStart(e, tab)}
            onDragEnd={onHeadDragEnd}
            onClick={() => onSwitch(tab.id)}
            style={{
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
              ...(collapsed ? COLLAPSED_HEAD : null),
            }}
          >
            {editingId === tab.id ? (
              <input
                data-testid="tab-rename-input"
                data-view-id={tab.id}
                value={draft}
                autoFocus
                // 여기 인라인 콜백 ref 로 select() 하면 매 렌더 재실행돼 타이핑이 깨진다(FIX 1).
                ref={inputRef}
                onChange={e => setDraft(e.target.value)}
                onKeyDown={e => {
                  // ★버블 차단★: 편집 중 키가 부모 탭(onClick=switch)·전역 키바인딩으로 새면 안 된다.
                  e.stopPropagation()
                  if (e.key === 'Enter') commitEdit(tab)
                  else if (e.key === 'Escape') cancelEdit()
                }}
                onBlur={() => commitEdit(tab)}
                // 입력 클릭/더블클릭이 부모 탭 onClick(switch)·onDoubleClick(편집 진입)으로 버블 금지.
                onClick={e => e.stopPropagation()}
                onDoubleClick={e => e.stopPropagation()}
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
                // ★단일 클릭 = 탭 전환(부모 onClick 로 버블), 더블클릭 = 인라인 편집 진입(§5 사람 UI 경로).★
                //   더블클릭 제스처는 dblclick 전에 click 을 2번 쏜다 — 그 클릭들이 부모 onClick(switch)로
                //   새면 편집 진입 전에 탭이 엉뚱하게 전환된다. e.detail>=2(더블클릭을 완성하는 두 번째 클릭)만
                //   stopPropagation 으로 삼켜 재전환을 막고, 진짜 단일 클릭(e.detail===1)은 그대로 전환시킨다.
                onClick={e => {
                  if (e.detail >= 2) e.stopPropagation()
                }}
                onDoubleClick={e => {
                  e.stopPropagation()
                  beginEdit(tab)
                }}
                style={{ overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: '160px' }}
              >
                {tab.name}
              </span>
            )}
            <button
              type="button"
              aria-label={t('tab.close', { name: tab.name })}
              data-testid="tab-close"
              // 부모 onClick(전환)로 버블 금지 → stopPropagation.
              onClick={e => {
                e.stopPropagation()
                onClose(tab.id)
              }}
              style={{
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
          </div>
        )
      })}
      <button
        type="button"
        aria-label={t('tab.create')}
        data-testid="tab-add"
        onClick={onCreate}
        style={{
          background: 'transparent',
          border: 'none',
          color: 'var(--text-muted)',
          cursor: 'pointer',
          padding: '0 12px',
          fontSize: '14px',
        }}
      >
        +
      </button>
    </div>
  )
}

// 떼어진 표시의 머리 접기 — 폭 0 으로 접고 DOM 에는 남긴다. `Started` 뒤 이렇게 접힌 머리도 `dragend` 를 받는다(실측
//   306/307 · 나머지 1 은 접히기 전에 끝난 끌기). 요소를 빼거나 `display: none` 으로 접는 것은 재지 않았다.
const COLLAPSED_HEAD: CSSProperties = {
  width: 0,
  minWidth: 0,
  padding: 0,
  borderRight: 'none',
  overflow: 'hidden',
  visibility: 'hidden',
}
