# TRD — 경계 리팩터링 1-3: OS 코드를 `platform` crate 로 모은다 (S21)

> 상태: **4판(2026-10-04) — 결정 반영(§7 · §8).** 코드는 아직 한 줄도 바뀌지 않았다.
> ★**착수 순서 — 1-3 은 1-1(base 범용 도우미)이 머지된 뒤에만 시작한다**★(파일이 크게 겹친다 — §5 선행 · §5-3). **원자적 쓰기(`write_atomic`) 통일은 storage P3 가 master 에 착지할 때까지 기다린다**(§5 U-W).
> **개정(2026-10-04, 결정 반영):** ① §7 의 선택 셋을 결정 기록으로 바꿨다 — 사용자 위임(2026-10-04 「알아서 진행해」) → 3판 권고안 채택: **D1 = (a) `windows` feature 를 쪼개지 않는다 · D2 = `engram-dashboard-platform` · D3 = B(OS 층만 platform · 중립 손잡이 `ProcessGroup` · `RetiringSignal` 은 agent 어댑터 `agent/src/transport/process_group.rs`)** ② 문서 전체를 D3=B 한 갈래로 맞췄다 — A 갈래 서술은 §3-4 「왜 A 가 아닌가」 한 단락으로 줄였다(§2-2 · §2-5 · §3-4 · §3-8 · §5-1 U3 · §5-2 · §5-3 · §6) ③ ADR-0275 로 박을 개정 목록을 새로 적었다(§8 — 0274 는 다른 브랜치가 쓴다) ④ 형제 1-1 의 결정 넷(S2=(e) 경고 없는 되찾기 · N1 기능 이름 `test-support` · C4=(a) 가짜 시계는 base `time` 안 · C5=(b) transport → base 는 3단계로)을 이 문서에 맞췄다(§3-8 · §4-3 · §5 선행 · §5-1 U5) ⑤ 「미검 · 열린 것」을 §9 로 밀었다.
> **개정(2026-10-03, 2차 리뷰 반영):** ① D3=B 의 약한 손잡이 만들기 경로를 끝까지 적었다 — stdio `process_group()` 은 `GroupOwner::downgrade()` 의 `GroupRef` 에 `RetiringSignal::of(&self.retiring)` 를 얹어 agent 어댑터로 싼다(물러남 표시를 빼면 잔여물 정리가 멈춤 조건을 잃는다) · 시험 도우미 `new_group` 은 B 면 agent 어댑터 시험 지원에 남고 platform `testing` 은 OS 수준 도우미 넷만 낸다 · A 의 같은 자리도 적었다(§2-2 · §2-5 · §3-4 · §3-8 · U3 · §7) ② discovery 는 1-3 뒤에도 base 를 쓴다 — 먼저 착지하는 1-1 이 discovery `Clock` 을 base `time::Clock` 의 하위 트레이트로 만든다(ADR-0269 결정 3-2). base 의 「링크된다」 서술에서 빠지는 것은 `net` 하나다(§4-1 · §4-3 · §5-1 U1 · §6) ③ 1-1 과의 `codex/transport.rs` 겹침을 개수 대신 「1-1 의 U3b 범위」로(§5 선행) ④ U3 의 `usage/process.rs` 몫 = `crate::platform` 참조 다섯(운영 셋 · 시험 둘)(§5-1) ⑤ U1 포인터에 `thread_lock.rs:1236`(§5-1) ⑥ CLAUDE.md 의 net 게이트 2b 줄을 고칠 자리에(§4-3 · §6).
> **개정(2026-10-03, 1차 리뷰(`/review trd full`) 반영):** ① U4 는 자기 두 파일만 지우고, agent `platform/` 정리 · `windows` → dev-dependency 는 뒤에 오는 U3 이 진다 — B 의 어댑터 자리 = `agent/src/transport/process_group.rs`(제안 · §3-4 · §5-1) ② U3 에 소스 문자열 시험(`codex/transport.rs` 의 가드 배치 시험)을 넣었다(§5-1) ③ U5 에 찾기 명단(`LookupGate`)의 독 처리 · 로그를 적었다(§5-1 · §3-7) ④ 불변식 게이트: `-g '*.rs'` · 괄호 한 겹 허용 · 기대 명단 근거 정정 · 한계 보강(§4-4) ⑤ 게이트 사본 · 낡는 주석 추가 — `docs/testing-strategy.md` · 메시징 이름 게이트에 `platform` · 루트 · net · discovery 매니페스트 주석(§4-3 · §6) ⑥ `GroupOwner` 신설은 두 안 모두 ADR-0266 결정 6 개정이다 · 「Clone 아님 = 통로 하나 강제」 과장 정정(§0 · §3-4 · §7) ⑦ 옛 R7(`daemon/src/experiment`)은 하네스 전용이라 남긴다(§2) ⑧ `test-support` 운영 누수 게이트 · `testing` 모듈 `cfg(any(test, …))`(§3-8 · §4-2) ⑨ base 게이트 ③ 의 1-1 쪽 이름 정정(`file` 은 1-1 이 미룬다 · §4-3).
> **범위 = 작업 순서 1-3 중 원자적 쓰기(`write_atomic`) 통일을 뺀 전부**(`docs/refactoring/architecture-discussion-2026-09-26.md` §10 — 원자적 쓰기는 storage P3 착지 뒤, §5 U-W).
> **배치 근거:** `docs/README.md` 「새 내용을 어디에 넣나」의 「새 기능 **설계 착수** → `process/SN-name/` 새 폴더」. 같은 단계의 형제 문서(1-1 base 공용 함수 TRD)가 이 폴더에 함께 선다.
> **표기:** 「실측」 = 이 판의 기준 커밋 `0ef6292`(브랜치 `v0.3.3/refactor/crate-boundaries`)에서 잰 것과 그 명령 · 「사용자 결정(날짜)」 = 문서에 적힌 사용자 결정만 · 「결정(2026-10-04)」 = 사용자 위임으로 권고안을 채택한 §7 의 셋(늘 「결정 D1」 · 「§7 D1」 꼴로 적는다 — §2-1 표의 행 id `D1` · `D2`(discovery 의 끄기 · WMI)와 다른 것이다) · 「ADR」 = 확정 ADR 본문 · 「제안」 = 이 TRD 의 안(crate 이름 말고는 이름이 전부 가안).
> 앵커: **ADR-0266**(이 crate 의 헌장) · **ADR-0275**(이 TRD 의 결정 기록 · ADR-0266 결정 6 개정 — 박은 내용 = §8 · 2026-10-04 에 1-1 몫과 한 ADR 로 박았다) · ADR-0268(platform 은 `tracing` 을 직접 부른다 · base 를 의존하지 않는다) · ADR-0269(원자적 쓰기는 platform 으로 — 결정 8 쪽 · base 입주 조건) · ADR-0175(결정 1·2 와 거부한 대안 첫 항목 중 platform 쪽을 뒤집는다) · ADR-0218 결정 11(뒤집는다) · ADR-0262(`ProcessGroup` 중립 손잡이 · 잔여물 정리 락 규칙) · ADR-0230(플랫폼 중립) · ADR-0024 · ADR-0271(discovery 나누기 = 2-2 · WMI 는 platform) · ADR-0001(kill 인과) · ADR-0012(모듈 격리 하네스) · step-log S21 · `docs/tracking.md` T-47.

---

## 0. 결론 (먼저)

```
① 새 워크스페이스 crate 하나 — 워크스페이스 의존 0, 서드파티는 windows(cfg(windows) 한정) + tracing 뿐.
     #[cfg(windows)] · cfg!(windows) · #[cfg(unix)] 는 이 crate 안에만 산다(테스트 · 하네스 · 예제 제외).

② 부르는 쪽은 함수와 핸들 타입만 부른다 — 컴파일 시점 분기, OS 고르기용 트레이트 객체 없음.
     시험용 가짜가 필요하면 부르는 쪽이 작은 트레이트를 둔다(discovery · thread_lock · leftover 에 이미 있다).

③ Job Object 는 platform 에서 「무리 주인(강)」과 「무리 손잡이(약)」 두 타입으로 감싼다(결정 D3 = B).
     한 무리의 강한 주인은 하나다 — 타입이 Clone 이 아니어서 그것(무리당 주인 하나)만 강제한다.
     주인이 통로라는 것은 배치가 지킨다(사용량 조회의 트리 손잡이도 자기 무리의 주인이다).
     pty · stdio · codex 통로의 #[cfg(windows)] job_handle 칸이 사라진다.
     중립 손잡이 ProcessGroup · RetiringSignal 은 agent 에 cfg 없는 어댑터로 남는다(agent/src/transport/process_group.rs).
     이 모양은 ADR-0266 결정 6(「한 덩이로」 · 「새 손잡이를 따로 짓지 않는다」)의 개정이다 → ADR-0275(§8).

④ 단위 아홉(U0~U8)로 나눠 옮긴다 — 어느 단위 뒤에 멈춰도 워크스페이스는 빌드 · 회귀 초록이다.
     U3(무리 손잡이 · 통로) 하나가 kill 인과를 지난다 → /qa full(GUI 포함).
```

**지금 하지 않는 것:** 원자적 쓰기 통일(storage P3 착지 뒤 — §5 U-W) · Windows 밖 구현 만들기(ADR-0266 결정 7) · 통로가 「띄운 뒤 Job 에 넣는」 틈 메우기(동작 변경 — §1).

---

## 1. 목표 · 비목표

### 하는 것

- OS 에 따라 달라지는 **운영** 코드를 전부 새 crate 로 옮긴다(ADR-0266 결정 1 — 판정 기준 = OS 의존 여부, 결정 2).
- 부르는 쪽에 샌 분기(통로들의 `#[cfg(windows)]` Job 칸)를 없앤다.
- Windows 밖 동작은 **지금 그대로** 옮긴다(§3-6 목록). 자리채움을 실물로 바꾸지 않는다(ADR-0266 결정 7).
- 게이트를 세운다(§4).

### 안 하는 것

- **원자적 쓰기(`write_atomic`) 여러 벌 통일** — storage P3 가 master 에 착지한 뒤(사용자 「알아서」 2026-10-03 → 메인 권고안 · 메모 §10 진행 방식). 그때까지 셸 `src-tauri/src/fsutil.rs` 의 `cfg!(windows)` 가 **알려진 운영 예외 하나**로 남는다(§4-4).
- **도메인 지식 이동** — `.claude` 경로 · codex 홈 하위 폴더 · 락 파일 이름 = 스레드 id · claude 의 `taskkill` 훅 모양 판정(`leftover.rs` `taskkill_names`)은 그대로 둔다(ADR-0266 결정 3).
- **통로의 「띄운 뒤 넣기」 틈** — `transport::pty` · `transport::stdio` · codex 통로는 자식을 띄운 **뒤** Job 에 넣어, 그 사이 손자가 빠질 수 있다(`agent/src/platform/windows.rs` 머리 주석이 자인). 사용량 조회만 멈춘 채 띄워 넣는다. 이번엔 위치만 옮기고 동작은 그대로 둔다.
- **discovery 나누기(2-2) · net 걷기(3-3)** — 여기서는 그 crate 들이 platform 을 부르게만 바꾼다.

---

## 2. 현황 실측 — OS 분기 목록

**명령(실측 `0ef6292`):** `rg -l 'cfg!?\(windows\)|cfg\(not\(windows\)\)|cfg\(target_os|cfg!\(target_os|cfg\(unix\)|cfg!\(unix\)|target_family' crates src-tauri` → 45 파일(매니페스트 4 포함). 그 뒤 파일마다 `rg -n '^\s*#\[cfg\(test\)\]'` 로 시험 모듈 시작 줄을 잡고 각 분기를 운영 · 시험으로 갈랐다. ★ADR-0266 의 명령(`cfg!?\(windows\)|cfg\(not\(windows\)\)`)은 `#[cfg(unix)]` · `cfg_attr(not(windows), …)` 를 못 문다★ — 해당 파일이 다른 줄로 걸려 이번엔 빠진 게 없었을 뿐이다. 그리고 `cfg` 없이 런타임에 OS 를 가르는 자리는 어느 정규식에도 안 걸려 따로 찾았다(`rg 'USERPROFILE|"HOME"|creation_flags|taskkill|share_mode'`) — 하나 나왔고(1판의 R7), 하네스 전용이라 남긴다(§2-3).

### 2-1. 옮길 것 — 운영 코드

