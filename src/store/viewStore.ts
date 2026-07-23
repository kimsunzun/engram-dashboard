// viewStore — 레이아웃 권위(src-tauri ViewManager, ADR-0035/0057)의 프론트 미러 + 제어 표면(§5).
//
// ★권위 = 백엔드★: 이 스토어는 레이아웃을 *직접 변형하지 않는다*. 액션은 대응 invoke 만 부르고,
// 실제 상태 갱신은 백엔드가 emit 하는 layout:updated / window:tabs-updated 를 listen 해서만 한다
// (낙관적 갱신 X). 그래서 사람 클릭이든 LLM(cdp eval → window.__engramLayout)이든 같은 invoke→emit
// 루프 한 곳을 지난다 — 두 입력이 같은 단일 control surface 를 흔든다(§5 손발/두뇌 분리).
//
// ★레이아웃은 agentClient/ProtocolClient seam(ADR-0011)을 거치지 않는다★ — 그건 *에이전트 명령*
// 전용(데몬 권위)이고, 레이아웃은 src-tauri 권위(ADR-0035)라 @tauri-apps/api invoke/listen 직접 호출.
//
// ★탭 소유 모델(ADR-0057)★: 옛 전역 activeViewId 는 사라졌다. 한 창(label)이 탭 목록(View 여러 벌)을
// 소유하고 그 안에서 전환한다. 프론트는 `windows: label → {tabs, active, version}` 로 창별 탭 상태를
// 미러하고(window:tabs-updated 소비), 렌더는 "이 웹뷰 창의 active 탭"(useCurrentViewId)이 정한다.
//   - main = windows["main"].active
//   - 팝업 = ?window=<label> 의 windows[label].active
//   - agent-tree = windows["main"].active 폴백(모델 밖 config 창 — §3-4/G7 특례)
//
// ★view_id 별 캐시 모델★(핵심 불변식): layout 을 view_id → {layout,focus,version} 캐시로 보유한다.
// 왜 캐시인가:
//   - 백엔드 ViewManager.version 은 *전역 단조 카운터*(모든 view 공유, manager.rs bump_version).
//     모든 snapshot 이 같은 전역 version 을 박는다 → "view_id 가 다르면 무조건 채택"식 가드는 틀렸다.
//     다른 view 의 늦은 emit(낮은 전역 version)이 stale 로 덮을 수 있다(F1+F4).
//   - 캐시는 view_id 별 독립 항목이라 다른 view 끼리 version 이 충돌하지 않는다. 같은 view 안에서는
//     전역 단조라 version 단조 비교가 그대로 성립 → stale emit 폐기가 view 별로 정확하다.
//   - keep-alive(ADR-0056): 창은 자기 tabs 전부를 마운트하고 활성만 표시한다 → 캐시는 활성 탭뿐 아니라
//     그 창의 모든 탭 layout 을 담아야 한다(숨은 탭도 렌더 유지). 각 슬롯 캔버스가 자기 view_id 캐시를 본다.

import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { create } from 'zustand'

import type { LayoutNode, SlotContent, SplitDir, ViewMeta, ViewSnapshot } from '../api/layoutTypes'
import { isRenderMode, type RenderMode } from '../components/slot/renderMode'
import { retryAsync } from '../util/retryInvoke'

/** 메인 창 label(백엔드 MAIN_WINDOW_LABEL 미러). agent-tree 폴백·기본 대상. */
export const MAIN_WINDOW_LABEL = 'main'

/**
 * window:tabs-updated / list_tabs 페이로드(WindowTabsPayload 미러, commands/layout.rs).
 * 옛 ViewListPayload{views, active_view_id}(전역) → 창별 {label, tabs, active, version}(ADR-0057).
 */
export interface WindowTabsPayload {
  label: string
  tabs: ViewMeta[]
  active: string
  version: number
}

/** 한 창의 탭 상태(프론트 미러). version = 전역 단조(stale emit 폐기용). */
export interface WindowTabs {
  tabs: ViewMeta[]
  active: string
  version: number
}

