// 앱 전역 에이전트 이벤트 배선.
//
// Tauri 이벤트를 직접 듣지 않고 agentClient 의 이벤트 구독 메서드를 소비한다.

import { agentClient } from '../api/clientFactory'
import { list as cmdList, run as cmdRun } from '../commands/registry'
import { useAgentStore } from './agentStore'
import { initMainWindowFromBackend, subscribeViewEvents } from './viewStore'

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
      // §5: command 레지스트리 제어 표면(ADR-0055) — 사람 클릭·전역 keydown 과 동일한 단일 진입점을
      //   LLM(cdp eval)이 부른다.
      //   ★새 전역 핸들을 만들지 말 것★: 레이아웃·탭·창·렌더모드는 전부 여기 등록된
      //   tab.*·window.*·slot.*·layout.setSlotContent 로 부른다. 핸들을 나란히 두면 같은 액션에 표면이
      //   둘이 되고, 한쪽만 고친 변경이 조용히 갈라진다.
      //   ★단 표면이 이미 하나인 것은 아니다★ — 버스 밖 전역 핸들이 아직 남아 있다. 여기를 읽고 "이제
      //   단일 표면"이라고 결론내지 말 것. 정책 정본 = CLAUDE.md 「LLM-우선 제어」, 살아 있는 대입만 뽑는
      //   법 = `rg "\)\.__ENGRAM_|\)\.__engram" src/ -g '!*.test.*'`(4줄 — 그중 `__engramCmd` 가 여기다.
      //   `).__NAME` 앵커를 빼면 타입 표기·주석까지 걸려 38줄이 된다). ★이 자리에 명단을 적지 말 것 —
      //   낡는다★. ★그 출력을 정의부 주석으로 판정하지 말 것★ — 남은 것 중 일부는 자기를 「정식 §5 표면」
      //   으로 소개해서, 주석만 보면 곁문인지 알 수 없다(판정은 CLAUDE.md 그 절이 한다).
      //   ★레지스트리는 상태 권위가 아니다★ — handler 가 기존 store 액션/invoke 로 라우팅한다(ADR-0035
      //   유지). run 은 handler 반환(일부 Promise)을 그대로 흘려보내 cdp eval 에서 await 가능.
      //   window.__engramCmd.run('slot.empty', { viewId, slotId })  // 실행(모르는 id 는 throw)
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
          // ★여기서 __engramCmd 를 지우지 않는다★: 재설치는 initEventBus 안에서 일어나는데 App 은
          // bootstrapDaemonIfNeeded 를 먼저 await 하므로(App.tsx), 지우면 그 사이 cdp 호출이 undefined 를
          // 만나고 데몬이 흔들리면 재시도 지연만큼 길어진다. HMR 세션이 옛 핸들을 문 채 남는 것은 알려진
          // dev 전용 동작이다 — 전체 새로고침이 푼다.
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
