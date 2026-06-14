# ADR-0007: epoch 맵교체 재구독

- 상태: 확정 (S9 §18-d)
- 관련: CLAUDE.md 핵심 불변식·프론트 통합 규칙 · `output_core.rs(epoch)` · `TerminalSlot.tsx`

## 맥락
restart/fresh fallback 시 **같은 AgentId로 세션 맵을 교체**한다. 프론트가 옛 구독을 유지하면 새 세션 출력을 못 받거나 옛 세션의 stale 알림을 받는다.

## 결정
같은 AgentId 맵 교체마다 **epoch +1**. 프론트는 구독 effect의 deps를 `[agentId, epoch]`로 두어, epoch가 바뀌면 reset → 재구독 → replay를 다시 탄다. status_changed에 epoch를 동봉해 프론트가 epoch 불일치 알림을 버린다.

## 거부한 대안
- **id만으로 구독** — 재시작을 감지 못 해 옛 채널에 머무름. stale terminal 알림이 새 세션을 덮음.

## 근거
S9 fable 리뷰(Mn-1: status_changed epoch 동봉)에서 stale Killed가 갓 살아난 세션을 덮는 경합을 확인 → epoch로 분별.

## 영향 / 불변식
- TerminalSlot 구독 effect deps는 반드시 `[agentId, epoch]`.
- 프론트 seq dedup과 함께 replay→live 순서 보존(→ ADR 없음, 프론트 통합 규칙).
