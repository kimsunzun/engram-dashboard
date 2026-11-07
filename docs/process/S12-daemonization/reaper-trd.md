# TRD — session Reaper + exit classification (ADR-0019 implementation design)

Basis: ADR-0019 (exit classification · disposition) · ADR-0001 (kill 2 verbs) · ADR-0005 (finalize once) · ADR-0006 (lock order) · ADR-0007 (epoch). Synthesis of consult `20260616-215346-consult-session-reaper` (GPT, Gemini and Claude blind cross-validation, judge's most trusted = GPT).

## Design (correctness-merge result)
**A single supervisor thread + std unbounded `mpsc<ReapMsg>`.** On termination the pump only emits an event; one reaper thread consumes it and does map removal + disposition + notification.

### New types
```rust
// 유저 의도 — kill 핸들러가 채움. PTY 관측 사실(TerminalReason)과 분리.
#[repr(u8)]
enum TerminationIntent { None = 0, UserKill = 1 }   // DaemonShutdown은 전역 플래그로 분리

// pump가 finish 승자일 때 1회 발행. intent/shutting_down은 ★finish 순간 snapshot★(live read 금지 — race).
struct ReapMsg {
    id: AgentId,
    epoch: u32,                 // stale done이 재spawn된 새 세션을 오삭제 못 하게(ADR-0007)
    reason: TerminalReason,     // 기존 enum: Exited{code}/Killed/Interrupted/StreamClosed/Cancelled/Error
    intent_at_finish: TerminationIntent,
    shutting_down_at_finish: bool,
}

enum Disposition { DeleteProfile, KeepDisableAutoRestore, KeepAsIs }
```

### Flow
1. **spawn_session**: inject `{ id, epoch, reaper_tx: Sender<ReapMsg> clone, intent: Arc<AtomicU8>, shutting_down: Arc<AtomicBool> }` into the pump (via transport.start or pump closure capture). `intent` is a new per-session atomic, `shutting_down` is a single manager-wide one. **On activation (spawn) set the profile's auto_restore=true** (so it is a boot-restore target after a forced termination).
2. **Pump termination (finish winner)**: on the very path where the existing `OutputCore.finish()`'s `finalized.swap(AcqRel)` returned true, load (snapshot) intent and shutting_down **at that moment**, build the `ReapMsg` → `reaper_tx.send`. (finish itself, done_tx and the split of status notification stay as in ADR-0005 — only the reaper_tx send is added.)
3. **Reaper loop (single thread)**: `while let Ok(msg) = rx.recv() { manager.reap_one(msg) }`. Ends when all Senders drop (+ optional explicit Stop message).
4. **reap_one**:
   ```
   let removed = { let mut m = sessions.write();
       if m.get(&id).map(|s| s.epoch) != Some(msg.epoch) { return; } // 유령/교체 무시
       m.remove(&id) };                       // ★ write lock 즉시 해제(Arc만 들고 나옴)
   let session = match removed { Some(s)=>s, None=>return };          // 패자 = no-op (idempotent)
   if !msg.shutting_down_at_finish {                                   // 셧다운이면 disposition 스킵
       apply_disposition(decide(&msg));                                // lock 밖: ProfileRegistry mutate(디스크 IO)
   }
   status_sink.agent_list_updated(list_agents());                      // lock 밖: 외부 콜백
   ```
5. **kill_agent** (changed): tag intent=UserKill **before shutdown** → `transport.shutdown()` → `core.join_pump(5s)`. **It does not do map removal, disposition or notification directly** (delegated to the reaper — done has a single consumer). join_pump only joins the pump thread.
6. **shutdown_all** (changed): `shutting_down.store(true)` **first** → then kill each session. (If the set comes later than the kill, a session that ended in that gap is misclassified as a crash — race.)

### decide(msg) — disposition decision
```
if msg.shutting_down_at_finish        => KeepAsIs            // 데몬 셧다운: 손 안 댐 → 부팅 복원
match (intent_at_finish, reason):
  (UserKill, _)                        => DeleteProfile       // 유저 kill
  (None, Exited{code:0})               => DeleteProfile       // 정상 /exit
  (None, _)                            => KeepDisableAutoRestore // 크래시/EOF/exit≠0/signal: 보수적
```
- **An unknown exit code (EOF/StreamClosed/Error) is also treated as a crash** (conservative — delete only when code 0 is certain). Consult agreement.
- **apply_disposition is downgrade-only**: it never *raises* auto_restore to true (KeepDisableAutoRestore only lowers it to false). This is the condition for the hard-kill safety net to hold — if the reaper cannot run, auto_restore=true remains → boot restore.

## Invariant compliance (0 changes)
- **kill 2 verbs (ADR-0001)**: shutdown→join_pump as is. The reaper only adds consuming done after that.
- **finalize once (ADR-0005)**: the pump's finalized.swap winner once, as is. reaper_tx.send happens once on that winner path.
- **Lock order (ADR-0006)**: the write-lock section = epoch check + remove only. ProfileRegistry mutate and status_sink are outside the lock. The sections holding the sessions lock and the ProfileRegistry lock do not overlap in time.
- **epoch (ADR-0007)**: verify the epoch matches before reaping → a ghost done whose place was taken by a restart does not delete the new session.
- **idempotency**: even if kill_agent (delegated) and the reaper see the same done, only the one winner whose remove is Some does disposition and notification.

## Tests (headless harness, real PTY)
- Natural exit (shell `exit 0`) → reaped from the session map + profile deleted + agent-list-updated once.
- Crash (`exit 1`) → profile kept + auto_restore=false (back to reserved).
- User kill → profile deleted (intent tagging path).
- Termination while shutting_down=true → profile kept (disposition skipped), map removal still happens.
- epoch race: an old-epoch ReapMsg does not delete the new session (current epoch).
- idempotency: the same ReapMsg twice → notification once.
- Lock/deadlock: kill_agent and a natural exit at the same time → no panic/hang.
```
cargo test -p engram-dashboard-core
```
