# Engram Dashboard

Tauri v2 + React 19 + TypeScript 기반 에이전트 모니터링 대시보드.
**View phase 완료. 현재 Backend phase** — Rust PTY 백엔드 구현 중.

## 현재 상태 (2026-06-11)

**LLD Stage1 확정** (`docs/backend-lld-stage1.md`) — fable/Gemini/GPT 3자 검증 완료, GO 판정.  
**다음 작업:** 2단계 모듈별 Rust 코드 구현.

### 백엔드 구현 순서
1. `src-tauri/src/pty/types.rs` — 타입 정의
2. `src-tauri/src/pty/session.rs` — PtySession 구조체
3. `src-tauri/src/pty/drain.rs` — drain thread
4. `src-tauri/src/pty/manager.rs` — PtyManager
5. `src-tauri/src/pty/platform/windows.rs` — Job Object
6. `src-tauri/src/logging/mod.rs` — 로그
7. `src-tauri/src/commands/` — Tauri command layer
8. `src-tauri/src/lib.rs` — AppState, 등록

### 핵심 설계 원칙 (LLD에서 확정)
- `pty/` 하위 파일은 **tauri import 금지** — OutputSink/StatusSink trait으로 추상화
- `AppState { manager: Arc<PtyManager> }` — 외부 Mutex 없음
- drain thread는 OS thread (blocking I/O와 자연 일치)
- PTY 출력 전달: `Channel<PtyEvent>` (emit_all 금지)
- kill 시퀀스: `child.kill() → child.wait() → TerminateJobObject(Windows) → master.take() → completion_channel.recv_timeout(5s)`
- replay→live: subscribers lock 보유 중 replay 전송 (순서 역전 방지)

### 의존성 (고정)
- `tauri = "2.4"` (2.5 Channel silent failure 이슈)
- `portable-pty = "0.8.1"` (smoke test 후 업그레이드)

### 2단계 첫 스파이크 (코드 전 Windows 실측)
1. spawn → 출력 수신 → kill → join 즉시 완료 (TerminateJobObject 검증)
2. 창 닫힘 후 Channel.send() 반환값 (silent failure 여부)

## 기술 스택

| 레이어 | 선택 |
|---|---|
| 앱 껍데기 | Tauri v2 (창만, invoke 없음) |
| UI | React 19 + TypeScript + Vite |
| 스타일 | CSS 변수 (TailwindCSS 미사용) |
| 상태 | Zustand |
| 터미널 | @xterm/xterm + @xterm/addon-fit + react-xtermjs |
| 패널 분할 | allotment |
| 에이전트 트리 | react-arborist |
| Diff 뷰 | @monaco-editor/react DiffEditor |
| 라우팅 | react-router-dom (hash routing) |

## 파일 구조

```
src/
├── App.tsx                          # 루트 라우터 (/ → AppLayout, /popup → PopupPage, /tree → TreePage)
├── index.css                        # @import theme.css, font.css
├── styles/
│   ├── theme.css                    # CSS 변수: dark/light/e-ink
│   └── font.css                     # CSS 변수: --font-ui/terminal/code/claude-*
├── store/
│   ├── themeStore.ts                # theme: 'dark'|'light'|'e-ink', setTheme
│   ├── agentStore.ts                # agents, groups, selectedAgentId, setSelectedAgent
│   └── slotStore.ts                 # slots: Slot[], focusedSlotId, setFocusedSlot, assignAgent
├── theme/
│   └── ThemeManager.ts              # 싱글턴, apply(theme) → setAttribute('data-theme', ...)
├── components/
│   ├── layout/
│   │   ├── AppLayout.tsx            # allotment: 사이드바 / (슬롯존 / DiffPanel / StatusBar)
│   │   ├── Sidebar.tsx              # AgentTree + 사이드바 접기/트리분리 버튼
│   │   ├── SlotPane.tsx             # layout용 슬롯 래퍼 (children 주입)
│   │   └── StatusBar.tsx            # 하단 24px 고정, Diff 토글
│   ├── agent/
│   │   └── AgentTree.tsx            # react-arborist, status별 색상, 비용 표시
│   ├── slot/
│   │   ├── SlotPane.tsx             # 포커스 테두리, 에이전트 오버레이, 우클릭 메뉴
│   │   ├── TerminalSlot.tsx         # xterm.js + FitAddon, ANSI 더미 출력
│   │   └── SlotContextMenu.tsx      # 분할/에이전트전환/닫기/팝업분리 메뉴
│   └── diff/
│       └── DiffPanel.tsx            # Monaco DiffEditor, Accept/Revert 버튼 (더미)
└── pages/
    ├── PopupPage.tsx                # /popup?slotId=N → 슬롯 단독 창
    └── TreePage.tsx                 # /tree → AgentTree 단독 창
```

