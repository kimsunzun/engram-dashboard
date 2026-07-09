# 핸드오프: "터미널 검은화면"=데몬 오염이 진짜 원인 → baseline 복원(코드 순변경 0) → 다음 = JSON 렌더(사용자 우선)

> master, HEAD 그대로. 세션 중 시도한 xterm-beta 승격/webgl 애드온/TerminalSlot 편집 **전부 git checkout 원복**. 앱 실행 중·터미널 정상 작동 실측 확인. 이번 세션 큰 소득 = "검은 화면"의 진짜 원인이 렌더러가 아니라 **데몬/orphan 프로세스 오염**(사용자 지적이 정확)이었음을 규명. 다음 = 사용자가 "중요"라 한 **JSON 구조화 렌더**.

## 한 줄 상태 + 다음 첫 액션
터미널은 정상(fresh claude 배너 렌더 실측). 세션 대부분을 터미널 렌더/클리핑 삽질로 썼으나 **그건 헛다리** — 사용자 본류는 **JSON(구조화) 렌더**.
**다음 첫 액션:** JSON 스코핑 — `src/lab/richslot/`(파싱+렌더 스파이크) vs 백엔드(stream-json 스폰·구조화 OutputChunk·capability 렌더러분기) 갭 매핑 → PRD. **핵심 안심 포인트: JSON은 xterm/PTY/webgl 경로를 안 씀**(구조화 데이터→평범한 React 컴포넌트) = 이번에 우릴 문 터미널 gremlin이 JSON엔 전이 안 됨. 오히려 더 견고.

## repo 상태
- **코드 순변경 0.** xterm 6.0.0→6.1-beta 승격 + webgl 애드온 + TerminalSlot 편집을 했다가 **전부 `git checkout package.json package-lock.json src/components/slot/TerminalSlot.tsx`로 원복** + `npm install`로 node_modules 복원. HEAD 그대로.
- working tree = `_wip/`(스크린샷 스크래치, 커밋 금지) + **새 파일** `docs/research/terminal-xterm-render-webview2-2026-07-02.md`(이번 조사 노트 — 커밋 대상). ★`git add -A`/`.` 금지 — 타깃 경로만.★
- 앱 **실행 중**: baseline, 디버그 포트 9223, 백그라운드 task `bxgh8v79y`. 데몬 깨끗, 에이전트/프로필 0(테스트분 proof/fresh2/t3 kill+deleteProfile 정리 완료).

## ★이번 세션 핵심 — "검은 화면"의 진짜 원인 (do-not 반복 금지)★
- 증상: 슬롯에 에이전트 배정해도 터미널이 빈 채 커서만 뜸.
- 나의 오진: renderer(webgl/xterm) 문제로 착각하고 크게 삽질(webgl 애드온·xterm beta 승격 등).
- **진짜 원인:** 세션 내내 앱 반복 재시작 → **orphan claude 프로세스 18개 + 재attach로 엉킨 데몬**(engram-dashboard-daemon 단일 인스턴스가 재시작 간 persist) → 출력 라우팅(구독/replay) 깨짐. 사용자가 "claude 실행이 안 되는 것"이라 정확히 지적함.
- **해결:** engram **데몬만** kill(`Get-Process`로 PID 확인 → `Stop-Process`; 자식 agent는 Job Object로 정리됨) → baseline clean 재기동 → fresh claude 즉시 정상 출력·렌더.
- **운영 수칙:** 빈 슬롯+커서만인데 에이전트 "Running"이면 → 렌더러 의심 **전에** `Get-Process claude` 개수·데몬 상태부터 확인. 반복 재시작이 누적 오염을 만듦. **`claude.exe` 무차별 kill 절대 금지**(내 세션·사용자 다른 claude 세션까지 죽음) — engram 데몬만 죽여 Job Object 정리.

