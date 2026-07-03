// RichSlot — 구조화(JSON 모드) 렌더 슬롯. 두 소스 모드(ADR-0044):
//
//  ① fixture 모드(<RichSlot />, agentId 없음) — M0 스파이크. 캡처한 stream-json 샘플을 통짜 파싱해
//     그린다(살아있는 에이전트/데몬 없이 스타일 튜닝용). ViewLayoutRenderer 의 richSlots 오버레이가 쓴다.
//  ② 라이브 모드(<RichSlot agentId epoch />) — M2 본체. 백엔드가 정제한 **구조화 출력 tag1** 프레임을
//     TerminalSlot 과 같은 구독 규율로 받아(효과 deps [agentId,epoch], seq dedup, replay, 정확한 해제)
//     StructuredEventAccumulator 로 누적해 그린다 + 하단 텍스트 입력창(Enter=전송, Shift+Enter=줄바꿈).
//
// ★S15 소스 전환(ADR-0045)★: 라이브 누산 소스가 S14 NDJSON 바이트(StreamAccumulator)에서 tag1
//   StructuredEvent(StructuredEventAccumulator)로 바뀌었다. 백엔드가 출력을 정제해 self-describing
//   이벤트로 흘리므로 프론트는 라인 재조립을 안 하고 이벤트 1건씩 소비한다. tag0(터미널 바이트)이 이
//   슬롯에 오면 무시한다(구조화 슬롯이라 렌더 대상 아님 — tag 게이트). NDJSON 통로/StreamAccumulator 는
//   fixture 스파이크(FixtureRichSlot)가 계속 쓰므로 남긴다.
//
// ★층 분리★: 라이브 모드도 파싱/누적은 순수 TS(structuredAccumulator.ts)가, 렌더는 랩 레이아웃
//   (ChatLayout)이 소유한다. 이 컴포넌트는 "구독 → 누산기 급이 → 결과 렌더 + 입력 캡처"라는 순수 I/O
//   배선만 한다(§5 손발/두뇌 분리: 프론트=I/O, 제어는 백엔드측 핸들).

import { useEffect, useRef, useState } from 'react'

import { agentClient } from '../../api/clientFactory'
import { FRAME_TAG_STRUCTURED_EVENT } from '../../api/wsFrame'
import type { OutputSubscription } from '../../api/agentClient'
import { useAgentStore } from '../../store/agentStore'
import { parseStreamJson } from '../../lab/richslot/parse'
import { StructuredEventAccumulator } from './structuredAccumulator'
import { LAYOUTS, ChatLayout, type LayoutKey } from '../../lab/richslot/layouts'
import {
  RenderSettingsProvider,
  type CodeRender,
  type DiffRender,
} from '../../lab/richslot/renderSettings'
import type { RichMessage } from '../../lab/richslot/types'
import showcaseFixture from '../../lab/richslot/fixtures/showcase.jsonl?raw'
import textFixture from '../../lab/richslot/fixtures/text.jsonl?raw'
import toolFixture from '../../lab/richslot/fixtures/tool.jsonl?raw'
import codeFixture from '../../lab/richslot/fixtures/code.jsonl?raw'
import reviewFixture from '../../lab/richslot/fixtures/review.jsonl?raw'

interface RichSlotProps {
  /** 지정되면 라이브 모드(그 에이전트의 실스트림 구독). 없으면 fixture 스파이크 모드. */
  agentId?: string
  /** 재spawn 재구독 트리거([agentId,epoch]). 라이브 모드에서만 의미. */
  epoch?: number
}

/** 소스 모드 분기 — agentId 있으면 라이브, 없으면 fixture 스파이크. */
export default function RichSlot({ agentId, epoch }: RichSlotProps) {
  if (agentId != null) return <LiveRichSlot agentId={agentId} epoch={epoch ?? 0} />
  return <FixtureRichSlot />
}

// ══════════════════════════════════════════════════════════════════════════════════
// ② 라이브 모드 — 실스트림 구독 + 누산 + 입력창
// ══════════════════════════════════════════════════════════════════════════════════

const LIVE_RENDER_SETTINGS: { codeRender: CodeRender; diffRender: DiffRender } = {
  codeRender: 'plain',
  diffRender: 'inline',
}

