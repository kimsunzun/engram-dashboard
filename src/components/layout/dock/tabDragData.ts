// TRD S21-slot-docking §4-2 "carrying": OS drag data of the tab drag — one value per type string. Owns building (source dragstart) · reading (target dragenter · drop). Pure (no DOM · Tauri).
// ★A target sees only `types` during dragover★ (`getData` only at drop — spike measurement), so the original size for the preview (R4) and the dragged tab shape for band validity (R8) ride in type string fields. A path through the shell · a channel between windows was rejected (ADR-0296 · TRD §4-2). Whether type string fields survive OLE between windows is untested (M3 — if they leak, the target reads no size · no shape).
// ★The type string is built and compared in lowercase★ — Chromium lowercases the `setData` format.
// ★Carry only this one format★ — a `text/plain` · URL would be received by another app as text.

import type { LayoutNode, SplitDir } from '../../../api/layoutTypes'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'

/** Head of our tab drag type string — `;key=value` fields are attached after it. */
export const TAB_MIME = 'application/x-engram-tab'

/** Version of the value — if it is not this, the reader sees it as not our drag value. */
const VALUE_VERSION = 1

/**
 * Value of the drag data (JSON), read only at drop. Same-named fields as `TabDrop` · `DropTo.new_window` (`size` = new window inner size, `null` = single-tab popout; `grab` = where the cursor will be in the new window viewport).
 */
export interface TabDragValue {
  v: typeof VALUE_VERSION
  drag: string
  view: string
  from: string
  initial: string | null
  size: SizeCss | null
  grab: PointCss
}

/** What the source carries in the type string. `original` = original size (R7 · CSS px — carried rounded to integers) · `shape` = the answer of `shapeOf`. */
export interface TabDragTypeInput {
  /** Tauri `getIdentifier()` — a key that does not accept drags of other builds (dev · release · spike). */
  app: string
  drag: string
  original: SizeCss | null
  shape: string | null
}

/**
 * Our drag as read from `types`. A missing or broken field is only that field `null` — no original size = half the area (the cap), no shape = one area (TRD §4-2 · §4-5).
 */
export interface TabDragHead {
  /** The type string exactly as carried in `dataTransfer.types` — the key for `getData` at drop. */
  mime: string
  drag: string | null
  original: SizeCss | null
  /** Layout shape of the dragged tab — the answer of `parseShape`. */
  shape: LayoutNode | null
}

// ── type string ────────────────────────────────────────────────────────────────

/** Type string of the source dragstart — `application/x-engram-tab;app=…;drag=…;w=…;h=…;shape=…` (a missing field is not carried). */
export function buildTabDragType(input: TabDragTypeInput): string {
  const parts = [TAB_MIME, `app=${input.app}`, `drag=${input.drag}`]
  if (input.original !== null) {
    parts.push(`w=${Math.round(input.original.w)}`, `h=${Math.round(input.original.h)}`)
  }
  if (input.shape !== null) parts.push(`shape=${input.shape}`)
  return parts.join(';').toLowerCase()
}

/**
 * Find our drag in `types` — the first type with our head whose `app` field equals `app` (case ignored); `null` otherwise (a file · link · tree row · tab drag of another build).
 */
export function readTabDragType(types: readonly string[], app: string): TabDragHead | null {
  const want = app.toLowerCase()
  for (const mime of types) {
    const [head, ...rest] = mime.toLowerCase().split(';')
    if (head !== TAB_MIME) continue
    const params = new Map<string, string>()
    for (const p of rest) {
      const eq = p.indexOf('=')
      if (eq > 0) params.set(p.slice(0, eq), p.slice(eq + 1))
    }
    if (params.get('app') !== want) continue
    const drag = params.get('drag')
    const shapeText = params.get('shape')
    return {
      mime,
      drag: drag !== undefined && drag.length > 0 ? drag : null,
      original: sizeOf(params.get('w'), params.get('h')),
      shape: shapeText === undefined ? null : parseShape(shapeText),
    }
  }
  return null
}

const POSITIVE_INT = /^[1-9]\d*$/

