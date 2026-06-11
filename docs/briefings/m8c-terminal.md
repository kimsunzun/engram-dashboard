# 모듈 8c — Phase 3c: TerminalSlot 실제 PTY 연결 브리핑 (담당: dcs24, Sonnet)

발신: ed12 (매니저)
근거: `docs/frontend-integration-lld.md` §4(TerminalSlot), §4-1(에러/Exited 가드), §8.
목적: **첫 E2E.** 더미 write → 실제 PTY subscribe. spawn→출력→입력→kill이 실제 창에서 동작.
선행: 기존 src/components/slot/TerminalSlot.tsx, SlotContextMenu.tsx, slotStore/agentStore 읽고 연결.

## 1. decodeBase64Bytes 헬퍼 (src/api/ 또는 util)
```ts
export function decodeBase64Bytes(b64: string): Uint8Array {
  // feature-detect Uint8Array.fromBase64 (신규), 없으면 atob fallback
  const f = (Uint8Array as any).fromBase64
  if (f) return f(b64)
  const bin = atob(b64); const arr = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i)
  return arr
}
```

## 2. TerminalSlot.tsx — 더미 제거, 실제 연결 (§4 패턴 그대로)

**subscribe (핵심 — C2/T-2/G-1 전부 준수):**
```ts
useEffect(() => {
  if (!agentId) return
  let sinkId = null, channel = null, cancelled = false
  terminal.reset()                          // C2: 구독 전 초기화(중복 방지)
  const lastSeqRef = { current: -1 }        // T-2/G-2: seq dedup
  ptyApi.subscribeOutput(agentId, (event) => {
    if (cancelled) return
    if (event.seq <= lastSeqRef.current) return   // 중복 drop
    lastSeqRef.current = event.seq
    terminal.write(decodeBase64Bytes(event.data_b64))
  }).then(result => {
    if (cancelled) { ptyApi.unsubscribeOutput(agentId, result.sinkId); return }
    sinkId = result.sinkId; channel = result.channel
  })
  return () => {
    cancelled = true
    if (channel) delete (channel as any).onmessage   // G-1: delete(null 아님), #13133
    if (sinkId) ptyApi.unsubscribeOutput(agentId, sinkId)
  }
}, [agentId])
```

**키 입력 → stdin:**
```ts
useEffect(() => {
  if (!agentId || !terminal) return
  const disp = terminal.onData(data => ptyApi.writeStdin(agentId, new TextEncoder().encode(data)))
  return () => disp.dispose()
}, [agentId, terminal])
```

**resize → resizePty:** 기존 FitAddon 흐름에 추가 — fit 후 `ptyApi.resizePty(agentId, terminal.cols, terminal.rows)`. (ResizeObserver/allotment 리사이즈 핸들러가 이미 있으면 거기에 호출 추가)

**Exited 가드 (M4):** agentStore의 해당 agent status.type이 Exited/Failed/Killed면 입력 비활성 + overlay 표시(종료됨). T-4대로 status는 표시용.

## 3. SlotContextMenu.tsx — spawn/kill 트리거 (§8)
- 기존 splitSlot 외에 **spawnAgent**(cwd) + **killAgent** 추가.
- spawnAgent: `ptyApi.spawnAgent(cwd)` → 반환 AgentInfo.id를 해당 slot에 할당(slotStore). cwd는 일단 고정 기본값(예: 사용자 홈 또는 입력 프롬프트 — 검증 단계라 단순하게).
- killAgent: 현재 slot의 agentId로 `ptyApi.killAgent`.

## 4. AgentTree.tsx — 더미 → 실제 (§8)
- `dummyAgents`/`dummyGroups` 제거 → `useAgentStore`의 `agents`(AgentInfo[]) 사용.
- status 색상: `agent.status.type`('Running'|'Exiting'|'Exited'|'Failed'|'Killed') 기반 분기.
- 트리 클릭 → 포커스 slot에 agentId 할당(기존 slotStore 흐름 유지).

## 5. (선행) 3b minor 수정 — initEventBus in-flight 가드
StrictMode 이중마운트 레이스 방지. 모듈 레벨:
```ts
let initPromise: Promise<void> | null = null
export function initEventBus() {
  if (initPromise) return initPromise   // in-flight면 같은 promise 반환
  initPromise = (async () => { /* 기존 로직 */ })()
  return initPromise
}
// HMR dispose에서 initPromise = null 도 리셋
```

## 규칙·품질
- C2(reset)/T-2(dedup)/G-1(delete onmessage)/명시적 unsubscribe 전부 필수 — 누락 시 메모리 누수·중복 출력.
- StrictMode 이중 effect 안전(cancelled flag).
- 주석: 각 가드의 이유(왜 reset, 왜 dedup, 왜 delete).

## 검증 & 보고 (E2E — 실제 화면)
`npm run tauri dev` 띄우고:
1. context menu로 spawnAgent → slot에 PTY 셸 출력 뜸
2. 키보드 입력 → 셸에 반영(echo)
3. 창/슬롯 리사이즈 → 셸 cols/rows 따라감(깨짐 없음)
4. killAgent → 종료 overlay, 목록에서 제거(T-4)

보고: `orch 12 "⟁dcs24 Phase3c 완료 — TerminalSlot subscribe/입력/resize + spawn/kill, E2E 화면동작: <관찰>"`
※ 이건 사용자가 직접 화면 확인할 단계다. 동작 스크린샷/로그 첨부하면 좋음. 막히면 30분 내 중간보고.
