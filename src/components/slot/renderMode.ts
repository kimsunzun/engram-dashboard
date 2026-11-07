// ★Frontend only★: the backend wire does not know this concept (not a target of layoutTypes re-export).

import type { AgentInfo } from '../../api/types'

export const RENDER_MODES = ['terminal', 'rich', 'dom'] as const

/** Slot renderer kind — 'terminal'=xterm, 'rich'=structured RichSlot, 'dom'=plain-text observation DomSlot (§5). */
export type RenderMode = (typeof RENDER_MODES)[number]

/**
 * Runtime RenderMode guard — validates values that come from outside the type, like the command argument bag. `slot.renderMode.set` filters with
 * this and throws, and `setRenderMode` also does not write an invalid mode by the same decision — if a wrong mode leaks into an
 * override, the LayoutLeaf switch quietly drops it to terminal.
 */
export function isRenderMode(mode: unknown): mode is RenderMode {
  return typeof mode === 'string' && (RENDER_MODES as readonly string[]).includes(mode)
}

/**
 * ★Do not store the wire boolean, derive it every time★: capabilities.output.structured is a wire-authoritative value, and
 * if the frontend copies and keeps it, it becomes a source of drift — at render time it is only derived from that value (the override is stored separately).
 */
export function defaultRenderMode(agent: AgentInfo): RenderMode {
  return agent.capabilities.output.structured ? 'rich' : 'terminal'
}
