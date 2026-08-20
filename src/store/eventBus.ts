// 앱 전역 에이전트 이벤트 배선.
//
// Tauri 이벤트를 직접 듣지 않고 agentClient 의 이벤트 구독 메서드를 소비한다.

import { agentClient } from '../api/clientFactory'
import { list as cmdList, run as cmdRun } from '../commands/registry'
import { useAgentStore } from './agentStore'
import { CHAT_STYLE_DEFAULTS, useChatStyleStore, type ChatStyleKey } from './chatStyleStore'
import {
  currentViewId,
  initMainWindowFromBackend,
  readWindowLabelFromHash,
  subscribeViewEvents,
  useViewStore,
} from './viewStore'

let unlistenFns: (() => void)[] = []
// StrictMode 이중마운트 레이스 방지
let initPromise: Promise<void> | null = null

/**
 * 프로필 목록 갱신(ADR-0018). 라이브 반영은 ProfileListUpdated broadcast 가 담당하고, 이 pull 은
 * 부팅 1회 + 재연결 resync + create/delete/activate(spawnProfile) 직후(broadcast 유실 대비) 호출된다.
 * (실행중 전환 자체는 agent-list-updated 가 처리 — 여기선 예약 목록만 새로 받는다.)
 */
export async function refreshProfiles(): Promise<void> {
  try {
    const profiles = await agentClient.listProfiles()
    useAgentStore.getState().setProfiles(profiles)
  } catch (err) {
    console.warn('[eventBus] refreshProfiles failed:', err)
  }
}

/**
 * 프리셋 목록 갱신(ADR-0061 — refreshProfiles 미러). 부팅 1회 + 재연결 resync 에서 호출해 store 미러를
 * 권위 목록으로 동기화한다. create/delete 직후 반영은 PresetListUpdated broadcast 가 담당(별도 pull 불필요).
 */
export async function refreshPresets(): Promise<void> {
  try {
    const presets = await agentClient.listPresets()
    useAgentStore.getState().setPresets(presets)
  } catch (err) {
    console.warn('[eventBus] refreshPresets failed:', err)
  }
}

/**
 * 재연결 직후 목록/프로필 재동기화(Q2). connected *재*전이에서만 호출(첫 연결 제외 — initEventBus
 * 의 lastState 가드). 권위 목록을 다시 끌어와 store 를 새로 쓴다 → 끊긴 동안 변경(spawn/kill/프로필) 반영.
 *
 * ★이 getAgents 는 출력 복구의 **주 경로**이기도 하다(ADR-0046 amend)★: 끊긴 뷰는 detached 로 앉아
 * 아무것도 보내지 않고, 그 명부에 자기 에이전트가 있다는 관측에서만 다시 붙는다
 * (protocolClient.observeRoster). 여기서 명부를 안 끌어오면 재연결 뒤 슬롯이 영영 안 붙는다 — "목록만
 * 새로 그리는 편의 기능"으로 읽고 지우지 말 것.
 *
 * ★실패는 삼킨다(사용자 결정 2026-08-20)★: 재시도 사다리를 두지 않는다. 수동 재연결이 이 경로를
 * 통째로 다시 돌리므로 탈출구가 이미 있다.
 */
async function resyncAfterReconnect(): Promise<void> {
  try {
    const agents = await agentClient.getAgents()
    useAgentStore.getState().setAgents(agents)
  } catch (err) {
    console.warn('[eventBus] resync getAgents failed:', err)
  }
  await refreshProfiles()
  await refreshPresets()
}

