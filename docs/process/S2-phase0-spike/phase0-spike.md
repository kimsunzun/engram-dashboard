# Phase 0 — Spike Test Briefing (owner: dco23, Opus)

From: ed12 (manager)
Purpose: **Before the main implementation**, verify the key assumptions of the PTY kill sequence on Windows by measurement.
If this fails, the LLD's kill design has to be fixed, so do this before the main code.

For the overall design context see `docs/backend-lld-stage1.md` (especially §6 drain/kill, §13 Windows JobObject).

---

## The 3 assumptions to verify

1. With **portable-pty 0.8.1**, a child process can be spawned normally on Windows and its stdout can be read.
2. After the kill sequence `child.kill() → child.wait() → TerminateJobObject → master.take()(drop)`,
   the **drain (reader) thread wakes up on EOF and exits within 5 seconds**.
   (The assumption: dropping the master makes ConPTY call ClosePseudoConsole → the reader is released with EOF/Err.)
3. **Windows Job Object + KILL_ON_JOB_CLOSE** also cleans up grandchild processes.

---

## What to do

### 1. Add Cargo.toml dependencies (LLD §2 — the pinned versions must never change)

```toml
[dependencies]
tauri        = { version = "2.4", features = [] }   # 기존 "2" → "2.4" 로 고정. 2.5 금지(Channel silent failure)
tauri-plugin-opener = "2"
serde        = { version = "1", features = ["derive"] }
serde_json   = "1"
portable-pty = "0.8.1"
uuid         = { version = "1.8", features = ["v4", "serde"] }
thiserror    = "1.0"
base64       = "0.22"
tokio        = { version = "1", features = ["rt-multi-thread", "macros"] }
tracing      = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "Win32_System_JobObjects",
    "Win32_Foundation",
    "Win32_System_Threading",
] }
```

### 2. Write the spike binary

Location: `src-tauri/examples/spike.rs` (the examples directory — does not pollute the main code, delete after verification)

Flow:
```
1. portable-pty: native_pty_system() → openpty(PtySize { rows:24, cols:80, .. })
2. CommandBuilder::new("cmd.exe"), cwd 지정 → pair.slave.spawn_command(cmd) → child
3. (Windows) JobObject 생성:
   - CreateJobObjectW
   - SetInformationJobObject(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
   - child.process_id() → OpenProcess → AssignProcessToJobObject
4. master.try_clone_reader() → OS thread spawn:
     loop { read into buf; eprintln 수신 바이트 수; break on 0(EOF)/Err }
     thread 종료 시 std::sync::mpsc 로 done 신호 send
5. 2초간 출력 수신 (cmd.exe 프롬프트/echo 등)
6. kill 시퀀스 실행 + 각 단계 타임스탬프 로그(std::time::Instant):
     child.kill() → child.wait() → TerminateJobObject(job, 1) → drop(master)
7. done 신호 recv_timeout(Duration::from_secs(5)):
     - Ok  → reader EOF로 정상 종료, 경과시간 출력  ✅
     - Err(Timeout) → reader가 안 풀림 ❌ (가정 2 깨짐 — 즉시 보고)
```

Print the time each step takes with eprintln using `Instant`. The key is to observe at which step the reader is released (the kill or the master drop).

### 3. Run and report

```
cd src-tauri
cargo run --example spike
```

Report the result to ed12 (pane 12) via orch:
- PASS: `orch 12 "⟁dco23 spike PASS — reader join Xms, 가정 1·2·3 확인"`
- FAIL: `orch 12 "⟁dco23 spike FAIL — <어느 가정/어느 단계에서 막힘> <로그 발췌>"`

---

## Cautions

- The spike is throwaway verification code. Writing the main modules (pty/session.rs etc.) is directed separately by ed12 after the spike is confirmed to pass.
- If you get stuck, report even an intermediate state within 30 minutes. Do not drag on alone.
- If you judge that a version change is needed, do not change it on your own — report to ed12 first.
