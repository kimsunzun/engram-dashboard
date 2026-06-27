# RichSlot 렌더링 레퍼런스 조사 보고서

**방법:** Claude(Sonnet) 5-갈래 팬아웃 + Codex blind 교차 + 교차 대조  
**날짜:** 2026-06-27  
**강도:** medium  
**확신도 범례:** 확실 / 가능성 높음 / 불확실  
**목적:** RichSlot 설계 시 "기존 잘 된 코드 위에 올리기" — raw 터미널 + 구조화 렌더링 레퍼런스

---

## 1. Claude Code CLI 출력 포맷

**[확실]** `--print` (`-p`) 플래그가 존재하며, `--output-format`이 세 가지:

| 플래그 조합 | 출력 형태 |
|---|---|
| `claude -p "query"` | text (기본) |
| `claude -p --output-format json "query"` | ResultMessage JSON 1개 |
| `claude -p --output-format stream-json "query"` | NDJSON 스트림 |
| `claude -p --output-format stream-json --include-partial-messages "query"` | 부분 스트리밍 이벤트 포함 |

### stream-json 스키마 — 메시지 union

```typescript
// ContentBlock union — 핵심
type ContentBlock =
  | { type: "text"; text: string }
  | { type: "thinking"; thinking: string; signature: string }
  | { type: "tool_use"; id: string; name: string; input: object }
  | { type: "tool_result"; tool_use_id: string; content?: ContentBlock[]; is_error?: boolean };

// 스트리밍 이벤트 순서
// message_start → content_block_start → content_block_delta (text_delta | input_json_delta)
// → content_block_stop → message_delta → message_stop → AssistantMessage → ResultMessage

// ResultMessage 주요 필드
type ResultMessage = {
  type: "result";
  subtype: "success" | "error_during_execution" | "error_max_turns" | "error_max_budget_usd";
  duration_ms: number; duration_api_ms: number; is_error: boolean;
  num_turns: number; session_id: string; stop_reason?: string;
  total_cost_usd?: number; result?: string; structured_output?: object;
};

// SDKPartialAssistantMessage (stream-json 부분 이벤트)
type SDKPartialAssistantMessage = {
  type: "stream_event";
  event: BetaRawMessageStreamEvent; // Anthropic SDK 원시 이벤트
  parent_tool_use_id: string | null;
  uuid: string; session_id: string;
  ttft_ms?: number; // message_start에만 존재
};
```

**출처:**
- https://code.claude.com/docs/en/cli-reference
- https://code.claude.com/docs/en/agent-sdk/streaming-output

---

## 2. Aider — 터미널 "raw + 구조화" 공존 레퍼런스

**[확실]** Python 기반, `rich` 라이브러리로 스트리밍 Markdown을 터미널에 렌더링.

### 핵심 파일

| 파일 | 역할 |
|---|---|
| `aider/mdstream.py` | `MarkdownStream` — stable tail 패턴 핵심 구현 |
| `aider/io.py` | `InputOutput` — rich.Console 래퍼, 메시지 타입별 출력 분기 |
| `aider/coders/base_coder.py` | LLM 스트리밍 루프, `MarkdownStream.update()` 호출 |
| `aider/coders/chat_chunks.py` | 구조화 프롬프트/메시지 청크 그루핑 |

### 핵심 패턴 — Stable Tail

```
[스크롤백에 고정된 안정 콘텐츠]   ← console.print()로 영구 출력
─────────────────────────────
[Live window (기본 ~6줄)]          ← rich.live.Live.update()로 계속 갱신
  │ 미완성 Markdown 여기 렌더링
```

- 렌더링 라인이 live_window 크기 초과 → 초과분을 스크롤백으로 "flushing"
- 스로틀링: `min_delay`를 동적 조정해 20fps 최소 유지
- `pretty=False`이면 raw delta를 `sys.stdout`에 직접 기록 (rich 우회)

**RichSlot 설계 적용점:** raw 터미널 모드(xterm)와 구조화 렌더링을 모드 플래그 하나로 전환하는 설계 근거.

