import { useRef, useEffect } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'

const DUMMY_LINES = [
  "Claude 비서 에이전트 v1.0\r\n",
  "\x1b[32m✓\x1b[0m 작업 완료: requirements.md 분석\r\n",
  "\x1b[33m⚠\x1b[0m 파일 경로: I:/Engram/core/dashboard/\r\n",
  "\x1b[31m✗\x1b[0m 오류: 파일을 찾을 수 없습니다\r\n",
  "\x1b[90m>\x1b[0m 대기 중...\r\n",
]

function getCssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
}

export default function TerminalSlot() {
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return

    const fitAddon = new FitAddon()
    const terminal = new Terminal({
      theme: {
        background: getCssVar('--bg') || '#0a0a0a',
        foreground: getCssVar('--text') || '#e0e0e0',
        cursor:     getCssVar('--accent') || '#4a9eff',
      },
      fontFamily: getCssVar('--font-terminal') || "'Cascadia Code', monospace",
      fontSize: 13,
      cursorBlink: true,
    })

    terminal.loadAddon(fitAddon)
    terminal.open(el)
    DUMMY_LINES.forEach(line => terminal.write(line))
    setTimeout(() => { try { fitAddon.fit() } catch {} }, 50)

    const ro = new ResizeObserver(() => { try { fitAddon.fit() } catch {} })
    ro.observe(el)

    return () => {
      ro.disconnect()
      terminal.dispose()
    }
  }, [])

  return <div ref={containerRef} style={{ flex: 1, minHeight: 0, width: '100%', height: '100%', overflow: 'hidden' }} />
}
