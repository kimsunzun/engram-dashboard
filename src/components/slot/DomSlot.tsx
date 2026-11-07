//! DomSlot — plain-text DOM render slot for debugging/observation (the observation tool of §5 LLM-first control).
//!
//! ★Role★: draws the *same output stream* as TerminalSlot as a plain `<pre>` instead of xterm (canvas glyphs).
//! Why it is needed: terminal-mode output has its glyphs rasterized to canvas by WebglAddon, so it cannot be read via
//! `document.body.innerText`/CDP eval → an LLM/automation has no way to observe or verify screen content.
//! DomSlot strips only ANSI from the same byte stream and attaches it as `<pre>` text so eval can read it.
//!
//! ★The subscription discipline mirrors TerminalSlot as is★ (see the per-line comments for the reasons — same shape as TerminalSlot)
//!
//! ★Scope★: a read-only observer. No input handling (input still goes via TerminalSlot/agentClient.writeStdin).
//! A full terminal emulator is not the goal — control sequences like cursor movement/screen clearing are stripped best-effort,
//! aiming only at "plain text is readable" (see the ANSI strip comment below).
//!
//! ★backfill (after ADR-0046)★: DOM mode also gets the daemon ring in full as backfill via requestReplay on subscribe —
//! swap/remount also re-replays everything through the per-view (slot) buffering→marker flush path, so output from before the swap is restored.
//! (The old limit "LIVE-forward only / no backfill on swap, ADR-0041" was resolved by ADR-0046's view-direct replay —
//! every mount is filled with a full requestReplay backfill.)

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

// Cap on accumulated text (about 200KB). Prevents unbounded growth — for observation, showing only recent output is enough, so only the tail is kept.
// (The goal is not full retention like terminal scrollback. On overflow the front is cut to keep the most recent ~200KB.)
const MAX_TEXT_LEN = 200_000

// ANSI/control-sequence strip regex (best-effort — not a full terminal emulator, see the file header).
//   - ESC [ ... <final>  = CSI sequence (color, cursor movement, screen clearing etc.). Swallows parameter/intermediate bytes and ends at final.
//   - ESC ] ... (BEL|ST) = OSC sequence (window title etc.). Terminated by BEL(\x07) or ST(ESC \).
//   - ESC <single>       = a 2-byte ESC sequence caught by neither of the above.
// The goal is "plain-text readability", not pixel reproduction — erasing color alone is enough for innerText observation.
// eslint-disable-next-line no-control-regex
const ANSI_RE = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]/g

function stripAnsi(s: string): string {
  return s.replace(ANSI_RE, '')
}

// Cap on pending (an incomplete ESC sequence) — stops an ESC with no terminator from piling up endlessly across chunks.
// CSI/OSC sequences are normally far shorter than this (a few bytes for color/cursor, tens of bytes for an OSC window title). Beyond it
// it is treated as "not a real sequence but a lone ESC" and simply let through (best-effort — see splitTrailingEsc below).
const MAX_PENDING_LEN = 256

