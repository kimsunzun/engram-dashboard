# TRD — 세션 Reaper + 종료 분류 (ADR-0019 구현 설계)

근거: ADR-0019(종료 분류·disposition) · ADR-0001(kill 2동사) · ADR-0005(finalize 1회) · ADR-0006(락 순서) · ADR-0007(epoch). consult `20260616-215346-consult-session-reaper`(GPT·Gemini·Claude 블라인드 교차검증, judge 최신뢰=GPT) 종합.

## 설계 (correctness-merge 결과)
**단일 supervisor 스레드 + std unbounded `mpsc<ReapMsg>`.** pump가 종료 시 이벤트만 발행, reaper 한 스레드가 소비해 맵 제거+disposition+통지.

### 새 타입
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

### 흐름
1. **spawn_session**: pump에 `{ id, epoch, reaper_tx: Sender<ReapMsg> clone, intent: Arc<AtomicU8>, shutting_down: Arc<AtomicBool> }`를 주입(transport.start 경유 또는 pump 클로저 캡처). `intent`는 세션별 신규 atomic, `shutting_down`은 manager 전역 1개. **활성화(spawn) 시 프로필 auto_restore=true로** 세팅(강제종료 후 부팅 복원 대상).
2. **pump 종료(finish 승자)**: 기존 `OutputCore.finish()`의 `finalized.swap(AcqRel)`가 true 반환한 그 경로에서, **그 순간** intent·shutting_down을 load(snapshot)해 `ReapMsg` 빌드 → `reaper_tx.send`. (finish 자체·done_tx·status 통지 분담은 ADR-0005 그대로 — reaper_tx 송신만 추가.)
3. **reaper 루프(단일 스레드)**: `while let Ok(msg) = rx.recv() { manager.reap_one(msg) }`. 모든 Sender drop 시 종료(+명시 Stop 메시지 옵션).
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
5. **kill_agent**(변경): intent=UserKill 태깅 **shutdown 전에** → `transport.shutdown()` → `core.join_pump(5s)`. **맵 제거·disposition·통지는 직접 안 함**(reaper에 위임 — done 단일 소비자). join_pump는 pump 스레드 join만.
6. **shutdown_all**(변경): `shutting_down.store(true)` **먼저** → 그 다음 각 세션 kill. (set이 kill보다 늦으면 그 틈에 종료된 세션이 크래시로 오분류 — race.)

### decide(msg) — disposition 판정
```
if msg.shutting_down_at_finish        => KeepAsIs            // 데몬 셧다운: 손 안 댐 → 부팅 복원
match (intent_at_finish, reason):
  (UserKill, _)                        => DeleteProfile       // 유저 kill
  (None, Exited{code:0})               => DeleteProfile       // 정상 /exit
  (None, _)                            => KeepDisableAutoRestore // 크래시/EOF/exit≠0/signal: 보수적
```
- **exit code 불명(EOF/StreamClosed/Error)도 크래시 취급**(보수적 — code 0 확실할 때만 삭제). consult 합의.
- **apply_disposition은 downgrade-only**: auto_restore를 절대 true로 *올리지 않음*(KeepDisableAutoRestore=false로만). 이게 하드킬 안전망 성립 조건 — reaper 못 돌면 auto_restore=true 잔류→부팅 복원.

## 불변식 준수 (변경 0)
- **kill 2동사(ADR-0001)**: shutdown→join_pump 그대로. reaper는 그 뒤 done 소비만 추가.
- **finalize 1회(ADR-0005)**: pump의 finalized.swap 승자 1회 그대로. reaper_tx.send는 그 승자 경로에서 1회.
- **락 순서(ADR-0006)**: write lock 구간=epoch검증+remove만. ProfileRegistry mutate·status_sink는 lock 밖. sessions lock과 ProfileRegistry lock 보유 구간 시간상 비중첩.
- **epoch(ADR-0007)**: reap 전 epoch 일치 검증 → restart로 자리 바뀐 유령 done이 새 세션 안 지움.
- **idempotency**: kill_agent(위임)·reaper가 같은 done을 봐도 remove Some 승자 1명만 disposition·통지.

## 테스트 (headless harness, 실제 PTY)
- 자연 종료(셸 `exit 0`) → 세션 맵에서 reap + 프로필 삭제 + agent-list-updated 1회.
- 크래시(`exit 1`) → 프로필 유지 + auto_restore=false(예약 복귀).
- 유저 kill → 프로필 삭제(intent 태깅 경로).
- shutting_down=true 중 종료 → 프로필 유지(disposition 스킵), 맵 제거는 됨.
- epoch race: 옛 epoch ReapMsg가 새 세션(현 epoch) 안 지움.
- idempotency: 같은 ReapMsg 2회 → 통지 1회.
- 락/데드락: kill_agent와 자연종료 동시 → 패닉/행 없음.
```
cargo test -p engram-dashboard-core
```
