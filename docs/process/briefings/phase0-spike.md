# Phase 0 — Spike Test 브리핑 (담당: dco23, Opus)

발신: ed12 (매니저)
목적: **본 구현 전에** Windows에서 PTY kill 시퀀스의 핵심 가정을 실측으로 검증한다.
이게 실패하면 LLD의 kill 설계를 고쳐야 하므로, 코드 본 구현보다 먼저 한다.

전체 설계 맥락은 `docs/backend-lld-stage1.md` (특히 §6 drain/kill, §13 Windows JobObject) 참조.

---

## 검증하려는 가정 3가지

1. **portable-pty 0.8.1** 로 Windows에서 자식 프로세스를 정상 spawn하고 stdout을 읽을 수 있다.
2. kill 시퀀스 `child.kill() → child.wait() → TerminateJobObject → master.take()(drop)` 후
   **drain(reader) 스레드가 5초 이내에 EOF로 깨어나 종료**된다.
   (master를 drop하면 ConPTY가 ClosePseudoConsole를 호출 → reader가 EOF/Err로 풀린다는 가정)
3. **Windows Job Object + KILL_ON_JOB_CLOSE** 로 손자 프로세스까지 정리된다.

---

## 할 일

### 1. Cargo.toml 의존성 추가 (LLD §2 — 버전 고정 절대 변경 금지)

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

### 2. spike 바이너리 작성

위치: `src-tauri/examples/spike.rs` (examples 디렉토리 — 본 코드 오염 없음, 검증 후 삭제)

흐름:
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

`Instant` 로 각 단계 소요시간을 eprintln 한다. 어떤 단계에서 reader가 풀리는지(kill인지 master drop인지) 관측이 핵심.

### 3. 실행 & 보고

```
cd src-tauri
cargo run --example spike
```

결과를 ed12(pane 12)에 orch로 보고:
- PASS: `orch 12 "⟁dco23 spike PASS — reader join Xms, 가정 1·2·3 확인"`
- FAIL: `orch 12 "⟁dco23 spike FAIL — <어느 가정/어느 단계에서 막힘> <로그 발췌>"`

---

## 주의

- spike는 throwaway 검증 코드다. 본 모듈(pty/session.rs 등) 작성은 spike 통과 확인 후 ed12가 별도 지시한다.
- 막히면 30분 안에 중간 상태라도 보고. 혼자 오래 끌지 말 것.
- 버전 변경이 필요하다고 판단되면 임의 변경 말고 ed12에 먼저 보고.
