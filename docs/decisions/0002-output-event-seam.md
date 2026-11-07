# ADR-0002: Output seam = OutputEvent (no terminal assumption)

- 상태: 확정 (S10 backend abstraction)
- 관련: CLAUDE.md 「백엔드 확장」 · `types.rs::OutputEvent` · `Capabilities.output`

## 맥락
What should the output of multiple backends (claude/codex console + API) be modeled as? A console produces terminal bytes, but an API produces structured text/usage/tool calls.

## 결정
`OutputEvent` is an **extensible enum** — terminal bytes are just one variant, and the API uses `TextDelta`/`Usage`/`ToolCall` and so on. The kind of output is distinguished by `capabilities.output` (terminal_bytes/markdown/tool_events/usage), and the slot picks the renderer that fits it (terminal = xterm / API = structured/markdown view).

## 거부한 대안
- **"Attach the API to a virtual terminal too and use the same byte stream"** — the assumption before S10. **Withdrawn.** An API may not be a terminal, so forcing bytes cannot express structured output (usage/tool call).

## 근거
The premise is a long-term multi-backend design (§0 low risk, long term → lay it down in advance). If the seam is tied to the terminal, introducing an API model means a full redesign.

## 영향 / 불변식
- **Output does not assume its kind (forcing a terminal is forbidden).**
- The body of the API variants (TextDelta/Usage etc.) is filled in when an API model appears (for now, only the enum slot).
