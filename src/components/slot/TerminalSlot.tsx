import { useRef, useEffect } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebglAddon } from '@xterm/addon-webgl'
import '@xterm/xterm/css/xterm.css'
import { agentClient } from '../../api/clientFactory'
import { FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputSubscription } from '../../api/agentClient'
import { useAgentStore } from '../../store/agentStore'

interface TerminalSlotProps {
  agentId: string | null
}

export default function TerminalSlot({ agentId }: TerminalSlotProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<Terminal | null>(null)
  const fitAddonRef = useRef<FitAddon | null>(null)
  // ResizeObserver 콜백에서 최신 agentId를 읽기 위한 ref
  const agentIdRef = useRef<string | null>(agentId)
  // onData 핸들러에서 terminated 상태 확인용 ref (§4-1: NotFound 스팸 방지)
  const isTerminatedRef = useRef(false)
  // resize debounce 타이머
  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    agentIdRef.current = agentId
  }, [agentId])

  const agents = useAgentStore(s => s.agents)
  const agent = agentId ? (agents.find(a => a.id === agentId) ?? null) : null
  // S9 §18-e: epoch이 바뀌면(재spawn) 재구독 트리거. status 변화만으론 effect가 안 돈다.
  const epoch = agent?.epoch ?? 0
  const isTerminated =
    agent != null &&
    (agent.status.type === 'Exited' ||
      agent.status.type === 'Killed' ||
      agent.status.type === 'Failed')

  // isTerminatedRef 동기화 — onData 클로저에서 최신 값 참조
  useEffect(() => { isTerminatedRef.current = isTerminated }, [isTerminated])

  // Terminal 인스턴스 초기화 (1회)
  useEffect(() => {
    if (!containerRef.current) return
    // WebGL/canvas 렌더러는 글리프를 canvas 2D(ctx.font)로 rasterize하는데 canvas 는 CSS var() 를
    // 해석 못 한다 → 'var(--font-terminal)' 을 그대로 넘기면 폰트 무효로 검은 화면. 생성 시점에
    // 실제 폰트 문자열로 해석해 넘긴다. (실측: canvas 가 '13px var(--font-terminal)' 거부→10px sans-serif)
    const fontFamily =
      getComputedStyle(document.documentElement).getPropertyValue('--font-terminal').trim() || 'monospace'
    const term = new Terminal({
      fontFamily,
      fontSize: 13,
      theme: { background: '#0a0a0a', foreground: '#e0e0e0', cursor: '#4a9eff' },
    })
    const fitAddon = new FitAddon()
    term.loadAddon(fitAddon)
    term.open(containerRef.current)
    // WebGL 렌더러 — DOM 렌더러는 customGlyphs 미지원이라 블록/박스드로잉 문자를 폰트에 위임,
    // 분수 DPI(rowHeight 비정수)에서 첫 행 상단 픽셀이 깎인다. WebGL은 이 글리프를 직접 그려
    // 클리핑 제거(조사: xterm.js #2409/#3807/#967). 미지원/컨텍스트 소실 시 DOM 자동 폴백.
    // open() 이후 로드 필수 — canvas 가 DOM에 붙은 뒤 WebGL 컨텍스트를 획득함.
    try {
      const webgl = new WebglAddon()
      webgl.onContextLoss(() => webgl.dispose())
      term.loadAddon(webgl)
    } catch (e) {
      // WebGL 미지원/로드 실패 → DOM 렌더러 폴백. 무로깅이면 클리핑 픽스가 조용히 무효화돼도 모르니 경고.
      console.warn('[TerminalSlot] WebGL 비활성 → DOM 렌더러로 폴백', e)
    }
    fitAddon.fit()
    terminalRef.current = term
    fitAddonRef.current = fitAddon

    const ro = new ResizeObserver(() => {
      fitAddon.fit()
      const aid = agentIdRef.current
      if (!aid) return
      // debounce 50ms — 드래그 중 매 프레임 IPC 발사 방지
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current)
      resizeTimerRef.current = setTimeout(() => {
        resizeTimerRef.current = null
        void agentClient.resizePty(aid, term.cols, term.rows)
      }, 50)
    })
    ro.observe(containerRef.current)

    return () => {
      ro.disconnect()
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current)
      term.dispose()
      terminalRef.current = null
      fitAddonRef.current = null
    }
  }, [])

  // PTY 출력 구독 (agentId 변경 시 재구독)
  useEffect(() => {
    const terminal = terminalRef.current
    if (!agentId || !terminal) return

    let sub: OutputSubscription | null = null
    let cancelled = false

    terminal.reset() // C2: 재구독 시 이전 출력 제거 (StrictMode 중복 방지)
    const lastSeq = { current: -1 } // T-2/G-2: seq dedup(컴포넌트 방어 — 클라도 내부 dedup)

    agentClient
      .subscribeOutput(agentId, chunk => {
        if (cancelled) return
        if (chunk.seq <= lastSeq.current) return // T-2: 순서 역전·중복 drop
        lastSeq.current = chunk.seq
        // ★tag 게이트(S15/ADR-0045)★: 이 슬롯은 터미널 raw 바이트(tag0)만 xterm 에 write 한다. tag1
        //   (StructuredEvent JSON)이 오면 무시한다 — RichSlot 이 tag0 을 무시하는 것과 정확히 대칭.
        //   구조화 에이전트에 터미널 슬롯이 붙거나(renderModeOverride·다중 구독) 배선 버그로 tag1 이
        //   공유 스트림(한 seq 공간)으로 새면, 게이트가 없을 때 JSON 바이트가 그대로 xterm 에 찍혀
        //   화면이 오염된다. seq 는 위에서 이미 전진시켰으므로(tag 무관 한 seq 공간) tag1 을 건너뛰어도
        //   dedup 은 정합하다.
        if (chunk.tag !== FRAME_TAG_TERMINAL_BYTES) return
        terminal.write(chunk.bytes) // 디코드는 클라 내부에서 끝남(transport 캡슐화)
      })
      .then(handle => {
        if (cancelled) {
          handle.unsubscribe()
          return
        }
        sub = handle
        // Task1(ADR-0036 carry-forward): 구독 직후 초기 크기 1회 전파. ResizeObserver 는 크기
        // *변화* 시에만 발화하므로, 슬롯이 처음부터 최종 크기면 한 번도 안 울려 PTY 가 spawn 시
        // 기본값(80×24)에 고착된다 → claude welcome 박스가 80칸 기준으로 그려져 좁은 슬롯에서 깨짐.
        // 그 빈 "초기 1회"를 여기서 채운다(gotty 패턴; client-first(ttyd)는 데몬이 View 를 모르는
        // ADR-0035 구조라 불가). 보내기 직전 fit() 으로 allotment 지연 레이아웃까지 반영한 최신
        // cols/rows 를 보장한다. resizePty 는 fire-and-forget(Resize 는 request_id 없음) — 직전
        // kill 등으로 실패해도 흡수. carrier 는 Phase B(TauriTransport)에서도 이 call-site 그대로.
        fitAddonRef.current?.fit()
        void agentClient.resizePty(agentId, terminal.cols, terminal.rows).catch(() => {})
      })
      // 구독 실패(예: 직전 kill로 NotFound)는 unhandled rejection 방지용으로 흡수.
      .catch(() => {})

    return () => {
      cancelled = true
      // unsubscribe 내부가 transport 정리(#13133 delete onmessage) + 백엔드 구독 해제까지 수행.
      sub?.unsubscribe()
    }
    // epoch 포함 — 재spawn(같은 agentId, epoch++) 시 reset → 재구독 → replay 재생 (S9 §18-e/f)
  }, [agentId, epoch])

  // 키 입력 → writeStdin (§4-1: terminated 후 입력 차단 + catch)
  useEffect(() => {
    const terminal = terminalRef.current
    if (!agentId || !terminal) return
    const disp = terminal.onData(data => {
      if (isTerminatedRef.current) return
      void agentClient.writeStdin(agentId, new TextEncoder().encode(data)).catch(() => {})
    })
    return () => disp.dispose()
  }, [agentId])

  return (
    <div style={{
      width: '100%',
      height: '100%',
      position: 'relative',
      boxSizing: 'border-box',
      padding: '4px 8px',        // 터미널 좌우 여백(wezterm 스타일). 여백만큼 inset → FitAddon이 그 크기로 cols/rows 계산.
      background: '#0a0a0a',     // 터미널 배경(Terminal theme)과 동일 → 여백이 seamless
    }}>
      <div ref={containerRef} style={{ width: '100%', height: '100%' }} />
      {isTerminated && (
        <div
          style={{
            position: 'absolute',
            inset: 0,
            background: 'rgba(0,0,0,0.6)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'var(--text-muted)',
            fontFamily: 'var(--font-ui)',
            fontSize: '13px',
            pointerEvents: 'none',
          }}
        >
          {agent!.status.type === 'Failed'
            ? `Failed: ${(agent!.status as { type: 'Failed'; message: string }).message}`
            : '종료됨'}
        </div>
      )}
    </div>
  )
}
