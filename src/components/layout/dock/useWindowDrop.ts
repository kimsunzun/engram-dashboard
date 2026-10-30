// TRD S21-slot-docking §4-3: 창 뿌리의 끌기 받기 — 이 문서의 dragenter · dragover · dragleave · drop 과 mousemove 를
//   `windowDrop`(정하는 곳)에 잇고 그 안내를 돌려준다. 바깥(스토어 · 탭바 기하 · Tauri)을 포트로 채우는 것도 여기다.
// ★끌기 리스너는 `document` 의 거품 단계다★ — 에이전트 트리(react-arborist → react-dnd HTML5 백엔드)가 `window` 거품 리스너로
//   자기 것이 아닌 dragover 의 `dropEffect` 를 `none` 으로 덮는다(TRD §4-2). 우리 끌기는 `document` 에서 끊어 거기 닿지 않게 한다 —
//   `window` 에 달면 같은 노드의 등록 순서가 이겨 끊기가 서지 않고, `document` 캡처에서 끊으면 아래 요소의 처리기까지 못 받는다(옛
//   HTML5 끌기 `dockDrag.ts` 의 같은 조항). 파일 · URL 놓기 막기(`fileDropGuard`)는 따로 그대로 산다 — 둘은 서로의 끌기를 보지 않는다.
// ★mousemove 는 `window` 캡처다★ — 안내 층은 포인터를 받지 않아 「덮개 위 이동」은 그 밑 요소에 온다 · 앱 처리기가 전파를 끊어도 본다.

import { useEffect, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from 'react'

import type { ViewMeta } from '../../../api/layoutTypes'
import { useViewStore } from '../../../store/viewStore'
import { askAppIdentifier, currentAppIdentifier, dragSource } from '../tabStrip/useTabDragSource'
import { shownTab } from './dragOverride'
import { localDrag } from './localDrag'
import type { OverlayView } from './overlayTypes'
import { tabDropClient } from './tabDrop'
import { createWindowDrop, type DropModel, type WindowDrop, type WindowDropPorts } from './windowDrop'

const MOVE_LISTEN: AddEventListenerOptions = { capture: true, passive: true }

export interface UseWindowDropOptions {
  label: string
  /** 탭 캔버스 영역 — 안내 층이 덮는 그 상자. */
  canvas: () => Element | null
}

/** 이 창에 지금 그릴 끌기 안내 — `DockOverlayView` 의 `guide`. */
// ADR-0296
export function useWindowDrop(o: UseWindowDropOptions): OverlayView | null {
  const live = useRef(o)
  useLayoutEffect(() => {
    live.current = o
  })
  const ctl = useMemo(() => createWindowDrop(portsOf(live)), [])

  useEffect(() => {
    askAppIdentifier()
    const off = attach(ctl)
    return () => {
      off()
      ctl.reset()
    }
  }, [ctl])

  return useSyncExternalStore(ctl.subscribe, ctl.getGuide)
}

function attach(ctl: WindowDrop): () => void {
  const enter = (e: DragEvent): void => ctl.dragEnter(e)
  const over = (e: DragEvent): void => ctl.dragOver(e)
  const leave = (e: DragEvent): void => ctl.dragLeave(e)
  const drop = (e: DragEvent): void => ctl.drop(e)
  const move = (e: MouseEvent): void => ctl.mouseMove(e.buttons)
  document.addEventListener('dragenter', enter)
  document.addEventListener('dragover', over)
  document.addEventListener('dragleave', leave)
  document.addEventListener('drop', drop)
  window.addEventListener('mousemove', move, MOVE_LISTEN)
  return () => {
    document.removeEventListener('dragenter', enter)
    document.removeEventListener('dragover', over)
    document.removeEventListener('dragleave', leave)
    document.removeEventListener('drop', drop)
    window.removeEventListener('mousemove', move, MOVE_LISTEN)
  }
}

function portsOf(live: { readonly current: UseWindowDropOptions }): WindowDropPorts {
  // 스토어는 적용마다 탭 목록을 새 배열로 둔다 — 같은 배열이면 id 목록도 같은 배열을 돌려준다.
  let ids: { from: ViewMeta[]; tabs: string[] } | null = null
  const model = (): DropModel => {
    const w = useViewStore.getState().windows[live.current.label]
    if (w === undefined) return { tabs: [], active: null }
    if (ids === null || ids.from !== w.tabs) ids = { from: w.tabs, tabs: w.tabs.map(t => t.id) }
    return { tabs: ids.tabs, active: w.active }
  }
  return {
    app: currentAppIdentifier,
    local: localDrag,
    canvasRect: () => live.current.canvas()?.getBoundingClientRect() ?? null,
    // `WindowLayout` 이 그리는 그 탭(`useShownTab` 과 같은 셈).
    shownTab: () => {
      const m = model()
      const s = dragSource().getSnapshot()
      return s.window === live.current.label ? shownTab(s.override, m) : m.active
    },
    view: id => useViewStore.getState().layouts[id] ?? null,
    model,
    send: drop => tabDropClient.send(drop),
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: h => clearTimeout(h as ReturnType<typeof setTimeout>),
    watchCanvas: onResize => {
      const el = live.current.canvas()
      // jsdom(vitest 에 setupFiles 없음)처럼 RO 가 없는 환경에선 지켜보지 않는다.
      if (el === null || typeof ResizeObserver === 'undefined') return () => {}
      const ro = new ResizeObserver(() => onResize())
      ro.observe(el)
      return () => ro.disconnect()
    },
  }
}
