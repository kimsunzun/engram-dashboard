//! DomSlot — plain-text DOM render slot for debugging/observation (§5 LLM-first control).
//!
//! ★Role★: draws the same output stream as TerminalSlot as a plain `<pre>` instead of xterm — WebglAddon rasterizes terminal output to canvas, so `document.body.innerText`/CDP eval cannot read it. DomSlot strips ANSI from the same bytes so eval can.
//! ★The subscription discipline mirrors TerminalSlot as is★ (per-line comments give the reasons).
//! ★Scope★: read-only observer, no input handling (input goes via TerminalSlot/agentClient.writeStdin). Not a terminal emulator — control sequences are stripped best-effort for "plain text is readable".
//! ★backfill (ADR-0046)★: every mount/swap/remount gets the daemon ring in full via requestReplay through the per-view (slot) buffering→marker flush path.

import { useEffect, useState } from 'react'

import { agentClient } from '../../api/clientFactory'
import { FRAME_TAG_TERMINAL_BYTES } from '../../api/wsFrame'
import type { OutputSubscription, ViewPhase } from '../../api/agentClient'
import { useAgentStore } from '../../store/agentStore'
import { ScrollArea } from '../ui/scroll-area'
import { SlotUnavailableVeil } from './SlotUnavailableVeil'
import { useScrollFollow } from './scrollFollow/useScrollFollow'
import { JumpToBottom } from './scrollFollow/JumpToBottom'

interface DomSlotProps {
  /** Subscription key (ADR-0046) = slot id. Two slots with the same agentId also get independent subscriptions and independent progress (bug B resolved). */
  viewId: string
  agentId: string
}

// Cap on accumulated text (~200KB): observation needs only recent output, so on overflow the front is cut.
const MAX_TEXT_LEN = 200_000

// ANSI/control-sequence strip regex (best-effort, see the file header): CSI `ESC [ … final` · OSC `ESC ] … (BEL|ST)` · a 2-byte ESC sequence caught by neither. Erasing color etc. is enough for innerText observation.
// eslint-disable-next-line no-control-regex
const ANSI_RE = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]/g

function stripAnsi(s: string): string {
  return s.replace(ANSI_RE, '')
}

// Cap on pending (an incomplete ESC sequence) — stops an unterminated ESC piling up across chunks. Real sequences are far shorter; beyond it a lone ESC is let through (see splitTrailingEsc).
const MAX_PENDING_LEN = 256

// ★ANSI sequences straddling a chunk boundary (FIX-3, best-effort)★: strip runs per chunk, so a sequence split across two chunks ("\x1b[" | "31mred") would leak raw ESC[... onto the screen. If a chunk ends with an ESC not terminated within it, that tail is cut off as pending and prepended to the next chunk.
// Heuristic: find the last ESC; if no sequence-terminating byte (CSI final @-~, OSC BEL/ESC) follows it, hold it (CSI · OSC · 2-byte ESC approximated).
function splitTrailingEsc(s: string): [string, string] {
  const esc = s.lastIndexOf('\x1b')
  if (esc < 0) return [s, '']
  const tail = s.slice(esc)
  // Because of lastIndexOf there is only one ESC in the tail → an ANSI_RE match means "complete".
  ANSI_RE.lastIndex = 0
  const m = ANSI_RE.exec(tail)
  if (m && m.index === 0 && m[0].length === tail.length) return [s, '']
  // Holds the incomplete tail. But past the cap (lone ESCs piling up with no terminator) it does not hold and lets everything through (prevents unbounded growth).
  if (tail.length > MAX_PENDING_LEN) return [s, '']
  return [s.slice(0, esc), tail]
}

