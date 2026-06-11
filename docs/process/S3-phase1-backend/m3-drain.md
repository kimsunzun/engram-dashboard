# 모듈 3 — pty/drain.rs 브리핑 (담당: dco23, Opus)

발신: ed12 (매니저)
근거: `docs/backend-lld-stage1.md` §6 (drain thread), §9 (상태머신), §10 (동시성).
**§6 의사코드를 그대로 구현**한다. 이 모듈의 불변식 위반은 데드락/유실로 직결.

## 목표

`src-tauri/src/pty/drain.rs`:
1. `spawn_drain_thread(...)` — OS thread 생성 (tokio 아님, std::thread)
2. `drain_loop(...)` — read→send 루프
3. drain 종료 시 상태 전이 + 완료 신호

## 시그니처 (인자 보강 — LLD §6 의사코드는 reader만 보여주나 실제 필요한 것)

```rust
pub fn spawn_drain_thread(
    session: Arc<PtySession>,
    reader: Box<dyn Read + Send>,        // master.try_clone_reader() 결과
    status_sink: Arc<dyn StatusSink>,    // 종료 시 status_changed 호출용 (PtySession엔 없음, manager가 보유)
    done_tx: std::sync::mpsc::Sender<()>,// G-1 완료 신호 (session.drain_done_rx와 짝)
) -> std::thread::JoinHandle<()>;
```

> session.new()가 drain_handle/done_rx를 None으로 두고 manager가 사후 주입하는 구조와 맞물린다.
> manager의 spawn_agent가: (tx,rx)=channel() → session.drain_done_rx에 rx 주입 → spawn_drain_thread(session, reader, status_sink, tx) → 반환 handle을 session.drain_handle에 주입.
> 이 연결은 manager.rs(모듈 5)에서 한다. drain.rs는 위 시그니처만 제공.

## drain_loop (§6 의사코드 — 그대로)

```
buf = [0u8; 4096]
loop {
    // 1. blocking read (read 자체가 자연 배칭)
    n = match reader.read(&mut buf) {
        Ok(0) | Err(_) => break,        // EOF(master drop) or Err → 종료
        Ok(n) => n,
    };
    // 2. shutdown 보조 확인 (EOF로 먼저 깨지는 게 보통)
    if session.shutdown.load(Relaxed) { break; }
    // 3. seq 발급 + 즉시 send (C2: partial batch 정체 없음)
    let seq = session.seq.fetch_add(1, Relaxed);
    let data = buf[..n].to_vec();
    let event = PtyEvent { agent_id: session.id, seq, data_b64: base64(&data) };
    // 4. replay 저장 (brief lock — 즉시 해제)
    session.replay.lock().push(PtyChunk { seq, data });
    // 5. subscriber 스냅샷 후 lock 밖 send  ★불변식★
    let sinks = session.subscribers.lock().clone();   // clone 후 즉시 lock 해제
    let mut dead = vec![];
    for sink in sinks {
        if sink.send(event.clone()).is_err() { dead.push(sink.sink_id()); }
    }
    // 6. 죽은 구독자 제거
    if !dead.is_empty() {
        session.subscribers.lock().retain(|s| !dead.contains(&s.sink_id()));
    }
}
// 루프 탈출 → 상태 전이(단일 함수, M5 race 방지) + 알림
// child.try_wait()로 exit code 판별, shutdown flag면 Killed, 아니면 Exited{code}
transition(&session, ...);
status_sink.status_changed(session.id, <new status>);
// G-1: 완료 신호 (kill_agent의 recv_timeout(5s)가 수신)
let _ = done_tx.send(());
```

## ★핵심 불변식 (리뷰 필수)★

1. **`sink.send()` 호출 시 어떤 lock도 보유 금지.** subscribers를 `clone()`으로 스냅샷 뜬 뒤 lock 해제하고, 그 복사본을 돌며 send. (§10 규칙3)
2. `replay.lock()` 과 `subscribers.lock()` 을 **동시 보유 금지** — 각각 짧게 잡고 즉시 해제. (subscribe 함수만 두 lock 동시, drain은 절대 금지)
3. 종료 경로는 한 번만 실행 — EOF/Err/shutdown 어느 것으로 깨든 transition은 단일 함수로 idempotent하게.

## 상태 전이 (transition)

§9 상태머신 참고. shutdown flag가 켜져 있으면 `Killed`, 아니면 child exit code로 `Exited{code}`.
transition 함수 위치는 dco23 판단(session impl 또는 drain 내부 헬퍼). 단 status는 Mutex<AgentStatus>이므로 lock 후 전이.

## 규칙·품질

- tauri import 0, unsafe 0. std::thread::spawn 사용.
- base64는 base64 crate (Engine::encode). `use base64::Engine`.
- 동시성 불변식마다 *왜* 한국어 주석. 특히 "lock 밖 send" 이유.
- **완료 전 cargo fmt 필수** → cargo fmt --check 통과. cargo build 통과.

## 보고

`orch 12 "⟁dco23 drain.rs 완료 — drain_loop(lock밖send)+종료전이+done_tx, fmt/build OK"`
주의: session.rs가 dr26 리뷰 중이다. session 구조 변경 블로커가 나오면 ed12가 알린다 — 그 경우 drain 일부 조정 가능. 막히면 30분 내 중간보고.
