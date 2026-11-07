// Usage limit slot (TRD S21 usage-limit-slot §1-8) — draws the remaining amount of enabled companies as a small display and opens a detail popup when pressed.
//
// ★Operations go only through commands (§5)★ — the ⟳ of the small display (all enabled companies) and the popup's per-company ⟳ (`vendor` argument) call `usageSlot.refresh`,
//   and the popup's display toggle calls `usageSlot.toggle*`. The slot right-click menu has no usage item (user decision 2026-09-29).
// ★Only enabled companies are drawn★ — a disabled company, even if the store has values, is nowhere in the small display · badge · popup detail, and its name remains only in the popup's display
//   toggle row (the place to turn it back on). Query demand (interest) is computed by the shell from the layout, so this component does not report interest.
// ★The width stage is chosen by the drawn width (R3)★ — for each stage the natural width of the hidden render is measured and compared with the real width. No px threshold
//   is set: when font · text · values change the threshold would have to move with them.
// ★Time is driven by a single per-minute tick★ — age · remaining time · next attempt flow locally with no request (R32).
// Values are kept as DOM text + `data-usage-vendor`/`data-usage-window` (R27 — LLM and cdp read them).

import { Fragment, useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import type { CSSProperties, RefObject } from 'react'
import { Hourglass, RefreshCw } from 'lucide-react'
import { openUrl } from '@tauri-apps/plugin-opener'

import type { SlotContent } from '../../api/layoutTypes'
import type { AgentBackendKind } from '../../api/types'
import { fireAndForget } from '../../commands/dispatch'
import { t } from '../../i18n'
import {
  blocksRefresh,
  useUsagePending,
  useUsageStore,
  useUsageVendor,
  type UsageVendorEntry,
} from '../../store/usageStore'
import { ANCHOR_GAP, clampMenuPosition } from './SlotContextMenu'
import VendorIcon from './VendorIcon'
import {
  STALE_AFTER_SECS,
  elapsedSecs,
  formatAge,
  formatDuration,
  formatResetAt,
  formatResetClock,
  isStale,
  oldestAgeSecs,
  readWindow,
  stateHead,
  statusLine,
  statusSentence,
  type WindowReading,
} from './usageFormat'

type UsageContent = Extract<SlotContent, { type: 'usage' }>
type Stage = 1 | 2 | 3
type ValueReading = Extract<WindowReading, { kind: 'value' }>

/**
 * Per company, the display field of the slot content and the command that flips that field. The popup's display toggle rows are drawn in the order of this table — when a company is added
 * (`AgentBackendKind`) the type catches the missing row.
 */
const VENDOR_SLOT: Record<AgentBackendKind, { show: Exclude<keyof UsageContent, 'type'>; toggle: string }> = {
  claude: { show: 'show_claude', toggle: 'usageSlot.toggleClaude' },
  codex: { show: 'show_codex', toggle: 'usageSlot.toggleCodex' },
}
const USAGE_VENDORS = Object.keys(VENDOR_SLOT) as AgentBackendKind[]

/**
 * The 「사용량 페이지 ↗」 (usage page) destination. ★It must match the shell's opener allow list (`src-tauri/capabilities/usage-links.json`) down to the character★
 * — if even one query or trailing slash differs the shell refuses to open it. `UsageSlot.test.tsx` does the comparison.
 */
export const USAGE_PAGE_URL: Record<AgentBackendKind, string> = {
  claude: 'https://claude.ai/settings/usage',
  codex: 'https://chatgpt.com/codex/settings/usage',
}

const TICK_MS = 60_000

// Inner padding of the summary button. The horizontal value enters the stage decision — the hidden copy is measured without padding, and the width the content will use is obtained by subtracting it from the slot width.
const SUMMARY_PAD_Y_PX = 4
const SUMMARY_PAD_X_PX = 6

interface WindowView {
  /** `five_hour`·`weekly`, and the popup's per-model window is `model:<label>`. */
  key: string
  label: string
  reading: WindowReading
}

interface VendorView {
  vendor: AgentBackendKind
  name: string
  entry: UsageVendorEntry | undefined
  /** Seconds elapsed since receipt. */
  elapsed: number
  /** One line for an abnormal state carrying the company name — null if `Ready` or no value. The badge and the top line of the popup use this same string. */
  line: string | null
  /** Short name of an abnormal state (for the summary button's accessibility name) — null if `Ready` or no value. */
  head: string | null
  refreshing: boolean
  rejected: boolean
  windows: WindowView[]
}

function vendorName(vendor: AgentBackendKind): string {
  switch (vendor) {
    case 'claude':
      return t('usage.vendorClaude')
    case 'codex':
      return t('usage.vendorCodex')
  }
}

function buildVendorView(
  vendor: AgentBackendKind,
  entry: UsageVendorEntry | undefined,
  pending: boolean,
  nowPerf: number,
  nowWall: number,
): VendorView {
  const name = vendorName(vendor)
  const snapshot = entry?.snapshot
  const elapsed = entry ? elapsedSecs(entry.receivedAt, nowPerf) : 0
  // `Unavailable` carries no value — even if something is carried it is not drawn (gray "—").
  const valueless = snapshot === undefined || snapshot.state.kind === 'Unavailable'
  const windowOf = (key: 'five_hour' | 'weekly') => (valueless ? null : snapshot[key])
  const sentence = snapshot ? statusSentence(snapshot.state, elapsed, nowWall) : null
  return {
    vendor,
    name,
    entry,
    elapsed,
    line: sentence === null ? null : statusLine(name, sentence),
    head: snapshot ? stateHead(snapshot.state, elapsed) : null,
    refreshing: pending || (snapshot?.in_flight ?? false),
    rejected: blocksRefresh(snapshot?.state),
    windows: [
      {
        key: 'five_hour',
        label: t('usage.windowFiveHour'),
        reading: readWindow(windowOf('five_hour'), elapsed, nowWall),
      },
      {
        key: 'weekly',
        label: t('usage.windowWeekly'),
        reading: readWindow(windowOf('weekly'), elapsed, nowWall),
      },
    ],
  }
}

function a11yValue(r: WindowReading): string {
  if (r.kind === 'value') return t('usage.meterValueText', { pct: String(r.visible) })
  if (r.kind === 'expired') return t('usage.resetWaitingShort')
  return t('usage.a11yNoValue')
}

/** Accessibility name of the summary button — company · the number visible per window · state. */
function summaryLabel(views: VendorView[]): string {
  const parts = views.map(view => {
    const [first, second] = view.windows.map(w => t('usage.a11yWindow', { window: w.label, value: a11yValue(w.reading) }))
    const refreshing = view.refreshing ? t('usage.refreshing') : null
    const status =
      view.head !== null && refreshing !== null
        ? t('usage.a11yStatusRefreshing', { status: view.head, refreshing })
        : (view.head ?? refreshing)
    return status === null
      ? t('usage.a11yVendor', { vendor: view.name, first, second })
      : t('usage.a11yVendorStatus', { vendor: view.name, first, second, status })
  })
  return parts.length === 2 ? t('usage.a11yTwoVendors', { first: parts[0], second: parts[1] }) : parts[0]
}

function useMinuteTick(): void {
  const [, setTick] = useState(0)
  useEffect(() => {
    const id = setInterval(() => setTick(n => n + 1), TICK_MS)
    return () => clearInterval(id)
  }, [])
}

export default function UsageSlot({
  content,
  viewId,
  slotId,
}: {
  content: UsageContent
  /** This slot's coordinates — ⟳ and the display toggle carry them to the command as they are. `null` = the active tab is not yet known (the toggle command refuses). */
  viewId: string | null
  slotId: string
}) {
  // TRD §3 #82: separately from the boot pull after the broadcast is connected, even if the slot is placed late it is drawn at once from the shell cache.
  useEffect(() => {
    void useUsageStore.getState().pull()
  }, [])
  useMinuteTick()
  const claude = useUsageVendor('claude')
  const codex = useUsageVendor('codex')
  const claudePending = useUsagePending('claude')
  const codexPending = useUsagePending('codex')

  const nowPerf = performance.now()
  const nowWall = Date.now() / 1000
  const views: VendorView[] = []
  if (content.show_claude) views.push(buildVendorView('claude', claude, claudePending, nowPerf, nowWall))
  if (content.show_codex) views.push(buildVendorView('codex', codex, codexPending, nowPerf, nowWall))

  return <UsageSummary views={views} nowWall={nowWall} content={content} viewId={viewId} slotId={slotId} />
}

// ── Small display ──────────────────────────────────────────────────────────────────────────────

interface Measures {
  /** Width of the slot root (`width:100%` — it does not change with content or stage). */
  rootW: number | null
  /**
   * Width of the ⟳ area (including the right margin of ⟳). If there is no ⟳ (a notice text) it is 0 — the observer does not notify a 0×0 element at the start of observation, so
   * it starts from 0, not null.
   */
  refreshW: number
  s1: number | null
  s2: number | null
}

/**
 * Width the content can use = slot width − ⟳ area − left/right margins of the summary button. The measurement copies (`s1`·`s2`) are measured without ⟳ — the ⟳ share is subtracted here
 * only once (putting it in the copies would subtract it twice).
 */
function contentAvail({ rootW, refreshW }: Measures): number | null {
  return rootW === null ? null : rootW - refreshW - 2 * SUMMARY_PAD_X_PX
}

// ADR-0259: the stage is chosen by width only — do not revive the height fallback (stage 3 if stage 2 is taller than the slot). If revived, in a low slot
//   stage 1 skips stage 2 and goes straight to stage 3. When height is short, the root's `overflow: hidden` cuts the lower line, and that line's
//   badge is hidden with it (the price accepted — PRD R31).
/** The widest stage that fits. If not yet measured, stage 1 (the most detailed). If narrower than stage 3, stage 3 is cut with an ellipsis. */
function pickStage(m: Measures): Stage {
  const { s1, s2 } = m
  const avail = contentAvail(m)
  if (avail === null || s1 === null) return 1
  if (s1 <= avail) return 1
  if (s2 !== null && s2 <= avail) return 2
  return 3
}

const ROOT_STYLE: CSSProperties = {
  position: 'relative',
  width: '100%',
  height: '100%',
  overflow: 'hidden',
  fontFamily: 'var(--font-ui)',
  fontSize: '12px',
  color: 'var(--text)',
}

const LINE_STYLE: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '0.6em',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  minWidth: 0,
}