export default function DomSlot({ viewId, agentId }: DomSlotProps) {
  const [text, setText] = useState('')
  const follow = useScrollFollow(viewId)
  const [phase, setPhase] = useState<ViewPhase | null>(null)

  const agents = useAgentStore(s => s.agents)
  const agent = agents.find(a => a.id === agentId) ?? null
  // ADR-0148: did we receive the authoritative registry? — a guard so the empty registry before receipt (right after boot/reconnect) is not mistaken for "gone".
  const agentsLoaded = useAgentStore(s => s.agentsLoaded)
  // ★Absent = found in terminal state ∪ unresolved even after the registry is received (ADR-0148)★: the latter is what a kill really produces (the reaper removes it from the registry); without it a dead slot looks alive, and this curtain is the slot's only status surface.
  const agentGone =
    (agentsLoaded && agent == null) ||
    (agent != null &&
      (agent.status.type === 'Exited' ||
        agent.status.type === 'Killed' ||
        agent.status.type === 'Failed'))

  // ADR-0148 decision 1 / ADR-0149 decision 4: the absent decision is the same for all three slots (RichSlot has the canonical form) — ended ∪ connection
  //   lost ∪ the subscription cannot produce output (without 'error', a slot that exhausted the ladder becomes a blank panel with no marker).
  const [connected, setConnected] = useState(() => agentClient.connectionState === 'connected')
  useEffect(() => agentClient.onConnectionStateChange(s => setConnected(s === 'connected')), [])
  const subscriptionDown = phase === 'detached' || phase === 'error'
  const agentUnavailable = agentGone || !connected || subscriptionDown

  useEffect(() => {
    setText('') // C2: StrictMode duplicate prevention
    setPhase(null)
    follow.pin() // ADR-0242: the history that arrives after clearing lands at the bottom.
    // stream=true keeps multibyte UTF-8 straddling chunk boundaries; swapped on another incarnation (onReset) so the previous incarnation's incomplete bytes do not stick a broken char onto the new replay.
    let decoder = new TextDecoder()
    const lastSeq = { current: -1 } // T-2/G-2: seq dedup (component defense — the client also dedups internally)
    // FIX-3: carries an incomplete ANSI tail to the next chunk (same lifecycle as the text accumulator, reset on every resubscribe).
    let pending = ''

    let sub: OutputSubscription | null = null
    let cancelled = false

    agentClient
      .subscribeOutput(
        viewId,
        agentId,
        chunk => {
          if (cancelled) return
          if (chunk.seq <= lastSeq.current) return
          lastSeq.current = chunk.seq
          // ★tag gate (S15/ADR-0045)★: DOM mode draws raw terminal bytes (tag0) only; tag1 (StructuredEvent JSON) is ignored like TerminalSlot — otherwise its JSON would go through ANSI strip and pollute the <pre>. seq is already advanced above, so dedup stays consistent.
          if (chunk.tag !== FRAME_TAG_TERMINAL_BYTES) return
          // Prepend the previous incomplete ESC tail, then cut the new tail again (FIX-3).
          const decoded = pending + decoder.decode(chunk.bytes, { stream: true })
          const [head, tail] = splitTrailingEsc(decoded)
          pending = tail
          const piece = stripAnsi(head)
          setText(prev => {
            const next = prev + piece
            return next.length > MAX_TEXT_LEN ? next.slice(next.length - MAX_TEXT_LEN) : next
          })
        },
        state => {
          if (cancelled) return
          setPhase(state)
        },
        // Clearing duty and mandatory onReset pass-through as in TerminalSlot (an empty <pre> reads as "nothing happened"). Own addition: chunk-boundary state belongs to the previous incarnation too — a held incomplete multibyte puts a replacement char before the new replay, and an incomplete ESC tail would swallow its first bytes.
        () => {
          if (cancelled) return
          setText('')
          follow.pin() // ADR-0242: the new incarnation's history lands at the bottom.
          lastSeq.current = -1
          pending = ''
          decoder = new TextDecoder()
        },
      )
      .then(handle => {
        if (cancelled) {
          handle.unsubscribe()
          return
        }
        sub = handle
      })
      .catch(() => {})

    return () => {
      cancelled = true
      sub?.unsubscribe()
    }
    // ★The incarnation (epoch) is not in this trigger — clearing is onReset alone (as TerminalSlot).★
    // ★Renderer swap/remount is covered by resubscribe with a full requestReplay backfill (ADR-0046)★. viewId is the subscription key (two slots with the same agentId are independent). ★Do not put `follow` in★: its `pin` is stable for the whole mount (ADR-0242 · `useScrollFollow`).
  }, [viewId, agentId])

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative', boxSizing: 'border-box' }}>
      {/* Scroll surface = the shared ScrollArea seam (ADR-0053) — replaces the old raw <pre overflow:auto> with an overlay scrollbar.
          The ref is forwarded to the Viewport (the real scroll node), which becomes the node that scroll-follow (ADR-0242) measures and writes.
          data-dom-mode / data-agent-id: the markers for checking whether DOM mode is mounted and its target in cdp eval/tests are kept on the inner
          <pre> (the observed text node) (same convention as RichSlot — readable via textContent).
          No input handling — read-only observer (input goes via TerminalSlot/agentClient.writeStdin, see the file header). */}
      <ScrollArea
        ref={follow.viewportRef}
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
        <JumpToBottom follow={follow} />
      </ScrollArea>
      {agentUnavailable && <SlotUnavailableVeil phase={phase} />}
    </div>
  )
}
