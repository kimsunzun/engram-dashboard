// 「마지막 실패」 종류별 처분표 — **화면이 문구를 얻는 유일한 곳**(ADR-0161 결정 5 · ADR-0162 §영향).
//
// ★컴포넌트에 문자열을 박지 않는다★: 박으면 종류가 늘 때 두 곳을 고쳐야 하고, 한쪽만 고친 화면이
//   조용히 옛 문구를 그린다. 트리 hover 도 슬롯 종료 화면도 아래 `failureLine` 하나를 부른다.
// ★이 표가 유일본이다 — 러스트 쪽에 짝 표를 만들지 말 것★: 통신 규격은 **종류만** 나르고(코어는 어휘만
//   소유한다), 「다시 해볼 가치」를 실제로 소비할 기능은 전부 이쪽이다(클릭 차단·「새 대화로 시작」 —
//   ADR-0161 결정 6 으로 후속). 양쪽에 두면 컴파일러가 못 맞추는 사본이 둘 생긴다. 소비자가 코어로
//   옮겨가면 그때 표도 함께 옮긴다.
// ★모르는 종류는 막지 않는다(fail-open)★: 아래 `SPECS` 에 없는 값이 오면 「그 밖」으로 떨어지고 그쪽은
//   재시도 가능이다. 막았다가 멀쩡한 항목을 못 여는 손해가 헛시도 한 번보다 크다.

import type { AgentFailureKind } from '../../api/types'
import { t } from '../../i18n'

export type FailureKindSpec = {
  /**
   * 다시 해볼 가치가 있나. ★아직 읽는 화면이 없다(예약)★ — 클릭 차단과 「새 대화로 시작」이 이 값을
   * 쓸 자리이고 둘 다 후속이다(ADR-0161 결정 6). 지금 지우면 그 기능이 표를 다시 발명한다.
   */
  retryable: boolean
  /** 무슨 일이 있었나 — 한 줄. */
  reason: string
  /** 그래서 뭘 하면 되나 — 한 줄. */
  action: string
}

/**
 * 종류 → {다시 해볼 가치 · 문구 · 권하는 행동}. 값은 호출 시점에 만든다(모듈 로드 시 상수로 굳히면
 * 나중에 로케일이 바뀌어도 문구가 안 따라온다 — `t()` 의 안정 API 계약).
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

/** 모르는 종류(백엔드가 어휘를 늘렸는데 프론트가 아직 모름)도 답을 낸다 — 「그 밖」으로 흡수. */
export function failureSpec(kind: AgentFailureKind): FailureKindSpec {
  const table = specs()
  return table[kind] ?? table.Other
}

/** 트리 hover·슬롯 종료 화면이 함께 쓰는 한 줄. `null` 이면 보일 것이 없다. */
export function failureLine(kind: AgentFailureKind | null | undefined): string | null {
  if (!kind) return null
  const spec = failureSpec(kind)
  return t('agent.failureLine', { reason: spec.reason, action: spec.action })
}
