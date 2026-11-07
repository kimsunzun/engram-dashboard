// TRD S26-window-chrome §7-4: three window buttons (minimize · maximize/restore · third). ★Pressing calls the registry `window.*` command — it does not call the Tauri window
//   API★ (AC10 — `src/__tests__/noWindowControl.test.ts`). A human press and the LLM drive the same command (CLAUDE.md
//   「LLM-우선 제어」). The third = `window.close` — for main the shell hides all windows, for a popout it closes (TRD §5-4).
// In a build with the snap layout overlay (TRD §8-4 — shell feature `snap-overlay` · off by default) on, the maximize button rectangle is reported to the shell, and since the
//   OS child window covering that spot takes the cursor, hover is drawn with the value the shell returns (`data-snap-hover`) (`snapOverlay.ts`). In that
//   build a mouse press is passed to the OS by the overlay (the same path as a double press on the title row) and a keyboard press is received by this button.

import { useRef, type CSSProperties, type ReactNode, type Ref } from 'react'
import { Copy, EyeOff, Minus, Square, X } from 'lucide-react'

import { fireAndForget } from '../../../commands/dispatch'
import { t } from '../../../i18n'
import { CAPTION_BUTTON_W } from './metrics'
import { useSnapOverlay, type SnapOverlayIpc } from './snapOverlay'

interface CaptionButtonsProps {
  label: string
  /** Is it the main window — the third button is 「모든 창 숨기기」 (eye cover · light hover), otherwise 「창 닫기」 (× · red hover). */
  main: boolean
  maximized: boolean
  /** Is it a build where the shell turned on the snap layout overlay (window fact `caption.snap_overlay`) — the maximize button rectangle is reported only when true. Default `false`. */
  snapOverlay?: boolean
  /** Only tests swap it in — the Tauri contact point for overlay reporting · hover listening. */
  snapIpc?: SnapOverlayIpc
}

export default function CaptionButtons({ label, main, maximized, snapOverlay = false, snapIpc }: CaptionButtonsProps) {
  const target = { window: label }
  const maxRef = useRef<HTMLButtonElement>(null)
  const snapHover = useSnapOverlay(snapOverlay, maxRef, snapIpc)
  return (
    <div data-testid="caption-buttons" style={{ display: 'flex', flex: '0 0 auto', alignSelf: 'stretch', borderBottom: '1px solid var(--tab-bar-edge)' }}>
      <CaptionButton testId="caption-minimize" name={t('window.captionMinimize')} onPress={() => fireAndForget('window.minimize', target)}>
        <Minus aria-hidden size={GLYPH} strokeWidth={GLYPH_STROKE} />
      </CaptionButton>
      {/* Maximize · restore is one element — the button the overlay measures stays the same even when the state changes. */}
      <CaptionButton
        ref={maxRef}
        testId="caption-maximize"
        name={maximized ? t('window.captionRestore') : t('window.captionMaximize')}
        snapHover={snapHover}
        onPress={() => fireAndForget(maximized ? 'window.unmaximize' : 'window.maximize', target)}
      >
        {maximized ? <Copy aria-hidden size={GLYPH_BOX} strokeWidth={GLYPH_STROKE} /> : <Square aria-hidden size={GLYPH_BOX} strokeWidth={GLYPH_STROKE} />}
      </CaptionButton>
      {main ? (
        <CaptionButton testId="caption-close" name={t('window.captionHideAll')} onPress={() => fireAndForget('window.close', target)}>
          <EyeOff aria-hidden size={GLYPH} strokeWidth={GLYPH_STROKE} />
        </CaptionButton>
      ) : (
        <CaptionButton testId="caption-close" name={t('window.captionClose')} danger onPress={() => fireAndForget('window.close', target)}>
          <X aria-hidden size={GLYPH} strokeWidth={GLYPH_STROKE} />
        </CaptionButton>
      )}
    </div>
  )
}

function CaptionButton({
  ref,
  testId,
  name,
  danger = false,
  snapHover,
  onPress,
  children,
}: {
  ref?: Ref<HTMLButtonElement>
  testId: string
  name: string
  danger?: boolean
  /** Snap overlay hover (maximize button only) — `undefined` = the attribute is not attached. */
  snapHover?: boolean
  onPress: () => void
  children: ReactNode
}) {
  return (
    <button
      ref={ref}
      type="button"
      data-testid={testId}
      data-snap-hover={snapHover === undefined ? undefined : snapHover ? 'true' : 'false'}
      aria-label={name}
      title={name}
      onClick={onPress}
      className={danger ? DANGER_CLASS : BUTTON_CLASS}
      style={BUTTON_STYLE}
    >
      {children}
    </button>
  )
}

// Glyph size = the value of the spike (2026-10-24) — not compared with the Windows 11 glyphs (untested · S11).
const GLYPH = 14
const GLYPH_BOX = 12
const GLYPH_STROKE = 1.5

// ★Keep class names as whole strings★ — Tailwind scans source text to generate classes, so joining fragments produces none. Do not put color ·
//   hover background in inline style — inline beats the hover class and the hover disappears. The focus border is drawn inside —
//   the rightmost button touches the window edge and an outer border is clipped. The `data-snap-hover` hover is attached to the maximize button only (head above).
const BUTTON_CLASS =
  'bg-transparent text-[color:var(--caption-fg)] hover:bg-[var(--caption-hover-bg)] data-[snap-hover=true]:bg-[var(--caption-hover-bg)] focus-visible:outline focus-visible:outline-1 focus-visible:outline-accent focus-visible:-outline-offset-2'
const DANGER_CLASS =
  'bg-transparent text-[color:var(--caption-fg)] hover:bg-[var(--caption-close-hover-bg)] hover:text-[color:var(--caption-close-hover-fg)] focus-visible:outline focus-visible:outline-1 focus-visible:outline-accent focus-visible:-outline-offset-2'

// Windows caption buttons use an arrow cursor.
const BUTTON_STYLE: CSSProperties = {
  width: CAPTION_BUTTON_W,
  padding: 0,
  border: 'none',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  cursor: 'default',
}
