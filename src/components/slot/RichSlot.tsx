// RichSlot — 구조화(JSON 모드) 라이브 렌더 슬롯(ADR-0044/0045).
//
//  라이브 모드(<RichSlot agentId epoch />) — 백엔드가 정제한 **구조화 출력 tag1** 프레임을 TerminalSlot 과
//  같은 구독 규율로 받아 StructuredEventAccumulator 로 누적해 그린다 + 텍스트 입력창(Enter=전송,
//  Shift+Enter=줄바꿈). 입력창 배치는 둘 — 대화가 있으면 하단 고정, 빈 상태면 화면 가운데(아래 ADR-0145).
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
// ★빈 상태(ADR-0145)★: 복원 완료 신호('live') + 0건 + 미전송일 때만 마스코트·"Claude Code"·가운데
//   입력창을 그린다. 판정에만 개입하고 구독·누산·전송 경로는 건드리지 않는다. 입력창은 두 배치가
//   같은 엘리먼트다(전송·IME·포커스 가드를 한 벌로 유지 — 갈라 두지 말 것).
//
// ★층 분리★: 파싱/누적은 순수 TS(structuredAccumulator.ts)가, 렌더는 전용 컴포넌트(StructuredTextView)가
//   소유한다. 이 컴포넌트는 "구독 → 누산기 급이 → 결과 렌더 + 입력 캡처"라는 순수 I/O 배선만 한다
//   (§5 손발/두뇌 분리: 프론트=I/O, 제어는 백엔드측 핸들).

import { useEffect, useRef, useState } from 'react'
import { PowerOff } from 'lucide-react'

import { agentClient } from '../../api/clientFactory'
import { FRAME_TAG_STRUCTURED_EVENT } from '../../api/wsFrame'
import type { OutputSubscription } from '../../api/agentClient'
import { useAgentStore } from '../../store/agentStore'
import { StructuredEventAccumulator, type StructuredItem } from './structuredAccumulator'
import { StructuredTextView } from './StructuredTextView'
import { ClaudeMascot } from './ClaudeMascot'
import { ScrollArea } from '../ui/scroll-area' // ADR-0053: 앱 전역 Radix 오버레이 스크롤바 seam
import { basename } from '../../util/basename'
import { t } from '../../i18n'

interface RichSlotProps {
  /** 구독 키(ADR-0046) = 슬롯 id. 같은 agentId 두 슬롯 독립 진도 — 버그 B 해소. */
  viewId?: string
  agentId: string
  /** 재spawn 재구독 트리거([agentId,epoch]). */
  epoch?: number
}