| id | 자리(실측 `0ef6292`) | 무엇 | 필요한 `windows` feature | 단위 |
|---|---|---|---|---|
| P1 | `crates/engram-dashboard-base/src/platform.rs` 1-280 | PID 생존 · 시작 시각 3갈래(`ProcessStart`) · 프로세스 표 · 자식 PID | `Win32_Foundation` · `Win32_System_Threading` · `Win32_System_Diagnostics_ToolHelp` | U1 |
| R1 | `agent/src/manager.rs:77-84` `default_shell` | 기본 셸(`cmd.exe` / `bash`) | 없음(std) | U2 |
| R2 | `agent/src/backend/mod.rs:46-60` `console_command` | CLI 를 `cmd.exe /c` 로 감싸기 | 없음 | U2 |
| R3 | `agent/src/backend/mod.rs:138` | `PATH` 키 대소문자 무시 | 없음 | U2 |
| R4 | `agent/src/backend/claude/mod.rs:1829-1836` `claude_home` | 홈 디렉터리(`USERPROFILE` / `HOME`) | 없음 | U2 |
| R5 | `agent/src/backend/codex/thread_lock.rs:230-238` `user_home` | 홈 디렉터리 — R4 의 둘째 사본 | 없음 | U2 |
| R6 | `daemon/src/lib.rs:109` `locate_send_exe` · `discovery/src/lib.rs:987` `locate_daemon_exe` | 실행 파일 이름에 `.exe` 붙이기 | 없음 | U2 |
| P4 | `agent/src/platform/file_holders.rs` | 이 파일을 연 프로세스(Restart Manager) · `RmSession` `Drop` 경고 로그 | `Win32_System_RestartManager` · `Win32_Foundation` | U4 |
| P5 | `agent/src/platform/process_tree.rs` | 뿌리 PID 아래 신원 목록(P1 두 함수를 조립 — 자체 `cfg` 는 시험뿐) | 없음(P1 위) | U4 |
| P2 | `agent/src/platform/windows.rs` 1-713 | Job Object 래퍼 · 멤버 명단 · 붙들기 · 가입 알림 포트 · `resume_suspended_process` · `LEFTOVER_EXIT_CODE` | `Win32_System_JobObjects` · `Win32_Security` · `Win32_Foundation` · `Win32_System_Threading` · `Win32_System_IO` · `Wdk_System_Threading` · `Win32_System_Diagnostics_ToolHelp` | U3 |
| P3 | `agent/src/platform/process_group.rs` 1-313 | ADR-0262 중립 손잡이 `ProcessGroup` 의 OS 갈래 | (P2 위) | U3 |
| P6 | `agent/src/platform/mod.rs:18-24` | `#[cfg(windows)]` 재수출 | — | U4 = `file_holders` · `process_tree` 의 mod 선언 둘 · U3 = 나머지와 파일 · 폴더 자체(§5-1) |
| U | `agent/src/usage/process.rs` 19-476 | 기동 플래그(창 없음 + 멈춘 채 / POSIX 새 그룹) · 트리 kill(Job / `kill -s KILL -- -<pgid>`) · 「프로그램 없음」 판정(cmd 9009 + `PATH`·`PATHEXT` 찾기 · 원격 경로 판정 · 찾기 명단 · 마감) · 자식 env 대소문자 무시 | 없음(std) + P2 의 `resume_suspended_process` | U5 |
| N | `net/src/instance.rs:166-228 · 289 · 306-315` | 공유 모드 「읽기만」 열기 · 원시 오류 32 · 5 · Windows 밖 `Unsupported` · `cfg_attr(not(windows), allow(dead_code))` 4줄 | 없음(std `OpenOptionsExt::share_mode`) | U6 |
| D1 | `discovery/src/lib.rs:911-931` `TaskKiller::kill` | `taskkill /PID … /F /T` · Windows 밖은 오류 | 없음(std) | U7 |
| D2 | `discovery/src/lib.rs:1060-1323` | COM 초기화 분류 · `ComGuard` · `wmi_spawn` · `wmi_create_raw` · `read_u32_prop` · `wmi_err` | `Win32_System_Com` · `Win32_System_Wmi` · `Win32_System_Rpc` · `Win32_System_Variant` (+ 지금 discovery 가 함께 적어 둔 `Win32_System_Threading` · `Win32_Foundation` 은 이 코드에 필요한지 **미검** — U7 에서 빼 보고 컴파일로 판정) | U7 |

**「창 없이 띄우기」(`CREATE_NO_WINDOW`)는 ADR-0266 의 OS 규칙 목록에 없던 넷째 규칙이다** — `agent/src/transport/stdio.rs:119-124` · `agent/src/backend/codex/transport.rs:1445-1450` · `agent/src/usage/process.rs:75-84` 에 같은 상수가 세 벌 있다(실측). 각 파일이 옮겨지는 단위(U3 · U5)에서 platform 함수 하나로 바꾼다.

### 2-2. 부르는 쪽에 샌 분기 — 없앤다

| 자리 | 지금 | U3 뒤 |
|---|---|---|
| `agent/src/transport/pty.rs:21 · 59-60 · 91-98 · 118-119 · 428-431` | `#[cfg(windows)] job_handle: JobObjectHandle` · 열 때 만들고 넣기 · `shutdown` 4번째 줄 `terminate(1)` | `group: GroupOwner`(가안) — 모든 OS 에서 같은 줄 |
| `agent/src/transport/stdio.rs:36-38 · 88-89 · 136-143 · 156-157 · 166-178 · 489-492` | 같은 칸 + `process_group()` 의 `cfg` 두 갈래 | 같은 칸 · `process_group()` 는 `cfg` 없는 한 식 — `self.group.downgrade()` 의 `GroupRef` 와 `RetiringSignal::of(&self.retiring)` 를 agent 어댑터 `ProcessGroup::new` 로 묶는다(D3 = B · §3-4 「약한 손잡이 만들기」). Windows 밖은 `downgrade` 가 `None` 이라 지금 답과 같다. ★물러남 표시를 빼고 `downgrade()` 만 내주면 안 된다★ — 잔여물 정리의 듣는 스레드(`leftover.rs:759`)와 끝내기 앞 재확인(`recheck` · `:2345`)이 멈춤 조건을 잃는다 |
| `agent/src/backend/codex/transport.rs:183-184 · 1395-1396 · 1464-1471 · 1494-1495 · 4239-4242` | 같은 칸(`Arc` — 약한 손잡이를 내주지는 않는다) | 같은 칸 |

### 2-3. 남길 것

- **시험 분기뿐인 파일**(분기가 전부 그 파일 `#[cfg(test)]` 아래 · `#[cfg(all(test, windows))]` 모듈 · `tests/`): agent `backend/claude/{leftover.rs, mod.rs, usage_probe.rs}` · `backend/codex/{mod.rs, transport.rs(6198~)}` · `backend/codex/thread_lock.rs:1222` · `backend/mod.rs:1595` · `manager.rs:3063~` · `session.rs:1213` · `usage/scratch.rs` · `transport/pty.rs:682 · 783` · `transport/stdio.rs:559~` · base `logging/mod.rs:618` · daemon `messaging_host.rs` · net `portfile.rs` · net `instance.rs:319 · 333`(시험 모듈) · discovery `lib.rs:1553 · 2754 · 2828` · `tests/` 아래 agent 8 · daemon 1 · discovery 1 파일. ★`backend/claude/mod.rs` 의 첫 `#[cfg(test)]`(1059)는 모듈이 아니라 낱개 항목이다★ — 「첫 `#[cfg(test)]` 뒤 = 시험」으로 자르면 운영 분기 R4(1829)를 시험으로 오판한다. 이 표는 시험 **모듈** 시작 줄(1967)로 갈랐다.
- **하네스 bin · 예제 · 하네스 모듈**(ADR-0266 「근거」 — 운영 빌드에 안 들어간다): daemon `src/bin/roundtrip_smoke.rs:905` · `src/bin/saturation_pilot.rs:1657`(둘 다 `required-features = ["test-harness"]`) · agent `examples/spike.rs` · `examples/spike_breakaway.rs` · ★daemon `src/experiment/transcript.rs:69-70` `claude_projects_dir`(1판의 R7 — 홈 디렉터리 셋째 사본, `cfg` 없이 `USERPROFILE` → `HOME` 순으로 런타임에 고른다)★. 1판은 이것을 운영으로 셌으나 `experiment` 모듈 전체가 `#[cfg(feature = "test-harness")]` 뒤다(`daemon/src/lib.rs:17-18` · 부르는 곳 = `saturation_pilot` 하나). 하네스는 남기는 규칙이므로 옮기지 않는다 — 옮기면 R4 · R5 뜻(Windows = `USERPROFILE` 만)을 따라 Windows 에서 `HOME` 폴백이 사라진다.
- **바이너리 속성 · 빌드 스크립트:** `daemon/src/main.rs:9` · `src-tauri/src/main.rs:2` 의 `windows_subsystem`(crate 수준 링커 속성 — 다른 crate 로 못 옮긴다) · `src-tauri/build.rs`(호스트 ≠ 타깃이라 `cfg` 가 아니라 `CARGO_CFG_TARGET_OS` 를 런타임에 읽는다 — ADR-0174).
- **주석뿐:** net `src/lib.rs:44 · 91`(`cfg(unix)` 를 글로 인용).

### 2-4. 미룬 것

- **`src-tauri/src/fsutil.rs:107`** — `write_atomic` 의 rename 재시도가 Windows 공유 · 잠금 위반(원시 32 · 33)을 본다. storage P3(`origin/v0.3.3/feat/storage` — master 미착지, 실측 `git merge-base --is-ancestor` 거짓 · 2026-10-03)가 이 파일을 크게 바꾼다(`write_atomic_unless` · `copy_atomic` · `copy_aside` · `retry_denied` 신설 · 같은 `cfg!(windows)` 판정은 `is_lock_contention` 으로 남는다 — `git show origin/v0.3.3/feat/storage:src-tauri/src/fsutil.rs`). → §5 U-W.

### 2-5. 기록과 어긋난 사실

| 기록 | 실제(실측 `0ef6292`) |
|---|---|
| 메모 §1 · ADR-0230 「영향」: `daemon/src/lib.rs:96,146`(앱 · CLI 두 곳) | 운영 분기는 `daemon/src/lib.rs:109` **하나**(CLI)다. 앱 이름을 가르던 자리는 없다. |
| 메모 §1: `discovery/src/lib.rs:1119-1320` · `:981` | `1060-1323`(COM 분류부터) · `987`. |
| 메모 §1: `agent/src/manager.rs:75` · `backend/claude/mod.rs:1012` · `backend/mod.rs:44,136` · `codex/transport.rs:177` | `77` · `1829` · `46-60, 138` · `183`. |
| 메모 §1: 「셸(`src-tauri/src`)에는 OS 분기가 없다」 | `src-tauri/src/fsutil.rs:107` 운영 분기가 있다(ADR-0266 재실측은 이미 반영 — 메모만 낡았다). |
| ADR-0266 「맥락」 · 재실측 목록 | 「창 없이 띄우기」 세 벌(§2-1)이 목록에 없다. (홈 디렉터리 셋째 사본 `daemon/src/experiment/transcript.rs:69` 도 없지만 하네스 전용이라 ADR 「근거」의 하네스 규칙과 맞는다 — §2-3.) |
| ADR-0266 결정 6 「`platform/process_group.rs:118-119`」 | `Weak<JobObjectHandle>` 칸은 `117-118`(사소). |
| CLAUDE.md 「의존성」이 함의하는 「agent 의 `windows` feature 가 platform 으로 옮겨 간다」 | agent `examples/spike*.rs` 가 `windows` crate 를 **직접** 쓴다(`rg -l 'windows::(Win32|Wdk|core)' crates`) — `cargo test` 는 예제를 컴파일하므로 agent 는 `windows` 를 **dev-dependency 로** 남겨야 한다(§3-9). `spike.rs` 는 `portable-pty` 도 써서 platform 으로 못 옮긴다. |
| ADR-0266 결정 6 「Job Object 래퍼와 `ProcessGroup` 을 한 덩이로 옮긴다」 | 두 파일만으로 끝나지 않는다 — leftover 의 실프로세스 시험이 `crate::platform::process_group::tests::{new_group, open_gate, spawn_gated_cmd, wait_until, is_ping}` 와 `#[cfg(test)]` 생성자 `ProcessGroup::detached` 를 쓴다(`leftover.rs:4221 · 6571` · `claude/mod.rs:6357-6359`). crate 를 넘으면 `#[cfg(test)]` 가 안 보이므로 OS 수준 도우미 넷(`spawn_gated_cmd` · `open_gate` · `wait_until` · `is_ping` — 지금 `windows.rs:729-758`)은 기능 플래그로 연다. `new_group` 과 `detached` 는 agent 쪽 `ProcessGroup` · `RetiringSignal` 을 만드는 것이라 agent 어댑터 시험 지원에 남는다(D3 = B · §3-8). |
| `windows.rs` 를 통째로 옮긴다(ADR-0262 「[구현] 고른 것」 — 사용자 결정 2026-09-29 「windows crate 와 io 만 · 통째로 옮길 수 있게」) | 그 파일에 claude 잔여물 정리의 **도메인 상수** `LEFTOVER_EXIT_CODE = 0x7440`(`windows.rs:53`)가 있고 `terminate_raw` 가 그것을 박아 쓴다(`:405-406`). 통째로 옮기면 도메인 값이 platform 에 들어간다(ADR-0266 결정 3) → 인자로 받게 바꾸고 상수는 agent 잔여물 정리 쪽으로 돌린다(§3-4 · ADR-0275 개정 — §8). |

---

## 3. crate 설계

### 3-1. 이름 · 위치 · 매니페스트

- **이름 = `engram-dashboard-platform`(결정 D2 · §7)**, 위치 `crates/engram-dashboard-platform/`. 접두 `engram-dashboard-` 는 필수다 — 상한 게이트가 워크스페이스 멤버를 그 이름 접두로 센다(ADR-0175 영향 · ADR-0151 「개명 함정」).
- 루트 `Cargo.toml` `members` 에 더한다.
- 매니페스트(제안):

```toml
[package]
name = "engram-dashboard-platform"

# ★워크스페이스 crate 를 여기 적지 않는다★ — base 도 아니다(ADR-0266 「거부한 대안」 둘째 · ADR-0268).
[dependencies]
tracing = "0.1"          # Drop 경로처럼 돌려줄 곳이 없는 실패만 직접 찍는다(ADR-0266 결정 9)

[features]
test-support = []        # 실프로세스 시험 도우미를 부르는 쪽 시험에 연다 — 모듈은 cfg(any(test, feature = "test-support"))(§3-8)
                         # 기능은 이것 하나뿐 — windows 바인딩을 기능별 feature 로 쪼개지 않는다(결정 D1 = a)

[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [ …§3-9 의 합집합… ] }
```

### 3-2. 모듈 배치 (제안)

| 모듈 | 담는 것 | 출처 |
|---|---|---|
| `process` | PID 판정 일체 · `subtree`(뿌리 아래 신원) · `kill_tree(pid)`(`taskkill /F /T`) | P1 · P5 · D1 |
| `group` (+ 비공개 `group/windows.rs`) | 무리 주인 · 무리 손잡이 · 붙든 멤버 · 가입 알림 포트 · 값 타입 · `resume_suspended_process` | P2 · P3 의 OS 갈래 |
| `file_holders` | 이 파일을 연 프로세스 | P4 |
| `spawn` (+ 비공개 `spawn/wmi.rs`) | 창 없이 띄우기 · 트리 뿌리 준비(멈춘 채 / 새 그룹) · 트리 손잡이 · 「프로그램 없음」 판정 · Job 밖에서 띄우기(WMI) | §2-1 셋째 규칙 · U · D2 |
| `shell` | `default_shell` · `console_command` | R1 · R2 |
| `env` | `home_dir` · `exe_file_name` · `env_key_eq` | R3 · R4 · R5 · R6 |
| `fs` | 「읽기만 공유」 열기 · 공유 위반 · 접근 거부 판정 (+ U-W 때 `write_atomic`) | N |
| `testing` (`test-support` 뒤) | 실프로세스 시험 도우미 | P2 · P3 의 시험 모듈 |