/** view_id 별 캐시 항목 — 그 view 가 마지막으로 채택한 레이아웃 + focus + (전역 단조) version. */
export interface CachedView {
  layout: LayoutNode
  focusedSlotId: string | null
  /** 이 항목을 마지막으로 채택한 전역 version(stale emit 가드 — 같은 view 안에서 단조 비교). */
  version: number
}

interface ViewState {
  /** view_id → 캐시된 레이아웃. 창 캔버스는 자기 창 tabs 의 각 view_id 캐시를 렌더한다(keep-alive). */
  layouts: Record<string, CachedView>
  /** 창 label → 그 창의 탭 상태(window:tabs-updated 미러). 탭바·useCurrentViewId 의 단일 출처. */
  windows: Record<string, WindowTabs>

  /**
   * ★렌더 모드 오버라이드(§5)★: slot node.id → 강제 RenderMode. caps 유도 기본 렌더러
   * (defaultRenderMode: structured→'rich' / else→'terminal')를 무시하고 지정한 렌더러를 마운트한다.
   * 미지정 slot 은 여기 키가 없어 기본 유도로 떨어진다(?? defaultRenderMode).
   * ★프론트 전용★ — 백엔드 wire LayoutNode 는 렌더 모드 개념을 모른다(override라 권위 레이아웃과 무관).
   * 그래서 이 필드는 invoke→emit 권위 루프를 안 탄다(위 "낙관적 갱신 X" 규칙의 프론트 전용 예외).
   */
  renderModeOverride: Record<string, RenderMode>

  // ── 탭/창 command(창 label 을 받는 창별 조작, ADR-0057) ──────────────────────────
  /** 창 `window` 에 새 빈-슬롯 탭 추가·활성화 → 새 view_id 반환. */
  createTab: (window: string, name?: string) => Promise<string>
  /** 창 `window` 의 탭 `view` 닫기(§5-2 상태기계). */
  closeTab: (window: string, viewId: string) => Promise<void>
  /** 창 `window` 의 활성 탭 변경(그 창만). */
  switchTab: (window: string, viewId: string) => Promise<void>
  /**
   * 탭(View) 이름 교체. view_id 전역 유니크라 window 는 안 받는다(백엔드 view_owner 파생). 백엔드
   * rename_tab 을 invoke 하고 실제 이름 반영은 window:tabs-updated emit 으로만(낙관 갱신 X — 이름 권위
   * = src-tauri, ADR-0035). §5 단일 표면: 사람 더블클릭 인라인 편집(TabBar)·LLM(__engramCmd → tab.rename)이
   * 같은 이 핸들로 수렴한다. 이름 정규화(trim/공백거부)는 호출부 경계(TabBar·tab.rename)가 담당.
   */
  renameTab: (viewId: string, name: string) => Promise<void>
  /** 빈 새 창(빈 탭 1개) 생성 → 새 창 label 반환(D-6). */
  createWindow: () => Promise<string>
  /** 창 `window` 통째 닫기(main 금지). */
  closeWindow: (window: string) => Promise<void>

