// viewStore — 레이아웃 권위(src-tauri ViewManager, ADR-0035)의 프론트 미러 + 제어 표면(§5).
//
// ★권위 = 백엔드★: 이 스토어는 레이아웃을 *직접 변형하지 않는다*. 액션은 대응 invoke 만 부르고,
// 실제 상태 갱신은 백엔드가 emit 하는 layout:updated / view:list-updated 를 listen 해서만 한다
// (낙관적 갱신 X). 그래서 사람 클릭이든 LLM(cdp eval → window.__engramLayout)이든 같은 invoke→emit
// 루프 한 곳을 지난다 — 두 입력이 같은 단일 control surface 를 흔든다(§5 손발/두뇌 분리).
//
// ★레이아웃은 agentClient/ProtocolClient seam(ADR-0011)을 거치지 않는다★ — 그건 *에이전트 명령*
// 전용(데몬 권위)이고, 레이아웃은 src-tauri 권위(ADR-0035)라 @tauri-apps/api invoke/listen 직접 호출.
//
// ★view_id 별 캐시 모델★(핵심 불변식): layout 을 view_id → {layout,focus,version} 캐시로 보유하고,
// 메인 창은 ★항상 activeViewId 의 캐시 항목만 렌더★ 한다. 왜 캐시인가:
//   - 백엔드 ViewManager.version 은 *전역 단조 카운터*(모든 view 공유, manager.rs bump_version).
//     모든 snapshot 이 같은 전역 version 을 박는다 → "view_id 가 다르면 무조건 채택"식 가드는 틀렸다.
//     다른 view 의 늦은 emit(낮은 전역 version)이 active 를 stale 로 덮거나, split_slot 이 active 무관하게
//     해당 view 스냅샷을 emit 하므로 비-active view 가 메인 캔버스를 가로챌 수 있다(F1+F4).
//   - 캐시는 view_id 별 독립 항목이라 다른 view 끼리 version 이 충돌하지 않는다. 같은 view 안에서는
//     전역 단조라 version 단조 비교가 그대로 성립 → stale emit 폐기가 view 별로 정확하다.
//   - switch_view 는 active_view_id 만 바꾼다(layout 불변). 캐시 모델에선 active 만 바뀌면 *이미 캐시된*
//     그 view layout 이 즉시 렌더된다 → F4·switch 순서·stale 덮어쓰기가 한 번에 닫힌다.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { create } from 'zustand'

import type { LayoutNode, SplitDir, ViewMeta, ViewSnapshot } from '../api/layoutTypes'
import { isRenderMode, type RenderMode } from '../components/slot/renderMode'

/** view:list-updated 페이로드(ViewListPayload 미러, commands/layout.rs). */
interface ViewListPayload {
  views: ViewMeta[]
  active_view_id: string
}

/** view_id 별 캐시 항목 — 그 view 가 마지막으로 채택한 레이아웃 + focus + (전역 단조) version. */
export interface CachedView {
  layout: LayoutNode
  focusedSlotId: string | null
  /** 이 항목을 마지막으로 채택한 전역 version(stale emit 가드 — 같은 view 안에서 단조 비교). */
  version: number
}

interface ViewState {
  /** view_id → 캐시된 레이아웃. 메인 창 렌더는 ★activeViewId 항목만★ 쓴다(active-only). */
  layouts: Record<string, CachedView>
  /** 탭 바용 view 목록(view:list-updated). */
  views: ViewMeta[]
  /** 메인 창 활성 탭(view:list-updated / 첫 layout emit). 렌더 대상 캐시 키. */
  activeViewId: string | null

  /**
   * ★M0 스파이크(임시) — ADR-0044★: fixture 로 구동되는 RichSlot(구조화 JSON 렌더)을 띄운 slot_id 집합.
   * ★프론트 전용 오버레이★ — 백엔드 wire LayoutNode 는 rich 개념을 모른다(M2 에서 transport caps 로
   * xterm↔RichSlot 를 정식 분기하기 전까지의 임시 마킹). 그래서 이 필드만은 invoke→emit 권위 루프를
   * 타지 않는다(위 "낙관적 갱신 X" 규칙의 스파이크 한정 예외). 실슬롯 콘텐츠(agent_id)는 불변.
   */
  richSlots: Record<string, true>

