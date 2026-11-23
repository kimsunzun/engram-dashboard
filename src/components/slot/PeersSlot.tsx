//! PeersSlot — the PC list slot: the requests waiting for a human (agents' trust changes and the questions Engram raises), this PC's ID
//!   and direct addresses, the paired PCs with their link state, PCs that knocked, the network settings and the regenerate-key button.
//!   ★It only shows the daemon's `peer.list` and calls `peer.*`★ — every behaviour is the daemon's.
//!
//! ★Actions run only on the human path★: each button calls a `humanOnly` command of `commands/peerCommands.ts` through `runAsHuman`
//!   (never `run`, never the bus). While this slot is open in any tab of any window, on screen or not, the daemon routes its approval
//!   questions here instead of the OS dialog (ADR-0410; the shell reports it — `daemon_client::peer_watch`), so a request card shows
//!   what the dialog shows: the daemon's own question text in the UI language (heading and lines — every value, the grouped IDs, what
//!   approving allows).
//! ★Mis-aimed clicks★ — nothing that says yes acts under a pointer that something else moved there:
//!   - the requests sit first, in a section that is always there; an answered card keeps its place for [`ANSWERED_MS`] as a placeholder
//!     with the outcome, so the next card does not slide under the pointer that just answered;
//!   - the buttons that say yes and the two-step buttons are held for [`HOLD_MS`] after anything that can move them: the cards shown,
//!     the UI language; the size of the slot, a section, a card or the scroll content (a row added or removed anywhere — the state
//!     row, a notice); a scroll of the slot; the window resized, coming to the front (`focus` — the click that activates a window
//!     raised by an agent) or becoming visible (`visibilitychange`); and this slot's own place in the layout (its rectangle in the
//!     shell's snapshot — a split ratio pair can move it without resizing it). The look follows React's render; ★the press itself is
//!     checked against [`MoveClock`] at click time★, which those changes stamp synchronously (the observer callback, the events, the
//!     render that sees a new key — a layout change renders this slot in the same commit that moves its leaf), so a click that lands
//!     before React has re-rendered the hold is refused too. The answer buttons have one width in every language;
//!   - Start fresh, regenerate and remove need a second click, not taken within [`HOLD_MS`] of the first.
//! ★Text from outside is data★: PC names, agent names, IDs, the daemon's lines and errors are React text nodes only (no HTML), each in a
//!   `<bdi>` so a right-to-left or control character cannot reorder the sentence around it.
//! ★Notices are kept as data and translated at render★ (ADR-0373) — a language switch re-words them.
//! ★Style = variables only (theme-compliant)★ like the other slot contents.

// ADR-0394

