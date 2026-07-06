// 앱 전역 에이전트 이벤트 배선 — 앱 시작 시 1회 호출(App.tsx).
// HMR 재평가 시 기존 구독 해제 후 재등록(중복 누적 방지).
//
// Tauri 이벤트를 직접 듣지 않고 agentClient(Embedded/Daemon 싱글톤)의 이벤트 구독 메서드를
// 소비한다 — 두 모드 공통 표면이라 데몬 모드(WS 이벤트)에서도 동일하게 트리·상태바가 갱신된다.

import { agentClient } from '../api/clientFactory'
import { useAgentStore } from './agentStore'
import { CHAT_STYLE_DEFAULTS, useChatStyleStore, type ChatStyleKey } from './chatStyleStore'
import { selectActiveView, subscribeViewEvents, useViewStore } from './viewStore'

let unlistenFns: (() => void)[] = []
// StrictMode 이중마운트 레이스 방지 — 진행 중인 promise가 있으면 재사용
let initPromise: Promise<void> | null = null

/**
 * 프로필 목록 갱신(ADR-0018). 백엔드에 프로필 변경 이벤트가 없으므로,
 * 부팅 1회 + create/delete/activate(spawnProfile) 직후 명시적으로 호출해 store 를 동기화한다.
 * (실행중 전환 자체는 기존 agent-list-updated 가 처리 — 여기선 예약 목록만 새로 받는다.)
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
 * 재연결 직후 목록/프로필 재동기화(Q2). connected *재*전이에서만 호출(첫 연결 제외 — initEventBus
 * 의 lastState 가드). 권위 목록을 다시 끌어와 store 를 새로 쓴다 → 끊긴 동안 변경(spawn/kill/프로필)
 * 반영. 출력 replay 재요청(ProtocolClient 가 connected 전이에서 뷰 buffering 리셋+requestReplay, ADR-0046)은
 * 건드리지 않는다(이미 자동 동작).
 */
async function resyncAfterReconnect(): Promise<void> {
  try {
    const agents = await agentClient.getAgents()
    useAgentStore.getState().setAgents(agents)
  } catch (err) {
    console.warn('[eventBus] resync getAgents failed:', err)
  }
  await refreshProfiles()
}

