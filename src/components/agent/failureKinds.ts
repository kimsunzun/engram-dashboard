// The per-kind disposition table of the "last failure" — **the only place the screen gets wording** (ADR-0172 decision 5 · ADR-0173 §Impact).
// ★Do not embed strings in components★: kinds growing would need two fixes, and a screen where only one was fixed silently draws the old wording. Both the tree hover and the slot exit screen call the single `failureLine` below.
// ★This table is the single copy — no pair table on the Rust side★: the wire spec carries **the kind only** (the core owns only the vocabulary), and every consumer of "worth retrying" (click blocking · "start a new conversation" — ADR-0172 decision 6 follow-up) is on this side; two copies the compiler cannot match are worse. If the consumer moves to the core, the table moves with it.
// ★Unknown kinds are not blocked (fail-open)★: a value not in `SPECS` falls to "other", which is retryable — being unable to open a perfectly good entry costs more than one wasted attempt.

import type { AgentFailureKind } from '../../api/types'
import { t } from '../../i18n'

export type FailureKindSpec = {
  /**
   * Is it worth retrying. ★No screen reads it yet (reserved)★ — click blocking and "start a new conversation" (ADR-0172 decision 6 follow-ups) will use it; removing it now would make those features reinvent the table.
   */
  retryable: boolean
  /** What happened — one line. */
  reason: string
  /** So what to do — one line. */
  action: string
}

/**
 * Kind → {worth retrying · wording · recommended action}. Values are made at call time — frozen at module load, the wording would not follow a later locale change (the stable API contract of `t()`).
 */
function specs(): Record<AgentFailureKind, FailureKindSpec> {
  return {
    NoConversationToResume: {
      retryable: false,
      reason: t('agent.failureNoConversation'),
      action: t('agent.failureNoConversationAction'),
    },
    SpawnFailed: {
      retryable: true,
      reason: t('agent.failureSpawn'),
      action: t('agent.failureSpawnAction'),
    },
    EarlyExitAfterResume: {
      retryable: true,
      reason: t('agent.failureEarlyExit'),
      action: t('agent.failureEarlyExitAction'),
    },
    Other: {
      retryable: true,
      reason: t('agent.failureOther'),
      action: t('agent.failureOtherAction'),
    },
  }
}

/** An unknown kind (the backend grew the vocabulary but the frontend does not know yet) also yields an answer — absorbed into 「other」. */
export function failureSpec(kind: AgentFailureKind): FailureKindSpec {
  const table = specs()
  return table[kind] ?? table.Other
}

/** The single line shared by the tree hover · the slot exit screen. If `null` there is nothing to show. */
export function failureLine(kind: AgentFailureKind | null | undefined): string | null {
  if (!kind) return null
  const spec = failureSpec(kind)
  return t('agent.failureLine', { reason: spec.reason, action: spec.action })
}
