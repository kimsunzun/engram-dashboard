# ADR-0011: agentClient control-surface facade (daemon preparation)

- 상태: 확정 (frontend integration, S12 preparation)
- 관련: CLAUDE.md §5·프론트 파일 구조 · `api/{agentClient,embeddedClient,clientFactory}.ts`

## 맥락
When daemonizing, if the frontend calls `invoke`/Channel directly, the transport path (in-process ↔ WebSocket) cannot be swapped. Also, §5 (LLM-first control) requires every action to have a single control surface that the LLM can reach.

## 결정
Components and stores depend **only on the `agentClient` interface** (no direct ptyApi calls). The current implementation is `embeddedClient` (in-process, encapsulating invoke/Channel). `clientFactory` creates the singleton and exposes it as `window.__ENGRAM_AGENT__` (the §5 control surface). The daemon-stage `DaemonClient` (WS) is added with the **same interface**.

## 거부한 대안
- **Scattered direct ptyApi invoke calls** — a daemon switch would force a full rewrite of every call site. The facade cannot be swapped.

## 근거
Replacing the facade in one place absorbs the in-process ↔ daemon switch (daemonization = facade swap, not a rewrite). Exposing it on window makes a human click and an LLM call share the same entry point.

## 영향 / 불변식
- **No direct ptyApi calls from components or stores** — always go through agentClient.
- DaemonClient must implement agentClient, not create a new interface.