  // ── View 내부 조작(view_id 전역 유니크라 시그니처 유지 — 소속 창은 백엔드 view_owner 파생) ──
  /** slot 분할 → 새 slot_id 반환. */
  split: (viewId: string, slotId: string, dir: SplitDir) => Promise<string>
  /**
   * slot 을 포커스로 지정(click-to-focus — ADR-0066 결정 1). 백엔드 focus_slot 을 invoke 하고 실제 링
   * 갱신은 layout:updated emit 으로만(낙관 갱신 X — focused_slot_id 권위 = src-tauri, ADR-0035/0066).
   * 사람 클릭·팔레트·키바인딩·LLM(window.__engramCmd → slot.focus)이 같은 이 핸들을 흔든다(§5).
   */
  focusSlot: (viewId: string, slotId: string) => Promise<void>
  /** slot 닫기(형제 승격). */
  closeSlot: (viewId: string, slotId: string) => Promise<void>
  /** slot 에 agent 참조 배정. */
  assignAgent: (viewId: string, slotId: string, agentId: string) => Promise<void>
  /**
   * slot 의 콘텐츠를 SlotContent 유니온 어느 것으로도 교체(ADR-0063 배치 제어 표면). 트리(agent_list)·
   * 팔레트(preset_palette)·비우기(empty)를 슬롯에 배치한다. 백엔드 set_slot_content 를 invoke 하고 실제
   * 반영은 layout:updated emit 으로만(낙관 갱신 X — 레이아웃 권위 = src-tauri, ADR-0035).
   * §5: window.__engramLayout.setSlotContent 로 LLM 도 호출.
   */
  setSlotContent: (viewId: string, slotId: string, content: SlotContent) => Promise<void>
  /**
   * slot 의 agent 를 다른 창의 새 탭으로 MOVE(detach, not mirror). 백엔드 move_slot_to_window 가 새 View
   * 생성 → agent 이전 → 대상 창(미지정 시 새 팝업 창) 새 탭 삽입 → 원본 슬롯 제거를 한다. 원본 슬롯 제거는
   * emit(layout:updated)으로 반영된다(낙관 갱신 X — 백엔드 권위, ADR-0035). 반환 = {window, tab}(G4).
   * §5: window.__engramLayout.moveSlotToWindow 로 LLM 도 호출.
   */
  moveSlotToWindow: (
    viewId: string,
    slotId: string,
    toWindow?: string,
  ) => Promise<{ window: string; tab: string }>

  // ── ★렌더 모드 오버라이드(§5)★ 지정/해제(프론트 전용, invoke 안 탐) ──────────────────────────
  /** slot 의 렌더러를 mode 로 강제(caps 유도 기본을 덮음). */
  setRenderMode: (nodeId: string, mode: RenderMode) => void
  /** slot 의 오버라이드 해제(caps 유도 기본 렌더러로 복귀). */
  clearRenderMode: (nodeId: string) => void

  // ── ★DOM 모드 별칭(§5 관측)★: 검증 툴링(window.__engramLayout)이 이 이름을 씀 — 이름 변경 금지 ──
  /** slot 을 DOM 모드로(= setRenderMode(id,'dom')). */
  enableDomMode: (nodeId: string) => void
  /** slot 의 DOM 모드 해제(= clearRenderMode(id)). */
  disableDomMode: (nodeId: string) => void
  /** slot 의 DOM 모드 토글(dom ↔ 기본). */
  toggleDomMode: (nodeId: string) => void

  // ── emit 수신 핸들러(eventBus/WindowLayout 이 listen 콜백에서 호출) ───────────────────────────
  /** layout:updated 수신 — 그 view_id 캐시 항목을 version 가드 통과 시 갱신. */
  applyLayoutUpdated: (snap: ViewSnapshot) => void
  /** window:tabs-updated / list_tabs 수신 — 그 창의 탭 목록/active 갱신(version stale 방어). */
  applyWindowTabsUpdated: (payload: WindowTabsPayload) => void
}

