import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { useSlotStore } from "./store/slotStore";
import { useThemeStore } from "./store/themeStore";
import { useAgentStore } from "./store/agentStore";

// LLM 제어 표면(CLAUDE.md §5) — 개발 빌드에서만 store 핸들을 window에 노출한다.
// 외부(cdp.mjs eval / CDP)에서 window.__engram.slot.getState()로 상태를 JSON으로 읽고
// getState().<액션>()으로 UI를 조작할 수 있다. 프로덕션 빌드(import.meta.env.DEV=false)에선 미노출.
if (import.meta.env.DEV) {
  (window as unknown as { __engram?: unknown }).__engram = {
    slot: useSlotStore,
    theme: useThemeStore,
    agent: useAgentStore,
  };
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