export default function RichSlot({ viewId, agentId, epoch }: RichSlotProps) {
  const vid = viewId ?? agentId
  const ep = epoch ?? 0
  // ★정체성(viewId·agentId·epoch)이 바뀌면 새로 마운트한다 — key★
  //   목록·복원표시·전송표시를 구독 effect 에서 내리면, 그 커밋 한 번은 **새 props + 옛 상태**로 그려진다
  //   (passive effect 는 페인트 뒤에 돈다). 같은 슬롯에 다른 에이전트를 배정하면 그 프레임에 이전
  //   에이전트의 대화가 스친다. key 로 인스턴스를 갈면 옛 상태를 물려받는 커밋이 아예 존재하지 않는다.
  //   부작용 = 입력 중이던 초안 소실. 다른 에이전트·새 세션으로 갈아타는 전환이라 수용한다.
  //   ★이 key 를 빼면 send 의 "세대" 가드를 따로 되살려야 한다★ — 지금은 재구독 = 새 인스턴스라,
  //   건너온 전송 실패가 옛 인스턴스의 setState 를 부르고 그건 무해한 no-op 이 된다.
  return <LiveRichSlot key={`${vid}|${agentId}|${ep}`} viewId={vid} agentId={agentId} epoch={ep} />
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
  // ADR-0145: 이력 복원이 끝났다는 신호('live')를 받았나. 빈 상태 표시의 게이트이며 재구독마다 내린다.
  const [replayDone, setReplayDone] = useState(false)
  // ★ADR-0145: "이 인스턴스에서 이미 보냈다"★ — 전송이 실제로 나갔다는 사실. 전송 자체가 실패하면
  //   아래 send 의 catch 가 되돌린다(아무것도 안 나간 슬롯은 첫 실행 화면으로 돌아가야 한다).
  //   awaiting 으로 대신하지 않는 이유 = awaiting 은 item 을 만들지 않는 tag1 프레임(빈 델타 · user uuid
  //   dedup 스킵 · 빈 MessageDone)에도 풀려서, 응답이 진행 중인데 items 가 아직 0건인 구간에 마스코트가
  //   되돌아온다. 빈 상태는 "대화를 시작했나"만 보면 되므로 그 사실을 따로 든다(awaiting 의 스트리밍 힌트
  //   역할은 그대로 — ADR-0044/0045).
  const [hasSent, setHasSent] = useState(false)
  // 재구독 effect 가 reset 으로 초기화한다(replay 규율).
  const accRef = useRef<StructuredEventAccumulator>(null as unknown as StructuredEventAccumulator)
  if (accRef.current === null) accRef.current = new StructuredEventAccumulator()
  const scrollRef = useRef<HTMLDivElement>(null)
  // 전송 순번 — 실패 콜백이 "내가 최신 전송인가"를 판정하는 근거. 앞선 전송의 뒤늦은 실패가 진행 중인
  //   최신 전송의 표시를 지우는 것을 막는다(state 가 아니라 ref: 표시에 안 쓰이니 리렌더 불필요).
  const sendSeqRef = useRef(0)
  // 이 인스턴스에서 성공적으로 나간 전송이 하나라도 있나. 있으면 이후 전송이 실패해도 첫 실행 화면으로
  //   되돌리지 않는다 — 앞선 전송의 응답이 오는 중일 수 있다.
  const sendOkRef = useRef(false)

  const agents = useAgentStore((s) => s.agents)
  const agent = agents.find((a) => a.id === agentId) ?? null
  // ADR-0148: 권위 명부를 받았나 — 받기 전(기동·재연결 직후)의 빈 명부를 "없어졌다" 로 오인하지 않기 위한 가드.
  const agentsLoaded = useAgentStore((s) => s.agentsLoaded)
  // ★부재 = terminal 상태로 발견 ∪ 명부 수신 후에도 해석 안 됨(ADR-0148)★. 후자가 종료(kill)의 실제 결말이다
  //   — reaper 가 세션을 수거하며 명부에서 지우므로 status 로는 잡히지 않는다. 이걸 부재로 안 보면 죽은
  //   에이전트의 입력창이 활성으로 남아 전송을 시도할 수 있다.
  const agentGone =
    (agentsLoaded && agent == null) ||
    (agent != null &&
      (agent.status.type === 'Exited' || agent.status.type === 'Killed' || agent.status.type === 'Failed'))

  // ADR-0146: 연결이 끊긴 것도 "타겟한 에이전트가 지금 없다" 로 같게 본다(사용자 결정). 기존 연결 상태
  //   표면(agentClient)을 **읽기만** 한다 — 새 전역 핸들·새 스토어를 만들지 않는다(ConnectionNotice 와
  //   동형). 구독 콜백은 등록 즉시 현재 상태로 1회 발화하므로 초기값 동기화가 따로 필요 없다.
  const [connected, setConnected] = useState(() => agentClient.connectionState === 'connected')
  useEffect(() => agentClient.onConnectionStateChange((s) => setConnected(s === 'connected')), [])
  const agentUnavailable = agentGone || !connected

  // ★정체성 라벨(§ user request)★: json 모드는 터미널의 claude 웰컴 배너 같은 "어느 에이전트인지" 신호가
  //   없다. 우측 상단에 작은 라벨을 오버랩해 이름만 표시한다(아래 render — 줄을 차지하지 않음). 표시명은
  //   트리(displayNameOf)와 동일 규칙: display_name override ?? agent.name ?? cwd basename(중복 이름 허용).
  const profiles = useAgentStore((s) => s.profiles)
  // profiles 는 실 store 에선 항상 배열([])이지만, 일부 단위테스트 mock 은 s.profiles 를 안 채운다 → 방어적 옵셔널.
  const profile = profiles?.find((p) => p.id === agentId) ?? null
  const cwd = agent?.cwd ?? profile?.cwd ?? ''
  // ★basename 만★: agent.name 은 프로필의 name(=우리가 createClaudeProfile 에 넘긴 full cwd)이라 풀 경로가
  //   뜬다 → 트리(displayNameOf)와 동일하게 display_name override 없으면 basename(cwd)만 쓴다("Filter Library").
  const headerName = profile?.display_name ?? basename(cwd)

  // 출력 구독 — TerminalSlot 규율 미러: seq dedup(컴포넌트 방어 — 클라도 내부 dedup),
  // 정확한 unsubscribe(stale 가드 토큰은 클라 소유).
  useEffect(() => {
    const acc = accRef.current
    acc.reset() // 히스토리 replay 가 동일 상태로 재구성(StrictMode 중복도 방지)
    setItems([])
    setTurnDone(false)
    setAwaiting(false) // 스트리밍 힌트 stale 방지
    // ADR-0145: 복원 완료 표시도 여기서 내린다 — 안 내리면 이전 세션의 완료 상태를 물려받아 목록이 빈
    //   복원 구간에 빈 상태가 떴다가 대화로 바뀌는 깜빡임이 되살아난다.
    // ★정체성 변경(viewId·agentId·epoch)은 key remount 가 처리한다★ — 이 자리의 초기화가 실제로 일 하는
    //   경우는 StrictMode 이중 마운트처럼 **같은 인스턴스에서 effect 가 다시 도는** 경로뿐이다(누산기 ref 는
    //   그 사이 살아남는다). 그래도 남겨 둔다 — key 가 사라지면 이게 유일한 방어선이다.
    setReplayDone(false)
    setHasSent(false)

    let sub: OutputSubscription | null = null
    let cancelled = false
    const lastSeq = { current: -1 }

    agentClient
      .subscribeOutput(
        viewId,
        agentId,
        (chunk) => {
          if (cancelled) return
          if (chunk.seq <= lastSeq.current) return
          lastSeq.current = chunk.seq
          // ★tag 게이트(S15/ADR-0045)★: 이 슬롯은 구조화(tag1)만 렌더한다. tag0(터미널 raw 바이트)이 오면
          //   무시한다 — 구조화 에이전트라도 백엔드가 tag0 을 흘릴 수 있고(과도기), xterm 이 아니라 여기서
          //   바이트를 파싱하면 깨진다. seq 는 위에서 이미 전진시켰으므로(tag 무관 한 seq 공간) dedup 은
          //   tag0 를 건너뛰어도 정합하다.
          if (chunk.tag !== FRAME_TAG_STRUCTURED_EVENT) return
          // tag1 payload = StructuredEvent JSON 1건.
          acc.feed(chunk.bytes)
          // 새 참조로 set(누산기 내부 배열을 in-place 갱신하므로, 상위 배열 참조를 새로 떠 리렌더 보장).
          setItems([...acc.snapshot()])
          setTurnDone(acc.isTurnDone())
          setAwaiting(false) // 이후 표시는 turnDone 이 주도
        },
        // ADR-0145: replay 상태 콜백 — 'live' 는 데몬이 복원 끝에 넣은 표식을 클라가 소비해 버퍼를 비운
        //   시점이다(protocolClient.flushToLive). 이력이 0건인 새 에이전트에도 같은 신호가 오므로
        //   "복원 끝 + 0건"이 정확히 갈린다. 'buffering'(복원 중) · 'error'(replay 재요청 소진)에서는
        //   내려 둔다 — 둘 다 빈 상태를 그리면 안 되는 구간이다(복원 중 깜빡임 / 실패는 현행 빈 화면 유지).
        (state) => {
          if (cancelled) return
          setReplayDone(state === 'live')
        },
      )
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

  // ★scrollRef = Radix Viewport(ScrollArea seam 이 forward)★:
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
    if (!input.trim() || agentGone) return
    // FIX 5a: 가드도 trim 으로 판정하므로 실제 전송도 trim 일관.
    const text = input.trim()
    setInput('')
    setAwaiting(true) // FIX 5b: 첫 바이트를 기다리지 않고 즉시 streaming 힌트로 전환
    setHasSent(true) // ADR-0145: 대화가 시작됐으므로 첫 실행 화면을 내린다(실패하면 아래에서 되돌린다)
    const token = ++sendSeqRef.current
    // ★write 실패 시 되돌리기★: writeStdin promise 가 reject 되면(전송 자체 실패) 응답 이벤트가 영영
    //   안 온다 → awaiting 이 계속 걸려 'streaming'/Thinking 표시가 무한 고착되고, hasSent 가 남아 아무것도
    //   보내지 못한 슬롯이 첫 실행 화면으로 못 돌아간다. 실패 경로에서 둘 다 되돌린다(파생 표현값만 교정 —
    //   WIRE 불변, ADR-0044/45/46).
    // ★단 최신 전송의 실패만 되돌린다★: 이 catch 는 어느 전송의 실패인지 스스로 모른다. A→B 로 보낸 뒤
    //   A 가 뒤늦게 reject 되면, 가드가 없으면 진행 중인 B 의 대기 표시를 지운다. 재구독을 건너온 실패는
    //   key remount 로 이미 옛 인스턴스에 떨어지므로(위 RichSlot) 여기서는 같은 인스턴스 안 순번만 본다.
    void agentClient
      .writeStdin(agentId, new TextEncoder().encode(text))
      .then(() => {
        sendOkRef.current = true
      })
      .catch((err) => {
        console.warn('[RichSlot] writeStdin failed — clearing awaiting:', err)
        if (sendSeqRef.current !== token) return
        setAwaiting(false)
        // 앞서 성공한 전송이 있으면 그 응답이 오는 중이므로 첫 실행 화면으로 되돌리지 않는다.
        if (!sendOkRef.current) setHasSent(false)
      })
  }

  // ★FIX 5★: 초기 turnDone=false 인데 items 가 비어 있으면(fresh/idle 슬롯) !turnDone 만으로 shimmer·streaming
  //   배지가 뜨는 오작동이 있었다. items.length>0 조건으로 좁혀 idle 을 idle 로 표시하되, (a) 실제 스트리밍 중
  //   신호와 (b) '전송 직후 첫 토큰 대기(awaiting)' 는 그대로 살린다.
  //   (파생 표현값 — 구독/누산/send 데이터 흐름은 건드리지 않는다. ADR-0044/0045/0046.)
  const streaming = awaiting || (!turnDone && items.length > 0)

  // ADR-0145: 빈 상태 = 복원 완료 신호 + 0건 + 이 구독에서 아직 안 보냄. 0건만 보고 그리면 이력이 있는
  //   세션도 복원이 끝나기 전엔 0건이라 안내가 떴다가 대화로 바뀐다(깜빡임). hasSent 를 함께 보는 이유 =
  //   첫 전송 직후 items 가 채워지기 전 구간도 이미 "대화 시작"이라 빈 상태가 아니다.
  const showEmpty = replayDone && !hasSent && items.length === 0

  return (
    <div
      // 빈 상태에선 루트가 곧 정렬 컨테이너다(justify-center) — [마스코트·문구·입력창] 묶음을 통째로
      //   세로 가운데 놓는다. 위아래 spacer 로 나누면 위쪽에 든 마스코트·문구 높이만큼 묶음이 위로 밀린다.
      // relative = 아래 부재 오버레이(absolute inset-0)의 앵커. 안쪽 absolute 요소(입력창 위 이름 라벨)는
      //   각자 relative 부모를 갖고 있어 이 추가에 영향받지 않는다.
      className={`relative flex h-full w-full flex-col bg-background${showEmpty ? ' justify-center' : ''}`}
      data-rich-live="1" // cdp eval 에서 라이브 RichSlot 마운트 여부 확인용
      data-agent-id={agentId}
    >
      {/* 대화 렌더(스크롤) — ScrollArea seam(ADR-0053: 앱 전역 Radix 오버레이 스크롤바). 순서 보존 item 스트림.
          ★scrollRef 는 이 seam 이 실제 스크롤 노드(Radix Viewport)로 forward 한다 — 아래 하단 고정 auto-scroll
          이 그 Viewport 노드를 겨눠야 새 출력이 바닥에 붙는다(회귀 주의). CC 룩 렌더는 StructuredTextView 소관.
          (구 "JSON ● idle" 슬림 헤더는 제거 — 상태 힌트는 스트림 끝 대기 인디케이터(WaitRow "Wait" tail) 로 대체.) */}
      {!showEmpty && (
        <ScrollArea ref={scrollRef} className="min-h-0 flex-1">
          <StructuredTextView items={items} streaming={streaming} />
        </ScrollArea>
      )}

      {/* ADR-0145 빈 상태 윗단 — 마스코트 + "Claude Code". 세로 중앙 정렬은 루트가 잡고(justify-center)
          여기는 자기 높이만 차지한다. 문구는 제품명이라 번역 대상이 아니다(ADR-0145).
          ★공간이 줄 때 줄어드는 쪽은 마스코트뿐★: 마스코트 칸만 shrink+overflow-hidden 이라 슬롯이
          낮아지면 자연스럽게 잘려 사라지고, 문구·입력창은 flex-none 으로 온전히 남는다
          (임계 높이로 통째 숨기던 방식은 툭 사라져 부자연스럽다는 사용자 판단으로 폐기). */}
      {showEmpty && (
        <div data-rich-empty="1" className="flex min-h-0 flex-col items-center px-4 pb-8">
          <div className="min-h-0 shrink overflow-hidden">
            <ClaudeMascot />
          </div>
          <div className="mt-3 flex-none text-[20px] font-semibold tracking-tight text-foreground">
            Claude Code
          </div>
        </div>
      )}

      {/* 입력창 — Enter 전송 / Shift+Enter 줄바꿈(별도 전송 버튼 없음). ★포커스 가드★: stopPropagation
          으로 키 입력이 상위/전역 키바인딩으로 새지 않게 한다(터미널 슬롯의 onData 캡처와 동형 격리).
          ★textarea 는 두 배치에서 같은 엘리먼트다(ADR-0145)★: 자리(부모 children 인덱스)를 고정하고
          className·rows 만 갈아 React 가 remount 하지 않게 한다 — 갈라 두면 전송·IME·포커스 가드가
          두 벌이 되고, 전환 순간 입력 중이던 포커스가 끊긴다. */}
      <div
        className={
          showEmpty
            ? 'flex flex-none items-stretch justify-center px-4'
            : 'relative flex flex-none items-stretch border-t border-border px-2 py-1.5'
        }
      >
        {/* ★정체성 라벨(§ user request)★: claude-code 터미널처럼 입력창 바로 위(우측)에 작은 라벨을 오버랩
            (absolute -top — 줄을 차지하지 않음)해 어느 에이전트인지 이름만 표시(중복 이름 허용). pointer-events-none
            으로 입력·스크롤을 막지 않는다. 상태 글리프는 트리가 담당.
            빈 상태에서는 접는다 — 입력창 바로 위가 "Claude Code" 문구 자리라 겹친다(ADR-0145 §2 구성). */}
        {!showEmpty && (
          <div
            data-rich-label="1"
            title={headerName}
            className="pointer-events-none absolute -top-5 right-3 z-10 max-w-[70%] truncate rounded border border-border bg-surface px-1.5 py-0.5 text-[11px] text-muted"
          >
            {headerName}
          </div>
        )}
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
          // 우선순위 = 종료 > 빈 상태 > 하단. 종료 표시가 먼저다(어느 배치든 종료면 그 사실이 이긴다).
          placeholder={
            agentGone
              ? t('agent.terminatedPlaceholder')
              : showEmpty
                ? t('agent.emptyInputPlaceholder')
                : t('agent.inputPlaceholder')
          }
          disabled={agentGone}
          rows={showEmpty ? 3 : 2}
          // 빈 상태만 둥근 모서리에 조금 크게(ADR-0145 §5) — 하단 배치는 기존 고대비 바 그대로.
          // 하단 좌우 여백은 대화 본문(ChatRow px-4)과 들여쓰기를 크게 어긋내지 않는 선으로 잡고 세로
          // 여백은 그대로 둬 바 높이·성격을 유지한다. 빈 상태는 넓은 박스라 좌우를 더 주고 위쪽도 한 단계 띄운다.
          className={
            showEmpty
              ? 'w-full max-w-[560px] resize-none rounded-xl border border-border bg-surface px-5 py-4 text-[14px] text-foreground outline-none placeholder:text-muted focus:border-accent disabled:opacity-50'
              : 'flex-1 resize-none rounded border border-border bg-surface px-3 py-1.5 text-[13px] text-foreground outline-none placeholder:text-muted focus:border-accent disabled:opacity-50'
          }
        />
      </div>

      {/* ADR-0146: 타겟한 에이전트가 지금 없을 때(프로세스 종료 · 연결 끊김 — 둘을 같게 본다) 슬롯을
          죽인다. ★화면 내용은 지우지 않는다★ — 대화든 첫 실행 화면이든 그 시점 모습을 그대로 두고
          배경색 반투명 막으로 덮어 흐리게 만든 뒤 가운데에 심볼 하나만 얹는다(문구·배너 금지 — 정보를
          하드하게 남기지 않는다는 사용자 지시). 마지막 자식으로 두는 이유 = 앞 자식들의 인덱스를 밀지
          않아야 입력창이 remount 되지 않는다(위 textarea 주석). pointer-events-none 으로 아래 조작을
          막지 않는다(입력창 비활성은 기존 disabled 가 담당). */}
      {agentUnavailable && (
        <div
          data-rich-dead="1"
          aria-hidden
          className="pointer-events-none absolute inset-0 z-20 flex items-center justify-center bg-background/70"
        >
          <PowerOff className="size-10 text-muted" />
        </div>
      )}
    </div>
  )
}
