# TRD — T-40: claude 훅 도중 끊기의 약 300 초 멈춤 완화 — 우리 Job 안의 끊기 뒤 잔여물만 끝낸다 (S21)

> 상태: **초안 5판 · 리뷰 3 라운드 반영 · 재검 전 (2026-09-30)** — 5판 = 리뷰 3 라운드(아키텍트 PASS · 끝내기 안전 PASS · codex BLOCK 한 건) 반영: ★시계 두 값을 자물쇠 **안**에서 읽어 기록 순서와 표본 순서를 맞췄다 · 전이는 단조 값을 되감지 않는다(§3-2 · §3-6)★ · 끝내기 확정 구간 안의 OS 호출을 정확히 `TerminateProcess` 한 번으로 줄였다 — 핸들 닫기 · 실패 분류는 놓은 뒤(§3-3 · §3-5) · 시험 명세 보강(§5) · `process_start(0)` = 못 읽음(§3-4). 4판 = 리뷰 2 라운드(codex BLOCK · 끝내기 안전 FIX · 아키텍트 FIX — 1 라운드 지적은 전부 해소 확인) 반영: ★확인과 `TerminateProcess` 사이의 틈을 닫았다 — 검증은 자물쇠 밖에서 핸들로, 「아직 해야 하나」 재확인과 `TerminateProcess` 는 자물쇠 안에서 한 번에(잎 자물쇠 규칙의 유일한 예외 · §3-3)★ · 일꾼 기동 실패가 에피소드를 버려두지 않는다(§3-2) · 멤버 PID 와 표 · 시작시각의 짝을 두 번 읽어 맞춘다(§3-4) · 걷기는 「사라짐」은 건너뛰고 「못 읽음」은 Job 멤버일 때만 멈춘다(§3-4) · 표 스냅숏은 「끝」 오류만 끝으로 본다(§3-4 · §11) · 시계 닻을 기록 · 판마다 다시 잡고 뒤로 뛰면 그만큼 쉬며, 멤버마다 「지금보다 미래에 태어났나」를 본다(§3-6) · 시험을 고쳤다(§5 — 실제 시계 음성 시험 · 좀비 부모 · 명단 늘리기 시험을 작게) · 사실 정정 둘(§3-0 스폰 · `open_spawn` 의 `control` 주석). 3판 = 리뷰 1 라운드 반영 · 결정 4. 2판 = 측정 스파이크(§3-0). 코드 무변경. 동작 결정은 사용자가 내렸고(§1) 이 문서는 그것을 구현 가능한 명세로 옮긴다. 사용자 체감이 없는 내부 구현은 이 문서가 골랐다(**[고름]**).
>
> **입력:** 추적 [`docs/tracking.md` T-40](../../tracking.md) · 조사 [`docs/research/claude-interrupt-hook-hang-2026-09-29.md`](../../research/claude-interrupt-hook-hang-2026-09-29.md)(사실의 정본 — 여기서 되풀지 않고 `조사 §n` 으로 가리킨다) · 앞선 TRD [`trd.md`](trd.md) §3-4(claude JSON 끊기 설계 — ADR-0238). 판독 기준 = 브랜치 `v0.3.3/feat/chat-ux` 머리 `cd08209`. 이 문서의 `파일:줄` 은 전부 그 커밋에서 직접 열었다(4판이 더한 것 포함). `windows` crate API 는 이 PC 레지스트리의 `windows-0.58.0` 소스에서 확인했다. 측정 스파이크(2026-09-29 · claude 2.1.284) = 세션 스크래치의 `t40-spike\WRITEUP.txt` 와 그 옆 로그(**휘발 · 커밋하지 않음** — §3-0).
>
> **앵커:** ADR-0001(kill 인과) · ADR-0004(백엔드 지식 격리) · ADR-0006(락 순서) · ADR-0012(시험대) · ADR-0175(바닥 crate 입주 조건) · ADR-0217(제어 끝점이 에이전트 신원을 싣는다) · ADR-0218(신원 = PID + 시작시각) · ADR-0230(플랫폼 중립) · ADR-0238(claude 끊기 · 턴 열림 문) · ADR-0244(「중단하는 중…」 동안 Esc 무시 — 프론트) · ADR-0245(터미널 모드는 끊기 명령 없음).
>
> 표기: **[고름]** = 이 문서가 고른 내부 구현(사유를 같이 적는다). **[사용자]** = 사용자 결정. **에피소드** = 끊기 한 판 — 받아들인 끊기로 시작해 그 턴의 끝 또는 정리 한 번으로 끝난다(§3-2).

---

## 0. 결론 (먼저)

| 무엇 | 어디 | 요지 |
|---|---|---|
| **끊기 기록** | `backend/claude` 의 턴 열림 문 `TurnGate` — `open` 을 자물쇠 안으로 옮기고 에피소드 · 시계 닻 칸을 둔다 | 끊기 줄 함수가 **자물쇠 하나 안에서** 「문이 열렸나」를 보고 에피소드를 적는다(시계 닻도 그 자리에서 다시 잡는다). decoder 의 턴 끝은 **같은 자물쇠 안에서** 문을 닫고 에피소드를 지운다 |
| **N 초 확인** | `backend/claude/leftover.rs`(신설)의 일꾼 스레드 — 화신당 동시에 하나 | 일꾼 하나가 에피소드를 정리와 뒤 확인까지 쥔다. 가장 최근 끊기에서 N 초 지나면 정리한다. ★후보마다 검증은 자물쇠 밖에서 핸들로 하고, 「같은 에피소드 · 턴 끝 없음 · 가장 최근 끊기에서 N 초」 재확인과 `TerminateProcess` 는 **자물쇠 안에서 한 번에** 한다 — 재확인과 끝내기 사이에 끊기나 턴 끝이 끼어들 틈이 없다★. 정리를 한 번 돈 에피소드는 다시 쓰지 않는다 |
| **고르기** | 같은 파일의 순수 함수들 | 보호 집합 = 뿌리 사슬 + 래퍼 층(claude + 통로가 붙인 콘솔 호스트) + 그 아래 한 층. 래퍼 층 식구 수가 예상 모양과 정확히 같을 때만 정리한다. 후보 = Job 멤버 − 보호 집합 · 생성 시각 ≥ 문턱 · 생성 시각 ≤ 지금 · **부모가 죽었다**(결정 4). 신원 = (PID, 생성 시각)이고 ★멤버의 시작시각은 표 스냅숏 앞뒤로 두 번 읽어 같을 때만 쓴다★. 명단 · 표 · 멤버 시작시각 · 시계 중 하나라도 못 믿으면 정리하지 않는다(닫힌 쪽). 스파이크에서 규칙은 3/3 멈춤에서 잔여물 하나만 골랐다(§3-0) |
| **OS 조각** | `platform/windows.rs` 의 `JobObjectHandle` — 명단 · 검증 · 끝내기 | ① Job 멤버 명단(완전할 때만) ② 멤버 검증 — **같은 핸들로** 「우리 Job 소속 · 생성 시각 일치 · 아직 살아 있음」 → 검증된 핸들 ③ 그 핸들로 `TerminateProcess`. 다른 OS 에는 Job 이 없어 조각 자체가 없다 |
| **닿는 길** | `StdioTransport::process_group()`(`pub(crate)`) → 중립 손잡이 `ProcessGroup` | 통로는 「내가 띄운 프로세스 무리의 약한 손잡이 + 내 스폰이 뿌리 아래 붙이는 프로세스 수(콘솔 호스트)」만 내준다 — claude 를 모른다. 비Windows 는 `None` |
| **바닥 crate** | `base` 의 `platform` 에 함수 둘 | ① 프로세스 표(pid, ppid) 한 장 — 「끝」 오류만 끝으로 보고 그 밖의 실패는 `None` ② 시작시각 조회의 결과를 「앎 · 사라짐 · 못 읽음」 셋으로 가른다. 기존 `child_pids` · `process_creation_time` 은 그 위로 옮겨 동작이 같다 |
| **고지 · 대체 · 끄기** | 없음 [사용자 결정 1·2·3] | 화면은 평소 끊김과 똑같다(정리 뒤 `result` 도 보통 끊김과 같은 모양 — §3-0 · 3/3). 로그(warn)만 남긴다. 정리 뒤에도 턴 끝이 안 와도 더 하지 않는다. 끄는 수단은 두지 않는다 |
| **영향 범위** | claude JSON(stream-json) 한 경로 | codex · 터미널 모드 · 프론트 · 선 타입 · 버스 명령 · `open_spawn` 시그니처는 무변경(§3-9) |

- **구현 순서**(§6): ① 바닥 함수 둘 + OS 조각 + 중립 손잡이 + 층 걷기 → ② 순수 코어(가짜 포트 시험) → ③ 배선 → ④ 문서(ADR · CLAUDE.md · `base` 머리). 어느 단계에서 멈춰도 빌드와 기존 시험이 선다.
- **새 의존·feature 없음**(§11). `base` 는 기존 입주자 `platform` 에 함수 둘이 늘고 기존 함수 둘이 그 위로 옮긴다(게이트 셋 통과 · 자리 근거 — §11).
- **새 ADR 필요** — 가안 ADR-0246(§9). ADR-0238 결정 3 · 5 와 ADR-0244 결정 2 를 개정한다.
- **사용자 결정 필요 = 0 건**(§10): 끄는 수단 = 두지 않는다(결정 3) · 부모가 살아 있는 후보 = 건드리지 않는다(결정 4).

---

## 1. 사용자 결정 (2026-09-29)

| # | 결정 | 출처 |
|---|---|---|
| **채택안** | 대시보드가 완화한다 — ★우리 Job Object 안의 멈춘 잔여물만 죽이고 claude 는 살린다(재시작 아님)★ | 조사 §7 · T-40 |
| **고르기** | 실제로 끊기를 보낸 Esc 의 시각을 적는다. N 초 뒤 그 끊기의 턴 끝이 없고 가장 최근 Esc 에서 N 초 이상 지났으면, Job 멤버 중 claude 아님 · claude 의 직계 자식 아님 · 적은 시각 뒤에 생성 · 아직 살아 있음 인 것을 죽인다. 고아 여부 · 「bash」 인지 · CPU 상태는 주 기준이 아니다 | 조사 §7 |
| **PID 규칙** | ★PID 부모만으로 고르지 말 것 — Job 명단 + 생성 시각★ | 조사 §4 |
| **시계 · N** | 커널 생성 시각을 같은 시계(시스템 시각)의 타임스탬프와 비교 · N = 3 초(상수 한 곳 · 실측에서 세게 검증) | 조사 §7 |
| **OS 조각** | Windows 전용 조각은 작게(Job 멤버 명단 + 하나 끝내기) · 다른 OS 무동작 · 나중에 별도 플랫폼 모듈로 옮길 수 있게 | 조사 §7 · T-40 |
| **결정 1** | ★정리가 일어나도 화면 고지 없음★ — 평소 끊김과 똑같이 보이고 로그(tracing)만 남긴다. 사용자: 「그걸 왜 알려」 | 사용자 2026-09-29 |
| **결정 2** | ★정리 뒤에도 턴 끝이 안 와도 대체(자동 재시작) 없음★ — 멈추면 사용자가 오늘처럼 죽이고 다시 연다. 사용자: 「너무 복잡해」 | 사용자 2026-09-29 |
| **결정 3** | ★이 정리를 끄는 수단(환경 변수 등)을 두지 않는다★ — 스파이크에서 같이 죽은 정당한 프로세스가 0 이었다(11/11 · §3-0). 사용자: 「ㅇㅇ 두지 않고」 | 사용자 2026-09-29 |
| **결정 4** | ★**부모가 살아 있으면 건드리지 않는다**★ — 후보는 부모가 죽은 것만 끝낸다(부모가 fork 도중 죽은 「반쯤 죽은」 잔여물의 모양 — 잰 잔여물 3/3 이 그랬다 · §3-0 Q1). 백그라운드 작업이 끊기 뒤 띄운 자식(부모 작업이 살아 있다)을 지킨다(§8 ③). 사용자: 「ㅇㅇ 조건 더해줘 어차피 잘 안일어나는 일이니깐」. ★위 「고르기」 행의 「고아 여부는 주 기준이 아니다」는 그대로다 — 이것은 주 기준(Job 멤버 · 보호 밖 · 문턱 뒤 · 생존) 위에 **덧붙인 거르개**이고 후보를 줄이기만 한다★ | 사용자 2026-09-29 |

T-40 과 조사 §7 의 「TRD 에서 물을 것」 두 개(①고지 ②대체)는 결정 1·2 로, 리뷰 1 라운드가 올린 둘(끄는 수단 · 부모가 산 후보)은 결정 3·4 로 닫혔다.

---

## 2. 바뀌는 자리

| 파일 | 무엇 | 단계 |
|---|---|---|
| `crates/engram-dashboard-base/src/platform.rs` | `process_parent_table() -> Option<Vec<(u32, u32)>>`(Toolhelp 스냅숏 한 장 · `ERROR_NO_MORE_FILES` 만 끝 · 그 밖의 실패 = `None`) · `process_start(pid) -> ProcessStart { Known(u64), Gone, Unknown }` 추가. `child_pids`(`:150-201` — `parent == 0` 조기 반환 유지)와 `process_creation_time`(`:14-44`)은 그 위로 옮긴다(동작 동일). 모듈 머리(`:1`) 갱신 | ① |
| `crates/engram-dashboard-base/src/lib.rs` | 입주자 요약(`:3-4`)의 `platform` 설명 갱신 | ① |
| `crates/engram-dashboard-agent/src/platform/windows.rs` | `JobObjectHandle::member_pids`(+ 첫 용량 이음새) · `verify_member` · 검증된 핸들의 `VerifiedMember::{terminate_raw, settle}` · 결과 enum 들(§3-5) — 이 파일은 `windows` crate 와 `io` 만 쓴다 | ① |
| `crates/engram-dashboard-agent/src/platform/process_group.rs` **신설** | 중립 손잡이 `ProcessGroup`(약한 Job 손잡이 + `root_attached`) + 중립 결과(Windows 결과를 감싼다) + 실프로세스 시험(시험마다 `#[cfg(windows)]`) | ① |
| `crates/engram-dashboard-agent/src/platform/process_tree.rs` | 깊이 제한 · 닫힌 실패의 순수 걷기 `walk_levels` — 규칙(부모보다 먼저 태어난 것은 자식 아님 · 신원 방문 표시)은 `walk`(`:105-126`)와 같다. `subtree` 는 그대로 | ① |
| `crates/engram-dashboard-agent/src/platform/mod.rs` | 모듈 등록 · 머리 doc 의 「플랫폼 질의 셋」 갱신(`:1-10`) | ① |
| `crates/engram-dashboard-agent/src/transport/stdio.rs` | `job_handle` 칸(`:72-73`)을 `Arc<JobObjectHandle>` 로 · `pub(crate) fn process_group()` 추가(콘솔 호스트 수는 `CREATE_NO_WINDOW` 블록 `:101-108` 과 같은 Windows 갈래 안에서 정한다) | ① |
| `crates/engram-dashboard-agent/src/backend/mod.rs` | `console_wrapper_depth()` 를 `console_command`(`:37-57`) 옆에 | ① |
| `crates/engram-dashboard-agent/src/backend/claude/leftover.rs` **신설** | 상수 · 에피소드 상태기계 · 시계 닻 · 순수 계획 함수 · 포트 트레이트 둘 · 일꾼 · 로그 | ② · ③ |
| `crates/engram-dashboard-agent/src/backend/claude/mod.rs` | `TurnGate`(`:771-788`)의 `open: AtomicBool` → `Mutex<GateState>` · decoder 의 문 여닫기(`:1111-1117`)를 전이 때만 자물쇠로 · `interrupt_line`(`:833-835`)이 자물쇠 안에서 확인 + 기록 · `open_spawn`(`:437-440`)이 정리기를 조립 · ★`open_spawn` 의 `control` 인자 주석(`:426-428` — 「여기서 또 읽으면 한 spawn 이 같은 값을 두 수단으로 보낸다」)과 버림 줄(`:431`)을 고친다 — 이제 `agent_id` 를 **로그 귀속으로만** 읽는다(통로로 다시 보내지 않으므로 그 주석이 막는 「두 수단」이 아니다)★ | ③ |