// Stages 1·2 put all lines on one grid — sharing cells is what lines up the bars · % · reset times of two companies · two windows vertically (user decision
//   2026-09-29). One window = name | bar | % | reset time.
const WINDOW_TRACKS = 'auto minmax(4em, 1fr) auto auto'
const STAGE1_COLUMNS = `auto ${WINDOW_TRACKS} ${WINDOW_TRACKS}`
const STAGE2_COLUMNS = `auto ${WINDOW_TRACKS}`
const WINDOW_TRACK_COUNT = 4

// Of the gaps between cells, bar↔% is the narrowest (user decision 2026-09-29 — so the bar and % read together). Between other cells a margin is added.
const GRID_STYLE: CSSProperties = {
  display: 'grid',
  alignItems: 'center',
  columnGap: '4px',
  rowGap: '2px',
  whiteSpace: 'nowrap',
  fontVariantNumeric: 'tabular-nums',
}
const NAME_TO_LABEL = '0.3em'
const GROUP_TO_GROUP = '0.9em'
const LABEL_TO_BAR = '0.25em'
const PCT_TO_RESET = '0.45em'

/** The on-screen rectangle of the summary button — the popup opens downward, and when flipped it opens from the summary's top edge (so it does not cover the summary). */
interface Anchor {
  left: number
  top: number
  bottom: number
}

