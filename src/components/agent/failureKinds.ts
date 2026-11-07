// The per-kind disposition table of 「마지막 실패」 (last failure) — **the only place the screen gets wording** (ADR-0172 decision 5 · ADR-0173 §영향).
//
// ★Do not embed strings in components★: if embedded, two places must be fixed when kinds grow, and a screen where only one was fixed
//   silently draws the old wording. Both the tree hover and the slot exit screen call the single `failureLine` below.
// ★This table is the single copy — do not make a pair table on the Rust side★: the wire spec carries **the kind only** (the core owns
//   only the vocabulary), and every feature that will actually consume 「다시 해볼 가치」 (worth retrying) is on this side (click blocking · 「새 대화로 시작」 (start a new conversation) —
//   a follow-up in ADR-0172 decision 6). Having both sides creates two copies the compiler cannot match. If the consumer moves to the core,
//   the table moves with it then.
// ★Unknown kinds are not blocked (fail-open)★: a value not in `SPECS` below falls to 「그 밖」 (other) and that one
//   is retryable. The harm of blocking and being unable to open a perfectly good entry is greater than one wasted attempt.

import type { AgentFailureKind } from '../../api/types'
import { t } from '../../i18n'

export type FailureKindSpec = {
  /**
   * Is it worth retrying. ★No screen reads it yet (reserved)★ — click blocking and 「새 대화로 시작」 (start a new conversation) are where this value
   * will be used and both are follow-ups (ADR-0172 decision 6). If removed now, those features would reinvent the table.
   */
  retryable: boolean
  /** What happened — one line. */
  reason: string
  /** So what to do — one line. */
  action: string
}

/**
 * Kind → {worth retrying · wording · recommended action}. Values are made at call time (if frozen as constants at module load,
 * the wording does not follow even when the locale changes later — the stable API contract of `t()`).
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

/** An unknown kind (the backend grew the vocabulary but the frontend does not know yet) also yields an answer — absorbed into 「그 밖」 (other). */
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
