# Engram 대시보드 — 기술 조사 결과

*2026-06-01 서브에이전트 7개 병렬 조사 (206K 토큰)*

---

## 1. 훔칠 만한 UX 패턴

**AgentHub의 FileWatcher → 반응형 트리**
에이전트 상태를 파일로 관리하고 chokidar가 변경 감지 → Zustand store 갱신 → Tree/Panel 자동 업데이트. DB 없이 파일이 source of truth. Engram의 `orchestra.md` 기반 아키텍처와 자연스럽게 맞음.

**Claude Squad의 yolo/auto-accept 모드**
"사람 확인 없이 자동 실행" 토글. 대시보드 하단 Monaco DiffEditor의 Accept 버튼과 쌍으로 — 토글 ON이면 diff를 표시만 하고 자동 accept, OFF면 수동 리뷰 게이트.

**VoltAgent의 4ms 코얼레스 패턴**
PTY stdout을 첫 바이트 도착 후 4ms 더 모아서 한 번에 xterm에 write. burst를 하나의 chunk로 묶어 렌더링 frame drop 방지.

**OpenHands의 accumulated_cost 실시간 표시**
각 에이전트 패널 헤더에 토큰/비용 카운터. status bar에 전체 합산도 표시.

**AgentsRoom의 Dynamic Island 스타일 플로팅 상태**
항상 위에 떠 있는 글로벌 상태 위젯 (몇 개 실행 중 / 오류 있음). 대시보드 최상단 고정 바로 구현.

**Composio AO의 CI 실패 → 자동 해당 에이전트 포커스**
에러 감지 시 트리 노드 빨간색 + 자동으로 해당 패널로 포커스 이동. PTY onExit 코드가 0이 아닐 때 트리거.

---

## 2. 기술 스택 최종 추천

| 레이어 | 선택 | 이유 |
|---|---|---|
| 데스크톱 래퍼 | Tauri v2 | 고정 |
| UI 프레임워크 | React 19 + TypeScript | 고정 |
| 빌드 툴 | Vite | Tauri 공식 템플릿 기본값 |
| 스타일링 | TailwindCSS v4 + shadcn/ui | 커스텀 색상 토큰으로 에이전트 상태색 관리 |
| 상태 관리 | **Zustand** | 3KB, FileWatcher 패턴과 자연스럽게 연동, `@tauri-store/zustand` 공식 존재 |
| 에이전트 트리 | **react-arborist** | 가상화 기본, `openAll/closeAll` API, MIT |
| 터미널 | @xterm/xterm + react-xtermjs | 다중 인스턴스는 `key` prop으로 격리 |
| 패널 분할 | **allotment** | VS Code 코드베이스 파생, 동적 pane 선언형, 중첩 분할 가능 |
| diff 에디터 | @monaco-editor/react DiffEditor | 고정. loader.config 로컬 번들 필수 |
| PTY 관리 | **portable-pty (Rust 직접)** | 다중 인스턴스 + Windows ConPTY 안정성. tauri-plugin-pty는 단일 인스턴스만 |
| Rust→TS 타입 | **tauri-specta** | invoke 시그니처 자동 TS 타입 생성 |
| 패널 리사이즈 감지 | ResizeObserver (직접) | allotment onResize + fitAddon.fit() 연결 |

---

## 3. 예상 구현 난이도 & 리스크

| 컴포넌트 | 난이도 | 주요 리스크 |
|---|---|---|
| Tauri 기본 셋업 + capabilities | 낮음 | v2 ACL 파일 누락 시 invoke 전부 막힘 |
| PTY spawn (단일) | 낮음 | Windows cwd 역슬래시 변환 누락 |
| **PTY 다중 인스턴스 관리** | **높음** | ConPTY 동시 생성/소멸 race condition. 전역 Mutex 직렬화 필수 |
| xterm.js React 다중 인스턴스 | 중간 | Strict Mode 2회 effect, dispose 타이밍 |
| allotment 동적 패널 | 낮음 | pane 추가 시 FitAddon.fit() 재호출 필요 |
| react-arborist 트리 | 낮음 | Zustand store → data prop 파생 함수 주의 |
| Monaco DiffEditor (Tauri 내) | 중간 | CDN 로딩 차단. 로컬 번들 설정 안 하면 오프라인 빈 화면 |
| Monaco Accept/Revert | 중간 | getLineChanges()가 마운트 직후 null. onDidUpdateDiff 이후에만 호출 가능 |
| PTY → xterm 스트리밍 | 중간 | Channel 타입 맞추기, 4ms 코얼레스 버퍼 구현 |
| Windows 전체 빌드 | 중간 | WebView2, ConPTY Job Object 없으면 orphan 프로세스 |