function sizeOf(w: string | undefined, h: string | undefined): SizeCss | null {
  if (w === undefined || h === undefined || !POSITIVE_INT.test(w) || !POSITIVE_INT.test(h)) return null
  return { w: Number(w), h: Number(h) }
}

// ── shape ───────────────────────────────────────────────────────────────────────

// Pre-order — area = `l` · split = `h<ratio>_` (left-right) · `v<ratio>_` (top-bottom) then two children (a → b). Ratio = JS `String(n)` (f64 read back as is, lowercase, no `;` · `=`), so it fits a type string field.
const DIR_CODE: Record<SplitDir, string> = { left_right: 'h', top_bottom: 'v' }

/** Shape of the layout only (no slot id · no content) — the value of the type string `shape` field. */
export function shapeOf(layout: LayoutNode): string {
  if (layout.type === 'slot') return 'l'
  return `${DIR_CODE[layout.dir]}${String(layout.ratio)}_${shapeOf(layout.a)}${shapeOf(layout.b)}`
}

/** Form of a finite number that `String(n)` emits (lowercase exponent). */
const NUMBER_TEXT = /^-?(?:\d+(?:\.\d+)?|\.\d+)(?:e[+-]?\d+)?$/

/**
 * `shapeOf` answer → layout tree. Ids are built in pre-order (area = `shape-slot-<k>` · split = `shape-split-<k>`), area content `empty`. `null` if the grammar does not fit or a ratio is not finite.
 */
export function parseShape(text: string): LayoutNode | null {
  let at = 0
  let slots = 0
  let splits = 0
  const node = (): LayoutNode | null => {
    const code = text[at]
    if (code === 'l') {
      at += 1
      return { type: 'slot', id: `shape-slot-${slots++}`, content: { type: 'empty' } }
    }
    const dir = code === 'h' ? 'left_right' : code === 'v' ? 'top_bottom' : null
    if (dir === null) return null
    const end = text.indexOf('_', at + 1)
    if (end < 0) return null
    const ratioText = text.slice(at + 1, end)
    if (!NUMBER_TEXT.test(ratioText)) return null
    const ratio = Number(ratioText)
    if (!Number.isFinite(ratio)) return null
    at = end + 1
    const id = `shape-split-${splits++}`
    const a = node()
    if (a === null) return null
    const b = node()
    if (b === null) return null
    return { type: 'split', id, dir, ratio, a, b }
  }
  const root = node()
  return root !== null && at === text.length ? root : null
}

// ── value ─────────────────────────────────────────────────────────────────────────

/** Value of the source dragstart. */
export function buildTabDragValue(value: Omit<TabDragValue, 'v'>): string {
  return JSON.stringify({ v: VALUE_VERSION, ...value })
}

/** Result of `getData` at drop → value. `null` if the JSON is broken or the field shape differs. */
export function readTabDragValue(raw: string): TabDragValue | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }
  if (parsed === null || typeof parsed !== 'object') return null
  const v = parsed as Record<string, unknown>
  if (v.v !== VALUE_VERSION) return null
  if (typeof v.drag !== 'string' || typeof v.view !== 'string' || typeof v.from !== 'string') return null
  if (v.initial !== null && typeof v.initial !== 'string') return null
  const size = v.size === null ? null : pairOf(v.size, 'w', 'h')
  if (v.size !== null && size === null) return null
  const grab = pairOf(v.grab, 'x', 'y')
  if (grab === null) return null
  return {
    v: VALUE_VERSION,
    drag: v.drag,
    view: v.view,
    from: v.from,
    initial: v.initial,
    size: size === null ? null : { w: size[0], h: size[1] },
    grab: { x: grab[0], y: grab[1] },
  }
}

function pairOf(raw: unknown, a: string, b: string): [number, number] | null {
  if (raw === null || typeof raw !== 'object') return null
  const o = raw as Record<string, unknown>
  const x = o[a]
  const y = o[b]
  if (typeof x !== 'number' || typeof y !== 'number' || !Number.isFinite(x) || !Number.isFinite(y)) return null
  return [x, y]
}
