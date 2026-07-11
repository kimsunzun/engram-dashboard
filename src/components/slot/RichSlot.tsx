// RichSlot — 구조화(JSON 모드) 라이브 렌더 슬롯(ADR-0044/0045).
//
//  라이브 모드(<RichSlot agentId epoch />) — 백엔드가 정제한 **구조화 출력 tag1** 프레임을 TerminalSlot 과
//  같은 구독 규율로 받아(효과 deps [agentId,epoch], seq dedup, replay, 정확한 해제) StructuredEventAccumulator
//  로 누적해 그린다 + 하단 텍스트 입력창(Enter=전송, Shift+Enter=줄바꿈).
//
// ★M0 fixture 스파이크 제거(Brick 1)★: 살아있는 에이전트/데몬 없이 stream-json 샘플을 통짜 파싱해 그리던
//   FixtureRichSlot(<RichSlot />, agentId 없음)과 그 lab/richslot 의존은 레거시 프론트 레이아웃 정리와 함께
//   제거됐다. 스타일 튜닝은 lab 엔트리(별도)로 하고, 이 컴포넌트는 라이브 경로만 소유한다.
//
// ★S15 소스 전환(ADR-0045)★: 라이브 누산 소스가 S14 NDJSON 바이트 파서에서 tag1
//   StructuredEvent(StructuredEventAccumulator)로 바뀌었다. 백엔드가 출력을 정제해 self-describing
//   이벤트로 흘리므로 프론트는 라인 재조립을 안 하고 이벤트 1건씩 소비한다. tag0(터미널 바이트)이 이 슬롯에
//   오면 무시한다(구조화 슬롯이라 렌더 대상 아님 — tag 게이트).
//
// ★층 분리★: 파싱/누적은 순수 TS(structuredAccumulator.ts)가, 렌더는 전용 컴포넌트(StructuredTextView)가
//   소유한다. 이 컴포넌트는 "구독 → 누산기 급이 → 결과 렌더 + 입력 캡처"라는 순수 I/O 배선만 한다
//   (§5 손발/두뇌 분리: 프론트=I/O, 제어는 백엔드측 핸들).

import { useEffect, useRef, useState } from 'react'

import { agentClient } from '../../api/clientFactory'
import { FRAME_TAG_STRUCTURED_EVENT } from '../../api/wsFrame'
import type { OutputSubscription } from '../../api/agentClient'
import { useAgentStore } from '../../store/agentStore'
import { StructuredEventAccumulator, type StructuredItem } from './structuredAccumulator'
import { StructuredTextView } from './StructuredTextView'
import { ScrollArea } from '../ui/scroll-area' // ADR-0053: 앱 전역 Radix 오버레이 스크롤바 seam

interface RichSlotProps {
  /** 구독 키(ADR-0046) = 슬롯 id. 같은 agentId 두 슬롯 독립 진도 — 버그 B 해소. */
  viewId?: string
  /** 라이브 대상 에이전트(그 에이전트의 실스트림 구독). */
  agentId: string
  /** 재spawn 재구독 트리거([agentId,epoch]). */
  epoch?: number
}

/** 라이브 구조화 슬롯 — agentId 의 실스트림을 구독해 누적·렌더한다. */
export default function RichSlot({ viewId, agentId, epoch }: RichSlotProps) {
  return <LiveRichSlot viewId={viewId ?? agentId} agentId={agentId} epoch={epoch ?? 0} />
}

// ══════════════════════════════════════════════════════════════════════════════════
// ② 라이브 모드 — 실스트림 구독 + 누산 + 입력창
// ══════════════════════════════════════════════════════════════════════════════════

