# 핸드오프: 출력 렌더링 (dashboard-main) — 터미널 픽스 완료 + RichSlot 랩 4레이아웃

**날짜:** 2026-06-27 (밤) · **세션:** `dashboard1`/dashboard-main 계열 — engram-dashboard **출력 렌더링 전담**
**모델:** Opus 4.8 (1M). 컨텍스트 ~51%(스크린샷 비중 큼 — 다음 세션은 snapshot 텍스트 위주로).
**이전 핸드오프 대체:** `2026-06-27-출력렌더링-PTYresize버그-richslot설계-리서치완료-결정대기.md` (그 이후 전부 진행됨).

---

## 0. 오케스트라 토폴로지 (역할 분담)

- **나(dashboard1)** = 출력 렌더링(슬롯 *내부*): TerminalSlot, RichSlot/랩.
- **dashboard2** = 레이아웃/메시지. **S14 레이아웃 리팩 진행 중**(TRD 리뷰 단계, 아직 코드 안 들어감). `SlotPane.tsx`·`SlotContextMenu.tsx` 공통 수정 대상 → **건드리기 전 핑 약속함**.
- **dashboard-qa** = 빌드/테스트/cdp 위탁처. ★**현재 orch 통신 두절**(wezterm `os error 232` 반복 — qa·dashboard2 둘 다 송신 실패). 그래서 self-check(tsc/test)·실측(chrome-devtools)을 **직접** 수행 중. dashboard2 답신은 사용자가 중계해줌.
- 오케스트라 규약: 첫 토큰 `⟁` → `I:\Engram\core\workflow\orchestra.md` 읽기. 송신 `orch <target> "⟁name 본문"`.

---

## 1. Task 1 (터미널 PTY resize 버그) — ★코드 완료★

**증상:** claude welcome 박스가 좁은 슬롯(좌우 split)에서 깨짐. **원인:** spawn 시 PTY 80×24 고정 + 구독 직후 초기 resize 전송 없음(ResizeObserver는 크기 *변화* 시에만 발화).

**픽스(적용됨):** `src/components/slot/TerminalSlot.tsx` 구독 effect `.then(handle)` 안에:
```tsx
fitAddonRef.current?.fit()
void agentClient.resizePty(agentId, terminal.cols, terminal.rows).catch(() => {})
```
= gotty 패턴(구독 직후 resize 1회). OSS 코드(ttyd/gotty, `I:/Engram_Workspace/references/`)로 검증. tsc 0 + 78 tests PASS. 랩에서 전파 실측(폭 100%→50% 시 134→66×46 확인).

**dashboard2 확정(resize 경로):** 현행 WS직결 `protocolClient.resizePty` 그대로. Phase B(TauriTransport, ADR-0036)에서도 인터페이스·의미론 불변 → **carry-forward 보장**(주석에 박음). 멀티뷰 크기충돌 정책(tmux식)은 Phase B·src-tauri 권위 = Task1 범위 밖.

**남은 1개:** 실 claude welcome redraw 최종 실측 = 메인 앱(`npm run tauri dev`) 필요 → **dashboard2 S14 리팩 끝난 뒤**. (TerminalSlot은 SlotPane과 별개 파일이라 충돌 없음.)

---

## 2. RichSlot 랩 — 출력 실험실 (영구 활용)

격리 랩(`src/lab/`). 메인 코드와 분리해 렌더링/컬러/레이아웃 실험. **dev server: `npm run dev:richslot` → http://localhost:1430/richslot.html** (포트 1430, Tauri 없이 순수 브라우저).

```
src/lab/
├── main.tsx              # entry: Terminal/JSON 토글 + width(100/50/30%) 토글 + layout 탭
├── terminal/
│   ├── TerminalView.tsx  # xterm + FitAddon, gotty resize 패턴(마운트 직후 resize)
│   └── fixtures.ts       # ANSI 컬러 합성 샘플
└── richslot/
    ├── types.ts          # ContentBlock(text/thinking/tool_use/tool_result) — 실측 기반
    ├── parse.ts          # stream-json NDJSON → RichMessage[] (파싱층, 순수 TS)
    ├── parse.test.ts     # 4 테스트 (vitest)
    ├── layouts.tsx       # ★레이아웃 후보 4종★ (렌더층)
    ├── layouts.css       # --lay-* CSS 변수 (컬러 커스텀 토대)
    ├── README.md
    └── fixtures/         # 실측 claude stream-json 캡처
        ├── text.jsonl    # 텍스트 응답
        ├── tool.jsonl    # thinking+tool_use+tool_result+text
        └── partial.jsonl # --include-partial-messages 델타
```

**★층 분리(ADR-0012)★:** 파싱층(types/parse, 프레임워크 무관) ↔ 렌더층(layouts, React). 입력은 `RichMessage[]` 인터페이스만 → mock fixture ↔ 실제 데몬 스트림 교체로 끝.

**레이아웃 4종(layouts.tsx, 탭 전환):** `timeline`(좌측 컬러바+행+접기) · `stream`(풀너비 흐름, tool 인라인) · `tlog`(터미널 모노 고밀도) · `card`(tool_use↔tool_result를 id로 페어링한 카드). 4개 다 같은 fixture를 다르게 렌더. blocks.tsx/RichSlot.tsx(구 단일스타일)는 삭제됨.

