# TRD — Boundary refactoring 1-3: gather OS code into the `platform` crate (S21)

> Status: **4th edition (2026-10-04) — decisions reflected (§7 · §8).** Not a single line of code has changed yet.
> ★**Start order — 1-3 starts only after 1-1 (base generic helpers) is merged**★ (the files overlap heavily — §5 Prerequisites · §5-3). **Unifying atomic writes (`write_atomic`) waits until storage P3 lands on master** (§5 U-W).
> **Revision (2026-10-04, decisions reflected):** ① Turned the three choices of §7 into a decision record — user delegation (2026-10-04 「알아서 진행해」) → adopted the 3rd edition's recommendation: **D1 = (a) do not split the `windows` feature · D2 = `engram-dashboard-platform` · D3 = B (only the OS layer in platform · neutral handle `ProcessGroup` · `RetiringSignal` in the agent adapter `agent/src/transport/process_group.rs`)** ② Aligned the whole document to the single branch D3=B — the description of branch A was reduced to one paragraph, 「왜 A 가 아닌가」, in §3-4 (§2-2 · §2-5 · §3-4 · §3-8 · §5-1 U3 · §5-2 · §5-3 · §6) ③ Newly wrote the list of amendments to be recorded in ADR-0275 (§8 — 0274 is written by another branch) ④ Aligned the four decisions of the sibling 1-1 (S2=(e) reclaim without a warning · N1 feature name `test-support` · C4=(a) the fake clock is inside base `time` · C5=(b) transport → base goes to stage 3) to this document (§3-8 · §4-3 · §5 Prerequisites · §5-1 U5) ⑤ Moved 「미검 · 열린 것」 to §9.
> **Revision (2026-10-03, 2nd-round review reflected):** ① Wrote out the weak-handle creation path of D3=B to the end — the stdio `process_group()` puts `RetiringSignal::of(&self.retiring)` on top of `GroupRef` of `GroupOwner::downgrade()` and wraps it in the agent adapter (without the retiring flag, leftover cleanup loses its stop condition) · if B, the test helper `new_group` stays in the agent adapter's test support and platform `testing` exports only the four OS-level helpers · also wrote the same place for A (§2-2 · §2-5 · §3-4 · §3-8 · U3 · §7) ② discovery still uses base after 1-3 — 1-1, which lands first, makes discovery `Clock` a sub-trait of base `time::Clock` (ADR-0269 decision 3-2). What drops out of the 「링크된다」 description of base is only `net` (§4-1 · §4-3 · §5-1 U1 · §6) ③ Stated the `codex/transport.rs` overlap with 1-1 by 「1-1 의 U3b 범위」 instead of by count (§5 Prerequisites) ④ U3's `usage/process.rs` part = five `crate::platform` references (three operational · two test) (§5-1) ⑤ Added `thread_lock.rs:1236` to the U1 pointers (§5-1) ⑥ Added the CLAUDE.md net gate 2b line to the places to fix (§4-3 · §6).
> **Revision (2026-10-03, 1st-round review (`/review trd full`) reflected):** ① U4 deletes only its own two files; the agent `platform/` cleanup and `windows` → dev-dependency fall to U3, which comes later — the adapter's location for B = `agent/src/transport/process_group.rs` (proposal · §3-4 · §5-1) ② Added a source-string test (the guard-placement test of `codex/transport.rs`) to U3 (§5-1) ③ Wrote the poison handling and logging of the lookup list (`LookupGate`) into U5 (§5-1 · §3-7) ④ Invariant gate: `-g '*.rs'` · allow one level of parentheses · corrected the basis of the expected list · limits addendum (§4-4) ⑤ Added gate copies and comments that go stale — `docs/testing-strategy.md` · `platform` in the messaging name gate · root · net · discovery manifest comments (§4-3 · §6) ⑥ Creating `GroupOwner` is an amendment of ADR-0266 decision 6 under both options · corrected an overstatement of 「Clone 아님 = 통로 하나 강제」 (§0 · §3-4 · §7) ⑦ Keep the old R7 (`daemon/src/experiment`) because it is harness-only (§2) ⑧ `test-support` operational leak gate · `testing` module `cfg(any(test, …))` (§3-8 · §4-2) ⑨ Corrected the 1-1-side name in base gate ③ (`file` is deferred by 1-1 · §4-3).
> **Scope = everything in work-order 1-3 except unifying atomic writes (`write_atomic`)** (`docs/refactoring/architecture-discussion-2026-09-26.md` §10 — atomic writes come after storage P3 lands, §5 U-W).
> **Placement rationale:** `docs/README.md`, 「새 내용을 어디에 넣나」: 「새 기능 **설계 착수** → `process/SN-name/` 새 폴더」. The sibling document of the same stage (the 1-1 base shared-function TRD) sits in this folder as well.
> **Notation:** 「실측」 = what was measured at this edition's base commit `0ef6292` (branch `v0.3.3/refactor/crate-boundaries`), and the command used · 「사용자 결정(날짜)」 = only user decisions recorded in a document · 「결정(2026-10-04)」 = the three items of §7 for which the recommendation was adopted by user delegation (always written in the form 「결정 D1」 · 「§7 D1」 — not the same as the row ids `D1` · `D2` (discovery's shutdown · WMI) in the §2-1 table) · 「ADR」 = the body of a confirmed ADR · 「제안」 = this TRD's proposal (apart from the crate name, all names are drafts).
> Anchors: **ADR-0266** (the charter of this crate) · **ADR-0275** (the decision record of this TRD · amends ADR-0266 decision 6 — what was recorded = §8 · recorded on 2026-10-04 together with the 1-1 part as one ADR) · ADR-0268 (platform calls `tracing` directly · does not depend on base) · ADR-0269 (atomic writes go to platform — decision 8 side · base tenancy condition) · ADR-0175 (reverses the platform side of decisions 1·2 and of the first item of the rejected alternatives) · ADR-0218 decision 11 (reversed) · ADR-0262 (`ProcessGroup` neutral handle · leftover-cleanup lock rule) · ADR-0230 (platform neutrality) · ADR-0024 · ADR-0271 (discovery split = 2-2 · WMI belongs to platform) · ADR-0001 (kill causality) · ADR-0012 (module isolation harness) · step-log S21 · `docs/tracking.md` T-47.

---

## 0. Conclusion (first)

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

**Not doing now:** unifying atomic writes (after storage P3 lands — §5 U-W) · building non-Windows implementations (ADR-0266 decision 7) · closing the gap 「띄운 뒤 Job 에 넣는」 (spawn, then add to the Job) in the links (a behavior change — §1).

---

## 1. Goals · non-goals

### What we do

- Move all **operational** code that varies by OS into the new crate (ADR-0266 decision 1 — the criterion for the decision = whether it depends on the OS, decision 2).
- Remove the branches that leaked into callers (the `#[cfg(windows)]` Job field of the links).
- Move non-Windows behavior **exactly as it is now** (list in §3-6). Do not turn placeholders into real implementations (ADR-0266 decision 7).
- Set up gates (§4).

### What we do not do

- **Unifying the several copies of atomic write (`write_atomic`)** — after storage P3 lands on master (user 「알아서」 2026-10-03 → main-session recommendation · memo §10 way of proceeding). Until then `cfg!(windows)` in shell `src-tauri/src/fsutil.rs` remains as **one known operational exception** (§4-4).
- **Moving domain knowledge** — the `.claude` path · codex home subfolders · lock file name = thread id · claude's `taskkill` hook-shape decision (`leftover.rs` `taskkill_names`) stay as they are (ADR-0266 decision 3).
- **The links' 「띄운 뒤 넣기」 gap** — `transport::pty` · `transport::stdio` · the codex link put the child into the Job **after** launching it, so a grandchild can slip out in between (the head comment of `agent/src/platform/windows.rs` admits it). Only the usage query launches the child suspended and then adds it. This time we only move the location and keep behavior the same.
- **discovery split (2-2) · stripping net (3-3)** — here we only change those crates to call platform.

---

## 2. Current state, measured — list of OS branches

**Command (measured `0ef6292`):** `rg -l 'cfg!?\(windows\)|cfg\(not\(windows\)\)|cfg\(target_os|cfg!\(target_os|cfg\(unix\)|cfg!\(unix\)|target_family' crates src-tauri` → 45 files (4 manifests included). Then for each file, `rg -n '^\s*#\[cfg\(test\)\]'` was used to find the start line of the test module, and each branch was classified as operational or test. ★ADR-0266's command (`cfg!?\(windows\)|cfg\(not\(windows\)\)`) cannot match `#[cfg(unix)]` · `cfg_attr(not(windows), …)`★ — the affected files matched on other lines, so nothing was missed this time, only by luck. Also, places that decide the OS at runtime without `cfg` match none of the regexes, so they were searched separately (`rg 'USERPROFILE|"HOME"|creation_flags|taskkill|share_mode'`) — one turned up (R7 of the 1st edition), and it stays because it is harness-only (§2-3).

### 2-1. To move — operational code

| id | Location (measured `0ef6292`) | What | Required `windows` features | Unit |
|---|---|---|---|---|
| P1 | `crates/engram-dashboard-base/src/platform.rs` 1-280 | PID liveness · start time, 3 variants (`ProcessStart`) · process table · child PIDs | `Win32_Foundation` · `Win32_System_Threading` · `Win32_System_Diagnostics_ToolHelp` | U1 |
| R1 | `agent/src/manager.rs:77-84` `default_shell` | default shell (`cmd.exe` / `bash`) | none (std) | U2 |
| R2 | `agent/src/backend/mod.rs:46-60` `console_command` | wrapping the CLI in `cmd.exe /c` | none | U2 |
| R3 | `agent/src/backend/mod.rs:138` | case-insensitive `PATH` keys | none | U2 |
| R4 | `agent/src/backend/claude/mod.rs:1829-1836` `claude_home` | home directory (`USERPROFILE` / `HOME`) | none | U2 |
| R5 | `agent/src/backend/codex/thread_lock.rs:230-238` `user_home` | home directory — the second copy of R4 | none | U2 |
| R6 | `daemon/src/lib.rs:109` `locate_send_exe` · `discovery/src/lib.rs:987` `locate_daemon_exe` | appending `.exe` to executable names | none | U2 |
| P4 | `agent/src/platform/file_holders.rs` | the processes that have this file open (Restart Manager) · `RmSession` `Drop` warning log | `Win32_System_RestartManager` · `Win32_Foundation` | U4 |
| P5 | `agent/src/platform/process_tree.rs` | identity list under a root PID (assembles the two P1 functions — its own `cfg` is for tests only) | none (on top of P1) | U4 |
| P2 | `agent/src/platform/windows.rs` 1-713 | Job Object wrapper · member list · holding · join-notification port · `resume_suspended_process` · `LEFTOVER_EXIT_CODE` | `Win32_System_JobObjects` · `Win32_Security` · `Win32_Foundation` · `Win32_System_Threading` · `Win32_System_IO` · `Wdk_System_Threading` · `Win32_System_Diagnostics_ToolHelp` | U3 |
| P3 | `agent/src/platform/process_group.rs` 1-313 | the OS branch of the ADR-0262 neutral handle `ProcessGroup` | (on top of P2) | U3 |
| P6 | `agent/src/platform/mod.rs:18-24` | `#[cfg(windows)]` re-export | — | U4 = the two mod declarations of `file_holders` · `process_tree` · U3 = the rest, and the file · folder itself (§5-1) |
| U | `agent/src/usage/process.rs` 19-476 | launch flags (no window + suspended / POSIX new group) · tree kill (Job / `kill -s KILL -- -<pgid>`) · 「프로그램 없음」 decision (cmd 9009 + `PATH`·`PATHEXT` lookup · remote-path decision · lookup list · deadline) · case-insensitive child env | none (std) + `resume_suspended_process` of P2 | U5 |
| N | `net/src/instance.rs:166-228 · 289 · 306-315` | opening in shared mode 「읽기만」 · raw errors 32 · 5 · non-Windows `Unsupported` · 4 lines of `cfg_attr(not(windows), allow(dead_code))` | none (std `OpenOptionsExt::share_mode`) | U6 |
| D1 | `discovery/src/lib.rs:911-931` `TaskKiller::kill` | `taskkill /PID … /F /T` · an error outside Windows | none (std) | U7 |
| D2 | `discovery/src/lib.rs:1060-1323` | COM init classification · `ComGuard` · `wmi_spawn` · `wmi_create_raw` · `read_u32_prop` · `wmi_err` | `Win32_System_Com` · `Win32_System_Wmi` · `Win32_System_Rpc` · `Win32_System_Variant` (+ whether `Win32_System_Threading` · `Win32_Foundation`, which discovery currently lists alongside, are needed by this code is **unchecked** — remove them in U7 and decide by compiling) | U7 |

