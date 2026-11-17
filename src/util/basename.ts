// ADR-0061: 이름을 저장하지 않고 cwd basename 을 쓴다.
//
// ★단일 출처★: 경로에서 이름을 파생하는 자리마다 같은 규칙을 써야 한다. 각자 복제하면 win/posix·root 엣지 처리가 갈려 표시가 어긋나므로 여기 한 곳에 둔다.

import { t } from '../i18n'

/** Key of the name shown for a blank path — resolved on each `basename` call, never kept (ADR-0373). */
export const PATH_NAME_PLACEHOLDER = 'common.noPath'

/**
 * The display name derived from `cwd`, in the current UI language when the path is blank — call it where the name is shown,
 *   and do not keep the result as a key or a comparison baseline (ADR-0373).
 *
 * ★The result is never blank (empty or whitespace only)★: the label above draws only this value, so a blank one would show
 *   the row as an empty cell.
 *
 * Edges with no basename or a misleading one, like a root or a drive root, are not derived and fall back to the raw cwd
 *   (collapsing to "C:" would mislead).
 */
export function basename(cwd: string): string {
  if (!cwd || cwd.trim().length === 0) return t(PATH_NAME_PLACEHOLDER)
  const trimmed = cwd.replace(/[\\/]+$/, '')
  if (trimmed.length === 0) return cwd
  const idx = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  const base = idx >= 0 ? trimmed.slice(idx + 1) : trimmed
  if (base.length === 0 || /^[A-Za-z]:$/.test(base)) return cwd
  return base
}
