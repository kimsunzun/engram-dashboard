// The per-kind disposition table (ADR-0172 decision 5) — guards that this table is the only place the screen gets wording.
//
// ★Do not write literals here★: if written, it passes even when the table and i18n diverge. Instead we measure "did the table come from i18n" and
//   "does an answer come out even when kinds grow".

import { describe, expect, it } from 'vitest'

import { failureLine, failureSpec } from './failureKinds'
import { t } from '../../i18n'
import type { AgentFailureKind } from '../../api/types'

const KINDS: AgentFailureKind[] = [
  'NoConversationToResume',
  'SpawnFailed',
  'EarlyExitAfterResume',
  'Other',
]

describe('failureKinds — disposition table', () => {
  it('every kind has non-empty text and action', () => {
    for (const k of KINDS) {
      const spec = failureSpec(k)
      expect(spec.reason.length).toBeGreaterThan(0)
      expect(spec.action.length).toBeGreaterThan(0)
    }
  })

  // ★This table is the single copy of this decision (there is no Rust pair table — failureKinds.ts header)★.
  it('the only locking kind is 「no conversation to take over」 and the rest are retryable (fail-open)', () => {
    expect(failureSpec('NoConversationToResume').retryable).toBe(false)
    expect(failureSpec('SpawnFailed').retryable).toBe(true)
    expect(failureSpec('EarlyExitAfterResume').retryable).toBe(true)
    expect(failureSpec('Other').retryable).toBe(true)
  })

  it('the text comes from i18n (no Korean is hard-coded in components or the table)', () => {
    expect(failureSpec('NoConversationToResume').reason).toBe(t('agent.failureNoConversation'))
    expect(failureSpec('SpawnFailed').reason).toBe(t('agent.failureSpawn'))
    expect(failureSpec('EarlyExitAfterResume').reason).toBe(t('agent.failureEarlyExit'))
    expect(failureSpec('Other').reason).toBe(t('agent.failureOther'))
  })

  it('one line carries the text and the recommended action together', () => {
    const spec = failureSpec('NoConversationToResume')
    expect(failureLine('NoConversationToResume')).toBe(
      t('agent.failureLine', { reason: spec.reason, action: spec.action }),
    )
  })

  it('a missing value has nothing to show (null/undefined)', () => {
    expect(failureLine(null)).toBeNull()
    expect(failureLine(undefined)).toBeNull()
  })

  it('★an unknown kind is not blocked (fail-open)★ — even if the backend widens its vocabulary the screen gives an answer', () => {
    const unknown = 'SomethingNewFromTheDaemon' as AgentFailureKind
    const spec = failureSpec(unknown)
    expect(spec.retryable).toBe(true)
    expect(spec.reason).toBe(failureSpec('Other').reason)
    expect(failureLine(unknown)).toBe(failureLine('Other'))
  })
})