프론트(`src/`) · 선 타입(`types.rs` 의 `OutputEvent`) · 프로토콜 · 데몬 · 셸 · `open_spawn` 시그니처는 무변경이다.

---

## 3. 설계

### 3-0. 실측 (스파이크 2026-09-29) — 이 설계가 기대는 사실

- **출처:** 세션 스크래치 `t40-spike\WRITEUP.txt` + `main.log` · `pilot.log` · `main\` · `pilot\`(요약 · 멤버 명단 · claude 원문) — ★**휘발 · 커밋하지 않음**★. 하네스 = Rust · `windows 0.58.0`(저장소와 같은 판). ★**스폰은 대시보드를 본떴을 뿐 같지 않다**★ — 같은 것 = `cmd.exe /c claude …` stream-json 인자 · 파이프 · `CREATE_NO_WINDOW` · `spawn` → `JobObjectHandle::new` → `assign`(`stdio.rs:110-127` · `platform/windows.rs:30-75` 를 옮김) · 환경 `MAX_THINKING_TOKENS=8000`(대시보드도 json 모드에서 기본 주입한다 — `claude/mod.rs:215-230` · ADR-0049). 다른 것 = 대시보드가 주지 않는 `--include-hook-events` 를 더했고(대시보드 인자 = `claude/mod.rs:170-201`), `--model haiku` 를 박았다(대시보드는 프로필 추가 인자로만 모델을 받는다 — 시험 `claude/mod.rs:3118-3141`), 데몬 끝점이 드는 `--mcp-config` · `--append-system-prompt-file` 은 뺐다. 측정 전용으로 Job 완료 포트를 붙여 새 멤버마다 곧바로 핸들을 열었다(PID 재사용 없음). ★**그래서 실제 대시보드 경로의 판정은 §7 G1 이다**★.
- **판:** claude **2.1.284**(`system/init`) — 조사는 2.1.283 이었다(오늘 npm 갱신). 사용자의 실제 훅(`UserPromptSubmit` 넷 — 느린 둘 = `handoff-trigger.sh` · `wiki-preconsult.sh`) · 보낸 뒤 200 ms 끊기(조사 실험 1 E).
- **시행:** claude 프로세스 넷에서 끊기 11 번 · **멈춤 3/11**. 멈춤 아닌 끊기의 `result` = 끊기 뒤 285–1394 ms.

| 물음 | 실측 | 이 문서에 준 것 |
|---|---|---|
| Q1 멈춤을 쥔 프로세스가 **우리 Job** 에 있나 | ★**예 · 3/3**★ — 멈춤마다 끊기 뒤 살아남은 것은 정확히 하나: `Git\usr\bin\bash.exe <훅>.sh` · 깊이 5 · 부모(깊이 4 훅 bash) 죽음 · 끊기 뒤 +203 / +244 / +233 ms 생성. 근거 셋 = Job 완료 포트의 새 멤버 알림 + `IsProcessInJob(h, 우리 Job)` · 끊기 +3 s · +10 s 의 `JobObjectBasicProcessIdList` 에 있음(멤버 = cmd · conhost · claude · 잔여물 = 4, 멈춤 아닌 시행 = 3) · `terminate_member` 모양의 끝내기 = 끝남. Job 밖 전역 훑기에서 훅 관련 프로세스 0. 곁증거: 훅 stderr 의 MSYS `child_copy: … windows pid 18236, Win32 error 299` 의 PID 가 그 잔여물 → **부모가 fork 복사 도중 죽어 반쯤 fork 된 자식**(추론 — 조사 §2 ②⑤ 와 맞물린다) | §8 ⑦ 전제 해소 · §5 재정의 |
| Q2 래퍼 깊이 | ★claude.exe 는 `cmd.exe` 의 **직계** 자식 — 런처 층 없음(4/4)★. `claude` 는 npm shim `claude.cmd` 이고 **같은 cmd.exe 안에서** `…\@anthropic-ai\claude-code\bin\claude.exe` 를 띄운다(node 없음). ★단 깊이 1 층은 **둘**이다★ — `conhost.exe`(스폰 전 시각 +4.6–4.9 ms · cmd 의 콘솔 호스트 — `CREATE_NO_WINDOW` 여도 생긴다) · `claude.exe`(+61–66 ms). 셋 다 우리 Job. 훅 사슬 = d2 `Git\bin\bash.exe`(claude 의 직계 자식 · 런처) → d3 conhost + `usr\bin\bash.exe -c` → d4 훅 bash → d5+ fork. taskkill.exe = d2 · 그 conhost = d3. `spawn` → `assign` 틈 = 33–40 µs · conhost 는 그 뒤 약 1.6–2.1 ms · claude.exe 는 약 58–63 ms | §3-4 claude 가려내기 · §3-8 · §7 G2 · §8 ④⑥ |
| Q3 끊기 뒤 우리 Job 에 생긴 것 | 시행마다 24–26 개: taskkill@d2 ×4(두 차례 — 끊기 +1–5 ms · 약 +1505 ms · 최장 278 ms) · 그 conhost@d3 ×4(최장 277 ms) · 훅 bash@d5 ×5 · @d6 ×5(보통 ≤212 ms) · cat/jq/awk/grep@d6–7(≤36 ms) · cygwin-console-helper@d6–7(≤88 ms) · conhost@d7–8(≤102 ms). ★**끊기 +3 s 에 살아 있던 것은 11/11 시행에서 멈춤 잔여물뿐**★. 잔여물 아닌 최장 = 같은 모양의 bash 1171 ms(스스로 끝났고 그 턴의 `result` 는 +1394 ms). ★안 잰 것★ = 도구 실행 중 끊기 · 백그라운드 작업 · stdio MCP 서버 · 끊기 뒤 시작하는 훅 | N = 3 초의 여유 · §7 G6 · §8 ③ |
| Q4 §3-4 규칙을 글자 그대로 | 후보 = 정확히 잔여물 하나(3/3) · 끊기 +3 s 와 +10 s 의 후보 집합이 같다(3/3). ★끝내기는 멈춤을 확인한 +10 s 에 했다 — +3 s 에 끝내 보지는 않았다★. 끝내기 → `result` = **18 / 19 / 21 ms** · 멈춘 훅의 `hook_response … cancelled` 가 같은 순간 · claude.exe 생존 · 이어 보낸 턴을 같은 프로세스가 1.9–2.1 s 에 답했다. Job 밖은 아무것도 안 끝냈다 | §0 · §3-1 시각 |
| 시계 | 뿌리 cmd.exe 생성 시각 − 스폰 직전 `SystemTime` = +0.67…+0.79 ms(4/4 · 음수 없음) · 끊기 뒤 첫 프로세스(taskkill) = 끊기 +1.1…1.4 ms → 커널 생성 시각은 여기서 1 ms 아래 해상도로 보인다(간접). 잔여물은 문턱보다 ≥ +203 ms 뒤 | §3-6 |
| 정리 뒤의 `result` 모양 | 멈춤 3 회 모두 `subtype:"error_during_execution"` · `is_error:true` · `terminal_reason:"aborted_streaming"` 이고 바로 앞에 합성 줄 `[Request interrupted by user]` 가 왔다 — 멈춤 아닌 끊기와 같은 모양(`claude/mod.rs:873` 의 끊김 판정이 그대로 잡는다 → `TurnEnd{Interrupted}`) | 결정 1(「평소 끊김과 똑같다」)이 선 줄 수준에서 성립 — GUI 는 §7 G1 |

### 3-1. 흐름 한눈에

```
Esc / 버스 agent.interrupt
 → manager.interrupt (manager.rs:2875-2877 — 명부 락은 get_session 뒤 이미 놓았다)
 → session.interrupt (session.rs:636-638 — 락 없음)
 → StdioTransport::interrupt (stdio.rs:395-408) → 꽂힌 끊기 줄 함수 호출
     [backend/claude] TurnGate 자물쇠 안에서 한 번에(시계 두 값 — 벽 · 단조 — 도 자물쇠 안에서 읽는다):
                      문이 닫혀 있으면 None(오늘 그대로 — 기록 없음)
                      열려 있으면 시계 닻을 다시 잡고 에피소드를 열거나(새 문턱) 이어 적고 · 일꾼이 없으면 띄울 몫을 받는다
                      자물쇠를 놓고 → (필요하면) 일꾼을 띄우고 → 줄을 준다
 → 통로가 줄을 입력 큐에 넣는다 → 라이터 스레드가 claude stdin 에 쓴다

(보통) claude 가 result 를 낸다 → decoder 가 Ended 로 분류 → 자물쇠 안에서 문 닫기 + 에피소드 지우기
       → 일꾼이 깨어 에피소드 없음을 보고 끝난다(아무 일 없음)

(멈춤) result 가 안 온다 → 일꾼이 「같은 에피소드 · 가장 최근 끊기에서 N 초」를 확인하고 정리 한 판
       → 명단 · 시작시각 ×2 · 표 · 층 걷기 · 고르기(하나라도 못 믿으면 멈추고 warn)
       → 후보마다: 자물쇠 밖에서 핸들로 검증 → 자물쇠 안에서 재확인 + TerminateProcess → 놓고 → warn 한 줄
       → 파이프가 닫혀 claude 가 result 를 낸다(스파이크 — 끝내기 뒤 18–21 ms · 3/3 · §3-0) → 평소 끊김과 같은 화면
         (조사 §3-3 의 0.2–0.4 초는 taskkill.exe 기동까지 잰 값이다)
       → (하나라도 끝냈는데 N 초 뒤에도 같은 에피소드면 warn 한 줄만 더 — 대체 없음, 결정 2)
```

### 3-2. 기록 — 문 확인과 기록을 한 자물쇠로

**2판의 결함(리뷰 1 A — 코드로 확인):** 2판은 `TurnGate.open`(`claude/mod.rs:771-788` — `Relaxed` 원자값)을 읽은 **뒤** 따로 기록했다. 「끊기 줄 함수가 열림을 읽음 → 펌프가 턴 끝으로 문을 닫고 기록을 지움 → 끊기 줄 함수가 기록」 순서가 나면 끝난 턴에 기록이 남고, 그 줄은 한가한 CLI 에 닿아 응답 한 줄만 내고 `result` 가 없어(`claude/mod.rs:763-764` — 실측 B2 S6) 기록이 영영 안 지워진다.

**[고름] `open` 을 자물쇠 안으로 옮기고, 확인 + 기록 · 닫기 + 지우기를 각각 한 번의 자물쇠 구간으로 한다.**

```rust
// backend/claude/mod.rs — 화신 공유 값(모양은 ADR-0238 결정 3 을 개정한다)
struct TurnGate { state: Mutex<GateState> }