// ★Keeping ANSI sequences that straddle a chunk boundary (FIX-3, best-effort)★: ANSI strip runs per chunk, but if a sequence is
// split across two chunks (e.g. the earlier chunk ends with "\x1b[" and the later one starts with "31mred") the regex cannot erase the
// half, and the raw ESC[... leaks onto the screen. So if the end of a chunk has an ESC *not terminated within this chunk*, everything from
// that ESC to the end is cut off and handed over as pending, to be prepended to the next chunk.
// The decision is a simple heuristic — find the last ESC (\x1b) position, and if no 'sequence-terminating byte' follows it,
// treat it as incomplete (not a full emulator, see the file header). An approximation covering CSI (ESC[…final @-~), OSC (ESC]…BEL/ST) and 2-byte
// ESC: if neither a CSI final (@-~) nor an OSC terminator (BEL/ESC) has appeared after the ESC yet, hold.
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
  // Held as React state and re-rendered — for observation, a declarative render instead of imperative write like xterm.
  const [text, setText] = useState('')
  const follow = useScrollFollow(viewId)
  // The phase the subscription last announced — the basis of the badge below (same shape as TerminalSlot).
  const [phase, setPhase] = useState<ViewPhase | null>(null)

  const agents = useAgentStore(s => s.agents)
  const agent = agents.find(a => a.id === agentId) ?? null
  // ADR-0148: did we receive the authoritative registry? — a guard so the empty registry before receipt (right after boot/reconnect) is not mistaken for "gone".
  const agentsLoaded = useAgentStore(s => s.agentsLoaded)
  // ★Absent = found in terminal state ∪ unresolved even after the registry is received (ADR-0148)★. The latter is the real outcome of a kill
  //   (the reaper removes it from the registry), so without it a dead slot looks alive — this slot is for observation, but
  //   one curtain is its only status surface, so if that vanishes no signal is left at all.
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
    setPhase(null) // The phase of a new subscription is set again by that subscription's notification.
    follow.pin() // ADR-0242: the history that arrives after clearing lands at the bottom.
    // stream=true keeps multibyte UTF-8 that straddles chunk boundaries. Swapped out when another incarnation attaches (onReset below) —
    //   holding the previous incarnation's incomplete bytes would stick a broken character onto the first character of the new replay.
    let decoder = new TextDecoder()
    const lastSeq = { current: -1 } // T-2/G-2: seq dedup (component defense — the client also dedups internally)
    // FIX-3: buffer that passes an incomplete ANSI sequence straddling a chunk boundary on to the next chunk (same lifecycle as the text accumulator —
    // reset here on every resubscribe). splitTrailingEsc fills it, and it is prepended to the next chunk.
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
          // ★tag gate (S15/ADR-0045)★: DOM mode is an observer that draws raw terminal bytes (tag0) as plain text.
          //   When tag1 (StructuredEvent JSON) arrives it is ignored — same shape as TerminalSlot (the same tag0 consumer). Without the gate,
          //   tag1 JSON bytes would go through ANSI strip and leak into the <pre> as is, polluting the observed text.
          //   seq was already advanced above, so skipping tag1 keeps dedup consistent (one seq space regardless of tag).
          if (chunk.tag !== FRAME_TAG_TERMINAL_BYTES) return
          // The incomplete ESC tail (pending) left by the previous chunk is prepended to this decode, then the new incomplete tail is
          // cut off again — that way a sequence split across two chunks is rejoined intact and stripped (FIX-3).
          const decoded = pending + decoder.decode(chunk.bytes, { stream: true })
          const [head, tail] = splitTrailingEsc(decoded)
          pending = tail
          const piece = stripAnsi(head)
          setText(prev => {
            const next = prev + piece
            // When over the cap, the front is cut to keep only the latest MAX_TEXT_LEN (prevents unbounded growth — see the constant's comment above).
            return next.length > MAX_TEXT_LEN ? next.slice(next.length - MAX_TEXT_LEN) : next
          })
        },
        state => {
          if (cancelled) return
          setPhase(state)
        },
        // The reasons for the clearing duty and the mandatory onReset pass-through are the same as TerminalSlot (for an observer, an empty <pre> reads as "nothing
        //   happened"). This slot's own addition = chunk-boundary state also belongs to the previous incarnation, so it is dropped with it —
        //   keeping a decoder that holds an incomplete multibyte puts a replacement character before the first char of the new replay, and if an incomplete ESC
        //   tail remains, the first bytes of the new replay are mistaken for a sequence and swallowed whole by strip.
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
      // A subscription failure (e.g. NotFound from a kill just before) is absorbed to prevent an unhandled rejection (same shape as TerminalSlot).
      .catch(() => {})

    return () => {
      cancelled = true
      sub?.unsubscribe()
    }
    // ★The incarnation (epoch) is not put in this trigger — the reason is the same as TerminalSlot (clearing is onReset alone).★
    // ★Renderer swap/remount is also covered by resubscribe with a full requestReplay backfill (ADR-0046)★ — output from before the swap is
    //   restored through view buffering→marker flush too (the old "LIVE-forward only" limit was resolved by ADR-0046, see the file header).
    // viewId included — it is the subscription key (ADR-0046, two slots with the same agentId are independent).
    // ★Do not put `follow` in★ — its `pin` is the same function for the whole mount lifetime (ADR-0242 · the head of `useScrollFollow`).
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
