# TRD — 패닉 정책 구현: 좁은 가둠 · 즉시 종료 훅 · unwind 전환 (S21)

> **상태: 11판 개정 2(2026-10-12 · 리뷰 1 라운드 반영).** 코드 변경 0 · 사용자 결정 전부 끝(§3) · 리뷰 대립 1건 사용자 재판정 대기(§4-2 자식 끄기). 개정 2 = `usage.get` 을 가둠에서 뺌(허용 목록 둘) · 표시 끔 원시와 그 자리 넷 · 「표시 범위 ⊆ catch 클로저」 불변식 · 펌프 패닉 때 자식 명시 종료 · U3 확인 거리.
> **이력:** 10판 = `git show a722a879:docs/process/S21-panic-policy/trd.md`(종료 꼬리 · 감시 스레드 · 결정 (다)(바)(바′)(라′) · 컴파일 가드는 새 방향에서 무효) · 11판 초안 = `bedda3f3`((가) 락 오염 · (나) 여러 걸음 구간 · 요청 쪽 입구 R1~R7 — 사용자 결정 ② 로 걷힘).
> **틀:** ADR-0288 · ADR-0290 + 사용자 결정 2026-10-10 · 2026-10-11~12 · 2026-10-12 ②(§1). ★세 ADR(0288 ~ 0290)은 브랜치 `origin/v0.3.3/feat/panic-policy` 에만 있다★ — 이 TRD 를 커밋하기 전에 그 브랜치를 들여야 링크가 선다. 구현 ADR = ADR-0292(예정).
> **대체하는 옛 기록:** `docs/process/S20-command-bus/trd.md` §4 ⑨ 「릴리즈 `panic = "abort"` 유지」. 그 절의 규약(명령 핸들러는 값으로 실패한다)은 산다.
> **표기:** 실측 = 트리 `743530fb`(코드는 10판이 잰 `2c25cdd` 와 같다 — 그 사이 커밋은 문서뿐). 경로의 `crates/engram-dashboard-` 는 줄여 `agent/` · `daemon/` · `base/` 처럼 쓴다. 「표시」 = 「가두는 중」 표시(§4-3). 「가안」 = 이름 · 모양 미확정. 확신도 표기가 없으면 확실이다.

---

## 0. 결론

```
① 가두는 것은 둘뿐이다 — 에이전트 하나만 섬기는 펌프 셋 · 라이터, 그리고 허용 목록의 읽기 명령 둘
   (agent.list · agent.listQueuedInputs)의 본문. 가둔 패닉 = 그 에이전트 Failed(자식 CLI 는 명시로 끈다) · 그 명령 INTERNAL 답.
② 그 밖의 패닉 = 사유를 로그하고 곧바로 abort — 오늘 배포판과 같은 결과. 가둔 코드 안에서도 여러 에이전트가
   함께 쓰는 상태를 고치는 호출은 표시를 꺼서 같은 길로 보낸다.
③ 둘을 가르는 것은 스레드 지역 「표시」 하나이고, 표시는 늘 그것을 잡는 catch 클로저 안에서만 선다.
④ 순서 = 원시 → 가두는 자리 → 훅 → 확인 · 감사 → 프로필 한 줄(§5).
   앞 단위는 abort 아래 아무 일도 안 한다 — 어디서 멈춰도 배포판은 오늘과 같고, 문제가 나면 마지막 한 줄만 되돌린다.
```

---

## 1. 정해진 틀

| 출처 | 내용 | 이 TRD |
|---|---|---|
| ADR-0288 결정 1 · 2 | 요청 하나 · 에이전트 하나의 패닉은 그것만 실패 | **좁힘**(사용자 결정 ②) — 요청은 읽기 명령 둘 · 에이전트는 펌프 · 라이터만(§4-2) |
| ADR-0288 결정 3 · ADR-0290 결정 2 | 공유 상태 패닉 · 가두지 못한 패닉 = 훅이 기존 정상 종료 경로를 당긴다 | **개정** — 사유 로그 → 즉시 종료(§4-4) |
| ADR-0288 결정 4 | Rust abort(이중 패닉 등) = 그냥 죽음 · 에이전트는 Job 이 정리 | 유지 |
| 사용자 결정 2026-10-10 | unwind 범위 = 워크스페이스 전체(데몬 · 셸 · CLI) · 셸에도 같은 훅 | 유지 — 셸은 훅만(§4-5) |
| 사용자 결정 2026-10-11~12 | 가두지 못한 패닉 = 즉시 종료(「복잡하게 할 필요없음」) · TRD · 구현은 큰 덩어리로 | 유지 |
| 사용자 결정 ② 2026-10-12 | 가둠은 상태를 안 바꾸거나 격리된 단위만 — 상태를 바꾸는 것은 가두지 않는다 | 이 개정 — 락 오염 정책 · 여러 걸음 구간 장치가 필요 없어졌다 |