import { createContext, useContext, useEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from 'react'

import type { PeerState } from '../../../crates/engram-dashboard-protocol/bindings/PeerState'
import type { PeerStatePc } from '../../../crates/engram-dashboard-protocol/bindings/PeerStatePc'
import type { PeerStateRequest } from '../../../crates/engram-dashboard-protocol/bindings/PeerStateRequest'
import { runAsHuman } from '../../commands/registry'
import { t } from '../../i18n'
import type { StringKey } from '../../i18n'
import { useLanguage } from '../../i18n/react'
import { usePeerStore } from '../../store/peerStore'
import { useViewStore } from '../../store/viewStore'
import { ScrollArea } from '../ui/scroll-area'

/** How long the buttons that say yes stay held after the request cards change — the dialog holds its acting buttons as long. */
export const HOLD_MS = 1000
/** How long an armed two-step button waits for its second click. */
export const CONFIRM_MS = 4000
/** How long an answered request keeps its place (a placeholder with the outcome). */
export const ANSWERED_MS = 3000
/** One width for a request card's answer buttons in every language — sized for the longer one; the text wraps inside. */
const ANSWER_WIDTH = '11em'

/** When something last moved the buttons that say yes (`performance.now()`) — read at click time (module head). */
export type MoveClock = { last: number }
/** Outside a slot nothing stamps it, so nothing is refused. */
const MoveClockContext = createContext<MoveClock>({ last: -Infinity })

/** States in which the daemon refuses a new key — the button is not offered then. */
const NO_REGENERATE = new Set(['key_transient', 'adopt_or_fresh', 'unavailable', 'not_started'])

/** The full ID in space-separated groups of four, for reading — the daemon's own grouping (`PeerId::grouped`). */
export function groupId(id: string): string {
  return id.match(/.{1,4}/g)?.join(' ') ?? id
}

/** Outside text, isolated from the sentence around it. */
function Outside({ children, mono }: { children: string; mono?: boolean }) {
  return (
    <bdi data-outside="" style={{ unicodeBidi: 'isolate', ...(mono ? { fontFamily: 'var(--font-mono, monospace)' } : {}) }}>
      {children}
    </bdi>
  )
}

/** A translated template whose `{word}` values stay isolated text nodes (a value is never parsed as markup or as a placeholder). */
function Sentence({ text, values }: { text: string; values: Record<string, string> }) {
  const parts = text.split(/(\{\w+\})/g)
  return (
    <>
      {parts.map((part, i) => {
        const m = /^\{(\w+)\}$/.exec(part)
        if (m && m[1] in values) return <Outside key={i}>{values[m[1]]}</Outside>
        return <span key={i}>{part}</span>
      })}
    </>
  )
}

const ONLINE_KEY = {
  online: 'peers.onlineOnline',
  offline: 'peers.onlineOffline',
  refused: 'peers.onlineRefused',
  version_mismatch: 'peers.onlineVersionMismatch',
  duplicate_identity: 'peers.onlineDuplicateIdentity',
  endpoint_error: 'peers.onlineEndpointError',
} as const satisfies Record<string, StringKey>

const KIND_KEY = {
  add: 'peers.kindAdd',
  setNetwork: 'peers.kindSetNetwork',
  regenerateKey: 'peers.kindRegenerateKey',
  rename: 'peers.kindRename',
  acceptList: 'peers.kindAcceptList',
  adopt: 'peers.kindAdopt',
} as const satisfies Record<string, StringKey>

const STATE_KEY = {
  not_started: 'peers.stateNotStarted',
  no_identity: 'peers.stateNoIdentity',
  key_unreadable: 'peers.stateKeyUnreadable',
  key_transient: 'peers.stateKeyTransient',
  adopt_or_fresh: 'peers.stateAdoptOrFresh',
  identity_in_use: 'peers.stateIdentityInUse',
  list_refused: 'peers.stateListRefused',
  list_changed_outside: 'peers.stateListChangedOutside',
  list_corrupt: 'peers.stateListCorrupt',
  unavailable: 'peers.stateUnavailable',
} as const satisfies Record<string, StringKey>

/** The table's key for an outside word, or `undefined` for a word this build does not know. */
function lookup<T extends Record<string, StringKey>>(table: T, word: string): T[keyof T] | undefined {
  return Object.prototype.hasOwnProperty.call(table, word) ? table[word as keyof T] : undefined
}

function OnlineWord({ word }: { word: string }) {
  const key = lookup(ONLINE_KEY, word)
  return key ? <>{t(key)}</> : <Sentence text={t('peers.onlineUnknown', { word: '{word}' })} values={{ word }} />
}

function KindWord({ word }: { word: string }) {
  const key = lookup(KIND_KEY, word)
  return key ? <>{t(key)}</> : <Sentence text={t('peers.kindUnknown', { word: '{word}' })} values={{ word }} />
}

function StateWord({ word }: { word: string }) {
  const key = lookup(STATE_KEY, word)
  return key ? <>{t(key)}</> : <Sentence text={t('peers.stateOther', { word: '{word}' })} values={{ word }} />
}

// ── notices: data, translated at render ──────────────────────────────────────

type Notice =
  | { kind: 'done' }
  | { kind: 'moved'; fields: string }
  | { kind: 'regenerated'; id: string }
  | { kind: 'removed'; name: string }
  | { kind: 'removedNotSaved'; name: string }
  | { kind: 'failed'; message: string }

/** Where a notice shows — next to what was acted on. */
type Scope = 'requests' | 'my' | 'pcs' | 'knocks' | 'network'

function NoticeLine({ notice }: { notice: Notice | undefined }) {
  if (!notice) return null
  let body: ReactNode
  switch (notice.kind) {
    case 'done':
      body = t('peers.done')
      break
    case 'moved':
      body = <Sentence text={t('peers.networkMoved', { fields: '{fields}' })} values={{ fields: notice.fields }} />
      break
    case 'regenerated':
      body = <Sentence text={t('peers.regenerated', { id: '{id}' })} values={{ id: groupId(notice.id) }} />
      break
    case 'removed':
      body = <Sentence text={t('peers.removed', { name: '{name}' })} values={{ name: notice.name }} />
      break
    case 'removedNotSaved':
      body = <Sentence text={t('peers.removedNotSaved', { name: '{name}' })} values={{ name: notice.name }} />
      break
    case 'failed':
      body = <Sentence text={t('peers.failed', { message: '{message}' })} values={{ message: notice.message }} />
      break
  }
  return (
    <div role="status" style={{ ...muted, marginTop: 4 }}>
      {body}
    </div>
  )
}

/** What one command's answer means for the human — read per command, never by a field that several answers share. */
function noticeOf(command: string, answer: unknown): Notice {
  const a = (answer ?? {}) as { id?: unknown; moved?: unknown; saved?: unknown; name?: unknown }
  switch (command) {
    case 'peers.remove': {
      const name = typeof a.name === 'string' ? a.name : ''
      return a.saved === false ? { kind: 'removedNotSaved', name } : { kind: 'removed', name }
    }
    case 'peers.regenerateKey':
      return typeof a.id === 'string' ? { kind: 'regenerated', id: a.id } : { kind: 'done' }
    case 'peers.approve':
      if (Array.isArray(a.moved) && a.moved.length > 0) return { kind: 'moved', fields: a.moved.join(', ') }
      // An approved new key or a fresh identity answers the new ID; no other approve carries one.
      if (typeof a.id === 'string') return { kind: 'regenerated', id: a.id }
      return { kind: 'done' }
    default:
      return { kind: 'done' }
  }
}

// ── style ────────────────────────────────────────────────────────────────────

const section: CSSProperties = { padding: '8px 10px', borderBottom: '1px solid var(--border)' }
const heading: CSSProperties = { fontWeight: 600, marginBottom: 4 }
const muted: CSSProperties = { color: 'var(--text-muted)' }
const row: CSSProperties = { display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 6, padding: '2px 0' }
const input: CSSProperties = {
  background: 'var(--bg)',
  color: 'var(--text)',
  border: '1px solid var(--border)',
  borderRadius: 4,
  padding: '1px 6px',
  fontFamily: 'var(--font-ui)',
  fontSize: 12,
  minWidth: 0,
}

/**
 * A slot button. ★`aria-disabled`, not `disabled`★ — a focused button that becomes unavailable keeps the focus (`disabled` drops it to
 * body); the press is blocked here. The unavailable look is shape and opacity, no color of its own, so it reads in every theme.
 * `fixed` = a request card's answer button: one width in every language (text wraps inside), so a language switch does not move the
 * next button under the pointer — never wider than its slot. `guarded` = a button that says yes: a press within [`HOLD_MS`] of the
 * last move ([`MoveClock`]) is refused, whatever the look says yet.
 */
function Btn({
  onClick,
  children,
  off,
  fixed,
  guarded,
  testId,
}: {
  onClick: () => void
  children: ReactNode
  off?: boolean
  fixed?: boolean
  guarded?: boolean
  testId?: string
}) {
  const clock = useContext(MoveClockContext)
  return (
    <button
      type="button"
      aria-disabled={off || undefined}
      data-testid={testId}
      onClick={() => {
        if (off) return
        if (guarded && performance.now() - clock.last < HOLD_MS) return
        onClick()
      }}
      style={{
        background: 'var(--bg-secondary)',
        color: 'var(--text)',
        border: '1px solid var(--border)',
        borderStyle: off ? 'dashed' : 'solid',
        borderRadius: 4,
        padding: '1px 8px',
        fontFamily: 'var(--font-ui)',
        fontSize: 12,
        cursor: off ? 'not-allowed' : 'pointer',
        opacity: off ? 0.45 : undefined,
        ...(fixed ? { width: ANSWER_WIDTH, maxWidth: '100%', minHeight: '3.2em', whiteSpace: 'normal' as const } : {}),
      }}
    >
      {children}
    </button>
  )
}

/**
 * A button that needs a second click: the first arms it (it says so and shows `warning`), a second one within [`CONFIRM_MS`] acts —
 * ★but not one within [`HOLD_MS`] of the arming★, so a double-click does not pass. Either press is guarded like a yes-button.
 */
function ConfirmBtn({
  onConfirm,
  children,
  warning,
  off,
  fixed,
  testId,
}: {
  onConfirm: () => void
  children: ReactNode
  warning: ReactNode
  off?: boolean
  fixed?: boolean
  testId?: string
}) {
  const [armedAt, setArmedAt] = useState<number | null>(null)
  useEffect(() => {
    if (armedAt === null) return
    const timer = setTimeout(() => setArmedAt(null), CONFIRM_MS)
    return () => clearTimeout(timer)
  }, [armedAt])
  return (
    <>
      <Btn
        testId={testId}
        off={off}
        fixed={fixed}
        guarded
        onClick={() => {
          if (armedAt === null) {
            setArmedAt(Date.now())
          } else if (Date.now() - armedAt >= HOLD_MS) {
            setArmedAt(null)
            onConfirm()
          }
        }}
      >
        {armedAt !== null ? t('peers.confirmAgain') : children}
      </Btn>
      {armedAt !== null && (
        <div role="alert" style={{ flexBasis: '100%' }}>
          {warning}
        </div>
      )}
    </>
  )
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <Btn
      onClick={() => {
        void navigator.clipboard?.writeText(text).then(
          () => setCopied(true),
          () => setCopied(false),
        )
      }}
    >
      {copied ? t('peers.copied') : t('peers.copy')}
    </Btn>
  )
}

