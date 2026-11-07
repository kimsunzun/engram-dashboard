// TRD S21-slot-docking §4-2 「실음」: OS drag data of the tab drag — one value per type string. This file owns building (source dragstart) · reading (target
//   dragenter · drop). Pure (no DOM · Tauri).
// ★All a target can see during dragover is `types`★ (`getData` only at drop — spike measurement) — so the original size needed by the preview (R4) and the dragged tab shape needed by
//   band validity (R8) are carried in fields of the type string. A path through the shell · a channel between windows was rejected (ADR-0296 ·
//   TRD §4-2). Whether a type string field cannot cross OLE between windows is untested (M3 — if it leaks, the target reads it as no size · no shape).
// ★The type string is built in lowercase and compared in lowercase★ — Chromium lowercases the format of `setData`.
// ★Carry only this one format★ — if `text/plain` · a URL is carried, another app receives it and text enters that app (the clause of the old HTML5 drag `TabBar`).

import type { LayoutNode, SplitDir } from '../../../api/layoutTypes'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'

/** Head of our tab drag type string — `;키=값` fields are attached after it. */
export const TAB_MIME = 'application/x-engram-tab'

/** Version of the value — if it is not this, the reader sees it as not our drag value. */
const VALUE_VERSION = 1

/**
 * Value of the drag data (JSON) — read only at drop. Meaning is the same-named fields of `TabDrop` · `DropTo.new_window` (`size` = new window inner
 * size · `null` = single-tab popout · `grab` = the point where the cursor will be in the new window viewport).
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
 * Our drag the target read from `types`. If a field is missing or broken, only that field is `null` — the target treats no original size = half the area (the cap),
 * no shape = one area (TRD §4-2 · §4-5).
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
 * Find our drag in `types` — the first type that is our head and whose `app` field equals `app` (case ignored). If none, `null` (a file ·
 * link · tree row · tab drag of another build).
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

// Pre-order — area = `l` · split = `h<비율>_` (left-right) · `v<비율>_` (top-bottom) followed by two children (a → b). Ratio = JS `String(n)` — f64 is read back
//   as is, is lowercase and has no `;` · `=`, so it fits in a type string field as is.
const DIR_CODE: Record<SplitDir, string> = { left_right: 'h', top_bottom: 'v' }

/** Shape of the layout only (no slot id · no content) — the value of the type string `shape` field. */
export function shapeOf(layout: LayoutNode): string {
  if (layout.type === 'slot') return 'l'
  return `${DIR_CODE[layout.dir]}${String(layout.ratio)}_${shapeOf(layout.a)}${shapeOf(layout.b)}`
}

/** Form of a finite number that `String(n)` emits (lowercase exponent). */
const NUMBER_TEXT = /^-?(?:\d+(?:\.\d+)?|\.\d+)(?:e[+-]?\d+)?$/

/**
 * Answer of `shapeOf` → layout tree. Ids are built and attached in pre-order (area = `shape-slot-<k>` · split = `shape-split-<k>`) · area content is
 * `empty`. `null` if it does not fit the grammar or a ratio is not a finite number.
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