- **근거(`docs/research/panic-practice-peers-2026-10-12.md`):** 「가두지 못한 패닉 = 끝」은 흔한 모양이다(tikv · materialize · zed · zellij — 결론 1). 가두는 범위는 대개 좁다 — rust-analyzer 는 읽기 전용 요청만 가두고 상태를 바꾸는 처리는 「isn't guarded by `catch_unwind`」로 둔다(그 저장소 `crates/rust-analyzer/src/handlers/dispatch.rs:34-44` — 결론 2). 「표시 + 표시 밖이면 훅이 끝냄」은 materialize 에 선례가 있다 — 스레드별 카운터 `CATCHING_UNWIND` · 그 밖이면 abort(`src/ore/src/panic.rs` — 결론 3). 상태를 바꾸는 쪽을 안 가두면 락 오염 규칙이 필요 없다(결론 4 · rust-analyzer 의 풀이).
- **따라 사라진 요구:** ADR-0290 「영향」의 「정상 종료 경로는 오염된 공유 락에서도 끝까지 가야 한다」는 패닉 종료가 그 경로를 탔기 때문이었다. 이제 패닉은 그 경로를 안 탄다 — StopDaemon 자체의 오염 내성은 범위 밖(§7).

---

## 2. 현황 (실측)

- **프로필:** `Cargo.toml:39-44` `[profile.release]` — `panic = "abort"`(`:40`). 워크스페이스 유일이라 데몬 · 셸 · CLI 가 함께 바뀐다. debug · 시험 빌드는 이미 unwind 다.
- **훅:** 데몬에만 있다 — `daemon/src/lib.rs:129-151`(로그 + 이전 훅) · 설치 `:458`(로깅 초기화 `:453` 뒤) · 부르는 곳은 운영 `daemon/src/main.rs:13` 하나라 시험 서버에는 훅이 없다. 셸 · CLI 에는 없다. std 의 훅은 **패닉한 스레드에서, 되감기 전에** 돈다.
- **로그 자기교착(알려진 미수정):** 로그 파일 Mutex 를 쥔 채 패닉하면 훅의 로그가 자기를 기다린다 — `base/src/logging/mod.rs:110-115`.
- **이미 있는 가둠:** 펌프 셋(`agent/src/transport/pty.rs:298` · `stdio.rs:241` · `agent/src/backend/codex/transport.rs:4018`) · 라이터(`agent/src/transport/input_queue.rs:288`) · 버스 명령 본문 `guard_panic`(`command/src/route.rs:20` — 모든 blocking 핸들러를 감싼다 `command/src/table.rs:339`). 배포판은 abort 라 지금 하나도 잡지 않는다. 펌프의 끝(`finish`)은 catch **밖**에서 돈다(`pty.rs:342` · `stdio.rs:308` · `codex/transport.rs:4021`).
- **데몬 명령 본문은 동기로 돈다:** 데몬 자기 명령(`agent.*` · `usage.*`)은 입구가 어디든(버스 1단계 blocking `daemon/src/command_delivery.rs:1361` · `/control/agent` · `/control/call`) `call_daemon_command`(`daemon/src/control/commands.rs:163`)를 지나고, 그 안의 `drive_to_completion`(`:250`)이 future 를 `now_or_never` 로 **부른 스레드에서** 끝낸다 — `guard_panic` 은 그 blocking 스레드에서 동기로 돈다.
- **`Read` 선언과 실제:** 데몬 `Read` 명령은 셋이다(`agent/src/commands.rs:212` `agent.list` · `:297` `agent.listQueuedInputs` · `:344` `usage.get`). 그러나 `usage.get` 은 조회를 띄운다(`:334-337`) — 사용량 책이 `in_flight` 를 세우고(`daemon/src/usage_service/book.rs:279-282`) 합류자를 붙이고 구독자 전부에게 내고 조회 스레드 · CLI 를 띄운다(`daemon/src/usage_service/mod.rs:341-386`). 효과 값은 이미 「id 를 붙들어 재실행을 막을까」를 가르는 계약이다(`daemon/src/control/commands.rs:286-291`).
- **Job:** 무리 주인이 Job 을 `KILL_ON_JOB_CLOSE` 로 만들고(`platform/src/group/windows.rs:153`) 그 핸들의 drop 이 Job 을 닫는다(`:601`). reaper 는 세션 `Arc` 를 놓을 뿐이고, 그 주석은 「자식은 이미 죽어 있다」를 전제한다(`agent/src/reaper.rs:65-70`).
- **abort 전제 기록:** 코드 57줄 · 30파일 + ADR 일곱 + TRD 여럿 — 목록과 정규식은 10판 §2-8.
- **CLI:** 단일 스레드(`cli/src` 에 스레드 · tokio 없음). **종료 코드:** 운영 코드에 프로세스 종료 코드를 판정하는 곳이 없다.