/** What a slot command came to. */
type Done = { ok: boolean; notice: Notice }

/** Runs one human-only slot command; its notice shows in `scope` (`null` = the caller shows it). */
type Act = (scope: Scope | null, id: string, args?: Record<string, unknown>) => Promise<Done>

/**
 * `true` while the buttons that say yes (and the two-step buttons) must not act: from the render in which `key` changes until
 * [`HOLD_MS`] after it. ★Derived during render★ — the very commit that shows changed cards already shows them held. ★Counted by
 * change, not by value★: every change of `key` is a new revision and the timer settles only the revision it was started for, so a
 * return to an already settled key (A → B → A) is held again. The render that sees a change also stamps `clock`.
 */
export function useHold(key: string, clock?: MoveClock): boolean {
  const seen = useRef<{ key: string | null; revision: number }>({ key: null, revision: 0 })
  if (seen.current.key !== key) {
    seen.current = { key, revision: seen.current.revision + 1 }
    if (clock) clock.last = performance.now()
  }
  const revision = seen.current.revision
  const [settled, setSettled] = useState(0)
  useEffect(() => {
    const timer = setTimeout(() => setSettled(revision), HOLD_MS)
    return () => clearTimeout(timer)
  }, [revision])
  return settled !== revision
}

/**
 * A counter that moves whenever the buttons may have moved without the cards changing: a box marked `data-peers-geom` inside `root`
 * (the slot, its sections, the request cards) or the scroll content changes size — a resized split, a card re-worded, an armed warning
 * row, a row added or removed — the scroll `viewport` scrolls, or the window is resized, comes to the front or becomes visible. Part
 * of the hold key; each move also stamps `clock` at once, before React renders. Without `ResizeObserver` only the events count.
 */