  /**
   * ★렌더 모드 오버라이드(§5)★: slot node.id → 강제 RenderMode. caps 유도 기본 렌더러
   * (defaultRenderMode: structured→'rich' / else→'terminal')를 무시하고 지정한 렌더러를 마운트한다.
   * 미지정 slot 은 여기 키가 없어 기본 유도로 떨어진다(?? defaultRenderMode).
   * ★프론트 전용★ — richSlots 와 동형으로 백엔드 wire LayoutNode 는 이 개념을 모른다(override라 권위
   * 레이아웃과 무관). 그래서 이 필드도 invoke→emit 권위 루프를 안 탄다(richSlots 와 같은 예외).
   */
  renderModeOverride: Record<string, RenderMode>

  /** 새 view 생성 → active. 반환 = 새 view_id(이걸로 이후 split 대상 지정). */
  createView: (name?: string) => Promise<string>
  /** view 닫기. active 면 다른 view 로 전환. */
  closeView: (viewId: string) => Promise<void>
  /** 메인 창 활성 탭 변경. */
  switchView: (viewId: string) => Promise<void>
  /** slot 분할 → 새 slot_id 반환. */
  split: (viewId: string, slotId: string, dir: SplitDir) => Promise<string>
  /** slot 닫기(형제 승격). */
  closeSlot: (viewId: string, slotId: string) => Promise<void>
  /** slot 에 agent 참조 배정. */
  assignAgent: (viewId: string, slotId: string, agentId: string) => Promise<void>

  // ── ★M0 스파이크(임시) — ADR-0044★ RichSlot 오버레이 마운트/해제(프론트 전용, invoke 안 탐) ──────
  /** slot 에 RichSlot(fixture 구동 JSON 모드) 스파이크를 띄운다. */
  mountRich: (slotId: string) => void
  /** slot 의 RichSlot 스파이크를 걷는다(다시 empty 로). */
  unmountRich: (slotId: string) => void

  // ── ★렌더 모드 오버라이드(§5)★ 지정/해제(프론트 전용, invoke 안 탐) ──────────────────────────
  /** slot 의 렌더러를 mode 로 강제(caps 유도 기본을 덮음). */
  setRenderMode: (nodeId: string, mode: RenderMode) => void
  /** slot 의 오버라이드 해제(caps 유도 기본 렌더러로 복귀). */
  clearRenderMode: (nodeId: string) => void

  // ── ★DOM 모드 얇은 별칭(§5 관측)★: setRenderMode/clearRenderMode 위 래퍼(검증 툴링이 이 이름을 씀) ──
  /** slot 을 DOM 모드로(= setRenderMode(id,'dom')). */
  enableDomMode: (nodeId: string) => void
  /** slot 의 DOM 모드 해제(= clearRenderMode(id)). */
  disableDomMode: (nodeId: string) => void
  /** slot 의 DOM 모드 토글(dom ↔ 기본). */
  toggleDomMode: (nodeId: string) => void

  // ── emit 수신 핸들러(eventBus 가 listen 콜백에서 호출) ───────────────────────────
  /** layout:updated 수신 — 그 view_id 캐시 항목을 version 가드 통과 시 갱신. */
  applyLayoutUpdated: (snap: ViewSnapshot) => void
  /** view:list-updated 수신 — 탭 목록/active 갱신. */
  applyViewListUpdated: (payload: ViewListPayload) => void

  // ── 부팅 초기화(eventBus 가 구독 등록 직후 1회 호출) ──────────────────────────────
  /** 부팅 시 백엔드의 현재 View 목록+active 를 pull 해 채우고, active 뷰 레이아웃을 캐시에 넣는다. */
  initFromBackend: () => Promise<void>
}