---

## 3. 사용자 결정 (2026-10-12 — 전부 끝)

> (가) · (나)는 사용자 결정 ②로 없어졌고, (라) 셸은 ②로 S1(훅만 · 셸 패닉 = 즉시 종료)로 정해졌다(§4-5). 아래 두 묶음은 권고안대로 정했다 — 아래 표 · 절은 그 근거로 남긴다.

| 묶음 | 결정 | 사용자 말 |
|---|---|---|
| (마) 가둔 에이전트 패닉의 고지 모양 | **N2a 지금 표면** — 화면은 `Failed` · 입력 거절 그대로 · 자세한 것은 로그(§4-4 훅은 표시와 무관하게 로그한다) | 「그냥 내부에서 대되 로그는 남겨지는건 맞지?」 |
| (사) 끝내기 수단 | **`std::process::abort`** | 「추천으로해」 |

### (마) 가둔 에이전트 패닉의 고지

버스 명령은 정해져 있다 — `INTERNAL` 「command handler panicked: …」(ADR-0159). 남은 것은 에이전트 쪽이다.

| 안 | 사용자 · LLM 이 보는 것 | 비용 |
|---|---|---|
| **N2a 지금 표면(권고)** | 펌프 = 그 에이전트 `Failed`(「pump panicked: …」) · 라이터 = 입력 거절 사유 | 0 |
| N2b 띠 알림 더함 | 위 + 띠 「에이전트 X 가 내부 오류로 멈췄다」 | 새 고지 장치(ADR-0288 이 미룬 무리) |

- 권고 N2a — 새 화면 장치가 없다. 즉시 종료는 지금의 「데몬 꺼짐」과 같게 보인다(사유 안내 없음 — 미룬 장치).

### (사) 끝내기 수단

| 안 | 결과 | 비용 |
|---|---|---|
| **abort — `std::process::abort`(권고)** | 오늘 배포판과 같은 끝(Windows 종료 코드 `0xC0000409` — `command/src/testing.rs:25-27` 의 실측 주석) | 새 코드 0 · std 뿐 · 전환 전후 겉모습이 같다 |
| 자기 `TerminateProcess`(코드 101) | DLL 분리 알림 없이 끝 · 종료 코드를 고를 수 있다 | platform 도우미 하나(OS 코드 — ADR-0266) · 훅에 주입 · 배포판 종료 코드가 101 로 바뀐다 |
| `process::exit(101)`(권고 안 함) | — | `ExitProcess` 가 다른 스레드를 먼저 끝낸 뒤 DLL 분리 알림을 돌아 거기서 멎을 수 있다(MS 문서 순서 · 가능성 높음 · 실측 아님) |

- 두 안에 대한 Windows 오류 보고(WER)의 반응이 다른지는 모른다(불확실) — abort 쪽은 오늘과 같다.

---

## 4. 설계

### 4-1. 가둠 기준

넷이 모두 설 때만 가둔다. 하나라도 빠지면 가두지 않는다 → 즉시 종료.

1. 그 일 단위의 **가장 바깥 입구**다.
2. **실패를 알릴 길**이 있다(`Failed` 상태 · 오류 답 — (마)).
3. 그 단위가 **상태를 바꾸지 않거나, 자기만의 상태만** 바꾼다. 공유 상태를 바꾸는 단위는 가두지 않는다(사용자 결정 ②) — 가둔 단위 안의 공유 상태 호출은 표시를 끈다(§4-2 표시 끔).
4. 그 단위를 버린 뒤 **나머지가 멀쩡**하다(매달리는 스레드 · 살아남는 자식 · 차오르는 파이프가 남지 않는다).

### 4-2. 가둠 표