function LiveRichSlot({ agentId, epoch }: { agentId: string; epoch: number }) {
  const [messages, setMessages] = useState<RichMessage[]>([])
  const [turnDone, setTurnDone] = useState(false)
  // ★로컬 awaiting 플래그(FIX 5b)★: 전송 직후~첫 응답 바이트 도착 사이의 공백을 메운다. turnDone 은
  //   누산기가 result 라인으로만 내리므로, 직전 턴이 idle 인 상태에서 새로 보내면 첫 바이트 전까지
  //   'idle' 로 보인다. 전송 즉시 이 플래그를 세워 'streaming' 으로 뒤집고, 응답 바이트가 오면 해제해
  //   이후 표시를 turnDone 에 넘긴다.
  const [awaiting, setAwaiting] = useState(false)
  const [input, setInput] = useState('')
  // 누산기는 렌더 간 유지(마운트 1회 생성). 재구독 effect 가 reset 으로 초기화한다(replay 규율).
  const accRef = useRef<StructuredEventAccumulator>(null as unknown as StructuredEventAccumulator)
  if (accRef.current === null) accRef.current = new StructuredEventAccumulator()
  const scrollRef = useRef<HTMLDivElement>(null)

  // 종료 판정(입력 비활성) — TerminalSlot 과 동일하게 store status 로 본다.
  const agents = useAgentStore((s) => s.agents)
  const agent = agents.find((a) => a.id === agentId) ?? null
  const isTerminated =
    agent != null &&
    (agent.status.type === 'Exited' || agent.status.type === 'Killed' || agent.status.type === 'Failed')

  // 출력 구독 — TerminalSlot 규율 미러: [agentId,epoch] deps, 구독 전 누산기 reset(=terminal.reset()),
  // seq dedup(컴포넌트 방어 — 클라도 내부 dedup), 정확한 unsubscribe(stale 가드 토큰은 클라 소유).
  useEffect(() => {
    const acc = accRef.current
    acc.reset() // 재구독 시 이전 누적 제거 → 히스토리 replay 가 동일 상태로 재구성(StrictMode 중복도 방지)
    setMessages([])
    setTurnDone(false)
    setAwaiting(false) // 재구독 경계에서 awaiting 초기화(스트리밍 힌트 stale 방지)

    let sub: OutputSubscription | null = null
    let cancelled = false
    const lastSeq = { current: -1 } // seq dedup(순서 역전·중복 drop)

    agentClient
      .subscribeOutput(agentId, (chunk) => {
        if (cancelled) return
        if (chunk.seq <= lastSeq.current) return
        lastSeq.current = chunk.seq
        // ★tag 게이트(S15/ADR-0045)★: 이 슬롯은 구조화(tag1)만 렌더한다. tag0(터미널 raw 바이트)이 오면
        //   무시한다 — 구조화 에이전트라도 백엔드가 tag0 을 흘릴 수 있고(과도기), xterm 이 아니라 여기서
        //   바이트를 파싱하면 깨진다. seq 는 위에서 이미 전진시켰으므로(tag 무관 한 seq 공간) dedup 은
        //   tag0 를 건너뛰어도 정합하다.
        if (chunk.tag !== FRAME_TAG_STRUCTURED_EVENT) return
        // tag1 payload = StructuredEvent JSON 1건 — 누산기가 파싱·누적(TextDelta 이어붙임)한다.
        acc.feed(chunk.bytes)
        // 새 참조로 set(누산기 내부 배열은 불변 갱신하지만, 상위 배열 참조를 새로 떠 리렌더 보장).
        setMessages([...acc.snapshot()])
        setTurnDone(acc.isTurnDone())
        setAwaiting(false) // 응답 이벤트 도착 → awaiting 해제(이후 표시는 turnDone 이 주도)
      })
      .then((handle) => {
        if (cancelled) {
          handle.unsubscribe()
          return
        }
        sub = handle
      })
      // 구독 실패(직전 kill 등)는 unhandled rejection 방지로 흡수.
      .catch(() => {})

    return () => {
      cancelled = true
      sub?.unsubscribe()
    }
  }, [agentId, epoch])

  // 새 메시지 도착 시 하단으로 스크롤(대화 UX).
  useEffect(() => {
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [messages])

  const send = (): void => {
    // ★1 전송 == 완결된 유저 턴 1개(ADR-0044/0004)★: 텍스트 전체를 한 번에 보낸다. 백엔드 encoder 가
    //   claude 유저 JSON 라인으로 감싸므로 여기서 개행 추가·래핑 금지(raw 텍스트 바이트만).
    // ★전송 게이트는 turnDone 을 검사하지 않는다 — 의도된 동작(ADR-0044 메커니즘 A: 스트리밍 중
    //   mid-turn 가이던스 주입 허용)★. 여기에 턴 잠금(스트리밍 중 전송 차단)을 넣지 말 것.
    if (!input.trim() || isTerminated) return
    // FIX 5a: 앞뒤 공백을 다듬은 텍스트를 전송(가드도 trim 으로 판정하므로 실제 전송도 trim 일관).
    const text = input.trim()
    void agentClient.writeStdin(agentId, new TextEncoder().encode(text)).catch(() => {})
    setInput('')
    setAwaiting(true) // FIX 5b: 첫 바이트를 기다리지 않고 즉시 streaming 힌트로 전환
  }

  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        boxSizing: 'border-box',
        background: 'var(--lay-bg)',
      }}
      data-rich-live="1" // cdp eval 에서 라이브 RichSlot 마운트 여부 확인용
      data-agent-id={agentId}
    >
      {/* 스트리밍/유휴 힌트(옵션) — result 라인 관측(turnDone)으로 판정(저렴) + 전송 직후 awaiting 브리지. */}
      <div style={LIVE_HEADER}>
        <span style={{ color: '#6aa0ff', fontWeight: 700 }}>JSON</span>
        {/* streaming = 응답 진행 중(!turnDone) 이거나 방금 전송해 첫 바이트 대기 중(awaiting, FIX 5b). */}
        <span style={{ color: !turnDone || awaiting ? '#caa' : '#6a6' }}>
          {!turnDone || awaiting ? '○ streaming' : '● idle'}
        </span>
        {isTerminated && <span style={{ color: '#c66' }}>— 종료됨</span>}
      </div>

      {/* 대화 렌더(스크롤). ChatLayout = 사용자 선택 레이아웃(M0 과 동일 RenderSettingsProvider 배선). */}
      <div ref={scrollRef} style={{ flex: 1, minHeight: 0, overflowY: 'auto' }}>
        <RenderSettingsProvider value={LIVE_RENDER_SETTINGS}>
          <ChatLayout messages={messages} />
        </RenderSettingsProvider>
      </div>

      {/* 입력창 — Enter 전송 / Shift+Enter 줄바꿈. ★포커스 가드★: stopPropagation 으로 키 입력이
          상위/전역 키바인딩으로 새지 않게 한다(터미널 슬롯의 onData 캡처와 동형 격리). */}
      <div style={INPUT_BAR}>
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation()
            // ★한국어 IME 조합 확정 Enter 오발사 방지(주 사용자가 한국어)★: WebView2 에서 한글 조합을
            //   확정하는 Enter 는 isComposing=true(keyCode 229)로 keydown 이 온다 — 이걸 전송으로 처리하면
            //   조합만 끝내려던 Enter 가 미완성 입력을 조기 전송한다. 조합 중 Enter 는 전송 분기 전에 흘려보낸다.
            if (e.nativeEvent.isComposing || e.keyCode === 229) return
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault()
              send()
            }
          }}
          placeholder={isTerminated ? '종료된 에이전트' : '메시지 입력 (Enter 전송 · Shift+Enter 줄바꿈)'}
          disabled={isTerminated}
          rows={2}
          style={INPUT_FIELD}
        />
        <button onClick={send} disabled={isTerminated || !input.trim()} style={SEND_BTN}>
          전송
        </button>
      </div>
    </div>
  )
}