**「창 없이 띄우기」 (`CREATE_NO_WINDOW`) is a fourth rule that was not in ADR-0266's list of OS rules** — the same constant exists in three copies in `agent/src/transport/stdio.rs:119-124` · `agent/src/backend/codex/transport.rs:1445-1450` · `agent/src/usage/process.rs:75-84` (measured). In the unit where each file moves (U3 · U5), it is replaced with one platform function.

### 2-2. Branches that leaked into callers — remove them

| Location | Now | After U3 |
|---|---|---|
| `agent/src/transport/pty.rs:21 · 59-60 · 91-98 · 118-119 · 428-431` | `#[cfg(windows)] job_handle: JobObjectHandle` · create when opening and insert · line 4 of `shutdown` `terminate(1)` | `group: GroupOwner` (draft) — the same line on every OS |
| `agent/src/transport/stdio.rs:36-38 · 88-89 · 136-143 · 156-157 · 166-178 · 489-492` | the same field + the two `cfg` branches of `process_group()` | the same field · `process_group()` is a single expression without `cfg` — bundle the `GroupRef` of `self.group.downgrade()` and `RetiringSignal::of(&self.retiring)` into the agent adapter `ProcessGroup::new` (D3 = B · §3-4 「약한 손잡이 만들기」). Outside Windows `downgrade` is `None`, so the answer is the same as now. ★Handing out only `downgrade()` without the retiring flag is not allowed★ — the leftover cleanup's listening thread (`leftover.rs:759`) and the pre-finish recheck (`recheck` · `:2345`) would lose their stop condition |
| `agent/src/backend/codex/transport.rs:183-184 · 1395-1396 · 1464-1471 · 1494-1495 · 4239-4242` | the same field (`Arc` — no weak handle is handed out) | the same field |

### 2-3. To keep

