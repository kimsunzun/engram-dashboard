// ADR-0051: dot-rail run-position 순수 계산(no React) — StructuredTextView 에서 분리한 pure util(ADR-0053
//   구조 분할). rail 연결선 clean-ends 분기의 입력을 만든다. React 무관이라 단위테스트 대상(railPositions.test).
//
// ★왜 분리★: StructuredTextView 는 렌더 dispatch 오케스트레이터로 남기고, 순수 로직(위치 계산)은 여기로
//   내려 경계를 명확히 한다(순수 로직 ↔ 컴포넌트). 동작은 이전 인라인 구현과 동일(리팩터, 행동 불변).

/**
 * ADR-0051: 렌더될 각 행의 "종류" — rail 연결선 run 계산의 입력.
 *   - 'assistant': rail 행(text·thinking·tool·generic·error + streaming tail) — 세로 thread 로 묶인다.
 *   - 'boundary' : user 버블·separator — rail 이 아니라 run 을 끊는다(턴 경계).
 *   - 'skip'     : DOM 을 만들지 않는 항목(usage·흡수된 tool_result). 시각적으로 없으므로 run 을 끊지
 *                  *않는다*(assistant 두 행 사이 usage 가 껴도 두 행은 화면상 인접 → 한 run).
 */
export type ChatRowKind = 'assistant' | 'boundary' | 'skip'

/** ADR-0051: rail 행의 run 내 위치 — 연결선 그리기 분기의 키. */
export type RailRunPosition = 'top' | 'mid' | 'bottom' | 'single'

/**
 * ADR-0051: run-position 순수 함수(단위테스트 대상 — StructuredTextView 순수성 유지, ADR-0050).
 * 행 종류 배열을 받아 각 assistant 행의 run 내 위치를 반환한다(assistant 가 아니면 null).
 * run = 'skip' 을 무시하고 이어지는 assistant 행들의 최대 연속 구간. run 경계 = 'boundary'.
 *   첫 행 = top · 마지막 = bottom · 가운데 = mid · 혼자 = single.
 * 이 위치로 연결선 clean-ends 를 만든다(top=아래로만, mid=관통, bottom=위로만, single=선 없음) —
 * 기존엔 모든 rail 행이 top-[-12px] 로 위 행에 붙어 최상단 dot(예: "Thought") 위로 선 stub 이 튀어나왔다.
 */
export function computeRailRunPositions(kinds: ChatRowKind[]): (RailRunPosition | null)[] {
  // 1) skip 을 제외한 "보이는 행"만 골라 run 계산 → 원래 인덱스로 되돌린다.
  const visible: { idx: number; kind: Exclude<ChatRowKind, 'skip'> }[] = []
  kinds.forEach((kind, idx) => {
    if (kind !== 'skip') visible.push({ idx, kind })
  })

  const out: (RailRunPosition | null)[] = kinds.map(() => null)

  for (let i = 0; i < visible.length; i++) {
    const { idx, kind } = visible[i]
    if (kind !== 'assistant') continue // boundary → 위치 없음
    const prevAssistant = i > 0 && visible[i - 1].kind === 'assistant'
    const nextAssistant = i < visible.length - 1 && visible[i + 1].kind === 'assistant'
    out[idx] =
      prevAssistant && nextAssistant
        ? 'mid'
        : !prevAssistant && nextAssistant
          ? 'top'
          : prevAssistant && !nextAssistant
            ? 'bottom'
            : 'single'
  }
  return out
}