function UsageSummary({
  views,
  nowWall,
  content,
  viewId,
  slotId,
}: {
  views: VendorView[]
  nowWall: number
  content: UsageContent
  viewId: string | null
  slotId: string
}) {
  const rootRef = useRef<HTMLDivElement>(null)
  const rowRef = useRef<HTMLDivElement>(null)
  const refreshAreaRef = useRef<HTMLSpanElement>(null)
  const measure1Ref = useRef<HTMLDivElement>(null)
  const measure2Ref = useRef<HTMLDivElement>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)
  const [measures, setMeasures] = useState<Measures>({ rootW: null, refreshW: 0, s1: null, s2: null })
  const [open, setOpen] = useState(false)

  // Slot width · ⟳ area · the natural widths of two stages are measured by one observer — the hidden render is called again because its size changes when values·text change.
  // ★Elements whose size changes with the chosen stage (summary button · content · line) are not measured★ — if the stage changes its own input, a width once
  //   shrunk to stage 3 does not widen again or the stage goes back and forth.
  useLayoutEffect(() => {
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(entries => {
      setMeasures(prev => {
        let next = prev
        const put = (key: keyof Measures, value: number) => {
          if (next[key] !== value) next = { ...next, [key]: value }
        }
        for (const entry of entries) {
          const { width } = entry.contentRect
          if (entry.target === rootRef.current) put('rootW', width)
          else if (entry.target === refreshAreaRef.current) put('refreshW', width)
          else if (entry.target === measure1Ref.current) put('s1', width)
          else if (entry.target === measure2Ref.current) put('s2', width)
        }
        return next
      })
    })
    for (const el of [rootRef.current, refreshAreaRef.current, measure1Ref.current, measure2Ref.current]) {
      if (el) observer.observe(el)
    }
    return () => observer.disconnect()
  }, [])

  const stage = pickStage(measures)

  const closePopup = useCallback((restoreFocus: boolean) => {
    setOpen(false)
    if (restoreFocus) buttonRef.current?.focus()
  }, [])

  const runSlotCommand = (id: string) => fireAndForget(id, { viewId, slotId, content })
  const refreshVendor = (vendor: AgentBackendKind) =>
    fireAndForget('usageSlot.refresh', { viewId, slotId, content, vendor })
  // Even if presses come in succession before the broadcast, the old `content` is sent as is — the toggle command writes only its own one field, so it does not
  //   undo another company's earlier press (`usageCommands`). The check mark is `content` too (no optimistic update — ADR-0035).
  const toggleShown = (vendor: AgentBackendKind) => runSlotCommand(VENDOR_SLOT[vendor].toggle)
  const shown = Object.fromEntries(USAGE_VENDORS.map(v => [v, content[VENDOR_SLOT[v].show]])) as Record<
    AgentBackendKind,
    boolean
  >

  return (
    <div ref={rootRef} data-usage-slot="" style={ROOT_STYLE}>
      {/* ★⟳ is a sibling outside the summary button★ — no button inside a button (the press would overlap with opening the popup).
          ★⟳ is right next to the content in every stage★ (position = ADR-0258 decision 1 · user 2026-09-30 「알아서 잘 붙이도록」 (attach it well on your own) ·
          method = TRD §3 #104) — in stages 1·2 the grid's bar cell (`1fr`) fills the summary button to the end of the line, and in stage 3 there is no cell to fill so the line shrinks to its own
          content width. If the content is wider than the slot the line stops at the slot width (`maxWidth` — without it the minimum
          width of a line that cannot wrap pushes ⟳ out of the slot) and the summary button shrinks and cuts the content — the ⟳ area does not shrink. So in stage 3, when
          values change the content width, like digit count or a badge, ⟳ moves with it. The querying indicator is ⟳'s own rotation so it does not change the content width (ADR-0261). */}
      <div
        ref={rowRef}
        data-usage-row=""
        style={{ display: 'flex', alignItems: 'center', maxWidth: '100%', width: stage === 3 ? 'fit-content' : undefined }}
      >
        <button
          ref={buttonRef}
          type="button"
          data-usage-summary=""
          aria-haspopup="dialog"
          aria-expanded={open}
          // Children of a button are flattened for assistive tech, so a short name is given instead of the bar·badge inside (D14). If there is only a notice text, that text is the name.
          aria-label={views.length > 0 ? summaryLabel(views) : undefined}
          // Keyboard (Enter·Space) calls the same click through native button activation (D14).
          // ADR-0259: it only opens and closes — do not attach a query when opening the popup. There is a periodic query, so not fetching on open is
          //   the custom, and attaching it raises the Claude 429 risk with every open action (429 is not caught by blocking during refusal — TRD §6 #21).
          onClick={() => setOpen(o => !o)}
          style={{
            display: 'block',
            flexGrow: 1,
            flexShrink: 1,
            flexBasis: 'auto',
            minWidth: 0,
            padding: `${SUMMARY_PAD_Y_PX}px ${SUMMARY_PAD_X_PX}px`,
            margin: 0,
            border: 0,
            background: 'transparent',
            color: 'inherit',
            font: 'inherit',
            textAlign: 'left',
            cursor: 'pointer',
          }}
        >
          <span data-usage-content="" data-usage-stage={stage} style={{ display: 'block', overflow: 'hidden' }}>
            {views.length === 0 ? (
              <span data-usage-hint="" style={{ color: 'var(--text-muted)' }}>
                {t('usage.hint')}
              </span>
            ) : (
              <StageRender stage={stage} views={views} nowWall={nowWall} measure={false} />
            )}
          </span>
        </button>
        {/* Kept even when there is no ⟳ — the observer receives the ⟳ area width (0 if none) from an element it grabbed once. */}
        <span ref={refreshAreaRef} data-usage-refresh-area="" style={{ flex: 'none', display: 'flex' }}>
          {views.length > 0 && <RefreshButton views={views} onRefresh={() => runSlotCommand('usageSlot.refresh')} />}
        </span>
      </div>
      <div
        aria-hidden="true"
        style={{ position: 'absolute', top: 0, left: 0, visibility: 'hidden', pointerEvents: 'none' }}
      >
        <div ref={measure1Ref} data-usage-measure="1" style={{ width: 'max-content' }}>
          <StageRender stage={1} views={views} nowWall={nowWall} measure />
        </div>
        <div ref={measure2Ref} data-usage-measure="2" style={{ width: 'max-content' }}>
          <StageRender stage={2} views={views} nowWall={nowWall} measure />
        </div>
      </div>
      {open && (
        <UsagePopup
          summaryRef={buttonRef}
          views={views}
          nowWall={nowWall}
          ownerRef={rowRef}
          shown={shown}
          onToggle={toggleShown}
          onRefresh={refreshVendor}
          onClose={closePopup}
        />
      )}
    </div>
  )
}

