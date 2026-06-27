# RichSlot Lab (스파이크)

격리 실험 공간. 메인 코드(SlotPane 등 레이아웃 리팩 진행 중)와 분리해 **RichSlot**(구조화 렌더 슬롯)을 독립 개발한다. 완성되면 컴포넌트를 메인 슬롯 시스템에 통합(import)한다. (ADR-0012 모듈 격리.)

## 이게 뭔가
Claude 출력을 두 모드로 본다 — 터미널(raw, 기존 `TerminalSlot`) vs **JSON(구조화, 이 RichSlot)**. JSON 모드는 `claude -p --output-format stream-json` 출력을 파싱해 코드블록·tool 호출·thinking 을 구조로 렌더한다. 리버트·diff·접기 같은 기능의 토대.
- `Rich` = rich text(서식 있는) 의 rich. 컴포넌트·kind 는 `rich`, 사용자 향 토글 레이블은 "JSON".

## 실행
- `npm run dev:richslot` → http://localhost:1430 (별도 포트, 메인 dev 1420 과 무관)
- `npm test` → `parse.test.ts` (vitest 자동 수집)

## 구조 (층 분리 — 통합·교체 대비)
- **파싱층** (프레임워크 무관 순수 TS): `types.ts`(ContentBlock) · `parse.ts`(stream-json → RichMessage[])
- **렌더층** (React): `RichSlot.tsx` · `blocks.tsx` · `richslot.css`
- `main.tsx` — 독립 entry(fixture 토글 → 파싱 → 렌더)
- `fixtures/*.jsonl` — **실측** `claude -p --output-format stream-json` 캡처
  - `text` = 텍스트 응답만 · `tool` = thinking+tool_use+tool_result+text · `partial` = `--include-partial-messages` 델타 스트림

## 입력 형식 (실측)
assistant/user 라인의 `message.content[]` = ContentBlock 배열. 4종: `text` / `thinking` / `tool_use` / `tool_result`. Anthropic Messages API 그대로. 상세는 `types.ts` 주석.

## 통합 계획
- RichSlot 입력은 `RichMessage[]` 인터페이스만 의존 → mock fixture ↔ 실제 데몬 스트림 교체로 끝.
- 메인 통합 시: `slotStore.SlotContent` 에 `{ kind: 'rich' }` 추가 + `SlotPane` 분기 + 토글 UI("Terminal / JSON").
- 백엔드측: `claude -p --output-format stream-json` spawn 경로 + `OutputChunk` 구조화 variant 연결(dashboard-main 영역, 굵은 설계 → ADR).

## 후속 (골격에 살 붙이기)
- TextBlock: `streamdown`(스트리밍 Markdown) + Prism(코드블록 강조) — 의존성 추가 시점
- ToolUseBlock: Edit/Write → 리버트 버튼 + diff 뷰(`input.old_string/new_string`)
- partial 델타 모드 파서(현재 통짜 모드만)
