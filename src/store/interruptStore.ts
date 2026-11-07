// ADR-0244: the 「중단하는 중」 (interrupting) from sending an interrupt in a chat area until that turn ends — each agent holds its interrupt request.
//   Frontend-only · in-memory · separate per window (a zustand module store — each window has its own instance).
//   ★The one place that sets it is the window command `agent.interrupt` (`commands/agentCommands.ts`)★ — a human Esc and the LLM take the same path
//   (CLAUDE.md 「LLM-우선 제어」). Do not make a second path. Where it is cleared —
//   ① when that request was refused (the command)
//   ② when the conversation view (RichSlot) received live a turn boundary (MessageDone · TurnEnd — regardless of outcome) · when it caught up for the first time and there was no open turn ·
//      when the waiting indicator goes on → off · on a new incarnation (clear) · when the agent is gone or the connection dropped
//   ③ when that agent's last conversation view is released (`watch` below) — a view whose subscription stopped (`detached` · `error`) counts as released.
//   ★If this window has no conversation view of that agent at all, it is not set (`watch`)★ — there is no one to clear it on seeing the turn end, so if set, afterwards
//   every interrupt command in this window answers 「이미 보냈다」 (already sent) and does not send. For the same reason it is cleared when the last view disappears.
//   An interrupt sent straight over the bus (`engram agent.interrupt …`) does not pass through this window's command so it is not set here — an intended scope.

import { create } from 'zustand'

export interface InterruptState {
  /** Agent that sent an interrupt and waits for that turn to end → its interrupt request. If there is no entry it is not waiting. */
  pending: Record<string, Promise<void>>
  /** Agent → number of conversation views of that agent currently mounted in this window. 0 if there is no entry. */
  views: Record<string, number>
  /**
   * A conversation view of that agent was mounted. @returns a release function — call it on unmount (cleanup). Calling it twice counts only once.
   *   When the last view is released the wait is cleared too.
   */
  watch(agentId: string): () => void
  /** That agent becomes waiting. If this window has no conversation view of that agent it does nothing (header comment). */
  begin(agentId: string, request: Promise<void>): void
  /**
   * Clear the wait. If `request` is given, clear only when what is held now is that request — so a late old request's refusal does not erase
   * the wait of the new request after it. If omitted, clear whoever's it is (turn end). If not waiting, does nothing.
   */
  end(agentId: string, request?: Promise<void>): void
}

// Agent ids also come from outside (the LLM) — reading with a bare index would read a prototype name like `'constructor'` as an entry.
function own<T>(record: Record<string, T>, key: string): T | undefined {
  return Object.prototype.hasOwnProperty.call(record, key) ? record[key] : undefined
}

function without<T>(record: Record<string, T>, key: string): Record<string, T> {
  const next = { ...record }
  delete next[key]
  return next
}

/** The interrupt request that agent is waiting on — `undefined` if none (not interrupting). */
export function pendingInterrupt(
  state: Pick<InterruptState, 'pending'>,
  agentId: string,
): Promise<void> | undefined {
  return own(state.pending, agentId)
}

export const useInterruptStore = create<InterruptState>((set, get) => ({
  pending: {},
  views: {},
  watch: agentId => {
    set(s => ({ views: { ...s.views, [agentId]: (own(s.views, agentId) ?? 0) + 1 } }))
    let released = false
    return () => {
      if (released) return
      released = true
      const left = (own(get().views, agentId) ?? 1) - 1
      if (left > 0) {
        set(s => ({ views: { ...s.views, [agentId]: left } }))
        return
      }
      set(s => ({ views: without(s.views, agentId), pending: without(s.pending, agentId) }))
    }
  },
  begin: (agentId, request) => {
    if ((own(get().views, agentId) ?? 0) === 0) return
    set(s => ({ pending: { ...s.pending, [agentId]: request } }))
  },
  end: (agentId, request) => {
    const current = own(get().pending, agentId)
    if (current === undefined || (request !== undefined && current !== request)) return
    set(s => ({ pending: without(s.pending, agentId) }))
  },
}))