- **Files with test branches only (시험 분기뿐인 파일)** (every branch is under that file's `#[cfg(test)]` · `#[cfg(all(test, windows))]` module · `tests/`): agent `backend/claude/{leftover.rs, mod.rs, usage_probe.rs}` · `backend/codex/{mod.rs, transport.rs(6198~)}` · `backend/codex/thread_lock.rs:1222` · `backend/mod.rs:1595` · `manager.rs:3063~` · `session.rs:1213` · `usage/scratch.rs` · `transport/pty.rs:682 · 783` · `transport/stdio.rs:559~` · base `logging/mod.rs:618` · daemon `messaging_host.rs` · net `portfile.rs` · net `instance.rs:319 · 333` (test module) · discovery `lib.rs:1553 · 2754 · 2828` · agent 8 · daemon 1 · discovery 1 files under `tests/`. ★The first `#[cfg(test)]` (1059) of `backend/claude/mod.rs` is not a module but a single item★ — cutting at 「첫 `#[cfg(test)]` 뒤 = 시험」 misclassifies operational branch R4 (1829) as a test. This table cuts at the start line of the test **module** (1967).
- **Harness bins · examples · harness modules** (ADR-0266 「근거」 — not included in operational builds): daemon `src/bin/roundtrip_smoke.rs:905` · `src/bin/saturation_pilot.rs:1657` (both `required-features = ["test-harness"]`) · agent `examples/spike.rs` · `examples/spike_breakaway.rs` · ★daemon `src/experiment/transcript.rs:69-70` `claude_projects_dir` (R7 of the 1st edition — a third copy of the home directory, chosen at runtime in the order `USERPROFILE` → `HOME` without `cfg`)★. The 1st edition counted this as operational, but the entire `experiment` module sits behind `#[cfg(feature = "test-harness")]` (`daemon/src/lib.rs:17-18` · the only caller = `saturation_pilot`). Since the rule is to leave harnesses in place, we do not move it — moving it would follow the meaning of R4 · R5 (Windows = `USERPROFILE` only) and remove the `HOME` fallback on Windows.
- **Binary attributes · build scripts:** the `windows_subsystem` of `daemon/src/main.rs:9` · `src-tauri/src/main.rs:2` (a crate-level linker attribute — cannot be moved to another crate) · `src-tauri/build.rs` (host ≠ target, so it reads `CARGO_CFG_TARGET_OS` at runtime rather than `cfg` — ADR-0174).
- **Comments only:** net `src/lib.rs:44 · 91` (quotes `cfg(unix)` in prose).

### 2-4. Deferred

- **`src-tauri/src/fsutil.rs:107`** — the rename retry of `write_atomic` sees Windows sharing · lock violations (raw 32 · 33). Storage P3 (`origin/v0.3.3/feat/storage` — not landed on master, measured: `git merge-base --is-ancestor` is false · 2026-10-03) changes this file heavily (`write_atomic_unless` · `copy_atomic` · `copy_aside` · new `retry_denied` · the same `cfg!(windows)` decision remains as `is_lock_contention` — `git show origin/v0.3.3/feat/storage:src-tauri/src/fsutil.rs`). → §5 U-W.

### 2-5. Facts that disagree with the records

| Record | Actual (measured `0ef6292`) |
|---|---|
| Memo §1 · ADR-0230 「영향」: `daemon/src/lib.rs:96,146` (app · CLI, two places) | There is **one** operational branch, `daemon/src/lib.rs:109` (the CLI). There is no place that branched on the app name. |
| Memo §1: `discovery/src/lib.rs:1119-1320` · `:981` | `1060-1323` (starting with the COM classification) · `987`. |
| Memo §1: `agent/src/manager.rs:75` · `backend/claude/mod.rs:1012` · `backend/mod.rs:44,136` · `codex/transport.rs:177` | `77` · `1829` · `46-60, 138` · `183`. |
| Memo §1: 「셸(`src-tauri/src`)에는 OS 분기가 없다」 | There is a `src-tauri/src/fsutil.rs:107` operational branch (ADR-0266's re-measurement already reflects it — only the memo is stale). |
| ADR-0266 「맥락」 · re-measurement list | The three copies of 「창 없이 띄우기」 (§2-1) are not in the list. (The third copy of the home directory, `daemon/src/experiment/transcript.rs:69`, is not there either, but since it is harness-only it matches the harness rule in ADR 「근거」 — §2-3.) |
| ADR-0266 decision 6 「`platform/process_group.rs:118-119`」 | The `Weak<JobObjectHandle>` field is `117-118` (trivial). |
| 「agent 의 `windows` feature 가 platform 으로 옮겨 간다」, which CLAUDE.md 「의존성」 implies | agent `examples/spike*.rs` uses the `windows` crate **directly** (`rg -l 'windows::(Win32|Wdk|core)' crates`) — since `cargo test` compiles the examples, agent must keep `windows` **as a dev-dependency** (§3-9). `spike.rs` also uses `portable-pty`, so it cannot move to platform. |
| ADR-0266 decision 6 「Job Object 래퍼와 `ProcessGroup` 을 한 덩이로 옮긴다」 | It does not end with two files — leftover's real-process tests use `crate::platform::process_group::tests::{new_group, open_gate, spawn_gated_cmd, wait_until, is_ping}` and the `#[cfg(test)]` constructor `ProcessGroup::detached` (`leftover.rs:4221 · 6571` · `claude/mod.rs:6357-6359`). Across the crate boundary `#[cfg(test)]` is not visible, so the four OS-level helpers (`spawn_gated_cmd` · `open_gate` · `wait_until` · `is_ping` — currently `windows.rs:729-758`) are opened with a feature flag. `new_group` and `detached` build the agent-side `ProcessGroup` · `RetiringSignal`, so they stay in the agent adapter's test support (D3 = B · §3-8). |
| Move `windows.rs` as a whole (ADR-0262 「[구현] 고른 것」 — user decision 2026-09-29 「windows crate 와 io 만 · 통째로 옮길 수 있게」) | That file holds the **domain constant** `LEFTOVER_EXIT_CODE = 0x7440` (`windows.rs:53`) of claude leftover cleanup, and `terminate_raw` hardcodes it (`:405-406`). Moving it whole would put a domain value into platform (ADR-0266 decision 3) → change it to take the value as an argument and send the constant back to the agent leftover-cleanup side (§3-4 · ADR-0275 amendment — §8). |

---

## 3. Crate design

### 3-1. Name · location · manifest

- **Name = `engram-dashboard-platform` (decision D2 · §7)**, location `crates/engram-dashboard-platform/`. The prefix `engram-dashboard-` is mandatory — the cap gate counts workspace members by that name prefix (ADR-0175 impact · ADR-0151 「개명 함정」).
- Add it to `members` of the root `Cargo.toml`.
- Manifest (proposal):

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

### 3-2. Module layout (proposal)

| Module | Contents | Source |
|---|---|---|
| `process` | all PID decisions · `subtree` (identities under a root) · `kill_tree(pid)` (`taskkill /F /T`) | P1 · P5 · D1 |
| `group` (+ private `group/windows.rs`) | group owner · group handle · held members · join-notification port · value types · `resume_suspended_process` | P2 · the OS branch of P3 |
| `file_holders` | the processes that have this file open | P4 |
| `spawn` (+ private `spawn/wmi.rs`) | launching without a window · preparing the tree root (suspended / new group) · tree handle · 「프로그램 없음」 decision · launching outside the Job (WMI) | §2-1 third rule · U · D2 |
| `shell` | `default_shell` · `console_command` | R1 · R2 |
| `env` | `home_dir` · `exe_file_name` · `env_key_eq` | R3 · R4 · R5 · R6 |
| `fs` | 「읽기만 공유」 opening · sharing violation · access-denied decision (+ `write_atomic` at U-W time) | N |
| `testing` (behind `test-support`) | real-process test helpers | the test modules of P2 · P3 |

`group/windows.rs` keeps the current `windows.rs` almost as it is (「`windows` crate 와 std 만」 leaf rule kept — ADR-0262). This layout was chosen to avoid three name collisions: ① agent's ADR-0262 `ProcessGroup` (the weak neutral handle) ② the usage query's private `ProcessTree` (the strong tree-kill handle — `usage/process.rs:437 · 471`) ③ agent's `platform::process_tree` module (identity-list query). All three are 「프로세스 무리/트리」, but they are different things, so on the platform side they are called by separate names: `group::GroupOwner` · `group::GroupRef` · `spawn::TreeRoot` · `process::subtree` (draft). It also does not collide with std's POSIX `CommandExt::process_group` (pgid).

### 3-3. Public interface (draft — functions + handle types, compile-time branching)

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

- **Takes no domain types** — instead of the usage query's `ProbeCommand` it takes raw values (program · arguments · env set/remove · working folder). Instead of discovery's `DiscoveryError` it returns `io::Error` / `DetachedSpawnError`, and discovery converts that into its own error (`SpawnFailed { rv }` from `Refused { rv }`).
- **The callers' test seams stay with the callers** (ADR-0266 decision 5): discovery `PidLiveness` · `Spawner` · `ProcessKiller` (`lib.rs:417 · 426 · 677` — only the real implementations `RealLiveness` · `WmiSpawner` · `TaskKiller` change to call platform functions) · thread_lock `LockHolderProbe` (`:49`) · leftover `Group` (`:522`) · usage `ProbeSpawner`.
- `default_shell` is currently called by daemon operational code (`daemon/src/connection_core.rs:29`) and several tests through the public path `engram_dashboard_agent::manager::default_shell`. **Leave no re-export and change the callers to the platform path** (proposal — do not keep two names for one thing).
- The meaning of `home_dir` is that of R4 · R5 unchanged (Windows = `USERPROFILE` only). We do not replace the harness's third copy (`experiment/transcript.rs` — `USERPROFILE` → `HOME` on every OS) with this function (§2-3).
- Do not replace it with `std::env::home_dir` — on Windows, if the environment variable is absent it falls through to the OS API, which differs from the current meaning.

### 3-4. Process-group handle (decision D3 = B · §7)

**Current shape (measured):**

- Strong owner = `JobObjectHandle` (`windows.rs:140` — `pub`). pty holds it by value, stdio · codex hold it as `Arc`, and only stdio hands out a weak handle as `Arc::downgrade` (`stdio.rs:166-178`).
- Weak neutral handle = `ProcessGroup { job: Weak<JobObjectHandle>, retiring: RetiringSignal }` (`process_group.rs:116-120`) — list · hold · join notification · 「사라졌나」. Outside Windows: empty list · cannot hold · port `Unsupported` · always 「사라짐」.
- Test seams: the held-member `Pinned` and port `Births` traits (`process_group.rs:79 · 99`) are in platform, and the group trait `Group` is in the caller, leftover (`leftover.rs:522`). The fakes `FakePin` · `FakePort` · `FakeGroup` · `GateGroup` · `IdlePort` are all in the leftover tests.

**Chosen shape (고른 모양) — B: only the OS layer in platform, the neutral handle in the agent adapter** (decision 2026-10-04 · §7 D3):

| Location | What |
|---|---|
| platform `group` (+ private `group/windows.rs`) | `windows.rs` (leaf — only the domain constant `LEFTOVER_EXIT_CODE` is removed) + on top of it the strong owner `GroupOwner` · weak handle `GroupRef` · concrete types `PinnedMember` · `BirthPort` · `MemberFacts` · `PortOutcome` · `MemberOutcome` (all names are drafts). **There is no trait object in the public API** — only concrete types |
| agent `transport/process_group.rs` | what is now `platform/process_group.rs` moves over as an adapter without `cfg` — `ProcessGroup { group: GroupRef, retiring }` · `RetiringSignal` · two traits (`Pinned` · `Births`) · `impl Pinned for` held members · `impl Births for BirthPort` · carrying values across (trait objects are created only inside this). Why it sits under the links = the link is what hands out that handle (`StdioTransport::process_group` — `stdio.rs:166`) and the retiring field that `RetiringSignal` reads is also owned by the link (CLAUDE.md 「소유권 분할」). **The names in ADR-0262 · CLAUDE.md 「핵심 불변식」 (`ProcessGroup` · `RetiringSignal`) stay in agent as they are; only the paths change** |
| agent `platform/` folder | disappears — together with `lib.rs` `pub mod platform;` (zero uses of `engram_dashboard_agent::platform` from outside crates — measured rg). If a module named `platform` were left in agent, its name would overlap with the platform crate and read as 「OS 코드가 아직 여기 있다」 |

- **Strong owner `GroupOwner` (draft)** — `new() -> io::Result<Self>` · `adopt(pid)` · `terminate(exit_code)` · `active_processes()` · `downgrade(&self) -> Option<GroupRef>`. **It is not `Clone`** — what the type enforces goes only as far as 「한 무리의 강한 주인은 하나」. 「그 주인이 통로다」 (ADR-0266 decision 6 · ADR-0262 impact 「소유권」) is kept by placement, not by type — besides the three links, the usage query's tree handle (currently the `job` field of `usage/process.rs:438` · `TreeRoot` after U5) also holds one owner of its own group. Outside Windows it is an owner that does nothing (`new` · `adopt` · `terminate` = `Ok`, `active_processes` = `Unsupported`, `downgrade` = `None` — the same answer as the `None` branch of the current `stdio.rs`).
- **`GroupRef` (weak)** = the OS branch of the current `ProcessGroup` — `member_pids` · `pin_member(pid, kill)` · `watch_births(start)` · `unwatch_births` · `is_gone` · the test-only `GroupRef::gone()`. The meaning of the answers is that of the two branches of `process_group.rs` unchanged (gone = `Ok(빈 목록)` · `Ok(None)` · `Err(GROUP_GONE)`).
- **Creating the weak handle (약한 손잡이 만들기)** (stdio `process_group()` — currently `stdio.rs:166-178`) — `GroupOwner::downgrade()` yields only the group handle. platform does not know the retiring flag. The link combines them in agent: `self.group.downgrade().map(|g| ProcessGroup::new(g, RetiringSignal::of(&self.retiring)))` (adapter `ProcessGroup::new(GroupRef, RetiringSignal)` — without `cfg`). ★Reducing it to just `downgrade()` drops the flag★ (§2-2).
- **Test seam** — `detached` stays in agent as it is (it is built with `GroupRef::gone()` — hence `gone()` is opened in platform behind `cfg(any(test, feature = "test-support"))`). `new_group` also stays in the agent adapter's test support — platform cannot build agent's `ProcessGroup` · `RetiringSignal` (§3-8).
- **`terminate_raw` takes the exit code as an argument** — `LEFTOVER_EXIT_CODE` goes back to agent (the leftover-cleanup side) (§2-5 · ADR-0266 decision 3).
- **Cost** — leave an adapter of about 100 lines (estimate — §9) in agent and strip only `cfg`.
- ★**This shape is an amendment of ADR-0266 decision 6 → ADR-0275 (§8)**★ — decision 6 wrote that `ProcessGroup` and the Job Object wrapper are moved 「한 덩이로」 and that 「새 손잡이를 따로 짓지 않는다」. B moves only the OS layer and builds `GroupOwner` · `GroupRef` · five concrete types anew. The substance of decision 6 (strong owner = link · weak handle outward · read-only flag · port only) stays. One could read `GroupOwner` as 「`JobObjectHandle` 의 겉만 바꾼 것」 (the same OS object · the same strong owner · adding only a no-op outside Windows and `downgrade`) and proceed without an amendment, but since the names · shapes · internals of the public types change (`Arc` is moved inside), this TRD does not rely on that reading.

**Why not A (왜 A 가 아닌가)** — A was the option of moving, as decision 6's 「한 덩이로」 wording says, `windows.rs` + all of `process_group.rs` (`ProcessGroup` · `RetiringSignal` · `Pinned` · `Births` · `ProcessFacts` · `PortEvent` · `MemberKill` · `GROUP_GONE`) to platform. The cost is smallest (mostly path changes), but platform's public API would gain the link concept `RetiringSignal` (reading a link's retiring field — unrelated to the OS) · test-seam traits · trait objects (`Box<dyn Pinned>` · `Arc<dyn Births>`), and `ProcessGroup::detached` would have to be opened as a `test-support` public constructor — which conflicts with ADR-0266 decision 2 (decision = OS dependence) and decision 5 (the trait for fakes belongs to **the user side**). And A would also have to build `GroupOwner` anew, so the 「새 손잡이를 따로 짓지 않는다」 wording had to be fixed either way — the gain from keeping the wording was only half.

**Common invariants (공통 불변식) (put into the coder brief):**

- **Keep the kill causality (ADR-0001) order** — `shutdown()`'s `child.kill + wait` → `group.terminate(1)` → master drop. In the place of the current `#[cfg(windows)]` block, the same line stands on every OS (outside Windows = no-op, so same as now).
- **Keep the drop order** — in all three links the Job field is the **last field** of the struct (`pty.rs:60` · `stdio.rs:89` · `codex/transport.rs:1396` — measured). Put the new field last as well. So that when a link is dropped and the Job handle closes, the moment `KILL_ON_JOB_CLOSE` runs is moved neither earlier nor later. ★No test measures this placement★ (neither a source-string test nor a comment anywhere — measured `rg '마지막 필드|마지막 칸|last field' crates/engram-dashboard-agent/src` → 0) — review guards it.
- **Leftover-cleanup lock rule (ADR-0262 decision 8 · CLAUDE.md 「핵심 불변식」 lock order)** — the only OS call made while holding the gate lock is `TerminateProcess` of the termination confirmation. So platform's `terminate_raw` · `classify` · `is_gone` use **neither logging nor locks** — platform becoming able to use `tracing` (ADR-0266 decision 9) must not spread to these functions. `terminate_raw` remains exactly one OS call.
- **`GroupRef` does not hold a Job** — it upgrades to the Job only when called (`is_gone` does not even upgrade), and the longest window is while `watch_births` calls `start` (as in the head comment of `process_group.rs`).

### 3-5. Error types

- The default is `std::io::Result` (every target already does this now · same rule as base — ADR-0269 decision 6).
- The one exception is WMI launching — the refusal code (`ReturnValue`) must be preserved so that discovery's `SpawnFailed { rv }` and the flag-diagnosis tests survive → `DetachedSpawnError { Refused { rv }, Io(io::Error) }` (draft). The raw call (`wmi_create_raw` — does not raise the RV as an error) is for diagnosis tests only, so it is made public behind `test-support`.
- 「이 OS 에는 수단이 없다」 is `io::ErrorKind::Unsupported`, and 「무리가 이미 사라졌다」 stays the current constant `GROUP_GONE = NotConnected`.

### 3-6. Non-Windows behavior — move it exactly as it is now

| Function · type | Non-Windows answer (now = after the move) | Current location |
|---|---|---|
| `process_start` / `process_creation_time` / `current_process_start_time` | `Unknown` / `None` / `None` | base `platform.rs:254-268` |
| `pid_alive` / `pid_alive_with_start_time` | `pid != 0` (start time ignored) | `:270-279` |
| `process_parent_table` / `child_pids` / `subtree` | `None` / empty list / empty list | `:260` |
| all of `GroupOwner` · `downgrade` | no-op `Ok` · `active_processes` = `Unsupported` · `None` | absence of the links' `cfg` · `stdio.rs:174-177` |
| `GroupRef` | empty list · cannot hold · port `Unsupported` · always gone | `process_group.rs:151 · 172 · 203 · 219 · 236` |
| `file_holders::holders_of` | `Ok(빈 목록)` | `file_holders.rs:102-105` |
| `kill_tree` | `Unsupported` (discovery converts it into the current wording 「`daemon_stop` 은 Windows 전용」) | `discovery/src/lib.rs:928-931` |
| `spawn_outside_job` | `Io(Unsupported)` | `:1319-1322` |
| `open_deny_write` / the two error decisions | `Unsupported` / always false | net `instance.rs:223-226 · 308-315` |
| `hide_console_window` | no-op | the three links |
| `prepare_tree_root` · `TreeRoot` | new process group · `kill -s KILL -- -<pgid>` · `members_gone` = true | `usage/process.rs:85-90 · 470-508` — ★this branch has never been compiled in this repository★ (head comment of that file) · it still is not compiled after the move |
| `MissingProgramCheck::judge` | always false (on POSIX the launch fails with `NotFound`) | `:152-160` |
| `default_shell` / `console_command` / `home_dir` / `exe_file_name` / `env_key_eq` | `bash` / run as is / `HOME` / not appended / exactly the same | R1~R6 |

### 3-7. Logging

- platform calls the `tracing` facade directly — it does not go through base (ADR-0266 decision 9 · ADR-0268 decision 1).
- **Principle (proposal): return results as values and let the caller log. Only a path with no way to return them (`Drop`) is logged by platform.** So the `Drop` warning of `RmSession` is moved as it is (`file_holders.rs:142` — the one that ADR-0266 decision 9 named), while the usage query's 「그룹 kill 실패 — 자식만 끊는다」 (`usage/process.rs:455 · 497`) is returned by `TreeRoot::kill` as `io::Result` and agent logs it with the current wording (「사용량 조회: …」) — **no domain words are taken into platform logs.** The debug line of `MissingProgramCheck` and the one-time 「막힌 찾기」 warning (`STUCK_LOOKUP_WARNED`) likewise surface the decision result (present · absent · unknown (blocked)) as a value, and agent does the logging. The poison handling of the lookup list (`LookupGate`) is in the U5 row (§5-1) — the poison also goes out as a 「모름(막힘)」 value and platform does not log.
- The lock rule of §3-4 — functions called inside a lock do not log.

### 3-8. Test-support feature `test-support`

- Across the crate boundary, `#[cfg(test)]` is not visible to the callers' tests. The four helpers (`spawn_gated_cmd` · `open_gate` · `wait_until` · `is_ping`) used by leftover's real-process tests (`#[cfg(all(test, windows))]` — `leftover.rs:4221 · 6563-6573`) are opened as module `testing` behind `test-support` (precedent = command crate `test-support` — `crates/engram-dashboard-command/Cargo.toml:10-11`). All four exchange only OS-level values (`Child` · PID · probe).
- **`new_group` (currently `process_group.rs:392-395` — returns `(Arc<JobObjectHandle>, ProcessGroup)`) does not go to platform `testing`** (D3 = B). The `ProcessGroup` · `RetiringSignal` it builds belong to the agent adapter, so platform cannot build them. It stays in the agent adapter's test support (the `#[cfg(all(test, windows))]` helper of `transport/process_group.rs`), builds via `GroupOwner::new()` → `downgrade()` → `ProcessGroup::new(g, RetiringSignal::of(&Arc::default()))` and returns `(GroupOwner, ProcessGroup)`. The imports of the leftover tests (`:4221-4226` · `:6571-6572` · `:6678` · `:6726` · `:6751`) split in two — the four helpers come from `engram_dashboard_platform::testing`, `new_group` from the agent adapter. The strong side's `job.assign(..)` · `job.terminate(1)` become `adopt` · `terminate`. Since that helper is `#[cfg(windows)]`, the adapter file falls under 「시험 분기뿐인 파일」 of the §4-4 expected list (re-measured at U8). Since `detached` also stays in agent, the only thing platform opens with a public constructor is `GroupRef::gone()` (§3-4).
- **The feature name `test-support` is the same as the 1-1 decision (N1 = `test-support`)** — base (places the fake clock `ManualClock` of 1-1 C4=(a) behind this feature inside `time`) · command · transport · platform use 「시험 도우미를 부르는 쪽 시험에 연다」 under one name. Features are per crate, so they do not enable one another — platform does not depend on base (ADR-0266 · ADR-0268).
- Add `engram-dashboard-platform = { …, features = ["test-support"] }` to agent `[dev-dependencies]` (same form as command). discovery's WMI diagnosis tests (`#[ignore]` · `lib.rs:2754 · 2828`) also use the raw call, so enable it in the same form.
- This `wait_until` (takes `what` and a probe and returns a value) is a **separate function with a different signature** from `wait_until(timeout, cond) -> bool` (ADR-0269 decision 5) that 1-1 gathers into base `testing`. Since platform does not depend on base (ADR-0266 · ADR-0268), they are not merged.
- ★**The module declaration is `#[cfg(any(test, feature = "test-support"))] pub mod testing;`** — gating it by `feature` alone is wrong★: platform's own moved tests (such as the real-process tests of `group`) use the same helper, and in the CI step `cargo test --locked -p engram-dashboard-platform` (§4-2 — no feature argument) `test-support` is off, so those tests do not compile. Precedent = transport (`any(test, feature = "test-support")`) · 1-1's base `testing` has the same form.
- Operational builds (`cargo build --locked`) do not enable this feature — it is measured with a machine gate (§4-2 ③).

### 3-9. `windows` feature (decision D1 = a · §7)

- The union platform holds (measured — the sum of the current base · agent · discovery manifests): `Win32_Foundation` · `Win32_System_Threading` · `Win32_System_Diagnostics_ToolHelp` · `Win32_System_JobObjects` · `Win32_Security` · `Win32_System_IO` · `Wdk_System_Threading` · `Win32_System_RestartManager` · `Win32_System_Com` · `Win32_System_Wmi` · `Win32_System_Rpc` · `Win32_System_Variant`.
- ★Do not remove `Win32_Security`★ — `CreateJobObjectW` is behind it and `Win32_System_JobObjects` does not imply it (CLAUDE.md 「의존성」 · the agent `Cargo.toml` comment — move it along with that comment).
- **What disappears from each crate:** the whole `[target.'cfg(windows)'.dependencies]` of base · discovery. agent disappears from the operational dependencies and **remains as `[target.'cfg(windows)'.dev-dependencies]`** — two examples use `windows` directly (§2-5). Its feature is the minimal set with which the examples compile (whether `Win32_Security` is additionally needed on `Win32_Foundation` · `Win32_System_JobObjects` · `Win32_System_Threading` is **unchecked** — decide by compiling in the relevant unit).
- **Do not split into per-capability cargo features** (decision D1 = a · §7) — one `windows` dependency turns on the whole union above. platform's only cargo feature is `test-support` (§3-8), so there is no feature-combination gate either (the net 5a · 5b form).

---

## 4. Dependency graph and gates

### 4-1. Dependency graph — direct workspace dependencies (measured `cargo tree -p <pkg> --depth 1 --prefix none -e normal --target all`)

| crate | now | after 1-3 |
|---|---|---|
| platform | — | **none** (leaf) |
| base | none | none — but the `windows` third-party dependency disappears |
| net | base · protocol | **platform** · protocol (no longer uses base — every symbol it used was a PID helper) |
| discovery | base · net · protocol | base · net · **platform** · protocol — ★base stays★: the one PID-helper line it uses now (`lib.rs:1043`) goes to platform, but 1-1, which lands first, makes discovery `Clock` a sub-trait of `engram_dashboard_base::time::Clock` (ADR-0269 decision 3-2 · 1-1 TRD §3-3 · U5) |
| agent | base · command | base · command · **platform** |
| daemon | agent · base · command · discovery · messaging · net · protocol | same + **platform** (`current_process_start_time` · `exe_file_name` · `default_shell`) |
| shell (`engram-dashboard`) | agent · base · command · discovery · net · protocol | **no change** (platform is reached only transitively through discovery. A direct dependency appears at U-W) |

### 4-2. New gates — platform (ADR-0266 「영향」)

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

- Three places to register (ADR-0175 impact · ADR-0266 impact): `.github/workflows/ci.yml` (steps ①② + the `cargo test --locked -p engram-dashboard-platform` step — CI does not use `--test-threads` · ③ at U3) · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md` (including the 4-pre-style path guard).
- ① is `--all-features`, so it also looks at the graph with `test-support` turned on — it also measures whether that feature pulls in any workspace crate.
- ③ is **registered at U3** — the `testing` module and agent's dev-dependency (`features = ["test-support"]`) first appear in that unit. Before that, the 1-line expectation of the paired line cannot stand. The shell reaches platform only transitively through discovery (§4-1) and takes the daemon as a dev-dependency, but since `-e normal` the dev edge is not counted. This gate measures the premise that, because the workspace is `resolver = "2"`, the features of the dev graph are not merged into the operational build.

### 4-3. Changed gates — new expected values

| Gate | Now | After the change | Changing unit | Places to fix (고칠 자리) |
|---|---|---|---|---|
| net gate 2a (symbol allowlist) | `rg -o --no-filename "engram_dashboard_base::[A-Za-z0-9_:]+" crates/engram-dashboard-net/src/ \| sort -u` → **exactly 2** | prefix to `engram_dashboard_platform::` · **exactly 2 after U1** (`…::process::pid_alive_with_start_time` · `…::process::current_process_start_time`) → **exactly 5 after U6** (+ `…::fs::open_deny_write` · `…::fs::is_sharing_violation` · `…::fs::is_access_denied`) — the names are drafts, the number is re-recorded when the API is fixed | U1 · U6 | `ci.yml:784-813` · `net/src/lib.rs:59-75` · CLAUDE.md 「네트워크 행 격리 게이트」 · `qa.md:163` · `docs/testing-strategy.md:72` |
| net gate 2b (0 lines) | `engram_dashboard_(a)gent` → 0 | unchanged. ★The paired description of 2a (「심볼이 사라진 게 아니라 옮겨 갔다」) must state that it switched once more, to 「agent → base → platform」★ | U1 | `ci.yml:815-831` comment · net header · CLAUDE.md 「네트워크 행 격리 게이트」 2b line (「`agent`에서 `base`로 이사하며 기대값 2가 crate 이름을 갈아탔고」) · `docs/testing-strategy.md:73` |
| net gate 3 (direct dependency cap) | **exactly 3** = net · base · protocol | **exactly 3** = net · **platform** · protocol | U1 | `ci.yml:833-852` · `net/src/lib.rs:83-93` · CLAUDE.md · `qa.md:165` · `docs/testing-strategy.md:74` |
| base tenant cross-reference ③ | `rg "(crate\|super)::(logging\|platform)" crates/engram-dashboard-base/src/` → 0 | remove `platform` from the name alphabet. 1-1 defers `file` (its G1 rule — the gate lists only names of tenants that actually exist) and adds `text\|time\|path` (1-1 U1) → `+sync` (U2) → `+testing` (U4). Since 1-1 lands first (§5 Prerequisites), U1 removes only `platform` from the regex 1-1 left behind → after both have landed = `rg "(crate\|super)::(logging\|text\|time\|path\|sync\|testing)" crates/engram-dashboard-base/src/` → 0 (a form with no exceptions — with 1-1 C4=(a) the fake clock stands inside `time` and `testing` does not call `time`) | U1 | `ci.yml:640-656` · base `lib.rs` header · CLAUDE.md · `qa.md:138` · `docs/testing-strategy.md:50 · 155` |
| messaging source-name gate | `rg "engram_dashboard_(agent\|base\|daemon\|protocol\|discovery\|command)" crates/engram-dashboard-messaging/src/` → 0 | add `platform` to the alphabet (the expectation of 0 stays). Precedent = names were added when a crate was born — `command` (ADR-0134) · `base` (ADR-0175) · basis = `ci.yml:511-514` comment 「새 워크스페이스 crate 가 생길 때마다 여기 이름을 더해야 보인다」 | U0 | `ci.yml:519` (+ :511-514 comment) · CLAUDE.md 「빌드·검증 명령」 · `qa.md:152` · `docs/testing-strategy.md:63` |
| base dependency cap · `use tauri` | 1 line · 0 lines | unchanged | description only, at U1 | from the 「headless 데몬 · `net` · `discovery` 에 링크된다」 description **only `net`** drops out — discovery keeps using base through 1-1's `time::Clock` (§4-1). Copies of the same description: base `lib.rs:35` header · CLAUDE.md 「빌드·검증 명령」 base `use tauri` line (:248) · `qa.md:137 · 148 · 307` · `docs/testing-strategy.md:49` |
| `cargo test -p engram-dashboard-base` | local `-- --test-threads=4` (basis = `child_pids_finds_spawned_child` of `platform`) | that basis goes to platform — the remaining `logging` tests do not launch processes (`rg 'Command::new\|process::Command\|current_exe' crates/engram-dashboard-base/src/logging crates/engram-dashboard-base/tests` → only 1 place of `current_exe` in operational code · measured). Since 1-3 starts after 1-1 merges, at U1 all of 1-1's tenants (`text` · `time` · `path` · `sync` · `testing`) are already in place — U1 reruns the same rg over all of base `src/` · `tests/` and removes it if the number of tests that launch processes is 0 (proposal) | U1 | CLAUDE.md 「빌드·검증 명령」 · `ci.yml:270-278` comment · `docs/testing-strategy.md:46 · 138` |
| net `server` feature | `dep:engram-dashboard-base` | `dep:engram-dashboard-platform` | U1 | `net/Cargo.toml` |
| discovery async-runtime import gate | 0 | unchanged — platform does not pull in tokio (stated in the platform header as something to uphold) | — | — |

★To keep net gate 2a alive, net must call platform by **full path or one-item `use`**★ — if it is imported with braces like `use engram_dashboard_platform::fs::{a, b}`, `rg -o` catches only `engram_dashboard_platform::fs::` line and the count goes off (base symbols are called by full path today as well — `portfile.rs:86`).

### 4-4. Invariant gate (proposal — ADR-0266 left the form of the gate to the main session's judgment)

ADR-0266 invariant 「`#[cfg(windows)]` · `cfg!(windows)` 는 platform crate 안에만」 currently has no machine gate. As a cheap early signal we place a **file allowlist**:

```bash
# platform 밖에서 OS cfg 를 쓰는 파일 목록 = 아래 고정 명단과 정확히 같아야 한다.
# 접두 (?:[^/]|/[^/])*? = 「아직 // 를 안 만났다」 → 주석 속 인용(net lib.rs:44 등)은 안 문다(replay_flight 게이트와 같은 꼴).
# 괄호 안 (?:[^)]|\([^()]*\))*? = 「닫는 괄호가 아닌 글자, 또는 안쪽이 한 겹인 닫힌 괄호 묶음」
#   → cfg(all(not(test), windows)) 처럼 안쪽 괄호가 한 번 닫힌 뒤에 오는 OS 낱말도 문다.
# -g '*.rs' = 매니페스트(Cargo.toml 의 [target.'cfg(windows)'.…])를 빼고 소스만 본다.
rg -l "^(?:[^/]|/[^/])*?\b(cfg!?|cfg_attr)\((?:[^)]|\([^()]*\))*?\b(windows|unix|target_os|target_family)\b" \
   crates src-tauri/src -g '*.rs' -g '!crates/engram-dashboard-platform/**' | sort
```

- **Three flaws in the 1st-edition regex (`\([^)]*`) (measured `0ef6292` — ran both forms on the same tree and compared):** ① without `-g '*.rs'`, the four `Cargo.toml` of agent · base · daemon · discovery got caught ② it could not match an OS word that comes after the inner parenthesis closes, so an operational branch like `#[cfg(all(not(test), windows))]` could slip through (the repository has no operational branch of that form now — the fixed form newly catches only two `main.rs`) ③ so for 「`main.rs` 둘이 `cfg_attr` 로 걸린다」 the basis of the expected list was false — the 1st-edition form catches neither of the two. The fixed form catches `.rs` 41 files (1st-edition form 43 − 4 manifests + 2 `main.rs`), and no other operational file is newly caught.
- Expected list = the 「시험 분기뿐인 파일」 (files with test branches only) of §2-3 + two harness bins + two agent examples (`examples/spike*.rs` — the 1st edition missed them) + `src-tauri/src/main.rs` · `daemon/src/main.rs` + **`src-tauri/src/fsutil.rs` (known operational exception until U-W)**. ★The reason the two `main.rs` get caught is not the cfg predicate★ — from `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` the matcher bites across the `not(…)` bundle into **the attribute value `"windows"`** (a matcher false positive). Because it is still a crate-level linker attribute and cannot be moved (§2-3), it stays on the list. The exact list is re-measured with this command at U8 and recorded.
- **Limits (no concealment):** adding a new operational branch inside a file already on the list is not caught — only new **files** are. Runtime OS selection (such as the harness's `experiment/transcript.rs`, which picks environment variables without `cfg`) is not caught either. ★An OS word that comes after two or more levels of closed parentheses (`cfg(all(not(any(test, x)), windows))`) and a `cfg` attribute spanning several lines are also not caught★ — both are 0 in the repository now (measured rg). If a word appears in an attribute value of `cfg_attr`, it matches even though it is not a predicate (`main.rs` above). Review fills the gap.
- Using `| sort` instead of `--sort path` is the same form as the existing gates.

---

## 5. Migration units

**Principle (user 2026-10-02 — 「망가지지 않는 단위로 잘 그룹지어서 작업하라」 · 「임시 땜빵 금지」):** cut each unit so that the workspace builds and regression stays green. Do not use stepping stones that temporarily keep the same code in two places — the unit that moves code changes the callers in one go. The order inside a unit is 「새 자리 만들기 → 부르는 곳 바꾸기 → 옛 자리 지우기」 (create the new location → change the callers → delete the old location), and no unit leaves another unit's old location behind.

**Prerequisites:** ① ★**U0 starts only after 1-1 (base generic helpers) is merged**★ — it overlaps heavily in files with 1-1's base `lib.rs` header · gate ③ · `sync` replacement (agent `codex/transport.rs` is in the scope of 1-1 TRD's U3b — the 1-1 side decides the count · beyond that, U3a's links · leftover · `usage/process.rs` and so on) (see 「겹침」 (overlap) below). ② Before each unit starts there is a point to roll back to, via the commit at that spot (global rule).

**Decisions of 1-1 that touch this document (2026-10-04 · authoritative source = the sibling 1-1 TRD — only the aligned result is written here):**

- **S2 = (e) lock poisoning is only reclaimed, without a warning (the poison is left)** — base `sync` also reclaims silently. It has the same meaning as the one line (`unwrap_or_else(PoisonError::into_inner)`) that U5's lookup list `release` uses inside platform. Since platform cannot call base, it remains as that one line (§5-1 U5).
- **N1 = feature name `test-support`** — the same name as platform's test feature (§3-8).
- **C4 = (a) the fake clock is in base `time` behind `test-support`** — base tenant no-reference gate ③ has no exception (§4-3).
- **C5 = (b) transport → base is deferred to stage 3** — transport does not depend on base this time. So the order in which U1 strips `windows` from base does not touch the transport graph (it would have depended on the order had 1-1 C5 been (a)).
- **platform stays independent of base** — none of the four above creates a platform → base edge (ADR-0266 · ADR-0268).

### 5-1. Unit list

| Unit | Scope | Crates · files touched | QA |
|---|---|---|---|
| **U0** | set up the empty crate · register gates · add `platform` to the messaging name gate | root `Cargo.toml` (+ head comment 「멤버 10」 → 11 · one platform line) · `Cargo.lock` · `crates/engram-dashboard-platform/{Cargo.toml, src/lib.rs}` (header = invariants · gates · `// ADR-0266`) · `ci.yml` (three new steps · messaging name gate `:519` and its comment) · CLAUDE.md 「빌드·검증 명령」 gate line · `qa.md` · `docs/testing-strategy.md:63` | standard |
| **U1** | move the PID helpers (P1) · strip `platform` and `windows` from base | platform `process.rs` · base `{lib.rs, platform.rs(삭제), Cargo.toml}` · net `{Cargo.toml(+ comment :43-45 「protocol·base」 · :73-77 · :135), portfile.rs, lib.rs header}` · discovery `{Cargo.toml(add the platform dependency · ★do not remove the base dependency★ — used by the 1-1 U5 `time::Clock` super-trait (§4-1). Of the base-line comment at :12-14, the reason 「liveness 판정 공유」 (shared liveness decision) moves to the platform line · in the comment at :19 forward closure {net, base, protocol} → {base, net, platform, protocol} — base stays (§4-1)), lib.rs:1043(only the PID helper import goes to platform)}` · daemon `{Cargo.toml, lib.rs:762, tests/ws_e2e.rs:2447}` · agent `{Cargo.toml, platform/{process_tree.rs, file_holders.rs, windows.rs}, backend/codex/{thread_lock.rs:279 · :1236(tests — `process_creation_time`), mod.rs:1107, transport.rs tests}, backend/claude/leftover.rs tests, usage/process.rs tests, tests/backend_contract.rs}` · root `Cargo.toml` comment (:2 · :12-13 base = 「로깅·PID 판정」) · `ci.yml` (net 2a · 3 · base ③) · `qa.md` (+ remove `net` from the base link description at :137 · 148 · 307) · CLAUDE.md gate line (+ the link description of base `use tauri` line :248 · the move description of the net 2b line — §4-3) · `docs/testing-strategy.md` (:46 · :49 · :50 · :72-74 · :138 · :155) | standard |
| **U2** | OS rules (R1~R6) | platform `{shell.rs, env.rs}` · agent `{manager.rs, backend/mod.rs, backend/claude/mod.rs, backend/codex/thread_lock.rs, tests/*(default_shell, 5 files)}` · daemon `{lib.rs:109, connection_core.rs, messaging_host.rs tests, tests/*(3 files)}` · discovery `lib.rs:987` — the harness's `experiment/transcript.rs` is not touched (§2-3) | standard (including the local real-claude test — `console_command` passes every CLI agent spawn · CI has no claude, so it cannot measure this) |
| **U3** | group owner · handle (P2 · P3 · the rest of P6) · stripping `cfg` from the three links · launching without a window (two links) · ★agent `platform/` folder cleanup · agent `windows` → dev-dependency (because it comes after U4 — §5-1 order note)★ · `test-support` operational leak gate (§4-2 ③) | platform `{group/mod.rs, group/windows.rs, spawn.rs(hide_console_window), testing.rs, Cargo.toml}` · agent `{platform/{windows.rs(deleted — `LEFTOVER_EXIT_CODE` goes to the agent leftover-cleanup side · §3-4), process_group.rs(rewritten as an adapter without cfg and moved to `transport/process_group.rs` — the test helper `new_group` also stays in that adapter's test support (§3-8)), mod.rs(deleted)}, lib.rs(`pub mod platform;` deleted), transport/{mod.rs(`process_group` mod declaration), pty.rs, stdio.rs(`process_group()` = `downgrade()`'s `GroupRef` + `RetiringSignal::of(&self.retiring)` → adapter `ProcessGroup::new` — ★carries the retiring flag★ §2-2 · §3-4), input_queue.rs:41 comment}, backend/codex/transport.rs(+ the source-string test below), backend/claude/{leftover.rs(test imports — the four helpers from platform `testing` · `new_group` from the agent adapter · `job.assign` · `job.terminate` → `adopt` · `terminate`), mod.rs}, usage/process.rs(`crate::platform` all references = the three operational ones `:438 · 444 · 451`(Job field · creation · wake-up) + `#[cfg(windows)]` two tests `:1389 · 1411`(wake-up) — if even one is left, once agent `platform/` is gone the test build breaks), Cargo.toml(`windows` → `[target.'cfg(windows)'.dev-dependencies]` · platform dev-dependency `test-support`)}` · `ci.yml` · CLAUDE.md · `qa.md` (gate ③). ★**One source-string test hard-codes the names being moved**★ — `backend/codex/transport.rs:6639-6658` `the_child_guard_is_armed_before_the_first_fallible_step_after_spawn` looks in the body of `open` for `"JobObjectHandle::new()?"` · `"job.assign(pid)?"` (panics if absent). Replace the two strings and the head comment (:6637) with the new spellings U3 uses (drafts `GroupOwner::new()?` · `.adopt(pid)?`) and measure **the invariant 「가드(`ChildGuard(Some(child))`)가 무리 만들기 · 넣기보다 먼저 선다」 unchanged**. Other tests of the same form do not use the names being moved (measured — 20 `include_str!` source tests · rg for string literals that contain the moved names) | ★**full** (including GUI) — kill causality (ADR-0001) passes through the Job (ADR-0266 impact)★ |
| **U4** | the processes that have the file open · identity list (P4 · P5) | platform `{file_holders.rs, process.rs(subtree)}` · agent `{platform/{file_holders.rs(deleted), process_tree.rs(deleted), mod.rs(the mod declarations of those two · head comment)}, backend/codex/{thread_lock.rs, mod.rs:3781}, Cargo.toml(`windows` feature: remove only `Win32_System_RestartManager` — proposal · confirm by compiling that no use remains)}`. ★Do not delete the folder · `windows.rs` · `process_group.rs`★ — at the time of U4 those two are still called by the three links · `usage/process.rs` · leftover (`stdio.rs:36 · 38` and so on) | standard (+ local codex session-id recovery test) |
| **U5** | OS layer of the usage query (U) | platform `spawn.rs` (`prepare_tree_root` · `TreeRoot` · `MissingProgramCheck`) · agent `usage/process.rs`. ★**platform handles the poison of the lookup list (`LookupGate` — `usage/process.rs:334-354`) by itself**★ — platform cannot call base `sync` (ADR-0266 · ADR-0268) · 1-1 U3a hands it over without moving the recovery form to sync (it only folds the helper into release and attaches a comment that points to 1-3 U5). ① Claiming a spot (`claim`): poison = 「도는 중」 → decision 「모름」 — the same meaning as now, and test `a_poisoned_gate_means_unknown_without_a_panic` (:1742) moves along with it. Since the poison is not cleared, once it is poisoned that process's lookups are always 「모름」 (same as now). ② Releasing a spot (`release` — `Drop` of `InFlight`): reclaim silently with `unwrap_or_else(PoisonError::into_inner)` (the same meaning as the current `lock` helper `:636`, in one line · do not place a new helper in platform). Since 1-1 U3a folds that helper into `release` and attaches the 「1-3 U5 로 간다」 comment, U5 moves that one line and removes the comment. The base `sync::lock` of 1-1 S2=(e) also reclaims without a warning, so the meanings do not diverge (§5 Prerequisites). ③ Logging: platform does not log (proposal) — the poison goes out as a 「모름(막힘)」 value and agent's one-time warning logs it with the current wording (§3-7), and recovery on the `Drop` side is not a failure and has nothing to return. Even now the poison and 「막힌 찾기」 go out with the same warning wording (a poison is reported as 「앞선 찾기가 아직 끝나지 않아」) — we do not fix it because this is a moving unit | standard + local `cargo test -p engram-dashboard-agent --test usage_probe_smoke -- --ignored` (all `#[ignore]` — requires a logged-in CLI) |
| **U6** | single-instance opening (N) | platform `fs.rs` · net `{instance.rs, lib.rs header}` · `ci.yml` (net 2a = 5) · CLAUDE.md · `qa.md` · `docs/testing-strategy.md:72` | standard (the instance tests measure the sharing semantics for real on Windows) |
| **U7** | process stop · WMI launching (D1 · D2) · stripping `windows` from discovery | platform `{process.rs(kill_tree), spawn.rs + spawn/wmi.rs, Cargo.toml}` · discovery `{lib.rs, Cargo.toml}` | **full** — launching the app starts the daemon via WMI (`ensure_daemon`), and stopping rides `taskkill`. Measuring the daemon start is the completion condition (the same yardstick as ADR-0271 decision 7) |
| **U8** | invariant gate (§4-4) · doc follow-ups (§6) | `ci.yml` · CLAUDE.md · `qa.md` · base `lib.rs` header · memo §11 | the docs · config scope of the `/qa` binding + `/review doc` (load-bearing docs) |
| U-W (deferred) | unify atomic writes → platform `fs` | shell `fsutil.rs` (shape after P3) · daemon `usage_service/reject_store.rs` · agent `persistence/{mod.rs, presets.rs}` · direct platform dependency in the shell | trigger = storage P3 landing on master. 「하나로 정할 동작」 is chosen by the user at that time (memo §10 · ADR-0266 decision 8) · ★→ TRD A (`trd-A-data-file-unification.md`) took it over and closed it (2026-10-10~11) — its home is not platform but base `file` (ADR-0291 decisions 1 · 3 amend ADR-0266 decision 8) · platform `fs` gets only the locked-state decision · retry · folder sync · `reject_store.rs` was removed before that by ADR-0284★ |

**Order:** U0 → U1 → U2 → U4 → U3 → U5 → U6 → U7 → U8.

- U1 must be first — the agent-side pieces (P4 · P5 · the P2 tests) currently call base's PID helpers, so unless those helpers already stand in platform, moving those pieces would require a platform → base edge (forbidden — ADR-0266 · ADR-0268).
- U3 must precede U5 — the usage query's tree handle uses the Job owner and the wake-up, so they must already exist in platform. U3 changes only the five `crate::platform` references of `usage/process.rs` (three operational · two `#[cfg(windows)]` tests — table above) to the platform path and leaves the rest to U5. If the two tests are missed, `cargo test` breaks even though the operational build stands.
- The reason for putting U4 before U3 = pull out the lower-risk part first so that U3's full QA looks at the kill causality alone. Swapping them does not break anything (the overlap is only agent `platform/mod.rs` · `Cargo.toml`) — ★the deletion of agent `platform/mod.rs` · the folder · `pub mod platform;` in `lib.rs` and the dev-dependency switch of agent `windows` belong to **whichever of the two comes later — in this order, U3**★ (table above). The unit that comes first deletes only its own files and their mod declarations. If the order is swapped (U3 → U4), this part goes to U4 — then U3 deletes `windows.rs`, handles `process_group.rs` and leaves the two lines `file_holders` · `process_tree` in `platform/mod.rs`.
- U6 · U7 can come anywhere after U1 (they do not touch agent).

### 5-2. Verification per unit

1. **Compare regression counts — run the same command before and after and compare the two numbers.** A target that vanishes silently (a lost test target · tests left uncompiled by `#[cfg]`) is silence, not failure.

   ```bash
   cargo test --workspace -- --test-threads=4 > "$SCRATCH/u<N>-<before|after>.log" 2>&1
   rg -c '^test result:' "$SCRATCH/u<N>-after.log"                     # 결과 줄 수
   rg '^test result:' "$SCRATCH/u<N>-after.log" \
     | sed -E 's/.* ([0-9]+) passed; ([0-9]+) failed; ([0-9]+) ignored.*/\1 \2 \3/' \
     | awk '{p+=$1; f+=$2; i+=$3} END {print "passed="p, "failed="f, "ignored="i}'
   ```

   If the terminal crashes, lower it to 2 or run it outside the process tree with `scripts/run-detached.ps1` (CLAUDE.md 「빌드·검증 명령」).

2. **Expected differences (the unit declares them and the result must match):**

   | Unit | Result lines | Pass · ignored total | Distribution shift (measured `rg -c '#\[(tokio::)?test'` · `rg -c '#\[ignore'`) |
   |---|---|---|---|
   | U0 | **+2** (platform lib unit · Doc-tests — a line is printed even for 0 tests) | unchanged | — |
   | U1 | unchanged | unchanged | base → platform 13 |
   | U2 | unchanged | unchanged + k new tests (the unit states the number) | if there are rule tests that move, agent → platform |
   | U3 | unchanged | unchanged | agent → platform: `windows.rs` 4 (1 ignored) + the real-process tests among `process_group.rs` (10 in total · 1 ignored — fake-based tests stay in the agent adapter · D3 = B). U3 declares how many go to which side |
   | U4 | unchanged | unchanged | agent → platform: `file_holders.rs` 2 · `process_tree.rs` 9 |
   | U5 | unchanged | unchanged | agent → platform: of `usage/process.rs` 34 (1 ignored), the OS-layer tests |
   | U6 | unchanged | unchanged + k new `fs` tests | the 11 instance tests stay in net (they measure the meaning of the guard) |
   | U7 | unchanged | unchanged | discovery → platform: 1 COM classification test · the two WMI `#[ignore]` stay in discovery (they need the daemon exe · `daemon.json`) |

   **Measure the baseline right before U0 starts** — the last record (CLAUDE.md 「빌드·검증 명령」 = 59 result lines · 3822 passed · 30 ignored, `cac9ee0`) may have gone stale because of the master absorption since. This TRD did not run it.

3. **Gates:** all gates of the crates the unit touched + `cargo fmt --check` + generated-output sync (agent `bindings/` — this migration must not change the ts-rs output) + from U0 on, platform gates ①② + from U1 on, the changed net 2a · 3 · base ③.

4. **QA:** the grade in the §5-1 table (`/qa` binding). U3 · U7 go through GUI measurement — launch the app with the `scripts/` launcher (do not launch it directly from the shell).

### 5-3. File overlap — serial · parallel

| | U1 | U2 | U3 | U4 | U5 | U6 | U7 |
|---|---|---|---|---|---|---|---|
| U2 | discovery `lib.rs` · `thread_lock.rs` · daemon `lib.rs` | | | | | | |
| U3 | agent `Cargo.toml` · `windows.rs` · leftover · codex transport · `usage/process.rs` · `ci.yml` · CLAUDE.md · `qa.md` (different lines) | `claude/mod.rs` | | | | | |
| U4 | `file_holders.rs` · `process_tree.rs` · `thread_lock.rs` · codex `mod.rs` | `thread_lock.rs` | agent `platform/mod.rs` · `Cargo.toml` | | | | |
| U5 | `usage/process.rs` | — | `usage/process.rs` | — | | | |
| U6 | net header · `ci.yml` · CLAUDE.md · `qa.md` | — | — | — | — | | |
| U7 | discovery `lib.rs` · `Cargo.toml` | discovery `lib.rs` | — | — | — | — | |

- Every unit also touches platform's `lib.rs` (mod declaration) · `Cargo.toml` · `Cargo.lock` — easy to merge, but still an overlap.
- **Recommended = one coder at a time, serially.** U1 must be alone (it touches almost every crate). If parallelism is used, it is only **U6 ∥ U7** after U1 (unrelated to agent · the overlap is three shared platform files + different lines of `ci.yml` · CLAUDE.md · `qa.md`).
- **Do not run 1-1 and 1-3 in parallel** — 1-1's `sync` replacement touches `codex/transport.rs` · `leftover.rs` · `usage/process.rs` · `stdio.rs`, the `Clock` merge touches discovery `lib.rs`, and adding tenants touches base `lib.rs` · gate ③.
- **Put the order 「중간 어디서 멈춰도 빌드가 서게」 (so that the build stands wherever it stops midway) in the U3 brief** — ① stand up `group` in platform (nobody calls it · build green) ② first stand up the agent adapter (`ProcessGroup::new(GroupRef, RetiringSignal)` · `detached` · test helper `new_group`) in `transport/process_group.rs`, and turn the three links, the five `crate::platform` references of usage, leftover, claude `mod.rs`, and the guard-placement source-string test toward it · toward platform ③ delete agent `windows.rs` and the old `platform/process_group.rs` (the adapter stood up in ②) ④ delete agent `platform/mod.rs` · `pub mod platform;` in `lib.rs` and turn `windows` into a dev-dependency (when U4 has finished first — §5-1). If it is cut in the middle of ②, roll back to the green tree with only ① left.

---

## 6. Document follow-ups (this TRD does not fix them — each unit or U8)

1. **CLAUDE.md 「백엔드 모듈 맵」** — add a platform entry (what · 0 workspace dependencies · `tracing` direct · two gates · authoritative source = that crate's `src/lib.rs` header · ADR-0266). base entry: the tenant description (「`platform`(PID liveness·…)」). In 「headless 데몬·`net`·`discovery`에 링크」 of the 「빌드·검증 명령」 base `use tauri` line (:248), remove **only `net`** — discovery keeps using base through 1-1's `time::Clock` (§4-1). agent entry: make the sentences 「남은 `platform`은 Job Object 래퍼 하나뿐이다」 (first line of memo §11 — stale even now: actually four) and 「`logging`과 PID 판정 헬퍼는 여기 없다」 point to platform. discovery entry: WMI launching · stopping go to platform.
2. **CLAUDE.md 「플랫폼 중립」** — replace the location of the precedent `console_command` and the way 「현황은 아직 못 미친다」 is found (`rg -l 'cfg!?\(windows\)' crates src-tauri`) with the §4-4 gate, and state the exception (`fsutil.rs` — until U-W).
3. **CLAUDE.md 「핵심 불변식」** — the `TerminateJobObject` subject in the kill-causality line (group owner `terminate`) · where the handle is defined in the ownership-split line (D3 = B — `ProcessGroup` · `RetiringSignal` names unchanged · the file is agent `transport/process_group.rs` · under it the OS layer `GroupOwner` · `GroupRef` is platform) · the scope that 「Clone 아님」 enforces (one owner per group — that the owner is the link is placement) · one phrase 「platform 의 `terminate_raw` 는 OS 호출 하나 · 로그 없음」 in the leftover-cleanup lock rule.
4. **CLAUDE.md 「의존성」** — `windows` entry: platform = union · agent = dev-dependency for examples · base · discovery = none. The `Win32_Security` warning goes to platform.
5. **CLAUDE.md 「빌드·검증 명령」 · 「네트워크 행 격리 게이트」** — platform test line · three gates (①② · `test-support` leak ③) · the flag basis of the base test line · base ③ · `platform` of the messaging name gate · net 2a (2 → 5) · 「`agent`에서 `base`로 이사하며 기대값 2가 crate 이름을 갈아탔고」 of the net 2b line (→ agent → base → platform) · net 3 (base → platform) · invariant gate.
6. **`.github/workflows/ci.yml`** — the steps · expected values · comments of §4-2 · §4-3 (including 「`platform` 스위트가 실 `cmd.exe` 를 띄운다」 of the base test step comment · the messaging name gate `:511-519`).
7. **`.claude/skill-bindings/qa.md` · `docs/testing-strategy.md`** — qa: 4b · 4c · base cap · messaging name gate (`:152`) · net 2a · 3 lines + platform gate (including the 4-pre-style path guard) · base link description (`:137` · `:148` · `:307` — remove only `net`). testing-strategy also holds separate copies of the gates: base flag basis (`:46` · `:138`) · base link description (`:49` — same) · base ③ (`:50` · `:155`) · messaging name gate (`:63`) · net 2a · 2b · 3 (`:72` · `:73` · `:74`). Fix them in the same unit (§4-3 「고칠 자리」).
8. **Crate headers · manifest comments** — base `src/lib.rs` header (tenants · gate ③ · 「왜 바닥 crate 인가」 · `:35` remove only `net` from the link description) · base `Cargo.toml` description · net `src/lib.rs` gate 2 · 3 sections · agent `Cargo.toml:21` comment · `agent/src/transport/input_queue.rs:41` · `backend/codex/thread_lock.rs:6` · the `engram_dashboard_base::platform` path in the `process_tree` head comment · ★root `Cargo.toml` head comment (`:2` 「멤버 10」 · base = 「로깅·PID 판정」 · `:12-13` 「로깅 + PID 판정만 담는 잎 crate」)★ · ★net `Cargo.toml` dependency-rule comment (`:43-45` 「지금 그 조건을 만족하는 것은 protocol·base」 · `:73-77` edge history · `:135` 「base 의 `platform::` 헬퍼」)★ · ★discovery `Cargo.toml:19`'s forward closure `{net, base, protocol}`★ · the base-line comment of discovery `Cargo.toml:12-14` (the reason for sharing liveness moves to the platform line — the base dependency itself stays because of 1-1's `time::Clock`).
9. **ADR** — record the amendments of §8 in the new ADR-0275 and stamp the ADRs being amended (numbering · stamping · linking = the `/adr` skill, 0274 is written by another branch). ★**Only this item is recorded before U0 starts — recorded on 2026-10-04 (together with the 1-1 part in one ADR · §8)**★ — it is not the part of each unit or U8 (CLAUDE.md 「설계 결정 기록」 — large decisions are recorded as soon as they are made). What goes stale only in paths (end of §8) is fixed by each unit when it moves those paths.
10. **Memo `docs/refactoring/architecture-discussion-2026-09-26.md`** — the line numbers in the §1 table (§2-5) · 「셸에는 OS 분기가 없다」 · the processed marker for the first line of §11.
11. **Code anchors** — `// ADR-0266` on the platform crate header and the group-owner · handle types (ADR-0266 「영향」). Add `// ADR-0275` to those two (`GroupOwner` · `GroupRef`) and to the head of the agent adapter `transport/process_group.rs` (the adapter together with the current `// ADR-0262`).

---

## 7. Decisions (2026-10-04)

> **Source: user delegation (2026-10-04 「알아서 진행해」) → recommendation adopted.** All three are exactly what the 3rd edition §7 recommended (checked character by character — D1 (a) · D2 `engram-dashboard-platform` · D3 B). The 3rd edition intended to ask D1 → D2 → D3 one at a time, but the delegation closed them all at once. What is to be recorded in an ADR is §8.

| | Decision | One-line reason |
|---|---|---|
| **D1** | **(a) Do not split the `windows` binding into per-capability features** | the gain exists only for `-p` standalone builds, and the 「PID 만 쓰는 소비자」 (net · discovery) that would benefit disappear in 3-3 · 2-2 |
| **D2** | **`engram-dashboard-platform`** | ADR-0266 · the memo already call it that, and it matches the old module names (`base::platform` · `agent::platform`), so it is easy to find from old records |
| **D3** | **B — only the OS layer in platform; the neutral handles `ProcessGroup` · `RetiringSignal` in the agent adapter (`agent/src/transport/process_group.rs`)** | fits ADR-0266 decisions 2 · 5, and no link concept or trait object enters platform's public API |

### D1. Whether to split the `windows` binding into per-capability cargo features (ADR-0266 「열린 것」) → (a)

- **What was chosen:** one `windows` dependency of platform with the union features (§3-9). The cost = when the consumers that use only PID decisions (net · discovery) are built **standalone**, the Job · Restart Manager · WMI bindings are compiled too (compile time — the linker drops unused functions · unmeasured statement · §9).
- **Reason:** in workspace builds · `cargo test --workspace` features are merged in one go and agent turns them all on, so the gain exists only for `-p` standalone builds (the same observation as ADR-0175 「거부한 대안」 option C — the shell takes the daemon as a dev-dependency, so they merge). And the two 「PID 만 쓰는 소비자」 that ADR-0266 named are about to disappear — net is stripped in 3-3, and discovery is split into daemon · shell in 2-2. The remaining consumers are only the daemon (turns all of them on through agent anyway) and the shell (uses WMI · stopping · PID, and takes the daemon as a dev-dependency).
- **Rejected:**
  - (b) per-capability features (`process` · `group` · `file-holders` · `spawn` …) · default empty (precedent: net `server`) — each capability would carry `#[cfg(feature = …)]` inside platform, CI would gain feature-combination gates (the net 5a · 5b form), and tests would split per capability. The only gain is the standalone build time above.
  - (c) a for now, b when a condition is met (trigger = a consumer not linked together with agent shows measured compile cost · recorded as T-) — the net · discovery that would be such consumers are about to disappear, so there is nothing left to watch.

### D2. Crate name → `engram-dashboard-platform`

- **Reason:** the table above.
- **Rejected:** `engram-dashboard-os` — short, but overlaps in meaning with `std::os` and may be read as 「std 확장」 · `engram-dashboard-sys` — a name ADR-0175 rejected (in Rust `-sys` is the convention for C FFI binding crates).
- The prefix `engram-dashboard-` was not something to choose — the cap gate counts workspace members by that prefix (§3-1). This name goes as it is into the §3-1 manifest and the gate commands of §4-2 (`cargo tree -p engram-dashboard-platform …` · `-i engram-dashboard-platform`).

### D3. Shape of the process-group handle (§3-4) → B

- **What was chosen:** the 「고른 모양」 (chosen shape) table of §3-4 — platform = `windows.rs` (leaf · `LEFTOVER_EXIT_CODE` removed) + `GroupOwner` · `GroupRef` · five concrete types · agent = adapter `transport/process_group.rs` without `cfg` (`ProcessGroup` · `RetiringSignal` · two traits) · the agent `platform/` folder disappears. The weak handle is created in agent by the link putting the retiring flag on `GroupRef` of `downgrade()`, and the test helper `new_group` also stays on the adapter side (platform `testing` gets only the four OS-level helpers — §3-8).
- **Reason:** fits ADR-0266 decision 2 (OS dependence is the criterion) · decision 5 (the trait for fakes is on the user side), and there are no trait objects in platform's API. The names in ADR-0262 · CLAUDE.md stay in agent. The adapter sits under the links because the link is what hands out the handle and holds the retiring field.
- **Rejected:** A — moving `windows.rs` + `process_group.rs` to platform, as the 「한 덩이로」 wording of ADR-0266 decision 6 says. The cost is smallest, but platform's public API would gain `RetiringSignal` (link concept) · test-seam traits · trait objects, and `detached` would have to be opened as a public constructor (§3-4 「왜 A 가 아닌가」).
- **Cost:** fix the two wordings 「한 덩이로」 · 「새 손잡이를 따로 짓지 않는다」 of ADR-0266 decision 6 (§8 1 · 2) · an adapter of about 100 lines in agent (estimate — §9).

**What is fixed along with the decision (3rd edition 「공통」 — things not asked because they are independent of the option):** add a new strong owner type and block `Clone` (enforcing only one owner per group) · `terminate_raw` takes the exit code as an argument · kill causality · drop order · lock rule (§3-4 「공통 불변식」). The first two come with an ADR amendment (§8 2 · 3).

**The scope of the document follow-ups is settled by B (§6 3 · 9)** — in CLAUDE.md 「핵심 불변식」, `ProcessGroup` · `RetiringSignal` keep their names and only the file location changes to agent `transport/process_group.rs`.

---

## 8. Amendments recorded in ADR-0275

> **New number = ADR-0275** (0274 is written by another branch). Basis = user delegation (2026-10-04 「알아서 진행해」) → adopting this TRD's recommendation · ADR-0266 decision 6 was the main session's judgment (user delegation) from the start. Numbering · `Amends` linking · the stamps on the amended ADRs are done by the `/adr` skill. ★**Recorded on 2026-10-04 — the precondition for starting U0 (§6 9) is met.**★ ADR-0275 is **one ADR** carrying this TRD's part (amending ADR-0266 · ADR-0262 — below) and the sibling 1-1's part (amending ADR-0269 · ADR-0268 — TRD 1-1 §4-2). For rejected alternatives only those this TRD weighed are carried over — nothing is invented (CLAUDE.md 「결정 날조 금지」).

1. **ADR-0266 decision 6 「`ProcessGroup` 과 Job Object 래퍼를 한 덩이로 옮긴 것이다」** (+ its rationale sentence 「그 두 파일이 이미 그렇게 옮기도록 짜여 있다」)
   - **What changes:** only the OS layer goes to platform — `windows.rs` (leaf) + on top of it `GroupOwner` · `GroupRef` · five concrete types. The neutral handles `ProcessGroup` · `RetiringSignal` · test-seam traits (`Pinned` · `Births`) stay in agent as an adapter without `cfg` (`agent/src/transport/process_group.rs`). 「약한 손잡이(`Weak<JobObjectHandle>` — `platform/process_group.rs:118-119`)」 in the sub-items is read as 「platform `GroupRef`(agent `ProcessGroup` 이 감싼다)」.
   - **Rejected alternative:** A — move both files entirely to platform as the wording says (§3-4 「왜 A 가 아닌가」 · §7 D3).
   - **Reason:** the criterion is OS dependence (decision 2), and `RetiringSignal` is reading the link's retiring field, which has nothing to do with the OS. The trait for fakes goes on the user side (decision 5). The substance of decision 6 (strong owner = link · weak handle outward · read-only flag · only the port attached once · kill causality unchanged) stays.
2. **ADR-0266 decision 6 「새 손잡이를 따로 짓지 않는다」** (+ 「pty · stdio · codex transport 가 `#[cfg(windows)]` 로 들고 다니는 Job 핸들은 이 손잡이 뒤로 들어간다」)
   - **What changes:** platform newly builds the strong owner `GroupOwner` (not `Clone` — enforcing only one strong owner per group · outside Windows = no-op) · the weak handle `GroupRef` · five concrete value types (names = this TRD's drafts · fixed at implementation). The Job fields of the three links go behind **the strong owner `GroupOwner`**, not a weak neutral handle. 「주인 = 통로」 is kept by placement, not by type — the usage query's tree handle (`TreeRoot`) is also the owner of its own group (already so now — `usage/process.rs:438`).
   - **Rejected alternative:** proceeding without an amendment by reading `GroupOwner` as 「`JobObjectHandle` 의 겉만 바꾼 것」 — the names · shapes · internals of the public types change (`Arc` is moved inside), so we do not rely on that reading (§3-4).
   - **Reason:** to remove the `#[cfg(windows)]` field that leaked into the links (decision 1), a strong type that holds it with the same line on every OS is needed, and since platform does not know agent's `ProcessGroup`, the weak handle that strong owner hands out must also be a platform type (D3 = B).
3. **Conflict between the rationale of ADR-0266 decision 6 (= ADR-0262 「[구현] 고른 것」 「통째로 옮길 수 있게」) and decision 3 「도메인 지식은 남긴다」**
   - **What changes:** `windows.rs` is moved not whole but without the domain constant `LEFTOVER_EXIT_CODE = 0x7440` of claude leftover cleanup (`windows.rs:53` — the exit code that leftover cleanup gives to a process it has finished, ADR-0262 decision 6 · G2). platform's `terminate_raw` takes the exit code as an argument and the constant goes back to the agent leftover-cleanup side — **decision 3 wins.** ADR-0262 also gets `Amended by` attached by this item.
   - **Rejected alternative:** moving the constant whole — a domain value would enter platform (§2-5).
   - **Reason:** decision 3 is itself the criterion that separates OS code from domain knowledge. Even when it takes the value as an argument, `terminate_raw` remains one OS call · no logging, so the leftover-cleanup lock rule (ADR-0262 decision 8) is unchanged. ★「통째로 옮길 수 있게」 is text that came from a user decision (2026-09-29)★ — its substance (`windows.rs` is a leaf that uses only the `windows` crate and io) stays; what changes is only that one constant goes out as an argument.
4. **ADR-0266 「영향」 「열린 것 — `windows` crate feature 를 기능별 cargo feature 로 나눌지」**
   - **What changes:** closed — we do not split (D1 (a)). One `windows` dependency of platform with the union, and platform's only cargo feature is `test-support`.
   - **Rejected alternative:** (b) per-capability features · default empty · (c) a for now, b when a condition is met (§7 D1).
   - **Reason:** §7 D1.
5. **The crate-name placeholder of ADR-0266** (the `<platform>` of the gate command in 「영향」)
   - **What changes:** filled with `engram-dashboard-platform` (D2).
   - **Rejected alternative:** `engram-dashboard-os` · `engram-dashboard-sys` (§7 D2).
   - **Reason:** §7 D2.
6. **ADR-0266 「영향」 invariant 「`#[cfg(windows)]` · `cfg!(windows)` 는 platform crate 안에만」 and the timing of decision 8 (「이 단계(1-3)로 온다 — platform 이 생긴 뒤에 합친다」)** — this comes not from D1~D3 but from this TRD's scope decision (user 「알아서」 2026-10-03 → main-session recommendation · §1).
   - **What changes:** one time-limited exception — `cfg!(windows)` of shell `src-tauri/src/fsutil.rs:107` stays until U-W (after storage P3 lands on master), and in the meantime is on the expected list of the invariant gate (§4-4). The work of decision 8 (merging the several copies of atomic write into platform `fs`) is within 1-3 but goes after P3.
   - **Rejected alternative:** move now — storage P3 changes the same file heavily (`write_atomic_unless` · `copy_atomic` · `copy_aside` · new `retry_denied` — §2-4).
   - **Reason:** if it is moved before P3, the same file must be adjusted again when P3 lands.

**Nothing to fix (checked):**

- **ADR-0175** — ADR-0266 has already amended the platform side of decisions 1 · 2 and of the first item of 「거부한 대안」. D1 (a) rests on the same observation as that ADR's rejection of option C (features are merged in the workspace), and D2 follows that ADR's rejection of `-sys` — no wording conflicts.
- **ADR-0218** — ADR-0266 has already amended the home of decision 11 (agent `platform`). On top of that, this TRD only decides the location of `file_holders` · `process_tree` (→ platform `process::subtree` · name is a draft), and leaves the lock directory · 「파일 이름 = 스레드 id」 knowledge in `backend/codex` (that half of decision 11 · ADR-0266 decision 3) — no wording conflicts.

**Only paths · facts go stale — not an amendment** (record them as facts in 「근거」 of ADR-0275 and do not fix the old ADR bodies · on the code side the `// ADR-` anchors follow):

- ADR-0262 「OS 조각」 · 「[구현] 고른 것」's `platform/windows.rs` · `process_group.rs` paths → platform `group/windows.rs` · agent `transport/process_group.rs`.
- ADR-0230 「관련」 · 「영향」's precedent path `backend/mod.rs:44` (`console_command`) → platform `shell::console_command` (name is a draft).
- `crates/engram-dashboard-agent/src/platform` of ADR-0218 「관련」 → platform crate.
- ADR-0266 「맥락」 · the re-measurement list lacks the three copies of 「창 없이 띄우기」 (`CREATE_NO_WINDOW`) (§2-1 · §2-5) · 「`windows` feature 목록이 crate 별로 옮겨 간다」 in 「영향」 does not entirely leave agent — it remains as a dev-dependency because of two examples (§2-5 · §3-9) · the line number of `Weak<JobObjectHandle>` in decision 6 is `117-118` (§2-5).

---

## 9. Unverified · open items (미검 · 열린 것)

- **The baseline regression count has not been run in this edition** (§5-2 — measured right before U0 starts).
- **Whether discovery's `Win32_System_Threading` · `Win32_Foundation` features are needed by the WMI code** — unchecked (remove and compile in U7).
- **The minimal `windows` feature the agent examples require** — unchecked (including `Win32_Security` or not).
- **The POSIX branch is not compiled even after the move** — development · CI are Windows only (ADR-0230 status). In particular the POSIX branch of `TreeRoot` has never been compiled from the start.
- **Compile cost of not splitting `feature`** — unmeasured statement (the cost of D1 (a)). The decision does not rely on this number (reason = the consumers disappear — §7 D1).
- **When storage P3 lands** — the U-W trigger. Which of the functions P3 added to `fsutil.rs` belong to the 「원자적 쓰기」 family is re-measured then. ★→ TRD A §2-1 measured it, and `fsutil.rs` went whole to base `file` (A U2)★
- **The final text of the sibling 1-1 TRD's base header** — the four decisions of 1-1 (S2 · N1 · C4 · C5) were aligned to this edition (§5 Prerequisites). But since 1-1 is being edited in parallel with this edition, its final text was not seen — U1's base edit goes on top of the shape after 1-1 merges.
- **The adapter's line count (about 100 lines)** — an estimate, not measured (the location was settled by the D3 decision — `agent/src/transport/process_group.rs`).
- **The agent-side home of `LEFTOVER_EXIT_CODE`** — whether the adapter's `impl Pinned` passes it, or an exit-code argument is added to the test-seam trait `Pinned::terminate_raw` and leftover passes it, is decided by U3 (the trait has no argument now — `process_group.rs:86` · caller `leftover.rs:2231`). Since the real-process tests moving to platform currently assert that constant (`process_group.rs:543 · 804`), on the platform side they are changed to the test's own value.
- **The expected list of the invariant gate (§4-4)** — re-measured with that command at U8 (the list shrinks as units move tests).
