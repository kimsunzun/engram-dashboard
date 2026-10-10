# TRD — 패닉 정책 구현: 가두는 입구 · 훅 종료 · unwind 전환 (S21)

> 상태: **6판(2026-10-11) — 리뷰 5회차(deep · codex BLOCK 1 · 리뷰어 둘 FIX → 잠정 BLOCK) 반영 · 새 사용자 결정 없음(전부 설계 · 문서 정정) · 사용자 결정 여섯 묶음 (가)~(바) 대기.** 코드는 한 줄도 바뀌지 않았다. 리뷰 항목별 대조는 §10(6판 §10-1 · 5판 §10-2 · 4판 §10-3 · 3판 §10-4 · 2판 §10-5).
> **6판 요지:** ① 셸 깨우기는 `app.exit` 를 부르지 않는다 — 주 스레드에 닫힘 하나를 게시(`AppHandle::run_on_main_thread`)해 트레이 정리(`catch_unwind` 안 · best effort) 뒤 `terminate_self(101)` · 게시가 실패하면 더 하지 않고 감시 시한이 끝낸다 → tauri `exit` 실패 갈래의 `process::exit` · tao 가 `Exit` 콜백 패닉을 삼키고 `process::exit(0)` 하는 길이 당겨진 길에서 빠진다 ② 셸 `RunEvent::Exit` 몸통 = `catch_unwind` · 끝에서 「잡혔거나 당겨졌으면 `terminate_self(101)`」 재확인(시작 실패 코드보다 앞) ③ 데몬 `main.rs` 의 당기지 않은 끝(`Ok` · `Err(1)`)도 끝내기 직전 `is_pulled()` 면 101 ④ 가둔 패닉 칸 = 문장 + 자리 · 깊이별 네 칸 · 경계가 읽으며 비운다 · 올리는 훅이 무장된 칸을 사유에 덧붙인다 ⑤ 진행 셈 SeqCst ⑥ 문장 정정 둘(되감는 중 `Drop` 안 중첩 패닉은 소멸자를 벗어날 때만 abort · 배포판 셸엔 stderr 가 없다) ⑦ ADR 번호는 supersede 실행 때 스크립트가 매긴다(전제 · 대안 — §8-1) · 0290 결정 끝 줄 개정은 무조건.
> **5판 요지:** ① 가둔 비오염 패닉은 훅이 판정 직후 돌아간다(로그 · 기본 훅 없음 — tracing 락을 쥔 채 난 패닉이 훅에서 자기를 기다렸다) · 자리는 스레드 지역 고정 칸에 베끼고 로그는 가둔 경계가 한다 ② 당겨진 끝은 전부 마지막 로그 뒤 `terminate_self(101)` — `process::exit` 밑 `ExitProcess` 는 감시까지 끝낸 뒤 DLL 분리 알림을 돈다(데몬 꼬리 · M6 = U5 · 셸 `Exit` 갈래 · 셸 `main.rs` 포착 = U6) ③ ADR-0292 = `/adr supersede --mode partial` 한 번(0288 · 0290 각 짧은 조항 · 상세는 본문) · 0290 결정 끝 줄(0290 ⑤) ④ `terminate_self` = `TerminateProcess` 뒤 잠듦 고리 · 그 밖 OS = `process::exit` 자리채움 ⑤ 거절 ② = 제어 라우트마다 `.layer`(맨 `route_layer` 걷음) ⑥ 깃발 뒤 StopDaemon 은 `shutdown_all` 없이 `Ok` ⑦ `wait_reason` 은 「올리는 훅 진행 중」 셈이 0 보다 클 때만 · 셸 5초 예산 ⑧ 「훅 코드는 깃발까지 할당 없음」 · std `payload_as_str` · 주 스레드 판정 = TLS 표지 ⑨ 셸 설치 = `run()` 머리 `Builder` 앞 ⑩ StopDaemon flush 손실 한 문장 · 감시 폴링 ≈1초 · M22 `AssertUnwindSafe` 근거.
> **4판 요지:** ① 셸도 훅 설치 직후 **따로 띄우는 감시 스레드**(데몬과 같은 모양 — 깨우기는 `app.exit(101)` 하나만 · tauri `AppHandle::exit` 의 실패 갈래가 부르는 스레드에서 로그 · 락 · `exit` 를 돌아 같은 스레드의 시한은 상한이 아니다) ② 감시의 끝내기 = platform 새 도우미 `process::terminate_self`(가안 — Windows `TerminateProcess`) · 꼬리 · M6 은 `process::exit(101)` 그대로 ③ 훅 순서 = **할당 없는 판정 → (셸) 표지 → 깃발 → 사유 문장 → 로그** · 꼬리는 `exit` 앞에서 사유를 최대 1초(가안) 기다린다 ④ 감시는 `cfg(panic = "unwind")` 빌드만 ⑤ (다) 거절: 제어 HTTP 는 **제어 라우트만**(MCP 는 도구 감싸개만) · WS 입구는 정리 꼴 명령을 거절하지 않는다 ⑥ M21 은 U4 에서 가둠 · 버림만, 사유 기록 · 당김은 U5 ⑦ M22 = 가드 대신 맨 `catch_unwind` → `Dropped` → `resume_unwind` ⑧ Amends 를 ADR 쌍마다 한 줄로(`/adr link` 한계) · ADR-0290 「영향」 셋째 줄 · 결정 3 · 문장 정정 여럿.
> **3판 요지:** ① 새 사용자 결정 **(바) 패닉 종료의 시한** — 권고 = 감시(데몬 = 훅 설치 때 띄우는 전용 스레드 · 셸 = 깨우기 스레드에 시한 — M19) ② (다) 거절을 **가장 바깥 입구 셋**으로 좁히고, 거절이 ADR-0290 결정 1 · ADR-0288 결정 3 을 일부 되돌린다는 것을 묻는 말에 넣었다 ③ 걸쇠를 주입받는 seam(시험이 프로세스 전역을 뒤집지 않게 — M20) ④ kill 길의 상태 싱크 가둠(M21) · `send_input` 스코프 가드(M22) · `run()` 지역값 drop(M23) ⑤ 셸 훅은 `MAIN_ESCALATED` 를 당기기 **앞**에 · PTY 감시는 사유를 자식 끄기 **앞**에 ⑥ U4 를 U2 **뒤**로(의존).
> **2판 요지:** ① 락 오염 정책을 메인 결정에서 **사용자 결정 (가)** 로 올렸다 — 1판의 P1 은 codex 통로처럼 일부러 범위 안에서 되찾던 자리를 전부 데몬 재시작으로 바꾼다 ② 1판이 놓친 **「표시 안에서 공유 상태를 여러 걸음에 걸쳐 바꾸는 구간」**(활성화 등록 창의 좀비 세션 등)을 (나)로 ③ 종료를 당긴 뒤 꼬리가 새 명령을 계속 받는 창을 (다)로 ④ net 은 포장 포트를 걷고 **net 자기 루프 = 3층** ⑤ 깨우기는 unpark 를 빼고 폴링으로 · 부팅 · 꼬리 패닉은 `main.rs` 최상위 포착 → `exit(101)` ⑥ U8 을 갈라 의미 재감사(U8a)를 프로필 전환(U7) **앞**으로.
> **결정 틀(재론하지 않는다):** ADR-0288 · ADR-0290(ADR-0289 는 통째 폐기) + 사용자 결정 2026-10-10 「unwind 범위 = 워크스페이스 전체 + 셸에도 같은 훅」. ★세 ADR 은 브랜치 `origin/v0.3.3/feat/panic-policy` 에 있고 이 브랜치(`v0.3.3/refactor/crate-boundaries`)에는 아직 없다★ — 이 TRD 를 커밋하기 전에 그 브랜치를 들이거나 함께 머지해야 링크가 선다. 구현 ADR 번호 = **ADR-0292**(예약 · 사용자 결정 2026-10-10) — 실제 번호는 supersede 실행 때 스크립트가 로컬 최대 +1 로 매긴다(전제 · 대안 = §8-1 · 6판).
> **이 TRD 가 대체하는 옛 기록:** TRD `docs/process/S20-command-bus/trd.md` §4 ⑨ 「릴리즈 프로필의 `panic = "abort"` 를 유지한다」(옛 사용자 결정). 그 절의 **규약**(명령 핸들러는 값으로 실패한다)은 그대로 살고, **프로필 결정과 그 귀결**(「릴리즈에서는 패닉 그물이 서지 않는다 — 그것이 의도다」)만 대체된다(§8-2).
> **배치 근거:** `docs/README.md` 「새 기능 **설계 착수** → `process/SN-name/`」. 경계 리팩터링(`S21-crate-boundaries/`)과 다른 주제다 — TRD A §0 · §1-3 이 「패닉 대 되찾기 정책 — 별 주제」로 뺐다. 스텝 번호는 `docs/process/step-log.md` 의 마지막 스텝 S21 을 잇는다(새 스텝은 사용자 결정이라 올리지 않는다).
> **표기:** 「실측」 = 트리 `2c25cdd`(이 브랜치 HEAD — 1판이 잰 `18e11e8` 뒤 코드 변경은 platform `fs.rs` 두 줄뿐이고 이 TRD 는 그 자리를 인용하지 않는다. 2판의 `파일:줄` 은 리뷰가 든 자리까지 다시 쟀다 · 3판이 더한 `파일:줄` 은 `b1abbb9`(`2c25cdd` 뒤 코드 변경 0)에서 · 4판이 더한 것은 `d73aaceb`(코드 변경 0)와 레지스트리의 tauri 2.11.3 원문에서 · 5판이 더한 것은 `85debb24`(코드 변경 0)와 패닉 정책 브랜치의 ADR-0290 원문에서 · 6판이 더한 것은 `bf3b74aa`(코드 변경 0)와 레지스트리의 tauri · tauri-runtime-wry 2.11.3 · tao 0.35.3 원문 · core 1.95 rlib 문자열 · adr 스크립트(`~/.claude/skills/adr/scripts/adr.mjs`)에서 쟀다) · 「사용자 결정」 = 사용자가 고를 것(겉으로 보이는 동작) · 「메인 결정(관행)」 = 안 보이는 내부라 메인이 정하고 보고할 것 · 「권고」 = 이 TRD 의 안(채택 전) · 「가안」 = 이름 · 모양 미확정 · 「미검」 = 돌려 보거나 원문과 대조하지 않은 것.
> 앵커: ADR-0288 · ADR-0290 · ADR-0001(kill 2동사) · ADR-0005(terminal 전이는 펌프 단독) · ADR-0006(락 순서) · ADR-0019(reaper 단일 소비자 · `shutting_down`) · ADR-0110(메시징 커널 무의존) · ADR-0127(턴 관측 정리 = 두 지점) · ADR-0129(net 경계) · ADR-0142(메시징 배달 · `in_flight_targets`) · ADR-0159(버스 패닉 답 = `INTERNAL` / `OUTCOME_UNKNOWN`) · ADR-0266 결정 2(platform 은 OS 의존 코드만) · ADR-0269(base 입주 조건) · ADR-0275 결정 1(base `sync` = 경고 없이 되찾기) · 조사 `docs/research/panic-policy-peers-2026-10-08.md` · `docs/research/stop-writes-after-internal-error-2026-10-10.md`(둘 다 패닉 정책 브랜치).

---

## 0. 결론 (먼저)

```
① 배포판을 unwind 로 바꾼다 — 워크스페이스 [profile.release] 한 줄(Cargo.toml:41). 가두는 자리가 사는 데몬 lib ·
     agent lib 에 컴파일 가드를 둬 abort 로 되돌리면 빌드가 멈춘다(cfg(panic = "unwind")).

② 「가두는 중」 표시 = 스레드 지역 깊이(const Cell · try_with). base 새 입주자 `panic`(가안)은 도메인 0 원시만 준다 —
     동기 contain · 비동기 Contained(poll 마다 깊이 · 가드 안에서 drop) · escalating(구간 안 깊이 0) ·
     is_poison_panic · 종료 걸쇠(SeqCst 깃발 + 첫 사유 + 첫 오염 자리). 「무엇을 종료로 올리나」 정책은 데몬 · 셸 조립부.

③ 훅(데몬 · 셸): 할당 없이 판정(표시 밖이거나 락 오염 패닉인가) → 가둔 패닉이면 문장 · 자리를 스레드 지역
     깊이별 칸에 베낀다 → 가둔 비오염이면 곧바로 돌아간다(로그 · 기본 훅 없음 — 로그는 가둔 경계가 · 5판 · 6판) → 올리면 [셸 주 스레드면 표지]
     → 깃발(락 · unpark 없음) → 첫 사유 문장(무장된 칸이 있으면 덧붙임 · 6판) → 로그(재진입 가드). 깨우기는 폴링 — 데몬 = accept 루프 select! 의 interval 팔이 깃발을 보고
     기존 종료 watch 에 true(StopDaemon 과 같은 신호) · 셸 = 깨우기 스레드가 park_timeout 고리로 보고 주 스레드에 닫힘 하나를
     게시(run_on_main_thread — 트레이 정리 뒤 terminate_self(101) · app.exit 는 부르지 않는다 · 6판).

④ 데몬 끝: 기존 꼬리(shutdown_all → flush → MCP) → 올리는 훅이 도는 중이면 사유를 최대 1초 기다려 로그 → 당겨졌으면
     platform terminate_self(101) — 런타임 drop 의 무한 대기(blocking 태스크)와 ExitProcess 의 DLL 분리 알림(감시까지
     먼저 끝낸다)을 건너뛰는 load-bearing 한 줄(5판). 부팅 · 꼬리 자체의 패닉은 main.rs 최상위 포착 → terminate_self(101)
     · 셸의 당겨진 끝(깨우기가 게시한 닫힘 · Exit 콜백 끝 재확인 · main.rs 포착 — 6판)도 같다. 당기지 않은 끝은 process::exit
     그대로 — 단 데몬 main.rs 는 그 직전에도 깃발을 다시 보고 섰으면 101(6판). 패닉 종료가 시한 안에 안 끝나면 훅 설치 직후 띄운 감시 스레드(데몬 · 셸 각 하나)가
     platform 즉시 끝내기(TerminateProcess · 코드 101)로 끝낸다 — (바) · 가안 데몬 30초 · 셸 5초.

⑤ 1층 = 데몬 처리기 몸통(WS 처리기 · 명령 태스크 · blocking · 제어 HTTP · MCP 도구 몸통). net 자기 루프 · 연결 수명
     · rmcp · hyper 내부 = 3층(→ 정상 종료). 2층 = (가)의 답. 여러 걸음 공유 변경 구간 = (나)의 답.

⑥ 단위 U1~U9 — 순서 = 표시 · 되찾기(U1~U4) → 의미 재감사(U8a) ∥ 훅(U5 · U6) → 프로필(U7) → 주석 · 문서(U8b · U9).
     어느 단위 뒤에 멈춰도 빌드 · 회귀가 초록이고 배포판 동작이 지금보다 나빠지지 않는다(§7-2 에서 다시 쟀다).
```

**사용자 결정 대기 — 묻는 순서(§3-1):**

| 순서 | 묶음 | 권고 | 뒤 묶음에 주는 영향 |
|---|---|---|---|
| 1 | **(가) 락 오염 정책 + 2층 경계** — 어떤 버그가 에이전트 하나로 끝나고 어떤 버그가 데몬 재시작이 되나 | **P2 + B1′** | U2 · U4 크기 · (마)의 2층 표면 개수. (나) · (다) · (라) · (바)의 선택지는 안 바뀐다 |
| 2 | **(나) 여러 걸음 공유 변경 구간** — 에이전트 시작 · 수거 · kill · 연결 붙이기 · 떼기 도중 패닉 | **E 올림**(그 구간 패닉 · 스레드 기동 실패 = 정상 종료) | U4 크기 |
| 3 | **(다) 종료를 당긴 뒤 새 일** — 꼬리가 도는 몇 초~수십 초 | **새 일 거절**(가장 바깥 입구 셋 — ADR-0290 결정 1 · ADR-0288 결정 3 을 일부 되돌린다) | 없음 |
| 4 | **(라) 셸 패닉** | **S1a 셸만 끈다 · 셸 안 가둠 없음**(버스 명령 패닉은 `INTERNAL` 답 · 창 닫힘 **둘 다**) | (S1a′ 면 셸에도 컴파일 가드 + 셸 오염 정책) |
| 5 | **(마) 1 · 2층 고지 모양** | **N1a 오류 답 · 연결 유지 + N2a 지금 표면** | 없음 |
| 6 | **(바) 패닉 종료가 멎을 때의 시한** — 되감기가 들이는 멎음(지금 abort 엔 없다) | **감시 시한 — 데몬 · 셸 둘 다**(가안 30초 · 5초 → 즉시 끝내기 · 코드 101) | U5 · U6 크기(몇 줄 — U1 의 platform 도우미는 당겨진 끝이 어느 답이든 쓴다 · 5판) |

**메인 결정(§3-2 — 권고 기본값을 적었다):** 표시 모양 · 집 · 훅 신호(폴링) · 로그 재진입 · 사유 기록 · 가둔 패닉의 진단 칸(M4 — 5판 · 6판 = 문장 + 자리 · 깊이별) · net(N1) · 메인 future(M6 — 6판 = 당기지 않은 끝도 깃발 재확인) · 종료 코드 · 당겨진 끝 = `terminate_self`(M7 — 5판 · 셸은 6판 = 게시 닫힘 · `Exit` 끝 재확인) · 시험 방아쇠 · 회귀망 · 컴파일 가드 · 크기 · T-52 · 종료 경로 오염 내성 · 스폰 경로 스코프 가드 · `Contained` drop · 관찰자 계약 · 셸 끝내기 절차(M18 — 6판 = `app.exit` 걷음) · abort 전제 코드 · 감시 시한 모양(M19 — (바)=감시일 때) · 걸쇠 주입 seam · kill 길 상태 싱크 · `send_input` 패닉 포착 · `run()` 지역값 drop.

**지금 하지 않는 것(ADR-0288 이 미룬 장치 그대로):** 종료 시간 제한(StopDaemon 꼬리의 무한 대기 포함 — §4-11. 패닉 종료의 시한만 (바)가 묻는다) · 셸의 사유 고지 · 다음 부팅의 패닉 표시 · 셸 상자 · 셸 자동 재기동 · 에이전트 PTY 를 데몬 밖 프로세스로(거부됨). 그리고 이 TRD 가 미루는 것: 정밀한 「공유 락을 쥔 채 났나」 추적(P3).

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
| P1 | 어떤 락이든 오염 = 데몬 정상 종료 | 에이전트 · 요청 하나로 끝나는 것은 **락을 하나도 안 쥔 채 난 패닉뿐**. codex 에이전트 버그 대부분 · 순서 락을 쥔 요청 패닉(다음 입력 때)도 재시작 | 작음 — 되찾기 자리가 관찰자를 부른다(종료 경로만 따로 — 메인 M14) · 유령 대기 입력(†) |
| P1o | P1 + 데이터 없는 순서 락(`Mutex<()>` — 운영 셋 `session.rs:74` · codex `:1097` · `manager.rs:694`)은 오염을 무시 | P1 에서 「순서 락 함정」(몇 분 뒤 남의 입력이 데몬을 끈다)만 빠진다. 대가 — 순서 락이 지키던 여러 걸음(래치 commit → encode → send → emit)이 끊긴 것을 알릴 신호가 사라진다 | 작음 — 타입으로 묶인 함수 하나(`Mutex<()>` 만 받는다) · 락 셋의 호출 자리만 바꾼다 · 유령 대기 입력(†) |
| **P2** | 공유 락(명부 · 프로필 · 제어 레지스트리 · 메시징 표 · 사용량 · 추적기) 오염 = 정상 종료 · **한 에이전트 화신 또는 한 연결이 소유한 락**(세션 · 출력 코어 · 통로 · codex 상태 · leftover · 래치 · 입력 큐 · 순서 락 · 연결 사용량 출구 · 연결 구독)의 오염 = 그 범위 안에서 되찾고 계속 | 한 에이전트의 버그 = 그 에이전트만(펌프 → `Failed` · 라이터 → 입력 거절 · 연결 → 오류). **공유 락을 쥔 채 났을 때만** 재시작. 대가 — 요청이 한 에이전트의 상태를 반쯤 바꾸다 죽으면 **그 에이전트는 계속 돈다**(이상하게 굴 수 있다 — 끄고 다시 켜면 된다 · 다른 에이전트의 상태와 저장된 공유 상태는 안 건드린다 — ★단 그 에이전트로 가는 우편은 멈출 수 있다(†)★) | 중간 — 되찾기 함수 두 갈래(관찰 · 범위) · 범위 쪽으로 옮길 자리 ≈ 되찾기 60 + `.expect` 40(대부분 파일 단위 기계 치환 — codex 통로 42 · `output_core.rs` 27 · `pty.rs` 6 …). 분류 안 된 자리 = 관찰(종료 쪽 — 안전한 쪽) · 유령 대기 입력(†) |
| (P3) | 「공유 락을 쥔 채 났나」를 스레드 지역 카운터로 추적 | P2 와 같은 결과를 정밀하게 | 큼 — 공유 락 타입을 전부 바꾼다. 미룬다(트리거 = P2 분류 오류가 실제로 관측될 때) · 유령 대기 입력(†) |
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
- **† 모든 안의 공통 대가 — 유령 대기 입력** — `send_input` 이 `emit(Queued)` 를 낸 뒤(`session.rs:303-311`) 가둔 패닉이 나면 `Dropped` 가 안 나가 대기 입력이 유령으로 남는다 → 대기 목록 표(`inputs_pending`)가 그 에이전트를 「바쁨」으로 세우고 그 칸엔 30분 fail-open 이 없다(사용자 결정 N8 · N11) → **그 에이전트로 가는 우편이 ✕ 나 데몬 재시작 때까지 멈춘다.** 원인이 가둔 패닉 자체라 오염 축의 어느 안도 막지 못한다. 설계 포착(`send_input` 을 맨 `catch_unwind` 로 감싸 `Dropped` 를 낸 뒤 다시 풀어 보낸다 · §4-7 M22 · 4판)이 좁히고, 못 찾은 같은 꼴 자리는 잔여(§4-11)다.
- **묶임** — 이 답이 U2 · U4 의 크기를 정하고(P2 = 기계 치환 ≈ 100곳 · B2 = U4b 생김), (마)의 2층 표면이 B1′ · B2 에서 늘어난다. (나) · (다) · (라) · (바)의 선택지는 바뀌지 않는다.

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
| **E 올림(권고)** | 위 구간의 버그 = **데몬 정상 종료**(에이전트 전부 끝남 · 활성화로 이어받기). 그 요청 자체는 오류 답을 받는다. ★버그만이 아니다★ — 활성화 창 안의 자원 실패(스레드를 못 만들 때 — 감시 기동 `.expect` `pty.rs:265` · 펌프 맨 `thread::spawn` `:269`)도 데몬 재시작이 된다. 지금 debug 빌드는 그 패닉을 활성화 blocking 의 `JoinError` 로 받아 `SpawnFailed` 로 답한다(`connection_core.rs:1366-1373`) | 작음 — 구간을 `escalating(…)`(깊이 0)으로 감싼다 · 구간당 ≈ 5~10줄 + 시험 |
| R 되돌림 | 그 작업 하나만 실패(시작 실패 · 수거 재시도 등) · 데몬 계속 | 큼 — 걸음마다 실패 가능하게 + 스코프 가드로 되돌림(명부 remove · 통로 shutdown · 프로필 두 번째 쓰기 · reaper 재시도) ≈ 150~250줄. ★reaper 고리(`reaper.rs:208` — 지금 표시 없는 catch)에 2층 표시 + 재시도가 있어야 선다★ — 없으면 `reap_one` 패닉이 표시 밖이라 훅이 어차피 올린다. 되돌림 코드가 되감기 도중 다시 패닉하면 abort |
| (그대로 가둠) | 좀비 세션 · 낡은 목록 줄이 조용히 남는다 | ADR-0288 결정 3 과 어긋난다 — 권고하지 않는다 |

- **권고 E** — 이 구간들은 「공유 상태를 바꾸다 난 패닉」의 정의 그대로라 ADR-0288 결정 3 이 이미 답을 줬다. R 은 실제 패닉이 관측된 구간부터 하나씩(트리거 = 로그의 사유 줄에 그 구간 위치가 찍힐 때).
- **둘 다에 붙는 것(메인 결정 M15):** 활성화 창에서 패닉 · 오류로 빠질 때 아직 펌프에 안 넘긴 통로를 drop 하지 않고 `transport.shutdown()`(kill 2동사 순서 — ADR-0001)을 부르는 스코프 가드. 지금은 drop 만 한다(통로에 `Drop` 없음 — Windows 에선 `GroupOwner` drop 이 Job 을 닫아 트리를 끄지만(`platform group/windows.rs:601` · `:153`) master 를 무리보다 먼저 닫는 순서는 ADR-0001 과 다르다).
- **서명 받을 잔여:** 이 목록은 찾은 구간뿐이다 — 표시 안에서 공유 상태를 락 밖으로 여러 걸음 바꾸는 **못 찾은** 구간은 반쯤 바뀐 상태로 산다(§4-11 첫 항목). E · R 어느 쪽이든 이 잔여를 받아들이는 것이 이 묶음 답의 일부다.

#### (다) 종료를 당긴 뒤 새 일 — 「꼬리가 도는 동안 들어온 명령」

**무엇이 문제인가:** 깃발이 서면 accept 루프는 새 연결을 안 받지만, **이미 붙은 WS 연결 · MCP HTTP 는 꼬리가 끝날 때까지 계속 처리된다** — `shutdown_all`(에이전트마다 `join_pump` 5초 · 병렬) → flush(5초 띠 둘) → MCP 종료(시한 없음 — §2-5). 그 몇 초~수십 초 동안 들어온 명령은 깨졌을 수 있는 메모리에서 `agents.json` · 프리셋을 쓸 수 있다. ADR-0288 이 종료를 고른 까닭(「반쯤 바뀐 메모리 상태가 저장으로 디스크에 실리면 멀쩡한 `agents.json` 까지 덮인다」)이 이 창에서 살아난다. ADR-0290 이 「저장 멈춤도 필요 없다 — 프로세스가 끝나므로」라 적은 전제는 이 창을 몰랐다.

| 안 | 사용자가 겪는 것 | 비용 |
|---|---|---|
| 받아들임 | 그 창의 명령은 평소처럼 처리 · 저장된다 | 0 |
| **새 일 거절(권고)** | 깃발이 선 뒤 들어온 새 명령 · 조회 · 도구 호출은 오류 답(「daemon is shutting down after an internal error」 — 가안 · 영어 사유 규칙) · 이미 돌던 것은 끝까지 간다 | 작음 — 가장 바깥 입구 셋(아래)에서 깃발 확인 ≈ 20~30줄 + 시험 5(입구마다 하나 · 거절 안 됨 둘 — 정리 꼴 명령 · MCP 세션 요청 · 4판) |

- ★**거절은 정해진 틀을 일부 되돌린다 — 그래서 사용자 결정이다**★ — ① ADR-0290 결정 1 「저장 멈춤은 하지 않는다(프로세스가 끝나므로)」를 꼬리 창에 한해 되돌린다(새 일 거절 = 그 창의 저장 멈춤) ② ADR-0288 결정 3 「이 단계를 위한 전용 장치는 만들지 않는다」에 패닉 종료에만 쓰는 확인 장치 하나를 더한다. 거절을 고르면 ADR-0292 를 만드는 `/adr supersede --mode partial` 의 두 대상 조항과 ADR-0292 본문에 적는다(§8-1 — 조건부 · (바)=감시도 ADR-0288 결정 3 을 건드리므로 같은 조항에 든다 · ADR-0290 결정 끝 줄 「ADR-0288 의 나머지(… 3 · 4 의 전용 장치를 두지 않는다 · 미룬 장치)는 그대로다」에 그 몫이 더해진다 — 그 줄의 개정 자체는 답과 무관하게 늘 든다 · 0290 ⑤ · 6판). 받아들임은 두 ADR 그대로이고, 대가는 위 창이 열린 채 남는 것이다.
- **거절 범위 = 가장 바깥 입구만** — WS 명령 해석 직후(`on_text` — 옛 명령 · 버스 명령 · ★정리 꼴 명령(StopDaemon · 구독 해지 · 뷰포트 놓기 꼴 — 목록은 U3 가 명령 어휘에서 고른다)은 거절하지 않는다★) · 제어 HTTP 의 **제어 라우트만**(`ControlRoute::ALL` — `agents.json` 을 쓰는 `agent.*` 동사도 이 길로 든다 · ★앱 전체가 아니다★: MCP 의 `nest_service`(`mcp_server.rs:1372`)도 같은 `.layer`(`:1401-1407`) 아래라 앱 전체에 두면 rmcp 세션 DELETE · SSE 끊기까지 거절한다 · §4-6) · MCP 도구 감싸개(MCP 의 거절은 여기 한 곳 — N1a 도구 오류 결과). **안쪽 홉(배달 태스크 → 1단계 blocking 등) · `on_connect` · `on_disconnect` · Close · 정리 걸음에는 두지 않는다** — 이미 받은 일을 가운데서 끊거나 정리를 막는다.
- **권고 거절** — 종료의 존재 이유(반쯤 바뀐 메모리가 디스크에 실리지 않게)를 지키는 가장 싼 길이다. 위 두 ADR 을 일부 되돌리는 값이 그보다 크다고 보면 받아들임이 맞다. StopDaemon 은 WS 입구의 허용 목록이라 깃발 뒤에도 거절되지 않는다 — ★깃발 뒤에는 `shutdown_all` 을 다시 돌리지 않고 `Ok` 로 답한다(꼬리가 진다 · `shutdown_all` 은 재진입 막이가 없다 `manager.rs:3113` · M12 · §4-4 · 5판)★. 깃발 앞에 온 StopDaemon 은 지금처럼 `shutdown_all` 을 먼저 끝내고 신호를 보낸다.

#### (라) 셸 패닉

셸은 「같은 훅」이다(사용자 결정 2026-10-10). 남은 것은 훅이 당긴 뒤 셸이 **무엇을** 하고, 셸 안에 「요청 하나」 가둠을 둘지다.