/**
 * The ⟳ of the small display — refreshes the enabled companies at once. The targets (among enabled companies, those without a visible refusal) are picked again by the command
 * from the store at press time.
 */
function RefreshButton({ views, onRefresh }: { views: VendorView[]; onRefresh: () => void }) {
  const targets = views.filter(v => !v.rejected)
  // This marker carries whether each company is being queried (R27 — cdp·LLM) — a single rotation does not show which company is running. A refusing company's
  //   query is carried too (the rotation looks only at the targets).
  const inFlight = views.filter(v => v.refreshing).map(v => v.vendor)
  return (
    <RefreshIconButton
      label={t('usage.refreshAll')}
      blocked={targets.length === 0}
      busy={targets.some(v => v.refreshing)}
      onRefresh={onRefresh}
      attrs={{
        'data-usage-refresh': '',
        ...(inFlight.length > 0 ? { 'data-usage-refreshing': inFlight.join(' ') } : {}),
      }}
      // The margin is put on the ⟳ side — the content width of the ⟳ area (`data-usage-refresh-area`) wraps this margin too and enters the ⟳ share of the stage decision.
      marginRight={SUMMARY_PAD_X_PX}
    />
  )
}

/**
 * A ⟳-shaped button — the small display's ⟳ and the popup's per-company ⟳ use the same press rule (ADR-0259 decision 2 — applying the global ⟳ rule).
 * `blocked` = a visible refusal so the press is blocked · `busy` = the target's query is running so the press is discarded. The icon rotates while querying.
 */
function RefreshIconButton({
  label,
  blocked,
  busy,
  onRefresh,
  attrs,
  marginRight,
}: {
  label: string
  blocked: boolean
  busy: boolean
  onRefresh: () => void
  attrs: Record<string, string>
  marginRight?: number
}) {
  // ADR-0261: rotation = starts turning when a query turns on (including automatic queries) and, after the query ends, stops at the boundary (`animationiteration`)
  //   of the wheel that was turning — at least one turn, and it does not jump to 0° at the end. On every press the icon is attached anew to restart the rotation from 0°
  //   (the confirmation of a press — even a press that sent no query). The wheel length is `animate-spin` (1 second) as it is.
  // ★It turns even under reduced motion and e-ink★ — this repo's rotating-symbol rule (`agent/agentGlyph.css` · `ui/loading-panel.css` —
  //   user decisions 2026-08-24·2026-09-24: a stopped spinner cannot be told apart from 「멈춤」 (stopped)).
  const [spinning, setSpinning] = useState(false)
  const [spinRun, setSpinRun] = useState(0)
  if (busy && !spinning) setSpinning(true)
  // ADR-0258: only the two below block a press — no time interval is set (the 429 that rapid presses would call is an accepted risk).
  // It is blocked during a visible refusal — within the deadline ⟳ neither queries nor rotates (R24 · D11). A press while a query is running is discarded —
  //   only the rotation restarts from the beginning (ADR-0261).
  return (
    <button
      type="button"
      {...attrs}
      aria-label={label}
      title={label}
      // ★It is `aria-disabled`, not `disabled`★ — at the moment a focused ⟳ turns into a refusal, `disabled` drops focus to
      //   body (HTML focus fixup). Focus is kept and only the press is blocked.
      aria-disabled={blocked || undefined}
      aria-busy={busy || undefined}
      onClick={() => {
        if (blocked) return
        if (!busy) onRefresh()
        setSpinning(true)
        setSpinRun(n => n + 1)
      }}
      style={{
        display: 'inline-flex',
        marginRight,
        padding: '2px',
        border: '1px solid var(--border)',
        borderRadius: '3px',
        background: 'transparent',
        color: 'inherit',
        cursor: blocked ? 'default' : busy ? 'progress' : 'pointer',
        opacity: blocked ? 0.4 : undefined,
      }}
    >
      <RefreshCw
        key={spinRun}
        aria-hidden="true"
        className={spinning ? 'size-3.5 animate-spin' : 'size-3.5'}
        onAnimationIteration={() => {
          if (!busy) setSpinning(false)
        }}
      />
    </button>
  )
}

/**
 * `measure` = a hidden copy that only measures size — draws the same text · same shape but carries no role · name · `data-usage-*` · animation
 * (so cdp and assistive tech do not read values twice). The line is a `span` too because it goes inside the summary button (button content = phrasing).
 */
