# 세션 경로 소유권·수명 지도 — 2026-09-09 실측 스냅샷

> ★**이 문서는 그 시점의 코드를 추적한 기록이고 정본이 아니다**★ — 정본은 코드와 그 안의 `// ADR-` 앵커다.
> 줄 번호는 **2026-09-09 · 브랜치 `v0.3.0/feat/codex-backend`** 기준이고, 코드가 움직이면 낡는다.
> 의심되면 여기 적힌 주장을 그 파일에서 다시 확인한다.
>
> **왜 만들었나:** codex 를 두 번째 백엔드로 붙이는 설계가 두 라운드 연속 적대 리뷰에서 BLOCK 을 받았고,
> 원인이 「층 이름(`transport`·`backend`·`decoder`·`capabilities`)으로 설계하고 코드를 나중에 대 본 것」으로
> 진단됐다. 블로커 13건 중 12건이 **외부 도구가 아니라 우리 구조와** 충돌했다. 그래서 설계를 더 쓰기 전에
> 「무엇이 무엇을 쥐고 있고, 누가 언제 지우고, 어느 층이 무엇을 알 수 있나」를 코드에서 먼저 뽑았다.
>
> **읽는 법 — 세 부분이다:** PART A = 객체별 소유·수명 목록(14개) · PART B = 설계가 실제로 필요로 하는
> 질문 10개의 답 · PART C = ★**「그 일을 할 함수나 필드가 없는 자리」 70건**★. 설계 문서(`docs/process/S21-codex-backend/trd-phase2a.md`)가
> PART B·C 를 입력으로 쓴다 — **거기서 베끼지 않고 가리킨다.**
>
> **본문은 영어다.** 추적 결과를 옮겨 적을 때 번역을 끼우면 원문 대조가 어려워지므로 그대로 실었다
> (이 저장소 문서 규약은 한국어이고, 이 머리말이 그 예외를 명시한다).

---

# Ownership map — traced facts for the second-backend design

READ-ONLY archaeology. Repo: `I:\Engram\apps\engram-dashboard-wt1`, branch `v0.3.0/feat/codex-backend`.
Every claim carries `file:line`. `UNVERIFIED` = inferred, line not read.
Paths are relative to repo root. Doc-comment quotes are translated/abridged in brackets; the
Korean original is at the cited line.

---

# PART A — per-object inventory

## A1. `AgentTransport` trait — `crates/engram-dashboard-agent/src/transport/mod.rs:41-57`

Full method list, verbatim signatures:

```rust
pub trait AgentTransport: Send + Sync {
    fn start(&self, core: Arc<OutputCore>);              // mod.rs:43
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError>;   // :45
    fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError>;    // :48
    fn interrupt(&self) -> Result<(), PtyError>;         // :51
    fn shutdown(&self);                                  // :54
    fn capabilities(&self) -> TransportCaps;             // :56
}
```

- **No method returns a reply.** Every fallible method returns `Result<(), PtyError>`; the only
  value-returning method is `capabilities()`, a pure snapshot of static booleans. There is **no
  request/response verb anywhere on this trait** (mod.rs:41-57).
- `&self` everywhere — no `&mut self`. Interior mutability is each impl's own business, and
  `Send + Sync` is required (mod.rs:41).
- Doc comments that forbid / constrain:
  - `start`: "출력 pump/stream 기동 → core 연결. spawn 직후 1회 호출." — called exactly once,
    right after spawn (mod.rs:42).
  - `resize`: "cols/rows의 보존(atomic 저장)은 AgentSession 책임." — transport must not own the
    persisted size (mod.rs:47).
  - `interrupt`: "≠kill. 진행 중 작업만 중단 — 프로세스는 살아 있다." (mod.rs:50).
  - `shutdown`: "자원 강제 종료(멱등). pump 종료 대기는 여기서 안 함(core.join_pump 몫)."
    — idempotent, and **must not** wait for the pump (mod.rs:53).
  - Module header: "transport는 바이트·이벤트를 만들어 `OutputCore::emit`/`finish`로 넘기기만 하면
    된다 … transport는 자기 자원의 수명만 책임진다." + "tauri import 0." (mod.rs:1-7).
- **What the trait does NOT have** (checked by reading the whole 57-line file): no `close_stdin`,
  no `request(...)`, no `cancel(id)`, no `is_alive()`, no `child_pid()`, no `kind()`/`name()`,
  no `set_decoder`, no way to learn *why* shutdown was called.

## A2. `OutputDecoder` trait — `transport/mod.rs:32-39`

```rust
pub trait OutputDecoder: Send {
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent>;   // mod.rs:35
    fn flush(&mut self) -> Vec<OutputEvent>;                  // mod.rs:38
}
```

- **Owner:** the pump thread, exclusively. Constructed by `backend::output_decoder(&profile.command)`
  (backend/mod.rs:477-479, called at manager.rs:1039), passed into `StdioTransport::open` as
  `Option<Box<dyn OutputDecoder>>` (stdio.rs:65-69), parked in `decoder: Mutex<Option<Box<dyn
  OutputDecoder>>>` (stdio.rs:53), then `take()`n in `start` and **moved into the pump thread**
  (stdio.rs:204-207, used at :228-235 and :251-257).
- Thread: pump thread only. Doc states this explicitly — "decoder 는 … 가변 상태를 들고, pump
  스레드(단일)가 `&mut` 로 배타 소유한다 — 그래서 `Send`(스레드로 move)만 요구하고 `Sync` 는 요구하지
  않는다(공유 접근 없음). epoch 교체 = 새 transport = 새 decoder 라 리셋이 자동이다." (mod.rs:26-29).
- **It can send nothing back.** Both methods return `Vec<OutputEvent>` and take no sink, no channel,
  no handle to the transport, no handle to the core. Its only output channel is the return value,
  which the pump immediately feeds to `pump_core.emit(ev)` (stdio.rs:230-232). A decoder cannot
  write to stdin, cannot ask a question, cannot see the child, cannot see the session.
- It is dropped when the pump thread exits (moved-in local, stdio.rs:209).
- Contract note: "agent 도메인 타입(OutputEvent)만 생성한다 — Serialize 무관(ADR-0003: agent 는 wire
  를 모른다)." (mod.rs:31).
- `PtyTransport` never receives one: `select_transport` drops it on the `Pty` arm with the comment
  "decoder 는 여기서 버려진다" (manager.rs:93-96).

## A3. `StdioTransport` — `crates/engram-dashboard-agent/src/transport/stdio.rs`

### Fields (stdio.rs:36-56) — every one

- `child: Arc<Mutex<Child>>` (:39) — **shared**, not exclusive. Touched by the pump thread
  (`try_wait`, :261-268) and by `shutdown` (`kill`+`wait`, :344-347). Doc: "std Child는 wait 후 exit
  status를 캐시하므로 shutdown이 먼저 reap해도 pump의 try_wait가 같은 status를 회수한다(이중 wait
  무해)." (:37-38)
- `stdin: Mutex<Option<ChildStdin>>` (:40) — **transport-private and exclusive**. No `Arc`, so no
  reference can leave the transport. Only two touchers: `send_input` (blocking `lock()`, :295) and
  `shutdown` (`try_lock`, :358).
- `stdout: Mutex<Option<ChildStdout>>` (:42) — exclusive until `start` `take()`s it and moves it
  into the pump thread (:147-150, read at :217). "None이면 이미 시작됨" is the idempotence guard.
- `stderr: Mutex<Option<ChildStderr>>` (:44) — exclusive until `start` `take()`s it into a
  **second** thread, the stderr drain (:167-171).
- `shutdown: Arc<AtomicBool>` (:46) — **shared**. Set `Release` by `shutdown()` (:340); read
  `Relaxed` mid-loop by the pump (:224) and `Acquire` twice after the loop (:251, :271). This one
  bit is the ONLY thing that distinguishes "our own kill" from "peer went away" anywhere in the
  transport.
- `structured: bool` (:50) — immutable, injected at the assembly point (`select_transport` calls
  `StdioTransport::open(spec, true, decoder)`, manager.rs:89) and reported verbatim by
  `capabilities()` (:373). Doc: "'구조화냐'는 파이프가 아니라 claude `--output-format`(backend/mode
  지식)이 정하므로, select_transport 가 mode 로부터 주입한다(하드코딩 금지 — 평문 stdio 엔 false)."
  (:47-49)
- `decoder: Mutex<Option<Box<dyn OutputDecoder>>>` (:53) — exclusive until `take()`n into the pump
  thread (:204-207).
- `job_handle: JobObjectHandle` `#[cfg(windows)]` (:55) — **transport-private, no `Arc`**.
  `shutdown()` calls `terminate(1)` (:353). Released only on transport drop.

### Construction / destruction

- Only constructor: `StdioTransport::open(spec, structured, decoder)` (stdio.rs:65-124). Spawns the
  child (`Command::spawn`, :92-94) with all three pipes (`Stdio::piped()` x3, :79-81) plus
  `CREATE_NO_WINDOW` on Windows (:85-90), then creates a fresh Job Object and assigns the child pid
  (:102-109). Returns `(StdioTransport, Option<u32> child_pid)`. **The pump is not started here** —
  "**pump는 아직 안 띄운다**(start에서)" (:59).
- Sole production caller: `select_transport` (manager.rs:87-90), itself called only by
  `spawn_session` (manager.rs:1240-1241).
- Dropped when the last `Arc<AgentSession>` drops, because the session holds it as
  `Box<dyn AgentTransport>` (session.rs:56). In the terminal path that is `drop(removed)` at
  reaper.rs:76, whose doc explicitly refuses to claim it is the last reference (reaper.rs:65-73 —
  the early-activation observer may hold the same Arc for up to its next 100 ms poll).

### `send_input` (stdio.rs:292-308)

Body: `self.stdin.lock()` (:295) then `guard.as_mut().ok_or(PtyError::WriteFailed("stdin closed"))?`
(:296-298) then `write_all(&bytes)?` and `flush()?` (:299-305).
**The `stdin` mutex is held across a blocking `write_all` + `flush`.** That single fact drives the
shutdown ordering invariant below. `InputEvent` has exactly one variant, `Raw(Vec<u8>)`
(types.rs:73-77) — there is one and only one input shape.

### `interrupt` (stdio.rs:316-321) — not implemented

Returns `Err(PtyError::Unsupported("StdioTransport::interrupt (ADR-0044 MVP 미지원 — 파이프 Ctrl-C
없음, 후속 스파이크)"))`.

### `resize` (stdio.rs:310-314) — `Err(PtyError::Unsupported("… 파이프는 터미널 크기 없음"))`

### `shutdown` — ordering invariant, quoted verbatim (stdio.rs:330-337)

> 순서 불변 — stdin close 는 kill 보다 절대 먼저 오면 안 된다(데드락, FIX 1):
> send_input 은 stdin Mutex 를 blocking write_all 내내 쥔다. 자식이 stdin 을 안 읽으면
> (파이프 backpressure) 그 write_all 이 영원히 블록해 락을 놓지 않는다. 이때 kill 전에
> `stdin.lock()` 으로 닫으려 하면 그 락을 영영 못 얻어 kill 에 도달조차 못 하고 → pump 가
> 깨지 못해 → core.join_pump 가 영구 hang 한다(ADR-0001 인과가 멈춤).

Continues (:334-337): "그래서 kill + Job terminate 를 먼저 한다 … 그 뒤에야 try_lock 으로 stdin 을
best-effort 정리한다(blocking lock 절대 금지). 그래프-exit-via-stdin-close 는 필요 없다 — 어차피
여기서 kill 하므로." (last sentence in the original: "graceful-exit-via-stdin-close 는 필요 없다")

Body in absolute order (stdio.rs:338-361):
1. `self.shutdown.store(true, Release)` (:340)
2. `child.lock(); let _ = child.kill(); let _ = child.wait();` (:344-347)
3. `#[cfg(windows)] let _ = self.job_handle.terminate(1);` (:351-354)
4. `if let Ok(mut guard) = self.stdin.try_lock() { let _ = guard.take(); }` — **best-effort only,
   silently skipped when contended** (:358-360)

Causality note (:325-328): "파이프는 자식 트리가 stdout write 핸들을 모두 닫아야 reader가 EOF로 깬다.
그래서 child.kill + Job terminate(손자 claude까지)로 트리를 통째 죽여 write 핸들을 닫는 것이 인과의
핵심이다(cmd.exe만 죽이고 claude가 살아 있으면 write 핸들이 안 닫혀 EOF가 안 온다)."

### The pump loop (stdio.rs:209-284)

- Bare `std::thread::spawn`, **unnamed** (:209). Entire body wrapped in
  `catch_unwind(AssertUnwindSafe(..))` (:212); a panic becomes
  `TerminalReason::Error("pump panicked: {msg}")` via `resolve_pump_reason` (:128-140, applied :278).
- Read: `reader.read(&mut buf)` into a 4096-byte stack buffer (:214, :217).
  `Ok(0) | Err(_) => break` (:218) — **EOF and read error are indistinguishable at this line.**
- Secondary shutdown check after each successful read, `Relaxed` (:224-226).
- Decode: `Some(dec) => for ev in dec.decode(&buf[..n]) { pump_core.emit(ev) }`, else
  `pump_core.emit(OutputEvent::TerminalBytes(buf[..n].to_vec()))` (:228-235).
- Post-loop `decoder.flush()` is **conditional on `!shutdown.load(Acquire)`** (:251-257). Reason
  (:242-247): a killed stream's tail is a truncated fragment, and flushing could synthesize a fake
  event — "flush(=consume_line 1회)가 우연히 그 잘린 조각을 파싱 가능한 JSON 으로 읽어 가짜
  이벤트(예: 부분 result → MessageDone)를 방출할 여지를 원천 차단한다."
  It also states the only discriminator that exists (:248-250): "read 레벨에선 자연 EOF 와 kill 이 둘
  다 Ok(0) 이라 구분이 안 되지만, kill 은 반드시 shutdown.store(Release) 를 거치므로 여기서 Acquire 로
  읽어 확실히 구분된다(아래 reason 산출과 동일 신호원)."
- Reason: `if shutdown.load(Acquire) { Killed } else { Exited { code } }` (:271-275), `code` from
  `child.try_wait()` (:260-269).
- Then `pump_core.finish(reason)` (:280), `let _ = done_tx.send(())` (:283). Back on the spawning
  thread: `core.attach_pump(handle, done_rx)` (:286).

### The stderr drain thread (stdio.rs:167-196)

- Named `"engram-stdio-stderr"` (:170). Reads `BufReader::lines()` in a loop (:172-173).
- Each non-empty line: `mask_secrets` (:180) then `diag_core.push_diagnostic(&masked)` (:183) then
  `tracing::debug!(target: "agent_stderr", …)` (:184). Order is load-bearing (:181-182): "쌓기가
  로그보다 먼저다: 기본 로그 필터는 warn 이라 아래 debug 줄은 평소 버려진다 — 분류를 그 줄에 매달면
  로그 레벨이 기능을 켜고 끈다."
- Three documented reasons for its existence: keep the child from blocking on a full stderr pipe
  (:153-154); stderr must NOT be merged into the output stream or NDJSON parsing breaks (:155-157);
  it is the **only** evidence of an activation failure on a structured session (:160-163).
- `Err(_) => break` on read error (:187) — thread simply ends, nothing is notified.
- Spawn failure is `warn`-logged and otherwise ignored (:193-195): the session then runs with **no
  diagnostic capture at all** and nothing downstream can tell.

### `capabilities` (stdio.rs:363-384)

`input{raw:true, message:false, attachment:false}` · `output{terminal_bytes:false, structured:
self.structured, markdown:false, tool_events:false, usage:false}` · `control{resize:false,
interrupt:false, cancel:false, graceful_shutdown:false}`. Only `structured` is dynamic; the other
eleven booleans are literals.

## A4. `PtyTransport` — contrast only (`transport/pty.rs`)

- Fields (pty.rs:25-36): `master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>` (:28),
  `writer: Mutex<Box<dyn Write + Send>>` (:29), `child: Arc<Mutex<Box<dyn Child + Send + Sync>>>`
  (:30), `shutdown: Arc<AtomicBool>` (:31), `reader: Mutex<Option<Box<dyn Read + Send>>>` (:33),
  `job_handle` (:35).
- **What it owns that stdio does not:** the ConPTY master — resizable and closable, and the closing
  is the EOF lever. Hence a real `resize` (pty.rs:297-309) and a real `interrupt` = write `0x03`
  (pty.rs:311-313). **What stdio owns that pty does not:** a separate stderr pipe. PTY merges stderr
  into the console stream, so the diagnostic buffer is always empty for PTY sessions
  (output_core.rs:59-61).
- **Natural-exit watcher** (pty.rs:178-202), named `"engram-pty-watcher"`: polls `child.try_wait()`
  every 50 ms (:186-192, :198); on exit does `master.lock().take()` (:195) → ConPTY close → reader
  EOF → the normal pump path. Reason (:160-166): "Windows ConPTY 는 master 가 살아있는 한 자식이 스스로
  exit 해도 reader 에 EOF 를 주지 않는다 … pump 의 blocking read 가 영원히 안 깬다 → core.finish
  미호출 → reaper 신호 안 감." It deliberately does **not** touch the shutdown flag (:168-172 —
  "set 하면 pump 가 Killed 로 전이한다"). The handle is detached, never joined (:201-202).
- stdio needs no watcher (stdio.rs:11-14): "파이프는 자식(및 자식 트리)이 write 핸들을 모두 닫으면
  read가 EOF(Ok(0))로 깬다 — 자연 종료든 kill이든 동일하게 pump가 깨므로 별도 watcher가 없다."
- `shutdown` order (pty.rs:316-335): flag → `child.kill()`+`wait()` → Job terminate →
  `master.take()` ("5. master.take() → drop → ClosePseudoConsole → reader EOF — 인과의 핵심", :334).
  Note the **opposite** write discipline: `send_input` locks `writer` (:284), and `shutdown` never
  touches `writer` — dropping the master is what unblocks a stuck write. stdio has no such lever,
  which is exactly why its ordering invariant exists.
