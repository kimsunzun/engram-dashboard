// 터미널 모드 랩 뷰 — xterm + FitAddon, OSS(gotty 변형) resize 패턴 격리 검증용.
//
// 메인 TerminalSlot.tsx 와 달리 agentClient(데몬) 의존이 없다. 입력은 props 로만 받는다
// (output = mock ANSI bytes, onResize = PTY 전파 mock). 이렇게 격리해야 데몬/PTY 없이
// "fit → resize 전파" 패턴 자체를 브라우저에서 검증할 수 있다.
//
// ★검증 대상 패턴(OSS 기준)★: 마운트(=구독) 직후 초기 크기를 1회 전파한다. ResizeObserver
// 는 크기 *변화* 시에만 발화하므로(처음부터 최종 크기인 슬롯에선 안 울림), 이 초기 1회가
// 없으면 PTY 가 spawn 시 기본값(80×24)에 고착된다 — 이게 메인 Task 1 버그의 핵심.
// 근거: ttyd(client-first) / gotty(구독 후 첫 resize) 코드 대조.

import { useRef, useEffect } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'

interface TerminalViewProps {
  /** mock PTY 출력(ANSI 포함). 실제 메인에선 subscribeOutput 청크가 이 자리. */
  output: string
  /** fit 결과를 "PTY 로 전파"하는 콜백 mock. 메인에선 agentClient.resizePty. */
  onResize?: (cols: number, rows: number) => void
}

export function TerminalView({ output, onResize }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  // onResize 최신값을 effect 재실행 없이 쓰기 위한 ref(메인 TerminalSlot 패턴 동일).
  const onResizeRef = useRef(onResize)
  useEffect(() => {
    onResizeRef.current = onResize
  }, [onResize])

  useEffect(() => {
    if (!containerRef.current) return
    const term = new Terminal({
      fontFamily: 'ui-monospace, "Cascadia Code", Consolas, monospace',
      fontSize: 13,
      theme: { background: '#0a0a0a', foreground: '#e0e0e0', cursor: '#4a9eff' },
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(containerRef.current)
    fit.fit()

    // ★OSS 패턴 핵심★: 마운트 직후 초기 크기 1회 전파(gotty 변형). 이게 Task 1 픽스.
    onResizeRef.current?.(term.cols, term.rows)

    term.write(output)

    const ro = new ResizeObserver(() => {
      fit.fit()
      onResizeRef.current?.(term.cols, term.rows)
    })
    ro.observe(containerRef.current)

    return () => {
      ro.disconnect()
      term.dispose()
    }
  }, [output])

  return <div ref={containerRef} style={{ width: '100%', height: '100%' }} />
}