| 안 | 동작 | 사용자가 겪는 것 | 비용 |
|---|---|---|---|
| **S1a(권고)** | 셸만 끈다 · 화면 상태 `Final` 은 안 쓴다 · **셸 안 가둠 없음** | 셸의 어떤 패닉이든 창이 닫힌다 — 데몬이 보낸 **버스 명령 처리 중 패닉이면 둘 다 일어난다**: `command::route::guard_panic` 이 `INTERNAL` 로 답하고, 훅(셸은 표시가 없다)이 셸을 닫는다 · 데몬 · 에이전트는 그대로 · 다음 셸 실행 = 비정상 종료 복원 물음(`state.crash.json`) | 작음 |
| S1a′ | S1a + 셸 버스 명령 처리를 1층으로 가둔다(`TaskSpawner` 태스크를 `Contained` 로) | 버스 명령 패닉 = `INTERNAL` 답 · 창 유지. 그 밖은 S1a 와 같다. 대가 — 그 명령이 화면 모델을 반쯤 바꾼 채 셸이 계속 돌고, 다음 정상 종료가 그 상태를 `Final` 로 싣는다 · ★그 패닉이 락을 오염시켰으면 그 뒤 명령이 조용히 실패하고 `Final` 이 끝내 안 쓰일 수 있다(아래)★ | 중간 — +20줄 · 셸 컴파일 가드 **+ 셸 오염 정책**(아래) |
| S1b | S1a 인데 `Final` 을 쓴다 | 다음 실행이 말없이 복원 — 패닉 순간의 화면이 「정상」으로 저장될 수 있다. 주 스레드 패닉은 `Exit` 사건이 없어 어차피 못 쓴다(§2-7) — 스레드에 따라 결말이 갈린다 | 작음 |
| (미룸) | 상자 뒤 종료 · 자동 재기동 | — | ADR-0288 이 복원 시스템과 묶어 미룬 「셸 사유 고지」 무리라 지금 안 올린다 |
| (권고 안 함) | 데몬도 끈다(`quit_app` 그대로) | 에이전트 전부 끝남 | 「패닉 종료 최소화」와 정면으로 어긋난다 |

- **권고 S1a** — 셸 패닉은 셸 메모리(화면 모델)를 믿을 수 없게 만든다. 셸은 상태가 사실상 화면 모델 하나라 요청과 공유 상태가 갈리지 않고, 셸 재시작이 싸다(데몬 · 에이전트가 산다). 어느 스레드에서 났든 다음 실행이 같은 물음을 낸다.
- **S1a′ 의 숨은 대가 — 셸 오염 정책이 따로 필요하다** — 셸 운영 코드는 락 오염을 `.lock().map_err(…)` 로 오류 값으로 바꿔 삼킨다(28곳 — `layout/apply.rs` 24 · `state/restore.rs` 2 · `state/boot_plugin.rs` 1 · `theme.rs` 1 · 실측). 훅도 관찰자도 거치지 않으므로, 가둔 패닉이 화면 락을 오염시키면 셸은 그 뒤 명령마다 조용히 실패하며 계속 돌고 `Final` 이 끝내 안 쓰일 수 있다. (가)는 데몬만 다루므로 S1a′ 를 고르면 셸 몫 오염 정책(이 자리들의 분류 · 관찰자 연결)이 함께 선다. S1a 에서는 셸 패닉이 곧 종료라 이 자리들이 문제 되지 않는다.

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

#### (바) 패닉 종료가 멎을 때의 시한 — 「종료가 끝나지 않으면」

**무엇이 문제인가:** 지금 배포판(abort)은 패닉 순간 프로세스가 즉사한다 — 멎을 틈이 없다. unwind 에서는 패닉 뒤에도 훅 · 되감기 · 꼬리가 돌고, 그 사이 멎으면 데몬 · 셸이 **돌지도 죽지도 않는 채** 남는다. 리뷰가 찾은 멎음:

| # | 자리 | 무엇이 멎나 |
|---|---|---|
| (1) | 셸 주 스레드 패닉 + 훅 로그 자기교착 | 로그 파일 잠금(`base/src/logging/mod.rs:101` — 알려진 미수정 `:110-115`)이나 tracing 내부 락(§4-11)을 쥔 채 패닉하면 훅의 로그가 자기를 기다린다 → 훅이 안 끝나 tao 가 되던지지 못하고, 깨우기는 주 스레드 패닉이면 손을 대지 않는다(§4-9) → **셸이 끝나지 않고 비정상 종료 복원도 그만큼 늦어진다** |
| (2) | 셸 주 스레드가 막힘 | 주 스레드가 패닉한 스레드를 기다리며 막혀 있거나, 주 스레드 패닉이 `App::run` 밖으로 풀리다 WebView2 · COM 객체의 Drop 에서 멎는다 → 깨우기(주 스레드에 게시하는 닫힘 — 6판 · 3~5판은 `app.exit`)가 아무것도 못 한다 |
| (3) | 데몬 로그 락 자기교착 | 패닉한 스레드가 tracing 락을 쥔 채 훅에서 멎으면 그 락을 기다리는 모든 로그 호출이 막힌다 — accept 루프 · 꼬리의 `info!`(예: `lib.rs:832`) 포함 → 깃발은 섰는데 종료가 멎고, 단일 인스턴스 잠금 · `daemon.json` 을 쥔 채 남는다 |
| (4) | MCP 종료 | `McpServerHandle::shutdown`(`control/mcp_server.rs:351` · 꼬리 `lib.rs:830-831`)의 시한 없는 await(§4-11) |

**왜 사용자 결정인가:** 종료 시간 제한은 ADR-0288 이 미룬 장치다 — 「근거」 「3 · 4 의 전용 장치(종료 시간 제한 · …)는 사용자가 『나중에 — 복원 시스템과 연관』으로 미뤘다」 · 「영향」 「시간 제한을 들이는 결정이 아니라 …」. 메인이 그 미룸을 뒤집지 않는다.

| 안 | 사용자가 겪는 것 | 비용 |
|---|---|---|
| 미룸 유지 | (1)~(4)가 나면 데몬 · 셸이 사람이 프로세스를 끌 때까지 멎은 채 남는다 — §4-11 에 받는 잔여로 적고 서명받는다 | 0 |
| 셸 시한만 | 셸의 (1) · (2)는 시한 뒤 끝난다(코드 101). 데몬의 (3) · (4)는 남는다 | 작음 |
| **감시 시한 — 데몬 · 셸 둘 다(권고)** | 깃발이 선 뒤 시한 안에 안 끝나면 그 자리에서 끝난다(코드 101) — 가안 **데몬 30초**(꼬리 = 병렬 `join_pump` 5초 + flush 5초 × 2 + MCP 를 덮는다) · **셸 5초**. 시한 안에 끝나면 지금 설계 그대로 · 시한으로 끝나도 `Final` 은 안 쓴다 · 깃발 없는 StopDaemon · 트레이 「데몬 끄기」에는 서지 않는다 — ★단 그 꼬리 도중 깃발이 서면(관찰자가 앞선 오염을 되찾음 · 표시 밖 정리 패닉 · M21) 그 끄기도 30초 상한 · 코드 101 을 받는다(같은 동작 — 깃발이 서면 어느 종료든 같다)★ · StopDaemon 중 깃발이 서고 flush 전 단계가 30초를 넘기면 그 flush 를 잃는다 — 깃발 없는 끄기도 멎으면 같다(5판 · 쓰기 도중 끊김은 안전하다 — base 원자 쓰기 · 임시 파일은 다음 부팅이 쓴다 · 로그 줄 중간 잘림은 받는다) | 작음 — 데몬 · 셸 각 몇 줄 + platform 도우미 하나 + 시험 1 |

- **모양(권고)** — 데몬 · 셸 모두 **훅 설치 직후 따로 띄우는 감시 스레드** 하나가 깃발을 본 뒤 시한을 기다리고, 아직 살아 있으면 platform 즉시 끝내기(가안 `process::terminate_self(101)` — Windows `TerminateProcess` · M19 · §4-4)를 부른다. 깨우기(데몬 = accept 루프 interval 팔 · 셸 = 깨우기 스레드)와 감시는 다른 스레드다 — ★셸 깨우기에 시한을 얹던 3판 모양은 상한이 아니었다★: tauri 2.11.3 `AppHandle::exit`(`app.rs:574-579`)는 `request_exit` 실패 갈래에서 `log::error!`(log → tracing 다리 — `base/src/logging/mod.rs:422` 의 `try_init`) → `cleanup_before_exit()`(`tray.icons.lock().unwrap()` `app.rs:1110`) → `process::exit` 를 **부르는 스레드에서** 돈다. (6판 — 깨우기는 이제 `app.exit` 자체를 부르지 않는다 · 주 스레드 게시 · §4-9. 감시를 따로 두는 까닭은 그대로다 — 깨우기 · 주 스레드가 멎어도 끝은 감시가 진다.) 감시는 로그 · 락을 쓰지 않는다 — (1) · (3)의 멎음이 로그 락이다. `process::exit` 를 쓰지 않는 까닭 = §9-12(5판 — 당겨진 끝도 전부 같은 즉시 끝내기다 · §4-4). 감시는 `cfg(panic = "unwind")` 빌드에서만 띄운다(abort 빌드는 패닉 = 즉사라 할 일이 없다).
- **권고 감시 시한 둘 다** — 이 TRD 의 원칙 「어느 단위 뒤에 멈춰도 … 배포판 동작이 지금보다 나빠지지 않는다」(§0 ⑥)를 지키는 길이다. 멎음은 지금의 즉사보다 나쁘다 — 즉사는 끝나기라도 하지만 멎은 프로세스는 자리(단일 인스턴스 잠금 · `daemon.json` · 셸 창)를 쥔 채 남는다. 셸만은 데몬 쪽 (3) · (4)를 남긴다.
- **채택하면** ADR-0292 가 ADR-0288 의 미룬 장치(위 「근거」 · 「영향」 두 줄 — 종료 시간 제한)와 **결정 3**(「이 단계를 위한 전용 장치는 만들지 않는다」 — 감시는 정상 종료 단계에 쓰는 전용 장치다)도 개정으로 적고, ADR-0290 결정 끝 줄(「ADR-0288 의 나머지(… 미룬 장치)는 그대로다」)에도 그 몫을 적는다(§8-1 0288 ③ · ④ · 0290 ⑤ — 그 줄의 개정은 늘 들고(6판 — 0292 가 늘 0288 결정 1 을 좁힌다) 감시 몫이 거기 더해진다 · (다)=거절의 몫과 같은 supersede 조항에 든다). 미룸 유지면 (1)~(4)가 §4-11 의 서명받을 잔여다.
- **묶임** — 없음. 앞 묶음의 답이 이 선택지를 바꾸지 않는다.

### 3-2. 메인 결정(관행) — 권고 기본값

> 번호는 1판을 잇는다 — 1판 M2(락 오염 정책)는 사용자 결정 (가)로 올라가 비었고, M14~M18 은 2판에서 · M19~M23 은 3판(리뷰 2회차)에서 새로 섰다. 4판(리뷰 3회차)은 새 번호 없이 M4 · M9 · M18~M23 을 고쳤다. 5판(리뷰 4회차)도 새 번호 없이 M4 · M6 · M7 · M12 · M18 · M19 · M22 를 고쳤다. 6판(리뷰 5회차)도 새 번호 없이 M4 · M6 · M7 · M16 · M18 · M19 를 고쳤다.

| id | 거리 | 기본값(권고) · 까닭 |
|---|---|---|
| **M1** | 표시 모양 · 집 | **base 새 입주자 `panic`(가안)** — 스레드 지역 깊이(`thread_local!` + `const { Cell::new(0) }` — const 초기화 · Drop 없는 칸은 해체되지 않아 `with` 가 실패하지 않는다. `try_with` 를 써도 무해하나 그 실패 갈래는 죽은 길이다 · 3판 정정) + `contain` · `Contained` · `escalating` · `payload_message` · `is_poison_panic` · 종료 걸쇠. ★**정책 함수(`verdict` — 「오염이면 표시와 무관하게 올린다」)는 base 에 두지 않는다**★ — 제품 정책이라 ADR-0269 입주 조건 ②(도메인 지식 0)와 긴장한다. base 는 원시만, 조립(정책)은 데몬 · 셸이 한다(각 ≈ 15줄 · 사본 둘은 각자 시험). 집 대안 — platform(OS 의존이 아니라 ADR-0266 결정 2 위반) · command(버스 도구 정체성 밖) · crate 마다 사본(입주 조건 ① 이 막으려는 것) |
| **M3** | 훅 신호 · 깨우기 | **깃발 = `AtomicBool` 하나에 `store(true, SeqCst)` — unpark 없음.** 깨우기 = 폴링: 데몬은 accept 루프 `select!` 에 `interval`(가안 200ms) 팔 하나 — 깃발이면 `shutdown_tx.send(true)`(`AgentConnections` 로 옮기기(`lib.rs:383`) 전에 뜬 사본 — 그 watch 의 운영 구독자는 루프 자신뿐이라 「같은 신호」를 한 길로 두는 몫 · §4-4) + 루프 탈출 · 셸은 깨우기 스레드가 `park_timeout(200ms)` 고리로 깃발을 본다. 까닭: 1판의 「깃발 쓰기 → 대기자 읽기 / 대기자 등록 → 깃발 읽기」는 SeqCst 울타리 없이는 store-buffering 으로 깨움을 잃는다 · `Thread::unpark` 는 OS 호출이라 훅 · 관찰자(임의의 락을 쥔 채 돈다)에서 부르면 leftover 잎 락 불변식(OS 호출 금지 — CLAUDE.md 핵심 불변식)을 깬다 · 깨우기 스레드 기동 실패 · 둘째 대기 · 스레드 죽음 = 「당겼는데 안 끝남」. 거부: watch 를 훅이 직접 `send`(내부 RwLock — 패닉한 스레드가 `watch::Ref` 를 쥐었으면 자기 교착) · tokio `Notify`(셸에 못 씀) |
| **M4** | 로그 재진입 · 사유 기록 | **L1** — `FileSink::make_writer`(`logging/mod.rs:127` · 잠금 `:130` · 쥔 가드 `FileSinkWriter::Live` `:119`)가 스레드 지역 「파일 쓰는 중」 표지(const `Cell` · `try_with`)를 보고 그 스레드의 재진입 쓰기를 버린다. ★1판의 「잃는 것 = 쓰기 도중 패닉한 그 줄 하나」는 틀렸다 — 잃는 것은 **재진입한 훅 자신의 보고 줄**이다★ → 그래서 **사유를 따로 남긴다**: 훅은 올리는 첫 패닉의 「스레드 · 위치 · 문장」을 걸쇠의 `OnceLock<String>` 에 **깃발 뒤에** 적고(4판 — 판정 · 깃발까지는 훅 **코드**가 할당하지 않는다 · std 가 훅 앞에 만드는 형식 payload 는 별개 · §4-3), 데몬 꼬리 · 셸 `Exit` 패닉 갈래가 그것을 로그한다. 1회차의 걱정(에이전트 0 이면 꼬리가 빨라 패닉한 스레드가 사유를 적기 전에 끝낸다)은 **꼬리가 마지막 끝내기 앞에서 사유 칸을 최대 1초(가안 · 폴링) 기다리는 것**으로 받되, ★올리는 훅이 도는 중일 때만 기다린다(5판)★ — 훅이 깃발 앞에서 「올리는 훅 진행 중」 원자 셈을 올리고 사유를 적은 뒤 내린다(올림 · 내림 · `wait_reason` 의 읽기 모두 SeqCst — 6판: 내림이 Relaxed 면 셈 0 을 본 꼬리가 그 앞의 사유 쓰기를 못 볼 수 있다 · 최소 조건 = 내림 Release + 읽기 Acquire) · 셈이 0 이면(관찰자 · M21 만 당김 · 훅이 이미 사유를 적고 나감) 곧바로 지금 값 → 헛된 1초가 없다. 그래도 비면 있는 것만 로그한다(받는 진단 손실). ★가둔 비오염 패닉(5판 — codex BLOCK)★: 훅은 판정 직후 돌아간다 — 로그도 기본 훅도 부르지 않는다. 그 스레드가 tracing 락을 쥔 채 패닉했으면(예: reload 층 읽기 락 아래 `info!(?x)` 의 `Debug` 구현 패닉 — `base/src/logging/mod.rs:404`) 훅의 로그가 자기를 기다리는데, 당김이 없어 감시도 놀고 그 스레드 · 곧 모든 로거가 멎는다(지금 abort 훅에도 있는 자기교착 — 기존 · 고치기 싸다). 진단은 가둔 경계로 옮긴다 — 훅이 문장(`payload_as_str` · 256바이트까지 — 문자 경계에서 자름) + 자리(파일 256바이트까지 · 줄 · 열)를 할당 · 락 없이 스레드 지역 **깊이별 칸**(`slot[depth % 4]` · const `thread_local!`)에 베끼고 「무장」을 세운다 · `contain` · `Contained` 의 catch 갈래가 **자기 깊이의 칸을 읽으며 비우고(take)** 잡은 payload 문장 + 그 자리를 한 줄 로그한다(6판 — 5판의 칸 하나는 tokio 일꾼 스레드에서 앞 태스크의 낡은 자리를 물려받고 중첩 경계끼리 덮었다 · §4-1 · §4-3). 올리는 훅은 (d) 에서 이 스레드에 무장된 칸이 있으면 「unwinding contained panic: <문장> @ <자리>」를 사유 · 로그에 덧붙인다(6판 — 가둔 A 가 되감는 중 `Drop` 이 오염 B 를 내 abort 하면 B 만 남던 것). 잃는 것 = 백트레이스 · 가둔 패닉의 기본 훅 stderr 줄(받는다 · §4-11). 오염 관찰자는 `#[track_caller]` 로 받은 `&'static Location` 을 `AtomicPtr` 에 compare_exchange(첫 것만 · 할당 · 락 · 로그 없음)로 적는다. tracing `EnvFilter` 도 내부 `RwLock` 둘을 잡는다(`tracing-subscriber 0.3.23 filter/env/mod.rs:203-204` · 오염은 `try_lock!` 이 삼킨다) — 깃발을 로그보다 먼저 세우는 순서가 그 자리의 교착도 덮는다 |
| **M5** | net | **N1 — net 은 손대지 않는다.** 1층 표시는 데몬 처리기 몸통에만 세우고(`on_connect` · `on_text` · `on_binary` · `on_inbound_saturated` · `on_disconnect`), net 의 하위 태스크 넷 · `handle_connection` · 연결 태스크(`lib.rs:395`)는 **표시 밖(3층 → 정상 종료)**. 까닭: 1판 N3(하위 태스크 포장)은 패닉을 「태스크 끝남」으로 바꾸는데 정리 select 가 dispatch · write 만 기다려(`ws.rs:541-554`) bypass 가 죽으면 연결이 반쯤 산다 · `handle_connection` 몸통 패닉은 하위 태스크를 떼어 내고 `on_disconnect` · `unregister` 를 건너뛴다(§2-4) — 연결 태스크에 표시를 세우면 그 반쪽 상태를 가둬 버린다. 대안(감독 닫기 — 포장이 패닉을 따로 알리고 감독이 그 연결을 닫음 + abort-on-drop 핸들 + select 가 넷 다 기다림)은 net 에 새 구조가 필요하다 — net 자기 루프는 작고 안정된 공유 기반 코드라 그 버그를 재시작으로 둔다. **대가: ADR-0288 결정 1 의 「연결 하나」가 「데몬 처리기 몸통」으로 좁아지고 ADR-0290 영향의 「연결 태스크 표시」를 따르지 않는다 → ADR-0292 가 Amends 로 적는다(§8-1).** net 의존 상한 · 심볼 · 게이트 무변경 |
| **M6** | 메인 future · 부팅 | **둘 다 잡는다.** ① 데몬 `main.rs` 가 `run()` future 전체를 `futures_util::FutureExt::catch_unwind` 로 감싸 패닉이면 사유 로그 → **async main 안에서** platform `terminate_self(101)`(5판 — 당겨진 끝 · M7) · 101 아닌 갈래(`Ok` · `Err(1)`)도 끝내기 직전 `is_pulled()` 를 다시 보고 섰으면 `terminate_self(101)`(6판 — 꼬리 끝 확인 뒤나 부팅 실패와 겹쳐 든 당김을 놓치지 않는다 · 데몬 꼬리와 같은 재확인 · 영향 작음) — 부팅 패닉(훅은 섰지만 watch · 꼬리가 아직 없다)과 꼬리 자체의 패닉을 받는다. 지금 abort 는 즉사하지만 unwind 에서 이것이 없으면 `run()` 이 풀려 런타임 drop 이 blocking 태스크를 **시한 없이** 기다린다(§2-1) — 단일 인스턴스 가드와 `daemon.json` 을 쥔 채 남는 데몬 ② accept 루프를 같은 `catch_unwind` 로 감싸(표시 없이 — 훅이 이미 당겼다) 잡히면 꼬리로 간다(정상 종료 경로를 탄다) — ★감싸는 자리는 `run_accept_loop` **안**의 `select!` 고리다★(그래야 루프 뒤 `sweeper.stop()` `lib.rs:438` 이 돈다 · 3판 정정). base 에 따로 타입을 두지 않는다(1판 `CaughtOnly` 걷음) |
| **M7** | 종료 코드 · 당겨진 끝 | **데몬 101 · 셸 101(상수 하나)** — 데몬은 꼬리 끝에서 깃발이면 `Err(101)` → `main.rs` 의 101 갈래가 platform `terminate_self(101)`(5판). ★당겨진 끝은 전부 마지막 로그 줄 뒤 `terminate_self(101)` 이다(5판)★ — 데몬 꼬리 끝 · M6 포착(`main.rs` 의 101 갈래 둘) · `main.rs` 의 101 아닌 갈래의 깃발 재확인(6판 — U5) · 셸 = 깨우기가 주 스레드에 게시한 닫힘(트레이 정리 뒤 · 6판 — `app.exit` 를 걷었다 · M18) · `RunEvent::Exit` 콜백 끝 재확인(잡혔거나 당겨졌으면 · 시작 실패 코드보다 앞 · 6판) · 셸 주 스레드 패닉(셸 `main.rs` 가 `run()` 을 `catch_unwind` — tao 가 `App::run` 밖으로 `resume_unwind` 한다 — U6). 까닭: `process::exit` 밑의 `ExitProcess` 는 다른 스레드를 — 감시까지 — 먼저 끝낸 뒤 로더 락 아래 `DLL_PROCESS_DETACH` 를 돈다(MS 문서 순서 · 실측 아님) → 분리 알림이 멎으면(셸의 WebView2 · COM DLL — 미검) 덮을 이가 없다(§9-12). 당기지 않은 끝(정상 끝 · `Err(1)`)은 `process::exit` 그대로(데몬은 그 직전 깃발 재확인 — 6판 · 셸은 tao `run` 의 `process::exit(0)` — 지금과 같다). ★load-bearing★: `Ok` 로 돌아가면 런타임 drop 이 매달릴 수 있는 blocking 태스크(예: 요청이 띄운 1단계 본문 · 활성화 blocking 이 멎은 채 남은 것)를 기다린다 — 패닉 경로는 그 대기를 건너뛴다. ★2판의 예 「오염으로 멎은 kill 스레드」는 틀렸다★ — 그 스레드는 런타임 drop 에 닿기 전에 꼬리 ③ 자체를 멈춘다(`shutdown_all` 의 `thread::scope` `manager.rs:3122-3128` 를 `lib.rs:823` 이 await) → 그 멎음은 (바)의 몫. `Err(101)` 과 끝내기 사이의 `run()` 지역값 drop 은 M23. 소비자는 없다(셸은 PID 로 데몬 생사를 본다) |
| **M8** | 실프로세스 시험 방아쇠 | `cfg(debug_assertions)` 아래에서만 읽는 환경변수(가안 `ENGRAM_DEBUG_PANIC=unmarked\|contained\|poison\|boot\|hang` — `hang` 은 3판 · 감시(M19) 시한도 같은 조건에서만 짧게 덮어쓴다) — 배포판 표면 0 |
| **M9** | 요청 경로 회귀망 | 요청 경로 파일(`agent_conn.rs` · `connection_core.rs` · `command_delivery.rs` · `control/mcp_server.rs`)의 시험 밖에 맨 `tokio::spawn(` · `spawn_blocking(` 이 없다는 소스 시험. **허용 목록**(3층 — 일부러 표시 밖) = StopDaemon 의 `spawn_blocking(shutdown_all)`(`connection_core.rs:1543` — 전역 전이라 1층이 아니다 · 그 안 패닉은 어차피 끄는 중) · 명령 자리 수거 태스크(`command_delivery.rs:537` — 4판 정정 · `:527` 은 함수 머리) · MCP serve 태스크(`mcp_server.rs:1411` — ★이 파일에서 남는 맨 spawn 은 이것 하나다★: 처리기 본문 `spawn_blocking` 여섯(`:513` · `:940` · `:1040` · `:1092` · `:1173` · `:1249`)은 `spawn_blocking_contained` 로 옮긴다 — §4-6) |
| **M10** | 컴파일 가드 | 데몬 lib · **agent lib**(가두는 자리가 사는 곳 — 1판의 `main.rs` 만으로는 lib 만 abort 로 짓는 길을 못 막는다)에 `#[cfg(not(panic = "unwind"))] compile_error!(…)`. 셸은 (라)=S1a′ 일 때만. CLI 는 두지 않는다 |
| **M11** | 바이너리 크기 | U7 앞뒤로 세 exe 크기를 재 보고 · 문턱 없음(30% 를 넘으면 사용자에게 — 가안) |
| **M12** | T-52 · 동시 종료 | **따로 둔다** — 패닉 경로는 `send_stop` 을 지나지 않는다(실프로세스 시험은 `send_stop` 결과를 판정에 안 쓴다). 동시 경합 하나를 기록한다: StopDaemon 의 `shutdown_all` 과 패닉 꼬리의 `shutdown_all` 이 겹치면 둘째 `join_pump` 가 `None` 을 받아(§2-5) 꼬리가 먼저 끝나 `terminate_self(101)` 할 수 있다 — StopDaemon 의 kill 스레드가 도는 중이라 그 답이 사라진다(T-52 와 같은 모양) · 남은 자식은 Job 이 끈다. ★깃발 **뒤** 들어온 StopDaemon(G2 허용 목록)은 `shutdown_all` 을 돌리지 않고 `Ok` 로 답한 뒤 같은 신호 · Close 한다(5판 — `connection_core.rs:1542-1543` 의 `spawn_blocking(shutdown_all)` 을 깃발이면 건너뛴다 · `shutdown_all` 은 재진입 막이가 없다 `manager.rs:3113` · 꼬리 `lib.rs:823-824` 가 진다 · 연결이 닫혀 클라이언트는 `DaemonClosed` 를 그대로 본다)★ → 겹침은 StopDaemon 이 깃발 **앞**에 `shutdown_all` 을 시작한 경우만 남는다. T-52 항목에 한 줄 |
| **M13** | abort 전제 코드 · 주석 | 「패닉 말고 값으로」 규칙은 전부 유지 · **의미 재감사(U8a)를 프로필 전환(U7) 앞에** — leftover K7 막 둘(ADR-0262 — 「배포판 abort 에 기댄다」) · ADR-0142 `in_flight_targets`(`FlightSettle` 이 오염이면 말없이 건너뜀 · `Reservation` 은 오류 로그만) · `DeliveryDone` 2층 배치 · 래치 · codex 라이터 · `lock_for_cleanup` 짝(`command_roster.rs:401-403` · `command_delivery.rs:994-996` — 둘 다 `sync::lock`) · 런타임 로그 문장 `messaging_host.rs:769`. 「릴리스에서 도달 불가」 **사실 주장만** 고치는 문장 손질은 U7 뒤(U8b) |
| **M14** | 종료 경로 오염 내성 | §2-5 의 락 자리 전부를 되찾기로: 명부(`get_session` · `shutdown_all`) · 제어 레지스트리 `revoke`(데몬) · `enter_exiting`(코어 status) · 추적기 `unwatch` · 통로 `shutdown`(PTY 자식 · master · stdio 자식) · 감시의 master 락. 공유 쪽 = 관찰(이미 당겨졌으면 무동작) · 에이전트 전용 쪽 = (가)=P2 면 범위 · P1 이면 관찰. 어느 안이든 **종료 경로는 끝까지 간다**(ADR-0288 영향) |
| **M15** | 스폰 경로 스코프 가드 | 활성화 창(`manager.rs:2050-2162`)에서 통로를 펌프에 넘기기 전 패닉 · 오류로 빠지면 `transport.shutdown()` 을 부르는 가드(되감기 중에도 — 그래서 `shutdown` 의 락은 M14 로 되찾기여야 한다. 아니면 이중 패닉 abort). 두 번 불려도 되는지(명부에 이미 올라 `shutdown_all` 도 부르는 경우)는 U4 가 확인한다 |
| **M16** | `Contained` · axum · rmcp | 패닉한 · 취소된 안쪽 future 는 **깊이 가드 안에서** drop 한다(패닉 뒤 drop 은 `catch_unwind` 로 · 취소 drop 은 `Contained::drop` 에서 — 되감는 중 `Drop` 안에서 난 중첩 패닉은 그 소멸자를 **벗어날 때만** abort 다(「panic in a destructor during cleanup」 — core 1.95 rlib 문자열로 본 추론 · 6판 정정: 5판의 「이미 되감는 중이면 std 가 중첩 패닉을 abort 하므로 `catch_unwind` 는 무력하다」는 틀렸다) → `Contained::drop` 의 `catch_unwind` 는 되감는 중에도 그것을 잡는다 · 안전에는 무해). axum 의 `Contained` 층은 **가장 바깥**(마지막 `.layer`)이어야 `bearer_auth` · 본문 크기 제한까지 덮는다. rmcp 도구 처리기는 그 층 밖이라(§2-4) **도구 몸통을 직접 감싼다**. hyper 가 `next.run` 뒤에 poll 하는 스트리밍 · SSE 본문은 표시 밖 → 올린다(§4-11) |
| **M17** | 관찰자 계약 | base `sync` 의 관찰 갈래는 오염을 보면 등록된 `fn(&'static Location<'static>)` 하나를 부른 뒤 되찾는다 · 등록 = `OnceLock` 한 번(조립부) · 관찰자는 **패닉 · 할당 · 락 · 로그 · 블로킹 금지**(되감기 중 `Drop` 안에서도 불린다) — 실물은 위치 기록(`AtomicPtr` CAS) + 깃발 세우기 둘뿐. `sync` 계약 2 개정 |
| **M18** | 셸 끝내기 절차 | 「`Final` 생략」은 **`is_pulled()` 로 판정**(훅이 동기로 세운다 — 1판의 「깨우기가 `wait()` 뒤 세우는 표지」는 사용자 끝내기가 먼저 줄 서면 어긋난다) · ★**깨우기는 `app.exit` 를 부르지 않는다(6판)**★ — 깃발을 보면 `AppHandle::run_on_main_thread`(tauri `app.rs:494-499` → wry `lib.rs:2777-2779` 의 `Message::Task` · 실행 = `lib.rs:3335`)로 **닫힘 하나**를 게시한다: `catch_unwind(AssertUnwindSafe(\|\| handle.cleanup_before_exit()))`(트레이 아이콘 지우기 · best effort) → `terminate_self(101)`. 게시가 실패하면(`Err` — 이벤트 루프가 없다) 더 하지 않는다 — 끝은 감시 시한(M19)이다. 실패 갈래는 `FailedToSendMessage` 를 돌려줄 뿐 로그 · `exit` 가 없다(wry `lib.rs:235-255` · tao `event_loop.rs:570-585` 의 `PostMessageW`). 까닭(리뷰 5회차): ① `AppHandle::exit` 의 `request_exit` 실패 갈래는 부른 스레드에서 `log::error!` → `cleanup_before_exit` → `process::exit`(`app.rs:574-579` · wry `lib.rs:2748-2755`) — 그 `ExitProcess` 가 감시까지 끝낸 뒤 DLL 분리를 돈다 ② tao 는 `Exit`(LoopDestroyed) 처리기를 자기 `catch_unwind` 안에서 돌리고 `reset_runner` 가 그 패닉을 버린 뒤 `run` 이 `process::exit(0)`(tao `runner.rs:95-108` · `:153-175` · `event_loop.rs:225-231`) — 콜백 안 패닉(예: `cleanup_before_exit` 의 `tray.icons.lock().unwrap()` `app.rs:1110`)이 코드 0 으로 끝난다 ③ 콜백의 진입 확인 뒤 든 당김(트레이 끄기의 `exit_session.shutdown()` ≤2초 동안 등)을 다시 보는 이가 없어 코드 0 · `Final` 이 쓰인다. → 당겨진 길은 `RunEvent::Exit` 에 닿지 않고 `Final` 도 안 쓰인다(실행 잠금은 끝내기가 핸들을 닫아 OS 가 푼다 — `state/lock.rs` · 명시 해제는 대기 단축용). ★**`RunEvent::Exit` 콜백(6판)**★ = 몸통 전체를 `catch_unwind(AssertUnwindSafe(..))` 로 — 진입 때 `is_pulled()` 면(사용자 끝내기와 당김이 겹친 경우) `wait_reason(1초)` 뒤 사유 로그 → `Final` 없이 실행 잠금 놓기 → `handle.cleanup_before_exit()` · 아니면 지금처럼 `exit_session.shutdown()` — 그리고 **끝에서** `if caught \|\| esc.is_pulled() { terminate_self(101) }`(데몬 꼬리와 같은 재확인 — ② · ③ 을 닫는다) · 그 뒤에 기존 `startup_failed` → `process::exit(STARTUP_FAILED_EXIT_CODE)`(`src-tauri/src/lib.rs:408-409` — 잡혔거나 당겨졌으면 101 이 앞선다). 콜백이 101 로 끝나면 tauri 자신의 뒤 정리(`app.rs:1433`)는 안 돈다 — 당겨진 갈래는 콜백 안에서 이미 불렀다. 훅이 **주 스레드**에서 올렸으면(주 스레드 판정 = 셸 `run()` 머리가 세우는 const TLS `Cell<bool>` — `thread::current()` 가 아니다 · 할당 없는 판정 · 5판) 깨우기는 게시하지 않는다(주 스레드는 tao 되던지기로 이미 풀려 나간다). ★그 표지(`MAIN_ESCALATED`)는 훅이 깃발을 당기기 **앞**에 SeqCst 로 세운다★(3판 · §4-3). 6판부터 표지는 안전에 필요하지 않다 — 주 스레드가 콜백에서 패닉하면 tao 가 그 뒤 사건을 처리기 없이 버려(`runner.rs:153-175` — 추론) 게시한 닫힘이 안 돈다 · 게시 실패는 무해 → 걷을지는 U6 이 고른다. 깨우기는 시한 · 로그를 갖지 않는다(4판 — 시한은 감시 M19). 셸 `main.rs` 포착(주 스레드 패닉)에는 `AppHandle` 이 없어 `cleanup_before_exit` 를 건너뛴다(유령 트레이 아이콘 받음 · §4-11) |
| **M19** | 패닉 종료 감시((바)=감시) | **데몬 · 셸 같은 모양(4판)** — 훅 설치(데몬 `lib.rs:458` · 셸 `run()` 머리 — `tauri::Builder` 앞 · 5판) 직후 띄우는 전용 std 스레드 하나 — `wait_polling(≈1초)` 로 깃발을 보고(5판 — 시한이 초 단위라 1초 늦음은 예산 안 · 깨우기 쪽 200ms 는 그대로), 본 시각부터 시한(가안 데몬 30초 · 셸 5초)이 지나면 **platform 즉시 끝내기**(가안 `engram_dashboard_platform::process::terminate_self(101)` — Windows = `TerminateProcess(GetCurrentProcess(), 101)` 뒤 잠듦 고리 · 그 밖 = `process::exit` 자리채움 · cfg 는 platform 안 · §4-4 · 5판)를 **직접** 부른다. `tracing` · 락을 쓰지 않는다 — 패닉한 스레드가 로그 락을 쥔 채 멎었거나 tokio 런타임 · 셸 주 스레드가 멎어도 돈다. ★`process::exit` 가 아니다★ — 그 길은 다른 스레드의 `exit` · 로더 락 뒤에 막힐 수 있다(§9-12 · 추론 · 미검). 즉시 끝내기는 DLL 분리 알림 · 로더 락 · atexit · TLS 해체 · std 끝 정리를 건너뛰고, 잃는 것이 없다 — 사유는 걸쇠에 이미 있고(꼬리 로그 줄을 잃는 것은 받는다) · 파일 싱크는 버퍼 없이 쓰고 · 자식은 Job kill-on-close 가 거둔다. 꼬리 · M6(데몬 `main.rs` 의 101 아닌 갈래 재확인 포함 — 6판) · 셸 깨우기가 게시한 닫힘 · 셸 `Exit` 콜백 끝 재확인(6판) · 셸 `main.rs` 포착의 당겨진 끝도 같은 `terminate_self(101)` 이다(5판 — M7 · 4판의 「꼬리 · M6 은 `process::exit(101)` 그대로」를 걷었다). **`cfg(panic = "unwind")` 빌드에서만 띄운다**(abort 에선 패닉 = 즉사라 할 일이 없다). 훅 설치 때 서므로 부팅 · 셸 setup 앞 멎음도 덮는다(§4-11). 기동 실패 = 데몬 부팅 실패(`Err(1)` — 감시 없이 돌지 않는다) · 셸은 그 자리에서 `process::exit(STARTUP_FAILED_EXIT_CODE)`(5판 — `Builder` 앞이라 `startup_failed`(`src-tauri/src/lib.rs:128`) 길이 아직 없다). 셸 깨우기는 시한을 갖지 않고, 게시가 실패하면 더 하지 않는다 — 그때 끝은 이 감시다(M18 · 6판). 시한 · 끝내기 동작은 주입(M20) — 시험은 짧은 시한 + 기록하는 끝내기. 상세 §4-4 · §4-9 |
| **M20** | 걸쇠 주입 seam(ADR-0012) | 걸쇠 `Escalation` 은 `new()` 로도 만든다. 프로세스 전역 `escalation()` 은 **조립 자리만**(훅 · 관찰자 등록 · 데몬 `run()` · `main.rs` · 셸 `run()`) 부르고, 나머지(1층 도우미 · `ConnectionCore` · accept 루프 · 감시 둘 · 셸 깨우기 · 셸 종료 세션 · 상태 싱크 · **제어 HTTP 의 가장 바깥 층과 제어 라우트 거절 층 · MCP 도구 감싸개**((다)의 `is_pulled()` — 걸쇠는 MCP 서버 상태로 넘긴다 · 4판))는 `&'static Escalation` 을 받는다 → 시험은 자기 인스턴스를 쓴다. ★시험 서버도 받는 쪽이다(4판)★ — `start_test_server`(`TestServerHandle` `lib.rs:853`)는 운영과 같은 `run_accept_loop`(`:995`)를 돌리고 `tests/ws_e2e.rs` · `src-tauri/src/daemon_client/tests.rs` 가 띄운다 → 새던 지역 인스턴스(`Box::leak`)를 넘기고 전역을 쓰지 않는다. 관찰자 칸(`sync::set_poison_observer` — 한 번뿐인 `OnceLock`)은 되돌릴 수 없으므로 **등록 상태를 재는 시험은 자기 통합 시험 바이너리**(base `tests/poison_observer.rs` 가안 — 결과 줄 +1)에 두고 lib 단위는 미등록 상태만 잰다. 까닭: 한 방향 전역(깃발 · 관찰자)을 lib 시험들이 함께 쓰면 U1 · U3 · U6 시험이 서로를 뒤집는다 |
| **M21** | kill 길의 상태 싱크 | 데몬 `DaemonStatusSink`(`status_fanout.rs:37` · 알림 `:47-`)의 `broadcast_text` 를 `contain` 으로 돌리고, 잡히면 **잡은 문장을 첫 사유로 적고**(4판 — `contain` 안이라 오염 아닌 패닉에는 훅이 아무것도 안 적었다) 걸쇠를 당긴 뒤(공유 팬아웃 패닉 = 3층 규칙 · 오염 패닉이면 훅이 이미 당겼다) 그 알림만 버리고 돌아온다. ★단위가 갈린다(4판)★ — U4 = 가둠 · 버림만 · 사유 기록 · 당김과 생성자의 걸쇠 인자는 U5(깃발을 보는 interval 팔 · 감시가 서는 단위 — U4 뒤 U5 전에 당기면 U3 의 거절만 깨어나 데몬이 끝나지 않은 채 새 일을 막는다). 실제 노출은 오염뿐이다 — `broadcast_text`(`net/src/ws.rs:208-217`)는 `.expect` + `try_send` + `warn!` 이라 패닉 길이 오염된 `.expect` 하나이고, 그때는 훅이 이미 당겼다(명시 당김은 그 밖의 패닉에 대한 덧대기). 까닭: `kill_agent` → `enter_exiting` → 이 싱크 → net `ConnRegistry` 의 `.expect("conn registry poisoned")`(`net/src/ws.rs:212`)가 `session.kill` **앞**에서 돈다 — 그 락 오염이 종료를 당겼다면 kill 스레드가 전부 거기서 패닉해 2동사(ADR-0001)가 하나도 안 돈다. net 무변경(M5) · 상세 §4-5 4 |
| **M22** | `send_input` 패닉 포착(3판 = 스코프 가드) | 분류 쓰기(`session.rs:284` `write_user_classified`)가 `emit(Queued)`(`:303`) 뒤 `send_input`(`:304`)으로 가는 갈래에서 `send_input` 을 **맨 `std::panic::catch_unwind`** 로 감싼다(4판 — `Drop` 가드를 걷었다). `AssertUnwindSafe` 가 필요하고(`dyn AgentTransport` 가 `RefUnwindSafe` 가 아니다) 정당하다 — catch 뒤에는 `core.emit(Dropped)` 만 돌고 통로 상태는 다시 풀기 전에 읽지 않는다(5판). 잡히면 그 id 의 `Dropped{Rejected}` 를 내고(되감는 중이 아니라 abort 갈래가 없다) `resume_unwind` 로 다시 풀어 보낸다(훅을 다시 부르지 않는다). ★`contain` 이 아니다★ — 훅은 패닉 순간 원래 깊이로 이미 판정했다 · `contain` 은 깊이를 올려 그 판정(표시 밖 → 올림)을 바꾼다. emit 이 오염으로 패닉하면 그 패닉이 대신 풀린다(오염 패닉 → 훅이 올린다) — 그래서 3판의 (가)=P1 · P1o 건너뜀과 그 「확인 뒤 행동」 경합이 없어졌다. 기존 `Err` 갈래는 그대로 · 잠금 순서 = `Dropped` 는 `input_order` 아래(문서화된 순서 그대로). 남는 거짓 `Dropped` = §4-11. 상세 §4-7 |
| **M23** | `run()` 지역값 drop | 패닉(M6 되감기) · `Err(101)` 갈래에서 `run()` 지역값이 `exit` 전에 drop 된다 — `AgentManager::drop`(`manager.rs:3212-3222` — reaper 시한 없는 join `:3219`) · `SessionTracker::drop`(`session_tracker.rs:196-204` — join + `.expect` `:200` · 되감는 중이면 abort). → ① 두 `Drop` 은 `thread::panicking()` 이면 join 을 건너뛰고 `.expect` 를 없앤다 · 같은 꼴 하나 더(4판): `SpawnReservation::drop`(`manager.rs:746-753` — `.expect("spawning set poisoned")`)은 (나)=E 활성화 창의 되감기 중에 돈다 → `sync::lock`(관찰 갈래)으로 바꾼다(U4 · join 이 없어 `panicking()` 판정은 필요 없다) ② 꼬리 끝 `Err(101)` 갈래는 그 전에 관리자 `Arc` 하나를 `mem::forget` 한다(수가 0 이 안 돼 `Drop` 이 안 돈다 · 그 밖의 지역값은 평소대로 drop). agent 가 「당겨졌나」를 읽게 하지 않는다(M20 — 전역은 조립 자리만) · 상세 §4-4 |

