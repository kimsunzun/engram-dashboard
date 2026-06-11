# 모듈 8b — Phase 3b: eventBus + agentStore 연결 브리핑 (담당: dcs24, Sonnet)

발신: ed12 (매니저)
근거: `docs/frontend-integration-lld.md` §7(eventBus), §8(교체 체크리스트), tracking T-4.
목적: 백엔드 event(agent-status-changed/agent-list-updated)를 받아 agentStore에 반영. 더미 agents → 실제.

## 1. src/store/eventBus.ts (LLD §7 그대로)

```ts
import { listen } from '@tauri-apps/api/event'
import { useAgentStore } from './agentStore'
import type { AgentStatus, AgentInfo } from '../api/types'

let unlistenFns: (() => void)[] = []

export async function initEventBus() {
  // HMR 재평가 시 기존 리스너 먼저 해제 후 재등록
  if (unlistenFns.length > 0) { unlistenFns.forEach(fn => fn()); unlistenFns = [] }

  unlistenFns.push(
    await listen<{ id: string; status: AgentStatus }>('agent-status-changed', (e) => {
      useAgentStore.getState().onStatusChanged(e.payload.id, e.payload.status)
    })
  )
  unlistenFns.push(
    await listen<AgentInfo[]>('agent-list-updated', (e) => {
      useAgentStore.getState().setAgents(e.payload)
    })
  )

  if (import.meta.hot) {
    import.meta.hot.dispose(() => { unlistenFns.forEach(fn => fn()); unlistenFns = [] })
  }
}
```

## 2. agentStore.ts — 더미 → 실제 (§8)

기존 `dummyAgents` 하드코딩 제거. 추가:
- `setAgents(agents: AgentInfo[])` — agent-list-updated로 전체 교체. **이게 권위 있는 목록(존재/제거 판정 기준).**
- `onStatusChanged(id, status)` — 해당 agent의 status만 갱신(없으면 무시). **표시용.**
- 초기 로드: 앱 시작 시 `ptyApi.getAgents()` → setAgents.

> **★T-4 (중요)★:** terminal(종료) 판정은 **agent-list-updated(목록에서 사라짐)** 로만 한다.
> `onStatusChanged`로 받은 Killed/Exited는 **뱃지 표시용**일 뿐 — 이걸로 agent를 목록에서 제거하지 말 것.
> (kill의 Exiting 알림과 drain의 terminal 알림이 lock 밖 동시 발생 → 수신 역순 가능. 목록은 manager의 agent_list_updated가 정정.)
> 기존 UI 더미 필드(cost 등)는 AgentInfo에 없으니 store에서 별도 관리하거나 UI 단에서 optional 처리.

## 3. App.tsx / main.tsx

- 앱 마운트 시 `initEventBus()` 1회 호출 + 초기 `getAgents()` 로드.
- StrictMode 이중 마운트 주의 — initEventBus는 내부적으로 기존 리스너 해제하므로 안전하나, useEffect cleanup 고려.

## 규칙·품질

- eventBus는 앱 전역 1회. 위치는 src/store/eventBus.ts.
- T-4 주석 필수(왜 목록 기준으로 판정하는지).
- 기존 컴포넌트(AgentTree 등)가 store 구독 중이면 깨지지 않게 — AgentInfo.status는 {type} 객체이므로 AgentTree의 status 색상 로직이 문자열 가정이면 조정 필요(3b 범위 내 최소 수정 or 3c로 메모).

## 검증 & 보고

- `npx tsc --noEmit` 통과. `npm run tauri dev` 떠서 콘솔 에러 없이 마운트되는지(아직 agent 없으니 빈 목록 정상).
- 보고: `orch 12 "⟁dcs24 Phase3b 완료 — eventBus+agentStore 실제연결, T-4 반영, tsc/마운트 OK"`

막히면 30분 내 중간보고. agentStore 더미 구조와 충돌 크면 질문.