export function initEventBus(): Promise<void> {
  if (initPromise) return initPromise

  initPromise = (async () => {
    try {
      // §5: 레이아웃 제어 표면을 window에 노출 → LLM(cdp eval 등)이 사람 UI와 동일한 단일 진입점을
      // 호출한다. ★옛 useSlotStore.dispatch(프론트 내부 처리)에서 백엔드 invoke 핸들러로 재연결★
      // (ADR-0035: 레이아웃 권위 = src-tauri). 각 액션은 viewStore → 대응 invoke → 백엔드 emit →
      // listen → 화면 반영 루프를 탄다. createView/split 은 Promise<id> 라 cdp eval 에서 await 가능.
      // 정식 command 버스 전까지의 임시 경로(CLAUDE.md §5 임시 경로 항).
      // ★렌더 모드 오버라이드(§5)★: 슬롯 렌더러(터미널/rich/dom)를 강제하는 프론트 전용 override.
      // richSlots 처럼 백엔드 invoke 를 안 타고 viewStore 프론트 상태만 흔든다(override라 권위 레이아웃과 무관).
      //   window.__engramLayout.setRenderMode('<nodeId>', 'dom'|'rich'|'terminal')  // 렌더러 강제
      //   window.__engramLayout.clearRenderMode('<nodeId>')                          // 해제(caps 유도 기본 복귀)
      // ★DOM 모드 별칭★: 평문 DOM(<pre>)로 렌더시켜 CDP eval/innerText 로 출력을 읽히게 한다(터미널
      // xterm 은 canvas 라 innerText 로 안 읽힘). set/clearRenderMode 위 얇은 래퍼 — 검증 툴링이 이 이름을 씀.
      //   window.__engramLayout.toggleDomMode('<nodeId>')   // slot node.id(=data-slot-id) 로 켬/끔(dom↔기본)
      //   window.__engramLayout.enableDomMode('<nodeId>')   // 켬(= setRenderMode(id,'dom')) · disableDomMode 로 끔
      ;(globalThis as Record<string, unknown>).__engramLayout = {
        createView: useViewStore.getState().createView,
        closeView: useViewStore.getState().closeView,
        switchView: useViewStore.getState().switchView,
        split: useViewStore.getState().split,
        closeSlot: useViewStore.getState().closeSlot,
        assignAgent: useViewStore.getState().assignAgent,
        setRenderMode: useViewStore.getState().setRenderMode,
        clearRenderMode: useViewStore.getState().clearRenderMode,
        enableDomMode: useViewStore.getState().enableDomMode,
        disableDomMode: useViewStore.getState().disableDomMode,
        toggleDomMode: useViewStore.getState().toggleDomMode,
      }

      // ★★★ M0 스파이크(임시) — ADR-0044 RichSlot 배선 ★★★: fixture 로 구동되는 구조화 렌더 슬롯
      // (JSON 모드)을 캔버스 슬롯에 소환하는 임시 제어 표면(§5 — cdp.mjs eval / 콘솔이 정식 command 버스
      // 전까지 쓰는 임시 경로). 백엔드 권위 레이아웃엔 안 닿는 프론트 전용 오버레이(viewStore.richSlots)를
      // 흔든다 — M2(StdioTransport 실스트림 + caps 분기) 서면 이 핸들·오버레이 통째로 제거.
      //   window.__richslot.mountFocused()  // active view 의 focused 슬롯에 RichSlot(fixture) 마운트
      //   window.__richslot.mount('<slotId>')   // 특정 슬롯(생략 시 focused)
      //   window.__richslot.unmount('<slotId>') // 해제(생략 시 focused)
      //   window.__richslot.list()           // 현재 rich 슬롯 id 목록
      //   await window.__richslot.spawnJson('<cwd>')  // ★M2★ json 모드 claude 프로필 생성+spawn → agentId
      //       반환한 agentId 를 __engramLayout.assignAgent(viewId, slotId, agentId) 로 슬롯에 배정하면
      //       ViewLayoutRenderer 가 caps.structured 로 라이브 RichSlot 을 띄운다(전체 E2E cdp 구동 경로).
      const focusedSlotId = (): string | null =>
        selectActiveView(useViewStore.getState())?.focusedSlotId ?? null
      const richslot = {
        mount: (slotId?: string): string | null => {
          const target = slotId ?? focusedSlotId()
          if (!target) {
            console.warn('[richslot] mount 대상 슬롯 없음 (focused slot 없음 — slotId 를 넘기세요)')
            return null
          }
          useViewStore.getState().mountRich(target)
          return target
        },
        mountFocused: (): string | null => richslot.mount(),
        unmount: (slotId?: string): string | null => {
          const target = slotId ?? focusedSlotId()
          if (!target) return null
          useViewStore.getState().unmountRich(target)
          return target
        },
        list: (): string[] => Object.keys(useViewStore.getState().richSlots),
        // ★M2 임시 제어 표면(§5)★: json 모드 claude 프로필을 만들어 곧바로 spawn 하고 agentId 를 돌려준다
        //   — 사람 UI 와 동일한 agentClient 호출(createClaudeProfile 'StreamJson' → spawnProfile)만 쓴다.
        //   정식 command 버스 전까지 cdp/콘솔이 JSON 모드 전체 E2E 를 구동하는 경로. cwd 는 실제 작업
        //   디렉터리를 넘긴다(생략 시 '.'=데몬 cwd). 실패 시 null(콘솔 에러 로깅).
        spawnJson: async (cwd?: string): Promise<string | null> => {
          try {
            const dir = cwd ?? '.'
            const stamp = new Date().toISOString().slice(11, 19)
            const profile = await agentClient.createClaudeProfile(
              `json-${stamp}`,
              dir,
              [],
              [],
              false, // auto_restore=false(부팅 자동 spawn 제외 — Sidebar 예약과 동일)
              'StreamJson',
            )
            await refreshProfiles() // 트리 미러 갱신(Sidebar create 와 동일 후처리)
            const info = await agentClient.spawnProfile(profile.id, false) // resume=false(json 은 fresh, ADR-0044)
            return info.id
          } catch (e) {
            console.error('[richslot.spawnJson]', e)
            return null
          }
        },
      }
      ;(globalThis as Record<string, unknown>).__richslot = richslot

      // ★채팅 스타일 control surface(§5, ADR-0051)★: 채팅 렌더 간격·폰트 토큰을 LLM 이 사람 UI 와
      //   동일한 store 액션(chatStyleStore)으로 조작한다. 프론트 전용 권위 + localStorage 영속. 값은 :root
      //   CSS 변수로 반영돼 StructuredTextView/chat.css 가 var() 로 읽는다.
      //   ★로드+적용은 여기가 아니라 main.tsx 최상단(loadAndApplyChatStyle)★ — 데몬 bootstrap 경로에
      //   의존하지 않도록 분리했다(FIX-1). 여기선 핸들만 노출한다(핸들 노출과 값 로드는 독립).
      //   window.__engramChat.get()                       // 현재 값 스냅샷(ChatStyleValues)
      //   window.__engramChat.set('railRowPt', '1.25rem')  // 단일 키 갱신(+ 적용·저장)
      //   window.__engramChat.patch({ fontSize:'14px', lineHeight:'1.6' })  // 부분 병합 갱신
      //   window.__engramChat.reset()                      // 기본값으로
      //   window.__engramChat.defaults                     // 기본값 참조(키 목록·초기값 확인용)
      ;(globalThis as Record<string, unknown>).__engramChat = {
        get: () => useChatStyleStore.getState().values,
        set: (key: ChatStyleKey, value: string) => useChatStyleStore.getState().setValue(key, value),
        patch: useChatStyleStore.getState().patch,
        reset: useChatStyleStore.getState().reset,
        defaults: CHAT_STYLE_DEFAULTS,
      }

      // HMR 재평가 시 기존 구독 먼저 해제
      if (unlistenFns.length > 0) {
        unlistenFns.forEach(fn => fn())
        unlistenFns = []
      }

      // 레이아웃 emit 구독(layout:updated / view:list-updated). agentClient 이벤트와 달리 src-tauri
      // 권위라 @tauri-apps/api listen 직접 사용(viewStore.subscribeViewEvents). dispose 를 같은
      // unlistenFns 에 모아 HMR/재호출 시 한꺼번에 해제(중복 구독 방지) — 아래 onAgentListUpdated 등과 동일 규율.
      // ★dispose 를 await 없이 즉시 push★(누수 가드): subscribeViewEvents 는 `{ dispose, ready }` 를 동기
      // 반환한다. dispose 를 먼저 unlistenFns 에 넣어둬야, 아래 ready await 가 pending 인 동안 정리(HMR
      // dispose/재-init)가 unlistenFns.forEach 를 돌려도 이 dispose 가 포함돼 늦게 끝난 등록이 누수되지
      // 않는다(예전엔 await 완료 후에야 disposer 를 push 해 이 윈도에서 영구 누수됐다).
      const viewSub = subscribeViewEvents()
      unlistenFns.push(viewSub.dispose)

      // ★HMR dispose 콜백을 ready await *전*에 등록★(누수 가드의 마지막 고리): 이 콜백이 unlistenFns
      // (이미 viewSub.dispose 포함)를 정리한다. 만약 아래 `await viewSub.ready` *뒤*에 등록하면, ready 가
      // pending 인 동안 HMR 이 와도 콜백이 아직 안 걸려 viewSub.dispose 를 부를 경로가 없다 → 늦게 등록
      // 완료된 layout 리스너가 누수된다(dispose 를 일찍 push 해도 그걸 *호출*할 HMR 콜백이 늦으면 무효).
      // 그래서 dispose push 직후·ready await 전에 등록해, ready pending 중 HMR 에서도 정리 경로를 보장한다.
      // 클로저가 참조하는 unlistenFns/initPromise 는 모듈 스코프 let 이라 위치 이동 후에도 최신 값을 읽는다.
      if (import.meta.hot) {
        import.meta.hot.dispose(() => {
          unlistenFns.forEach(fn => fn())
          unlistenFns = []
          initPromise = null
        })
      }

      // ★등록 완료를 await★(F-listen): listen() 은 async 라 등록이 끝나기 전 도착한 init pull 결과나 백엔드
      // emit 은 핸들러가 없어 누락된다. ready 를 기다린 뒤에야 initFromBackend 를 부른다. ready 는 dispose 가
      // 먼저 와도·등록이 실패해도 정상 종료(hang 금지)하므로 이 await 가 막히지 않는다.
      //
      // ★layout 구독 실패를 agentClient 구독과 격리★(fate-sharing 차단): ready 가 reject 하면(한쪽 listen
      // 등록 IPC 실패) layout(ADR-0035 권위)만 실패한 것이지 agentClient(ADR-0011 권위, 트리·상태바·재연결)는
      // 무관하다. catch 하지 않으면 아래 onAgentListUpdated/onStatusChanged/... 등록이 통째로 안 돼 도메인이
      // 죽는다. 그래서 ready 실패는 warn 으로만 로깅하고 init 은 계속 진행한다. catch 에서 viewSub.dispose()
      // 를 명시 호출해 성공한 부분 등록분을 정리한다(idempotent + unlistenFns 에도 있어 중복 호출 noop 안전).
      try {
        await viewSub.ready
      } catch (err) {
        console.warn('[eventBus] layout 구독(subscribeViewEvents) 실패 — agentClient 구독은 계속:', err)
        viewSub.dispose()
      }

      // 부팅 init — 백엔드 기본 View 는 부팅 전 생성돼 emit 으로 안 닿으므로(변경 직후만 emit), read-only
      // list_views/get_view 로 현재 목록+active 레이아웃을 끌어와 화면을 즉시 그린다. ★구독을 먼저 건 뒤★
      // 호출 — init 도중 들어온 emit 을 놓치지 않고, 더 최신이면 캐시 version 가드가 pull 결과를 덮는다(역전 방지).
      void useViewStore.getState().initFromBackend().catch(err => {
        console.warn('[eventBus] viewStore initFromBackend failed:', err)
      })

      // 등록은 sync(disposer 즉시 반환) — await 불필요. agentClient 가 모드별 transport 를 숨긴다.

      // 권위 있는 목록 교체(존재/제거 판정 기준, T-4)
      unlistenFns.push(
        agentClient.onAgentListUpdated(agents => {
          useAgentStore.getState().setAgents(agents)
        }),
      )

      // 개별 status 갱신(뱃지 표시용, 목록 제거 안 함, T-4)
      unlistenFns.push(
        agentClient.onStatusChanged((id, status) => {
          useAgentStore.getState().onStatusChanged(id, status)
        }),
      )

      // 부팅 복원 결과(S9). 현재는 로그만 — UX 배너는 추후.
      unlistenFns.push(
        agentClient.onRestoreResult(report => {
          console.info('[restore]', report.agent_id, report.outcome.type, report.outcome)
        }),
      )

      // 프로필 목록 라이브 갱신(깡통/예약, ADR-0018 후속). daemon 모드는 ProfileListUpdated
      // broadcast 로 동작, embedded 는 후속 backend broadcast 흡수 자리(현재 미도달). store 미러 교체.
      unlistenFns.push(
        agentClient.onProfileListUpdated(profiles => {
          useAgentStore.getState().setProfiles(profiles)
        }),
      )

      // 재연결 시 목록/프로필 재동기화(Q2). 출력 스트림은 ProtocolClient 가 connected 전이에서 뷰
      // buffering 리셋+requestReplay 로 자동 복구하나(ADR-0046), 에이전트 트리·프로필 목록은 재동기화
      // 트리거가 없어 stale 이 된다(끊긴 동안의
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

      // (HMR dispose 콜백은 위 viewSub.dispose push 직후·ready await 전에 등록 — FIX-1: ready pending
      // 중 HMR 에서도 정리 경로 보장. agentClient 핸들들도 같은 unlistenFns 에 모여 그 콜백이 함께 해제한다.)
    } catch (err) {
      console.error('[eventBus] init failed:', err)
      initPromise = null // 고착 방지 — 다음 호출 시 재시도 허용
      throw err
    }
  })()

  return initPromise
}
