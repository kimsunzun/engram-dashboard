// RichSlot(라이브 모드) send() 실패 경로 회귀 — 로컬 UI-state 에러 처리(WIRE 불변 ADR-0044/45/46).
//
// 배경: send() 는 전송 직후 awaiting=true 로 즉시 streaming 힌트를 켠다(FIX 5b). 응답 이벤트가 도착하면
//   awaiting 이 해제되지만, writeStdin 자체가 reject 되면 응답이 영영 안 와 awaiting 이 걸린 채 남는다 →
//   파생 streaming(= awaiting || (!turnDone && items.length>0))이 계속 true 라 표시가 고착.
//   fix: catch 에서 setAwaiting(false). 여기서 그 복귀를 관측한다.
//
// ★관측 표면(ADR-0053 헤더 제거 이후)★: 구 "JSON ● idle/○ streaming" 슬림 헤더가 제거돼, streaming 의
//   유일한 시각 신호는 스트림 끝 대기 인디케이터(WaitRow "Wait" 라벨, StructuredTextView)뿐이다. 이 tail 은
//   streaming 이면 뜬다(showTail = streaming). 그래서 관측 가능한 상태를 만들려고, 구독 콜백을 캡처해
//   TextDelta + MessageDone 을 먹인다 → items=[text,separator] & turnDone=true. 그러면 streaming = awaiting
//   로 좁혀져(!turnDone 항이 죽음), "Wait" tail 의 유무가 곧 awaiting 의 거울이 된다.
//
// 전략: agentClient(clientFactory)·agentStore 를 slotTagGate.test.tsx 와 동일 패턴으로 stub. subscribeOutput
//   콜백을 캡처(onChunk)해 tag1(StructuredEvent) chunk 를 주입하고, writeStdin 을 reject/resolve 로 갈아끼운다.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { FRAME_TAG_STRUCTURED_EVENT } from '../../api/wsFrame'
import type { OutputChunk, ViewPhase } from '../../api/agentClient'
import { t } from '../../i18n'

// ── subscribeOutput 콜백 캡처 + writeStdin holder(테스트마다 갈아끼움). ──
const captured = vi.hoisted(() => ({
  onChunk: null as ((c: OutputChunk) => void) | null,
  onState: null as ((s: ViewPhase) => void) | null,
  onReset: null as (() => void) | null,
}))
const clientMock = vi.hoisted(() => ({
  writeStdin: vi.fn(async () => undefined) as (id: string, bytes: Uint8Array) => Promise<void>,
  // 연결 상태 표면(ADR-0148 부재 판정의 절반) — 등록 즉시 현재 상태로 1회 발화하는 실물 계약을 따른다.
  connectionState: 'connected' as 'connected' | 'reconnecting' | 'down',
  stateCbs: new Set<(s: 'connected' | 'reconnecting' | 'down') => void>(),
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    // ADR-0046 시그니처 (viewId, agentId, onChunk, onState?, onReset?).
    subscribeOutput: vi.fn(
      async (
        _viewId: string,
        _agentId: string,
        onChunk: (c: OutputChunk) => void,
        onState?: (s: ViewPhase) => void,
        onReset?: () => void,
      ) => {
        captured.onChunk = onChunk
        captured.onState = onState ?? null
        captured.onReset = onReset ?? null
        return { unsubscribe: vi.fn() }
      },
    ),
    writeStdin: (id: string, bytes: Uint8Array) => clientMock.writeStdin(id, bytes),
    resizePty: vi.fn(async () => undefined),
    get connectionState() {
      return clientMock.connectionState
    },
    onConnectionStateChange: (cb: (s: 'connected' | 'reconnecting' | 'down') => void) => {
      clientMock.stateCbs.add(cb)
      cb(clientMock.connectionState) // 실물과 동일: 등록 즉시 현재 상태 1회 통지.
      return () => clientMock.stateCbs.delete(cb)
    },
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — 슬롯이 부재 판정용으로 agents·agentsLoaded 를 조회한다. ──
// agentsLoaded=false 가 기본 = "권위 명부 미수신" → 빈 목록을 부재로 오인하지 않는다(ADR-0148 가드).
const agentStoreState = vi.hoisted(() => ({
  agents: [] as unknown[],
  agentsLoaded: false,
}))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
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
 * 이 상태에서 streaming = awaiting 로 좁혀져(!turnDone 항 무력화) "Wait" tail 이 awaiting 을 그대로 반영.
 */
function feedCompletedTurn(): void {
  act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'assistant reply' }))))
  act(() => captured.onChunk!(tag1(1, JSON.stringify({ type: 'MessageDone' }))))
}