export const useViewStore = create<ViewState>((set, get) => ({
  layouts: {},
  views: [],
  activeViewId: null,
  richSlots: {}, // ★스파이크★ 프론트 전용 오버레이(ADR-0044 M0) — 아래 mountRich/unmountRich 로만 갱신.
  renderModeOverride: {}, // ★오버라이드★ 프론트 전용(§5) — 아래 set/clearRenderMode(+DOM 별칭)로만 갱신.

  createView: viewName => invoke<string>('create_view', { name: viewName ?? null }),
  closeView: viewId => invoke<void>('close_view', { viewId }),
  switchView: viewId => invoke<void>('switch_view', { viewId }),
  split: (viewId, slotId, dir) => invoke<string>('split_slot', { viewId, slotId, dir }),
  closeSlot: (viewId, slotId) => {
    // slot 이 사라지므로 그 slot 의 렌더 오버라이드 엔트리도 즉시 제거(누수 방지 — 프론트 전용 상태라
    // invoke→emit 권위 루프를 안 타는 richSlots/renderModeOverride 는 여기서 낙관적으로 정리한다).
    get().clearRenderMode(slotId)
    return invoke<void>('close_slot', { viewId, slotId })
  },
  assignAgent: (viewId, slotId, agentId) => {
    // slot UUID 는 재배정에도 안정(agent_id 만 바뀐다) → 이전 agent 를 위해 건 오버라이드가 새 agent 에
    // 조용히 적용되면 안 된다. 그래서 assign 시 그 slot 의 오버라이드를 clear 해 새 agent 는 caps 유도
    // 기본으로 시작하게 한다(프론트 전용 낙관 갱신 — richSlots 와 동형, 권위 루프 밖).
    get().clearRenderMode(slotId)
    return invoke<void>('assign_agent', { viewId, slotId, agentId })
  },

  // ★M0 스파이크 예외★: 다른 액션과 달리 invoke 를 안 부른다 — 백엔드가 rich 개념을 모르므로(M2 caps
  // 정식화 전) "권위=백엔드·낙관 갱신 X" 규칙의 스파이크 한정 예외로 프론트 상태를 직접 set 한다.
  mountRich: slotId => set(state => ({ richSlots: { ...state.richSlots, [slotId]: true } })),
  unmountRich: slotId =>
    set(state => {
      const next = { ...state.richSlots }
      delete next[slotId]
      return { richSlots: next }
    }),

  // ★렌더 모드 오버라이드도 richSlots 와 같은 프론트 전용 예외★: invoke 안 부르고 프론트 상태만 set
  // (override 라 백엔드 권위 레이아웃과 무관).
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
    // 다르면 무조건 채택" 같은 분기가 필요 없다(그 분기가 F1+F4 의 뿌리였다: 다른 view 의 낮은 전역
    // version emit 이 active 를 stale 로 덮거나 비-active view 가 메인 캔버스를 가로챔). 첫 수신은 캐시에
    // 항목이 없어 자동 채택(version === undefined 비교).
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

  applyViewListUpdated: payload => {
    // 탭 목록·active 를 미러(active-only 렌더 메커니즘은 //! 헤더 캐시 모델 참조).
    set({ views: payload.views, activeViewId: payload.active_view_id })
  },

  // 부팅 init — 왜 필요한가: ViewManager 는 부팅 시 기본 View("View 1")를 자동 생성하지만 그 생성은
  // *부팅 전*이라 emit 으로 닿지 않는다(변경 핸들러는 변경 직후에만 emit). 그래서 webview 는 active
  // view id 를 발견할 경로가 없어 첫 createView/split 전까진 화면이 빈다. read-only 조회(list_views)로
  // 목록+active 를 채운 뒤, active 뷰의 레이아웃(get_view)을 받아 ViewLayoutRenderer 가 부팅 즉시 그리게 한다.
  // ★유령 View 생성 없이★ — pull 만(version 안 올림, broadcast 없음). 구독을 먼저 걸어둔 뒤 호출되므로
  // init 도중 들어온 emit(layout:updated)이 더 최신이면, 이 pull 결과를 applyLayoutUpdated 의 캐시 version
  // 비교가 걸러낸다(옛 pull 이 새 emit 을 덮지 않음, F2) — init 도 동일 가드/캐시를 탄다.
  //
  // ★init race 가드(2차 리뷰)★: layout 차원은 위 version 가드가 막지만 list/active 차원엔 가드가 없다
  // — applyViewListUpdated 는 activeViewId/views 를 그냥 set 한다. list_views 와 get_view 두 await 사이에
  // 외부 emit(view:list-updated/layout:updated)이 도착해 active/list 를 더 최신으로 바꿔두면, 늦게 끝난
  // 이 stale init 의 list payload 가 그 새 상태를 덮을 수 있다. 그래서 init 시작 시 generation 을 잡고
  // (markExternalViewEvent 는 listen 핸들러에서만 호출돼 "내가 시작한 이후 외부 emit 이 있었나"를 기록),
  // 적용 직전 superseded 면 list payload 적용을 건너뛴다(one-shot init 이라 단순 generation 가드로 충분).
  initFromBackend: async () => {
    const myGen = beginInitGeneration()
    const payload = await invoke<ViewListPayload>('list_views')
    // 두 await 사이/직전에 외부 emit 이 active/list 를 갱신했으면 이 stale list 적용을 건너뛴다.
    // ★layout 차원(아래 get_view pull)은 가드를 안 건다★ — 막는 주체는 *active-only 렌더*다(FIX-5 정정).
    // stale active_view_id(=v1)로 get_view 를 불러 v1 캐시에 낡은 snapshot 이 들어가도, 메인 창은
    // activeViewId(이미 외부 emit 으로 v2)의 캐시만 렌더하므로 비-active(v1) 캐시는 화면에 안 뜬다.
    // (version 가드는 *같은 view_id 내* stale 만 막지, "stale active_view_id 로 get_view 호출" 자체는
    // 못 막는다 — 그건 active-only 가 흡수한다. 그래서 layout 엔 별도 가드 불필요.)
    if (!isInitSuperseded(myGen)) get().applyViewListUpdated(payload)
    const snap = await invoke<ViewSnapshot>('get_view', { viewId: payload.active_view_id })
    get().applyLayoutUpdated(snap)
  },
}))