function useGeometry(root: RefObject<HTMLElement | null>, viewport: RefObject<HTMLElement | null>, clock: MoveClock): number {
  const [moves, setMoves] = useState(0)
  // One observer and the boxes it watches, per setup: a Strict Mode re-run disconnects the first observer, so the second must watch
  //   every box again — a set that outlived its observer would skip them.
  const watch = useRef<{ observer: ResizeObserver; watched: WeakSet<Element> } | null>(null)
  useEffect(() => {
    const moved = () => {
      clock.last = performance.now()
      setMoves(n => n + 1)
    }
    const scroller = viewport.current
    scroller?.addEventListener('scroll', moved, { passive: true })
    window.addEventListener('resize', moved)
    window.addEventListener('focus', moved)
    document.addEventListener('visibilitychange', moved)
    if (typeof ResizeObserver !== 'undefined') {
      const sizes = new WeakMap<Element, string>()
      const observer = new ResizeObserver(entries => {
        let changed = false
        for (const entry of entries) {
          const size = `${entry.contentRect.width}x${entry.contentRect.height}`
          if (sizes.get(entry.target) !== size) {
            changed = changed || sizes.has(entry.target)
            sizes.set(entry.target, size)
          }
        }
        if (changed) moved()
      })
      watch.current = { observer, watched: new WeakSet() }
    }
    return () => {
      scroller?.removeEventListener('scroll', moved)
      window.removeEventListener('resize', moved)
      window.removeEventListener('focus', moved)
      document.removeEventListener('visibilitychange', moved)
      watch.current?.observer.disconnect()
      watch.current = null
    }
  }, [root, viewport, clock])
  // Watch new boxes after every render (cards come and go); a box is watched once per observer. The scroll content (the viewport's
  //   child) grows or shrinks on any row added or removed, which moves everything below it without resizing those boxes.
  useEffect(() => {
    const current = watch.current
    const el = root.current
    if (!current || !el) return
    const content = viewport.current?.firstElementChild
    for (const box of [el, ...(content ? [content] : []), ...Array.from(el.querySelectorAll('[data-peers-geom]'))]) {
      if (!current.watched.has(box)) {
        current.watched.add(box)
        current.observer.observe(box)
      }
    }
  })
  return moves
}

/** `viewId` · `slotId` = where this slot sits (`LayoutLeaf`), to read its own rectangle; without them only the rest of the key holds. */
export default function PeersSlot({ viewId = null, slotId = null }: { viewId?: string | null; slotId?: string | null }) {
  // Re-render on a language switch — every text here is made at render (ADR-0373).
  const lang = useLanguage()
  const state = usePeerStore(s => s.state)
  const readFailed = usePeerStore(s => s.readFailed)
  const [busy, setBusy] = useState(false)
  const [notices, setNotices] = useState<Partial<Record<Scope, Notice>>>({})
  const [ghosts, setGhosts] = useState<Ghost[]>([])
  const root = useRef<HTMLDivElement>(null)
  const viewport = useRef<HTMLDivElement>(null)
  const clock = useRef<MoveClock>({ last: -Infinity }).current
  const geometry = useGeometry(root, viewport, clock)

  // The slot's own read: makes this PC's ID the first time (ADR-0389 d3) and fills a window that missed the broadcasts.
  useEffect(() => {
    void usePeerStore.getState().read()
  }, [])

  // Anything that can move a button under a resting pointer is in the key: the cards shown (incl. answered placeholders, in order),
  //   the language (button and line widths), the boxes' sizes and the window events (`geometry`) and this slot's place.
  const shown = state ? merged(state.requests, ghosts).map(item => ('ghost' in item ? `~${item.ghost.id}` : item.request.request)) : []
  // This slot's own rectangle in the shell's snapshot (ADR-0227 flat leaves): a compensating split ratio pair moves it without
  //   resizing it, so no box observer sees it. A string, so the store wakes this slot only when the place changes.
  const place = useViewStore(s => {
    const r = viewId !== null && slotId !== null ? s.layouts[viewId]?.slotRects.find(x => x.slot_id === slotId) : undefined
    return r ? `${r.x0},${r.y0},${r.x1},${r.y1}` : ''
  })
  const held = useHold(`${lang}|${geometry}|${place}|${shown.join(',')}`, clock)

  const act: Act = async (scope, id, args) => {
    setBusy(true)
    if (scope) setNotices(n => ({ ...n, [scope]: undefined }))
    let done: Done
    try {
      done = { ok: true, notice: noticeOf(id, await runAsHuman(id, args)) }
    } catch (e) {
      done = { ok: false, notice: { kind: 'failed', message: e instanceof Error ? e.message : String(e) } }
    } finally {
      setBusy(false)
    }
    if (scope) setNotices(n => ({ ...n, [scope]: done.notice }))
    return done
  }

  return (
    <MoveClockContext.Provider value={clock}>
      <div
        ref={root}
        data-peers-slot=""
        style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          flexDirection: 'column',
          fontFamily: 'var(--font-ui)',
          fontSize: 12,
          color: 'var(--text)',
          // An unbroken run (a long name, an ID) wraps instead of widening the scroll content past the slot's right edge.
          overflowWrap: 'anywhere',
        }}
      >
        <ScrollArea ref={viewport} style={{ flex: 1, minHeight: 0 }}>
          {state === null ? (
            <div style={{ ...section, ...muted }}>{readFailed ? t('peers.readFailed') : t('peers.loading')}</div>
          ) : (
            <>
              {/* First and always present, so the first request does not push other rows under the pointer. */}
              <RequestsSection
                requests={state.requests}
                ghosts={ghosts}
                setGhosts={setGhosts}
                held={held}
                busy={busy}
                act={act}
                notice={notices.requests}
                setNotice={notice => setNotices(n => ({ ...n, requests: notice }))}
              />
              {state.state !== 'ready' && (
                <div role="alert" style={section} data-peers-geom="">
                  <StateWord word={state.state} />
                </div>
              )}
              <MySection state={state} held={held} busy={busy} act={act} notice={notices.my} />
              <PcsSection pcs={state.pcs} held={held} busy={busy} act={act} notice={notices.pcs} />
              {state.knocks.length > 0 && <KnocksSection state={state} busy={busy} act={act} notice={notices.knocks} />}
              {/* Keyed by the settings, so a change from elsewhere resets the draft fields. */}
              {state.network !== null && (
                <NetworkSection key={JSON.stringify(state.network)} state={state} busy={busy} act={act} notice={notices.network} />
              )}
            </>
          )}
        </ScrollArea>
      </div>
    </MoveClockContext.Provider>
  )
}

