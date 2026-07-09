# ADR-0058: spawn_into 명시 backend = pre-spawn fail-loud (데몬 wire 부재 — 조용한 셸 대체 금지)

- 상태: 확정 (2026-07-09, 근거: 스테이지 5 커밋 88134e2 + /review code full 폐쇄 PASS)
- 관련: CLAUDE.md §3(backend 지식 격리) · ADR-0004(backend seam) · `src-tauri/src/commands/layout.rs:346-374`(backend fail-loud 가드) · `crates/engram-dashboard-daemon/src/connection_core.rs:852`(SpawnByCwd=기본 셸) · TRD `docs/process/B-wezterm-tabs/TRD.md` §6 D-7 · step-log Phase 2 스테이지 5

## 맥락
Phase 2 탭 스테이지 5의 `spawn_into(window, tab?, slot?, backend?, cwd)`는 스폰+create_tab+슬롯 배정을 한 방에 조립하는 command다(TRD D-7). 시그니처에 `backend` 인자가 있으나, 실제 데몬 스폰 wire인 `SpawnByCwd{cwd}`(`connection_core.rs:852`)는 **cwd만 받고 backend 선택 인자가 없다** — 데몬은 무조건 기본 백엔드(현재 `default_shell()`)를 스폰한다. 즉 command 표면엔 backend 선택이 있는데 데몬까지 그 의도를 전달할 통로가 없다. 호출자가 `backend="claude"`를 넘겨도 실제로는 셸이 뜬다.

## 결정
명시된 `backend` 요청은 **스폰 전에 거부**(fail-loud)한다. 통과 = `backend` 미지정(`None`/빈/공백)뿐이며, `"claude"`를 포함한 **어떤 명시값도 거부**한다(현재 스폰되는 건 셸이므로 "claude 지원"은 거짓말). 거부는 에이전트 생성 이전 단계라 부작용이 없다(아직 아무것도 안 죽음 → `alive_err` 불필요). 실제 backend 선택은 **후속**으로 protocol crate의 `SpawnByCwd` 확장 + 데몬 dispatch 확장이 필요하며, 그때 별도 ADR로 박는다.

## 거부한 대안
- **지금 프로토콜 확장(1b) — backend 선택 wire를 즉시 추가.** spawn_into 스테이지가 얇은 조립 슬라이스(TRD D-7)인데 protocol crate `SpawnByCwd` + 데몬 dispatch까지 파고들면 스테이지 범위를 크게 넘긴다. backend 선택은 아직 실수요(codex/gemini 미연결)가 없어 지금 확장은 검증 안 된 가정 위의 투자다(CLAUDE.md 아키텍처 §0 — 고비용·불확실은 껍데기만). → 껍데기(인자만) 두고 실수요 때 채우기로 미룸.
- **요청 backend 조용히 무시하고 기본 셸 스폰.** 초기 후보였으나 호출자가 원한 것과 다른 에이전트를 조용히 받는 오작동을 부른다 — LLM이 `backend="claude"`로 스폰했다고 믿고 이후 조작을 이어가면 전부 어긋난다. fail-loud가 조용한 오작동보다 안전.

## 근거
- 데몬 wire 실측: `SpawnByCwd`는 cwd만 소비하고 backend를 무시한다(`connection_core.rs:852`) — command 표면의 backend 인자가 데몬까지 도달할 경로가 없음을 코드로 확인.
- `/review code full` 2-family 적대 리뷰에서 doc-aware 리뷰어가 초기 "backend 무시" 구현을 "거짓말(claude 지원 표방)"로 지적 → fail-loud로 교정 후 폐쇄 PASS.
- 사용자 결정: 이번 세션 대화에서 "1-a"(지연·fail-loud) 확정.

## 영향 / 불변식
- **backend를 SpawnByCwd로 우겨넣지 않는다** — wire가 없다. 프로토콜 확장(deferred) 전엔 명시 backend fail-loud를 유지하고, 기본 스폰으로 조용히 대체하지 않는다.
- 가드는 `src-tauri/src/commands/layout.rs`의 스폰 전 검증 블록(현재 `:366-374`)에 산다. 후속으로 데몬 spawn-protocol을 확장해 backend 선택이 실제 동작하게 되면 이 ADR을 supersede하고 가드를 완화한다.
- ADR-0004(backend 지식 격리)와 정합: claude 전용 인자는 `backend/claude.rs`에만 있고, spawn_into는 backend를 dispatch로 다룰 뿐 직접 알지 않는다.
