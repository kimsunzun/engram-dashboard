# LLM 제어 — 남은 갭

> **상태:** 트리거 대기 · **정본 색인:** [`../todo.md`](../todo.md) · 원칙 = `CLAUDE.md` 「LLM-우선 제어」

정식 표면(데몬 버스 · 프론트 `__engramCmd`)으로 아직 닿지 않는 조작이다.

- **트리 선택 상태** — 에이전트 트리의 선택을 바꾸는 명령이 없다. 읽기는 행의 `data-selected` 속성뿐이다(`src/components/agent/AgentList.tsx`).
- **터미널 모드 에이전트 끊기** — `agent.interrupt` 는 끊기를 못 하는 통로에 `CONFLICT` 로 답하고(`crates/engram-dashboard-agent/src/commands.rs` `verb_interrupt`), 버스에 날 입력 명령이 없다. ADR-0245 · 사용자 수락.
- **셸 명령 표는 셸이 데몬에 붙어 있을 때만 닿는다** — `src-tauri/src/layout/commands.rs` 의 명령 전부(창 · 탭 · 슬롯 · 분할 · `settings.*` 등 — `usage.*` 처럼 데몬이 주인인 것은 제외). `docs/process/S21-storage/trd.md` §12 R9.
- **창 자리 · 크기 · 스냅 명령이 없다** — 창 상태(`window.minimize` · `maximize` · `unmaximize` · `raise` 등)와 탭 옮기기(`tab.move` · `tab.dock` · `tab.detach`)는 버스 명령이지만 창 자리 · 크기는 사람의 OS 제스처뿐이다. ADR-0302 · ADR-0303 결정 4 · ADR-0304.
- **버스 밖 전역 핸들** — `__engramCmd` 말고도 전역 핸들이 남아 있다. 명단을 여기 적지 않는다 — 찾는 법 = `rg "\)\.__ENGRAM_|\)\.__engram" src/ -g '!*.test.*'`(결과 중 `__engramCmd` 는 정식 표면이고 나머지가 갭이다). 핸들 주석은 갭인지의 근거가 아니다.
- 키바인딩 재지정 = `docs/tracking.md` T-42.

**지금 안 하는 이유:** 각 갭을 닫을 작업이 잡히지 않았다 — 해당 기능을 만질 때 그 갭을 함께 닫는다.