function StageRender({
  stage,
  views,
  nowWall,
  measure,
}: {
  stage: Stage
  views: VendorView[]
  nowWall: number
  measure: boolean
}) {
  if (stage === 1) {
    return (
      <span
        {...(measure ? {} : { 'data-usage-grid': '' })}
        style={{ ...GRID_STYLE, gridTemplateColumns: STAGE1_COLUMNS }}
      >
        {views.map((view, v) => (
          <Fragment key={view.vendor}>
            <NameUnit view={view} measure={measure} cell={{ gridRow: v + 1, gridColumn: 1 }} />
            {view.windows.map((w, i) => (
              <WindowCells
                key={w.key}
                view={view}
                w={w}
                nowWall={nowWall}
                measure={measure}
                row={v + 1}
                column={2 + i * WINDOW_TRACK_COUNT}
                lead={i === 0 ? NAME_TO_LABEL : GROUP_TO_GROUP}
              />
            ))}
          </Fragment>
        ))}
      </span>
    )
  }
  if (stage === 2) {
    return (
      <span
        {...(measure ? {} : { 'data-usage-grid': '' })}
        style={{ ...GRID_STYLE, gridTemplateColumns: STAGE2_COLUMNS }}
      >
        {views.map((view, v) => {
          const first = v * view.windows.length + 1
          return (
            <Fragment key={view.vendor}>
              <NameUnit
                view={view}
                measure={measure}
                cell={{ gridRow: `${first} / span ${view.windows.length}`, gridColumn: 1 }}
              />
              {view.windows.map((w, i) => (
                <WindowCells
                  key={w.key}
                  view={view}
                  w={w}
                  nowWall={nowWall}
                  measure={measure}
                  row={first + i}
                  column={2}
                  lead={NAME_TO_LABEL}
                />
              ))}
            </Fragment>
          )
        })}
      </span>
    )
  }
  return (
    <>
      {views.map(view => (
        <span key={view.vendor} style={LINE_STYLE}>
          <NameUnit view={view} measure={measure} />
          {/* Ellipsis only on the number side — the icon·badge do not shrink so they are not cut (R31). */}
          <span
            data-usage-numbers=""
            style={{
              flex: '1 1 auto',
              minWidth: 0,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {view.windows.map((w, i) => (
              <Fragment key={w.key}>
                {i > 0 && '·'}
                <span {...windowAttrs(view, w, measure)}>
                  <ValueText w={w} bare dimStale measure={measure} />
                </span>
              </Fragment>
            ))}
          </span>
        </span>
      ))}
    </>
  )
}

function windowAttrs(view: VendorView, w: WindowView, measure: boolean): Record<string, string> {
  return measure ? {} : { 'data-usage-vendor': view.vendor, 'data-usage-window': w.key }
}

const NAME_UNIT_GAP = '0.15em'

/** Company icon + badge — one lump that does not shrink (so the badge is not cut by the ellipsis — R31). */
function NameUnit({ view, measure, cell }: { view: VendorView; measure: boolean; cell?: CSSProperties }) {
  return (
    <span style={{ ...cell, flex: 'none', display: 'inline-flex', alignItems: 'center', gap: NAME_UNIT_GAP }}>
      <VendorIcon
        vendor={view.vendor}
        label={measure ? null : view.name}
        {...(measure ? {} : { 'data-usage-icon': view.vendor })}
      />
      {view.line !== null && <Badge view={view} measure={measure} />}
    </span>
  )
}

/** Abnormal state marker — the name · sentence is the same as the popup's top line. */
function Badge({ view, measure }: { view: VendorView; measure: boolean }) {
  const label = view.line ?? ''
  return (
    <span
      {...(measure ? {} : { 'data-usage-badge': view.vendor, role: 'img', 'aria-label': label, title: label })}
      style={{ color: 'var(--usage-badge)', fontWeight: 700 }}
    >
      !
    </span>
  )
}

/**
 * One window of the stage 1·2 grid — name | bar (only when there is a value) | remaining % | reset time, four cells from `column`. If expired, one 「리셋됨 — 갱신 대기」 (reset — waiting for refresh) cell covers the
 * bar · % · reset time cells. The wrapping `span` is `display: contents` so the cells become grid items directly, and it
 * carries only the value marker (`data-usage-vendor`·`data-usage-window`).
 */
function WindowCells({
  view,
  w,
  nowWall,
  measure,
  row,
  column,
  lead,
}: {
  view: VendorView
  w: WindowView
  nowWall: number
  measure: boolean
  row: number
  column: number
  lead: string
}) {
  const r = w.reading
  const at = (offset: number, span = 1): CSSProperties => ({
    gridRow: row,
    gridColumn: span === 1 ? column + offset : `${column + offset} / span ${span}`,
  })
  return (
    <span {...windowAttrs(view, w, measure)} style={{ display: 'contents' }}>
      <span style={{ ...at(0), marginLeft: lead, color: 'var(--text-muted)' }}>{w.label}</span>
      {r.kind === 'expired' ? (
        <span style={{ ...at(1, 3), marginLeft: LABEL_TO_BAR }}>
          <ValueText w={w} bare={false} dimStale measure={measure} />
        </span>
      ) : (
        <>
          {r.kind === 'value' && (
            <Bar
              view={view}
              label={w.label}
              r={r}
              meter={false}
              measure={measure}
              cell={{ ...at(1), width: 'auto', marginLeft: LABEL_TO_BAR }}
            />
          )}
          <span style={{ ...at(2), textAlign: 'right' }}>
            <ValueText w={w} bare={false} dimStale measure={measure} />
          </span>
          {r.resetsAt !== null && (
            <ResetMark
              resetsAt={r.resetsAt}
              nowWall={nowWall}
              measure={measure}
              attrs={{ 'data-usage-reset-clock': '' }}
              style={{ ...at(3), marginLeft: PCT_TO_RESET }}
            />
          )}
        </>
      )}
    </span>
  )
}

const RESET_ICON_GAP = '0.15em'

// ADR-0259: reset = hourglass + time — the word 「리셋」 (reset) remains only in the tooltip (`title`). Do not use arrow icons (`History`·`RotateCcw`·
//   `RotateCw`·`Repeat` etc.) — they get mixed up with ⟳.
/**
 * Reset time — shared by the small display and the popup. Tooltip = 「리셋 11:29」 (reset 11:29) (on another day, date + time). `measure` = hidden copy (no name ·
 * role · `attrs`).
 */
function ResetMark({
  resetsAt,
  nowWall,
  measure,
  attrs,
  style,
}: {
  resetsAt: number
  nowWall: number
  measure: boolean
  attrs?: Record<string, string>
  style?: CSSProperties
}) {
  const name = formatResetClock(resetsAt, nowWall)
  return (
    <span
      // No separate `aria-label` — `role="img"` gets its name from `title`, and if the same text were also given as `aria-label`,
      //   the `title` unused for the name would attach as a description and the same words would be read twice (accname).
      {...(measure ? {} : { ...attrs, role: 'img', title: name })}
      style={{ display: 'inline-flex', alignItems: 'center', gap: RESET_ICON_GAP, color: 'var(--text-muted)', ...style }}
    >
      <Hourglass aria-hidden="true" className="size-3" />
      {formatResetAt(resetsAt, nowWall)}
    </span>
  )
}

/**
 * Bar — drawn only when there is a value (with no value there is no bar at all — it is not drawn as 0%, R21). `meter` = exposed to assistive tech
 * as a bar (D14 — popup). Inside the summary button the button name stands in, so it is hidden.
 * ★Color only by tokens★ — in e-ink the same tokens resolve to hatching · black fill · thick border (R11). `cell` = grid cell position · width (stages 1·2 fill
 * the cell width).
 */
function Bar({
  view,
  label,
  r,
  meter,
  measure,
  cell,
}: {
  view: VendorView
  label: string
  r: ValueReading
  meter: boolean
  measure: boolean
  cell?: CSSProperties
}) {
  const a11y = meter
    ? {
        role: 'meter',
        'aria-valuemin': 0,
        'aria-valuemax': 100,
        'aria-valuenow': r.visible,
        'aria-valuetext': t('usage.meterValueText', { pct: String(r.visible) }),
        'aria-label': t('usage.meterLabel', { vendor: view.name, window: label }),
      }
    : { 'aria-hidden': true }
  return (
    <span
      {...a11y}
      {...(measure ? {} : { 'data-usage-bar': '' })}
      style={{
        flex: 'none',
        display: 'inline-block',
        width: '4em',
        height: '0.6em',
        boxSizing: 'border-box',
        overflow: 'hidden',
        borderRadius: '2px',
        background: 'var(--usage-track)',
        border: r.level === 'danger' ? 'var(--usage-danger-border)' : 'var(--usage-bar-border)',
        ...cell,
      }}
    >
      <span
        data-usage-fill=""
        style={{ display: 'block', height: '100%', width: `${r.visible}%`, background: `var(--usage-${r.level}-fill)` }}
      />
    </span>
  )
}

/**
 * Remaining % text. `bare` = stage 3 where only the number remains (without 「%」, expiry is a short phrase). An old value dims only the number — the age
 * phrase is only in the popup (R32 · user decision 2026-09-27). ★No symbol is attached below 20% either★ — the cue outside color is the e-ink bar
 * shape (user decision 2026-09-29 — R10 amended).
 */
function ValueText({
  w,
  bare,
  dimStale,
  measure,
}: {
  w: WindowView
  bare: boolean
  dimStale: boolean
  measure: boolean
}) {
  const r = w.reading
  const dataValue = measure ? {} : { 'data-usage-value': '' }
  if (r.kind === 'expired') {
    return (
      <span {...dataValue} {...(measure ? {} : { 'data-usage-expired': '' })} style={{ color: 'var(--text-muted)' }}>
        {bare ? t('usage.resetWaitingShort') : t('usage.resetWaiting')}
      </span>
    )
  }
  if (r.kind === 'none') {
    return (
      <span {...dataValue} style={{ color: 'var(--usage-none)' }}>
        —
      </span>
    )
  }
  const dim = dimStale && r.stale
  return (
    <span
      {...dataValue}
      {...(dim && !measure
        ? { 'data-usage-stale': '', title: t('usage.stale', { minutes: String(STALE_AFTER_SECS / 60) }) }
        : {})}
      data-usage-level={measure ? undefined : r.level}
      style={{ color: `var(--usage-${r.level})`, opacity: dim ? 0.5 : undefined }}
    >
      {`${r.visible}${bare ? '' : '%'}`}
    </span>
  )
}

// ── Popup ──────────────────────────────────────────────────────────────────────────────────

/**
 * Opening downward comes first, and if it does not fit it flips upward from the summary's top edge — flipping from the summary's bottom edge would make the popup cover the
 * summary. If it fits in neither down nor up, it falls back to the same push as the slot right-click menu, and ★then the popup may partly cover the summary★
 * (keeping it on screen comes first). Horizontally it is exactly that menu's rule (R7).
 */
function placePopup(anchor: Anchor, w: number, h: number, vw: number, vh: number): { top: number; left: number } {
  const pushed = clampMenuPosition(anchor.left, anchor.bottom, w, h, vw, vh)
  const below = anchor.bottom + ANCHOR_GAP
  if (below + h <= vh) return { top: below, left: pushed.left }
  const above = anchor.top - ANCHOR_GAP - h
  if (above >= 0) return { top: above, left: pushed.left }
  return pushed
}

// ADR-0261: the popup's two company areas are one table — columns are set on one grid and the company area · window rows inherit those columns (`subgrid`). If each
//   row were laid out separately, a long name (`gpt-reserve`) would push only that row's bar, and the % digit count would push only that row's hourglass.
//   Columns = name | bar | remaining % | reset time | remaining time 「(… 뒤)」 (… left). The company head · link row span all columns.
const POPUP_TABLE_STYLE: CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(5, auto)',
  // So that when the popup is wider than the table (a long status line) the `auto` columns do not share the spare width and stretch — if they stretched the gap between bar and % would open up.
  justifyContent: 'start',
  columnGap: '0.5em',
  // Between company areas — the same as the gap between the popup's other lumps.
  rowGap: '8px',
  whiteSpace: 'nowrap',
  fontVariantNumeric: 'tabular-nums',
}
const POPUP_SUBGRID: CSSProperties = { display: 'grid', gridTemplateColumns: 'subgrid', gridColumn: '1 / -1' }
const POPUP_FULL_ROW: CSSProperties = { gridColumn: '1 / -1' }
// Pin the window row height — a row with mixed fonts (Hangul fallback font · Latin name) has a different content height per row, so row spacing becomes uneven.
const POPUP_WINDOW_ROW_HEIGHT = '1.5em'

// ADR-0259: the popup does not call a query — the only way to fetch anew inside the popup is the per-company ⟳ (the reason for no query on open = the summary
//   button onClick).
function UsagePopup({
  summaryRef,
  views,
  nowWall,
  ownerRef,
  shown,
  onToggle,
  onRefresh,
  onClose,
}: {
  /** The summary button the popup attaches to. */
  summaryRef: RefObject<HTMLElement | null>
  views: VendorView[]
  nowWall: number
  /** The summary row that opened the popup (summary button · ⟳) — presses and focus inside it do not count as outside the popup. */
  ownerRef: RefObject<HTMLDivElement | null>
  shown: Record<AgentBackendKind, boolean>
  onToggle: (vendor: AgentBackendKind) => void
  /** Refresh one company — the popup stays open. */
  onRefresh: (vendor: AgentBackendKind) => void
  onClose: (restoreFocus: boolean) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null)

  // Measured on every draw of the popup and summary, but stops if the position is the same — the popup height varies with values, and if a company is enabled while open the summary
  //   grid gains a row and keeping the summary rectangle of the moment it opened would make the popup cover that row. The summary's size changes only when this popup's parent redraws
  //   (content · stage · slot size are received as state by the parent's measurement observer) — so it is not observed separately. Only a position shift
  //   without a size change (swapping same-size slots etc.) is not measured again.
  useLayoutEffect(() => {
    const popup = ref.current
    const summary = summaryRef.current
    if (!popup || !summary) return
    const { width, height } = popup.getBoundingClientRect()
    const next = placePopup(summary.getBoundingClientRect(), width, height, window.innerWidth, window.innerHeight)
    setPos(prev => (prev !== null && prev.top === next.top && prev.left === next.left ? prev : next))
  })

  // ★Focus is moved only after the position is set★ — the first commit is before measurement so it is `visibility:hidden`, and Chromium refuses focus()
  //   on an element that is not visible (then Esc·Tab would not reach the popup).
  const positioned = pos !== null
  useEffect(() => {
    if (positioned) ref.current?.focus()
  }, [positioned])

  useEffect(() => {
    const inPopup = (node: Node | null) => node !== null && (ref.current?.contains(node) ?? false)
    const inOwner = (node: Node | null) => node !== null && (ownerRef.current?.contains(node) ?? false)
    // Esc only when focus is in the popup · summary row — it does not intercept Esc elsewhere. ★It closes when focus is nowhere (body·null) too★ —
    //   if the focused element inside the popup goes away (redrawn and gone etc.) focus drops to body, and blocking then leaves the mouse as the only way to
    //   close. When focus went to another element, focusin below has already closed the popup. Focus that was on the summary row (⟳ etc.) is left in place.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      const active = document.activeElement
      if (active === null || active === document.body || inPopup(active)) onClose(true)
      else if (inOwner(active)) onClose(false)
    }
    // A press on the summary row does not close — the summary button closes by its own click (closing here too would make the click reopen it at once), and ⟳
    //   refreshes with the popup left open (so one can press it while looking at the detail).
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (!inPopup(target) && !inOwner(target)) onClose(false)
    }
    // Closes when focus goes outside, but the focus is left where it went (it is not taken back).
    const onFocusIn = (e: FocusEvent) => {
      const target = e.target as Node
      if (!inPopup(target) && !inOwner(target)) onClose(false)
    }
    document.addEventListener('keydown', onKey)
    // Capture phase — a press on a window-drag spot does not reach the bubble phase (`layout/titleRow/dragRegion.ts`). That press does not move focus either, so
    //   focusin below does not come.
    document.addEventListener('mousedown', onDown, true)
    document.addEventListener('focusin', onFocusIn)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('mousedown', onDown, true)
      document.removeEventListener('focusin', onFocusIn)
    }
  }, [onClose, ownerRef])

  const troubled = views.filter(v => v.line !== null)
  return (
    <div
      ref={ref}
      role="dialog"
      aria-label={t('usage.popupLabel')}
      data-usage-popup=""
      tabIndex={-1}
      style={{
        position: 'fixed',
        top: pos?.top ?? 0,
        left: pos?.left ?? 0,
        visibility: pos ? 'visible' : 'hidden',
        zIndex: 1000,
        display: 'flex',
        flexDirection: 'column',
        gap: '8px',
        minWidth: '220px',
        maxWidth: 'min(90vw, 32rem)',
        padding: '8px 10px',
        background: 'var(--bg-secondary)',
        border: '1px solid var(--border)',
        borderRadius: '4px',
        boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
        color: 'var(--text)',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
        outline: 'none',
        cursor: 'default',
      }}
    >
      {troubled.length > 0 && (
        // The canonical place for the status sentence (R31 — touch has no hover). It is the same string as the badge.
        <div data-usage-status-lines="" style={{ display: 'flex', flexDirection: 'column', gap: '2px' }}>
          {troubled.map(view => (
            <div key={view.vendor} data-usage-status-line={view.vendor} style={{ color: 'var(--usage-badge)' }}>
              <span data-usage-status-text="">{view.line}</span>
            </div>
          ))}
        </div>
      )}
      {views.length > 0 && (
        <div data-usage-popup-table="" style={POPUP_TABLE_STYLE}>
          {views.map(view => (
            <VendorDetail key={view.vendor} view={view} nowWall={nowWall} onRefresh={onRefresh} />
          ))}
        </div>
      )}
      <ShowToggles shown={shown} onToggle={onToggle} divided={views.length > 0} />
    </div>
  )
}

