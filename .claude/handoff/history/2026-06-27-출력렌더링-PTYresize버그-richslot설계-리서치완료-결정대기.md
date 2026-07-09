# 핸드오프: 출력 렌더링 (dashboard-main) — PTY resize 버그 + RichSlot 설계

**날짜:** 2026-06-27 18:15
**세션 정체성:** `dashboard-main` (orchestra 패널) — engram-dashboard **출력 렌더링 전담**
**상태:** 리서치 완료, 구현 착수 전 **설계 결정 대기**
**모델:** 이 세션은 Sonnet 4.6로 돌았음. 다음 세션은 Opus 4.8(1M)로 전환됨.

---

## 0. 오케스트라 토폴로지 (역할 분담 — 중요)

이 작업은 멀티 패널 협업이다. 내 영역 밖을 건드리지 말 것.

- **dashboard-main (나)** = 출력 렌더링 (슬롯 *내부* 렌더링). TerminalSlot, RichSlot.
- **dashboard2** = 레이아웃 / 메시지 담당. **`SlotPane.tsx` 등 레이아웃 리팩토링 진행 중** → 충돌 주의. 최신 핸드오프 `claude-20260627-s14-layout.md`가 그쪽 것.
- **dashboard-qa** = 빌드/테스트/cdp 실측 **전담**. 직접 돌리지 말고 위탁:
  `orch dashboard-qa "QA 요청: [범위/내용]"` → 결과는 qa가 직접 회신.

**보고:** 설계 결정 필요/블로커/Task 완료 시 → `orch dashboard-main` (메인 오케스트레이터)으로.
오케스트라 규약: 메시지 첫 토큰 `⟁` 수신 시 `I:\Engram\core\workflow\orchestra.md` 읽기. 송신은 `orch <target> "⟁name 본문"`.

---

## 1. 작업 범위 (건드는 곳 / 안 건드는 곳)

**건드는 것 (내 영역):**
- `src/components/slot/TerminalSlot.tsx` — Task 1 버그픽스
- `src/components/slot/RichSlot.tsx` — Task 2 신규 (아직 없음)

**건드릴 수도 있는 것 (조율 필요):**
- `src/components/slot/SlotPane.tsx` — 토글 와이어링 시. **dashboard2 리팩 중이라 충돌 위험** → 리팩 끝나고 붙이거나, dashboard2와 조율 후.

**안 건드리는 것:**
- Rust 백엔드/프로토콜 — **Task 1·Task2-Level1은 백엔드 변경 0**. 이미 `subscribeOutput` → bytes 스트림 존재. 렌더만 다르게.
- 단, Task2-Level2(완전 구현)는 백엔드 CommandSpec 변경 필요 (§4 참조) → 그건 사용자 결정 + ADR + dashboard-main 조율 사항.

---

## 2. Task 1: 터미널 렌더 버그 (PTY 80×24 고정)

**증상:** claude welcome 화면 글자 겹침.

**원인 (확정):**
- `crates/.../agent/manager.rs:36-37` 에 `DEFAULT_COLS=80 / DEFAULT_ROWS=24` 상수 고정. `spawn_session`(L186, L220-230)이 이 값으로 PTY 생성.
- 프론트 `TerminalSlot.tsx`: spawn 후 xterm fit 크기를 PTY에 전파하는 경로가 **없음**.
  - 초기화 effect(deps `[]`, 1회)에서만 `fitAddon.fit()` + ResizeObserver 등록.
  - ResizeObserver는 **컨테이너 크기 *변화* 시에만** 발화 → 새 agentId 배정 시엔 안 울림.
  - subscribe effect(deps `[agentId, epoch]`)는 구독만 하고 `resizePty` 안 보냄.
- → claude가 80칸 기준 welcome 박스 렌더, xterm은 실제(더 넓은) 크기 → 커서 위치 어긋나 글자 겹침.

**리서치로 검증:** "PTY 기본크기 spawn → 클라 구독 직후 즉시 resize 전송"이 표준 패턴 (VS Code/ttyd/xterm 공통). spawn 직후 즉시 resize는 SIGWINCH 손실 위험 있으나, 우리는 구독 시점에 보내므로 안전.

**픽스 (확정, 1줄):** `TerminalSlot.tsx` subscribe effect의 `.then(handle => {...})` 안에 추가:
```tsx
void agentClient.resizePty(agentId, terminal.cols, terminal.rows).catch(() => {})
```
(이미 `agentClient.resizePty`, `terminal.cols/rows` 존재. `protocolClient.resizePty`는 fire-and-forget.)

**미착수.** 사용자 승인 후 구현 → dashboard-qa에 cdp 실측(`window.__ENGRAM_AGENT__.resizePty`, 포트 9223) 위탁.

---

## 3. Task 2: RichSlot (터미널 ↔ 구조화 렌더 토글) — 설계 결정 대기

**개념:** 같은 출력을 xterm 대신 구조화(코드블록 강조 / 섹션 접기) 렌더. TerminalSlot과 **병렬 슬롯 타입**(대체 아님). 사용자가 "터미널 모드 / json 모드 토글"로 표현.

**스파이크라 디자인 완벽 불필요.**

### Page 내부 구조 (브리핑한 그림)
```
SlotPane (레이아웃 컨테이너 — dashboard2 영역)
└── 슬롯 (mode 분기)
    ├── mode "terminal" → TerminalSlot → xterm.write(bytes)
    └── mode "rich"     → RichSlot → ContentBlock 타입별 렌더러
                          ├ text       → <Streamdown> (스트리밍 Markdown)
                          ├ tool_use   → <ToolCallCard>
                          ├ tool_result→ <ToolResultCard>
                          └ thinking   → 접기/펼치기
```

