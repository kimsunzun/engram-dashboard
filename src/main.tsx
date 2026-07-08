import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { useThemeStore } from "./store/themeStore";
import { useAgentStore } from "./store/agentStore";
import { loadAndApplyChatStyle, useChatStyleStore } from "./store/chatStyleStore"; // ADR-0051

// ADR-0051 (FIX-1): 저장된 채팅 스타일을 첫 렌더 전에 로드·적용한다 — 데몬 bootstrap 경로와 무관하게(프론트
// 전용 상태). 데몬이 멈춰도 스타일이 적용되고, 정상 부팅에서도 기본값 깜빡임을 최소화한다. window 핸들 노출은
// eventBus 에 남는다(값 로드와 핸들 노출은 서로 독립).
loadAndApplyChatStyle();

// LLM 제어 표면(CLAUDE.md §5) — 개발 빌드에서만 store 핸들을 window에 노출한다.
// 외부(cdp.mjs eval / CDP)에서 window.__engram.<store>.getState()로 상태를 JSON으로 읽고
// getState().<액션>()으로 UI를 조작할 수 있다. 프로덕션 빌드(import.meta.env.DEV=false)에선 미노출.
// ★레이아웃(슬롯/뷰)은 여기 없다★ — 백엔드 권위(ADR-0035)라 window.__engramLayout(viewStore 경유
// invoke)이 제어 표면이다(옛 slotStore.slot 핸들은 Brick 1 에서 제거). 여긴 프론트 전용 store 만.
if (import.meta.env.DEV) {
  (window as unknown as { __engram?: unknown }).__engram = {
    theme: useThemeStore,
    agent: useAgentStore,
    chatStyle: useChatStyleStore, // ADR-0051 — 채팅 스타일 store 핸들(값 스냅샷·액션 직접 조회)
  };
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