- `capabilities` (pty.rs:337-360): `terminal_bytes:true, structured:false`; `resize:true,
  interrupt:true, cancel:false, graceful_shutdown:false`.

## A5. `ApiTransport` — `transport/api.rs:14-73`

Unit struct, no fields. `start` no-op (:29); `send_input`/`resize`/`interrupt` all return
`PtyError::Unsupported` (:31-48); `shutdown` no-op (:50); all twelve caps `false` (:52-73).
"manager 라우팅은 없음" (:4) — **unreachable**: `select_transport` has exactly two arms
(manager.rs:85-97) and neither constructs it. It is a placeholder, not a seam in use.

## A6. `OutputCore` — `crates/engram-dashboard-agent/src/output_core.rs`

### Fields (output_core.rs:42-78) — every one

Immutable after construction: `id: AgentId` (:44), `epoch: u32` (:45), `turn: TurnWiring` (:77 —
"생성 후 불변이라 hot path 에 원자 load 도 락도 없다", :76).

Mutable, each behind its own independent lock (the doc's stated reason, :40-41 — "필드별 독립 Mutex …
emit이 replay/subscribers lock만 짧게 잡는 동안 다른 경로(status 등)와 교착 없이 병행 가능"):

- `seq: AtomicU64` (:48) — issued only inside the `replay` lock (see emit).
- `status: Mutex<AgentStatus>` (:49) — initial `Running` (:135).
- `finalized: AtomicBool` (:50) — the one-shot gate.
- `subscribers: Mutex<Vec<Arc<dyn OutputSink>>>` (:53).
- `replay: Mutex<Ring>` (:56) — the replay ring, byte- and count-capped (`Ring`, :759-830).
- `diagnostics: Mutex<String>` (:62) — bounded by `DIAGNOSTIC_CAP_BYTES = 8 * 1024` (:38). Doc:
  "이 화신의 stderr 텍스트 꼬리(bounded). 출력 링과 분리된 별도 버퍼 … PTY 세션은 stderr 가 콘솔
  스트림에 병합돼 오므로 여기는 항상 비어 있다(정상)" (:59-61).
- `status_sink: Arc<dyn StatusSink>` (:65) — injected, shared with the manager (manager.rs:1248).
- `drain_handle: Mutex<Option<JoinHandle<()>>>` (:68) — **written at :396, never read anywhere.**
  Verified: `rg drain_handle` in the agent crate returns exactly three hits — the declaration (:68),
  the `None` init (:141), and the store in `attach_pump` (:396). **The pump thread's JoinHandle is
  stored and never joined.** The pump is effectively detached; the handle is dropped with the core.
- `drain_done_rx: Mutex<Option<Receiver<()>>>` (:69) — `take()`n by `join_pump` (:383-393).
- `on_terminal: Mutex<Option<OnTerminalHook>>` (:73) — `Box<dyn Fn(TerminalReason) + Send + Sync>`
  (:26). Optional because unit tests do not inject one (:72).

`TurnWiring` (:87-122) holds `table: Arc<TurnObservations>` and `classify: TurnClassifier` (:88-89).
It is a **required** `new` argument on purpose (:83-86): "사후 주입(setter)이면 새 spawn 경로가 그걸
빠뜨려도 컴파일이 통과하고, 그 에이전트는 영구히 idle 로 보고돼 턴 중에 메일이 꽂힌다 — 에러도 로그도
없이." `TurnWiring::detached()` (:116-121) is the harness escape and is `#[doc(hidden)]`;
the doc concedes there is **no gate** enforcing that production never calls it (:103-105): "그 구분을
강제하는 장치는 없다 … 이건 컴파일러가 아니라 규약이 지키는 경계다."

### Who may call what

- `emit` (:203-291) — **producers only.** Two callers exist in production: the pump thread
  (stdio.rs:231/234, pty.rs) and the *input-echo* path on whatever thread called write
  (session.rs:176). This is stated twice as load-bearing (output_core.rs:212-213; turn.rs:14-17).
- `finish` (:299-352) — **pump only, exactly once.** "pump가 루프 탈출 후 1회 호출" (:294);
  "terminal 알림 주체는 pump(=여기) 단독" (:296).
- `seed` (:182-193) — `spawn_session` only, and only **before** the sessions-map insert (:161-167).
- `subscribe`/`subscribe_from`/`unsubscribe`/`snapshot`/`terminal_tail`/`diagnostic_tail`/`status` —
  consumers (manager, daemon).
- `set_on_terminal` (:155-157) — `spawn_session` only, before publication (:152).
- `attach_pump` (:395-398) — `transport.start` only, synchronously inside it (:395).
- `join_pump` (:383-393) — the kill path (`AgentSession::kill`, session.rs:245).
- `push_diagnostic` (:601-620) — the stderr drain thread only (:602).

### `emit` — the exact sequence (output_core.rs:203-291)

1. `estimate_cost_bytes(&event)` (:204).
2. **`replay` lock held**: `seq = self.seq.fetch_add(1, Relaxed)` then `replay.push(StoredOutput{seq,
   event: event.clone(), cost_bytes})` (:220-229). The seq issue is *inside* the lock deliberately
   (:211-219): two concurrent emitters could otherwise push N+1 before N and break the
   `partition_point(|c| c.seq <= s)` assumption in `subscribe_from`.
3. Lock released. Turn observation, **before fanout** (:231-234 rationale): if
   `(self.turn.classify)(&event)` yields a signal (:234) →
   `self.turn.table.observe(self.id, self.epoch, seq, signal)` (:238) → then
   `if self.finalized.load(Acquire) { self.turn.table.forget(self.id, self.epoch) }` (:250-251) →
   then `if signal == TurnSignal::Ended { self.status_sink.turn_ended(self.id, self.epoch) }`
   (:258).
   - The finalize recheck is explicitly load-bearing (:241-254) and the ordering proof rests on the
     **table's mutex**, not on the `Acquire`: "무엇이 순서를 만드나 = 표의 뮤텍스(이 인자를 지우지 말
     것) … 그래서 이 load 의 `Acquire` 가 근거가 아니다 — `Relaxed` 여도 결론은 같고, 순서를 나르는
     것은 뮤텍스다. 표 갱신을 락 밖(lock-free)으로 '최적화' 하면 이 증명이 조용히 무너진다."
   - The doorbell is deliberately asymmetric — **no epoch, no finalize, no seq gate** (:258-261):
     "표는 상태라 죽은/옛 화신이 쓰면 산 화신의 사실이 오염되지만, 도어벨은 일회성 자극이라 잉여는
     빈 큐 no-op 으로 흡수되고 … ('누락 < 잉여')."
4. Payload mapping: `TerminalBytes(v) => OutputPayload::Bytes(v)`, everything else
   `=> OutputPayload::Event(other)` (:266-269).
5. Fanout: clone the `subscribers` vec, **release the lock**, then `sink.send(frame)` on the copy
   (:277-287). Invariant quoted (:198-202): "sink.send() 호출 시 어떤 lock도 보유하지 않는다 …
   replay lock과 subscribers lock을 동시에 보유하지 않는다(각각 짧게). 두 lock 동시 취득은 subscribe
   함수 단독 예외이며 emit은 절대 금지."
6. Dead sinks (send returned `Err`) are retained-out in a second short lock (:288-290).

### `finish` (output_core.rs:299-352) — the finalize swap

1. `if self.finalized.swap(true, AcqRel) { return; }` (:300-302) — **the entire one-shot gate.**
2. `reason.clone()` → `AgentStatus` map (:306-315): `Exited{code}`→`Exited{code}`,
   `Killed`→`Killed`, `Interrupted`→`Killed`, `StreamClosed`→`Exited{code:None}`,
   `Cancelled`→`Killed`, `Error(s)`→`Failed{message:s}`.
3. `*status = new_status.clone()` under the status lock, then **release** (:317-320).
4. `self.turn.table.forget(self.id, self.epoch)` (:327). Doc (:321-326) states why here and not in
   the reaper: "reaper 는 이 finish 가 보낸 ReapMsg 로만 깨어나므로 늘 뒤고, 무엇보다 `finalized`
   플래그와 같은 지점이어야 emit 쪽 지각 삽입과의 경쟁이 순서로 닫힌다."
5. `self.status_sink.status_changed(self.id, new_status, self.epoch)` — outside the status lock
   (:334-336, rule cited at :333).
6. `on_terminal` hook called under its own short lock (:340-350) → this is what mints the `ReapMsg`.

### `join_pump` (output_core.rs:383-393)

`take()`s `drain_done_rx` and does `rx.recv_timeout(timeout)`, **discarding the result**. So a
timeout is indistinguishable from success at the call site, and the receiver is consumed — a second
`join_pump` is an immediate no-op. It does **not** join the thread (see `drain_handle` above).

### `snapshot` (output_core.rs:543-582) — asymmetric, lossy for structured events

`filter_map` keeps only `TerminalBytes` → `OutputChunk{seq, data}`; every structured variant is
`tracing::warn!`-ed and dropped (:558-579). The variant-name match arm (:562-570) is a hand-written
seven-way list — a new `OutputEvent` variant forces an edit here (non-exhaustive `ref other` would
not compile against a new variant since the arms are exhaustive by name). Doc admits the asymmetry
(:551-557): "subscribe_from replay 경로는 payload-generic 이라 정상 전달돼 두 복원 경로가 비대칭."

### `terminal_tail` / `diagnostic_tail` (output_core.rs:584-590, :622-627)

The pair that feeds failure classification. Contract (:585-590 and session.rs:298-302): they fill
**mutually exclusively** — pipe/structured sessions fill diagnostics and leave the ring empty; PTY
sessions do the reverse. The caller must concatenate both, and `early_activation_verdict` does
exactly that (manager.rs:1566-1573).

## A7. `AgentSession` — `crates/engram-dashboard-agent/src/session.rs`

### Fields (session.rs:26-56) — every one

- `pub id: AgentId` (:27), `pub cwd: PathBuf` (:28), `pub epoch: u32` (:29) — public and immutable.
  The immutability of `epoch` is what makes `write_stdin_observed_if_epoch` sound (manager.rs:1673-1677).
- `pub cols: AtomicU16`, `pub rows: AtomicU16` (:30-31).
- `intent: Arc<AtomicU8>` (:33) — `Arc` "because the finalize hook closure captures the same value"
  (:32). This is the `TerminationIntent` cell.
- `backend_caps: BackendCaps` (:35) — injected by `spawn_agent` from `profile.command` (:34-35;
  manager.rs:1033).
- `encoder: InputEncoder` (:38) — a `Copy` enum tag, not an object (backend/mod.rs:382-390).
- `reads_messages: bool` (:47) — mail-recipient eligibility. Held by the **session**, not the
  profile, deliberately (:43-46): "`DeleteProfile` 은 산 세션을 죽이지 않는다. 프로필로 판정하면
  프로필이 지워진 산 셸이 '모름' 이 되어 명단에 되돌아오고, 봉투가 명령으로 실행된다."
- `submit_pacing: Duration` (:53) — hard-set by `new` from `backend::SUBMIT_PACING`, **not** a
  constructor argument, on purpose (:50-52): "호출자가 값을 고를 수 있게 하면 어느 조립 경로 하나가 0 을
  넘기는 순간 이 결함이 조용히 재발한다."
- `sleeper: fn(Duration)` (:56) — test seam; production is `blocking_sleep` (:62-64).
- `core: Arc<OutputCore>` (:55).
- `transport: Box<dyn AgentTransport>` (:56) — **`Box`, not `Arc`.** Answering the question
  directly: the transport is owned solely by the session; no other object can hold a reference to
  it, and it is destroyed exactly when the last `Arc<AgentSession>` drops. There is no way to obtain
  a `&dyn AgentTransport` from outside — no accessor exists (verified: no `fn transport` in the file).

### Construction / destruction

- `AgentSession::new(...)` (session.rs:68-99) takes eleven arguments; the sole production caller is
  `spawn_session` (manager.rs:1294-1306). "**start는 여기서 호출하지 않는다** — manager가 new 이전에
  `transport.start(core.clone())`를 직접 부른다" (:66-67) — note the comment is stale about the
  order: in current code `start_pump` is called *after* `new` and after the map insert
  (manager.rs:1337).
- Removed from the map (and thus dropped, modulo other Arc holders) only by `reaper.rs:53-76`.
  A `#[cfg(feature = "test-harness")]` back door inserts sessions directly (`insert_test_session`,
  manager.rs:1710-1714) and its doc states such sessions are **never reaped** (:1704-1706).

### `write_input_observed` (session.rs:156-183) — the whole input path

```rust
let msg_uuid = uuid::Uuid::new_v4();                          // :162
let encoded = self.encoder.encode(bytes, msg_uuid);           // :163
self.transport.send_input(InputEvent::Raw(encoded))?;         // :164
if let Some(event) = self.encoder.input_echo_event(bytes, msg_uuid) {
    self.core.emit(event);                                    // :176
}
Ok(WriteOutcome { bytes_requested: n, bytes_written: n, msg_uuid, epoch: self.epoch })  // :179-185
```

- **The encoder is reached as a `Copy` enum tag on the session**, and every decision behind it is a
  free-function dispatch into `backend_for_encoder` (backend/mod.rs:296-302). The session does not
  know the wire shape: "session은 태그만 들고 형태를 모른다" (:132).
- `bytes_written` is *assumed* equal to `bytes_requested`; completeness is signalled by `Ok`/`Err`
  only (:151-155, and types.rs:663).
- The synthetic echo is emitted **on the caller's thread** after a successful write (:176) — this is
  the second `emit` caller and therefore the second writer into the turn table.