// ADR-0258: the human UI for turning on and off is this one place — the slot right-click menu has no usage item.
/**
 * The display toggle row at the bottom of the popup — carries both enabled and disabled companies (this is the place to turn a disabled company back on). Check state draws the slot content
 * as it is — a press goes through the command, and when the layout returns the changed content the check follows.
 */
function ShowToggles({
  shown,
  onToggle,
  divided,
}: {
  shown: Record<AgentBackendKind, boolean>
  onToggle: (vendor: AgentBackendKind) => void
  divided: boolean
}) {
  const headId = useId()
  return (
    <div
      role="group"
      aria-labelledby={headId}
      data-usage-show-toggles=""
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        alignItems: 'center',
        columnGap: '0.8em',
        rowGap: '2px',
        ...(divided ? { borderTop: '1px solid var(--border)', paddingTop: '6px' } : {}),
      }}
    >
      <span id={headId} style={{ color: 'var(--text-muted)' }}>
        {t('usage.showOnSlot')}
      </span>
      {USAGE_VENDORS.map(vendor => (
        <label
          key={vendor}
          style={{ display: 'inline-flex', alignItems: 'center', gap: '0.3em', whiteSpace: 'nowrap', cursor: 'pointer' }}
        >
          <input
            type="checkbox"
            data-usage-show={vendor}
            checked={shown[vendor]}
            onChange={() => onToggle(vendor)}
            style={{ margin: 0, cursor: 'pointer' }}
          />
          {vendorName(vendor)}
        </label>
      ))}
    </div>
  )
}