export const useViewStore = create<ViewState>((set, get) => ({
  layouts: {},
  windows: {},
  renderModeOverride: {}, // 갱신 경로 = set/clearRenderMode(+DOM 별칭)만(계약은 필드 JSDoc).

  createTab: (window, name) => invoke<string>('create_tab', { window, name: name ?? null }),
  closeTab: (window, viewId) => invoke<void>('close_tab', { window, view: viewId }),
  switchTab: (window, viewId) => invoke<void>('switch_tab', { window, view: viewId }),
  createWindow: () => invoke<string>('create_window'),
  closeWindow: window => invoke<void>('close_window', { window }),

  split: (viewId, slotId, dir) => invoke<string>('split_slot', { viewId, slotId, dir }),
  // ADR-0057/0035: 낙관 갱신 X — invoke 만 부르고 이름은 window:tabs-updated emit 으로만 반영(백엔드 권위).
  //   §5 단일 표면(사람 더블클릭 + LLM tab.rename 이 여기로 수렴). trim/공백거부는 호출부(TabBar·tab.rename).
  renameTab: (viewId, name) => invoke<void>('rename_tab', { viewId, name }),
  // ADR-0066: 낙관 갱신 X — invoke 만 부르고 링은 layout:updated emit 으로만 갱신(백엔드 권위, ADR-0035).
  focusSlot: (viewId, slotId) => invoke<void>('focus_slot', { viewId, slotId }),
  closeSlot: (viewId, slotId) => {
    // slot 이 사라지므로 그 slot 의 렌더 오버라이드 엔트리도 즉시 제거(누수 방지 — 프론트 전용 상태인
    // renderModeOverride 는 invoke→emit 권위 루프를 안 타므로 여기서 낙관적으로 정리한다).
    get().clearRenderMode(slotId)
    return invoke<void>('close_slot', { viewId, slotId })
  },
  assignAgent: (viewId, slotId, agentId) => {
    // slot UUID 는 재배정에도 안정(agent_id 만 바뀐다) → 이전 agent 를 위해 건 오버라이드가 새 agent 에
    // 조용히 적용되면 안 된다. 그래서 assign 시 그 slot 의 오버라이드를 clear 해 새 agent 는 caps 유도
    // 기본으로 시작하게 한다(프론트 전용 낙관 갱신 — renderModeOverride, 권위 루프 밖).
    get().clearRenderMode(slotId)
    return invoke<void>('assign_agent', { viewId, slotId, agentId })
  },
  setSlotContent: (viewId, slotId, content) => {
    // slot 콘텐츠가 통째 바뀌므로(에이전트→트리 등) 그 slot 의 렌더 오버라이드도 즉시 정리(assignAgent 와
    // 동형 — 프론트 전용 상태라 emit 권위 루프 밖). 실제 콘텐츠 교체는 백엔드 set_slot_content 가 하고
    // layout:updated emit 으로 반영된다(낙관 갱신 X, ADR-0035).
    get().clearRenderMode(slotId)
    return invoke<void>('set_slot_content', { viewId, slotId, content })
  },
  moveSlotToWindow: (viewId, slotId, toWindow) => {
    // slot 이 원본 창에서 사라지므로(MOVE) 그 slot 의 렌더 오버라이드 엔트리도 즉시 정리(누수 방지 —
    // 프론트 전용 상태라 emit 루프 밖). 실제 슬롯 제거·대상 창 탭 삽입은 백엔드 move_slot_to_window 가
    // 하고 emit(양 창 window:tabs-updated + 원본 layout:updated)으로 반영된다.
    get().clearRenderMode(slotId)
    return invoke<{ window: string; tab: string }>('move_slot_to_window', {
      viewId,
      slotId,
      toWindow: toWindow ?? null,
    })
  },

  // ★렌더 모드 오버라이드 = 프론트 전용 예외★: invoke 안 부르고 프론트 상태만 set(override 라 백엔드
  // 권위 레이아웃과 무관 — 백엔드 wire LayoutNode 는 렌더 모드 개념을 모른다).
  setRenderMode: (nodeId, mode) => {
    // ★미타입 JS 진입 가드(FIX-4)★: window.__engramLayout 경유로 임의 문자열이 올 수 있다. 유효 mode 가
    // 아니면 store 에 쓰지 않고 no-op — 무효 값이 오버라이드로 새면 ViewLayoutRenderer switch 가 그걸
    // 조용히 terminal 로 떨어뜨려(default) 의도와 다른 렌더가 되므로, 쓰기 전에 걸러 경고만 남긴다.
    if (!isRenderMode(mode)) {
      console.warn(`[viewStore] setRenderMode: 무효 mode 무시 — ${JSON.stringify(mode)}`)
      return
    }
    set(state => ({ renderModeOverride: { ...state.renderModeOverride, [nodeId]: mode } }))
  },
  clearRenderMode: nodeId =>
    set(state => {
      const next = { ...state.renderModeOverride }
      delete next[nodeId] // 키 제거 → 렌더러가 ?? defaultRenderMode 로 caps 유도 기본으로 복귀.
      return { renderModeOverride: next }
    }),

  // ★DOM 모드 = 얇은 별칭★(검증 툴링이 이 이름을 계속 씀 — 제거 금지). enable/disable/toggle 을
  // set/clearRenderMode 위에 얹은 래퍼로만 정의(도메인 상태는 renderModeOverride 하나뿐).
  enableDomMode: nodeId => get().setRenderMode(nodeId, 'dom'),
  disableDomMode: nodeId => get().clearRenderMode(nodeId),
  toggleDomMode: nodeId =>
    get().renderModeOverride[nodeId] === 'dom'
      ? get().clearRenderMode(nodeId)
      : get().setRenderMode(nodeId, 'dom'),

  applyLayoutUpdated: snap => {
    // 그 view_id 캐시 항목의 version 보다 클 때만 갱신(전역 단조라 같은 view 내 단조 비교가 성립).
    // ★다른 view 항목과는 version 을 비교하지 않는다★ — 캐시가 독립 항목이라 충돌이 없고, "view_id 가
    // 다르면 무조건 채택" 같은 분기가 필요 없다(그 분기가 F1+F4 의 뿌리였다). 첫 수신은 캐시에 항목이
    // 없어 자동 채택(version === undefined 비교).
    const prev = get().layouts[snap.view_id]
    if (prev && snap.version <= prev.version) return
    set(state => ({
      layouts: {
        ...state.layouts,
        [snap.view_id]: {
          layout: snap.layout,
          focusedSlotId: snap.focused_slot_id,
          version: snap.version,
        },
      },
    }))
  },

  applyWindowTabsUpdated: payload => {
    // 그 창의 탭 목록·active 를 미러. ★version stale 방어(G10)★: 같은 창의 낮은/같은 전역 version emit 은
    // 폐기한다(전역 단조라 창 내 비교가 성립). 첫 수신(prev 없음)은 자동 채택. 다른 창끼리는 version 을
    // 비교하지 않는다(창별 독립 엔트리 — layout 캐시와 동일 규율).
    const prev = get().windows[payload.label]
    if (prev && payload.version <= prev.version) return
    set(state => ({
      windows: {
        ...state.windows,
        [payload.label]: {
          tabs: payload.tabs,
          active: payload.active,
          version: payload.version,
        },
      },
    }))
  },
}))

