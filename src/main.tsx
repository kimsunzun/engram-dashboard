import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { useThemeStore } from "./store/themeStore";
import { useAgentStore } from "./store/agentStore";
import { applyBootTheme } from "./theme/bootTheme";
import { settingsClient } from "./api/settingsClient";
import { installChatStyleApplier } from "./store/chatStyleStore";

// ADR-0265: take the shell settings and paint the chat style — independent of daemon bootstrap (the owner of the settings is the shell), hooked before the first render.
// Values arrive asynchronously, and the first paint before that is the `--chat-*` fallback in `theme.css` (TRD S21-storage §10 F6 (a)).
// It runs once per page so no disposer is held — moving it into an App effect would make the effect return two disposers
// (StrictMode double run).
installChatStyleApplier();
settingsClient.install();

// ★Always plant data-theme before the first paint★ — color tokens exist only inside `:root[data-theme='…']` in `styles/theme.css`
// and have no fallback. While the attribute is absent `--text`·`--border` are entirely undefined, so index.css lays down only a dark background
// and text · borders fall to the UA default (black) = black text on a black background. That span is as long as the IPC round trip until the theme the shell decided
// is attached (`theme/uiSettings.ts`). If the shell planted a value while creating the window, that value is kept (that function's doc).
// ★Do not move this line back into an effect inside App★ (after createRoot is already too late).
applyBootTheme();

// LLM control surface (CLAUDE.md 「LLM-우선 제어」) — expose store handles on window in development builds only.
// From outside (cdp.mjs eval / CDP) one can read state as JSON with window.__engram.<store>.getState() and
// operate the UI with getState().<action>(). Not exposed in production builds (import.meta.env.DEV=false).
// ★Layout (slot/view) is not here★ — it is backend authority (ADR-0035), so its control surface is the command registry
// (window.__engramCmd).
// ★The authority of theme is the shell★ — the shell computes the window theme (`window.setTheme`) ?? the shell setting `theme.default` (ADR-0265) and
// pushes it (delivery = `theme/uiSettings.ts`). A value changed through the handle exposed here is not saved, so the shell's next push overwrites it.
if (import.meta.env.DEV) {
  (window as unknown as { __engram?: unknown }).__engram = {
    theme: useThemeStore,
    agent: useAgentStore,
  };
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
