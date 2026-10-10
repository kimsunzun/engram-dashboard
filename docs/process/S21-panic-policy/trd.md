# TRD — 패닉 정책 구현: 가두는 입구 · 훅 종료 · unwind 전환 (S21)

> 상태: **2판(2026-10-11) — 리뷰 1회차(deep · 리뷰어 셋 모두 FIX · BLOCK 0) 반영 · 사용자 결정 다섯 묶음 (가)~(마) 대기.** 코드는 한 줄도 바뀌지 않았다. 리뷰 항목별 대조는 §10.
> **2판 요지:** ① 락 오염 정책을 메인 결정에서 **사용자 결정 (가)** 로 올렸다 — 1판의 P1 은 codex 통로처럼 일부러 범위 안에서 되찾던 자리를 전부 데몬 재시작으로 바꾼다 ② 1판이 놓친 **「표시 안에서 공유 상태를 여러 걸음에 걸쳐 바꾸는 구간」**(활성화 등록 창의 좀비 세션 등)을 (나)로 ③ 종료를 당긴 뒤 꼬리가 새 명령을 계속 받는 창을 (다)로 ④ net 은 포장 포트를 걷고 **net 자기 루프 = 3층** ⑤ 깨우기는 unpark 를 빼고 폴링으로 · 부팅 · 꼬리 패닉은 `main.rs` 최상위 포착 → `exit(101)` ⑥ U8 을 갈라 의미 재감사(U8a)를 프로필 전환(U7) **앞**으로.
> **결정 틀(재론하지 않는다):** ADR-0288 · ADR-0290(ADR-0289 는 통째 폐기) + 사용자 결정 2026-10-10 「unwind 범위 = 워크스페이스 전체 + 셸에도 같은 훅」. ★세 ADR 은 브랜치 `origin/v0.3.3/feat/panic-policy` 에 있고 이 브랜치(`v0.3.3/refactor/crate-boundaries`)에는 아직 없다★ — 이 TRD 를 커밋하기 전에 그 브랜치를 들이거나 함께 머지해야 링크가 선다. 구현 ADR 번호 = **ADR-0292**(예약 · 사용자 결정 2026-10-10).
> **이 TRD 가 대체하는 옛 기록:** TRD `docs/process/S20-command-bus/trd.md` §4 ⑨ 「릴리즈 프로필의 `panic = "abort"` 를 유지한다」(옛 사용자 결정). 그 절의 **규약**(명령 핸들러는 값으로 실패한다)은 그대로 살고, **프로필 결정과 그 귀결**(「릴리즈에서는 패닉 그물이 서지 않는다 — 그것이 의도다」)만 대체된다(§8-2).
> **배치 근거:** `docs/README.md` 「새 기능 **설계 착수** → `process/SN-name/`」. 경계 리팩터링(`S21-crate-boundaries/`)과 다른 주제다 — TRD A §0 · §1-3 이 「패닉 대 되찾기 정책 — 별 주제」로 뺐다. 스텝 번호는 `docs/process/step-log.md` 의 마지막 스텝 S21 을 잇는다(새 스텝은 사용자 결정이라 올리지 않는다).
> **표기:** 「실측」 = 트리 `2c25cdd`(이 브랜치 HEAD — 1판이 잰 `18e11e8` 뒤 코드 변경은 platform `fs.rs` 두 줄뿐이고 이 TRD 는 그 자리를 인용하지 않는다. 2판의 `파일:줄` 은 리뷰가 든 자리까지 다시 쟀다) · 「사용자 결정」 = 사용자가 고를 것(겉으로 보이는 동작) · 「메인 결정(관행)」 = 안 보이는 내부라 메인이 정하고 보고할 것 · 「권고」 = 이 TRD 의 안(채택 전) · 「가안」 = 이름 · 모양 미확정 · 「미검」 = 돌려 보거나 원문과 대조하지 않은 것.
> 앵커: ADR-0288 · ADR-0290 · ADR-0001(kill 2동사) · ADR-0005(terminal 전이는 펌프 단독) · ADR-0006(락 순서) · ADR-0019(reaper 단일 소비자 · `shutting_down`) · ADR-0110(메시징 커널 무의존) · ADR-0127(턴 관측 정리 = 두 지점) · ADR-0129(net 경계) · ADR-0142(메시징 배달 · `in_flight_targets`) · ADR-0159(버스 패닉 답 = `INTERNAL` / `OUTCOME_UNKNOWN`) · ADR-0266 결정 2(platform 은 OS 의존 코드만) · ADR-0269(base 입주 조건) · ADR-0275 결정 1(base `sync` = 경고 없이 되찾기) · 조사 `docs/research/panic-policy-peers-2026-10-08.md` · `docs/research/stop-writes-after-internal-error-2026-10-10.md`(둘 다 패닉 정책 브랜치).

---

## 0. 결론 (먼저)

```
① 배포판을 unwind 로 바꾼다 — 워크스페이스 [profile.release] 한 줄(Cargo.toml:41). 가두는 자리가 사는 데몬 lib ·
     agent lib 에 컴파일 가드를 둬 abort 로 되돌리면 빌드가 멈춘다(cfg(panic = "unwind")).

② 「가두는 중」 표시 = 스레드 지역 깊이(const Cell · try_with). base 새 입주자 `panic`(가안)은 도메인 0 원시만 준다 —
     동기 contain · 비동기 Contained(poll 마다 깊이 · 가드 안에서 drop) · escalating(구간 안 깊이 0) ·
     is_poison_panic · 종료 걸쇠(SeqCst 깃발 + 첫 사유 + 첫 오염 자리). 「무엇을 종료로 올리나」 정책은 데몬 · 셸 조립부.

③ 훅(데몬 · 셸): 표시 밖이거나 락 오염 패닉이면 사유를 적고 → 깃발을 세우고(락 · unpark 없음) → 로그(재진입 가드).
     깨우기는 폴링 — 데몬 = accept 루프 select! 의 interval 팔이 깃발을 보고 기존 종료 watch 에 true(StopDaemon 과
     같은 신호) · 셸 = 깨우기 스레드가 park_timeout 고리로 보고 app.exit.

④ 데몬 끝: 기존 꼬리(shutdown_all → flush → MCP) → 당겨졌으면 process::exit(101) — 런타임 drop 의 무한 대기
     (blocking 태스크)를 건너뛰는 load-bearing 한 줄. 부팅 · 꼬리 자체의 패닉은 main.rs 최상위 포착 → exit(101).

⑤ 1층 = 데몬 처리기 몸통(WS 처리기 · 명령 태스크 · blocking · 제어 HTTP · MCP 도구 몸통). net 자기 루프 · 연결 수명
     · rmcp · hyper 내부 = 3층(→ 정상 종료). 2층 = (가)의 답. 여러 걸음 공유 변경 구간 = (나)의 답.

⑥ 단위 U1~U9 — 순서 = 표시 · 되찾기(U1~U4) → 의미 재감사(U8a) ∥ 훅(U5 · U6) → 프로필(U7) → 주석 · 문서(U8b · U9).
     어느 단위 뒤에 멈춰도 빌드 · 회귀가 초록이고 배포판 동작이 지금보다 나빠지지 않는다(§7-2 에서 다시 쟀다).
```

**사용자 결정 대기 — 묻는 순서(§3-1):**

| 순서 | 묶음 | 권고 | 뒤 묶음에 주는 영향 |
|---|---|---|---|
| 1 | **(가) 락 오염 정책 + 2층 경계** — 어떤 버그가 에이전트 하나로 끝나고 어떤 버그가 데몬 재시작이 되나 | **P2 + B1′** | U2 · U4 크기 · (마)의 2층 표면 개수. (나) · (다) · (라)의 선택지는 안 바뀐다 |
| 2 | **(나) 여러 걸음 공유 변경 구간** — 에이전트 시작 · 수거 · kill · 연결 붙이기 · 떼기 도중 패닉 | **E 올림**(그 구간 패닉 = 정상 종료) | U4 크기 |
| 3 | **(다) 종료를 당긴 뒤 새 일** — 꼬리가 도는 몇 초~수십 초 | **새 일 거절** | 없음 |
| 4 | **(라) 셸 패닉** | **S1a 셸만 끈다 · 셸 안 가둠 없음**(버스 명령 패닉도 창이 닫힌다) | (S1a′ 면 셸에도 컴파일 가드) |
| 5 | **(마) 1 · 2층 고지 모양** | **N1a 오류 답 · 연결 유지 + N2a 지금 표면** | 없음 |

**메인 결정(§3-2 — 권고 기본값을 적었다):** 표시 모양 · 집 · 훅 신호(폴링) · 로그 재진입 · 사유 기록 · net(N1) · 메인 future · 종료 코드 · 시험 방아쇠 · 회귀망 · 컴파일 가드 · 크기 · T-52 · 종료 경로 오염 내성 · 스폰 경로 스코프 가드 · `Contained` drop · 관찰자 계약 · 셸 끝내기 절차 · abort 전제 코드.

**지금 하지 않는 것(ADR-0288 이 미룬 장치 그대로):** 종료 시간 제한(꼬리의 남은 무한 대기 포함 — §4-11) · 셸의 사유 고지 · 다음 부팅의 패닉 표시 · 셸 상자 · 셸 자동 재기동 · 에이전트 PTY 를 데몬 밖 프로세스로(거부됨). 그리고 이 TRD 가 미루는 것: 정밀한 「공유 락을 쥔 채 났나」 추적(P3).

---

## 1. 정해진 틀 (재론하지 않는다)

| 출처 | 내용 | 이 TRD 의 자리 |
|---|---|---|
| ADR-0288 결정 1 | 요청 하나(명령 · 조회 · 연결)의 패닉 = 그 요청만 실패, 데몬 계속 | §4-6 · ★M5 가 「연결 하나」의 범위를 좁힌다(§8-1 Amends)★ |
| ADR-0288 결정 2 | 에이전트 하나만 섬기는 코드의 패닉 = 그 에이전트만 정리 · 정리 호출자 셋째 금지(ADR-0127) | §4-7 · (가) — 답에 따라 범위가 좁아진다(§8-1 Amends) |
| ADR-0288 · 0290 결정 3 | 공유 상태 패닉 = **기존 정상 종료 경로**(StopDaemon · 트레이 「데몬 끄기」와 같은 신호) · 전용 장치 없음 · 강한 경고 · 저장 멈춤 없음 | §4-4 · (나) · (다) |
| ADR-0288 결정 4 | Rust abort(이중 패닉 · Drop 패닉 · FFI 건넘) = 그냥 죽음 · Job 이 에이전트 정리 | §4-11 |
| ADR-0290 결정 2 | 표시 밖 패닉 = 훅이 정상 종료를 당긴다(zellij 모델) · 표시 모양은 TRD | §4-1 · §4-3 |
| ADR-0290 영향 | 훅은 락을 잡지 않는다 · 로그 Mutex 재진입 교착 창(`base/src/logging/mod.rs:110-115`)을 함께 본다 · **표시가 훅보다 먼저** · 종료 경로는 오염 락을 견딘다 · T-52 를 함께 본다 · 메인 future 를 가둘지 TRD 가 본다 · 연결 태스크(`lib.rs:394`)에 표시가 필요하다 | §4-3 · §4-4 · §4-5 · §7-2 · ★연결 태스크 표시는 세우지 않는다(M5 — §8-1 Amends)★ |
| 사용자 결정 2026-10-10 | unwind 범위 = **워크스페이스 전체**(데몬 · 셸 · CLI) + **셸에도 같은 훅** | §4-9 · §4-10 |
| ADR-0288 근거 | 미룬 장치 — 종료 시간 제한 · 셸 사유 고지 · 다음 부팅 복원 표시(「나중에 — 복원 시스템과 연관」) | §0 「지금 하지 않는 것」 |

---

## 2. 현황 실측 (트리 `2c25cdd`)

### 2-1. 프로필 · 훅 · 진입점

- `Cargo.toml:40-45` `[profile.release]` = `panic = "abort"` · `codegen-units = 1` · `lto = true` · `opt-level = "s"` · `strip = true`. 워크스페이스 유일 — 데몬 · 셸 · CLI 가 함께 바뀐다(Cargo 는 패키지별 `panic` 을 못 준다 — ADR-0288 영향).
- 데몬 훅 `daemon/src/lib.rs:129-151` `install_panic_hook`(Once · `tracing::error!` · 이전 훅 호출) · 설치 `:458`(로깅 초기화 `:453` 뒤) · 주석 `:455-457` 「데몬 전체는 죽이지 않는다 — 연결 task 는 tokio 가 격리 · pump 는 B-2」(abort 아래 거짓 · unwind 아래도 이제 틀린다).
- 데몬 `main.rs` = `#[tokio::main]`(기본 멀티스레드) · `run().await` 한 번 · `Err(code)` 면 **async main 안에서** `std::process::exit(code)`(`main.rs:11-16`) — 런타임 drop 을 건너뛰는 길이 이미 있다. `run()` 을 부르는 곳은 여기 하나라 **훅은 운영 데몬 프로세스에만 선다**(시험은 시험 서버 함수를 쓴다).
- tokio 런타임 drop 은 `spawn_blocking` 태스크가 끝날 때까지 **시한 없이** 기다린다(tokio 1.52.3 `runtime/runtime.rs:39-44` 문서 · `blocking/pool.rs:282-285`). tokio 에 atexit 훅은 없다 — `process::exit` 는 그 대기를 건너뛴다.
- 셸 = 훅 없음(`src-tauri/src` 에 `set_hook` 0). 시험 전용 훅 바꾸기 = `command/src/testing.rs:23-30` · `messaging/src/service.rs:9551` · `:9689`.

### 2-2. 운영 가둠 자리 (시험 모듈 밖 `catch_unwind` · 되감기 Drop 설계)

| 자리 | 섬기는 단위 | 잡은 뒤 결말(지금 debug) | 2판 배치 |
|---|---|---|---|
| `agent/src/transport/pty.rs:298` PTY 펌프 B-2 | 에이전트 하나 | `finish(Error("pump panicked: …"))` → `Failed` | 2층 |
| `agent/src/transport/stdio.rs:241` stdio 펌프 | 에이전트 하나 | 같음 | 2층 |
| `agent/src/backend/codex/transport.rs:4018` codex 리더 펌프(맨 `thread::spawn` `:4017`) | 에이전트 하나 | 같음(`resolve_pump_reason`) | 2층 |
| `codex/transport.rs:2420` 첫 턴 세션 id 기록 | 에이전트 하나(공유 프로필 저장을 부른다) | 그 화신의 연결을 내린다 | 2층 |
| `agent/src/transport/input_queue.rs:288` · `:300` 라이터 몸통 · 덩이 뒤 부를 것 | 에이전트 하나 | 큐를 닫는다(입력이 사유와 함께 거절 — 주석 `:273-279`) · 부를 것 패닉은 다음 덩이 계속 | 2층 |
| `agent/src/backend/claude/leftover.rs:722` 듣는 차례 + 일꾼(`:975-980` 기동 · `run_worker` `:1984` — 되감기 가드 `WorkerGuard::drop` `:1997-2001`) | 에이전트 하나(화신마다) | 포트를 굳히고 기록 끔 · 뗌 | 2층 |
| `agent/src/reaper.rs:208` 메시지 하나(스레드 `:195`) | **전역 스레드** · 일 단위 = 에이전트 하나의 수거 | 로그 · 다음 메시지 | **(나)** — `reap_one` 은 공유 변경 넷(§2-5) |
| `daemon/src/messaging_host.rs:759-775` `DeliveryDone`(수신자 배달마다 `spawn_blocking` — `spawn_delivery` `:779` · `:804-812`) | 수신 에이전트 하나(그 수신자 레인) | 되감기면 경고 + `tx.send(id)` 로 레인을 놓는다. 경고 문장(`:769`)이 「debug 한정, release=abort」 | 2층(결말 설계됨) · **U8a 감사 조건**(아래 둘) |
| `messaging/src/service.rs:2718` `Reservation` Drop · `:3505` `FlightSettle` Drop | 배달 하나 | 예약 되돌림(락이 바쁘거나 오염이면 오류 로그만) · `in_flight_targets` 제거 + settle(오염이면 `if let Ok` 로 **말없이 건너뜀**) | U8a — 메시징 커널은 base 를 못 부른다(ADR-0110) |
| `daemon/src/lib.rs:729` sweep 틱(태스크 `:715`) | **전역** — 메시징 파킹 TTL · busy fail-open | 경고 · 다음 틱 | 3층(catch 는 남겨 종료까지 루프를 살린다) |
| `daemon/src/control/registry.rs:333` 배달 관측 싱크 | 요청 안(운영 observer = `None`) | 경고 삼킴 | 바깥 입구를 따른다 |
| `command/src/route.rs:23` `guard_panic` · `:76` `CatchUnwind` · `:125` · `:148` | 버스 명령 하나 | `INTERNAL` / `OUTCOME_UNKNOWN`(ADR-0159) | 1층 — command 는 의존 0 이라 **부르는 쪽이 표시 안에서 부른다** |
| `command/src/link.rs:81` 답 배달 | 버스 명령 하나 | 삼킴 | 1층(같음) |

