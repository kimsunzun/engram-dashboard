// TRD S21-slot-docking §1-4: 한 창(웹뷰)의 끌기 대상 쪽과 세션 구독 — 우리 MIME 끌기를 받아 판정하고(`pickTarget`) 그린
//   것을 셸 장부에 알리며(`dock_drag_over` · `dock_drop`), `dock:session` · `dock:regions` 를 `drag_id` 별로 접어 떼어진
//   표시 · 가이드 상태를 낸다. 끌기 출처(탭 머리 `dragstart` · `dragend`)는 `TabBar` 몫이라 여기 없다.
//
// ★결정은 셸이 한다★ — 여기는 「무엇을 그렸나」만 보고하고, 놓기 판정 · 영역 계산 · 끝 판정은 장부(`dock/book.rs`)가
//   한다. 그래서 보고는 바뀔 때만 낸다(움직일 때마다가 아니다): 그린 것(`shown`) · 쓴 영역 번호 · 보이던 탭.
// ★창당 하나★ — 상태가 모듈 범위인 것은 `windowCanvasReport.ts` 와 같은 까닭이다(같은 웹뷰에서 컴포넌트 인스턴스가
//   바뀌어도 리스너 · 세션 기억은 하나). 진입점 = `mountDockDrag`(리스너 달기) · `useDockSession` · `useDockGuide` ·
//   `useDisplayedTab`. 시험은 `createDockDrag` 로 가짜 IPC 를 꽂는다(ADR-0012).
// ★끌기는 LLM 명령이 아니다★(TRD §3) — 사람 손의 입력과 그 측정이라 레지스트리 · 전역 핸들을 두지 않는다.

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useSyncExternalStore } from 'react'

import type { DockPullReply } from '../../../../src-tauri/bindings/DockPullReply'
import type { DockRegions } from '../../../../src-tauri/bindings/DockRegions'
import type { DragOverReport } from '../../../../src-tauri/bindings/DragOverReport'
import type { DropReport } from '../../../../src-tauri/bindings/DropReport'
import type { Picked } from '../../../../src-tauri/bindings/Picked'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SessionEvent } from '../../../../src-tauri/bindings/SessionEvent'
import type { Started } from '../../../../src-tauri/bindings/Started'
import { useViewStore } from '../../../store/viewStore'
import { retryAsync, RetryCancelledError } from '../../../util/retryInvoke'
import { pickTarget } from './dockPick'

/** OS 끌기 데이터의 우리 형식 — 탭 머리 `dragstart` 가 이것 하나만 싣는다(TRD §1-4 「출처」). */
export const DOCK_MIME = 'application/x-engram-tab'

/** 보이지 않는 탭 머리 위에 이만큼 머물면 그 탭으로 전환한다(PRD R6b). */
export const HOVER_SWITCH_MS = 500

/** 세션이 끝난 뒤 · 그 창의 끝 증거 뒤 Esc 를 더 삼키는 시간(TRD §1-4 「Esc 삼키기」). */
export const ESC_SWALLOW_TAIL_MS = 200

export const EVT_DOCK_SESSION = 'dock:session'
export const EVT_DOCK_REGIONS = 'dock:regions'

/** 지금 세션 — `dock:session` 의 `started` · `display` 칸에 그 `drag_id` 를 더한 것. */
export interface DockSession extends Started {
  drag_id: string
}

/** 이 창에 그릴 가이드 — `DockOverlay` 의 입력. 모든 사각형은 영역(`regions`) 그대로다. */
export interface DockGuide {
  regions: DockRegions
  /** 그린 판정 — 셸에 보고한 `shown` 과 같다. */
  shown: Picked
  /** 캔버스 몫(바깥 띠 · 칸 띠)을 그리나 — 이 창이 보이던 탭이 영역의 `view_id` 일 때만(TRD §1-4 · 5.1 C1). 탭바 몫은 이것과 무관하다. */
  canvas: boolean
  /** 커서 밑 칸 — `canvas` 일 때만 값이 있다. */
  cursorSlot: string | null
  /** 전환 대기 중인 탭 머리. `seq` 는 대기마다 바뀐다 — 진행 표시를 새로 시작하는 열쇠. */
  hover: { viewId: string; seq: number } | null
}

/** Tauri 와의 접점. `target` = 그 label 로 지목된 emit 만 받는다 · `null` = 전체 emit. */
export interface DockIpc {
  listen(event: string, handler: (payload: unknown) => void, target: string | null): Promise<() => void>
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>
}