export function initEventBus(): Promise<void> {
  if (initPromise) return initPromise

  initPromise = (async () => {
    try {
      // §5: 레이아웃 제어 표면을 window에 노출 → LLM(cdp eval 등)이 사람 UI와 동일한 단일 진입점을
      // 호출한다. ★레이아웃 권위 = src-tauri(ADR-0035/0057)★.
      // 정식 command 버스 전까지의 임시 경로(CLAUDE.md §5 임시 경로 항).
      // ★렌더 모드 오버라이드(§5)★: 슬롯 렌더러(터미널/rich/dom)를 강제하는 프론트 전용 override.
      // 백엔드 invoke 를 안 타고 viewStore 프론트 상태만 흔든다(override라 권위 레이아웃과 무관).
      //   window.__engramLayout.clearRenderMode('<nodeId>')   // 해제(caps 유도 기본 복귀)
      // ★DOM 모드 별칭★: 평문 DOM(<pre>)로 렌더시켜 CDP eval/innerText 로 출력을 읽히게 한다(터미널
      // xterm 은 canvas 라 innerText 로 안 읽힘). set/clearRenderMode 위 얇은 래퍼 — 검증 툴링이 이 이름을 씀.
      //   window.__engramLayout.toggleDomMode('<nodeId>')   // slot node.id(=data-slot-id) 로 켬/끔(dom↔기본)
      // ★탭 소유 모델(ADR-0057)★: 탭/창 조작은 창 label 을 받는 탭-언어 표면이다.
      ;(globalThis as Record<string, unknown>).__engramLayout = {
        // 탭/창 command(window 생략 시 이 웹뷰 창).
        createTab: (window?: string, name?: string) =>
          useViewStore.getState().createTab(window ?? readWindowLabelFromHash(), name),
        closeTab: (viewId: string, window?: string) =>
          useViewStore.getState().closeTab(window ?? readWindowLabelFromHash(), viewId),
        switchTab: (viewId: string, window?: string) =>
          useViewStore.getState().switchTab(window ?? readWindowLabelFromHash(), viewId),
        createWindow: useViewStore.getState().createWindow,
        closeWindow: useViewStore.getState().closeWindow,
        split: useViewStore.getState().split,
        closeSlot: useViewStore.getState().closeSlot,
        assignAgent: useViewStore.getState().assignAgent,
        // ★슬롯 콘텐츠 배치(§5, ADR-0063)★: assignAgent 미러. 트리(agent_list)·팔레트(preset_palette)·
        //   비우기(empty)·에이전트 배정을 SlotContent 유니온으로 교체한다.
        //   invoke→emit 권위 루프(ADR-0035 낙관 갱신 X). __engramCmd.run('layout.setSlotContent') 와 병행 경로.
        setSlotContent: useViewStore.getState().setSlotContent,
        // ★슬롯 이동(§5)★: LLM 이 slot id 만으로 부를 수 있게 원본 viewId 를 currentViewId() 로 해소한다.
        //   toWindow 미지정 → 새 팝업 창. 팝업 창 안에서 호출되면 그 창의 active 탭으로 떨어져, 팝업 안
        //   LLM/CDP 가 엉뚱한 main view 를 집는 것을 막는다. slotId 가 그 view 밖이면 백엔드가
        //   SlotNotFound Err(방어). 명시적 (viewId, slotId, toWindow) 호출은 viewStore.moveSlotToWindow 직접.
        moveSlotToWindow: (slotId: string, toWindow?: string) => {
          const viewId = currentViewId()
          if (!viewId) return Promise.reject(new Error('view id 미확정 — move 대상 뷰 없음'))
          return useViewStore.getState().moveSlotToWindow(viewId, slotId, toWindow)
        },
        setRenderMode: useViewStore.getState().setRenderMode,
        clearRenderMode: useViewStore.getState().clearRenderMode,
        enableDomMode: useViewStore.getState().enableDomMode,
        disableDomMode: useViewStore.getState().disableDomMode,
        toggleDomMode: useViewStore.getState().toggleDomMode,
      }

      // ★채팅 스타일 control surface(§5, ADR-0051)★: 채팅 렌더 간격·폰트 토큰을 LLM 이 사람 UI 와
      //   동일한 store 액션(chatStyleStore)으로 조작한다. 프론트 전용 권위 + localStorage 영속. 값은 :root
      //   CSS 변수로 반영돼 StructuredTextView/chat.css 가 var() 로 읽는다.
      //   ★로드+적용은 여기가 아니라 main.tsx 최상단(loadAndApplyChatStyle)★ — 데몬 bootstrap 경로에
      //   의존하지 않도록 분리했다(FIX-1). 여기선 핸들만 노출한다(핸들 노출과 값 로드는 독립).
      ;(globalThis as Record<string, unknown>).__engramChat = {
        get: () => useChatStyleStore.getState().values,
        set: (key: ChatStyleKey, value: string) => useChatStyleStore.getState().setValue(key, value),
        patch: useChatStyleStore.getState().patch,
        reset: useChatStyleStore.getState().reset,
        defaults: CHAT_STYLE_DEFAULTS,
      }

      // §5: command 레지스트리 제어 표면(ADR-0055) — 사람 클릭·전역 keydown 과 동일한 단일 진입점을
      //   LLM(cdp eval)이 부른다.
      //   ★레지스트리는 상태 권위가 아니다★ — handler 가 기존 store 액션/invoke 로 라우팅한다(ADR-0035
      //   유지). run 은 handler 반환(일부 Promise)을 그대로 흘려보내 cdp eval 에서 await 가능.
      //   window.__engramCmd.run('theme.set', { theme:'light' })  // 실행(모르는 id 는 throw)
      // ★전체 command 를 window 에 노출하는 것은 의도적이다(WONTFIX)★: CLAUDE.md §5(모든 기능은 LLM 제어
      //   가능해야 한다) / ADR-0055 의 설계 요구다. "allowlist 로 일부만 노출" 대안은 §5(LLM 이 메인 조작
      //   주체)와 정면 충돌해 기각됐다. 이 표면은 보안 취약점이 아니라 제어 계약이다(리뷰어 재제기 방지 앵커).
      ;(globalThis as Record<string, unknown>).__engramCmd = {
        list: cmdList,
        run: cmdRun,
      }

      // HMR 재평가 시 기존 구독 먼저 해제
      if (unlistenFns.length > 0) {
        unlistenFns.forEach(fn => fn())
        unlistenFns = []
      }

      // 레이아웃 emit 구독(layout:updated / window:tabs-updated). agentClient 이벤트와 달리 src-tauri
      // 권위라 @tauri-apps/api listen 직접 사용(viewStore.subscribeViewEvents).
      // ★dispose 를 await 없이 즉시 push★(누수 가드): dispose 를 먼저 unlistenFns 에 넣어둬야, 아래 ready
      // await 가 pending 인 동안 정리(HMR dispose/재-init)가 unlistenFns.forEach 를 돌려도 이 dispose 가
      // 포함돼 늦게 끝난 등록이 누수되지 않는다(예전엔 await 완료 후에야 disposer 를 push 해 이 윈도에서
      // 영구 누수됐다).
      const viewSub = subscribeViewEvents()
      unlistenFns.push(viewSub.dispose)

      // ★HMR dispose 콜백을 ready await *전*에 등록★(누수 가드의 마지막 고리): 이 콜백이 unlistenFns
      // (이미 viewSub.dispose 포함)를 정리한다. 만약 아래 `await viewSub.ready` *뒤*에 등록하면, ready 가
      // pending 인 동안 HMR 이 와도 콜백이 아직 안 걸려 viewSub.dispose 를 부를 경로가 없다 → 늦게 등록
      // 완료된 layout 리스너가 누수된다.
      // 클로저가 참조하는 unlistenFns/initPromise 는 모듈 스코프 let 이라 위치 이동 후에도 최신 값을 읽는다.
      if (import.meta.hot) {
        import.meta.hot.dispose(() => {
          unlistenFns.forEach(fn => fn())
          unlistenFns = []
          initPromise = null
        })
      }

      // ★등록 완료를 await★(F-listen): listen() 은 async 라 등록이 끝나기 전 도착한 init pull 결과나 백엔드
      // emit 은 핸들러가 없어 누락된다. ready 를 기다린 뒤에야 initMainWindowFromBackend 를 부른다. ready 는
      // dispose 가 먼저 와도·등록이 실패해도 정상 종료(hang 금지)하므로 이 await 가 막히지 않는다.
      //
      // ★layout 구독 실패를 agentClient 구독과 격리★(fate-sharing 차단): ready 가 reject 하면(한쪽 listen
      // 등록 IPC 실패) layout(ADR-0035 권위)만 실패한 것이지 agentClient(ADR-0011 권위, 트리·상태바·재연결)는
      // 무관하다. catch 하지 않으면 아래 onAgentListUpdated/onStatusChanged/... 등록이 통째로 안 돼 도메인이
      // 죽는다. catch 의 viewSub.dispose() 는 성공한 부분 등록분을 정리한다(idempotent + unlistenFns 에도
      // 있어 중복 호출 noop 안전).
      try {
        await viewSub.ready
      } catch (err) {
        console.warn('[eventBus] layout 구독(subscribeViewEvents) 실패 — agentClient 구독은 계속:', err)
        viewSub.dispose()
      }

      // 부팅 init — 백엔드 기본 View 는 부팅 전 생성돼 emit 으로 안 닿으므로(변경 직후만 emit), read-only
      // list_tabs("main")/get_view 로 main 창의 탭+active 레이아웃을 끌어와 화면을 즉시 그린다. ★구독을
      // 먼저 건 뒤★ 호출 — init 도중 들어온 emit 을 놓치지 않고, 더 최신이면 창/캐시 version 가드가 pull
      // 결과를 덮는다(역전 방지). (팝업 창의 탭 상태는 각 WindowLayout 이 mount 시 자기 label 로 pull.)
      // ADR-0102: 최종 실패(initMainWindowFromBackend 가 유계 재시도 소진 후 throw)는 조용한 warn 이
      //   아니라 error 로 표면화한다 — main 은 이벤트 복구 경로가 없어 여기서 신호를 안 남기면 로딩 고착이
      //   원인 불명이 된다. (가시적 UI 에러는 main WindowLayout 이 자기 pull 재시도 소진 시 렌더한다.)
      void initMainWindowFromBackend().catch(err => {
        console.error('[eventBus] initMainWindowFromBackend 최종 실패(재시도 소진):', err)
      })

      unlistenFns.push(
        agentClient.onAgentListUpdated(agents => {
          useAgentStore.getState().setAgents(agents)
        }),
      )

      unlistenFns.push(
        agentClient.onStatusChanged((id, status) => {
          useAgentStore.getState().onStatusChanged(id, status)
        }),
      )

      unlistenFns.push(
        agentClient.onRestoreResult(report => {
          console.info('[restore]', report.agent_id, report.outcome.type, report.outcome)
        }),
      )

      unlistenFns.push(
        agentClient.onProfileListUpdated(profiles => {
          useAgentStore.getState().setProfiles(profiles)
        }),
      )

      // create/delete 후 전 창이 이 이벤트로 store 미러를 교체한다(멀티창 동기화).
      unlistenFns.push(
        agentClient.onPresetListUpdated(presets => {
          useAgentStore.getState().setPresets(presets)
        }),
      )

      // 재연결 시 목록/프로필 재동기화(Q2) + 출력 뷰 재부착의 계기(위 resyncAfterReconnect 주석).
      // 에이전트 트리·프로필 목록은 이 트리거가 없으면 stale 이 된다(끊긴 동안의
      // spawn/kill/프로필 변경 broadcast 를 놓침). connected 로 *재*전이할 때만 권위 목록을 다시
      // 끌어와 store 를 새로 쓴다. ★첫 connected 는 스킵★ — App.tsx 부팅 로드(getAgents/
      // refreshProfiles 1회)와 중복 방지. lastState 가드는 ProtocolClient.lastState 패턴과 동일
      // (prev!=='connected' && cur==='connected'), 초기값은 현재 상태로 둬 첫 통지가 connected 여도
      // 재전이로 오인하지 않는다.
      let lastConn = agentClient.connectionState
      unlistenFns.push(
        agentClient.onConnectionStateChange(state => {
          const prev = lastConn
          lastConn = state
          if (state === 'connected' && prev !== 'connected') {
            void resyncAfterReconnect()
          }
        }),
      )
    } catch (err) {
      console.error('[eventBus] init failed:', err)
      initPromise = null // 고착 방지 — 다음 호출 시 재시도 허용
      throw err
    }
  })()

  return initPromise
}
