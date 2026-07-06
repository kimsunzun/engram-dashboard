import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { useSlotStore } from "./store/slotStore";
import { useThemeStore } from "./store/themeStore";
import { useAgentStore } from "./store/agentStore";
import { loadAndApplyChatStyle, useChatStyleStore } from "./store/chatStyleStore"; // ADR-0051

// ADR-0051 (FIX-1): 저장된 채팅 스타일을 첫 렌더 전에 로드·적용한다 — 데몬 bootstrap 경로와 무관하게(프론트
// 전용 상태). 데몬이 멈춰도 스타일이 적용되고, 정상 부팅에서도 기본값 깜빡임을 최소화한다. window 핸들 노출은
// eventBus 에 남는다(값 로드와 핸들 노출은 서로 독립).
loadAndApplyChatStyle();

// LLM 제어 표면(CLAUDE.md §5) — 개발 빌드에서만 store 핸들을 window에 노출한다.
// 외부(cdp.mjs eval / CDP)에서 window.__engram.slot.getState()로 상태를 JSON으로 읽고
// getState().<액션>()으로 UI를 조작할 수 있다. 프로덕션 빌드(import.meta.env.DEV=false)에선 미노출.
if (import.meta.env.DEV) {
  (window as unknown as { __engram?: unknown }).__engram = {
    slot: useSlotStore,
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