| # | 자리 | 단위 | 실패 알림 | 할 일 |
|---|---|---|---|---|
| A1 | PTY 펌프(`agent/src/transport/pty.rs:298` · 스레드 `:269`) | 에이전트 하나 | `Failed` | 지금의 `catch_unwind` 를 `contain` 으로 바꾼다 · 패닉 갈래에서 자식을 끈다 |
| A2 | stdio 펌프(`agent/src/transport/stdio.rs:241`) | 에이전트 하나 | `Failed` | 같음 |
| A3 | codex 리더 펌프(`agent/src/backend/codex/transport.rs:4018` · 스레드 `:4017`) | 에이전트 하나 | `Failed` | 같음 |
| A4 | 라이터 `input_queue::drain`(`agent/src/transport/input_queue.rs:288` — PTY · stdio 라이터 스레드가 부른다 `pty.rs:208` · `stdio.rs:208`) | 그 에이전트의 입력 | 큐 닫힘 · 입력 거절 사유(`:273-279`) | `catch_unwind` → `contain` |
| B1 | 허용 목록 명령 본문 — `agent.list` · `agent.listQueuedInputs` | 명령 하나 | `INTERNAL` | `call_daemon_command` 의 `drive_to_completion` 호출(`daemon/src/control/commands.rs:183`)을 `contain` 으로 감싼다 — 이름이 허용 목록에 있을 때만 |

- **B1 = 이름 허용 목록이지 효과 술어가 아니다.** `usage.get` 은 `Read` 로 선언됐지만 공유 사용량 책을 바꾼다(§2) — 가두면 판정(`Start`) 뒤 패닉이 `in_flight` 를 영영 남긴다. `effect == Read` 로 가르면 앞으로 더해지는 `Read` 명령이 말없이 가둬진다. 목록은 데몬 쪽 상수 하나이고, 시험이 그 목록을 못 박는다(§5 U2).
- **B1 의 범위:** 표시가 명령 본문을 부르는 한 자리에 서므로 같은 명령이 제어 HTTP(`/control/agent` · `/control/call`)로 와도 본문은 같이 가둬진다. 그 HTTP 처리기 자체 · 다른 명령 · 2단계 전달(`daemon/src/command_delivery.rs:1820` 의 `route`)은 표시 밖이다. 처리기 패닉은 안쪽 `guard_panic` 이 먼저 `INTERNAL` 로 답하고, 그 바깥 얇은 구간(`now_or_never` 기계)의 패닉은 우리 `contain` 이 같은 `INTERNAL` 로 접는다 — 그래서 표시와 잡는 자리 사이에 빈틈이 없다. 이것은 1단계 경로의 셋째 그물이라 `command_delivery.rs:1397` 의 「세 번째 그물을 세우지 않는다」 주석을 U2 에서 고친다.
- **펌프 패닉 갈래의 자식 끄기(A1~A3):** 잡으면 `finish` 앞에서 자식 CLI 를 명시로 끈다(통로의 자식 손잡이 또는 무리 — 구현 때 고른다 · 표시 밖에서 돈다). 마지막 `Arc` drop 에 맡기면 Job 닫힘이 세션의 다른 보유자에 묶인다(§2). ★리뷰어 대립 — blind = 명시 종료 필수 / doc-aware = 마지막 Arc drop 으로 충분(다른 보유자는 ≤100ms 일시) → 자율 모드 보수 채택 · 사용자 재판정 대기★. 펌프 종류마다 U4 가 「자식이 사라지는가」를 잰다.

**표시 끔(`uncontained`) — 가둔 코드가 부르는 공유 상태 고침:** 그 안 패닉 = 즉시 종료.

| # | 자리 | 닿는 곳 |
|---|---|---|
| a | codex 리더의 첫 턴 포트 호출(`codex/transport.rs:3671` — `witness_first_turn` `:3641`) | 래치 commit(`agent/src/manager.rs:589-612`) → `ProfileRegistry::commit_session_id`(`agent/src/profile.rs:853`) — 공유 프로필 표 |
| b | `OutputCore::report_usage` 안 한 곳(`agent/src/output_core.rs:393-397` — stdio · codex 펌프가 부른다 `stdio.rs:262` · `:286` · `codex/transport.rs:3707` · `:3818`) | `UsageService::observe`(`daemon/src/usage_service/observe.rs:40-41` → `mod.rs:204`) — 계정 단위 공유 책 |
| c | 라이터의 덩이 뒤 부를 것(`input_queue.rs:298-309` — 그 안쪽 catch 째) | claude 잔여물 정리 `written`(`agent/src/backend/claude/leftover.rs:1857-1872`) — 문 `GateCell` 을 고친다 |
| d | claude decoder 의 문 여닫기(`agent/src/backend/claude/mod.rs:1200-1204` — A2 안) | `GateCell` 의 `open_turn` · `deliver` · `close_turn`(`leftover.rs:1927-1956`) |