/** 연결 상태 전이(실물 ProtocolClient 가 구독자 전원에게 통지하는 경로와 동형). */
function setConnection(state: 'connected' | 'reconnecting' | 'down'): void {
  clientMock.connectionState = state
  act(() => {
    for (const cb of clientMock.stateCbs) cb(state)
  })
}

/** 부재 오버레이(ADR-0148 결정 2·3) — 흐림 막 + 심볼. 시각값(투명도·크기)은 단언하지 않는다. */
function deadOverlay(): HTMLElement | null {
  return document.querySelector('[data-slot-dead="1"]')
}

beforeEach(() => {
  captured.onChunk = null
  captured.onState = null
  captured.onReset = null
  clientMock.writeStdin = vi.fn(async () => undefined)
  clientMock.connectionState = 'connected'
  clientMock.stateCbs.clear()
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('RichSlot(live) — send() 실패 시 awaiting 해제', () => {
  it('writeStdin 이 reject 되면 "Wait" 스트리밍 신호가 고착되지 않고 idle 로 복귀한다', async () => {
    // console.warn 은 fix 의 에러 표면 — 테스트 로그 오염 방지 겸 호출 관측용으로 잠재운다.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.writeStdin = vi.fn(async () => {
      throw new Error('write failed')
    })

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    expect(captured.onChunk).toBeTruthy()

    // 완결된 1턴을 주입 → turnDone=true & 콘텐츠 존재. 이 상태에서 streaming = awaiting.
    feedCompletedTurn()
    expect(screen.queryByText('Wait')).toBeNull()

    // send() 는 즉시 awaiting=true 로 streaming 힌트를 켠다.
    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })

    expect(clientMock.writeStdin).toHaveBeenCalledTimes(1)

    // reject 반영(catch → setAwaiting(false)) 마이크로태스크 flush.
    await flush()

    expect(screen.queryByText('Wait')).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('writeStdin 이 성공하면(응답 전) awaiting 이 유지돼 "Wait" 신호가 켜진다(대조군)', async () => {
    clientMock.writeStdin = vi.fn(async () => undefined) // 성공 = resolve

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    feedCompletedTurn()
    expect(screen.queryByText('Wait')).toBeNull()

    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'hello' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()

    // 성공 경로 — 아직 응답 이벤트가 없으므로 awaiting 브리지로 streaming 유지.
    expect(screen.getByText('Wait')).toBeTruthy()
  })
})

// ★후속 전송 flicker 회귀★: 완결된 직전 턴(turnDone=true) 뒤에 새로 전송하면, 첫 assistant 토큰보다
//   합성 user 에코가 먼저 도착한다. 그 에코가 awaiting 을 해제하는 순간에도 "Wait" 이 유지돼야 한다
//   (누산기 fix: user Structured 가 turnDone=false 로 내려 streaming 파생을 살린다). fix 전엔 이 순간
//   streaming = awaiting(false) || (!turnDone(true) && items>0) = false 로 떨어져 "Wait" 이 깜빡 꺼졌다.
describe('RichSlot(live) — 후속 전송 시 합성 user 에코가 "Wait" 을 끄지 않는다(flicker FIX)', () => {
  it('완결 턴 뒤 전송 → user 에코 도착 후에도 "Wait" 이 유지된다', async () => {
    clientMock.writeStdin = vi.fn(async () => undefined) // 성공 = resolve

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    // 직전 턴 완결(turnDone=true) + 콘텐츠 존재.
    feedCompletedTurn()
    expect(screen.queryByText('Wait')).toBeNull()

    // 후속 전송 — awaiting=true 로 streaming 힌트 on.
    const textarea = screen.getByPlaceholderText(/메시지 입력/)
    fireEvent.change(textarea, { target: { value: 'follow-up' } })
    fireEvent.keyDown(textarea, { key: 'Enter' })
    await flush()
    expect(screen.getByText('Wait')).toBeTruthy()

    // 합성 user 에코가 첫 assistant 토큰보다 먼저 도착(json 모드 write_input 직후). 구독 콜백이
    //   setAwaiting(false) 하지만, 누산기가 turnDone=false 로 내려 파생 streaming 이 유지된다.
    act(() =>
      captured.onChunk!(
        tag1(2, JSON.stringify({
          type: 'Structured',
          kind: 'user',
          json: JSON.stringify({ type: 'text', text: 'follow-up', uuid: 'U' }),
        })),
      ),
    )

    expect(screen.getByText('Wait')).toBeTruthy()
  })
})