### 2-3. 가두지 않은 스레드 · 태스크

- **에이전트당(catch 없음):**
  - PTY 감시 `pty.rs:249-265`(`Builder…spawn(..).expect("spawn pty watcher thread")`). **자연 종료**의 결말은 설계돼 있다 — 감시가 master 를 놓아(`:259-260`) 리더 EOF → 펌프 `finish`(`:340`). ★**감시가 패닉하면 master 를 놓지 않는다**★(되감기 가드 없음) — ConPTY 는 자식이 끝나도 EOF 를 안 줘 펌프가 `shutdown()` 까지 매달린다. 감시는 자식 락을 `sync::lock`(`:258`)으로 · master 락을 `.expect`(`:260`)로 잡는다.
  - PTY 펌프 스레드 기동 `pty.rs:269` = 맨 `std::thread::spawn`(기동 실패 = 패닉).
  - stderr 배수 `transport/spawn.rs:250-268`(줄마다 `for` · catch 없음). 패닉 원 = `push_diagnostic` → `output_core.rs:1052` `diagnostics.lock().expect`. 죽으면 stderr 가 안 비워져 **파이프가 차면 자식의 stderr 쓰기가 막힌다** · 진단 락이 오염돼 뒤의 `push_diagnostic` 도 패닉한다.
  - codex 라이터 `codex/transport.rs:3973`(세션 id 기록 한 자리만 가둔다 — 그 밖 패닉이면 「닫힘 표식은 안 서므로 pump 도 reaper 도 움직이지 않는다」 — 헤더 `:82`) · codex `thread_lock.rs:398`(1판 실측).
- **전역:** reaper `reaper.rs:195` · 세션 추적 `session_tracker.rs:153`(관리자당 하나 · 고리 `:157` 에 catch 없음 · `stop()` `:188-192` 는 시한 없는 join 이지만 실제로는 잠 한 번 + 조회 한 번으로 끝난다) · 사용량 스케줄러 `daemon/src/usage_service/schedule.rs:36` · 사용량 조회 `agent/src/usage/process.rs:254`(아무도 join 하지 않는다 — 모듈 문서 `:7`) · platform `spawn/cmd_lookup.rs:236-238` · 메시징 flush 일꾼 = **tokio 태스크 둘**(`messaging_host.rs:708-709` — 1판의 「스레드」는 틀렸다) · sweep `lib.rs:715` · 명령 자리 수거기 = **tokio 태스크**(`command_delivery.rs:527` `spawn_sweeper` — 1판의 「스레드 `:537`」은 틀렸다).

### 2-4. 요청 처리 경로의 실제 모양

- accept 루프(`lib.rs:343` `run_accept_loop` · `select!` `:387` — 팔 셋: accept `:388` · 종료 watch `:414-423` · ctrl_c `:424`)가 연결마다 `tokio::spawn(handle_connection)`(`:395-406` · 핸들 안 쥠). net `handle_connection` 은 핸드셰이크(`ws.rs:323` · 인증 첫 프레임 ~`:432` · `registry.register` `:437`) → `on_connect`(`:447` · 하위 태스크 전) → **하위 태스크 넷**(read `:477` · bypass `:489` · dispatch `:498` · write `:506`) → 정리 `select!`(`:541-554`).
  - ★**정리 select 는 dispatch · write 두 핸들만 기다린다**★ — 각 팔은 read 를 abort · `finish_bypass_lane`(`:875`) · 다른 핸들 abort. bypass 가 끝나도 정리는 안 시작하고, read 는 포화 bypass 갈래에서 죽은 레인을 볼 때(`:781`)만 멈춘다. 하위 태스크 **안**의 패닉은 그 JoinHandle 이 `Err` 로 풀려 dispatch · write 팔이 정리를 탄다(read 패닉은 `inbound_tx` drop → dispatch 끝).
  - ★**하위 태스크를 띄운 뒤 `handle_connection` 몸통이 패닉하면**★ 핸들 drop 으로 하위 태스크가 떨어져 나가고 `on_disconnect`(`:557`) · `registry.unregister`(`:559`)를 건너뛴다. net 에 abort-on-drop 포장은 없다(명시 `.abort()` 만 — `:544` · `:546` · `:550` · `:552` · `:884`).
  - 데몬 처리기 호출 자리 = dispatch 태스크(`ws.rs:808`) · bypass 태스크(`:846`) · 연결 태스크(`on_connect` · `on_disconnect`).
- 데몬 처리기(`daemon/src/agent_conn.rs`): `on_connect` `:282`(`ConnUsageOutlet` 부착 `:307`) · `on_text` `:314`(떼어 낸 명령 = `tokio::spawn` `:336`) · `on_binary` `:375` · `on_inbound_saturated` `:405`(`command_request_id` `:433`) · `on_disconnect` `:450`(동기 — `.expect("subs poisoned")` `:498` · `.expect("owned_viewports poisoned")` `:515`: 오염이면 그 안에서 패닉해 net 의 `unregister` 를 건너뛴다).
- `connection_core`: 활성화 `spawn_blocking` 둘(`:1366` · `:1760` → `mgr.activate_profile` · `JoinError`(패닉 포함)는 `SpawnFailed("activation task failed: …")` 로 접힌다 — 지금 debug 에서 이미 가둔다) · 버스 배달 태스크 `:2106`(떼어 냄) → `command_delivery::deliver` → 1단계 본문 `spawn_blocking` `command_delivery.rs:1361`(`JoinError` 는 `is_panic()` 로 로그 `:1450-1458`). StopDaemon `:1512-1549` = `let _ = spawn_blocking(shutdown_all).await`(`:1543` · `JoinError` 무시) → 답 `:1545` → `shutdown_tx.send(true)` `:1547`.
- 제어 평면 HTTP(axum 0.8 · `control/mcp_server.rs`): 처리기 본문 `spawn_blocking` 여섯(`:513` · `:940` · `:1040` · `:1092` · `:1173` · `:1249`) · `bearer_auth` 함수 `:663`(`next.run` `:796`) · 층 순서 = `.layer(from_fn_with_state(bearer_auth))` `:1402` 다음 `.layer(RequestBodyLimitLayer)` `:1406` — **나중에 더한 층이 바깥**이라 지금 가장 바깥은 본문 크기 제한이다 · serve 태스크 `:1411`(axum graceful). MCP 도구(`#[tool]` `:399` · `:430` · `:525`) ★**는 rmcp 가 요청마다 띄우는 태스크 안에서 돈다**★(rmcp 2.2.0 — `tower.rs:670` 세션 일꾼 `tokio::spawn` · `service.rs:1184` 요청마다 `spawn_service_task` · 무상태 갈래 `tower.rs:1253-1257`) — axum 층이 `next.run` 을 감싸도 도구 처리기의 패닉을 못 덮는다(1판 §9-2 미검 → 확인).

### 2-5. 정상 종료 경로

- 신호 = `watch::channel(false)`(`lib.rs:790` — 부팅 10단계라 훅 설치 `:458` 보다 한참 뒤) · `run_accept_loop(…)` 호출 `:794-808` · 루프가 끝나면 `sweeper.stop().await` `:438`.
- 꼬리 순서(`lib.rs:811-837`): ① `restore_handle.abort()` + await(`:811-812` — 지금은 무동작 `spawn_blocking`) ② `sweep_task.abort()` + await(`:816-817`) ③ `spawn_blocking(shutdown_all).await`(`:823` · `JoinError` 는 경고만) ④ `flush_worker.shutdown().await`(`:827` — 태스크마다 5초 띠 · 넘으면 경고 후 떼어 냄 `messaging_host.rs:681-698` · `:714`) ⑤ `McpServerHandle::shutdown`(`:830-833` → `mcp_server.rs:351-356` 토큰 취소 후 serve 핸들을 **시한 없이** await) ⑥ 로그 · `Ok(())` `:837`.
- `AgentManager::shutdown_all`(`agent/src/manager.rs:3108`): `shutting_down.store` `:3113` → `tracker.stop()` `:3116` → `sessions.read().expect("sessions poisoned")` `:3119` → `thread::scope` 로 에이전트마다 `kill_agent`(`:3122-3124`).
  - `thread::scope` 는 한 스레드가 패닉해도 나머지를 기다린 뒤 다시 패닉한다 — 한 에이전트의 kill 패닉이 남의 kill 을 막지 않는다. 막는 것은 scope 앞 `:3119` 와 `kill_agent` 의 `session.kill` 앞 줄들이다.
  - `kill_agent`(`:3052`) 의 락 자리: `get_session` `:3053`(`:3137` `.read().expect`) → `begin_retire` `:3057`(원자값) → `control.revoke` `:3065`(데몬 구현 `control/registry.rs:261` `.write().expect("control registry poisoned")` — 같은 꼴 `:130` · `:158` · `:170` · `:204` · `:229` · `:240` · `:284` · `:293` · `:302`) → `set_intent` `:3067`(원자값) → `enter_exiting` `:3069`(`output_core.rs:803` `status.lock().expect`) → `session.kill` `:3074` → 통로 `shutdown`(자식 `.expect` `pty.rs:412` · master `.expect` `:421` · stdio 자식 `.expect` `stdio.rs:394`) → `tracker.unwatch` `:3077`(`session_tracker.rs:133-134` `.expect`). ★**이 길에 base `sync` 되찾기는 하나도 없다**★.
  - `join_pump`(`output_core.rs:822-831`)는 끝 알림 **수신자**를 꺼낸다(`take`) — 동시에 부른 둘째 호출자는 `None` 을 받고 곧바로 돌아온다.
- 막히면(예: 감시 패닉이 자식 락 · master 락을 오염 → 종료 경로의 `.expect` 가 둘째 패닉): 그 에이전트의 자식이 안 죽고 → 펌프가 안 끝나고 → 꼬리 ④ 는 5초 띠로 넘어가지만 그 자식은 데몬 프로세스가 끝나 Job 이 닫힐 때까지 산다.

### 2-6. 락 오염 현황 — ★운영 코드는 이미 「범위 안에서 되찾기」를 일부러 쓴다★

- base `sync`(`lock` `:24` · `read` `:28` · `write` `:32` · `wait_timeout` `:36-43` — 전부 `unwrap_or_else(PoisonError::into_inner)`)는 말없이 되찾는다. 헤더 계약 2(`sync.rs:7-9`) = 로그 · 다른 락 · 밖 호출 없음 · 계약 3(`:10-11`) = 「운영 빌드는 이 갈래에 닿지 않는다(abort)」 — unwind 가 되면 **거짓이 된다.**
- agent crate 운영 코드(어림 · 시험 모듈 앞까지): base `sync` 되찾기 **약 76** · `.expect("…poison…")` 락 **약 55**. 되찾기가 몰린 파일 = `codex/transport.rs` **42** · `input_queue.rs` 7 · `persistence/mod.rs` 7(공유 프로필) · `stdio.rs` 3 · `usage/process.rs` 3. `.expect` 가 몰린 파일 = `output_core.rs` **27** · `manager.rs` 10 · `pty.rs` 6 · `session_tracker.rs` 6.
- **일부러 범위 안에서 되찾는 자리(리뷰가 든 것 · 실측):** codex 상태 락(`sync::` 42 · `:3345` · `:3470` · `:3535` 등) · PTY 자식 락(`pty.rs:258` · `:319` — 펌프 주석 `:287-296` 은 core 락만 「의도적으로 fail-fast(expect)」라 적는다) · 첫 제출 래치(`session_id_latch.rs:26-28`) · reaper(`reaper.rs:54` `sync::write`) · 연결 사용량 출구(`agent_conn.rs:212-215`) · 입력 큐 7 · leftover(`GateCell` `leftover.rs:1717-1718`).
- **되감기 도중 자기 락을 다시 잡는 자리:** `ReaderExit::drop`(`codex/transport.rs:2901` → `:2918`) · leftover `Opener::drop`(`:1697-1704`) · `WorkerGuard::drop`(`:1997` → `:2001`). 이들은 자기가 막 오염시킨 락을 되감기 중에 곧바로 다시 잡는다 — 「오염 = 종료」 정책이면 **그 즉시** 종료가 당겨진다.
- **데이터 없는 순서 락(`Mutex<()>`, 운영):** `session.rs:74` `input_order`(`sync::lock` `:285` · `:565` — 래치 commit · encode · send · emit 을 걸쳐 쥔다) · codex `transport.rs:1097` `order` · `manager.rs:694` `name_allocation`. 셸에도 셋(`theme.rs:247` · `settings/mod.rs:145` · `commands/discovery.rs:34`).
- 펌프 B-2 의 오염 범위 논증(`pty.rs:287-296`)은 「tokio 가 그 task 만 격리」(`:293-294`)에 기댄다 — 훅이 들어오면 그 근거가 바뀐다.
- platform `cmd_lookup.rs:184-195` 는 base 를 의존하지 않아 자기 줄로 되찾는다(`:193-194`) — 관찰자에 닿지 않는다. 그 스레드는 3층이라 패닉하면 훅이 올린다(무해).
- 메시징 커널(`messaging`)은 워크스페이스 의존 0(ADR-0110)이라 자기 std 락을 쓴다 — 관찰자에 못 닿는다(§2-2 의 `FlightSettle` 처럼 `if let Ok` 로 건너뛰는 자리는 말없이 산다).

### 2-7. 셸

- 끝내기 = `tray/actions.rs:182` `quit_app` — 데몬 graceful stop(`send_stop` · 결과 무시) → `app.exit(0)` `:192`. 셸만 끄는 도우미는 없다.
- `RunEvent::Exit`(`src-tauri/src/lib.rs:400-411`) → 늘 `exit_session.shutdown()`(화면 상태 `Final` 쓰기 `clean_exit: true` — `state/saver.rs:571` · 셸 실행 잠금 놓기 · 기록기 답 마감 2초 `saver.rs:44`) · 시작 실패일 때만 `std::process::exit(STARTUP_FAILED_EXIT_CODE)` `:408-409`(주석 `:403-407` — tao 가 코드를 안 넘긴다 · 그 길엔 트레이가 아직 없다: 트레이는 시작 실패 판정 `:222-238` 뒤 `:275` 에서 만든다).
- Tauri 2.11.3: **사용자 `RunEvent::Exit` 콜백이 `cleanup_before_exit` 보다 먼저 돈다**(`app.rs:1430-1433`) — 그 콜백 안의 `process::exit` 는 트레이 아이콘 지우기(`cleanup_before_exit` `app.rs:1108` `pub` — 트레이 · 리소스 표 · 창 숨기기)를 건너뛴다(트레이 아이콘은 drop 때 `NIM_DELETE` — `tray-icon 0.24.1 windows/mod.rs:297-300`. 「유령 아이콘이 남는다」는 추론 · 미검). `AppHandle::exit` = `request_exit`(이벤트 루프 프록시) · 프록시 송신이 실패하면 **부른 스레드에서** `cleanup_before_exit` + `process::exit`(`app.rs:574-579`).
- **주 스레드 패닉 = tao 가 잡아 되던진다**(tao 0.35.3 `runner.rs:153-175` 가 `catch_unwind` 로 페이로드를 쥐고 · `event_loop.rs:267-272` 가 `DispatchMessageW` 뒤 `resume_unwind`) → 주 스레드가 `App::run` 밖으로 풀려 프로세스 종료 코드 101 · `RunEvent::Exit` 없음 · `cleanup_before_exit` 없음 → `Final` 이 안 쓰인다. 1판의 「FFI 경계에서 abort」 추정은 창 프로시저에 대해선 틀렸다 — **남은 abort 후보는 WebView2 COM 콜백**(웹뷰 IPC 로 부르는 동기 Tauri 명령 등 · 미검).
- 셸 버스 명령(데몬 → 셸)은 연결 태스크 밖의 tokio 태스크에서 돈다(`daemon_client/inbound.rs` 헤더 · `TaskSpawner` `:59-70`).
- 기록기 스레드 `saver.rs:292`(`PanicNote` `:352-361` — 문서 `:350-351` 「디버그 · 시험 빌드에서만 닿는다」) 외 셸 스레드 · 태스크는 1판 목록 그대로(트레이 관찰자 `tray/mod.rs:149` · `output_router.rs:405` · `:430` · `output_channel.rs:131` · `daemon_client` 연결 태스크 · `state/boot_plugin.rs:1242` · `:1280` · `daemon_client/refusal.rs:240`). 셸의 운영 `catch_unwind` = 0.