---

## 4. 설계 (권고안 기준 — (가)=P2+B1′ · (나)=E · (다)=거절 · (라)=S1a · (마)=N1a·N2a · (바)=감시 시한 둘 다. 답이 다르면 해당 절만 바뀐다)

### 4-1. base `panic` 입주자 (가안)

```rust
// crates/engram-dashboard-base/src/panic.rs — 이름 · 모양 전부 가안. std + base 가 이미 지는 `tracing` facade
//   (가둔 경계의 로그 · 5판 — `crate::logging` 은 안 부른다 · 게이트 ③)만 쓴다(새 의존 0). 정책 0.

/// 이 스레드가 지금 「가두는 중」인가. const · Drop 없는 칸이라 TLS 해체가 없다 — 늘 읽힌다(M1).
pub fn is_contained() -> bool;

/// 가둔 패닉의 칸 — 훅이 할당 · 락 없이 베끼는 스레드 지역 고정 칸 넷(const `thread_local!` · Drop 없음 ·
///   깊이별 `slot[depth % 4]` — 6판). 칸 하나 = 문장(`payload_as_str` · 256바이트까지) + 파일(256바이트까지 ·
///   둘 다 문자 경계에서 자름) + 줄 + 열 + 「무장」. 쓰는 이 = 훅 하나(지금 깊이 칸 — 늘 덮는다) ·
///   읽는 이 = 아래 catch 갈래(자기 깊이 칸을 읽으며 비운다 — take) · 올리는 훅의 (d)(아래 `armed_contained`).
///   5판의 `record_contained_location`(자리만 · 칸 하나)을 넓혔다 — 칸 하나는 tokio 일꾼 스레드에서 앞 태스크의
///   낡은 자리를 물려받고 중첩 경계끼리 덮었다. 입구가 깊이를 올릴 때 그 칸을 비운다(가안 — 안쪽의 남의
///   `catch_unwind` 가 삼켜 소비되지 않은 칸이 다음 입구로 새지 않게 · U1). 깊이 5 이상은 칸을 돌려 쓴다(받는다).
pub fn record_contained(at: &Location<'_>, text: Option<&str>);

/// 이 스레드의 무장된 칸을 값으로 베낀다(고정 크기 `Copy` · 할당 · 락 없음 · 비우지 않는다 — 6판).
///   올리는 훅이 (a′) 가 덮기 **전에** 스택에 받아 (d) 의 사유 · (e) 의 로그에 덧붙인다(§4-3).
pub fn armed_contained() -> Option<ContainedNote>;

/// 동기 입구 — 깊이를 올린 채 f 를 돌리고 패닉을 값으로 접는다. 깊이는 되감기에도 내려간다(가드 drop).
///   잡으면 payload 문장 + 자기 깊이 칸의 자리(읽으며 비움 — 6판)를 `Caught` 에 싣고 한 줄 로그한다(5판 —
///   훅은 가둔 비오염 패닉을 로그하지 않는다 · 되감기가 끝나 락을 놓은 뒤라 여기서는 할당 · 로그가 안전하다).
///   칸이 무장돼 있지 않으면 문장만 싣는다. `Contained` 도 같다.
pub fn contain<T>(f: impl FnOnce() -> T) -> Result<T, Caught>;

/// 비동기 입구 — poll 마다 깊이를 올리고 그 poll 의 패닉을 접는다. 패닉한 future 는 다시 poll 하지 않고
///   깊이 가드 안에서 drop 한다. 취소 drop 도 Drop 에서 깊이를 올린 채 한다(이미 되감는 중이면 그대로).
///   ★태스크는 스레드를 옮겨 다니므로 「태스크 하나에 한 번」이 아니라 「poll 마다」다★.
pub struct Contained<F>;                 // impl Future<Output = Result<F::Output, Caught>>

/// 표시 안에서 일부러 「표시 밖」을 만드는 구간 — 깊이를 0 으로 두고 f 를 돌린 뒤 되돌린다(되감기에도).
///   잡지 않는다 — 패닉은 그대로 바깥 입구까지 풀린다(훅은 표시 밖으로 본다).
pub fn escalating<T>(f: impl FnOnce() -> T) -> T;

pub struct Caught { /* 패닉 문장 + 자리(5판) */ }    // .message() · .location()
pub struct ContainedNote { /* 칸 하나의 값 사본 — 문장 · 파일 · 줄 · 열(6판) */ }

/// 패닉 값 → 문장(지금 사본 일곱 — 펌프 셋 · input_queue · leftover · reaper · 데몬 훅.
///   command `route.rs` 의 사본은 command 가 의존 0 이라 남는다).
pub fn payload_message(payload: &(dyn Any + Send)) -> String;

// 훅 판정의 할당 없는 빌림은 std `PanicHookInfo::payload_as_str`(1.88 안정 · 툴체인 1.95)를 쓴다 —
//   4판의 `payload_str` 는 두지 않는다(5판 · §4-3).

/// std 락 오염 패닉 문장인가(`.expect` / `.unwrap` 의 `PoisonError` Debug 문자열). 도메인 0 원시 — 정책 아님.
pub fn is_poison_panic(message: &str) -> bool;

/// 프로세스 종료 걸쇠 — 한 번 서면 안 내려간다. 할당 · 락 · OS 호출 없는 쪽은 pull · is_pulled · record_site.
pub struct Escalation;                   // AtomicBool + AtomicUsize(올리는 훅 진행 중 · 5판) + OnceLock<String> + AtomicPtr<Location<'static>>
impl Escalation {
    pub const fn new() -> Self;                         // 시험 · 조립용 지역 인스턴스(M20)
    pub fn pull(&self);                                 // store(true, SeqCst)
    pub fn is_pulled(&self) -> bool;                    // load(SeqCst)
    pub fn hook_enter(&self);                           // 진행 중 +1(SeqCst) — 올리는 훅이 pull 앞에(5판)
    pub fn hook_exit(&self);                            // 진행 중 −1(SeqCst — 6판 · 최소 Release) — 올리는 훅이 record_reason 뒤에(5판)
    pub fn record_reason(&self, f: impl FnOnce() -> String);   // 첫 것만(OnceLock) — 훅은 pull 뒤에(4판) · M21 은 pull 앞에
    pub fn record_site(&self, at: &'static Location<'static>); // 첫 것만(CAS) — 관찰자
    pub fn reason(&self) -> Option<&str>;
    pub fn wait_reason(&self, up_to: Duration) -> Option<&str>; // 진행 중 > 0 일 때만 폴링 유계 대기 · 0 이면 곧바로 지금 값
                                                                //   — 꼬리 · 셸 Exit 진입 갈래(가안 1초 · 4판 · 셈은 5판)
                                                                //   셈 읽기 = SeqCst(6판 · 최소 Acquire — 0 을 보면 그 앞 사유 쓰기도 보인다)
    pub fn site(&self) -> Option<&'static Location<'static>>;
    pub fn wait_polling(&self, every: Duration);       // park_timeout 고리 — 셸 깨우기(200ms) · 감시 스레드 둘(≈1초 · 5판 · M19)
}
/// 프로세스 전역 걸쇠 — ★조립 자리(훅 · 관찰자 등록 · run() · main.rs)만 부른다★. 나머지는 &'static Escalation 을 받는다(M20).
///   즉시 끝내기는 여기 없다 — OS 의존이라 platform `process::terminate_self`(가안 · M19 · §4-4).
pub fn escalation() -> &'static Escalation;
```

- **입주 조건(ADR-0269):** ① 지금 여러 곳에 사본(페이로드 문장 일곱) + 소비자 셋(agent · 데몬 · 셸) ② 도메인 지식 0 — 「오염이면 올린다」 같은 정책은 없다(M1) ③ 다른 입주자를 부르지 않는다 — `sync` 관찰자 · `logging` 재진입 가드와 잇는 것은 **base 밖**(데몬 · 셸 조립부).
- **base `sync` 개정(U1):** 관찰 갈래 = 지금 이름(`lock` · `read` · `write` · `wait_timeout`)에 `#[track_caller]` + 오염이면 관찰자 호출 뒤 되찾기 · 범위 갈래(가안 `lock_scoped` 등 — (가)=P2) = 지금처럼 말없이 · (가)=P1o 면 범위 갈래 대신 `lock_order(&Mutex<()>)` 하나(타입으로 순서 락에만 묶인다) · `set_poison_observer` 한 번. **기본값 = 관찰**(분류 안 된 자리는 종료 쪽 — 안전한 쪽). 계약 2 = 「로그 · 다른 락 · 밖 호출을 하지 않는다 — 단 등록된 관찰자 하나는 부른다(M17)」 · 계약 3 = unwind 아래 운영도 닿는다.
- **base `logging` 개정(U1 — M4 L1):** 스레드 지역 「파일 쓰는 중」 표지 — `FileSinkWriter::Live` 가 세우고 drop 이 내린다 · 섰으면 `make_writer` 가 `Void`. 주석 `:110-115` 「알려진 미수정」을 닫는다 — ★닫는 것은 같은 스레드 재진입뿐이다★(다른 스레드가 쥔 tracing 락에 막히는 것은 (바) · §4-11).
- **걸쇠 주입 seam(M20 · ADR-0012):** 깃발 · 사유 · 자리를 읽고 쓰는 런타임 코드는 전부 `&'static Escalation` 을 인자 · 칸으로 받는다 — 데몬 1층 도우미(§4-6) · `ConnectionCore` · accept 루프 · 감시 스레드 둘(§4-4 · §4-9) · 상태 싱크(M21) · 제어 HTTP 바깥 층 · 거절 층 · MCP 도구 감싸개(MCP 서버 상태로 — 4판) · 셸 깨우기 · 종료 세션(§4-9) · 시험 서버(`start_test_server` — 지역 인스턴스 · 4판). 전역 `escalation()` 은 조립 자리에서 그 인자로 넘길 때만 부른다. 시험은 `Box::leak(Box::new(Escalation::new()))` 같은 지역 인스턴스로 잰다. 관찰자 칸은 한 번뿐인 전역이라 등록 상태 시험은 자기 통합 시험 바이너리에 둔다(M20 — base `tests/poison_observer.rs` 가안).

### 4-2. 층 경계 표 (권고안)

| 층 | 자리 | 표시 |
|---|---|---|
| 1 요청 | WS 처리기 다섯 몸통(`agent_conn.rs` `:282` · `:314` · `:375` · `:405` · `:450` — 수명 처리기 둘의 공유 걸음은 (나)) · 떼어 낸 명령 태스크(`:336`) · 활성화 blocking 둘(`connection_core.rs:1366` · `:1760` — 그 안 등록 창은 (나)) · 버스 배달 태스크(`:2106`) · 1단계 본문 blocking(`command_delivery.rs:1361`) · 제어 HTTP(가장 바깥 axum 층 + blocking 여섯) · MCP 도구 몸통 셋 | `Contained` / `contain` |
| 2 에이전트 하나 | 펌프 셋 · 라이터 몸통 · 첫 턴 세션 id 기록 · leftover 듣는 차례 · 일꾼 · **PTY 감시 · stderr 배수(B1′)** · 메시징 수신자 배달(`spawn_delivery` — U8a 감사 뒤) | `contain` |
| 3 공유 · 그 밖 | 전역 루프(reaper 고리 · sweep · 추적 · 사용량 · flush · 수거기) · 데몬 부팅 · 메인 future · net 자기 태스크 · 연결 태스크 · StopDaemon `shutdown_all` · codex 라이터 · thread_lock · 셸 전부((라)=S1a) · 써드파티 내부(rmcp 세션 일꾼 · hyper · 스트리밍 본문) | 없음 → 정상 종료 |
| 3 — 가둔 뒤 당김(4판) | 상태 싱크 broadcast — `DaemonStatusSink` 의 `broadcast_text`(M21 — kill 길이 끝까지 가게 가두되 공유 팬아웃 패닉이라 당긴다 · 실제 노출은 오염뿐 `net/src/ws.rs:208-217`) | `contain` → 사유 · 당김(U5) |
| (나) 구간 | 활성화 등록 창 · 요청에서 부른 `kill_agent` 앞 줄 · 연결 수명 처리기(`on_connect` · `on_disconnect`)의 공유 걸음(U3 분류) · (`reap_one` — 아래) | `escalating` — 표시 안이어도 표시 밖 |

reaper 고리 자체의 `catch_unwind`(`reaper.rs:208`)는 남긴다 — 종료가 끝날 때까지 고리를 살려 다른 에이전트의 수거 메시지를 계속 받는다(sweep 과 같은 까닭). 표시는 세우지 않는다 — ★그래서 `reap_one` 은 감쌀 것 없이 이미 표시 밖이다★(깊이 0 · 훅이 올린다). 2판의 「`reap_one` 을 `escalating` 으로」는 무의미한 일이라 걷었다(3판). (나)=R 이면 고리에 2층 표시 + 재시도가 들어 그때 비로소 구간 처리가 필요하다(§3-1 (나) R).

### 4-3. 훅 (데몬 · 셸 공통 모양 — 조립부마다 한 벌)

```
hook(info):                                     // 패닉한 스레드에서 · 되감기 전에 (5판 순서)
  // (a) 할당 없는 판정 — 빌림 · 부분 문자열 확인 · 깊이 읽기만
  contained = base::panic::is_contained()       // const Cell — 늘 읽힌다(M1)
  text      = info.payload_as_str()             // std 1.88 — &str 빌림(None = 문장 없는 payload · 5판)
  escalate  = !contained || text.is_some_and(is_poison_panic)  // 정책 = 조립부(M1)
  prior     = if escalate: base::panic::armed_contained()   // (a″) 무장된 칸을 스택에 값으로 — (a′) 가 덮기 전에 · 할당 없음(6판)
  if contained: base::panic::record_contained(info.location(), text)  // (a′) 지금 깊이 칸에 문장 + 자리 · 무장 — 할당 · 락 없음(5판 · 6판)
  if !escalate: return                          // 가둔 비오염 패닉 — 로그 · 기본 훅 없음 · 로그는 가둔 경계가(5판 · §4-1)
  [셸] if IS_MAIN.get(): MAIN_ESCALATED.store(true, SeqCst)  // (b) 깃발보다 먼저(M18) · IS_MAIN = run() 머리가 세운 const TLS Cell<bool>(5판)
  ESC.hook_enter()                              // (b′) 올리는 훅 진행 중 +1(SeqCst) — wait_reason 이 이것만 기다린다(5판)
  ESC.pull()                                    // (c) SeqCst store 하나 — 락 · unpark · 할당 없음
  ESC.record_reason(|| format!("{thread} {location}: {msg}{prior_note}"))   // (d) 첫 사유 — msg = payload_message(…) · 여기서 처음 할당
                                                //   prior_note = prior 가 있으면 " (unwinding contained panic: <문장> @ <자리>)"(6판)
  ESC.hook_exit()                               // (d′) 진행 중 −1(SeqCst — 6판)
  tracing::error!(thread, location, "스레드 panic: {msg}{prior_note}")  // (e) 락을 잡는다(아래) · 같은 스레드 재진입이면 버려진다(M4 L1)
                                                //   [셸] 로깅이 서기 전(boot 플러그인 앞)이면 무동작(§4-9 · 5판)
  prev(info)                                    // (f) 기본 훅(stderr 잠금)
```

- **가둔 패닉 칸 · 덧붙임(6판 — 리뷰 5회차)** — 칸은 문장까지 싣고(문장 · 파일 각 256바이트 · 문자 경계에서 자름 · 할당 없음) 깊이별 넷이며, 경계는 자기 깊이 칸을 읽으며 비운다(§4-1). 올리는 훅은 (a″) 에서 이 스레드의 무장된 칸을 스택에 받아 (d) · (e) 에 「unwinding contained panic: <문장> @ <자리>」로 덧붙인다 — 가둔 A 가 되감는 중 `Drop` 이 오염된 락의 `.expect` 로 B 를 내고 B 가 그 소멸자를 벗어나 abort 하면, 5판은 B 의 사유만 남겼다(A 의 칸은 경계가 읽기 전에 프로세스가 끝난다). (a″) 가 (a′) 앞인 까닭 — 같은 깊이의 B 가 (a′) 에서 A 의 칸을 덮는다. 남는 것 = B 가 가둔 **비오염** 패닉이면 훅이 판정 직후 돌아가 abort 앞에 아무것도 안 남는다(§4-11).