/**
 * Detail of one company — head (name · plan (only when present) · age · that company's own ⟳) · absolute reset time per window · per-model windows (only those with a value
 * — D8) · link. The age is once in the head and is not repeated on every window row (ADR-0259 decision 1). The window row columns are the popup table's columns (ADR-0261).
 */
function VendorDetail({
  view,
  nowWall,
  onRefresh,
}: {
  view: VendorView
  nowWall: number
  onRefresh: (vendor: AgentBackendKind) => void
}) {
  const snapshot = view.entry?.snapshot
  // ADR-0259: the head's age = the oldest age among the two windows, 5-hour and weekly (`view.windows`) — do not include the per-model windows added below
  //   (user decision). The pickup does not carry per-model windows, so only that window ages by hours while the conversation continues, and including it would make the head look
  //   stale (amber) even though both windows are fresh. If neither of the two windows has a value, no age is written. Amber looks at this value too.
  const age = oldestAgeSecs(view.windows.map(w => w.reading))
  const stale = age !== null && isStale(age)
  const rows: WindowView[] = [...view.windows]
  if (snapshot && snapshot.state.kind !== 'Unavailable') {
    for (const scoped of snapshot.model_scoped) {
      if (scoped.window.used_pct === null) continue
      rows.push({
        key: `model:${scoped.label}`,
        label: scoped.label,
        reading: readWindow(scoped.window, view.elapsed, nowWall),
      })
    }
  }
  const url = USAGE_PAGE_URL[view.vendor]
  return (
    <section data-usage-popup-vendor={view.vendor} style={{ ...POPUP_SUBGRID, rowGap: '3px' }}>
      <div style={{ ...POPUP_FULL_ROW, display: 'flex', alignItems: 'center', gap: '0.6em' }}>
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: '0.35em', fontWeight: 600 }}>
          {/* The name text is right next to it so the icon is decoration — so assistive tech does not read the name twice. */}
          <VendorIcon vendor={view.vendor} label={null} data-usage-icon={view.vendor} />
          <span>{view.name}</span>
        </span>
        {snapshot?.plan != null && (
          <span data-usage-plan="" style={{ color: 'var(--text-muted)' }}>
            {t('usage.plan', { plan: snapshot.plan })}
          </span>
        )}
        {snapshot?.plan != null && age !== null && (
          <span aria-hidden="true" style={{ color: 'var(--text-muted)' }}>
            {t('usage.headSeparator')}
          </span>
        )}
        {/* ADR-0259: age and ⟳ are kept together — only a ⟳ right next to the time reads as refresh (user decision). Do not attach a clock icon
            to the age (「아이콘 2개 나와서 난잡」 (two icons show up, cluttered)). */}
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: '0.35em' }}>
          {age !== null && (
            <span
              data-usage-age=""
              {...(stale
                ? { 'data-usage-stale': '', title: t('usage.stale', { minutes: String(STALE_AFTER_SECS / 60) }) }
                : {})}
              style={{ color: stale ? 'var(--usage-stale)' : 'var(--text-muted)' }}
            >
              {formatAge(age)}
            </span>
          )}
          <RefreshIconButton
            label={t('usage.refreshVendor', { vendor: view.name })}
            blocked={view.rejected}
            busy={view.refreshing}
            onRefresh={() => onRefresh(view.vendor)}
            attrs={{
              'data-usage-vendor-refresh': view.vendor,
              ...(view.refreshing ? { 'data-usage-refreshing': view.vendor } : {}),
            }}
          />
        </span>
      </div>
      {rows.map(row => (
        <PopupWindowRow key={row.key} view={view} row={row} nowWall={nowWall} />
      ))}
      <div style={POPUP_FULL_ROW}>
        <button
          type="button"
          data-usage-link={view.vendor}
          title={t('usage.linkTitle', { vendor: view.name, url })}
          onClick={() => {
            openUrl(url).catch((err: unknown) => console.warn(`[UsageSlot] openUrl(${url}) 실패:`, err))
          }}
          style={{
            padding: 0,
            border: 0,
            background: 'transparent',
            color: 'var(--accent)',
            font: 'inherit',
            cursor: 'pointer',
            textDecoration: 'underline',
          }}
        >
          {t('usage.link')}
        </button>
      </div>
    </section>
  )
}

