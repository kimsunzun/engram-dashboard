// 종류별 처분표(ADR-0161 결정 5) — 화면이 문구를 얻는 유일한 곳이 이 표라는 것을 지킨다.
//
// ★리터럴을 여기 적지 않는다★: 적으면 표와 i18n 이 갈려도 통과한다. 대신 "표가 i18n 에서 왔나" 와
//   "종류가 늘어도 답이 나오나" 를 잰다.

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

  // ★이 표가 이 판정의 유일본이다(러스트 짝 표는 없다 — failureKinds.ts 헤더)★.
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
