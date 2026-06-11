# GPT 검수 결과: Engram Dashboard View 사양서 (파일 첨부 방식)

**검수 일시:** 2026-06-07

---

모델을 제가 GPT-4o로 전환할 수는 없지만, 첨부된 view-spec.md 기준으로 최신 라이브러리 상태까지 확인해 검토했습니다. 결론부터 말하면 **v2는 방향이 맞고, View MVP로는 충분히 구현 가능**합니다. 다만 **React 19 의존성 pinning, xterm 생명주기, Tauri 다중 창/도킹, PTY 계약**은 사양서에 더 박아두는 게 좋습니다.

---

## 1. v1 지적사항이 v2에 제대로 반영됐나요?

대부분 잘 반영됐습니다. 특히 다음 변경은 적절합니다.

| 항목 | 판단 |
|---|---|
| react-xtermjs 제거 → 직접 wrapper | 좋습니다. GPL 회피뿐 아니라 xterm 생명주기 제어에도 유리합니다. |
| 슬롯 전환 단일클릭 → 더블클릭 | 적절합니다. 트리 선택, 우클릭 메뉴, 드래그/포커스 충돌을 줄입니다. |
| 알림/이벤트 패널 추가 | AI 에이전트 관리 도구에는 필수에 가깝습니다. |
| font-size / line-height CSS 변수화 | 좋습니다. 터미널·코드·Claude prose 영역을 나눠 관리할 수 있습니다. |
| ThemeManager 단일 진입점 | 반드시 필요합니다. 창 단위 테마 전환까지 고려하면 좋은 선택입니다. |
| resize debounce | allotment + xterm 조합에서는 필수입니다. |
| Start / Stop / Kill 더미 컨트롤 | View 단계에서 UX 검증하기 좋습니다. |

다만 "반영 완료"라기보다는 **사양서 레벨에서 반영**된 상태입니다. 구현 검수 기준으로는 `mount/unmount cleanup`, `React StrictMode에서 xterm 중복 생성 방지`, `FitAddon.fit() 호출 조건`, `다중 창 상태 동기화`까지 acceptance criteria로 추가하는 게 좋습니다.

---

## 2. 추가로 놓친 부분이 있나요?

네. 아래는 꼭 보강하는 편이 좋습니다.

**첫째, Agent / Slot / PTY의 데이터 모델 분리**가 더 명확해야 합니다. 지금 "슬롯 = 디스플레이 컨테이너, PTY 프로세스와 분리"라고 적은 건 좋지만, 실제 구현에서는 `AgentSession`, `SlotView`, `TerminalInstance`, `WindowInstance`를 분리해야 합니다. 그래야 같은 에이전트를 다른 슬롯으로 옮기거나, 팝업 창으로 분리하거나, 다시 도킹할 때 상태가 꼬이지 않습니다.

**둘째, Tauri 다중 창 설계**가 부족합니다. "팝업 분리 / 다시 도킹"은 적혀 있지만, 어느 상태가 메인 프로세스에 있고 어느 상태가 WebView별 Zustand에 있는지 정해야 합니다. 추천은 **agent/session 상태 = 중앙 store**, **slot layout = window별 store**, **terminal DOM/xterm 인스턴스 = 창별 소유**입니다.

**셋째, xterm.js의 가상 스크롤백 표현은 수정하는 게 좋습니다.** "보이는 행만 DOM 렌더"라고 되어 있는데, 이걸 직접 구현하려 하면 xterm 내부 buffer/renderer와 충돌할 수 있습니다. 사양에는 "xterm.js 내부 scrollback 사용, scrollback limit 설정, write batching 적용" 정도로 바꾸는 편이 안전합니다.

**넷째, LinkProvider 보안 정책**이 필요합니다. 파일경로, URL, diff 클릭은 편하지만, 로컬 파일 열기·외부 URL 열기·명령 실행과 연결되면 위험합니다. 최소한 `file://` 직접 실행 금지, workspace path whitelist, URL은 외부 브라우저 confirm 또는 allowlist, diff link는 내부 라우팅만 허용 같은 규칙이 있어야 합니다.

**다섯째, 키보드 UX**가 빠져 있습니다. AI 에이전트 관리 도구는 마우스보다 키보드 사용이 많을 가능성이 큽니다. 예를 들어 `Ctrl+1~4` 슬롯 포커스, `Ctrl+Shift+S` split, `Ctrl+Shift+P` command palette, `Esc` 패널 닫기, `F2` 이름 변경 정도는 초기에 잡아두면 좋습니다.

---

## 3. React 19 + Monaco Editor 호환성 이슈 / pinning 조합

React 19 + Monaco는 가능하지만, 버전 pinning을 강하게 추천합니다.

React 19 쪽에서는 새 JSX Transform이 사실상 필요하고, `ReactDOM.render`는 제거되어 `createRoot`로 가야 합니다. 공식 업그레이드 문서도 React 19 설치 시 `react`, `react-dom`, `@types/react`, `@types/react-dom`을 함께 맞추도록 안내합니다.

`@monaco-editor/react`는 현재 README에서 React 19 지원을 명시하고 있고, `Editor`, `DiffEditor`, `loader`를 제공합니다. 또한 로컬 번들 방식은 `loader.config({ monaco })`로 가능하지만, Vite에서는 Monaco worker들을 별도로 import해야 한다고 문서화되어 있습니다.

**추천 조합:**