function LiveRichSlot({ viewId, agentId, epoch }: { viewId: string; agentId: string; epoch: number }) {
  // 순서 보존 렌더 item 스트림(text/칩/구분선) — 누산기 스냅샷을 그대로 담는다(ADR-0045 §52).
  const [items, setItems] = useState<StructuredItem[]>([])
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
    setItems([])
    setTurnDone(false)
    setAwaiting(false) // 재구독 경계에서 awaiting 초기화(스트리밍 힌트 stale 방지)

    let sub: OutputSubscription | null = null
    let cancelled = false
    const lastSeq = { current: -1 } // seq dedup(순서 역전·중복 drop)

    agentClient
      .subscribeOutput(viewId, agentId, (chunk) => {
        if (cancelled) return
        if (chunk.seq <= lastSeq.current) return
        lastSeq.current = chunk.seq
        // ★tag 게이트(S15/ADR-0045)★: 이 슬롯은 구조화(tag1)만 렌더한다. tag0(터미널 raw 바이트)이 오면
        //   무시한다 — 구조화 에이전트라도 백엔드가 tag0 을 흘릴 수 있고(과도기), xterm 이 아니라 여기서
        //   바이트를 파싱하면 깨진다. seq 는 위에서 이미 전진시켰으므로(tag 무관 한 seq 공간) dedup 은
        //   tag0 를 건너뛰어도 정합하다.
        if (chunk.tag !== FRAME_TAG_STRUCTURED_EVENT) return
        // tag1 payload = StructuredEvent JSON 1건 — 누산기가 파싱·순서 보존 item 스트림에 누적한다.
        acc.feed(chunk.bytes)
        // 새 참조로 set(누산기 내부 배열을 in-place 갱신하므로, 상위 배열 참조를 새로 떠 리렌더 보장).
        setItems([...acc.snapshot()])
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
    // viewId 포함 — 구독 키(ADR-0046, 같은 agentId 두 슬롯 독립). epoch = 재spawn 재구독 트리거.
  }, [viewId, agentId, epoch])

  // 새 item 도착 시 하단으로 스크롤(대화 UX). ★scrollRef = Radix Viewport(ScrollArea seam 이 forward)★:
  //   Radix ScrollArea 의 실제 스크롤 엘리먼트는 Root 가 아니라 Viewport 다(ADR-0053). auto-scroll 이
  //   이 Viewport 노드의 scrollTop 을 겨눠야 새 출력이 바닥에 붙는다(Root 를 겨누면 스크롤 안 됨 — 회귀 주의).
  useEffect(() => {
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [items])

  const send = (): void => {
    // ★1 전송 == 완결된 유저 턴 1개(ADR-0044/0004)★: 텍스트 전체를 한 번에 보낸다. 백엔드 encoder 가
    //   claude 유저 JSON 라인으로 감싸므로 여기서 개행 추가·래핑 금지(raw 텍스트 바이트만).
    // ★전송 게이트는 turnDone 을 검사하지 않는다 — 의도된 동작(ADR-0044 메커니즘 A: 스트리밍 중
    //   mid-turn 가이던스 주입 허용)★. 여기에 턴 잠금(스트리밍 중 전송 차단)을 넣지 말 것.
    if (!input.trim() || isTerminated) return
    // FIX 5a: 앞뒤 공백을 다듬은 텍스트를 전송(가드도 trim 으로 판정하므로 실제 전송도 trim 일관).
    const text = input.trim()
    setInput('')
    setAwaiting(true) // FIX 5b: 첫 바이트를 기다리지 않고 즉시 streaming 힌트로 전환
    // ★write 실패 시 awaiting 해제★: writeStdin promise 가 reject 되면(전송 자체 실패) 응답 이벤트가
    //   영영 안 온다 → awaiting 이 계속 걸려 'streaming'/Thinking 표시가 무한 고착된다. 실패 경로에서
    //   awaiting 을 되돌려 UI 를 idle 로 복귀시킨다(파생 streaming 값만 교정 — WIRE 불변, ADR-0044/45/46).
    void agentClient.writeStdin(agentId, new TextEncoder().encode(text)).catch((err) => {
      console.warn('[RichSlot] writeStdin failed — clearing awaiting:', err)
      setAwaiting(false)
    })
  }

  // 스트리밍 표시 = 방금 전송해 첫 바이트 대기 중(awaiting)이거나, 실제 응답 진행 중(!turnDone && 이미 item 존재).
  //   ★FIX 5★: 초기 turnDone=false 인데 items 가 비어 있으면(fresh/idle 슬롯) !turnDone 만으로 shimmer·streaming
  //   배지가 뜨는 오작동이 있었다. items.length>0 조건으로 좁혀 idle 을 idle 로 표시하되, (a) 실제 스트리밍 중
  //   신호와 (b) '전송 직후 첫 토큰 대기(awaiting)' 는 그대로 살린다.
  //   (파생 표현값 — 구독/누산/send 데이터 흐름은 건드리지 않는다. ADR-0044/0045/0046.)
  const streaming = awaiting || (!turnDone && items.length > 0)

  return (
    <div
      className="flex h-full w-full flex-col bg-background"
      data-rich-live="1" // cdp eval 에서 라이브 RichSlot 마운트 여부 확인용
      data-agent-id={agentId}
    >
      {/* 대화 렌더(스크롤) — ScrollArea seam(ADR-0053: 앱 전역 Radix 오버레이 스크롤바). 순서 보존 item 스트림.
          ★scrollRef 는 이 seam 이 실제 스크롤 노드(Radix Viewport)로 forward 한다 — 아래 하단 고정 auto-scroll
          이 그 Viewport 노드를 겨눠야 새 출력이 바닥에 붙는다(회귀 주의). CC 룩 렌더는 StructuredTextView 소관.
          (구 "JSON ● idle" 슬림 헤더는 제거 — 상태 힌트는 스트림 끝 대기 인디케이터(WaitRow "Wait" tail) 로 대체.) */}
      <ScrollArea ref={scrollRef} className="min-h-0 flex-1">
        <StructuredTextView items={items} streaming={streaming} />
      </ScrollArea>

      {/* 입력창 — Enter 전송 / Shift+Enter 줄바꿈(별도 전송 버튼 없음). ★포커스 가드★: stopPropagation
          으로 키 입력이 상위/전역 키바인딩으로 새지 않게 한다(터미널 슬롯의 onData 캡처와 동형 격리). */}
      <div className="flex flex-none items-stretch border-t border-border px-2 py-1.5">
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
          className="flex-1 resize-none rounded border border-border bg-surface px-2 py-1.5 text-[13px] text-foreground outline-none placeholder:text-muted focus:border-accent disabled:opacity-50"
        />
      </div>
    </div>
  )
}