`group/windows.rs` 는 지금 `windows.rs` 를 거의 그대로 둔다(「`windows` crate 와 std 만」 잎 규칙 유지 — ADR-0262). 이름 충돌 셋을 피하려고 고른 배치다: ① agent 의 ADR-0262 `ProcessGroup`(약한 중립 손잡이) ② 사용량 조회의 비공개 `ProcessTree`(강한 트리 kill 손잡이 — `usage/process.rs:437 · 471`) ③ agent 의 `platform::process_tree` 모듈(신원 목록 질의). 셋 다 「프로세스 무리/트리」인데 서로 다른 물건이라, platform 쪽은 `group::GroupOwner` · `group::GroupRef` · `spawn::TreeRoot` · `process::subtree` 로 갈라 부른다(가안). std 의 POSIX `CommandExt::process_group`(pgid)과도 겹치지 않는다.

### 3-3. 공개 인터페이스 (가안 — 함수 + 핸들 타입, 컴파일 시점 분기)

```rust
// process — P1 그대로 + P5 · D1
pub enum ProcessStart { Known(u64), Gone, Unknown }
pub fn process_start(pid: u32) -> ProcessStart;
pub fn process_creation_time(pid: u32) -> Option<u64>;
pub fn current_process_start_time() -> Option<u64>;
pub fn pid_alive(pid: u32) -> bool;
pub fn pid_alive_with_start_time(pid: u32, expected_start: u64) -> bool;
pub fn process_parent_table() -> Option<Vec<(u32, u32)>>;
pub fn child_pids(parent: u32) -> Vec<u32>;
pub struct ProcessIdentity { pub pid: u32, pub start_time: u64 }
pub fn subtree(root: u32, root_start_time: u64) -> Vec<ProcessIdentity>;
pub fn kill_tree(pid: u32) -> io::Result<()>;            // Windows 밖 = Err(Unsupported)

// shell · env — std 만
pub fn default_shell() -> &'static str;
pub fn console_command(program: &str, args: Vec<String>) -> (String, Vec<String>);
pub fn home_dir() -> Option<PathBuf>;                       // 지금 의미 그대로: USERPROFILE / HOME 환경변수
pub fn exe_file_name(stem: &str) -> String;                 // std::env::consts::EXE_SUFFIX 로 — 이 함수 안에도 cfg 없음
pub fn env_key_eq(a: &str, b: &str) -> bool;                // Windows = ASCII 대소문자 무시

// fs
pub fn open_deny_write(path: &Path) -> io::Result<File>;    // 읽기+쓰기+만들기 · 남에게는 읽기만 공유 · Windows 밖 = Err(Unsupported)
pub fn is_sharing_violation(e: &io::Error) -> bool;         // Windows 원시 32
pub fn is_access_denied(e: &io::Error) -> bool;             // Windows 원시 5

// spawn
pub fn hide_console_window(cmd: &mut Command);              // CREATE_NO_WINDOW · Windows 밖 무동작
pub fn prepare_tree_root(cmd: &mut Command);                // Windows = 창 없음 + 멈춘 채 · POSIX = 새 그룹
pub struct TreeRoot { … }                                   // attach(&Child) · start(&Child) · kill(&self, &mut Child) -> io::Result<()> · members_gone()
pub struct MissingProgramCheck { … }                        // new(program, args, env_set, env_remove, cwd) · judge(code, deadline) -> bool
pub enum DetachedSpawnError { Refused { rv: u32 }, Io(io::Error) }
pub fn spawn_outside_job(exe: &Path, console: bool) -> Result<(), DetachedSpawnError>;   // WMI · Windows 밖 = Io(Unsupported)
```

- **도메인 타입을 받지 않는다** — 사용량 조회의 `ProbeCommand` 대신 원시 값(프로그램 · 인자 · env 설정/제거 · 작업 폴더)을 받는다. discovery 의 `DiscoveryError` 대신 `io::Error` / `DetachedSpawnError` 를 돌려주고 discovery 가 제 오류로 옮긴다(`SpawnFailed { rv }` 는 `Refused { rv }` 에서).
- **부르는 쪽의 시험 seam 은 그대로 부르는 쪽에 있다**(ADR-0266 결정 5): discovery `PidLiveness` · `Spawner` · `ProcessKiller`(`lib.rs:417 · 426 · 677` — 실물 구현 `RealLiveness` · `WmiSpawner` · `TaskKiller` 만 platform 함수를 부르게 바뀐다) · thread_lock `LockHolderProbe`(`:49`) · leftover `Group`(`:522`) · 사용량 `ProbeSpawner`.
- `default_shell` 은 지금 `engram_dashboard_agent::manager::default_shell` 이라는 공개 경로로 데몬 운영 코드(`daemon/src/connection_core.rs:29`)와 시험 여럿이 부른다. **재수출을 남기지 않고 부르는 곳을 platform 경로로 바꾼다**(제안 — 같은 것에 이름 둘을 두지 않는다).
- `home_dir` 의 뜻은 R4 · R5 그대로다(Windows = `USERPROFILE` 만). 하네스의 셋째 사본(`experiment/transcript.rs` — 모든 OS 에서 `USERPROFILE` → `HOME`)은 이 함수로 바꾸지 않는다(§2-3).
- `std::env::home_dir` 로 바꾸지 않는다 — Windows 에서 환경변수가 없으면 OS API 로 넘어가 지금 뜻과 달라진다.

### 3-4. 프로세스 그룹 손잡이 (결정 D3 = B · §7)

**지금 모양(실측):**

- 강한 주인 = `JobObjectHandle`(`windows.rs:140` — `pub`). pty 는 값으로, stdio · codex 는 `Arc` 로 쥐고, stdio 만 `Arc::downgrade` 로 약한 손잡이를 내준다(`stdio.rs:166-178`).
- 약한 중립 손잡이 = `ProcessGroup { job: Weak<JobObjectHandle>, retiring: RetiringSignal }`(`process_group.rs:116-120`) — 명단 · 붙들기 · 가입 알림 · 「사라졌나」. Windows 밖은 명단 빔 · 못 붙듦 · 포트 `Unsupported` · 늘 「사라짐」.
- 시험 seam: 붙든 멤버 `Pinned` · 포트 `Births` 트레이트(`process_group.rs:79 · 99`)는 platform 에, 무리 트레이트 `Group` 은 부르는 쪽 leftover 에 있다(`leftover.rs:522`). 가짜 `FakePin` · `FakePort` · `FakeGroup` · `GateGroup` · `IdlePort` 는 전부 leftover 시험에 있다.

**고른 모양 — B: OS 층만 platform, 중립 손잡이는 agent 어댑터**(결정 2026-10-04 · §7 D3):

| 자리 | 무엇 |
|---|---|
| platform `group`(+ 비공개 `group/windows.rs`) | `windows.rs`(잎 — 도메인 상수 `LEFTOVER_EXIT_CODE` 만 뺀다) + 그 겉의 강한 주인 `GroupOwner` · 약한 손잡이 `GroupRef` · 구체 타입 `PinnedMember` · `BirthPort` · `MemberFacts` · `PortOutcome` · `MemberOutcome`(이름 전부 가안). **공개 API 에 트레이트 객체가 없다** — 구체 타입뿐이다 |
| agent `transport/process_group.rs` | 지금 `platform/process_group.rs` 가 `cfg` 없는 어댑터로 옮겨 온다 — `ProcessGroup { group: GroupRef, retiring }` · `RetiringSignal` · 두 트레이트(`Pinned` · `Births`) · `impl Pinned for` 붙든 멤버 · `impl Births for BirthPort` · 값 옮겨 싣기(트레이트 객체는 이 안에서만 만든다). 통로 밑에 두는 까닭 = 그 손잡이를 내주는 것이 통로(`StdioTransport::process_group` — `stdio.rs:166`)이고 `RetiringSignal` 이 읽는 물러남 칸도 통로 소유다(CLAUDE.md 「소유권 분할」). **ADR-0262 · CLAUDE.md 「핵심 불변식」의 이름(`ProcessGroup` · `RetiringSignal`)은 agent 에 그대로 남고 경로만 바뀐다** |
| agent `platform/` 폴더 | 사라진다 — `lib.rs` 의 `pub mod platform;` 째(바깥 crate 의 `engram_dashboard_agent::platform` 사용 0 — 실측 rg). agent 에 `platform` 이름의 모듈을 남기면 platform crate 와 이름이 겹쳐 「OS 코드가 아직 여기 있다」로 읽힌다 |

- **강한 주인 `GroupOwner`(가안)** — `new() -> io::Result<Self>` · `adopt(pid)` · `terminate(exit_code)` · `active_processes()` · `downgrade(&self) -> Option<GroupRef>`. **`Clone` 이 아니다** — 타입이 강제하는 것은 「한 무리의 강한 주인은 하나」까지다. 「그 주인이 통로다」(ADR-0266 결정 6 · ADR-0262 영향 「소유권」)는 타입이 아니라 배치가 지킨다 — 통로 셋 말고 사용량 조회의 트리 손잡이(지금 `usage/process.rs:438` 의 `job` 칸 · U5 뒤 `TreeRoot`)도 자기 무리의 주인을 하나 쥔다. Windows 밖은 아무것도 안 하는 주인(`new` · `adopt` · `terminate` = `Ok`, `active_processes` = `Unsupported`, `downgrade` = `None` — 지금 `stdio.rs` 의 `None` 갈래와 같은 답).
- **`GroupRef`(약한)** = 지금 `ProcessGroup` 의 OS 갈래 — `member_pids` · `pin_member(pid, kill)` · `watch_births(start)` · `unwatch_births` · `is_gone` · 시험용 `GroupRef::gone()`. 답의 뜻은 `process_group.rs` 의 두 갈래 그대로다(사라짐 = `Ok(빈 목록)` · `Ok(None)` · `Err(GROUP_GONE)`).
- **약한 손잡이 만들기**(stdio `process_group()` — 지금 `stdio.rs:166-178`) — `GroupOwner::downgrade()` 는 무리 손잡이만 낸다. platform 은 물러남 표시를 모른다. 통로가 agent 에서 묶는다: `self.group.downgrade().map(|g| ProcessGroup::new(g, RetiringSignal::of(&self.retiring)))`(어댑터 `ProcessGroup::new(GroupRef, RetiringSignal)` — `cfg` 없음). ★`downgrade()` 하나로 줄이면 표시가 빠진다★(§2-2).
- **시험 seam** — `detached` 는 agent 에 그대로다(`GroupRef::gone()` 으로 만든다 — 그래서 `gone()` 은 platform 에서 `cfg(any(test, feature = "test-support"))` 뒤에 연다). `new_group` 도 agent 어댑터 시험 지원에 남는다 — platform 은 agent 의 `ProcessGroup` · `RetiringSignal` 을 못 만든다(§3-8).
- **`terminate_raw` 는 끝 코드를 인자로 받는다** — `LEFTOVER_EXIT_CODE` 는 agent(잔여물 정리 쪽)로 돌아간다(§2-5 · ADR-0266 결정 3).
- **품** — 어댑터 약 100줄(어림 — §9)을 agent 에 남기고 `cfg` 만 걷는다.
- ★**이 모양은 ADR-0266 결정 6 의 개정이다 → ADR-0275(§8)**★ — 결정 6 은 `ProcessGroup` 과 Job Object 래퍼를 「한 덩이로」 옮기고 「새 손잡이를 따로 짓지 않는다」고 적었다. B 는 OS 층만 옮기고 `GroupOwner` · `GroupRef` · 구체 타입 다섯을 새로 짓는다. 결정 6 의 실질(강한 주인 = 통로 · 밖으로는 약한 손잡이 · 읽기 전용 표시 · 포트만)은 그대로다. `GroupOwner` 를 「`JobObjectHandle` 의 겉만 바꾼 것」(같은 OS 객체 · 같은 강한 주인 · Windows 밖 무동작과 `downgrade` 만 더함)으로 읽어 개정 없이 갈 수도 있으나, 공개 타입의 이름 · 모양 · 내부(`Arc` 를 안으로 넣는다)가 바뀌므로 이 TRD 는 그 해석에 기대지 않는다.

**왜 A 가 아닌가** — A 는 결정 6 의 「한 덩이로」 문구대로 `windows.rs` + `process_group.rs` 전부(`ProcessGroup` · `RetiringSignal` · `Pinned` · `Births` · `ProcessFacts` · `PortEvent` · `MemberKill` · `GROUP_GONE`)를 platform 으로 옮기는 안이었다. 품은 가장 작지만(경로 바꾸기 위주) platform 공개 API 에 통로 개념 `RetiringSignal`(통로의 물러남 칸 읽기 — OS 와 무관) · 시험 seam 트레이트 · 트레이트 객체(`Box<dyn Pinned>` · `Arc<dyn Births>`)가 들어가고, `ProcessGroup::detached` 를 `test-support` 공개 생성자로 열어야 한다 — ADR-0266 결정 2(판정 = OS 의존)와 결정 5(가짜용 트레이트는 **쓰는 쪽이**)에 어긋난다. 그리고 A 도 `GroupOwner` 를 새로 지어야 해서 「새 손잡이를 따로 짓지 않는다」 문구는 어느 쪽이든 고쳐야 했다 — 문구를 지키는 이득이 반쪽뿐이었다.

**공통 불변식(코더 지시서에 박는다):**

- **kill 인과(ADR-0001) 순서 그대로** — `shutdown()` 의 `child.kill + wait` → `group.terminate(1)` → master drop. 지금 `#[cfg(windows)]` 블록 자리에 같은 줄이 모든 OS 에서 선다(Windows 밖 = 무동작이라 지금과 같다).
- **drop 순서 그대로** — 세 통로 모두 Job 칸이 구조체의 **마지막 필드**다(`pty.rs:60` · `stdio.rs:89` · `codex/transport.rs:1396` — 실측). 새 칸도 마지막에 둔다. 통로가 drop 될 때 Job 핸들이 닫혀 `KILL_ON_JOB_CLOSE` 가 도는 시점이 앞당겨지거나 늦어지지 않게. ★이 배치를 재는 시험은 없다★(소스 문자열 시험 · 주석 어디에도 — 실측 `rg '마지막 필드|마지막 칸|last field' crates/engram-dashboard-agent/src` → 0) — 리뷰가 지킨다.
- **잔여물 정리 락 규칙(ADR-0262 결정 8 · CLAUDE.md 「핵심 불변식」 락 순서)** — 문 자물쇠를 쥔 채 부르는 OS 호출은 끝내기 확정의 `TerminateProcess` 하나뿐이다. 그래서 platform 의 `terminate_raw` · `classify` · `is_gone` 은 **로그도 락도 안 쓴다** — platform 이 `tracing` 을 쓸 수 있게 된 것(ADR-0266 결정 9)이 이 함수들에 번지면 안 된다. `terminate_raw` 는 OS 호출 정확히 하나로 남는다.
- **`GroupRef` 는 Job 을 붙들지 않는다** — 부를 때만 올리고(`is_gone` 은 올리지도 않는다), 가장 긴 창은 `watch_births` 가 `start` 를 부르는 동안이다(`process_group.rs` 머리 주석 그대로).