```json
{
  "react": "19.x",
  "react-dom": "19.x",
  "@types/react": "19.x",
  "@types/react-dom": "19.x",
  "@monaco-editor/react": "4.8.0-rc.3 또는 @next",
  "monaco-editor": "0.55.1"
}
```

단, `@monaco-editor/react`는 README와 릴리스 상태가 약간 혼재되어 있습니다. README에는 React 19 지원이 표시되어 있지만, 설치 설명에는 React 19 사용자는 `@next`를 보라고 되어 있고, GitHub 릴리스 영역에는 `v4.8.0-rc.3`가 최신 릴리스로 보입니다. 따라서 **React 19를 고정할 거면 `@monaco-editor/react@next` 또는 명시적 rc 버전으로 lockfile까지 고정**하는 편이 안전합니다.

Monaco 본체는 공식 페이지 기준 최신 릴리스가 `0.55.1`로 표시됩니다. 다만 2026년에 `monaco-editor` 하위 의존성 보안 이슈를 지적한 GitHub issue도 있으므로, 실제 고정 버전은 `npm audit` 결과까지 보고 결정하는 게 좋습니다.

---

## 4. allotment + xterm.js 직접 wrapper 조합에서 알려진 문제

이 조합은 실무적으로 가능합니다. 다만 아래 문제가 자주 납니다.

**첫째, xterm 생성 시점 문제**입니다. xterm은 DOM 요소에 `open()`된 뒤 크기를 계산해야 합니다. 공식 예제도 DOM에 붙인 뒤 `open()`하는 흐름입니다. `display:none`, 아직 크기 0인 pane, 접힌 pane에서 `fit()`을 호출하면 cols/rows가 잘못 계산될 수 있습니다.

**둘째, resize 폭주 문제**입니다. allotment 드래그 중 `ResizeObserver`가 매우 자주 발생하고, 그때마다 `FitAddon.fit()`을 호출하면 렌더링이 버벅일 수 있습니다. 사양서의 debounce 방향은 맞습니다. 더 안전하게는 `ResizeObserver → requestAnimationFrame → debounce/throttle → visible && width>0 && height>0일 때만 fit()` 구조를 추천합니다.

**셋째, React 19 StrictMode에서 중복 생성/해제 문제**입니다. 개발 모드에서 effect가 예상보다 많이 호출되며, xterm 인스턴스와 event listener가 중복될 수 있습니다. wrapper에는 반드시 `terminal.dispose()`, addon dispose, ResizeObserver disconnect, 링크 provider dispose를 넣어야 합니다.

**넷째, Tauri 팝업 창으로 이동할 때 xterm 인스턴스를 그대로 옮기면 안 됩니다.** WebView/document가 달라지므로 terminal DOM을 이동시키기보다 "같은 AgentSession에 새 TerminalView를 attach"하는 구조가 안전합니다. 참고로 xterm 5.4.0 릴리스에는 multi-window app support를 돕는 `documentOverride` 옵션이 추가되었습니다.

**다섯째, allotment 자체의 React 19 peer dependency 경고 가능성**이 있습니다. allotment는 React 19에서 동작 테스트가 있었다는 issue가 있지만, peer dependency가 `^17 || ^18`로 묶여 있다는 보고가 열린 상태입니다. 실제 설치 시 npm/pnpm에서 peer warning이 날 수 있으니 lockfile 기준으로 먼저 검증해야 합니다.

---

## 5. 이 설계로 실용적인 AI 에이전트 관리 도구 구현이 가능한가요?

**네, 가능합니다.** 오히려 현재 View 사양은 "WezTerm 여러 창을 사람이 직접 관리하는 방식"을 전용 UI로 대체하기에 좋은 구조입니다. Agent Tree, 동적 슬롯, xterm 터미널, 이벤트 패널, DiffEditor, 상태바의 조합도 AI 에이전트 운영 도구에 잘 맞습니다.

다만 MVP 범위는 이렇게 자르는 걸 추천합니다.

- **MVP 1차**: Agent Tree, 4-slot 레이아웃, 더미 xterm 출력, 포커스 슬롯, 상태바, 이벤트 패널
- **MVP 2차**: 직접 xterm wrapper 완성, LinkProvider, resize 안정화, 테마/폰트 즉시 반영
- **MVP 3차**: Monaco DiffEditor 로컬 번들, diff link 클릭 → 하단 diff 패널 연동
- **MVP 4차**: Rust PTY 연결, cols/rows resize 전달, Start/Stop/Kill 실제 연결
- **후순위**: 팝업 창끼리 합치기, 메인 창 재도킹, 복잡한 다중 창 세션 복원

최종 판단은 **"진행해도 되는 설계"**입니다. 단, 지금 문서에 아래 4개만 추가하면 훨씬 안전해집니다.

```
추가 권장 사양:
- AgentSession / SlotView / TerminalView / WindowInstance 데이터 모델 분리
- xterm wrapper lifecycle: create, open, fit, resize, dispose 규칙 명시
- Tauri multi-window 상태 동기화 및 도킹 정책 명시
- LinkProvider 보안 정책: file path whitelist, external URL confirm, diff internal route only
```

가장 큰 리스크는 UI 라이브러리 선택이 아니라 **"터미널 프로세스 상태와 슬롯 표시 상태를 얼마나 깨끗하게 분리하느냐"**입니다. 이 부분만 잘 잡으면 실용적인 AI 에이전트 대시보드로 충분히 갈 수 있습니다.

---

**대화 URL:** https://chatgpt.com/c/6a2565cd-d9f4-8324-ab7d-d0a47a008af6
