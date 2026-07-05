// RichSlot 파싱층 — ContentBlock 타입 (실측 기반, 프레임워크 무관).
//
// 출처: 실제 `claude -p --output-format stream-json` 캡처(fixtures/*.jsonl).
// Anthropic Messages API 의 message.content[] 를 그대로 따른다 — claude 가 형식을
// 바꾸면 여기만 고치면 된다(렌더층은 ContentBlock 만 의존, claude 를 모름).

/** assistant/user 메시지의 content[] 한 칸. 실측 4종(text/thinking/tool_use/tool_result). */
export type ContentBlock =
  | { type: 'text'; text: string }
  | { type: 'thinking'; thinking: string; signature?: string }
  | { type: 'tool_use'; id: string; name: string; input: Record<string, unknown> }
  // tool_result 는 user 메시지 안에 실려 온다. content 는 string(Read 결과 등) 또는 배열.
  | {
      type: 'tool_result'
      tool_use_id: string
      content: string | unknown[]
      is_error?: boolean | null
    }

/** 정규화된 한 메시지 — role + 블록들. system/init·result·rate_limit_event 메타 라인은 제외. */
export interface RichMessage {
  role: 'assistant' | 'user'
  blocks: ContentBlock[]
  /**
   * assistant 메시지의 `message.id`(Anthropic API 메시지 id). fixture 통짜 파서(fixtureParse)가
   * 채우는 병합/식별 키(있으면 CardLayout 등이 tool_use↔result 를 id 로 페어링). 라이브 tag1 경로는
   * 이 타입을 안 쓰므로(structuredAccumulator 는 StructuredItem 산출) fixture 렌더에만 의미. optional —
   * 렌더층(layouts)은 이 필드에 의존하지 않는다.
   */
  id?: string
}
