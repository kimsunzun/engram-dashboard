# LLD Stage 1 검토 — fable

**검토자:** fable (pane 8), 2026-06-11
**대상:** `backend-lld-stage1.md`
**방법:** adversarial — 통과시키지 않을 이유를 우선 탐색. 확신 수준 표기: [확실] / [가능성 높음] / [불확실]

---

## 종합 판정: 조건부 GO

구조 골격(OutputSink 추상화, 필드별 Mutex, OS thread drain, 모듈 격리 원칙)은 타당하다. 그러나 **Critical 4건은 코드 작성 전에 반드시 설계를 수정**해야 한다 — 그대로 구현하면 컴파일 실패가 아니라 "돌아가는데 터미널이 멈춰 보이는" 류의 디버깅 비싼 버그가 된다. Critical 반영 후 2단계 진행을 권한다. 전면 재설계는 불필요하다.

---

## Critical — 구현 착수 전 수정 필수

### C1. `pty/` Tauri 격리 원칙이 LLD 내부에서 이미 깨져 있다 [확실]

§1은 "pty/ 하위 모든 파일 tauri import 금지 + CI grep 강제"를 선언하는데:

- §5 `PtyManager`(pty/manager.rs)가 `app_handle: Option<AppHandle>` 필드를 가진다 — `AppHandle`은 tauri 타입.
- §6 drain 의사코드 마지막 줄이 `emit_agent_status_changed(...)` — drain.rs(역시 격리 대상)에서 Tauri event emit.

선언한 CI lint가 자기 설계를 reject한다. **수정:** `OutputSink`와 동일 패턴으로 `StatusSink` trait(예: `fn status_changed(&self, id: AgentId, status: AgentStatus)`)을 types.rs에 추가하고, commands 층에서 AppHandle 기반 impl을 주입. `PtyManager.app_handle` 필드 삭제. 부수 효과로 `set_app_handle(&mut self)` 문제(Arc 공유 후 `&mut` 호출 불가)도 함께 사라진다.

### C2. 배칭 로직에 "마지막 partial batch 정체" 버그 — 프롬프트가 화면에 안 뜬다 [확실]

§6 의사코드 흐름: read 반환 → batch 누적 → `batch.len() < 4096 && elapsed < 8ms` 이면 `continue` → **다시 blocking read로 복귀**.

출력 burst가 4KB 경계가 아닌 곳에서 끝나고(항상 그렇다 — 프롬프트, 키 입력 echo, 짧은 응답) 마지막 send 후 8ms 미경과 상태면, 잔여 batch를 든 채 blocking read에 들어가 **다음 출력이 올 때까지 무기한 미전송**. Claude Code가 프롬프트를 찍고 입력을 기다리는 바로 그 순간이 정확히 이 케이스다 — 사용자는 빈 화면을 보고, 입력할 이유가 없으니 다음 read도 영원히 안 온다. 시간 조건(8ms)은 read가 반환된 뒤에만 평가되므로 타이머 역할을 못 한다.

**수정 — 둘 중 하나 결정 필요:**
- (a) **2-thread 분리(권장):** reader thread는 blocking read → 내부 `mpsc::Sender`로 즉시 push만. batcher thread가 `recv_timeout(8ms)`로 수신하며 배칭 — timeout 발생 시 잔여 batch 무조건 flush. blocking read와 타이머가 분리되어 정체 불가능.
- (b) **Rust 단 배칭 포기:** read 반환분을 즉시 send (read 자체가 자연 배칭 — ConPTY는 보통 수백 byte~수 KB 단위로 반환). 프론트 xterm.js 단 8~16ms 배칭(이미 계획됨)에 위임. 구현 최소.

(b)로 시작해 성능 실측 후 (a) 전환이 현실적이다. 어느 쪽이든 §6 의사코드는 폐기·재작성.

### C3. `child.kill()` → EOF 가정이 Windows ConPTY에서 보장되지 않는다 — kill마다 5초 hang + thread leak [가능성 높음]

