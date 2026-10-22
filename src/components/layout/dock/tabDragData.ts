// TRD S21-slot-docking §4-2 「실음」: 탭 끌기의 OS 끌기 자료 — 타입 문자열 하나에 값 하나. 짓기(출처 dragstart) · 읽기(대상
//   dragenter · drop)를 이 파일이 갖는다. 순수하다(DOM · Tauri 0).
// ★대상이 dragover 동안 볼 수 있는 것은 `types` 뿐이다★(`getData` 는 drop 에서만 — 스파이크 실측) — 그래서 미리보기(R4)에 드는
//   원래 크기와 띠 유효성(R8)에 드는 끄는 탭 모양을 타입 문자열의 칸에 싣는다. 셸 경유 · 창 사이 채널은 거부했다(ADR-0296 ·
//   TRD §4-2). 타입 문자열 칸이 창 사이 OLE 를 못 건너는지는 미검이다(M3 — 새면 대상이 크기 · 모양 없음으로 읽는다).
// ★타입 문자열은 소문자로 짓고 소문자로 견준다★ — Chromium 이 `setData` 의 형식을 소문자로 바꾼다.
// ★이 형식 하나만 싣는다★ — `text/plain` · URL 을 실으면 다른 앱이 받아 그 앱에 글자가 들어간다(옛 HTML5 끌기 `TabBar` 조항).

import type { LayoutNode, SplitDir } from '../../../api/layoutTypes'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'

/** 우리 탭 끌기 타입 문자열의 머리 — 뒤에 `;키=값` 칸이 붙는다. */
export const TAB_MIME = 'application/x-engram-tab'

/** 값의 판 — 읽는 쪽은 이것이 아니면 우리 끌기 값이 아니라고 본다. */
const VALUE_VERSION = 1

/**
 * 끌기 자료의 값(JSON) — drop 에서만 읽힌다. 뜻은 `TabDrop` · `DropTo.new_window` 의 같은 이름 칸이다(`size` = 새 창 안쪽
 * 크기 · `null` = 탭 하나뿐인 팝아웃 · `grab` = 새 창 뷰포트에서 커서가 올 점).
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

/** 출처가 타입 문자열에 싣는 것. `original` = 원래 크기(R7 · CSS px — 정수로 반올림해 싣는다) · `shape` = `shapeOf` 의 답. */
export interface TabDragTypeInput {
  /** Tauri `getIdentifier()` — 다른 판(개발 · 배포 · 스파이크)의 끌기를 받지 않는 열쇠. */
  app: string
  drag: string
  original: SizeCss | null
  shape: string | null
}

/**
 * 대상이 `types` 에서 읽은 우리 끌기. 칸이 없거나 깨졌으면 그 칸만 `null` 이다 — 대상은 원래 크기 없음 = 칸 절반 몫(상한),
 * 모양 없음 = 칸 하나로 친다(TRD §4-2 · §4-5).
 */
export interface TabDragHead {
  /** `dataTransfer.types` 에 실린 그대로의 타입 문자열 — drop 에서 `getData` 의 열쇠. */
  mime: string
  drag: string | null
  original: SizeCss | null
  /** 끄는 탭 레이아웃 모양 — `parseShape` 의 답. */
  shape: LayoutNode | null
}

// ── 타입 문자열 ────────────────────────────────────────────────────────────────

/** 출처 dragstart 의 타입 문자열 — `application/x-engram-tab;app=…;drag=…;w=…;h=…;shape=…`(없는 칸은 싣지 않는다). */
export function buildTabDragType(input: TabDragTypeInput): string {
  const parts = [TAB_MIME, `app=${input.app}`, `drag=${input.drag}`]
  if (input.original !== null) {
    parts.push(`w=${Math.round(input.original.w)}`, `h=${Math.round(input.original.h)}`)
  }
  if (input.shape !== null) parts.push(`shape=${input.shape}`)
  return parts.join(';').toLowerCase()
}

/**
 * `types` 에서 우리 끌기를 찾는다 — 우리 머리이고 `app` 칸이 `app` 과 같은(대소문자 무시) 첫 타입. 없으면 `null`(파일 ·
 * 링크 · 트리 행 · 다른 판의 탭 끌기).
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

// ── 모양 ───────────────────────────────────────────────────────────────────────

// 전위 순 — 칸 = `l` · 분할 = `h<비율>_`(좌우) · `v<비율>_`(위아래) 뒤에 두 자식(a → b). 비율 = JS `String(n)` — f64 를
//   그대로 되읽고, 소문자이며 `;` · `=` 가 없어 타입 문자열 칸에 그대로 든다.
const DIR_CODE: Record<SplitDir, string> = { left_right: 'h', top_bottom: 'v' }

/** 레이아웃의 모양만(슬롯 id · 내용 없음) — 타입 문자열 `shape` 칸의 값. */
export function shapeOf(layout: LayoutNode): string {
  if (layout.type === 'slot') return 'l'
  return `${DIR_CODE[layout.dir]}${String(layout.ratio)}_${shapeOf(layout.a)}${shapeOf(layout.b)}`
}

/** `String(n)` 이 내는 유한 수의 꼴(소문자 지수). */
const NUMBER_TEXT = /^-?(?:\d+(?:\.\d+)?|\.\d+)(?:e[+-]?\d+)?$/

/**
 * `shapeOf` 의 답 → 레이아웃 나무. id 는 전위 순서로 지어 붙인다(칸 = `shape-slot-<k>` · 분할 = `shape-split-<k>`) · 칸 내용은
 * `empty`. 문법에 맞지 않거나 비율이 유한 수가 아니면 `null`.
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

// ── 값 ─────────────────────────────────────────────────────────────────────────

/** 출처 dragstart 의 값. */
export function buildTabDragValue(value: Omit<TabDragValue, 'v'>): string {
  return JSON.stringify({ v: VALUE_VERSION, ...value })
}

/** drop 의 `getData` 결과 → 값. JSON 이 깨졌거나 칸 모양이 다르면 `null`. */
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
