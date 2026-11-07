// Test only — reads the golden shared with the agent reducer (`queued_input_golden.json`) **from its path** (ADR-0231).
// ★No copy is kept★ — with a copy, fixing only the Rust-side golden would leave the TS test green and the two editions quietly diverge.
// The canonical file format is `_format` at the head of that file.

import goldenSource from '../../../../crates/engram-dashboard-agent/src/queued_input_golden.json?raw'
import type { QueuedInputEvent } from '../../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import type { QueuedEntry } from '../queuedInputReducer'

/** Golden row = same words as a list query row (`state` · `cancel`). `sent` is carried as `true` only on a row where the handoff marker stood (absent = not handed off). */
export interface GoldenRow {
  id: string
  text: string
  state: 'queued' | 'cancelling'
  cancel: null | { answer: 'none' | 'not_removed'; vendor_closed: boolean }
  sent?: true
}

export interface GoldenCase {
  name: string
  events: QueuedInputEvent[]
  expect: {
    items: GoldenRow[]
    tombstones: Record<string, boolean>
    tombstone_count: number
    absent: string[]
    ack_unavailable: boolean
    outcomes: Record<string, string[]>
  }
}

export interface QueuedInputGolden {
  tombstone_cap: number
  cases: GoldenCase[]
}

export const queuedInputGolden = JSON.parse(goldenSource) as QueuedInputGolden

export function goldenRowOf(entry: QueuedEntry): GoldenRow {
  const row: GoldenRow =
    entry.phase.state === 'queued'
      ? { id: entry.id, text: entry.text, state: 'queued', cancel: null }
      : {
          id: entry.id,
          text: entry.text,
          state: 'cancelling',
          cancel: { answer: entry.phase.answer, vendor_closed: entry.phase.vendorClosed },
        }
  if (entry.sent) row.sent = true
  return row
}
