//! DomSlot — 디버그/관측용 평문 DOM 렌더 슬롯(§5 LLM-우선 제어의 관측 수단).
//!
//! ★역할★: TerminalSlot 과 *같은 출력 스트림*을 xterm(canvas 글리프) 대신 평문 `<pre>` 로 그린다.
//! 왜 필요한가: 터미널 모드 출력은 WebglAddon 이 글리프를 canvas 로 rasterize 하므로
//! `document.body.innerText`/CDP eval 로 읽히지 않는다 → LLM/자동화가 화면 내용을 관측·검증할 길이 없다.
//! DomSlot 은 같은 바이트 스트림을 ANSI 만 벗겨 `<pre>` 텍스트로 붙여 eval 로 읽히게 한다.
//!
//! ★구독 규율은 TerminalSlot 을 그대로 미러★: effect deps [agentId,epoch], 구독 전 누적 초기화,
//! chunk.seq dedup, cancelled 가드, cleanup 에서 unsubscribe. (근거는 각 라인 주석 참조 — TerminalSlot 동형)
//!
//! ★범위★: read-only 관측기다. 입력 처리 없음(입력은 여전히 TerminalSlot/agentClient.writeStdin 경로).
//! 완전한 터미널 에뮬레이터가 목표가 아니다 — 커서 이동/화면 지우기 같은 제어열은 best-effort 로 벗겨
//! "평문이 읽히는" 수준만 노린다(아래 ANSI strip 주석 참조).
//!
//! ★backfill(ADR-0046 이후)★: DOM 모드도 구독 시 requestReplay 로 데몬 ring 전량을 backfill 받는다 —
//! 스왑/remount 도 뷰(slot) 단위 buffering→마커 flush 경로로 전량 재replay 되어 스왑 이전 출력이 복원된다.
//! (구 한계 "LIVE-forward 만 / 스왑 시 backfill 안 됨, ADR-0041" 은 ADR-0046 뷰 직결 replay 로 해소 —
//! 어느 mount 든 requestReplay 전량 backfill 로 채운다.)

import { useEffect, useRef, useState } from 'react'

import { agentClient } from '../../api/clientFactory'
import { FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputSubscription } from '../../api/agentClient'
import { useAgentStore } from '../../store/agentStore'
import { ScrollArea } from '../ui/scroll-area'
import { t } from '../../i18n'

interface DomSlotProps {
  /** 구독 키(ADR-0046) = 슬롯 id. 같은 agentId 두 슬롯도 독립 구독·독립 진도(버그 B 해소). */
  viewId: string
  agentId: string
  /** 재spawn 재구독 트리거([agentId,epoch]) — TerminalSlot 과 동일. */
  epoch: number
}

// 누적 텍스트 상한(약 200KB). 무한 성장 방지 — 관측용이라 최근 출력만 보이면 충분하므로 tail 만 남긴다.
// (터미널 스크롤백처럼 완전 보존이 목적이 아님. 넘치면 앞부분을 잘라 최근 ~200KB 유지.)
const MAX_TEXT_LEN = 200_000

// ANSI/제어열 strip 정규식(best-effort — 완전한 터미널 에뮬레이터 아님, 파일 헤더 참조).
//   - ESC [ ... <final>  = CSI 시퀀스(색·커서이동·화면지우기 등). 파라미터/중간 바이트 삼키고 final 로 끝.
//   - ESC ] ... (BEL|ST) = OSC 시퀀스(창 제목 등). BEL(\x07) 또는 ST(ESC \) 로 종료.
//   - ESC <single>       = 위 둘에 안 걸리는 2바이트 ESC 시퀀스.
// 목적은 "평문 가독"이지 픽셀 재현이 아니다 — 색만 지워도 innerText 관측엔 충분하다.
// eslint-disable-next-line no-control-regex
const ANSI_RE = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]/g

function stripAnsi(s: string): string {
  return s.replace(ANSI_RE, '')
}

// pending(미완 ESC 시퀀스) 상한 — 종료 바이트 없는 ESC 가 청크를 넘어 무한히 쌓이는 걸 막는다.
// CSI/OSC 시퀀스는 정상적으로 이보다 훨씬 짧다(색·커서열 수 바이트, OSC 창제목 수십 바이트). 초과하면
// "이건 진짜 시퀀스가 아니라 lone ESC" 로 보고 그냥 흘려보낸다(best-effort — 아래 splitTrailingEsc 참조).
const MAX_PENDING_LEN = 256