- Call contract for structured mode, quoted (:136-140): "json 모드에서 `1 write_input 호출 == 완결된
  유저 턴 1개` … 터미널 경로처럼 키 입력 1글자씩 호출하면 글자마다 한 글자짜리 잘못된 턴이 만들어져
  대화가 깨진다."

### `submit_input_observed` (session.rs:203-225)

`write_input_observed` then, if `encoder.submit_sequence()` is `Some`, sleeps `submit_pacing`
(:208) and issues a **second** `send_input` with the submit bytes (:214). If the second write fails
it returns `Err` after a `warn` that names the exact residue ("본문은 썼으나 제출 write 실패 — 수신자
입력창에 미제출 봉투가 남았고 재시도가 그 위에 덧쓴다", :215-221). Why this layer (:195-197):
"transport 를 소유해 `send_input` 을 두 번 낼 수 있는 가장 낮은 층이 여기다."
`SUBMIT_PACING = 500ms` (backend/mod.rs:463), `Raw => Some(b"\r")`, `ClaudeStreamJson => None`
(backend/mod.rs:451-454).

### `kill` (session.rs:244-247)

```rust
pub fn kill(&self, timeout: Duration) {
    self.transport.shutdown();
    self.core.join_pump(timeout);
}
```
Doc: "**이 2동사 순서(shutdown THEN join_pump)가 kill 인과의 핵심.** … 역전 시 hang(아직 살아있는
pump를 기다림)" (:241-243).

### `interrupt` / `resize` (session.rs:236-238, :228-233)

`interrupt` is pure delegation, no local state. `resize` updates the atomics **only on success**
(:229-232) — "실패 시 옛 값 유지" (:228).

### `capabilities` (session.rs:258-260)

`Capabilities::compose(self.transport.capabilities(), self.backend_caps.clone())`.

## A8. `AgentManager` — `crates/engram-dashboard-agent/src/manager.rs`

### Fields (manager.rs:358-419)

- `sessions: Arc<RwLock<HashMap<AgentId, Arc<AgentSession>>>>` (:359) — **shared with the reaper**
  (same `Arc`, cloned at manager.rs:515 into `ReaperDeps.sessions`, reaper.rs:39).
- `status_sink: Arc<dyn StatusSink>` (:360) — shared into every `OutputCore` (:1248) and the reaper.
- `profiles: Arc<ProfileRegistry>` (:362) — "프로필 단일 소유자"; **no public accessor** (there is
  `pub fn presets` at :541 but no `pub fn profiles`).
- `presets: Arc<PresetRegistry>` (:366), `tracker: Arc<SessionTracker>` (:367).
- `shutting_down: Arc<AtomicBool>` (:372) — captured by every finalize hook (:1279).
- `reaper_tx: Sender<ReaperCmd>` (:375), `reaper_handle: Option<JoinHandle<()>>` (:377).
- `control: Arc<dyn ControlChannel>` (:382) — shared with the reaper.
- `spawning: Arc<Mutex<HashSet<AgentId>>>` (:390) — the double-spawn reservation set; declared a
  **separate leaf lock** (":390 — sessions 맵과 별개 leaf lock … 이 Mutex 보유 중 sessions/status 락을
  잡지 않는다").
- `name_allocation: Arc<Mutex<()>>` (:407) — stateless serialization lock. Order rule quoted (:398-400):
  "락 순서 = name_allocation → sessions/profiles 단방향. 이 락을 잡는 곳은 셋
  (`create_agent`·`rename_agent`·`register_for_spawn`)이고 셋 다 잡은 뒤에야 명부를 만진다."
  Cost warning (:401-406): the critical section does a `dunce::canonicalize` syscall per dormant
  agent and a whole-file `agents.json` write; but "메일 배달은 막히지 않는다 — 이 성질을 깨뜨리지 말 것."
- `turns: Arc<TurnObservations>` (:417) — leaf, shared with every core.

### `sessions` lock discipline

Stated at the module head (manager.rs:10-12): "`sessions` RwLock은 조회 전용이다. Arc<AgentSession>을
clone하고 lock을 즉시 해제한 뒤에야 session 내부 lock(core/transport)을 취득한다. sessions lock 보유 중
session 내부 lock 취득은 금지(데드락 방지)." Enforced by funnelling every read through
`get_session` (manager.rs:1833-1842), which clones the `Arc` and drops the guard in one expression.
Write-lock holders: `spawn_session`'s insert (:1322-1325), `insert_test_session` (:1710-1714,
feature-gated), and `reaper.rs:53-64`.

### `select_transport` (manager.rs:79-101) — the single assembly point

Two arms only: `StdioNdjson => StdioTransport::open(spec, true, decoder)` (:87-90) and
`Pty => PtyTransport::open(spec, cols, rows)` (:92-96). The `structured` flag is hard-coded `true`
on the stdio arm — the *only* way a stdio transport can report `structured:false` is a call site
that does not exist in production. Doc (:69-77): "transport 종류 선택뿐 아니라 출력이 구조화(NDJSON)
인지도 여기서 결정해 주입한다 … 이 함수는 어느 backend 도 이름으로 모른다."

### `spawn_agent` (manager.rs:912-1110) — ordered call sequence

1. Pre-check `get_session(profile.id)` → `Ok` means `SpawnOutcome::Moot` (:923-930). Known
   **unclosed race**, documented (:917-922): the read lock is dropped before the write-lock insert,
   so two connections can both pass this and `activate_profile`'s pre-check. "이 window 는 ADR-0082
   이전부터 있던 선재(pre-existing) 레이스이며 이번 변경이 도입하지도 닫지도 않았다."
2. `SpawnReservation::reserve` (:932-940) — second guard, RAII (:423-455).
3. `register_for_spawn(profile)?` (:942) — name allocation + roster capacity.
4. `dunce::canonicalize(&profile.cwd)` best-effort (:945).
5. Session-id minting (:954-963): `needs = backend::needs_session(&profile.command)`; if `needs`,
   `Resume => profiles.ensure_session_id(id)` (:957), `Fresh => profiles.new_session_id(id)` (:958);
   else `None`. Doc (:948-953) states this is the single authority: "spawn_agent 이 이 판정의 단일
   권위점이라 어떤 호출자(Spawn/SpawnProfile/restore/fallback)든 mode 만 맞게 넘기면 sid 충돌이 원천
   봉인된다."
6. **Epoch minting** (:978): `self.profiles.epoch_for_spawn(profile.id)` — `?` aborts the spawn if
   the profile vanished. Doc (:965-977) is emphatic that this must stay in one place: "옛날엔 발급이
   `activate_profile` 의 Resume 갈래에만 있어서 Fresh 재spawn 경로들이 죽은 화신의 표식을 그대로
   재사용했다. 그 재사용은 (AgentId, epoch) 를 키로 쓰는 모든 구조를 무너뜨린다." Mode-agnostic by
   design.
7. Control-channel provision, backend-conditional and fail-closed (:999-1013), then `ProvisionGuard`
   armed (:1014-1020).
8. `spec = backend::build_command_spec(...)` (:1022-1029).
9. Backend-derived facts pulled here, because `spawn_session` does not know the backend (:1033-1044):
   `backend_caps` (:1033), `transport_shape` (:1037), `input_encoder` (:1038), `output_decoder`
   (:1039), `turn_classifier` (:1040), `reads_messages` (:1044).
10. `seed_events` (:1047-1053) — `Resume` + `Some(sid)` → `backend::resume_transcript_events`, else
    empty.
11. `spawn_session(...)` (:1055-1066).
12. `provision_guard.disarm()` (:1068-1070).
13. Session-id watcher attach (:1076-1082): only if `sid.is_some() && child_pid.is_some() && needs`,
    and only if `backend::session_id_source(...)` returns `Some` → `self.tracker.watch(id, source)`.
14. `agent_info(&session)` + `status_sink.agent_list_updated(self.list_agents())` (:1086-1088).

### `spawn_session` (manager.rs:1227-1341) — the ordering that matters

```
select_transport(...)                                        // :1240-1241
OutputCore::new(id, epoch, status_sink, TurnWiring::new(turns, classifier))  // :1245-1250
core.seed(seed_events)               // :1269  — BEFORE publish (ADR-0079)
core.set_on_terminal(hook)           // :1280  — mints ReapMsg, snapshots intent + shutting_down
AgentSession::new(...)               // :1294-1306
self.turns.register(id, epoch)       // :1316  — BEFORE the map insert
sessions.write().insert(id, session) // :1322-1325 — BEFORE start_pump (ADR-0019)
profiles.update_with(|p| p.auto_restore = true)  // :1335 — BEFORE start_pump
session.start_pump()                 // :1337  — LAST
```

Each of those four "before"s has its own quoted invariant:
- seed before publish (:1255-1266): "empty-ring replay" and "seq interleave" windows are closed
  *because* nothing can reach the core before the insert.
- `turns.register` before insert (:1308-1315): "뒤집히면 그 첫 신호가 앞 화신 표식과 안 맞아 버려지고,
  이 화신은 앞 화신의 항목이 거둬질 때까지 미관측(=턴 아님)으로 답한다 — 턴 중 우편 주입이 그 결말이다."
- insert before `start_pump` (:1317-1321): "insert 전에 start 하면 빠른 종료 시 hook send 가 맵에 없는
  id 를 가리켜 reap 가 no-op→세션 좀비화."
- `auto_restore = true` before `start_pump` (:1327-1334): otherwise an instantly-crashing agent's
  reaper downgrade is overwritten by this flip → crash loop.

### `TerminationIntent` — what it is and who can read it

- Type: `#[repr(u8)] enum TerminationIntent { None = 0, UserKill = 1 }` (types.rs:96-101), with
  `from_u8` mapping anything unknown to `None` (:103-113).
- Storage: `Arc<AtomicU8>` on the session (`intent`, session.rs:33), created in `spawn_session`
  (:1275) and shared with the finalize hook closure (:1277).
- Written by exactly one verb: `AgentSession::set_intent` (session.rs:111-113), whose only
  production caller is `kill_agent` (manager.rs:1768). Values other than `UserKill` are never
  stored.
- **Read by exactly one reader**: the finalize hook, which snapshots it into `ReapMsg.intent_at_finish`
  (manager.rs:1284-1288). Doc (:1274-1276): "core.finish 의 finalize 승자 경로에서 1회 호출되며, 그
  순간 intent·shutting_down 을 snapshot 해 ReapMsg 를 송신한다(reap 시점 live read 금지 — 크래시→
  유저kill 오분류 race 방지)."
- **There is no getter.** `rg` over the crate shows `set_intent` and the hook's `intent_hook.load`;
  no `fn intent()` exists on `AgentSession`. So nothing below the manager, and nothing at all outside
  the finalize hook, can read it — including the transport, the decoder, and the pump.
- Design rationale, quoted (types.rs:89-93): "유저 의도 — kill 핸들러가 채운다(ADR-0019). PTY 관측
  사실(TerminalReason)과 **분리**한다: 종료를 관측해 의도를 추론하면 데몬 셧다운 Job-kill 이 유저 kill
  로 오분류되므로, 의도는 '종료를 일으킨 행동 지점'(kill 커맨드 핸들러)에서 명시적으로 태깅한다."
- **And today it changes nothing.** `decide(&msg)` (reaper.rs:105-111) branches only on
  `shutting_down_at_finish`; `intent_at_finish` is carried in `ReapMsg` (types.rs:122) and never
  consulted. ADR-0083 collapsed the disposition table, so the field is a live wire with no consumer.

### Kill paths — every one

1. `kill_agent(agent_id)` (manager.rs:1757-1783) — ordered: `control.revoke(id, epoch)` **first**
   (:1766, rationale :1759-1765 — the 5 s join window would otherwise leave a live token) →
   `session.set_intent(UserKill)` (:1768) → `session.enter_exiting()` (:1770) →
   `session.kill(Duration::from_secs(5))` (:1775) → `tracker.unwatch(agent_id)` (:1778).
   It does **not** remove the session from the map: "**맵 제거·disposition·통지는 하지 않는다** …
   반환 직후에도 세션이 아직 맵에 있을 수 있다" (:1753-1756).
2. `shutdown_all()` (manager.rs:1807-1831) — `shutting_down.store(true)` **first** (:1812, rationale
   :1808-1811) → `tracker.stop()` (:1815) → snapshot ids under a read lock (:1820-1823) → a
   `thread::scope` fan-out of `kill_agent` per id (:1824-1830).
3. `Drop for AgentManager` (manager.rs:1911-1921) — stops the reaper thread.
4. `PtyTransport`'s watcher-initiated natural exit (pty.rs:195) — not a "kill", but it also reaches
   `finish` and thus the same reap path.
5. **Implicit**: dropping the last `Arc<AgentSession>` drops the transport, which drops the Job
   handle. On Windows a Job Object handle drop terminates remaining members — reaper.rs:71 notes
   "`JobObjectHandle::drop` 은 마지막 참조가 사라질 때 그대로 돈다."

There is **no cancel path, no graceful-shutdown path, and no interrupt-then-kill path.**
`ControlCaps.graceful_shutdown` is `false` in all three transports (stdio.rs:381, pty.rs:357,
api.rs:70) and nothing reads it.

## A9. The reaper — `crates/engram-dashboard-agent/src/reaper.rs`

- Single serial supervisor thread named `"engram-reaper"`, created once by `spawn_reaper(deps)`
  (reaper.rs:193) from `AgentManager::new_with_control` (manager.rs:519). Consumes
  `ReaperCmd::{Reap(ReapMsg), Stop}` (reaper.rs:31-34) off an `mpsc` channel; "Stop 없이도 모든
  Sender drop 시 recv 가 Err 로 끝나 루프가 종료된다" (:30).
- `ReaperDeps` shares the manager's own `Arc`s verbatim (:37-43) — "사본 금지" (:36-37).
- **`reap_one` is the only thing that removes a session** (reaper.rs:47-97), in this order:
  1. Write lock: `match sessions.get(&msg.id) { Some(s) if s.epoch == msg.epoch => sessions.remove(&msg.id), _ => return }`
     (:57-64). **Epoch equality, not ordering** (:16-18). Poison-tolerant via `into_inner` (:52-56).
  2. `if removed.is_none() { return }` — idempotence, one winner (:66-69).
  3. `drop(removed)` (:76) — and the doc explicitly refuses to claim this is the last `Arc`
     reference (:65-73): the early-activation observer may hold it for up to 100 ms more.
  4. `self.control.revoke(msg.id, msg.epoch)` (:83) — "여기가 모든 terminal 의 단일 수렴점" (:78).
  5. `if !msg.shutting_down_at_finish { apply_disposition(profiles, id, epoch, decide(&msg)) }`
     (:86-89), **outside** the sessions lock because it does disk IO (:85).
  6. `status_sink.agent_list_updated(list_agents(&sessions, &profiles))` (:92-93).
- `decide` (:105-111): `shutting_down_at_finish => KeepAsIs`, everything else
  `=> KeepDisableAutoRestore`. Table quoted at :100-104. **No delete disposition exists** — ADR-0083
  (types.rs:127-130): "reaper 는 어떤 종료에도 프로필을 자동 삭제하지 않는다."
- `apply_disposition` (:124-142) is **downgrade-only** and takes the epoch guard *inside* the
  profile lock closure (:134-136), never touching the sessions lock (:121-123).
- The reaper knows nothing about: the turn table (verified — no reference in the file), the
  `SessionTracker` (manager.rs:1777: "reaper 는 tracker 를 모른다"), the transport, or the decoder.

## A10. The two activation entrances

Both derive the mode from **`profile.backend_session_id.is_some()`** and nothing else.

1. **Command bus / LLM entrance** — `crates/engram-dashboard-agent/src/commands.rs`.
   `agent.spawn` splits on its arguments (`target` xor `cwd`, commands.rs:485-501).
   - `wake_existing` (commands.rs:510-529): `resolve(host, token)` → `host.agent_snapshot(id)` →
     ```rust
     let mode = if profile.backend_session_id.is_some() { SpawnMode::Resume } else { SpawnMode::Fresh };  // :522-526
     let started = host.activate_profile(&profile, mode);                                                 // :526
     ```
     Doc (:520-521): "모드 유도 규칙은 WS 경로와 같은 것을 쓴다(ADR-0076) … 여기서 다른 규칙을 쓰면
     같은 에이전트가 어느 입구로 깨우느냐에 따라 대화 이력을 잃는다."
   - `create_and_start` (commands.rs:533-...): always `SpawnMode::Fresh` (:547), with the command
     built from `backend_command(AgentBackend::Claude, NEW_AGENT_OUTPUT_FORMAT)` (:544) — the verb
     has no backend or format field ("이 동사에는 형식·백엔드 칸이 없다", :537).
2. **Socket entrance** — `crates/engram-dashboard-daemon/src/connection_core.rs`.
   - `AgentCommand::SpawnProfile { profile_id, resume, request_id }` (:1109-1113):
     ```rust
     let mode = if resume || profile.backend_session_id.is_some() { SpawnMode::Resume } else { SpawnMode::Fresh };  // :1126-1130
     let started = manager.activate_profile(&profile, mode);                                                        // :1131
     ```
     So the wire `resume` flag is an **OR**, not the deciding input. Doc (:1114-1123): "저장된 세션이
     있으면 wire `resume` 플래그(프론트는 false 로 보낸다)와 무관하게 항상 Resume 이다 … 단 '안전하다'
     고 읽지 말 것: 방금 발급한 sid 에는 이어받을 대화 실물이 없어서 claude 는 즉사한다."
   - `AgentCommand::Spawn { profile_id, request_id }` (:795-817): **always `SpawnMode::Fresh`**
     (:809), with no mode derivation at all. Doc (:800-806) explains it routes through
     `activate_profile` rather than `spawn_agent` so both handles move the same lever.
- `activate_profile` itself (manager.rs:1113-1180) has three branches: already-running →
  return the live info and write nothing (:1117-1126); `Fresh` → `spawn_agent(Fresh)` (:1128-1146);
  `Resume` → `resume_no_fallback` (:1148), which spawns and then **blocks** up to
  `EARLY_EXIT_WINDOW = 3s` (manager.rs:53) polling `early_activation_verdict` at 100 ms
  (manager.rs:1580-1584).
- `early_activation_verdict` (manager.rs:1538-1585) is the only place that reads the diagnostic
  buffer for a verdict: it concatenates `terminal_tail(FAILURE_TAIL_BYTES)` and `diagnostic_tail()`
  (:1568-1573) and asks `backend::resume_failure_kind(command, &session.diagnostic_tail())` (:1577)
  each iteration. **Note it passes only the diagnostics to the classifier, while the concatenated
  evidence string is used only for the reason text** — an asymmetry (:1568-1577).

## A11. `ProfileRegistry` — `crates/engram-dashboard-agent/src/profile.rs`

### The profile record (`AgentProfile`, profile.rs:127; derives at :126)

Per-**profile** (durable config): `id: AgentId` (:129), `name: String` (:130),
`display_name: Option<String>` (:137), `parent_id: Option<AgentId>` (:145),
`command: AgentCommand` (:147), `cwd: PathBuf` (:152), `env: Vec<(String,String)>` (:155),
`old_session_ids: Vec<Uuid>` (:168), `created_at: i64` (:225).

Per-**incarnation** (goes stale, and by attribute never reaches disk):
`epoch: u32` (:190-191, `#[serde(skip_deserializing, serialize_with = "serialize_zero_placeholder")]`),
`last_failure: Option<AgentFailureKind>` (:207-208, `#[serde(skip)]`).

Hybrid — durable on disk but mutated by runtime observation:
`backend_session_id: Option<Uuid>` (:165), `auto_restore: bool` (:210, raised at manager.rs:1335,
lowered at reaper.rs:136), `last_active: i64` (:226, sole writer `observe_session_id` at :636).

Reserved with **no production writer at all**: `restart_policy` (:213-214), `restart_count`
(:217-218), `failed_reason` (:222-223), and `last_start_at: Option<i64>` (:229-230) — the last one
has only its `None` initializer (:261) and a wire mirror (connection_core.rs:498); its own doc says
it is not used for reset decisions (:228). Removal of the reserved trio is explicitly forbidden
(:74-81, :212-223).

Invariants quoted:
- `cwd` is never canonicalized (:149-152): "★저장된 값은 raw 다 — 정규화되지 않는다(load-bearing)★:
  이 필드를 canonicalize 하는 코드는 없다 … 정규화는 이 값을 쓰는 쪽이 각자 한다."
- `backend_session_id` is mutable and backend-neutral by name (:157-165): "현재 백엔드 세션 id.
  **가변** … ★이름이 중립인 것은 의도★ — 누가 id 를 뽑는지를 말하지 않는다. claude 는 우리가 정한
  id 를 `--session-id` 로 건네받고, codex 는 자기가 뽑은 id 를 우리에게 알린다." Renamed from
  `claude_session_id` on 2026-09-07 with no back-compat shim (:163-164).
- `last_failure` single-writer rule (:201-203): only `AgentManager::note_activation_result`;
  "호출자를 늘리면 인과가 갈라진다."
- Save-inside-the-lock discipline (:365-372): the profiles lock → store lock order is one-way, so
  `persisted == observed` and no interleaved A/B snapshot can leave memory newest and disk stale.

### `observe_session_id` (profile.rs:629-639)

`pub fn observe_session_id(&self, id: AgentId, new_sid: Uuid) -> bool`. Body: a `mutate_if` closure —
if `p.backend_session_id != Some(new_sid)`, push the old value onto `old_session_ids`, store the new
one, set `last_active = now_millis()`, return `true`; otherwise `false` and **no disk write**
(:630-639).

- **No epoch / incarnation guard.** Any differing sid overwrites unconditionally. Contrast
  `set_last_failure`, which *does* take an `incarnation` parameter (:575-583). A late poll from a
  dead incarnation can still write.
- Sole production caller: the `SessionTracker` `on_change` closure, daemon/src/lib.rs:288 (closure
  built at lib.rs:283-289). **The return value is discarded.**
- Thread: the single OS thread named `"session-tracker"` (session_tracker.rs:153-155), one thread
  for all agents (:91). It is called **after** the watch-list mutex is released (:160-177), so the
  chain is `watched` → release → `profiles` → store lock.
- Cannot fail — returns `bool`, not `Result`. And `true` does not mean persisted: a disk write
  failure is swallowed inside the store with only `tracing::error!` (persistence/mod.rs:102-104).