- **가둔 비오염 패닉은 판정 직후 돌아간다(5판 — codex BLOCK)** — 4판은 모든 패닉을 (e) · (f) 로 보냈다. 그 스레드가 tracing 락을 쥔 채 패닉하면(예: reload 층 읽기 락 아래 `info!(?x)` 의 `Debug` 구현 패닉 — `base/src/logging/mod.rs:404`) (e) 가 자기를 기다린다 — 가둔 패닉이라 당김이 없으니 감시도 놀고, 그 스레드 · 곧 그 락을 기다리는 모든 로거가 멎는다(지금 abort 훅에도 같은 자기교착이 있다 — 기존 · 고치기 싸다). 진단은 가둔 경계로 옮긴다: (a′) 의 칸(문장 · 파일 각 256바이트까지 · 줄 · 열 — 6판)을 `contain` · `Contained` 의 catch 갈래가 잡은 payload 문장과 함께 한 줄 로그한다 — 되감기가 끝나 그 락을 놓은 뒤다(§4-1). 잃는 것 = 백트레이스 · 가둔 패닉의 기본 훅 stderr 줄 — 받는다(§4-11). 올리는 패닉은 그대로 판정 → 표지 → 깃발 → 사유 → 로그 → 기본 훅이다(가둔 오염 패닉도 올리는 쪽이라 자리 칸과 사유 둘 다 남는다).
- **순서의 까닭(4판 — T3)** — 3판은 사유 문장을 깃발 **앞**에서 만들었다(할당 · 포맷 — 시한 없는 일이 깃발보다 먼저). 이제 깃발까지는 빌림 · 원자값뿐이고 할당은 (d)부터다. 대가 = 꼬리가 사유보다 먼저 끝날 수 있다(1회차 걱정) → 꼬리가 마지막 끝내기 앞에서 `wait_reason(1초 · 가안)` 으로 기다린 뒤 있는 것을 로그한다 — 올리는 훅이 (b′)~(d′) 사이에 있을 때만 기다린다(5판 · 셸 `Exit` 진입 갈래도 같다 — 6판부터 셸 게시 닫힘은 기다리지도 로그하지도 않는다 · §4-4 · §4-9). 그래도 비면 받는 진단 손실. ★「할당 없음」은 「훅 **코드**는 깃발까지 할당 없음」이다(5판)★ — std 는 훅을 부르기 전에 형식 payload(`FormatStringPayload` — `Err` 위의 `.expect` · `.unwrap` 전부)를 만든다(추론 · rust-src 없음). 그 할당이 할당기 안에서 패닉하면 abort 다 — 무해하다.
- **훅이 잡는 락 — (a)~(c) · (a′) · (a″) · (b′) 만 락이 없다** — (d) 는 할당하고, (e) 의 `tracing::error!` 는 tracing 의 reload 층 · `EnvFilter` 내부 `RwLock`(`base/src/logging/mod.rs:404` · M4) · 파일 싱크 Mutex · stdout 잠금을 지나고 (f) 기본 훅은 stderr 를 잠근다. 깃발을 그보다 먼저 세우는 것이 이 순서의 요점이다 — (d)~(f) 가 멎어도 깃발은 섰다. 그 락을 **다른 스레드**가 기다리며 멎는 것(accept 루프 · 꼬리의 로그)은 (바) 감시(M19)가 끝낸다.
- **`MAIN_ESCALATED` 를 당기기 앞에(D1)** — 깨우기는 깃발을 본 뒤 표지를 읽는다(§4-9). 「주 스레드인가」는 셸 `run()` 머리가 세우는 const TLS `Cell<bool>`(`IS_MAIN`)로 판정한다 — `thread::current()` 는 할당 없음을 기댈 수 없다(5판). 표지가 깃발 뒤에 서면 깨우기가 그 틈에 「배경 스레드 패닉」으로 읽고 `app.exit` 를 부른다(M18 이 막으려던 경합 — 5판까지). 대안 = 원자값 하나에 「주 스레드가 당김 / 그 밖이 당김」 — 같은 효과라 둘 중 무엇이든(U6 가 고른다). ★6판부터 이 표지는 안전에 필요하지 않다★ — 깨우기는 `app.exit` 대신 주 스레드에 닫힘을 게시하고, 주 스레드가 패닉했으면 tao 가 그 닫힘을 처리기 없이 버린다(M18 · 추론). 5판의 「순서로 못 막는 변형」(배경 먼저 · 주 스레드 나중)도 해가 없어졌다(§4-11).
- **오염 패닉은 표시와 무관하게 올린다** — (가)=P2 에서도 같다: 범위 락은 범위 갈래로 되찾아 패닉하지 않으므로, 이 규칙에 걸리는 것은 공유 락의 `.expect` 와 아직 분류 안 된 자리뿐이다(종료 쪽 — 안전한 쪽).
- **조립(`run()`):** 로깅 → 훅 설치(지금 `:458`) + `sync::set_poison_observer(|at| { ESC.record_site(at); ESC.pull() })` → **곧바로 감시 스레드(M19 · §4-4 — `cfg(panic = "unwind")` 빌드만)** — 그 뒤에야 부팅이 이어진다. 부팅 중에 이미 당겨졌으면 accept 루프의 interval 팔이 첫 틱에서 본다(watch 가 훅보다 늦게 생기는 것 — §2-5 — 을 폴링이 흡수한다). ★accept 루프 **앞** 부팅 걸음에서 당겨진 채 멎으면 깃발을 보는 이가 루프뿐이라 깨울 길이 없다★ — 그런 대기는 못 찾았고(리뷰 · 확신도 낮음), 감시가 훅 설치부터 돌아 덮는다(§4-11).
- **이중 당김 · 종료 중 패닉** — `pull` 은 멱등이다. 꼬리가 도는 중의 패닉은 로그 · 사유 기록(이미 섰으면 무시)만 남는다.
- **시험(M20)** — 훅 · 관찰자 등록은 `run()` 에서만 선다(§2-1). 시험은 정책 함수(조립부의 순수 함수 — 걸쇠를 인자로)와 지역 `Escalation` 인스턴스로 잰다 · 관찰자 등록 상태는 자기 통합 시험 바이너리(§4-1).

### 4-4. 종료 (데몬)

- **깨우기 = accept 루프의 interval 팔**(M3) — 깃발이면 `shutdown_tx.send(true)` → 루프 탈출 → `sweeper.stop()`(`lib.rs:438`) → 꼬리. 보내는 신호는 **`StopDaemon` 이 보내는 그 watch 의 `true`** 다(ADR-0288 결정 3 「기존 정상 종료 경로 · 같은 신호」). ★3판 정정★ — 그 watch 의 운영 구독자는 accept 루프 **하나뿐**이고(2판의 「명령 자리 수거 태스크 등 다른 구독자」는 틀렸다 — 수거기는 루프 뒤 `sweeper.stop()` 으로 멈춘다), `shutdown_tx` 는 `AgentConnections` 로 옮겨진다(`lib.rs:383`) → interval 팔은 그 **앞에서 뜬 사본**으로 보낸다. 보내는 까닭은 「같은 신호」를 한 길로 두는 것이고, 루프는 보내자마자 스스로 빠진다.
- **꼬리의 사유 로그**(깃발이면 「패닉으로 정상 종료 — 사유: … · 오염 감지 자리: …」) — 꼬리 머리에서 한 번(사유가 이미 적혔으면 함께 · 꼬리 뒤 걸음이 멎어도 남는다) · ★꼬리 끝 `Err(101)` 앞에서 머리가 사유를 못 찍었으면 `wait_reason(1초 · 가안)` 으로 기다린 뒤 있는 것을 찍는다(4판 — 훅이 사유를 깃발 뒤에 적으므로 · §4-3)★ — 올리는 훅이 진행 중일 때만 기다린다(5판 — 셈이 0 이면 곧바로 · 관찰자 · M21 만 당긴 경우 헛된 1초가 없다). 단 로그 락 자체가 멎었으면 꼬리는 이 줄에서 멎는다 → 감시(아래)가 끝낸다. 꼬리 끝에서 깃발이면 `Err(101)` → `main.rs` 의 101 갈래가 `terminate_self(101)`(M7 · 아래 「당겨진 끝」 — 당기지 않은 끝은 `process::exit` 그대로).
- **당겨진 끝 = `terminate_self(101)`(5판 — M7)** — `process::exit` 밑의 `ExitProcess` 는 부른 스레드 밖 스레드를 — 감시까지 — 먼저 끝낸 뒤 로더 락 아래 `DLL_PROCESS_DETACH` 를 돈다(MS 문서 순서 · 실측 아님). 그 분리 알림이 멎으면(셸의 WebView2 · COM DLL — 미검) 덮을 이가 없다. 그래서 당겨진 끝은 전부 마지막 로그 줄 뒤 platform `terminate_self(101)` 로 끝낸다 — **데몬(U5)** = 꼬리 끝 · M6 포착(`main.rs` 의 101 갈래 둘) · `main.rs` 의 101 아닌 갈래의 깃발 재확인(6판 · 아래 메인 future) · **셸(U6)** = 깨우기가 주 스레드에 게시한 닫힘(트레이 정리 뒤 · 6판) · `RunEvent::Exit` 콜백 끝 재확인(잡혔거나 당겨졌으면 · 6판) · 주 스레드 패닉(셸 `main.rs` 가 `run()` 을 `catch_unwind` 로 감싼다 — tao 가 `App::run` 밖으로 `resume_unwind` 한다 · §4-9). 정상 끝 · 부팅 실패(`Err(1)`) 같은 당기지 않은 끝은 `process::exit` 그대로다(데몬은 그 직전 깃발 재확인 — 6판).
- **감시 스레드(M19 · (바)=감시)** — `run()` 이 훅 설치(`:458`) 직후 띄우는 이름 붙은 std 스레드 하나(가안 `engram-panic-watchdog` · ★`cfg(panic = "unwind")` 빌드만 — abort 에선 할 일이 없다★): `esc.wait_polling(≈1초)`(5판 — 시한이 초 단위라 1초 늦음은 예산 안 · accept 루프 interval 팔 200ms 는 그대로) → 본 시각 기록 → 시한(가안 30초 · 주입)까지 `park_timeout` 고리 → 아직 살아 있으면 platform 즉시 끝내기 `process::terminate_self(101)`(가안) 를 **직접**. ★`tracing` · 락 · 할당을 쓰지 않는다★ — 패닉한 스레드가 tracing 락을 쥐고 멎었거나 tokio 런타임이 통째로 멎어도 돈다. 사유는 이미 걸쇠의 `OnceLock` 에 있고, 그 경우 꼬리 로그 줄을 잃는 것은 받는다. 정상 경로(꼬리가 시한 안에 `terminate_self(101)` — 5판)와 겹쳐도 둘 다 같은 코드다. 기동 실패 = **부팅 실패**(`Err(1)` — 부팅 실패 갈래와 같다 · 감시 없이 돌지 않는다 — 스레드 하나를 못 띄우는 프로세스는 곧 다른 기동에서도 실패한다). 훅 설치 때 서므로 accept 루프 앞 부팅 멎음도 덮는다(§4-3). 주입(M20): 걸쇠 · 시한 · 끝내기 동작(운영 = `terminate_self` · 시험 = 채널에 기록) — 시험은 시한 수십 ms 로 「당김 → 시한 뒤 끝내기 한 번」 · 「안 당김 → 안 부름」. 깃발 없는 StopDaemon · 트레이 「데몬 끄기」에는 서지 않는다 — 그 꼬리 도중 깃발이 서면(§3-1 (바)) 그 끄기도 시한 · 101 을 받는다.
- **platform 즉시 끝내기(가안 `engram_dashboard_platform::process::terminate_self(code: u32) -> !` · 4판 — T2 · 모양 5판)** — Windows = `TerminateProcess(GetCurrentProcess(), code)`(`Win32_System_Threading` — platform 이 이미 켠 feature · `TerminateProcess` 는 `group/windows.rs:46` 이 이미 쓴다) 뒤 잠듦 고리 — `TerminateProcess` 는 비동기라 돌아올 수 있고 FALSE 도 낸다 → `-> !` 를 지키려 `loop { sleep }`. ★`abort()` 로 대신하지 않는다★ — fastfail 이 다른 종료 코드로 경합한다. 그 밖 = `std::process::exit(code)` 자리채움(5판 — 지금 non-Windows 갈래와 같은 꼴 `process.rs:385-411` · ★`extern "C" _exit` 를 새로 두지 않는다★: platform 헤더 규칙 3 `lib.rs:38-43` · ADR-0266 결정 7 — 다른 OS 구현을 새로 만들지 않는다). cfg 는 platform `process.rs` 안에서만 가르고 부르는 쪽(데몬 · 셸의 감시와 당겨진 끝)은 cfg 없이 부른다(ADR-0266). 로그 · 할당 · 락 없음. ≈ 10~15줄 · U1 은 책임 목록 둘도 고친다 — `platform/src/lib.rs:11-12`(입주자 `process` 설명) · `process.rs:1-3`(모듈 머리). 단위 U1(§7-1 — 소비자 U5 · U6 가 다른 물결이라 둘 다보다 앞선 단위에 둔다). 운영 시험은 U5 실프로세스 ① · ⑤(당겨진 끝) · ⑥(감시가 끝냄 → 코드 101)이 한다 — 단위 시험은 자기 프로세스를 끝내므로 두지 않는다.
- **`run()` 지역값(M23)** — 패닉(M6 되감기) · `Err(101)` 갈래 모두 `exit` 앞에서 지역값이 drop 된다. 관리자의 `Drop`(reaper 시한 없는 join `manager.rs:3219`)과 추적기의 `Drop`(join + `.expect` `session_tracker.rs:200` — 되감는 중이면 abort)이 그 사이에 멎거나 죽인다. → 두 `Drop` 은 `thread::panicking()` 이면 join 을 건너뛰고 `.expect` 를 없애며(U4), `Err(101)` 갈래는 반환 전에 관리자 `Arc` 하나를 `mem::forget` 한다(U5). 단일 인스턴스 가드 · `daemon.json` 같은 그 밖의 지역값은 평소대로 drop 된다.
- **StopDaemon 과 다른 점** — StopDaemon 은 `shutdown_all` 을 먼저 돌리고 답한 뒤 보낸다. 패닉 경로는 답할 상대가 없으니 곧바로 보내고 `shutdown_all` 은 꼬리 ③ 이 돈다. ★깃발 뒤에 온 StopDaemon(WS 허용 목록 · G2)은 `shutdown_all` 을 다시 돌리지 않고 `Ok` 로 답한 뒤 같은 신호 · Close 한다(5판 — `connection_core.rs:1542-1543` 의 `spawn_blocking(shutdown_all)` 을 깃발이면 건너뛴다 · `shutdown_all` 은 재진입 막이가 없다 `manager.rs:3113` · 꼬리 `lib.rs:823-824` 가 진다 · 연결이 닫혀 클라이언트는 `DaemonClosed` 를 그대로 본다 · M12)★. `shutting_down` 을 먼저 세우므로(ADR-0019) reaper 는 프로필을 손대지 않는다 — 다음 부팅은 지금처럼(운영 데몬의 부팅 자동 복원은 꺼져 있다 — `lib.rs:781-787` · 2026-07-09 사용자 결정의 stopgap) 활성화로 이어받는다(첫 제출 뒤 영속된 세션 id — ADR-0226).
- **(다)=거절** — 가장 바깥 입구 셋(WS 명령 해석 직후 — 정리 꼴 명령 제외 · 제어 HTTP 의 제어 라우트 — MCP 경로 제외 · MCP 도구 감싸개 — 4판)만 `is_pulled()` 면 새 일을 오류로 답한다(§4-6). 안쪽 홉 · 연결 수명 · 정리 걸음은 깃발을 보지 않는다.
- **메인 future · 부팅(M6)** — `main.rs`: `match AssertUnwindSafe(run()).catch_unwind().await { Ok(Ok(())) => { if esc.is_pulled() { terminate_self(101) } }, Ok(Err(101)) => terminate_self(101), Ok(Err(c)) => { if esc.is_pulled() { terminate_self(101) }; exit(c) }, Err(_) => { 사유 로그; terminate_self(101) } }`(5판 — 101 갈래 둘 = 당겨진 끝 · ★6판 — 101 아닌 갈래(`Ok` · `Err(1)`)도 끝내기 직전 깃발을 다시 본다★: 꼬리 끝 확인 뒤 · 부팅 실패와 겹쳐 든 당김이 `process::exit` · main 반환의 `ExitProcess` 로 새지 않게 — 데몬 꼬리 · 셸 `Exit` 콜백 끝과 같은 재확인 · 영향 작음 · `esc` = 조립 자리라 전역 `escalation()`). accept 루프도 같은 꼴로 감싸 잡히면 꼬리로 — ★감싸는 자리는 `run_accept_loop` **안**의 `select!` 고리★(호출 `lib.rs:794-808` 을 통째로 감싸면 잡힐 때 `sweeper.stop()` `:438` 을 건너뛴다 · 3판 정정).

### 4-5. 락 오염 ((가)=P2)

1. **공유 락의 `.expect` · `.unwrap`** — 오염이면 둘째 패닉 → 훅이 문장으로 알아보고 표시와 무관하게 올린다. 그 패닉은 표시 안이면 그 요청 · 에이전트 실패로 끝나고 데몬은 곧 정상 종료한다.
2. **공유 락의 base `sync` 되찾기(관찰 갈래)** — 되찾아 계속 가되(종료 경로가 끝까지 가야 하므로) 관찰자가 위치를 적고 당긴다.
3. **에이전트 · 연결 소유 락** — 범위 갈래로 되찾고 계속. 옮길 자리(파일 단위): codex `transport.rs`(되찾기 42) · `input_queue.rs` 7 · `stdio.rs` · `pty.rs`(되찾기 · `.expect` 6) · `output_core.rs`(`.expect` 27 — `finish` 의 `replay` `:692` · `enter_exiting` `:803` · 진단 `:1052` 포함) · leftover(`GateCell` 감싸개 · 되감기 Drop) · 래치 · `session.rs` 순서 락 · `agent_conn.rs`(`ConnUsageOutlet` `:212-215` · `on_disconnect` 의 연결 전용 `.expect`). **공유로 남는 것**: 명부(`manager.rs`) · 프로필(`persistence/mod.rs` 7) · 제어 레지스트리 · 추적기 · 메시징 · 사용량.
   - **메인 규칙 — 데이터 없는 순서 락(`Mutex<()>`)은 P2 에서 범위로 되찾는다**(3판) — 순서 락이 지키는 데이터는 자기 락과 오염 정책을 따로 갖는다(오염 정보는 거기 남는다). 그래서 공유인 `name_allocation`(`manager.rs:694`)도 범위다. ★U4 가 명시로 바꾼다 — 기본값 관찰에 두지 않는다★(두면 이름 할당 중 패닉 하나가 몇 분 뒤 남의 활성화로 데몬을 끈다 — §3-1 (가)의 「순서 락 함정」).
4. **종료 경로를 끝까지(M14)** — §2-5 의 락 자리 전부 되찾기. `thread::scope` 덕에 한 에이전트의 kill 패닉이 남의 kill 을 막지 않는 성질은 그대로 둔다.
   - **kill 길의 상태 싱크(M21 · 3판)** — `kill_agent` 의 `enter_exiting`(`manager.rs:3069`)이 부르는 운영 status sink 사슬(`lib.rs:327-339` — 바닥 고리 `DaemonStatusSink`)은 net `ConnRegistry::broadcast_text` 의 `.expect("conn registry poisoned")`(`net/src/ws.rs:212`)에 닿고, 이것이 `session.kill` **앞**이다. 그 락 오염이 종료를 당긴 원인이면 kill 스레드마다 여기서 패닉해 2동사가 하나도 안 돈다(자식은 Job 이 닫힐 때까지 산다). → `DaemonStatusSink` 의 `broadcast_text` 호출(`status_fanout.rs:47-`)을 `contain` 으로 돌리고, 잡히면 잡은 문장을 첫 사유로 적고 걸쇠를 당긴 뒤(공유 팬아웃 패닉 = 3층 · 오염이면 훅이 이미 당겼다) 그 알림만 버린다 — 가둠 · 버림은 U4, 사유 · 당김은 U5(M21 · 4판). 펌프 `finish` 의 terminal 알림도 같은 사슬을 지나 함께 덮인다. **net 은 손대지 않는다**(M5). 거부: net 쪽 되찾기(M5) · 종료 경로만 싱크 패닉을 견디게 하기(평시 요청 경로의 같은 패닉은 1층에 갇혀 — 오염이 아니면 — 공유 팬아웃 패닉이 조용히 산다).
5. **바꾸지 않는 것** — 위 밖의 공유 `.expect` 자리는 그대로다(1 이 덮는다).

### 4-6. 1층 입구 (데몬)

- **WS 처리기((마)=N1a):** `on_text` 는 명령을 해석한 뒤 처리를 `Contained` 로 감싸고, 잡히면 `command_request_id`(`agent_conn.rs:433` 이 이미 쓰는 것)로 `request_id` 를 실은 `Error` 를 낸 뒤 연결을 계속한다. `on_binary` · `on_inbound_saturated` 는 같은 꼴 · `on_connect` 는 신호가 `BoxFuture<()>`(`net/src/frame_port.rs:164-168`)라 오류로 못 돌려준다 — 잡히면 로그 + 그 연결 큐에 닫기를 넣는다(`on_text` 의 Close 갈래가 쓰는 길 — U3 가 확인) · `on_disconnect` 는 잡히면 로그 · 계속(net 의 `unregister` 가 이어서 돈다). 두 수명 처리기의 공유 걸음은 (나).
- **떼어 낸 명령 · 배달 태스크 · blocking:** 데몬 도우미 둘(가안 `spawn_contained` · `spawn_blocking_contained` — 새 작은 모듈 가안 `containment.rs`) — 잡히면 그 자리의 기존 `JoinError` 갈래와 같은 오류로 접는다(활성화 = `SpawnFailed` · 1단계 본문 = 기존 로그 갈래). 버스 1단계 본문은 그 안에서 `command::route` 가 다시 잡아 `INTERNAL` 로 답한다(표시는 바깥이 이미 세웠다). 도우미는 깃발을 보지 않는다(안쪽 홉 — 아래 (다)). 걸쇠는 도우미 인자 · `ConnectionCore` 칸으로 받는다(M20 — 전역은 조립 자리만).
- **제어 HTTP:** axum `middleware::from_fn` 하나가 `next.run(req)` 를 `Contained` 로 감싸 잡히면 500 + 문장 · **마지막 `.layer`(가장 바깥)**. 처리기 본문 `spawn_blocking` 여섯(`mcp_server.rs:513` · `:940` · `:1040` · `:1092` · `:1173` · `:1249`)은 **`spawn_blocking_contained` 로 옮긴다** — blocking 스레드는 깊이가 0 이라 바깥 층의 표시가 닿지 않는다(2판의 「`contain` 으로 감싼다」는 맨 `spawn_blocking(` 을 남겨 M9 소스 시험과 어긋났다 · 3판). 이 파일에서 남는 맨 spawn 은 serve 태스크 `:1411` 하나다(M9). **MCP 도구 셋은 몸통을 직접 감싼다**(rmcp 요청 태스크 안 — §2-4) → 잡히면 도구 오류 결과. 바깥 층 · 거절 층 · 도구 감싸개는 걸쇠를 MCP 서버 상태로 받는다(M20 · 4판).
- **(다)=거절 — 가장 바깥 입구에서만**(§3-1 (다) · 3판 · 4판 정정): ① WS `on_text` 의 명령 해석 직후(옛 명령 · 버스 명령) — ★정리 꼴 명령은 허용 목록(StopDaemon · 구독 해지 · 뷰포트 놓기 꼴 — U3 가 명령 어휘에서 고른다)★: 「정리 걸음에는 두지 않는다」를 이 입구에서도 참으로 둔다 · 허용된 StopDaemon 은 깃발 뒤 `shutdown_all` 없이 `Ok` 로 답한다(§4-4 · M12 · 5판) ② 제어 HTTP 의 **제어 라우트만**(`agents.json` 을 쓰는 `agent.*` 동사도 이 길이다 · 2판은 이 길을 거절 밖에 뒀다) — ★앱 전체 층이 아니다★: `nest_service(MCP_PATH, …)`(`mcp_server.rs:1372`)도 같은 `.layer`(`:1401-1407`) 아래라 거기 두면 rmcp 세션 DELETE · SSE 끊기까지 거절한다. 모양 = `ControlRoute::ALL` 고리(`mcp_server.rs:1373-1400`) 안 각 갈래의 라우트에 거는 `.layer`(라우트마다) 또는 바깥 층 안의 경로 확인(`ControlRoute::ALL` 경로와 대조) — ★맨 `route_layer` 는 걷는다(5판)★: axum 0.8.9 `route_layer` 는 그때까지 붙은 엔드포인트를 전부 감싸고, 고리보다 먼저 붙은 `nest_service(MCP_PATH)`(`:1372`)도 그 안에 든다(`routing/path_router.rs:311-332` · `:246-270`). 둘 중 무엇이든 U3 가 고르고 「MCP 경로는 거절 안 됨」 시험은 그대로 둔다 ③ MCP 도구 감싸개 — MCP 의 거절은 이 한 곳이고 모양은 N1a 도구 오류 결과. 셋 다 `is_pulled()` 면 실행 없이 오류 답(「daemon is shutting down after an internal error」 — 가안). ★두지 않는 자리★: 안쪽 홉(버스 배달 태스크 → 1단계 blocking · 활성화 blocking · 두 도우미) · `on_connect` · `on_disconnect` · Close · 정리 걸음 — 이미 받은 일을 가운데서 끊거나 정리를 막는다. 2판의 「두 도우미와 WS 처리기 입구」는 안쪽 홉을 끊고 HTTP 를 빠뜨려 걷었다.
- **net · 연결 태스크:** 손대지 않는다(M5).

### 4-7. 2층 입구 (에이전트) · 불변식

- 기존 자리의 `catch_unwind(AssertUnwindSafe(…))` 를 `contain(…)` 으로 바꾼다 — 잡은 뒤 결말 코드는 그대로다.
- **PTY 감시(B1′):** 감시 몸통을 `contain` 으로 감싸고 잡히면 「자연 종료 때 하는 일」을 한다. ★순서가 load-bearing 이다(3판)★ — ① 통로의 실패 사유 칸(가안)에 「pty watcher panicked: …」를 **먼저** 싣는다(칸 락 아래 쓰기 또는 Release store) ② 그다음 자식 끄기(`sync` 범위) ③ master 놓기. 펌프는 EOF 를 본 **뒤**, ★`shutdown` · `try_wait` 분류(`pty.rs:321-333`) **앞**에서★(4판 — 뒤면 감시가 자식을 껐으니 `Exited{code}` 로 분류된다) 그 칸을 읽어(같은 락 또는 Acquire) 있으면 `Error(사유)` 로 끝낸다. 거꾸로면 펌프가 EOF 로 먼저 `finish` 해 finalize 1회(ADR-0005)가 `Exited` 로 굳고 사유를 잃는다. 종료 전이는 여전히 펌프 단독(ADR-0005)이고 새 정리 호출자가 아니다(ADR-0127).
- **stderr 배수(B1′):** 줄마다 `contain`. ★첫 패닉을 잡은 뒤에는 해석을 멈추고 읽어 버리기만 한다★(3판 — 해석 쪽 결함이면 줄마다 다시 패닉해 패닉 · 로그 폭풍이 된다) — 파이프는 계속 비워진다. 진단 락은 (가)=P2 면 범위 갈래다.
- **`send_input` 패닉 포착(M22 · 4판 — 3판 스코프 가드를 바꿨다)** — 유령 대기 입력(§3-1 (가) †)을 좁힌다. 분류 쓰기(`session.rs:284` `write_user_classified`)의 「먼저 emit, 뒤에 send」 갈래(`emit(Queued)` `:303` → `send_input` `:304`)에서 `send_input` 을 맨 `std::panic::catch_unwind` 로 감싼다(기존 `Err` 갈래는 그대로 스스로 `Dropped` 를 낸다 · `AssertUnwindSafe` 는 필요하고 정당하다 — `dyn AgentTransport` 가 `RefUnwindSafe` 가 아니고, catch 뒤에는 `core.emit(Dropped)` 만 돌며 통로 상태는 다시 풀기 전에 읽지 않는다 · 5판). 잡히면 그 id 의 `Dropped{cause: Rejected}` 를 내고 `resume_unwind` 로 다시 풀어 보낸다 → 대기 목록 표가 그 에이전트를 「바쁨」으로 붙들지 않고, 패닉은 바깥 입구까지 원래대로 간다. ★`contain` 이 아니다★ — 훅은 패닉 순간 원래 깊이로 이미 판정했고(`resume_unwind` 는 훅을 다시 부르지 않는다), `contain` 은 깊이를 올려 그 판정을 바꾼다. `Dropped` 는 되감는 중이 아니라 catch 갈래에서 내므로 abort 갈래가 없다 — emit 이 오염으로 패닉하면 그 패닉이 대신 풀린다(오염 패닉 → 훅이 올린다). 그래서 3판의 「P1 · P1o 면 `is_poisoned()` 로 건너뛴다」와 그 확인 뒤 행동 경합이 없어졌다 · (가) 어느 안이든 같다. 「먼저 send, 뒤에 emit」 갈래(`:300-301`)는 send 패닉이면 `Queued` 가 안 나가 유령이 없다. 남는 거짓 `Dropped` = §4-11. 파일 = `session.rs`(U4).
- **finalize 1회(ADR-0005):** 펌프의 `finish` 는 지금처럼 `contain` **밖**에서 한 번 불린다.
- **정리 호출자 둘(ADR-0127):** 새 호출자를 만들지 않는다. B2 를 고르면 그 스레드는 「통로가 자식을 끄고 실패 사유 칸을 남긴다」까지만 하고 종료 전이는 펌프가 낸다.
- **kill 인과(ADR-0001):** 정상 종료는 `shutdown_all` → `kill_agent`(`begin_retire` → … → `session.kill`) 그대로다. §4-5 의 되찾기는 같은 줄에서 락 획득 방식만 바꾼다.
- **락 순서(ADR-0006) · leftover 잎 락:** 훅 **코드**는 깃발 당기기까지(판정 · 무장 칸 스택 사본 · 칸 베낌 · 표지 · 진행 셈 · 깃발 — 4판 · 5판 · 6판) 락도 OS 호출(unpark)도 할당도 없다 — std 가 훅을 부르기 전에 만든 형식 payload 는 별개다(§4-3 · 5판). 가둔 비오염 패닉은 자리 베낌 뒤 돌아가 락을 하나도 안 잡는다(5판). 올리는 패닉은 그 뒤 사유 문장을 할당하고, 로그 · 기본 훅은 락을 잡는다(tracing reload · `EnvFilter` · 파일 싱크 · stdout · stderr — §4-3 · 2판의 「로그 싱크 말고 아무 락도 안 잡는다」는 틀렸다). 그 락에서 멎어도 깃발은 이미 섰고, 남이 그 락에 막히는 멎음은 (바) 감시가 끝낸다. 관찰자는 원자값 둘뿐이다(M17).
- **등록 순서(ADR-0019):** 손대지 않는다 — (나)의 스코프 가드는 등록 순서를 바꾸지 않고 실패 갈래만 더한다.

### 4-8. 여러 걸음 공유 변경 구간 ((나)=E)

- 활성화 등록 창(명부 insert 앞 seed · register 부터 `start_pump` 까지) · 요청에서 부른 `kill_agent` 의 앞 줄 · 연결 수명 처리기의 공유 걸음(U3 가 분류)을 `escalating(…)` 으로 감싼다. 패닉하면 훅이 올리고, 패닉은 바깥 입구까지 풀려 그 요청은 기존 오류로 답한다. `reap_one`(`:56-86`)은 감싸지 않는다 — 표시 없는 reaper 고리 안이라 이미 표시 밖이다(§4-2 · 3판).
- **스코프 가드(M15)** — 활성화 창에서 빠지면(패닉 · 오류) `transport.shutdown()`. ★가드가 쥐는 것은 걸음에 따라 바뀐다(3판)★ — 통로가 `AgentSession` 으로 옮겨지기 전에는 통로를, 옮겨진 뒤에는 그 세션 핸들을 쥐고 같은 `shutdown` 에 닿는다. 펌프에 넘긴 뒤에는 가드를 끈다. 명부에 이미 올랐으면 `shutdown_all` 이 다시 끄므로 `shutdown` 이 두 번 — 그리고 **동시에**(StopDaemon 과 패닉 꼬리 · §9-8) — 불려도 되는지 U4 가 시험으로 본다 — 두 번은 확인 시험 · 동시는 패닉 · abort · 멎음이 없다는 **연기 시험**(4판 — 자식 락이 두 부름을 줄 세워 seam 없이는 실패할 수 없다 · §9-8).
  - ★되감는 중의 `shutdown` 은 그 안 락이 되찾기여야 한다★ — 지금 `pty.rs` 의 자식 · master `.expect`(`:412` · `:421`)가 오염을 만나면 `Drop` 안 둘째 패닉 = abort. 그 되찾기는 U2(통로 파일 주인)가 하므로 **U4 는 U2 뒤**다(§7-2 · 3판).
  - 가드는 등록을 되돌리지 않는다 — 활성화 창이 올린 턴 관측(`turns`) · 대기 목록 표(`inputs_pending`) 등록은 남는다. E 에서는 곧 종료라 무해하고, (나)=R 이면 되돌림 목록에 든다.