### 2-8. abort 전제 기록

- 코드 — 넓힌 정규식으로 **57줄 · 30파일**:
  `rg -n 'panic\s*=\s*"?abort|panic=abort|abort 빌드|abort 아래|릴리스는 \`panic|릴리즈 프로필의 한계|release=abort|abort 프로필|abort 라|abort라|릴리스 빌드는 abort|release 는 abort|릴리스.*abort|릴리즈.*abort' crates src-tauri/src -g '*.rs'`.
  1판 정규식(앞 여섯 갈래 — 50줄)이 놓친 일곱 = `command/src/route.rs:114` · `command/src/link.rs:166` · `daemon/src/bin/saturation_pilot.rs:193` · `daemon/src/usage_service/schedule.rs:91` · `usage_service/mod.rs:322` · `:450`(같은 파일 `:557` 은 1판 정규식이 잡는다) · `daemon/src/messaging_host.rs:769`(**런타임 로그 문장**). 두 부류 — (a) 「릴리스에서 도달 불가」라는 **사실 주장** · (b) 「abort 라 패닉하지 않게 짠다」는 **규칙의 사유**(규칙은 산다 · 사유만 바뀐다).
- ADR 7 개 = 0142 · 0199 · 0226 · 0233 · 0262 · 0275 · 0281 · TRD S20 §4 ⑨ · `.claude/skill-bindings/review.md:20`(정본 포인터가 S20 ⑨) · 날짜 박힌 TRD 다섯(`S21-usage-limit-slot` · `S21-storage` · `trd-1-1-base-helpers` · `trd-resume-after-first-turn` · `trd-t40`). CLAUDE.md 에는 abort 언급이 없다.

### 2-9. 입력 자료 · 1판과 어긋난 사실

1. **운영 가둠 자리 목록** — `daemon/src/usage_service/mod.rs:726` 은 시험 대역(`#[cfg(test)] mod tests` `:611` 안)이다. 셸 `state/restore.rs:1272` · `state/boot_plugin.rs:631` · `command/src/link.rs:136` · `:154` 도 시험 안이다(ADR-0290 영향 · 범위 조사가 운영으로 적었다).
2. **연결 태스크 하나 = 1층 입구** — 요청 처리는 그 태스크 밖(net 하위 태스크 넷 · 떼어 낸 명령 태스크 · blocking · rmcp 요청 태스크)에서 돈다(§2-4).
3. **`manager.rs` 줄** — ADR-0288 · 0290 의 `:2943` · 범위 조사의 `:924` · `:3153` · `:3186` 은 낡았다(지금 §2-5).
4. **「에이전트당 스레드」 목록** — `session_tracker.rs:153` 과 `usage/process.rs:254` 는 전역이다.
5. **「오염 → 패닉 → 훅 종료」** — 운영 락 대부분이 말없이 되찾고 표시 안의 오염 패닉은 가둬진다(§2-6) → 그대로는 어떤 오염 정책도 서지 않는다.
6. **1판 자신의 오류(리뷰 · 2판 실측):** PTY 감시 「결말이 설계됐다」는 자연 종료에만 맞다(§2-3) · flush 일꾼 · 명령 자리 수거기는 스레드가 아니라 tokio 태스크다 · `join_pump` 는 핸들이 아니라 수신자를 꺼낸다 · 셸 주 스레드 패닉은 abort 가 아니라 tao 되던지기다 · 1판 §4-10 첫 항목의 「ADR-0288 결정 1 이 받아들인 것」은 오귀속이다(결정 3 이 공유 상태 패닉 = 종료라 적는다 — §4-11).

---

## 3. 결정 거리

### 3-1. 사용자 결정 — 묻는 순서대로

> 각 묶음은 한 번에 하나씩 묻는다. 묶음 안의 선택지는 「사용자가 겪는 것」으로 적었다. 권고는 이 TRD 의 안이지 결정이 아니다.

#### (가) 락 오염 정책 + 2층 경계 — 「어떤 버그가 에이전트 하나로 끝나고 어떤 버그가 데몬 재시작이 되나」

**왜 하나로 묶나:** 2층(에이전트 하나) 입구가 패닉을 가둬도, 그 패닉이 쥐고 있던 락을 정리 코드가 다시 만질 때 「오염 = 종료」 규칙이 데몬을 끈다. 그래서 오염 정책이 2층 경계의 실제 폭을 정한다. 리뷰가 잰 근거: codex 통로는 상태를 거의 늘 락 안에서 다루고(되찾기 42곳) 리더가 되감기 중 그 락을 다시 잡는다(`ReaderExit::drop` `:2918`) — **「모든 오염 = 종료」면 codex 에이전트의 거의 모든 패닉이 데몬 재시작**이 된다. 펌프가 `emit` 안(`replay` 락)에서 패닉하면 `finish` 의 `.expect("replay poisoned")`(`output_core.rs:692`)가 곧바로 올린다. 순서 락(`input_order`)은 요청 하나가 쥔 채 죽으면 몇 분 뒤 **다른 사람의 입력**이 데몬을 끈다.

**오염 축:**

| 안 | 규칙 | 사용자가 겪는 것 | 비용 |
|---|---|---|---|
| P1 | 어떤 락이든 오염 = 데몬 정상 종료 | 에이전트 · 요청 하나로 끝나는 것은 **락을 하나도 안 쥔 채 난 패닉뿐**. codex 에이전트 버그 대부분 · 순서 락을 쥔 요청 패닉(다음 입력 때)도 재시작 | 작음 — 되찾기 자리가 관찰자를 부른다(종료 경로만 따로 — 메인 M14) |
| P1o | P1 + 데이터 없는 순서 락(`Mutex<()>` — 운영 셋 `session.rs:74` · codex `:1097` · `manager.rs:694`)은 오염을 무시 | P1 에서 「순서 락 함정」(몇 분 뒤 남의 입력이 데몬을 끈다)만 빠진다. 대가 — 순서 락이 지키던 여러 걸음(래치 commit → encode → send → emit)이 끊긴 것을 알릴 신호가 사라진다 | 작음 — 타입으로 묶인 함수 하나(`Mutex<()>` 만 받는다) · 락 셋의 호출 자리만 바꾼다 |
| **P2** | 공유 락(명부 · 프로필 · 제어 레지스트리 · 메시징 표 · 사용량 · 추적기) 오염 = 정상 종료 · **한 에이전트 화신 또는 한 연결이 소유한 락**(세션 · 출력 코어 · 통로 · codex 상태 · leftover · 래치 · 입력 큐 · 순서 락 · 연결 사용량 출구 · 연결 구독)의 오염 = 그 범위 안에서 되찾고 계속 | 한 에이전트의 버그 = 그 에이전트만(펌프 → `Failed` · 라이터 → 입력 거절 · 연결 → 오류). **공유 락을 쥔 채 났을 때만** 재시작. 대가 — 요청이 한 에이전트의 상태를 반쯤 바꾸다 죽으면 **그 에이전트는 계속 돈다**(이상하게 굴 수 있다 — 끄고 다시 켜면 된다 · 다른 에이전트와 저장된 공유 상태는 안전) | 중간 — 되찾기 함수 두 갈래(관찰 · 범위) · 범위 쪽으로 옮길 자리 ≈ 되찾기 60 + `.expect` 40(대부분 파일 단위 기계 치환 — codex 통로 42 · `output_core.rs` 27 · `pty.rs` 6 …). 분류 안 된 자리 = 관찰(종료 쪽 — 안전한 쪽) |
| (P3) | 「공유 락을 쥔 채 났나」를 스레드 지역 카운터로 추적 | P2 와 같은 결과를 정밀하게 | 큼 — 공유 락 타입을 전부 바꾼다. 미룬다(트리거 = P2 분류 오류가 실제로 관측될 때) |
| — | 지금 그대로(오염을 말없이 되찾고 계속) | 조용한 생존 | ADR-0290 위반 — 안 올린다 |

**경계 축(2층 — 「에이전트 하나」로 가두는 스레드):**

| 안 | 가두는 것 | 표시 밖(→ 데몬 정상 종료) | 비용 |
|---|---|---|---|
| B1 | 결말이 **이미 설계된** 자리 — 펌프 셋 · 라이터 몸통(큐 닫기) · 첫 턴 세션 id 기록 · leftover 듣는 차례 · 일꾼 · 메시징 수신자 배달(`DeliveryDone` — U8a 감사 조건) | PTY 감시 · stderr 배수 · codex 라이터(기록 밖) · codex thread_lock · 전역 루프 | 표시 ≈ 8 · 새 코드 ≈ 0 |
| **B1′** | B1 + **PTY 감시**(되감기 가드 — 자식 끄기 · master 놓기 · 실패 사유 칸 → 펌프가 `Error("pty watcher panicked: …")` 로 끝낸다) + **stderr 배수**(줄마다 가두고 계속 비운다 — 파이프가 안 찬다) | codex 라이터 · thread_lock · 전역 루프 | +40~60줄 · ★감시는 「몇 줄」이 아니다★ — 패닉하면 master 를 안 놓아(§2-3) 가드가 필요하다 |
| B2 | B1′ + codex 라이터 · thread_lock(같은 「통로가 자식을 끄고 사유 칸 → 펌프가 끝낸다」 길) | 전역 루프만 | +120~250줄 · 정리 호출자 셋째 금지(ADR-0127) 때문에 그 스레드가 직접 `finish` 를 못 부른다 |

**선택지(묶음):**

| 묶음 | 오염 · 경계 | 에이전트 하나로 끝나는 버그 | 데몬 재시작이 되는 버그 |
|---|---|---|---|
| 보수 | P1(또는 P1o) + B1 | 펌프 · 라이터 · 첫 턴 기록 · leftover · 메시지 배달에서 **락을 안 쥐고** 난 것 | 그 밖 전부 — codex 에이전트 버그 대부분 · PTY 감시 · stderr |
| **권고** | **P2 + B1′** | 한 에이전트의 펌프 · 라이터 · 감시 · stderr · 첫 턴 기록 · leftover · 메시지 배달 버그(그 에이전트 자기 락을 쥐었어도) | 공유 락을 쥔 채 난 것 · codex 라이터 · thread_lock · 전역 루프 · net · (나) 구간 |
| 넓게 | P2 + B2 | 권고 + codex 라이터 · thread_lock | 공유 락 · 전역 루프 · net · (나) 구간 |

- **권고 = P2 + B1′** — 까닭: ① 사용자 방향 「패닉 종료 최소화」(ADR-0288) ② ADR-0288 의 가름이 글자 그대로 「에이전트 하나 대 공유 상태」다 — P2 는 그 가름을 락 단위로 옮긴 것이다 ③ P1 은 2층을 codex 에서 사실상 이름뿐으로 만든다(위 근거). B1′ 은 감시 · stderr 의 결말을 가드 하나로 설계할 수 있어 싸고, codex 라이터는 「통로가 자식을 끈다」 길을 새로 짜야 해서 B2 로 미룬다(트리거 = 로그의 「표시 밖 패닉」 줄에 codex 라이터가 실제로 나올 때).
- **P2 의 정직한 대가** — 「그 에이전트는 계속 돈다」는 ADR-0288 결정 2(「그 에이전트만 **정리**」)보다 약하다. 정리하려면 B2 식 길이 요청 경로에도 필요하다. 채택하면 ADR-0292 가 이 차이를 적는다(§8-1).
- **묶임** — 이 답이 U2 · U4 의 크기를 정하고(P2 = 기계 치환 ≈ 100곳 · B2 = U4b 생김), (마)의 2층 표면이 B1′ · B2 에서 늘어난다. (나) · (다) · (라)의 선택지는 바뀌지 않는다.

#### (나) 여러 걸음 공유 변경 구간 — 「에이전트 시작 · 수거 · kill · 연결 붙이기 · 떼기 도중 버그」

**무엇이 문제인가:** 표시(1 · 2층) 안에서 **공유 상태를 여러 걸음에 걸쳐 바꾸는** 구간이 있다. 가운데서 패닉하면 입구가 가둬 요청은 실패로 끝나지만 공유 상태는 반쯤 바뀐 채 남는다 — ADR-0288 결정 3(공유 상태 패닉 = 정상 종료)이 막으려던 바로 그것이고, 락을 안 쥐었으면 (가)의 어떤 정책도 못 잡는다. 실측한 구간:

| 구간 | 걸음 | 가운데서 패닉하면 |
|---|---|---|
| 활성화 등록 창 `manager.rs:2050-2162`(1층 활성화 blocking 안 — `connection_core.rs:1366` · `:1760`) | `OutputCore::new` · seed · register → 명부 insert `:2145-2148` → `profiles.update_with` `:2160` → `start_pump` `:2162` → `transport.start`(감시 `.expect` `pty.rs:265` · 펌프 맨 `thread::spawn` `:269`) | **좀비 세션** — 명부엔 있고 펌프는 없다: 끝나지 않고(ADR-0005) 수거되지 않으며(ADR-0019) `kill_agent` 의 `join_pump` 는 수신자가 없어 곧바로 돌아온다 |
| reaper 메시지 `reap_one`(`reaper.rs:48`) | 명부 remove `:56` → `control.revoke` `:76` → 프로필 처분 `:81` → `agent_list_updated` `:86` | 명부에서 빠졌는데 화면 목록엔 남는 줄 · 처분 안 실린 프로필 |
| kill 앞 줄 `kill_agent`(요청에서 부를 때) | `begin_retire` → `control.revoke` → `set_intent` → `enter_exiting` → `session.kill` → `tracker.unwatch`(§2-5) | 자격은 걷혔는데 에이전트는 산다 |
| 연결 수명 `on_connect`(`agent_conn.rs:282` — 사용량 출구 부착 `:307`) · `on_disconnect`(`:450-` — 구독 해제 `:498` · 뷰포트 소유 해제 `:515` → net `unregister`) | 연결을 공유 표에 붙이고 떼는 걸음 | 반쯤 붙거나 떨어진 연결 — U3 가 공유 · 연결 전용 걸음을 가른다(후보) |

| 안 | 사용자가 겪는 것 | 비용 |
|---|---|---|
| **E 올림(권고)** | 위 구간의 버그 = **데몬 정상 종료**(에이전트 전부 끝남 · 활성화로 이어받기). 그 요청 자체는 오류 답을 받는다 | 작음 — 구간을 `escalating(…)`(깊이 0)으로 감싼다 · 구간당 ≈ 5~10줄 + 시험 |
| R 되돌림 | 그 작업 하나만 실패(시작 실패 · 수거 재시도 등) · 데몬 계속 | 큼 — 걸음마다 실패 가능하게 + 스코프 가드로 되돌림(명부 remove · 통로 shutdown · 프로필 두 번째 쓰기 · reaper 재시도) ≈ 150~250줄. 되돌림 코드가 되감기 도중 다시 패닉하면 abort |
| (그대로 가둠) | 좀비 세션 · 낡은 목록 줄이 조용히 남는다 | ADR-0288 결정 3 과 어긋난다 — 권고하지 않는다 |

