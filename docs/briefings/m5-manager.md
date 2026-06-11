# 모듈 5 — pty/manager.rs 브리핑 (담당: dco23, Opus)

발신: ed12 (매니저)
근거: `docs/backend-lld-stage1.md` §5 (PtyManager), §6 (kill 시퀀스), §10 (락순서), 자원소유표.
**Phase 1 마지막 모듈. 모든 모듈의 결합부**다. session/drain/windows/types를 전부 묶는다.

## 구조체 (§5)

```rust
pub struct PtyManager {
    sessions: Arc<RwLock<HashMap<AgentId, Arc<PtySession>>>>,
    status_sink: Arc<dyn StatusSink>,   // C1: Tauri AppHandle 아님. trait 주입(테스트 시 Noop)
}
```

## 메서드 (§5 시그니처 그대로)

new / spawn_agent / subscribe / unsubscribe / write_stdin / resize / kill_agent / list_agents / get_snapshot / shutdown_all

### spawn_agent(&self, cwd: &Path) -> Result<AgentInfo, PtyError>

```
1. native_pty_system().openpty(PtySize{rows,cols,..})   // 기본 24x80
2. CommandBuilder("claude" 또는 설정된 셸), cwd 지정 → pair.slave.spawn_command() → child
   (지금은 검증용으로 cmd.exe/pwsh 등 받아도 됨 — 인자화는 추후. LLD 기본 동작 우선)
3. #[cfg(windows)] JobObjectHandle::new() → assign(child.process_id())   // spike/windows.rs 그대로
4. reader = pair.master.try_clone_reader()   // ★ master를 session에 넣기 전에 reader 확보 ★
5. PtySession::new(PtySessionInit{ id, cwd, master: pair.master, writer: pair.master.take_writer(), child, cols, rows, job_handle, ... })
   → Arc<PtySession>
6. (done_tx, done_rx) = std::sync::mpsc::channel()
   session.drain_done_rx.lock() = Some(done_rx)
7. handle = spawn_drain_thread(session.clone(), reader, self.status_sink.clone(), done_tx)
   session.drain_handle.lock() = Some(handle)
8. sessions.write().insert(id, session)
9. self.status_sink.agent_list_updated(self.list_agents())   // 목록 갱신 알림
10. return AgentInfo
```

### kill_agent(&self, agent_id) -> Result<(), PtyError>  ★§6 6단계 그대로★

```
0. sessions.read() 로 Arc<PtySession> clone, read lock 즉시 해제   (§10 규칙1)
1. session.shutdown.store(true, Release)
2. session.child.lock().kill()
3. session.child.lock().wait()                       // reap, 좀비 방지
4. #[cfg(windows)] session.job_handle.terminate(1)   // 손자까지 전멸 → ConPTY slave 해제
5. session.master.lock().take()                      // C3: master drop → ClosePseudoConsole → reader EOF
6. session.drain_done_rx.lock().take()
       .and_then(|rx| rx.recv_timeout(Duration::from_secs(5)).ok())
   // timeout이면 drain_handle detach (drop) — Arc 참조 끊기면 자연 정리, leak 아님
7. sessions.write().remove(&agent_id)
8. self.status_sink.agent_list_updated(self.list_agents())
```

> **상태 알림 책임 분담 (중복 호출 금지):**
> - 개별 상태 전이(`status_changed`, Killed/Exited)는 **drain thread가 단독** 호출(transition에서). kill_agent는 호출하지 않는다.
> - 목록 변경(`agent_list_updated`)은 **manager가** spawn/kill 끝에 호출.

### 나머지

- **subscribe/unsubscribe**: sessions.read()로 session 찾아 `session.subscribe(sink)` / `session.unsubscribe(sink_id)` 위임. (C4 로직은 session.rs에 이미 있음)
- **write_stdin**: session 찾아 `session.writer.lock().write_all(data)`.
- **resize**: session 찾아 `session.master.lock()`에 resize + `session.cols/rows` atomic 갱신.
- **list_agents**: sessions.read() 순회 → Vec<AgentInfo> (각 session의 status/cols/rows snapshot).
- **get_snapshot**: session.replay.lock().snapshot().
- **shutdown_all**: 모든 agent_id에 kill_agent 호출(또는 동등 정리). 앱 종료 경로.

## 불변 규칙 (리뷰 필수)

- **락순서 §10 규칙1**: `sessions` RwLock은 조회용 — Arc clone 뜨고 **즉시 해제**한 뒤 session 내부 lock 취득. sessions lock 보유 중 session 내부 lock 취득 금지.
- kill_agent의 6단계 순서 절대 변경 금지(spike로 검증된 순서). 특히 master.take()는 4(Job terminate) 이후 5.
- tauri import 0. (status_sink는 trait, AppHandle 아님)
- 에러: 없는 agent_id → PtyError. lock poison → expect(fail-fast, D-3 정책).

## 코드 품질

- spawn_agent/kill_agent 각 단계에 *왜* 한국어 주석 (특히 reader 확보 타이밍, master.take 위치, 알림 분담).
- **완료 전 cargo fmt 필수** → fmt --check 통과. cargo build 통과 (이제 dead_code 경고 대부분 해소될 것).

## 보고

`orch 12 "⟁dco23 manager.rs 완료 — spawn/kill(6단계)/subscribe위임/resize/shutdown_all, fmt/build OK, dead_code 경고 N건"`
PtySession::new의 PtySessionInit 필드가 session.rs 정의와 안 맞으면 맞춰서 조정(같은 작성자니 일관되게). 막히면 30분 내 중간보고.