- 감시 기동 `.expect("spawn pty watcher thread")`(`pty.rs:265`) · 펌프 맨 `thread::spawn`(`:269`)은 버그가 아니라 자원 실패다 — 오류로 바꿔 `start` 를 실패 가능하게 하는 것은 R 쪽 일이라 E 에서는 그대로 둔다(그 패닉도 E 가 정상 종료로 올린다).

### 4-9. 셸 ((라)=S1a)

- 같은 훅 모양(§4-3)이고 셸은 표시를 세우지 않으므로 **모든 패닉이 당긴다.** 설치 = 셸 `run()` 머리 — ★`tauri::Builder`(`lib.rs:45`) **앞**(5판)★ — 주 스레드 표지 `IS_MAIN`(const TLS `Cell<bool>`) 세움 + 훅 + `sync::set_poison_observer` + **곧바로 감시 스레드**(M19 · 아래 — `cfg(panic = "unwind")` 빌드만). 4판의 「로깅 뒤」는 틀렸다 — 로깅은 두 번째로 등록되는 boot 플러그인(`lib.rs:52-60`) 안에서 선다(`state/boot_plugin.rs:89`) → 그 뒤로 미루면 플러그인 setup 앞 패닉 · 멎음을 못 덮는다. 그 subscriber 가 서기 전 훅의 (e) 는 무동작이다 — 사유는 걸쇠에만 남는다. ★배포판 셸은 콘솔이 없다(`src-tauri/src/main.rs:2` `windows_subsystem = "windows"`) → (f) 의 stderr 도 어디에도 안 남아, 로깅 앞 당김의 흔적은 종료 코드 101 뿐이다(지금 abort 보다 나쁘지 않다 · 6판 정정 — 5판의 「(f) 의 stderr 에 남는다」는 debug 빌드에만 맞다)★. 감시 기동 실패 = 그 자리에서 `process::exit(STARTUP_FAILED_EXIT_CODE)`(`startup_failed`(`lib.rs:128`) 길은 아직 없다 · 당기지 않은 끝).
- 깨우기 스레드는 setup 에서 `AppHandle` 과 걸쇠(M20)를 받아 띄운다: `esc.wait_polling(200ms)` → `MAIN_ESCALATED` 가 아니면 `app.run_on_main_thread(닫힘)` 한 번 — **그것뿐이다**(6판 · 시한 · 로그 없음 · ★`app.exit` 를 부르지 않는다★ — M18). 닫힘 = `catch_unwind(AssertUnwindSafe(|| handle.cleanup_before_exit()))`(트레이 아이콘 · best effort) → `terminate_self(101)`. 게시가 `Err` 면(이벤트 루프가 없다) 더 하지 않는다 — 끝은 감시 시한이다. 당겨진 길은 `RunEvent::Exit` 에 닿지 않아 `Final` 이 안 쓰인다 · 실행 잠금은 끝내기가 핸들을 닫아 OS 가 푼다(`state/lock.rs`). setup 전에 당겨졌으면 첫 확인에서 게시한다. 기동 실패 = setup 실패(시작 실패 갈래).
  - ★`run_on_main_thread` 원문 대조(6판)★ — tauri 2.11.3 `AppHandle::run_on_main_thread`(`app.rs:494-499` · `-> crate::Result<()>`) → wry `run_on_main_thread`(`tauri-runtime-wry 2.11.3 lib.rs:2777-2779` — `Message::Task`) → `send_user_message`(`lib.rs:235-255`): 부른 스레드가 주 스레드면 그 자리에서 돌리고, 아니면 `proxy.send_event` 의 실패를 `Error::FailedToSendMessage` 로 돌려줄 뿐이다 — **로그 · `cleanup_before_exit` · `exit` 가 없다**(`AppHandle::exit` 의 실패 갈래 `app.rs:574-579` 와 다르다). tao 의 `send_event` = 주 스레드 숨은 창에 `PostMessageW` · 실패면 `EventLoopClosed`(tao 0.35.3 `platform_impl/windows/event_loop.rs:570-585`). 닫힘은 이벤트 루프 콜백 안 `Message::Task(task) => task()`(wry `lib.rs:3335`)로 주 스레드에서 돈다 — 루프가 살아 있을 때라 `cleanup_before_exit`(`app.rs:1108-1121` — 트레이 표 락 · 리소스 표 · 창 숨기기 · 창 숨기기는 주 스레드라 그 자리에서 처리)가 기대는 루프 · 창이 있다. 주의 셋: ① 게시 성공이 실행을 보장하지 않는다 — 루프가 LoopDestroyed 를 돌기 시작한 뒤(숨은 창은 아직 있다) 게시하면 `PostMessageW` 는 성공하고 닫힘은 안 돈다(추론) → 그 당김은 `Exit` 콜백 끝 재확인이나 감시가 받는다 ② 주 스레드가 모달 고리 · 막힘 중이면 닫힘이 늦거나 안 돈다(미검 · §9-15) → 감시 ③ `cleanup_before_exit` 는 「돌아오면 곧바로 끝내라」 계약(`app.rs:1106-1107`)이고 바로 뒤 `terminate_self` 가 그것을 지킨다 · 그 안 패닉(오염된 트레이 표 락 `app.rs:1110`)은 닫힘의 `catch_unwind` 가 받고 끝내기는 그대로 간다(닫힘은 tao 의 `catch_unwind` 안에서 돌지만 `terminate_self` 가 돌아오지 않아 tao 의 삼킴에 닿지 않는다).
  - ★셸 감시는 깨우기와 다른 스레드다(4판 — T1)★ — 3판은 시한을 깨우기에 얹고 「따로 두는 대안은 그 밖은 같다」고 적었으나 틀렸다: tauri 2.11.3 `AppHandle::exit`(`app.rs:574-579`)는 `request_exit` 가 실패하면 `log::error!`(log → tracing 다리 — `base/src/logging/mod.rs:422` 의 `try_init`) → `cleanup_before_exit()`(`tray.icons.lock().unwrap()` `app.rs:1110`) → `process::exit` 를 **부르는 스레드에서** 돈다 — 같은 스레드의 시한은 상한이 아니다. 감시 = 훅 설치 직후 띄우는 std 스레드 — 걸쇠 · 시한(가안 5초)만 받고 `AppHandle` 을 모른다 · 깃발을 본 뒤 시한이 지나면 `terminate_self(101)`(`Final` 안 씀 · **`MAIN_ESCALATED` 여도 간다** — 주 스레드 되던지기가 훅 로그 자기교착 · 막힌 주 스레드 · COM `Drop` 에서 멎는 경우((바)의 (1) · (2))가 바로 그것이다). 훅 설치 때 서므로 setup 앞 멎음도 덮는다. 로그 · 락을 쓰지 않는다. (6판 — 깨우기는 이제 `app.exit` 를 부르지 않지만 감시를 따로 두는 까닭은 같다: 주 스레드가 멎으면 게시한 닫힘도 안 돈다.) 사유 로그는 훅의 (e) 줄이 주된 기록이다 — 게시한 닫힘은 로그하지 않고(6판 — 정리 · 끝내기만) · `RunEvent::Exit` 진입 갈래(사용자 끝내기와 당김이 겹친 경우 — 아래)만 `wait_reason` 뒤 다시 찍는다 · 주 스레드 패닉은 `Exit` 사건이 없어 (e) 줄이 유일하다(§4-11).
- `RunEvent::Exit`(`lib.rs:401`) — ★몸통 전체를 `catch_unwind(AssertUnwindSafe(..))` 로 돌린다(6판 · M18)★: 진입 때 `is_pulled()` 면(6판부터 사용자 끝내기 · 시작 실패와 당김이 겹친 경우뿐 — 당겨진 길은 게시한 닫힘이 끝낸다) 사유를 `wait_reason(1초 · 가안)` 으로 기다려 로그(4판 — 훅이 사유를 깃발 뒤에 적는다 · 올리는 훅이 진행 중일 때만 기다린다 · 5판) → `Final` 쓰기는 건너뛰고 실행 잠금은 놓은 뒤 `handle.cleanup_before_exit()` · 아니면 지금처럼 `exit_session.shutdown()`. **끝에서** `if caught || esc.is_pulled() { terminate_self(101) }` → 그 뒤 기존 `startup_failed` 갈래(`lib.rs:408-409`) — 잡혔거나 당겨졌으면 101 이 시작 실패 코드보다 앞선다(6판). 끝 재확인이 닫는 것 = tao 가 이 콜백의 패닉을 삼키고 `process::exit(0)` 하는 길(tao `runner.rs:95-108` · `:153-175` · `event_loop.rs:225-231`) · 진입 확인 뒤 든 당김(`exit_session.shutdown()` 의 ≤2초 동안 등 — 그 당김의 닫힘은 루프가 끝나는 중이라 안 돌 수 있다). 이 줄들에서 멎으면 감시가 끝낸다. 콜백이 101 로 끝나면 tauri 의 뒤 정리(`app.rs:1433`)는 안 돈다(당겨진 갈래는 콜백 안에서 이미 불렀다).
- **셸 5초 예산(5판 · 6판)** — 당겨진 길 = 깨우기 폴링(≤200ms) + 주 스레드가 게시를 집어 들기까지 + `cleanup_before_exit` · 끝내기(추론 — 1초 안팎) · `Exit` 진입 갈래(겹친 경우) = `wait_reason`(≤1초 — 진행 중일 때만) + 종료 세션의 기록기 답 대기(≤2초 — `REPLY_DEADLINE` `state/saver.rs:44`) + `cleanup_before_exit` · 끝내기 ≈ 3.2초 + 정리. 둘 다 시한 5초 안에 든다 — 그 밖의 멎음만 감시가 끊는다. `exit_session.shutdown()` 에 「`Final` 없이」 갈래가 하나 늘고, 종료 세션은 걸쇠를 인자로 받는다(M20 — 시험이 전역을 안 뒤집게).
- **데몬은 건드리지 않는다** — `quit_app` 을 부르지 않는다.
- 주 스레드 패닉: 훅이 당기고 로그한 뒤 tao 가 되던져 `App::run` 밖으로 풀린다 → 셸 `main.rs` 가 `run()` 을 `catch_unwind` 로 받아 `terminate_self(101)`(5판 — U6 · 없으면 std 의 main 반환 길이 `ExitProcess` 를 탄다 · §4-4 「당겨진 끝」) → 종료 코드 101 · `Exit` 사건 없음 · `Final` 안 쓰임(S1a 와 같은 결말) · `cleanup_before_exit` 없음 — `main.rs` 포착 자리엔 `AppHandle` 이 없다(6판 — 받는다 · 트레이 아이콘이 남을 수 있다 — 미검 · §4-11). 깨우기는 표지를 보고 게시하지 않는다(게시해도 tao 가 패닉 뒤 사건을 버린다 — M18).

### 4-10. 프로필 · 컴파일 가드

- `Cargo.toml` `[profile.release]` 의 `panic = "abort"` → `panic = "unwind"`(지우지 않고 명시 + `# ADR-0292` 주석 — 기본값에 기대면 다음 사람이 「빠졌다」로 읽고 abort 를 다시 넣는다).
- 데몬 lib · agent lib: `#[cfg(not(panic = "unwind"))] compile_error!("… ADR-0292 …");`(M10). dev 프로필은 원래 unwind 라 시험 · 개발 빌드는 그대로다.
- CLI 는 가드 없이 함께 unwind 가 된다 — 패닉 종료 코드가 fast-fail(`0xC0000409`)에서 101 로 바뀐다.

### 4-11. 남는 위험 (알고 받는 것 — ★표는 사용자 서명이 필요한 것)

- ★**못 찾은 여러 걸음 공유 변경**★ — 1 · 2층 코드가 락 밖에서 공유 상태를 반쯤 바꾸다 난 패닉은 가둬지고 그 상태는 남는다. 이것은 ADR-0288 결정 1 이 받아들인 것이 **아니다**(1판의 오귀속 — 결정 3 은 공유 상태 패닉 = 종료라 적는다). (나)가 찾은 구간만 올리고, 못 찾은 구간은 ADR-0288 결정 3 과의 틈으로 남는다.
- ★**(가)의 대가**★ — P1: 락을 쥔 2층 패닉 대부분이 재시작 · P2: 요청이 반쯤 바꾼 에이전트 상태로 그 에이전트가 계속 돈다.
- **정상 종료가 멎는 경우** — 종료 시간 제한은 미룬 장치다(깃발 없는 StopDaemon · 트레이 끄기는 (바)와 무관하게 계속 시한 없음 — 그 꼬리 도중 깃발이 서면 (바)=감시가 덮는다 · 4판). 패닉 경로는 런타임 drop 대기를 `terminate_self(101)` 로 건너뛰지만(M7 · 5판) 꼬리 안의 기다림은 남는다: `McpServerHandle::shutdown` 의 시한 없는 await(axum graceful 이 붙은 연결을 기다린다 — rmcp 는 자식 토큰으로 세션을 닫는다(`mcp_server.rs:1332`)고 보이나 미검) · `tracker.stop()` join(실제로는 짧다) · **자식 락 아래 `child.wait()`**(`pty.rs:414` · `stdio.rs:396` — kill 뒤에도 자식이 안 끝나면 그 kill 스레드가 멎고, `shutdown_all` 의 `thread::scope`(`manager.rs:3122-3128`)를 기다리는 꼬리 ③(`lib.rs:823`)이 함께 멎는다 · 3판). flush 는 5초 띠 둘로 유한하다. (바)=감시면 패닉 경로의 이 멎음은 감시가 끝낸다(M19). StopDaemon 중 깃발이 서고 flush 전 단계가 30초를 넘기면 그 flush 를 잃는다 — 깃발 없는 끄기도 멎으면 같다(5판). 쓰기 도중 끊김은 안전하다(base 원자 쓰기 · 남은 임시 파일은 다음 부팅이 쓴다) · 로그 줄 중간 잘림은 받는다.
- ★**패닉 종료의 멎음**★ — 되감기가 abort 에 없던 멎음을 들인다(셸 훅 로그 자기교착 · 막힌 주 스레드 · 데몬 로그 락 · MCP await · 위 `child.wait()`) — 시한을 둘지 = 사용자 결정 (바)(§3-1). (바)=감시면 4판이 둘을 더 닫았다 — `process::exit` 가 멎은 스레드 · 로더 락에 막히는 경우(감시는 그 길 대신 즉시 끝내기 — §9-12) · 셸 setup **앞**에서 당겨진 채 멎는 경우(셸 감시도 훅 설치 때 선다). ★4판의 「남는 것 = 감시 스레드가 스스로 멎는 경우뿐」은 틀렸다(5판)★ — 4판은 꼬리 · M6 · 셸 `Exit` 갈래를 `process::exit` 로 끝냈는데, 그 밑 `ExitProcess` 는 감시까지 먼저 끝낸 뒤 DLL 분리 알림을 돈다 → 그 분리가 멎으면 덮을 이가 없었다. 5판은 당겨진 끝을 전부 `terminate_self` 로 바꿨다(§4-4). 남는 것 = ① 감시 스레드가 기동에 성공한 뒤 스스로 멎는 경우(로그 · 락 · 할당이 없어 길을 못 찾았다) ② 당기지 않은 정상 끝의 tao `process::exit(0)`(지금과 같다)에 섞여 든 당김 — 셸 `Exit` 콜백의 끝 재확인 **뒤** · tao `process::exit(0)` **앞**의 좁은 틈에 든 당김, 그리고 우리 콜백 뒤 tauri 자신의 `cleanup_before_exit`(`app.rs:1433`)에서 난 새 패닉(훅은 당기지만 tao 가 삼킨다)은 코드 0 · `Final` 쓰인 채 `ExitProcess` 로 끝난다(추론 · 받는다 — 패닉이 종료 직전이고 결말이 정상 끝과 같다). ★5판의 ② 「tauri `AppHandle::exit` 실패 갈래(`app.rs:574-579`)가 깨우기 스레드에서 부르는 `process::exit`」는 닫혔다(6판)★ — 깨우기가 `app.exit` 를 부르지 않는다(주 스레드 게시 · M18). ★tao 가 `Exit` 콜백 패닉을 삼키고 `process::exit(0)` 하는 길(tao `runner.rs:95-108` · `event_loop.rs:225-231`)도 닫혔다(6판)★ — 콜백 몸통의 `catch_unwind` + 끝 재확인(§4-9).
- **팬아웃 seq 구멍** — 출력 코어 `fan_out`(`output_core.rs:351-380`)은 구독자 사본을 돌며 `send` 한다 — 요청 스레드에서 가둔 패닉이 그 고리 가운데서 나면 남은 구독자는 그 seq 를 못 받는다 → 그 뷰는 붙듦 상한까지 멈춘 뒤 전량 replay 로 회복(「replay→live」 불변식의 기존 회복 길).
- **reaper 의 낡은 목록 줄** — (나)=R 이거나 reaper 를 그대로 가두면 `sessions.remove` 뒤 `agent_list_updated` 전 패닉이 낡은 줄을 남긴다. (나)=E 면 정상 종료로 간다.
- **동시 종료 경합** — StopDaemon 과 패닉 꼬리가 겹치면 StopDaemon 답이 사라질 수 있다(M12) · 남은 자식은 Job 이 끈다.
- **net 자기 루프 · 연결 수명 · 써드파티 내부 패닉 = 재시작**(M5) · **스트리밍 · SSE 본문**(hyper 가 `next.run` 뒤에 poll) = 표시 밖 → 재시작.
- **abort 갈래** — 이중 패닉(훅 안 패닉 — std 「panicked while processing panic」 · 되감는 중 소멸자를 **벗어난** 패닉) · FFI 건넘 · WebView2 COM 콜백 패닉(셸 — 미검)은 지금처럼 즉사(Job 이 에이전트 정리). ★되감는 중 `Drop` 안 중첩 패닉 자체는 치명이 아니다 — 소멸자를 벗어날 때만 abort 다(「panic in a destructor during cleanup」 — core 1.95 rlib 문자열로 본 추론)★ → `Contained::drop` 의 `catch_unwind` 는 되감는 중에도 잡는다(6판 정정 — 5판의 「되감는 중엔 무력하다」는 틀렸다 · M16). 되감는 중 도는 `Drop` 의 잡히지 않은 `.expect` 는 이 갈래로 간다 — 찾은 것(`SessionTracker::drop` · `SpawnReservation::drop` `manager.rs:746-753` — (나)=E 활성화 창의 되감기에서 돈다 · 4판)은 M23 이 걷는다. command `route.rs` 주석의 「핸들러 · 링크 future 는 drop 에서 패닉하면 안 된다」가 계속 유효하다.
- **부팅 중 결정적 패닉의 재기동 고리** — 지금(abort)과 같다(이제 `terminate_self(101)` 로 끝날 뿐 — 5판).
- **표시 누락** — 새 spawn 이 표시 없이 들어오면 그 자리 패닉은 재시작이다(안전한 쪽). M9 가 요청 경로 파일에서만 막는다.
- **가둔 패닉의 진단 손실(5판 — 받는다)** — 가둔 비오염 패닉은 훅이 판정 직후 돌아가 로그 · 기본 훅을 부르지 않는다(§4-3) → 백트레이스(`RUST_BACKTRACE`)와 기본 훅의 stderr 줄을 잃는다. 남는 것 = 가둔 경계의 한 줄(잡은 문장 + 자리 — 파일 256바이트까지 · 줄 · 열). 대가로 tracing 락을 쥔 채 난 가둔 패닉의 훅 자기교착이 없어진다(지금 abort 훅에도 있던 기존 결함). ★6판 — 가둔 A 가 되감는 중 `Drop` 에서 B 가 나 소멸자를 벗어나 abort 하는 경우★: B 가 올리는 패닉(오염 등)이면 B 의 사유에 A 의 칸이 덧붙는다(§4-3 (a″)) · ★B 가 가둔 **비오염** 패닉이면 훅이 판정 직후 돌아가 A · B 어느 것도 기록되지 않는다(받는다)★.
- **훅의 할당 · tracing 락** — ★5판부터 올리는 패닉에만 해당한다(가둔 비오염 패닉은 판정 직후 돌아간다)★. 사유 문장은 훅에서 깃발 **뒤에** 할당한다(4판 · std 기본 훅도 할당한다). tracing 내부(reload · `EnvFilter` `RwLock`)를 쥔 채 난 패닉이면 훅의 로그가 교착할 수 있다. 깃발은 이미 섰지만 ★「다른 스레드가 종료를 탄다 · 로그만 잃는다」는 2판의 틀린 말이다(3판)★ — 그 락을 기다리는 **모든** 로그 호출이 막히므로 accept 루프 · 꼬리의 `info!`(예: `lib.rs:832`)도 멎고, 데몬은 단일 인스턴스 잠금 · `daemon.json` 을 쥔 채 남는다((바)의 (3)). (바)=감시면 감시(로그 안 씀)가 시한 뒤 끝낸다 · 미룸 유지면 서명받을 잔여.
- **셸 패닉의 사유 기록(6판 — 주 스레드 패닉만이 아니다)** — 깨우기 · 게시한 닫힘이 로그를 안 쓰고(§4-9) 당겨진 길은 `Exit` 사건에 닿지 않으므로(주 스레드 패닉은 `Exit` 사건이 아예 없다) 훅의 (e) 줄이 유일하다(사용자 끝내기와 겹친 `Exit` 진입 갈래만 다시 찍는다). 그 스레드가 파일 쓰기 도중 패닉했으면 그 줄은 재진입 가드가 버린다(M4 L1) → 사유가 로그에 안 남는다(창은 닫히고 다음 실행이 복원을 묻는다).
- **셸 「배경 먼저 · 주 스레드 나중」 경합 — 6판에서 닫혔다** — 5판까지는 배경 스레드 패닉이 먼저 당기고(표지 없음) 주 스레드가 뒤이어 따로 패닉하면 깨우기가 표지 없이 `app.exit` 를 불러, 이벤트 루프가 없으면 `request_exit` 실패 갈래가 깨우기 스레드에서 창 · 트레이를 만졌다. 6판의 깨우기는 `app.exit` 를 부르지 않는다 — 게시한 닫힘은 주 스레드에서만 돌고, 주 스레드가 패닉했으면 tao 가 그 사건을 처리기 없이 버리며(`runner.rs:153-175` — 추론), 루프가 없으면 게시가 `Err` 로 끝난다(M18). 끝은 그대로 `main.rs` 포착이나 감시(M19)다.
- **유령 대기 입력의 남은 틈** — `send_input` 포착(M22)은 「먼저 emit, 뒤에 일」 꼴의 **못 찾은** 자리를 덮지 않는다 → 그 에이전트로 가는 우편이 ✕ · 재시작까지 멈춘다(§3-1 (가) †). (3판의 「P1 · P1o 면 건너뜀」 틈은 4판에서 없어졌다 — M22.)
- **거짓 `Dropped`(4판 — 받는다)** — 통로가 바이트를 이미 받아 둔 뒤(`pty.rs:360-363`) 같은 `send_input` 안에서 패닉하면 M22 가 `Dropped{Rejected}` 를 내지만 입력은 실제로 나간다 → 벤더 `Delivered` 가 묘비 id 에 오고, 사용자가 다시 보내면 입력이 둘이 된다. 드물다 · 통로 trait 은 바꾸지 않는다.
- **부팅 대기(확신도 낮음 · 리뷰)** — 깃발을 보는 것은 accept 루프뿐이라 그 **앞** 부팅 걸음이 당겨진 채 멎으면 깨울 길이 없다. 그런 대기는 못 찾았다. (바)=감시면 감시가 훅 설치부터 돌아 덮는다 → 잔여 아님 · 미룸 유지면 이 줄이 남는다.
- **셸 주 스레드 패닉 뒤 트레이 아이콘** — `cleanup_before_exit` 가 안 돌아 남을 수 있다(마우스를 올리면 사라지는 Windows 동작 — 추론 · 미검). 셸 `main.rs` 포착 자리엔 `AppHandle` 이 없어 부를 수도 없다 — 받는다(6판). 감시 시한으로 끝난 경우(게시 실패 · 주 스레드 멎음)도 같다.

---

## 5. 사용자가 보는 변화 (권고안 기준 — 괄호는 (가) 다른 안)

| # | 상황 | 지금(배포판) | 바뀐 뒤 |
|---|---|---|---|
| 1 | 버스 명령 · WS 명령 · 제어 HTTP · MCP 도구 처리 중 패닉 | 데몬 즉사 → 에이전트 전부 죽음 | 그 요청만 오류 답(N1a) · 데몬 · 에이전트 계속. **단 공유 락을 쥔 채였고 누가 그 락을 다시 만지면 그때 정상 종료**(P1: 어떤 락이든 · P1o: 순서 락 빼고 어떤 락이든) · 입력 보내기 도중이었으면 그 에이전트로 가는 우편이 ✕ · 재시작까지 멈출 수 있다(유령 대기 입력 — 모든 안 · §3-1 (가) †) |
| 2 | 에이전트 하나의 코드 패닉(펌프 · 감시 · stderr · 라이터 · 첫 턴 기록 · leftover · 메시지 배달) | 데몬 즉사 | 그 에이전트 `Failed` · 라이터면 입력만 거절 · 나머지 계속. **공유 락을 쥔 채였으면 정상 종료**(P1: 그 에이전트 자기 락이어도 종료 · B1: 감시 · stderr 는 종료) |
| 3 | 에이전트 시작 · 수거 · kill · 연결 붙이기 · 떼기 도중 패닉((나)) | 데몬 즉사 | 데몬 정상 종료(그 요청은 오류 답) · 시작 도중 스레드를 못 만든 자원 실패도 같다(지금 debug 는 `SpawnFailed` 답) |
| 4 | 공유 상태 · 표시 밖 · 공유 락 오염 · codex 라이터 · net 루프 패닉 | 데몬 즉사 | 데몬 정상 종료(에이전트 정리 · 화면 「데몬 꺼짐」 · 꼬리 동안 새 명령 · 조회 · 도구 호출은 오류 답(다 — 정리 꼴 명령 빼고) · 종료 코드 101) — 다음 데몬 부팅은 지금처럼 자동 복원 안 함 · 활성화로 이어받기 |
| 5 | 데몬 부팅 중 패닉 | 즉사 | 곧바로 종료(코드 101 — 꼬리 없음) |
| 6 | 셸 패닉(버스 명령 처리 포함) | 셸 즉사 · 데몬 산다 | 셸만 닫힘(버스 명령이면 호출자는 `INTERNAL` 답도 받는다) · 데몬 산다 · 다음 실행 = 비정상 종료 복원 물음 · 종료 코드 101 — 트레이 「완전 종료」 도중 난 패닉도 101(6판 — 종료 직전의 좁은 틈은 0 · §4-11) · 트레이 아이콘은 지운다(배경 스레드 패닉 — 주 스레드 패닉 · 감시 시한으로 끝나면 남을 수 있다 · 6판) |
| 7 | 이중 패닉 · FFI · WebView2 콜백 | 즉사 | 같다 |
| 8 | 개발(debug) 빌드 | 표시 없는 패닉 = 그 스레드만 조용히 죽음 | 데몬 · 셸이 정상 종료한다(개발자가 바로 본다) |
| 9 | 배포 exe 크기 | — | 커진다(미측정 — M11) |
| 10 | CLI 패닉 | fast-fail 코드 | 종료 코드 101 |
| 11 | 패닉 뒤 종료가 멎음((바) — 훅 로그 자기교착 · 막힌 주 스레드 · MCP 종료) | 없음(즉사라 멎을 틈이 없다) | 데몬 30초 · 셸 5초 뒤 강제 종료(코드 101 · 가안) — 깃발 없는 StopDaemon · 트레이 끄기는 그대로지만 그 꼬리 도중 깃발이 서면(앞선 오염의 관찰 · 정리 중 패닉) 그 끄기도 같은 상한 · 코드 101 · StopDaemon 중 깃발이 서고 flush 전 단계가 30초를 넘기면 그 flush 를 잃는다 — 깃발 없는 끄기도 멎으면 같다(5판) · (바)=미룸 유지면 사람이 끌 때까지 멎은 채 남는다 · 셸 시한만이면 데몬 쪽은 남는다 |

---

## 6. 게이트 · 의존

- **새 의존 0** — base `panic` 은 std + base 가 이미 지는 `tracing`(가둔 경계 로그 · 5판) · 데몬은 이미 axum · tokio · futures-util 을 진다.
- **base 게이트:** `use tauri` 0 · 의존 상한(자기 자신 1줄) 그대로 · **입주자 무참조 ③ 의 알파벳에 `panic`**(정본 `base/src/lib.rs:54`) — 사본 여섯(`CLAUDE.md:255` · `.github/workflows/ci.yml:668` · `.claude/skill-bindings/qa.md:161` · `docs/testing-strategy.md:50` · `:179` · `base/src/lib.rs:54`) + 입주자 명단 사본(base `lib.rs` 헤더 「여덟」 `:4` · 선언 `:75-84` · CLAUDE.md base 항목 · 루트 `Cargo.toml` 멤버 주석 · `docs/testing-strategy.md` 의 base 단위 목록 · base 시험 줄). ★주석에 `crate::panic` · `super::panic` 철자를 쓰지 않는다(게이트 ③ 이 잡는다)★.
- **net:** 무변경(M5 N1) — 의존 상한 · 심볼 allowlist · `--all-features` 그대로.
- **platform(4판 — U1):** 즉시 끝내기 `process::terminate_self` 하나(§4-4 — 감시와 당겨진 끝이 부른다 · 5판) + 책임 목록 둘(`lib.rs:11-12` · `process.rs:1-3` · 5판). cfg 는 platform `process.rs` 안이라 OS 게이트 명단(④ cfg · ⑥ `std::os` — 둘 다 platform **밖** 명단)은 무변경 · 짝(platform 안 1파일 이상)은 이미 선다 · ⑤ OS crate 운영 의존 무변경 · `windows` feature 추가 없음(`Win32_System_Threading` 이미 켜짐) · 새 의존 0(non-Windows = `process::exit` 자리채움 — `extern "C"` 선언 없음 · 헤더 규칙 3 · 5판) · `use tauri` 0 · 의존 상한(자기 자신 1줄) 그대로. 부르는 쪽(데몬 · 셸)은 이미 platform 을 운영 의존으로 진다.
- **메시징:** 의존 0 유지 — 관찰자를 못 부르는 것은 U8a 가 문장으로 다룬다(§2-6).
- **새 게이트:** 컴파일 가드(M10 — 코드 안 · 데몬 lib · agent lib) · 요청 경로 맨 spawn 소스 시험(M9 — 허용 목록 = `connection_core.rs:1543` · `command_delivery.rs:537`(4판 정정) · `mcp_server.rs:1411` 셋뿐 · `mcp_server.rs` 의 처리기 blocking 여섯은 도우미로 옮겨 목록에 없다) · U8b 뒤 §2-8 의 넓힌 정규식으로 남은 줄 확인(문서 확인 — CI 아님).
- **CI:** 워크플로 변경 = 게이트 ③ 문자열 하나. 릴리스 잡은 프로필을 그대로 따른다. 새 통합 시험 바이너리 둘은 워크스페이스 회귀가 그대로 줍는다(스텝 추가 없음 — 미검).
- **회귀 결과 줄:** +2(가안) — U1 이 base `tests/poison_observer.rs`(관찰자 등록 상태 — M20)를, U5 가 데몬 `tests/` 에 실프로세스 시험 파일 하나를 더한다. 나머지 단위 0.