/** 현재 active view 의 캐시 항목(없으면 null) — 창 캔버스 렌더 selector(그 view_id 캐시 조회). */
export function selectView(state: ViewState, viewId: string | null): CachedView | null {
  return viewId ? (state.layouts[viewId] ?? null) : null
}

// ── 이 웹뷰가 어느 창인지 판정(§3-3/§3-4, G7) ────────────────────────────────────────────────
/**
 * 이 웹뷰 창의 label 을 URL 해시로 판정한다. 팝업 라우트(`#/popup?window=<label>`)면 그 label,
 * 그 외(메인 `#/`·트리 `#/tree`)면 `"main"`.
 *
 * ★왜 URL 인가★: 팝업 출력 Channel 은 getCurrentWindow().label() 로 구독하지만(agent.rs), 프론트가
 * 자기 활성 탭을 알려면 창 label → windows[label].active 경로가 필요하다. label 은 URL(`?window=`)이
 * SSOT 다(§3-3). ★단일 출처★: WindowLayout·useCurrentViewId·SlotContextMenu 가 같은 이 함수를 써
 * "이 창이 어느 창인지"를 한 곳으로 모은다.
 */
export function readWindowLabelFromHash(): string {
  // hash 예: "#/popup?window=slot-popup-3". '?' 뒤를 URLSearchParams 로 파싱.
  const hash = window.location.hash
  const qIndex = hash.indexOf('?')
  if (qIndex < 0) return MAIN_WINDOW_LABEL
  // ★라우트 스코핑★: `?window=` 는 팝업 라우트에서만 신뢰한다. 라우트 경로(= '#' 과 '?' 사이)가 정확히
  // `/popup` 일 때만 파싱. 그 외 hash(메인 `#/?window=x` 같은 도달불가 상태 포함)는 main 폴백.
  const path = hash.slice(0, qIndex)
  if (path !== '#/popup') return MAIN_WINDOW_LABEL
  const params = new URLSearchParams(hash.slice(qIndex + 1))
  return params.get('window') ?? MAIN_WINDOW_LABEL
}