// ── init race 가드 상태(프론트 전용, one-shot init) ──────────────────────────────────────
// initGeneration: init 시작마다 +1. lastExternalEventGen: 그 generation 동안 외부 view emit 을 봤으면
// 그 값으로 박힌다. isInitSuperseded(gen) = init 시작 이후 외부 emit 이 있었나(둘이 같으면 supersede).
// ★markExternalViewEvent 는 listen 핸들러(subscribeViewEvents)에서만 호출★ — init 자신의 직접 apply
// 호출은 마킹하지 않아 자기 자신을 supersede 로 오인하지 않는다(외부 emit 만 "더 최신"의 정의).
let initGeneration = 0
let lastExternalEventGen = -1

function beginInitGeneration(): number {
  return ++initGeneration
}
/** init 시작(myGen) 이후 외부 view emit 이 있었으면 true → 이 stale init 의 list payload 적용을 건너뛴다. */
function isInitSuperseded(myGen: number): boolean {
  return lastExternalEventGen >= myGen
}
/** listen 핸들러가 외부 emit 수신 시 호출 — 진행 중 init 을 supersede 로 표시(가장 최근 generation 기준). */
function markExternalViewEvent(): void {
  lastExternalEventGen = initGeneration
}

/** 현재 active view 의 캐시 항목(없으면 null) — 메인 창 렌더 selector 의 단일 출처(active-only). */
export function selectActiveView(state: ViewState): CachedView | null {
  return state.activeViewId ? (state.layouts[state.activeViewId] ?? null) : null
}

