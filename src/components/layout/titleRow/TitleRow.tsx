// TRD S26-window-chrome §7-1 · §7-2 · §7-5: the title row at the very top of the window = tab bar → empty draggable area → number badge → window buttons. The web only draws —
//   window state is read as window facts (`dock/windowFrame` — pushed by the shell), and changes are made only by registry commands (`CaptionButtons`).
// ★Tabs shrink first★ — only the tab bar shrinks (`TabBar` root `flex: 1 1 0px`) and the rest is fixed. So when the row narrows, the tab bar's measured width shrinks and
//   tabs shrink and hide by the Chrome rules (`tabStrip/layout.ts` · R13).
// Draggable area = this row itself (the top margin of normal state · the number badge position — the badge does not receive presses) · the empty part of the tab strip (`TabBar`) · the empty draggable area
//   (`dragRegion.ts`). Dimming = the text tokens under `data-window-active` (`styles/theme.css`).
// ★Waiting mode (prewarmed window — ADR-0299) also draws this row · only the number badge and window buttons are left out (`standby` · `number` = `null`)★ — it is outside the shell window model,
//   so there is no number and window commands cannot find that window. If the row were removed entirely, the tab bar would be remounted on adoption (the tab bar row node changes —
//   the empty frame clause of `WindowLayout`), and the canvas size during waiting that the shell copies into the model on adoption (`src-tauri/src/spare/mod.rs`) would be off from the screen
//   after adoption by the top margin. Loading · boot failure screens also draw this row — even if there is no number yet, there are window buttons · a draggable area (with no OS
//   frame, this row is the only place to move and close the window · `WindowLayout`).
// The minimum window size (§7-7) is computed from what this row drew — ★sent only after the window number is received★ (waiting · loading · boot failure do not send).
//   The shell withdraws the open mark set when it opened the window at that window's next report (head of `src-tauri/src/chrome/book.rs`), but for a prewarmed window the report
//   during waiting and the report after adoption are on the same page, so they cannot be told apart by report sequence number — if a report during waiting withdraws the adoption-time mark first, the adopted window is not
//   pulled wider. Widening by the width of a screen without a number (no badge) is the same mismatch.

import {
  useLayoutEffect,
  useRef,
  useSyncExternalStore,
  type CSSProperties,
  type ReactNode,
  type Ref,
} from 'react'

import { MAIN_WINDOW_LABEL } from '../../../store/viewStore'
import { windowFrame, type WindowFrameSource } from '../dock/windowFrame'
import CaptionButtons from './CaptionButtons'
import { dragProps } from './dragRegion'
import { GRAB_MIN_W, minWindowSize, titleRowHeight } from './metrics'
import { reportMinWindowSize } from './minSizeReport'
import NumberMarker from './NumberMarker'

interface TitleRowProps {
  label: string
  /** Visible window number — `null` = not yet (waiting mode · loading · boot failure). The badge is not drawn and the minimum window size is not sent. */
  number: number | null
  /** Waiting mode (the empty frame of a prewarmed window) — window buttons are not drawn. Default `false`. */
  standby?: boolean
  /** Window facts — receiving (`install`) is attached by the tab bar's drag hook (`pullFrame`). Tests swap it in. */
  frame?: WindowFrameSource
  /** Row element — the tab bar's drop decision measures the tab bar zone against this whole row (`UseTabDragSourceOptions.titleRow`). */
  ref?: Ref<HTMLDivElement>
  /** This window's tab bar (`TabBar`). */
  children: ReactNode
}

export default function TitleRow({ label, number, standby = false, frame = windowFrame, ref, children }: TitleRowProps) {
  const facts = useSyncExternalStore(frame.subscribe, frame.current)
  // Before window facts are received = not maximized · front window · draws buttons (the current OS chrome shape — if it changes after receipt, the tab width changes once).
  const maximized = facts?.maximized ?? false
  const active = facts?.focused ?? true
  const buttons = !standby && (facts?.caption.buttons ?? true)
  const leading = facts?.caption.leading_inset ?? 0

  // ★It is a layout effect★ — the report must be handed over before the `WindowLayout` first-screen effect (passive) of the same commit so that the first-screen notice
  //   waits for that report (§7-7). When it re-measures = when the number · buttons · left margin change and when the badge width changes (digits · font · scale — observed). If there is no
  //   number it is not sent (head).
  const markerRef = useRef<HTMLSpanElement>(null)
  useLayoutEffect(() => {
    if (number === null) return
    const measure = (): void =>
      reportMinWindowSize(
        minWindowSize({
          leadingInset: leading,
          markerW: markerRef.current === null ? null : markerRef.current.getBoundingClientRect().width,
          buttons,
        }),
      )
    measure()
    const marker = markerRef.current
    // In environments without an observer such as jsdom, it passes with one measurement.
    if (marker === null || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(measure)
    ro.observe(marker)
    return () => ro.disconnect()
  }, [number, buttons, leading])

  return (
    <div
      ref={ref}
      data-testid="title-row"
      {...dragProps()}
      data-window-active={active ? 'true' : 'false'}
      data-maximized={maximized ? 'true' : 'false'}
      style={{
        display: 'flex',
        alignItems: 'flex-end',
        flexShrink: 0,
        height: titleRowHeight(maximized),
        boxSizing: 'border-box',
        paddingLeft: leading,
        background: 'var(--tab-bar-bg)',
        fontFamily: 'var(--font-ui)',
        userSelect: 'none',
      }}
    >
      {children}
      <div data-testid="title-grab-gap" aria-hidden {...dragProps()} style={GRAB_GAP} />
      {number !== null && <NumberMarker ref={markerRef} n={number} />}
      {buttons && (
        <CaptionButtons label={label} main={label === MAIN_WINDOW_LABEL} maximized={maximized} snapOverlay={facts?.caption.snap_overlay ?? false} />
      )}
    </div>
  )
}

const GRAB_GAP: CSSProperties = {
  flex: `0 0 ${GRAB_MIN_W}px`,
  alignSelf: 'stretch',
  boxSizing: 'border-box',
  borderBottom: '1px solid var(--tab-bar-edge)',
}
