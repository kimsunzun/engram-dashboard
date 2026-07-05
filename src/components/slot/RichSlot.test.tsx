// RichSlot(라이브 모드) send() 실패 경로 회귀 — writeStdin 이 reject 되면 awaiting 을 해제해
//   'streaming'/Thinking 표시가 무한 고착되지 않는지 단언(로컬 UI-state 에러 처리, WIRE 불변 ADR-0044/45/46).
//
// 배경: send() 는 전송 직후 awaiting=true 로 즉시 streaming 힌트를 켠다(FIX 5b). 응답 이벤트가 도착하면
//   awaiting 이 해제되지만, writeStdin 자체가 reject 되면 응답이 영영 안 와 awaiting 이 걸린 채 남는다 →
//   파생 streaming(= awaiting || (!turnDone && items.length>0))이 계속 true 라 헤더 배지·Thinking 이 고착.
//   fix: catch 에서 setAwaiting(false). 여기서 그 복귀를 관측한다.
//
// 전략: agentClient(clientFactory)·agentStore 를 slotTagGate.test.tsx 와 동일 패턴으로 stub.
//   writeStdin 을 reject 하도록 mock → 입력 후 send → 마이크로태스크 flush → 헤더가 idle 인지 관측.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── agentClient stub — writeStdin 을 테스트마다 갈아끼울 수 있게 holder 로 노출. ──
const clientMock = vi.hoisted(() => ({
  writeStdin: vi.fn(async () => undefined) as (id: string, bytes: Uint8Array) => Promise<void>,
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    // ADR-0046 시그니처 (viewId, agentId, onChunk, onState?). 이 테스트는 send() 만 검증하므로
    // 구독은 즉시 no-op handle 을 돌려준다(구독 콜백은 부르지 않음 → 오직 send 경로만 관측).
    subscribeOutput: vi.fn(async () => ({ unsubscribe: vi.fn() })),
    writeStdin: (id: string, bytes: Uint8Array) => clientMock.writeStdin(id, bytes),
    resizePty: vi.fn(async () => undefined),
    connectionState: 'connected',
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — 슬롯이 종료 판정용으로 useAgentStore(s => s.agents) 를 조회. 빈 목록 = 살아있음. ──
const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[] }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: { agents: unknown[] }) => unknown) => selector(agentStoreState),
}))

// ── 테스트 대상 ────────────────────────────────────────────────────────────────
import RichSlot from './RichSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'

/** subscribeOutput/writeStdin async 마이크로태스크를 비운다(구독 등록·write reject 반영). */
async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

beforeEach(() => {
  clientMock.writeStdin = vi.fn(async () => undefined)
  agentStoreState.agents = []
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('RichSlot(live) — send() 실패 시 awaiting 해제', () => {
  it('writeStdin 이 reject 되면 streaming 배지·Thinking 이 고착되지 않고 idle 로 복귀한다', async () => {
    // console.warn 은 fix 의 에러 표면 — 테스트 로그 오염 방지 겸 호출 관측용으로 잠재운다.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // 전송 자체 실패(write reject)를 재현.
    clientMock.writeStdin = vi.fn(async () => {
      throw new Error('write failed')
    })

    render(<RichSlot viewId="v1" agentId={AGENT} epoch={0} />)
    await flush()

    // 초기: 아무 것도 안 보냈으니 idle.
    expect(screen.getByText('● idle')).toBeTruthy()

    // 입력 후 전송 — send() 는 즉시 awaiting=true 로 streaming 힌트를 켠다.
    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    const sendBtn = screen.getByRole('button', { name: '전송' })
    fireEvent.click(sendBtn)

    // write 가 시도됐는지 확인(전송 경로 진입).
    expect(clientMock.writeStdin).toHaveBeenCalledTimes(1)

    // reject 반영(catch → setAwaiting(false)) 마이크로태스크 flush.
    await flush()

    // ★핵심 단언★: awaiting 이 해제돼 UI 가 idle 로 복귀 — streaming 배지·Thinking 이 고착되지 않는다.
    expect(screen.getByText('● idle')).toBeTruthy()
    expect(screen.queryByText('○ streaming')).toBeNull()
    expect(screen.queryByText('Thinking')).toBeNull()

    // fix 의 에러 표면(console.warn)이 실제로 호출됐는지도 확인.
    expect(warn).toHaveBeenCalled()
  })

  it('writeStdin 이 성공하면(응답 전) awaiting 이 유지돼 streaming 힌트가 켜진다(대조군)', async () => {
    clientMock.writeStdin = vi.fn(async () => undefined) // 성공 = resolve

    render(<RichSlot viewId="v1" agentId={AGENT} epoch={0} />)
    await flush()

    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.click(screen.getByRole('button', { name: '전송' }))
    await flush()

    // 성공 경로 — 아직 응답 이벤트가 없으므로 awaiting 브리지로 streaming 유지.
    expect(screen.getByText('○ streaming')).toBeTruthy()
    expect(screen.queryByText('● idle')).toBeNull()
  })
})