- Value producer (claude only): `ClaudeSessionIdSource::poll`
  (backend/claude/session_file.rs:158-224), which resolves the pid file directly or by scanning the
  whole directory (:98-123), guards against pid reuse (:204-206), and gives up after
  `MAX_RESOLVE_ATTEMPTS = 15` polls → `Degraded`, never polled again (:36, :184-192;
  session_tracker.rs:166-170). Poll interval 1 s (session_tracker.rs:33).
- The port default is `None` (backend/mod.rs:251-258). Only claude implements it
  (backend/claude/mod.rs:394-401). **Codex never reaches the watch at all** — its
  `needs_session()` is `false` (backend/codex/mod.rs:59-61), which gates manager.rs:1076-1082.

### `mutate_if` (profile.rs:400-408)

`fn mutate_if(&self, f: impl FnOnce(&mut HashMap<AgentId, AgentProfile>) -> bool) -> bool` —
**private**. Takes `self.profiles: Mutex<HashMap<AgentId, AgentProfile>>` (:374) via
`.lock().expect("profiles poisoned")` → **panics on poison** (every registry method does; contrast
the reaper's poison-tolerant `sessions` handling at reaper.rs:53-56).
Guarantee: closure commit + `normalize_hierarchy` + snapshot + `store.save` all inside one critical
section (:401-408). Predicate `false` → skips normalize, snapshot and save, returns `false`
(:403-407).

- **Hazard, unenforced:** the closure receives `&mut` to the map and there is no rollback. A closure
  that mutates and then returns `false` leaves an in-memory change that is never persisted and never
  normalized. Today's only caller does not do that (:630-639) but nothing prevents it.
- A caller **cannot** observe the pre/post value — the return type is a fixed `bool`. The generic
  sibling `mutate<R>` (:389) can carry a value out; that is how `ensure_session_id` (:596),
  `new_session_id` (:614) and `epoch_for_spawn` (:667) return their Uuid/u32. Reading a value
  requires a separate, non-atomic `get()` (:420).
- `PresetRegistry` has only the unconditional `mutate` — no `mutate_if` (preset.rs:80-86).

### Persistence timing

- `ProfileStore::save` is called from **exactly two production lines**: profile.rs:395 (inside
  `mutate`) and profile.rs:406 (inside `mutate_if`). No other production caller repo-wide.
- **Fully synchronous with the mutation, inside the profiles mutex. No debounce, no batching, no
  dirty flag, no shutdown flush.** Every mutating verb rewrites the entire file: `upsert` (:438),
  `upsert_preserving_hierarchy` (:459), `remove` (:479), `reparent` (:514), `update_with` (:542),
  `ensure_session_id` (:596), `new_session_id` (:614), `epoch_for_spawn` (:667),
  `observe_session_id` (:629).
- Deliberate bypass: `set_last_failure` takes the mutex directly and never saves (:574-590),
  justified at :568-571 ("`#[serde(skip)]` 이라 저장해도 파일 내용이 한 바이트도 달라지지 않는다").
  Note the inconsistency: `epoch_for_spawn` triggers a full save on **every spawn** although the
  serialized bytes are unchanged (epoch always writes `0`).
- Path `<data_dir>/agents.json`, temp `agents.json.tmp` (persistence/mod.rs:22-24, :50-54);
  `data_dir` = `default_data_dir()` (daemon/src/lib.rs:252, :62; discovery/src/lib.rs:84).
- Format `serde_json::to_vec_pretty(ProfilesFile { schema_version: 1, profiles })`
  (persistence/mod.rs:32-35, :60-64), `SCHEMA_VERSION = 1` (:21).
- Atomic write: `create_dir_all` → `File::create(tmp)` → `write_all` → `sync_all` → `fs::rename` →
  best-effort parent-dir fsync (:57-81). **Write failure is logged and swallowed** — nothing reaches
  the caller, the UI, or the LLM (:99-106; contract at profile.rs:310-311).
- Load failure matrix (`load` has exactly one caller, `ProfileRegistry::new` at profile.rs:380):
  NotFound → empty (:110-113); other IO error → warn + empty (:114-118); parse error → rename to
  `agents.json.corrupt-<ms>` then empty (:127-131, :86-93);
  **schema_version mismatch → empty, file left in place with NO backup rename** (:121-126), so the
  next mutation's save overwrites the preserved file. That is an unprotected data-loss path, unlike
  the parse-error path.
- Crash loses: any sid change not yet observed. Exposure window = poll interval (1 s) plus pid
  resolution latency (up to 15 polls). `epoch` and `last_failure` never reach disk by design.

### Lifetime relative to a process incarnation — the load-bearing answer

**The registry and every profile entry survive child death, pump end, and sessions-map removal.
Registry lifetime = daemon process lifetime; entry lifetime = disk-file lifetime.**

- Construction: `ProfileRegistry::new(store)` loads from disk once and **never re-reads**
  (profile.rs:379-387). Sole production site: `build_daemon_wiring_with_store`, daemon/src/lib.rs:281.
- `Arc` holders: `AgentManager.profiles` (manager.rs:362, moved in at :488/:505); `ReaperDeps.profiles`
  (reaper.rs:39, cloned at manager.rs:515 into the reaper thread); the `SessionTracker` `on_change`
  closure (daemon/src/lib.rs:283-288).
- **No `impl Drop for ProfileRegistry` anywhere.** The agent crate's Drop impls are only
  manager.rs:440 / :471 / :1911, session_tracker.rs:196, platform/windows.rs:84.
- Session death does not touch entries: `decide()` returns `KeepDisableAutoRestore` for every
  runtime exit (reaper.rs:108-113) and `apply_disposition` only lowers `auto_restore` under an epoch
  equality guard (:129-142). Doc (reaper.rs:104-106): "모든 런타임 종료는 세션만 맵에서 수거하고
  프로필은 시체로 보존한다(backend_session_id 유지 → 재활성화 시 --resume 로 이어받음)."
- **Entry removal has exactly one trigger**: `ProfileRegistry::remove` (:479) ←
  `AgentManager::delete_agent` (manager.rs:690-692) ← the explicit user/LLM delete verb. No reaper
  removal, no crash pruning, no TTL, no shutdown sweep.
- Corollary: the *entry* survives; the incarnation-scoped fields inside it (`epoch`, `last_failure`)
  are the ones that die — by serde attribute, not by any removal path.

### The incarnation marker (`epoch`)

- Minted by `random_incarnation_tag()` (profile.rs:687-690) — the top four bytes of a fresh
  `Uuid::new_v4()`. Committed by `epoch_for_spawn` (:667-676) with a `while next == p.epoch` retry;
  `None` means the profile vanished and the caller must abort.
- Single production mint site: manager.rs:978, mode-agnostic (profile.rs:658-661).
- **Equality only** (:170-174): "★화신(incarnation) 하나를 가리키는 불투명 표식★ — 순서에 뜻이 없다 …
  그래서 비교는 일치/불일치만 쓴다 — 두 값의 대소로 '더 새 것' 을 유도하지 말 것."
- Read/write asymmetry (:176-186): "★읽기를 건너뛰는 것이 의미의 일부다★: 화신은 이 데몬 프로세스보다
  오래 살지 못하므로 그 표식을 디스크에서 되살리지 않는다 … ★그런데 쓰기는 건너뛰지 않는다 — 키는 `0`
  으로 실어 보낸다★: 앞 릴리스의 구조체는 이 필드를 필수로 선언했으므로 키가 없는 파일을 읽으면
  `missing field` 로 파싱이 깨진다." Writer: `serialize_zero_placeholder` always emits literal `0`
  (:681-683).
- **The asymmetry's boundary is disk only (`agents.json`).** The wire mirror is a plain
  `pub epoch: u32` with no attributes (protocol/src/domain.rs:340) and `profile_to_wire` copies the
  live value (connection_core.rs:490). No wire→core `AgentProfile` conversion exists in production,
  so nothing injects an epoch into the registry from outside. Contract test at
  persistence/mod.rs:196-224.
- Honest limit stated in-source (:646-652): the only real guarantee is *adjacency within one
  process*; a cross-process collision remains 2^-32 and is never detected because the value is
  never read back.

### Every disk file the agent crate touches

Writes (ours): `<data_dir>/agents.json` + `.tmp` + `.corrupt-<ms>`, owner `FileProfileStore`
(persistence/mod.rs:22-24, :39, :57-93). `<data_dir>/presets.json` + `.tmp` + `.corrupt-<ms>`, owner
`FilePresetStore`, a deliberate clone of the same strategy (persistence/presets.rs:1-6, :18-20, :36,
:53-86); `Preset {id, cwd, name}` (preset.rs:24-33).

Reads only (claude-owned, unofficial formats):
`<CLAUDE_CONFIG_DIR | ~/.claude>/sessions/<pid>.json` — single read plus whole-directory
`read_dir` scan, partial-parse struct with all fields optional
(backend/claude/session_file.rs:44-56, :63-81, :98-123, :199-208; root resolution
backend/claude/mod.rs:1032-1039).
`<CLAUDE_CONFIG_DIR | ~/.claude>/projects/<slug>/<sid>.jsonl` — tail read capped at 4 MiB
(`TRANSCRIPT_TAIL_BYTES`, backend/claude/mod.rs:1013), path :1042-1047, slug rule (every
non-alphanumeric → `-`) :1029-1035, reader :1125-1160.

Not this crate: agent log files belong to the `base` crate's `logging`; the per-agent mcp-config file
is written and deleted by the daemon and arrives here only as an opaque `ControlEndpoint.config_path`
string (types.rs:322-330). Verified: no other `fs::` read/write in the agent crate outside
`#[cfg(test)]`.

### Resume/restore inputs that exist today

With a live read path:
- `backend_session_id: Option<Uuid>` — the **only** value that resumes a conversation today.
  Writers `ensure_session_id` (:596), `new_session_id` (:614), `observe_session_id` (:629). Readers:
  the resumable gate (manager.rs:1375-1376), manager.rs:652, and the claude backend, which turns it
  into `--resume <sid>` (backend/claude/mod.rs:136, :164, :1229) or `--session-id <sid>` for Fresh
  (:135, :163, :1211).
- `cwd` — read at spawn (canonicalized, manager.rs:944) and to derive the transcript slug
  (backend/claude/mod.rs:1042).
- `command: AgentCommand` — `#[serde(tag="kind")]`, so the enum shape is a **disk contract**
  (profile.rs:42-43, :56-64).
- `auto_restore` — raised at manager.rs:1335, lowered at reaper.rs:136-140, read by `restorable()`
  (:428) → `restore_all` (manager.rs:1347).
- The claude transcript `.jsonl` — not persisted by us; read at Resume only for stream-json claude,
  to seed the replay ring (manager.rs:1047-1053; backend/claude/mod.rs:378-388). Terminal claude
  deliberately skips it.

Written but never read back into a spawn:
- `old_session_ids` (:618, :633) — mirrored to wire (connection_core.rs:489) and to the frontend
  (`src/api/types.ts:147`), and **never fed into any spawn**.
- `last_active` (:636) — wire mirror only (connection_core.rs:497); consulted by no restore decision.
- `epoch` — written as `0`, never read from disk.
- `last_start_at`, `restart_policy`, `restart_count`, `failed_reason` — no writer at all.

**Codex has zero persisted resume state today.** `needs_session()==false`
(backend/codex/mod.rs:56-61) → `sid = None` at manager.rs:953-961 → `backend_session_id` stays
`None` → `restore_one`'s resumable gate is false (manager.rs:1375-1381) → codex always spawns Fresh.
`capabilities().session.resume = false` with the stated reason "호출자가 sid 를 못 정하므로 무손실
복원이 성립하지 않는다" (backend/codex/mod.rs:141-148). `codex resume <id>` is listed as known but
unwired (:14, :87-88), and `build_spec` asserts no session flag is assembled (:90-96, test :245).
The `gemini` backend is an unreachable stub — `pub mod gemini` exists (backend/mod.rs:14) but there
is no `AgentCommand::Gemini` variant and no arm in `backend_for` (:283-289).

## A12. `TurnObservations` — `crates/engram-dashboard-agent/src/turn.rs`

### The stored fact (turn.rs:76-83)

```rust
struct Entry {
    epoch: u32,
    in_turn: bool,
    last_signal: Instant,
    last_seq: u64,   // 갱신한 출력 seq (같은 epoch 안에서만 비교 가능)
}
```
Private. The public read projection **drops `epoch` and `last_seq`** — consumers never see the seq
axis (turn.rs:56-64):
```rust
/// 부재(`Option::None`)는 **미관측**이지 idle 이 아니다 — 둘을 어떻게 취급할지는 소비자가 정한다
pub struct TurnObservation { pub in_turn: bool, pub last_signal: Instant }
```

One field: `entries: Mutex<HashMap<AgentId, Entry>>` (turn.rs:73). Key = `AgentId` = `Uuid`.
Epoch is deliberately a **value, not part of the key** (:22-27) — one entry per id; a second map
would create a lock-order rule. `std::sync::Mutex`, poison-tolerant (:103-107). Declared a **leaf**
lock: never held with another, never held across an outbound call (:17-20).
`Arc` owner is `AgentManager.turns` (manager.rs:417, constructed :511, handed out by `pub fn turns()`
:537-539). Every `OutputCore` holds a clone via `TurnWiring` (output_core.rs:87-90, wired
manager.rs:1249). The only non-test holder outside the agent crate is
`ManagerTurnFacts { turns: Arc<TurnObservations> }` (daemon/src/messaging_host.rs:242-251).

### Every write site

Two mutating verbs: `register_at` (unconditional insert, :124-134; wrapper `register` :119-121) and
`observe_at` (guarded insert, :178-204; wrapper `observe` :138-140).

- `AgentManager::spawn_session` → `self.turns.register(id, epoch)` (manager.rs:1316). Thread = the
  spawn caller's thread (control/MCP/Tauri command thread). **Sole `register` caller in the tree.**
- `OutputCore::emit` → `self.turn.table.observe(self.id, self.epoch, seq, signal)`
  (output_core.rs:238), reached only when the classifier returns a signal (:234).
- **`emit` has two calling threads, and this is load-bearing, not incidental** (turn.rs:13-16):
  - the output pump thread — stdio decode loop `pump_core.emit(ev)` (stdio.rs:231), decoder EOF
    flush (:254), raw fallback (:234); PTY `pump_core.emit(OutputEvent::TerminalBytes(...))`
    (pty.rs:241), which never produces a turn signal;
  - the **input-echo thread**, i.e. whoever injected — `self.core.emit(event)` (session.rs:176),
    reached from `write_stdin_observed` (manager.rs:1638), `submit_stdin_observed` (:1651),
    `write_stdin_observed_if_epoch` (:1695). Concretely: the mail flush lane's blocking thread,
    MCP/HTTP handler threads, Tauri command threads.
- Deliberate non-writer: `OutputCore::seed` never touches the table (output_core.rs:178-193), and
  the synthetic closing `MessageDone` appended to a resumed transcript is replay-buffer-only
  (backend/claude/mod.rs:1089-1090, :1094-1099).

### Every delete/clear site

The project claim "cleanup at exactly two points = `finish` + `emit`'s finalize recheck" is **true
for `forget`, and incomplete as a statement about entry removal.**

- `forget` (turn.rs:242-247) — epoch-guarded removal:
  `if g.get(&id).is_some_and(|e| e.epoch == epoch) { g.remove(&id); }`.
- `forget` caller #1: `OutputCore::finish` (output_core.rs:327), the finalize-winner path, so
  exactly once, on the pump thread.
- `forget` caller #2: inside `emit`, retracting what it just inserted if the core already finalized
  (output_core.rs:250-252), on whichever thread called `emit`. `rg` confirms these are the only two
  production `forget` calls.
- **Third removal path, not a `forget`:** `register_at`'s unconditional `insert` evicts whatever
  entry held that id (turn.rs:125-133), documented as such at :109 ("있던 항목이 무엇이든 무조건
  갈아치운다"), with a regression test at :296-297.
- **Whole-map clear: no mechanism exists.** No `clear`, `retain` or drain; the only `remove` in the
  file is :245. The map dies only when the `AgentManager` (and its `Arc`) drops.
- The reaper does **not** clean up: `reaper.rs` has no reference to the turn layer (removed by
  ADR-0127 결정 5 / 거부한 대안 (d)). Agent deletion, kill, and epoch bump do not remove either —
  only `finish` (kill → shutdown → EOF → pump `finish`) or the next `register`.

### The epoch-mismatch drop rule (turn.rs:178-187)

```rust
pub fn observe_at(&self, id: AgentId, epoch: u32, seq: u64, signal: TurnSignal, at: Instant) {
    let mut g = self.lock();
    if let Some(cur) = g.get(&id) {
        if cur.epoch != epoch { return; }
        if seq < cur.last_seq { return; }
    }
```
- **Equality only, never ordering** (:151-153, :33-37) — the marker is a per-incarnation 32-bit
  random, so `<`/`>` are meaningless. The regression test deliberately inverts the tags (live 5 <
  dying 900) at :311-336.
- **The incoming observation is dropped; the stored entry is untouched** — an early `return` before
  any insert. Not overwritten.
- Second guard on the same path: `seq < cur.last_seq` → drop, valid only within one epoch since a
  new incarnation restarts seq at 0 (:184-186, :158-165; test :421-432).
- On the accepted path, `last_signal = at.max(cur.last_signal)` — the staleness axis never rewinds
  (:191-194; test :398-418).
- **Open door:** with no entry at all the observation is accepted unconditionally (the `if let Some`
  simply does not fire) — deliberate, for harness/detached assemblies (:154-156).
- Failure mode if it drops the wrong one: dropping the **live** incarnation's signal (i.e. `register`
  running late, after that incarnation's first signal) leaves the agent permanently unobserved =
  "not in turn" until the old entry is `forget`-ed → mail injected mid-turn (:111-114;
  manager.rs:1310-1312). Dropping the dying one is the intended case; residual risk is the 2^-32 tag
  collision, whose stated outcome is early injection, explicitly preferred over undelivered mail
  (:173-176).

## A13. The messaging busy gate — `crates/engram-dashboard-messaging/src/busy.rs`

### What it reads (busy.rs:177-186)

```rust
pub fn is_busy(&self, id: PeerId, epoch: u32) -> bool {
    let Some(fact) = self.facts.turn_fact(id, epoch) else { return false; };
    if !fact.in_turn { return false; }
    let ledger = self.stale.lock().expect("busy stale ledger poisoned");
    ledger.get(&(id, epoch)) != Some(&fact.last_signal)
}
```
Inputs: `(PeerId, u32)`, the port's `Option<TurnFact>`, and
`BusyPolicy.stale: Mutex<HashMap<(PeerId,u32), Instant>>` (:124). **It reads no clock** (:115-117).

The trait the service asks through (busy.rs:59-65):
```rust
/// ★계약★: 순수 조회 — 부작용 없음, 블로킹 없음(짧은 락만). messaging 락을 든 채 불려도 안전해야 한다
pub trait BusyGate: Send + Sync { fn is_busy(&self, id: PeerId, epoch: u32) -> bool; }
```
`impl BusyGate for BusyPolicy` (:189-193). The unwired fallback `AlwaysIdleGate` returns `false`
always (:68-74) and is the default in `MessagingService::new` (service.rs:546).

**The port trait** (the kernel has zero workspace deps), busy.rs:83-95:
```rust
/// ★읽기 전용이 계약이다★: 이 포트에 "지워라/표시해라" 를 추가하지 말 것
pub trait TurnFacts: Send + Sync {
    fn turn_fact(&self, id: PeerId, epoch: u32) -> Option<TurnFact>;
    fn in_turn_snapshot(&self) -> Vec<(PeerId, u32, Instant)>;
}
```
`TurnFact { in_turn: bool, last_signal: Instant }` (:76-81) — both values returned together so no
torn read across two queries (:87-88).

Daemon-side adapter: `impl TurnFacts for ManagerTurnFacts`, a field-for-field translation of
`TurnObservation` that holds the `Arc` directly so no `sessions` lock is on the delivery decision
path (messaging_host.rs:254-265, rationale :235-241). Assembled by `busy_gate_for_manager`
(:287-292), called at daemon boot (daemon/src/lib.rs:631-634).

Single query point, which ANDs in a per-agent class flag (service.rs:624-626):
```rust
fn gate_says_busy(&self, target: &LiveAgent) -> bool {
    target.turn_signal && self.busy.is_busy(target.id, target.epoch)
}
```
Sole call site service.rs:1566, inside `drain_queue`'s state-lock critical section (justified by the
`BusyGate` contract, :1463-1466). A busy verdict restores the messages to the queue and sets
`report.gated = true` (:1566-1571).
`LiveAgent.turn_signal = a.capabilities.output.structured` (messaging_host.rs:104) — documented as a
**proxy** for "has a turn-event decoder" (busy.rs:19-24; service.rs:425-435). Positive-knowledge-only:
an unobserved `(id, epoch)` means idle, hence immediate injection (busy.rs:10-12, :178-180).

### The fail-open bound

- `pub const BUSY_MAX_TURN: Duration = Duration::from_secs(30 * 60);` (busy.rs:57, rationale :44-56).
- Compared in exactly one place (busy.rs:145-151), boundary `>=` (tests pin both sides, :320-326):
  `.filter(|(_, _, last)| now.saturating_duration_since(*last) >= BUSY_MAX_TURN)`.
- What fires: (a) the `(id, epoch) → last_signal` pair is written into `BusyPolicy.stale`, and **that
  ledger entry *is* the flipped verdict** — `is_busy` returns false while the ledger value equals the
  current `last_signal` (:156-163, :184-185); (b) one `tracing::warn!` per newly-stale id (:167-171);
  (c) `notifier.notify_idle(id)` outside the ledger lock (:166-173).
  **The fact is not deleted** — asserted by the test
  `the_ceiling_flips_the_verdict_without_touching_the_facts` (:314-335); non-destructiveness is
  ADR-0113 결정 2 / ADR-0127 결정 4, restated at :14-17.
- The ledger is **replaced** each sweep with "what is stale now", so revived/reaped incarnations do
  not accumulate (:156-164); a fresh signal invalidates the verdict without waiting for a sweep
  (:122-123; test :338-347).
- Clock source: `std::time::Instant` (monotonic) end to end — stamped in `register`/`observe`
  (turn.rs:120, :139), and `now` is injected into the sweep from the daemon ticker
  (daemon/src/lib.rs:664). Kernel purity forbids the kernel reading a clock
  (messaging/src/lib.rs:15-24).
- Sweep cadence 60 s, a long-lived tokio task whose tick body is wrapped in `catch_unwind` so a
  panic does not silently stop both TTL expiry and this fail-open (daemon/src/lib.rs:655-682;
  `sweep_busy.sweep_stale_busy(now)` at :674; first tick skipped :660-661).
- **Daemon restart survival: none, and moot.** No persistence of turn facts anywhere; both the fact
  table (manager.rs:511) and the stale ledger (busy.rs:132) are in-memory. After a restart every
  `(id, epoch)` is unobserved → idle → immediate injection. Parked mail is likewise in-memory
  (messaging_host.rs:351-352).

### What the gate can and cannot observe

- **Cannot** distinguish "thinking" from "dead with a leaked fact" — `is_busy` reads only `in_turn`
  and `last_signal`; no pid, no status, no transport (busy.rs:177-186). The only discriminator
  available is age, i.e. the 30-minute ceiling.
- **Cannot** see the transport kind: the port carries `(PeerId, u32)` in and `TurnFact` out
  (:89-95); the adapter adds nothing (messaging_host.rs:254-265).
- **Cannot** see the backend: `TurnSignal` is a single shared vocabulary precisely so consumers never
  learn the backend (turn.rs:47-54; backend/mod.rs:136-137). No `AgentCommand` or backend name
  reaches the messaging crate — compiler-enforced zero workspace deps (messaging/src/lib.rs:5-13).
- **Cannot** see whether the child is alive: no liveness predicate in `busy.rs`;
  `AgentStatus::is_live` lives on the daemon side (messaging_host.rs:95-97).
- Two partial, indirect exceptions, both at the **caller** rather than in the gate: roster membership
  (`gate_says_busy` is only asked about a `LiveAgent` produced by `live_agents()`, which filters
  `is_live(a) && a.reads_messages` — messaging_host.rs:155-165), and the `turn_signal` capability
  proxy (:104).
- **The sweep path has no such filter**: `in_turn_snapshot()` returns every in-turn entry regardless
  of liveness (turn.rs:224-230 → busy.rs:146-151). So a leaked fact (an entry `finish` never reached)
  produces a warn plus a doorbell every 30 min forever, indistinguishable from a real long turn.
- **Cannot** tell "no signal ever came" from "turn ended": both are `in_turn == false`; `register`
  seeds `in_turn: false` and the entry survives `Ended` (turn.rs:117-118; tests :269-278, :437-445).

### Coupling to byte-stream heuristics

Nothing in `turn.rs` or `busy.rs` parses bytes, matches a regex, or times prompts. The coupling is
one layer down, in what the classifier is fed.

- No regex, no timing, no prompt detection on this path: `rg "regex|Regex|Instant|std::time|timeout|thread::sleep"`
  over `backend/claude/mod.rs` → 0 hits. The only time value in the fact layer is `Instant`
  staleness.
- Turn signals exist **only** where an `OutputDecoder` was constructed. Classifier dispatch
  backend/mod.rs:336-338; trait default is silence (:143-145 + `no_turn_signals` :271-273).
  `ShellBackend`, `CodexBackend` and `GeminiBackend` do **not** override it (0 hits for
  `turn_classifier|TurnSignal` in those three files) ⇒ **every codex agent today is permanently
  unobserved ⇒ always idle ⇒ mail injected immediately.**
- The whole mapping is seven lines over decoded events, not bytes (backend/claude/mod.rs:547-555):
  `TextDelta | ToolCall | Structured => Progress`; `MessageDone => Ended`;
  `Usage | Error | TerminalBytes => None`.
- **Where the turn START signal comes from today**, two distinct origins, both `Progress`:
  1. **Input echo, synthesized locally at write time** — the one that catches a mail-injected or
     human-typed turn start. `write_input_observed` → `self.core.emit(event)` (session.rs:175-176);
     the event is built by the claude encoder as `OutputEvent::Structured { kind: "user", .. }`
     (backend/claude/mod.rs:361-366) and classified `Progress` via the `Structured` arm (:551). That
     the `Structured` arm is load-bearing for exactly this reason: :529-532.
  2. **The first decoded assistant/tool line from the child** — NDJSON split on `\n`, strict UTF-8,
     `serde_json`, unknown types skipped: `ClaudeStreamDecoder::decode` (:722-758), `consume_line`
     (:783-799); emitted from the pump at stdio.rs:231.
- Turn END comes solely from the `result` NDJSON line → `MessageDone` (:817, :868-871). Fragilities
  that would vanish with real structured boundaries:
  - `turn_id` and `message_id` are hardcoded `None` (:869-870, doc :539-542) — no correlation key,
    so no turn counting and no fencing; "last observation wins".
  - `Structured` is classified without looking at `kind`; if claude ever emits a structured line
    **outside** a turn, that agent goes false-busy until the 30-minute ceiling (:533-535) — exactly
    the failure ADR-0127 거부한 대안 (b) names.
  - Decoder overflow resync discards a whole line to the next `\n` and emits `OutputEvent::Error`
    (:749-756). `Error` maps to `None` (:553), so a dropped `result` line silently loses the only
    `Ended` signal ⇒ stuck turn ⇒ 30-minute fail-open.
  - Kill-path flush suppression: on shutdown the decoder tail is deliberately not flushed, to avoid
    a truncated fragment parsing as a fake `MessageDone` (stdio.rs:238-257).
  - PTY mode produces only `TerminalBytes` → `None`; the same classifier is reused with no mode
    branch (:543-544; pty.rs:241).
  - `Entry.last_seq` exists **only** because `emit` has two concurrent callers, one of which is the
    byte pump (turn.rs:158-163). Structured boundaries on a single ordered channel would make it
    dead weight.
  - `UNVERIFIED` (documented as such in-source): whether a nested Task subagent's `result` line
    leaks as a **parent** turn end (busy.rs:31-33; backend/claude/mod.rs:540-542).

## A14. Frontend consumption (`src/`)

### The frame tag — the only discriminant that reaches the frontend

Binary frame header `[tag:1][agentId:16][epoch:4 BE][seq:8 BE][payload]` — `src/api/wsFrame.ts:3`.
Constants (a frontend copy of `codec.rs`): `FRAME_TAG_TERMINAL_BYTES = 0` (:8),
`FRAME_TAG_STRUCTURED_EVENT = 1` (:9), `FRAME_TAG_REPLAY_MARKER = 255` (:15).

**Any tag other than 0/1 is dropped at decode with no log and no counter** — `src/api/wsFrame.ts:35`:
```ts
if (tag !== FRAME_TAG_TERMINAL_BYTES && tag !== FRAME_TAG_STRUCTURED_EVENT) return null
```
Callers do `if (!f) return` — `src/api/tauriTransport.ts:339`, `src/api/wsTransport.ts:294`. Both
carriers funnel through the same decoder, so this line is the single chokepoint.

The public frontend frame type carries three fields and nothing else —
`src/api/agentClient.ts:22-32`: `{ seq: number; tag: number; bytes: Uint8Array }`.
Transport-level twin at `src/api/transport.ts:20`. `ProtocolClient` never branches on the tag; it
passes it through (`src/api/protocolClient.ts:270`, :392, contract stated :248).

The tag-1 payload type is the generated binding, imported **directly from the Rust crate directory**
with no re-export in `src/api/` — `src/components/slot/structuredAccumulator.ts:18` imports
`../../../crates/engram-dashboard-protocol/bindings/StructuredEvent`. Definition at
`crates/engram-dashboard-protocol/bindings/StructuredEvent.ts:24`: a six-arm union tagged on
`"type"` — `TextDelta | ToolCall | Usage | MessageDone | Error | Structured{kind,json}`.

`OutputChunk.ts` (the wire enum with `TerminalBytes`/`TextDelta`/…,
`crates/engram-dashboard-protocol/bindings/OutputChunk.ts:10`) is **not consumed anywhere in
`src/`** — it is snapshot-only per its own doc (:7-8).
`AgentBackendKind = "claude" | "codex"` exists in the frontend (`src/api/types.ts:112`) but is
**spawn-only**; its own doc forbids render branching (:110).

Every place the frontend branches on `tag` (exhaustive, non-test): `src/api/wsFrame.ts:35` ·
`src/components/slot/TerminalSlot.tsx:261` · `src/components/slot/RichSlot.tsx:161` ·
`src/components/slot/DomSlot.tsx:132`.

### The three slot gates — all bare early returns

- **TerminalSlot** (`src/components/slot/TerminalSlot.tsx`): callback order is cancelled guard (:252)
  → seq dedup (:253-254) → **tag gate** (:261) → `terminal.write` (:262). Drop mechanism is a bare
  early return, no log, no counter: `if (chunk.tag !== FRAME_TAG_TERMINAL_BYTES) return`. Accepts
  tag0 only. **The seq high-water is advanced *before* the gate** (:254), deliberately (:259-260).
- **RichSlot** / `LiveRichSlot` (`src/components/slot/RichSlot.tsx:61`): cancelled (:154) → seq dedup
  (:155-156) → **tag gate** (:161) → `acc.feed(chunk.bytes)` (:163). Accepts tag1 only; drops tag0
  explicitly because a structured agent may still emit tag0 during transition (:157-160).
- **DomSlot** (`src/components/slot/DomSlot.tsx:132`): accepts **tag0 only** — it is a plaintext
  `<pre>` observer of the *terminal* stream, not a chat view (:128-131).

Regression coverage for the gates exists: `src/components/slot/slotTagGate.test.tsx:1-9`.

### The render-mode selector — per-slot, front-end only

Three modes: `RENDER_MODES = ['terminal', 'rich', 'dom']` (`src/components/slot/renderMode.ts:5`).
Derivation is a single boolean, recomputed at render time and never stored (:23-25):
```ts
export function defaultRenderMode(agent: AgentInfo): RenderMode {
  return agent.capabilities.output.structured ? 'rich' : 'terminal'
}
```
That flag is `OutputCaps.structured` (`src/api/types.ts:30`; wire mirror
`crates/engram-dashboard-protocol/bindings/OutputCaps.ts:11`) — i.e. the value stdio hard-codes to
`true` at manager.rs:89.

Resolution: `renderModeOverride[slotId] ?? defaultRenderMode(agent)`
(`src/components/layout/ViewLayoutRenderer.tsx:78-79`); mount switch :209-220 with a `default:` arm
that falls through to `TerminalSlot` (:217-219). An invalid override therefore renders as a terminal
(flagged at `src/components/slot/renderMode.ts:12-13`).

Storage: `renderModeOverride: Record<string /*slot node.id*/, RenderMode>` in the front-end Zustand
view store (`src/store/viewStore.ts:78`, initial `{}` :161). This field explicitly **does not** go
through the backend authority loop (:72-77, restated `src/commands/renderModeCommands.ts:4-5`), has
**no disk persistence, and is per-webview-window** (`src/commands/renderModeCommands.ts:9-10`).
Writers: `setRenderMode` (:209-218, invalid mode → `console.warn` + ignore :213-215),
`clearRenderMode` (:219-224), dom aliases (:228-233); auto-cleared on slot re-content (:178, :185,
:192, :199).

**An LLM/command path exists and is release-reachable**, but with two holes:
five commands registered — `slot.renderMode.set` (`src/commands/renderModeCommands.ts:71`),
`slot.renderMode.clear` (:84), `slot.domMode.enable` (:95), `slot.domMode.disable` (:104),
`slot.domMode.toggle` (:113); args `{slotId, mode}` with no `viewId` (:31-35); invalid `mode` throws
rather than no-op (:60-67). Reachable via `window.__engramCmd.run(...)`, installed
**unconditionally** — not behind `import.meta.env.DEV` (`src/store/eventBus.ts:86-89`, intentional
per :83-85).
- **No human UI path and no keybinding** — they have no `registerSlotMenu` contribution (contributions
  exist only in `agentCommands.ts:169`, `presetCommands.ts:84`, `slotContentCommands.ts:94/110/134`,
  `slotCommands.ts:137`), self-documented at `src/commands/renderModeCommands.ts:39`; no hits in
  `src/commands/keybindings.ts`.
- **They are deliberately excluded from the cross-window command bus** because they declare no `help`
  (the `offeredCommands` gate in `src/commands/viewCommandBridge.ts`), per
  `src/commands/renderModeCommands.ts:8-18`. So a backend-side caller cannot reach them — only
  in-window `__engramCmd`/CDP can.
- The slot memoizes the last mounted mode for the dead-agent case and carries **only the mode**
  (`src/components/layout/ViewLayoutRenderer.tsx:61`, effect :88-96, consumption
  `renderAs = mode ?? kept?.mode` :209). Known in-source limitation: during agent absence there is
  no mode derivation, so `setRenderMode`/`clearRenderMode` are **silent no-ops that report success**
  (:60).

### The structured accumulator — an unrecognized `type` is silently swallowed

Object: `StructuredEventAccumulator` (`src/components/slot/structuredAccumulator.ts:31`). Its render
model is an order-preserving item stream, not a message list; the item union is
`text | tool | usage | error | structured | separator` (:21-29).

`feed()` (:51-63): empty payload → return (:53); malformed JSON → `console.warn` + return (:57-60);
otherwise `this.consume(ev)` (:62).

`consume()` opens at :66 and closes at :138, and **there is no `default:` arm**. Cases: `TextDelta`
(:67), `ToolCall` (:80), `Usage` (:90), `Error` (:98), `Structured` (:103), `MessageDone` (:129).

**Exact behaviour for an unrecognized `ev.type`, traced to the pixel:** every arm is a `case` on a
literal string; nothing matches; control leaves the switch at :138 and the method returns.
Concretely **none** of these happen: no item is pushed (`this.items` untouched); `this.nextId` is
**not** incremented (it is bumped only inside cases — :75, :86, :95, :100, :126, :134);
`this.turnDone` unchanged; `this.seenUserUuids` unchanged. **No throw, no warn, no counter, no row,
no placeholder.** TypeScript makes it a compile-time-unreachable branch, so there is no runtime
marker of any kind.

**What the caller does anyway** (`src/components/slot/RichSlot.tsx:155-167`):
- `lastSeq.current = chunk.seq` (:156) — the seq high-water advances before `feed`, regardless.
- `setItems([...acc.snapshot()])` (:165) — called unconditionally, producing a **new array reference
  with identical contents**. That re-renders `StructuredTextView` and, because the auto-scroll
  effect's deps are `[items]`, **fires a scroll-to-bottom** (:221-224).
- `setTurnDone(acc.isTurnDone())` (:166) — reads back an unchanged flag.
- `setAwaiting(false)` (:167) — **the "waiting for first token" indicator is cleared by an event that
  rendered nothing.** This exact class of frame is called out at :77-80.

**The one escape hatch that does exist** is the *modelled* `Structured{kind,json}` variant, not an
unknown-`type` handler (`src/components/slot/structuredAccumulator.ts:125-126`):
```ts
// 탈출구 이벤트 — 알 수 없는 종류(kind)도 흘려 유실 방지.
this.items.push({ kind: 'structured', label: ev.kind, json: ev.json, itemId: this.nextId++ })
```
An unknown `kind` renders as a collapsed `GenericItemRow` (label + pretty-printed JSON): dispatch
`src/components/slot/StructuredTextView.tsx:397-401`, component :279-304, JSON body via
`InertCode`/`pretty` :299, :41-47, :195-201. `rowKindOf` classifies unknown labels as `'assistant'`
(:334). **So the door is on `Structured.kind`, and only there.**

Downstream render switches also lack default arms and would go quiet if a new `StructuredItem` kind
were added: `renderItem` (:346-427, returns `undefined` → React renders nothing) and `rowKindOf`
(:320-336, returns `undefined`, which `computeRailRunPositions` treats as neither skip nor assistant
→ `null` position, `src/components/slot/chat/railPositions.ts:14`, :21). Hypothetical-future, not
reachable today. **No test exercises an unknown top-level `type`** —
`src/components/slot/structuredAccumulator.test.ts` has no such case.

### Where a raw/native label or raw JSON can reach the screen today

Release-reachable, no env gate, no dev flag, no hidden route:
- **`GenericItemRow`** — label string + pretty-printed JSON blob, inside the chat slot. Label is
  `ev.kind` straight off the wire; body is `JSON.stringify(JSON.parse(json), null, 2)` with a
  raw-string fallback (`src/components/slot/StructuredTextView.tsx:279-304`, :299, :41-47). Reached
  for any `Structured` item whose label is not `user`/`thinking` and whose json is not a
  `tool_result` (:362, :364, :385, :397). **This is the only existing surface that already renders
  an un-modelled backend string next to its payload.**
- **Tool IN/OUT panes** — `pretty(argsJson)` and raw `result.content` in literal `<pre><code>`,
  markdown deliberately never applied (:257, :269; mechanism + rationale :189-201).
- **`Error` item message**, raw backend string (`structuredAccumulator.ts:100` →
  `StructuredTextView.tsx:416-422`).
- **`DomSlot` `<pre>`** — the whole terminal byte stream, ANSI-stripped, appended as plaintext, 200 KB
  tail cap (`src/components/slot/DomSlot.tsx:36`, :44-48, :135-143, :198). Gate = render mode `'dom'`
  only, i.e. `__engramCmd` in a release build; no menu item and no key.
- **`ConnectionNotice`** — the raw daemon connection-failure reason, verbatim, in a `role="alert"`
  banner (`src/components/layout/ConnectionNotice.tsx:22`, :48; source
  `src/api/protocolClient.ts:190`, :194). Built precisely because release WebView2 has no devtools
  (:3-5).
- **Agent tree row tooltip** — `String(e)` of a rejected action promise interpolated into an i18n
  template and shown as the row's `title` (`src/components/agent/AgentList.tsx:182`, :215, :227,
  :242, :262, :296 → `rowTitle` :364 → `title={rowTitle}` :419). The row body shows only a fixed
  badge (:526-530).
