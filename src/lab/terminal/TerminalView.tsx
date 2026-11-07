// Terminal mode lab view — xterm + FitAddon, for isolated verification of the OSS (gotty variant) resize pattern.
//
// Unlike the main TerminalSlot.tsx it has no dependency on agentClient (the daemon). Input comes only through props
// (output = mock ANSI bytes, onResize = PTY propagation mock). Isolating it this way lets us verify the
// "fit → resize propagation" pattern itself in a browser without a daemon/PTY.
//
// ★Pattern under verification (OSS baseline)★: propagate the initial size once right after mount (= subscribe). ResizeObserver
// fires only on size *change* (it does not fire in a slot that was at the final size from the start), so without this initial one-time call
// the PTY gets stuck at the spawn default (80×24) — that is the core of the main Task 1 bug.
// Basis: comparison of ttyd (client-first) / gotty (first resize after subscribe) code.

import { useRef, useEffect } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'

interface TerminalViewProps {
  /** In the real main, a subscribeOutput chunk goes in this place. */
  output: string
  /** In main it is agentClient.resizePty. */
  onResize?: (cols: number, rows: number) => void
}

export function TerminalView({ output, onResize }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  // A ref to use the latest onResize without re-running the effect (same as the main TerminalSlot pattern).
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

    // ★Core of the OSS pattern★: propagate the initial size once right after mount (gotty variant). This is the Task 1 fix.
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