function MySection({
  state,
  held,
  busy,
  act,
  notice,
}: {
  state: PeerState
  held: boolean
  busy: boolean
  act: Act
  notice: Notice | undefined
}) {
  return (
    <div style={section} data-peers-geom="">
      <div style={heading}>{t('peers.myId')}</div>
      {state.id !== null ? (
        <div style={row}>
          <span data-testid="peers-my-id">
            <Outside mono>{groupId(state.id)}</Outside>
          </span>
          <CopyButton text={state.id} />
        </div>
      ) : (
        <div style={muted}>{t('peers.stateNoIdentity')}</div>
      )}
      <div style={muted}>{t('peers.myIdHint')}</div>
      <div style={{ ...heading, marginTop: 6 }}>{t('peers.myAddresses')}</div>
      {state.addresses.length === 0 ? (
        <div style={muted}>{t('peers.noAddresses')}</div>
      ) : (
        state.addresses.map(address => (
          <div key={address} style={row}>
            <Outside mono>{address}</Outside>
            <CopyButton text={address} />
          </div>
        ))
      )}
      {state.endpoint === 'error' && state.endpoint_error !== null && (
        <div role="alert">
          <Sentence text={t('peers.endpointError', { error: '{error}' })} values={{ error: state.endpoint_error }} />
        </div>
      )}
      {!NO_REGENERATE.has(state.state) && (
        <div style={{ ...row, marginTop: 6 }}>
          <ConfirmBtn
            testId="peers-regenerate"
            off={busy || held}
            warning={t('peers.regenerateConfirm')}
            onConfirm={() => void act('my', 'peers.regenerateKey')}
          >
            {t('peers.regenerate')}
          </ConfirmBtn>
        </div>
      )}
      <NoticeLine notice={notice} />
    </div>
  )
}

/** An answered request's place, kept for [`ANSWERED_MS`] so the next card does not slide under the pointer that just answered. */
type Ghost = {
  id: string
  index: number
  height: number
  verdict: 'approved' | 'denied'
  /** What was answered, kept as the request's data and worded at render: its translated heading, else its kind and summary. */
  heading: string
  kind: string
  summary: string
  notice: Notice
}

type Shown = { request: PeerStateRequest } | { ghost: Ghost }

/** The cards in order: an answered request shows as its placeholder, and a placeholder whose request is gone keeps its place. */
function merged(requests: PeerStateRequest[], ghosts: Ghost[]): Shown[] {
  const byId = new Map(ghosts.map(g => [g.id, g]))
  const out: Shown[] = requests.map(r => {
    const ghost = byId.get(r.request)
    return ghost ? { ghost } : { request: r }
  })
  const listed = new Set(requests.map(r => r.request))
  for (const ghost of [...ghosts].sort((a, b) => a.index - b.index)) {
    if (!listed.has(ghost.id)) out.splice(Math.min(ghost.index, out.length), 0, { ghost })
  }
  return out
}

