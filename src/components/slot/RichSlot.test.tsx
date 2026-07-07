// RichSlot(라이브 모드) send() 실패 경로 회귀 — writeStdin 이 reject 되면 awaiting 을 해제해
//   'streaming' 표시가 무한 고착되지 않는지 단언(로컬 UI-state 에러 처리, WIRE 불변 ADR-0044/45/46).
//
// 배경: send() 는 전송 직후 awaiting=true 로 즉시 streaming 힌트를 켠다(FIX 5b). 응답 이벤트가 도착하면
//   awaiting 이 해제되지만, writeStdin 자체가 reject 되면 응답이 영영 안 와 awaiting 이 걸린 채 남는다 →
//   파생 streaming(= awaiting || (!turnDone && items.length>0))이 계속 true 라 표시가 고착.
//   fix: catch 에서 setAwaiting(false). 여기서 그 복귀를 관측한다.
//
// ★관측 표면(ADR-0053 헤더 제거 이후)★: 구 "JSON ● idle/○ streaming" 슬림 헤더가 제거돼, streaming 의
//   유일한 시각 신호는 스트림 끝 "Thinking…" tail(StructuredTextView)뿐이다. 이 tail 은 streaming && 콘텐츠
//   존재(items.length>0)일 때만 뜬다. 그래서 관측 가능한 상태를 만들려고, 구독 콜백을 캡처해 TextDelta +
//   MessageDone 을 먹인다 → items=[text,separator] & turnDone=true. 그러면 streaming = awaiting 로 좁혀져
//   (!turnDone 항이 죽음), "Thinking…" tail 의 유무가 곧 awaiting 의 거울이 된다.
//
// 전략: agentClient(clientFactory)·agentStore 를 slotTagGate.test.tsx 와 동일 패턴으로 stub. subscribeOutput
//   콜백을 캡처(onChunk)해 tag1(StructuredEvent) chunk 를 주입하고, writeStdin 을 reject/resolve 로 갈아끼운다.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FRAME_TAG_STRUCTURED_EVENT } from '../../api/wsFrame'
import type { OutputChunk } from '../../api/agentClient'

// ── subscribeOutput 콜백 캡처 + writeStdin holder(테스트마다 갈아끼움). ──
const captured = vi.hoisted(() => ({ onChunk: null as ((c: OutputChunk) => void) | null }))
const clientMock = vi.hoisted(() => ({
  writeStdin: vi.fn(async () => undefined) as (id: string, bytes: Uint8Array) => Promise<void>,
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    // ADR-0046 시그니처 (viewId, agentId, onChunk, onState?). onChunk 를 캡처해 chunk 를 주입한다.
    subscribeOutput: vi.fn(
      async (_viewId: string, _agentId: string, onChunk: (c: OutputChunk) => void) => {
        captured.onChunk = onChunk
        return { unsubscribe: vi.fn() }
      },
    ),
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
const enc = new TextEncoder()

/** tag1 = StructuredEvent JSON chunk(구조화 슬롯이 소비하는 유일 tag). */
function tag1(seq: number, json: string): OutputChunk {
  return { seq, tag: FRAME_TAG_STRUCTURED_EVENT, bytes: enc.encode(json) }
}

/** subscribeOutput/writeStdin async 마이크로태스크를 비운다(구독 등록·write reject 반영). */
async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

/**
 * 콘텐츠 1턴을 완결 상태로 주입한다(TextDelta → MessageDone). 결과: items=[text,separator], turnDone=true.
 * 이 상태에서 streaming = awaiting 로 좁혀져(!turnDone 항 무력화) "Thinking…" tail 이 awaiting 을 그대로 반영.
 */
function feedCompletedTurn(): void {
  act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'assistant reply' }))))
  act(() => captured.onChunk!(tag1(1, JSON.stringify({ type: 'MessageDone' }))))
}

beforeEach(() => {
  captured.onChunk = null
  clientMock.writeStdin = vi.fn(async () => undefined)
  agentStoreState.agents = []
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('RichSlot(live) — send() 실패 시 awaiting 해제', () => {
  it('writeStdin 이 reject 되면 "Thinking…" 스트리밍 신호가 고착되지 않고 idle 로 복귀한다', async () => {
    // console.warn 은 fix 의 에러 표면 — 테스트 로그 오염 방지 겸 호출 관측용으로 잠재운다.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // 전송 자체 실패(write reject)를 재현.
    clientMock.writeStdin = vi.fn(async () => {
      throw new Error('write failed')
    })

    render(<RichSlot viewId="v1" agentId={AGENT} epoch={0} />)
    await flush()
    expect(captured.onChunk).toBeTruthy()

    // 완결된 1턴을 주입 → turnDone=true & 콘텐츠 존재. 이 상태에서 streaming = awaiting.
    feedCompletedTurn()
    // 초기: 아무 것도 안 보냈으니 idle — "Thinking…" tail 이 없다.
    expect(screen.queryByText('Thinking…')).toBeNull()

    // 입력 후 전송(Enter 경로) — send() 는 즉시 awaiting=true 로 streaming 힌트를 켠다.
    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })

    // write 가 시도됐는지 확인(전송 경로 진입).
    expect(clientMock.writeStdin).toHaveBeenCalledTimes(1)

    // reject 반영(catch → setAwaiting(false)) 마이크로태스크 flush.
    await flush()

    // ★핵심 단언★: awaiting 이 해제돼 UI 가 idle 로 복귀 — "Thinking…" 신호가 고착되지 않는다.
    expect(screen.queryByText('Thinking…')).toBeNull()

    // fix 의 에러 표면(console.warn)이 실제로 호출됐는지도 확인.
    expect(warn).toHaveBeenCalled()
  })

  it('writeStdin 이 성공하면(응답 전) awaiting 이 유지돼 "Thinking…" 신호가 켜진다(대조군)', async () => {
    clientMock.writeStdin = vi.fn(async () => undefined) // 성공 = resolve

    render(<RichSlot viewId="v1" agentId={AGENT} epoch={0} />)
    await flush()

    feedCompletedTurn()
    expect(screen.queryByText('Thinking…')).toBeNull()

    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()

    // 성공 경로 — 아직 응답 이벤트가 없으므로 awaiting 브리지로 streaming 유지 → "Thinking…" tail 이 뜬다.
    expect(screen.getByText('Thinking…')).toBeTruthy()
  })
})