- c · d 의 까닭: 잔여물 정리(ADR-0262)는 막 둘이 abort 에 기댄다(그 ADR 의 K7) — 가두면 그 전제가 깨진다. c 의 안쪽 catch 는 운영에서 닿지 않게 된다(시험 빌드엔 훅이 없어 산다).
- **표시를 끄지 않는 것:** 턴 관측 표 · 대기 목록 표(`agent/src/turn.rs:94` · `agent/src/inputs_pending.rs:21` — 펌프의 `emit` 이 부른다 `output_core.rs:432`)는 여러 에이전트 공유 `Mutex` 지만 임계 구역이 그 에이전트 자기 항목의 평범한 `HashMap` 조작이라 패닉원이 없다(리뷰 확인).

**가두지 않는 것(= 즉시 종료):** WS 명령 처리기 몸통 · 떼어 낸 명령 태스크(`daemon/src/agent_conn.rs:314` · `:375` · `:336` — 기본은 상태를 바꾼다고 본다 · 읽기 전용 WS 명령이 확인되면 그때 더한다) · 활성화(`daemon/src/connection_core.rs:1366` · `:1760`) · 허용 목록 밖 버스 명령(`usage.get` 포함) · 제어 HTTP · MCP 도구(`daemon/src/control/mcp_server.rs`) · codex 라이터와 그 세션 id 기록(`record_session_id` `codex/transport.rs:2393` · 그 catch `:2420` — `writer_loop` `:2484` 가 `:2521` 에서 부른다 · 위 a 와 다른 길) · PTY 감시 · stderr 배수 · claude 잔여물 정리 스레드 · 메시징 배달 · reaper · sweep · 세션 추적 · 사용량 · accept 루프 · 메인 future · net · rmcp · hyper 내부 · 셸 전부. 이 자리의 기존 `catch_unwind`(reaper `agent/src/reaper.rs:208` · sweep `daemon/src/lib.rs:729` 등)는 운영에서 닿지 않는 코드가 되지만 시험 빌드에는 훅이 없어 살아 있으므로 걷지 않는다(§4-6 ②).

### 4-3. 표시 — base 새 입주자 `panic`(가안)

- 스레드 지역 깊이 하나(const `Cell` · `try_with` — 스레드 해체 중에도 읽힌다 · 못 읽으면 「표시 없음」).
- 원시(이름 가안): `contain(f) -> Result<T, Caught>`(깊이 올림과 catch 를 한 몸으로) · `uncontained(f)`(그 안에서 깊이 0 · 끝나면 되돌림) · 페이로드 → 문장(지금 사본 일곱 — 10판 §2-2).
- ★**불변식 — 표시 범위 ⊆ 그것을 잡는 catch 클로저.**★ 스레드 머리에 따로 선 깊이 가드는 두지 않는다. 펌프의 `finish` 는 catch 밖(§2)이라 표시 없이 돈다 — 표시된 채 `finish` 가 패닉하면 잡을 이가 없어 그 에이전트가 끝 전이 없이 `Running` 으로 남고 CLI 도 산다(ADR-0005 · ADR-0290).
- **비동기 표시는 두지 않는다** — 가두는 자리가 전부 동기다(A1~A4 = 전용 스레드 · B1 = blocking 스레드). 나중에 async 자리를 가두면 스레드 지역 표시는 틀린다 — 그때 materialize 의 태스크 지역 표시(`CATCHING_UNWIND_ASYNC` · 그 저장소 `src/ore/src/future.rs`)를 더한다.
- **표시는 스레드를 따라가지 않는다** — 가둔 단위가 띄운 스레드 · 태스크는 표시 밖이다(= 즉시 종료 · 안전한 쪽).
- **왜 표시가 있어야 하나** — 훅이 `catch_unwind` 보다 먼저 돈다(§2). 표시 없이 「잡히지 않은 패닉 = 종료」 훅을 걸면 가두려던 패닉까지 끝낸다.
- 입주 조건(ADR-0269): 소비자 agent · 데몬 · 셸 + 페이로드 사본 · 도메인 지식 0 · 다른 입주자 무참조 — 게이트 ③ 정규식(CLAUDE.md 「빌드·검증 명령」 · `ci.yml` 사본)에 `panic` 을 더한다. command crate 는 base 를 못 부르므로(의존 0) B1 의 표시는 부르는 쪽(데몬)이 세운다.

### 4-4. 훅 — 데몬 · 셸이 같은 것을 설치

```
패닉 → 훅(패닉한 스레드 · 되감기 전)
  ① 사유 줄을 로그한다(스레드 · 문장 · 자리 — 지금 데몬 훅의 줄) · 이전 훅(std 기본 — debug 의 stderr)을 부른다
  ② 표시가 섰으면 돌아간다 → 되감기가 그 contain 까지 가서 실패로 접힌다((마))
  ③ 표시가 없으면 std::process::abort((사))
```

