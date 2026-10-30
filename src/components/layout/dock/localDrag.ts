// TRD S21-slot-docking §4-2 · §4-3: 웹뷰(JS 맥락)마다 하나인 탭 끌기의 훅 사이 상태 — 끌기 출처(`tabStrip/useTabDragSource`)와
//   뿌리 받기(`dock/useWindowDrop`)가 **같은 웹뷰 안에서** 주고받는 것만 담는다. 창 사이로는 아무것도 건너지 않는다 — 다른 창이
//   아는 것은 끌기 자료의 타입 문자열뿐이다(ADR-0296 · TRD §2-2).
// 칸마다 쓰는 쪽이 하나다: `outgoing` = 출처(dragstart 에 세우고 바꿔 그리기가 풀리면 걷는다) · `sourceStrip` = 출처 창의 뿌리
//   dragover(첫 값만 출처 dragstart 가 둔다 — 아래 `setSourceStrip`) · `targetMarker` = 다른 창의 뿌리 dragover(그리기는 `TabBar`) ·
//   제자리 놓기 표시 = 뿌리 drop 이 적고 출처 dragend 가 가져간다 · 탭바 기하 = `TabBar` 가 등록하고 뿌리가 읽는다.
// ★끌기는 LLM 명령이 아니다★(TRD §3-6) — 사람 손의 입력과 그 측정이라 전역 핸들 · 버스에 올리지 않는다.

import { useSyncExternalStore } from 'react'

import type { StripBounds } from '../tabStrip/reorderGeometry'
import type { RectCss } from './overlayTypes'

/** 이 웹뷰에서 나간 끌기 — 출처 창에만 있다. */
export interface OutgoingDrag {
  drag: string
  /** 출처 창 label. */
  window: string
  /** 끄는 탭. */
  view: string
  /** `JudgeStrip.source.grabOffset` — dragstart 에 한 번 셈한 값(`reorderGeometry.grabOffset`). */
  grabOffset: number
}

/**
 * 출처 창 자기 탭바 위 판정(`judge` 의 `strip` 답) — 탭바 좌표(0 = `StripGeometry.left`). `index` = 끄는 탭을 뺀 목록의 자리 ·
 * `left` = 끄는 머리 왼쪽(`draggedLeft`) · `x` = 그 dragover 의 커서 x(순서 문턱 `moveDue` 의 재료 — `left` 는 끌기 영역 끝에서
 * 잘려 커서로 되돌릴 수 없다).
 */
export interface SourceStripMove {
  index: number
  left: number
  x: number
}

/**
 * 이 웹뷰 탭바의 판정 재료 — `TabBar` 가 지금 그리는 순서 그대로다(끄는 동안 = 확정한 순서 이동을 실은 순서). 좌표 = 뷰포트
 * CSS px. 출처 창 자기 탭바에 놓을 자리는 `tabs.indexOf(끄는 탭)` 이다(확정한 자리 — `judge` 의 날 답이 아니다).
 */
export interface StripGeometry {
  /** 탭바 좌표 0 = 탭 끌기 영역 왼쪽(첫 탭 왼쪽)의 뷰포트 x. */
  left: number
  /** 지금 그리는 순서의 이상적 경계(끄는 탭 포함 · 끄는 탭은 숨김이어도 보이는 탭으로 센다). */
  bounds: StripBounds
  /** `dragAreaWidth` 의 `available` — 잰 값(`null` = 재지 못함). */
  available: number | null
  /**
   * 탭바 구역 = 바탕 사각형(첫 탭 왼쪽 ~ `+` 오른 끝 × 제목 줄 안의 탭바면 제목 줄 · 아니면 탭바 줄 — `tabStrip/measure.ts`
   * `stripZoneArea`)의 위아래 `STRIP_ZONE_PX`(`bands.stripZone`) — 뷰포트 좌표.
   */
  zoneViewport: RectCss
  /** `bounds.tabs` 순서의 탭 id. */
  tabs: readonly string[]
}

