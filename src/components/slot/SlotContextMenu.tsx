// ★Role★: human click, palette, keybinding and LLM all pass through the same command and id (§5 single control surface, ADR-0055); the menu never calls the store directly (ADR-0064 invariant).
// ★One menu component★ (ADR-0064 §5): content (PresetPalette/AgentList) no longer owns its own pane menu.

import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { CSSProperties, SyntheticEvent } from 'react'

import { fireAndForget } from '../../commands/dispatch'
import type { CommandArgs } from '../../commands/registry'
import type { ResolvedSlotMenuItem, SlotMenuCtx } from '../../commands/slotMenu'
import { useLanguage } from '../../i18n/react'

/** Keeps the menu from sitting flush against the window edge. */
const MENU_MARGIN = 4

/**
 * Gap between the anchor (opening point) and the menu rectangle — ★larger than the double-click recognition radius★ (ADR-0143 decision 4). Windows treats a second click inside a rectangle around the first as a double-click; its width/height are `SM_CXDOUBLECLK`/`SM_CYDOUBLECLK` (default 4px), so the radius is 2px — 4x here covers a user-doubled tolerance (registry `DoubleClickWidth`).
 * With gap 0 the anchor would sit flush against the menu corner, and double-clicking an empty slot would land the second click on an item and run it — in a lower slot the menu flips upward so `Close` ends up under the cursor, and the layout is not on disk so it cannot be undone.
 */
export const ANCHOR_GAP = 8

/**
 * Prevents the menu being cut off at the window edge (Bug1). The anchor stays `ANCHOR_GAP` outside the menu whichever way it is placed.
 */
export function clampMenuPosition(
  x: number,
  y: number,
  w: number,
  h: number,
  vw: number,
  vh: number,
): { top: number; left: number } {
  return { top: placeAlongAxis(y, h, vh), left: placeAlongAxis(x, w, vw) }
}

// ★Flip, do not push★: pushing moves the anchor *inside* the menu and puts an item under the cursor; an empty slot opens the same menu on left-click (ADR-0141·0142), so one double-click would run the item under the cursor (Close removes the slot). Flipping keeps the anchor outside in any direction.
// ★ANCHOR_GAP is added in both directions (ADR-0143 decision 4)★: merely touching the corner lets a double-click's second click land on an item — the gap is applied in this one helper so both axes and directions share the rule.
// Only when it still leaves the screen after flipping (menu larger than the viewport) does it fall back to pushing — "stays on screen" then beats the anchor rule.
function placeAlongAxis(anchor: number, size: number, viewport: number): number {
  const forward = anchor + ANCHOR_GAP
  if (forward + size <= viewport) return forward
  const flipped = anchor - ANCHOR_GAP - size
  if (flipped >= 0) return flipped
  return Math.max(MENU_MARGIN, Math.min(anchor, viewport - size - MENU_MARGIN))
}

export function flyoutPosition(
  anchorLeft: number,
  anchorRight: number,
  anchorTop: number,
  fw: number,
  fh: number,
  vw: number,
  vh: number,
): { top: number; left: number } {
  const overflowRight = anchorRight + fw > vw
  const fitsLeft = anchorLeft - fw >= MENU_MARGIN
  const left = overflowRight && fitsLeft ? anchorLeft - fw : anchorRight
  // Extreme-case defense for when it still overflows after flipping.
  const clampedLeft = Math.max(MENU_MARGIN, Math.min(left, Math.max(MENU_MARGIN, vw - fw - MENU_MARGIN)))
  const top =
    anchorTop + fh > vh ? Math.max(MENU_MARGIN, Math.min(anchorTop, vh - fh - MENU_MARGIN)) : anchorTop
  return { top, left: clampedLeft }
}

interface SlotContextMenuProps {
  x: number
  y: number
  /** Items already sorted by group·order and registry-resolved (buildSlotMenu output). */
  items: ResolvedSlotMenuItem[]
  ctx: SlotMenuCtx
  onClose: () => void
}

