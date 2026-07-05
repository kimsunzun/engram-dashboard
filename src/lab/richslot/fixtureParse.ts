// RichSlot fixture 파서 — stream-json NDJSON → RichMessage[] (순수 TS, React 무관, fixture 통짜 전용).
//
// ★fixture 전용(라이브 아님)★: 캡처한 stream-json 샘플(fixtures/*.jsonl)을 스타일 튜닝용으로 통짜 파싱한다.
//   라이브 구조화 출력은 백엔드가 정제한 tag1 StructuredEvent 로 흐르고(ADR-0045), 그 누적은
//   components/slot/structuredAccumulator.ts 가 한다. 이 파서는 살아있는 에이전트/데몬 없이 도는
//   FixtureRichSlot·lab 진입점만 쓴다. (구 lab/richslot/parse.ts·streamParse.ts 는 S15 에서 제거됨 — F5.)
//
// ★층 분리★: 이 함수는 claude stream-json 형식만 안다. 렌더층(layouts)은 RichMessage[] 만 받으므로
//   입력 소스가 fixture 든 무엇이든 동일하게 돈다. claude 가 형식을 바꾸면 여기만 고친다.

import type { ContentBlock, RichMessage } from './types'

/**
 * stream-json NDJSON 텍스트 → 렌더 가능한 RichMessage 목록(fixture 통짜 파싱).
 * - assistant/user 라인만 메시지로 수집(그 안 message.content[] = ContentBlock 배열).
 * - system/init·result·rate_limit_event 등 메타 라인, 빈 줄, 비-JSON(stderr 혼입)은 안전하게 스킵.
 * - 라인마다 새 메시지로 append 만 한다(id 병합 X — fixture 는 완료 세션이라 병합 불필요).
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
    const rec = obj as { type?: string; message?: { id?: string; content?: unknown } }
    if (rec.type !== 'assistant' && rec.type !== 'user') continue // result·메타 라인 스킵
    const content = rec.message?.content
    if (!Array.isArray(content)) continue
    messages.push({ role: rec.type, blocks: content as ContentBlock[], id: rec.message?.id })
  }
  return messages
}