function RequestsSection({
  requests,
  ghosts,
  setGhosts,
  held,
  busy,
  act,
  notice,
  setNotice,
}: {
  requests: PeerStateRequest[]
  ghosts: Ghost[]
  setGhosts: (update: (ghosts: Ghost[]) => Ghost[]) => void
  held: boolean
  busy: boolean
  act: Act
  notice: Notice | undefined
  setNotice: (notice: Notice | undefined) => void
}) {
  const listKey = requests.map(r => r.request).join('|')
  const ref = useRef<HTMLDivElement>(null)
  const seen = useRef<Set<string>>(new Set())
  // A request this slot has not shown before → bring the section into view (the attention cue beside the count badge).
  useEffect(() => {
    const fresh = requests.some(r => !seen.current.has(r.request))
    seen.current = new Set(requests.map(r => r.request))
    if (fresh) ref.current?.scrollIntoView?.({ block: 'nearest' })
  }, [listKey]) // eslint-disable-line react-hooks/exhaustive-deps

  /** Answers one card; on success its place keeps a placeholder with the outcome, on failure the card stays and the line says why. */
  const answer = async (
    r: PeerStateRequest,
    index: number,
    card: HTMLElement | null,
    verdict: Ghost['verdict'],
    command: string,
    args: Record<string, unknown>,
  ) => {
    const height = card?.getBoundingClientRect().height ?? 0
    setNotice(undefined)
    const done = await act(null, command, args)
    if (!done.ok) {
      setNotice(done.notice)
      return
    }
    const ghost: Ghost = { id: r.request, index, height, verdict, heading: r.heading, kind: r.kind, summary: r.summary, notice: done.notice }
    setGhosts(gs => [...gs.filter(g => g.id !== ghost.id), ghost])
    setTimeout(() => setGhosts(gs => gs.filter(g => g !== ghost)), ANSWERED_MS)
  }

  const items = merged(requests, ghosts)
  return (
    <div ref={ref} style={section} data-testid="peers-requests" data-peers-geom="" data-waiting={requests.length}>
      <div style={heading}>
        {t('peers.requests')}
        {requests.length > 0 && (
          <span data-testid="peers-requests-count" style={{ marginLeft: 6, border: '1px solid var(--border)', borderRadius: 8, padding: '0 6px' }}>
            {t('peers.requestsWaiting', { count: String(requests.length) })}
          </span>
        )}
      </div>
      {items.length === 0 && <div style={muted}>{t('peers.noRequests')}</div>}
      {items.map((item, index) =>
        'ghost' in item ? (
          <GhostCard key={`~${item.ghost.id}`} ghost={item.ghost} />
        ) : (
          <RequestCard
            key={item.request.request}
            request={item.request}
            held={held}
            busy={busy}
            answer={(card, verdict, command, args) => void answer(item.request, index, card, verdict, command, args)}
          />
        ),
      )}
      <NoticeLine notice={notice} />
    </div>
  )
}

function GhostCard({ ghost }: { ghost: Ghost }) {
  const key = ghost.verdict === 'approved' ? 'peers.answeredApproved' : 'peers.answeredDenied'
  return (
    <div
      data-ghost={ghost.id}
      data-peers-geom=""
      style={{ padding: '6px 0', borderTop: '1px dashed var(--border)', minHeight: ghost.height, boxSizing: 'border-box', ...muted }}
    >
      {/* The summary is the daemon's English — only an older daemon without a heading falls back to it, after the kind word, as its card did. */}
      {ghost.heading === '' && (
        <div>
          <KindWord word={ghost.kind} />
        </div>
      )}
      <Sentence text={t(key, { what: '{what}' })} values={{ what: ghost.heading !== '' ? ghost.heading : ghost.summary }} />
      <NoticeLine notice={ghost.notice} />
    </div>
  )
}

type Answer = (card: HTMLElement | null, verdict: Ghost['verdict'], command: string, args: Record<string, unknown>) => void

function RequestCard({ request: r, held, busy, answer }: { request: PeerStateRequest; held: boolean; busy: boolean; answer: Answer }) {
  const card = useRef<HTMLDivElement>(null)
  return (
    <div ref={card} data-request={r.request} data-peers-geom="" style={{ padding: '6px 0', borderTop: '1px dashed var(--border)' }}>
      <div style={{ fontWeight: 600 }}>{r.heading !== '' ? <Outside>{r.heading}</Outside> : <KindWord word={r.kind} />}</div>
      {/* The one-line summary is the daemon's English; the translated heading and lines say it all — it shows only for an older daemon. */}
      {r.heading === '' && (
        <div style={muted}>
          <Outside>{r.summary}</Outside>
        </div>
      )}
      <div data-testid="peers-request-lines" style={{ marginTop: 4 }}>
        {r.lines.map((line, i) =>
          line === '' ? (
            <div key={i} style={{ height: 6 }} />
          ) : (
            <div key={i}>
              <Outside>{line}</Outside>
            </div>
          ),
        )}
      </div>
      <div style={{ ...muted, marginTop: 4 }}>
        {r.agent_id !== null ? (
          // The system-written id first, then the agent's own name (quoted by the daemon) — as the dialog does.
          r.agent_name !== null ? (
            <Sentence text={t('peers.askedBy', { id: '{id}', name: '{name}' })} values={{ id: r.agent_id.slice(0, 8), name: r.agent_name }} />
          ) : (
            <Sentence text={t('peers.askedByNoName', { id: '{id}' })} values={{ id: r.agent_id.slice(0, 8) }} />
          )
        ) : (
          t('peers.askedBySystem')
        )}
        {r.origin !== null && (
          <>
            {' · '}
            <Sentence text={t('peers.origin', { pc: '{pc}' })} values={{ pc: r.origin }} />
          </>
        )}
      </div>
      <div style={row}>
        <RequestButtons request={r} held={held} busy={busy} answer={(...a) => answer(card.current, ...a)} />
      </div>
    </div>
  )
}