export default function SlotContextMenu({ x, y, items, ctx, onClose }: SlotContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ top: number; left: number }>({ top: y, left: x })
  // ADR-0373: a language switch changes the titles and so the menu size — measure again.
  const lang = useLanguage()

  // ★Position correction before paint (Bug1)★: useLayoutEffect avoids a visual jump (measuring needs the mount, so up to one frame of re-layout is accepted).
  useLayoutEffect(() => {
    if (!ref.current) return
    const rect = ref.current.getBoundingClientRect()
    setPos(clampMenuPosition(x, y, rect.width, rect.height, window.innerWidth, window.innerHeight))
    // items.length is in deps: if the item count changes while the menu stays open at the same x/y, the height changes and needs re-measuring (`items` is a new array every render, so length is a stable trigger).
  }, [x, y, items.length, lang])

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    // Capture phase — pressing the window-drag area never reaches the bubble phase (`layout/titleRow/dragRegion.ts`).
    document.addEventListener('mousedown', handler, true)
    return () => document.removeEventListener('mousedown', handler, true)
  }, [onClose])

  // ★Its role is drawing only — there is no keyboard navigation (arrows, Home/End, type-ahead)★ (known gap · this whole menu).
  return (
    <div
      ref={ref}
      role="menu"
      // ADR-0237: while open, the chat area's Esc does not interrupt the turn (marker of `interruptKey.ts`) — this menu does not close on Esc, but an Esc pressed with the menu up must not interrupt the turn behind it.
      data-engram-overlay="1"
      style={{
        position: 'fixed',
        top: pos.top,
        left: pos.left,
        background: 'var(--bg-secondary)',
        border: '1px solid var(--border)',
        borderRadius: '4px',
        zIndex: 1000,
        minWidth: '150px',
        boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
      }}
    >
      {items.map(item => (
        <div key={item.id} role="none">
          {item.separatorBefore && (
            <div role="separator" style={{ height: '1px', background: 'var(--border)', margin: '2px 0' }} />
          )}
          <MenuRow item={item} ctx={ctx} onClose={onClose} />
        </div>
      ))}
    </div>
  )
}

const ROW_STYLE: CSSProperties = { padding: '6px 12px', cursor: 'pointer', color: 'var(--text)' }
function highlightOn(e: SyntheticEvent<HTMLElement>) {
  e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)'
}
function highlightOff(e: SyntheticEvent<HTMLElement>) {
  e.currentTarget.style.background = 'transparent'
}

const DISABLED_ROW_STYLE: CSSProperties = { ...ROW_STYLE, cursor: 'default', opacity: 0.4 }

// ★The execution bag = slot coordinates + slot content only★ — pick fields so that fields added to ctx for menu-state verdicts are not passed on. The caller's identity is not carried either: `__engramCmd` can build the same bag, so the argument value cannot be the caller verdict (`registry.ts` `humanOnly` doc — the closing axis is the caller).
function commandArgs(ctx: SlotMenuCtx): CommandArgs {
  return { viewId: ctx.viewId, slotId: ctx.slotId, agentId: ctx.agentId, content: ctx.content }
}

function runItem(id: string, ctx: SlotMenuCtx, onClose: () => void) {
  // ADR-0064/0055: reuse the palette/keybinding/LLM helper — do not re-implement the sync throw, async reject and thenable-swallow safety net.
  fireAndForget(id, commandArgs(ctx))
  onClose()
}

// ★Called during render, so a throw must not escape★ (fail-loud but crash-free, as slotMenu FIX-1) — without an error boundary one contribution's throw blanks the screen. Verdict failure = off · disabled.
function evaluate(item: ResolvedSlotMenuItem, what: 'checked' | 'enabled', fn: () => boolean): boolean {
  try {
    return fn()
  } catch (err) {
    console.error(`[SlotContextMenu] ${what} verdict failed — drawing "${item.id}" as ${what === 'checked' ? 'off' : 'disabled'}:`, err)
    return false
  }
}