- **`WindowLayout` load failure** with the raw window label
  (`src/components/layout/WindowLayout.tsx:158`, template `src/i18n/ko.ts:50`).

Not reachable / gated:
- `window.__engram` store handles (theme/agent/chatStyle) — behind `if (import.meta.env.DEV)`
  (`src/main.tsx:33-39`, stated :26-28).
- `window.__engramCmd` — **not** gated; installed in `initEventBus` for all builds
  (`src/store/eventBus.ts:86-89`), intentional per :83-85.
- `src/lab/terminal/TerminalView.tsx` + `fixtures.ts` — **dead code**. Nothing imports them
  (`TerminalView.tsx:17`, :24 are the only hits) and routes are only `/`, `/tree`, `/popup`
  (`src/App.tsx:63-68`). No lab HTML entry. Unreachable in any build.
- `SlotUnavailableVeil` — deliberately shows **no text at all**, only a `PowerOff` glyph; the old
  `Failed: <message>` text was removed by user decision (:7-10, :28-41).
- `SubscribeFailed.reason` and unattributed `AgentEvent::Error.message` — `console.warn` only, never
  rendered (`src/api/protocolClient.ts:635`, :644).
- `chatStyleStore` writes — no release path at all (`src/store/chatStyleStore.ts:5-12`).