export interface DockDragDeps {
  ipc: DockIpc
  pick: (regions: DockRegions, point: PointCss, scroll: number | null, sourceView: string) => Picked
  /** 그 창의 모델 활성 탭 — 탭 상태를 아직 모르면 `null`. */
  activeTab: (label: string) => string | null
  /** 그 창의 모델 활성 탭이 바뀔 때 부른다. 반환 = 해제. */
  subscribeActive: (label: string, cb: () => void) => () => void
  /** R6b 탭 전환 — 사람 클릭과 같은 길(`viewStore.switchTab`). */
  switchTab: (label: string, viewId: string) => void
}

export interface MountOptions {
  /**
   * 이 창 캔버스가 지금 보이는 탭(TRD §1-4 `displayed`). 기본은 `displayedTabOf(모델 활성, label, 지금 세션)` 이다 —
   * 화면이 다른 규칙으로 탭을 보이면 같은 값을 여기 넘긴다(보고 · 그리기가 화면과 갈라지면 놓기가 무변경이 된다).
   */
  displayed?: () => string | null
}

export interface DockDragController {
  /** 그 창의 리스너를 단다. 반환 = 떼기(여러 번 불러도 된다). 마지막 떼기가 세션 기억까지 비운다. */
  mount(label: string, options?: MountOptions): () => void
  getSession(): DockSession | null
  getGuide(): DockGuide | null
  subscribe(cb: () => void): () => void
}

/**
 * 창 `label` 의 캔버스가 보일 탭 — 그 창이 끄는 탭의 출처이고 모델 활성이 그 탭이면 떼어진 자리의 승계 탭
 * (`display_active` · `null` = 빈 캔버스 — 메인 마지막 탭), 아니면 모델 활성 그대로(TRD §4-1).
 * 화면(`WindowLayout`)과 보고가 이 함수 하나를 함께 써야 「보이던 탭」이 갈라지지 않는다.
 */
export function displayedTabOf(
  active: string | null,
  label: string,
  session: DockSession | null,
): string | null {
  if (session !== null && session.source_label === label && active === session.source_view) {
    return session.display_active
  }
  return active
}

const NONE: Picked = { type: 'none' }

/** 기억하는 `drag_id` 수 — 끝난 id 는 늦은 사건을 버리려고 남긴다(셸 장부의 `ended` 넷보다 넉넉히). */
const MAX_RECORDS = 16

interface DragRecord {
  id: string
  /** 받아들인 가장 큰 세션 `seq`. `-1` = 세션 사건을 못 받음(영역 · 당기기만) — 어느 진짜 사건이든 이긴다. */
  seq: number
  session: Started | null
  regions: DockRegions | null
  ended: boolean
  /** 처음 안 순서 — 「지금 끌기」 = 끝나지 않은 것 중 가장 늦게 안 것(TRD §1-4 「받기」). */
  learned: number
  pointerSeen: boolean
  escStopped: boolean
}

/** 마지막으로 그린 것 — 놓기가 이것을 싣는다(`shown = 마지막으로 그린 것` · TRD §1-4 「drop」). */
interface Drawn {
  dragId: string | null
  rev: number
  barRev: number
  displayed: string | null
  shown: Picked
  point: PointCss
}

function pickedKey(p: Picked): string {
  switch (p.type) {
    case 'none':
      return 'n'
    case 'seam':
      return `s:${p.anchor ?? ''}:${p.side}`
    case 'tab_bar':
      return `t:${p.index}:${p.over ?? ''}`
    case 'reorder':
      return `r:${p.index}`
  }
}

function isOurs(e: DragEvent): boolean {
  const types = e.dataTransfer?.types
  return types !== undefined && Array.from(types).includes(DOCK_MIME)
}

// TRD §1-4 「늘 — 파일 끌어 놓기 막기」 · PRD AC22: 드롭 처리기를 끈 창에서 웹뷰가 놓기로 탐색하는 것은 파일 · URL 을
//   실은 끌기뿐이다(Chromium `DragData::ContainsURL`). ★그 둘만 막는다★ — 다른 끌기까지 막으면 입력창(챗 작성란 등)에
//   글자를 끌어 놓는 기본 동작이 거절된다.
function navigates(e: DragEvent): boolean {
  const types = e.dataTransfer?.types
  if (types === undefined) return false
  const list = Array.from(types)
  return list.includes('Files') || list.includes('text/uri-list')
}