- 훅 몸통은 base `panic`(가안)에 두고 끝내기를 인자로 받는다(시험이 프로세스를 죽이지 않게). 설치 = 데몬 지금 자리(`daemon/src/lib.rs:458`) · 셸 `run()` 머리(`src-tauri/src/lib.rs:28` · `Builder` 앞 — 셸 로깅 초기화가 그 뒤 `src-tauri/src/state/boot_plugin.rs:89` 라 그 앞 패닉은 사유 줄 없이 끝난다).
- **훅은 표시와 무관하게 로그한다** — 표시 밖 갈래가 어차피 로그하므로 자기교착(§2)은 로그 쪽에서 닫아야 하고, 닫고 나면 표시 안에서 조용히 할 까닭이 없다. 패닉 자리(파일 · 줄)는 훅에서만 보이므로 `guard_panic` 처럼 잡고 로그하지 않는 자리의 기록도 이렇게 남는다. 가두는 자리는 패닉을 다시 로그하지 않고 실패 표면만 낸다.
- **자기교착은 로그 쪽에서 닫는다** — base `logging` 파일 싱크에 스레드 지역 「이 스레드가 파일에 쓰는 중」 표지를 두고, 섰으면 그 줄을 버린다. 닫는 것은 같은 스레드 재진입뿐이다(나머지는 §7).

### 4-5. 셸 · CLI · 이어질 단계

- **셸 = 훅만(사용자 결정 ② 로 정해짐).** 셸의 어떤 패닉이든 사유 로그 → 셸 즉시 종료 — 오늘과 같다. 데몬 · 에이전트는 계속 · 다음 셸 실행은 비정상 종료 복원을 묻는다. 셸의 `Read` 명령(`src-tauri/src/layout/commands.rs`)도 가두지 않는다(셸 버스 입구 `src-tauri/src/daemon_client/inbound.rs:217`). ★훅이 있어야 오늘과 같다★ — 훅 없이 unwind 로 바꾸면 배경 스레드 패닉이 그 스레드만 조용히 죽인다.
- **CLI = 훅 없음.** 단일 스레드라 unwind 면 main 패닉이 곧 프로세스 끝(종료 코드 101 · 기본 훅 stderr) — 오늘 abort 와 겉으로 같다(가능성 높음).
- 다음 단계가 transport crate(`crates/engram-dashboard-transport`)를 셸 연결 층에 붙이며 새 연결 · 펌프 태스크를 들인다 — 그 태스크는 들어올 때 §4-1 기준을 따른다(셸 쪽이면 가두지 않는다).

### 4-6. 확인 · 감사 거리 (결정 아님 — U4)

1. **허용 목록 두 명령의 본문이 정말 상태를 바꾸지 않는가** — 한 번 본다. 선언은 계약이고 어긋남을 코드가 못 잡는다(`daemon/src/control/commands.rs:286-291`).
2. **abort 전제 감사** — 표시 밖 코드는 「패닉 = 프로세스 끝」이 그대로 참이다(훅이 끝낸다). 전제가 무너지는 것은 가두는 다섯 자리의 몸통과 그 몸통이 되감기 때 도는 `Drop` 뿐이다. 볼 것:
   - base `sync` 계약 3(`base/src/sync.rs:10-11` — 「운영 빌드는 오염에 닿지 않는다」) — 펌프가 `sync` 로 락을 되찾으므로(`pty.rs:322` 등) 다시 쓴다.
   - **출력 코어의 오염 길** — `emit` 은 `replay` 를 쥔 채 적고(`agent/src/output_core.rs:422-429` · 목록 사건의 `.expect("한 줄 이상")` `:614` 도 그 안이다) 거기서 난 패닉은 `replay` 를 오염시킨다 → 표시 없는 `finish` 의 `replay.lock().expect`(`:692`)가 다시 패닉해 데몬이 끝난다(안전한 쪽).
   - **틀리게 되는 주석** — `pty.rs:283-297` 의 「(b) 그 재-panic 은 연결 task 안이라 tokio 가 그 task 만 격리」는 훅이 들어오면 거짓이다. 「release=abort」 정규식으로는 안 잡히므로 따로 적는다.
   - 「release=abort」를 글자 그대로 적은 주석 · 로그 문장(예: `daemon/src/messaging_host.rs:769`)은 「표시 밖 패닉 = 즉시 종료」로 고친다.
   - ADR-0262 K7 · 메시징 커널의 되돌림 `Drop` 은 가두지 않는 코드(또는 표시 끔 c · d)에서 돌아 전제가 그대로 참이다 — 범위 밖.