### 3-5. 오류 타입

- 기본은 `std::io::Result` 다(지금 모든 대상이 이미 그렇다 · base 와 같은 규칙 — ADR-0269 결정 6).
- 예외는 WMI 띄우기 하나 — 거절 코드(`ReturnValue`)를 보존해야 discovery 의 `SpawnFailed { rv }` 와 플래그 진단 시험이 산다 → `DetachedSpawnError { Refused { rv }, Io(io::Error) }`(가안). 원시 호출(`wmi_create_raw` — RV 를 오류로 올리지 않는다)은 진단 시험 전용이라 `test-support` 뒤 공개로 둔다.
- 「이 OS 에는 수단이 없다」는 `io::ErrorKind::Unsupported`, 「무리가 이미 사라졌다」는 지금 상수 `GROUP_GONE = NotConnected` 그대로.

### 3-6. Windows 밖 동작 — 지금 그대로 옮긴다

| 함수 · 타입 | Windows 밖 답(지금 = 옮긴 뒤) | 지금 자리 |
|---|---|---|
| `process_start` / `process_creation_time` / `current_process_start_time` | `Unknown` / `None` / `None` | base `platform.rs:254-268` |
| `pid_alive` / `pid_alive_with_start_time` | `pid != 0`(시작 시각 무시) | `:270-279` |
| `process_parent_table` / `child_pids` / `subtree` | `None` / 빈 목록 / 빈 목록 | `:260` |
| `GroupOwner` 일체 · `downgrade` | 무동작 `Ok` · `active_processes` = `Unsupported` · `None` | 통로의 `cfg` 부재 · `stdio.rs:174-177` |
| `GroupRef` | 명단 빔 · 못 붙듦 · 포트 `Unsupported` · 늘 사라짐 | `process_group.rs:151 · 172 · 203 · 219 · 236` |
| `file_holders::holders_of` | `Ok(빈 목록)` | `file_holders.rs:102-105` |
| `kill_tree` | `Unsupported`(discovery 가 지금 문구 「`daemon_stop` 은 Windows 전용」으로 옮긴다) | `discovery/src/lib.rs:928-931` |
| `spawn_outside_job` | `Io(Unsupported)` | `:1319-1322` |
| `open_deny_write` / 오류 판정 둘 | `Unsupported` / 늘 거짓 | net `instance.rs:223-226 · 308-315` |
| `hide_console_window` | 무동작 | 세 통로 |
| `prepare_tree_root` · `TreeRoot` | 새 프로세스 그룹 · `kill -s KILL -- -<pgid>` · `members_gone` = 참 | `usage/process.rs:85-90 · 470-508` — ★이 갈래는 이 저장소에서 컴파일된 적이 없다★(그 파일 머리 주석) · 옮겨도 여전히 안 된다 |
| `MissingProgramCheck::judge` | 늘 거짓(POSIX 는 기동이 `NotFound` 로 실패) | `:152-160` |
| `default_shell` / `console_command` / `home_dir` / `exe_file_name` / `env_key_eq` | `bash` / 그대로 실행 / `HOME` / 붙이지 않음 / 정확히 같음 | R1~R6 |

### 3-7. 로그

- platform 은 `tracing` facade 를 직접 부른다 — base 를 거치지 않는다(ADR-0266 결정 9 · ADR-0268 결정 1).
- **원칙(제안): 결과는 값으로 돌려주고 부르는 쪽이 찍는다. 돌려줄 길이 없는 경로(`Drop`)만 platform 이 찍는다.** 그래서 `RmSession` 의 `Drop` 경고는 그대로 옮기고(`file_holders.rs:142` — ADR-0266 결정 9 가 지명한 그것), 사용량 조회의 「그룹 kill 실패 — 자식만 끊는다」(`usage/process.rs:455 · 497`)는 `TreeRoot::kill` 이 `io::Result` 로 돌려주고 agent 가 지금 문구(「사용량 조회: …」)로 찍는다 — **도메인 낱말을 platform 로그에 가져가지 않는다.** `MissingProgramCheck` 의 debug 줄과 「막힌 찾기」 1회 경고(`STUCK_LOOKUP_WARNED`)도 같은 원칙으로 판정 결과(있음 · 없음 · 모름(막힘))를 값으로 내고 찍기는 agent 가 한다. 찾기 명단(`LookupGate`)의 독 처리는 U5 행(§5-1) — 독도 「모름(막힘)」 값으로 나가고 platform 은 찍지 않는다.
- §3-4 의 락 규칙 — 잠금 안에서 불리는 함수는 찍지 않는다.

### 3-8. 시험 지원 기능 `test-support`

- crate 를 넘으면 `#[cfg(test)]` 는 부르는 쪽 시험에 안 보인다. leftover 의 실프로세스 시험(`#[cfg(all(test, windows))]` — `leftover.rs:4221 · 6563-6573`)이 쓰는 도우미 넷(`spawn_gated_cmd` · `open_gate` · `wait_until` · `is_ping`)을 `test-support` 뒤 `testing` 모듈로 연다(선례 = command crate `test-support` — `crates/engram-dashboard-command/Cargo.toml:10-11`). 넷 다 OS 수준 값만 주고받는다(`Child` · PID · 탐침).
- **`new_group`(지금 `process_group.rs:392-395` — `(Arc<JobObjectHandle>, ProcessGroup)` 을 돌려준다)은 platform `testing` 에 가지 않는다**(D3 = B). 그것이 만드는 `ProcessGroup` · `RetiringSignal` 이 agent 어댑터 것이라 platform 이 못 만든다. agent 어댑터의 시험 지원(`transport/process_group.rs` 의 `#[cfg(all(test, windows))]` 도우미)에 남아 `GroupOwner::new()` → `downgrade()` → `ProcessGroup::new(g, RetiringSignal::of(&Arc::default()))` 로 만들고 `(GroupOwner, ProcessGroup)` 을 돌려준다. leftover 시험(`:4221-4226` · `:6571-6572` · `:6678` · `:6726` · `:6751`)의 import 는 둘로 갈린다 — 도우미 넷은 `engram_dashboard_platform::testing`, `new_group` 은 agent 어댑터에서. 강한 쪽의 `job.assign(..)` · `job.terminate(1)` 은 `adopt` · `terminate` 로 바뀐다. 그 도우미가 `#[cfg(windows)]` 이므로 어댑터 파일은 §4-4 기대 명단의 「시험 분기뿐인 파일」에 든다(U8 재측정). `detached` 도 agent 에 남으므로 platform 이 공개 생성자로 여는 것은 `GroupRef::gone()` 하나다(§3-4).
- **기능 이름 `test-support` 는 1-1 의 결정(N1 = `test-support`)과 같다** — base(1-1 C4=(a) 의 가짜 시계 `ManualClock` 을 `time` 안 이 기능 뒤에 둔다) · command · transport · platform 이 「시험 도우미를 부르는 쪽 시험에 연다」를 한 이름으로 쓴다. 기능은 crate 마다 따로라 서로 켜지 않는다 — platform 은 base 를 의존하지 않는다(ADR-0266 · ADR-0268).
- agent `[dev-dependencies]` 에 `engram-dashboard-platform = { …, features = ["test-support"] }` 를 더한다(command 와 같은 꼴). discovery 도 WMI 진단 시험(`#[ignore]` · `lib.rs:2754 · 2828`)이 원시 호출을 쓰므로 같은 꼴로 켠다.
- 이 `wait_until`(`what` 과 탐침을 받아 값을 돌려주는 꼴)은 1-1 이 base `testing` 으로 모으는 `wait_until(timeout, cond) -> bool`(ADR-0269 결정 5)과 **시그니처가 다른 별개 함수**다. platform 은 base 를 의존하지 않으므로(ADR-0266 · ADR-0268) 합치지 않는다.
- ★**모듈 선언은 `#[cfg(any(test, feature = "test-support"))] pub mod testing;`** 이다 — `feature` 하나로만 걸면 안 된다★: 옮겨 온 platform 자기 시험(`group` 의 실프로세스 시험 등)이 같은 도우미를 쓰는데, CI 스텝 `cargo test --locked -p engram-dashboard-platform`(§4-2 — 기능 인자 없음)에서는 `test-support` 가 꺼져 있어 그 시험이 컴파일되지 않는다. 선례 = transport(`any(test, feature = "test-support")`) · 1-1 의 base `testing` 도 같은 꼴이다.
- 운영 빌드(`cargo build --locked`)는 이 기능을 켜지 않는다 — 기계 게이트로 잰다(§4-2 ③).

### 3-9. `windows` feature (결정 D1 = a · §7)

- platform 이 갖는 합집합(실측 — 지금 base · agent · discovery 매니페스트의 합): `Win32_Foundation` · `Win32_System_Threading` · `Win32_System_Diagnostics_ToolHelp` · `Win32_System_JobObjects` · `Win32_Security` · `Win32_System_IO` · `Wdk_System_Threading` · `Win32_System_RestartManager` · `Win32_System_Com` · `Win32_System_Wmi` · `Win32_System_Rpc` · `Win32_System_Variant`.
- ★`Win32_Security` 를 빼지 말 것★ — `CreateJobObjectW` 가 그 뒤에 있고 `Win32_System_JobObjects` 가 함의하지 않는다(CLAUDE.md 「의존성」 · agent `Cargo.toml` 주석 — 그 주석째 옮긴다).
- **각 crate 에서 사라지는 것:** base · discovery 의 `[target.'cfg(windows)'.dependencies]` 통째. agent 는 운영 의존에서 사라지고 **`[target.'cfg(windows)'.dev-dependencies]` 로 남는다** — 예제 둘이 `windows` 를 직접 쓴다(§2-5). 그 feature 는 예제가 컴파일되는 최소 집합(`Win32_Foundation` · `Win32_System_JobObjects` · `Win32_System_Threading` 에 `Win32_Security` 가 더 필요한지는 **미검** — 해당 단위에서 컴파일로 정한다).
- **기능별 cargo feature 로 쪼개지 않는다**(결정 D1 = a · §7) — `windows` 의존 하나에 위 합집합을 다 켠다. platform 의 cargo feature 는 `test-support` 하나뿐이고(§3-8), 그래서 기능 조합 게이트(net 5a · 5b 꼴)도 없다.

---

## 4. 의존 그래프와 게이트

### 4-1. 의존 그래프 — 직접 워크스페이스 의존 (실측 `cargo tree -p <pkg> --depth 1 --prefix none -e normal --target all`)

| crate | 지금 | 1-3 뒤 |
|---|---|---|
| platform | — | **없음**(잎) |
| base | 없음 | 없음 — 단 `windows` 서드파티 의존이 사라진다 |
| net | base · protocol | **platform** · protocol(base 를 더는 안 쓴다 — 쓰던 심볼이 전부 PID 헬퍼였다) |
| discovery | base · net · protocol | base · net · **platform** · protocol — ★base 는 남는다★: 지금 쓰는 PID 헬퍼 한 줄(`lib.rs:1043`)은 platform 으로 가지만, 먼저 착지하는 1-1 이 discovery `Clock` 을 `engram_dashboard_base::time::Clock` 의 하위 트레이트로 만든다(ADR-0269 결정 3-2 · 1-1 TRD §3-3 · U5) |
| agent | base · command | base · command · **platform** |
| daemon | agent · base · command · discovery · messaging · net · protocol | 같음 + **platform**(`current_process_start_time` · `exe_file_name` · `default_shell`) |
| 셸(`engram-dashboard`) | agent · base · command · discovery · net · protocol | **변화 없음**(platform 은 discovery 를 거쳐 전이로만 닿는다. U-W 때 직접 의존이 생긴다) |

### 4-2. 새 게이트 — platform (ADR-0266 「영향」)

```bash
# ① 워크스페이스 의존 상한 → 정확히 1줄(자기 자신). windows · tracing 은 접두가 달라 세지 않는다.
cargo tree -p engram-dashboard-platform --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u
# ② Tauri import 0 — 경로부터 확인(경로가 없어도 rg 는 0줄 = 통과로 읽힌다)
test -d crates/engram-dashboard-platform/src || { echo "FAIL — 게이트 경로 부재"; false; }
rg "^\s*use tauri" crates/engram-dashboard-platform/src/
# ③ 시험 기능이 운영 그래프에 없다 → 둘 다 0줄 (1-1 의 G2 와 같은 꼴)
cargo tree --locked -p engram-dashboard-daemon -e normal,features -i engram-dashboard-platform --target all | rg 'feature "test-support"'
cargo tree --locked -p engram-dashboard        -e normal,features -i engram-dashboard-platform --target all | rg 'feature "test-support"'
#   짝 — 눈먼 게이트 방지 → 1줄 이상(기능 이름을 바꾸면 위 0 기대가 조용히 통과하므로)
cargo tree --locked -p engram-dashboard-agent -e normal,dev,features -i engram-dashboard-platform --target all | rg 'feature "test-support"'
# 로컬 시험 — 실 자식 프로세스를 띄우는 시험(child_pids · Job · RM · 트리)이 따라오므로 플래그를 붙인다
cargo test -p engram-dashboard-platform -- --test-threads=4
```