function slotAt(r: DockRegions, p: PointCss): string | null {
  const x = p.x - r.canvas_origin.x
  const y = p.y - r.canvas_origin.y
  const hit = r.slots.find(s => x >= s.frame.x0 && x < s.frame.x1 && y >= s.frame.y0 && y < s.frame.y1)
  return hit?.slot_id ?? null
}

function asSessionEvent(raw: unknown): SessionEvent | null {
  if (raw === null || typeof raw !== 'object') return null
  const v = raw as Record<string, unknown>
  const phase = v.phase as Record<string, unknown> | null | undefined
  if (typeof v.drag_id !== 'string' || typeof v.seq !== 'number') return null
  if (phase === null || typeof phase !== 'object' || typeof phase.type !== 'string') return null
  if (phase.type === 'ended') return raw as SessionEvent
  if (phase.type !== 'started' && phase.type !== 'display') return null
  if (typeof phase.source_label !== 'string' || typeof phase.source_view !== 'string') return null
  return raw as SessionEvent
}

function asRegions(raw: unknown): DockRegions | null {
  if (raw === null || typeof raw !== 'object') return null
  const v = raw as Record<string, unknown>
  if (typeof v.drag_id !== 'string' || typeof v.rev !== 'number' || typeof v.bar_rev !== 'number') return null
  if (!Array.isArray(v.slots) || !Array.isArray(v.outer)) return null
  return raw as DockRegions
}

function parseDragData(raw: string): { drag_id: string; view_id: string } | null {
  try {
    const v = JSON.parse(raw) as Record<string, unknown>
    if (typeof v.drag_id === 'string' && typeof v.view_id === 'string') {
      return { drag_id: v.drag_id, view_id: v.view_id }
    }
  } catch {
    // 아래 null — 형식이 다른 끌기 데이터다.
  }
  return null
}

