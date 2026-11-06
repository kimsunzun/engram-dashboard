# Module 6b — Headless backend test briefing (owner: dcs24, Sonnet)

From: ed12 (manager)
Source: backend-lld-stage1.md §11 (tests), the OutputSink/StatusSink traits (types.rs).
Purpose: **without the frontend (Tauri/React)**, measure the whole backend PTY flow through logs. A core user requirement.

## Why it is possible

The backend is abstracted behind the `OutputSink`/`StatusSink` traits and has no Tauri dependency (zero imports in pty/).
If you make a test sink (`LogSink`) and inject it into PtyManager, you can observe PTY output and status through logs without a real Tauri Channel.

## 1. LogSink (test sink)

Define it inside `examples/headless.rs` (or a separate module, your choice). Implement both OutputSink and StatusSink:

```rust
struct LogSink { id: SinkId }

impl OutputSink for LogSink {
    fn send(&self, event: PtyEvent) -> Result<(), SinkError> {
        // PTY 출력 수신 로그. data_b64를 디코드해 사람이 읽게 일부 출력.
        let bytes = base64_decode(&event.data_b64);
        tracing::info!(agent=%event.agent_id, seq=event.seq, "PTY out: {:?}", String::from_utf8_lossy(&bytes));
        Ok(())
    }
    fn sink_id(&self) -> SinkId { self.id }
}

impl StatusSink for LogSink {
    fn status_changed(&self, id: AgentId, status: AgentStatus) {
        tracing::info!(agent=%id, ?status, "STATUS changed");   // Running→Exiting→Killed 관찰
    }
    fn agent_list_updated(&self, agents: Vec<AgentInfo>) {
        tracing::info!(count=agents.len(), "agent list updated");
    }
}
```

> ⚠️ A comment is required: this LogSink writes PTY output to the log as is. **When connecting a real claude agent, API key masking (tracking T-1) is needed** — it is safe for now because the test uses cmd.exe/pwsh, but state it in a comment.

## 2. examples/headless.rs scenario

```rust
fn main() {
    init_logging();                          // 테스트니 set_log_level("debug")로 켠다
    set_log_level("debug").unwrap();

    let status_sink = Arc::new(LogSink::new());
    let manager = PtyManager::new(status_sink.clone());

    // 1) spawn — cmd.exe(또는 pwsh) 띄움
    let info = manager.spawn_agent(Path::new(".")).expect("spawn");
    tracing::info!(?info, "spawned");

    // 2) subscribe — 출력 수신 시작
    let out_sink = Arc::new(LogSink::new());
    let sink_id = manager.subscribe(info.id, out_sink).expect("subscribe");

    // 3) 잠시 대기하며 프롬프트/출력 수신 (sleep 1~2s)
    std::thread::sleep(Duration::from_secs(2));

    // 4) write_stdin — echo 명령
    manager.write_stdin(info.id, b"echo headless-test\r\n").expect("write");
    std::thread::sleep(Duration::from_secs(1));

    // 5) resize
    manager.resize(info.id, 100, 30).expect("resize");

    // 6) kill — Running→Exiting→Killed 전이가 로그에 찍혀야 함
    manager.kill_agent(info.id).expect("kill");

    // 7) 종료 확인 — list_agents 비었는지
    tracing::info!(remaining=manager.list_agents().len(), "after kill");
}
```

## Pass criteria (this is the Phase 1 finish line)

PASS if the following are observed in the log, in order:
1. `spawned` + agent list updated(count=1)
2. Several PTY out events (the cmd prompt)
3. After the write, the `echo headless-test` output is visible in PTY out
4. STATUS: transitions in the order **Running → Exiting → Killed**
5. After kill, the list is empty (count=0), recv_timeout does not trigger, and it ends immediately (in ms, as in the spike)
6. The process exits cleanly (no zombie/hang)

## Run and report

```
cd src-tauri
cargo run --example headless
```

`orch 12 "⟁dcs24 headless PASS — spawn/out/write echo확인/resize/Running→Exiting→Killed/즉시종료, 로그발췌"` (on FAIL, which step/log)

Adding example dependencies to Cargo.toml (dev-dependencies if needed) is up to you. If stuck, send an interim report within 30 minutes.
