// ADR-0061: names are not stored; the cwd basename is used. ★Single source★: the preset display name (PresetPalette) and the agent row display name (AgentList) must derive names by the same rule — copies would diverge on win/posix and root edges.

import { t } from '../i18n'

/** Key of the name shown for a blank path — resolved on each `basename` call, never kept (ADR-0373). */
export const PATH_NAME_PLACEHOLDER = 'common.noPath'

/**
 * Display name derived from `cwd`; in the current UI language when the path is blank — call it where the name is shown, never keep the result as a key or comparison baseline (ADR-0373).
 * ★The result is never blank (empty or whitespace only)★: the label draws only this value, so a blank one would show an empty cell.
 * Edges with no or a misleading basename (a root, a drive root) are not derived and fall back to the raw cwd (collapsing to "C:" would mislead).
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