### 4-7. 프로필 전환

`Cargo.toml:40` `panic = "abort"` 한 줄을 지운다(기본 unwind). 앞 단위가 전부 abort 아래 무동작이라 되돌림 = 이 한 줄. 그래서 「abort 면 빌드를 멈추는」 컴파일 가드는 두지 않는다. 실행 파일 크기 변화를 이 단위에서 잰다(ADR-0288 「영향」이 재지 않았다고 적었다).

---

## 5. 단위

| 단위 | 바뀌는 것 | 배포판(abort) | 검증 |
|---|---|---|---|
| U1 원시 | base `panic`(`contain` · `uncontained` · 페이로드 문장 · 훅 몸통 — 아무도 안 부름) · `logging` 재진입 표지 · 게이트 ③ 이름 | 같다 | base 단위 시험 — 깊이 증감 · 중첩 · 되감기 뒤 복귀 · `uncontained` 안 깊이 0 과 복귀 · 훅 판정(끝내기 주입) · 재진입 표지 |
| U2 가두는 자리 | §4-2 — A1~A4 를 `contain` 으로 · A1~A3 패닉 갈래의 자식 끄기 · 표시 끔 a~d · B1 + 허용 목록 상수 · `command_delivery.rs:1397` 주석 | 같다(catch 가 안 잡고 패닉 갈래에 닿지 않는다) | 기존 펌프 · 라이터 패닉 시험이 그대로 초록 · `finish` 가 표시 없이 도는지 · a~d 안 깊이 0 · **허용 목록 시험**(목록 = 정확히 둘 · 둘 다 표에 있고 `Read` · 두 본문 안에서 표시가 서고 그 밖 명령에선 안 선다) |
| U3 훅 | 데몬 훅을 base 훅으로 바꾼다 · 셸에 설치 | 같다(로그 → abort) | 자식 프로세스 시험(§6). ★확인 거리★ — debug 데몬은 이때부터 tokio 가 격리하던 패닉에 끝난다: 종료 중 타이머 poll(`daemon/src/usage_service/mod.rs:320-322`) · 연결 태스크의 `debug_assert`(`daemon/src/agent_conn.rs:108`) 등. `/qa full` · `stop_smoke` 가 흔들릴 수 있어 그 둘을 먼저 돌려 본다(시험 서버엔 훅이 없어 단위 시험은 그대로) |
| U4 확인 · 감사 | §4-6 둘 — 고칠 자리 · 주석 · 로그 문장 | 같다 | 펌프 종류마다(PTY · stdio · codex) 패닉 → 자식 CLI 프로세스가 사라지는지 · 고친 자리마다 되감기 시험 |
| U5 전환 | `Cargo.toml:40` 한 줄 | **unwind** | release 빌드 + `/qa full`(실 창 · 실 데몬 정상 동작) · 크기 |

- **순서 고정:** U2 가 U3 보다 앞(뒤집히면 debug 에서 지금 가둬지던 펌프 패닉이 데몬을 끈다) · U4 가 U5 보다 앞. U2 의 agent 몫 · daemon 몫은 파일이 안 겹쳐 나란히 갈 수 있다.
- 어느 단위 뒤에 멈춰도 빌드 · 회귀가 초록이고 배포판은 오늘과 같다(위 「배포판」 칸).

---

## 6. 시험 전략

- **훅은 프로세스 전역이고, 표시 밖 갈래는 프로세스를 끝낸다** — 그래서 둘로 나눈다.
  - **프로세스 안:** 훅 몸통을 끝내기 주입으로 부른다(U1). 전역 훅을 실제로 갈아 끼우는 시험은 한 자물쇠로 줄 세운다 — `command/src/testing.rs:8-30` `with_quiet_panic_hook` 꼴(되돌리기는 `Drop` 이 아니라 잡은 뒤).
  - **자식 프로세스:** 시험 바이너리가 자기 자신을 환경 변수 하나와 `--exact <시험 이름>` 으로 다시 띄운다. 자식은 실제 훅을 설치하고 (a) 표시 밖 패닉 · `uncontained` 안 패닉 → 부모가 「성공 아님으로 끝남 + 로그 파일의 사유 줄」 (b) `contain` 안 패닉 → 「계속 돎」을 본다. 저장소에 이 꼴의 선례는 없다(새 꼴). 데몬 crate 에 둔다 — 그 crate 시험 줄은 이미 `--test-threads=4` 를 붙인다(CLAUDE.md 「병렬은 테스트 바이너리마다」).