§6 종료 메커니즘과 검토질문 1에 대한 답. ConPTY의 알려진 동작: **클라이언트 프로세스가 죽어도 output pipe의 ReadFile은 pseudoconsole이 닫힐 때까지(ClosePseudoConsole) 반환되지 않는 케이스가 있다** (Windows 버전·conpty 빌드에 따라 동작이 다름 — Microsoft 공식 샘플도 "프로세스 종료 후 pseudoconsole을 닫고 나서 잔여 출력을 drain"하는 순서를 쓴다). portable-pty에서 ClosePseudoConsole은 **master drop** 시 일어난다. 현재 설계는 master를 `PtySession.master`에 계속 보유하므로:

```
kill_agent: join 대기 ← drain: read 블록 ← EOF: master drop 필요 ← master drop: session 제거 후 ← session 제거: join 후
```

순환 — 5초 타임아웃이 매번 발동하고 detach된 thread + conhost가 누적된다. "어쩌다 실패"가 아니라 해당 환경에선 **매 kill마다** 발생.

**수정:** `master`를 `Mutex<Option<Box<dyn MasterPty>>>`로 바꾸고 kill 순서를 `shutdown.store → child.kill() → child.wait()(reap) → master.lock().take()로 drop (ClosePseudoConsole → reader가 Err/EOF) → join(timeout)`으로. Exiting 이후 resize가 불가능해지는 건 의미상 올바른 부수효과다. 타임아웃+detach는 최후 방어선으로 유지하되 정상 경로가 되어선 안 된다. EOF 대신 `Err(BrokEN_PIPE)`로 반환될 수 있으므로 §6 의사코드의 `Ok(0) | Err(_) => break` 분기는 그대로 유효. **[검증]** 2단계 첫 스파이크에서 "kill 후 join이 timeout 없이 즉시 완료"를 Windows 실기기로 확인하는 것을 최우선 항목으로.

### C4. replay→live 전환: gap/중복은 없지만 **도착 순서 역전**이 가능하다 [확실 — 경합 발생 시]

§7의 분석은 "replay 0..N, live N+1.. → gap 없음"까지는 맞다. 그러나 replay 전송이 lock 해제 **후**에 일어나므로: 구독 등록 직후 drain 사이클이 돌면 sink는 **live chunk(N+1)를 replay(0..N)보다 먼저 수신**한다. Tauri Channel은 send 호출 순서대로 전달하므로, 서로 다른 두 스레드(subscribe 스레드의 replay send vs drain thread의 live send)가 같은 channel에 보내는 순서는 보장 대상이 아니다. xterm.js에 N+1 → 0..N 순으로 써지면 화면 깨짐. 고속 출력 중 후발 attach(LLD 자신의 검증 포인트 시나리오)에서 재현된다.

**수정 — 셋 중 하나 결정 필요:**
- (a) **subscribers lock을 쥔 채 replay 전송 (단순, 권장):** 락 순서 규칙 3의 명시적 예외로 문서화. drain은 그 agent의 step 6에서만 잠깐 대기(PTY 커널 버퍼가 흡수). attach는 일회성 이벤트라 비용 허용 가능.
- (b) sink에 `ready` gate + pending queue: drain이 not-ready sink엔 pending에 적재, replay 완료 후 flush + ready. 락 규칙 무결성 유지, 구현 복잡.
- (c) 프론트엔드 seq 재정렬 버퍼: 백엔드 무수정, 복잡성이 프론트로 이동.

### C2~C4는 LLD가 스스로 "해결했다"고 선언한 절(§6, §7)에 있는 결함이라는 점을 강조한다 — 2단계 코드 검증에서 이 세 곳은 의사코드 대비 적합성이 아니라 수정안 대비로 봐야 한다.

---

## Major — 2단계 착수 전 결정 필요

### M1. `AppState { manager: Arc<Mutex<PtyManager>> }` → `Arc<PtyManager>` (검토질문 5 답: 그렇다, 바꿔야 한다) [확실]

PtyManager는 이미 내부 동기화(`Arc<RwLock<HashMap>>`)를 가지며 모든 메서드가 `&self`다. 외부 Mutex는 불필요할 뿐 아니라 유해하다: `kill_agent`의 **join 5초 대기 동안 전체 에이전트의 모든 command가 전역 차단**된다(다른 agent의 write_stdin까지). 유일한 `&mut`인 `set_app_handle`은 C1의 StatusSink 주입(생성자 인자 또는 `OnceLock`)으로 대체. 추가로: async command 안에서 blocking 작업(join 5s, blocking write)을 하면 tokio worker를 점유하므로, `kill_agent`/`shutdown_all`은 `spawn_blocking`으로 감싸거나 sync command로.