export function createDockDrag(deps: DockDragDeps): DockDragController {
  const { ipc } = deps
  const records = new Map<string, DragRecord>()
  let learnedSeq = 0

  let label = ''
  let displayedOverride: (() => string | null) | undefined
  let mounts = 0
  // 붙일 때마다 오른다 — 뗀 뒤에 풀린 `listen` 등록 · 당기기 답 · 시한을 버리는 표식.
  let gen = 0
  let handles: Array<() => void> = []
  let unsubActive: (() => void) | null = null

  let inside = false
  let lastPoint: PointCss | null = null
  let drawn: Drawn | null = null
  let lastSentKey: string | null = null
  let pulling = false
  let hover: { viewId: string; seq: number; timer: ReturnType<typeof setTimeout> } | null = null
  let hoverFired: string | null = null
  let hoverSeq = 0
  let escTailFor: string | null = null
  let escTailTimer: ReturnType<typeof setTimeout> | null = null
  const evidenceTimers = new Set<ReturnType<typeof setTimeout>>()
  // ★같은 창의 Over · Left 는 낸 순서대로 셸에 닿게 한 줄로 보낸다★ — 장부는 창별 「마지막 Over 뒤 Left 가 왔나」를
  //   도착 순서로 가른다(TRD §1-3 「보인 것 보고」). 겹친 invoke 의 도착 순서는 문서에 없다(`windowCanvasReport.ts`).
  //   대가: 응답이 끝내 안 오면 그 창의 보고는 거기서 멈춘다(놓기 `dock_drop` 은 이 줄을 타지 않는다).
  let overChain: Promise<void> = Promise.resolve()

  let sessionSnap: DockSession | null = null
  let guideSnap: DockGuide | null = null
  let notifiedSession: DockSession | null = null
  let notifiedGuide: DockGuide | null = null
  const listeners = new Set<{ cb: () => void }>()

  // 스냅숏은 바뀔 때만 새 객체라 참조 비교로 충분하다 — `dragover` 마다 구독자를 깨우지 않는다.
  function notify(): void {
    if (sessionSnap === notifiedSession && guideSnap === notifiedGuide) return
    notifiedSession = sessionSnap
    notifiedGuide = guideSnap
    for (const { cb } of [...listeners]) {
      try {
        cb()
      } catch (err) {
        console.error('[dockDrag] 구독자 오류:', err)
      }
    }
  }

  function current(): DragRecord | null {
    let best: DragRecord | null = null
    for (const r of records.values()) {
      if (!r.ended && (best === null || r.learned > best.learned)) best = r
    }
    return best
  }

  function recordOf(id: string): DragRecord {
    let rec = records.get(id)
    if (rec) return rec
    rec = {
      id,
      seq: -1,
      session: null,
      regions: null,
      ended: false,
      learned: ++learnedSeq,
      pointerSeen: false,
      escStopped: false,
    }
    records.set(id, rec)
    if (records.size > MAX_RECORDS) {
      const all = [...records.values()].sort((a, b) => a.learned - b.learned)
      const victim = all.find(r => r.ended) ?? all[0]
      records.delete(victim.id)
    }
    return rec
  }

  function displayedNow(): string | null {
    if (displayedOverride) return displayedOverride()
    return displayedTabOf(deps.activeTab(label), label, sessionSnap)
  }

  function syncSession(): void {
    const rec = current()
    const s = rec?.session ?? null
    const next: DockSession | null = rec && s ? { drag_id: rec.id, ...s } : null
    const same =
      next === sessionSnap ||
      (next !== null &&
        sessionSnap !== null &&
        next.drag_id === sessionSnap.drag_id &&
        next.source_label === sessionSnap.source_label &&
        next.source_view === sessionSnap.source_view &&
        next.display_active === sessionSnap.display_active)
    if (!same) sessionSnap = next
  }

  function setGuide(next: DockGuide | null): void {
    const prev = guideSnap
    const same =
      next === prev ||
      (next !== null &&
        prev !== null &&
        next.regions === prev.regions &&
        pickedKey(next.shown) === pickedKey(prev.shown) &&
        next.canvas === prev.canvas &&
        next.cursorSlot === prev.cursorSlot &&
        next.hover?.seq === prev.hover?.seq)
    if (!same) guideSnap = next
  }

  // ── 머묾(R6b) ───────────────────────────────────────────────────────────────

  function cancelHover(): void {
    if (hover) clearTimeout(hover.timer)
    hover = null
  }

  function updateHover(shown: Picked, displayed: string | null): void {
    const over = shown.type === 'tab_bar' ? shown.over : null
    const target = over !== null && over !== displayed ? over : null
    if (target === null) {
      cancelHover()
      hoverFired = null
      return
    }
    if (hover?.viewId === target || hoverFired === target) return
    cancelHover()
    hoverFired = null
    const g = gen
    hover = {
      viewId: target,
      seq: ++hoverSeq,
      timer: setTimeout(() => {
        if (g !== gen || hover?.viewId !== target) return
        hover = null
        // 같은 머리에 머무는 동안 다시 걸지 않는다 — 전환이 실패해도 같은 머리를 거듭 두드리지 않게.
        hoverFired = target
        deps.switchTab(label, target)
        if (guideSnap) setGuide({ ...guideSnap, hover: null })
        notify()
      }, HOVER_SWITCH_MS),
    }
  }

  // ── 판정 · 그리기 · 보고 ────────────────────────────────────────────────────────

  function sendOver(dragId: string, report: DragOverReport): void {
    overChain = overChain
      .then(() => ipc.invoke<void>('dock_drag_over', { drag_id: dragId, report }))
      .catch(err => console.warn('[dockDrag] dock_drag_over 실패:', err))
  }

  function evaluate(point: PointCss): void {
    const rec = current()
    const regions = rec?.regions ?? null
    const displayed = displayedNow()
    let shown: Picked = NONE
    let canvas = false
    let cursorSlot: string | null = null
    if (regions !== null && !regions.blocked) {
      // 판정 스크롤 = 보고 · 놓기에 실은 스크롤이어야 한다 — 셸이 그것(없으면 영역이 잰 스크롤)으로 다시 고르므로, 다르면
      //   넘친 탭바에서 그린 자리 ≠ 셸이 고른 자리가 되어 놓기가 거절된다. 보고가 스크롤을 싣지 않으므로 `null` 이다.
      const picked = deps.pick(regions, point, null, rec?.session?.source_view ?? '')
      canvas = displayed === regions.view_id
      // ★보이던 탭이 영역의 탭이 아니면 캔버스 가이드를 그리지 않는다 — 탭바는 그대로다★(TRD §1-4 · 5.2): 탭 목록
      //   알림과 영역이 엇갈린 틈에 다른 탭 칸 위에 옛 가이드를 그리지 않게. 탭바 영역은 보이는 탭과 무관하다.
      shown = picked.type === 'seam' && !canvas ? NONE : picked
      cursorSlot = canvas ? slotAt(regions, point) : null
    }
    drawn = {
      dragId: rec?.id ?? null,
      rev: regions?.rev ?? 0,
      barRev: regions?.bar_rev ?? 0,
      displayed,
      shown,
      point,
    }
    updateHover(shown, displayed)
    setGuide(
      regions !== null && !regions.blocked
        ? {
            regions,
            shown,
            canvas,
            cursorSlot,
            hover: hover ? { viewId: hover.viewId, seq: hover.seq } : null,
          }
        : null,
    )
    if (drawn.dragId === null) return
    const key = `${drawn.dragId}|${drawn.rev}|${drawn.barRev}|${displayed ?? ''}|${pickedKey(shown)}`
    if (key === lastSentKey) return
    lastSentKey = key
    sendOver(drawn.dragId, {
      type: 'over',
      rev: drawn.rev,
      bar_rev: drawn.barRev,
      displayed,
      shown,
      point,
      scroll: null,
    })
  }

  /** 영역 · 세션 · 보이던 탭이 바뀌면 — 끄는 중이면 마지막 점에서 다시 판정한다(움직이지 않아도 가이드가 따라간다). */
  function refresh(): void {
    if (inside && lastPoint) evaluate(lastPoint)
    notify()
  }

  function clearTarget(): void {
    inside = false
    lastPoint = null
    drawn = null
    lastSentKey = null
    cancelHover()
    hoverFired = null
    setGuide(null)
  }

  function pullOnce(): void {
    if (pulling) return
    pulling = true
    const g = gen
    ipc
      .invoke<DockPullReply | null>('dock_pull')
      .then(reply => {
        if (g !== gen || reply === null || reply.regions === null) return
        // `started` 는 `drag_id` 가 없어 영역이 함께 올 때만 그 id 로 안다. 세션 사건을 이미 받았으면 그쪽이 이긴다.
        const rec = takeRegions(reply.regions)
        if (rec && rec.session === null) rec.session = reply.started
        syncSession()
        refresh()
      })
      .catch(err => console.warn('[dockDrag] dock_pull 실패:', err))
      .finally(() => {
        if (g === gen) pulling = false
      })
  }

  // ── 사건 받기 ─────────────────────────────────────────────────────────────────

  function takeRegions(r: DockRegions): DragRecord | null {
    const rec = recordOf(r.drag_id)
    if (rec.ended) return null
    if (rec.regions !== null && r.rev <= rec.regions.rev) return rec
    rec.regions = r
    return rec
  }

  function onRegions(raw: unknown): void {
    const r = asRegions(raw)
    if (r === null) {
      console.warn('[dockDrag] 읽을 수 없는 dock:regions 를 버린다:', raw)
      return
    }
    if (takeRegions(r) === null) return
    syncSession()
    refresh()
  }

  function onSession(raw: unknown): void {
    const ev = asSessionEvent(raw)
    if (ev === null) {
      console.warn('[dockDrag] 읽을 수 없는 dock:session 을 버린다:', raw)
      return
    }
    const rec = recordOf(ev.drag_id)
    if (rec.ended) return
    if (ev.phase.type === 'ended') {
      endRecord(rec)
      return
    }
    if (ev.seq <= rec.seq) return
    rec.seq = ev.seq
    rec.session = {
      source_label: ev.phase.source_label,
      source_view: ev.phase.source_view,
      display_active: ev.phase.display_active,
    }
    syncSession()
    refresh()
  }

  function endRecord(rec: DragRecord): void {
    const swallowing = rec.session !== null && !rec.escStopped
    const owns = drawn === null || drawn.dragId === null || drawn.dragId === rec.id
    rec.ended = true
    rec.regions = null
    rec.session = null
    if (swallowing) {
      if (escTailTimer !== null) clearTimeout(escTailTimer)
      escTailFor = rec.id
      escTailTimer = setTimeout(() => {
        escTailFor = null
        escTailTimer = null
      }, ESC_SWALLOW_TAIL_MS)
    }
    // ★그 끌기가 그린 것이면 무조건 지운다★(R13b · AC16) — 어느 창이 떠남을 놓쳤어도 끝 방송 한 장이 가이드 · 머묾을
    //   거둔다. ★다른 끌기가 그린 것은 남긴다★ — 늦게 닿은 옛 `Ended` 가 새 끌기의 가이드 · 그린 것을 지우면 다음
    //   `dragover` 전의 놓기가 rev 0 · 가이드 없음으로 나가 무변경이 된다(`dock:session` 은 스레드 사이에서 뒤바뀌어 닿는다).
    //   모르는 id 로 그렸으면(`drawn.dragId === null`) 잃을 것이 없으니 지운다 — 남기면 묵은 「안에 있음」 · 점에서 다음
    //   끌기의 영역이 `refresh` 로 유령 가이드 · Over 를 낸다.
    if (owns) clearTarget()
    syncSession()
    notify()
  }

  // ── DOM 리스너 ────────────────────────────────────────────────────────────────
  // ★우리 MIME 사건은 `document` 에서 끊는다(`stopPropagation`)★(TRD §1-4 「다른 끌기 리스너」): 에이전트 트리의 react-dnd
  //   HTML5 백엔드가 뿌리 `window` 의 버블 리스너로 자기 끌기가 아닌 `dragover` 를 `dropEffect = 'none'` 으로 덮는다.
  //   ★리스너를 `window` 나 캡처로 옮기지 말 것★ — `window` 면 같은 노드의 등록 순서가 이겨 끊기가 안 서고, `document`
  //   캡처에서 끊으면 아래 요소의 처리기까지 못 받는다.

  function onDragEnterOver(e: DragEvent): void {
    if (!isOurs(e)) {
      // ★`dragenter` 도 막는다★ — Chromium 은 커서 밑 요소가 바뀐 걸음에 `dragenter` 만 낸다(`EventHandler::
      //   UpdateDragAndDrop`). 그것을 안 막으면 그 걸음 동안 문서가 끌기를 안 받는 것으로 쳐져 파일 · URL 에 copy 가 서고,
      //   그 틈(~16–60 ms)에 떼면 `drop` 사건 없이(`PerformDrop`) 새 탭 탐색으로 간다 — 아래 `drop` 막기가 안 돈다.
      // ★끊지 않는다★: 트리 재부모화 끌기는 뒤의 window 리스너(react-dnd)가 자기 `dropEffect` 로 덮어 그대로 돈다.
      if (navigates(e)) {
        e.preventDefault()
        if (e.dataTransfer) e.dataTransfer.dropEffect = 'none'
      }
      return
    }
    e.preventDefault()
    e.stopPropagation()
    const rec = current()
    // ★세션을 몰라도 받는다★(TRD §1-4 · 5.1 C3) — 이웃 창으로 빠르게 끈 끌기는 그 창에 `Started` 보다 먼저 닿는다.
    if (e.dataTransfer) e.dataTransfer.dropEffect = rec?.regions?.blocked ? 'none' : 'move'
    if (!inside) {
      inside = true
      if (!rec?.regions) pullOnce()
    }
    lastPoint = { x: e.clientX, y: e.clientY }
    evaluate(lastPoint)
    notify()
  }

  function onDragLeave(e: DragEvent): void {
    if (!isOurs(e)) return
    e.stopPropagation()
    // 관련 요소가 있으면 문서 안 요소 사이를 건넌 것이다 — Chromium 은 새 요소의 `dragenter` 를 먼저 낸다.
    if (e.relatedTarget !== null) return
    const wasInside = inside
    const dragId = drawn?.dragId ?? current()?.id ?? null
    clearTarget()
    if (wasInside && dragId !== null) sendOver(dragId, { type: 'left' })
    notify()
  }

  function onDrop(e: DragEvent): void {
    if (!isOurs(e)) {
      if (navigates(e)) e.preventDefault()
      return
    }
    e.preventDefault()
    e.stopPropagation()
    const data = parseDragData(e.dataTransfer?.getData(DOCK_MIME) ?? '')
    const d = drawn
    const displayed = displayedNow()
    clearTarget()
    notify()
    if (data === null) {
      console.warn('[dockDrag] 끌기 데이터를 못 읽어 놓기를 버린다')
      return
    }
    // ★그린 것을 그린 점과 함께 싣는다★ — 셸은 그 점을 같은 영역으로 다시 판정해 그린 것과 대조한다(M1). 마지막
    //   `dragover` 뒤 몇 px 움직여 뗀 점을 실으면 사용자가 본 가이드와 다른 판정이 나와 무변경이 될 수 있다.
    //   모르는 id(이 창이 아무것도 못 그림)여도 보낸다 — 셸이 시작보다 먼저 온 놓기를 붙든다(§1-3 「이른 끝 신호」).
    const report: DropReport =
      d !== null && d.dragId === data.drag_id
        ? {
            drag_id: data.drag_id,
            view_id: data.view_id,
            rev: d.rev,
            bar_rev: d.barRev,
            displayed: d.displayed,
            shown: d.shown,
            point: d.point,
            scroll: null,
          }
        : {
            drag_id: data.drag_id,
            view_id: data.view_id,
            rev: 0,
            bar_rev: 0,
            displayed,
            shown: NONE,
            point: { x: e.clientX, y: e.clientY },
            scroll: null,
          }
    ipc.invoke<void>('dock_drop', report).catch(err => console.warn('[dockDrag] dock_drop 실패:', err))
  }

  function onPointer(e: PointerEvent): void {
    // 끄는 동안 웹뷰에는 포인터 사건이 오지 않는다 — 무엇이든 왔다면 이 창 위에 끌기가 없다. 끝 신호를 놓친 끌기의
    //   가이드 · 「안에 있음」을 거둔다. ★`Left` 는 보내지 않는다★ — 장부의 유실 대비는 이 창을 「아직 위」로 쳐야
    //   마지막 보고로 판정한다(TRD §1-3 `dock_pointer_seen`).
    if (inside || guideSnap !== null) {
      clearTarget()
      notify()
    }
    if (e.pointerType !== 'mouse') return
    // 버튼을 쥔 이동은 증거가 아니다. 끄는 동안 웹뷰에 온 이동은 0/307 이었고, 이 거름이 실제로 거르는 것은 Esc 로 끝낼
    //   때(버튼은 아직 쥠) 끝과 거의 같이 커서 밑 창에 오는 buttons=1 이동이다(99/99 — 그 끌기는 끝 신호가 이미 났다).
    //   실측 = TRD §8-2 S1 ③(2026-10-17).
    if (e.type === 'pointermove' && e.buttons !== 0) return
    const rec = current()
    if (rec === null || rec.session === null || rec.pointerSeen) return
    rec.pointerSeen = true
    ipc
      .invoke<void>('dock_pointer_seen', { drag_id: rec.id })
      .catch(err => console.warn('[dockDrag] dock_pointer_seen 실패:', err))
    // Esc 삼키기 상한(TRD 5.1 N9) — 끝 신호를 잃어 세션이 남아도 마우스를 다시 쓴 창의 Esc(claude 턴 끊기)를 계속 먹지 않는다.
    const g = gen
    const timer = setTimeout(() => {
      evidenceTimers.delete(timer)
      if (g !== gen) return
      rec.escStopped = true
      if (escTailFor === rec.id) escTailFor = null
    }, ESC_SWALLOW_TAIL_MS)
    evidenceTimers.add(timer)
  }

  function onKeyDown(e: KeyboardEvent): void {
    if (e.key !== 'Escape') return
    const rec = current()
    const live = rec !== null && rec.session !== null && !rec.escStopped
    if (!live && escTailFor === null) return
    // 보내지 않는다 — Esc 는 OS 끌기 루프가 받아 끌기를 끝낸다. OS 가 그 keydown 을 웹뷰에도 넘기는지는 모른다(S1 ⑥) —
    //   넘기면 터미널(claude 턴 끊기) · 입력창에 닿지 않게 하는 보험이다. 같은 window 캡처의 뒤 리스너도 막는다.
    e.preventDefault()
    e.stopPropagation()
    e.stopImmediatePropagation()
  }

  // ── 붙이기 · 떼기 ─────────────────────────────────────────────────────────────

  function adopt(g: number, off: () => void): void {
    if (g !== gen) off()
    else handles.push(off)
  }

  function register(g: number, event: string, target: string | null, h: (p: unknown) => void): void {
    void retryAsync(() => ipc.listen(event, p => (g === gen ? h(p) : undefined), target), {
      isCancelled: () => g !== gen,
      onRetry: (err, n) => console.warn(`[dockDrag] ${event} 구독 재시도 #${n}:`, err),
    })
      .then(off => adopt(g, off))
      .catch(err => {
        if (err instanceof RetryCancelledError) return
        console.error(`[dockDrag] ${event} 구독 최종 실패 — 이 창은 끌기 가이드를 못 그린다:`, err)
      })
  }

  function attach(): void {
    const g = ++gen
    document.addEventListener('dragenter', onDragEnterOver)
    document.addEventListener('dragover', onDragEnterOver)
    document.addEventListener('dragleave', onDragLeave)
    document.addEventListener('drop', onDrop)
    window.addEventListener('keydown', onKeyDown, true)
    window.addEventListener('pointermove', onPointer, true)
    window.addEventListener('pointerdown', onPointer, true)
    unsubActive = deps.subscribeActive(label, refresh)
    register(g, EVT_DOCK_SESSION, null, onSession)
    // ★`target` = 자기 label★ — 셸은 창마다 `emit_to(label, …)` 로 그 창 영역을 보내는데, 기본 타깃(`Any`)은 남의 창
    //   영역까지 받아 엉뚱한 가이드를 그리고 모든 놓기가 옛 번호로 무변경이 된다(`theme/uiSettings.ts` 의 같은 조항).
    register(g, EVT_DOCK_REGIONS, label, onRegions)
  }

  function detach(): void {
    gen++
    document.removeEventListener('dragenter', onDragEnterOver)
    document.removeEventListener('dragover', onDragEnterOver)
    document.removeEventListener('dragleave', onDragLeave)
    document.removeEventListener('drop', onDrop)
    window.removeEventListener('keydown', onKeyDown, true)
    window.removeEventListener('pointermove', onPointer, true)
    window.removeEventListener('pointerdown', onPointer, true)
    unsubActive?.()
    unsubActive = null
    for (const off of handles) off()
    handles = []
    clearTarget()
    for (const t of evidenceTimers) clearTimeout(t)
    evidenceTimers.clear()
    if (escTailTimer !== null) clearTimeout(escTailTimer)
    escTailTimer = null
    escTailFor = null
    pulling = false
    // 떼어 둔 동안의 사건은 못 받으므로 세션 기억을 비운다 — 다시 붙으면 사건 · 당기기로 다시 안다.
    records.clear()
    sessionSnap = null
    notify()
  }

  return {
    mount(nextLabel, options) {
      if (mounts > 0 && nextLabel !== label) {
        console.warn(`[dockDrag] 이미 '${label}' 로 붙어 있어 '${nextLabel}' 는 무시한다`)
      } else if (mounts === 0) {
        label = nextLabel
        displayedOverride = options?.displayed
        attach()
      }
      mounts++
      let released = false
      return () => {
        if (released) return
        released = true
        mounts--
        if (mounts === 0) detach()
      }
    },
    getSession: () => sessionSnap,
    getGuide: () => guideSnap,
    subscribe(cb) {
      const entry = { cb }
      listeners.add(entry)
      return () => {
        listeners.delete(entry)
      }
    },
  }
}

