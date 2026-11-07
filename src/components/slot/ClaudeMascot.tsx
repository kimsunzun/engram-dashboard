// ClaudeMascot — pixel-grid mascot drawn in the JSON-mode empty state (ADR-0145).
//
// ★The only places to tune are the three constants below★ — shape (MASCOT_ROWS) · color · cell size. The user looks at the screen and
//   edits the shape, so it is kept apart from the render code.
//
// Why no image asset: it blurs with scaling. Why no text blocks: monospace cells are not
// square, so the character stretches vertically (the alternative ADR-0145 rejected).

/**
 * Mascot dot layout. `█` = filled cell, any other character (space) = empty cell.
 * Even if line lengths differ, the right side is padded to the longest line, so editing the shape only means editing this array.
 *
 * Reference points against the original (Claude Code mascot "Clawd") — do not break these four when editing:
 * a nub sticking out on each side of the body at mid height (with only one side it looks like an arm is missing) ·
 * eyes = two thin vertical bars (thicker ones make a different sticker variant) ·
 * legs = four short blocks (two pairs, with a wide gap between the pairs. Each eye sits above the middle of a pair) ·
 * body width:height ≈ 1.5.
 */
const MASCOT_ROWS = [
  '   ████████████████████   ',
  '   ████████████████████   ',
  '   ████████████████████   ',
  '   ████ ██████████ ████   ',
  '   ████ ██████████ ████   ',
  '   ████ ██████████ ████   ',
  '   ████ ██████████ ████   ',
  '██████████████████████████',
  '██████████████████████████',
  '██████████████████████████',
  '   ████████████████████   ',
  '   ████████████████████   ',
  '   ████████████████████   ',
  '     ██ ██      ██ ██     ',
  '     ██ ██      ██ ██     ',
  '     ██ ██      ██ ██     ',
  '     ██ ██      ██ ██     ',
]

/** Approximates the original body color (terracotta-ish apricot) — does not follow the theme accent color (ADR-0145 user decision). */
const MASCOT_COLOR = '#c97b5c'

/** Side of one cell (px). Same value for width and height = 1:1 square cells. */
const MASCOT_CELL_PX = 6

const FILLED = '█'

const MASCOT_COLS = Math.max(...MASCOT_ROWS.map(row => row.length))

/**
 * List of horizontally adjacent cells merged into one rect. Emitting a node per cell would grow the DOM by the number of grid cells,
 * multiplied by the number of slots showing the empty state. The data is constant, so it is computed once at module load.
 */
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

export function ClaudeMascot() {
  const rows = MASCOT_ROWS.length
  return (
    <svg
      data-rich-mascot="1" // The measurement script finds it by this attribute.
      data-rich-brand="claude" // Which backend it is (a stable token — richBranding.ts is where it is defined). Shape and color are unchanged.
      aria-hidden
      width={MASCOT_COLS * MASCOT_CELL_PX}
      height={rows * MASCOT_CELL_PX}
      // viewBox = grid cell units → display size is set by the cell constants above alone, and the shape scales up and down as is.
      viewBox={`0 0 ${MASCOT_COLS} ${rows}`}
      // Cell edges sit on integer coordinates, which removes anti-aliasing seams (translucent lines).
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