## CSS 변수

### 테마 (`data-theme` attribute on `:root`)
| 변수 | dark | light | e-ink |
|---|---|---|---|
| `--bg` | #0a0a0a | #f5f5f5 | #ffffff |
| `--bg-secondary` | #111 | #fff | #f0f0f0 |
| `--text` | #e0e0e0 | #1a1a1a | #000000 |
| `--text-muted` | #888 | #666 | #444 |
| `--border` | #333 | #ccc | #000 |
| `--accent` | #4a9eff | #0066cc | #000 |

### 폰트
| 변수 | 기본값 | 용도 |
|---|---|---|
| `--font-ui` | JetBrains Mono | 메뉴, 레이블 |
| `--font-terminal` | Cascadia Code | xterm.js |
| `--font-code` | Fira Code | Monaco diff |
| `--font-claude-prose` | Inter | Claude 일반 텍스트 |
| `--font-claude-code` | JetBrains Mono | Claude 코드블록 |
| `--font-claude-path` | Cascadia Code | 파일경로 |
| `--font-claude-header` | Inter | 헤더/타이틀 |

## 상태 구조

```ts
// slotStore
interface Slot { id: number; agentId: string | null }
// slots: 현재 2개 고정 [{ id:1 }, { id:2 }]
// → 동적 분할 구현 시 트리 구조로 변경 예정

// agentStore
// agents: 더미 3개 (비서/코더/리뷰어)
// groups: 더미 1개 (코딩룰)
// → 백엔드 연결 시 Tauri invoke로 교체
```

## 창 구성 (tauri.conf.json)

| label | 용도 | 기본 |
|---|---|---|
| main | 메인 대시보드 | visible |
| slot-popup | 슬롯 팝업 분리 | hidden, /popup?slotId=N |
| agent-tree | 에이전트 트리 분리 | hidden, /tree |

## View phase 완료 현황

- [x] Step 1 — 스캐폴딩
- [x] Step 2 — 테마 시스템
- [x] Step 3 — 폰트 시스템
- [x] Step 4 — 레이아웃 셸
- [x] Step 5 — 에이전트 트리 (더미)
- [x] Step 6 — 슬롯 컴포넌트
- [x] Step 7 — xterm.js 더미 출력
- [x] Step 8 — Monaco DiffEditor
- [x] Step 9 — 슬롯 팝업 분리
- [x] Step 10 — 에이전트 트리 분리
- [x] 슬롯 동적 분할 (SlotNode/SplitNode 재귀 트리, LayoutRenderer)
- [ ] 팝업→메인 도킹 (백엔드 단계로 미룸)

## 더미 → 실제 전환 시 교체 지점

| 현재 | 교체 대상 |
|---|---|
| `dummyAgents` in agentStore | Tauri invoke → Rust PTY 프로세스 목록 |
| `TerminalSlot` 더미 write | xterm.js ↔ Rust PTY pty_read 이벤트 |
| DiffPanel 하드코딩 diff | Tauri invoke → 실제 파일 diff |
| `window.open` 팝업 | Tauri WebviewWindow API |