/**
 * ★이 웹뷰 창의 현재 active 탭 view id(§3-4, G7).★ 창별 상태에서 파생한다:
 *   - main = windows["main"].active
 *   - 팝업 = ?window=<label> 의 windows[label].active
 *   - agent-tree = URL 이 `#/tree` 라 위 판정이 main 폴백 → windows["main"].active(모델 밖 config 창 특례)
 * 못 구하면 null(그 창의 탭 상태가 아직 안 도착 — 부팅 직후 pull 전).
 *
 * 사람 클릭(SlotContextMenu)·LLM(window.__engramLayout)이 같은 이 판정을 공유해 "자기 창 active 탭"으로
 * 동작한다 — 팝업 안 호출이 엉뚱한 main view 를 건드리지 않는다.
 */
export function useCurrentViewId(): string | null {
  const label = readWindowLabelFromHash()
  return useViewStore(s => s.windows[label]?.active ?? null)
}

/** 훅 밖(이벤트 핸들러·__engramLayout)에서 현재 창 active 탭 조회. useCurrentViewId 의 non-hook 판. */
export function currentViewId(): string | null {
  const label = readWindowLabelFromHash()
  return useViewStore.getState().windows[label]?.active ?? null
}

// ── emit 구독 등록(eventBus 에서 1회 호출, HMR/중복 가드는 eventBus 가 dispose 로 관리) ──────────
// 반환 = `{ dispose, ready }`(동기). 호출자는 dispose 를 await 없이 *즉시* 손에 쥐고(누수 0 — 아래),
// ready 를 await 한 뒤에야 init pull 을 돌린다(F-listen). listen() 은 async 라 등록이 끝나기 전 도착한
// emit 은 핸들러가 없어 누락된다 → 등록 완료를 ready 로 노출해 호출자가 그 뒤에 pull 을 부른다.
//
// ★왜 동기 dispose 인가(dead-branch 누수 방지)★: 등록 await 후 disposer 반환 금지 — 호출자(eventBus)가
// 그 await 가 pending 인 동안 정리(HMR dispose/재-init)를 돌리면 아직 disposer 를 *못 받아* unlisten 을 못 걸고
// → 늦게 등록 완료된 리스너가 영구 누수된다. 표준 해법(RxJS Subscription/useSyncExternalStore: teardown 핸들
// 동기 확보)대로, dispose 를 등록 await 없이 즉시 돌려주고 등록은 백그라운드로 시작한다.
// 근거: docs/research/async-subscribe-cleanup-race-2026-06-28.md.
//
// ★탭 소유 모델(ADR-0057)★: 옛 view:list-updated(전역) 대신 window:tabs-updated(창별)를 듣는다.
// layout:updated 는 그대로. 각 WindowLayout 도 자기 label 필터로 이걸 듣지만, 여기(전역 구독)는
// main 창의 탭 상태를 store 에 채워 AppLayout/AgentTree 가 즉시 반응하게 한다(모든 label 을 store 에 미러).
export function subscribeViewEvents(): { dispose: () => void; ready: Promise<void> } {
  let disposed = false
  // 등록 완료된 unlisten 핸들들(아직 해제 전인 것만). dispose 는 여기 담긴 것만 호출 → idempotent 보장.
  const handles: UnlistenFn[] = []

  const store = useViewStore.getState()

  // 갓 등록된 핸들을 받는 단일 경로. dispose 가 이미 불렸으면(disposed) 즉시 해제(늦게 도착한 핸들 누수
  // 가드 — 이 경로가 ready-전 dispose 의 실제 동작 분기), 아니면 handles 에 모아 나중 dispose 가 해제한다.
  const adopt = (fn: UnlistenFn): void => {
    if (disposed) {
      fn()
    } else {
      handles.push(fn)
    }
  }

  // 등록을 백그라운드로 시작 — dispose 는 이 await 를 기다리지 않고 즉시 반환된다(누수 가드의 핵심).
  // ready 는 두 등록 모두 settle 된 뒤 resolve(F-listen). 한쪽 실패 시: ready = Promise.all 이라 *즉시*
  // reject 한다(hang 금지). dispose 가 먼저 와도 adopt 가 도착한 핸들을 즉시 해제하므로 ready 는 정상 종료.
  const ready = Promise.all([
    listen<ViewSnapshot>('layout:updated', e => {
      store.applyLayoutUpdated(e.payload)
    }).then(adopt),
    listen<WindowTabsPayload>('window:tabs-updated', e => {
      store.applyWindowTabsUpdated(e.payload)
    }).then(adopt),
  ]).then(() => undefined)

  const dispose = (): void => {
    disposed = true
    // 이미 등록 완료돼 handles 에 담긴 핸들을 모두 해제하고 비운다 → 두 번 불려도(idempotent) 빈 배열이라
    // noop. ready 전이라 아직 비어 있으면, 늦게 도착할 핸들은 위 adopt 의 disposed 분기가 즉시 해제한다.
    while (handles.length > 0) handles.pop()!()
  }

  return { dispose, ready }
}