function RequestButtons({
  request,
  held,
  busy,
  answer,
}: {
  request: PeerStateRequest
  held: boolean
  busy: boolean
  answer: (verdict: Ghost['verdict'], command: string, args: Record<string, unknown>) => void
}) {
  const id = request.request
  // The buttons that say yes wait out the hold; the ones that say no never do.
  const yesOff = busy || held
  const approve = (extra: Record<string, unknown> = {}) => answer('approved', 'peers.approve', { request: id, ...extra })
  const deny = (extra: Record<string, unknown> = {}) => answer('denied', 'peers.deny', { request: id, ...extra })
  if (request.kind === 'adopt') {
    // Adopt / not this run / start fresh — the third answer is an approve with `start_fresh`, never a deny with `stop_asking`.
    return (
      <>
        <Btn fixed guarded off={yesOff} testId="peers-adopt" onClick={() => approve()}>
          {t('peers.adopt')}
        </Btn>
        <Btn fixed off={busy} onClick={() => deny()}>
          {t('peers.notThisRun')}
        </Btn>
        <ConfirmBtn
          fixed
          off={yesOff}
          testId="peers-start-fresh"
          warning={t('peers.startFreshConfirm')}
          onConfirm={() => approve({ startFresh: true })}
        >
          {t('peers.startFresh')}
        </ConfirmBtn>
      </>
    )
  }
  if (request.kind === 'acceptList') {
    return (
      <>
        <Btn fixed guarded off={yesOff} testId="peers-accept" onClick={() => approve()}>
          {t('peers.acceptList')}
        </Btn>
        <Btn fixed off={busy} onClick={() => deny()}>
          {t('peers.notNow')}
        </Btn>
        <Btn fixed off={busy} onClick={() => deny({ stopAsking: true })}>
          {t('peers.notThisRun')}
        </Btn>
      </>
    )
  }
  return (
    <>
      <Btn fixed guarded off={yesOff} testId="peers-approve" onClick={() => approve()}>
        {t('peers.approve')}
      </Btn>
      <Btn fixed off={busy} testId="peers-deny" onClick={() => deny()}>
        {t('peers.deny')}
      </Btn>
      <Btn fixed off={busy} testId="peers-deny-stop" onClick={() => deny({ stopAsking: true })}>
        {t('peers.denyAndStop')}
      </Btn>
    </>
  )
}

function PcsSection({
  pcs,
  held,
  busy,
  act,
  notice,
}: {
  pcs: PeerStatePc[]
  held: boolean
  busy: boolean
  act: Act
  notice: Notice | undefined
}) {
  const [adding, setAdding] = useState(false)
  return (
    <div style={section} data-peers-geom="">
      <div style={heading}>{t('peers.pcs')}</div>
      {pcs.length === 0 && <div style={muted}>{t('peers.noPcs')}</div>}
      {pcs.map(pc => (
        <PcRow key={pc.id} pc={pc} held={held} busy={busy} act={act} />
      ))}
      {adding ? (
        <AddForm scope="pcs" busy={busy} act={act} onDone={() => setAdding(false)} />
      ) : (
        <div style={row}>
          <Btn testId="peers-add" off={busy} onClick={() => setAdding(true)}>
            {t('peers.add')}
          </Btn>
        </div>
      )}
      <NoticeLine notice={notice} />
    </div>
  )
}

function PcRow({ pc, held, busy, act }: { pc: PeerStatePc; held: boolean; busy: boolean; act: Act }) {
  const [editing, setEditing] = useState<'name' | 'address' | null>(null)
  const [draft, setDraft] = useState('')
  const start = (what: 'name' | 'address') => {
    setEditing(what)
    setDraft(what === 'name' ? pc.name : (pc.address ?? ''))
  }
  // The edit stays open with what was typed until the daemon applies it — a refusal loses nothing.
  const save = async () => {
    const done =
      editing === 'name'
        ? await act('pcs', 'peers.rename', { id: pc.id, name: draft })
        : await act('pcs', 'peers.setAddress', { id: pc.id, address: draft })
    if (done.ok) setEditing(null)
  }
  return (
    <div data-pc={pc.id} style={{ padding: '4px 0' }}>
      <div style={row}>
        <strong>
          <Outside>{pc.name}</Outside>
        </strong>
        <span data-online={pc.online}>
          <OnlineWord word={pc.online} />
        </span>
        {pc.address !== null && (
          <span style={muted}>
            <Outside mono>{pc.address}</Outside>
          </span>
        )}
      </div>
      <div style={muted}>
        <Outside mono>{groupId(pc.id)}</Outside>
      </div>
      {pc.online === 'refused' && <div style={muted}>{t('peers.refusedHint')}</div>}
      {pc.online === 'version_mismatch' && <div style={muted}>{t('peers.versionHint')}</div>}
      {pc.warning !== null && (
        <div role="alert">
          <Outside>{pc.warning}</Outside>
        </div>
      )}
      {editing !== null ? (
        <div style={row}>
          <input
            style={input}
            aria-label={editing === 'name' ? t('peers.name') : t('peers.address')}
            placeholder={editing === 'address' ? t('peers.addressPlaceholder') : undefined}
            value={draft}
            onChange={e => setDraft(e.target.value)}
          />
          <Btn off={busy} testId="peers-edit-save" onClick={() => void save()}>
            {t('peers.save')}
          </Btn>
          <Btn onClick={() => setEditing(null)}>{t('peers.cancel')}</Btn>
        </div>
      ) : (
        <div style={row}>
          <Btn off={busy} onClick={() => start('name')}>
            {t('peers.rename')}
          </Btn>
          <Btn off={busy} onClick={() => start('address')}>
            {t('peers.editAddress')}
          </Btn>
          <ConfirmBtn
            off={busy || held}
            testId="peers-remove"
            warning={<Sentence text={t('peers.removeConfirm', { name: '{name}' })} values={{ name: pc.name }} />}
            onConfirm={() => void act('pcs', 'peers.remove', { id: pc.id })}
          >
            {t('peers.remove')}
          </ConfirmBtn>
        </div>
      )}
    </div>
  )
}

