// 테스트 전용 — agent 환원기와 공유하는 골든(`queued_input_golden.json`)을 **경로에서** 읽는다(ADR-0231).
// ★사본을 두지 않는다★ — 사본이면 Rust 쪽 골든만 고쳐도 TS 시험이 초록으로 남아 두 판이 조용히 갈라진다.
// 파일 형식의 정본은 그 파일 머리의 `_format` 이다.

import goldenSource from '../../../../crates/engram-dashboard-agent/src/queued_input_golden.json?raw'
import type { QueuedInputEvent } from '../../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import type { QueuedEntry } from '../queuedInputReducer'

/** 골든 행 = 목록 조회 행과 같은 낱말(`state` · `cancel`). */
export interface GoldenRow {
  id: string
  text: string
  state: 'queued' | 'cancelling'
  cancel: null | { answer: 'none' | 'not_removed'; vendor_closed: boolean }
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
  return entry.phase.state === 'queued'
    ? { id: entry.id, text: entry.text, state: 'queued', cancel: null }
    : {
        id: entry.id,
        text: entry.text,
        state: 'cancelling',
        cancel: { answer: entry.phase.answer, vendor_closed: entry.phase.vendorClosed },
      }
}