// ★청크 경계에 걸친 ANSI 시퀀스 보존(FIX-3, best-effort)★: ANSI strip 은 청크 단위로 도는데, 시퀀스가
// 두 청크에 쪼개지면(예: 앞 청크가 "\x1b[" 로 끝, 뒤 청크가 "31mred" 로 시작) 정규식이 반쪽을 못 지워
// 원문 ESC[... 가 화면에 샌다. 그래서 청크 끝에 *이 청크 안에서 종료되지 않은* ESC 가 있으면 그 ESC 부터
// 끝까지를 잘라 다음 청크 앞에 이어 붙일 pending 으로 넘긴다.
//   반환 [head, pending]: head=지금 strip·append 할 부분, pending=다음 청크 앞에 prepend 할 미완 꼬리.
// 판정은 단순 휴리스틱 — 마지막 ESC(\x1b) 위치를 찾아, 그 뒤에 '시퀀스를 종료시키는 바이트'가 없으면
// 미완으로 본다(완전한 에뮬레이터 아님, 파일 헤더 참조). CSI(ESC[…final @-~)·OSC(ESC]…BEL/ST)·2바이트
// ESC 를 모두 아우르는 근사: ESC 뒤에 CSI final(@-~) 또는 OSC 종료(BEL/ESC)가 아직 안 나왔으면 hold.
function splitTrailingEsc(s: string): [string, string] {
  const esc = s.lastIndexOf('\x1b')
  if (esc < 0) return [s, '']
  const tail = s.slice(esc) // 마지막 ESC 부터 끝까지
  // 이 tail 이 이미 완결된 시퀀스면(ANSI_RE 가 tail 시작에서 매치) hold 불필요 — 전체를 head 로.
  // (lastIndexOf 라 tail 안에 ESC 는 하나뿐 → 매치가 곧 "완결"을 뜻한다.)
  ANSI_RE.lastIndex = 0
  const m = ANSI_RE.exec(tail)
  if (m && m.index === 0 && m[0].length === tail.length) return [s, '']
  // 미완 꼬리를 hold. 단 상한 초과(종료 없는 lone ESC 누적)면 hold 하지 않고 전부 흘려보낸다(무한성장 방지).
  if (tail.length > MAX_PENDING_LEN) return [s, '']
  return [s.slice(0, esc), tail]
}