function AddForm({
  scope,
  busy,
  act,
  onDone,
  initialId,
}: {
  scope: Scope
  busy: boolean
  act: Act
  onDone: () => void
  initialId?: string
}) {
  const [id, setId] = useState(initialId ?? '')
  const [name, setName] = useState('')
  const [address, setAddress] = useState('')
  return (
    <div data-testid="peers-add-form">
      {initialId !== undefined && <div role="note">{t('peers.knockConfirm')}</div>}
      <div style={row}>
        <input style={{ ...input, flex: 2 }} aria-label={t('peers.id')} placeholder={t('peers.id')} value={id} onChange={e => setId(e.target.value)} />
        <input style={{ ...input, flex: 1 }} aria-label={t('peers.name')} placeholder={t('peers.name')} value={name} onChange={e => setName(e.target.value)} />
        <input
          style={{ ...input, flex: 1 }}
          aria-label={t('peers.address')}
          placeholder={t('peers.addressPlaceholder')}
          value={address}
          onChange={e => setAddress(e.target.value)}
        />
      </div>
      <div style={row}>
        <Btn
          testId="peers-add-save"
          off={busy || id.trim() === '' || name.trim() === ''}
          onClick={() => {
            // The form stays with what was typed until the daemon applies it — a refusal loses nothing.
            void act(scope, 'peers.add', { id, name, address }).then(done => {
              if (done.ok) onDone()
            })
          }}
        >
          {t('peers.save')}
        </Btn>
        <Btn onClick={onDone}>{t('peers.cancel')}</Btn>
      </div>
    </div>
  )
}

function KnocksSection({ state, busy, act, notice }: { state: PeerState; busy: boolean; act: Act; notice: Notice | undefined }) {
  const [adding, setAdding] = useState<string | null>(null)
  return (
    <div style={section}>
      <div style={heading}>{t('peers.knocks')}</div>
      {state.knocks.map(k => (
        <div key={k.id} data-knock={k.id} style={{ padding: '2px 0' }}>
          <div style={{ wordBreak: 'break-all' }}>
            <Outside mono>{groupId(k.id)}</Outside>
          </div>
          <div style={row}>
            <span style={muted}>{t('peers.knockCount', { count: String(k.count) })}</span>
            <Btn off={busy} onClick={() => setAdding(k.id)}>
              {t('peers.knockAdd')}
            </Btn>
          </div>
          {adding === k.id && <AddForm scope="knocks" busy={busy} act={act} initialId={k.id} onDone={() => setAdding(null)} />}
        </div>
      ))}
      <NoticeLine notice={notice} />
    </div>
  )
}

function NetworkSection({ state, busy, act, notice }: { state: PeerState; busy: boolean; act: Act; notice: Notice | undefined }) {
  const network = state.network!
  const [port, setPort] = useState(network.port === null ? '' : String(network.port))
  const [relay, setRelay] = useState(network.relay)
  const [lookup, setLookup] = useState(network.address_lookup)
  const [mdns, setMdns] = useState(network.mdns)
  return (
    <div style={section} data-testid="peers-network">
      <div style={heading}>{t('peers.network')}</div>
      <div style={row}>
        <label>
          {t('peers.port')}{' '}
          <input
            style={{ ...input, width: 90 }}
            value={port}
            placeholder={t('peers.portDefault', { port: String(network.listen_port) })}
            onChange={e => setPort(e.target.value)}
          />
        </label>
      </div>
      <div style={row}>
        <label>
          {t('peers.relay')} <input style={input} value={relay} placeholder={t('peers.helperHint')} onChange={e => setRelay(e.target.value)} />
        </label>
      </div>
      <div style={row}>
        <label>
          {t('peers.addressLookup')}{' '}
          <input style={input} value={lookup} placeholder={t('peers.helperHint')} onChange={e => setLookup(e.target.value)} />
        </label>
      </div>
      <div style={row}>
        <label>
          <input type="checkbox" checked={mdns} onChange={e => setMdns(e.target.checked)} /> {t('peers.mdns')}
        </label>
      </div>
      <div style={row}>
        <Btn
          off={busy}
          onClick={() =>
            void act('network', 'peers.setNetwork', {
              port: port.trim() === '' ? 'default' : port,
              relay,
              address_lookup: lookup,
              mdns,
            })
          }
        >
          {t('peers.apply')}
        </Btn>
      </div>
      <NoticeLine notice={notice} />
    </div>
  )
}
