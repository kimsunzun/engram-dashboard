# ADR-0041: 데몬 출력 구독 소유 = layout 델타 단독 (프론트 직접 구독 차단)

- 상태: 확정 (2026-07-01, 근거: S14 모듈① 1차 적대 리뷰 BLOCK-1)
- 관련: CLAUDE.md §아키텍처 §5 · ADR-0035 · ADR-0037 · ADR-0040 · src-tauri/src/commands/agent.rs(forward_daemon_command) · src/api/protocolClient.ts · step-log S14

## 맥락
출력 구독(데몬에 "이 agent 출력을 보내라"는 wire `Subscribe`/`Unsubscribe`)을 **누가 소유하나**를 확정해야 했다. 창이 여럿이고 같은 agent를 동시에 보는 멀티뷰 구조(ADR-0040)에서, 프론트가 창마다 출력 렌더러를 붙일 때 데몬에 `Subscribe{after_seq:null}`(FromOldest)를 함께 forward 하면, N개 창이 같은 agent를 보면 데몬이 N번 전체 스트림을 재전송한다(중복 폭주 + 진도·순서 혼란). ADR-0035(레이아웃 권위 = src-tauri)·ADR-0037(전송 의미론 = Rust 단독)의 연장선에서 구독 소유권을 못 박을 필요가 있었다.

## 결정
데몬 출력 구독(wire `Subscribe`/`Unsubscribe`)은 **src-tauri의 layout 델타가 단독 소유**한다.
- 레이아웃 권위(`OutputRouter`, ADR-0035 SSOT)가 (window, agent) 집합을 rebuild할 때만 데몬에 `Subscribe`/`Unsubscribe`를 보낸다(`send_subscription_delta`).
- 프론트 `ProtocolClient.subscribeOutput`은 **렌더러 등록만** 한다 — 데몬에 `Subscribe`를 보내지 않는다.
- `forward_daemon_command`는 프론트가 보낸 `Subscribe`/`Unsubscribe`를 **명시적으로 차단**한다(받아도 무시 + 경고 로그).

## 거부한 대안
- **프론트 소유(창마다 `subscribeOutput`이 데몬 `Subscribe`를 forward)** — N창이 같은 agent를 보면 N번 FromOldest 전체 재전송(storm) + 창마다 다른 진도로 순서/중복 혼란. 또한 데몬이 "어느 창이 보는지"를 알아야 해 UI에 결합된다(ADR-0035 데몬 UI 불가지론 위반).
- **데몬이 구독 멤버십(창 배치)을 직접 추적** — 데몬이 layout을 알아야 해 같은 결합 문제. ADR-0035가 거부한 "데몬이 View를 안다" 모델.

## 근거
1차 적대 리뷰에서 Codex(blind)가 "프론트 직접 구독 부활 = N창 FromOldest storm"을 **BLOCK-1**로 적출했다(opus는 놓침 — cross-family 교차검증 가치 실증). ADR-0035/0037의 직접 귀결이다. `protocolClient.test.ts`에 "subscribeOutput 첫 구독은 데몬에 Subscribe를 안 보낸다" 회귀 단언으로 박았다.

## 영향 / 불변식
- `forward_daemon_command`가 프론트발 `Subscribe`/`Unsubscribe`를 차단한다(`src-tauri/src/commands/agent.rs`).
- `ProtocolClient.subscribeOutput`은 렌더러 등록만, wire `Subscribe` 미발신(`src/api/protocolClient.ts`).
- 데몬 wire 구독은 `router.rebuild → send_subscription_delta`(`src-tauri/src/commands/layout.rs`) 단독 발신.
- **어기면(프론트 직접 구독 부활):** 멀티뷰에서 N창 FromOldest 중복 storm + 진도 혼란 회귀.