/** One action row — top level and submenu children draw the same thing. */
function LeafRow({ item, ctx, onClose }: { item: ResolvedSlotMenuItem; ctx: SlotMenuCtx; onClose: () => void }) {
  const checkedFn = item.checked
  const enabledFn = item.enabled
  const checked = checkedFn ? evaluate(item, 'checked', () => checkedFn(ctx)) : undefined
  const enabled = enabledFn ? evaluate(item, 'enabled', () => enabledFn(ctx)) : true
  return (
    <div
      data-slot-menu-item={item.id}
      {...(checked === undefined ? { role: 'menuitem' } : { role: 'menuitemcheckbox', 'aria-checked': checked })}
      {...(enabled ? {} : { 'aria-disabled': true, 'data-slot-menu-disabled': '' })}
      style={enabled ? ROW_STYLE : DISABLED_ROW_STYLE}
      onMouseEnter={enabled ? highlightOn : undefined}
      onMouseLeave={enabled ? highlightOff : undefined}
      onClick={e => {
        e.stopPropagation()
        if (enabled) runItem(item.id, ctx, onClose)
      }}
    >
      {checked !== undefined && (
        <span aria-hidden="true" style={{ display: 'inline-block', width: '1.2em' }}>
          {checked ? '✓' : ''}
        </span>
      )}
      {item.title}
    </div>
  )
}

/** Children run through the same dispatch path as a leaf (§5 — a submenu is presentation only). */
function MenuRow({ item, ctx, onClose }: { item: ResolvedSlotMenuItem; ctx: SlotMenuCtx; onClose: () => void }) {
  const isContainer = !!item.children && item.children.length > 0
  const rowRef = useRef<HTMLDivElement>(null)
  const flyoutRef = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const [flyoutPos, setFlyoutPos] = useState<{ top: number; left: number } | null>(null)
  // ADR-0373: the flyout's size follows its titles' language — measure again on a switch.
  const lang = useLanguage()

  useLayoutEffect(() => {
    if (!isContainer || !open || !rowRef.current || !flyoutRef.current) return
    const anchor = rowRef.current.getBoundingClientRect()
    const fly = flyoutRef.current.getBoundingClientRect()
    setFlyoutPos(
      flyoutPosition(anchor.left, anchor.right, anchor.top, fly.width, fly.height, window.innerWidth, window.innerHeight),
    )
  }, [isContainer, open, lang])

  if (!isContainer) return <LeafRow item={item} ctx={ctx} onClose={onClose} />

  return (
    <div
      ref={rowRef}
      // cdp/tests identify the container by data-attr.
      data-slot-menu-container={item.id}
      role="none"
      style={{ position: 'relative' }}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => {
        setOpen(false)
        setFlyoutPos(null)
      }}
    >
      <div
        role="menuitem"
        aria-haspopup="menu"
        aria-expanded={open}
        style={{ ...ROW_STYLE, display: 'flex', justifyContent: 'space-between', gap: '12px', alignItems: 'center' }}
        tabIndex={0}
        onFocus={() => setOpen(true)}
        onMouseEnter={highlightOn}
        onMouseLeave={highlightOff}
      >
        <span>{item.title}</span>
        <span style={{ opacity: 0.6 }}>▶</span>
      </div>
      {open && (
        <div
          ref={flyoutRef}
          data-slot-menu-flyout={item.id}
          role="menu"
          style={{
            position: 'fixed',
            top: flyoutPos?.top ?? 0,
            left: flyoutPos?.left ?? 0,
            // Hidden before measuring (flyoutPos=null) to hide the jump (shown after clamping).
            visibility: flyoutPos ? 'visible' : 'hidden',
            background: 'var(--bg-secondary)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            zIndex: 1001,
            minWidth: '150px',
            boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
          }}
        >
          {item.children!.map(child => (
            <LeafRow key={child.id} item={child} ctx={ctx} onClose={onClose} />
          ))}
        </div>
      )}
    </div>
  )
}