- 등록할 곳 셋(ADR-0175 영향 · ADR-0266 영향): `.github/workflows/ci.yml`(①② 스텝 + `cargo test --locked -p engram-dashboard-platform` 스텝 — CI 는 `--test-threads` 를 쓰지 않는다 · ③ 은 U3 에서) · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md`(4-pre 꼴 경로 가드 포함).
- ①은 `--all-features` 이므로 `test-support` 를 켠 그래프도 본다 — 그 기능이 워크스페이스 crate 를 끌어오지 않는지까지 잰다.
- ③은 **U3 에서 등록한다** — `testing` 모듈과 agent 의 dev-dependency(`features = ["test-support"]`)가 그 단위에서 처음 선다. 그 전에는 짝 줄의 1줄 기대가 설 수 없다. 셸은 platform 을 discovery 를 거쳐 전이로만 닿고(§4-1) 데몬을 dev-dependency 로 물지만 `-e normal` 이라 dev 간선은 세지 않는다. 워크스페이스가 `resolver = "2"` 라 dev 그래프의 기능이 운영 빌드로 합쳐지지 않는다는 전제를 이 게이트가 잰다.

### 4-3. 바뀌는 게이트 — 새 기대값

| 게이트 | 지금 | 바뀐 뒤 | 바뀌는 단위 | 고칠 자리 |
|---|---|---|---|---|
| net gate 2a(심볼 allowlist) | `rg -o --no-filename "engram_dashboard_base::[A-Za-z0-9_:]+" crates/engram-dashboard-net/src/ \| sort -u` → **정확히 2** | 접두를 `engram_dashboard_platform::` 로 · **U1 뒤 정확히 2**(`…::process::pid_alive_with_start_time` · `…::process::current_process_start_time`) → **U6 뒤 정확히 5**(+ `…::fs::open_deny_write` · `…::fs::is_sharing_violation` · `…::fs::is_access_denied`) — 이름은 가안, 수는 API 확정 때 다시 박는다 | U1 · U6 | `ci.yml:784-813` · `net/src/lib.rs:59-75` · CLAUDE.md 「네트워크 행 격리 게이트」 · `qa.md:163` · `docs/testing-strategy.md:72` |
| net gate 2b(0줄) | `engram_dashboard_(a)gent` → 0 | 그대로. ★2a 의 짝 서술(「심볼이 사라진 게 아니라 옮겨 갔다」)은 「agent → base → platform」으로 한 번 더 갈아탄 것을 적는다★ | U1 | `ci.yml:815-831` 주석 · net 헤더 · CLAUDE.md 「네트워크 행 격리 게이트」 2b 줄(「`agent`에서 `base`로 이사하며 기대값 2가 crate 이름을 갈아탔고」) · `docs/testing-strategy.md:73` |
| net gate 3(직접 의존 상한) | **정확히 3** = net · base · protocol | **정확히 3** = net · **platform** · protocol | U1 | `ci.yml:833-852` · `net/src/lib.rs:83-93` · CLAUDE.md · `qa.md:165` · `docs/testing-strategy.md:74` |
| base 입주자 상호 참조 ③ | `rg "(crate\|super)::(logging\|platform)" crates/engram-dashboard-base/src/` → 0 | 이름 알파벳에서 `platform` 을 뺀다. 1-1 은 `file` 을 미루고(그 G1 규칙 — 게이트는 실재 입주자 이름만 적는다) `text\|time\|path`(1-1 U1) → `+sync`(U2) → `+testing`(U4) 를 더한다. 1-1 이 먼저 착지하므로(§5 선행) U1 은 1-1 이 남긴 정규식에서 `platform` 하나만 뺀다 → 둘 다 착지한 뒤 = `rg "(crate\|super)::(logging\|text\|time\|path\|sync\|testing)" crates/engram-dashboard-base/src/` → 0(예외 없는 꼴 — 1-1 C4=(a) 로 가짜 시계가 `time` 안에 서서 `testing` 이 `time` 을 부르지 않는다) | U1 | `ci.yml:640-656` · base `lib.rs` 헤더 · CLAUDE.md · `qa.md:138` · `docs/testing-strategy.md:50 · 155` |
| messaging 소스 이름 게이트 | `rg "engram_dashboard_(agent\|base\|daemon\|protocol\|discovery\|command)" crates/engram-dashboard-messaging/src/` → 0 | 알파벳에 `platform` 을 더한다(0 기대 그대로). 선례 = crate 가 태어날 때 이름을 더했다 — `command`(ADR-0134) · `base`(ADR-0175) · 근거 = `ci.yml:511-514` 주석 「새 워크스페이스 crate 가 생길 때마다 여기 이름을 더해야 보인다」 | U0 | `ci.yml:519`(+ :511-514 주석) · CLAUDE.md 「빌드·검증 명령」 · `qa.md:152` · `docs/testing-strategy.md:63` |
| base 의존 상한 · `use tauri` | 1줄 · 0줄 | 그대로 | 서술만 U1 | 「headless 데몬 · `net` · `discovery` 에 링크된다」 서술에서 **`net` 만** 빠진다 — discovery 는 1-1 의 `time::Clock` 으로 base 를 계속 쓴다(§4-1). 같은 서술의 사본: base `lib.rs:35` 헤더 · CLAUDE.md 「빌드·검증 명령」 base `use tauri` 줄(:248) · `qa.md:137 · 148 · 307` · `docs/testing-strategy.md:49` |
| `cargo test -p engram-dashboard-base` | 로컬 `-- --test-threads=4`(근거 = `platform` 의 `child_pids_finds_spawned_child`) | 그 근거가 platform 으로 간다 — 남는 `logging` 시험은 프로세스를 안 띄운다(`rg 'Command::new\|process::Command\|current_exe' crates/engram-dashboard-base/src/logging crates/engram-dashboard-base/tests` → 운영 코드의 `current_exe` 1곳뿐 · 실측). 1-3 은 1-1 머지 뒤에 착수하므로 U1 시점엔 1-1 의 입주자(`text` · `time` · `path` · `sync` · `testing`)가 다 서 있다 — U1 이 같은 rg 를 base `src/` · `tests/` 전체에 다시 돌려 프로세스를 띄우는 시험이 0 이면 뗀다(제안) | U1 | CLAUDE.md 「빌드·검증 명령」 · `ci.yml:270-278` 주석 · `docs/testing-strategy.md:46 · 138` |
| net `server` feature | `dep:engram-dashboard-base` | `dep:engram-dashboard-platform` | U1 | `net/Cargo.toml` |
| discovery async-runtime 반입 게이트 | 0 | 그대로 — platform 은 tokio 를 끌지 않는다(지켜야 할 것으로 platform 헤더에 적는다) | — | — |

★net gate 2a 를 살리려면 net 이 platform 을 **완전경로나 낱개 `use`** 로 불러야 한다★ — `use engram_dashboard_platform::fs::{a, b}` 처럼 중괄호로 가져오면 `rg -o` 가 `engram_dashboard_platform::fs::` 한 줄만 잡아 수가 어긋난다(지금 base 심볼도 완전경로로 부른다 — `portfile.rs:86`).

### 4-4. 불변식 게이트 (제안 — ADR-0266 이 게이트 꼴을 메인 판단으로 남겼다)

ADR-0266 불변식 「`#[cfg(windows)]` · `cfg!(windows)` 는 platform crate 안에만」에는 지금 기계 게이트가 없다. 값싼 조기 신호로 **파일 allowlist** 를 둔다:

```bash
# platform 밖에서 OS cfg 를 쓰는 파일 목록 = 아래 고정 명단과 정확히 같아야 한다.
# 접두 (?:[^/]|/[^/])*? = 「아직 // 를 안 만났다」 → 주석 속 인용(net lib.rs:44 등)은 안 문다(replay_flight 게이트와 같은 꼴).
# 괄호 안 (?:[^)]|\([^()]*\))*? = 「닫는 괄호가 아닌 글자, 또는 안쪽이 한 겹인 닫힌 괄호 묶음」
#   → cfg(all(not(test), windows)) 처럼 안쪽 괄호가 한 번 닫힌 뒤에 오는 OS 낱말도 문다.
# -g '*.rs' = 매니페스트(Cargo.toml 의 [target.'cfg(windows)'.…])를 빼고 소스만 본다.
rg -l "^(?:[^/]|/[^/])*?\b(cfg!?|cfg_attr)\((?:[^)]|\([^()]*\))*?\b(windows|unix|target_os|target_family)\b" \
   crates src-tauri/src -g '*.rs' -g '!crates/engram-dashboard-platform/**' | sort
```

- **1판 정규식(`\([^)]*`)의 결함 셋(실측 `0ef6292` — 두 꼴을 같은 트리에서 돌려 비교):** ① `-g '*.rs'` 가 없어 agent · base · daemon · discovery 의 `Cargo.toml` 넷이 걸렸다 ② 안쪽 괄호가 닫힌 뒤의 OS 낱말을 못 물어 `#[cfg(all(not(test), windows))]` 같은 운영 분기가 빠져나갈 수 있었다(지금 저장소에 그 꼴의 운영 분기는 없다 — 고친 꼴이 새로 문 것은 `main.rs` 둘뿐) ③ 그래서 「`main.rs` 둘이 `cfg_attr` 로 걸린다」는 기대 명단의 근거가 거짓이었다 — 1판 꼴은 둘 다 못 문다. 고친 꼴은 `.rs` 41 파일을 물고(1판 꼴 43 − 매니페스트 4 + `main.rs` 2), 그 밖에 새로 걸린 운영 파일은 없다.
- 기대 명단 = §2-3 의 「시험 분기뿐인 파일」 + 하네스 bin 둘 + agent 예제 둘(`examples/spike*.rs` — 1판은 빠뜨렸다) + `src-tauri/src/main.rs` · `daemon/src/main.rs` + **`src-tauri/src/fsutil.rs`(U-W 까지의 알려진 운영 예외)**. ★`main.rs` 둘이 걸리는 까닭은 cfg 술어가 아니다★ — `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` 에서 `not(…)` 묶음을 넘어 **속성 값 `"windows"`** 를 문다(매처의 오탐). 그래도 crate 수준 링커 속성이라 옮길 수 없으므로(§2-3) 명단에 둔다. 정확한 명단은 U8 에서 이 명령으로 다시 재서 박는다.
- **한계(은폐 금지):** 명단에 이미 있는 파일 안에 운영 분기를 새로 넣으면 못 잡는다 — 새 **파일**만 잡는다. 런타임 OS 가름(하네스의 `experiment/transcript.rs` 같은 `cfg` 없는 환경변수 고르기)도 못 잡는다. ★괄호가 두 겹 이상 닫힌 뒤에 오는 OS 낱말(`cfg(all(not(any(test, x)), windows))`)과 여러 줄에 걸친 `cfg` 속성도 못 문다★ — 지금 저장소에는 둘 다 0 이다(실측 rg). `cfg_attr` 의 속성 값에 낱말이 오면 술어가 아닌데도 문다(위 `main.rs`). 리뷰가 메운다.
- `--sort path` 대신 `| sort` 를 쓰는 것은 기존 게이트와 같은 꼴이다.

---

## 5. 이전 단위

**원칙(사용자 2026-10-02 — 「망가지지 않는 단위로 잘 그룹지어서 작업하라」 · 「임시 땜빵 금지」):** 단위마다 워크스페이스가 빌드되고 회귀가 초록인 채로 끊는다. 같은 코드를 두 곳에 잠깐 두는 징검다리를 쓰지 않는다 — 옮기는 단위가 부르는 곳까지 한 번에 바꾼다. 단위 안 순서는 「새 자리 만들기 → 부르는 곳 바꾸기 → 옛 자리 지우기」이고, 어느 단위도 다른 단위의 옛 자리를 남겨 두지 않는다.

**선행:** ① ★**1-1(base 범용 도우미)이 머지된 뒤에만 U0 에 착수한다**★ — 1-1 의 base `lib.rs` 헤더 · 게이트 ③ · `sync` 교체(agent `codex/transport.rs` 는 1-1 TRD 의 U3b 범위 — 개수는 그쪽이 정한다 · 그 밖 U3a 의 통로 · leftover · `usage/process.rs` 등)와 파일이 크게 겹친다(아래 「겹침」). ② 각 단위 착수 전 그 자리 커밋으로 되돌릴 지점이 있다(전역 규칙).

**1-1 의 결정 중 이 문서에 닿는 것(2026-10-04 · 정본 = 형제 1-1 TRD — 여기는 맞춘 결과만 적는다):**

- **S2 = (e) 락 오염은 경고 없이 되찾기만 한다(독은 둔다)** — base `sync` 도 말없이 되찾는다. U5 의 찾기 명단 `release` 가 platform 안에서 쓰는 한 줄(`unwrap_or_else(PoisonError::into_inner)`)과 뜻이 같다. platform 은 base 를 못 부르므로 그 한 줄로 남는다(§5-1 U5).
- **N1 = 기능 이름 `test-support`** — platform 의 시험 기능과 같은 이름이다(§3-8).
- **C4 = (a) 가짜 시계는 base `time` 안 `test-support` 뒤** — base 입주자 무참조 게이트 ③ 에 예외가 없다(§4-3).
- **C5 = (b) transport → base 는 3단계로 미룬다** — transport 는 이번에 base 를 의존하지 않는다. 그래서 U1 이 base 에서 `windows` 를 걷는 순서가 transport 그래프에 닿지 않는다(1-1 C5 (a) 였다면 순서에 달렸던 것).
- **platform 은 그대로 base 와 무관하다** — 위 넷 어느 것도 platform → base 간선을 만들지 않는다(ADR-0266 · ADR-0268).

### 5-1. 단위 목록