const tauriDeps: DockDragDeps = {
  ipc: {
    listen: (event, handler, target) =>
      listen<unknown>(event, e => handler(e.payload), target === null ? undefined : { target }),
    invoke: <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args),
  },
  pick: pickTarget,
  activeTab: l => useViewStore.getState().windows[l]?.active ?? null,
  subscribeActive: (l, cb) =>
    useViewStore.subscribe((s, prev) => {
      if (s.windows[l]?.active !== prev.windows[l]?.active) cb()
    }),
  switchTab: (l, viewId) =>
    void useViewStore
      .getState()
      .switchTab(l, viewId)
      .catch(err => console.error('[dockDrag] 머묾 탭 전환 실패:', err)),
}

let instance: DockDragController | null = null

function dockDrag(): DockDragController {
  instance ??= createDockDrag(tauriDeps)
  return instance
}

/**
 * 창 `label`(그 창의 Tauri label — `WindowLayout` 의 `label`)의 끌기 대상 · 세션 리스너를 단다. 반환 = 떼기.
 * 창당 하나만 붙는다 — 붙어 있는 동안 다른 label 로 부르면 무시하고 경고한다.
 */
export function mountDockDrag(label: string, options?: MountOptions): () => void {
  return dockDrag().mount(label, options)
}

/** 지금 살아 있는 세션(떼어진 표시의 재료) — 없거나 끝났으면 `null`. */
export function useDockSession(): DockSession | null {
  const ctl = dockDrag()
  return useSyncExternalStore(ctl.subscribe, ctl.getSession)
}

/** 이 창에 지금 그릴 가이드 — 없으면 `null`. */
export function useDockGuide(): DockGuide | null {
  const ctl = dockDrag()
  return useSyncExternalStore(ctl.subscribe, ctl.getGuide)
}

/** 창 `label` 의 캔버스가 지금 보일 탭 — `displayedTabOf` 의 훅 판(떼어진 표시 포함). */
export function useDisplayedTab(label: string): string | null {
  const active = useViewStore(s => s.windows[label]?.active ?? null)
  return displayedTabOf(active, label, useDockSession())
}

/** 시험 전용 — 모듈 인스턴스를 버린다. 붙어 있으면 먼저 떼야 한다. */
export function __resetDockDragForTest(): void {
  instance = null
}