const LIVE_HEADER: React.CSSProperties = {
  flex: '0 0 auto',
  display: 'flex',
  gap: 10,
  alignItems: 'center',
  padding: '4px 8px',
  background: '#111',
  color: '#ccc',
  fontFamily: 'system-ui, sans-serif',
  fontSize: 12,
  borderBottom: '1px solid #2a2a2a',
}
const INPUT_BAR: React.CSSProperties = {
  flex: '0 0 auto',
  display: 'flex',
  gap: 6,
  alignItems: 'stretch',
  padding: '6px 8px',
  background: '#111',
  borderTop: '1px solid #2a2a2a',
}
const INPUT_FIELD: React.CSSProperties = {
  flex: 1,
  resize: 'none',
  fontFamily: 'var(--font-ui, system-ui, sans-serif)',
  fontSize: 13,
  background: '#1a1a1a',
  color: '#e0e0e0',
  border: '1px solid #2a2a2a',
  borderRadius: 4,
  padding: '6px 8px',
  boxSizing: 'border-box',
}
const SEND_BTN: React.CSSProperties = {
  flex: '0 0 auto',
  cursor: 'pointer',
  background: '#2a4a8a',
  color: '#fff',
  border: 'none',
  borderRadius: 4,
  padding: '0 14px',
  fontSize: 12,
}

