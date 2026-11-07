# ADR-0247: Usage picking is received through the StatusSink hook and pump drain and applied straight to the book on the pump thread

- 상태: 확정 (2026-09-27, basis: user decision + TRD §7 #3)
- 관련: TRD §7 #3(attached to #9) · `docs/decisions/0001-kill-2동사.md`(pump ownership discipline) · step-log S21

## 맥락
We had to decide where to pick up the usage signal that a conversational agent drops in passing (passive) while it outputs, and when to reflect it in the state (the book). The original decision was "collect it in the daemon pending map and merge it on the next request", but then the picked result cannot reach the screen until the next request arrives.

## 결정
The picking outlet is the **StatusSink hook + pump drain**, and it is **applied straight to the book (the usage state) on the pump thread**. This lock is a leaf lock that holds no I/O · await · other lock, so we judged it safe to take the lock and merge on the pump thread, and emit outside the lock.

## 거부한 대안
- **Injecting a port into `open_spawn`** — handing the decoder a port adds a field to `open_spawn`, so the three implementations and about ten call sites change along with it, and it gives the decoder side effects (drain keeps the decoder pure — TRD §1-2 「훅만으로는 안 맞는 이유」 · §3 #1).
- **The `Structured` outlet** (codex `decoder.rs:18-25` — lines at time of writing) — the usage limit is per-account state, but `OutputEvent::Structured` is conversation output that rides the per-agent replay ring and subscriber fan-out. Also, if that escape hatch were used, the frontend would draw `kind` as a label and the payload as a code block, so codex method names · protocol JSON would show on the user's screen. Source = the pre-TRD fact survey `.claude/handoff/attachments/usage-slot-trd-facts-2026-09-27.md:22-30` · the head comment of `crates/engram-dashboard-agent/src/backend/codex/decoder.rs` (「모르는 것은 버린다 — `Structured` 를 배출구로 쓰지 않는다」 · 「사용량 한도 알림은 이벤트가 아니라 `take_usage` 로 나간다」).
- **Daemon pending map + merge on request** (the old decision) — replaced by the 2026-09-27 user decision: the reason for replacing it is that the picked result cannot reach the screen until the next request.
- **A global FIFO queue** — it drops items when full and the drain order can be reversed.

The old rejected item "take the book lock and merge on the pump thread" was **flipped to adopted** by this revision — the ground is that the lock is a leaf lock with no I/O · await · other lock.

## 근거
User decision 2026-09-27 · TRD §7 #3.

## 영향 / 불변식
The pattern of holding a lock and merging on the pump thread is now allowed by this decision **only for this leaf lock** — do not generalize it to other heavy locks (holding I/O · await · another lock). Implementation commits: `7aad8f7`, `4fd426a`, `56776d6`.