- **권고 E** — 이 구간들은 「공유 상태를 바꾸다 난 패닉」의 정의 그대로라 ADR-0288 결정 3 이 이미 답을 줬다. R 은 실제 패닉이 관측된 구간부터 하나씩(트리거 = 로그의 사유 줄에 그 구간 위치가 찍힐 때).
- **둘 다에 붙는 것(메인 결정 M15):** 활성화 창에서 패닉 · 오류로 빠질 때 아직 펌프에 안 넘긴 통로를 drop 하지 않고 `transport.shutdown()`(kill 2동사 순서 — ADR-0001)을 부르는 스코프 가드. 지금은 drop 만 한다(통로에 `Drop` 없음 — Windows 에선 `GroupOwner` drop 이 Job 을 닫아 트리를 끄지만(`platform group/windows.rs:601` · `:153`) master 를 무리보다 먼저 닫는 순서는 ADR-0001 과 다르다).
- **서명 받을 잔여:** 이 목록은 찾은 구간뿐이다 — 표시 안에서 공유 상태를 락 밖으로 여러 걸음 바꾸는 **못 찾은** 구간은 반쯤 바뀐 상태로 산다(§4-11 첫 항목). E · R 어느 쪽이든 이 잔여를 받아들이는 것이 이 묶음 답의 일부다.

#### (다) 종료를 당긴 뒤 새 일 — 「꼬리가 도는 동안 들어온 명령」

**무엇이 문제인가:** 깃발이 서면 accept 루프는 새 연결을 안 받지만, **이미 붙은 WS 연결 · MCP HTTP 는 꼬리가 끝날 때까지 계속 처리된다** — `shutdown_all`(에이전트마다 `join_pump` 5초 · 병렬) → flush(5초 띠 둘) → MCP 종료(시한 없음 — §2-5). 그 몇 초~수십 초 동안 들어온 명령은 깨졌을 수 있는 메모리에서 `agents.json` · 프리셋을 쓸 수 있다. ADR-0288 이 종료를 고른 까닭(「반쯤 바뀐 메모리 상태가 저장으로 디스크에 실리면 멀쩡한 `agents.json` 까지 덮인다」)이 이 창에서 살아난다. ADR-0290 이 「저장 멈춤도 필요 없다 — 프로세스가 끝나므로」라 적은 전제는 이 창을 몰랐다.

| 안 | 사용자가 겪는 것 | 비용 |
|---|---|---|
| 받아들임 | 그 창의 명령은 평소처럼 처리 · 저장된다 | 0 |
| **새 일 거절(권고)** | 깃발이 선 뒤 들어온 새 명령 · 조회 · 도구 호출은 오류 답(「daemon is shutting down after an internal error」 — 가안 · 영어 사유 규칙) · 이미 돌던 것은 끝까지 간다 | 작음 — 1층 도우미 입구에서 깃발 확인 ≈ 15~20줄 + 시험 1. ADR-0288 「전용 장치 없음」과의 긴장 — 패닉 종료에만 쓰는 확인 한 줄이다 |

- **권고 거절** — 종료의 존재 이유를 지키는 가장 싼 길이다. StopDaemon 경로는 건드리지 않는다(그 경로는 `shutdown_all` 을 먼저 끝내고 신호를 보낸다 — 패닉 깃발만 본다).

#### (라) 셸 패닉

셸은 「같은 훅」이다(사용자 결정 2026-10-10). 남은 것은 훅이 당긴 뒤 셸이 **무엇을** 하고, 셸 안에 「요청 하나」 가둠을 둘지다.

| 안 | 동작 | 사용자가 겪는 것 | 비용 |
|---|---|---|---|
| **S1a(권고)** | 셸만 끈다 · 화면 상태 `Final` 은 안 쓴다 · **셸 안 가둠 없음** | 셸의 어떤 패닉이든(데몬이 보낸 **버스 명령 처리 중 패닉 포함** — `INTERNAL` 답 대신 창이 닫힌다) 창이 닫힌다 · 데몬 · 에이전트는 그대로 · 다음 셸 실행 = 비정상 종료 복원 물음(`state.crash.json`) | 작음 |
| S1a′ | S1a + 셸 버스 명령 처리를 1층으로 가둔다(`TaskSpawner` 태스크를 `Contained` 로) | 버스 명령 패닉 = `INTERNAL` 답 · 창 유지. 그 밖은 S1a 와 같다. 대가 — 그 명령이 화면 모델을 반쯤 바꾼 채 셸이 계속 돌고, 다음 정상 종료가 그 상태를 `Final` 로 싣는다 | 작음(+20줄 · 셸 컴파일 가드) |
| S1b | S1a 인데 `Final` 을 쓴다 | 다음 실행이 말없이 복원 — 패닉 순간의 화면이 「정상」으로 저장될 수 있다. 주 스레드 패닉은 `Exit` 사건이 없어 어차피 못 쓴다(§2-7) — 스레드에 따라 결말이 갈린다 | 작음 |
| (미룸) | 상자 뒤 종료 · 자동 재기동 | — | ADR-0288 이 복원 시스템과 묶어 미룬 「셸 사유 고지」 무리라 지금 안 올린다 |
| (권고 안 함) | 데몬도 끈다(`quit_app` 그대로) | 에이전트 전부 끝남 | 「패닉 종료 최소화」와 정면으로 어긋난다 |

- **권고 S1a** — 셸 패닉은 셸 메모리(화면 모델)를 믿을 수 없게 만든다. 셸은 상태가 사실상 화면 모델 하나라 요청과 공유 상태가 갈리지 않고, 셸 재시작이 싸다(데몬 · 에이전트가 산다). 어느 스레드에서 났든 다음 실행이 같은 물음을 낸다.

#### (마) 1 · 2층 고지 모양

| 층 | 안 | 사람 · LLM 이 보는 것 |
|---|---|---|
| 1 — 버스 명령 | (정해져 있다 — ADR-0159) | `INTERNAL` 「command handler panicked: <패닉 문장>」 · 전달 중이면 `OUTCOME_UNKNOWN` |
| 1 — WS 옛 명령 · 제어 HTTP · MCP 도구 | **N1a 오류 답 · 연결 유지(권고)** | WS = 그 `request_id` 를 실은 `Error`(「internal error: <패닉 문장>」 — 가안 · 영어 사유) · HTTP = 500 + 같은 문장 · MCP = 도구 오류 결과. 연결은 계속 |
| | N1b 연결을 닫는다 | 답 없이 끊김 → 클라이언트는 마감까지 기다리거나 재연결 · 같은 연결의 다른 진행 중 명령도 잃는다 |
| 2 — 펌프 · 감시 · 라이터 등 | **N2a 지금 표면 그대로(권고)** | 펌프 · 감시 = 그 에이전트 `Failed`(사유 「pump panicked: …」 · 「pty watcher panicked: …」) · 라이터 = 입력 거절 사유 · stderr 줄 = 그 줄만 버려짐(로그) |
| | N2b 띠 알림 더함 | 위 + 띠(ADR-0180 자격) 「에이전트 X 가 내부 오류로 멈췄다」 — 새 고지 장치(미룬 장치 무리) |
| 3 — 데몬 정상 종료 | (결정 아님 — 미룬 장치) | 지금 「데몬 꺼짐」과 같다(트레이 회색 · 연결 띠) · 사유 안내 없음 · (다)=거절이면 꼬리 동안 새 명령이 오류 답 |

- **권고 N1a · N2a** — 새 화면 장치가 없다. N1a 는 버스 명령이 이미 하는 일(값으로 답한다)을 옛 WS 명령 · HTTP · MCP 에 맞춘 것이다.
- ★라이터 패닉의 결말은 「에이전트 하나 정리」가 아니라 「그 에이전트의 입력만 죽음」이다★ — 지금 코드가 고른 결말(`input_queue.rs:273-279`)이고 (가) 어느 안도 바꾸지 않는다.

### 3-2. 메인 결정(관행) — 권고 기본값

> 번호는 1판을 잇는다 — 1판 M2(락 오염 정책)는 사용자 결정 (가)로 올라가 비었고, M14~M18 은 2판에서 새로 섰다.

| id | 거리 | 기본값(권고) · 까닭 |
|---|---|---|
| **M1** | 표시 모양 · 집 | **base 새 입주자 `panic`(가안)** — 스레드 지역 깊이(`thread_local!` + `const { Cell::new(0) }` · 읽기는 `try_with` — TLS 가 이미 해체된 스레드에서 훅이 돌면 「표시 밖」으로 읽는다) + `contain` · `Contained` · `escalating` · `payload_message` · `is_poison_panic` · 종료 걸쇠. ★**정책 함수(`verdict` — 「오염이면 표시와 무관하게 올린다」)는 base 에 두지 않는다**★ — 제품 정책이라 ADR-0269 입주 조건 ②(도메인 지식 0)와 긴장한다. base 는 원시만, 조립(정책)은 데몬 · 셸이 한다(각 ≈ 15줄 · 사본 둘은 각자 시험). 집 대안 — platform(OS 의존이 아니라 ADR-0266 결정 2 위반) · command(버스 도구 정체성 밖) · crate 마다 사본(입주 조건 ① 이 막으려는 것) |
| **M3** | 훅 신호 · 깨우기 | **깃발 = `AtomicBool` 하나에 `store(true, SeqCst)` — unpark 없음.** 깨우기 = 폴링: 데몬은 accept 루프 `select!` 에 `interval`(가안 200ms) 팔 하나 — 깃발이면 `shutdown_tx.send(true)`(다른 구독자용) + 루프 탈출 · 셸은 깨우기 스레드가 `park_timeout(200ms)` 고리로 깃발을 본다. 까닭: 1판의 「깃발 쓰기 → 대기자 읽기 / 대기자 등록 → 깃발 읽기」는 SeqCst 울타리 없이는 store-buffering 으로 깨움을 잃는다 · `Thread::unpark` 는 OS 호출이라 훅 · 관찰자(임의의 락을 쥔 채 돈다)에서 부르면 leftover 잎 락 불변식(OS 호출 금지 — CLAUDE.md 핵심 불변식)을 깬다 · 깨우기 스레드 기동 실패 · 둘째 대기 · 스레드 죽음 = 「당겼는데 안 끝남」. 거부: watch 를 훅이 직접 `send`(내부 RwLock — 패닉한 스레드가 `watch::Ref` 를 쥐었으면 자기 교착) · tokio `Notify`(셸에 못 씀) |
| **M4** | 로그 재진입 · 사유 기록 | **L1** — `FileSink::make_writer`(`logging/mod.rs:127` · 잠금 `:130` · 쥔 가드 `FileSinkWriter::Live` `:119`)가 스레드 지역 「파일 쓰는 중」 표지(const `Cell` · `try_with`)를 보고 그 스레드의 재진입 쓰기를 버린다. ★1판의 「잃는 것 = 쓰기 도중 패닉한 그 줄 하나」는 틀렸다 — 잃는 것은 **재진입한 훅 자신의 보고 줄**이다★ → 그래서 **사유를 따로 남긴다**: 훅은 올리는 첫 패닉의 「스레드 · 위치 · 문장」을 걸쇠의 `OnceLock<String>` 에 **깃발보다 먼저** 적고, 데몬 꼬리 · 셸 깨우기가 그것을 로그한다(에이전트 0 이면 꼬리가 빨라 패닉한 스레드가 로그를 쓰기 전에 `exit` 할 수 있다). 오염 관찰자는 `#[track_caller]` 로 받은 `&'static Location` 을 `AtomicPtr` 에 compare_exchange(첫 것만 · 할당 · 락 · 로그 없음)로 적는다. tracing `EnvFilter` 도 내부 `RwLock` 둘을 잡는다(`tracing-subscriber 0.3.23 filter/env/mod.rs:203-204` · 오염은 `try_lock!` 이 삼킨다) — 깃발을 로그보다 먼저 세우는 순서가 그 자리의 교착도 덮는다 |
| **M5** | net | **N1 — net 은 손대지 않는다.** 1층 표시는 데몬 처리기 몸통에만 세우고(`on_connect` · `on_text` · `on_binary` · `on_inbound_saturated` · `on_disconnect`), net 의 하위 태스크 넷 · `handle_connection` · 연결 태스크(`lib.rs:395`)는 **표시 밖(3층 → 정상 종료)**. 까닭: 1판 N3(하위 태스크 포장)은 패닉을 「태스크 끝남」으로 바꾸는데 정리 select 가 dispatch · write 만 기다려(`ws.rs:541-554`) bypass 가 죽으면 연결이 반쯤 산다 · `handle_connection` 몸통 패닉은 하위 태스크를 떼어 내고 `on_disconnect` · `unregister` 를 건너뛴다(§2-4) — 연결 태스크에 표시를 세우면 그 반쪽 상태를 가둬 버린다. 대안(감독 닫기 — 포장이 패닉을 따로 알리고 감독이 그 연결을 닫음 + abort-on-drop 핸들 + select 가 넷 다 기다림)은 net 에 새 구조가 필요하다 — net 자기 루프는 작고 안정된 공유 기반 코드라 그 버그를 재시작으로 둔다. **대가: ADR-0288 결정 1 의 「연결 하나」가 「데몬 처리기 몸통」으로 좁아지고 ADR-0290 영향의 「연결 태스크 표시」를 따르지 않는다 → ADR-0292 가 Amends 로 적는다(§8-1).** net 의존 상한 · 심볼 · 게이트 무변경 |
| **M6** | 메인 future · 부팅 | **둘 다 잡는다.** ① 데몬 `main.rs` 가 `run()` future 전체를 `futures_util::FutureExt::catch_unwind` 로 감싸 패닉이면 사유 로그 → **async main 안에서** `process::exit(101)` — 부팅 패닉(훅은 섰지만 watch · 꼬리가 아직 없다)과 꼬리 자체의 패닉을 받는다. 지금 abort 는 즉사하지만 unwind 에서 이것이 없으면 `run()` 이 풀려 런타임 drop 이 blocking 태스크를 **시한 없이** 기다린다(§2-1) — 단일 인스턴스 가드와 `daemon.json` 을 쥔 채 남는 데몬 ② accept 루프 future 를 같은 `catch_unwind` 로 감싸(표시 없이 — 훅이 이미 당겼다) 잡히면 꼬리로 간다(정상 종료 경로를 탄다). base 에 따로 타입을 두지 않는다(1판 `CaughtOnly` 걷음) |
| **M7** | 종료 코드 | **데몬 101 · 셸 101(상수 하나)** — 데몬은 꼬리 끝에서 깃발이면 `Err(101)` → `main.rs` 의 기존 `process::exit`. ★load-bearing★: `Ok` 로 돌아가면 런타임 drop 이 매달릴 수 있는 blocking 태스크(예: 오염으로 멎은 kill 스레드)를 기다린다 — 패닉 경로는 그 대기를 건너뛴다. 소비자는 없다(셸은 PID 로 데몬 생사를 본다) |
| **M8** | 실프로세스 시험 방아쇠 | `cfg(debug_assertions)` 아래에서만 읽는 환경변수(가안 `ENGRAM_DEBUG_PANIC=unmarked\|contained\|poison\|boot`) — 배포판 표면 0 |
| **M9** | 요청 경로 회귀망 | 요청 경로 파일(`agent_conn.rs` · `connection_core.rs` · `command_delivery.rs` · `control/mcp_server.rs`)의 시험 밖에 맨 `tokio::spawn(` · `spawn_blocking(` 이 없다는 소스 시험. **허용 목록**(3층 — 일부러 표시 밖) = StopDaemon 의 `spawn_blocking(shutdown_all)`(`connection_core.rs:1543` — 전역 전이라 1층이 아니다 · 그 안 패닉은 어차피 끄는 중) · 명령 자리 수거 태스크(`command_delivery.rs:527`) · MCP serve 태스크(`mcp_server.rs:1411`) |
| **M10** | 컴파일 가드 | 데몬 lib · **agent lib**(가두는 자리가 사는 곳 — 1판의 `main.rs` 만으로는 lib 만 abort 로 짓는 길을 못 막는다)에 `#[cfg(not(panic = "unwind"))] compile_error!(…)`. 셸은 (라)=S1a′ 일 때만. CLI 는 두지 않는다 |
| **M11** | 바이너리 크기 | U7 앞뒤로 세 exe 크기를 재 보고 · 문턱 없음(30% 를 넘으면 사용자에게 — 가안) |
| **M12** | T-52 · 동시 종료 | **따로 둔다** — 패닉 경로는 `send_stop` 을 지나지 않는다(실프로세스 시험은 `send_stop` 결과를 판정에 안 쓴다). 동시 경합 하나를 기록한다: StopDaemon 의 `shutdown_all` 과 패닉 꼬리의 `shutdown_all` 이 겹치면 둘째 `join_pump` 가 `None` 을 받아(§2-5) 꼬리가 먼저 끝나 `exit(101)` 할 수 있다 — StopDaemon 의 kill 스레드가 도는 중이라 그 답이 사라진다(T-52 와 같은 모양) · 남은 자식은 Job 이 끈다. T-52 항목에 한 줄 |
| **M13** | abort 전제 코드 · 주석 | 「패닉 말고 값으로」 규칙은 전부 유지 · **의미 재감사(U8a)를 프로필 전환(U7) 앞에** — leftover K7 막 둘(ADR-0262 — 「배포판 abort 에 기댄다」) · ADR-0142 `in_flight_targets`(`FlightSettle` 이 오염이면 말없이 건너뜀 · `Reservation` 은 오류 로그만) · `DeliveryDone` 2층 배치 · 래치 · codex 라이터 · `lock_for_cleanup` 짝(`command_roster.rs:401-403` · `command_delivery.rs:994-996` — 둘 다 `sync::lock`) · 런타임 로그 문장 `messaging_host.rs:769`. 「릴리스에서 도달 불가」 **사실 주장만** 고치는 문장 손질은 U7 뒤(U8b) |
| **M14** | 종료 경로 오염 내성 | §2-5 의 락 자리 전부를 되찾기로: 명부(`get_session` · `shutdown_all`) · 제어 레지스트리 `revoke`(데몬) · `enter_exiting`(코어 status) · 추적기 `unwatch` · 통로 `shutdown`(PTY 자식 · master · stdio 자식) · 감시의 master 락. 공유 쪽 = 관찰(이미 당겨졌으면 무동작) · 에이전트 전용 쪽 = (가)=P2 면 범위 · P1 이면 관찰. 어느 안이든 **종료 경로는 끝까지 간다**(ADR-0288 영향) |
| **M15** | 스폰 경로 스코프 가드 | 활성화 창(`manager.rs:2050-2162`)에서 통로를 펌프에 넘기기 전 패닉 · 오류로 빠지면 `transport.shutdown()` 을 부르는 가드(되감기 중에도 — 그래서 `shutdown` 의 락은 M14 로 되찾기여야 한다. 아니면 이중 패닉 abort). 두 번 불려도 되는지(명부에 이미 올라 `shutdown_all` 도 부르는 경우)는 U4 가 확인한다 |
| **M16** | `Contained` · axum · rmcp | 패닉한 · 취소된 안쪽 future 는 **깊이 가드 안에서** drop 한다(패닉 뒤 drop 은 `catch_unwind` 로 · 취소 drop 은 `Contained::drop` 에서 — 이미 되감는 중이면 std 가 중첩 패닉을 abort 하므로 그때의 `catch_unwind` 는 무력하다 · 받아들인다). axum 의 `Contained` 층은 **가장 바깥**(마지막 `.layer`)이어야 `bearer_auth` · 본문 크기 제한까지 덮는다. rmcp 도구 처리기는 그 층 밖이라(§2-4) **도구 몸통을 직접 감싼다**. hyper 가 `next.run` 뒤에 poll 하는 스트리밍 · SSE 본문은 표시 밖 → 올린다(§4-11) |
| **M17** | 관찰자 계약 | base `sync` 의 관찰 갈래는 오염을 보면 등록된 `fn(&'static Location<'static>)` 하나를 부른 뒤 되찾는다 · 등록 = `OnceLock` 한 번(조립부) · 관찰자는 **패닉 · 할당 · 락 · 로그 · 블로킹 금지**(되감기 중 `Drop` 안에서도 불린다) — 실물은 위치 기록(`AtomicPtr` CAS) + 깃발 세우기 둘뿐. `sync` 계약 2 개정 |
| **M18** | 셸 끝내기 절차 | 「`Final` 생략」은 **`is_pulled()` 로 판정**(훅이 동기로 세운다 — 1판의 「깨우기가 `wait()` 뒤 세우는 표지」는 사용자 끝내기가 먼저 줄 서면 어긋난다) · `RunEvent::Exit` 패닉 갈래 = 실행 잠금 놓기 → `handle.cleanup_before_exit()` → `process::exit(101)`(사용자 콜백이 정리보다 먼저 돌기 때문 — §2-7) · 훅이 **주 스레드**에서 올렸으면 깨우기는 `app.exit` 를 부르지 않는다(주 스레드는 tao 되던지기로 이미 풀려 나간다 — 동시에 부르면 `request_exit` 실패 갈래가 다른 스레드에서 창 · 트레이를 만진다) |