## 클리핑(원래 cosmetic 이슈) — 미해결·보류 (env-blocked)
로고 블록글리프 상단 살짝 깎임(분수 DPI + DOM 렌더러). 사용자 "대충 OK". 정식 픽스=canvas/webgl(customGlyphs)인데 이 환경에서 셋 다 막힘(webgl 텍스처 실패·canvas애드온 xterm6 dead-end·beta 승격 regression). 상세·재검증 포인트 = 위 research 노트.

## 검증 상태 (쌍)
### 확인된 것(green)
- 터미널 렌더/입력: baseline에서 fresh claude 배너 정상(domTextLen 518, 스샷 `_wip/clean-test.png`). 이전에 "안녕" 입력→응답도 됨.
- 데몬-오염 근본원인: 데몬 kill + clean 재기동으로 재현→해소 실측.
### 검증 안 됨(오신뢰 금지)
- **JSON 렌더 경로 전혀 미착수**(스코핑도 안 함).
- **M1(트리 "포커스 슬롯에 배치"→viewStore 재배선) 미구현** — 사람 UI 클릭이 죽은 `slotStore`로 새는 원래 갭 그대로. (사용자가 "우클릭 실행 없어도 됨"이라 우선순위 낮춤.) 참고: `viewStore.assignAgent(view,slot,agent)`=`assign_agent` invoke는 살아있고 cdp로 실증됨.
- **webgl이 이 WebView2에서 "근본 불가"인지 미확정** — 폰트버그+beta 승격과 뒤섞인 실측이라 재검증 필요(research 노트 do-not).

## do-not / 실패한 접근
- **xterm beta 승격 금지** — 6.1-beta로 DOM 렌더까지 깨짐. 렌더러 변경은 baseline에서 한 조각씩 CDP 검증하며.
- **빈 슬롯을 renderer부터 의심 금지** — 데몬/orphan 먼저.
- **CDP 스샷은 WebGL(GPU) canvas 못 잡음** — DOM/canvas만 잡힘. webgl 검증은 사람 눈 필요. 자율작업은 CDP로 잡히는 렌더러(DOM/canvas) 선호.
- 폰트 함정: canvas/webgl에 `fontFamily:'var(--font-terminal)'` 그대로 주면 검음(canvas가 CSS var 못 해석) — 실제 폰트로 해석해 넘겨야(research 노트). DOM은 무관.
- `_wip/` 커밋 금지. `slotStore`(옛 number id, 죽은 경로) vs `viewStore`(UUID, 백엔드권위) 혼동 금지.

## 다음 우선순위 (사용자 명시)
1. **JSON 구조화 렌더(M2) — 사용자가 "중요"라 한 본류.** lab richslot(렌더 절반 완성) 재사용 + 백엔드 stream-json 스폰·구조화 OutputChunk·capability 렌더러분기(굵은 설계+ADR). 첫 조사: claude 대화형(멀티턴) stream-json I/O 실동작(`--output-format/--input-format stream-json`) — 추측 금지, 스파이크/조사.
2. (낮음) M1 사람 UI 재배선 · 클리핑 재도전.

## 참조 (읽을 것만)
- 이번 조사·실측 상세: `docs/research/terminal-xterm-render-webview2-2026-07-02.md`
- JSON: `src/lab/richslot/README.md`·`types.ts`·`parse.ts` · 렌더러분기 갭 = `src/components/layout/ViewLayoutRenderer.tsx`(agent_id 하드코딩) · `src/api/types.ts`(OutputCaps 미사용)
- 터미널 배관: `src/components/slot/TerminalSlot.tsx`(구독→xterm) · `src/store/viewStore.ts`(assign_agent invoke) · cdp 제어핸들 `window.__engramLayout`·`window.__ENGRAM_AGENT__`(메서드: spawnAgent/killAgent/createClaudeProfile/spawnProfile/deleteProfile/assignAgent 등)
- ADR: 0002/0030(출력 capability=렌더러 선택) · 0029(데몬=에이전트 호스트) · 0035(레이아웃 권위)
