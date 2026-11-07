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

describe('failureKinds — 처분표', () => {
  it('종류마다 문구·행동이 비어 있지 않다', () => {
    for (const k of KINDS) {
      const spec = failureSpec(k)
      expect(spec.reason.length).toBeGreaterThan(0)
      expect(spec.action.length).toBeGreaterThan(0)
    }
  })

  // ★This table is the single copy of this decision (there is no Rust pair table — failureKinds.ts header)★.
  it('잠그는 종류는 「이어받을 대화 없음」 하나뿐이고 나머지는 재시도 가능(fail-open)', () => {
    expect(failureSpec('NoConversationToResume').retryable).toBe(false)
    expect(failureSpec('SpawnFailed').retryable).toBe(true)
    expect(failureSpec('EarlyExitAfterResume').retryable).toBe(true)
    expect(failureSpec('Other').retryable).toBe(true)
  })

  it('문구는 i18n 에서 온다(컴포넌트·표 어디에도 한국어를 박지 않는다)', () => {
    expect(failureSpec('NoConversationToResume').reason).toBe(t('agent.failureNoConversation'))
    expect(failureSpec('SpawnFailed').reason).toBe(t('agent.failureSpawn'))
    expect(failureSpec('EarlyExitAfterResume').reason).toBe(t('agent.failureEarlyExit'))
    expect(failureSpec('Other').reason).toBe(t('agent.failureOther'))
  })

  it('한 줄은 문구와 권하는 행동을 함께 담는다', () => {
    const spec = failureSpec('NoConversationToResume')
    expect(failureLine('NoConversationToResume')).toBe(
      t('agent.failureLine', { reason: spec.reason, action: spec.action }),
    )
  })

  it('없는 값은 보일 것이 없다(null/undefined)', () => {
    expect(failureLine(null)).toBeNull()
    expect(failureLine(undefined)).toBeNull()
  })

  it('★모르는 종류는 막지 않는다(fail-open)★ — 백엔드가 어휘를 늘려도 화면이 답을 낸다', () => {
    const unknown = 'SomethingNewFromTheDaemon' as AgentFailureKind
    const spec = failureSpec(unknown)
    expect(spec.retryable).toBe(true)
    expect(spec.reason).toBe(failureSpec('Other').reason)
    expect(failureLine(unknown)).toBe(failureLine('Other'))
  })
})
