# ADR-0035: 레이아웃 권위 = src-tauri 클라 (데몬 UI 불가지론, 에디터 모델)

- 상태: 확정 (2026-06-27, 근거: `/research` deep 보고서 + 사용자 결정)
- 관련: CLAUDE.md §5(LLM 제어)·§아키텍처 0~5 · ADR-0029(데몬=데이터 단일소유)·ADR-0004(backend 격리)·ADR-0006(락 순서)·ADR-0011(agentClient 제어표면)·ADR-0036(전송 중계 — 이 결정의 토폴로지 짝) · `docs/research/multi-window-layout-authority-topology-research-2026-06-27.md` · step-log S14

## 맥락
S14 멀티 페이지 레이아웃 — 단일 슬롯 트리를 다중 View(탭 전환 + 팝업 창 독립 분할)로 확장. 핵심 미결: **레이아웃 상태(Views·split 트리·slot→agent 바인딩)의 single source of truth를 어디에 두나** — (a) 에이전트 데몬 / (b) src-tauri Rust / (c) 각 창 JS. rev.1·rev.2는 JS authority(c)였고 `/review trd`에서 2회 BLOCK — Tauri는 창마다 JS 컨텍스트가 격리되어(공유 메모리 없음) 팝업이 생기는 순간 메인·팝업 store가 별개 인스턴스가 되는 split-brain이 원인. rev.3은 데몬 권위(a)로 기울었으나 "데몬이 UI를 알아야 하나"라는 층 섞임 논란으로 재검토.

## 결정
**레이아웃 권위 = src-tauri Rust.** `AppState`에 `Arc<Mutex<ViewManager>>`(views·active_view_id·window_bindings)를 둔다. 데몬은 **View를 일절 모른다**(에이전트만 — 호스팅·I/O·정책). 각 창 JS는 **순수 렌더러**로, src-tauri의 `layout:updated`/`view:list-updated` emit을 받아 미러하고, 레이아웃 커맨드는 invoke로 src-tauri에 보낸다. OS 창 lifecycle(팝업 생성/닫기)도 src-tauri 소유(데몬은 Tauri 창을 못 만든다). §5 LLM 제어 = `window.__engramLayout`이 invoke 래퍼로 사람 클릭과 동일 핸들 노출.

## 거부한 대안
- **(a) 데몬이 레이아웃 권위 소유** — engram은 **에이전트와 슬롯이 디커플링**돼 있다: `close_view`해도 에이전트는 안 죽고(데몬 생존) 슬롯에 재배정 가능. 즉 슬롯/View는 에이전트 위에 얹은 *표시(presentation) 레이어*지 세션 자체가 아니다. tmux/Wezterm Mux가 레이아웃을 서버에 두는 건 거기선 **pane==PTY**(레이아웃=세션)이기 때문 — engram엔 그 전제가 없다. 따라서 우리는 tmux 모델이 아니라 **에디터 모델**(VS Code/Zed: 파일·언어서버=공유 백엔드, 에디터 그리드=클라 로컬)에 속한다. UI를 에이전트 호스트에 결합하면 ADR-0004 백엔드 격리 정신도 위배. (리서치 보고서 §제약적합도표.)
- **(c) 각 창 JS가 권위** — Tauri 창마다 JS 컨텍스트 격리 → 팝업 store와 메인 store가 별개 → close/dispatch가 한쪽만 정리하는 split-brain. rev.1·rev.2 2회 BLOCK으로 실증됨.

## 근거
`/research` deep(Claude Sonnet 3갈래 + Codex 2회 BLIND 교차 → opus 적대검증, 사실 차원 전면 수렴). 두 관행이 갈림: **터미널 멀티플렉서=서버 권위 / GUI 에디터=클라 로컬**, 그리고 *왜 갈리는지*(내구 객체가 서버측 PTY냐 / UI 그리드를 그 창이 그리느냐)까지 양 family 동일 진단. engram의 **에이전트/슬롯 디커플링**(이미 박힌 자기 설계 — ADR-0016/0017, close_view 정책)이 결정 기준 → 에디터 모델 → 클라(src-tauri) 권위. ADR-0029 "데몬=데이터 단일소유"는 *에이전트 데이터* 이중호스팅 금지지 레이아웃을 데몬에 넣으라는 게 아니며, 레이아웃은 신규 클라 관심사로 src-tauri 단일소유라 이중소유가 아니다(무위반).

## 영향 / 불변식
- `ViewManager`는 src-tauri `AppState` 소유, `Arc<Mutex<>>`(invoke 스레드풀 동시접근). **락 해제 후 emit**(ADR-0006 락순서 — emit 중 락 보유 금지).
- 데몬 crate/protocol에 View·slot·layout 타입이 새지 않는다(데몬 UI 불가지론 유지 — `rg "View\|Layout\|Slot"` 데몬 crate 0 지향).
- 프론트는 `layout:updated`/`view:list-updated` Tauri listen으로 미러(WS agentClient와 별개 채널 — 관심사 분리). 권위는 JS 아님 → split-brain 불가.
- `slotId` = UUID(창 간 전역 고유).
- §5: `window.__engramLayout.*` invoke 래퍼 = LLM·사람 동일 제어 표면. 프론트는 순수 I/O 유지.
- 토폴로지(창↔src-tauri↔데몬 연결)는 ADR-0036이 규정.