// backend/claude/leftover.rs — 순수 상태(시계 값은 인자로 받는다 · OS 없음 · 패닉 없음)
pub(super) struct GateState {
    open: bool,                        // 쓰는 이는 decoder 하나
    episode: Option<Episode>,          // 지금의 끊기 한 판
    next_gen: u64,                     // 에피소드 세대 — 새 에피소드마다 +1
    worker: bool,                      // 이 화신에 일꾼이 떠 있나
    anchor: Option<ClockAnchor>,       // 벽시계 ↔ 단조 시계 닻(§3-6) — 정리기가 없으면 None
    clock_hold_until: Option<Instant>, // 뒤로 뛴 시계를 본 뒤 새 에피소드의 판을 막는 끝(§3-6)
}
struct Episode {
    gen: u64,
    first_wall: u64,     // 이 에피소드 첫 끊기의 벽시계(FILETIME 척도) — 생성 시각 문턱
    first_mono: Instant, // 같은 순간의 단조 시각
    last_mono: Instant,  // 이 에피소드 가장 최근 끊기의 단조 시각 — 「N 초 지났나」
    clock_ok: bool,      // 첫 끊기 때 시계를 믿을 수 있었나(§3-6)
    phase: Phase,
}
enum Phase { Waiting, Cleaning, Spent { at: Instant }, Done }
```

- **끊기 줄 함수**(`interrupt_line` — 오늘 `claude/mod.rs:833-835`) — 자물쇠를 잡고 **그 안에서** 벽 · 단조 시각을 읽은 뒤 같은 구간의 `try_interrupt(now_wall, now_mono)`:
  - ★**왜 자물쇠 안에서 읽나 [리뷰 3 codex]**★ — 4판은 밖에서 읽었다. 끊기 A 가 t1 을 읽고 멈춘 사이 B 가 t2 > t1 을 읽고 먼저 기록하면, 뒤이어 들어온 A 가 `last_mono` 를 t1 로 **되감아** 일꾼이 B 의 t2 + N 이 차기 전(t1 + N)에 끝낼 수 있었고, A 의 묵은 표본이 시계 닻을 과거로 돌릴 수도 있었다. 자물쇠 안에서 읽으면 기록 순서가 곧 표본 순서다. 읽기는 막히지 않는 카운터 · 시스템 시각 읽기라 §3-3 자물쇠 규칙의 예외 ⓐ 로 적는다.
  - ★**전이는 단조 값을 되감지 않는다(방어 · 순수 함수 안)**★ — 표본이 어디서 왔든 `last_mono = max(last_mono, 표본)` · 닻은 표본의 단조 값이 닻보다 이르면 다시 잡지 않는다 · 새 에피소드의 `(first_wall, first_mono)` 는 표본이 닻보다 이르면 **닻 값으로 올려** 쓴다(닻의 벽시계는 그 표본보다 늦은 순간의 값이라 문턱이 늦어질 뿐 — 덜 고르는 쪽). 그래서 앞 에피소드가 닫힌 뒤(턴 끝 · 정리)의 새 에피소드가 그보다 오래된 표본으로 문턱을 앞당길 길이 없다 — 닻은 기록 · 판마다 앞으로만 옮겨지고, 첫 끊기의 문턱은 적어도 그 닻의 순간이다.
  - `open == false` → 거절(`None` · 기록 없음 — 오늘 그대로).
  - 정리기가 없으면(비Windows · 조립 실패) 문 확인만 하고 줄을 준다 — 에피소드를 만들지 않는다.
  - 시계 닻을 본다(§3-6 — 다시 잡거나 · 뒤로 뛰었으면 쉼 끝을 세운다). 잰 값은 돌려받아 자물쇠 밖에서 로그로.
  - 에피소드 없음 · `Spent` · `Done` → **새 에피소드**(`gen = next_gen++` · 새 `first_wall` · `first_mono` · `clock_ok = 쉼 끝이 지났나` · `Waiting`).
  - `Waiting` → `last_mono` 만 민다(문턱 유지).
  - `Cleaning` → `last_mono` 를 밀고 `Waiting` 으로 되돌린다 — 도는 판이 다음 끝내기 확정(§3-3)에서 멈춘다.
  - `worker == false` 면 참으로 바꾸고 「일꾼을 띄워라」를 돌려준다. 자물쇠를 놓은 **뒤** 띄운다.
  - 적는 시점은 줄을 돌려주기 **전**(곧 통로가 큐에 넣기 전)이라 문턱은 claude 가 그 줄을 받는 시각보다 늘 이르다.
- **일꾼 기동 실패 [리뷰 2 C2 — 고름: 에피소드를 `Done` 으로]:** 자물쇠를 다시 잡아 `worker = false` 로 되돌리고 **지금의 에피소드(세대가 무엇이든)를 `Done`** 으로 둔 뒤 warn. 사유: 그 사이 다른 끊기가 같은 에피소드를 밀었거나 새 에피소드를 열었어도 「일꾼이 떠 있다」고 믿고 아무도 띄우지 않았으므로, 에피소드를 그대로 두면 일꾼 없는 에피소드가 남는다. 곧바로 다시 띄우지 않는 것은 스레드 기동 실패가 자원 고갈이라 곧 다시 실패할 가능성이 높고, 되풀이에는 따로 상한이 들기 때문이다. 결과는 놓치는 쪽이다(그 끊기는 오늘처럼 멈출 수 있다) · 다음 끊기는 새 에피소드로 다시 띄운다.
- **줄이 큐에 못 들어간 경우 [리뷰 2 A7 — 문서화 · 코드 변경 없음, 사유]:** 통로가 `push` 에 실패하면(`stdio.rs:403`) 에피소드는 남는다. 큐가 닫히는 길은 셋이다 — ⓐ `shutdown()`(`stdio.rs:434` — Job 도 끝나 판이 아무것도 못 고른다) ⓑ 라이터 기동 실패(`stdio.rs:266-272` — 스폰 직후라 입력이 한 번도 claude 에 닿지 않아 claude 가 턴을 열지 않고, 문은 진행 줄에서만 열리므로(`claude/mod.rs:1111-1117`) 에피소드가 생길 수 없다) ⓒ 쓰기 오류 · 라이터 패닉(`input_queue.rs:266-289` — claude 의 stdin 이 깨져 그 뒤 어떤 입력도 못 간다). ⓒ 에서는 `push` 가 **성공한 뒤** 쓰기가 실패할 수도 있어(그 모듈 계약 — 「받아 둔 뒤의 실패」는 호출자에게 돌아갈 길이 없다 · `input_queue.rs:266-269`), `push` 실패에만 거는 되돌리기는 구멍을 반만 막고 `InterruptLine` 계약(ADR-0238 결정 2)을 넓힌다. 그래서 되돌리지 않고 §8 ⑬ 에 한계로 적는다 — 남는 노출은 「stdin 이 깨진 채 도는 턴」에서 판 한 번이고, 고르는 것은 결정 4 로 부모가 죽은 것뿐이다.
- **decoder**(`consume_live_line` — `claude/mod.rs:1111-1117`) — `open` 을 쓰는 것은 decoder 하나뿐이라, 자기 쪽에 그림자 값을 두고 **바뀔 때만** 자물쇠를 잡는다(턴마다 두 번꼴):
  - 진행(`Progress`)이고 그림자가 닫힘 → `open = true`.
  - 끝(`Ended`) → `open = false` + `episode = None`(한 구간). 진행은 에피소드를 건드리지 않는다. 이어받기(`LineSource::Transcript`)는 이 자리를 지나지 않는다(오늘 그대로).
- **불변식:** 자물쇠 밖에서 보이는 어떤 순간에도 「에피소드가 `Waiting`/`Cleaning` 이면 그것은 문이 열린 뒤 받아들인 끊기다」 · 「에피소드가 `Waiting`/`Cleaning`/`Spent` 이면 일꾼이 떠 있다」(기동 실패는 에피소드를 `Done` 으로 내려 두 번째 불변식을 지킨다).
- **자물쇠 규칙:** §3-3 끝의 문단이 정본이다(잎 자물쇠 + 예외 둘 — 시계 읽기 · `TerminateProcess` 호출 하나). 독(poison)을 견딘다 — 잡기는 늘 `lock().unwrap_or_else(PoisonError::into_inner)`(선례 `stdio.rs:160`). 안의 셈은 필드 대입과 `checked_add` · `saturating_duration_since` 뿐이라 패닉이 없다.
- **사용자 규칙과의 대응 [고름 — 사유]:** 사용자 규칙은 「실제로 끊기를 보낸 Esc 의 시각 · 새 턴의 뒤 Esc 는 덮어쓴다 · 되풀이 Esc 는 무시되어 적히지 않는다(ADR-0244) · 따로 지우지 않는다」다.
  - Esc 한 번의 경로에서는 에피소드의 첫 끊기 = 그 Esc 라 문턱 · 대기 기준이 규칙과 같다.
  - ADR-0244 의 무시는 **프론트 창마다** 선다(결정 a) — 버스의 LLM 이나 다른 창의 끊기는 같은 턴 안에서 또 온다. 정리 전이면 같은 에피소드로 이어 적고, 정리를 한 번 돈 뒤면 새 에피소드(새 문턱)다.
  - ★「따로 지우지 않는다」와는 다르다★ — 턴 끝에서 지우고, 정리를 한 번 돈 에피소드는 다시 쓰지 않는다(한 번 쓰고 버림). 그렇지 않으면 턴 끝을 놓친 기록(§8 ②)이 다음 턴들로 옛 문턱을 끌고 간다(리뷰 1 B). 사용자 체감은 없다 — 오살을 줄이는 쪽으로만 다르다.
- **Esc 와 버스 명령을 가르지 않는다 [고름]:** backend 에는 둘 다 같은 `interrupt()` 로 닿고(CLAUDE.md 「LLM-우선 제어」 — 같은 핸들), 멈춤도 같게 난다.

### 3-3. N 초 확인 — 일꾼 하나가 에피소드를 끝까지 쥔다

- **상수:** `leftover.rs` 의 `const INTERRUPT_LEFTOVER_GRACE: Duration = Duration::from_secs(3);` — 대기(N)와 정리 뒤 확인 간격이 이 하나를 쓴다. 스파이크(§3-0 Q3)의 여유: 멈춤 아닌 끊기의 `result` ≤ 1394 ms · 끊기 뒤 생긴 정상 프로세스의 최장 수명 1171 ms · 끊기 +3 s 에 살아 있던 것은 11/11 에서 잔여물뿐.
- **일꾼 = 화신당 동시에 하나인 스레드**(이름 `engram-claude-leftover`). `worker` 칸이 자물쇠 안에 있어, 일꾼이 떠 있는 동안 새 끊기는 일꾼을 더 띄우지 않고 상태만 바꾼다. 일꾼은 **현재 에피소드**를 처리한다 — 에피소드가 바뀌면(새 세대) 그것을 이어 맡는다.
- **일꾼의 한 걸음 = 자물쇠 한 구간의 `next_step(now_mono)`:**

  | 상태 | 걸음 |
  |---|---|
  | 에피소드 없음 · `Done` | `worker = false` → 끝난다 |
  | `Waiting` · 가장 최근 끊기 + N 전 | 남은 시간을 받아 자물쇠 밖에서 잔다 |
  | `Waiting` · 지남 | `Cleaning` 으로 바꾸고 `(gen, first_wall, clock_ok)` 를 받아 자물쇠 밖에서 **정리 한 판**(§3-4) |
  | `Spent{at}` · `at + N` 전 | 남은 시간만큼 잔다 |
  | `Spent{at}` · 지남 | `Done` 으로 바꾸고 warn 「정리 뒤에도 턴 끝이 없다」 한 줄 → 다음 걸음에서 끝난다. **다시 죽이지 않고 재시작도 없다(결정 2)** |

- ★**끝내기 확정 — 재확인과 `TerminateProcess` 를 한 자물쇠 구간에서 [리뷰 2 C1]**★. 3판은 재확인(`still_due`)을 자물쇠 안에서 한 뒤 **놓고** 끝냈다 — 그 틈에 새 끊기나 턴 끝이 들어와도 일꾼은 끝냈다. 4판의 후보 하나 처리:
  1. 자물쇠 **밖**: `verify(who)` — 같은 핸들로 우리 Job 소속 · 생성 시각 · 생존을 확인하고 **검증된 핸들**을 받는다(§3-5). 실패면 그 후보를 건너뛴다(끝내지 않는다).
  2. 자물쇠를 잡고 **그 안에서** 단조 시각을 읽어 `still_due(gen, now_mono)` — 에피소드가 있고 · 세대가 같고 · `Cleaning` 이고 · 가장 최근 끊기에서 N 이 지났나. 거짓이면 핸들을 **쥔 채** 자물쇠를 놓고, 놓은 **뒤에** 핸들을 버리고(`CloseHandle`) 그 판의 나머지를 버린다(debug).
  3. 참이면 **자물쇠를 쥔 채** 검증된 핸들로 `TerminateProcess` **호출 하나만** 하고 그 날것 결과(성공/실패)를 쥔 채 자물쇠를 놓는다.
  4. 놓은 **뒤에** 결과를 가른다 — 실패면 `WaitForSingleObject(h, 0)` 로 「그 사이 스스로 끝났나(`Gone`)」를 보고, 핸들을 닫고, 로그를 쓴다(리뷰 3 아키텍트 — 4판은 핸들을 소비하는 `terminate(self)` 라 닫기 · 실패 분류가 자물쇠 안에서 돌았다).
  - 그래서 끊기 줄 함수 · decoder 가 자물쇠를 잡는 순간은 「끝내기 전」이거나 「끝내기 뒤」 둘 중 하나다 — 판 도중의 새 끊기(`Waiting` 으로 되돌림)나 턴 끝(에피소드 지움)은 그 뒤의 어떤 `TerminateProcess` 도 막는다.
  - **더 깔끔한 순서는 찾지 못했다:** 「끝내는 중」 표식을 자물쇠 안에 세우고 놓은 뒤 끝내는 방식은 표식이 선 동안의 끊기 · 턴 끝을 기다리게(막게) 하거나 무시하게 되어 같은 문제로 돌아간다. 재확인과 끝내기가 원자여야 하는 이상, 가장 짧은 원자 구간이 이것이다.
- **판 끝 `finish_pass(gen, killed, now)`(자물쇠 한 구간):** 같은 세대가 아직 `Cleaning` 이면 — 하나라도 끝냈으면 `Spent{at: now}`(N 뒤 확인), 하나도 못 끝냈으면 곧바로 `Done`(판 자신의 warn 이 사유를 이미 남겼다 — §3-8). `Waiting` 으로 되돌아갔거나 에피소드가 없거나 세대가 다르면 건드리지 않는다.
- **한 번 쓰고 버림:** `Spent`/`Done` 인 에피소드는 다시 판을 돌지 않는다. 그 턴에 새 끊기가 오면 **새 에피소드 · 새 문턱**이다(§3-2). 옛 에피소드의 N 뒤 확인 warn 은 세대가 달라져 나가지 않는다.
- **취소 신호가 없다 — 깰 때마다 다시 잰다:**

  | 경우 | 무엇이 일어나나 |
  |---|---|
  | 턴 끝이 왔다 | decoder 가 에피소드를 지웠다 → 깨어 「없음」 → 끝 · 판 도중이면 다음 끝내기 확정에서 멈춘다 |
  | 새 끊기가 왔다(정리 전) | 같은 에피소드의 `last_mono` 가 밀렸다 → 남은 만큼 더 잔다 |
  | 새 끊기가 왔다(판 도중) | `Waiting` 으로 되돌아갔다 → 판이 다음 확정에서 멈추고 새 끊기 + N 까지 기다린 뒤 **같은 문턱**으로 다시 판을 돈다 |
  | 새 끊기가 왔다(정리 뒤) | 새 에피소드 → 같은 일꾼이 이어 맡는다 |
  | 세션 kill · 자연 종료 | 손잡이가 `Weak` 라 통로가 사라졌으면 명단이 빈다 · 아직 통로가 있어도 `shutdown()` 이 Job 을 통째 끝냈으면(`stdio.rs:443-448`) 명단이 비거나 멤버가 `Gone` 이다 → 아무것도 안 죽이고 에피소드는 `Done` |

- **수명 상한:** 일꾼은 마지막으로 받아들인 끊기 뒤 늦어도 **2N + 정리 시간** 안에 스스로 끝난다(끊기가 계속 오면 그만큼 산다 — 동시에 하나라 쌓이지 않는다). join 하는 이 없음(`stdio.rs:248-253` 라이터와 같은 모양). 쥐는 것은 `Arc<TurnGate>` · 정리기 `Arc` · `Weak` Job 손잡이뿐이라 **프로세스 수명을 늘리지 않는다**(§4 소유권).
- ★**자물쇠 규칙(정본 — CLAUDE.md 락 순서 줄이 이 문장을 옮긴다)**★ — **`TurnGate.state` 는 잎 자물쇠다: 쥔 채 로그 · 스레드 기동 · emit · 잠 · 다른 자물쇠 · 그 밖의 OS 호출을 하지 않으며, 예외는 둘뿐이다 — ⓐ 시계 두 값 읽기(단조 카운터 · 시스템 시각 — 막히지 않는 읽기 · 기록 순서를 표본 순서와 맞추려고) ⓑ 끝내기 확정에서 검증된 핸들로 부르는 `TerminateProcess` 호출 정확히 하나(핸들 닫기 · 실패 분류 · 로그는 놓은 뒤).** 예외 ⓑ 가 안전한 이유: `TerminateProcess` 는 비동기다 — 종료를 시작하고 곧 돌아오며 대상이 끝나기를 기다리지 않는다(MS 문서의 서술 · 이 문서가 재지는 않았다) · 정리 판(드묾 — 멈춤 때만) 동안에만 일어난다 · 그 자물쇠를 기다릴 수 있는 것은 끊기 줄 함수(사람 · LLM 속도)와 decoder 의 문 전이(턴마다 두 번꼴)뿐이고 기다림은 그 호출 한 번 길이다 · 안에서 다른 자물쇠를 잡지 않아 락 순서에 간선이 없다. 그 밖의 OS 호출(명단 · 표 · 시작시각 · 검증 · 핸들 닫기)은 전부 자물쇠 밖이다.

### 3-4. 고르기 — 정리 한 판의 순서와 순수 함수

**판의 순서 [고름] — 앞에서부터 하나라도 서지 않으면 그 판은 아무것도 안 끝낸다(닫힌 실패):**

| # | 단계 | 서지 않으면 |
|---|---|---|
| 0 | 약한 손잡이를 올린다 | 통로가 사라짐 → debug |
| 1 | 에피소드의 `clock_ok`(§3-6) | 첫 끊기 무렵 시계가 뒤로 뛰었다 → **warn** |
| 2 | Job 명단(완전한 것만 — §3-5) | 조회 실패 · 불완전 → **warn** · 빈 명단(에이전트가 이미 죽음 · Job 이 끝남) → debug — ★모양 가드 전에 끊는다★(가짜 모양 warn 방지) |
| 3 | 멤버 시작시각 첫 읽기 `s1`(명단 직후) | — (못 읽은 멤버는 그 멤버만 빠진다) |
| 4 | 프로세스 표 한 장(`base::platform::process_parent_table`) | 스냅숏 실패 · 목록 중간 오류 → **warn** |
| 5 | 멤버 시작시각 둘째 읽기 `s2`(표 직후) — ★`s1 == s2` 이고 둘 다 앎일 때만 그 멤버를 쓰고, 그 값을 끝내기의 기대 시작시각으로 쓴다★ | — (어긋난 멤버는 그 멤버만 빠진다) |
| 6 | 층 걷기 `walk_levels`(깊이 0 ‥ depth+1 — 표 기반) | 뿌리 신원 어긋남 · 보호 깊이 안의 **Job 멤버**가 시작시각을 못 읽음 → **warn** |
| 7 | 모양 가드 `keep_set` | 래퍼 층 식구 수 ≠ 기대 → **warn** |
| 8 | 고르기 `select_leftovers`(보호 밖 · 문턱 ≤ 생성 ≤ 지금 · 부모 죽음) | 후보 0 → **warn**(부모가 살아 있어 뺀 수 · 미래 생성으로 뺀 수를 함께) |
| 9 | 후보마다 끝내기 확정(§3-3) | 검증 실패 → 그 후보만 건너뜀 · 재확인 거짓 → 나머지 버림(debug) |

**멤버 PID ↔ 표 ↔ 시작시각의 짝 [리뷰 2 K1 — 고름]:** 명단(2) · 표(4) · 시작시각은 서로 다른 순간에 뜬다. 멤버 X(PID p)가 표 **뒤**에 죽고 p 가 우리 Job 의 새 프로세스 Z 에게 넘어가면, 시작시각 한 번만 읽으면 (p, Z 의 시작) 이 **X 의 표 줄(X 의 부모)**과 짝지어진다 — X 의 부모가 죽었으면 Z 가 「부모 죽음」으로 읽혀, 부모가 살아 있는 Z(예: 도는 `cargo` 의 새 `rustc`)를 끝낸다. 그래서 시작시각을 **표 앞뒤로 두 번** 읽는다: `s1 == s2` 면 p 는 그 사이 내내 같은 프로세스였다(PID 는 프로세스 객체가 남아 있는 동안 재사용되지 않고, 같은 번호의 새 주인은 다른 시작시각을 갖는다) → 표 줄도 그 프로세스의 것이다. 어긋나면 그 멤버를 뺀다. 끝내기의 기대 시작시각도 이 값이라, 검증(§3-5)이 다시 한 번 같은 프로세스인지 본다.

**보호 집합 — 사용자 규칙의 「claude」·「claude 의 직계 자식」을 구조로 가린다 [고름]:**

- 뿌리 = 통로가 띄운 자식. Windows 에서 그것은 claude 가 아니라 `cmd.exe` 래퍼다(`console_command` — `backend/mod.rs:37-57` · `stdio.rs:414-415` · `:443` 「손자(cmd 아래 claude)」). claude 는 뿌리에서 **래퍼 깊이**만큼 내려간 층(「래퍼 층」)에 있다.
- ★**래퍼 층은 claude 하나가 아니다(스파이크 Q2 · 4/4)**★ — Windows 에서는 `cmd.exe` 의 콘솔 호스트 `conhost.exe` 가 같은 층에 있다. 그래서 「claude 층이 비면 정리하지 않는다」는 거짓 안심이다 — claude 가 죽어도 cmd.exe 가 사는 동안 conhost 가 층을 채운다.
- **래퍼 모양 = 두 사실의 합성 [고름 — 리뷰 1 F]:** 사실마다 주인이 다르고, 각 사실의 `#[cfg]` 는 그 주인의 **함수 하나** 안에서만 선다(CLAUDE.md 「플랫폼 중립」).
  - **래퍼 깊이** = `console_wrapper_depth()`(`console_command` 옆 · Windows 1 = `cmd.exe /c` · 그 밖 0).
  - **스폰이 뿌리 아래 붙이는 프로세스 수** = `ProcessGroup::root_attached()` — 통로가 정한다. Windows 의 stdio 통로는 `CREATE_NO_WINDOW`(`stdio.rs:101-108`)로 콘솔 앱을 띄워 숨은 콘솔의 호스트가 뿌리의 자식으로 붙으므로 1 이다(스파이크 Q2). 그 값은 `process_group()` 의 Windows 갈래 안에 둔다. PTY 스폰이었다면 호스트가 다른 자리에 붙을 것이다(추론) — 그래서 `console_command` 의 사실이 아니다.
  - **합성**은 claude `open_spawn` 에서 한다(두 선택이 만나는 곳 · `#[cfg]` 없음): `WrapperShape::compose(depth, root_attached)` → `layer_size = 1 + (depth == 1 ? root_attached : 0)` — 깊이 0 이면 붙은 프로세스는 프로그램의 자식(보호 층 안)이고, 깊이 1 이면 프로그램의 형제다.