---

## 7. 단위

**원칙(TRD A §6 과 같다):** 단위마다 워크스페이스 빌드 · 회귀 초록으로 끊는다 · 단위 안 순서 = 「새 자리 만들기 → 부르는 곳을 파일 단위로 → 옛 자리 지우기」(중간 어디서 멈춰도 빌드가 선다) · 착수 전 로컬 커밋으로 되돌릴 지점 · 주석은 `/code-conventions` 주석 규약 주입 · **ADR-0292 는 U1 바로 앞**(§8-1) · 코드 단위는 자기가 바꾸는 줄의 abort 전제 주석만 고치고 나머지는 U8a(의미) · U8b(문장)가 쓸어 담는다 · 크기가 애매한 단위는 파일 하나로 파일럿을 먼저 돈다.

### 7-1. 단위 목록 (크기는 권고안 기준 · 괄호는 (가) 다른 안)

| id | 범위 | 건드리는 파일 | 선행 | 크기(어림) | QA |
|---|---|---|---|---|---|
| **U1** | base `panic`(§4-1 — `Escalation::new` · `wait_reason` + 진행 셈(5판 · SeqCst 6판) · 가둔 칸과 catch 갈래 로그(5판 — 6판 = 문장 + 자리 · 깊이별 넷 · 읽으며 비움 · `armed_contained`) 포함 · `payload_str` 은 두지 않는다 — std `payload_as_str` · M20) · `sync` 관찰 갈래 · 범위 갈래((가)) · 관찰자 등록 · `logging` 재진입 가드 · 게이트 ③ · 명단 사본 · **platform 즉시 끝내기 `process::terminate_self`(4판 · §4-4 — 소비자 U5 · U6 가 다른 물결이라 둘보다 앞선 이 단위에 둔다 · platform 파일은 다른 단위와 안 겹친다)** | base `src/panic.rs`(새) · `src/sync.rs` · `src/logging/mod.rs` · `src/lib.rs` · `tests/poison_observer.rs`(새 · 가안 — M20) · base `Cargo.toml` description · 게이트 ③ 사본 다섯 · 명단 사본 · platform `src/process.rs`(+ 책임 목록 `:1-3`) · `src/lib.rs`(입주자 설명 `:11-12` — 5판) | ADR-0292 | 중간(+410 — 6판 칸 넷 · 문장 ≈ +25 · platform ≈ +15 포함 · 시험 ≈ 20(`terminate_self` 는 자기 프로세스를 끝내 단위 시험 없음 — U5 ⑥ 이 잰다) — ★걸쇠 시험은 전부 지역 인스턴스(M20)★: 깊이 증감 · 중첩 · 되감기 뒤 복귀 · `Contained` poll 마다(스레드 옮김 흉내) · 패닉 · 취소 drop 이 깊이 안 · `escalating` 0 과 복귀 · `is_poison_panic` 핀(Mutex `.expect` · `.unwrap` · RwLock) · 걸쇠(SeqCst · 폴링 대기 먼저/나중 · 첫 사유 · 첫 자리) · `wait_reason` = 진행 셈 0 이면 곧바로 / 셈 > 0 이면 사유가 적히거나 시한까지 기다림(5판) · 가둔 경계 로그(5판 — 훅 흉내로 자리 칸을 채운 뒤 패닉 → `Caught` 에 문장 · 파일 · 줄 · 열이 실리고 한 줄 로그 · 파일 256바이트 자름이 문자 경계에서) · 가둔 칸(6판): 문장 256바이트 자름이 문자 경계에서 · 경계가 자기 깊이 칸을 읽으며 비움(둘째 읽기 = 빔) · 중첩 경계(깊이 1 · 2)가 서로 덮지 않음 · 같은 스레드의 앞 입구가 남긴 무장 칸을 다음 입구가 물려받지 않음(tokio 일꾼 흉내 — 한 스레드 두 `Contained` · 안쪽 `catch_unwind` 가 삼킨 칸 포함) · `armed_contained` 는 비우지 않음 · lib 단위 = 미등록이면 말없음 · 재진입 가드 / `tests/poison_observer.rs` = 관찰 갈래가 호출 위치를 넘김 · 범위 갈래는 안 부름 · 결과 줄 +1) | standard |
| **U2** | 에이전트 2층(§4-7) · B1′ 감시 가드 + 사유 칸(★사유 먼저 · 자식 끄기 · master 놓기 순 — D2★) · stderr 줄 가둠(★첫 패닉 뒤 읽어 버리기만 — D5★) · 통로 `shutdown` 락 되찾기(M14 — 파일 주인 · U4 의 M15 가드가 이것에 기댄다) · reaper 고리 주석(표시 없음 = 3층 · `reap_one` 은 감싸지 않는다 — §4-2) · 페이로드 사본 여섯 걷기 · (가)=P2 범위 치환(이 파일들) | agent `transport/{pty,stdio,input_queue,spawn}.rs` · `backend/codex/transport.rs` · `backend/claude/leftover.rs` · `session_id_latch.rs` · `reaper.rs` | U1 | 중간(+150/−90 + P2 치환 ≈ 60곳 · 시험 더함: 감시 패닉 → 펌프가 `Error("pty watcher panicked: …")` 로 끝남(`Exited` 로 굳지 않음) · stderr 해석 패닉 뒤 줄은 해석 없이 비워짐 · 패닉 한 번만 로그) · (P1: +130/−90 · B2: U4b 별도). 크면 셋으로 가른다 — `transport/` · codex · claude+래치+reaper(겹침 없음) | standard |
| **U3** | 데몬 1층(§4-6) · (다) 거절(★가장 바깥 입구 셋만★ — 제어 HTTP 는 라우트마다 `.layer` 또는 경로 확인 · 맨 `route_layer` 아님 · 5판) · 깃발 뒤 StopDaemon = `shutdown_all` 없이 `Ok`(M12 · 5판 — (다) 답과 무관 · `ConnectionCore` 걸쇠 칸을 쓴다) · 처리기 blocking 여섯 → `spawn_blocking_contained` · 걸쇠 주입(도우미 인자 · `ConnectionCore` 칸 — M20) · 연결 수명 처리기 분류((나)) · 연결 소유 락 범위 치환(P2) · 맨 spawn 소스 시험(M9) | daemon `agent_conn.rs` · `connection_core.rs` · `command_delivery.rs` · `control/mcp_server.rs` · 새 `containment.rs`(가안) · `lib.rs`(걸쇠를 넘기는 조립 줄 · 시험 서버 조립 — 지역 인스턴스 · M20) | U1 | 중간(+240 · 시험(걸쇠는 지역 인스턴스): 패닉 처리기 → 오류 답 + 연결 유지 · blocking 패닉 → 기존 오류 갈래 · HTTP 500(가장 바깥 층 — `bearer_auth` 안 패닉 포함) · 처리기 blocking 패닉 → 500 · MCP 도구 오류 · 깃발 뒤 거절 = 입구마다 하나(WS 명령 · 제어 HTTP `agent.*` · MCP 도구) · ★깃발 뒤에도 거절 안 됨(4판): WS 정리 꼴 명령(StopDaemon 등) · MCP 경로의 rmcp 세션 요청(DELETE 등 — 제어 라우트 층 밖 · 5판 = 제어 라우트 여섯은 모두 거절되고 MCP 는 아님을 함께 본다)★ · 깃발 뒤 StopDaemon → `Ok` · Close · 에이전트를 끄지 않음(`shutdown_all` 안 부름 — 5판) · 깃발 뒤에도 안쪽 홉(배달 태스크 → 1단계 blocking)과 `on_disconnect` 는 돈다 · 소스 시험(허용 셋 — `mcp_server.rs` 는 `:1411` 하나)) | standard |
| **U4** | 종료 경로 오염 내성(M14 — 통로 몫 빼고) · kill 길 상태 싱크 **가둠 · 버림만**(M21 — 사유 · 당김 · 생성자 걸쇠 인자는 U5 · 4판) · 활성화 창 `escalating` + 스코프 가드(M15 — 통로 → 세션 핸들) · 요청 kill 앞 줄 `escalating` · `send_input` 패닉 포착(M22 — 맨 `catch_unwind` · 4판) · 관리자 · 추적기 `Drop`(M23 ①) · `SpawnReservation::drop` → `sync::lock`(M23 ① · 4판) · `name_allocation` 범위로 명시(§4-5 메인 규칙) · 코어 · 세션 범위 치환(P2) | agent `manager.rs` · `session.rs` · `output_core.rs` · `session_tracker.rs` · daemon `control/registry.rs`(`revoke` 등) · `status_fanout.rs`(생성자 서명은 그대로) | U1 · **U2**(★M15 가드의 되감는 중 `shutdown` 이 U2 의 `pty.rs` 되찾기에 기댄다 — 그 전엔 `.expect` 둘째 패닉 = abort★ · 3판의 「U3(생성자 자리)」 의존은 생성자 인자가 U5 로 가며 없어졌다 · 물결 3 그대로) | 중간(+185 + P2 치환 ≈ 36곳 · 시험: 명부 락 오염 뒤 `shutdown_all` 이 전부 끄고 돌아온다 · 한 에이전트의 코어 락 오염이 남의 kill 을 안 막는다 · 레지스트리 오염 뒤 `kill_agent` 완주 · 상태 싱크 패닉 뒤에도 `kill_agent` 가 2동사를 끝까지(걸쇠는 U5) · 활성화 창 패닉 → 통로 `shutdown` 불림 · 명부에 좀비 없음 · `spawning` 집합 오염 뒤 `SpawnReservation` drop 이 abort 없이 · ★겹친 종료의 **연기 시험**(4판 — 패닉 · abort · 멎음 없음만 본다): `shutdown_all` 둘이 동시에(StopDaemon · 패닉 꼬리 흉내) 실 PTY 에이전트의 `transport.shutdown` 을 부른다 — 자식 락이 줄 세워 seam 없이는 실패할 수 없다(§9-8)★ · `send_input` 이 emit 뒤 패닉 → `Dropped{Rejected}` 하나 + 패닉은 다시 풀림(`resume_unwind`) · 되감는 중 `Drop` 은 join 안 함 · `name_allocation` 오염 뒤 활성화 계속) | standard |
| (U4b) | (가)=넓게일 때만 — codex 라이터 · thread_lock 의 「펌프로 가는 길」 | codex `transport.rs` · `thread_lock.rs` | U2 · U4 | +120~250 | standard + 실프로세스 |
| **U8a** | 의미 재감사 + 필요한 코드(M13) — K7 · `in_flight_targets`(`FlightSettle` · `Reservation`) · `DeliveryDone` 2층 표시 · 래치(★ADR-0226 0턴 id 불변식 — 래치 commit 뒤 send 전의 가둔 패닉은 0턴 id 를 영속한다 · 리뷰 판단 = commit-먼저 설계가 이미 받는 창과 같은 모양이라 abort 보다 나쁘지 않다 · U8a 가 확인해 적는다★) · codex 라이터 · `lock_for_cleanup` 짝 · 런타임 로그 문장 `:769`. 결과 표 = ADR-0292 「영향」 끝의 날짜 박은 추기 · 이 TRD 부록 — 결정 ③ 의 `DeliveryDone` 줄 판정도 그 추기로 싣는다(말없는 본문 수정 아님 · §8-1) | `leftover.rs` · `session_id_latch.rs` · codex `transport.rs` · daemon `messaging_host.rs` · `command_roster.rs` · `command_delivery.rs` · messaging `service.rs` | U2 · U3 (★파일이 겹친다★) | 중간(감사 + 코드 ≤ 100줄 · 결과에 따라 +α) | standard + `/review code light` |
| **U5** | 데몬 훅 조립(4판 순서 — 할당 없는 판정 → 깃발 → 사유 · §4-3) · 관찰자 등록 · **감시 스레드(M19 — 훅 설치 직후 · `cfg(panic = "unwind")` 만 · 끝내기 = `terminate_self` · 기동 실패 = `Err(1)`)** · 걸쇠 전역은 여기서만 꺼내 넘김(M20) · accept 루프 interval 팔(`shutdown_tx` 사본 — `lib.rs:383` 앞) + 포착(`run_accept_loop` 안 `select!` 고리) · 꼬리 사유 로그(끝의 `wait_reason` 1초 · 4판) · `Err(101)` 앞 관리자 `Arc` `mem::forget`(M23 ②) · **M21 의 사유 기록 · 당김 + `DaemonStatusSink::new` 걸쇠 인자(부르는 곳 넷 — `lib.rs:334` · `connection_core.rs:3021` · `agent_conn.rs:922` · `status_fanout.rs:90` · 4판)** · `main.rs` 최상위 포착 · ★당겨진 끝 = `main.rs` 의 101 갈래 둘(꼬리 `Err(101)` · M6 포착)이 마지막 로그 뒤 `terminate_self(101)`(5판 — M7)★ · `main.rs` 의 101 아닌 갈래(`Ok` · `Err(1)`)도 끝내기 직전 `is_pulled()` 면 `terminate_self(101)`(6판 — M6) · 훅의 무장 칸 덧붙임((a″) · 6판) · 방아쇠(M8) · 실프로세스 시험 | daemon `lib.rs`(훅 · 감시 · 조립 · 루프 · 꼬리 · 주석 `:455-457`) · `main.rs`(101 갈래 → `terminate_self` — 5판) · `status_fanout.rs` · `connection_core.rs` · `agent_conn.rs`(시험 모듈 생성자 자리만) · 새 `tests/panic_policy.rs`(가안) | U1(platform 도우미) · U2 · U3 · U4 (★표시 · 종료 경로가 먼저★) | 중간(+235 — 감시 ≈ +30 · M21 당김 ≈ +10 · 6판 덧붙임 · 재확인 ≈ +10 포함 · 단위 시험(지역 걸쇠 · 시한 수십 ms · 기록하는 끝내기): 당김 → 시한 뒤 끝내기 한 번 · 안 당김 → 안 부름 · 상태 싱크 패닉 → 잡은 문장이 첫 사유 · 걸쇠가 섬 · 훅 순수 함수 = 무장 칸이 있으면 사유에 「unwinding contained panic」이 덧붙음 / 없으면 안 붙음(6판) · `main.rs` 재확인 두 줄은 시험 seam 을 두지 않는다(가안 — 리뷰로 본다) · 시험 파일 +1 결과 줄) | **full** — 실프로세스(debug 데몬 exe · 임시 `ENGRAM_DATA_DIR`): ① 표시 밖 패닉 → N초 안에 끝남 · 코드 101 · 로그에 사유 + 「데몬 종료 완료」 ② 가둔 패닉 → N초 뒤에도 살아 `ListAgents` 에 답함 ③ 공유 락 오염(가둔 패닉이 오염 → 다음 접근) → 정상 종료 + 오염 자리 로그 ④ 셸 에이전트 하나를 띄운 뒤 ① → 자식 사라짐 ⑤ 부팅 패닉 → 곧바로 101(매달림 없음) ⑥ 꼬리 멎음 흉내(방아쇠 `hang` 가안 — 꼬리 머리에서 잠듦 · debug 전용 짧은 시한) → 감시가 시한 뒤 `terminate_self` 로 101(4판 · 5판 — ① · ⑤ 도 당겨진 끝이라 같은 도우미로 끝난다). `send_stop` 결과는 판정에 안 쓴다(M12) |
| **U6** | 셸 훅((라) — 4판 순서 · `MAIN_ESCALATED` 를 당기기 앞에 · D1) · **감시 스레드(M19 — 훅 설치 직후 · 깨우기와 다른 스레드 · `cfg(panic = "unwind")` 만 · 끝내기 = `terminate_self` · 4판)** · 깨우기 스레드(주 스레드 게시 `run_on_main_thread` 하나만 — `app.exit` 없음 · 시한 · 로그 없음 · 6판) · 게시 닫힘(`catch_unwind` 안 `cleanup_before_exit` → `terminate_self(101)` · 6판) · `Exit` 콜백(몸통 `catch_unwind` · 진입 갈래 = `wait_reason` 1초 뒤 사유 로그 · `Final` 없음 · 끝 재확인 `caught \|\| is_pulled()` → `terminate_self(101)` 이 시작 실패 코드보다 앞 · 6판) · ★설치 = `run()` 머리 `Builder` 앞 · 주 스레드 표지 `IS_MAIN`(const TLS) — 5판★ · ★셸 `main.rs` 가 `run()` 을 `catch_unwind` → `terminate_self(101)`(5판 — M7)★ · `Final` 없는 종료 세션 · 걸쇠 주입(M20) | `src-tauri/src/lib.rs` · `src-tauri/src/main.rs`(5판) · `state/` 의 종료 세션 · 새 작은 모듈(가안) · 방아쇠 | U1(platform 도우미 포함) | 작음~중간(+195 — 감시 ≈ +25 · 6판 게시 닫힘 · `Exit` 콜백 포착 · 재확인 ≈ +20 · `lib_unit` 시험(지역 걸쇠 · 짧은 시한 · 게시 · 정리 · 종료 세션 · 끝내기를 주입해 기록): 깨우기 = 표지 없음 → 게시 한 번 / 표지 있음 → 게시 없음 / 안 당김 → 없음 / 게시 `Err` → 더 하는 일 없음(끝내기 안 부름 · 6판) · 닫힘 = 정리 seam 이 패닉해도 끝내기(101) 한 번(6판) · `Exit` 콜백 = 종료 세션 seam 이 패닉 → 기록된 끝내기(101) 한 번 / 진입 뒤 당김(종료 세션 seam 안에서 걸쇠를 당김) → 101 / 시작 실패 + 당김 → 101(시작 실패 코드 아님) / 안 당김 · 안 잡힘 → 끝내기 없음(6판) · 감시 = 당김 → 시한 뒤 끝내기 한 번(표지와 무관) / 안 당김 → 없음) | **full** — GUI: 배경 스레드 패닉 → 창이 닫힘 · 코드 101 · 데몬 PID 생존 · `engram agent list` 그대로 · 트레이 아이콘 사라짐 · 다음 실행에 복원 물음 / 주 스레드 패닉 → tao 되던지기 → 셸 `main.rs` 포착의 `terminate_self` 로 코드 101(abort 아님 · 5판) · 트레이 아이콘 상태 기록 / 버스 명령 패닉 → 창이 닫힘(S1a) |
| **U7** | 프로필 unwind · 컴파일 가드(데몬 lib · agent lib) · 크기 | 루트 `Cargo.toml:41` · 데몬 · agent `lib.rs` | U4 · U5 · U6 · **U8a** | 작음(+15) | **full** — 릴리스 세 exe 빌드 · 크기 앞뒤 기록(M11) · 가드 증명(스크래치 워크트리에서 abort 로 되돌려 빌드 실패 1회) · `scripts/` 런처로 릴리스 기동 · 에이전트 하나 띄우고 끄기 |
| **U8b** | abort 전제 **문장** 손질(사실 주장 · 사유) · base `sync` 계약 3 문장 | §2-8 넓힌 정규식의 남은 줄(≈ 57 중 앞 단위가 안 고친 것) | U7 | 중간(≈ 50~60줄 · ≈ 30파일 · 의미 변경 0) | standard |
| **U9** | ADR 도장(§8-2 의 0288 · 0290 밖 일곱 — `/adr link` · 0288 · 0290 은 ADR-0292 를 만든 supersede 가 이미 박았다 · 5판) · 문서(§8) | §8-2 · §8-3 목록 | U7 | 문서 | `/qa` 문서 범위 + `/review doc`(load-bearing) |

### 7-2. 순서 · 물결 · 어디서 멈춰도 서는 까닭

```
물결 0:  ADR-0292(사용자 결정 (가)~(바) 뒤)
물결 1:  U1
물결 2:  U2  ∥  U3                 (agent 통로 · backend · reaper  /  daemon 요청 경로)
물결 3:  U4  ∥  U8a  ∥  U6         (U4 = U2 뒤 — agent manager · session · core · tracker + daemon registry · status_fanout
                                     /  U8a = U2 · U3 파일 위  /  U6 = 셸 — 셋 안 겹침)
         (U4b — (가)=넓게일 때 U2 · U4 뒤)
물결 4:  U5                          (U2 · U3 · U4 뒤 — daemon lib · main · tests)
물결 5:  U7                          (U4 · U5 · U6 · U8a 뒤)
물결 6:  U8b  ∥  U9
```

다시 잰 근거(2판 변경 반영 · 3판 = U4 를 U2 뒤로 · 감시는 U5 · U6 · 4판 = M21 당김을 U5 로 · 도우미는 U1):

- **U4 는 U2 뒤(3판)** — 파일 겹침이 아니라 의존이다: U4 의 스코프 가드(M15)는 되감는 중에 `transport.shutdown()` 을 부르는데, U2 가 `pty.rs` 의 자식 · master `.expect`(`:412` · `:421`)를 되찾기로 바꾸기 전에는 그 락이 오염돼 있으면 `Drop` 안 둘째 패닉 = abort 다. 3판은 「U4 의 상태 싱크 생성자(M21)가 U3 파일의 시험 모듈을 건드리므로 U3 뒤이기도 하다」고 적었으나 4판에서 그 생성자 인자는 U5 로 갔다(M21) — U4 의 U3 의존은 없어졌고 물결 3 은 그대로다.
- **감시(M19)는 U5 · U6 에 · 즉시 끝내기 도우미는 U1 에 든다(4판)** — U1 뒤 도우미는 아무도 안 부른다. U1~U4 뒤에 멈추면 감시가 없지만 훅도 없으니 당길 깃발이 안 선다(무동작). U5 · U6 뒤엔 debug 에서 감시가 돈다 · 배포판(abort)에선 감시를 띄우지도 않는다(`cfg(panic = "unwind")` — U7 이 프로필을 바꾸면 함께 선다).

- **U1~U4 뒤에 멈추면** — 표시 · 되찾기 · `escalating` 구간 · 스코프 가드 · 거절 확인만 있고 훅도 관찰자 등록도 없다. ★걸쇠를 당기는 코드도 없다(4판 정정 — S1)★: 3판은 U4 에 M21 의 명시 당김을 두었는데, 그러면 U4 뒤 U5 전에 상태 싱크 패닉 하나가 깃발을 세워 U3 의 거절만 깨어난다 — 깃발을 보고 끝낼 interval 팔 · 감시가 아직 없어 데몬이 끝나지 않은 채 새 일을 막는다. 4판은 U4 = 가둠 · 버림만 · 당김은 U5(그 소비자와 함께). 관찰 갈래는 관찰자가 없으면 지금처럼 말없이 되찾고 · `escalating` 은 훅이 없으면 효과가 없고 · 거절 확인은 깃발이 안 서니 무동작이다. debug = 잡던 자리는 같은 결말 · 새로 잡는 자리(WS 처리기 · HTTP · MCP · 감시 · stderr)는 더 나은 결말 · 스코프 가드는 실패 갈래를 덜 나쁘게. 배포판 = abort 그대로.
- **U8a 뒤** — 감사가 고른 코드는 abort 아래 도달 불가 갈래이거나(배포판 무변화) debug 결말을 바로잡는 것이다. 배포판 = abort 그대로.
- **U5 · U6 뒤 U7 전에 멈추면** — debug 는 새 정책대로 돈다. 배포판은 여전히 abort — 훅은 abort 직전에 당긴 뒤 같은 즉사를 맞는다 · `main.rs` 의 `catch_unwind` 는 abort 에서 아무것도 안 잡는다. 지금보다 나빠지는 자리가 없다.
- **U7 을 U4 · U5 · U6 · U8a 보다 먼저 하면 안 된다** — 훅 없는 unwind 배포판은 표시 밖 패닉이 그 스레드만 조용히 죽이는 모양(ADR-0290 이 거부한 것) · 종료 경로 오염 내성 없는 unwind 는 「돌지도 죽지도 않는」 데몬 · 감사 안 한 abort 전제 코드(K7 등)가 배포된다. 컴파일 가드는 U7 에서 함께 든다(먼저 넣으면 abort 프로필에서 빌드가 깨진다).
- **표시(U2 · U3)를 훅(U5)보다 먼저** — ADR-0290 의 순서 제약 그대로다.
- **머지** — 어느 접두든 머지해도 된다(위가 근거). 병렬 물결은 워크트리를 갈라 돌고 접점(§4-1 API · 관찰 · 범위 함수 이름 · `containment.rs` 도우미 서명)을 U1 착지 모양으로 못 박은 뒤 띄운다.
- 한 사람이 직렬로 돌면: U1 → U2 → U3 → U4 → U8a → U5 → U6 → U7 → U8b → U9(2판의 「U1 → U4 → U2 …」는 U4 의 U2 의존을 어겨 3판에서 바꿨다).

### 7-3. 파일 겹침

| | U2 | U3 | U4 | U8a | U5 | U6 | U8b |
|---|---|---|---|---|---|---|---|
| U1 | — | — | — | — | — | — | base `sync.rs`(U1 = 계약 2 · 갈래 · U8b = 계약 3 문장) |
| U2 | | — | 파일 겹침 없음 · **의존**(M15 가드가 `pty.rs` 되찾기에 기댄다) → **U4 는 U2 뒤**(3판) | `leftover.rs` · `session_id_latch.rs` · codex `transport.rs` → **U8a 는 U2 뒤** | — | — | 통로 파일의 남은 문장 |
| U3 | — | | —(4판 — 상태 싱크 생성자 인자가 U5 로 가 3판의 겹침이 없어졌다) | `command_delivery.rs` → **U8a 는 U3 뒤** | daemon `lib.rs` 조립 줄(걸쇠 주입 — M20) · `DaemonStatusSink` 시험 생성자(`connection_core.rs:3021` · `agent_conn.rs:922` — 4판) → U5 는 U3 뒤(이미) | — | 요청 경로 파일의 남은 문장 |
| U4 | — | — | | — | `status_fanout.rs`(U4 = 가둠 · U5 = 사유 · 당김 · 생성자 인자 — 4판) → U5 는 U4 뒤(이미) | — | `manager.rs` 문장 |
| U5 | — | — | — | — | | — | daemon `lib.rs` 의 남은 문장(sweep `:729` 경고 등) |

### 7-4. 단위 안 순서 (빌드가 서게)

- **U1** ① `panic.rs` 와 시험(아무도 안 부름) ② `sync` 관찰 갈래(등록 0 이면 지금과 같다) + 범위 갈래(새 이름 · 아무도 안 부름) ③ `logging` 재진입 가드 ④ 헤더 · 게이트 ③ · 명단 사본 ⑤ platform `process::terminate_self`(아무도 안 부름 · 4판) + 책임 목록 `lib.rs:11-12` · `process.rs:1-3`(5판).
- **U2** 파일마다 한 걸음(펌프 셋 → 라이터 → 감시 가드 · 사유 칸(사유 먼저) → stderr(첫 패닉 뒤 버리기) → 통로 `shutdown` 락 → codex 기록 → leftover → reaper 고리 주석) · 사본 걷기 · 범위 치환은 그 파일 걸음에서.
- **U3** ① `containment.rs`(걸쇠 인자 · 아무도 안 부름) ② `ConnectionCore` 걸쇠 칸 + 조립 줄 ③ 처리기 · spawn 자리를 파일 단위로(`mcp_server.rs` blocking 여섯 포함) ④ HTTP 층 · MCP 도구 ⑤ 거절 확인(바깥 입구 셋 — 제어 HTTP 는 라우트마다 · 5판) + 깃발 뒤 StopDaemon 갈래(5판 — 깃발이 안 서니 무동작) ⑥ 소스 시험.
- **U4** `get_session` → `shutdown_all` → `kill_agent` 앞 줄 → `enter_exiting` · `finish` → 상태 싱크 가둠 · 버림(M21 — 생성자 서명 그대로 · 당김 없음 · 4판) → 추적기 → 레지스트리 → 관리자 · 추적기 `Drop` · `SpawnReservation::drop`(M23 ①) → `send_input` 포착(M22) → 활성화 창(가드 · `escalating`) → `name_allocation` → 범위 치환 — 걸음마다 선다.
- **U8a** ① 자리마다 「unwind 아래 무엇이 닿나」 표 ② 표가 요구하는 코드(작은 걸음) ③ ADR-0292 「영향」 끝 추기 · 부록에 결과(`DeliveryDone` 줄 판정 포함 — §8-1).
- **U5** ① 훅 판정을 조립부 순수 함수로(당기기 없음 · 걸쇠 인자) ② 감시 스레드(M19 — 시한 · 끝내기 주입 · 단위 시험) — 훅이 아직 안 당기므로 무동작 ③ 관찰자 등록 + accept 루프 interval 팔 + 꼬리 사유 로그(`wait_reason`) · 코드 · `mem::forget`(M23 ②) ③′ `DaemonStatusSink::new` 걸쇠 인자(부르는 곳 넷 한 걸음 — `lib.rs:334` · `connection_core.rs:3021` · `agent_conn.rs:922` · `status_fanout.rs:90`) + M21 사유 기록 · 당김(4판) ④ 당기기 켬 ⑤ accept 루프(`run_accept_loop` 안) · `main.rs` 포착 + 101 갈래 둘 → `terminate_self(101)`(5판) + 101 아닌 갈래의 깃발 재확인(6판) ⑥ 방아쇠 · 실프로세스 시험. (훅의 무장 칸 덧붙임 (a″) 은 ① 의 순수 함수에 든다 — 6판.)
- **U6** ① 종료 세션의 「`Final` 없이」 갈래(걸쇠 인자) ② 감시 스레드(시한 · 끝내기 주입 · 아직 안 당기므로 무동작 — 4판) ③ 훅(판정 → 표지 → 깃발 → 사유 순 · `run()` 머리 `Builder` 앞 · `IS_MAIN` — 5판) · 깨우기 스레드(주 스레드 게시 하나 — 닫힘 = `catch_unwind` 정리 → `terminate_self(101)` · `app.exit` 없음 · 시한 · 로그 없음 — 6판) · `Exit` 콜백(몸통 `catch_unwind` · 진입 갈래(`wait_reason` 뒤 사유 로그 · `Final` 없음) · 끝 재확인 → `terminate_self(101)` 이 `startup_failed` 보다 앞 — 6판) ③′ 셸 `main.rs` 의 `catch_unwind` → `terminate_self(101)`(5판) ④ 방아쇠 · 시험.
- **U7** ① 크기 기준 재기 ② 프로필 + 가드 한 커밋 ③ 크기 · 릴리스 QA.