export default function DomSlot({ viewId, agentId, epoch }: DomSlotProps) {
  // 누적 출력(평문). React state 로 들고 리렌더 — 관측용이라 xterm 같은 명령형 write 대신 선언적 렌더.
  const [text, setText] = useState('')
  // ★scrollRef = ScrollArea Viewport(공용 seam 이 forward, ADR-0053)★: 실제 overflow/scrollTop 노드는
  //   Radix Viewport 다. 하단 고정 auto-scroll(scrollTop=scrollHeight)이 이 노드를 겨눠야 tail 이 붙는다
  //   (구 raw <pre overflow:auto> 는 pre 자신이 스크롤 노드였음 — seam 전환으로 대상이 Viewport 로 이동).
  const scrollRef = useRef<HTMLDivElement>(null)

  // 종료 판정(오버레이 표시용) — TerminalSlot/RichSlot 과 동일하게 store status 로 본다.
  const agents = useAgentStore(s => s.agents)
  const agent = agents.find(a => a.id === agentId) ?? null
  const isTerminated =
    agent != null &&
    (agent.status.type === 'Exited' ||
      agent.status.type === 'Killed' ||
      agent.status.type === 'Failed')

  // 출력 구독 — TerminalSlot 규율 미러(파일 헤더 참조).
  useEffect(() => {
    setText('') // C2: 재구독 시 이전 출력 제거(StrictMode 중복·재spawn replay 재구성)
    const decoder = new TextDecoder() // stream=true 로 청크 경계에 걸친 멀티바이트 UTF-8 보존
    const lastSeq = { current: -1 } // T-2/G-2: seq dedup(컴포넌트 방어 — 클라도 내부 dedup)
    // FIX-3: 청크 경계에 걸린 미완 ANSI 시퀀스를 다음 청크로 넘길 버퍼(text 누적기와 같은 lifecycle —
    // 재구독마다 여기서 초기화). splitTrailingEsc 가 채우고, 다음 청크 앞에 prepend 된다.
    let pending = ''

    let sub: OutputSubscription | null = null
    let cancelled = false

    agentClient
      .subscribeOutput(viewId, agentId, chunk => {
        if (cancelled) return
        if (chunk.seq <= lastSeq.current) return // T-2: 순서 역전·중복 drop
        lastSeq.current = chunk.seq
        // ★tag 게이트(S15/ADR-0045)★: DOM 모드는 터미널 raw 바이트(tag0)를 평문으로 그리는 관측기다.
        //   tag1(StructuredEvent JSON)이 오면 무시한다 — TerminalSlot 과 동형(같은 tag0 소비자). 게이트가
        //   없으면 tag1 JSON 바이트가 ANSI strip 을 거쳐 <pre> 에 그대로 새어 관측 텍스트가 오염된다.
        //   seq 는 위에서 이미 전진시켰으므로 tag1 을 건너뛰어도 dedup 정합(tag 무관 한 seq 공간).
        if (chunk.tag !== FRAME_TAG_TERMINAL_BYTES) return
        // 이전 청크가 남긴 미완 ESC 꼬리(pending)를 이번 디코드 앞에 이어 붙인 뒤, 새 미완 꼬리를 다시
        // 잘라낸다 — 그래야 두 청크에 쪼개진 시퀀스가 온전히 이어져 strip 된다(FIX-3).
        const decoded = pending + decoder.decode(chunk.bytes, { stream: true })
        const [head, tail] = splitTrailingEsc(decoded)
        pending = tail
        const piece = stripAnsi(head)
        setText(prev => {
          const next = prev + piece
          // 상한 초과 시 앞부분 잘라 최근 MAX_TEXT_LEN 만 유지(무한 성장 방지 — 위 상수 주석).
          return next.length > MAX_TEXT_LEN ? next.slice(next.length - MAX_TEXT_LEN) : next
        })
      })
      .then(handle => {
        if (cancelled) {
          handle.unsubscribe()
          return
        }
        sub = handle
      })
      // 구독 실패(예: 직전 kill 로 NotFound)는 unhandled rejection 방지용으로 흡수(TerminalSlot 동형).
      .catch(() => {})

    return () => {
      cancelled = true
      sub?.unsubscribe()
    }
    // epoch 포함 — 재spawn(같은 agentId, epoch++) 시 reset → 재구독 → replay 재생(S9 §18-e/f).
    // ★렌더러 스왑/remount 도 재구독이 requestReplay 전량 backfill 로 해소(ADR-0046)★ — 스왑 이전 출력도
    //   뷰 buffering→마커 flush 로 복원된다(구 "LIVE-forward 만" 한계는 ADR-0046 로 해소, 파일 헤더 참조).
    // viewId 포함 — 구독 키(ADR-0046, 같은 agentId 두 슬롯 독립).
  }, [viewId, agentId, epoch])

  // 새 출력 도착 시 하단으로 자동 스크롤(터미널 tail 관측 UX). ★대상 = ScrollArea Viewport★(위 scrollRef 주석).
  useEffect(() => {
    const el = scrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [text])

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative', boxSizing: 'border-box' }}>
      {/* 스크롤 표면 = 공용 ScrollArea seam(ADR-0053) — 구 raw <pre overflow:auto> 를 오버레이 스크롤바로
          교체. ref 는 Viewport(실제 스크롤 노드)로 forward 되어 하단 고정 auto-scroll 대상이 된다.
          data-dom-mode / data-agent-id: cdp eval·테스트에서 DOM 모드 마운트 여부·대상 확인용 마커는 안쪽
          <pre>(관측 텍스트 노드)에 유지한다(RichSlot 관례 동형 — textContent 로 읽힌다).
          입력 처리 없음 — read-only 관측기(입력은 TerminalSlot/agentClient.writeStdin 경로, 파일 헤더 참조). */}
      <ScrollArea
        ref={scrollRef}
        style={{ width: '100%', height: '100%', background: 'var(--bg)' }}
      >
        <pre
          data-dom-mode="1"
          data-agent-id={agentId}
          style={{
            margin: 0,
            padding: '4px 8px',
            boxSizing: 'border-box',
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
            color: 'var(--text)',
            fontFamily: 'var(--font-terminal)',
            fontSize: '13px',
          }}
        >
          {text}
        </pre>
      </ScrollArea>
      {isTerminated && (
        <div
          style={{
            position: 'absolute',
            top: 0,
            right: 0,
            padding: '2px 6px',
            color: 'var(--text-muted)',
            fontFamily: 'var(--font-ui)',
            fontSize: '11px',
            pointerEvents: 'none',
          }}
        >
          {agent!.status.type === 'Failed'
            ? `Failed: ${(agent!.status as { type: 'Failed'; message: string }).message}`
            : t('agent.terminatedOverlay')}
        </div>
      )}
    </div>
  )
}
