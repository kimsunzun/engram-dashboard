// CodexMascot — the codex-side marker drawn in the JSON-mode empty state. A sibling of ClaudeMascot that follows its render technique (dot grid →
//   horizontal run merge → viewBox in cell units) as is.
//
// ★Why the file is split instead of editing ClaudeMascot★: that one has a single call site, so touching one line there changes the claude
//   panel too. The two markers point to different products, so each owns its shape and color.
//
// ★Do not copy a vendor logo★: instead of stamping the trademark as is, it uses the prompt symbol (`>_`). The selection criterion is ADR-0062's
//   「모양이 1차 신호」 (shape is the primary signal) — it does not overlap Clawd's blocky silhouette, so they stay distinct even on e-ink (color neutralized).
//
// ★The grid size is 26×17, the same as ClaudeMascot★ — the footprint is matched so the empty-state group does not
//   move up or down depending on the backend. Only the shape differs; cell size and alignment rules are identical to the sibling.

/**
 * Marker dot layout. `█` = filled cell, any other character (space) = empty cell.
 *
 * Shape = the prompt `>_`. Do not break these two when editing:
 * the chevron is symmetric top to bottom with a single vertex on the right (with two it reads as a diamond, not an arrow) ·
 * the underscore starts on the same row as the chevron's lower tail and extends right (raised, it looks like an equals sign).
 */
const MASCOT_ROWS = [
  '   ███                    ',
  '    ███                   ',
  '     ███                  ',
  '      ███                 ',
  '       ███                ',
  '        ███               ',
  '         ███              ',
  '          ███             ',
  '           ███            ',
  '          ███             ',
  '         ███              ',
  '        ███               ',
  '       ███                ',
  '      ███                 ',
  '     ███       ██████████ ',
  '    ███        ██████████ ',
  '   ███         ██████████ ',
]

/**
 * codex marker color (teal) — does not follow the theme accent color (same discipline as the sibling ClaudeMascot).
 * ★The same value as `--rich-brand` in `richBranding.css`★ — the marker and the panel tone come from one color.
 */
const MASCOT_COLOR = '#10a37f'

/** Side of one cell (px). Same value as the sibling = same display size. */
const MASCOT_CELL_PX = 6

const FILLED = '█'

const MASCOT_COLS = Math.max(...MASCOT_ROWS.map(row => row.length))

/** List of horizontally adjacent cells merged into one rect (same reason as the sibling — emitting a node per cell would grow the DOM). */
const MASCOT_RUNS: { x: number; y: number; w: number }[] = MASCOT_ROWS.flatMap((row, y) => {
  const runs: { x: number; y: number; w: number }[] = []
  let x = 0
  while (x < MASCOT_COLS) {
    if (row[x] !== FILLED) {
      x += 1
      continue
    }
    let w = 1
    while (x + w < MASCOT_COLS && row[x + w] === FILLED) w += 1
    runs.push({ x, y, w })
    x += w
  }
  return runs
})

export function CodexMascot() {
  const rows = MASCOT_ROWS.length
  return (
    <svg
      data-rich-mascot="1" // Same surface as the sibling — 「빈 상태 표식이 떴나」 (did the empty-state marker show up?) is checked by this attribute regardless of backend.
      data-rich-brand="codex" // Which backend it is (a stable token — richBranding.ts is where it is defined).
      aria-hidden
      width={MASCOT_COLS * MASCOT_CELL_PX}
      height={rows * MASCOT_CELL_PX}
      viewBox={`0 0 ${MASCOT_COLS} ${rows}`}
      shapeRendering="crispEdges"
    >
      {MASCOT_RUNS.map(run => (
        <rect
          key={`${run.y}:${run.x}`}
          x={run.x}
          y={run.y}
          width={run.w}
          height={1}
          fill={MASCOT_COLOR}
        />
      ))}
    </svg>
  )
}