| 단위 | 범위 | 건드리는 crate · 파일 | QA |
|---|---|---|---|
| **U0** | 빈 crate 세우기 · 게이트 등록 · 메시징 이름 게이트에 `platform` | 루트 `Cargo.toml`(+ 머리 주석 「멤버 10」 → 11 · platform 한 줄) · `Cargo.lock` · `crates/engram-dashboard-platform/{Cargo.toml, src/lib.rs}`(헤더 = 불변식 · 게이트 · `// ADR-0266`) · `ci.yml`(새 스텝 셋 · 메시징 이름 게이트 `:519` 와 그 주석) · CLAUDE.md 「빌드·검증 명령」 게이트 줄 · `qa.md` · `docs/testing-strategy.md:63` | standard |
| **U1** | PID 헬퍼(P1) 이사 · base 에서 `platform` 과 `windows` 걷기 | platform `process.rs` · base `{lib.rs, platform.rs(삭제), Cargo.toml}` · net `{Cargo.toml(+ 주석 :43-45 「protocol·base」 · :73-77 · :135), portfile.rs, lib.rs 헤더}` · discovery `{Cargo.toml(platform 의존을 더한다 · ★base 의존은 지우지 않는다★ — 1-1 U5 의 `time::Clock` 상위 트레이트가 쓴다(§4-1). :12-14 의 base 줄 주석 중 「liveness 판정 공유」 사유는 platform 줄로 옮긴다 · 주석 :19 forward 폐포 {net, base, protocol} → {base, net, platform, protocol} — base 는 남는다(§4-1)), lib.rs:1043(PID 헬퍼 import 만 platform 으로)}` · daemon `{Cargo.toml, lib.rs:762, tests/ws_e2e.rs:2447}` · agent `{Cargo.toml, platform/{process_tree.rs, file_holders.rs, windows.rs}, backend/codex/{thread_lock.rs:279 · :1236(시험 — `process_creation_time`), mod.rs:1107, transport.rs 시험}, backend/claude/leftover.rs 시험, usage/process.rs 시험, tests/backend_contract.rs}` · 루트 `Cargo.toml` 주석(:2 · :12-13 의 base = 「로깅·PID 판정」) · `ci.yml`(net 2a · 3 · base ③) · `qa.md`(+ base 링크 서술 :137 · 148 · 307 에서 `net` 빼기) · CLAUDE.md 게이트 줄(+ base `use tauri` 줄 :248 의 링크 서술 · net 2b 줄의 이사 서술 — §4-3) · `docs/testing-strategy.md`(:46 · :49 · :50 · :72-74 · :138 · :155) | standard |
| **U2** | OS 규칙(R1~R6) | platform `{shell.rs, env.rs}` · agent `{manager.rs, backend/mod.rs, backend/claude/mod.rs, backend/codex/thread_lock.rs, tests/*(default_shell 5파일)}` · daemon `{lib.rs:109, connection_core.rs, messaging_host.rs 시험, tests/*(3파일)}` · discovery `lib.rs:987` — 하네스의 `experiment/transcript.rs` 는 건드리지 않는다(§2-3) | standard(로컬 실 claude 시험 포함 — `console_command` 가 모든 CLI 에이전트 스폰을 지난다 · CI 는 claude 가 없어 못 잰다) |
| **U3** | 무리 주인 · 손잡이(P2 · P3 · P6 나머지) · 세 통로의 `cfg` 걷기 · 창 없이 띄우기(통로 둘) · ★agent `platform/` 폴더 정리 · agent `windows` → dev-dependency(U4 뒤라서 — §5-1 순서 주)★ · `test-support` 운영 누수 게이트(§4-2 ③) | platform `{group/mod.rs, group/windows.rs, spawn.rs(hide_console_window), testing.rs, Cargo.toml}` · agent `{platform/{windows.rs(삭제 — `LEFTOVER_EXIT_CODE` 는 agent 잔여물 정리 쪽으로 · §3-4), process_group.rs(cfg 없는 어댑터로 고쳐 `transport/process_group.rs` 로 옮김 — 시험 도우미 `new_group` 도 그 어댑터 시험 지원에 남는다(§3-8)), mod.rs(삭제)}, lib.rs(`pub mod platform;` 삭제), transport/{mod.rs(`process_group` mod 선언), pty.rs, stdio.rs(`process_group()` = `downgrade()` 의 `GroupRef` + `RetiringSignal::of(&self.retiring)` → 어댑터 `ProcessGroup::new` — ★물러남 표시를 싣는다★ §2-2 · §3-4), input_queue.rs:41 주석}, backend/codex/transport.rs(+ 아래 소스 문자열 시험), backend/claude/{leftover.rs(시험 import — 도우미 넷은 platform `testing` · `new_group` 은 agent 어댑터 · `job.assign` · `job.terminate` → `adopt` · `terminate`), mod.rs}, usage/process.rs(`crate::platform` 참조 전부 = 운영 셋 `:438 · 444 · 451`(Job 칸 · 만들기 · 깨우기) + `#[cfg(windows)]` 시험 둘 `:1389 · 1411`(깨우기) — 하나라도 남기면 agent `platform/` 이 사라진 뒤 시험 빌드가 깨진다), Cargo.toml(`windows` → `[target.'cfg(windows)'.dev-dependencies]` · platform dev-dependency `test-support`)}` · `ci.yml` · CLAUDE.md · `qa.md`(게이트 ③). ★**소스 문자열 시험 하나가 옮겨지는 이름을 박아 둔다**★ — `backend/codex/transport.rs:6639-6658` `the_child_guard_is_armed_before_the_first_fallible_step_after_spawn` 이 `open` 본문에서 `"JobObjectHandle::new()?"` · `"job.assign(pid)?"` 를 찾는다(없으면 패닉). U3 이 쓰는 새 철자(가안 `GroupOwner::new()?` · `.adopt(pid)?`)로 두 문자열과 머리 주석(:6637)을 바꾸고 **불변식 「가드(`ChildGuard(Some(child))`)가 무리 만들기 · 넣기보다 먼저 선다」는 그대로** 잰다. 같은 꼴의 다른 시험은 옮겨지는 이름을 안 쓴다(실측 — `include_str!` 소스 시험 20곳 · 옮겨지는 이름을 담은 문자열 리터럴 rg) | ★**full**(GUI 포함) — kill 인과(ADR-0001)가 Job 을 지난다(ADR-0266 영향)★ |
| **U4** | 파일을 연 프로세스 · 신원 목록(P4 · P5) | platform `{file_holders.rs, process.rs(subtree)}` · agent `{platform/{file_holders.rs(삭제), process_tree.rs(삭제), mod.rs(그 둘의 mod 선언 · 머리 주석)}, backend/codex/{thread_lock.rs, mod.rs:3781}, Cargo.toml(`windows` feature 에서 `Win32_System_RestartManager` 만 뺀다 — 제안 · 남은 쓰임 0 을 컴파일로 확인)}`. ★폴더 · `windows.rs` · `process_group.rs` 는 지우지 않는다★ — U4 시점에 그 둘은 아직 통로 셋 · `usage/process.rs` · leftover 가 부른다(`stdio.rs:36 · 38` 등) | standard(+ 로컬 codex 세션 id 회수 시험) |
| **U5** | 사용량 조회의 OS 층(U) | platform `spawn.rs`(`prepare_tree_root` · `TreeRoot` · `MissingProgramCheck`) · agent `usage/process.rs`. ★**찾기 명단(`LookupGate` — `usage/process.rs:334-354`)의 독 처리를 platform 안에서 스스로 한다**★ — platform 은 base `sync` 를 못 부른다(ADR-0266 · ADR-0268) · 1-1 U3a 는 복구 꼴을 sync 로 옮기지 않고(도우미만 release 안으로 접고 1-3 U5 를 가리키는 주석을 달아) 넘긴다. ① 자리 잡기(`claim`): 독 = 「도는 중」 → 판정 「모름」 — 지금 뜻 그대로이고 시험 `a_poisoned_gate_means_unknown_without_a_panic`(:1742)이 함께 옮겨 간다. 독을 걷지 않으므로 한 번 독이 들면 그 프로세스의 찾기는 늘 「모름」이다(지금과 같다) ② 자리 비우기(`release` — `InFlight` 의 `Drop`): `unwrap_or_else(PoisonError::into_inner)` 로 말없이 되찾는다(지금 `lock` 도우미 `:636` 와 같은 뜻을 한 줄로 · 도우미를 platform 에 새로 두지 않는다). 1-1 U3a 가 그 도우미를 `release` 안으로 접고 「1-3 U5 로 간다」 주석을 달아 두므로, U5 는 그 한 줄을 옮기고 주석을 걷는다. 1-1 S2=(e) 의 base `sync::lock` 도 경고 없이 되찾으므로 뜻이 갈리지 않는다(§5 선행). ③ 로그: platform 은 찍지 않는다(제안) — 독은 「모름(막힘)」 값으로 나가 agent 의 1회 경고가 지금 문구로 찍고(§3-7), `Drop` 쪽 회복은 실패가 아니라 돌려줄 것도 없다. 지금도 독과 「막힌 찾기」는 같은 경고 문구로 나간다(독인데 「앞선 찾기가 아직 끝나지 않아」라고 적힌다) — 옮기는 단위라 고치지 않는다 | standard + 로컬 `cargo test -p engram-dashboard-agent --test usage_probe_smoke -- --ignored`(전부 `#[ignore]` — 로그인된 CLI 필요) |
| **U6** | 단일 인스턴스 열기(N) | platform `fs.rs` · net `{instance.rs, lib.rs 헤더}` · `ci.yml`(net 2a = 5) · CLAUDE.md · `qa.md` · `docs/testing-strategy.md:72` | standard(instance 시험이 Windows 에서 공유 의미를 실물로 잰다) |
| **U7** | 프로세스 끄기 · WMI 띄우기(D1 · D2) · discovery 의 `windows` 걷기 | platform `{process.rs(kill_tree), spawn.rs + spawn/wmi.rs, Cargo.toml}` · discovery `{lib.rs, Cargo.toml}` | **full** — 앱 실행이 데몬을 WMI 로 띄우고(`ensure_daemon`), 끄기가 `taskkill` 을 탄다. 데몬 기동 실측이 완료 조건(ADR-0271 결정 7 과 같은 잣대) |
| **U8** | 불변식 게이트(§4-4) · 문서 후속(§6) | `ci.yml` · CLAUDE.md · `qa.md` · base `lib.rs` 헤더 · 메모 §11 | `/qa` 바인딩의 문서 · 설정 범위 + `/review doc`(load-bearing 문서) |
| U-W (미룸) | 원자적 쓰기 통일 → platform `fs` | 셸 `fsutil.rs`(P3 뒤 모양) · daemon `usage_service/reject_store.rs` · agent `persistence/{mod.rs, presets.rs}` · 셸에 platform 직접 의존 | 트리거 = storage P3 의 master 착지. 「하나로 정할 동작」은 그때 사용자 선택(메모 §10 · ADR-0266 결정 8) |

**순서:** U0 → U1 → U2 → U4 → U3 → U5 → U6 → U7 → U8.

- U1 이 맨 앞이어야 한다 — agent 쪽 조각(P4 · P5 · P2 시험)이 지금 base 의 PID 헬퍼를 부르므로, 그 헬퍼가 platform 에 먼저 서 있지 않으면 그 조각들을 옮기는 순간 platform → base 간선이 필요해진다(금지 — ADR-0266 · ADR-0268).
- U3 은 U5 보다 앞이어야 한다 — 사용량 조회의 트리 손잡이가 Job 주인과 깨우기를 쓰므로 그것이 platform 에 먼저 있어야 한다. U3 은 `usage/process.rs` 의 `crate::platform` 참조 다섯(운영 셋 · `#[cfg(windows)]` 시험 둘 — 위 표)만 platform 경로로 바꾸고 나머지는 U5 에 둔다. 시험 둘을 빠뜨리면 운영 빌드는 서도 `cargo test` 가 깨진다.
- U4 를 U3 앞에 둔 이유 = 위험이 낮은 것을 먼저 빼서 U3 의 full QA 가 kill 인과 하나만 보게 한다. 둘은 바꿔도 깨지지 않는다(겹침은 agent `platform/mod.rs` · `Cargo.toml` 뿐) — ★agent `platform/mod.rs` · 폴더 · `lib.rs` 의 `pub mod platform;` 삭제와 agent `windows` 의 dev-dependency 전환은 둘 중 **뒤에 오는 단위**가 가진다 — 이 순서에서는 U3 이다★(위 표). 먼저 오는 단위는 자기 파일과 그 mod 선언만 지운다. 순서를 바꾸면(U3 → U4) 이 몫이 U4 로 간다 — 그때 U3 은 `windows.rs` 를 지우고 `process_group.rs` 를 처리한 뒤 `platform/mod.rs` 에 `file_holders` · `process_tree` 두 줄을 남긴다.
- U6 · U7 은 U1 뒤 어디에 와도 된다(agent 를 안 건드린다).

### 5-2. 단위마다 검증

1. **회귀 수 대조 — 앞뒤로 같은 명령을 돌려 두 수를 비교한다.** 대상이 조용히 사라지는 것(시험 타깃 소실 · `#[cfg]` 로 시험이 안 컴파일됨)은 실패가 아니라 침묵이다.

   ```bash
   cargo test --workspace -- --test-threads=4 > "$SCRATCH/u<N>-<before|after>.log" 2>&1
   rg -c '^test result:' "$SCRATCH/u<N>-after.log"                     # 결과 줄 수
   rg '^test result:' "$SCRATCH/u<N>-after.log" \
     | sed -E 's/.* ([0-9]+) passed; ([0-9]+) failed; ([0-9]+) ignored.*/\1 \2 \3/' \
     | awk '{p+=$1; f+=$2; i+=$3} END {print "passed="p, "failed="f, "ignored="i}'
   ```

   터미널이 크래시하면 2로 낮추거나 `scripts/run-detached.ps1` 로 프로세스 트리 밖에서 돌린다(CLAUDE.md 「빌드·검증 명령」).

2. **기대 차이(단위가 선언하고 결과가 맞아야 한다):**

   | 단위 | 결과 줄 | 통과 · 무시 합계 | 분포 이동(실측 `rg -c '#\[(tokio::)?test'` · `rg -c '#\[ignore'`) |
   |---|---|---|---|
   | U0 | **+2**(platform lib 단위 · Doc-tests — 0건짜리도 줄이 찍힌다) | 그대로 | — |
   | U1 | 그대로 | 그대로 | base → platform 13 |
   | U2 | 그대로 | 그대로 + 새 시험 k(단위가 수를 적는다) | 옮겨 가는 규칙 시험이 있으면 agent → platform |
   | U3 | 그대로 | 그대로 | agent → platform: `windows.rs` 4(무시 1) + `process_group.rs` 중 실프로세스 시험(전체 10 · 무시 1 — 가짜 기반 시험은 agent 어댑터에 남는다 · D3 = B). 몇 개가 어느 쪽인지는 U3 이 선언한다 |
   | U4 | 그대로 | 그대로 | agent → platform: `file_holders.rs` 2 · `process_tree.rs` 9 |
   | U5 | 그대로 | 그대로 | agent → platform: `usage/process.rs` 34(무시 1) 중 OS 층 시험 |
   | U6 | 그대로 | 그대로 + 새 `fs` 시험 k | instance 시험 11 은 net 에 남는다(가드의 뜻을 잰다) |
   | U7 | 그대로 | 그대로 | discovery → platform: COM 분류 시험 1 · WMI `#[ignore]` 둘은 discovery 에 남는다(데몬 exe · `daemon.json` 이 필요) |

   **기준선은 U0 착수 직전에 잰다** — 마지막 기록(CLAUDE.md 「빌드·검증 명령」 = 결과 줄 59 · 3822 통과 · 30 무시, `cac9ee0`)은 그 뒤 master 흡수로 낡았을 수 있다. 이 TRD 는 돌려 보지 않았다.