---

## 4. 시작 순서

**Phase 1 — 뼈대 (1~2일)**
1. `npm create tauri-app` → React + TypeScript
2. capabilities/default.json 최소 권한
3. Vite + TailwindCSS + shadcn/ui
4. tauri-specta 설치, 더미 invoke로 타입 생성 확인

**Phase 2 — PTY 싱글 인스턴스 (2~3일)**
5. `portable-pty` Cargo 의존성
6. `pty_open` / `pty_write` / `pty_resize` / `pty_close` 커맨드 (단일 세션)
7. xterm.js 연결, Channel 스트리밍 확인
8. Windows cwd 역슬래시 문제 해결 확인

**Phase 3 — 다중 PTY + 패널 (2~3일)**
9. `HashMap<u32, Arc<Session>>` 다중 세션 구조
10. 전역 ConPTY Mutex
11. allotment 동적 n분할 패널
12. 패널 추가/제거 시 PTY spawn/close 연동

**Phase 4 — 에이전트 트리 (1~2일)**
13. Zustand agentStore (id, name, status, ptyId)
14. react-arborist NodeRenderer 상태별 아이콘/색상
15. Rust `agent-status` emit → listen → store → tree 반영

**Phase 5 — Monaco DiffEditor (1~2일)**
16. `loader.config({ monaco })` 로컬 번들
17. `vite-plugin-monaco-editor` 언어 워커 지정
18. `onDidUpdateDiff` 이후 Accept/Revert 버튼

**Phase 6 — UX 마감 (1일)**
19. 글로벌 상태 바 (실행 중 수, 오류 여부)
20. yolo 모드 토글 (diff 자동 accept)
21. 비정상 종료 시 트리 노드 빨간색 + 포커스 이동

---

## 5. 놓치기 쉬운 함정

**Windows ConPTY race condition** ← 가장 위험
`portable-pty` known issue (#356) — openpty와 drop이 겹치면 console 깨짐.
전역 `static CONPTY_LIFECYCLE_LOCK: Mutex<()>` 로 spawn과 drop 모두 직렬화 필수.
없으면 에이전트 2개 이상 동시 시작 시 랜덤으로 터미널 깨짐.

**pty_close blocking**
Windows에서 `ClosePseudoConsole`이 conhost drain 대기로 blocking됨.
Session drop을 별도 스레드에서 실행해야 Tauri 커맨드 스레드가 막히지 않음.
→ `thread::spawn(move || drop(session))` 패턴.

**cwd 슬래시**
`CreateProcessW`는 forward-slash cwd 거부. Rust에서 `cwd.replace('/', "\\")` 필수.

**xterm.js Strict Mode 2회 effect**
개발 환경에서 effect 2번 실행 → `terminal.open(container)` 두 번 호출 → 터미널 깨짐.
`termRef.current` 존재하면 early return guard 필수.

**Monaco CDN 차단**
`loader.config({ monaco })`를 App 진입점에서 즉시 실행하지 않으면 CDN 요청 시도.
Tauri CSP가 막으면 빈 화면 + 에러 메시지 없음.

**getLineChanges() null**
DiffEditor onMount에서 바로 호출하면 null.
`editor.onDidUpdateDiff(() => { ... })` 이벤트 구독 후 호출해야 유효한 결과.

**allotment + FitAddon 연동**
allotment 드래그 리사이즈 시 `window.resize` 이벤트 발생 안 함.
`onVisibleChange` + `ResizeObserver`를 각 xterm 컨테이너에 붙여야 cols/rows 동기화.
안 하면 줄바꿈 어긋남.

**Monaco 번들 크기**
gzip 후 2~5MB. `languageWorkers: ["editorWorkerService"]`만으로 diff 뷰 동작.
TypeScript worker 포함 시 +40%.

**tauri-specta 초기 설정**
나중에 추가하면 기존 invoke 전부 재작업. 처음부터 설정할 것.
