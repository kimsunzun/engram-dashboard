// ★Why split★ (ADR-0051 rail / ADR-0053 structural split): StructuredTextView is left as the render dispatch orchestrator,
//   and pure logic (position calculation) goes down here to make the boundary clear (pure logic ↔ component).

export type ChatRowKind = 'assistant' | 'boundary' | 'skip'

export type RailRunPosition = 'top' | 'mid' | 'bottom' | 'single'

/**
 * Before, every rail row attached to the row above at top-[-12px], so a stub stuck out above the topmost dot (e.g. "Thought").
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
