// ★왜 분리★(ADR-0051 rail / ADR-0053 구조 분할): StructuredTextView 는 렌더 dispatch 오케스트레이터로
//   남기고, 순수 로직(위치 계산)은 여기로 내려 경계를 명확히 한다(순수 로직 ↔ 컴포넌트).

export type ChatRowKind = 'assistant' | 'boundary' | 'skip'

export type RailRunPosition = 'top' | 'mid' | 'bottom' | 'single'

/**
 * 기존엔 모든 rail 행이 top-[-12px] 로 위 행에 붙어 최상단 dot(예: "Thought") 위로 선 stub 이 튀어나왔다.
 */
export function computeRailRunPositions(kinds: ChatRowKind[]): (RailRunPosition | null)[] {
  const visible: { idx: number; kind: Exclude<ChatRowKind, 'skip'> }[] = []
  kinds.forEach((kind, idx) => {
    if (kind !== 'skip') visible.push({ idx, kind })
  })

  const out: (RailRunPosition | null)[] = kinds.map(() => null)

  for (let i = 0; i < visible.length; i++) {
    const { idx, kind } = visible[i]
    if (kind !== 'assistant') continue
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