- ★**모양 가드 — 래퍼 층의 식구 수가 `layer_size` 와 정확히 같을 때만 정리한다**★. 적으면(claude 가 죽어 conhost 만 · 뿌리 신원이 어긋나 층이 빔) 멈추고 · 많으면(예상 밖 모양) 멈춘다. 스폰 방식이 바뀌어 모양이 어긋나면 **무동작**으로 드러난다(오살 아님).
- 보호 집합 = 깊이 0 ‥ depth+1 의 신원 전부 = 뿌리 사슬 · 래퍼 층(claude **와 콘솔 호스트**) · 그 아래 한 층(claude 의 직계 자식 · 콘솔 호스트의 자식 — 후자는 관측된 적이 없다). 보호를 넓힐 뿐이다(덜 죽이는 쪽).
- **대가 — claude 의 PID 를 따로 집지 않는다.** 로그 · G2 는 래퍼 층 전체를 싣는다(§3-8 · §7).
- **거부한 가려내기:** ⓐ 이름으로 콘솔 호스트를 가린다(`conhost.exe`) — 실행파일 이름 읽기(OS 호출 하나 더)와 손으로 드는 이름 목록이 든다. ⓑ 생성 순서(콘솔 호스트가 먼저 — +5 ms 대 +60 ms) — 시각의 우연에 기댄다. ⓒ 가드 없이 래퍼 층 전체 보호 — 「claude 가 죽었다」를 못 가린다.

**나무 걷기 — 닫힌 실패, 단 「사라짐」은 건너뛴다 [리뷰 1 J · 리뷰 2 K5/A6 · A1]:**

- `base::platform::process_parent_table() -> Option<Vec<(u32 /*pid*/, u32 /*ppid*/)>>` — Toolhelp 스냅숏 **한 장**. ★`Process32NextW` 의 `ERROR_NO_MORE_FILES` 만 목록 끝이고, 그 밖의 오류는 `None`★(오늘 `child_pids` 는 어떤 오류든 끝으로 본다 — `base/src/platform.rs:189-191`. 목록이 잘리면 ppid 가 빠져 멤버가 「부모 죽음」으로 읽히고 보호 층이 줄어든다). 스냅숏 생성 · 첫 항목 실패도 `None`. `child_pids` 는 그 위로 옮긴다(`parent == 0` 조기 반환 유지 — 시험 `:266` · `None` 이면 오늘처럼 빈 목록).
- `base::platform::process_start(pid) -> ProcessStart { Known(u64), Gone, Unknown }` — `OpenProcess` 가 `ERROR_INVALID_PARAMETER`(87)로 실패하면 `Gone`(그 번호의 프로세스가 없다 — 같은 파일의 `alive_from_open_error` 가 이미 쓰는 판정 · `:62-88`), 그 밖의 여는 실패(`ERROR_ACCESS_DENIED` 등) · `GetProcessTimes` 실패는 `Unknown`. ★`pid == 0` 은 열지 않고 곧바로 `Unknown`★(리뷰 3 — System Idle Process 는 살아 있는데 `OpenProcess` 가 바로 그 87 로 실패하는 유일한 경우라 `Gone` 으로 오독된다). `process_creation_time` 은 `Known` 만 `Some` 으로 옮긴다(동작 동일 — `process_creation_time(0) == None` 시험 `:239` 그대로).
- 순수 `process_tree::walk_levels(root, max_depth, children: &dyn Fn(u32) -> Option<Vec<ProcessIdentity>>) -> Option<Vec<(ProcessIdentity, usize)>>` — `walk` 의 두 규칙을 그대로 쓰고, `children` 이 `None` 을 주면 **전체가 `None`** 이다.
- 실제 `children(pid)` = 표에서 ppid 가 pid 인 줄마다 `process_start`:
  - `Known` → 신원으로 쓴다.
  - `Gone` → **건너뛴다**(그 사이 끝난 프로세스 — 보호할 것도 끝낼 것도 없다 · 끝내기도 `Gone` 이다).
  - `Unknown` 이고 그 pid 가 **Job 멤버**(2 단계 명단) → **`None`**(보호해야 할지 모르는 우리 프로세스를 가릴 수 없다).
  - `Unknown` 이고 Job 멤버가 **아님** → 건너뛴다. 예: 우리 PID 와 같은 번호를 ppid 로 단 남의 프로세스(ppid 가 묵은 값). 멤버가 아니면 후보가 될 수 없고, Job 소속은 자식에게 이어지므로 그 아래에도 멤버가 없다.
- 뿌리 신원 확인(시작시각 대조)은 걷기 전에 한다 — 어긋나면 `None`.

**순수 함수:**

```rust
/// 래퍼가 만드는 모양 — 두 사실의 합성(순수 · cfg 없음).
pub(crate) struct WrapperShape { pub(crate) depth: usize, pub(crate) layer_size: usize }
impl WrapperShape { pub(crate) fn compose(depth: usize, root_attached: usize) -> Self; }
/// 걷기 결과에서 보호 집합 — 래퍼 층 식구 수가 `layer_size` 와 다르면 `None`.
fn keep_set(levels: &[(ProcessIdentity, usize)], shape: WrapperShape) -> Option<Vec<ProcessIdentity>>;
/// 명단 · 두 번 읽은 시작시각 · 표에서 멤버와 부모 상태를 만든다 — s1 ≠ s2 · 못 읽음 · ★표에 자기 줄이 없는 멤버★는 뺀다
/// (자기 ppid 를 모르면 부모를 판정할 수 없다 — 스냅숏 전에 끝났거나 그 뒤에 명단에 들었다).
fn members_with_parents(member_pids: &[u32], s1: &dyn Fn(u32) -> ProcessStart, table: &[(u32, u32)],
                        s2: &dyn Fn(u32) -> ProcessStart, parent_start: &dyn Fn(u32) -> ProcessStart) -> Vec<Member>;
struct Member { id: ProcessIdentity, parent: ParentState }
enum ParentState { Alive(ProcessIdentity), Dead, Unknown }
/// 후보 = 보호 집합 밖(신원 비교) · threshold ≤ 생성 ≤ now_wall · 부모 `Dead`(결정 4).
fn select_leftovers(members: &[Member], keep: &[ProcessIdentity], threshold: u64, now_wall: u64) -> Vec<Member>;
```

- **부모 상태(결정 4) — 걷기와 같은 표 한 장에서 [PID 재사용 규칙]:** 멤버의 ppid 를 판의 표(4 단계)에서 읽는다. 표가 실패하면 판 전체가 멈추므로 부모 판정이 따로 실패할 길이 없다.
  - ★멤버 **자신의** 줄이 표에 없으면 그 멤버를 뺀다(리뷰 3)★ — ppid 를 모르면 부모를 판정할 수 없다.
  - `Alive(p)` = ppid 가 표에 있고 · `process_start` 가 `Known` 이고 · **시작시각 ≤ 멤버 시작시각**(`walk` 의 규칙과 같다 · 같은 눈금 허용).
  - `Dead` = ppid 가 표에 없다 · `Gone` 이다 · 또는 **멤버보다 늦게 태어난 프로세스가 그 번호를 쥐고 있다**(PID 재사용 — 진짜 부모는 멤버를 낳을 때 살아 있었으므로 그 번호의 새 주인은 멤버보다 늦게 태어날 수밖에 없다).
  - `Unknown` = 표에 있는데 `process_start` 가 `Unknown`(권한 밖 프로세스가 번호를 쥠 등) → **후보에서 뺀다**(판정할 수 없으면 덜 죽이는 쪽).
  - ★표가 **끝났지만 아직 참조되는(좀비) 프로세스**를 싣는다면 죽은 부모가 `Alive` 로 읽혀 잔여물을 놓친다★(놓치는 쪽). Toolhelp 가 좀비를 싣지 않는다는 가정은 스파이크의 간접 증거(잔여물의 부모가 「죽음」으로 보였다 · 3/3)뿐이라 실프로세스 시험으로 박는다(§5 실프로세스 ⑤ · 리뷰 2 K4).