**출처:**
- https://raw.githubusercontent.com/Aider-AI/aider/main/aider/mdstream.py
- https://raw.githubusercontent.com/Aider-AI/aider/main/aider/io.py
- https://raw.githubusercontent.com/Aider-AI/aider/main/aider/coders/base_coder.py

---

## 3. OpenHands — 웹 UI "터미널 + 구조화" 분리 레퍼런스

**[가능성 높음]** React 기반 SPA. 터미널과 메시지를 **완전히 다른 컴포넌트**로 분리.

### 프론트엔드 스택 (package.json 확인)

```
React 19 + React Router 7 + Vite + Tailwind + Zustand + TanStack Query
+ Socket.IO (실시간 통신)
+ @xterm/xterm + @xterm/addon-fit    ← raw 터미널 출력
+ react-markdown + remark-gfm + remark-breaks  ← 구조화 메시지
+ react-syntax-highlighter           ← 코드블록 하이라이팅
```

### 컴포넌트 구조

```
frontend/src/components/features/
  chat/
    chat-message.tsx         → MarkdownRenderer로 어시스턴트 텍스트 렌더
    event-message.tsx        → 이벤트 타입별 분기:
      isErrorObservation()   → ErrorEventMessage
      isUserMessage()        → UserAssistantEventMessage
      isMcpObservation()     → McpEventMessage
      isFinishAction()       → FinishEventMessage
      (fallback)             → GenericEventMessageWrapper
  terminal/                  → xterm.js 기반 raw 터미널 (독립 컴포넌트)
  diff-viewer/               → 파일 변경 diff 뷰어
  markdown/                  → MarkdownRenderer 구현
```

**출처:**
- https://github.com/OpenHands/OpenHands/blob/main/frontend/package.json
- https://github.com/OpenHands/OpenHands/blob/main/frontend/src/components/features/chat/event-message.tsx

---

## 4. Zed AI Agent Panel

**[가능성 높음]** **React 아님 — Rust + GPUI** (Zed 전용 GPU 가속 UI 프레임워크). 코드 직접 차용 불가, 설계 아이디어만 참조.

### 구조화된 스레드 엔트리 모델

```rust
// crates/agent_ui/src/conversation_view.rs
// 스레드 엔트리를 구조화된 타입으로 관리
AssistantMessageChunk → Markdown 컴포넌트로 렌더
ToolCall { status: ToolCallStatus, content: ToolCallContent } → 별도 tool card로 렌더
```

### 이벤트 드리븐 스트리밍

```
AcpThreadEvent::NewEntry       → 새 스레드 엔트리 추가
AcpThreadEvent::EntryUpdated   → 기존 엔트리 갱신 (스트리밍 청크 도착)
AcpThreadEvent::ToolAuthorizationRequested → 퍼미션 UI 표시
```

**RichSlot 설계 적용점:** "어시스턴트 메시지를 타입 유니온으로 모델링하고, 각 타입마다 전용 렌더러를 배정"하는 아키텍처.

**출처:**
- https://github.com/zed-industries/zed/tree/main/crates/agent_ui/src

---

## 5. React 스트리밍 Markdown 라이브러리

### 5-1. `streamdown` — **[확실] 현재 표준**

```bash
npm install streamdown
```

**핵심 특징:**
- AI 스트리밍 전용 설계 — 미완성 Markdown 블록 우아하게 처리
- `react-markdown` drop-in 대체
- Vercel AI SDK `useChat` 훅과 first-class 통합
- shadcn/ui 기반 디자인 시스템
- 플러그인 아키텍처: `code` / `mermaid` / `math`(KaTeX) / `cjk` 선택 설치
- 최신 버전: v2.5+ (인라인 KaTeX, 스태거 애니메이션, CSV 내보내기)

**기본 사용:**
```tsx
import { Streamdown } from "streamdown";
import { useChat } from "@ai-sdk/react";

export function Chat() {
  const { messages, status } = useChat();
  return messages.map((m) =>
    m.parts.map((part, i) =>
      part.type === "text" ? (
        <Streamdown key={i} isAnimating={status === "streaming"}>
          {part.text}
        </Streamdown>
      ) : null
    )
  );
}
```