// ── 부팅 init — main 창 탭 상태 + active 탭 레이아웃 pull(read-only) ──────────────────────────
/**
 * 백엔드의 main 창 탭 상태를 pull 해 채우고, active 탭 레이아웃을 캐시에 넣는다.
 * 호출 규약: eventBus 가 구독 등록 직후(ready 후) 1회 호출.
 *
 * 왜 필요한가: ViewManager 는 부팅 시 기본 View 를 자동 생성하지만 그 생성은 *부팅 전*이라 emit 으로
 * 안 닿는다(변경 핸들러는 변경 직후에만 emit). read-only list_tabs("main")/get_view 로 현재 탭+active
 * 레이아웃을 끌어와 화면을 즉시 그린다. ★유령 View 생성 없이★ — pull 만(version 안 올림, broadcast 없음).
 * 구독을 먼저 걸어둔 뒤 호출되므로 init 도중 들어온 emit 이 더 최신이면 캐시/창 version 가드가 pull 을
 * 걸러낸다(옛 pull 이 새 emit 을 덮지 않음).
 */
export async function initMainWindowFromBackend(): Promise<void> {
  // ADR-0102: 두 부팅 pull 을 유계 재시도로 감싼다 — main 창은 이벤트 복구 경로가 없어(window:tabs-updated 는
  //   탭 변형 시에만 발화) 이 read-only pull 이 조기 transient 로 한 번 실패하면 화면이 채워지지 않는다.
  //   재시도 소진 시엔 throw 로 최종 실패를 호출자(eventBus)에게 전파해 console.error 로 표면화한다(조용히
  //   삼키지 않음). 성공 시 캐시/창 version 가드가 그 사이 도착한 더 최신 emit 을 덮지 않게 막는다(역전 방지).
  const payload = await retryAsync(() =>
    invoke<WindowTabsPayload>('list_tabs', { window: MAIN_WINDOW_LABEL }),
  )
  useViewStore.getState().applyWindowTabsUpdated(payload)
  // active 탭 레이아웃도 pull — 부팅 즉시 캔버스가 그려지게. (keep-alive 라 나머지 탭은 WindowLayout
  // 이 마운트 시 각자 get_view 로 채운다. main 부팅 렌더엔 active 만 있으면 충분.) 이것도 재시도한다.
  const snap = await retryAsync(() =>
    invoke<ViewSnapshot>('get_view', { viewId: payload.active }),
  )
  useViewStore.getState().applyLayoutUpdated(snap)
}

// getCurrentWindow 를 여기서 재노출 — 팝업 자가닫힘(0탭)에서 창 close 에 쓴다(§7-1). import 경로 통일.
export { getCurrentWindow }
