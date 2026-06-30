# ADR-0042: 구독 델타 = slot 단위 diff (agent-union 한계 보완)

- 상태: 확정 (2026-07-01, 근거: S14 모듈① 4차 FIX-3)
- 관련: ADR-0040 · ADR-0041 · ADR-0043 · src-tauri/src/output_router.rs · src-tauri/src/commands/layout.rs · step-log S14

## 맥락
layout이 바뀔 때(창 배치·agent 배정 변경) 데몬 구독과 출력 평면 cursor를 어떻게 동기화하나. 처음엔 "현재 보이는 agent 집합"(agent-union)만 diff했는데, 같은 agent를 보는 **창 수**가 바뀌는 경우를 놓친다:
- **1→2**: 이미 보던 agent를 새 창이 추가로 보기 시작 — agent 집합은 그대로라 델타 0 → 새 창 빈 화면.
- **2→1**: 여러 창 중 하나만 닫힘 — agent 집합은 그대로라 델타 0 → 닫힌 창의 cursor 누수.

## 결정
구독 델타(`SubscriptionDelta`)를 **slot 단위((window, agent) 쌍) diff**로 산출한다.
- agent 단위 토글(`to_subscribe`/`to_unsubscribe` — 데몬 wire 구독 발신용)에 더해,
- **slot 단위 변화**를 별도로 잡는다: `slots_to_replay`(새로 생긴 (window, agent) → mount-즉시-replay) · `slots_to_drop`(사라진 (window, agent) → cursor 폐기, 마지막이면 content drop).
- `OutputRouter`가 두 축(agent 집합 ↔ slot 집합)을 함께 산출하고, `send_subscription_delta`가 네 메서드(subscribe/unsubscribe/replay_slots/drop_slots)를 호출한다.

## 거부한 대안
- **agent-union diff만** — 1→2·2→1(부분 창 변경)을 못 잡아 새 창 빈 화면 + 죽은 cursor 누수. (위 맥락의 근본 한계.)
- **매 layout마다 cursor 전체 재구성(diff 없이 rebuild)** — 정상 전달 중인 cursor를 날려 중복 replay·진도 손실. delta가 정상 cursor를 보존하는 이점을 버린다.

## 근거
4차 FIX-3에서 slot 단위 diff로 1→2/2→1 케이스를 닫았다(`commands/layout.rs send_subscription_delta`·`connection.rs DropSlots` arm 주석). 채널 full silent drop로 새어나간 full 누수는 `sweep_orphans`/`reconcile_slots`(ADR-0043)가 별도로 흡수한다 — 델타는 정상 경로, reconcile은 안전망으로 역할을 분리했다.

## 영향 / 불변식
- `SubscriptionDelta`가 `slots_to_replay`/`slots_to_drop`을 든다(`src-tauri/src/output_router.rs`).
- `send_subscription_delta`가 4개 메서드를 호출한다(`src-tauri/src/commands/layout.rs`).
- slot 키 = (window_label, agent) = `ViewSlotKey`.
- **어기면(agent-union만 diff):** 같은 agent를 보는 창 수 변화(부분 mount/close) 누락 → 빈 화면·cursor 누수.
