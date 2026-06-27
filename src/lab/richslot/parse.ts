// RichSlot 파싱층 — stream-json NDJSON → RichMessage[] (순수 TS, React 무관).
//
// 통짜 모드(`--output-format stream-json`, partial 없음) 전용. 각 라인이 완성된 JSON
// 객체다(message_start/content_block_delta 같은 증분 이벤트 없음). partial 델타 모드
// (`--include-partial-messages`)는 후속 단계 — 여기선 통짜만(단위테스트 입력이 가장 단순).
//
// ★층 분리 핵심★: 이 함수는 claude stream-json 형식만 안다. 렌더층(RichSlot)은 결과인
// RichMessage[]만 받으므로, 입력 소스가 mock fixture든 실제 데몬 스트림이든 동일하게 돈다.

import type { ContentBlock, RichMessage } from './types'

/**
 * stream-json NDJSON 텍스트 → 렌더 가능한 RichMessage 목록.
 * - assistant/user 라인만 메시지로 수집(그 안 message.content[] = ContentBlock 배열).
 * - system/init·result·rate_limit_event 등 메타 라인은 스킵(렌더 대상 아님).
 * - 비-JSON 라인(예: "Warning: no stdin..." stderr 혼입)도 안전하게 스킵.
 */
export function parseStreamJson(ndjson: string): RichMessage[] {
  const messages: RichMessage[] = []
  for (const line of ndjson.split('\n')) {
    const trimmed = line.trim()
    if (!trimmed) continue
    let obj: unknown
    try {
      obj = JSON.parse(trimmed)
    } catch {
      continue // 경고/비-JSON 라인 — 무시
    }
    const rec = obj as { type?: string; message?: { content?: unknown } }
    if (rec.type !== 'assistant' && rec.type !== 'user') continue
    const content = rec.message?.content
    if (!Array.isArray(content)) continue
    messages.push({ role: rec.type, blocks: content as ContentBlock[] })
  }
  return messages
}