- `select_leftovers` 의 조건은 넷의 **AND** — 보호 밖 · `created >= threshold` · ★`created <= now_wall`(판의 벽시계 — 지금보다 미래에 태어난 것으로 읽히면 그 뒤 시계가 뒤로 뛰었다는 뜻이라 뺀다 · §3-6)★ · `parent == Dead`. 결정 4 는 마지막 조건 하나로만 들어가고 후보를 줄이기만 한다. 주 기준은 여전히 앞의 것들(+ 끝내기 직전의 생존 확인)이다 — 조사 §7 「고아 여부는 주 기준이 아니다」와 어긋나지 않는다.
- 잰 잔여물 3/3 은 부모(깊이 4 훅 bash)가 죽어 있었다(§3-0 Q1) → 결정 4 뒤에도 그대로 고른다(스파이크 Q4 의 후보 집합과 같다).
- 비교 키는 **신원(PID + 시작시각) 쌍**이다(`ProcessIdentity` — `process_tree.rs:13-23`, ADR-0218). 보호 집합의 PID 와 같지만 시작시각이 다른 멤버는 다른 프로세스라 보호받지 않는다.
- 문턱 비교는 `>=` 다 — 같은 눈금은 넣는다(해상도 논의 = §3-6).
- **보호 층 밖이지만 결정 4 가 지키는 것(§8 ⑪):** taskkill.exe 는 claude 의 직계 자식(d2)이라 보호되지만 그 콘솔 호스트(d3)는 보호 층 밖이다. taskkill 이 N 초 넘게 매달려 있다면(upstream #67888) 그 conhost 의 부모(taskkill)가 살아 있으므로 결정 4 로 후보에서 빠진다. 스파이크에서 taskkill 최장 278 ms.

### 3-5. OS 조각 — Job 멤버 명단 + 검증 + 끝내기

**`platform/windows.rs` 의 `JobObjectHandle`(`:19-82`)에 명단 · 검증과 검증된 핸들의 끝내기.** 이 파일은 `windows` crate 와 `io` 만 쓴다 — 결과 enum 도 여기 두고(리뷰 1 E) 중립 `process_group.rs` 가 감싼다. 그래서 이 파일을 나중에 별도 플랫폼 모듈로 통째 옮길 수 있다(사용자 결정 「OS 조각」). 「하나 끝내기」가 검증과 끝내기 두 호출로 갈린 것은 끝내기 확정(§3-3)을 자물쇠 안의 `TerminateProcess` 하나로 줄이기 위해서다.

```rust
pub enum MemberCheck { Ready(VerifiedMember), Gone, NotOurs }
pub enum MemberOutcome { Terminated, Gone }
/// 검증을 마친 프로세스 핸들 — 쥐고 있는 동안 그 PID 는 재사용되지 않는다. drop = CloseHandle.
pub struct VerifiedMember { /* HANDLE */ }

/// 지금 이 Job 에 든 프로세스 PID 전부(중첩 Job 의 멤버 포함). ★완전할 때만 Ok★.
pub fn member_pids(&self) -> io::Result<Vec<u32>>;
/// 같은 규칙 · 첫 용량만 주입(시험 이음새 — 늘리기 · 되풀이 경로를 적은 프로세스로 잰다).
pub(crate) fn member_pids_with_capacity(&self, initial: usize) -> io::Result<Vec<u32>>;
/// 그 신원이 아직 **이 Job 의 산 멤버**인지 같은 핸들로 본다. 끝내지 않는다.
pub fn verify_member(&self, pid: u32, expected_start: u64) -> io::Result<MemberCheck>;
impl VerifiedMember {
    /// 검증한 그 핸들로 TerminateProcess **호출 하나만**(비동기 — 대상의 종료를 기다리지 않는다). 날것 결과만 돌려준다 —
    /// 끝내기 확정의 자물쇠 안에서 부르는 유일한 OS 호출이다(§3-3).
    pub fn terminate_raw(&self, exit_code: u32) -> io::Result<()>;
    /// 자물쇠를 놓은 뒤 부른다 — 날것 결과를 가르고(실패면 WaitForSingleObject(h, 0) 로 Gone 인지) 핸들을 닫는다(self 소비).
    pub fn settle(self, raw: io::Result<()>) -> io::Result<MemberOutcome>;
}
```

- **명단 [리뷰 1 L · 리뷰 2 A2]:** `QueryInformationJobObject(JobObjectBasicProcessIdList)` 를 **상한 있는 되풀이**(최대 4 회)로 묻는다. 버퍼는 구조체 정렬로 잡고 첫 용량 64(운영) · 모자라면 `NumberOfAssignedProcesses` 의 두 배 + 16 으로 늘린다. 머리 · 항목은 구조체 필드(`NumberOfAssignedProcesses` · `NumberOfProcessIdsInList` · `ProcessIdList` 의 필드 오프셋 — 항목은 `ULONG_PTR`)로 읽는다. ★`NumberOfProcessIdsInList == NumberOfAssignedProcesses` 일 때만 완전하다★ — 아니면(또는 `ERROR_MORE_DATA`) 다시 묻고, 되풀이가 다 차면 `Err`(닫힌 실패). 항목을 `u32` 로 못 바꾸면 `Err`. 첫 용량은 이음새(`member_pids_with_capacity`)로 시험에서 1 로 줄여 늘리기 · 되풀이를 프로세스 서넛으로 잰다.
- **검증 [리뷰 1 L]:** `OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE)` → `IsProcessInJob(h, self.handle)`(★Job 인자는 늘 우리 핸들 — NULL 이면 「아무 Job 에나 있나」가 된다★) → `GetProcessTimes` 생성 시각 == `expected_start` → `WaitForSingleObject(h, 0)`.
  - `OpenProcess` 가 `ERROR_INVALID_PARAMETER` 면 `Gone` · 그 밖의 여는 실패는 `Err` · `IsProcessInJob` 거짓 · 생성 시각 불일치 = `NotOurs` · 그 호출 자체의 실패 = `Err` · 대기 결과 `WAIT_OBJECT_0` = `Gone` · `WAIT_TIMEOUT` = `Ready(핸들)` · 그 밖 = `Err`. ★`Ready` 가 아니면 끝내기로 가지 않는다★.
  - 생존은 `GetExitCodeProcess == STILL_ACTIVE` 로 재지 않는다 — 종료 코드 259 로 끝난 프로세스가 산 것으로 보인다.
- **끝내기:** `terminate_raw` = 그 핸들로 `TerminateProcess(h, LEFTOVER_EXIT_CODE)` 한 번 · 날것 결과만(자물쇠 안). `settle`(자물쇠 밖) = 성공이면 `Terminated` · 실패면 — 검증 뒤 그 사이 스스로 끝났을 수 있다 — `WaitForSingleObject(h, 0) == WAIT_OBJECT_0` 이면 `Gone`, 아니면 `Err` · 그리고 핸들을 닫는다. 재확인이 거짓인 갈래도 핸들 닫기는 자물쇠를 놓은 뒤다. `LEFTOVER_EXIT_CODE` = `0x7440` [고름] — 사후 조사에서 「T-40 정리가 끝냈다」를 종료 코드만으로 알아보게 한다. `shutdown()` 의 Job 끝내기(`:447` — 1)와 다르다.
- **중립 손잡이(`platform/process_group.rs`):**

```rust
pub(crate) enum Verify { Ready(Box<dyn ReadyKill>), Gone, NotOurs }   // Windows 에선 MemberCheck 를 옮겨 담는다
pub(crate) trait ReadyKill: Send {
    fn terminate_raw(&self) -> io::Result<()>;                                  // 자물쇠 안 — OS 호출 하나
    fn settle(self: Box<Self>, raw: io::Result<()>) -> io::Result<MemberKill>;   // 자물쇠 밖 — 분류 + 닫기
}
pub(crate) enum MemberKill { Terminated, Gone }
pub(crate) struct ProcessGroup { #[cfg(windows)] job: Weak<JobObjectHandle>, root_attached: usize }
impl ProcessGroup {
    /// 완전한 멤버 PID 명단. 통로가 사라졌으면 Ok(빈 목록).
    pub(crate) fn member_pids(&self) -> io::Result<Vec<u32>>;
    pub(crate) fn verify(&self, who: ProcessIdentity) -> io::Result<Verify>;
    /// 이 스폰이 뿌리 아래 붙이는 프로세스 수(§3-4 래퍼 모양).
    pub(crate) fn root_attached(&self) -> usize;
}
```

- `#[cfg]` 갈래는 이 모듈과 통로 접근자 안에서만 선다. 비Windows 판은 빈 명단 · `Gone` 만 돌려주는 껍데기이고, `StdioTransport::process_group()` 은 비Windows 에서 `None` 이다 — 호출자(`backend/claude`)에는 `cfg!(windows)` 가 없다. 비Windows 에서는 정리기가 조립되지 않는다(오늘과 같다).
- **통로:** `job_handle`(`stdio.rs:72-73`)을 `Arc<JobObjectHandle>` 로 바꾸고 **`pub(crate) fn process_group(&self) -> Option<ProcessGroup>`**(`StdioTransport` 는 `pub` 이지만 이 접근자는 crate 밖에 내놓지 않는다)이 `Arc::downgrade` 와 `root_attached` 를 담아 준다. 통로는 무엇에 쓰이는지 모른다(ADR-0044 「바보 파이프」 · CLAUDE.md 「백엔드 확장」). `shutdown()` 의 Job 끝내기(`:443-448`)와 drop 의 `KILL_ON_JOB_CLOSE` 는 그대로다.
- **왜 약한 손잡이인가:** 일꾼이 강한 `Arc` 를 쥐면 통로가 사라진 뒤에도 Job 핸들이 안 닫혀 `KILL_ON_JOB_CLOSE` 가 늦어진다. `Weak` 는 판 하나 동안만 올린다.
- **Windows API 확인(`windows-0.58.0` 소스):** `QueryInformationJobObject` · `IsProcessInJob` · `JobObjectBasicProcessIdList` · `JOBOBJECT_BASIC_PROCESS_ID_LIST`(항목 = `[usize; 1]` = `ULONG_PTR`) = `Win32_System_JobObjects` · `OpenProcess` · `GetProcessTimes` · `WaitForSingleObject` · `PROCESS_SYNCHRONIZE` · `TerminateProcess` = `Win32_System_Threading` · `WAIT_OBJECT_0` · `WAIT_TIMEOUT` = `Win32_Foundation`. 셋 다 이미 켜져 있다(`crates/engram-dashboard-agent/Cargo.toml:49-56`).

### 3-6. 시계

- **벽시계 = std `SystemTime::now()` 를 FILETIME 척도(1601 기점 · 100 ns)로 바꾼 값 [고름].** 커널 생성 시각(`GetProcessTimes`)과 같은 시스템 시각이다(사용자 결정). `GetSystemTimeAsFileTime` 을 직접 부르면 `Win32_System_SystemInformation` feature 가 새로 든다. ★1970 ↔ 1601 기점 변환이 틀리면 모든 멤버가 문턱 뒤로 읽혀 **떼로 죽인다** — 실제 시계 음성 시험이 막는다(§5 실프로세스 ②)★.
- **해상도 — 놓치기만 한다:** 커널 생성 시각이 벽시계보다 거칠다면(한 틱 안으로 내림) 문턱 **직후 한 틱 안에** 생긴 잔여물이 빠진다(오늘처럼 멈춤). 해상도로는 문턱 전 프로세스가 문턱 뒤로 읽히지 않는다. 간접 실측(스파이크): 뿌리 생성 − 스폰 직전 `SystemTime` = +0.67…+0.79 ms(4/4 · 음수 없음) · 끊기 뒤 첫 프로세스 = +1.1…1.4 ms → 1 ms 아래로 보인다. 잔여물은 문턱보다 **≥ +203 ms** 뒤였다(3/3).
- **벽시계가 뒤로 뛰는 것 — 잘못 고를 수 있다 [리뷰 1 I]:** 비교는 멤버의 생성 시각(태어날 때 찍힘)과 `first_wall`(Esc 때 찍힘)이므로, 위험한 틈은 **멤버가 태어난 뒤 ~ Esc 기록 전** 사이의 뒤로 뛰기다. X 만큼 뒤로 뛰었으면 Esc 전 X 안에 태어난 프로세스가 `created >= threshold` 를 지난다. 같은 뛰기는 `walk` 의 「부모보다 먼저 태어난 것은 자식 아님」(`process_tree.rs:114`)과 결정 4 의 부모 판정도 속인다. 반대로 **기록 뒤**의 뒤로 뛰기는 안전하다 — 기록 뒤에 태어난 것이 더 이르게 읽혀 빠질 뿐이다(놓치는 쪽). 앞으로 뛰기도 안전하다.
- **시계 닻 — 기록 · 판마다 다시 잡고, 뒤로 뛰면 그만큼 쉰다 [리뷰 2 K3/A3 — 고름]:** 3판은 닻을 조립 때 한 번만 잡아, 정당한 1 초 넘는 NTP 보정 한 번이나 긴 세션의 슬루 누적이 그 화신의 정리를 영영 끄게 했다.
  - `ClockAnchor { wall, mono }` 는 `GateState` 안에 있다(정리기 조립 때 처음 — 뿌리 스폰 직후 · claude.exe 보다 앞).
  - `observe_clock(now_wall, now_mono)`(자물쇠 안 · 순수 — 두 값도 자물쇠 안에서 읽는다 · §3-2) = `drift = anchor.wall + (now_mono − anchor.mono) − now_wall`(양수 = 벽시계가 뒤처짐 = 뒤로 뜀). ★표본의 단조 값이 닻보다 이르면(있어서는 안 되는 역순 표본) 닻을 건드리지 않는다 — 닻은 앞으로만 간다★.
    - `drift ≤ CLOCK_STEP_TOLERANCE`(1 초 [고름]) → **닻을 지금으로 다시 잡는다**(허용치 안의 슬루 · 오차가 쌓이지 않는다).
    - `drift > 허용치` → `clock_hold_until = max(기존, now_mono + drift)` 로 세우고 닻을 지금으로 다시 잡는다. 뒤로 X 뛰었으면 **단조 시간으로 X 가 지난 뒤**부터는 뛰기 전에 태어난 프로세스의 생성 시각이 모두 벽시계보다 앞선다 — 그때부터 연 에피소드는 안전하다.
    - 앞으로 뛰었으면(음수) 다시 잡는다.
  - **어디서 보나:** ⓐ 끊기 줄 함수의 `try_interrupt`(기록과 한 구간) — 새 에피소드의 `clock_ok = (clock_hold_until 이 없거나 이미 지남)` ⓑ 판 시작 — 쉼 끝 · 닻 갱신만 하고 판을 막지는 않는다(기록 뒤의 뛰기는 안전하므로). 이 구간에서 읽은 벽시계를 고르기의 `now_wall` 로 쓴다(그 뒤 태어난 것은 미래 생성으로 빠진다 — 덜 고르는 쪽). 판의 `clock_ok` 가 거짓이면 판은 아무것도 안 끝낸다(§3-4 1 단계 · warn).
  - **멤버마다 한 겹 더:** 고르기의 `created <= now_wall`(§3-4) — 지금 벽시계보다 미래에 태어난 것으로 읽히는 멤버는 그 뒤 시계가 뒤로 뛰었다는 증거라 뺀다. 닻이 허용치 안이라 못 본 뛰기의 일부도 여기서 걸린다.
  - **재기:** 모든 판의 로그(끝냄 · 못 끝냄 둘 다)가 그 판에서 잰 `clock_drift_ms` 를 싣고, 허용치를 넘는 뛰기는 본 자리에서 warn 한다 — 빈도가 로그로 쌓인다.
  - **남는 노출:** 허용치(1 초) 안의 뒤로 뛰기는 못 본다 — Esc 전 그만큼 안에 태어난 프로세스가 문턱을 지날 수 있고, 부모와 자식이 태어난 사이에 그런 뛰기가 끼면 자식의 생성 시각이 부모보다 이르게 읽혀 **살아 있는 부모가 「부모 죽음」(재사용)으로** 뒤집힐 수 있다(§8 ⑤ · 드묾).
- **대기(N 초)는 단조 시각(`Instant`)으로 잰다** — 벽시계가 뛰어도 「N 초 지났나」가 흔들리지 않는다.

### 3-7. PID 재사용 · 조사 §4 사고 규칙 대조

| 틈 | 막는 것 |
|---|---|
| 후보가 우리 것인가 | 후보는 **Job 명단**에서만 나온다 — 커널이 추적하는 소속이다. PID 부모 연결로 후보를 넓히지 않는다(조사 §4 규칙). 명단이 불완전하면 판을 멈춘다 |
| 명단 · 표 · 시작시각 사이에 멤버 번호가 재사용됨 | 시작시각을 표 앞뒤로 두 번 읽어 같을 때만 쓴다(§3-4 · 리뷰 2 K1) |
| 검증 뒤 그 PID 가 죽고 남에게 재사용됨 | 검증과 끝내기가 **같은 핸들**이다 — 열린 핸들은 그 프로세스를 붙들어 번호가 재사용되지 않는다 |
| 재확인과 끝내기 사이에 턴 끝 · 새 끊기 | 재확인과 `TerminateProcess` 가 한 자물쇠 구간이다(§3-3 · 리뷰 2 C1) |
| 보호 집합이 PID 재사용에 속음 | 걷기는 `walk` 규칙(부모보다 먼저 태어난 「자식」은 버린다)을 쓴다. claude 는 살아 있어 그 PID 가 남에게 넘어가 있을 수 없다 |
| 보호 집합이 **실패**에 줄어듦 | 표 목록 중간 오류 · 보호 깊이의 **Job 멤버** 시작시각 못 읽음 → 판 전체 `None`(§3-4). 「사라짐」과 멤버 아닌 남의 「못 읽음」만 건너뛴다 |
| 보호 집합 · 부모 판정이 **시계 뛰기**에 속음 | 시계 닻 + 쉼 · 멤버의 `created <= now_wall`(§3-6) |
| 부모가 좀비로 표에 남음 | 놓치는 쪽뿐(죽은 부모가 `Alive`) · 실프로세스 시험이 박는다(§5 ⑤) |

### 3-8. 로그

`docs/reference/logging-conventions.md` 「형식」 — 메시지는 한국어 한 줄, 식별자·수치는 필드. 레벨 기준 = 같은 문서 `:23`(warn = 비정상이나 안전 폴백).

**에이전트 귀속 [리뷰 1 C]:** `open_spawn` 은 이미 `control: Option<&ControlEndpoint>` 를 받고(`backend/mod.rs:466-478` · 넘기는 자리 `manager.rs:1508`) 그 끝점이 `agent_id` 를 싣는다(`types.rs:649`). 그 칸의 doc 은 「backend 조립 인자로 따로 받는 형태로 되돌리지 말 것」이라 적는다(`types.rs:644-647` · ADR-0217 결정 5). 그래서 정리기는 `control.map(|c| c.agent_id)` 를 쥐고 로그 필드 `agent` 로 싣는다. ★claude `open_spawn` 은 지금 `control` 을 일부러 읽지 않는다고 적고 버린다(`claude/mod.rs:426-431` — 「여기서 또 읽으면 한 spawn 이 같은 값을 두 수단으로 보낸다」) — ③ 단계가 그 주석과 버림 줄을 고친다(리뷰 2 A5): 이제 `agent_id` 하나를 **로그 귀속으로만** 읽고 통로로 다시 보내지 않으므로 그 주석이 막는 「두 수단」이 아니다★.
- `None` 인 때 = 제어 채널이 없는 스폰(`manager.rs:1369-1395` — Noop 제어 평면 · 시험 하네스). 그때는 `agent` 필드를 빼고 `root_pid` 가 상관 키다.

| 사건 | 레벨 | 필드 |
|---|---|---|
| 정리함(하나 이상 끝냄) | **warn** | `agent` · `root_pid` · `wrapper_layer`(래퍼 층 PID 목록) · `waited_ms`(첫 끊기부터) · `terminated`(PID · 문턱 뒤 ms · 죽은 부모의 PID) · `skipped`(`Gone`/`NotOurs`/오류 수) · `parent_alive`(결정 4 로 뺀 수) · `clock_drift_ms` |
| **에피소드가 있는데 아무것도 못 끝냄** — 후보 0(`parent_alive` · 부모 `Unknown` · 미래 생성 수를 함께) · 모양 어긋남(`wrapper_layer` · `expected`) · 명단 조회 실패/불완전 · 표 스냅숏 실패 · 보호 층 멤버 시작시각 못 읽음 · 첫 끊기 무렵 시계 뛰기(`clock_ok` 거짓) · 후보가 전부 `Gone`/`NotOurs`/오류 | **warn** — 사유를 필드 `reason` 으로 | `agent` · `root_pid` · `waited_ms` · `clock_drift_ms` · 사유별 값 |
| 시계가 허용치 넘게 뒤로 뜀(기록 · 판에서 봄) | warn | `agent` · `stepped_ms` · `hold_ms` |
| 끝내기 하나의 오류(`io::Error`) | warn | `agent` · `pid` · `: {e}` |
| 정리 뒤 N 초에도 같은 에피소드 | warn — ★하나 이상 끝낸 판 뒤에만★(못 끝낸 판은 위 줄이 이미 사유를 남겼다) | `agent` · `waited_ms` — 동작 없음(결정 2) |
| 빈 명단(에이전트가 이미 죽음) · 손잡이 사라짐 · 턴 끝이 먼저 옴 · 판 도중 새 끊기/턴 끝으로 멈춤 · 시작시각이 두 번 읽기에서 어긋나 뺀 멤버 | debug | `agent` · 사유 |
| 일꾼 기동 실패(에피소드 `Done`) | warn | `agent` · `: {e}` |

- 로그는 전부 자물쇠를 놓은 뒤 쓴다(§3-3 자물쇠 규칙).
- **`wrapper_layer` 를 싣는 이유:** 기존 `session_tracker` 의 warn 「PID shim 감지」(`backend/claude/session_file.rs:171-180`)가 claude 자신이 적은 PID(`resolved_pid`)를 남긴다. 그 PID 가 `wrapper_layer` 안에 있으면 래퍼 모양 가정이 맞았다는 것이 로그로 확인된다(§7 G2).

### 3-9. 영향이 없는 것

- **codex:** codex 통로(`TransportOwned`)는 자기 끊기(`turn/interrupt` + `turn_id`)를 쓰고 이 끊기 줄 함수를 지나지 않는다 — 정리기가 조립되지 않는다. 조사가 잰 결함은 claude CLI 의 훅 취소 경로다(조사 §2).
- **터미널 모드(claude · codex PTY):** 통로가 끊기 명령을 받지 않는다(ADR-0245 · `PtyTransport` 는 `Unsupported`) → 기록이 없다. 사람이 TUI 에 직접 친 Esc 는 우리를 지나지 않는다. 조사 §2 ⑧(n=1 · 잠정)도 TUI 에서는 300 초 턴 멈춤이 보이지 않았다.
- **프론트 · 선 타입 · 버스:** 새 사건 · 새 명령 · 새 i18n 키 없음(결정 1). 「중단하는 중…」은 평소처럼 턴 끝 사건에서 풀린다 — 멈춤 경우에는 약 N 초 + 수십 ms 뒤다(끝내기 → `result` 18–21 ms · §3-0). 정리 뒤의 `result` 는 보통 끊김과 같은 `aborted_streaming` 이고 앞에 합성 끊김 줄이 와서(§3-0 · 3/3) 번역 · 화면이 보통 끊김과 같다(ADR-0243 · `claude/mod.rs:873`).
- **CLAUDE.md 「LLM-우선 제어」:** 자동 동작이라 사람 조작이 없고, 새 제어 표면도 없다. LLM 의 `agent.interrupt` 는 사람의 Esc 와 같은 정리를 받는다(§3-2).
- **턴 관측(ADR-0127):** 두 지점(`finish` · `emit` 재확인)은 그대로다. 에피소드 지우기는 턴 관측이 아니라 backend decoder 안의 문 닫기 자리에 얹힌 것이다 — 셋째 호출자가 아니다.

---

## 4. 불변식 대조

| 원칙 · 불변식 | 이 설계에서 |
|---|---|
| **코어 격리**(ADR-0003) | 전부 `agent` · `base` crate · tauri import 0 · `OutputSink`/`StatusSink` 무변경 |
| **백엔드 확장**(ADR-0004) | claude 지식(끊기 줄 · 턴 끝 · 「무엇을 언제 정리하나」)은 `backend/claude` 에만 있다. 통로는 자기 프로세스 무리의 약한 손잡이와 자기 스폰이 붙이는 프로세스 수만 내준다. manager · `open_spawn` 시그니처는 바뀌지 않는다(에이전트 귀속은 기존 제어 끝점에서 — §3-8) |
| **플랫폼 중립**(ADR-0230) | OS 갈래 = `platform/windows.rs`(Job 조각) · `platform/process_group.rs`(한 모듈 안의 `#[cfg]`) · `StdioTransport::process_group()`(이미 `#[cfg(windows)]` 인 Job 칸의 주인 — 콘솔 호스트 수도 여기) · `console_wrapper_depth()`(`console_command` 옆) · `base::platform` 의 새 함수 둘(기존 `#[cfg]` 쌍 모양). 부르는 쪽에 `cfg!(windows)` 없음. 시험 파일을 통째로 막지 않는다(§5) |
| **락 순서**(ADR-0006 · ADR-0231) | ★새 락 = `TurnGate.state` 하나 — **잎**이다★(규칙 정본 = §3-3 끝). 쥔 채 로그 · 스레드 기동 · emit · 잠 · 다른 자물쇠 · 그 밖의 OS 호출을 하지 않는다 — **예외 둘 = 시계 두 값 읽기(막히지 않음) · 끝내기 확정의 `TerminateProcess` 호출 정확히 하나**(비동기 · 드문 판에서만 · 핸들 닫기와 분류는 놓은 뒤 · 다른 자물쇠 없음). 쥐는 자리 셋: 끊기 줄 함수(부르는 쪽이 아무 락도 안 쥔다 — `session.rs:636-638` · `manager.rs:2875-2877`) · decoder(펌프가 decode 를 emit 전에 락 없이 부른다 — `stdio.rs:309-313`) · 일꾼. 기존 락(`input_order` · replay · status · subscribers · 통로의 child/stdin)과 간선이 없다. 독을 견딘다(§3-2) |
| **소유권 분할** | Job 은 여전히 transport 가 만들고 끝낸다 — 칸이 `Arc` 가 되고 밖으로는 `Weak` 만 나간다. ★새 사실: backend 의 일꾼 스레드가 그 약한 손잡이로 **멤버 하나씩** 검증하고 `TerminateProcess` 할 수 있다★(Job 전체가 아니라 · claude 는 보호) → CLAUDE.md 「핵심 불변식」 소유권 줄 갱신(§6 ④). kill 인과(ADR-0001 — `shutdown` → Job 끝내기 → 펌프 EOF)는 무변경 |
| **ADR-0238 턴 열림 문** | 여닫기 규칙(진행에 열고 끝에 닫는다) · 거절(`None` → `Unsupported`) · 잔여 경합 서술 · `InterruptLine` 계약(결정 2)은 그대로다. 모양이 원자값에서 자물쇠 안 상태로 바뀌고(결정 3 개정), 「우리가 끊기를 보냈다」 기록이 생긴다(결정 5 개정 — 결과 분류가 아니라 정리 용도 · §9) |
| **바닥 crate 입주 조건**(ADR-0175) | 새 입주자 없음 — 기존 입주자 `platform` 에 함수 둘. 자리 근거 = §11 |

---

## 5. 시험 (ADR-0012)

**순수 — 모든 OS에서 돈다(`leftover.rs` · `process_tree.rs` · `base` 의 `#[cfg(test)]`):**

| 대상 | 경우 |
|---|---|
| **에피소드 상태기계** | 닫힌 문 → 거절 · 기록 없음 · 첫 끊기 → `Waiting` + 일꾼 몫 · 둘째(`Waiting`) → `last_mono` 만 · 일꾼 몫 없음 · 문턱 유지 · 끝(`close_turn`) → 에피소드 없음 · ★불변식 시험: 임의의 전이열 뒤 「`Waiting`/`Cleaning` ⇒ 문 열림」 · 「`Waiting`/`Cleaning`/`Spent` ⇒ `worker`」★ |
| **리뷰 1 A 엇갈림** | `close_turn` 뒤의 `try_interrupt` 는 거절 · 기록 없음 · 두 스레드가 `close_turn`/`try_interrupt` 를 번갈아 두드려도 위 불변식이 선다(스트레스 · 짧게) |
| **리뷰 2 C1 확정 틈** | 가짜 포트의 `verify` 안에서(자물쇠 밖) `close_turn` 을 부르면 → 그 후보의 `terminate_raw` 가 **불리지 않는다** · 같은 자리에서 `try_interrupt` 를 부르면 → 불리지 않고 새 끊기 + N 까지 아무것도 안 끝난다 · ★확정 구간 안에서 불리는 포트 호출은 `terminate_raw` 하나뿐 — 가짜 포트가 `terminate_raw` 중에는 자물쇠가 잡혀 있고 `settle`(닫기 · 분류) · `verify` · 그 밖의 호출 중에는 잡혀 있지 않음을 확인 · 재확인 거짓 갈래의 핸들 버리기도 자물쇠 밖(리뷰 3 아키텍트)★ |
| **리뷰 3 codex 역순 표본** | ⓐ 순수: 전이에 표본을 역순으로 먹인다 — t2 를 먼저 기록한 뒤 t1(< t2) → `last_mono` 는 t2 에 머문다 · 닻의 단조 값은 되감기지 않는다 · 닻보다 이른 표본으로 연 새 에피소드의 `first_wall`/`first_mono` 는 닻 값으로 올라간다 ⓑ 장벽 시험: 두 스레드가 장벽에서 함께 출발해 `try_interrupt` 경로를 두드린다 — 가짜 시계가 읽기마다 전역 순번을 찍는다 → 기록(자물쇠 구간)의 순서와 그 안에서 읽힌 표본의 순번이 같은 순서다 · 기록마다 `last_mono` · 닻의 단조 값이 줄지 않는다 · 일꾼의 첫 확정은 가장 늦은 표본 + N 전에 오지 않는다 |
| **리뷰 2 C2 기동 실패** | 일꾼 기동 실패 전에 다른 끊기가 에피소드를 밀었거나 새로 열었어도 → 기동 실패 뒤 `worker == false` · 에피소드 `Done` · 다음 끊기는 새 에피소드 + 기동 시도 |
| **리뷰 1 B ① 판 도중 끊기** | `Cleaning` 중 끊기 → `Waiting` · 다음 확정 거짓 → 나머지 버림 · 새 끊기 + N 전엔 아무것도 안 끝냄 · 다음 판의 문턱 = 같은 첫 끊기 · 일꾼 기동 1 회 |
| **리뷰 1 B ① 정리 뒤 끊기** | `Spent` 중 끊기 → 새 세대 · 새 문턱 · 일꾼 기동 없음 · 옛 세대의 N 뒤 warn 안 나감 |
| **리뷰 1 B ② 묵은 기록** | 턴 끝을 놓친 `Waiting` → 판 **한 번**만 돌고 `Spent`/`Done` · 그 뒤 새 끊기 없이는 판 없음 · 새 끊기는 새 문턱 |
| **일꾼 수명** | 에피소드 없음 · `Done` → 일꾼 끝 · `worker` 내림 · 가짜 시계로 「마지막 끊기 + 2N + 판」 안에 끝남 |
| **독** | 자물쇠를 쥔 스레드를 패닉시킨 뒤에도 `try_interrupt` · `close_turn` · `next_step` 이 돈다 |
| **판 순서(가짜 포트)** | 빈 명단 → debug 로 끝나고 모양 warn 없음 · 명단 `Err` → warn · `clock_ok` 거짓 → warn · 표 `None` → warn · 모양 어긋남 → warn · 후보 0 → warn · 하나 이상 끝냄 → warn + N 뒤 확인 · 못 끝냄 → N 뒤 확인 없음 · ★스파이크 Q3 모양의 표에서 후보 = 잔여물 하나★ |
| **리뷰 2 K1 짝** | 멤버 p 의 `s1 ≠ s2`(표 앞뒤 사이에 번호가 새 프로세스로 넘어감) → 그 멤버 빠짐 · `s2` 가 `Gone`/`Unknown` → 빠짐 · 표 줄 p 가 옛 신원(부모 죽음)이고 멤버는 새 신원(부모 살아 있음)인 모양 → 끝내지 않는다 · 끝내기의 기대 시작시각 = `s2` |
| `select_leftovers` | 문턱 앞 = 빠짐 · 문턱과 같음 = 듦 · ★생성 > 판의 벽시계 = 빠짐★ · 보호 신원 = 빠짐 · 보호 집합과 PID 는 같고 시작시각이 다른 멤버 = 듦 · 부모 `Alive` = 빠짐 · `Dead` = 듦 · `Unknown` = 빠짐 · 백그라운드 빌드 모양(살아 있는 `cargo` 아래 문턱 뒤 `rustc`) = 빠짐 · 스파이크 잔여물 모양 = 듦 |
| `members_with_parents` | ★멤버 자신의 줄이 표에 없음 = 그 멤버 빠짐★ · 부모가 표에 없음 = `Dead` · 부모 `Gone` = `Dead` · 표의 ppid 번호를 멤버보다 늦게 태어난 프로세스가 쥠 = `Dead` · 부모 시작시각 ≤ 멤버 = `Alive` · 같은 눈금 = `Alive` · 부모 `Unknown` = `Unknown` |
| `keep_set` · `compose` | `compose(1, 1) = {1, 2}` — {conhost, claude} = `Some` · {conhost} 하나 = `None` · 셋 = `None` · 빈 층 = `None` · `compose(0, 0) = {0, 1}` — {뿌리} = `Some` · `compose(0, 1) = {0, 1}` |
| `walk_levels` · `children` | `children` 이 `None` → 전체 `None` · ★보호 깊이의 자식이 `Gone` → 건너뛰고 계속★ · ★`Unknown` 이고 Job 멤버 아님 → 건너뜀 · `Unknown` 이고 Job 멤버 → `None`(리뷰 2 K5)★ · ppid 가 claude 인데 claude 보다 먼저 태어난 항목 = 자식 아님 · 깊이 상한 · 순환처럼 보이는 표에서 끝남 |
| 시계 닻 | 허용치 안 드리프트 → 다시 잡음(기록마다 조금씩 쌓여도 넘지 않는다) · 뒤로 1 초 초과 → 쉼 끝 = 지금 + 뛴 양 · 쉼 중에 연 에피소드 `clock_ok` 거짓 · 쉼 뒤 에피소드 참 · 판 시작의 뛰기 → 판은 막지 않고 쉼만 · 앞으로 뛰기 → 다시 잡음 · FILETIME 변환(1970 ↔ 1601 기점 상수) 골든 |
| `base::process_parent_table` 의 끝 판정 | 목록 순회 오류 분류를 순수 함수로 뽑아 — `ERROR_NO_MORE_FILES` = 끝 · 그 밖 = 실패(`None`) · `child_pids(0)` 은 빈 목록(기존 시험 `:266` 유지) |
| decoder 배선(`claude/mod.rs` 시험 — `gated_decoder` `:5623-5628` 모양) | 진행 줄 → 끊기 함수가 줄을 주고 에피소드가 선다 · `result` 줄 → 지워진다 · 끊긴 `result`(`TurnEnd{Interrupted}`)도 지운다 · 닫힌 문의 끊기는 기록 없음 · 이어받기 줄은 건드리지 않는다 · 진행 줄이 이어져도 자물쇠는 전이 때만 잡힌다 |

**실프로세스 — `platform/process_group.rs` 의 시험마다 `#[cfg(windows)]`(선례 = `process_tree.rs:244-262`). 파일 전체를 `#![cfg(windows)]` 로 막지 않는다 — 비Windows 에는 「`process_group()` 이 `None` · 껍데기가 빈 명단」 시험이 따로 선다. ★한 시험이 띄우는 프로세스는 서넛 이하로 둔다(CLAUDE.md — 프로세스 생성 몰림이 개발 PC 터미널을 죽인다)★:**

1. Job 에 넣은 `cmd.exe /c ping …` 의 손자 `ping.exe` 가 `member_pids()` 에 든다 · 검증 → `Ready` · 끝내기 = `Terminated` · 곧 사라진다 · 종료 코드 = `LEFTOVER_EXIT_CODE`.
2. ★**실제 시계 음성 시험(무시 안 함 · 리뷰 2 K2 로 다시 짬)**★ — 3판의 모양(A · B 가 시험 프로세스의 자식)은 결정 4 뒤로 둘 다 부모가 살아 있어 문턱을 한 번도 거치지 않는다. 그래서: 중간 다리(`cmd.exe /c start "" /b <오래 사는 자식>` — 다리는 곧 끝난다)로 A 를 시험 Job 에 띄우고 다리가 끝나기를 기다린 뒤 ★다리의 `Child` 를 버려 핸들을 닫는다(리뷰 3 — 「A 의 부모 죽음」이 좀비 가정에 기대지 않게 · 좀비는 ⑤ 가 따로 잰다)★ → 실제 `wall_now()` 를 읽는다 → 같은 방식으로 B 를 띄운다. ★다리는 `CREATE_NO_WINDOW` 없이 시험의 콘솔을 물려받게 띄운다고 적는다 — 다리가 자기 콘솔 호스트를 얻으면 그것도 우리 Job · 문턱 뒤 · 다리가 끝나면 부모 죽음 · B 가 붙어 있는 동안 삶이라 함께 고를 수 있다(시험 러너에 콘솔이 없으면 새로 생길 수 있다)★. 그 뒤 ⓐ ★A · B 가 `member_pids()` 에 있음을 먼저 단언★ ⓑ **A 의 부모 상태가 `Dead` 임을 단언**한다(그래야 A 가 빠지는 이유가 문턱뿐이다) ⓒ 판의 고르기(보호 집합 비움) — ★B ∈ 고른 것 · A ∉ 고른 것 · 나머지는 전부 ppid 가 다리 둘 중 하나인 것(다리의 콘솔 호스트)★ ⓓ 실제 멤버의 생성 시각으로 술어를 직접 단언 — `created(A) < wall` · `created(B) >= wall` ⓔ 시험 프로세스 자신의 생성 시각 < `wall_now()`. 기점 변환 결함(모두가 문턱 뒤로 읽혀 떼로 죽이는 것)을 잡는다.
3. ★**시작시각이 어긋난 신원 = `NotOurs` — 우리 Job 의 산 멤버로 잰다**★ · 그 프로세스는 끝난 뒤에도 살아 있다.
4. ★**다른 Job 의 프로세스는 끝내지 않는다**★ — 남(F)을 **다른 Job** 에 넣고 그 신원을 주면 `NotOurs` · F 는 살아 있다(조사 §4 사고의 회귀망). ★**중첩 Job 변형**★ — 부모 Job P 에 X 를 넣은 뒤 우리 Job 에도 넣어 우리 Job 을 P 아래로 중첩 · F 는 P 에만 → `NotOurs` · F 생존(`IsProcessInJob` 에 NULL Job 을 넘기는 결함을 잡는다).
5. ★**좀비 부모 [리뷰 2 K4]**★ — Job 안의 P 가 자식 C 를 띄운다(`cmd.exe /c start "" /b ping …` 모양) · 시험이 P 의 핸들을 쥔 채 P 를 끝낸다 → `process_parent_table` 에 P 가 **없고** C 의 부모 상태 = `Dead`. 표가 좀비를 싣는다면 이 시험이 빨개지고 결정 4 가 잔여물을 놓치는 모양이 드러난다(프로세스 셋 — ②와 같이 다리가 콘솔 호스트를 얻으면 넷 · 다리는 시험의 콘솔을 물려받게 띄운다).
6. 명단 늘리기 — `member_pids_with_capacity(1)` 로 멤버 서넛을 물어 늘리기 · 되풀이 경로를 지나고 전부 나온다(리뷰 2 A2 — 3판의 80 개 띄우기를 대신한다).
7. `#[ignore]`(Git Bash 필요): **명시 경로**(`C:\Program Files\Git\usr\bin\bash.exe` — 없으면 명시 패닉 · `PATH` 의 `bash` 는 WSL 일 수 있어 쓰지 않는다)로 `-c "(sleep 30 &)"` 를 띄워 부모 없는 MSYS fork 자식이 **우리 Job 멤버로 남는지** · 부모 상태 `Dead` 인지 · 끝나는지. ★이 시험은 **Job 상속**만 잰다 — 멈춤은 재현하지 않는다(실제 멈춤을 쥔 것은 반쯤 fork 된 자식 — §3-0 Q1)★.

`base` 는 자기 시험(`child_pids_finds_spawned_child` 옆)에 「띄운 자식이 표에 부모와 함께 든다」 · 「`process_start` 가 끝난 번호에 `Gone`」 · ★「`process_start(0)` = `Unknown`」(리뷰 3)★을 더한다. 프로세스를 띄우는 시험이라 `cargo test -p engram-dashboard-agent -- --test-threads=4` · `cargo test -p engram-dashboard-base -- --test-threads=4` 로 돈다(CLAUDE.md 「병렬은 테스트 바이너리마다 걸린다」). CI 는 1–6 을 Windows 러너에서 돈다.

---

## 6. 구현 순서 — 어디서 멈춰도 빌드가 선다

한 무리(`agent` · `base`)라 **코더 하나 · 순차**가 기본이다. ①과 ②는 파일이 안 겹쳐 둘로 나눌 수 있다 — 그러려면 메인이 접점(`ProcessIdentity`(기존) · `ProcessStart` · `Verify`/`ReadyKill`/`MemberKill` · `WrapperShape` · §3-4 의 순수 함수 · 아래 포트 둘의 시그니처)을 먼저 못 박는다.

```rust
// leftover.rs 의 포트 — 실물은 ③ 에서 조립한다
pub(super) trait ProcessPort: Send + Sync {
    fn member_pids(&self) -> io::Result<Vec<u32>>;           // 완전한 명단만 · 통로가 사라졌으면 Ok(빈)
    fn parent_table(&self) -> Option<Vec<(u32, u32)>>;       // 표 한 장 · 실패 = None
    fn start(&self, pid: u32) -> ProcessStart;               // Known / Gone / Unknown
    fn verify(&self, who: ProcessIdentity) -> io::Result<Verify>;   // 자물쇠 밖 · Ready 의 terminate_raw 만 자물쇠 안(settle 은 밖)
}
pub(super) trait LeftoverClock: Send + Sync {
    fn wall_now(&self) -> u64;          // FILETIME 척도
    fn mono_now(&self) -> Instant;
    fn sleep(&self, d: Duration);
    fn spawn(&self, body: Box<dyn FnOnce() + Send>) -> io::Result<()>;
}
```

| 단계 | 내용 | 멈춰도 서는 이유 | 커밋 |
|---|---|---|---|
| ① | `base::platform::{process_parent_table, process_start}` + `child_pids` · `process_creation_time` 을 그 위로(동작 동일) + `base` 머리 둘(`lib.rs:3-4` · `platform.rs:1`) · OS 조각(`member_pids`(+ 이음새) · `verify_member` · `VerifiedMember::{terminate_raw, settle}` · 결과 enum) · `ProcessGroup` · 중립 결과 · `process_tree::walk_levels` · 통로 `Arc` 칸 + `pub(crate) process_group()` · `console_wrapper_depth()` · §5 실프로세스 시험 · `walk_levels` · 표 시험 | 새 함수는 시험 말고는 아무도 안 부른다 · `child_pids` · `process_creation_time` 기존 시험과 소비자가 동작 불변을 잰다 · 통로의 Job 동작 무변경 | 1 |
| ② | `leftover.rs`: 상수 · `GateState` 전이(시계 닻 포함) · `WrapperShape` · 순수 계획 함수 · 포트 둘 · 일꾼 몸통 · 끝내기 확정 · 판 · 로그 · 가짜 포트 시험 | 모듈은 등록되지만 배선이 없다 | 1 |
| ③ | `TurnGate` 를 `Mutex<GateState>` 로(끊기 줄 함수 · decoder 가 새 연산을 쓴다) · `open_spawn`(`:437-440`)이 `t.process_group()` · 뿌리 신원(`process_start(pid)`) · `WrapperShape::compose(console_wrapper_depth(), group.root_attached())` · 첫 시계 닻 · `control.map(|c| c.agent_id)` 로 정리기 조립(손잡이 · 뿌리 신원이 없으면 `None` — 오늘 그대로) · ★`control` 인자 주석(`:426-428`)과 버림 줄(`:431`) 고침★ · 실물 포트 둘 · decoder 배선 시험 · 앵커 | 이 단계가 처음으로 동작을 바꾼다 · 기존 끊기 시험(`gated_decoder`)이 문 동작 불변을 잰다 | 1 |
| ④ | 문서 — ADR-0246(`/adr` — ADR-0238 결정 3 · 5 와 ADR-0244 결정 2 개정 링크 · §9) · CLAUDE.md: ⓐ 「핵심 불변식」 소유권 분할 줄의 `transport=…/job` → 「Job 은 transport 가 만들고 끝내며 밖으로는 약한 손잡이만 · backend 일꾼이 멤버 하나씩 검증하고 끝낼 수 있다」 ⓑ `TurnGate` 줄(「decoder 와 그 함수가 같은 `Arc` 로 쥔다」)에 자물쇠 안 상태 · 에피소드 · 일꾼 ⓒ 락 순서에 새 잎 락 `TurnGate.state` 와 그 예외 둘(§3-3 끝의 정본 문장을 그대로) ⓓ 「백엔드 모듈 맵」 `base` 항목의 `platform` 설명(「PID liveness·프로세스 시작시각·자식 PID 열거」 → 프로세스 표 · 시작시각 세 갈래 판정) · T-40 은 GUI 실측 뒤 해소 | load-bearing 문서 → `/review doc` | 1 |

- `output_decoder()`(`claude/mod.rs:480-491` — 운영 밖 조립)는 자기 문을 만들 뿐 정리기를 꽂지 않는다.
- CI 에 `-D warnings` 가 없어(`.github/workflows/ci.yml` 확인) ①② 사이의 미사용 경고는 게이트가 아니다 — ③ 에서 전부 쓰인다.

---

## 7. 검증 계획

**기계 게이트(`/qa`):** `cargo test -p engram-dashboard-agent -- --test-threads=4` · `cargo test -p engram-dashboard-base -- --test-threads=4` · 워크스페이스 회귀 · `cargo fmt --check` · 코어 격리 `rg "^\s*use tauri" crates/engram-dashboard-agent/src/` · `crates/engram-dashboard-base/src/`(→ 0) · `base` 입주자 상호 무참조 · `base` 의존 상한(→ 1 줄). 프론트 무변경이라 `npm test` 는 `/qa` 바인딩 범위대로.

**GUI 실측(`/qa full` 격리 인스턴스 · 실제 대시보드 스폰 · 실제 claude · 실제 전역 훅 — ★N 을 세게 재는 자리이자 스파이크가 못 본 실제 경로의 판정★):**

| # | 시나리오 | 통과 조건 |
|---|---|---|
| G1 | 조사 실험 1 E 재현 — 실제 `UserPromptSubmit` 훅 · 보낸 뒤 약 0.2 초 Esc · **10 회 이상** | 매 회 「중단하는 중…」이 **N + 1 초(4 초) 안에** 풀린다(스파이크: 멈춤 아닌 끊기 ≤ 1.4 초 · 정리 뒤 `result` 18–21 ms) · 멈춘 회만 정리 warn · 나머지는 warn 0 · 멈춘 회의 화면이 보통 끊김과 같다(끊김 표시 행 — ADR-0243 · 오류 행 없음) · 다음 턴이 같은 claude 에서 답한다. ★스파이크는 +10 s 에 끝냈다 — +3 s 에 실제로 끝내는 것은 여기서 처음 잰다★ |
| G2 | 로그 대조 | `session_tracker` warn 의 `resolved_pid` 가 정리 warn 의 `wrapper_layer` 안에 있다(스파이크 Q2 = {conhost, claude}) · 「아무것도 못 끝냄」 warn 0 · 끝낸 PID 가 전부 정리 warn 에 있고 종료 코드 = `0x7440` · `clock_drift_ms` 가 허용치 안 |
| G3 | 조사 E-late(보낸 뒤 3 초 Esc) · 훅 없음 | 정리 warn 0 |
| G4 | 조사 B20(선언된 `sleep 20` 훅이 **Esc 전에** 시작) | 끝낸 것 0 · 「후보 0」 warn 한 줄(기대) · 턴 끝은 오늘처럼 약 20 초 |
| G5 | 정리가 난 턴 전후의 MCP 서버 · 다른 에이전트 · 다른 워크트리 빌드 | 살아 있다(조사 §4 같은 오살 없음) |
| G6 | ★스파이크가 안 덮은 것★ — ⓐ Bash 도구가 도는 중(예: 긴 `sleep`)에 Esc ⓑ 대시보드 실제 스폰(데몬 MCP 끝점 포함)에서 끊기 ⓒ ★**백그라운드 작업이 도는 중**(Bash `run_in_background` 로 `cargo build` 등 — 끊기 뒤에도 자식 `rustc`/`link.exe` 를 새로 띄운다)에 훅 멈춤을 일으킨다★ | ⓐ 끊기 뒤 N 초 안에 턴 끝이 오면 정리 warn 0 · 안 오면 끝낸 것이 전부 끊기 뒤 생긴 · 보호 밖 · 부모가 죽은 것 ⓑ 정리 전후 우리 MCP 연결이 안 끊긴다 ⓒ 빌드 자식(부모 작업이 살아 있다)은 하나도 안 끝나고(결정 4) 잔여물만 끝난다 · 빌드가 성공한다 |

- 멈춤은 경합이라(스파이크 3/11 · 조사 §2 ⑦ — 7 번 중 2 번) G1 에서 멈춤이 한 번도 안 나면 표본을 늘린다. 멈춤 0 으로 끝나면 「정리 경로 미관측」으로 보고하고 PASS 로 적지 않는다.
- ★G1 에서 잔여물이 정리되지 않고 300 초가 그대로면★ 「아무것도 못 끝냄」 warn 의 `reason` 부터 본다(모양 · 시계 · 명단 · 표 · 부모 상태) — 어느 쪽이든 닫힌 실패라 오살은 없다.

---

## 8. 위험 · 알려진 한계

1. **잔여 경합(ADR-0238 결정 7):** 끊기 줄이 앞 턴의 `result` 뒤 다음 턴에 닿으면, 앞 턴의 끝이 에피소드를 지워 그 다음 턴의 멈춤은 정리되지 않는다 — 오늘 그대로. claude 끊기 줄에 턴 id 가 없어 닫을 수 없다.
2. **턴 끝을 놓침 — `result` 유실(4 MiB 재동기 · 못 읽음 · 벤더 이상) · 문 구멍(`claude/mod.rs:759-764`):** 턴이 실제로 끝났는데 decoder 가 모르면 그 에피소드는 N 에 **판을 한 번** 돈다 — 끝난 턴과 멈춘 턴을 가를 수 없다. 그때의 후보에 **다음 턴의 프로세스**가 들 수 있다(예: 대기 입력 B 의 훅 bash). ★fork 도중의 훅 bash 를 끝내면 그 자체가 300 초 잔여물을 만들 수 있다(조사 §2 ② 의 모양 · 추론)★. 막는 것 둘: ⓐ 결정 4 — 도는 훅의 bash 사슬은 부모가 살아 있어 후보가 못 된다(스파이크 Q2 의 사슬: d2 런처 → d3 → d4 → d5 가 모두 산 부모를 둔다). 남는 것은 부모가 이미 죽은 것뿐이다. ⓑ 한 번 쓰고 버림(§3-3). 문 구멍 상태의 끊기는 한가한 CLI 에 닿아 `result` 가 없으므로 그 끊기마다 판이 한 번 돌지만, 그 끊기 뒤 태어난 것만 고른다(한가한 CLI 아래엔 대개 없다).
3. **끊기 뒤 태어나 N 초 넘게 사는 정당한 프로세스 — 결정 4 로 좁혔다:**
   - ★**백그라운드 작업의 자식(리뷰 1 M 이 찾은 피해 부류) — 이제 빠진다**★: claude 의 백그라운드 작업(Bash `run_in_background` — 예: `cargo build`)이 [Esc, Esc+N] 에 새로 띄운 자식(`rustc` · `link.exe` — 깊이 4 이상)은 부모 작업이 살아 있어 후보가 못 된다(결정 4 · §7 G6 ⓒ). 부모 판정의 짝 틈은 두 번 읽기가 막는다(§3-4 · 리뷰 2 K1).
   - 도는 훅의 손자도 같은 이유로 빠진다(부모가 살아 있다).
   - **남는 노출** = 끊기 뒤 태어나 N 에 살아 있는데 **부모가 이미 죽은** 정당한 프로세스(예: 스스로 부모를 떠나는 데몬화 — 끊기 뒤 N 초 안에 claude 아래에서 새로 떠야 한다). 스파이크 11/11 에서 그런 것은 없었다. 조사 §7 「잘못 발화해도 맞는 것은 Esc 뒤에 생긴 프로세스뿐이다」.
   - ★**결정 4 로 새로 놓치는 것**★ = 멈춤을 쥔 것의 부모가 **살아 있는** 모양(아직 관측 없음 — 잰 3/3 은 부모가 죽어 있었다) · 표가 좀비 부모를 싣는 경우(§5 ⑤ 가 박는다). 그 모양이 나오면 정리는 그것을 건드리지 않고 그 끊기는 오늘처럼 멈춘다(「후보 0」 warn 이 부모가 살아 있어 뺀 수를 남긴다). 사용자: 「어차피 잘 안일어나는 일이니깐」.
4. **래퍼 모양 가정 — 실측으로 섰다(스파이크 Q2 · 4/4):** claude.exe 는 `cmd.exe` 의 직계 자식이고 런처 층이 없다. 같은 층의 콘솔 호스트는 §3-4 가 층 식구 수로 흡수했다. 남는 위험: 설치 방식이 바뀌어 런처가 한 겹 끼면 래퍼 층이 {conhost, 런처} 로 모양 검사를 **통과**하고 「claude 의 직계 자식」이 한 층 어긋난다. G2 의 `resolved_pid ∈ wrapper_layer` 대조가 그것을 잡는다(런처면 실패한다).
5. **시계:** 해상도는 놓치기만 한다(§3-6). 허용치(1 초)를 넘는 뒤로 뛰기는 닻이 보고 그만큼 쉰다(놓치는 쪽 · warn). ★허용치 안의 뒤로 뛰기는 못 본다★ — Esc 전 그만큼 안에 태어난 프로세스가 문턱을 지날 수 있고, 부모와 자식이 태어난 사이에 그런 뛰기가 끼면 자식이 부모보다 이르게 읽혀 **살아 있는 부모가 「부모 죽음」(재사용)으로 뒤집힐 수 있다**(결정 4 가 지키던 자식이 후보가 된다 · 드묾 — 두 탄생이 뛰기 폭 안에 붙어 있어야 한다). 멤버의 `created <= now_wall` 이 그 일부를 거른다. 닻은 기록 · 판마다 다시 잡혀 슬루는 쌓이지 않는다 — 다만 긴 휴지 뒤 첫 끊기에서는 그 사이 쌓인 슬루가 허용치를 넘으면 한 번 쉰다(빈도는 `clock_drift_ms` 로그로 잰다).
6. **스폰 직후의 Job 편입 틈 — 실측(스파이크 Q2):** `spawn` 이 돌아온 뒤 `assign`(`stdio.rs:110-127`) 완료까지 33–40 µs 였고 claude.exe 는 그 약 58–63 ms 뒤에 생겼다 → claude 트리가 Job 밖으로 샐 여지는 사실상 없다(4/4 모두 Job 안). ★콘솔 호스트는 `assign` 뒤 약 1.6–2.1 ms 에 생겨 여유가 좁다★ — 부하가 크면 Job 밖에 날 수 있고 그러면 `KILL_ON_JOB_CLOSE` 가 그것을 못 거둔다. T-40 범위 밖이다(걷기는 Job 이 아니라 프로세스 표를 걸어 모양 가드에는 영향이 없다) — 기록만 한다.
7. **전제 — 실측으로 섰다(스파이크 Q1 · 3/3):** 멈춤을 쥔 잔여물은 우리 Job 멤버였다(완료 포트 알림 · `IsProcessInJob` · 멤버 명단 셋 다). 표본은 멈춤 3 번이다.
8. **대체 없음(결정 2):** 정리가 턴을 못 풀면 사용자가 죽이고 다시 연다 — warn 한 줄이 그 사실을 남긴다.
9. **Windows 에서만 동작한다.** 다른 OS 는 무동작이다(조사 §8 — Windows 에서만 쟀다).
10. **upstream 이 고치면(#85250) 이 경로는 잠든다** — 턴 끝이 N 안에 오므로 판이 돌지 않는다. 걷어낼 필요는 없고, 걷을지는 그때 정한다. 스파이크의 claude 2.1.284 에서도 멈춤은 그대로 났다(3/11).
11. **taskkill 의 콘솔 호스트는 보호 층 밖이다(§3-4) — 결정 4 가 막는다:** taskkill 이 N 초 넘게 매달리면(upstream #67888) 그 conhost 의 부모가 살아 있으므로 후보가 못 된다. taskkill 이 죽은 뒤에도 그 conhost 가 남았다면 후보가 된다(부모 죽음 · 문턱 뒤) — 스파이크에서 taskkill 최장 278 ms · 그 conhost 최장 277 ms 로 함께 끝났다(44 개 관측).
12. **닫힌 실패의 대가:** 명단 불완전 · 표 목록 중간 오류 · 보호 깊이의 Job 멤버 시작시각 못 읽음 · 첫 끊기 무렵 시계 뛰기 · 일꾼 기동 실패 가운데 하나라도 나면 그 판(또는 에피소드)은 아무것도 안 끝낸다 — 그 끊기는 오늘처럼 멈춘다(warn 이 사유를 남긴다). 한 에피소드에 판은 한 번이라 다시 시도하지 않는다.
13. **줄이 claude 에 안 닿았는데 에피소드가 남는 경우(§3-2 · 리뷰 2 A7):** 입력 큐가 쓰기 오류 · 라이터 패닉으로 닫혀 claude 의 stdin 이 깨진 채 턴이 돌면, 끊기는 전해지지 않았는데 에피소드가 남아 N 에 판이 한 번 돈다(`push` 성공 뒤 쓰기가 실패하는 경우도 같다 — 되돌릴 신호가 없다). 고르는 것은 끊기 뒤 태어나 N 에 살아 있고 부모가 죽은 보호 밖 멤버뿐이다. 라이터 기동 실패(스폰 직후)는 문이 열리지 않아 해당 없다 · `shutdown()` 은 Job 이 끝나 해당 없다.
14. **끝내기 확정은 잎 자물쇠 안의 OS 호출 하나(`TerminateProcess`)다(§3-3 — 시계 읽기 외엔 그것뿐):** `TerminateProcess` 가 드물게 늦으면(대상이 커널 안에서 끝내기 어려운 상태 등 · 관측 없음) 그동안 끊기 줄 함수와 decoder 의 문 전이가 기다린다. 판 동안에만 · 후보 수만큼이다.

---

## 9. ADR 후보 · 앵커

**가안 ADR-0246 — 「claude 끊기 뒤 턴 끝이 N 초 안 오면 우리 Job 안의 끊기 뒤 생긴 잔여물만 끝내고 claude 는 살린다 — 화면 고지도 대체도 끄는 수단도 없다」** (번호 = `/adr` 가 채번 · `docs/decisions` 마지막 = 0245 확인)

- **결정 [사용자]:** §1 표 전부(채택안 · 고르기 · PID 규칙 · 시계 · N · OS 조각 · 결정 1 · 2 · 3 · 4). ★결정 4 = 부모가 살아 있으면 건드리지 않는다 — 후보는 부모가 죽은 것만(부모 판정 = 표의 ppid 가 살아 있고 자기보다 먼저 태어났나 · 번호를 늦게 태어난 프로세스가 쥐면 죽은 것 · 판정 불가면 건드리지 않음). 주 기준 위의 덧붙인 거르개이고 「고아 여부는 주 기준이 아니다」(조사 §7)와 어긋나지 않는다★.
- **구현 세부 [TRD — 뒤집을 수 있다]:** 문 확인과 기록을 한 자물쇠로(§3-2) · 에피소드 = 첫 끊기가 문턱 · 가장 최근 끊기가 대기 · 한 번 쓰고 버림 · 일꾼 하나 · 일꾼 기동 실패는 에피소드 `Done`(§3-2 · §3-3) · ★끝내기 확정 = 자물쇠 밖 핸들 검증 + 자물쇠 안 재확인과 `TerminateProcess` 호출 하나 · 닫기와 분류는 놓은 뒤 · 시계 두 값은 자물쇠 안에서 읽고 전이는 단조 값을 되감지 않는다★(§3-2 · §3-3) · 래퍼 모양 = 래퍼 깊이 + 스폰이 붙인 프로세스 수 · 모양이 어긋나면 정리 안 함(§3-4) · 멤버 시작시각 두 번 읽기 · 명단 · 표 · 멤버 시작시각 실패는 닫힌 쪽 · 사라짐은 건너뜀(§3-4) · 시계 닻 다시 잡기와 쉼 · 멤버의 미래 생성 거르기(§3-6) · 끝내기 = 같은 핸들로 우리 Job 소속 · 신원 · 생존 확인(§3-5).
- **개정 링크(명시):**
  - **ADR-0238 결정 3** — `TurnGate { open: AtomicBool }` 가 `Mutex<GateState>`(열림 + 에피소드 + 일꾼 + 시계 닻 칸)로 바뀐다. 여닫기 규칙과 거절 계약은 그대로다.
  - **ADR-0238 결정 5** — 「우리가 끊기를 보냈다」 표식은 결과 분류용 예비로만 두기로 했었다(`TurnGate.interrupt_sent`). 이 ADR 이 그 자리에 **정리 용도의** 기록(에피소드)을 세운다. 끊김 분류는 여전히 `terminal_reason` 이 하고 이 기록을 읽지 않는다.
  - **ADR-0244 결정 2** — 「백엔드는 바뀌지 않는다 · 턴마다 끊기 한 번 표식은 대체된다」. 이 ADR 로 backend 가 바뀐다: 되풀이 끊기를 거절하지는 않지만(막는 자리는 여전히 프론트) 에피소드로 **기록한다**.
- **거부한 대안(출처가 있는 것만):**
  - **claude 재시작(죽이고 이어받기)** — 채택안이 「claude 는 살린다(재시작이 아니다)」(조사 §7). 피어 중 t3code(`query.close()` 뒤 `--resume`) · happy(죽이기 + resume)가 이 모양이다(조사 §6).
  - **정리했다는 화면 고지** — 사용자 「그걸 왜 알려」(결정 1).
  - **정리 뒤에도 안 끝나면 자동 재시작** — 사용자 「너무 복잡해」(결정 2).
  - **끄는 수단(환경 변수)** — 사용자 「ㅇㅇ 두지 않고」(결정 3).
  - **부모가 산 후보도 고른다(규칙 글자 그대로)** — 백그라운드 작업이 끊기 뒤 띄운 자식을 끝낼 수 있다(리뷰 1 M). 사용자가 좁히기를 골랐다: 「ㅇㅇ 조건 더해줘 어차피 잘 안일어나는 일이니깐」(결정 4).
  - **PID 부모 연결로 고르기**(부모 뒤 생성 검사를 더해도) — 조사 §4 사고(남의 `link.exe` 등을 죽였고, PID 재사용으로 21 번 중 4 번 · 18 번 중 3 번 남의 것을 골랐다).
  - **고아 여부 · 「bash」 인지 · CPU 상태를 주 기준으로** — 정당한 고아(예: dev 서버)가 있고 한가한 프로세스는 얼어 보인다(조사 §7).
  - **피어의 다른 모양(조사 §6)** — 유예 뒤 로컬에서 턴을 닫기(Zed 30 초 — 「a new session may be required」) · 트리 통째 죽이기(vibe-kanban · paseo · crystal) · 끊지 않고 기다리기(VS Code 확장 · claude-code-action). 조사 §6 은 「claude 를 살려 둔 채 자손만 죽이는 피어는 없다」고 적고, 사용자는 §7 을 골랐다.
  - **upstream 수정을 기다리기** — CHANGELOG 2.1.284 까지 수정 없음 · 「claude 의 일반 결함이지만 우리 모드에서 드러난다 → 대시보드가 완화한다」(조사 §7 판단).
  - [TRD] **`open_spawn` 에 `agent_id` 인자** — 제어 끝점이 이미 싣고, 따로 받는 형태로 되돌리지 말라는 결정이 있다(ADR-0217 결정 5 · `types.rs:644-647`).
  - [TRD] **재확인 뒤 자물쇠를 놓고 끝내기** — 그 틈에 턴 끝 · 새 끊기가 들어와도 끝낸다(리뷰 2 C1).
  - [TRD] **`push` 실패 때 에피소드 되돌리기** — `push` 성공 뒤의 쓰기 실패를 못 덮어 구멍을 반만 막고 `InterruptLine` 계약을 넓힌다(§3-2 · 리뷰 2 A7).

**앵커(`// ADR-0246`) 달 자리:** `JobObjectHandle::member_pids` · `verify_member` · `VerifiedMember::{terminate_raw, settle}` · `MemberCheck`/`MemberOutcome` · `platform/process_group.rs` 머리 · `process_tree::walk_levels` · `base::platform::process_parent_table` · `process_start` · `StdioTransport::process_group` · `console_wrapper_depth` · `leftover.rs` 머리 · `INTERRUPT_LEFTOVER_GRACE` · `CLOCK_STEP_TOLERANCE` · `LEFTOVER_EXIT_CODE` · `GateState` · 끝내기 확정 구간 · `select_leftovers` · `members_with_parents` · `keep_set` · `WrapperShape::compose` · `TurnGate` · decoder 의 문 여닫기 · `interrupt_line` 의 `try_interrupt` · `open_spawn` 의 정리기 조립과 `control` 주석. 기존 `// ADR-0238` 앵커는 그대로 둔다.

---

## 10. 사용자 결정 필요

> 4판 기준: **남은 사용자 결정 없음.** 리뷰 2 라운드는 사용자 체감이 있는 새 갈림을 만들지 않았다(전부 내부 구현).

1. **이 정리를 끄는 수단을 둘지 — 닫힘: 두지 않는다**(결정 3 · 사용자 2026-09-29 · 「ㅇㅇ 두지 않고」).
2. **부모가 살아 있는 후보를 건드릴지 — 닫힘: 건드리지 않는다**(결정 4 · 사용자 2026-09-29 · 「ㅇㅇ 조건 더해줘 어차피 잘 안일어나는 일이니깐」). 올렸던 두 갈래: (a) 규칙 글자 그대로 — 백그라운드 작업이 끊기 뒤 띄운 자식이 끝내질 수 있다 · (b) 좁히기 — 부모가 산 것은 빼고 부모가 죽은 것만 고른다(잰 잔여물 3/3 이 그 모양). 사용자는 (b) 를 「부모가 살아 있으면 건드리지 않는다」로 정했다 — 메인이 올린 (b) 의 「문턱 전에 태어난 부모」 단서 없이, **부모가 살아 있기만 하면** 뺀다(§3-4). 대가 = 부모가 산 멈춤 잔여물은 못 고른다(§8 ③).

---

## 11. 의존성 · 자리

- **새 crate 없음 · 새 `windows` feature 없음.** 쓰는 API 는 `Win32_System_JobObjects`(`QueryInformationJobObject` · `IsProcessInJob` · `JobObjectBasicProcessIdList`) · `Win32_System_Threading`(`OpenProcess` · `GetProcessTimes` · `WaitForSingleObject` · `PROCESS_SYNCHRONIZE` · `TerminateProcess`) · `Win32_Foundation`(`WAIT_OBJECT_0` · `WAIT_TIMEOUT`) — 전부 `crates/engram-dashboard-agent/Cargo.toml:49-56` 에 이미 켜져 있다(`windows-0.58.0` 소스에서 정의 위치 확인). `base` 의 새 함수는 이미 켜진 `Win32_System_Diagnostics_ToolHelp` · `Win32_System_Threading` · `Win32_Foundation`(`crates/engram-dashboard-base/Cargo.toml:19-24`)만 쓴다.
- 벽시계는 std `SystemTime` 이다 — `GetSystemTimeAsFileTime` 을 쓰면 `Win32_System_SystemInformation` 이 새로 들어 피했다.
- **`base` 변경 = 기존 입주자 `platform` 에 함수 둘(`process_parent_table` · `process_start`) + 기존 함수 둘(`child_pids` · `process_creation_time`)을 그 위로.** 게이트 셋(CLAUDE.md 「빌드·검증 명령」)을 그대로 지난다 — 워크스페이스 의존 0(의존 상한 1 줄) · `use tauri` 0 · 입주자(`logging` · `platform`) 상호 무참조.
- **왜 `agent` 가 아니라 `base` 인가 [리뷰 2 A1 — 자리 근거]:** ADR-0175 결정 2 는 Job Object 래퍼를 소비자가 `agent` 안(통로 둘)뿐이라 `agent` 에 남겼고, `process_tree` · `file_holders` 도 같은 「소비자 하나」 사유로 `agent` 에 있다(`platform/mod.rs:4-7`). 이번 두 함수는 사정이 다르다:
  - ⓐ **이미 `base` 에 있는 것의 몸통이다** — `child_pids` 의 Toolhelp 순회와 `process_creation_time` 의 여는 판정을 각각 그 위로 옮긴다. `agent` 에 두면 같은 Toolhelp 순회가 두 crate 에 사본으로 생기거나(「재사용, 복제 금지」), `base` 의 기존 함수를 `agent` 가 감싸 오류를 다시 가려야 한다.
  - ⓑ **feature 가 이미 `base` 에만 있다** — Toolhelp(`Win32_System_Diagnostics_ToolHelp`)는 `base` 만 켠다(CLAUDE.md 「의존성」 — `agent` 는 Job Object · Restart Manager 쪽). `agent` 에 두면 그 feature 를 `agent` 에 새로 켜야 한다.
  - ⓒ **소비자 둘 이상(입주 조건 ①)** — 옮긴 뒤 `child_pids` · `process_creation_time`(과 그 위의 `pid_alive_with_start_time` · `current_process_start_time`)의 기존 소비자(`net` · `discovery` · `daemon` · `agent` 의 codex backend · `process_tree` · 시험들 — `rg -l "engram_dashboard_base::platform" crates`)가 이 몸통을 쓰고, 정리기가 하나 더 쓴다 · 도메인 지식 0(조건 ②) · 입주자 무참조(조건 ③).
- **`base` 머리 갱신(① 단계):** `crates/engram-dashboard-base/src/lib.rs:3-4`(입주자 요약 — 「PID liveness · 프로세스 시작시각 · 자식 PID 열거」 → 프로세스 표 · 시작시각 세 갈래 판정을 더한다) · `crates/engram-dashboard-base/src/platform.rs:1`(모듈 머리). CLAUDE.md 「백엔드 모듈 맵」의 `base` 항목은 ④ 단계(§6).