/**
 * One window row of the popup — the table's five cells are position-pinned (even with no bar because there is no value or no reset time the later cells are not pulled
 * forward). If expired, one 「리셋됨 — 갱신 대기」 (reset — waiting for refresh) cell covers from the bar to the end.
 */
function PopupWindowRow({ view, row, nowWall }: { view: VendorView; row: WindowView; nowWall: number }) {
  const r = row.reading
  const resetsAt = r.kind === 'expired' ? null : r.resetsAt
  const resetInSecs = r.kind === 'expired' ? null : r.resetInSecs
  return (
    <div
      data-usage-vendor={view.vendor}
      data-usage-window={row.key}
      style={{ ...POPUP_SUBGRID, alignItems: 'center', height: POPUP_WINDOW_ROW_HEIGHT }}
    >
      {/* Left indentation is put inside the name cell — row-side margin gets mixed into the width of subgrid's first column. */}
      <span style={{ gridColumn: 1, paddingLeft: '0.75em', color: 'var(--text-muted)' }}>{row.label}</span>
      {r.kind === 'expired' ? (
        <span style={{ gridColumn: '2 / -1' }}>
          <ValueText w={row} bare={false} dimStale={false} measure={false} />
        </span>
      ) : (
        <>
          {r.kind === 'value' && (
            <Bar view={view} label={row.label} r={r} meter measure={false} cell={{ gridColumn: 2 }} />
          )}
          {/* The popup is not dimmed — staleness shows as the age in the company head turning amber (R32 · ADR-0259). */}
          <span style={{ gridColumn: 3, textAlign: 'right' }}>
            <ValueText w={row} bare={false} dimStale={false} measure={false} />
          </span>
        </>
      )}
      {resetsAt !== null && resetInSecs !== null && (
        // The remaining time is put outside the name — the marker that carries the name (「리셋 11:29」) hides the text inside from assistive tech, so if put inside
        //   the remaining time would not be read.
        // The whitespace character between the two is for the side that reads DOM text (R27 — LLM·cdp). The visible gap is drawn by the gap between cells — a text
        //   of only whitespace inside the grid is not drawn. The wrapping `span` is `display: contents` so the two cells become cells of the row directly.
        <span data-usage-reset-at="" style={{ display: 'contents' }}>
          <ResetMark resetsAt={resetsAt} nowWall={nowWall} measure={false} style={{ gridColumn: 4 }} />{' '}
          <span data-usage-reset-in="" style={{ gridColumn: 5, color: 'var(--text-muted)' }}>
            {t('usage.resetIn', { duration: formatDuration(resetInSecs) })}
          </span>
        </span>
      )}
    </div>
  )
}
