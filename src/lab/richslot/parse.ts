// RichSlot 파싱층 — stream-json NDJSON → RichMessage[] (순수 TS, React 무관).
//
// 통짜 모드(`--output-format stream-json`, partial 없음) 전용. 각 라인이 완성된 JSON
// 객체다(message_start/content_block_delta 같은 증분 이벤트 없음). partial 델타 모드
// (`--include-partial-messages`)는 후속 단계 — 여기선 통짜만(단위테스트 입력이 가장 단순).
//
// ★층 분리 핵심★: 이 함수는 claude stream-json 형식만 안다. 렌더층(RichSlot)은 결과인
// RichMessage[]만 받으므로, 입력 소스가 mock fixture든 실제 데몬 스트림이든 동일하게 돈다.
//
// ★단일 라인 파서(parseStreamLine)★: 통짜 파서(parseStreamJson, fixture 용)와 라이브 누산기
// (streamParse.ts)가 **한 벌의 라인 해석**을 공유하도록 라인 1개 해석을 여기로 뽑았다.

import type { ContentBlock, RichMessage } from './types'

/** 한 NDJSON 라인의 해석 결과. `message`=렌더 대상 / `result`=턴 종료 신호 / null=그 외(메타·비JSON). */
export type ParsedStreamLine =
  | { kind: 'message'; role: 'assistant' | 'user'; id?: string; blocks: ContentBlock[] }
  | { kind: 'result' }

/**
 * NDJSON 한 라인 → 파싱 결과.
 * - assistant/user 라인 → `{ kind:'message', role, id?, blocks }` (message.content[] = ContentBlock 배열).
 * - result 라인 → `{ kind:'result' }` (턴 종료 신호 — 라이브 입력 UX 용, 렌더 대상 아님).
 * - system/init·rate_limit_event 등 메타 라인, 빈 줄, 비-JSON(예: stderr 경고) → null.
 * id 는 assistant 병합 키(message.id) — 라이브 누산기가 같은 id 라인 블록을 이어붙이는 데 쓴다.
 */
export function parseStreamLine(line: string): ParsedStreamLine | null {
  const trimmed = line.trim()
  if (!trimmed) return null
  let obj: unknown
  try {
    obj = JSON.parse(trimmed)
  } catch {
    return null // 경고/비-JSON 라인 — 무시
  }
  const rec = obj as { type?: string; message?: { id?: string; content?: unknown } }
  if (rec.type === 'result') return { kind: 'result' }
  if (rec.type !== 'assistant' && rec.type !== 'user') return null
  const content = rec.message?.content
  if (!Array.isArray(content)) return null
  return {
    kind: 'message',
    role: rec.type,
    id: rec.message?.id,
    blocks: content as ContentBlock[],
  }
}

/**
 * stream-json NDJSON 텍스트 → 렌더 가능한 RichMessage 목록(fixture 통짜 파싱용).
 * - assistant/user 라인만 메시지로 수집(그 안 message.content[] = ContentBlock 배열).
 * - system/init·result·rate_limit_event 등 메타 라인은 스킵(렌더 대상 아님).
 * - 비-JSON 라인(예: "Warning: no stdin..." stderr 혼입)도 안전하게 스킵.
 * ★라이브 아님★: 라인마다 새 메시지를 append 만 한다(id 병합 X). 라이브 누산은 streamParse.ts.
 */
export function parseStreamJson(ndjson: string): RichMessage[] {
  const messages: RichMessage[] = []
  for (const line of ndjson.split('\n')) {
    const parsed = parseStreamLine(line)
    if (parsed && parsed.kind === 'message') {
      messages.push({ role: parsed.role, blocks: parsed.blocks, id: parsed.id })
    }
  }
  return messages
}