---

## 4. ★미결정 사항 (다음 세션이 사용자에게 받아야 할 결정)★

**RichSlot 구현 레벨 — 사용자 선택 대기 중이었음:**

- **Level 1 (스파이크용, 백엔드 변경 0):** PTY 그대로 → RichSlot에서 ANSI 제거 → Markdown 렌더.
  - 코드블록/텍스트는 예쁘게 나오나 **tool_use 구분 불가** (raw 터미널 텍스트라서).
  - 의존성: `strip-ansi`(또는 regex) + Markdown 렌더러.
- **Level 2 (완전 구현, 백엔드 변경 필요):** spawn 시 `claude -p --output-format stream-json` → ContentBlock 파싱 → 타입별 렌더.
  - tool_use/thinking 블록 완전 구분. 단 **백엔드 CommandSpec 변경 + 양 모드 spawn 분기 = 굵은 설계 → ADR + dashboard-main 조율 필요.**

→ **다음 세션 첫 액션: 사용자에게 Level 1/2 중 선택 요청.** (AskUserQuestion 권장. 스파이크 취지 보면 Level 1이 자연스러우나 사용자 결정.)

**토글 UI 위치:** SlotPane에 붙일지 — dashboard2 레이아웃 리팩과 조율 필요. 리팩 후로 미룰지 결정.

---

## 5. 리서치 결론 (압축 — 재조사 불필요)

medium 강도, Claude 팬아웃 + Codex 독립 교차 + 메인 레벨 교차 대조. **본격 적대검증(반증 서브에이전트)은 미실시** — 결론은 "교차 대조로 수렴" 수준이지 적대검증 통과 아님. 큰 결정(특히 Level 2 백엔드 변경) 확정 전엔 핵심 클레임 재확인 권장. 불일치는 Warp 내부 OSC 번호만 유보(설계 무영향).

**상세 보고서:** `docs/research/richslot-rendering-reference-research-2026-06-27.md` (존재 확인됨). study-notes: `.claude/skills/research/study-notes/2026-06-27-richslot-rendering-reference.md`. 다음 세션은 이 파일을 읽으면 됨 — 메인에서 재조사 금지.

**핵심 결론:**
- **Claude Code CLI 출력 2모드 (확실):** 터미널 = PTY interactive(ANSI). 구조화 = `claude -p --output-format stream-json`(NDJSON). 렌더러 차이가 아니라 **spawn 방식 자체가 다름**.
  - stream-json 스키마: `ContentBlock` union = `text` | `tool_use` | `tool_result` | `thinking`. 출처: code.claude.com/docs/en/agent-sdk/streaming-output
- **스트리밍 Markdown 렌더러 (확실):** `streamdown` (Vercel, 미완성 블록 처리 내장, react-markdown drop-in). 현 사실상 표준. `react-markdown`은 누적문자열로 우회 가능(OpenHands 방식).
- **Syntax highlight (확실):** 클라 스트리밍엔 **Prism** (경량·빠름). Shiki는 무거워 부적합(WASM). highlight.js는 부분 chunk autodetect 불안정.
- **ANSI 처리 (확실):** strip-ansi=제거만(stateless, plain text용). ansi-to-html/ansi_up=HTML 변환(stateful 필요). 용도 구분 필수.
- **레퍼런스 (소스 공개):** OpenHands frontend = xterm(raw) + react-markdown(구조화) **완전 독립 인스턴스** 공존. Aider `mdstream.py` = Stable Tail 패턴(터미널 전용). Zed = AssistantMessageChunk/ToolCall 분리(Rust, 차용 불가·패턴 참조).
- **재연결/replay:** 우리 현재 = 데몬 ring buffer + `after_seq` resume로 raw bytes 재생. VS Code는 headless xterm + addon-serialize. 우리 방식 동작하나 스크롤백 크기 비례 파싱비용.

---

## 6. 다음 세션 즉시 할 일 (순서)

1. 이 문서 + `docs/research/richslot-rendering-reference-research-2026-06-27.md` 읽기.
2. 사용자에게 **RichSlot Level 1/2 결정** 요청 (§4).
3. **Task 1 픽스 먼저** (1줄, 백엔드 무관) → 코더 서브에이전트 스폰(구현 실행 규약: 메인 직접 편집 금지) → dashboard-qa 위탁 검증.
4. RichSlot 구현은 Level 결정 후. Level 2면 ADR + dashboard-main 조율.

**구현 실행 규약 리마인드 (CLAUDE.md 강제):** 비자명 코드 변경은 코더(opus/sonnet) 스폰 → `/review` → `/qa`. 메인은 오케스트레이션만. Task1 1줄 픽스는 인라인 예외 가능하나 QA build/test는 돌릴 것.

---

## 7. 이번 세션 메모

- 컨텍스트 과소진 원인: 파악 단계에서 manager.rs(587줄)·pty.rs(527줄) 등 대용량 파일을 **메인에서 직접 읽음** + 리서치 결과를 압축 없이 수신. → 다음 세션은 파악도 Explore 위임해 결론만 회수할 것.
- 리서치 study-notes는 `.claude/skills/research/study-notes/`에 누적됐을 수 있음(학습 장치).