### Seq / dedup / replay boundary

- **Two dedup layers, both by seq high-water, both tag-blind.** (1) `ProtocolClient` per view: live
  arm `if (f.seq <= st.lastDeliveredSeq) continue` (`src/api/protocolClient.ts:268-270`), flush arm
  (:390-392); tag-blindness is a stated contract (:248-250). (2) Per-slot local `lastSeq` ref applied
  **before** the tag gate in all three slots (`TerminalSlot.tsx:245`, :253-254; `RichSlot.tsx:147`,
  :155-156; `DomSlot.tsx:112`, :126-127).
- **Epoch guard**, equality only: `if (st.epoch !== undefined && f.epoch !== st.epoch) continue`
  (:266). Frames never adopt an epoch; only a success marker does (:273-277).
- **`terminal.reset()` — exactly two call sites**, both in TerminalSlot: on (re)subscribe (:243) and
  inside the `onReset` clear callback (:277). Peers do the equivalent: `setItems([])` + `acc.reset()`
  (`RichSlot.tsx:131-132`, :187-188), `setText('')` (`DomSlot.tsx:107`, :155). Every `onReset` also
  rolls the local seq guard back to `-1` (`TerminalSlot.tsx:278`, `RichSlot.tsx:198`,
  `DomSlot.tsx:156`).
- **The replay→live boundary marker is a synthetic `tag=255` frame** on the same output channel,
  produced by `src-tauri` and never by the daemon codec (`src/api/wsFrame.ts:12-19`; decoder :55-72,
  payload `[tag255][agentId:16][epoch:4][gen:8 BE][flags:1]`, bit0 truncated / bit1 failed :16-19,
  :64-71). Normalized to a `replayBoundary` control event by the Tauri carrier
  (`src/api/tauriTransport.ts:326-337`, checked **before** `decodeOutputFrame` :325). The WS carrier
  has no marker frame and synthesizes the boundary from `SubscribeAck`→`ReplayComplete`
  (`src/api/wsTransport.ts:286-290`).
- Boundary evaluation happens at marker arrival, not at registration
  (`src/api/protocolClient.ts:328-369`): buffering-only (:333); held-marker path when own `gen` is
  unknown (:339-347); **gen fence** `if (m.gen < st.myGen) return` (:352); `failed` marker → keep
  the buffer and push the re-request ladder (:353-363); epoch fence for success markers (:367);
  then `flushToLive` (:368).
- `flushToLive` is the actual transition (:371-399): adopts the epoch (:374); fires the consumer
  clear signal `onReset()` plus `lastDeliveredSeq = -1` **immediately before** delivery, same tick
  (:377-381); filters foreign-epoch frames and sorts by seq (:386); delivers with dedup (:389-393);
  `truncated` → `console.warn` (:394); `phase='live'` (:395); `onState('live')` (:398).
- Incarnation rotation is declared by the **authoritative roster alone**, not by frames or sockets
  (:590-614), equality-only comparison (:600-607). Subscription effect deps are `[viewId, agentId]`
  with the epoch deliberately excluded (`TerminalSlot.tsx:320`, rationale :315-319;
  `RichSlot.tsx:216`; `DomSlot.tsx:175`).
- Buffer overflow discards the whole buffer and invalidates the gen fence before re-requesting
  (:287-306).

---

# PART B — the questions the design actually needs answered

## B1. Can any code *above* the transport write bytes to the child's stdin, or close it?

**Write: yes, but only through one funnel, and never as raw bytes of the caller's choosing.**
**Close: no mechanism exists.**

- The single funnel: `AgentSession::write_input_observed` → `self.encoder.encode(bytes, msg_uuid)` →
  `self.transport.send_input(InputEvent::Raw(encoded))` (session.rs:162-164). Every caller above is
  a thin wrapper: `write_input` (session.rs:142-144), `submit_input_observed` (:203-225),
  `AgentManager::{write_stdin, write_stdin_observed, submit_stdin_observed,
  write_stdin_observed_if_epoch}` (manager.rs:1629, :1633, :1646, :1680).
- Verified by exhaustive search: the only non-test `send_input` call sites in the whole workspace are
  session.rs:164, session.rs:214, and pty.rs:312 (`interrupt` writing `0x03` to itself).
  **No code path reaches `send_input` without going through `AgentSession`.**
- The bytes are **always** passed through the backend encoder first (session.rs:163), so a caller
  cannot choose the framing: `Raw` copies verbatim, `ClaudeStreamJson` wraps into one JSON line
  (backend/mod.rs:409-417). A caller who wants a *different* framing has no lever.
- **Closing stdin: no mechanism exists.** `AgentTransport` has no `close_stdin` (transport/mod.rs:41-57,
  read in full). The only place `stdin` is dropped is inside `StdioTransport::shutdown` step 4
  (stdio.rs:358-360), *after* the child has already been killed, and it is `try_lock` best-effort —
  it may be skipped. To close stdin without killing the child you would need: **a new trait method**
  (contract change, all three impls), plus a lock-order decision — because the existing invariant
  (stdio.rs:330-337) says a blocking `stdin.lock()` before a kill deadlocks against a stuck
  `write_all`. A `close_stdin` verb that blocks on that mutex reintroduces exactly the hang that
  invariant was written to prevent. Invasiveness: **contract change + a documented lock-order hazard.**

## B2. Can any code above the transport learn "this shutdown is our own deliberate kill"?

**Below/inside the transport: yes, via one shared bit. Above the transport: no — nothing can read it.**

- The bit is `StdioTransport.shutdown: Arc<AtomicBool>` (stdio.rs:46), set `Release` by
  `shutdown()` (:340) and read `Acquire` by the pump (:251, :271). It is the *only* discriminator,
  and its necessity is documented (stdio.rs:248-250): "read 레벨에선 자연 EOF 와 kill 이 둘 다 Ok(0)
  이라 구분이 안 되지만, kill 은 반드시 shutdown.store(Release) 를 거치므로 여기서 Acquire 로 읽어
  확실히 구분된다."
- **Owner:** the transport, privately. There is no accessor — no `is_shutting_down()` on
  `AgentTransport` (transport/mod.rs:41-57).
- What escapes upward is only the *classification*: `TerminalReason::Killed` vs
  `Exited { code }` (stdio.rs:271-275) → `finish` → `AgentStatus::Killed` vs `Exited` (output_core.rs:306-315).
  Two problems for a design that needs the distinction:
  1. It arrives **only at terminal time**, on the pump thread, after the process is already gone.
     Nothing can consult it *during* a shutdown.
  2. It is **lossy**: `Killed` and `Interrupted` and `Cancelled` all map to `AgentStatus::Killed`
     (output_core.rs:308-312), so a consumer of the status cannot tell them apart.
- The *user-intent* axis exists separately as `TerminationIntent` (types.rs:96-101) on the session
  (`intent: Arc<AtomicU8>`, session.rs:33). **It has no getter** and exactly one reader, the finalize
  hook that snapshots it into `ReapMsg.intent_at_finish` (manager.rs:1284-1288). Even that value is
  then **never consulted** — `decide()` branches only on `shutting_down_at_finish`
  (reaper.rs:105-111). So today the intent bit is written by `kill_agent` (manager.rs:1768),
  snapshotted once, and discarded.
- Also note the third axis: the daemon-wide `shutting_down: Arc<AtomicBool>` (manager.rs:372), also
  read only by the finalize hook (:1279, :1288). So there are **three separate "why did this die"
  bits**, none readable by a layer that could act on it while the child is still alive.
- To give a layer above the transport this knowledge: **new field or new trait method**, plus the
  question of *who* is told (the transport does not know about the decoder, and the decoder cannot be
  called from the shutdown thread because the pump owns it `&mut` — transport/mod.rs:26-29).
  Invasiveness: **contract change; and the decoder's single-owner rule blocks the obvious route of
  telling the decoder.**

## B3. Is there any object in the session path that survives a process incarnation?

**Yes — exactly two, and only one of them can hold anything.** Full candidate enumeration with
lifetimes:

| candidate | lifetime | survives incarnation? | can hold recovery state? |
|---|---|---|---|
| `ProfileRegistry` + each `AgentProfile` entry | daemon process / disk file. No `Drop` impl anywhere; entry removal has exactly one trigger, the explicit delete verb (profile.rs:479 ← manager.rs:690-692) | **yes** | **yes — this is the only durable slot.** But `epoch` and `last_failure` are `#[serde(skip*)]` (profile.rs:190, :207) and the only functional resume input is `backend_session_id: Option<Uuid>` (:165) |
| `TurnObservations` (the fact table) | daemon process (`Arc` on the manager, manager.rs:417/:511). Entry per id is evicted by the next `register` (turn.rs:125-133) or removed by `forget` | **table yes, entry no** | no — no serde, no persistence, and its port is read-only by contract (busy.rs:86) |
| `SessionTracker` | daemon process (`Arc`, manager.rs:367). One polling thread | **yes** | no — `WatchEntry` holds a `Box<dyn SessionIdSource>` whose learned `resolved_pid` dies with the incarnation (session_file.rs:136). And it leaks: the only `unwatch` is in `kill_agent` (manager.rs:1778), and "reaper 는 tracker 를 모른다" (:1777) — a naturally-exited agent leaves a live polling entry for the daemon's lifetime |
| `BusyPolicy.stale` ledger | daemon process (busy.rs:132) | **yes** | no — replaced wholesale each sweep (busy.rs:156-164) |
| `AgentSession` | one incarnation. Dropped at reaper.rs:76 | no | — |
| `OutputCore` (ring, seq, subscribers, diagnostics) | one incarnation, dropped with the session | no | no — `output_core.rs` has no serde derive at all |
| `StdioTransport` / `PtyTransport` (+ stdin, Job handle, decoder) | one incarnation, `Box`ed inside the session | no | — |
| `OutputDecoder` | one incarnation, moved into the pump thread (stdio.rs:204-209) | no | no — and by design: "epoch 교체 = 새 transport = 새 decoder 라 리셋이 자동이다" (transport/mod.rs:28-29) |
| `ControlChannel` token + mcp-config file | `(AgentId, epoch)` — revoked by `kill_agent` (manager.rs:1766) and again by the reaper (reaper.rs:83) | no | no — deliberately scoped to one incarnation |
| The `intent` `Arc<AtomicU8>` | one incarnation (created manager.rs:1275) | no | — |

**Consequence for the design:** the only place a new backend can leave a resume handle across
incarnations is a field on `AgentProfile`, and today the only shaped slot is `Option<Uuid>`
(profile.rs:165). A non-UUID handle (a string conversation id, a rollout path) has **no slot**.

## B4. Which threads exist in a live session, what does each block on?

Per live session:

| thread | created | blocks on | can it block without stopping reads from the child? |
|---|---|---|---|
| output pump (unnamed) | stdio.rs:209 / pty.rs (inside `start`) | `reader.read(&mut buf)` (stdio.rs:217) | **no — it *is* the reader.** Anything that blocks here stops all output |
| stderr drain, `"engram-stdio-stderr"` | stdio.rs:169-171 (stdio only) | `BufReader::lines()` (stdio.rs:173) | **yes** — independent of stdout. But if it stops, the child eventually blocks on a full stderr pipe (stdio.rs:153-154), which stops the child, which stops output |
| PTY watcher, `"engram-pty-watcher"` | pty.rs:181-200 (pty only), detached | 50 ms `sleep` loop + a short `child.try_wait()` under the child mutex (pty.rs:186-192) | **yes** |
| the caller's own thread doing a write | whoever called `write_stdin*` | `write_all`/`flush` on stdin **while holding the `stdin` mutex** (stdio.rs:295-304) | **yes — this is the one that matters.** Reads continue; but it can block forever on pipe back-pressure, and while it does, `shutdown` cannot take that mutex (which is why kill comes first — stdio.rs:330-337) |

Process-wide, shared by all sessions:

| thread | created | blocks on |
|---|---|---|
| reaper, `"engram-reaper"` | reaper.rs (`spawn_reaper`, called manager.rs:519) | `rx.recv()` — **serial for every session in the daemon** |
| session tracker, `"session-tracker"` | session_tracker.rs:153-155 | 1 s sleep; calls `SessionIdSource::poll` **while holding the watch-list mutex** (session_tracker.rs:44-46) |
| busy sweep (tokio task) | daemon/src/lib.rs:655-682 | 60 s tick |

**Answer to the specific question:** the writer thread and the stderr drain can block without
stopping reads. The pump cannot, by construction. Note also that **the pump's `JoinHandle` is stored
and never joined** — `drain_handle` is written at output_core.rs:396 and read nowhere (verified by
`rg`); `join_pump` waits on the separate `done_rx` channel and **discards the timeout result**
(output_core.rs:383-393). So a wedged pump is invisible: `kill` returns after 5 s either way.

## B5. Is there anywhere in the session path that returns a value to a caller?

