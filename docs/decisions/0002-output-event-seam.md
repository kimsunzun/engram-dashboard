# ADR-0002: 출력 seam = OutputEvent (터미널 가정 금지)

- 상태: 확정 (S10 백엔드 추상화)
- 관련: CLAUDE.md §2 · `types.rs::OutputEvent` · `Capabilities.output`

## 맥락
멀티 백엔드(claude/codex 콘솔 + API)의 출력을 무엇으로 모델링할지. 콘솔은 터미널 바이트지만 API는 구조화된 텍스트/usage/tool call이다.

## 결정
`OutputEvent`를 **확장 enum**으로 둔다 — 터미널 바이트는 한 variant일 뿐이고, API는 `TextDelta`/`Usage`/`ToolCall` 등. 출력 종류는 `capabilities.output`(terminal_bytes/markdown/tool_events/usage)로 구분하고, 슬롯이 그에 맞는 렌더러를 고른다(터미널=xterm / API=구조화·마크다운 뷰).

## 거부한 대안
- **"API도 가상 터미널에 물려 같은 바이트 스트림으로"** — S10 이전 가정. **폐기.** API는 터미널이 아닐 수 있어 바이트 강제는 구조화 출력(usage/tool call)을 표현 못 한다.

## 근거
장기 멀티백엔드 전제(§0 저위험·장기 → 미리 깐다). seam을 터미널에 묶으면 API 모델 도입 때 전면 재설계.

## 영향 / 불변식
- **출력은 종류를 가정하지 않는다(터미널 강제 금지).**
- API variant(TextDelta/Usage 등) 본체는 API 모델 등장 때 채움(현재 enum 자리만).
