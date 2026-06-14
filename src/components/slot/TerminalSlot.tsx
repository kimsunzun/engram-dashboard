import { useRef, useEffect } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { agentClient } from '../../api/clientFactory'
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
    const term = new Terminal({
      fontFamily: 'var(--font-terminal)',
      fontSize: 13,
      theme: { background: '#0a0a0a', foreground: '#e0e0e0', cursor: '#4a9eff' },
    })
    const fitAddon = new FitAddon()
    term.loadAddon(fitAddon)
    term.open(containerRef.current)
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
        terminal.write(chunk.bytes) // 디코드는 클라 내부에서 끝남(transport 캡슐화)
      })
      .then(handle => {
        if (cancelled) {
          handle.unsubscribe()
          return
        }
        sub = handle
      })

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
    <div style={{ width: '100%', height: '100%', position: 'relative' }}>
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