---

## 4. 설계 (권고안 기준 — (가)=P2+B1′ · (나)=E · (다)=거절 · (라)=S1a · (마)=N1a·N2a. 답이 다르면 해당 절만 바뀐다)

### 4-1. base `panic` 입주자 (가안)

```rust
// crates/engram-dashboard-base/src/panic.rs — 이름 · 모양 전부 가안. std 만 쓴다(새 의존 0). 정책 0.

/// 이 스레드가 지금 「가두는 중」인가. TLS 해체 뒤면 false(try_with).
pub fn is_contained() -> bool;

/// 동기 입구 — 깊이를 올린 채 f 를 돌리고 패닉을 값으로 접는다. 깊이는 되감기에도 내려간다(가드 drop).
pub fn contain<T>(f: impl FnOnce() -> T) -> Result<T, Caught>;

/// 비동기 입구 — poll 마다 깊이를 올리고 그 poll 의 패닉을 접는다. 패닉한 future 는 다시 poll 하지 않고
///   깊이 가드 안에서 drop 한다. 취소 drop 도 Drop 에서 깊이를 올린 채 한다(이미 되감는 중이면 그대로).
///   ★태스크는 스레드를 옮겨 다니므로 「태스크 하나에 한 번」이 아니라 「poll 마다」다★.
pub struct Contained<F>;                 // impl Future<Output = Result<F::Output, Caught>>

/// 표시 안에서 일부러 「표시 밖」을 만드는 구간 — 깊이를 0 으로 두고 f 를 돌린 뒤 되돌린다(되감기에도).
///   잡지 않는다 — 패닉은 그대로 바깥 입구까지 풀린다(훅은 표시 밖으로 본다).
pub fn escalating<T>(f: impl FnOnce() -> T) -> T;

pub struct Caught { /* 패닉 문장 */ }    // .message()

/// 패닉 값 → 문장(지금 사본 일곱 — 펌프 셋 · input_queue · leftover · reaper · 데몬 훅.
///   command `route.rs` 의 사본은 command 가 의존 0 이라 남는다).
pub fn payload_message(payload: &(dyn Any + Send)) -> String;

/// std 락 오염 패닉 문장인가(`.expect` / `.unwrap` 의 `PoisonError` Debug 문자열). 도메인 0 원시 — 정책 아님.
pub fn is_poison_panic(message: &str) -> bool;

/// 프로세스 종료 걸쇠 — 한 번 서면 안 내려간다. 할당 · 락 · OS 호출 없는 쪽은 pull · is_pulled · record_site.
pub struct Escalation;                   // AtomicBool + OnceLock<String> + AtomicPtr<Location<'static>>
impl Escalation {
    pub fn pull(&self);                                 // store(true, SeqCst)
    pub fn is_pulled(&self) -> bool;                    // load(SeqCst)
    pub fn record_reason(&self, f: impl FnOnce() -> String);   // 첫 것만(OnceLock) — 훅이 pull 앞에
    pub fn record_site(&self, at: &'static Location<'static>); // 첫 것만(CAS) — 관찰자
    pub fn reason(&self) -> Option<&str>;
    pub fn site(&self) -> Option<&'static Location<'static>>;
    pub fn wait_polling(&self, every: Duration);       // park_timeout 고리 — 셸 깨우기 스레드
}
pub fn escalation() -> &'static Escalation;
```

- **입주 조건(ADR-0269):** ① 지금 여러 곳에 사본(페이로드 문장 일곱) + 소비자 셋(agent · 데몬 · 셸) ② 도메인 지식 0 — 「오염이면 올린다」 같은 정책은 없다(M1) ③ 다른 입주자를 부르지 않는다 — `sync` 관찰자 · `logging` 재진입 가드와 잇는 것은 **base 밖**(데몬 · 셸 조립부).
- **base `sync` 개정(U1):** 관찰 갈래 = 지금 이름(`lock` · `read` · `write` · `wait_timeout`)에 `#[track_caller]` + 오염이면 관찰자 호출 뒤 되찾기 · 범위 갈래(가안 `lock_scoped` 등 — (가)=P2) = 지금처럼 말없이 · (가)=P1o 면 범위 갈래 대신 `lock_order(&Mutex<()>)` 하나(타입으로 순서 락에만 묶인다) · `set_poison_observer` 한 번. **기본값 = 관찰**(분류 안 된 자리는 종료 쪽 — 안전한 쪽). 계약 2 = 「로그 · 다른 락 · 밖 호출을 하지 않는다 — 단 등록된 관찰자 하나는 부른다(M17)」 · 계약 3 = unwind 아래 운영도 닿는다.
- **base `logging` 개정(U1 — M4 L1):** 스레드 지역 「파일 쓰는 중」 표지 — `FileSinkWriter::Live` 가 세우고 drop 이 내린다 · 섰으면 `make_writer` 가 `Void`. 주석 `:110-115` 「알려진 미수정」을 닫는다.

### 4-2. 층 경계 표 (권고안)

| 층 | 자리 | 표시 |
|---|---|---|
| 1 요청 | WS 처리기 다섯 몸통(`agent_conn.rs` `:282` · `:314` · `:375` · `:405` · `:450` — 수명 처리기 둘의 공유 걸음은 (나)) · 떼어 낸 명령 태스크(`:336`) · 활성화 blocking 둘(`connection_core.rs:1366` · `:1760` — 그 안 등록 창은 (나)) · 버스 배달 태스크(`:2106`) · 1단계 본문 blocking(`command_delivery.rs:1361`) · 제어 HTTP(가장 바깥 axum 층 + blocking 여섯) · MCP 도구 몸통 셋 | `Contained` / `contain` |
| 2 에이전트 하나 | 펌프 셋 · 라이터 몸통 · 첫 턴 세션 id 기록 · leftover 듣는 차례 · 일꾼 · **PTY 감시 · stderr 배수(B1′)** · 메시징 수신자 배달(`spawn_delivery` — U8a 감사 뒤) | `contain` |
| 3 공유 · 그 밖 | 전역 루프(reaper 고리 · sweep · 추적 · 사용량 · flush · 수거기) · 데몬 부팅 · 메인 future · net 자기 태스크 · 연결 태스크 · StopDaemon `shutdown_all` · codex 라이터 · thread_lock · 셸 전부((라)=S1a) · 써드파티 내부(rmcp 세션 일꾼 · hyper · 스트리밍 본문) | 없음 → 정상 종료 |
| (나) 구간 | 활성화 등록 창 · `reap_one` · 요청에서 부른 `kill_agent` 앞 줄 · 연결 수명 처리기(`on_connect` · `on_disconnect`)의 공유 걸음(U3 분류) | `escalating` — 표시 안이어도 표시 밖 |

reaper 고리 자체의 `catch_unwind`(`reaper.rs:208`)는 남긴다 — 종료가 끝날 때까지 고리를 살려 다른 에이전트의 수거 메시지를 계속 받는다(sweep 과 같은 까닭). 표시는 세우지 않는다.

### 4-3. 훅 (데몬 · 셸 공통 모양 — 조립부마다 한 벌)

```
hook(info):                                     // 패닉한 스레드에서 · 되감기 전에
  contained = base::panic::is_contained()       // TLS try_with
  msg       = payload_message(info.payload())
  escalate  = !contained || is_poison_panic(&msg)          // 정책 = 조립부(M1)
  if escalate:
      ESC.record_reason(|| format!("{thread} {location}: {msg}"))   // ① 첫 사유 — 깃발보다 먼저
      ESC.pull()                                            // ② SeqCst store 하나 — 락 · unpark 없음
      [셸] if 주 스레드: MAIN_ESCALATED.store(true)         //    깨우기가 app.exit 를 건너뛰게(M18)
  tracing::error!(thread, location, contained, "스레드 panic: {msg}")  // ③ 재진입이면 버려진다(M4 L1)
  prev(info)                                                // ④ 기본 훅(stderr)
```

- **오염 패닉은 표시와 무관하게 올린다** — (가)=P2 에서도 같다: 범위 락은 범위 갈래로 되찾아 패닉하지 않으므로, 이 규칙에 걸리는 것은 공유 락의 `.expect` 와 아직 분류 안 된 자리뿐이다(종료 쪽 — 안전한 쪽).
- **조립(`run()`):** 로깅 → 훅 설치(지금 `:458`) + `sync::set_poison_observer(|at| { ESC.record_site(at); ESC.pull() })`. 부팅 중에 이미 당겨졌으면 accept 루프의 interval 팔이 첫 틱에서 본다(watch 가 훅보다 늦게 생기는 것 — §2-5 — 을 폴링이 흡수한다).
- **이중 당김 · 종료 중 패닉** — `pull` 은 멱등이다. 꼬리가 도는 중의 패닉은 로그 · 사유 기록(이미 섰으면 무시)만 남는다.
- **시험** — 훅은 `run()` 에서만 선다(§2-1). 시험은 정책 함수(조립부의 순수 함수)와 지역 `Escalation` 인스턴스로 잰다.

### 4-4. 종료 (데몬)

- **깨우기 = accept 루프의 interval 팔**(M3) — 깃발이면 `shutdown_tx.send(true)` → 루프 탈출 → `sweeper.stop()` → 꼬리. 보내는 신호는 **`StopDaemon` 이 보내는 그 watch 의 `true`** 다(ADR-0288 결정 3 「기존 정상 종료 경로 · 같은 신호」) — 다른 구독자(명령 자리 수거 태스크 등)도 같은 신호로 끝난다.
- **꼬리 머리에서 사유를 로그**(깃발이면 「패닉으로 정상 종료 — 사유: … · 오염 감지 자리: …」) — 꼬리가 멎어도 사유는 남는다. 꼬리 끝에서 깃발이면 `Err(101)` → `main.rs` 의 `process::exit`(M7).
- **StopDaemon 과 다른 점** — StopDaemon 은 `shutdown_all` 을 먼저 돌리고 답한 뒤 보낸다. 패닉 경로는 답할 상대가 없으니 곧바로 보내고 `shutdown_all` 은 꼬리 ③ 이 돈다. `shutting_down` 을 먼저 세우므로(ADR-0019) reaper 는 프로필을 손대지 않는다 — 다음 부팅은 지금처럼(운영 데몬의 부팅 자동 복원은 꺼져 있다 — `lib.rs:781-787` · 2026-07-09 사용자 결정의 stopgap) 활성화로 이어받는다(첫 제출 뒤 영속된 세션 id — ADR-0226).
- **(다)=거절** — §4-6 의 1층 도우미 입구가 `is_pulled()` 면 새 일을 오류로 답한다.
- **메인 future · 부팅(M6)** — `main.rs`: `match AssertUnwindSafe(run()).catch_unwind().await { Ok(Ok(())) => {}, Ok(Err(c)) => exit(c), Err(_) => { 사유 로그; exit(101) } }`. accept 루프도 같은 꼴로 감싸 잡히면 꼬리로.

### 4-5. 락 오염 ((가)=P2)