// ── emit 구독 등록(eventBus 에서 1회 호출, HMR/중복 가드는 eventBus 가 dispose 로 관리) ──────────
// 반환 = `{ dispose, ready }`(동기). 호출자는 dispose 를 await 없이 *즉시* 손에 쥐고(눈수 0 — 아래),
// ready 를 await 한 뒤에야 init 을 돌린다(F-listen). listen() 은 async 라 등록이 끝나기 전 도착한 emit 은
// 핸들러가 없어 누락된다 → 등록 완료를 ready 로 노출해 호출자가 그 뒤에 initFromBackend 를 부른다.
//
// ★왜 동기 dispose 인가(이전 dead-branch 누수 수정)★: 예전엔 `await Promise.all([listen,listen])` 한 뒤에야
// 동기 disposer 를 반환했다. 그래서 호출자(eventBus)가 그 await 가 pending 인 동안 정리(HMR dispose/재-init)를
// 돌리면, 아직 disposer 를 *못 받아* unlisten 을 못 걸고 → 늦게 등록 완료된 리스너가 영구 누수됐다(예전
// if(disposed) 가드는 disposed 를 true 로 만드는 게 그 반환된 disposer 뿐이라 await 중엔 절대 true 가 안 되는
// dead branch 였다). 표준 해법(RxJS Subscription/useSyncExternalStore: teardown 핸들 동기 확보)대로,
// dispose 를 등록 await 없이 즉시 돌려주고 등록은 백그라운드로 시작한다. 근거: docs/research/async-subscribe-cleanup-race-2026-06-28.md.
//
// ★race 가드(실제로 동작하는 경로)★: dispose 가 ready *전에* 불려 disposed=true 가 되면, 백그라운드 등록이
// 늦게 끝나 도착한 unlisten 핸들을 등록 콜백 안에서 disposed 확인 후 *즉시* 호출한다(cancelled 무관 — 안
// 부르면 누수). 이번엔 disposed 를 set 하는 dispose 가 await 없이 반환돼 등록 완료 시점에 이미 true 일 수
// 있으므로 분기가 실제로 탄다(예전 dead branch 와 대비). 부분 등록(한쪽 성공·다른쪽 reject)도 성공분을 정리.
// ★view emit 핸들러는 markExternalViewEvent 를 호출★ — 외부 emit 만 진행 중 init 을 supersede 로 표시
// (init race 가드). init 자신의 직접 apply 는 이 경로를 안 거쳐 자기 supersede 오인이 없다.
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
  // reject 한다(hang 금지 — 호출자 await ready 가 막히지 않게). dispose 가 먼저 와도 adopt 가 도착한 핸들을
  // 즉시 해제하므로 ready 는 그대로 정상 종료한다.
  //
  // ★계약 ④ — ready reject 시 "성공한 쪽 핸들 정리"는 *호출자의 dispose 호출 책임*★(Codex C3): Promise.all
  // 이 한쪽 reject 로 즉시 reject 해도, 다른 쪽 listen 은 백그라운드로 계속 등록될 수 있다(아직 settle 전).
  // 이 함수는 그 늦은 성공 핸들을 *자동으로* 정리하지 않는다 — adopt 가 disposed 면 즉시 해제하므로, 호출자가
  // ready reject 를 catch 해 dispose() 만 불러주면 (이미 도착한 성공분은 handles 에서, 늦게 도착할 성공분은
  // adopt 의 disposed 분기에서) 모두 해제된다. 즉 dispose 호출이 정리의 트리거다(eventBus FIX-2 가 그렇게 한다).
  const ready = Promise.all([
    listen<ViewSnapshot>('layout:updated', e => {
      markExternalViewEvent()
      store.applyLayoutUpdated(e.payload)
    }).then(adopt),
    listen<ViewListPayload>('view:list-updated', e => {
      markExternalViewEvent()
      store.applyViewListUpdated(e.payload)
    }).then(adopt),
  ]).then(() => undefined)

  const dispose = (): void => {
    disposed = true
    // 이미 등록 완료돼 handles 에 담긴 핸들을 모두 해제하고 비운다 → 두 번 불려도(idempotent) 빈 배열이라
    // noop, 이미 해제한 핸들을 재호출하지 않는다. ready 전이라 아직 비어 있으면, 늦게 도착할 핸들은 위
    // adopt 의 disposed 분기가 즉시 해제한다.
    while (handles.length > 0) handles.pop()!()
  }

  return { dispose, ready }
}
