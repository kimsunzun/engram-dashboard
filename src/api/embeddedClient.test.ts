// EmbeddedClient 이벤트 구독 단위테스트 — Tauri listen 래핑(상태/목록/복원 공통 표면).
//
// @tauri-apps/api/event 의 listen 을 mock 해서, on* 메서드가 올바른 이벤트명을 구독하고
// payload 를 cb 시그니처로 전달하는지 + sync disposer 가 unlisten 을 호출하는지 검증한다.
// @tauri-apps/api/core 도 mock(모듈 로드 시 Channel/invoke import 평가 — 실제 Tauri 접속 0).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// listen mock: 등록 시 (event, handler) 를 기록하고, 테스트가 수동으로 handler 를 발화시킨다.
// listen 은 async(Promise<unlisten>) — resolve 를 제어해 sync disposer 의 취소 안전성도 검증.
interface ListenReg {
  event: string
  handler: (e: { payload: unknown }) => void
  unlisten: ReturnType<typeof vi.fn>
}
const regs: ListenReg[] = []
let resolveListen: (() => void)[] = []
const listenMock = vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
  const unlisten = vi.fn()
  regs.push({ event, handler, unlisten })
  // resolve 를 지연 가능하게: 기본은 즉시 resolve.
  return new Promise<() => void>((resolve) => {
    resolveListen.push(() => resolve(unlisten))
    // 기본 즉시 resolve(취소 안전 테스트만 수동 제어).
  })
})

vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: unknown }) => void) => listenMock(event, handler),
}))
// core 는 모듈 로드용 stub(이벤트 메서드는 core 미사용).
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  Channel: class {},
}))

import { EmbeddedClient } from './embeddedClient'
import type { AgentInfo, AgentStatus, RestoreReport } from './types'

/** 가장 최근 등록을 resolve(listen promise 완료) + 마이크로태스크 통과. */
async function flushListen(): Promise<void> {
  for (const r of resolveListen) r()
  resolveListen = []
  await Promise.resolve()
  await Promise.resolve()
}

beforeEach(() => {
  regs.length = 0
  resolveListen = []
  listenMock.mockClear()
})
afterEach(() => {
  vi.restoreAllMocks()
})

describe('EmbeddedClient 이벤트 구독', () => {
  it('onAgentListUpdated → agent-list-updated 구독 + payload(AgentInfo[]) 를 cb 로 전달', async () => {
    const client = new EmbeddedClient()
    const seen: AgentInfo[][] = []
    client.onAgentListUpdated((agents) => seen.push(agents))
    expect(listenMock).toHaveBeenCalledWith('agent-list-updated', expect.any(Function))
    const reg = regs.find((r) => r.event === 'agent-list-updated')!
    const list = [{ id: 'a1' }] as unknown as AgentInfo[]
    reg.handler({ payload: list })
    expect(seen).toEqual([list])
  })

  it('onStatusChanged → agent-status-changed 의 {id,status,epoch} 를 (id,status,epoch) 로 전달', () => {
    const client = new EmbeddedClient()
    const calls: Array<[string, AgentStatus, number]> = []
    client.onStatusChanged((id, status, epoch) => calls.push([id, status, epoch]))
    const reg = regs.find((r) => r.event === 'agent-status-changed')!
    const status: AgentStatus = { type: 'Running' }
    reg.handler({ payload: { id: 'x', status, epoch: 2 } })
    expect(calls).toEqual([['x', status, 2]])
  })

  it('onRestoreResult → agent-restore-result 의 RestoreReport 를 cb 로 전달', () => {
    const client = new EmbeddedClient()
    const seen: RestoreReport[] = []
    client.onRestoreResult((report) => seen.push(report))
    const reg = regs.find((r) => r.event === 'agent-restore-result')!
    const report = { agent_id: 'r', epoch: 0, outcome: { type: 'Started' } } as RestoreReport
    reg.handler({ payload: report })
    expect(seen).toEqual([report])
  })

  it('disposer 호출 → listen 이 반환한 unlisten 을 호출한다(resolve 후)', async () => {
    const client = new EmbeddedClient()
    const off = client.onAgentListUpdated(() => {})
    await flushListen() // listen promise resolve → 내부에 unlisten 보관
    const reg = regs[0]
    expect(reg.unlisten).not.toHaveBeenCalled()
    off()
    expect(reg.unlisten).toHaveBeenCalledTimes(1)
  })

  it('취소 안전: resolve 전에 disposer 호출 → resolve 시 즉시 unlisten(리스너 누수 방지)', async () => {
    const client = new EmbeddedClient()
    const off = client.onAgentListUpdated(() => {})
    // 아직 listen promise resolve 안 됨 — 이 시점 disposer 호출(cancelled 플래그 set).
    off()
    const reg = regs[0]
    expect(reg.unlisten).not.toHaveBeenCalled() // 아직 unlisten 핸들 없음
    await flushListen() // 뒤늦게 resolve → cancelled 면 즉시 unlisten
    expect(reg.unlisten).toHaveBeenCalledTimes(1)
  })
})