1. **공유 락의 `.expect` · `.unwrap`** — 오염이면 둘째 패닉 → 훅이 문장으로 알아보고 표시와 무관하게 올린다. 그 패닉은 표시 안이면 그 요청 · 에이전트 실패로 끝나고 데몬은 곧 정상 종료한다.
2. **공유 락의 base `sync` 되찾기(관찰 갈래)** — 되찾아 계속 가되(종료 경로가 끝까지 가야 하므로) 관찰자가 위치를 적고 당긴다.
3. **에이전트 · 연결 소유 락** — 범위 갈래로 되찾고 계속. 옮길 자리(파일 단위): codex `transport.rs`(되찾기 42) · `input_queue.rs` 7 · `stdio.rs` · `pty.rs`(되찾기 · `.expect` 6) · `output_core.rs`(`.expect` 27 — `finish` 의 `replay` `:692` · `enter_exiting` `:803` · 진단 `:1052` 포함) · leftover(`GateCell` 감싸개 · 되감기 Drop) · 래치 · `session.rs` 순서 락 · `agent_conn.rs`(`ConnUsageOutlet` `:212-215` · `on_disconnect` 의 연결 전용 `.expect`). **공유로 남는 것**: 명부(`manager.rs`) · 프로필(`persistence/mod.rs` 7) · 제어 레지스트리 · 추적기 · 메시징 · 사용량 · `name_allocation`(공유 순서 락 — 데이터가 없으므로 P2 에서도 범위로 둘지 U4 가 정한다: 지키는 것이 이름 할당 순서뿐이라 오염 정보가 없다).
4. **종료 경로를 끝까지(M14)** — §2-5 의 락 자리 전부 되찾기. `thread::scope` 덕에 한 에이전트의 kill 패닉이 남의 kill 을 막지 않는 성질은 그대로 둔다.
5. **바꾸지 않는 것** — 위 밖의 공유 `.expect` 자리는 그대로다(1 이 덮는다).

### 4-6. 1층 입구 (데몬)

- **WS 처리기((마)=N1a):** `on_text` 는 명령을 해석한 뒤 처리를 `Contained` 로 감싸고, 잡히면 `command_request_id`(`agent_conn.rs:433` 이 이미 쓰는 것)로 `request_id` 를 실은 `Error` 를 낸 뒤 연결을 계속한다. `on_binary` · `on_inbound_saturated` 는 같은 꼴 · `on_connect` 는 신호가 `BoxFuture<()>`(`net/src/frame_port.rs:164-168`)라 오류로 못 돌려준다 — 잡히면 로그 + 그 연결 큐에 닫기를 넣는다(`on_text` 의 Close 갈래가 쓰는 길 — U3 가 확인) · `on_disconnect` 는 잡히면 로그 · 계속(net 의 `unregister` 가 이어서 돈다). 두 수명 처리기의 공유 걸음은 (나).
- **떼어 낸 명령 · 배달 태스크 · blocking:** 데몬 도우미 둘(가안 `spawn_contained` · `spawn_blocking_contained` — 새 작은 모듈 가안 `containment.rs`) — 잡히면 그 자리의 기존 `JoinError` 갈래와 같은 오류로 접는다(활성화 = `SpawnFailed` · 1단계 본문 = 기존 로그 갈래). 버스 1단계 본문은 그 안에서 `command::route` 가 다시 잡아 `INTERNAL` 로 답한다(표시는 바깥이 이미 세웠다). **(다)=거절** = 두 도우미와 WS 처리기 입구에서 `is_pulled()` 면 실행 없이 오류 답.
- **제어 HTTP:** axum `middleware::from_fn` 하나가 `next.run(req)` 를 `Contained` 로 감싸 잡히면 500 + 문장 · **마지막 `.layer`(가장 바깥)** · blocking 여섯은 `contain`. **MCP 도구 셋은 몸통을 직접 감싼다**(rmcp 요청 태스크 안 — §2-4) → 잡히면 도구 오류 결과.
- **net · 연결 태스크:** 손대지 않는다(M5).

### 4-7. 2층 입구 (에이전트) · 불변식

- 기존 자리의 `catch_unwind(AssertUnwindSafe(…))` 를 `contain(…)` 으로 바꾼다 — 잡은 뒤 결말 코드는 그대로다.
- **PTY 감시(B1′):** 감시 몸통을 `contain` 으로 감싸고 잡히면 「자연 종료 때 하는 일」을 한다 — 자식 끄기(`sync` 범위) · master 놓기 · 통로의 실패 사유 칸(가안)에 「pty watcher panicked: …」. 펌프는 EOF 로 `finish` 를 부르며 그 칸이 있으면 `Error(사유)` 로 끝낸다 — 종료 전이는 여전히 펌프 단독(ADR-0005)이고 새 정리 호출자가 아니다(ADR-0127).
- **stderr 배수(B1′):** 줄마다 `contain` · 잡히면 그 줄만 버리고 계속 비운다(파이프가 안 찬다). 진단 락은 (가)=P2 면 범위 갈래라 다음 줄이 다시 패닉하지 않는다.
- **finalize 1회(ADR-0005):** 펌프의 `finish` 는 지금처럼 `contain` **밖**에서 한 번 불린다.
- **정리 호출자 둘(ADR-0127):** 새 호출자를 만들지 않는다. B2 를 고르면 그 스레드는 「통로가 자식을 끄고 실패 사유 칸을 남긴다」까지만 하고 종료 전이는 펌프가 낸다.
- **kill 인과(ADR-0001):** 정상 종료는 `shutdown_all` → `kill_agent`(`begin_retire` → … → `session.kill`) 그대로다. §4-5 의 되찾기는 같은 줄에서 락 획득 방식만 바꾼다.
- **락 순서(ADR-0006) · leftover 잎 락:** 훅은 로그 싱크 말고 아무 락도 잡지 않고(그 싱크도 재진입 가드) OS 호출(unpark)도 하지 않는다. 관찰자는 원자값 둘뿐이다(M17).
- **등록 순서(ADR-0019):** 손대지 않는다 — (나)의 스코프 가드는 등록 순서를 바꾸지 않고 실패 갈래만 더한다.

### 4-8. 여러 걸음 공유 변경 구간 ((나)=E)

- 활성화 등록 창(명부 insert 앞 seed · register 부터 `start_pump` 까지) · `reap_one` 의 `:56-86` · 요청에서 부른 `kill_agent` 의 앞 줄 · 연결 수명 처리기의 공유 걸음(U3 가 분류)을 `escalating(…)` 으로 감싼다. 패닉하면 훅이 올리고, 패닉은 바깥 입구까지 풀려 그 요청은 기존 오류로 답한다.
- **스코프 가드(M15)** — 활성화 창에서 펌프에 넘기기 전 통로를 쥐고, 빠지면(패닉 · 오류) `transport.shutdown()`. 명부에 이미 올랐으면 `shutdown_all` 이 다시 끄므로 `shutdown` 이 두 번 불려도 되는지 U4 가 확인한다.
- 감시 기동 `.expect("spawn pty watcher thread")`(`pty.rs:265`) · 펌프 맨 `thread::spawn`(`:269`)은 버그가 아니라 자원 실패다 — 오류로 바꿔 `start` 를 실패 가능하게 하는 것은 R 쪽 일이라 E 에서는 그대로 둔다(그 패닉도 E 가 정상 종료로 올린다).

### 4-9. 셸 ((라)=S1a)

- 같은 훅 모양(§4-3)이고 셸은 표시를 세우지 않으므로 **모든 패닉이 당긴다.** 설치 = 셸 `run()` 머리(로깅 뒤) + `sync::set_poison_observer`.
- 깨우기 스레드는 setup 에서 `AppHandle` 을 받아 띄운다: `ESC.wait_polling(200ms)` → 사유 로그 → 주 스레드가 올렸으면 아무것도 안 함 · 아니면 `app.exit(101)`. setup 전에 당겨졌으면 첫 확인에서 끝낸다. 기동 실패 = setup 실패(시작 실패 갈래).
- `RunEvent::Exit`(`lib.rs:401`): `is_pulled()` 면 `Final` 쓰기는 건너뛰고 실행 잠금은 놓은 뒤 `handle.cleanup_before_exit()` → `process::exit(101)`(M18). `exit_session.shutdown()` 에 「`Final` 없이」 갈래가 하나 는다.
- **데몬은 건드리지 않는다** — `quit_app` 을 부르지 않는다.
- 주 스레드 패닉: 훅이 당기고 로그한 뒤 tao 가 되던져 `App::run` 밖으로 풀린다 → 종료 코드 101 · `Exit` 사건 없음 · `Final` 안 쓰임(S1a 와 같은 결말) · `cleanup_before_exit` 없음(트레이 아이콘이 남을 수 있다 — 미검).

### 4-10. 프로필 · 컴파일 가드

- `Cargo.toml` `[profile.release]` 의 `panic = "abort"` → `panic = "unwind"`(지우지 않고 명시 + `# ADR-0292` 주석 — 기본값에 기대면 다음 사람이 「빠졌다」로 읽고 abort 를 다시 넣는다).
- 데몬 lib · agent lib: `#[cfg(not(panic = "unwind"))] compile_error!("… ADR-0292 …");`(M10). dev 프로필은 원래 unwind 라 시험 · 개발 빌드는 그대로다.
- CLI 는 가드 없이 함께 unwind 가 된다 — 패닉 종료 코드가 fast-fail(`0xC0000409`)에서 101 로 바뀐다.

### 4-11. 남는 위험 (알고 받는 것 — ★표는 사용자 서명이 필요한 것)

- ★**못 찾은 여러 걸음 공유 변경**★ — 1 · 2층 코드가 락 밖에서 공유 상태를 반쯤 바꾸다 난 패닉은 가둬지고 그 상태는 남는다. 이것은 ADR-0288 결정 1 이 받아들인 것이 **아니다**(1판의 오귀속 — 결정 3 은 공유 상태 패닉 = 종료라 적는다). (나)가 찾은 구간만 올리고, 못 찾은 구간은 ADR-0288 결정 3 과의 틈으로 남는다.
- ★**(가)의 대가**★ — P1: 락을 쥔 2층 패닉 대부분이 재시작 · P2: 요청이 반쯤 바꾼 에이전트 상태로 그 에이전트가 계속 돈다.
- **정상 종료가 멎는 경우** — 종료 시간 제한은 미룬 장치다. 패닉 경로는 런타임 drop 대기를 `exit(101)` 로 건너뛰지만(M7) 꼬리 안의 기다림은 남는다: `McpServerHandle::shutdown` 의 시한 없는 await(axum graceful 이 붙은 연결을 기다린다 — rmcp 는 자식 토큰으로 세션을 닫는다(`mcp_server.rs:1332`)고 보이나 미검) · `tracker.stop()` join(실제로는 짧다). flush 는 5초 띠 둘로 유한하다.
- **팬아웃 seq 구멍** — 출력 코어 `fan_out`(`output_core.rs:351-380`)은 구독자 사본을 돌며 `send` 한다 — 요청 스레드에서 가둔 패닉이 그 고리 가운데서 나면 남은 구독자는 그 seq 를 못 받는다 → 그 뷰는 붙듦 상한까지 멈춘 뒤 전량 replay 로 회복(「replay→live」 불변식의 기존 회복 길).
- **reaper 의 낡은 목록 줄** — (나)=R 이거나 reaper 를 그대로 가두면 `sessions.remove` 뒤 `agent_list_updated` 전 패닉이 낡은 줄을 남긴다. (나)=E 면 정상 종료로 간다.
- **동시 종료 경합** — StopDaemon 과 패닉 꼬리가 겹치면 StopDaemon 답이 사라질 수 있다(M12) · 남은 자식은 Job 이 끈다.
- **net 자기 루프 · 연결 수명 · 써드파티 내부 패닉 = 재시작**(M5) · **스트리밍 · SSE 본문**(hyper 가 `next.run` 뒤에 poll) = 표시 밖 → 재시작.
- **abort 갈래** — 이중 패닉 · Drop 안 패닉(되감는 중) · FFI 건넘 · WebView2 COM 콜백 패닉(셸 — 미검)은 지금처럼 즉사(Job 이 에이전트 정리). `Contained::drop` 의 `catch_unwind` 는 되감는 중엔 무력하다(M16). command `route.rs` 주석의 「핸들러 · 링크 future 는 drop 에서 패닉하면 안 된다」가 계속 유효하다.
- **부팅 중 결정적 패닉의 재기동 고리** — 지금(abort)과 같다(이제 `exit(101)` 로 끝날 뿐).
- **표시 누락** — 새 spawn 이 표시 없이 들어오면 그 자리 패닉은 재시작이다(안전한 쪽). M9 가 요청 경로 파일에서만 막는다.
- **훅의 할당 · tracing 락** — 사유 문장은 훅에서 할당한다(std 기본 훅도 한다). tracing 내부(`EnvFilter` `RwLock`)를 쥔 채 난 패닉이면 훅의 로그가 교착할 수 있다 — 깃발은 이미 섰으므로 다른 스레드의 accept 루프가 종료를 탄다(로그만 잃는다 · 미검).
- **셸 주 스레드 패닉 뒤 트레이 아이콘** — `cleanup_before_exit` 가 안 돌아 남을 수 있다(마우스를 올리면 사라지는 Windows 동작 — 추론 · 미검).

---

## 5. 사용자가 보는 변화 (권고안 기준 — 괄호는 (가) 다른 안)

| # | 상황 | 지금(배포판) | 바뀐 뒤 |
|---|---|---|---|
| 1 | 버스 명령 · WS 명령 · 제어 HTTP · MCP 도구 처리 중 패닉 | 데몬 즉사 → 에이전트 전부 죽음 | 그 요청만 오류 답(N1a) · 데몬 · 에이전트 계속. **단 공유 락을 쥔 채였고 누가 그 락을 다시 만지면 그때 정상 종료**(P1: 어떤 락이든 · P1o: 순서 락 빼고 어떤 락이든) |
| 2 | 에이전트 하나의 코드 패닉(펌프 · 감시 · stderr · 라이터 · 첫 턴 기록 · leftover · 메시지 배달) | 데몬 즉사 | 그 에이전트 `Failed` · 라이터면 입력만 거절 · 나머지 계속. **공유 락을 쥔 채였으면 정상 종료**(P1: 그 에이전트 자기 락이어도 종료 · B1: 감시 · stderr 는 종료) |
| 3 | 에이전트 시작 · 수거 · kill · 연결 붙이기 · 떼기 도중 패닉((나)) | 데몬 즉사 | 데몬 정상 종료(그 요청은 오류 답) |
| 4 | 공유 상태 · 표시 밖 · 공유 락 오염 · codex 라이터 · net 루프 패닉 | 데몬 즉사 | 데몬 정상 종료(에이전트 정리 · 화면 「데몬 꺼짐」 · 꼬리 동안 새 명령은 오류 답(다) · 종료 코드 101) — 다음 데몬 부팅은 지금처럼 자동 복원 안 함 · 활성화로 이어받기 |
| 5 | 데몬 부팅 중 패닉 | 즉사 | 곧바로 종료(코드 101 — 꼬리 없음) |
| 6 | 셸 패닉(버스 명령 처리 포함) | 셸 즉사 · 데몬 산다 | 셸만 닫힘 · 데몬 산다 · 다음 실행 = 비정상 종료 복원 물음 |
| 7 | 이중 패닉 · FFI · WebView2 콜백 | 즉사 | 같다 |
| 8 | 개발(debug) 빌드 | 표시 없는 패닉 = 그 스레드만 조용히 죽음 | 데몬 · 셸이 정상 종료한다(개발자가 바로 본다) |
| 9 | 배포 exe 크기 | — | 커진다(미측정 — M11) |
| 10 | CLI 패닉 | fast-fail 코드 | 종료 코드 101 |

---

## 6. 게이트 · 의존

- **새 의존 0** — base `panic` 은 std 만 · 데몬은 이미 axum · tokio · futures-util 을 진다.
- **base 게이트:** `use tauri` 0 · 의존 상한(자기 자신 1줄) 그대로 · **입주자 무참조 ③ 의 알파벳에 `panic`**(정본 `base/src/lib.rs:54`) — 사본 여섯(`CLAUDE.md:255` · `.github/workflows/ci.yml:668` · `.claude/skill-bindings/qa.md:161` · `docs/testing-strategy.md:50` · `:179` · `base/src/lib.rs:54`) + 입주자 명단 사본(base `lib.rs` 헤더 「여덟」 `:4` · 선언 `:75-84` · CLAUDE.md base 항목 · 루트 `Cargo.toml` 멤버 주석 · `docs/testing-strategy.md` 의 base 단위 목록 · base 시험 줄). ★주석에 `crate::panic` · `super::panic` 철자를 쓰지 않는다(게이트 ③ 이 잡는다)★.
- **net:** 무변경(M5 N1) — 의존 상한 · 심볼 allowlist · `--all-features` 그대로.
- **메시징:** 의존 0 유지 — 관찰자를 못 부르는 것은 U8a 가 문장으로 다룬다(§2-6).
- **새 게이트:** 컴파일 가드(M10 — 코드 안 · 데몬 lib · agent lib) · 요청 경로 맨 spawn 소스 시험(M9) · U8b 뒤 §2-8 의 넓힌 정규식으로 남은 줄 확인(문서 확인 — CI 아님).
- **CI:** 워크플로 변경 = 게이트 ③ 문자열 하나. 릴리스 잡은 프로필을 그대로 따른다.
- **회귀 결과 줄:** U5 가 데몬 `tests/` 에 실프로세스 시험 파일 하나를 더하면 +1(가안). 나머지 단위 0.