**출처:**
- https://github.com/vercel/streamdown
- https://streamdown.ai/

### 5-2. `react-markdown` — **[확실] 생태계 최성숙**

```bash
npm install react-markdown remark-gfm
```

- 스트리밍 전용 아님 — 누적 문자열 전달 방식으로 사용
- `rehype-*` / `remark-*` 플러그인 생태계 광범위
- OpenHands가 이 방식으로 사용 중
- 스트리밍 중 전체 재파싱 = 성능 이슈 (streamdown이 이를 해결)

**출처:** https://github.com/remarkjs/react-markdown

### 5-3. Vercel AI SDK (`ai` + `@ai-sdk/react`) — **[확실]**

```bash
npm install ai @ai-sdk/react
```

- `useChat` 훅: 스트림 상태, 메시지 파트(`text` / `tool-invocation` / `reasoning` / `source`) 관리
- 스트리밍 렌더링 자체는 `streamdown`이나 `react-markdown`에 위임
- tool_use 블록은 `tool-invocation` part로 타입 분리

**출처:**
- https://ai-sdk.dev/docs
- https://ai-sdk.dev/docs/reference/ai-sdk-ui/use-chat

---

## 교차 검증표 (Claude ↔ Codex)

| 클레임 | Claude | Codex | 판정 |
|---|---|---|---|
| `--output-format stream-json` 존재 | 확인 | 확인 | 합의(확실) |
| ContentBlock union (Text/ToolUse/ToolResult) | 확인 | 확인 | 합의(확실) |
| `streamdown` = 스트리밍 MD 표준 | 확인 | 확인 | 합의(확실) |
| OpenHands = xterm + react-markdown 분리 | 확인 | 확인 | 합의(가능성 높음) |
| Zed = React 아님 (Rust/GPUI) | 확인 | 확인 | 합의(확실) |
| Aider mdstream.py stable tail 패턴 | 상세 확인 | 경로 확인 | 합의(확실) |

---

## RichSlot 설계를 위한 핵심 결론

### 1. ContentBlock 타입 유니온 + 전용 렌더러 배정 (Zed/Claude SDK 참조)

```tsx
function renderBlock(block: ContentBlock) {
  switch (block.type) {
    case "text":        return <Streamdown isAnimating={streaming}>{block.text}</Streamdown>;
    case "tool_use":    return <ToolCallCard name={block.name} input={block.input} />;
    case "tool_result": return <ToolResultCard content={block.content} isError={block.is_error} />;
    case "thinking":    return <ThinkingBlock text={block.thinking} />;
  }
}
```

### 2. raw 터미널 ↔ 구조화 렌더링을 독립 컴포넌트로 분리 (OpenHands 참조)

- xterm.js 인스턴스 (raw PTY 출력 전용) ↔ Markdown 메시지 뷰 (구조화 전용)
- capability 플래그로 슬롯이 렌더러를 선택 (ADR-0002/ADR-0030 참조)

### 3. 스트리밍 Markdown = `streamdown` 권장

- 미완성 블록 처리 + `isAnimating` + Vercel AI SDK 통합이 모두 해결
- `react-markdown`은 스트리밍 미지원 → 누적 문자열 방식으로도 사용 가능 (OpenHands 방식)

### 4. Claude Code 연동 = `--output-format stream-json`

- `StreamEvent.event.type === "content_block_start"` 에서 block 타입 감지
- `input_json_delta`로 tool_use 입력 스트리밍 가능

---

## 공백 / 한계

- OpenHands `MarkdownRenderer` 내부 구현 — 소스 접근 제한으로 미확인 (불확실)
- Zed `conversation_view.rs` 전체 로딩 실패 — 세부 렌더링 분기 미확인 (불확실)
- `streamdown` v2.5 이상 API — 문서에서 확인, 소스 private 경로 접근 불가 (가능성 높음)
- Aider `mdstream.py` stable tail live_window 기본값 (~6줄 추정) — 코드 직접 확인 필요 (불확실)