// ★ADR-0145 빈 상태★: JSON 모드 첫 실행 화면(마스코트 + "Claude Code" + 가운데 입력창).
//
// 핵심 회귀 = **표시 게이트가 "0건"이 아니라 "복원 완료 신호('live') + 0건"이라는 것**. 챗 뷰는 마운트 시
//   목록을 비우고 구독을 걸므로 이력이 있는 세션도 복원이 끝나기 전엔 0건이다 → 0건만 보면 안내 화면이
//   떴다가 대화로 바뀌는 깜빡임이 난다. 그 신호는 subscribeOutput 의 4번째 인자(상태 콜백)로 온다.
//
// 관측 표면: 빈 상태 컨테이너 [data-rich-empty] · 마스코트 [data-rich-mascot] · 하단 배치에만 붙는
//   정체성 라벨 [data-rich-label] · 입력창 className(빈 상태 rounded-xl ↔ 하단 flex-1).

/** 빈 상태 컨테이너(마스코트 + 문구). null = 기존(대화) 레이아웃. */
function emptyState(): HTMLElement | null {
  return document.querySelector('[data-rich-empty="1"]')
}

function mascot(): HTMLElement | null {
  return document.querySelector('[data-rich-mascot="1"]')
}

/** 입력창은 두 배치가 같은 엘리먼트라 문서에 항상 1개다(이 함수가 그 전제를 겸사겸사 지킨다). */
function textarea(): HTMLTextAreaElement {
  const els = document.querySelectorAll('textarea')
  if (els.length !== 1) throw new Error(`textarea 는 1개여야 한다 — 실제 ${els.length}개`)
  return els[0]
}

/** replay 국면 콜백 발화(구독 등록 이후에만 유효). */
function fireState(state: ViewPhase): void {
  act(() => captured.onState!(state))
}

/** 비우기 신호 발화 — 실물은 새 화신의 전량 replay 배달 **직전** 같은 틱에 부른다. */
function fireReset(): void {
  act(() => captured.onReset!())
}