### 7-5. 단위마다 검증

1. **회귀 수 대조** — `cargo test --workspace -- --test-threads=4` 를 단위 앞뒤로 · `test result:` 줄 수와 통과 총계 둘 다. 기준선은 U1 착수 직전에 다시 잰다(이 TRD 는 돌려 보지 않았다). 기대 = 결과 줄은 U1 +1(base `tests/poison_observer.rs` — M20) · U5 +1(가안) · 통과 수는 단위가 선언한다.
2. **게이트** — 건드린 crate 의 게이트 전부 + `cargo fmt --check` + base 게이트 ③(U1 뒤 알파벳에 `panic`) · `cargo test -p engram-dashboard --test lib_unit`(U6) · platform 게이트 넷(`use tauri` · 의존 상한 · ④ · ⑥ — U1 · 4판) · 데몬 · agent · platform 은 `--test-threads=4`(실 프로세스 — U4 의 동시 `shutdown_all` 시험이 실 PTY 를 띄운다).
3. **시험이 지킬 모양(3판)** — ① 걸쇠를 쓰는 시험은 전부 지역 인스턴스(M20) — 프로세스 전역 `escalation()` · 관찰자 칸을 lib 시험이 건드리면 리뷰에서 막는다(한 방향 전역이라 같은 바이너리의 다른 시험을 뒤집는다) ② 관찰자 등록 상태는 base `tests/poison_observer.rs` 하나에서만 ③ 시한 시험(감시 둘 — M19)은 주입한 시한 수십 ms · 기록하는 끝내기로 — 실 `terminate_self` 를 부르는 것은 U5 실프로세스 ① · ⑤ · ⑥ 과 U6 GUI(당겨진 끝 — 5판)뿐이고 단위 시험은 부르지 않는다 · `wait_reason` 의 진행 셈 시험(U1 — 5판)도 지역 인스턴스로 · U6 의 깨우기 게시 · 닫힘 · `Exit` 콜백 시험(6판)은 게시 · 정리 · 끝내기를 주입해 기록한다(실 `AppHandle` 없음) ④ 동시 종료(U4 · §9-8)는 두 스레드가 `shutdown_all` 을 겹쳐 부르는 **연기 시험**이다 — 패닉 · abort · 멎음 없음만 본다. 자식 락(`pty.rs:412-414` · `stdio.rs:394-396`)이 두 부름을 줄 세워 seam 없이는 실패할 수 없으므로 §9-8 을 닫는 증명으로 치지 않는다(4판 — 3판의 「차례로는 통과로 치지 않는다」는 걷었다).
4. **QA 등급** — 표의 등급(`/qa` 바인딩). full 은 앱을 `scripts/` 런처로 띄운다(셸에서 직접 띄우지 않는다).

---

## 8. ADR · 문서 후속

### 8-1. ADR-0292 (U1 바로 앞 · `/adr supersede --mode partial` — 번호는 실행 때 스크립트가 매긴다 · 전제 = 아래 링크 · 6판)

- **제목(가안):** 패닉 정책 구현 — 가두는 입구는 스레드 표시를 세우고 · 표시 밖이나 공유 락 오염 패닉은 훅이 정상 종료를 당기며 · 배포판은 unwind 다
- **결정(채택된 것만 적는다):** ① 배포판 unwind(워크스페이스 전체 — 사용자 결정 2026-10-10) + 데몬 lib · agent lib 컴파일 가드 ② 표시 = base `panic`(스레드 지역 깊이 · 동기 · poll 단위 비동기 · `escalating`) — 원시만, 정책은 조립부 ③ 층 경계 표((가) 결과 + net 자기 루프 · 연결 수명 = 3층) ④ 락 오염 정책((가) 결과 · 관찰자 계약) ⑤ 여러 걸음 공유 변경 구간((나) 결과) + 스폰 경로 스코프 가드 ⑥ 훅 — 할당 없는 판정 → 가둔 패닉은 문장 · 자리를 스레드 지역 깊이별 칸에 베낌(경계가 읽으며 비움 — 6판) → 가둔 비오염 패닉은 돌아감(로그는 가둔 경계 — 5판) → (셸 주 스레드 표지) → 깃발(SeqCst · unpark 없음) → 첫 사유 기록(무장된 칸 덧붙임 — 6판) → 로그(재진입 가드) · 꼬리는 올리는 훅이 도는 중일 때만 사유를 유계로 기다린다 ⑦ 깨우기 = 폴링(데몬 accept 루프 팔 · 셸 깨우기 스레드 — 셸은 `app.exit` 대신 주 스레드에 닫힘 게시 · 6판) ⑧ 데몬 끝 = 기존 꼬리 + 당겨진 끝은 마지막 로그 뒤 platform `terminate_self(101)`(load-bearing — 데몬 꼬리 · `main.rs` 최상위 포착과 101 아닌 갈래의 깃발 재확인 · 셸 게시 닫힘 · 셸 `Exit` 콜백 끝 재확인 · 셸 `main.rs` 포착 · 5판 · 6판) · 깃발 뒤 StopDaemon 은 `shutdown_all` 없이 `Ok`(5판) ⑨ 당긴 뒤 새 일((다) 결과) ⑩ 셸 동작((라) 결과) ⑪ 고지 모양((마) 결과) ⑫ 패닉 종료의 시한((바) 결과 — 감시면 데몬 · 셸 각 훅 설치 직후 전용 스레드(unwind 빌드만) · 끝내기 = platform 즉시 끝내기 · 감시는 로그 · 락을 안 쓴다). ★0288 · 0290 개정의 상세(아래 ①~⑤ · ①~⑤)는 이 「결정」 · 「영향」 본문에 쓴다 — supersede 조항은 짧은 명사구뿐이다(5판)★.
- **U8a 이전에 못 정하는 줄 — 적는 법을 미리 박는다(3판):** ADR-0292 는 U1 앞에 쓰이므로 결정 ③ 의 `DeliveryDone`(메시징 수신자 배달 — §2-2) 줄은 「U8a 판정 대기」 표시로 둔다. U8a 의 판정(2층 유지 · 3층으로 · 코드 손질)은 ADR-0292 「영향」 끝에 **날짜 박은 추기**로 싣고 그 줄 표시를 추기 링크로 바꾼다 — 말없는 본문 수정이 아니다. 판정이 결정 ③ 의 배치를 뒤집으면(2층 → 3층) 추기가 아니라 새 ADR(번복 = 새 번호 · `docs/decisions/README.md`). U8a 의 다른 결과(K7 · `in_flight_targets` · 래치 0턴 id 등)도 같은 추기에 든다.
- **거부한 대안:** 사용자 · 메인이 이 TRD 의 선택지를 고를 때 실제로 버린 것만, 그때 준 까닭으로 적는다 — ★이 TRD 의 「권고하지 않음」을 그대로 옮겨 적지 않는다(결정 날조 금지)★. 메인 결정에서 실제로 버린 것(근거가 이 TRD 에 있다): 깨우기 unpark(store-buffering · 잎 락 불변식 — M3) · watch 직접 송신(M3) · net 하위 태스크 포장(N3 — 반쪽 연결 · M5) · base 정책 함수(ADR-0269 ② — M1) · kill 길 상태 싱크를 net 쪽에서 되찾기(M5 · M21) · agent 가 전역 걸쇠를 읽어 `Drop` join 을 건너뛰기(M20 · M23) · 셸 깨우기 스레드에 시한 얹기(tauri `AppHandle::exit` 실패 갈래가 그 스레드에서 막힌다 — M18 · M19 · 4판) · 셸 깨우기의 `app.exit`(실패 갈래의 `process::exit` 가 감시를 먼저 끝냄 · tao 가 `Exit` 콜백 패닉을 삼키고 코드 0 · 진입 확인 뒤 든 당김을 아무도 다시 안 봄 — M18 · 6판) · 감시의 끝내기를 `process::exit` 로(§9-12 · 4판) · `send_input` 의 `Drop` 가드(되감는 중 둘째 패닉 = abort · 오염 확인 뒤 행동 경합 — M22 · 4판).
- **링크:**
  - Implements ADR-0288 · ADR-0290.
  - ★**ADR-0292 는 `/adr supersede --mode partial` 한 번으로 만들고 0288 · 0290 을 그 한 실행에 함께 넘긴다(5판 — 4판의 「`/adr new` 때 Amends 를 적는다」는 틀렸다)**★ — `/adr new` 는 채번 · 스캐폴드 · 인덱스뿐이고 링크를 쓰지 않는다. 양방향 `Amends` / `Amended by` 와 옛 ADR 상태줄 도장(` · 부분 폐기 by ADR-NNNN (조항)`)은 `supersede`(부분) · `link` 만 박는다(`~/.claude/skills/adr/references/flow.md:53-59` · `:70-80`). 두 대상은 한 실행에 — `supersede --old 0288 --clause "<조항>" --old 0290 --clause "<조항>" --mode partial --title "<제목>"`(바인딩 `.claude/skill-bindings/adr.md:27` · 나눠 돌리면 새 ADR 이 둘 생긴다 · `flow.md:29` · `:58`).
  - ★**실행 전제(6판 — 「예약된 0292」를 걷었다)**★ — ① 0288 · 0290 · 0291 이 **모두** 든 트리에서 돌린다 = 패닉 정책 브랜치(0288 · 0290)를 작업 브랜치(0291 이 있다)에 들인 뒤(그 머지는 메인의 남은 걸음이고 master 가 아니다). 스크립트는 대상이 하나라도 없으면 아무것도 쓰지 않고 멈추고(`~/.claude/skills/adr/scripts/adr.mjs:732-733` 「옛 ADR 없음」) · 번호는 로컬 최대 +1 로 매긴다(`:403-406` `nextNum` · 호출 `:755`) — 0291 이 없는 트리면 0291 을 다시 쓴다. ② 돌리기 전에 모든 원격 브랜치의 최대 번호를 본다 — `for r in $(git for-each-ref --format='%(refname)' refs/remotes); do git -c core.quotepath=false ls-tree --name-only "$r" -- docs/decisions/; done | grep -o '/[0-9]\{4\}-' | sort -u | tail -1`(`core.quotepath=false` = 한글 파일명이 8진 탈출로 나오지 않게). 로컬 최대 +1 이 그 이하면 멈추고 메인이 고른다. ③ **대안**(메인이 ADR 때 고른다) — 지금 `/adr new` 로 번호를 잡고 스캐폴드를 push 한 뒤, 쌍마다 `/adr link` 한 번씩(0292→0288 · 0292→0290) — link 산출은 supersede(부분)와 모양이 같고(리뷰 대조 · `flow.md:53-59` · `:70-80`) 쌍마다 한 번이라 같은 쌍 둘째 조항 한계(아래)에 걸리지 않는다.
  - **조항 = 대상마다 짧은 명사구 하나** — 가운뎃점 · em-dash · 링크 키워드 · 불균형 괄호를 넣을 수 없다(`flow.md:33` — 스크립트가 거부한다). 가안: 0288 = 「결정 1과 2의 범위와 영향 서술」((다)=거절 · (바)=감시면 「결정 3의 전용 장치」 · (바)=감시면 「미룬 시간 제한」을 더한다) · 0290 = 「영향 서술과 결정 끝 유지 문장」(늘 — 6판 · 0290 ⑤ · (다)=거절이면 「결정 1」 을 더한다). **아래 ①~⑤ 의 상세는 ADR-0292 의 「결정」 · 「영향」 본문에 쓴다**(사용자 답에 맞는 것만 남긴다).
  - ★**그 두 쌍에 뒤에 `/adr link` 를 다시 돌리지 않는다**★ — 같은 쌍의 둘째 조항은 「already-linked」 멱등 no-op 이고(`~/.claude/skills/adr/feedback.md:151`) 박힌 조항을 넓힐 스크립트 길도 없다(같은 파일 2026-10-06 항목) → 조항은 사용자 결정 (가)~(바) 뒤 supersede 때 한 번에 확정한다. 채번도 스크립트가 한다(로컬 최대 +1 — 원격 최대 번호를 먼저 보는 전제는 위 · 같은 파일 2026-10-06 · 2026-10-08 항목 — 채번 충돌 실발생).
  - **ADR-0288 몫(조항은 위 명사구 · 상세 = ADR-0292 본문 — 결정 1 · 2 · 3 · 「근거」 셋째 · 「영향」 넷째 · 다섯째 줄)** —
    ① **결정 1 범위** — 「연결 하나」 = 데몬 처리기 몸통 · net 자기 루프 · 연결 수명 코드 = 3층(M5). (나)=E 면 「명령 하나」 안의 여러 걸음 공유 변경 구간도 정상 종료로 간다.
    ② **결정 2 범위** — (가)가 B2 가 아니면 에이전트당 스레드 일부(codex 라이터 · thread_lock · B1 이면 감시 · stderr 도)가 정상 종료로 간다 · (가)=P1 이면 락을 쥔 2층 패닉도 · (가)=P2 면 「그 에이전트만 **정리**」 대신 요청이 반쯤 바꾼 에이전트는 계속 돈다 · (나)=E 면 에이전트 시작 · 수거 · kill 구간이 정상 종료로 간다. (사용자 답에 맞춰 해당 줄만 남긴다.)
    ③ **결정 3(조건부 — (다)=거절 · (바)=감시 중 하나라도)** — 「이 단계를 위한 전용 장치는 만들지 않는다」에 패닉 종료 전용 장치를 더한다: (다)=거절이면 입구의 `is_pulled()` 확인 · (바)=감시면 그 단계(정상 종료)를 끝까지 지키는 감시 스레드(4판 — 3판은 (바)의 Amends 에 결정 3 을 빠뜨렸다). 둘 다 아니면 이 조항이 빠진다.
    ④ **「근거」 셋째 · 「영향」 넷째 줄(조건부 — (바)=감시)** — 미룬 장치 「종료 시간 제한」 중 **패닉 종료의 시한만** 든다(감시 — 데몬 30초 · 셸 5초 가안). 「영향」 넷째 줄의 「미룬 시간 제한도 없어 … 시간 제한을 들이는 결정이 아니라」가 패닉 경로에서는 더 맞지 않는다. 깃발 없는 StopDaemon · 트레이 끄기의 시한과 셸 사유 고지 · 다음 부팅 표시는 계속 미룬다. 미룸 유지면 이 조항이 빠지고 (바)의 멎음 넷이 §4-11 서명 잔여다. 셸 시한만이면 「셸 시한만」으로 좁힌다.
    ⑤ **「영향」 다섯째 줄 — 사실 정정(3판 · 답과 무관)** — 「다음 부팅이 그 에이전트들을 평소처럼 되살린다」는 틀렸다: 운영 데몬의 부팅 자동 복원은 꺼져 있다(`daemon/src/lib.rs:781-787` · 2026-07-09 사용자 결정의 stopgap) → 다음 부팅은 되살리지 않고, 사용자가 활성화로 이어받는다(첫 제출 뒤 영속된 세션 id — ADR-0226).
  - **ADR-0290 몫(조항은 위 명사구 · 상세 = ADR-0292 본문 — 결정 1 · 결정 끝 줄 · 「영향」 첫째 · 셋째 · 넷째 줄)** —
    ① **「영향」 넷째 줄(연결 태스크 표시 · 표시 모양)** — 연결 태스크(`lib.rs:395`)에는 표시를 세우지 않는다(M5) · 「이미 가두던 reaper」는 고리만 살리고 표시를 세우지 않는다 — 그래서 `reap_one` 패닉은 이미 표시 밖으로 올라간다((나)=E · §4-2).
    ② **「영향」 셋째 줄(4판 · 답과 무관)** — 「unwind 에서도 프로세스가 곧바로 끝나는 자리 = 메인 future 와 그 안의 연결 수락 루프에서 난 패닉 · … 정상 종료를 거치지 않는다」: 연결 수락 루프의 패닉은 M6 ② 가 `run_accept_loop` 안에서 잡아 꼬리(정상 종료)로 보낸다. 메인 future 자체(부팅 · 꼬리)의 패닉은 그 줄대로 정상 종료 없이 `terminate_self(101)` 이다(M6 ① · 5판).
    ③ **「영향」 첫 줄 — 사실 정정(3판 · 답과 무관)** — 「다음 부팅 복원을 지금처럼 탄다」는 틀렸다 — 근거는 위 0288 ⑤ 와 같다.
    ④ **결정 1(조건부 — (다)=거절)** — 「저장 멈춤은 하지 않는다(프로세스가 끝나므로)」를 꼬리 창에 한해 되돌린다: 깃발 뒤 가장 바깥 입구 셋이 새 일을 거절한다(= 그 창의 저장 멈춤). 그 결정의 전제가 몰랐던 꼬리 창(§3-1 (다))이 사유다. 받아들임이면 이 조항과 0288 ③ 의 (다) 몫이 빠진다.
    ⑤ **결정 끝 줄(무조건 — 6판 · 5판은 조건부)** — 「ADR-0288 의 나머지(결정 1 · 2 · 4 · unwind 전제 · TRD 로 넘긴 항목 · 3 · 4 의 전용 장치를 두지 않는다 · 미룬 장치)는 그대로다」(패닉 정책 브랜치의 ADR-0290 파일 15줄 — 6판 재대조): 0290 이 0288 의 그 몫을 「그대로」라 다시 적었는데 그 괄호에 **결정 1 · 2** 가 들어 있다 → 0292 는 답과 무관하게 늘 0288 결정 1 을 좁히고(0288 ① · M5) (나)=E 면 결정 2 도 좁히므로(0288 ②) 이 줄은 늘 더는 그대로가 아니다. 거기에 (다)=거절이면 「전용 장치를 두지 않는다」에 입구 확인이 · (바)=감시면 그것과 「미룬 장치」에 감시 스레드 · 패닉 종료 시한이 더해진다(0288 ③ · ④). ★0290 조항 가안은 늘 「결정 끝 유지 문장」을 싣는다★(5판의 「둘 다 아니면 빠진다」는 틀렸다).
  - §8-2 의 나머지 일곱(0142 · 0199 · 0226 · 0233 · 0262 · 0275 · 0281)은 U9 에 `/adr link` — 쌍마다 한 번이라 0288 · 0290 과 겹치지 않는다.
  - 앵커 `// ADR-0292` = 훅 조립(데몬 · 셸) · `panic` 모듈 머리 · `sync` 관찰 갈래 · 프로필 줄 · 컴파일 가드 둘 · `escalating` 구간 · `main.rs` 포착(데몬 · 셸) · 당겨진 끝의 `terminate_self(101)` 줄(5판) · 감시 스레드 둘 · platform `terminate_self`(당겨진 끝이 늘 부른다 · 5판) · (다) 입구 셋((다)=거절일 때).
- **TRD S20 §4 ⑨ 대체 표시** — ADR 은 TRD 를 링크로 폐기하지 못하므로 ADR-0292 「맥락」에 「TRD S20 §4 ⑨ 의 프로필 결정을 대체한다(핸들러 규약은 유효)」를 적고, 그 절 머리에 대체 표시를 단다(§8-2).

### 8-2. 개정 도장 (`/adr link` — 그 전제를 실제로 바꾸는 단위 착지 뒤 · U9 · 0288 · 0290 두 행은 §8-1 의 supersede 가 박는다 — 5판)

| 대상 | 무엇이 바뀌나 |
|---|---|
| ADR-0288 | ★조항 = §8-1 의 짧은 명사구 — ADR-0292 를 만드는 `/adr supersede --mode partial` 이 양방향 링크 · 상태줄 도장을 박는다 · 상세는 ADR-0292 본문 · U9 에 이 쌍의 `/adr link` 를 하지 않는다(「already-linked」 no-op — 5판)★ — 결정 1 · 2 범위 · 「영향」 다섯째 줄 「다음 부팅이 되살린다」 → 부팅 자동 복원 꺼짐 · 활성화로 이어받기(사실 정정 · `lib.rs:781-787`) · (다)=거절 · (바)=감시면 결정 3 「전용 장치 없음」 · (바)=감시면 「근거」 셋째 · 「영향」 넷째 줄의 미룬 시간 제한(패닉 종료 몫만) |
| ADR-0290 | ★조항 = §8-1 의 짧은 명사구 — 0288 행과 같은 supersede 실행이 박는다(5판)★ — 「영향」 넷째 줄(연결 태스크 표시 · reaper) · 「영향」 셋째 줄(연결 수락 루프 패닉 → 꼬리 · M6 ② — 4판) · 「영향」 첫 줄 「다음 부팅 복원을 지금처럼 탄다」 → 같은 사실 정정 · (다)=거절이면 결정 1 「저장 멈춤은 하지 않는다」(꼬리 창 한정) · 결정 끝 줄 「ADR-0288 의 나머지(결정 1 · 2 · … 전용 장치를 두지 않는다 · 미룬 장치)는 그대로다」(0290 ⑤ — ★늘 든다(6판 — 그 괄호의 결정 1 을 0292 가 늘 좁힌다)★ · (다)=거절 · (바)=감시 몫은 거기 더해진다) |
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
- CLAUDE.md — 「핵심 불변식」에 한 항목(가둠 표시는 1 · 2층 입구만 · 표시 밖 · 공유 락 오염 = 정상 종료 · `escalating` 구간 · 2층은 펌프의 `finish` 로만 끝난다(ADR-0127 과 같은 줄) · 훅 코드는 깃발까지 락 · OS 호출 · 할당 · 로그 없음(std 가 훅 앞에 만드는 형식 payload 는 별개 — 5판) · 가둔 비오염 패닉은 훅이 판정 직후 돌아가고 로그는 가둔 경계가(5판) · 관찰자는 락 · OS 호출 · 로그 없음 · 감시는 락 · 로그 없음 · 감시와 당겨진 끝은 platform 즉시 끝내기(`process::exit` 아님 — 5판) · 걸쇠 전역은 조립 자리만) · 「백엔드 모듈 맵」 base 항목(입주자 아홉 · `sync` 관찰 · 범위 갈래) · platform 항목(`process` 에 즉시 끝내기 `terminate_self` — 4판) · 게이트 ③ 줄 · 회귀 수치(U5 뒤).
- `docs/testing-strategy.md` — 게이트 ③ 사본 둘 · base 단위 목록.
- `docs/tracking.md` — T-52 에 한 줄(M12 — 패닉 종료는 `send_stop` 을 안 탄다 · 동시 종료 경합) · 이 TRD 가 미룬 것 T- 로(P3 · (가)에서 안 고른 B2 갈래 · (나)=R 구간 · 셸 상자 · 재기동 · StopDaemon 꼬리의 무한 대기(MCP · 자식 락 아래 `child.wait()`) · (바)=미룸 유지면 패닉 종료 멎음 넷). ADR-0288 이 미룬 장치(종료 시간 제한 · 셸 사유 고지 · 다음 부팅 패닉 표시)가 이미 T- 로 있는지 확인하고 없으면 연다.
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
8. **`transport.shutdown()` 두 번 · 동시 부름의 안전** — 스코프 가드와 `shutdown_all` 이 겹칠 때(M15) · ★StopDaemon 의 `shutdown_all` 과 패닉 꼬리의 `shutdown_all` 이 **동시에** 같은 통로의 `shutdown` 을 부를 때(M12 · 3판)★ — 자식 락 아래 `kill` · `wait`(`pty.rs:412-414` · `stdio.rs:394-396`) · 무리 끄기 · master drop 이 두 스레드에서 겹친다. U4 의 동시 시험(§7-1 · §7-5 3 ④)은 **연기 시험**이다(4판) — 자식 락(`pty.rs:412-414` · `stdio.rs:394-396`)이 두 부름을 줄 세우므로 seam 없이는 실패할 수 없다. 패닉 · abort · 멎음이 없다는 것만 보고 이 항목을 닫지는 않는다(락 밖 걸음 — 무리 끄기 · master drop — 의 겹침은 미검으로 남는다).
9. **통로 drop 의 ConPTY 순서** — 지금처럼 `shutdown` 없이 drop 되면 master(ClosePseudoConsole)가 무리보다 먼저 닫히는데, 그때 막히는지 미검. 스코프 가드가 그 길을 없앤다.
10. **셸 주 스레드 패닉 뒤 트레이 유령 아이콘** — 추론 · 미검(U6 관측).
11. **(가)~(바)의 사용자 답** — §4 는 권고안 기준이다. 답이 다르면 §4-2 · §4-4(감시 — platform 도우미는 당겨진 끝이 어느 답이든 쓴다 · 5판) · §4-5 · §4-6 · §4-7 · §4-8 · §4-9(셸 감시) · §4-11 · §7-1(U2 · U3 · U4 · U4b · U5 · U6 크기) · §8-1 · §8-2(조건부 Amends)만 바뀐다.
12. **`process::exit` 가 막힐 수 있는가 → 감시 · 당겨진 끝은 쓰지 않는다(4판 — T2 · 5판 · 6판)** — 3판의 이 항목은 기억에 기댔다(std 정리 방식 · `ExitProcess` 순서) — 걷는다. 4판의 말: std `process::exit` 는 다른 스레드가 이미 도는 `exit` 나 로더 락 뒤에 막힐 수 있다(추론 · 미검 — 이 PC 에 rust-src 가 없어 원문 대조 못 했다). 그래서 감시(M19)는 platform 즉시 끝내기 `terminate_self`(Windows `TerminateProcess(GetCurrentProcess(), 101)` — DLL 분리 알림 · 로더 락 · atexit · TLS 해체 · std 끝 정리를 건너뛴다)를 부른다. ★5판 정정 — 4판의 「꼬리 · M6 의 정상 끝은 `process::exit(101)` 그대로다 — 그쪽은 멎음이 관측되면 감시가 덮는다」는 틀렸다★: `process::exit` 밑의 `ExitProcess` 는 부른 스레드 밖 스레드를 — 감시까지 — 먼저 끝낸 뒤 로더 락 아래 `DLL_PROCESS_DETACH` 를 돈다(MS 문서 순서 · 실측 아님). 그 분리 알림이 멎으면(셸의 WebView2 · COM DLL — 미검) 감시는 이미 없다. → 당겨진 끝은 전부 마지막 로그 뒤 `terminate_self(101)` 이다(데몬 꼬리 · M6(+ 101 아닌 갈래 재확인 — 6판) = U5 · 셸 게시 닫힘 · `Exit` 콜백 끝 재확인(6판) · 셸 `main.rs` 포착 = U6 · §4-4 · M7). 당기지 않은 끝은 `process::exit` 그대로(지금과 같다 — 패닉 경로가 아니다 · 데몬 `main.rs` 는 그 직전 깃발을 다시 본다 — 6판). ★6판 — 5판이 적은 「우리 손 밖에 남는 `ExitProcess` = tauri `AppHandle::exit` 실패 갈래」는 닫혔다★: 셸 깨우기가 `app.exit` 를 부르지 않고 주 스레드에 닫힘을 게시한다(`run_on_main_thread` 의 실패 갈래는 `Err` 만 돌려준다 — tauri `app.rs:494-499` · wry `lib.rs:235-255` · `:2777-2779`) · tao 가 `Exit` 콜백 패닉을 삼키고 `process::exit(0)` 하는 길은 콜백 끝 재확인이 막는다(§4-9). 남는 우리 손 밖 `ExitProcess` = 당기지 않은 정상 끝의 tao `process::exit(0)`(지금과 같다)과 그 끝 재확인 뒤의 좁은 틈(§4-11). 남는 미검: non-Windows 갈래(`process::exit` 자리채움 — 5판 · 이 저장소엔 non-Windows 빌드가 없다) · `TerminateProcess` 가 자기 프로세스에서 돌아오지 않는다는 것(돌아와도 잠듦 고리가 받는다 — 5판)은 U5 실프로세스 ① · ⑤ · ⑥ 이 Windows 에서 잰다.
13. **부팅 대기(확신도 낮음)** — accept 루프 앞 부팅 걸음 중 당겨진 깃발에 막히는 대기는 못 찾았다(리뷰 · 저자 둘 다). (바)=감시면 닫힌다(§4-11).
14. **셸 감시 시한과 tao 되던지기의 겹침** — 시한(5초) 안에 주 스레드 되던지기가 끝나면 셸 `main.rs` 포착의 101(`terminate_self` — 5판)이, 아니면 감시의 101 이 먼저다 — 둘 다 같은 코드라 어느 쪽이 이겨도 결말은 같다(`Final` 없음 · `cleanup_before_exit` 없음 — 10).
15. **게시한 닫힘이 실제로 도는가(6판)** — 셸 깨우기가 `run_on_main_thread` 로 게시한 닫힘은 ① 루프가 LoopDestroyed 를 돌기 시작한 뒤 게시되면 `PostMessageW` 는 성공하고 안 돌 수 있다(추론 — 그 당김은 `Exit` 콜백 끝 재확인이 받는다) ② 주 스레드가 모달 고리(창 끌기 · 크기 바꾸기 · 네이티브 대화상자) 중이면 언제 도는지 미검 ③ 주 스레드 패닉 뒤 tao 가 사건을 처리기 없이 버린다는 것(`runner.rs:153-175`)은 원문으로 본 추론이다. 어느 경우든 끝은 감시 시한(M19)이 진다 — U6 GUI 가 배경 스레드 패닉의 닫힘(트레이 아이콘이 사라짐 · 코드 101)을 관측한다.

---

## 10. 개정 대조

### 10-1. 6판 개정 대조 (리뷰 5회차 → 6판)

> 입력 = 리뷰 5회차 취합(deep · 스냅숏 `bf3b74aa` · codex BLOCK 1 · 리뷰어 둘 FIX → 잠정 BLOCK). 남은 종료 구멍은 전부 **셸의 당겨진 끝이 tauri · tao 의 종료 장치를 지나는 길**이었고 데몬 종료 표는 유계였다. 항목은 전부 설계 · 문서 정정이고 새 사용자 결정은 없다(메인 결정). 4회차 항목 중 C1(진단 칸 결함) · C2(셸 `Exit` 구멍) · C4(일부)가 아래 D · E · A2 로 닫힌다. 저자가 원문과 대조한 것: tauri 2.11.3 `app.rs:494-499` · `:574-579` · `:1106-1121` · `:1430-1437` · tauri-runtime-wry 2.11.3 `lib.rs:235-255` · `:2748-2755` · `:2777-2779` · `:3335` · tao 0.35.3 `platform_impl/windows/event_loop.rs:225-231` · `:570-585` · `event_loop/runner.rs:95-108` · `:153-175` · `src-tauri/src/main.rs:2` · `src-tauri/src/lib.rs:400-411` · daemon `main.rs:11-16` · `state/lock.rs` · `state/boot_plugin.rs:11-13` · ADR-0290 15줄(패닉 정책 브랜치) · `adr.mjs:403-406` · `:732-733` · `:755` · core 1.95 rlib 의 「panic in a destructor during cleanup」 · std 의 「panicked while processing panic」. 대조하지 않은 것(리뷰가 든 그대로): `/adr link` 산출이 supersede(부분)와 같은 모양이라는 것 · 모달 고리 중 게시 처리(§9-15).