---

## 7. 단위

**원칙(TRD A §6 과 같다):** 단위마다 워크스페이스 빌드 · 회귀 초록으로 끊는다 · 단위 안 순서 = 「새 자리 만들기 → 부르는 곳을 파일 단위로 → 옛 자리 지우기」(중간 어디서 멈춰도 빌드가 선다) · 착수 전 로컬 커밋으로 되돌릴 지점 · 주석은 `/code-conventions` 주석 규약 주입 · **ADR-0292 는 U1 바로 앞**(§8-1) · 코드 단위는 자기가 바꾸는 줄의 abort 전제 주석만 고치고 나머지는 U8a(의미) · U8b(문장)가 쓸어 담는다 · 크기가 애매한 단위는 파일 하나로 파일럿을 먼저 돈다.

### 7-1. 단위 목록 (크기는 권고안 기준 · 괄호는 (가) 다른 안)

| id | 범위 | 건드리는 파일 | 선행 | 크기(어림) | QA |
|---|---|---|---|---|---|
| **U1** | base `panic`(§4-1) · `sync` 관찰 갈래 · 범위 갈래((가)) · 관찰자 등록 · `logging` 재진입 가드 · 게이트 ③ · 명단 사본 | base `src/panic.rs`(새) · `src/sync.rs` · `src/logging/mod.rs` · `src/lib.rs` · base `Cargo.toml` description · 게이트 ③ 사본 다섯 · 명단 사본 | ADR-0292 | 중간(+350 · 시험 ≈ 20: 깊이 증감 · 중첩 · 되감기 뒤 복귀 · `Contained` poll 마다(스레드 옮김 흉내) · 패닉 · 취소 drop 이 깊이 안 · `escalating` 0 과 복귀 · `is_poison_panic` 핀(Mutex `.expect` · `.unwrap` · RwLock) · 걸쇠(SeqCst · 폴링 대기 먼저/나중 · 첫 사유 · 첫 자리) · 관찰 갈래가 호출 위치를 넘김 · 범위 갈래는 안 부름 · 미등록이면 말없음 · 재진입 가드) | standard |
| **U2** | 에이전트 2층(§4-7) · B1′ 감시 가드 + 사유 칸 · stderr 줄 가둠 · 통로 `shutdown` 락 되찾기(M14 — 파일 주인) · `reap_one` 구간((나)) · 페이로드 사본 여섯 걷기 · (가)=P2 범위 치환(이 파일들) | agent `transport/{pty,stdio,input_queue,spawn}.rs` · `backend/codex/transport.rs` · `backend/claude/leftover.rs` · `session_id_latch.rs` · `reaper.rs` | U1 | 중간(+150/−90 + P2 치환 ≈ 60곳) · (P1: +130/−90 · B2: U4b 별도). 크면 셋으로 가른다 — `transport/` · codex · claude+래치+reaper(겹침 없음) | standard |
| **U3** | 데몬 1층(§4-6) · (다) 거절 · 연결 수명 처리기 분류((나)) · 연결 소유 락 범위 치환(P2) · 맨 spawn 소스 시험(M9) | daemon `agent_conn.rs` · `connection_core.rs` · `command_delivery.rs` · `control/mcp_server.rs` · 새 `containment.rs`(가안) | U1 | 중간(+220 · 시험: 패닉 처리기 → 오류 답 + 연결 유지 · blocking 패닉 → 기존 오류 갈래 · HTTP 500 · MCP 도구 오류 · 깃발 뒤 새 일 거절 · 소스 시험) | standard |
| **U4** | 종료 경로 오염 내성(M14 — 통로 몫 빼고) · 활성화 창 `escalating` + 스코프 가드(M15) · 요청 kill 앞 줄 `escalating` · 코어 · 세션 범위 치환(P2) | agent `manager.rs` · `session.rs` · `output_core.rs` · `session_tracker.rs` · daemon `control/registry.rs`(`revoke` 등) | U1 | 중간(+130 + P2 치환 ≈ 35곳 · 시험: 명부 락 오염 뒤 `shutdown_all` 이 전부 끄고 돌아온다 · 한 에이전트의 코어 락 오염이 남의 kill 을 안 막는다 · 레지스트리 오염 뒤 `kill_agent` 완주 · 활성화 창 패닉 → 통로 `shutdown` 불림 · 명부에 좀비 없음) | standard |
| (U4b) | (가)=넓게일 때만 — codex 라이터 · thread_lock 의 「펌프로 가는 길」 | codex `transport.rs` · `thread_lock.rs` | U2 · U4 | +120~250 | standard + 실프로세스 |
| **U8a** | 의미 재감사 + 필요한 코드(M13) — K7 · `in_flight_targets`(`FlightSettle` · `Reservation`) · `DeliveryDone` 2층 표시 · 래치 · codex 라이터 · `lock_for_cleanup` 짝 · 런타임 로그 문장 `:769`. 결과 표 = ADR-0292 「영향」 · 이 TRD 부록 | `leftover.rs` · `session_id_latch.rs` · codex `transport.rs` · daemon `messaging_host.rs` · `command_roster.rs` · `command_delivery.rs` · messaging `service.rs` | U2 · U3 (★파일이 겹친다★) | 중간(감사 + 코드 ≤ 100줄 · 결과에 따라 +α) | standard + `/review code light` |
| **U5** | 데몬 훅 조립 · 관찰자 등록 · accept 루프 interval 팔 + 포착 · 꼬리 사유 로그 · `Err(101)` · `main.rs` 최상위 포착 · 방아쇠(M8) · 실프로세스 시험 | daemon `lib.rs`(훅 · 조립 · 루프 · 꼬리 · 주석 `:455-457`) · `main.rs` · 새 `tests/panic_policy.rs`(가안) | U2 · U3 · U4 (★표시 · 종료 경로가 먼저★) | 중간(+180 · 시험 파일 +1 결과 줄) | **full** — 실프로세스(debug 데몬 exe · 임시 `ENGRAM_DATA_DIR`): ① 표시 밖 패닉 → N초 안에 끝남 · 코드 101 · 로그에 사유 + 「데몬 종료 완료」 ② 가둔 패닉 → N초 뒤에도 살아 `ListAgents` 에 답함 ③ 공유 락 오염(가둔 패닉이 오염 → 다음 접근) → 정상 종료 + 오염 자리 로그 ④ 셸 에이전트 하나를 띄운 뒤 ① → 자식 사라짐 ⑤ 부팅 패닉 → 곧바로 101(매달림 없음). `send_stop` 결과는 판정에 안 쓴다(M12) |
| **U6** | 셸 훅((라)) · 깨우기 스레드 · `Exit` 패닉 갈래 · `Final` 없는 종료 세션 | `src-tauri/src/lib.rs` · `state/` 의 종료 세션 · 새 작은 모듈(가안) · 방아쇠 | U1 | 작음~중간(+140 · `lib_unit` 시험) | **full** — GUI: 배경 스레드 패닉 → 창이 닫힘 · 코드 101 · 데몬 PID 생존 · `engram agent list` 그대로 · 트레이 아이콘 사라짐 · 다음 실행에 복원 물음 / 주 스레드 패닉 → tao 되던지기로 코드 101(abort 아님) · 트레이 아이콘 상태 기록 / 버스 명령 패닉 → 창이 닫힘(S1a) |
| **U7** | 프로필 unwind · 컴파일 가드(데몬 lib · agent lib) · 크기 | 루트 `Cargo.toml:41` · 데몬 · agent `lib.rs` | U4 · U5 · U6 · **U8a** | 작음(+15) | **full** — 릴리스 세 exe 빌드 · 크기 앞뒤 기록(M11) · 가드 증명(스크래치 워크트리에서 abort 로 되돌려 빌드 실패 1회) · `scripts/` 런처로 릴리스 기동 · 에이전트 하나 띄우고 끄기 |
| **U8b** | abort 전제 **문장** 손질(사실 주장 · 사유) · base `sync` 계약 3 문장 | §2-8 넓힌 정규식의 남은 줄(≈ 57 중 앞 단위가 안 고친 것) | U7 | 중간(≈ 50~60줄 · ≈ 30파일 · 의미 변경 0) | standard |
| **U9** | ADR 도장 · 문서(§8) | §8-2 · §8-3 목록 | U7 | 문서 | `/qa` 문서 범위 + `/review doc`(load-bearing) |

### 7-2. 순서 · 물결 · 어디서 멈춰도 서는 까닭

```
물결 0:  ADR-0292(사용자 결정 (가)~(마) 뒤)
물결 1:  U1
물결 2:  U2  ∥  U3  ∥  U4          (agent 통로 · backend · reaper  /  daemon 요청 경로  /  agent manager · session · core · tracker + daemon registry)
         (U4b — (가)=넓게일 때 U2 · U4 뒤)
물결 3:  U8a  ∥  U5  ∥  U6         (U8a = U2 · U3 파일 위 · U5 = daemon lib · main · tests · U6 = 셸 — 안 겹침)
물결 4:  U7                          (U4 · U5 · U6 · U8a 뒤)
물결 5:  U8b  ∥  U9
```

다시 잰 근거(2판 변경 반영):

- **U1~U4 뒤에 멈추면** — 표시 · 되찾기 · `escalating` 구간 · 스코프 가드 · 거절 확인만 있고 훅도 관찰자 등록도 없다. 관찰 갈래는 관찰자가 없으면 지금처럼 말없이 되찾고 · `escalating` 은 훅이 없으면 효과가 없고 · 거절 확인은 깃발이 안 서니 무동작이다. debug = 잡던 자리는 같은 결말 · 새로 잡는 자리(WS 처리기 · HTTP · MCP · 감시 · stderr)는 더 나은 결말 · 스코프 가드는 실패 갈래를 덜 나쁘게. 배포판 = abort 그대로.
- **U8a 뒤** — 감사가 고른 코드는 abort 아래 도달 불가 갈래이거나(배포판 무변화) debug 결말을 바로잡는 것이다. 배포판 = abort 그대로.
- **U5 · U6 뒤 U7 전에 멈추면** — debug 는 새 정책대로 돈다. 배포판은 여전히 abort — 훅은 abort 직전에 당긴 뒤 같은 즉사를 맞는다 · `main.rs` 의 `catch_unwind` 는 abort 에서 아무것도 안 잡는다. 지금보다 나빠지는 자리가 없다.
- **U7 을 U4 · U5 · U6 · U8a 보다 먼저 하면 안 된다** — 훅 없는 unwind 배포판은 표시 밖 패닉이 그 스레드만 조용히 죽이는 모양(ADR-0290 이 거부한 것) · 종료 경로 오염 내성 없는 unwind 는 「돌지도 죽지도 않는」 데몬 · 감사 안 한 abort 전제 코드(K7 등)가 배포된다. 컴파일 가드는 U7 에서 함께 든다(먼저 넣으면 abort 프로필에서 빌드가 깨진다).
- **표시(U2 · U3)를 훅(U5)보다 먼저** — ADR-0290 의 순서 제약 그대로다.
- **머지** — 어느 접두든 머지해도 된다(위가 근거). 병렬 물결은 워크트리를 갈라 돌고 접점(§4-1 API · 관찰 · 범위 함수 이름 · `containment.rs` 도우미 서명)을 U1 착지 모양으로 못 박은 뒤 띄운다.
- 한 사람이 직렬로 돌면: U1 → U4 → U2 → U3 → U8a → U5 → U6 → U7 → U8b → U9.

### 7-3. 파일 겹침

| | U2 | U3 | U4 | U8a | U5 | U6 | U8b |
|---|---|---|---|---|---|---|---|
| U1 | — | — | — | — | — | — | base `sync.rs`(U1 = 계약 2 · 갈래 · U8b = 계약 3 문장) |
| U2 | | — | — | `leftover.rs` · `session_id_latch.rs` · codex `transport.rs` → **U8a 는 U2 뒤** | — | — | 통로 파일의 남은 문장 |
| U3 | — | | — | `command_delivery.rs` → **U8a 는 U3 뒤** | — | — | 요청 경로 파일의 남은 문장 |
| U4 | — | — | | — | — | — | `manager.rs` 문장 |
| U5 | — | — | — | — | | — | daemon `lib.rs` 의 남은 문장(sweep `:729` 경고 등) |

### 7-4. 단위 안 순서 (빌드가 서게)

- **U1** ① `panic.rs` 와 시험(아무도 안 부름) ② `sync` 관찰 갈래(등록 0 이면 지금과 같다) + 범위 갈래(새 이름 · 아무도 안 부름) ③ `logging` 재진입 가드 ④ 헤더 · 게이트 ③ · 명단 사본.
- **U2** 파일마다 한 걸음(펌프 셋 → 라이터 → 감시 가드 · 사유 칸 → stderr → 통로 `shutdown` 락 → codex 기록 → leftover → reaper 구간) · 사본 걷기 · 범위 치환은 그 파일 걸음에서.
- **U3** ① `containment.rs`(아무도 안 부름) ② 처리기 · spawn 자리를 파일 단위로 ③ HTTP 층 · MCP 도구 ④ 거절 확인 ⑤ 소스 시험.
- **U4** `get_session` → `shutdown_all` → `kill_agent` 앞 줄 → `enter_exiting` · `finish` → 추적기 → 레지스트리 → 활성화 창(가드 · `escalating`) → 범위 치환 — 걸음마다 선다.
- **U8a** ① 자리마다 「unwind 아래 무엇이 닿나」 표 ② 표가 요구하는 코드(작은 걸음) ③ ADR-0292 영향 · 부록에 결과.
- **U5** ① 훅 판정을 조립부 순수 함수로(당기기 없음) ② 관찰자 등록 + accept 루프 interval 팔 + 꼬리 사유 로그 · 코드 ③ 당기기 켬 ④ accept 루프 · `main.rs` 포착 ⑤ 방아쇠 · 실프로세스 시험.
- **U6** ① 종료 세션의 「`Final` 없이」 갈래 ② 훅 · 깨우기 스레드 · `Exit` 패닉 갈래 ③ 방아쇠 · 시험.
- **U7** ① 크기 기준 재기 ② 프로필 + 가드 한 커밋 ③ 크기 · 릴리스 QA.

### 7-5. 단위마다 검증

1. **회귀 수 대조** — `cargo test --workspace -- --test-threads=4` 를 단위 앞뒤로 · `test result:` 줄 수와 통과 총계 둘 다. 기준선은 U1 착수 직전에 다시 잰다(이 TRD 는 돌려 보지 않았다). 기대 = 결과 줄은 U5 만 +1(가안) · 통과 수는 단위가 선언한다.
2. **게이트** — 건드린 crate 의 게이트 전부 + `cargo fmt --check` + base 게이트 ③(U1 뒤 알파벳에 `panic`) · `cargo test -p engram-dashboard --test lib_unit`(U6) · 데몬 · agent 는 `--test-threads=4`(실 프로세스).
3. **QA 등급** — 표의 등급(`/qa` 바인딩). full 은 앱을 `scripts/` 런처로 띄운다(셸에서 직접 띄우지 않는다).

---

## 8. ADR · 문서 후속

### 8-1. ADR-0292 (U1 바로 앞 · `/adr new` — 번호는 예약된 0292)