// ══════════════════════════════════════════════════════════════════════════════════
// ① fixture 모드(M0 스파이크) — 캡처 stream-json 통짜 파싱, 스타일 튜닝용(라이브 아님)
// ══════════════════════════════════════════════════════════════════════════════════

// 실측 stream-json 캡처(랩과 동일 Vite raw import). showcase = kitchen-sink(모든 블록 종류 1개씩).
const FIXTURES: Record<string, string> = {
  showcase: showcaseFixture,
  text: textFixture,
  tool: toolFixture,
  code: codeFixture,
  review: reviewFixture,
}

const TOOLBAR: React.CSSProperties = {
  flex: '0 0 auto',
  display: 'flex',
  gap: 10,
  alignItems: 'center',
  flexWrap: 'wrap',
  padding: '4px 8px',
  background: '#111',
  color: '#ccc',
  fontFamily: 'system-ui, sans-serif',
  fontSize: 12,
  borderBottom: '1px solid #2a2a2a',
}
const SELECT: React.CSSProperties = { fontFamily: 'inherit', fontSize: 12 }
const DIM: React.CSSProperties = { color: '#888' }

function FixtureRichSlot() {
  const [fixture, setFixture] = useState('showcase')
  const [layout, setLayout] = useState<LayoutKey>('chat') // 기본 = 대화형(가독 결과)
  // 코드/diff 렌더 — 기본은 가벼운 자체 렌더. 'monaco' 로 켜야 무거운 Monaco 청크가 lazy 로드된다.
  const [codeRender, setCodeRender] = useState<CodeRender>('plain')
  const [diffRender, setDiffRender] = useState<DiffRender>('inline')

  const LayoutComp = LAYOUTS[layout].Comp
  const messages = parseStreamJson(FIXTURES[fixture]) // 통짜 파싱(라이브 아님) — layout 변경 시 재파싱

  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        boxSizing: 'border-box',
        background: 'var(--lay-bg)', // chat 레이아웃은 배경이 없어 랩 다크 톤(#0a0a0a)을 슬롯이 깐다
      }}
      data-rich-spike="1" // cdp eval 에서 RichSlot 마운트 여부 확인용
    >
      <div style={TOOLBAR}>
        <span style={{ color: '#6aa0ff', fontWeight: 700 }}>JSON 스파이크</span>
        <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <span style={DIM}>fixture:</span>
          <select value={fixture} onChange={e => setFixture(e.target.value)} style={SELECT}>
            {Object.keys(FIXTURES).map(k => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
        </span>
        <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <span style={DIM}>layout:</span>
          <select
            value={layout}
            onChange={e => setLayout(e.target.value as LayoutKey)}
            style={SELECT}
          >
            {(Object.keys(LAYOUTS) as LayoutKey[]).map(k => (
              <option key={k} value={k}>
                {LAYOUTS[k].label}
              </option>
            ))}
          </select>
        </span>
        <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <span style={DIM}>code:</span>
          <select
            value={codeRender}
            onChange={e => setCodeRender(e.target.value as CodeRender)}
            style={SELECT}
          >
            <option value="plain">plain</option>
            <option value="monaco">monaco</option>
          </select>
        </span>
        <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <span style={DIM}>diff:</span>
          <select
            value={diffRender}
            onChange={e => setDiffRender(e.target.value as DiffRender)}
            style={SELECT}
          >
            <option value="inline">inline</option>
            <option value="monaco">monaco</option>
          </select>
        </span>
      </div>

      {/* 선택한 layout 이 선택한 fixture 를 렌더(스크롤). 레이아웃 컴포넌트가 자체 overflow-y 를 가짐. */}
      <div style={{ flex: 1, minHeight: 0, overflowY: 'auto' }}>
        <RenderSettingsProvider value={{ codeRender, diffRender }}>
          <LayoutComp messages={messages} />
        </RenderSettingsProvider>
      </div>
    </div>
  )
}