---

## 3. S14 레이아웃 구조 (dashboard2, ADR-0035/0036) — 내 작업의 전제

- **레이아웃 권위 = src-tauri Rust(ViewManager).** 데몬 = 에이전트만(View 일절 모름).
- **모든 트래픽 src-tauri 단일 choke point.** 창은 src-tauri하고만 IPC.
- **렌더링은 WebView(React/xterm) 유지** — Rust는 레이아웃 *관리*만, 그리기는 창. → **내 랩(React) 유효**.
- **출력:** Phase A=현행 WS직결 / Phase B=src-tauri OutputRouter가 agentId로 해당 창 라우팅.
- ★**파싱층(ADR-0002 capability.output) 무영향**★ — 출력 '라우팅'만 바뀌고 파싱 로직 불변. 내가 층 분리한 게 정확히 맞음.
- 슬롯 = `LayoutNode.Slot{id:Uuid, agent_id:Option<String>}`, **slotId number→UUID**(SlotPane 영역, TerminalSlot은 agentId만 받아 무관).
- 상세: `docs/process/S14-multi-page-layout/trd.md` rev.4 + `docs/decisions/0035,0036`.

**패턴 A/B 결론:** 터미널 resize는 **패턴 B(뷰 붙을 때 resize) 구조적 확정.** 패턴 A(client-first spawn 크기)는 "데몬=View 모름"(ADR-0035) 위반이라 불가.

---

## 4. 시장조사 (research, Claude+Codex 교차, 2026-06-27)

AI 출력 UI 레이아웃 5대 유형: 채팅버블 / 블록카드(Warp) / 타임라인(OpenHands) / 노트북셀(Jupyter) / IDE패널(Cursor·Zed). **좁은 슬롯 Top: 두 family 모두 타임라인형 1위** + 풀너비스트림/터미널로그. study-notes: `.claude/skills/research/study-notes/2026-06-27-richslot-rendering-reference.md` 등.

**유명도/유저반응(가벼운 조사, 확신도):** Cursor=IDE 표준·최고 인기(가능성높음) / Claude Code=터미널, 개발자 급상승(가능성높음) / Claude.ai·ChatGPT=풀너비스트림, 대중 최다(확실). **Claude 데스크톱 앱 = 풀너비 스트림형.**

---

## 5. ★현재 위치 + 다음 작업★

**막힌 지점:** 사용자가 4개 레이아웃에 **공감이 안 됨** — 이유는 **fixture가 빈약**(현재 `tool.jsonl` = package.json Read 하나뿐). 레이아웃 차이가 체감되려면 **풍부한 예제**(코드 작성, git diff, 연속 tool, 긴 텍스트)가 필요.

**다음 세션 즉시 할 일:**
1. **풍부한 fixture 만들기** — 실제 claude에게 "코드 파일 작성 + git diff 보기" 류 작업(read-only 안전 범위 권장, 또는 합성)을 시켜 stream-json 캡처. `claude -p --output-format stream-json --verbose --model claude-haiku-4-5-20251001`(haiku=싸다). Edit/Write/Bash(git) tool_use + 코드블록 + diff가 담긴 fixture를 `src/lab/richslot/fixtures/`에 추가.
2. 그걸로 4개 레이아웃 **재비교** → 사용자가 취향으로 **레이아웃 최종 선택**(사용자 결정 사항).
3. 선택된 레이아웃에 **살 붙이기**: Markdown 렌더(streamdown — ★의존성 추가라 package.json = dashboard2 조율 필요, 또는 자체 미니 마크다운 먼저) + 코드블록 강조(Prism) + 접기 정책 + 리버트/diff 버튼(tool_use.id↔tool_result.tool_use_id 페어링 활용) + 풀 컬러 커스텀(--lay-* 변수).

**미결정(사용자):** 레이아웃 최종 선택(풍부 fixture 본 후) · streamdown 의존성 추가 시점.

---

## 6. 환경/주의

- **dev server**: `npm run dev:richslot` 백그라운드 실행 중(bnbu5drol). 죽었으면 재시작.
- **Chrome 실측**: chrome-devtools MCP가 9222 필요 → 별도 프로필로 띄워둠(`Start-Process chrome --remote-debugging-port=9222 --user-data-dir=C:\Temp\chrome-lab-profile`). `list_pages`로 확인.
- **orch 두절**: wezterm 232 — qa/dashboard2 송신 불가. 사용자 중계 필요.
- **커밋 안 함**: master 브랜치, dashboard2가 같은 트리에서 S14 리팩 중. 터미널 픽스(TerminalSlot.tsx)+랩 커밋은 dashboard2 조율 후 브랜치 따서.
- **claude stream-json 형식 실측 요약**: `assistant`/`user` 라인의 `message.content[]` = ContentBlock 배열(text/thinking/tool_use/tool_result). `--include-partial-messages`면 content_block_delta 스트림. 상세 `src/lab/richslot/types.ts` 주석.

---

## 7. 핸드오프 종료 체크 (CLAUDE.md)
- 새 ADR: 내 영역(View 스파이크) 신규 ADR 없음. 패턴 B/resize 경로는 dashboard2 ADR-0036 열린사항에 박힘.
- step-log: 랩 스파이크라 미기록(핸드오프로 갈음) — 정식 통합 시 step-log 추가.