### M2. 구독 해제를 send-실패 감지에만 의존 — 가정 미검증 + 명시적 unsubscribe API 부재 [불확실 — 그래서 위험]

§12(c)의 "창 닫힘 → Channel drop → 다음 send에서 SinkError" 경로에서, **webview가 죽은 뒤 `Channel::send`가 실제로 `Err`를 반환하는지는 Tauri v2에서 보장 문서가 없다** — 조용히 Ok를 반환하면 죽은 sink가 영구 누적되어 chunk마다 직렬화 낭비가 쌓인다. 또한:

- commands 목록에 `unsubscribe`가 없다. §11 자원표는 "or unsubscribe"라고 쓰는데 그 API가 설계에 존재하지 않는다.
- React 18 StrictMode는 dev에서 effect를 2회 실행한다 — §(final.md) TerminalSlot의 cleanup이 "GC 시 자동 제거"(no-op)이므로 **dev에서 출력 2배 중복**이 그대로 발생한다.

**수정:** ① `subscribe`가 `SinkId` 반환 + `unsubscribe_agent_output(agent_id, sink_id)` command 추가, 프론트 effect cleanup에서 호출. ② Rust 측 `WindowEvent::Destroyed` 훅에서 해당 창 sink 일괄 제거(브라우저식 GC 의존 제거). ③ send-실패 감지는 3차 방어선으로 유지. 2단계 첫 스파이크에서 "창 닫은 뒤 send 반환값" 실측 항목에 추가.

### M3. Cargo.toml 오류 3건 + 버전 주장 1건 미확인

- `tauri-build = { version = "2.0", build-dependencies = true }` — 존재하지 않는 키. `[build-dependencies]` 섹션으로 이동해야 한다. [확실]
- `thiserror` 미선언 — §3이 `#[derive(thiserror::Error)]`를 쓰는데 dependencies에 없다. 컴파일 실패. [확실]
- `uuid`에 `serde` feature 누락 — `AgentId = Uuid`가 Serialize 파생 구조체들에 포함되므로 `features = ["v4", "serde"]` 필요. 컴파일 실패. [확실]
- "portable-pty 0.9.x Windows garbage 이슈로 0.8.1 고정" — **이 이슈의 출처를 나는 확인할 수 없다** [불확실]. 근거(issue 링크)를 LLD에 박거나, 근거 없으면 "최신 안정 버전 + smoke test"로 문구 수정. 잘못된 핀 고정은 업스트림 수정을 놓치는 비용이 있다.

### M4. PtyEvent wire format — `Vec<u8>` + serde JSON은 byte당 정수 배열로 직렬화된다 [가능성 높음]

`#[derive(Serialize)]` + `data: Vec<u8>`를 Tauri Channel로 보내면 기본 직렬화에서 `[27,91,51,49,...]` 형태 JSON 숫자 배열이 된다 — 원본 대비 3~4배 크기 + 프론트 파싱 비용. 고빈도 PTY 스트림과 2MB replay에서 체감되는 수준. Gemini 1차 지적("Vec<u8>/Uint8Array")이 §3에서 "String 변환 없음"으로만 반영됐는데, **String을 피하는 것과 바이너리로 전송되는 것은 다른 문제다.** Tauri v2는 raw payload 경로(`tauri::ipc::InvokeResponseBody::Raw(Vec<u8>)`를 Channel로 송신)가 있다 — 단 이 경우 seq/agent_id 메타데이터 프레이밍(약식 헤더)을 직접 설계해야 한다. **결정 필요:** raw + 자체 프레이밍 vs JSON + base64 문자열(중간 절충) vs 일단 정수배열로 가고 실측 후 교체. Channel 타입 시그니처가 바뀌는 결정이므로 2단계 전에 정하는 게 싸다.

### M5. 상태 전이 소유권 race + 모호한 전이 3건 [확실 — 모호함 자체가]