| 리뷰 항목 | 6판에서 바뀐 것 · 자리 |
|---|---|
| E1 — 셸 깨우기의 `app.exit`: 실패 갈래가 부른 스레드에서 `process::exit`(감시를 먼저 끝냄) · 깨우기는 `app.exit` 를 걷고 주 스레드에 닫힘 하나를 게시 | 닫힘 = `catch_unwind` 안 `cleanup_before_exit` → `terminate_self(101)` · 게시 실패면 더 안 함(감시 시한) · `run_on_main_thread` 원문 대조(실패 = `Err` 만) — **M18** · **§4-9 깨우기 + 원문 대조 하위 항목** · M7 · M19 · §0 ③ · ④ · §3-1 (바) 표 (1) · (2) · 모양 · §4-3 표지 항목 · §4-4 당겨진 끝 · §8-1 결정 ⑦ · ⑧ · 거부한 대안 · §9-12 · §9-15(새) · §7-1 U6 · §7-4 U6 ③ · §7-5 3 ③ |
| E2 — tao 가 `Exit` 콜백 패닉을 삼키고 `process::exit(0)` · 진입 확인 뒤 든 당김을 아무도 안 봄 | `RunEvent::Exit` 몸통 `catch_unwind` + 끝 재확인 `caught \|\| is_pulled()` → `terminate_self(101)` · 시작 실패 코드보다 101 이 앞 — **M18** · **§4-9 `Exit`** · §4-9 5초 예산 · M7 · §4-4 당겨진 끝 · §8-1 결정 ⑧ · §7-1 U6(범위 · 시험 둘 + 더) · §7-4 U6 ③ |
| E3 — 데몬 `main.rs` 의 101 아닌 끝(`Ok` · `Err(1)`) | 끝내기 직전 `is_pulled()` 면 `terminate_self(101)` — **M6** · **§4-4 메인 future** · M7 · M19 · §4-4 당겨진 끝 · §0 ④ · §8-1 결정 ⑧ · §9-12 · §7-1 U5 · §7-4 U5 ⑤ |
| E4 — 종료 장치 서술 갱신 · 셸 `main.rs` 포착은 `cleanup_before_exit` 를 건너뜀 | §4-11(「패닉 종료의 멎음」 ② 갈아 씀 — `app.exit` 실패 갈래 · tao 삼킴 닫힘 · 끝 재확인 뒤 좁은 틈 · tauri 뒤 정리 패닉은 받는 잔여 · 「배경 먼저 · 주 스레드 나중」 닫힘 · 사유 기록 항목 넓힘 · 트레이 아이콘 항목) · §5 6행 · §9-12 · §4-9 주 스레드 패닉(`AppHandle` 없음 — 받는다) · M18 끝 · §7-1 U6 크기(+175 → +195) |
| D1 — 칸에 문장도 · 무장 | 문장(`payload_as_str` · 256바이트 · 문자 경계) + 자리 + 「무장」 — **§4-1**(`record_contained` · `ContainedNote`) · §4-3 (a′) · 새 항목 · M4 · §0 ③ · §7-1 U1 |
| D2 — 깊이별 칸 · 경계가 읽으며 비움 | `slot[depth % 4]` · catch 갈래가 자기 깊이 칸을 take · 입구가 칸을 비움(가안) — **§4-1** · M4 · §8-1 결정 ⑥ · §7-1 U1 시험(둘째 읽기 빔 · 중첩 · 물려받지 않음) |
| D3 — 올리는 훅이 무장 칸을 덧붙임 | (a″) 스택 사본(덮기 전) → (d) · (e) 에 「unwinding contained panic: …」 — **§4-3 의사코드 · 새 항목** · §4-1 `armed_contained` · M4 · §4-7 락 순서 · §4-11 진단 손실(남는 것 = B 가 가둔 비오염이면 기록 없음) · §8-1 결정 ⑥ · §7-1 U5 시험 · §7-4 U5 |
| D4 — 되감는 중 `Drop` 안 중첩 패닉은 소멸자를 벗어날 때만 abort | 5판 「`catch_unwind` 무력」 정정 — **M16** · **§4-11 abort 갈래** |
| D5 — 진행 셈 순서 | `hook_enter` · `hook_exit` · `wait_reason` 읽기 SeqCst(최소 Release / Acquire) — §4-1 · §4-3 (b′) · (d′) · M4 · §7-1 U1 |
| D6 — 「(f) 의 stderr 에 남는다」 | 배포판 셸은 콘솔 없음(`src-tauri/src/main.rs:2`) → 로깅 앞 당김은 코드 101 만 — **§4-9 설치** |
| A1 — 「예약된 0292」 | 번호 = 실행 때 스크립트(로컬 최대 +1) · 전제(0288 · 0290 · 0291 든 트리 · 원격 최대 번호 먼저 · 대상 없으면 쓰지 않고 멈춤) · 대안(`/adr new` + push 뒤 쌍마다 `/adr link`) — **§8-1 머리 · 링크 새 하위 항목** · 채번 줄 · 머리글 결정 틀 줄 |
| A2 — 0290 ⑤ 는 무조건 | 그 괄호에 결정 1 · 2 → 0292 가 늘 결정 1 을 좁힌다 · 조항 가안이 늘 「결정 끝 유지 문장」 — **§8-1 0290 ⑤** · 조항 가안 · §8-2 0290 행 · §3-1 (다) · (바) |

**거부 · 일부 거부한 것(근거):** 없음 — 전부 받아들였다. 저자가 덧붙이거나 고른 것: ① E1 — 게시 닫힘은 결정대로 정리 · 끝내기만 하고 사유를 로그하지 않는다 → 셸 당김의 사유 기록은 훅 (e) 하나(§4-11 항목을 넓혔다) · 실행 잠금은 끝내기가 핸들을 닫아 OS 가 푼다(`state/lock.rs`) ② E1 — `MAIN_ESCALATED` 는 남기되 안전에 필요하지 않다고 적고 걷을지는 U6 에 넘겼다(tao 가 주 스레드 패닉 뒤 사건을 버린다 — 추론) ③ E2 — 진입 갈래(사용자 끝내기와 겹친 당김)는 5판 `Exit` 패닉 갈래를 그대로 둔다 · tauri 의 뒤 정리(`app.rs:1433`) 패닉 · 끝 재확인 뒤 좁은 틈은 §4-11 받는 잔여로 ④ D2 — 「입구가 칸을 비움」(가안)과 깊이 5 이상 칸 돌려 쓰기를 더했다 ⑤ D3 — 무장 칸을 (a′) 가 덮기 **전에** 스택에 받는 (a″) 순서(같은 깊이의 B 가 A 의 칸을 덮으므로) ⑥ E3 — `main.rs` 재확인 두 줄은 시험 seam 을 두지 않는다(가안).

### 10-2. 5판 개정 대조 (리뷰 4회차 → 5판)

> 입력 = 리뷰 4회차 취합(deep · 스냅숏 `85debb24` · codex BLOCK 1 · 리뷰어 둘 FIX · BLOCK 없음 → 잠정 BLOCK). 핵심 불변식(kill 2동사 · finalize 1회 · 락 순서 · leftover 잎 락 · ADR-0127)은 깨지 않았다. 항목은 전부 설계 · 문서 정정이고 새 사용자 결정은 없다(메인 결정). 3회차 항목 중 일부만 닫혔던 T2 · T3 · G1 과 되돌아간 A3 은 아래 C2 · C5(T2) · C1 · C8 · C9(T3) · C6(G1) · C3(A3)이 닫는다. 저자가 코드 · 원문과 대조한 것: adr 스킬 `references/flow.md:29` · `:33` · `:53-59` · `:70-80` · 바인딩 `.claude/skill-bindings/adr.md:27` · ADR-0290 결정 끝 줄(패닉 정책 브랜치 · 15줄) · `platform/src/process.rs:1-3` · `:385-411` · `platform/src/lib.rs:11-12` · `:38-43` · `connection_core.rs:1542-1543` · daemon `lib.rs:823-824` · `main.rs:13-14` · `manager.rs:3113` · `mcp_server.rs:1372-1400` · `src-tauri/src/lib.rs:45` · `:52-60` · `:128` · `:408-409` · `state/boot_plugin.rs:89` · `state/saver.rs:44`. 대조하지 않은 것(리뷰가 든 그대로): axum 0.8.9 `routing/path_router.rs` · `PanicHookInfo::payload_as_str` 의 안정 판 · `FormatStringPayload` · `ExitProcess` 의 스레드 끝내기 → 분리 알림 순서.

| 리뷰 항목 | 5판에서 바뀐 것 · 자리 |
|---|---|
| C1 — 가둔 비오염 패닉의 훅 로그가 tracing 락 자기교착(깃발 앞 · 감시 안 깨어남 — codex BLOCK) | 훅이 판정 직후 돌아감 · 자리(파일 256바이트 · 줄 · 열)를 const TLS 고정 칸에 · 로그는 `contain` · `Contained` catch 갈래 — **§4-3 의사코드 (a′)** · 새 항목 · §4-1(`record_contained_location` · `Caught.location()` · 머리 주석) · M4 · §0 ③ · §4-7 락 순서 · §4-11(진단 손실 새 항목 · tracing 락 항목 좁힘) · §6 새 의존 · §7-1 U1 시험 · §8-1 결정 ⑥ · §8-3 |
| C2 — `ExitProcess` 가 감시까지 끝낸 뒤 DLL 분리에서 멎으면 덮을 이가 없다 | 당겨진 끝 = 마지막 로그 뒤 `terminate_self(101)` — **§4-4 새 항목 「당겨진 끝」** · M6 · **M7** · M18 · M19 · §0 ④ · §4-4 꼬리 · 메인 future · §4-9 `Exit` 갈래 · 주 스레드 패닉 · §4-11(「감시만」 정정 · tauri 실패 갈래 잔여) · **§9-12** · §9-14 · §7-1 U5(`main.rs` 101 갈래 둘) · U6(셸 `main.rs` · `Exit` 갈래) · §7-4 U5 ⑤ · U6 ③ · ③′ · §7-5 3 ③ · §8-1 결정 ⑧ · 앵커 · §0 표 (바) 영향 · §9-11 |
| C3 — `/adr new` 는 링크를 안 쓴다 · Amends 는 supersede(부분) · link 만 · 조항 문구 제약 | ADR-0292 = `/adr supersede --mode partial` 한 번(0288 · 0290 함께 · 짧은 명사구 조항 · 상세는 본문 · 뒤 `/adr link` 없음 · 채번 확인) — **§8-1 머리 · 링크** · 결정 ⑫ 끝 · §8-2 머리 · 두 행 · §7-1 U9 · §3-1 (다) · (바) |
| C4 — ADR-0290 결정 끝 줄 「ADR-0288 의 나머지(…)는 그대로다」 | **§8-1 0290 ⑤**(조건부 — (다)=거절 또는 (바)=감시) · §8-2 0290 행 · §3-1 (다) · (바) |
| C5 — `terminate_self` 모양 · 다른 OS 갈래 | `TerminateProcess` 뒤 잠듦 고리(`abort()` 아님) · 그 밖 = `process::exit` 자리채움(`extern "C" _exit` 없음 — 헤더 규칙 3 · ADR-0266 결정 7) · 책임 목록 둘 — **§4-4 platform 항목** · M19 · §6 platform · §7-1 U1 파일 · §7-4 U1 ⑤ · §9-12 |
| C6 — 맨 `route_layer` 는 먼저 붙은 `nest_service(MCP_PATH)` 까지 덮는다 | 제어 라우트마다 `.layer` 또는 경로 확인 · 맨 `route_layer` 걷음 — **§4-6 거절 ②** · §7-1 U3(범위 · 시험) · §7-4 U3 ⑤ |
| C7 — 깃발 뒤 StopDaemon 의 둘째 `shutdown_all`(재진입 막이 없음) | 깃발이면 `shutdown_all` 없이 `Ok` · 같은 신호 · Close — **M12** · §3-1 (다) · §4-4 StopDaemon · §4-6 ① · §8-1 결정 ⑧ · §7-1 U3(범위 · 시험) · §7-4 U3 ⑤ |
| C8 — `wait_reason` 의 헛된 대기 · 셸 5초 예산 | 「올리는 훅 진행 중」 원자 셈(`hook_enter` · `hook_exit`) · 셈 > 0 일 때만 대기 — §4-1 · §4-3 (b′) · (d′) · M4 · §4-4 꼬리 · §4-9 `Exit` 갈래 · **§4-9 셸 5초 예산 새 항목** · §0 ④ · §7-1 U1 시험 · §7-5 3 ③ |
| C9 — 「할당 없음」 문장 · 주 스레드 판정 · `payload_str` | 「훅 **코드**는 깃발까지 할당 없음」(std 형식 payload 는 별개) — §4-3 · §4-7 락 순서 · §8-3 · M4 · 주 스레드 = const TLS `IS_MAIN` — M18 · §4-3 · §4-9 · std `payload_as_str` — §4-1 · §4-3 · §7-1 U1 |
| C10 — 셸 설치 자리(로깅은 boot 플러그인 안에서 선다) | `run()` 머리 `Builder` 앞 · 그 전 (e) 무동작 · 감시 기동 실패 = `process::exit(STARTUP_FAILED_EXIT_CODE)` — **§4-9 설치** · M19 · §4-3 의사코드 · §7-1 U6 · §7-4 U6 ③ |
| C11 — StopDaemon 중 깃발 · flush 손실 | 한 문장(+ 쓰기 도중 끊김 안전 · 로그 줄 잘림 받음) — §3-1 (바) 표 · §5 11행 · §4-11 정상 종료 멎음 |
| C12 — 감시 폴링 주기 | ≈1초(깨우기 200ms 그대로) — M19 · §4-4 감시 · §4-1 `wait_polling` 주석 |
| C13 — M22 `AssertUnwindSafe` | 필요(`dyn AgentTransport` 가 `RefUnwindSafe` 아님) · 정당(catch 뒤 `emit(Dropped)` 만) — M22 · §4-7 |

**거부 · 일부 거부한 것(근거):** 없음 — 전부 받아들였다. 저자가 덧붙이거나 고른 것: ① C1 — 자리 칸은 가둔 패닉이면 오염이어도 채운다(가둔 오염 패닉도 catch 갈래가 자리를 싣는다) ② C2 — 우리 손 밖 `ExitProcess`(tauri `AppHandle::exit` 실패 갈래)를 §4-11 잔여로 적었다 · `terminate_self` 가 당겨진 끝에 늘 쓰여 (바) 답과 무관해졌다(§0 표 · §9-11 · 앵커) ③ C3 — 조항 가안 문구(§8-1) ④ C7 — 단위 = U3(`ConnectionCore` 걸쇠 칸 · (다) 답과 무관 · U5 전엔 무동작) ⑤ C10 — 감시 기동 실패 코드 = 기존 `STARTUP_FAILED_EXIT_CODE`(`src-tauri/src/lib.rs:408-409`).

### 10-3. 4판 개정 대조 (리뷰 3회차 → 4판)

> 입력 = 리뷰 3회차 취합(deep · 스냅숏 `d73aaceb` · codex BLOCK · 리뷰어 둘 FIX → 잠정 BLOCK · codex BLOCK 1 은 리뷰어 둘이 따로 확인). 항목은 전부 설계 · 문서 정정이고 새 사용자 결정은 없다(메인 결정). 2회차 항목 중 일부만 닫혔던 셋 — UC(거절 층이 MCP 까지 덮음) · D6(M22 모양) · D9(실패할 수 없는 시험) — 은 아래 G1 · I1 · A6 이 닫는다. 저자가 코드 · 원문과 대조한 것: tauri 2.11.3 `app.rs:574-579` · `:1110` · `command_delivery.rs:537` · `DaemonStatusSink::new` 넷 · `manager.rs:746-753` · `pty.rs:321-333` · ADR-0290 「영향」 셋째 줄 원문(패닉 정책 브랜치).

| 리뷰 항목 | 4판에서 바뀐 것 · 자리 |
|---|---|
| T1 — 셸 시한이 깨우기와 같은 스레드라 상한이 아니다(`AppHandle::exit` 실패 갈래 = 로그 · 트레이 락 · `exit` 를 그 스레드에서) · 3판 「그 밖은 같다」 오류 | 셸도 훅 설치 직후 따로 띄우는 감시 · 깨우기 = `app.exit(101)` 하나 — M18 · **M19** · §3-1 (바) 모양 · §4-9 · §4-11(셸 setup 앞 멎음 닫힘) · §7-1 U6(+175) · §7-4 U6 · §9-14 · §8-1 거부한 대안 |
| T2 — 감시의 끝내기 = platform 즉시 끝내기 · §9-12 기억 서술 정정 | platform `process::terminate_self`(가안) — M19 · §4-1 주석 · **§4-4 새 항목** · §6 platform 게이트 · §7-1 U1(+385 — 소비자 U5 · U6 가 다른 물결이라 U1) · §7-4 U1 ⑤ · §7-5 2 · §8-1 앵커 · §8-3 CLAUDE.md platform 항목 · **§9-12 다시 씀** |
| T3 — 깃발 앞의 시한 없는 일(할당 · 포맷) vs 1회차 「빠른 꼬리가 사유를 잃음」 | 훅 순서 (a) 할당 없는 판정 → (b) 표지 → (c) 깃발 → (d) 사유 → (e) 로그 — **§4-3 의사코드 다시 씀** · `payload_str` · `wait_reason`(§4-1) · M4 · §0 ③ · ④ · §4-4 꼬리 사유 로그(끝에서 1초) · §4-7 락 순서 · §4-9 `Exit` 갈래(같은 대기 — 저자가 셸에 넓혔다) · §8-1 결정 ⑥ |
| T4 — 감시는 `cfg(panic = "unwind")` 빌드만 | M19 · §3-1 (바) 모양 · §4-3 조립 · §4-4 · §4-9 · §7-1 U5 · U6 · §7-2 감시 항목 |
| G1 — 거절 ② 를 앱 전체 층에 두면 MCP `nest_service`(`mcp_server.rs:1372` · 층 `:1401-1407`)의 세션 DELETE · SSE 끊기까지 거절 | 제어 라우트만 · MCP 거절은 도구 감싸개 하나 — §3-1 (다) · §4-4 (다) · **§4-6 거절 ②** · §7-1 U3 시험 |
| G2 — 거절 ① 이 정리 꼴 명령까지 막는다 | WS 입구 허용 목록(StopDaemon · 구독 해지 · 뷰포트 놓기 꼴) — §3-1 (다) · §4-4 · §4-6 ① · §7-1 U3 시험 |
| S1 — U4 의 M21 명시 당김이 U4 뒤 U5 전에 거절만 깨운다 | U4 = 가둠 · 버림만 · 사유 · 당김 · 생성자 인자는 U5 — M21 · §4-5 4 · §7-1 U4 · U5(선행 · 파일) · **§7-2 멈춤 항목** · §7-3 · §7-4 U4 · U5 ③′ |
| S2 — `contain` 안의 비오염 패닉은 훅이 사유를 안 적는다 | M21 이 잡은 문장을 당기기 앞에 첫 사유로 — M21 · §4-1 `record_reason` 주석 · §4-5 4 · §7-1 U5 시험 |
| S3 — 층 표에 상태 싱크가 없다 · 실제 노출은 오염뿐 | §4-2 새 행 「3 — 가둔 뒤 당김」 · M21(`net/src/ws.rs:208-217`) |
| I1 — M22 `Drop` 가드의 P1 · P1o 건너뜀 · 확인 뒤 행동 경합 | 맨 `catch_unwind` → `Dropped` → `resume_unwind`(`contain` 아님) — **M22** · **§4-7** · §3-1 (가) † · §4-11 · §7-1 U4 시험 · §7-4 · §8-1 거부한 대안 · §0 메인 결정 목록 |
| I2 — 통로가 받아 둔 뒤 패닉하면 거짓 `Dropped` | §4-11 새 잔여(통로 trait 무변경) |
| A1 — (바)=감시의 Amends 에 ADR-0288 결정 3 | §3-1 (바) · §3-1 (다)(합침) · §8-1 0288 ③ · §8-2 |
| A2 — ADR-0290 「영향」 셋째 줄(메인 future · 연결 수락 루프 패닉 = 정상 종료 없음)을 M6 ② 가 바꾼다 | 원문 확인(패닉 정책 브랜치 · 셋째 줄이 인용과 같다) — §8-1 0290 ② · §8-2 |
| A3 — `/adr link` 는 같은 쌍에 둘째 조항을 못 더한다(`~/.claude/skills/adr/feedback.md:151`) | §8-1 — 쌍마다 합친 한 줄(0288 ①~⑤ · 0290 ①~④)을 `/adr new` 때 · §8-2 두 행 |
| A4 — M20 읽는 쪽 목록 · 시험 서버 · `DaemonStatusSink::new` 부르는 곳은 넷 | M20 · §4-1 seam · §4-6 HTTP · §7-1 U3(시험 서버 조립) · U5(넷) · §7-4 U5 ③′ |
| A5 — M9 앵커 `command_delivery.rs:527` 은 함수 머리 | `:537` — M9 · §6 |
| A6 — D9 시험은 실패할 수 없다(자식 락이 줄 세움) | 연기 시험으로 이름 바꿈 · 「차례로는 통과로 치지 않는다」 걷음 — §4-8 · §7-1 U4 · §7-5 3 ④ · §9-8 |
| A7 — `SpawnReservation::drop` 의 `.expect`(`manager.rs:746-753`)가 (나)=E 되감기 중에 돈다 | `sync::lock` 으로(U4) — M23 · §4-11 abort 갈래 · §7-1 U4 · §7-4 |
| A8 — 「StopDaemon · 트레이 끄기는 건드리지 않는다」는 과장(그 꼬리 도중 깃발이 설 수 있다) | §3-1 (바) 표 · §4-4 감시 · §4-11 정상 종료 멎음 · §5 11행 · §8-1 0288 ④ |
| A9 — 펌프는 사유 칸을 `shutdown` · `try_wait` 분류(`pty.rs:321-333`) **앞**에서 읽는다 | §4-7 PTY 감시 |

**거부 · 일부 거부한 것(근거):** 없음 — 전부 받아들였다. 저자가 덧붙인 것 둘: ① T3 의 유계 사유 대기를 셸 `RunEvent::Exit` 패닉 갈래에도 적용(같은 1회차 걱정이 셸에도 선다 · §4-9) ② G1 의 모양(`route_layer` · 경로 확인)은 `route_layer` 와 이미 붙은 `nest_service` 의 관계가 미검이라 U3 가 시험으로 고른다(§4-6).

### 10-4. 3판 개정 대조 (리뷰 2회차 → 3판)

> 입력 = 리뷰 2회차 취합(deep · 스냅숏 `c7bf5056` · codex BLOCK · 리뷰어 둘 FIX → 잠정 BLOCK). 1회차 항목 중 일부만 닫혔던 것(셸 종료 순서 · M7 · R-M2 `name_allocation` · R-F6 입구 범위)은 아래 D1 · D7 · D14 · UC 행이 닫는다. 취합의 사실 셋은 저자가 바로잡아 실었다 — 셸 `.lock().map_err` 는 24 가 아니라 28곳 · ADR-0288 의 시간 제한 미룸은 「영향」이 아니라 「근거」 셋째 줄(「영향」 넷째 줄은 그 귀결) · 데몬에는 깨우기 스레드가 없다(깨우기 = accept 루프 interval 팔).

| 리뷰 항목 | 3판에서 바뀐 것 · 자리 |
|---|---|
| UA — `send_input` 의 `emit(Queued)` 뒤 가둔 패닉 = 유령 대기 입력(우편 멈춤 · 30분 fail-open 없음) · P2 「다른 에이전트 안전」 문구와 어긋남 | §3-1 (가) †(오염 축 모든 안의 비용 칸) · §5 1행 · 가드 = **M22**(§3-2 · §4-7) · 남은 틈 §4-11 · §7-1 U4 |
| UB — (나) E 의 자원 실패(스레드 기동)도 재시작 · R 은 reaper 고리 표시 + 재시도가 있어야 선다 | §3-1 (나) E · R 행 · §5 3행 · §4-2 · §4-8(`reap_one`) |
| UC — (다) 거절이 ADR-0290 결정 1 · ADR-0288 결정 3 을 일부 되돌린다 · 범위 = 가장 바깥 입구만 · 제어 HTTP `agent.*` 가 빠져 있었다 | §0 표 · §3-1 (다) · §4-4 · **§4-6 거절 범위 다시 씀** · §7-1 U3 시험 · §8-1 · §8-2 조건부 Amends |
| UD — (라) S1a = `INTERNAL` 답 + 창 닫힘 **둘 다** · S1a′ 는 셸 오염 정책이 따로 필요 | §0 표 · §3-1 (라)(28곳) · §5 6행 |
| UE — 새 결정 (바) 패닉 종료의 시한(되감기가 들이는 멎음 넷) | §0 ④ · 표 6행 · §3-1 (바) · §5 11행 · 설계 = **M19**(§3-2 · §4-4 감시 스레드 · §4-9 깨우기 시한) · §4-11 · §7-1 U5 · U6 · §8-1 · §8-2 조건부 Amends · §9-12 |
| D1 — 셸 훅: `MAIN_ESCALATED` 를 당기기 앞에 · 깨우기는 `app.exit` 먼저 · 순서로 못 막는 변형 | §4-3 훅 순서(①′) · M18 · §4-9 깨우기 · §4-11 경합 잔여 · §7-1 U6 — 깨우기의 로그는 `Exit` 갈래로 옮겼다(아래) |
| D2 — PTY 감시 catch 갈래: 사유를 자식 끄기 · master 놓기 **앞**에 · 펌프는 EOF 뒤 읽음 | §4-7 감시 · §7-1 U2 시험 · §7-4 |
| D3 — 시험 seam: 걸쇠를 주입 · 전역은 조립 자리만 · 관찰자 칸 | **M20**(§3-2) · §4-1 · §4-3 시험 · §4-6 · §4-9 · §6 결과 줄 +1 · §7-1 U1 · U3 · U5 · U6 · §7-5 3 |
| D4 — kill 길 `enter_exiting` → 상태 싱크 → net `ws.rs:212` `.expect` 가 `session.kill` 앞 | **M21**(§3-2 · §4-5 4) · §7-1 U4 · §7-3 · net 무변경 |
| D5 — stderr: 첫 패닉 뒤 해석 멈추고 버리기만 | §4-7 stderr · §7-1 U2 · §7-4 |
| D6 — `send_input` 스코프 가드 | **M22** · §4-7 · §3-1 (가) † 포인터(§4-7 M22) · §4-11 · §7-1 U4 |
| D7 — `run()` 지역값 drop(관리자 · 추적기 `Drop`) · M7 예 정정 · `child.wait()` 무한 대기 | **M23**(§3-2 · §4-4) · M7 예 정정 · §4-11(`child.wait()` `pty.rs:414` · `stdio.rs:396` · abort 갈래) · §7-1 U4 · U5 |
| D8 — 문장 정정: watch 구독자 하나 · `sweeper.stop()` · `shutdown_tx` 사본 · `try_with` · 훅이 락을 잡는다 · 「로그만 잃는다」 · `escalating(reap_one)` 무의미 | M3 · §4-4 깨우기(`lib.rs:383` · `:438`) · M6 · §4-4 포착 자리(`run_accept_loop` 안) · M1 · §4-1 · §4-3 「훅이 잡는 락」 · §4-7 락 순서 · §4-11 tracing 락 · §4-2 · §4-8 · §8-3 CLAUDE.md 문구 |
| D9 — StopDaemon 과 패닉 꼬리의 **동시** `transport.shutdown` 시험 | §4-8 · §7-1 U4 · §7-5 3 ④ · §9-8 |
| D10 — U4 는 U2 뒤(의존) · 가드는 통로 → 세션 핸들 · 등록 안 되돌림 | §4-8 · §7-1 U4 선행 · §7-2 물결 · 직렬 · §7-3 |
| D11 — ADR-0292 결정 ③ `DeliveryDone` 줄 기록법 · ADR-0288 「영향」 부팅 복원 문장이 거짓 | §8-1 추기 규칙 · 사실 정정 Amends(ADR-0290 「영향」 첫 줄도) · §8-2 · §7-1 U8a |
| D12 — 부팅 중 깃발을 보는 이는 accept 루프뿐(확신도 낮음) | §4-3 조립 · §4-11(감시로 잔여 아님 — (바)=감시일 때) · §9-13 |
| D13 — U8a 래치 감사에 ADR-0226 0턴 id 불변식 | §7-1 U8a |
| D14 — `name_allocation`: 데이터 없는 순서 락은 P2 에서 범위 · 기본 관찰에 두지 않는다 | §4-5 3 메인 규칙 · §7-1 U4 |
| (메인 지시) (다) 입구 범위 정합 | §4-4 · §4-6 · `mcp_server.rs` blocking 여섯 → `spawn_blocking_contained` · M9 허용 목록(§3-2 · §6 — 그 파일은 `:1411` 하나) · §7-1 U3 |
| (메인 결정) (바) 감시 모양 | M19 — 데몬 = 훅 설치 직후 전용 스레드 · 로그 · 락 없음 · 기동 실패 = 부팅 실패 / 셸 = 깨우기 시한 5초 · `MAIN_ESCALATED` 여도 · `Final` 없음 · 주입 · M8 `hang` |

**거부 · 일부 거부한 것(근거):**

- **D1 「깨우기는 `app.exit` 먼저, 로그는 뒤」** — `app.exit` 먼저는 받았다. 로그는 깨우기에서 빼 `RunEvent::Exit` 패닉 갈래로 옮겼다 — (바)의 셸 시한을 같은 깨우기 스레드에 얹으므로, 그 스레드가 로그 락에 막히면 시한도 막힌다(시한의 표적이 바로 그 멎음이다). 대가 = 주 스레드 패닉의 사유 기록은 훅 줄 하나뿐(§4-11). 대안 = 셸에도 감시 스레드를 따로 두고 깨우기는 로그까지 하는 것(§4-9) — 메인 결정 「기존 깨우기에 시한」을 따랐다.
- **D3 「관찰자 칸을 주입 가능하게」** — 고르지 않고 「등록 상태는 자기 통합 시험 바이너리」 쪽을 골랐다. 관찰자는 base `sync` 의 자유 함수들이 부르므로 칸을 주입하려면 그 함수 전부가 인자를 받아야 한다(저자 판단).
- **D4 「또는 종료 경로가 싱크 패닉을 견딘다」** — 고르지 않았다(§4-5 4 — 평시 요청 경로의 같은 패닉이 1층에 갇힌다).
- **D7 「또는 당겨졌으면 join 을 건너뛴다」** — agent 가 전역 걸쇠를 읽어야 해서(M20 과 어긋남) `thread::panicking()` + `mem::forget` 조합을 골랐다.
- 그 밖 항목은 전부 받아들였다(위 표).

### 10-5. 2판 개정 대조 (리뷰 1회차 → 2판)

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