- **제목(가안):** 패닉 정책 구현 — 가두는 입구는 스레드 표시를 세우고 · 표시 밖이나 공유 락 오염 패닉은 훅이 정상 종료를 당기며 · 배포판은 unwind 다
- **결정(채택된 것만 적는다):** ① 배포판 unwind(워크스페이스 전체 — 사용자 결정 2026-10-10) + 데몬 lib · agent lib 컴파일 가드 ② 표시 = base `panic`(스레드 지역 깊이 · 동기 · poll 단위 비동기 · `escalating`) — 원시만, 정책은 조립부 ③ 층 경계 표((가) 결과 + net 자기 루프 · 연결 수명 = 3층) ④ 락 오염 정책((가) 결과 · 관찰자 계약) ⑤ 여러 걸음 공유 변경 구간((나) 결과) + 스폰 경로 스코프 가드 ⑥ 훅 — 첫 사유 기록 → 깃발(SeqCst · unpark 없음) → 로그(재진입 가드) ⑦ 깨우기 = 폴링(데몬 accept 루프 팔 · 셸 깨우기 스레드) ⑧ 데몬 끝 = 기존 꼬리 + `exit(101)`(load-bearing) · `main.rs` 최상위 포착 ⑨ 당긴 뒤 새 일((다) 결과) ⑩ 셸 동작((라) 결과) ⑪ 고지 모양((마) 결과).
- **거부한 대안:** 사용자 · 메인이 이 TRD 의 선택지를 고를 때 실제로 버린 것만, 그때 준 까닭으로 적는다 — ★이 TRD 의 「권고하지 않음」을 그대로 옮겨 적지 않는다(결정 날조 금지)★. 메인 결정에서 실제로 버린 것(근거가 이 TRD 에 있다): 깨우기 unpark(store-buffering · 잎 락 불변식 — M3) · watch 직접 송신(M3) · net 하위 태스크 포장(N3 — 반쪽 연결 · M5) · base 정책 함수(ADR-0269 ② — M1).
- **링크:**
  - Implements ADR-0288 · ADR-0290.
  - **Amends ADR-0288 (결정 1 범위)** — 「연결 하나」 = 데몬 처리기 몸통 · net 자기 루프 · 연결 수명 코드 = 3층(M5). (나)=E 면 「명령 하나」 안의 여러 걸음 공유 변경 구간도 정상 종료로 간다.
  - **Amends ADR-0288 (결정 2 범위)** — (가)가 B2 가 아니면 에이전트당 스레드 일부(codex 라이터 · thread_lock · B1 이면 감시 · stderr 도)가 정상 종료로 간다 · (가)=P1 이면 락을 쥔 2층 패닉도 · (가)=P2 면 「그 에이전트만 **정리**」 대신 요청이 반쯤 바꾼 에이전트는 계속 돈다 · (나)=E 면 에이전트 시작 · 수거 · kill 구간이 정상 종료로 간다. (사용자 답에 맞춰 해당 줄만 남긴다.)
  - **Amends ADR-0290 (영향 — 연결 태스크 표시 · 표시 모양)** — 연결 태스크(`lib.rs:395`)에는 표시를 세우지 않는다(M5) · 「이미 가두던 reaper」는 (나)에 따라 고리만 살리고 구간은 올린다.
  - Amends §8-2 의 일곱.
  - 앵커 `// ADR-0292` = 훅 조립(데몬 · 셸) · `panic` 모듈 머리 · `sync` 관찰 갈래 · 프로필 줄 · 컴파일 가드 둘 · `escalating` 구간 · `main.rs` 포착 · `exit(101)` 줄.
- **TRD S20 §4 ⑨ 대체 표시** — ADR 은 TRD 를 링크로 폐기하지 못하므로 ADR-0292 「맥락」에 「TRD S20 §4 ⑨ 의 프로필 결정을 대체한다(핸들러 규약은 유효)」를 적고, 그 절 머리에 대체 표시를 단다(§8-2).

### 8-2. 개정 도장 (`/adr link` — 그 전제를 실제로 바꾸는 단위 착지 뒤 · U9)

| 대상 | 무엇이 바뀌나 |
|---|---|
| ADR-0288 | 결정 1 · 2 범위(§8-1) |
| ADR-0290 | 영향 — 연결 태스크 표시 · reaper(§8-1) |
| ADR-0142 | 「수용한 잔여(주입 중 언와인딩 → 재기동 신호 누락)는 debug · test 에만」 → U8a 결과에 따라: 배달이 2층으로 가둬지면 배포판에서도 닿는다 · `FlightSettle` 이 오염이면 말없이 건너뛰는 자리의 판정 |
| ADR-0199 | 「이 갈래는 릴리스에서 도달 불가」 → 닿는다(세션 id 기록 패닉이 2층으로 잡힌다) |
| ADR-0226 | 래치 `catch_unwind` 기각 사유 「abort 라 죽은 코드」 → 「바깥 입구 표시가 가둔다」(결론 = 여전히 래치에 catch 없음) |
| ADR-0233 | 리더 스레드에서 도는 commit 의 패닉 = 리더 펌프가 가둔다(2층) |
| ADR-0262 | K7 — 막 둘이 abort 에 기대던 자리의 U8a 재분석 결과 |
| ADR-0275 | 결정 1 · S2(a) 기각 사유의 「운영은 오염 갈래에 닿지 않는다」 전제 → 닿는다 · 되찾기는 관찰 · 범위 두 갈래(`sync` 계약 2 · 3) |
| ADR-0281 | 「dialog 플러그인을 빼면 거절 박스가 앱 종료」의 사유 → 셸 훅이 끝낸다(결론 같음) |
| TRD S20 §4 ⑨ | 절 머리에 대체 표시(「프로필 결정 · 귀결은 ADR-0292 · 이 TRD 가 대체 — 핸들러 규약은 유효」) — TRD 라 ADR 링크가 아니라 표시 |

### 8-3. 그 밖 문서

- `.claude/skill-bindings/review.md:20` — 핸들러 규약의 「근거 · 귀결 정본」 포인터를 S20 ⑨ → ADR-0292 로.
- CLAUDE.md — 「핵심 불변식」에 한 항목(가둠 표시는 1 · 2층 입구만 · 표시 밖 · 공유 락 오염 = 정상 종료 · `escalating` 구간 · 2층은 펌프의 `finish` 로만 끝난다(ADR-0127 과 같은 줄) · 훅 · 관찰자는 락 · OS 호출 없음) · 「백엔드 모듈 맵」 base 항목(입주자 아홉 · `sync` 관찰 · 범위 갈래) · 게이트 ③ 줄 · 회귀 수치(U5 뒤).
- `docs/testing-strategy.md` — 게이트 ③ 사본 둘 · base 단위 목록.
- `docs/tracking.md` — T-52 에 한 줄(M12 — 패닉 종료는 `send_stop` 을 안 탄다 · 동시 종료 경합) · 이 TRD 가 미룬 것 T- 로(P3 · (가)에서 안 고른 B2 갈래 · (나)=R 구간 · 셸 상자 · 재기동 · 꼬리 MCP 무한 대기). ADR-0288 이 미룬 장치(종료 시간 제한 · 셸 사유 고지 · 다음 부팅 패닉 표시)가 이미 T- 로 있는지 확인하고 없으면 연다.
- `docs/process/step-log.md` — 단위마다 항목 · **이 TRD 를 링크한다(고아 금지)**.
- 날짜 박힌 TRD 다섯(§2-8) — 고치지 않는다(스냅숏). load-bearing 인 자리가 나오면 그 자리만 표시.

---

## 9. 미검 · 열린 것

1. **WebView2 COM 콜백을 건너는 패닉** — 웹뷰 IPC 로 부르는 동기 Tauri 명령 등이 `extern "system"` 콜백 안에서 패닉하면 abort 일 수 있다(미검 · 기억). 창 프로시저 쪽은 tao 되던지기로 확인했다(§2-7). U6 이 관측한다. 결말은 S1a 와 같아(셸만 끝 · `Final` 없음) 설계를 바꾸지 않는다 — 다른 점은 트레이 정리가 없다는 것.
2. **오염 패닉 문장의 `PoisonError`** — std `Debug` 문자열에 기댄다. U1 시험이 지금 툴체인(1.95.0)의 모양을 못 박는다.
3. **`Thread::park_timeout` 의 Windows 구현** — 이 PC 에 rust-src 가 없어 원문 대조 못 했다. 설계는 거짓 깨어남을 견디는 고리(std 문서 계약)에만 기댄다 · unpark 는 쓰지 않는다.
4. **바이너리 크기** — 미측정(M11 · U7).
5. **K7 · `in_flight_targets` 등 의미 재감사** — U8a 몫이다. 결과가 코드 손질이면 U8a 가 커진다.
6. **`transport` crate(`crates/engram-dashboard-transport`)의 태스크** — 운영 부착이 진행 중이다. 부착되면 그 태스크도 §4-2 표 기준을 따른다.
7. **MCP 종료 대기** — rmcp 가 자식 취소 토큰으로 세션 스트림을 닫아 axum graceful 이 유한하게 끝나는지(§4-11) 미검.
8. **`transport.shutdown()` 두 번 부름의 안전** — 스코프 가드와 `shutdown_all` 이 겹칠 때(M15) · U4 가 확인한다.
9. **통로 drop 의 ConPTY 순서** — 지금처럼 `shutdown` 없이 drop 되면 master(ClosePseudoConsole)가 무리보다 먼저 닫히는데, 그때 막히는지 미검. 스코프 가드가 그 길을 없앤다.
10. **셸 주 스레드 패닉 뒤 트레이 유령 아이콘** — 추론 · 미검(U6 관측).
11. **(가)~(마)의 사용자 답** — §4 는 권고안 기준이다. 답이 다르면 §4-2 · §4-5 · §4-6 · §4-7 · §4-8 · §4-9 · §7-1(U2 · U3 · U4 · U4b · U6 크기)만 바뀐다.

---

## 10. 2판 개정 대조 (리뷰 1회차 → 2판)

| 리뷰 항목 | 2판에서 바뀐 것 |
|---|---|
| R-M2 — P1 이 범위 안 되찾기를 재시작으로 만든다 · 순서 락 · `replay` · 되감기 재잠금 | 메인 M2 를 걷고 **사용자 결정 (가)** 로 올림 · 인벤토리 §2-6(되찾기 76 · `.expect` 55 · codex 42 · 되감기 재잠금 셋 · 순서 락 셋) · 선택지 P1 / P1o / P2 · §5 1 · 2행을 「락을 안 쥐었을 때만」으로 정직하게 |
| R-F2 — §4-10 오귀속 · 활성화 좀비 세션 | §4-11 첫 항목 정정(결정 3) · **사용자 결정 (나)**(E 올림 / R 되돌림) · `escalating` 원시(§4-1) · 구간 표(활성화 · `reap_one` · 요청 kill · 연결 수명 처리기) · 스코프 가드 M15 |
| R-F6 — 당긴 뒤 꼬리가 계속 섬긴다 | **사용자 결정 (다)**(받아들임 / 새 일 거절) |
| R-F14 — 셸 버스 명령 패닉이 창을 닫는다 | (라)의 S1a 에 명시 + **S1a′**(셸 버스 명령 가둠) 선택지 |
| R-UD1 — reaper · B1′ · Amends · B2 비용 | reaper → (나) 구간으로(고리는 3층 catch 유지) · **B1′** 추가 — 단 감시는 「이미 설계된 결말」이 자연 종료에만 맞아(`pty.rs:259-260` · 패닉이면 master 를 안 놓는다) 되감기 가드 + 사유 칸이 필요하다고 정정(+40~60줄) · B2 비용은 그대로(codex 라이터) · §8-1 **Amends ADR-0288 (결정 2 범위)** |
| net — 포장하면 반쪽 연결 · 몸통 패닉이 하위 태스크를 떼어 냄 | M5 = **N1**(net 무변경 · 데몬 처리기 몸통만 · 연결 태스크 표시 안 함) · 1판 N3 철회 · Amends ADR-0288 결정 1 · ADR-0290 영향 |
| 부팅 · 메인 future | M6 — `main.rs` 최상위 `catch_unwind` → `exit(101)` + accept 루프 포착 → 꼬리 · 1판 `CaughtOnly` 걷음 |
| 단위 순서 — U8 이 U7 뒤 | **U8a**(의미 재감사 + 코드 — U7 앞) · **U8b**(문장 — U7 뒤) |
| 종료 경로 오염 — `revoke` · 통로 `shutdown` 락 | M14 · U4 에 레지스트리(`revoke` `:261` 외 아홉) · `enter_exiting` · 추적기 · U2 에 통로 `shutdown`(파일 주인) |
| 깨우기 — store-buffering · unpark 가 OS 호출 · 깨우기 스레드 실패 · TLS | M3 — SeqCst 깃발 하나 · unpark 없음 · 셸 = `park_timeout` 고리 · 데몬 = accept 루프 interval 팔 · TLS = const `Cell` + `try_with` · `EnvFilter` 락 주석 |
| 로그 사유 — 당김이 로그보다 먼저면 사유를 잃는다 | M4 — 첫 사유 `OnceLock<String>`(깃발 앞) · 관찰자 `#[track_caller]` → `AtomicPtr` · 꼬리 · 깨우기가 로그 · L1 문장 정정 |
| 셸 끝내기 — `cleanup_before_exit` · 표지 경합 · tao 되던지기 | M18 · §2-7 · §4-9 — `is_pulled()` 판정 · `cleanup_before_exit()` 뒤 `exit` · 주 스레드 패닉이면 깨우기가 `app.exit` 안 부름 · 1판 「FFI abort」 정정 · U6 QA 기대(주 스레드 → 101) |
| 인벤토리 — StopDaemon · 동시 종료 · Drop 설계 · 정규식 | StopDaemon `shutdown_all` → M9 허용 목록(3층) · 동시 종료 경합 M12 · §2-2 에 `DeliveryDone` · `FlightSettle` · `Reservation`(줄 바로잡음 — `Reservation` `:2718` · `FlightSettle` `:3505`) · 정규식 넓힘(57줄 · 30파일 · 놓친 일곱) |
| `Contained` drop · axum 층 · 관찰자 · cmd_lookup | M16 · M17 — 가드 안 drop(되감는 중엔 무력함 명시) · 가장 바깥 층 · rmcp 도구 몸통 직접(확인됨) · 관찰자 금지 목록 · cmd_lookup 무해(§2-6) |
| base 거주 — `verdict` 가 정책 | M1 — base 는 `is_poison_panic` 원시만 · 정책은 데몬 · 셸 조립부 · M10 가드를 데몬 lib · agent lib 로 |
| M7 — `exit` 가 런타임 drop 을 건너뛴다 | M7 load-bearing 명시(tokio 문서 확인) · 꼬리의 남은 무한 대기 둘을 §4-11 에(1판 「flush hang 같음」 정정 — flush 는 5초 띠로 유한) |
| 받아들일 위험 — 팬아웃 · `reap_one` · 스폰 경로 | §4-11 에 팬아웃 seq 구멍 · `reap_one` 낡은 줄(조건부) · 스폰 경로는 위험이 아니라 M15 로 막음 |
| 입력 자료 오류(저자 실측) | 유지 + 1판 자신의 오류 다섯을 §2-9 6 에 |

**거부 · 일부 거부한 것(근거):**

- **데몬 깨우기 스레드(리뷰: 「데몬 · 셸 같게 `park_timeout` 스레드 + accept 루프 띠」)** — 데몬에서는 스레드를 두지 않고 accept 루프의 interval 팔 하나로 한다. 근거: 깨우기 스레드가 하는 일은 `shutdown_tx.send(true)` 하나이고 그 신호의 소비자는 accept 루프의 종료 watch 팔(`lib.rs:414-423`)인데, 그 팔과 interval 팔은 **같은 `select!`(`:387`)** 안에서 같은 런타임 위에 돈다 — interval 팔을 멈출 상황(런타임 정지)이면 watch 팔도 멈춘다. 스레드는 기동 실패 · 죽음이라는 실패 모드만 더한다. interval 팔이 `send(true)` 로 다른 구독자에게도 같은 신호를 준다. 셸은 폴링할 루프가 없어 스레드를 둔다(리뷰안 그대로).
- **B1′ 「감시 · stderr 는 몇 줄씩」** — stderr 는 맞고, 감시는 틀렸다(위 R-UD1 행 · `pty.rs:259-260` · 패닉 경로에 master drop 없음).
- 그 밖 리뷰 항목은 전부 받아들였다(위 표).
