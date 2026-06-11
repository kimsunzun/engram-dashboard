# 모듈 6b — Headless 백엔드 테스트 브리핑 (담당: dcs24, Sonnet)

발신: ed12 (매니저)
근거: backend-lld-stage1.md §11(테스트), OutputSink/StatusSink trait(types.rs).
목적: **프론트엔드(Tauri/React) 없이** 백엔드 PTY 전체 흐름을 로그로 실측한다. 사용자 핵심 요구사항.

## 왜 가능한가

백엔드는 `OutputSink`/`StatusSink` trait으로 추상화돼 Tauri 의존이 없다(pty/ import 0개).
테스트용 sink(`LogSink`)를 만들어 PtyManager에 주입하면, 실제 Tauri Channel 없이 PTY 출력·상태를 로그로 관찰할 수 있다.

## 1. LogSink (테스트용 sink)

`examples/headless.rs` 안에 정의 (또는 별도 모듈, 자유). OutputSink + StatusSink 둘 다 구현:

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

> ⚠️ 주석 필수: 이 LogSink는 PTY 출력을 그대로 로그에 찍는다. **실제 claude 에이전트 연결 시에는 API 키 마스킹(tracking T-1)이 필요** — 지금은 cmd.exe/pwsh 테스트라 안전하지만 주석으로 명시.

## 2. examples/headless.rs 시나리오

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

## 검증 기준 (이게 Phase 1 결승선)

로그에서 다음이 순서대로 관찰돼야 PASS:
1. `spawned` + agent list updated(count=1)
2. PTY out 이벤트 여러 건 (cmd 프롬프트)
3. write 후 `echo headless-test` 출력이 PTY out에 보임
4. STATUS: **Running → Exiting → Killed** 순서로 전이
5. kill 후 list 비고(count=0), recv_timeout 안 걸리고 즉시 종료(spike처럼 ms 단위)
6. 프로세스 깔끔히 종료(좀비/행 없음)

## 실행 & 보고

```
cd src-tauri
cargo run --example headless
```

`orch 12 "⟁dcs24 headless PASS — spawn/out/write echo확인/resize/Running→Exiting→Killed/즉시종료, 로그발췌"` (FAIL이면 어느 단계/로그)

Cargo.toml에 examples 의존(필요시 dev-dependencies) 추가는 자유. 막히면 30분 내 중간보고.