- **Killed vs Exited race:** §6 의사코드는 loop 탈출 시 무조건 `update_status(Exited/Failed)`인데, §9는 `Exiting → Killed`(drain 수행), §6의 kill 순서 step 7은 kill_agent가 `status = Killed` 설정. 세 곳이 서로 다르다. kill 직후 EOF로 빠진 drain이 Exited를 써버리면 Killed가 영영 안 나온다. **수정:** 전이를 단일 함수 `transition(session, event)`로 모으고, status lock 안에서 `shutdown` 플래그를 보고 Killed/Exited를 판정. 전이 수행 주체를 "drain thread 단독"으로 통일하는 게 가장 단순하다.
- **Failed vs Exited 기준 모순:** §9는 `code ≠ 0 → Failed`, §12(a)는 "Exited { code } or Failed { message }". **권장:** 프로세스가 종료된 모든 경우 `Exited { code }`(code≠0 포함), `Failed`는 spawn 실패·내부 오류 전용. 프론트가 code로 비정상 여부 표시.
- **Starting → Running 트리거 "child 프로세스 시작 확인"** — 무엇으로 확인하는지 미정의(첫 read 성공? spawn 반환?). spawn_command가 Ok면 즉시 Running으로 간주하고 Starting을 제거하는 단순화도 검토할 것.

### M6. drain thread JoinHandle의 보관 위치가 정의돼 있지 않다 [확실]

§11 자원표는 소유자를 "PtyManager (JoinHandle)"라 하는데 §5 PtyManager struct에 해당 필드가 없다. kill_agent와 shutdown_all이 join하려면 어딘가에 있어야 한다. **권장:** `PtySession.drain_handle: Mutex<Option<JoinHandle<()>>>` (drain thread는 Arc<PtySession>을 들지만 JoinHandle은 세션을 참조하지 않으므로 순환 없음).

---

## Minor — 2단계에서 처리 가능

1. **`Box<dyn OutputSink>` vs §6의 `clone_ref()` 모순** — Box는 clone 불가. `subscribers: Mutex<Vec<Arc<dyn OutputSink>>>`로 확정하라. [확실]
2. **§4 `job_handle: HANDLE` (raw) vs §13 `JobObjectHandle`(Drop wrapper) 불일치** — §4도 wrapper 타입으로. raw HANDLE은 Drop이 없어 leak. [확실]
3. **Job Object 할당 창(window):** spawn 후 `AssignProcessToJobObject` 사이에 child가 만든 손자 프로세스는 job 밖이다. `create_and_assign(pid)`는 pid가 아니라 process HANDLE이 필요(OpenProcess 경유). 실용상 허용 가능한 잔여 위험으로 문서화. [확실]
4. **`#![windows_subsystem = "windows"]` 무조건 + lib.rs 위치** — debug 빌드에서도 콘솔이 사라져 로그를 잃는다. final.md의 원래 형태(`cfg_attr(not(debug_assertions), ...)`)가 맞고, 위치는 bin crate(main.rs). [확실]
5. **§12(a)와 §6 의사코드 불일치:** "마지막 flush (남은 batch 전송)"가 의사코드 loop 탈출 후 경로에 없다. C2 재작성 시 함께 반영. [확실]
6. **spawn_agent에 초기 cols/rows 인자가 없다** — 80x24로 떴다가 resize되면 TUI 첫 화면이 깨진다. `spawn_agent(cwd, cols, rows)`로. 
7. **slave 핸들 미기재** — openpty의 PtyPair 중 slave는 spawn_command 후 즉시 drop해야 한다(유지 시 EOF 감지 방해 가능). §11 자원표에 행 추가.
8. **replay 재생 화면 충실도** — 1차 리뷰 추적표는 "§7 (주석)"이라 하지만 §7에 실제 처리가 없다. Claude Code 같은 풀스크린 TUI의 VT 스트림을 중간부터 재생하면 화면이 깨진다. 현실적 완화: attach 후 rows±1 → 원복 resize "nudge"로 ConPTY 전체 repaint 유도. 완전 해결은 불가능함을 비목표로 명시하는 것도 방법. [가능성 높음]
9. **workspace root 검증의 출처 미정의** — final.md 보안 절의 "cwd canonicalize + workspace root 밖 거부"가 LLD에서 사라졌다. `CwdDenied` 에러만 남아 있음. root를 어디서 읽는지(설정 파일? 상수?) 미결정.
10. **테스트가 spawn하는 binary** — "실행 binary 고정" 보안 원칙과 headless 테스트(§14)가 충돌. IPC에 노출되지 않는 내부 생성자(`spawn_with_command`)로 테스트만 임의 커맨드(`cmd /c echo`) 주입 허용을 권장.