describe('RichSlot(live) — ADR-0145 첫 실행 빈 상태', () => {
  it('복원 중(live 미발화)에는 0건이어도 빈 상태를 그리지 않는다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    // 4번째 인자(상태 콜백) 전달 자체가 회귀 대상 — 안 넘기면 게이트가 영영 안 열린다.
    expect(captured.onState).toBeTruthy()
    expect(emptyState()).toBeNull()
    expect(screen.queryByText('Claude Code')).toBeNull()

    fireState('buffering')
    expect(emptyState()).toBeNull()
  })

  it("'live' + 0건이면 마스코트·문구·둥근 입력창을 그린다", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    expect(emptyState()).not.toBeNull()
    expect(mascot()).not.toBeNull()
    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(textarea().className).toContain('rounded-xl')
    // 마스코트는 순수 장식이라 접근성 트리에서 숨긴다(ADR-0146 불변식) — 스크린리더가 도트를 읽지 않는다.
    expect(mascot()?.getAttribute('aria-hidden')).toBe('true')
  })

  it("'error' 로 끝나면 0건이어도 빈 상태를 그리지 않는다(현행 빈 화면 유지)", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('error')

    expect(emptyState()).toBeNull()
  })

  // ★재부착 회귀(protocolClient 의 buffering 통지와 한 쌍)★: 데몬이 끊겼다 붙으면 뷰가 live →
  //   buffering 으로 되돌아가 full replay 를 다시 받는다. 그 통지를 안 듣고 '복원 완료'를 래치해 두면,
  //   끊긴 동안 쌓인 이력이 쏟아질 때 마스코트가 대화로 뒤집힌다.
  it("live 이후 'buffering' 이 다시 오면 빈 상태를 내린다(재replay 깜빡임 차단)", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    fireState('buffering')
    expect(emptyState()).toBeNull()

    // 재replay 로 이력이 도착 — 빈 상태는 이미 내려가 있어 전환이 눈에 띄지 않는다.
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'restored' }))))
    expect(emptyState()).toBeNull()
  })

  // ★재replay 누적 회귀(사용자 결정 2026-08-20)★: 비우기 신호는 다른 화신이 프레임을 0 부터 다시 매기고
  //   구독이 진도 커서를 버렸다는 뜻이다. 누산기를 안 비우면 처음부터 다시 오는 이력이 지금 대화 위에
  //   겹쳐 쌓이고, 로컬 seq 가드를 안 되돌리면 그 프레임이 전부 탈락해 대화가 안 돌아온다.
  it('onReset 은 대화를 비우고 새 화신을 한 벌로만 다시 그린다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    // 비우기 콜백 전달 자체가 회귀 대상 — 안 넘기면 겹쳐 그려도 오류가 어디에도 안 남는다.
    expect(captured.onReset).toBeTypeOf('function')
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'session one' }))))
    fireState('live')
    expect(screen.getAllByText('session one')).toHaveLength(1)

    fireReset()
    expect(screen.queryByText('session one')).toBeNull()

    // 재기동된 데몬의 새 화신 — seq 가 0 부터 다시 온다.
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'session one' }))))
    fireState('live')
    expect(screen.getAllByText('session one')).toHaveLength(1)
  })

  // ★국면 통지로는 비우지 않는다(반대 방향 회귀)★: 소켓만 깜빡이고 돌아온 경우 클라는 커서를 지킨 채
  //   전량 재replay 를 내고 겹치는 앞부분을 dedup 이 흡수한다 — 그 구간도 'buffering' 이라, 국면으로
  //   비우면 잠깐 끊겼다 올 때마다 대화가 사라졌다 다시 그려진다.
  it.each<ViewPhase>(['detached', 'buffering', 'live', 'error'])(
    "'%s' 국면은 대화를 그대로 둔다",
    async (phase) => {
      render(<RichSlot viewId="v1" agentId={AGENT} />)
      await flush()
      act(() =>
        captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: 'session one' }))),
      )
      fireState('live')

      fireState(phase)
      expect(screen.getAllByText('session one')).toHaveLength(1)
    },
  )

  // ★비우기는 전송 흔적까지 함께 내린다★: 이 신호는 **다른 화신**의 이력이 온다는 뜻이라, 앞서 보낸
  //   메시지는 이미 사라진 화신이 받았고 답은 영영 오지 않는다. 남겨 두면 이력 0건인 새 화신에 첫 실행
  //   화면이 안 뜨고("빈 슬롯인데 대화 중"), 대기 표시가 답 없이 고착된다.
  //   ★같은 화신 이어보기에는 이 신호가 오지 않는다★ — 그 구간의 대기 표시는 바로 아래 국면 케이스가 잰다.
  it('onReset 은 전송 흔적도 함께 내려 새 화신을 첫 실행 화면으로 되돌린다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()
    expect(screen.getByText('Wait')).toBeTruthy() // 대기 표시(awaiting)

    // 다른 화신이 붙었다 — 이력 0건으로 끝나면 첫 실행 화면이 정답이다.
    fireReset()
    fireState('live')
    expect(emptyState()).not.toBeNull()
  })

  // 반대 방향: 같은 화신으로 다시 붙는 구간(비우기 신호 없음)에서는 대기 표시가 살아 있어야 한다 —
  //   나간 전송은 재부착으로 취소되지 않는다.
  it('비우기 없이 국면만 오가면 전송 흔적을 지키지 않는다(대기 표시 유지)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    fireState('buffering')
    fireState('live')
    expect(emptyState()).toBeNull()
    expect(screen.getByText('Wait')).toBeTruthy()
  })

  // 부재를 표면에 남긴다 — 명부에 그 에이전트가 없는 동안 슬롯이 "살아 있는데 조용한" 것처럼 보이면
  //   안 된다(ADR-0148 의 부재 막을 detached 에도 건다).
  it("'detached' 는 부재 막을 띄운다", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    expect(deadOverlay()).toBeNull()

    fireState('detached')
    expect(deadOverlay()).not.toBeNull()
  })

  // 재요청 사다리를 소진한 뷰도 같다 — 아무것도 못 받는 상태인데 표시가 없으면 조용한 빈 판이 된다.
  it("'error' 도 부재 막을 띄운다(세 슬롯 동형)", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('error')
    expect(deadOverlay()?.getAttribute('data-slot-phase')).toBe('error')
  })

  it("'live' 여도 대화 item 이 있으면 빈 상태가 아니다", async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    fireState('live')

    expect(emptyState()).toBeNull()
    expect(textarea().className).toContain('flex-1')
  })

  it('첫 전송 즉시 빈 상태가 사라지고 입력창이 하단 배치로 돌아간다 — 같은 엘리먼트(remount 없음)', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    const before = textarea()
    before.focus()
    fireEvent.change(before, { target: { value: 'hello' } })
    fireEvent.keyDown(before, { key: 'Enter' })
    await flush()

    expect(emptyState()).toBeNull()
    // 두 배치가 한 엘리먼트라야 전환 중 입력 포커스가 끊기지 않는다(ADR-0145 §5).
    expect(textarea()).toBe(before)
    expect(document.activeElement).toBe(before)
    expect(before.className).toContain('flex-1')
    expect(document.querySelector('[data-rich-label="1"]')).not.toBeNull()
  })

  // 재구독이 도는 경로는 **배정이 다른 에이전트로 바뀌는 것** 하나다(재spawn 은 구독을 갈지 않는다 —
  //   아래 '정체성' describe). 그 전환에서 복원 완료 표시를 물려받으면, 새 에이전트의 이력이 도착하기
  //   전 구간에 빈 상태가 떴다가 대화로 바뀌는 깜빡임이 된다.
  it('다른 에이전트로 배정이 바뀌면 복원 완료 표시가 내려가 live 재발화 전까지 빈 상태가 아니다', async () => {
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    rerender(<RichSlot viewId="v1" agentId="eeee-ffff-0000-1111" />)
    await flush()
    expect(emptyState()).toBeNull()

    fireState('live') // 새 구독의 상태 콜백
    expect(emptyState()).not.toBeNull()
  })

  // 배치마다 문구가 갈린다: 빈 상태는 사용자 지정 인사말, 하단은 Enter/Shift+Enter 조작 안내.
  //   두 배치가 한 엘리먼트를 공유하므로 키 분기가 무너져도 화면만 봐선 안 드러난다 → 여기서 못 박는다.
  it('placeholder 는 빈 상태와 하단 배치가 서로 다른 문구를 쓴다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    expect(textarea().placeholder).toBe(t('agent.emptyInputPlaceholder'))
    expect(textarea().placeholder).toBe('메시지를 입력하세요')

    fireEvent.change(textarea(), { target: { value: 'hi' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    expect(emptyState()).toBeNull()
    expect(textarea().placeholder).toBe(t('agent.inputPlaceholder'))
  })

  it('종료된 에이전트는 빈 상태 배치에서도 종료 문구가 이긴다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Exited' } }]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    // 배치는 빈 상태인데(마스코트·문구 표시) 안내만 종료 쪽으로 갈린다.
    expect(emptyState()).not.toBeNull()
    expect(textarea().placeholder).toBe(t('agent.terminatedPlaceholder'))
  })

})

