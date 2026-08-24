import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
// ★Allotment 전역 CSS(회귀 수정)★: split-view 패널의 절대배치·높이 채움은 이 CSS 가 있어야 동작한다.
// 옛 AppLayout 이 이걸 import 했는데 ADR-0063 셸 재작성에서 누락돼 부팅 split 이 높이 붕괴했다. 모든 창
// (main·팝업·tree)이 공유하는 엔트리(main.tsx)에서 전역 로드해 ViewLayoutRenderer 의 Allotment 를 살린다.
import "allotment/dist/style.css";
import { useThemeStore } from "./store/themeStore";
import { useAgentStore } from "./store/agentStore";
import { themeManager } from "./theme/ThemeManager";
import { loadAndApplyChatStyle, useChatStyleStore } from "./store/chatStyleStore"; // ADR-0051

// ADR-0051 (FIX-1): 저장된 채팅 스타일을 첫 렌더 전에 로드·적용한다 — 데몬 bootstrap 경로와 무관하게(프론트
// 전용 상태). 데몬이 멈춰도 스타일이 적용되고, 정상 부팅에서도 기본값 깜빡임을 최소화한다.
loadAndApplyChatStyle();

// ★첫 페인트 전에 data-theme 를 반드시 박는다★ — 색 토큰은 `styles/theme.css` 의 `:root[data-theme='…']`
// 안에만 있고 폴백이 없다. 속성이 없는 동안에는 `--text`·`--border` 가 통째로 미정의라 index.css 가 배경만
// 어둡게 깔고 글자·테두리는 UA 기본값(검정)으로 떨어진다 = 검은 배경에 검은 글자. 그 구간은 디스크 테마가
// 붙기까지의 IPC 왕복만큼 길다(`theme/uiSettings.ts`) — 그래서 여기서 dark 를 먼저 박아 최악을 「dark→저장
// 테마 한 번 전환」으로 줄인다. ★이 줄을 App 안 effect 로 되돌리지 말 것★(createRoot 뒤면 이미 늦다).
// 값이 dark 인 것은 themeStore 의 초기값·index.css 폴백과 같은 값을 쓰기 위해서다.
themeManager.apply("dark");

// LLM 제어 표면(CLAUDE.md §5) — 개발 빌드에서만 store 핸들을 window에 노출한다.
// 외부(cdp.mjs eval / CDP)에서 window.__engram.<store>.getState()로 상태를 JSON으로 읽고
// getState().<액션>()으로 UI를 조작할 수 있다. 프로덕션 빌드(import.meta.env.DEV=false)에선 미노출.
// ★레이아웃(슬롯/뷰)은 여기 없다★ — 백엔드 권위(ADR-0035)라 그 제어 표면은 command 레지스트리다
// (window.__engramCmd).
// ★theme 은 이제 반쯤 셸 소유다★ — 부팅값과 `ui.refresh` 는 디스크(`ui-settings.json`)에서 오고
// (`theme/uiSettings.ts`), 여기 노출한 핸들로 바꾼 값은 저장되지 않아 다음 refresh 가 덮는다.
if (import.meta.env.DEV) {
  (window as unknown as { __engram?: unknown }).__engram = {
    theme: useThemeStore,
    agent: useAgentStore,
    chatStyle: useChatStyleStore, // ADR-0051
  };
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