3. **게이트:** 그 단위가 건드린 crate 의 게이트 전부 + `cargo fmt --check` + 생성물 sync(agent `bindings/` — 이 이전이 ts-rs 출력을 바꾸지 않아야 한다) + U0 뒤로는 platform 게이트 ①② + U1 뒤로는 바뀐 net 2a · 3 · base ③.

4. **QA:** §5-1 표의 등급(`/qa` 바인딩). U3 · U7 은 GUI 실측까지 — 앱은 `scripts/` 런처로 띄운다(셸에서 직접 띄우지 않는다).

### 5-3. 파일 겹침 — 직렬 · 병렬

| | U1 | U2 | U3 | U4 | U5 | U6 | U7 |
|---|---|---|---|---|---|---|---|
| U2 | discovery `lib.rs` · `thread_lock.rs` · daemon `lib.rs` | | | | | | |
| U3 | agent `Cargo.toml` · `windows.rs` · leftover · codex transport · `usage/process.rs` · `ci.yml` · CLAUDE.md · `qa.md`(다른 줄) | `claude/mod.rs` | | | | | |
| U4 | `file_holders.rs` · `process_tree.rs` · `thread_lock.rs` · codex `mod.rs` | `thread_lock.rs` | agent `platform/mod.rs` · `Cargo.toml` | | | | |
| U5 | `usage/process.rs` | — | `usage/process.rs` | — | | | |
| U6 | net 헤더 · `ci.yml` · CLAUDE.md · `qa.md` | — | — | — | — | | |
| U7 | discovery `lib.rs` · `Cargo.toml` | discovery `lib.rs` | — | — | — | — | |

- 모든 단위가 platform 의 `lib.rs`(mod 선언) · `Cargo.toml` · `Cargo.lock` 을 함께 건드린다 — 합치기는 쉬우나 겹침이다.
- **권장 = 한 코더씩 직렬.** U1 은 반드시 혼자다(거의 모든 crate 를 건드린다). 병렬을 쓴다면 U1 뒤 **U6 ∥ U7** 둘뿐이다(agent 무관 · 겹침은 platform 공용 파일 셋 + `ci.yml` · CLAUDE.md · `qa.md` 의 서로 다른 줄).
- **1-1 과 1-3 은 병렬로 돌리지 않는다** — 1-1 의 `sync` 교체가 `codex/transport.rs` · `leftover.rs` · `usage/process.rs` · `stdio.rs` 를, `Clock` 합치기가 discovery `lib.rs` 를, 입주자 추가가 base `lib.rs` · 게이트 ③ 을 건드린다.
- **U3 지시서에는 「중간 어디서 멈춰도 빌드가 서게」 순서를 박는다** — ① platform 에 `group` 을 세운다(아무도 안 부름 · 빌드 초록) ② agent 어댑터(`ProcessGroup::new(GroupRef, RetiringSignal)` · `detached` · 시험 도우미 `new_group`)를 `transport/process_group.rs` 에 먼저 세우고, 통로 셋과 usage 의 `crate::platform` 참조 다섯 · leftover · claude `mod.rs` · 가드 배치 소스 문자열 시험을 그리로 · platform 으로 돌린다 ③ agent `windows.rs` 와 옛 `platform/process_group.rs` 를 지운다(어댑터는 ②에서 섰다) ④ agent `platform/mod.rs` · `lib.rs` 의 `pub mod platform;` 을 지우고 `windows` 를 dev-dependency 로 돌린다(U4 가 먼저 끝났을 때 — §5-1). ②의 중간에서 끊기면 ①만 남은 초록 트리로 되돌린다.

---

## 6. 문서 후속 (이 TRD 는 고치지 않는다 — 각 단위 또는 U8)

1. **CLAUDE.md 「백엔드 모듈 맵」** — platform 항목 신설(무엇 · 워크스페이스 의존 0 · `tracing` 직접 · 게이트 둘 · 정본 = 그 crate `src/lib.rs` 헤더 · ADR-0266). base 항목: 입주자 서술(「`platform`(PID liveness·…)」). 「빌드·검증 명령」 base `use tauri` 줄(:248)의 「headless 데몬·`net`·`discovery`에 링크」에서는 **`net` 만** 뺀다 — discovery 는 1-1 의 `time::Clock` 으로 base 를 계속 쓴다(§4-1). agent 항목: 「남은 `platform`은 Job Object 래퍼 하나뿐이다」(메모 §11 첫 줄 — 지금도 낡았다: 실제 넷)와 「`logging`과 PID 판정 헬퍼는 여기 없다」 문장을 platform 을 가리키게. discovery 항목: WMI 띄우기 · 끄기가 platform 으로.
2. **CLAUDE.md 「플랫폼 중립」** — 선례 `console_command` 의 자리 · 「현황은 아직 못 미친다」의 찾는 법(`rg -l 'cfg!?\(windows\)' crates src-tauri`)을 §4-4 게이트로 바꾸고 예외(`fsutil.rs` — U-W 까지)를 적는다.
3. **CLAUDE.md 「핵심 불변식」** — kill 인과 줄의 `TerminateJobObject` 주체(무리 주인 `terminate`) · 소유권 분할 줄의 손잡이 정의 자리(D3 = B — `ProcessGroup` · `RetiringSignal` 이름은 그대로 · 파일은 agent `transport/process_group.rs` · 그 아래 OS 층 `GroupOwner` · `GroupRef` 는 platform) · 「Clone 아님」이 강제하는 범위(무리당 주인 하나 — 주인이 통로라는 것은 배치) · 잔여물 정리 락 규칙에 「platform 의 `terminate_raw` 는 OS 호출 하나 · 로그 없음」 한 구절.
4. **CLAUDE.md 「의존성」** — `windows` 항목: platform = 합집합 · agent = 예제용 dev-dependency · base · discovery = 없음. `Win32_Security` 경고는 platform 으로.
5. **CLAUDE.md 「빌드·검증 명령」 · 「네트워크 행 격리 게이트」** — platform 시험 줄 · 게이트 셋(①② · `test-support` 누수 ③) · base 시험 줄의 플래그 근거 · base ③ · 메시징 이름 게이트의 `platform` · net 2a(2 → 5) · net 2b 줄의 「`agent`에서 `base`로 이사하며 기대값 2가 crate 이름을 갈아탔고」(→ agent → base → platform) · net 3(base → platform) · 불변식 게이트.
6. **`.github/workflows/ci.yml`** — §4-2 · §4-3 의 스텝 · 기대값 · 주석(base 시험 스텝 주석의 「`platform` 스위트가 실 `cmd.exe` 를 띄운다」 · 메시징 이름 게이트 `:511-519` 포함).
7. **`.claude/skill-bindings/qa.md` · `docs/testing-strategy.md`** — qa: 4b · 4c · base 상한 · 메시징 이름 게이트(`:152`) · net 2a · 3 줄 + platform 게이트(4-pre 꼴 경로 가드) · base 링크 서술(`:137` · `:148` · `:307` — `net` 만 뺀다). testing-strategy 에도 게이트 사본이 따로 산다: base 플래그 근거(`:46` · `:138`) · base 링크 서술(`:49` — 같다) · base ③(`:50` · `:155`) · 메시징 이름 게이트(`:63`) · net 2a · 2b · 3(`:72` · `:73` · `:74`). 같은 단위에서 고친다(§4-3 「고칠 자리」).
8. **crate 헤더 · 매니페스트 주석** — base `src/lib.rs` 헤더(입주자 · 게이트 ③ · 「왜 바닥 crate 인가」 · `:35` 링크 서술에서 `net` 만 빼기) · base `Cargo.toml` description · net `src/lib.rs` 게이트 2 · 3 절 · agent `Cargo.toml:21` 주석 · `agent/src/transport/input_queue.rs:41` · `backend/codex/thread_lock.rs:6` · `process_tree` 머리 주석의 `engram_dashboard_base::platform` 경로 · ★루트 `Cargo.toml` 머리 주석(`:2` 「멤버 10」 · base = 「로깅·PID 판정」 · `:12-13` 「로깅 + PID 판정만 담는 잎 crate」)★ · ★net `Cargo.toml` 의존 규칙 주석(`:43-45` 「지금 그 조건을 만족하는 것은 protocol·base」 · `:73-77` 간선 이력 · `:135` 「base 의 `platform::` 헬퍼」)★ · ★discovery `Cargo.toml:19` 의 forward 폐포 `{net, base, protocol}`★ · discovery `Cargo.toml:12-14` 의 base 줄 주석(liveness 공유 사유는 platform 줄로 — base 의존 자체는 1-1 의 `time::Clock` 때문에 남는다).
9. **ADR** — 새 ADR-0275 에 §8 의 개정을 박고 개정당하는 ADR 에 도장을 찍는다(채번 · 도장 · 링크 = `/adr` 스킬 · 0274 는 다른 브랜치가 쓴다). ★**이 항목만은 U0 착수 전에 박는다 — 2026-10-04 에 박았다(1-1 몫과 한 ADR · §8)**★ — 각 단위 · U8 몫이 아니다(CLAUDE.md 「설계 결정 기록」 — 굵은 결정은 결정 즉시). 경로만 낡는 것(§8 끝)은 각 단위가 그 경로를 옮길 때 고친다.
10. **메모 `docs/refactoring/architecture-discussion-2026-09-26.md`** — §1 표의 줄 번호(§2-5) · 「셸에는 OS 분기가 없다」 · §11 첫 줄 처리 표시.
11. **코드 앵커** — `// ADR-0266` 을 platform crate 헤더와 무리 주인 · 손잡이 타입에(ADR-0266 「영향」). 그 둘(`GroupOwner` · `GroupRef`)과 agent 어댑터 `transport/process_group.rs` 머리에 `// ADR-0275` 를 더한다(어댑터는 지금의 `// ADR-0262` 와 함께).

---

## 7. 결정 (2026-10-04)

> **출처: 사용자 위임(2026-10-04 「알아서 진행해」) → 권고안 채택.** 셋 다 3판 §7 이 권고한 안 그대로다(글자 대조 — D1 (a) · D2 `engram-dashboard-platform` · D3 B). 3판은 D1 → D2 → D3 순으로 하나씩 물으려 했으나 위임으로 한 번에 닫혔다. ADR 로 박을 몫은 §8.

| | 결정 | 한 줄 이유 |
|---|---|---|
| **D1** | **(a) `windows` 바인딩을 기능별 feature 로 쪼개지 않는다** | 이득이 `-p` 단독 빌드에만 서고, 그 이득을 볼 「PID 만 쓰는 소비자」(net · discovery)가 3-3 · 2-2 에서 사라진다 |
| **D2** | **`engram-dashboard-platform`** | ADR-0266 · 메모가 이미 그렇게 부르고, 옛 모듈 이름(`base::platform` · `agent::platform`)과 같아 옛 기록에서 찾아가기 쉽다 |
| **D3** | **B — OS 층만 platform, 중립 손잡이 `ProcessGroup` · `RetiringSignal` 은 agent 어댑터(`agent/src/transport/process_group.rs`)** | ADR-0266 결정 2 · 5 와 맞고 platform 공개 API 에 통로 개념 · 트레이트 객체가 들어가지 않는다 |

### D1. `windows` 바인딩을 기능별 cargo feature 로 쪼갤지 (ADR-0266 「열린 것」) → (a)

- **고른 것:** platform 의 `windows` 의존 하나에 합집합 feature(§3-9). 대가 = PID 판정만 쓰는 쪽(net · discovery)을 **단독**으로 빌드할 때 Job · Restart Manager · WMI 바인딩까지 컴파일된다(컴파일 시간 — 링크는 안 쓰는 함수를 버린다 · 미측정 서술 · §9).
- **이유:** 워크스페이스 빌드 · `cargo test --workspace` 에서는 feature 가 한 번에 합쳐지고 agent 가 전부 켜므로 이득은 `-p` 단독 빌드에만 선다(ADR-0175 「거부한 대안」 옵션 C 와 같은 관찰 — 셸이 데몬을 dev-dependency 로 물어 합쳐진다). 그리고 ADR-0266 이 이름 붙인 「PID 만 쓰는 소비자」 둘이 곧 사라진다 — net 은 3-3 에서 걷히고, discovery 는 2-2 에서 데몬 · 셸로 나뉜다. 남는 소비자는 데몬(agent 를 통해 어차피 전부 켬)과 셸(WMI · 끄기 · PID 를 쓰고, 데몬을 dev-dependency 로 문다)뿐이다.
- **거부:**
  - (b) 기능별 feature(`process` · `group` · `file-holders` · `spawn` …) · 기본 비움(net `server` 선례) — platform 안에 `#[cfg(feature = …)]` 이 기능마다 붙고, CI 에 기능 조합 게이트(net 5a · 5b 꼴)가 늘고, 시험도 기능별로 갈린다. 얻는 것은 위 단독 빌드 시간뿐이다.
  - (c) 지금은 a, 조건이 서면 b(트리거 = agent 와 함께 링크되지 않는 소비자가 컴파일 비용을 재서 보일 때 · T- 로 적는다) — 그 소비자가 될 net · discovery 가 곧 사라져 지켜볼 대상이 남지 않는다.

### D2. crate 이름 → `engram-dashboard-platform`

- **이유:** 위 표.
- **거부:** `engram-dashboard-os` — 짧지만 `std::os` 와 뜻이 겹쳐 「std 확장」으로 읽힐 수 있다 · `engram-dashboard-sys` — ADR-0175 가 기각한 이름(Rust 에서 `-sys` 는 C FFI 바인딩 crate 관례).
- 접두 `engram-dashboard-` 는 고를 것이 아니었다 — 상한 게이트가 워크스페이스 멤버를 그 접두로 센다(§3-1). 이 이름이 §3-1 매니페스트와 §4-2 의 게이트 명령(`cargo tree -p engram-dashboard-platform …` · `-i engram-dashboard-platform`)에 그대로 들어간다.

### D3. 프로세스 그룹 손잡이의 모양 (§3-4) → B