**In the session path (us → child): no. Nothing anywhere returns a reply.**

- `AgentTransport`: every fallible method returns `Result<(), PtyError>`; only `capabilities()`
  returns data, and it is a static snapshot (transport/mod.rs:41-57).
- `OutputDecoder`: returns `Vec<OutputEvent>` and has no channel, no sink, no handle — it cannot
  correlate, cannot ask, cannot reply (transport/mod.rs:32-39).
- `OutputSink::send(frame) -> Result<(), SinkError>` (types.rs:706) — one-way.
- `StatusSink` — all four methods return `()` (types.rs:711-733).
- The closest existing thing is `WriteOutcome` (types.rs:679-696), returned by
  `write_input_observed`. But it is a **local receipt, not a reply**: it reports
  `bytes_requested`/`bytes_written` (both set to `bytes.len()` unconditionally, session.rs:180-184),
  the locally-minted `msg_uuid`, and the session's `epoch`. Nothing in it came from the child.
- Request/response *does* exist in the system, but on two other axes, neither of which is the session
  path: (a) client ↔ daemon, via `request_id` + `reply(sink, request_id, result)`
  (connection_core.rs:792, :1143); (b) agent → daemon, via the MCP control channel
  (`ControlChannel::provision` hands the child a URL + bearer token, types.rs:316-330) — that is the
  child calling *us*.

**Smallest place a request→response could live without changing the transport contract:**
`AgentSession`. It already owns the transport (the only object that does — session.rs:56, and the
transport is a `Box`, so no one else can hold it), it already mints a per-write correlation id
(`msg_uuid`, session.rs:162) which the backend already threads into the outbound bytes *and* the
echo event, and it already sits on the write thread that could wait. What it does **not** have is
any sight of the reply: inbound events go pump → `core.emit` → sinks (stdio.rs:231) and never pass
through the session. So a session-level request/response needs a **new inbound path** from the pump
to a session-owned waiter map. Two sub-problems, both real:
  1. The decoder is owned `&mut` by the pump thread and cannot be shared (transport/mod.rs:26-29), so
     the correlation has to happen either inside the decoder (which cannot send) or in the pump's
     `emit` loop (which is `OutputCore`, and `OutputCore` does not know the session).
  2. Any waiter that blocks must not block the pump — and the pump is the only reader (B4).
Invasiveness: **new field + a new inbound notification edge; no transport contract change required
if the notification rides on the existing `OutputEvent` stream.**

## B6. Who can set or override the five capability domains?

**Five domains, two sources, one sanctioned constructor, and no per-spawn injection point.**

- `Capabilities { input, output, control, session, model }` (types.rs:451-456).
  `Capabilities::compose(t: TransportCaps, b: BackendCaps)` is documented as "Capabilities 의 **유일한
  정상 생성 경로** — 출처가 섞이지 않게 타입으로 박았다" (types.rs:476-487). The split is enforced by
  the types: `TransportCaps` has no `session`/`model` fields (:461-464, comment :460),
  `BackendCaps` has no `input`/`output`/`control` (:470-473, comment :467-468).
- Sole production call site: `AgentSession::capabilities` (session.rs:259).
- **Distinct points that produce a value today — five:**
  1. `StdioTransport::capabilities` (stdio.rs:363-384) — 11 literals + `self.structured`.
  2. `PtyTransport::capabilities` (pty.rs:337-360) — 12 literals.
  3. `ApiTransport::capabilities` (api.rs:52-73) — 12 literals; unreachable (B/A5).
  4. `backend::backend_caps(c)` → `AgentBackend::capabilities(&self, command)`
     (backend/mod.rs:328-330; trait :104), per backend. It takes the `command` because "같은
     프로그램(claude)이라도 모드에 따라 caps 가 다르다" (backend/mod.rs:98-103).
  5. The `structured` injection in `select_transport` (manager.rs:89) — this is the only value
     computed at assembly time rather than declared by an impl.
- **Can a value be injected per spawn? Only one, and only indirectly.** `structured` is passed as a
  positional `bool` to `StdioTransport::open`, and `select_transport` hard-codes it to `true` on the
  `StdioNdjson` arm (manager.rs:89) — the shape *is* the flag. Everything else is a compile-time
  literal or a pure function of `profile.command`. **There is no per-spawn capability override, no
  profile field for capabilities, and no runtime setter** — `rg` finds no `set_capabilities` anywhere.
  To vary a capability per spawn you must either add an `AgentCommand` payload field that the
  backend's `capabilities(command)` reads (the existing, sanctioned route), or add a parameter to
  `select_transport` and the transport constructor (the `structured` precedent).
- Who *consumes* the domains, so you know what an override would move:
  `output.structured` drives the frontend renderer choice (`src/components/slot/renderMode.ts:24`)
  **and** the messaging busy gate's `turn_signal` flag (messaging_host.rs:104 → service.rs:625).
  `session.resume` gates restore (manager.rs:1375-1376 reads `backend_session_id`, and
  backend/codex/mod.rs:141-148 declares `resume:false`). `control.resize`/`interrupt` are declared
  and **not read anywhere in the delivery path** (UNVERIFIED for the frontend beyond
  `src/api/types.ts`). `control.cancel` and `control.graceful_shutdown` are `false` everywhere and
  read by nothing.

## B7. What happens today to a structured event whose kind the frontend does not recognize?

Two very different answers depending on *where* the unknown-ness sits. Traced to the pixel:

