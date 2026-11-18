import { describe, expect, it } from 'vitest'

import { computeRailRunPositions, type ChatRowKind } from './railPositions'

describe('computeRailRunPositions (ADR-0051 dot-rail clean-ends)', () => {
  it('a single assistant row only → single (isolated dot — no connector)', () => {
    expect(computeRailRunPositions(['assistant'])).toEqual(['single'])
  })

  it('3 consecutive assistant rows → top/mid/bottom', () => {
    const kinds: ChatRowKind[] = ['assistant', 'assistant', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual(['top', 'mid', 'bottom'])
  })

  it('boundary (user bubble/separator) breaks the run', () => {
    const kinds: ChatRowKind[] = ['assistant', 'assistant', 'boundary', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual(['top', 'bottom', null, 'single'])
  })

  it('skip (usage/absorbed tool_result) does not break the run (no DOM → visually adjacent)', () => {
    const kinds: ChatRowKind[] = ['assistant', 'skip', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual(['top', null, 'bottom'])
  })

  it('mixed sequence starting/ending with a boundary', () => {
    const kinds: ChatRowKind[] = [
      'boundary',
      'assistant',
      'assistant',
      'boundary',
      'assistant',
      'assistant',
      'assistant',
    ]
    expect(computeRailRunPositions(kinds)).toEqual([
      null,
      'top',
      'bottom',
      null,
      'top',
      'mid',
      'bottom',
    ])
  })

  it('if all are skip there is no position (all null)', () => {
    expect(computeRailRunPositions(['skip', 'skip'])).toEqual([null, null])
  })

  it('a leading skip does not collapse top (the first assistant is still top)', () => {
    const kinds: ChatRowKind[] = ['skip', 'assistant', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual([null, 'top', 'bottom'])
  })
})
