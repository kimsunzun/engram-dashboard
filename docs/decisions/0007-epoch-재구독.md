# ADR-0007: Resubscribe on epoch map replacement

- 상태: 확정 (S9 §18-d) · 부분 폐기 by ADR-0163 (the two clauses: epoch +1 increment on every map replacement and the resubscribe deps)
- 관련: CLAUDE.md 핵심 불변식·프론트 통합 규칙 · `output_core.rs(epoch)` · `TerminalSlot.tsx` · Amended by ADR-0046 (frontend epoch authority clause: SubscribeAck alone → src-tauri decide_epoch as the first-stage filter + adopt the epoch of the filtered frame/marker — it left the [agentId, epoch] resubscribe principle as is at that time, but ADR-0163 later discarded that deps clause) · Amended by ADR-0163 (the two clauses: epoch +1 increment on every map replacement and the resubscribe deps)

## 맥락
On restart/fresh fallback, the session map is replaced **under the same AgentId**. If the frontend keeps the old subscription, it either never receives output from the new session or receives stale notifications from the old one.

## 결정
Every same-AgentId map replacement bumps the **epoch by +1**. The frontend sets the deps of the subscription effect to `[agentId, epoch]`, so when the epoch changes it goes through reset → resubscribe → replay again. status_changed carries the epoch, so the frontend drops notifications with a mismatched epoch.

## 거부한 대안
- **Subscribe by id only** — a restart goes undetected and the frontend stays on the old channel. A stale terminal notification overwrites the new session.

## 근거
The S9 fable review (Mn-1: status_changed carries the epoch) confirmed a race in which a stale Killed overwrites a freshly revived session → the epoch tells them apart.

## 영향 / 불변식
- The TerminalSlot subscription effect deps must be `[agentId, epoch]`.
- Together with frontend seq dedup, the replay→live order is preserved (→ no ADR, frontend integration rule).