**(a) Unknown `Structured.kind` (the modelled escape hatch) → it renders.**
`OutputEvent::Structured { kind, json }` (types.rs:66-68, "위 정형 variant로 안 잡히는 backend별
구조화 이벤트의 탈출구(forward-compat)") → `emit` → `OutputPayload::Event` (output_core.rs:268) →
daemon codec → `tag=1` frame → `RichSlot` tag gate passes (`RichSlot.tsx:161`) → `acc.feed` →
`consume`'s `Structured` case pushes `{ kind: 'structured', label: ev.kind, json: ev.json, itemId }`
(`structuredAccumulator.ts:125-126`, comment "알 수 없는 종류(kind)도 흘려 유실 방지") →
`renderItem` dispatch (`StructuredTextView.tsx:397-401`) → `GenericItemRow` (:279-304) renders the
**label plus pretty-printed JSON** in a collapsed row (:299 via `pretty`, :41-47). `rowKindOf`
classifies the unknown label as `'assistant'` (:334). **This works today and is the one door that
exists.**

**(b) Unknown top-level `StructuredEvent.type` → it vanishes, and it vanishes twice over.**
- On the Rust side, `OutputEvent` is a closed enum (types.rs:37-69). A new event shape cannot exist
  without either an enum variant (which forces edits at output_core.rs:562-570 and
  backend/claude/mod.rs:547-555) or being smuggled through `Structured`.
- On the frontend, `consume()`'s switch (`structuredAccumulator.ts:66-138`) has **no `default:` arm**.
  An unmatched `type` pushes no item, does not bump `nextId` (:75, :86, :95, :100, :126, :134 are the
  only increments), does not set `turnDone`, throws nothing, logs nothing, counts nothing.
- And the caller still acts as if something arrived (`RichSlot.tsx:155-167`): the seq high-water
  advances (:156), `setItems([...acc.snapshot()])` produces a new array reference with identical
  contents (:165) which re-renders and **fires the auto-scroll** (:221-224), and `setAwaiting(false)`
  (:167) **clears the "waiting for first token" indicator** — so the UI reports progress for an
  event that rendered nothing.
- Pixel-level result: **the screen is byte-identical to before the event, except the spinner is
  gone.** And there is no console output, in a release WebView2 with no devtools (the very reason
  `ConnectionNotice` exists, `ConnectionNotice.tsx:3-5`).

**(c) A tag the frontend does not recognize (e.g. a hypothetical `tag=2`) → it never arrives.**
`src/api/wsFrame.ts:35` returns `null` for anything but 0 and 1 (255 is handled separately), and both
carriers do `if (!f) return` (`tauriTransport.ts:339`, `wsTransport.ts:294`). No log, no counter.
`ProtocolClient` never sees it.

## B8. Concurrent stdin writes, and a child that stops reading

**Concurrent writes: serialized, correctly, by the `stdin` mutex. Not interleaved, not lost.**
`send_input` takes `self.stdin.lock()` and holds it across `write_all` + `flush`
(stdio.rs:295-304), so N concurrent callers become N sequential complete writes. This is asserted by
an integration test whose header calls it "물리 파이프 계층의 **응용계층(application-layer) 직렬화**"
(`crates/engram-dashboard-agent/tests/stdio_physical_pipe.rs:91`) and by a daemon test asserting
"캡처된 write 수 == N(각 send_input 이 완결 봉투 1개 — 잘림/합병 없음)"
(`crates/engram-dashboard-daemon/tests/control_send.rs:1491`).
PTY does the same with its `writer` mutex (pty.rs:284-292).

**Child stops reading: the writer blocks forever, holding the mutex, and everything that needs that
mutex queues behind it.** Consequences, all traced:
- Every other writer for that agent blocks in `stdin.lock()` (stdio.rs:295). Since the mail delivery
  lane writes on a blocking thread (messaging_host.rs:803-804), that lane stalls for that agent.
- `shutdown()` **must not** take the mutex before killing — that is the stdio.rs:330-337 invariant,
  and the regression test `shutdown_completes_even_if_send_input_blocks_on_full_pipe`
  (stdio.rs:455-507) pins it with a 10 s deadline. Because kill comes first, the pipe breaks, the
  blocked `write_all` returns `Err`, and the lock is released — then step 4's `try_lock` may or may
  not succeed (stdio.rs:358), and either way the OS reclaims the handle on drop (:356-357).
- **Reads are unaffected** — the pump is a separate thread on a separate handle (B4).
- **There is no timeout, no size cap, and no back-pressure signal.** A caller cannot ask "is stdin
  writable", cannot bound the wait, and cannot cancel. The only escape is killing the child.
- `write_all` semantics are load-bearing for the caller's accounting: a partial write is never
  reported as `Ok` (session.rs:151-155, types.rs:663) — completeness is `Ok` vs `Err`, not a byte
  comparison.

## B9. Ordering between "visible in the roster" and "pump starts"

**Insert strictly precedes `start_pump`, and the invariant that names it is ADR-0019 (the reaper
ordering invariant).**

Code order in `spawn_session`: `sessions.write().insert(id, session.clone())` (manager.rs:1322-1325)
then `session.start_pump()` (:1337). Quoted invariant (manager.rs:1317-1321):
"★ADR-0019 — sessions 등록은 pump 기동(start)보다 **먼저**★: finish hook 이 ReapMsg 를 보내는데,
pump 가 즉시 EOF→finish 하면 그 시점에 세션이 맵에 있어야 reaper 가 reap 한다. insert 전에 start 하면
빠른 종료 시 hook send 가 맵에 없는 id 를 가리켜 reap 가 no-op→세션 좀비화." Restated at
session.rs:117-120 and in the project CLAUDE.md's invariant list ("등록 순서") — and the failure mode
is **silent at runtime**; only the reaper test catches a regression.

Three more orderings ride on the same line and are each separately named:
- `core.seed(...)` **before** the insert — ADR-0079 "seed-before-publish" (manager.rs:1255-1266;
  output_core.rs:161-167). Rationale: before the insert nothing can reach the core, because both the
  subscribe and emit paths go through the sessions map.
- `turns.register(id, epoch)` **before** the insert — ADR-0113 (manager.rs:1308-1315).
- `profiles.update_with(|p| p.auto_restore = true)` **before** `start_pump` — otherwise the reaper's
  downgrade for an instantly-crashing child gets overwritten (manager.rs:1327-1334).
- `core.set_on_terminal(hook)` before all of it (manager.rs:1280).

Note the honest caveat: `attach_pump` happens *synchronously inside* `transport.start`
(stdio.rs:286), so the insert order does not affect `join_pump` (manager.rs:1320-1321).

## B10. When the child dies mid-turn — every mutation and drop, in order

Assume a stdio/structured session, natural death (no `shutdown()` call), with a turn in progress
(so the table holds `in_turn: true` for that `(id, epoch)`).

1. Child exits → child tree closes all stdout write handles → **pump thread**
   `reader.read()` returns `Ok(0)` → `break` (stdio.rs:217-218).
2. `shutdown.load(Acquire)` is `false` → `decoder.flush()` runs; any completed tail events go to
   `pump_core.emit(ev)` (stdio.rs:251-256). If one of them is a `MessageDone`, step 4's turn cleanup
   is preceded by a normal `Ended` observation; if the tail was truncated, it is not, and the entry
   stays `in_turn: true` until step 5c.
3. `child.try_wait()` under the shared child mutex → `code` (stdio.rs:260-269);
   `reason = TerminalReason::Exited { code }` (stdio.rs:274). *(Panic variant: `catch_unwind` at
   stdio.rs:212 → `resolve_pump_reason` → `TerminalReason::Error("pump panicked: …")`, :128-140.)*
4. `pump_core.finish(reason)` (stdio.rs:280) → inside `OutputCore::finish`:
   - a. `finalized.swap(true, AcqRel)` → `false`, this thread is the winner (output_core.rs:300).
   - b. `*status = Exited { code }` under the status lock, lock released (output_core.rs:317-320).
   - c. `turn.table.forget(id, epoch)` — **the turn fact is removed here**, epoch-guarded
     (output_core.rs:327 → turn.rs:242-247). This is what unblocks a parked mail consumer.
   - d. `status_sink.status_changed(id, Exited{code}, epoch)` (output_core.rs:334-336) → the daemon
     fans this out to clients.
   - e. `on_terminal` hook (output_core.rs:340-350) → builds `ReapMsg { id, epoch, reason,
     intent_at_finish: from_u8(intent.load(SeqCst)), shutting_down_at_finish: shutting_down.load(SeqCst) }`
     and `reaper_tx.send(ReaperCmd::Reap(msg))` (manager.rs:1281-1303). Send failure is ignored.
5. `done_tx.send(())` (stdio.rs:283) → unblocks any `join_pump` waiter. **Pump thread exits.** Its
   `JoinHandle` remains parked in `core.drain_handle`, never joined (output_core.rs:396).
6. The **stderr drain thread** hits EOF on `lines()` and returns (stdio.rs:173-189) — no
   notification, and whatever it accumulated stays in `core.diagnostics` until the core drops.
7. **Reaper thread** dequeues and runs `reap_one` (reaper.rs:47-97):
   - a. sessions write lock: `s.epoch == msg.epoch` → `sessions.remove(&msg.id)` (reaper.rs:57-64).
     Lock released.
   - b. `drop(removed)` (reaper.rs:76) — decrements the `Arc<AgentSession>`. **May or may not be the
     last reference**: `early_activation_verdict` can hold the same `Arc` for up to its next 100 ms
     poll (reaper.rs:65-73, manager.rs:1584).
   - c. `control.revoke(msg.id, msg.epoch)` (reaper.rs:83) — deletes the bearer token and the
     mcp-config/settings files for that `(id, epoch)`.
   - d. `if !shutting_down_at_finish`: `decide(&msg)` → `KeepDisableAutoRestore` (reaper.rs:108-113)
     → `apply_disposition` → `profiles.update_with(|p| if p.epoch == reaped_epoch { p.auto_restore =
     false })` (reaper.rs:129-142) → **and that `update_with` writes the entire `agents.json` to
     disk synchronously inside the profiles mutex** (profile.rs:542 → :395).
   - e. `status_sink.agent_list_updated(list_agents(...))` (reaper.rs:92-93).
8. When the last `Arc<AgentSession>` actually drops: `AgentSession` drops →
   `Box<dyn AgentTransport>` drops → `StdioTransport` drops → the remaining `Option<ChildStdin>`
   (if step 4 of shutdown never ran, i.e. this whole path, so **it is still open**), the
   `Arc<Mutex<Child>>`, and `JobObjectHandle` drop. The `Arc<OutputCore>` drops with it, taking the
   ring, the diagnostics buffer, the subscriber list, and the unjoined pump `JoinHandle`.

**What is NOT touched by any of this — verified:**
- `profile.backend_session_id` and `old_session_ids` survive untouched ("시체 보존", reaper.rs:104-106).
- `profile.epoch` stays at the dead incarnation's value until the next `epoch_for_spawn`
  (manager.rs:978).
- `profile.last_failure` is not written by this path — only `note_activation_result` writes it
  (profile.rs:201-203), and a mid-turn death outside the 3 s activation window reaches no writer.
- **The `SessionTracker` watch entry is not removed.** `unwatch` is only called from `kill_agent`
  (manager.rs:1778); the reaper does not know about the tracker (:1777). A naturally-dead agent
  leaves a polling entry alive for the daemon's lifetime.
- The turn table's *map* is untouched; only that one entry was removed (step 4c).
- No frontend "the agent died mid-turn" signal exists beyond `status_changed` + `agent_list_updated`;
  the chat slot's `turnDone` flag simply never flips (`structuredAccumulator.ts:129-137`).

---

# PART C — the "no door" list

Every place a plausible design would need a door that does not exist. One line each: what a layer
would want to do, and why it cannot today.

## Transport / process control

1. **Close the child's stdin without killing it** (the normal way to tell a JSON-RPC peer "no more
   requests") — no method on `AgentTransport` (transport/mod.rs:41-57); the only `stdin.take()` is
   step 4 of `shutdown`, *after* the kill, and it is `try_lock` best-effort (stdio.rs:358-360).
   Adding a blocking one reintroduces the deadlock the ordering invariant exists to prevent
   (stdio.rs:330-337).
2. **Shut down gracefully — send a shutdown request and wait for the peer to exit on its own** — no
   verb exists; `ControlCaps.graceful_shutdown` is `false` in all three transports (stdio.rs:381,
   pty.rs:357, api.rs:70) and **nothing reads it**. `shutdown()` is defined as
   "자원 강제 종료(멱등)" (transport/mod.rs:53) and unconditionally kills the process tree.
3. **Cancel an in-flight operation** — `interrupt()` on stdio returns `Unsupported`
   (stdio.rs:316-321) and `ControlCaps.cancel` is `false` everywhere and read by nothing.
4. **Learn, above the transport, that a shutdown is our own deliberate kill** — the discriminating
   bit is transport-private (`shutdown: Arc<AtomicBool>`, stdio.rs:46) with no accessor; what escapes
   is only the post-mortem `TerminalReason`, and it is lossy (`Killed`/`Interrupted`/`Cancelled` all
   collapse to `AgentStatus::Killed`, output_core.rs:308-312).
5. **Read the user's kill intent from anywhere but the finalize hook** — `TerminationIntent` has a
   setter (session.rs:111-113) and no getter; its single reader snapshots it into `ReapMsg`
   (manager.rs:1284-1288), where `decide()` then ignores it entirely (reaper.rs:105-111).
6. **Ask "is this child still alive?" through the session or transport** — no `is_alive`, no
   `child_pid` accessor after spawn; `child_pid` is returned once by `open` (stdio.rs:69) and only
   the tracker keeps it. `AgentStatus::is_live` (types.rs:22) reads the *core's* status, which is
   only written at terminal time.
7. **Ask the transport what kind it is, or what backend it serves** — no `kind()`, no `name()`; the
   only self-description is the twelve `capabilities()` booleans (transport/mod.rs:56).
8. **Bound, cancel, or poll a stdin write** — `send_input` is an unbounded blocking `write_all` under
   a mutex (stdio.rs:295-304); no timeout, no `try_write`, no writability query, no back-pressure
   signal. The only escape from a non-reading child is killing it.
9. **Write bytes in a framing of the caller's choosing** — every write is forced through
   `encoder.encode()` (session.rs:163), and `InputEvent` has exactly one variant, `Raw(Vec<u8>)`
   (types.rs:73-77). A caller cannot bypass the encoder or add a second frame kind without a
   contract change.
10. **Detect a natural child exit on a pipe transport without relying on EOF** — stdio has no
    watcher by design (stdio.rs:11-14); if the child leaks a stdout write handle to a grandchild that
    outlives it, nothing wakes the pump and nothing times out (`join_pump` discards its timeout
    result, output_core.rs:383-393).
11. **Notice that a pump thread is wedged** — `drain_handle` is written (output_core.rs:396) and read
    nowhere; the pump is never joined, and `kill` returns after 5 s whether or not the pump ever
    finished.
12. **Recover from a failed stderr drain** — if the drain thread fails to spawn it is `warn`-logged
    and ignored (stdio.rs:193-195), and the session then runs with **no diagnostic capture at all**
    with nothing downstream able to tell.
13. **Give a decoder more than one output shape** — `OutputDecoder` returns `Vec<OutputEvent>` and
    holds no sink, no channel, and no handle (transport/mod.rs:32-39); it cannot write to stdin,
    cannot signal an error out of band, cannot ask a question, and cannot see the session, the core,
    or the child.
14. **Reach the decoder from anywhere but the pump** — it is `take()`n and moved into the pump thread
    (stdio.rs:204-209) and requires only `Send`, not `Sync`, by explicit design
    (transport/mod.rs:26-29). Nothing can hand it a message, reconfigure it, or query it.
15. **Distinguish EOF from a read error** — `Ok(0) | Err(_) => break` (stdio.rs:218); the pump cannot
    tell "peer closed cleanly" from "the pipe broke", and both become `Exited { code }`.

## Request / response

16. **Issue a request and await a reply** — nothing in the session path returns a value from the
    child: `AgentTransport` returns `Result<(), PtyError>` (transport/mod.rs:45-54), `OutputSink::send`
    is one-way (types.rs:706), `StatusSink` returns `()` (types.rs:711-733). `WriteOutcome`
    (types.rs:679-696) is a **local receipt** whose byte counts are set to `bytes.len()`
    unconditionally (session.rs:180-184) — nothing in it came from the child.
17. **Correlate an inbound event with the write that caused it** — the correlation id exists
    (`msg_uuid`, session.rs:162, threaded into both the outbound line and the local echo) but the
    inbound path never returns to the session: pump → `core.emit` → sinks (stdio.rs:231). The session
    never sees a single inbound byte.
18. **Carry a correlation id on a structured event** — `turn_id`/`message_id` exist on
    `OutputEvent` (types.rs:41-62) but claude hardcodes both to `None`
    (backend/claude/mod.rs:869-870) and the frontend accumulator reads **neither** in any arm
    (`structuredAccumulator.ts:66-138`).
19. **Time out or cancel a pending request** — there is no pending-request table anywhere in the
    session path, hence nothing to time out.

## Capabilities

20. **Override a capability per spawn** — the only assembly-time injected value is `structured`, and
    `select_transport` hard-codes it to `true` on the `StdioNdjson` arm (manager.rs:89). Every other
    one of the twelve transport booleans is a compile-time literal (stdio.rs:363-384, pty.rs:337-360)
    and there is **no setter anywhere** (`rg set_capabilities` → 0).
21. **Declare a capability the split does not have a home for** (e.g. "supports request/response",
    "supports mid-turn cancel", "resume handle is opaque") — the five domains are closed structs
    (types.rs:451-527) and the source split is enforced by the types: transport cannot express
    `session`/`model`, backend cannot express `input`/`output`/`control` (:460, :467-468).
22. **Tell the frontend which renderer to use** — the concept does not exist on the wire
    (`src/components/slot/renderMode.ts:1`); the only backend input is the single boolean
    `output.structured` (:24), which reaches only two of the three modes and cannot name `'dom'`.
23. **Register a fourth renderer** — `RENDER_MODES` is a frozen 3-tuple (:5) and the mount site is a
    hardcoded switch whose `default:` silently means terminal
    (`src/components/layout/ViewLayoutRenderer.tsx:210-220`). A backend needing another presentation
    gets a terminal.
24. **Flip a slot's renderer from the backend or the command bus** — the five override commands are
    deliberately excluded from the cross-window bus because they declare no `help`
    (`src/commands/renderModeCommands.ts:8-18`), so only in-window `__engramCmd`/CDP can reach them;
    they also have no menu item and no keybinding (:39).

## Frontend consumption

25. **Render an unknown top-level `StructuredEvent.type`** — `consume()`'s switch has no `default:`
    arm (`src/components/slot/structuredAccumulator.ts:66-138`); an unmatched `type` pushes no item,
    bumps no counter, logs nothing, throws nothing — and the caller still advances the seq high-water
    (`RichSlot.tsx:156`), still re-renders and auto-scrolls (:165, :221-224), and still clears the
    "waiting" indicator (:167). **The pixel result is "nothing happened, but the spinner is gone."**
26. **Get a new frame tag to the frontend at all** — `src/api/wsFrame.ts:35` hard-drops everything
    but 0, 1 and 255 before `ProtocolClient` ever sees it, silently: no unknown-tag callback, no
    passthrough, no counter. Until that line is edited, a `tag=2` stream is indistinguishable from an
    idle agent.
27. **Attach per-frame metadata** — `OutputChunk` is `{seq, tag, bytes}` and nothing else
    (`src/api/agentClient.ts:22-32`); `epoch` is consumed inside `ProtocolClient` (:266, :270) and
    never handed to a slot. There is no room for a backend id, a stream id, a turn id, or a mime type.
28. **Display a mixed stream** — all three slots are single-tag consumers by construction
    (`TerminalSlot.tsx:261` tag0, `RichSlot.tsx:161` tag1, `DomSlot.tsx:132` tag0). **No component
    accepts both tags**, so a backend interleaving raw bytes and structured events in one seq space
    cannot be shown coherently anywhere — one half is always dropped, silently, with the seq cursor
    already past it.
29. **Surface any dropped frame to a human** — every drop is a bare `return` (`wsFrame.ts:35`,
    `TerminalSlot.tsx:261`, `RichSlot.tsx:161`, `DomSlot.tsx:132`) or an unmatched switch
    (`structuredAccumulator.ts:138`). Release WebView2 has no devtools — the stated reason
    `ConnectionNotice` had to exist (`ConnectionNotice.tsx:3-5`). So "we don't model this event
    shape" and "the agent produced no output" look identical on screen.
30. **See raw tag-1 bytes anywhere** — `DomSlot` is the closest thing to a raw view and it drops
    tag1 by design (`DomSlot.tsx:132`); routes are only `/`, `/tree`, `/popup` (`src/App.tsx:63-68`)
    and `src/lab/` is unimported dead code. There is no raw-frame inspector.
31. **Bound accumulator growth** — `StructuredEventAccumulator.items` grows without limit for the
    life of a subscription; no cap, no trim, no eviction anywhere in
    `src/components/slot/structuredAccumulator.ts` (contrast `DomSlot`'s 200 KB cap, :36).
32. **Persist or share a render-mode choice** — `renderModeOverride` is in-memory and per-webview
    (`src/store/viewStore.ts:78`, `renderModeCommands.ts:9-10`); restart or a second window reverts
    to the capability-derived default. And during agent absence there is no derivation, so set/clear
    are **silent no-ops that report success** (`ViewLayoutRenderer.tsx:60`).
33. **Restore a structured event from the snapshot path** — `OutputCore::snapshot` keeps only
    `TerminalBytes` and `warn`-drops every structured variant (output_core.rs:558-579), so the two
    restore paths (`subscribe_from` replay vs `get_snapshot`) are asymmetric by the code's own
    admission (:551-557).

## Turn state / busy gate

34. **Signal "a turn started" out of band** — the only input to the fact layer is
    `TurnClassifier = fn(&OutputEvent) -> Option<TurnSignal>` (backend/mod.rs:268). A backend that
    knows its own turn boundaries structurally has no way to say so except by emitting an
    `OutputEvent` that the classifier happens to map.
35. **Push a turn-*start* notification** — `StatusSink` has `turn_ended` only (types.rs:732); there is
    no `turn_started`.
36. **Read turn state from the frontend or an LLM** — nothing turn-shaped exists in the wire protocol
    or in `src-tauri`; the daemon's `turn_ended` decorator forwards inward to a default no-op and its
    own comment calls the path "예정" (messaging_host.rs:878-882).
37. **Ask "is this agent busy" without an epoch** — every read verb requires `(id, epoch)`
    (turn.rs:207, :219; busy.rs:91).
38. **Correlate a turn end to its start / count turns** — `turn_id`/`message_id` are always `None`
    (backend/claude/mod.rs:869-870), so the table is "last observation wins" (turn.rs:191-194).
39. **Clear or mark a fact from a consumer** — forbidden by explicit port contract (busy.rs:86:
    "이 포트에 '지워라/표시해라' 를 추가하지 말 것"); the only removal verbs are `forget`
    (turn.rs:242) and `register`'s eviction (:125-133).
40. **Clear the whole fact map** — no `clear`, no `retain`, no drain (turn.rs, read in full); the map
    dies only with the `AgentManager`.
41. **Have the reaper clean up a leaked turn fact** — `reaper.rs` has no reference to the turn layer
    (deliberate, ADR-0127 결정 5); an entry `finish` never reached produces a warn plus a doorbell
    every 30 min forever, indistinguishable from a real long turn (busy.rs:146-151).
42. **Tell "this backend cannot report turns" from "this backend is idle"** — both are
    absence/`in_turn: false` in the fact layer; the distinction is smuggled in as the roster's
    `turn_signal = capabilities.output.structured` proxy (messaging_host.rs:104, busy.rs:19-24).
    **Today every codex agent is permanently unobserved** because `CodexBackend` does not override
    `turn_classifier` (backend/mod.rs:143-145 default) ⇒ always idle ⇒ mail injected immediately.
43. **Force a turn to "ended" on interrupt or cancel** — `interrupt` emits no turn signal
    (manager.rs:1748-1750); the entry stays `in_turn: true` until a real `MessageDone`, a `finish`,
    or the 30-minute sweep.
44. **Use a different staleness ceiling per consumer** — `BUSY_MAX_TURN` is a `pub const` in the mail
    kernel (busy.rs:57); a second consumer wanting another bound must build its own ledger.
45. **Survive a daemon restart with turn state** — no persistence anywhere; after a restart every
    `(id, epoch)` is unobserved ⇒ idle ⇒ immediate injection (busy.rs:178-180).

## Profile / persistence / resume

46. **Store a non-UUID resume handle** (a conversation id string, a rollout file path) — the only
    slot is `backend_session_id: Option<Uuid>` (profile.rs:165). **No mechanism exists.**
47. **Let a backend report a session id it minted itself** — the port is pull-only polling
    (session_tracker.rs:47-51, backend/mod.rs:251-258) and there is no `AgentBackend` method to push
    one. And the pull path is gated on `needs_session()` (manager.rs:1076-1082), which codex sets to
    `false` (backend/codex/mod.rs:59-61) — so a codex agent can never record one.
48. **Learn a session id from the child's own output stream** — the decoder cannot send anything
    (item 13), and `observe_session_id`'s only caller is the tracker's file-polling closure
    (daemon/src/lib.rs:288).
49. **Guard a session-id observation by incarnation** — `observe_session_id` takes no `incarnation`
    argument (profile.rs:629), unlike `set_last_failure` (:575). A late poll from a dead incarnation
    overwrites the live one's sid unconditionally.
50. **Record when a process last started** — `last_start_at` has **no writer at all**
    (profile.rs:229-230; only the `None` init at :261 and a wire mirror at
    connection_core.rs:498). A dead slot.
51. **Persist an incarnation marker, or which incarnation produced an observation** — `epoch` is
    `skip_deserializing` and always serialized as literal `0` (profile.rs:190, :681-683); deliberate,
    but it means nothing on disk can be attributed to an incarnation.
52. **Persist replay/output recovery state** (ring contents, seq cursor, turn facts) — neither
    `output_core.rs` nor `turn.rs` has a serde derive at all.
53. **Persist a resume attempt count or last-resume outcome** — `RestoreReport`/`RestoreOutcome` are
    `Serialize`-only wire reports (profile.rs:92-121), never stored; `last_failure` is
    `#[serde(skip)]` (:207) so it dies with the daemon.
54. **Surface a save failure to a caller, the UI, or the LLM** — swallowed with `tracing::error!`
    only (persistence/mod.rs:99-106); the contract explicitly says so (profile.rs:310-311).
55. **Batch, debounce, or flush-on-shutdown the profile store** — every mutation rewrites the whole
    file synchronously inside the profiles mutex (profile.rs:395, :406); there is no dirty flag and
    no hook to attach one. `epoch_for_spawn` pays a full file write on **every spawn** for bytes that
    never change.
56. **Migrate `agents.json` across schema versions** — a version mismatch yields an empty in-memory
    list with the file left **un-backed-up** (persistence/mod.rs:121-126), and the next save
    overwrites it (profile.rs:395). Unlike the parse-error path, which does rename to `.corrupt-<ms>`.
57. **Read back what was persisted** — `ProfileStore::load` has exactly one caller,
    `ProfileRegistry::new` (profile.rs:380); nothing re-reads after boot.
58. **Roll back a partially-applied `mutate_if` closure** — the closure gets `&mut` to the map with
    no rollback; returning `false` leaves an unpersisted, unnormalized in-memory change
    (profile.rs:403-407). Unenforced hazard.
59. **Garbage-collect corpse profiles** — removal has one trigger, the explicit delete verb
    (manager.rs:690-692); no TTL, no capacity eviction, no crash pruning.
60. **Unwatch a tracker entry when the child dies on its own** — the only `unwatch` is in `kill_agent`
    (manager.rs:1778) and "reaper 는 tracker 를 모른다" (:1777). Naturally-exited agents leave a live
    polling entry for the daemon's lifetime.
61. **Re-arm a `Degraded` session-id observer** — the doc says unwatch+watch is required
    (session_tracker.rs:60-62) and nothing calls that pair.

## Manager / lifecycle

62. **Choose a transport for a reason other than the backend's declared shape** —
    `select_transport` has exactly two arms and takes `TransportShape` only (manager.rs:79-101);
    `ApiTransport` exists but is unreachable (api.rs:4, "manager 라우팅은 없음").
63. **Prevent a double spawn of the same profile from two connections** — the `get_session`
    pre-check drops its read lock before the write-lock insert, and the code documents the window as
    open and pre-existing (manager.rs:917-922). `SpawnReservation` (:932-940) narrows but does not
    close it.
64. **Remove a session from the map from anywhere but the reaper** — `reap_one` is the only remover
    (reaper.rs:57-64); `kill_agent` explicitly does not (manager.rs:1753-1756), so a caller cannot
    assert "it is gone" without polling.
65. **Have a disposition other than keep-or-downgrade** — `Disposition` has two variants and no
    delete (types.rs:127-137, ADR-0083); `apply_disposition` is downgrade-only (reaper.rs:129-142).
66. **Reap concurrently, or survive a wedged reap** — one global serial thread (reaper.rs:193, from
    manager.rs:519) consumes every session's terminal for the whole daemon.
67. **Enforce that production never assembles a core with turn observation disabled** —
    `TurnWiring::detached()` is `#[doc(hidden)]` and the doc concedes "그 구분을 강제하는 장치는
    **없다** … 이건 컴파일러가 아니라 규약이 지키는 경계다" (output_core.rs:103-105).
68. **Choose the activation mode on any basis other than "is a session id stored"** — all three
    entrances read exactly `profile.backend_session_id.is_some()` (commands.rs:521-525;
    connection_core.rs:1126-1130) or hardcode `Fresh` (connection_core.rs:809). The wire `resume`
    flag is only OR-ed in, never decisive.
69. **Verify a resume actually resumed** — the only check is a **blocking 3-second poll** of the
    child's status and its stderr text (`EARLY_EXIT_WINDOW`, manager.rs:53; loop :1562-1584),
    classified by a per-backend string matcher over the diagnostic tail
    (`backend::resume_failure_kind`, :1577). There is no protocol-level confirmation.
70. **Classify an activation failure from anything but text** — ``early_activation_verdict`` feeds
    `resume_failure_kind` only `session.diagnostic_tail()` (manager.rs:1577) while the concatenated
    terminal+diagnostic evidence is used only for the human-readable reason (:1568-1573) — an
    asymmetry, so a PTY-mode failure whose evidence lives in the terminal ring is never classified.