- 가두는 자리의 시험은 debug 에서 돈다 — 시험 빌드는 이미 unwind 라 U2 부터 바로 잰다.
- **배포판의 패닉 길 자체는 cargo 시험으로 못 잰다** — 시험 · 벤치 타깃은 프로필의 `panic` 설정을 무시한다(가능성 높음 — Cargo 문서). unwind 로 바뀐 뒤엔 debug 와 같은 코드라 debug 자식 프로세스 시험으로 갈음하고, U5 는 release 정상 동작 실측만 한다.

---

## 7. 남는 것 · 범위 밖

**사용자 결정 ② 가 받은 값:** 상태를 바꾸는 요청 · 활성화 · `usage.get` 의 버그 = 데몬 즉시 종료(에이전트 전부 끝남 · 다음 기동이 이어받음). 지금 배포판과 같다.

**받는 잔여:**

- **허용 목록 명령의 본문이 나중에 상태를 바꾸게 고쳐지면** 그대로 가둬진다 — 허용 목록 시험은 이름만 못 박는다. 목록 상수 옆 주석이 그 계약을 적는다.
- **가둔 펌프의 오염 연쇄** — 출력 코어 락을 쥔 채 난 패닉은 표시 없는 `finish` 에서 데몬 종료로 번진다(§4-6 ②). 안전한 쪽이라 받는다.
- **팬아웃 seq 구멍** — 가둔 펌프가 `fan_out`(`agent/src/output_core.rs:351`) 가운데서 패닉하면 남은 구독자는 그 seq 를 못 받는다 → 기존 replay 회복 길로 돈다(그 에이전트는 어차피 `Failed`).
- **훅의 로그가 멎는 경우** — 패닉한 스레드가 tracing 내부 락(reload · `EnvFilter`)을 쥔 채 나면 훅이 멎는다(오늘 훅과 같다) · 다른 스레드가 로그 락을 쥔 채 멎으면 훅도 기다린다(모든 로그 호출과 같다).
- **Rust abort 갈래** — 이중 패닉 · 소멸자를 벗어난 패닉 · FFI 건넘 · 셸 WebView2 COM 콜백(미검) = ADR-0288 결정 4.
- **표시 누락 · 표시 끔 누락** — 새 가둠 자리가 표시 없이 들어오면 즉시 종료(안전한 쪽). 가둔 코드가 새로 공유 상태를 고치게 되면서 표시 끔을 빠뜨리면 그 반쯤 바뀐 상태가 남는다 — 리뷰가 본다.
- **부팅 중 결정적 패닉의 재기동 고리 · 셸 즉시 종료 뒤 트레이 아이콘이 마우스를 올릴 때까지 남음(추론)** — 오늘 abort 와 같다.

**넓힐 때:** 가두지 않는 자리를 가두려면 §4-1 넷을 다시 대 본다. 트리거 = 로그의 사유 줄에 그 자리가 실제로 나올 때. 상태를 바꾸는 자리를 가두는 날 락 오염 정책 · 여러 걸음 구간 장치(11판 초안 (가)(나) — `git show bedda3f3:docs/process/S21-panic-policy/trd.md`)가 다시 필요해진다.

**범위 밖:**

- 정상 종료(StopDaemon · 트레이 「데몬 끄기」)의 오염 내성 · 종료 시간 제한 · T-52 — 패닉이 그 길을 타지 않는다.
- ADR-0288 이 미룬 장치 — 셸 사유 고지 · 다음 부팅의 패닉 표시 · 셸 자동 재기동 · 에이전트 PTY 를 데몬 밖 프로세스로(거부됨).

---

## 8. ADR · 문서 후속

- **ADR-0292(예정)** — `/adr supersede --mode partial` 로 ADR-0288 결정 1 · 2(가둠을 좁힌다 — 사용자 결정 ②) · 결정 3 과 ADR-0290 결정 2(가두지 못한 패닉 = 사유 로그 → 즉시 종료 · 정상 종료 경로를 당기지 않는다)를 개정한다. 본문 = 가둠 기준(§4-1) · 가둠 표 · 허용 목록 · 표시 끔(§4-2) · 표시 불변식 · 훅 모양 · §3 답 둘 · S20 TRD §4 ⑨ 프로필 결정의 대체 · 근거 조사(§1). 시점 = U1 앞. 전제 = ADR-0288 ~ 0290 이 이 브랜치에 들어와 있을 것.
- base `sync` 헤더 계약 3 · CLAUDE.md 「빌드·검증 명령」의 base 게이트 ③ 줄 · step-log 한 줄 — 해당 단위와 같은 변경에서.