- **고른 것:** §3-4 「고른 모양」 표 — platform = `windows.rs`(잎 · `LEFTOVER_EXIT_CODE` 뺌) + `GroupOwner` · `GroupRef` · 구체 타입 다섯 · agent = `cfg` 없는 어댑터 `transport/process_group.rs`(`ProcessGroup` · `RetiringSignal` · 두 트레이트) · agent `platform/` 폴더는 사라진다. 약한 손잡이는 통로가 `downgrade()` 의 `GroupRef` 에 물러남 표시를 얹어 agent 에서 만들고, 시험 도우미 `new_group` 도 어댑터 쪽에 남는다(platform `testing` 은 OS 수준 도우미 넷만 — §3-8).
- **이유:** ADR-0266 결정 2(OS 의존이 기준) · 결정 5(가짜용 트레이트는 쓰는 쪽)와 맞고 platform API 에 트레이트 객체가 없다. ADR-0262 · CLAUDE.md 의 이름이 agent 에 그대로 남는다. 어댑터를 통로 밑에 두는 것은 손잡이를 내주고 물러남 칸을 쥔 것이 통로라서다.
- **거부:** A — ADR-0266 결정 6 의 「한 덩이로」 문구대로 `windows.rs` + `process_group.rs` 를 platform 으로. 품은 가장 작지만 platform 공개 API 에 `RetiringSignal`(통로 개념) · 시험 seam 트레이트 · 트레이트 객체가 들어가고 `detached` 를 공개 생성자로 열어야 한다(§3-4 「왜 A 가 아닌가」).
- **대가:** ADR-0266 결정 6 의 「한 덩이로」 · 「새 손잡이를 따로 짓지 않는다」 두 문구를 고친다(§8 1 · 2) · agent 에 어댑터 약 100줄(어림 — §9).

**결정과 함께 확정되는 것(3판 「공통」 — 안과 무관해 묻지 않던 것):** 강한 주인 타입을 새로 두고 `Clone` 을 막는다(무리당 주인 하나만 강제) · `terminate_raw` 는 끝 코드를 인자로 · kill 인과 · drop 순서 · 락 규칙(§3-4 「공통 불변식」). 앞의 둘은 ADR 개정이 따른다(§8 2 · 3).

**문서 후속 범위가 B 로 정해졌다(§6 3 · 9)** — CLAUDE.md 「핵심 불변식」의 `ProcessGroup` · `RetiringSignal` 은 이름이 그대로이고 파일 자리만 agent `transport/process_group.rs` 로 바뀐다.

---

## 8. ADR-0275 에 박은 개정

> **새 번호 = ADR-0275**(0274 는 다른 브랜치가 쓴다). 근거 = 사용자 위임(2026-10-04 「알아서 진행해」) → 이 TRD 의 권고안 채택 · ADR-0266 결정 6 은 처음부터 메인 판단(사용자 위임)이었다. 채번 · `Amends` 링크 · 개정당하는 ADR 의 도장은 `/adr` 스킬이 한다. ★**2026-10-04 에 박았다 — U0 착수 전제(§6 9)는 채워졌다.**★ ADR-0275 는 이 TRD 몫(ADR-0266 · ADR-0262 개정 — 아래)과 형제 1-1 몫(ADR-0269 · ADR-0268 개정 — TRD 1-1 §4-2)을 함께 실은 **한 ADR** 이다. 거부한 대안은 이 TRD 가 따진 것만 옮긴다 — 지어내지 않는다(CLAUDE.md 「결정 날조 금지」).

1. **ADR-0266 결정 6 「`ProcessGroup` 과 Job Object 래퍼를 한 덩이로 옮긴 것이다」**(+ 그 근거 문장 「그 두 파일이 이미 그렇게 옮기도록 짜여 있다」)
   - **바뀌는 것:** OS 층만 platform 으로 간다 — `windows.rs`(잎) + 그 겉의 `GroupOwner` · `GroupRef` · 구체 타입 다섯. 중립 손잡이 `ProcessGroup` · `RetiringSignal` · 시험 seam 트레이트(`Pinned` · `Births`)는 agent 에 `cfg` 없는 어댑터로 남는다(`agent/src/transport/process_group.rs`). 하위 항목의 「약한 손잡이(`Weak<JobObjectHandle>` — `platform/process_group.rs:118-119`)」는 「platform `GroupRef`(agent `ProcessGroup` 이 감싼다)」로 읽는다.
   - **거부한 대안:** A — 문구대로 두 파일을 다 platform 으로(§3-4 「왜 A 가 아닌가」 · §7 D3).
   - **이유:** 판정 기준은 OS 의존이고(결정 2) `RetiringSignal` 은 통로의 물러남 칸 읽기라 OS 와 무관하다. 가짜용 트레이트는 쓰는 쪽에 둔다(결정 5). 결정 6 의 실질(강한 주인 = 통로 · 밖으로는 약한 손잡이 · 읽기 전용 표시 · 한 번 붙이는 포트만 · kill 인과 무변경)은 그대로다.
2. **ADR-0266 결정 6 「새 손잡이를 따로 짓지 않는다」**(+ 「pty · stdio · codex transport 가 `#[cfg(windows)]` 로 들고 다니는 Job 핸들은 이 손잡이 뒤로 들어간다」)
   - **바뀌는 것:** platform 이 강한 주인 `GroupOwner`(`Clone` 아님 — 무리당 강한 주인 하나만 강제 · Windows 밖 = 무동작) · 약한 손잡이 `GroupRef` · 구체 값 타입 다섯을 새로 짓는다(이름 = 이 TRD 가안 · 구현 때 확정). 통로 셋의 Job 칸은 약한 중립 손잡이가 아니라 **강한 주인 `GroupOwner`** 뒤로 들어간다. 「주인 = 통로」는 타입이 아니라 배치가 지킨다 — 사용량 조회의 트리 손잡이(`TreeRoot`)도 자기 무리의 주인이다(지금도 그렇다 — `usage/process.rs:438`).
   - **거부한 대안:** `GroupOwner` 를 「`JobObjectHandle` 의 겉만 바꾼 것」으로 읽어 개정 없이 감 — 공개 타입의 이름 · 모양 · 내부(`Arc` 를 안으로 넣는다)가 바뀌므로 그 해석에 기대지 않는다(§3-4).
   - **이유:** 통로에 샌 `#[cfg(windows)]` 칸을 없애려면(결정 1) 모든 OS 에서 같은 줄로 쥘 강한 타입이 있어야 하고, platform 은 agent 의 `ProcessGroup` 을 모르므로 그 강한 주인이 내주는 약한 손잡이도 platform 타입이어야 한다(D3 = B).
3. **ADR-0266 결정 6 의 근거(= ADR-0262 「[구현] 고른 것」 「통째로 옮길 수 있게」)와 결정 3 「도메인 지식은 남긴다」의 충돌**
   - **바뀌는 것:** `windows.rs` 는 통째로가 아니라 claude 잔여물 정리의 도메인 상수 `LEFTOVER_EXIT_CODE = 0x7440`(`windows.rs:53` — 잔여물 정리가 끝낸 프로세스에 주는 종료 코드, ADR-0262 결정 6 · G2)을 빼고 옮긴다. platform 의 `terminate_raw` 는 끝 코드를 인자로 받고, 상수는 agent 잔여물 정리 쪽으로 돌아간다 — **결정 3 이 이긴다.** ADR-0262 에도 이 항목으로 `Amended by` 를 단다.
   - **거부한 대안:** 상수째 통째로 옮김 — 도메인 값이 platform 에 들어간다(§2-5).
   - **이유:** 결정 3 은 OS 코드와 도메인 지식을 가르는 기준 자체다. 인자로 받아도 `terminate_raw` 는 OS 호출 하나 · 로그 없음으로 남아 잔여물 정리 락 규칙(ADR-0262 결정 8)이 그대로다. ★「통째로 옮길 수 있게」는 사용자 결정(2026-09-29)에서 온 글이다★ — 그 실질(`windows.rs` 는 `windows` crate 와 io 만 쓰는 잎)은 그대로이고, 바뀌는 것은 상수 하나가 인자로 나가는 것뿐이다.
4. **ADR-0266 「영향」 「열린 것 — `windows` crate feature 를 기능별 cargo feature 로 나눌지」**
   - **바뀌는 것:** 닫는다 — 나누지 않는다(D1 (a)). platform 의 `windows` 의존 하나에 합집합이고, platform 의 cargo feature 는 `test-support` 하나다.
   - **거부한 대안:** (b) 기능별 feature · 기본 비움 · (c) 지금 a, 조건이 서면 b(§7 D1).
   - **이유:** §7 D1.
5. **ADR-0266 의 crate 이름 자리채움**(「영향」 게이트 명령의 `<platform>`)
   - **바뀌는 것:** `engram-dashboard-platform` 으로 채운다(D2).
   - **거부한 대안:** `engram-dashboard-os` · `engram-dashboard-sys`(§7 D2).
   - **이유:** §7 D2.
6. **ADR-0266 「영향」 불변식 「`#[cfg(windows)]` · `cfg!(windows)` 는 platform crate 안에만」과 결정 8 의 시점(「이 단계(1-3)로 온다 — platform 이 생긴 뒤에 합친다」)** — D1~D3 이 아니라 이 TRD 의 범위 결정(사용자 「알아서」 2026-10-03 → 메인 권고안 · §1)에서 온다.
   - **바뀌는 것:** 시한부 예외 하나 — 셸 `src-tauri/src/fsutil.rs:107` 의 `cfg!(windows)` 는 U-W(storage P3 가 master 에 착지한 뒤)까지 남고, 그 사이 불변식 게이트(§4-4)의 기대 명단에 든다. 결정 8 의 일(원자적 쓰기 여러 벌을 platform `fs` 로 합치기)은 1-3 안이지만 P3 뒤로 간다.
   - **거부한 대안:** 지금 옮김 — storage P3 이 같은 파일을 크게 바꾼다(`write_atomic_unless` · `copy_atomic` · `copy_aside` · `retry_denied` 신설 — §2-4).
   - **이유:** P3 앞에서 옮기면 P3 착지 때 같은 파일을 다시 맞춰야 한다.

**고칠 글이 없는 것(확인함):**

- **ADR-0175** — 결정 1 · 2 와 「거부한 대안」 첫 항목의 platform 쪽은 ADR-0266 이 이미 개정했다. D1 (a) 는 그 ADR 의 옵션 C 기각과 같은 관찰(feature 는 워크스페이스에서 합쳐진다)에 서고, D2 는 그 ADR 의 `-sys` 기각을 따른다 — 어긋나는 문구가 없다.
- **ADR-0218** — 결정 11 의 거처(agent `platform`)는 ADR-0266 이 이미 개정했다. 이 TRD 는 그 위에서 `file_holders` · `process_tree`(→ platform `process::subtree` · 이름 가안)의 자리만 정하고, 락 디렉터리 · 「파일 이름 = 스레드 id」 지식은 `backend/codex` 에 남긴다(결정 11 의 그 절반 · ADR-0266 결정 3) — 어긋나는 문구가 없다.

**경로 · 사실만 낡는 것 — 개정이 아니다**(ADR-0275 「근거」에 사실로 적고 옛 ADR 본문은 고치지 않는다 · 코드 쪽은 `// ADR-` 앵커가 따라간다):

- ADR-0262 「OS 조각」 · 「[구현] 고른 것」의 `platform/windows.rs` · `process_group.rs` 경로 → platform `group/windows.rs` · agent `transport/process_group.rs`.
- ADR-0230 「관련」 · 「영향」의 선례 경로 `backend/mod.rs:44`(`console_command`) → platform `shell::console_command`(이름 가안).
- ADR-0218 「관련」의 `crates/engram-dashboard-agent/src/platform` → platform crate.
- ADR-0266 「맥락」 · 재실측 목록에 「창 없이 띄우기」(`CREATE_NO_WINDOW`) 세 벌이 없다(§2-1 · §2-5) · 「영향」의 「`windows` feature 목록이 crate 별로 옮겨 간다」는 agent 에서 다 떠나지 않는다 — 예제 둘 때문에 dev-dependency 로 남는다(§2-5 · §3-9) · 결정 6 의 `Weak<JobObjectHandle>` 줄 번호는 `117-118`(§2-5).

---

## 9. 미검 · 열린 것

- **기준선 회귀 수를 이 판에서 돌려 보지 않았다**(§5-2 — U0 착수 직전에 잰다).
- **discovery 의 `Win32_System_Threading` · `Win32_Foundation` feature 가 WMI 코드에 필요한지** — 미검(U7 에서 빼 보고 컴파일로).
- **agent 예제가 요구하는 최소 `windows` feature** — 미검(`Win32_Security` 포함 여부).
- **POSIX 갈래는 옮긴 뒤에도 컴파일되지 않는다** — 개발 · CI 가 Windows 뿐이다(ADR-0230 현황). 특히 `TreeRoot` 의 POSIX 갈래는 처음부터 컴파일된 적이 없다.
- **`feature` 를 안 쪼갤 때의 컴파일 비용** — 미측정 서술(D1 (a) 의 대가). 결정은 이 수치에 기대지 않는다(이유 = 소비자가 사라진다 — §7 D1).
- **storage P3 착지 시점** — U-W 트리거. P3 이 `fsutil.rs` 에 더한 함수 중 무엇이 「원자적 쓰기」 가족인지는 그때 다시 잰다.
- **1-1 형제 TRD 의 base 헤더 최종 글** — 1-1 의 결정 넷(S2 · N1 · C4 · C5)은 이 판에 맞췄다(§5 선행). 다만 1-1 이 이 판과 나란히 고쳐지는 중이라 그 최종 글은 보지 않았다 — U1 의 base 편집은 1-1 머지 뒤의 모양 위에 얹는다.
- **어댑터의 줄 수(약 100줄)** — 어림이고 재지 않았다(자리는 D3 결정으로 정해졌다 — `agent/src/transport/process_group.rs`).
- **`LEFTOVER_EXIT_CODE` 의 agent 쪽 자리** — 어댑터의 `impl Pinned` 가 넘길지, 시험 seam 트레이트 `Pinned::terminate_raw` 에 끝 코드 인자를 붙여 leftover 가 넘길지는 U3 이 정한다(지금 트레이트는 인자가 없다 — `process_group.rs:86` · 부르는 곳 `leftover.rs:2231`). platform 으로 옮겨 가는 실프로세스 시험이 지금 그 상수를 단언하므로(`process_group.rs:543 · 804`) platform 쪽에서는 시험 자기 값으로 바꾼다.
- **불변식 게이트(§4-4)의 기대 명단** — U8 에서 그 명령으로 다시 잰다(단위들이 시험을 옮기며 명단이 줄어든다).