---

## 검토 요청 질문 6건 — 직접 답변

1. **child.kill() → EOF, ConPTY에서 동작?** 보장 안 됨 — C3 참조. EOF 대신 Err 반환 케이스: 있다(broken pipe). master drop을 종료 시퀀스에 넣어야 한다. [가능성 높음]
2. **subscribers+replay 동시 lock 데드락?** 명세대로면 없다 — drain은 두 락을 동시에 들지 않고(각 statement에서 임시 guard), write_stdin은 writer만 든다. 단 이는 "drain이 둘을 동시에 들지 않는다"는 **암묵 불변식**에 기댄 것이므로 규칙 2에 "drain thread는 session 내부 lock을 2개 이상 동시 보유 금지"를 명문으로 추가하라. 그리고 C4 수정안 (a)를 채택하면 예외 1건이 생기니 함께 문서화. [확실]
3. **Channel을 Box\<dyn OutputSink\>로 래핑 시 known issue?** `tauri::ipc::Channel`은 Clone+Send+Sync라 래핑 자체는 문제없다. 실제 위험은 ① 죽은 webview에 대한 send가 Err를 주는지 미보장(M2) ② wire format 비효율(M4). [가능성 높음]
4. **RwLock sessions 기아?** 사실상 없다. read 보유가 lookup 수 마이크로초뿐이고, drain thread는 sessions 맵을 아예 안 잡는다(Arc 직접 보유). 경합 주체가 키 입력 빈도의 write_stdin 정도라 starvation이 성립할 부하가 아니다. 비문제. [확실]
5. **AppState Mutex\<PtyManager\>?** 바꿔야 한다 — `Arc<PtyManager>`. M1 참조. [확실]
6. **놓친 이슈?** C1(격리 모순), C2(배칭 정체), C4(순서 역전), M2(unsubscribe 부재 + StrictMode 이중 구독), M4(wire format), M5(상태 race), M6(JoinHandle 위치), Minor 6/7/9/10.

---

## 추가 질문: "이 LLD 기준으로 구현 시작해도 되는가?"

**지금 그대로는 아니다. Critical 4건(C1~C4)을 LLD에 반영·재확정한 뒤 시작하라.** 골격은 건강하므로 반영은 국소 수정으로 끝난다(전면 재설계 불필요).

**아직 결정 안 된 위험 항목 (구현 중 만나면 비싸지는 순):**

| # | 미결정 사항 | 위험도 | 어디서 |
|---|---|---|---|
| 1 | 배칭 구조: 2-thread vs 즉시 send (C2) | 코어 루프 전면 좌우 | §6 |
| 2 | PtyEvent wire format: raw IPC vs JSON (M4) | Channel 시그니처·프론트 코드 좌우 | §3, §8 |
| 3 | replay 순서 보장 방식: lock 내 전송 vs gate vs 프론트 재정렬 (C4) | §7 |
| 4 | StatusSink 도입 형태 (C1) | pty/ 전체 시그니처 | §5, §6 |
| 5 | unsubscribe API + 창 파괴 훅 (M2) | command 표면 | §8 |
| 6 | 상태 전이 단일 소유 규칙 + Failed/Exited 기준 (M5) | §9 |
| 7 | JoinHandle 보관 위치 (M6) | §4 or §5 |
| 8 | workspace root 출처, 테스트 binary 주입점 (Minor 9·10) | §14, 보안 |

**2단계 진행 방식 권고:** 모듈별 코드에 들어가기 전에, **첫 스파이크 1개**(spawn → 출력 수신 → kill → join 즉시 완료)를 Windows 실기기에서 돌려 C3 가정(master drop → read 해제)과 M2 가정(창 닫힘 후 send 반환값)을 실측으로 못박아라. 이 둘은 문서 검토로는 끝까지 [가능성 높음]/[불확실]을 못 벗어나는, 환경 의존 동작이다.