/** 구독자가 읽는 칸 — 칸이 바뀔 때마다 새 객체다. */
export interface LocalDragSnapshot {
  outgoing: OutgoingDrag | null
  sourceStrip: SourceStripMove | null
  targetMarker: number | null
}

export interface LocalDrag {
  getOutgoing(): OutgoingDrag | null
  setOutgoing(d: OutgoingDrag | null): void
  /**
   * 출처 창 뿌리 dragover 마다 그 판정을 적는다 — 자기 탭바 구역이면 `strip` 답 · 그 밖(창을 떠난 dragleave 포함)이면 `null`.
   * 출처 dragstart 가 「누른 점 = 구역 안」인 첫 값을 둔다 — 그래서 `null` 은 늘 「구역을 떠남」이다.
   */
  setSourceStrip(m: SourceStripMove | null): void
  getSourceStrip(): SourceStripMove | null
  /** 다른 창 탭바의 2 px 세로선 x(탭바 좌표 · `dropMarkerX`) — `null` = 없음. */
  setTargetMarker(x: number | null): void
  getTargetMarker(): number | null
  /** 뿌리 drop 이 아무것도 보내지 않은 같은 창 같은 자리 놓기(TRD §2-4 ④) — 출처 dragend 가 `takeLocalDrop` 으로 가져간다. */
  markLocalDrop(drag: string): void
  /** 그 끌기에 표시가 있었으면 지우고 `true`(한 번만). */
  takeLocalDrop(drag: string): boolean
  /** 반환 = 떼기 — 마지막에 등록한 것이 읽힌다. */
  registerStripGeometry(fn: () => StripGeometry | null): () => void
  /** 탭바가 없으면 `null`. */
  readStripGeometry(): StripGeometry | null
  /** 칸이 바뀔 때마다(같은 값을 다시 적으면 부르지 않는다). 반환 = 떼기. */
  subscribe(cb: () => void): () => void
  getSnapshot(): LocalDragSnapshot
}

export function createLocalDrag(): LocalDrag {
  let snap: LocalDragSnapshot = { outgoing: null, sourceStrip: null, targetMarker: null }
  let localDrop: string | null = null
  const geometries: Array<{ fn: () => StripGeometry | null }> = []
  const listeners = new Set<() => void>()

  const write = <K extends keyof LocalDragSnapshot>(key: K, value: LocalDragSnapshot[K]): void => {
    if (Object.is(snap[key], value)) return
    snap = { ...snap, [key]: value }
    for (const cb of [...listeners]) cb()
  }

  return {
    getOutgoing: () => snap.outgoing,
    setOutgoing: d => write('outgoing', d),
    setSourceStrip: m => write('sourceStrip', m),
    getSourceStrip: () => snap.sourceStrip,
    setTargetMarker: x => write('targetMarker', x),
    getTargetMarker: () => snap.targetMarker,
    markLocalDrop: drag => {
      localDrop = drag
    },
    takeLocalDrop: drag => {
      if (localDrop !== drag) return false
      localDrop = null
      return true
    },
    registerStripGeometry(fn) {
      const entry = { fn }
      geometries.push(entry)
      return () => {
        const at = geometries.indexOf(entry)
        if (at >= 0) geometries.splice(at, 1)
      }
    },
    readStripGeometry: () => geometries[geometries.length - 1]?.fn() ?? null,
    subscribe(cb) {
      const entry = (): void => cb()
      listeners.add(entry)
      return () => {
        listeners.delete(entry)
      }
    },
    getSnapshot: () => snap,
  }
}

/** 웹뷰마다 하나. */
export const localDrag: LocalDrag = createLocalDrag()

/** `select` 는 칸을 그대로 꺼내야 한다 — 새 객체를 만들면 그릴 때마다 바뀐 값으로 읽힌다. */
export function useLocalDrag<T>(select: (s: LocalDragSnapshot) => T, store: LocalDrag = localDrag): T {
  return useSyncExternalStore(store.subscribe, () => select(store.getSnapshot()))
}