// ★ADR-0145 빈 상태 게이트가 awaiting 이 아니라 "이 구독에서 이미 보냈다" 인 이유★
//
// awaiting 은 (a) item 을 만들지 않는 tag1 프레임에도 풀리고(빈 TextDelta · user uuid dedup 스킵 ·
// 빈 MessageDone), (b) writeStdin 실패 catch 가 어느 전송의 것인지 모른 채 푼다. 둘 다 "응답 대기 중인데
// items 는 아직 0건" 인 구간을 만들고, 그 구간에 마스코트가 되살아나면 대화 위로 첫 화면이 덮인다.
describe('RichSlot(live) — ADR-0145 빈 상태 억제는 전송 사실을 따른다(awaiting 아님)', () => {
  it('전송 후 item 을 만들지 않는 프레임(빈 델타)이 와도 빈 상태로 돌아가지 않는다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()

    // 빈 델타 = 누산기가 item 을 만들지 않고 흘려보내는 프레임(structuredAccumulator :69).
    //   구독 콜백은 이 프레임에도 awaiting 을 푼다 → awaiting 게이트였다면 여기서 마스코트가 되살아난다.
    act(() => captured.onChunk!(tag1(0, JSON.stringify({ type: 'TextDelta', text: '' }))))
    expect(emptyState()).toBeNull()
  })

  it('앞 화신에 보낸 전송의 뒤늦은 실패가 새 화신의 전송 표시를 되돌리지 않는다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // 전송 A — 아직 결말이 안 난 promise 를 쥐고 있다가 나중에 reject 한다.
    let rejectA!: (e: Error) => void
    clientMock.writeStdin = vi.fn(
      () =>
        new Promise<void>((_, rej) => {
          rejectA = rej
        }),
    )

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    fireEvent.change(textarea(), { target: { value: 'send A' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    // 다른 화신이 붙었다(재spawn) → 비우기 신호가 전송 흔적을 내리고, live 를 다시 받아 빈 상태로 복귀.
    //   ★인스턴스는 그대로다★ — 재spawn 이 컴포넌트를 갈지 않으므로 건너온 실패는 **같은 인스턴스**로
    //   떨어진다. 그걸 막는 것이 아래 순번 가드다.
    fireReset()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    // 전송 B(새 화신) — 성공 경로.
    clientMock.writeStdin = vi.fn(async () => undefined)
    fireEvent.change(textarea(), { target: { value: 'send B' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()

    // 이제 A 가 실패한다. A 는 사라진 화신에 보낸 전송이므로 지금 화면의 어떤 표시도 건드리지 못해야
    //   한다 — 빈 상태로 되돌아가지도, B 의 대기 표시가 꺼지지도 않는다.
    rejectA(new Error('write failed'))
    await flush()
    expect(emptyState()).toBeNull()
    expect(screen.getByText('Wait')).toBeTruthy()
    expect(warn).toHaveBeenCalled()
  })

  it('같은 인스턴스에서도 앞선 전송의 뒤늦은 실패가 최신 전송의 대기 표시를 지우지 않는다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    let rejectA!: (e: Error) => void
    clientMock.writeStdin = vi.fn(
      () =>
        new Promise<void>((_, rej) => {
          rejectA = rej
        }),
    )

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    // 전송 A — 결말 미정.
    fireEvent.change(textarea(), { target: { value: 'send A' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(screen.getByText('Wait')).toBeTruthy()

    // 전송 B(성공) — 최신 전송이 B 로 넘어간다.
    clientMock.writeStdin = vi.fn(async () => undefined)
    fireEvent.change(textarea(), { target: { value: 'send B' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    // 뒤늦은 A 실패 — 최신 전송이 아니므로 아무 표시도 건드리지 않는다(B 는 여전히 응답 대기 중).
    rejectA(new Error('write failed'))
    await flush()
    expect(screen.getByText('Wait')).toBeTruthy()
    expect(emptyState()).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('첫 전송이 실패하면 빈 상태로 되돌아간다 — 실제로 아무것도 안 나갔다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.writeStdin = vi.fn(async () => {
      throw new Error('write failed')
    })

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    fireEvent.change(textarea(), { target: { value: 'hello' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    // 전송 시점엔 낙관적으로 내린다(성공을 기다리며 대화 화면으로).
    expect(emptyState()).toBeNull()

    // 실패가 확인되면 되돌아온다 — 안 되돌리면 재구독 전까지 첫 실행 화면이 영구히 안 뜬다.
    await flush()
    expect(emptyState()).not.toBeNull()
    expect(screen.queryByText('Wait')).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('성공한 전송이 있으면 이후 전송이 실패해도 빈 상태로 되돌아가지 않는다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    clientMock.writeStdin = vi.fn(async () => undefined)

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    // A: 성공(응답은 아직 없어 items 는 0건 — 되돌림 판정이 items 만 봐선 갈리지 않는 구간).
    fireEvent.change(textarea(), { target: { value: 'send A' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    // B: 실패. A 의 응답이 오는 중일 수 있으므로 첫 실행 화면으로 돌아가면 안 된다.
    clientMock.writeStdin = vi.fn(async () => {
      throw new Error('write failed')
    })
    fireEvent.change(textarea(), { target: { value: 'send B' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    expect(emptyState()).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  // ★비우기를 건너온 성공은 새 화신의 전송 사실이 아니다★: 전송 A 가 날아가는 중에 화신이 갈리면 A 의
  //   결말은 이미 사라진 화신의 것이다. 그 성공이 "이 인스턴스에서 나간 전송이 있다" 를 세우면, 새 화신에서
  //   실제로 나가지 못한 전송이 실패해도 첫 실행 화면으로 못 돌아가 아무것도 못 보낸 슬롯이 대화 중으로
  //   남는다. 순번 가드는 성공·실패 양쪽에 걸려야 하고, 비우기도 그 순번을 올려야 한다 — 한쪽만이면
  //   나머지 한쪽으로 이 케이스가 그대로 통과한다.
  it('비우기 뒤 도착한 앞 화신 전송의 성공은 새 화신의 실패 되돌리기를 막지 않는다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // 전송 A — 결말을 테스트가 쥐고 있다가 비우기 **뒤에** 성공으로 끝낸다.
    let resolveA!: () => void
    clientMock.writeStdin = vi.fn(
      () =>
        new Promise<void>((res) => {
          resolveA = res
        }),
    )

    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')
    fireEvent.change(textarea(), { target: { value: 'send A' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(emptyState()).toBeNull()

    // 다른 화신이 붙었다 — 전송 흔적이 내려가고 첫 실행 화면으로 돌아온다(인스턴스는 그대로).
    fireReset()
    fireState('live')
    expect(emptyState()).not.toBeNull()

    // 사라진 화신에 보낸 A 가 이제 성공으로 결말난다.
    resolveA()
    await flush()

    // 새 화신에 보낸 B 는 실패 — 이 화신은 아무것도 받지 못했으므로 첫 실행 화면이 정답이다.
    clientMock.writeStdin = vi.fn(async () => {
      throw new Error('write failed')
    })
    fireEvent.change(textarea(), { target: { value: 'send B' } })
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()

    expect(emptyState()).not.toBeNull()
    expect(screen.queryByText('Wait')).toBeNull()
    expect(warn).toHaveBeenCalled()
  })
})

// ★ADR-0148: 타겟한 에이전트가 없는 슬롯★
//
// 프로세스 종료와 연결 끊김을 같게 본다(사용자 결정). 화면 내용은 지우지 않고 — 대화든 첫 실행 화면이든
// 그 시점 모습 그대로 — 흐림 막 + 심볼만 얹는다. 죽은 슬롯이 "이제 시작하면 됨" 처럼 보이던 것이 결함이었다.
describe('RichSlot(live) — ADR-0148 에이전트 부재 표현', () => {
  it('종료된 에이전트: 대화 내용을 지우지 않고 흐림 막 + 심볼만 얹는다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Killed' } }]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()

    // 내용 유지 + 오버레이 존재가 한 화면에 같이 성립해야 한다.
    expect(screen.getByText('assistant reply')).toBeTruthy()
    expect(deadOverlay()).not.toBeNull()
    // 심볼만 — 문구·배너를 넣지 않는다(정보를 하드하게 남기지 않는다는 결정).
    expect(deadOverlay()?.textContent).toBe('')
    expect(deadOverlay()?.querySelector('svg')).not.toBeNull()
    // 입력창은 기존 비활성 처리 그대로.
    expect(textarea().disabled).toBe(true)
    expect(textarea().placeholder).toBe(t('agent.terminatedPlaceholder'))
  })

  it('빈 상태(0건)에서 종료돼도 마스코트·문구를 남긴 채 흐림 막이 덮는다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Exited' } }]
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    fireState('live')

    expect(emptyState()).not.toBeNull()
    expect(mascot()).not.toBeNull()
    expect(screen.getByText('Claude Code')).toBeTruthy()
    expect(deadOverlay()).not.toBeNull()
  })

  // ★종료(kill)의 실제 결말★: reaper 가 세션을 수거하며 **명부에서 지운다**(프로필은 시체로 남는다).
  //   그래서 status 로는 안 잡히고, 명부를 받은 뒤에도 해석 안 되는 id 가 곧 부재다. 이걸 부재로 안 보면
  //   죽은 에이전트의 입력창이 활성으로 남는다.
  it('명부 수신 후 해석 안 되는 에이전트도 부재로 본다 — 내용 유지 + 오버레이 + 입력 차단', async () => {
    // 살아있는 동안 대화가 쌓인 상태를 만든다(명부에 있음).
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' } }]
    agentStoreState.agentsLoaded = true
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    expect(deadOverlay()).toBeNull()
    expect(textarea().disabled).toBe(false)

    // 수거 완료 = 명부에서 사라짐(프로필만 남는다 — 이 컴포넌트는 프로필을 부재 판정에 쓰지 않는다).
    agentStoreState.agents = []
    rerender(<RichSlot viewId="v1" agentId={AGENT} />)

    expect(screen.getByText('assistant reply')).toBeTruthy() // 내용은 그대로
    expect(deadOverlay()).not.toBeNull()
    expect(textarea().disabled).toBe(true)
    expect(textarea().placeholder).toBe(t('agent.terminatedPlaceholder'))
  })

  it('명부를 아직 못 받은 구간(agentsLoaded=false)은 부재로 보지 않는다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()

    // agents 가 비어 있지만 명부 미수신이므로 판정을 보류한다 — 기동 직후 슬롯을 죽이면 안 된다.
    expect(deadOverlay()).toBeNull()
    expect(textarea().disabled).toBe(false)
  })

  it('연결이 끊기면 살아있는 에이전트도 같게 취급하고, 다시 붙으면 걷힌다', async () => {
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    expect(deadOverlay()).toBeNull()

    setConnection('down')
    expect(deadOverlay()).not.toBeNull()
    expect(screen.getByText('assistant reply')).toBeTruthy() // 내용은 그대로

    setConnection('reconnecting') // 붙기 전까진 여전히 부재
    expect(deadOverlay()).not.toBeNull()

    setConnection('connected')
    expect(deadOverlay()).toBeNull()
  })

  // ★막을 올리는 조건 전부가 입력을 막는 조건이다(ADR-0148 결정 1 / ADR-0149 결정 4)★: 막은
  //   pointer-events-none 이라 조작을 가리지 않고, disabled 도 프로그램적 키 이벤트를 막지 못한다.
  //   종료만 보고 막으면 연결이 끊긴 슬롯에서 전송이 나가 실패하고, 사용자에겐 무반응으로만 보인다.
  //   여기서는 에이전트를 명부에 **살려 둔 채** 연결만 끊어 종료가 아닌 갈래를 잰다.
  it('연결이 끊겨 막이 뜬 동안은 에이전트가 살아 있어도 전송이 막히고 입력창이 비활성이다', async () => {
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' } }]
    agentStoreState.agentsLoaded = true
    render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    expect(textarea().disabled).toBe(false)

    // 초안은 붙어 있는 동안 넣는다 — 끊긴 뒤엔 입력창이 비활성이라 값을 채울 수 없다.
    fireEvent.change(textarea(), { target: { value: 'hello' } })

    setConnection('down')
    expect(deadOverlay()).not.toBeNull()
    expect(textarea().disabled).toBe(true)
    ;(clientMock.writeStdin as ReturnType<typeof vi.fn>).mockClear()

    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(clientMock.writeStdin).not.toHaveBeenCalled()
    // 초안은 남는다 — 막힌 전송이 입력을 삼키면 사용자가 쓴 것이 사라진다.
    expect(textarea().value).toBe('hello')

    // 다시 붙으면 그 초안이 그대로 나간다(가드가 잠긴 채 고착되지 않는다).
    setConnection('connected')
    expect(textarea().disabled).toBe(false)
    fireEvent.keyDown(textarea(), { key: 'Enter' })
    await flush()
    expect(clientMock.writeStdin).toHaveBeenCalledTimes(1)
  })
})

// ★정체성 변경 = 새 인스턴스(key remount)★
//
// 구독 effect 가 목록·표시를 내리는 방식만으로는, 정체성이 바뀐 **첫 커밋**이 새 props + 옛 상태로 그려진다
// (passive effect 는 페인트 뒤). 같은 슬롯에 다른 에이전트를 배정하면 그 프레임에 이전 에이전트의 대화가
// 스친다. effect 를 flush 한 뒤 단언하면 이 결함이 안 보이므로, 여기서는 "인스턴스가 갈렸나"를 직접 본다
// (DOM 노드 교체 = 언마운트 후 재마운트).
describe('RichSlot(live) — 정체성이 바뀌면 인스턴스를 새로 마운트한다', () => {
  const OTHER = 'eeee-ffff-0000-1111'

  it('agentId 가 바뀌면 슬롯 루트 노드가 교체된다(옛 상태를 물려받는 커밋이 없다)', async () => {
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()
    expect(screen.getByText('assistant reply')).toBeTruthy()

    const rootBefore = document.querySelector('[data-rich-live="1"]')
    rerender(<RichSlot viewId="v1" agentId={OTHER} />)

    expect(document.querySelector('[data-rich-live="1"]')).not.toBe(rootBefore)
    expect(document.querySelector('[data-rich-live="1"]')?.getAttribute('data-agent-id')).toBe(OTHER)
    expect(screen.queryByText('assistant reply')).toBeNull()
  })

  // ★반대 방향(옛 회귀를 되살리지 말 것)★: 한때는 화신 번호가 이 key 에 들어 있어 재spawn 이 인스턴스를
  //   갈았다. 그러면 replay 가 오기도 전에 대화가 지워지는데, 데몬은 종료와 함께 ring 을 버리므로 그
  //   대화는 어디서도 못 살린다. 지금은 같은 인스턴스를 유지하고, 지우는 일은 **새 화신의 이력이 실제로
  //   도착했을 때** 비우기 신호가 한다(위 onReset 케이스들).
  it('같은 agentId 로 다시 그려도 인스턴스를 갈지 않는다(대화 보존)', async () => {
    const { rerender } = render(<RichSlot viewId="v1" agentId={AGENT} />)
    await flush()
    feedCompletedTurn()

    const rootBefore = document.querySelector('[data-rich-live="1"]')
    rerender(<RichSlot viewId="v1" agentId={AGENT} />)

    expect(document.querySelector('[data-rich-live="1"]')).toBe(rootBefore)
    expect(screen.getByText('assistant reply')).toBeTruthy()
  })
})
