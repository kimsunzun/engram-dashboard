# Engram Dashboard — View 사양서 v2 (GPT 검수용)

> GPT 검수 방법: 이 파일을 첨부파일로 업로드해서 전달. 내용을 채팅창에 직접 붙여넣지 않는다.

## 프로젝트 개요

**Engram Dashboard**: 여러 Claude Code AI 에이전트를 동시에 관리하는 데스크톱 앱.
현재 WezTerm 터미널 패널로 에이전트를 관리하는데, 이를 전용 데스크톱 앱으로 대체.

**현재 단계**: 백엔드(Rust PTY) 연결 전, 프론트엔드 View를 더미 데이터로 완성하는 단계.

---

## 기술 스택 (View 한정)

| 레이어 | 선택 | 비고 |
|---|---|---|
| 앱 껍데기 | Tauri v2 | Rust 백엔드 + Edge WebView2 |
| UI | React 19 + TypeScript + Vite | |
| 스타일 | TailwindCSS v4 + Radix UI | shadcn 아님 — Radix 직접 사용 |
| 상태 | Zustand | 더미 데이터 관리 |
| 터미널 뷰 | @xterm/xterm + **직접 wrapper** | react-xtermjs GPL 라이선스 문제로 직접 작성 |
| 패널 분할 | allotment | VS Code 파생 |
| 에이전트 트리 | react-arborist | |
| Diff 뷰 | @monaco-editor/react DiffEditor | 로컬 번들 필수 |
| 폰트 | CSS 변수 기반 | ThemeManager 단일 진입점 |

---

## 전체 레이아웃

```
┌─────────────┬──────────────────────────────────┐
│ Agent Tree  │  Slot 1        │  Slot 2         │
│ (사이드바)  ├────────────────┼─────────────────│
│             │  Slot 3        │  Slot 4         │
├─────────────┴──────────────────────────────────┤
│ 알림/이벤트 패널  │  Status Bar                 │
└─────────────────────────────────────────────────┘
```

- 사이드바(에이전트 트리): 접기/펼치기, 별도 창 분리 가능
- 메인 영역: N개 슬롯 (동적 분할)
- 하단: 알림/이벤트 패널 + 상태바

---

## 에이전트 트리

**데이터 모델**:
```ts
agents = [
  { id: '1', name: '비서', type: '비서', status: 'running', cost: '$0.12' },
  { id: '2', name: '코더-1', type: '코더', status: 'idle', cost: '$0.21' },
  { id: '3', name: '코더-2', type: '코더', status: 'running', cost: '$0.08' },
]
groups = [
  { id: 'g1', name: '코딩룰', members: ['1', '2', '3'] },
]
```

**트리 표시 규칙**:
```
AGENTS
── 비서  ●  $0.12
▼ 코더
   ── 코더-1  ◌  $0.21
   ── 코더-2  ●  $0.08

GROUPS
▼ 코딩룰
   비서 · 코더-1 · 코더-2
```
- 같은 type 1개: flat / 2개+: 그룹핑
- 상태 아이콘: running(초록) / idle(회색) / error(빨간)
- 부모 노드 집계 뱃지 (트리 닫혀도 상태 파악)
- 토큰/비용 토글 표시
- 우클릭 컨텍스트 메뉴: Start / Stop / Kill (더미)

**슬롯 전환**: 에이전트 **더블클릭** → 포커스 슬롯이 해당 에이전트로 전환
(단일클릭은 우클릭 메뉴와 충돌 위험으로 더블클릭 채택)

---

## 슬롯 시스템

- **슬롯** = 디스플레이 컨테이너 (PTY 프로세스와 분리)
- **포커스 슬롯**: 마지막 클릭 슬롯 = 활성 (테두리 강조)
- 에이전트 트리 더블클릭 → 포커스 슬롯 전환
- 에이전트 이름: xterm.js 우하단 absolute 오버레이 (헤더 바 없음)
- 인스턴스 1개: 이름만 / 2개+: ▼ 드롭다운

**슬롯 컨트롤 (우클릭 메뉴)**:
- 가로/세로 분할
- 팝업 분리 → 별도 Tauri 창
- 에이전트 전환
- 닫기

**도킹**:
- 팝업 창끼리 합치기
- 메인 창에 다시 도킹

---

## 터미널 — xterm.js

- 직접 wrapper 작성 (`react-xtermjs` GPL 대체)
- 더미 ANSI 출력으로 테스트
- **LinkProvider**: 파일경로/URL/diff 클릭 인터랙션
- **resize debounce**: allotment 드래그 → ResizeObserver → debounce → FitAddon.fit()
- 가상 스크롤백 (보이는 행만 DOM 렌더)

---

## 테마 시스템

```css
:root[data-theme="dark"]  { --bg: #0a0a0a; --text: #e0e0e0; --border: #333; }
:root[data-theme="light"] { --bg: #f5f5f5; --text: #1a1a1a; --border: #ccc; }
:root[data-theme="e-ink"] { --bg: #ffffff; --text: #000000; --border: #000; }
```
- 창 단위 테마 전환
- **ThemeManager** 단일 진입점으로 관리
- 핫리로드 (재시작 없이 즉시 반영)
- 투박한 고대비 — 그림자/그라디언트 없음

---

## 폰트 시스템

```css
/* 패밀리 */
--font-ui: 'JetBrains Mono', monospace;
--font-terminal: 'Cascadia Code', monospace;
--font-code: 'Fira Code', monospace;
--font-claude-prose: 'Inter', sans-serif;
--font-claude-code: 'JetBrains Mono', monospace;
--font-claude-path: 'Cascadia Code', monospace;
--font-claude-header: 'Inter', sans-serif;

/* 사이즈 */
--font-size-ui: 13px;
--font-size-terminal: 14px;
--font-size-code: 13px;

/* 행간 */
--line-height-ui: 1.4;
--line-height-terminal: 1.2;
```
- 설정 JSON에서 교체 가능, 변경 시 즉시 반영

---

## 알림/이벤트 패널

- 에이전트에서 오는 이벤트를 한 곳에 모음
- 이벤트 종류: 승인 요청 / 작업 완료 / 에러 발생 / 사용자 지정 알림
- 승인은 보통 없어야 정상 — 알림 용도가 주목적
- 위치: 하단 (레이아웃 내 토글 가능)

---

## Monaco DiffEditor

- 로컬 번들 (`loader.config({ monaco })`)
- 더미 diff 표시
- Accept / Revert 버튼 (onDidUpdateDiff 이후 활성화)
- 하단 패널 토글

---

## 상태바

- 실행 중 에이전트 수 / 오류 여부 / 전체 토큰 합산
- 하단 고정

---

## 검수 요청 사항

v1 대비 변경된 사항:
- react-xtermjs → 직접 wrapper로 교체
- 슬롯 전환 단일클릭 → 더블클릭
- 알림/이벤트 패널 추가
- font-size/line-height CSS 변수 추가
- ThemeManager 단일 진입점
- resize debounce
- Start/Stop/Kill 더미 컨트롤 추가

검토 요청:
1. v1 지적사항이 이번 v2에서 제대로 반영됐나요?
2. 추가로 놓친 부분이 있나요?
3. React 19 + Monaco Editor 버전 호환성 이슈가 있나요? 버전 pinning 필요한 조합?
4. allotment + xterm.js 직접 wrapper 조합에서 알려진 문제?
5. 전반적으로 이 설계로 실용적인 AI 에이전트 관리 도구 구현이 가능한가요?
