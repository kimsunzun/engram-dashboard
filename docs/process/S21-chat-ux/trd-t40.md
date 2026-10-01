# TRD — T-40: claude 훅 도중 끊기의 약 300 초 멈춤 완화 — 우리 Job 안의 끊기 뒤 잔여물만 끝낸다 (S21)

> 상태: ★**착지 (2026-10-01) — 구현 ①②③ 커밋(`ccde81a` · `235cbca` · `e75f955`) · GUI 실측 G1 PASS · 결정 = ADR-0262**★(ADR-0257 · ADR-0238 · ADR-0244 부분 대체). ④ 문서 착지에서 구현 리뷰가 찾은 서술을 고쳤다 — §3-2 쓰기 명단 실패 · §3-4 런처 · 걸음 상한 · 훅 모양 · taskkill 모양 · §3-7 · §8 ⑰ⓑ 같은 눈금 · §3-8 패닉 레벨(§2 · §3-3 · §6 의 같은 말 포함) — 고친 자리마다 「착지 고침」. 구현 리뷰가 남긴 알려진 한계 K1–K9 = §8 ㉔(사용자 결정 2026-10-01 「일단 재현 가능한것만 구현하자. 좀 현실적인 사항으로 단순화」 — 코드 없이 기록). ★§5 실프로세스 시험 13 은 그 결정으로 짓지 않았고 · §7 G1b · G3–G6 은 돌리지 않았다★. 아래는 착지 전 머리(그대로).
>
> 14판 머리: **초안 14판 · 끊기 뒤 탄생 기록 + Esc 때 스냅숏 · 리뷰 전 (2026-09-30)** — 14판 = 13판에 확인 라운드 고침 셋: ⓐ ★「쓰기 전 탄생은 후보가 아니다」를 알림이 쌓이는 때(문서 없음)가 아니라 **쓰기 확인 때 찍은 Job 멤버 번호 명단(쓰기 명단 W)** 으로 세운다 — 13판의 표지 패킷은 걷었다★ ⓑ 듣는 스레드를 **먼저** 띄우고 그 뒤에 Job 을 포트에 붙인다(붙이기가 실패하면 스레드는 끝난다 — 쌓일 알림이 없다) ⓒ 쓰기와 W 사이에 태어난 것은 빠진다(놓침 — 잰 여유 · 시험). 13판 = 12판에 라운드 4 고침: ⓐ (14판이 대체) ⓑ 포트 떼기가 실패하면 듣는 스레드가 계속 꺼내 버린다 · 듣는 스레드 패닉 막이 ⓒ taskkill 고리에도 생성 순서 ⓓ 끝내기가 거절되면 판 전체를 멈춘다 ⓔ 「묵은」이면 에피소드를 세우지 않는다 · 여는 이 가드가 모든 비정상 출구를 덮는다 ⓕ 기록 세부(켤 때 경계 · 넘침 초기화 · 기록이 사라지면 `snapshot_failed(port)`) ⓖ 한 차례에 확인 마감 하나 ⓗ 생성 순서 검사는 사용자 결정이 아니라 **[고름]**(사용자에게는 알렸다) ⓘ §9 를 ADR 줄 단위로 ⓙ 첫 에피소드의 붙일 때 되알림 ⓚ `tear_down_failed_activation` 도 물러남 시작을 부른다. 12판의 뼈대(결정 11 — 첫 끊기부터 판까지 가입 알림으로 태어나는 것을 곧바로 열어 기록 · 끊기 전 조상은 Esc 때 스냅숏 · 고리마다 부모 생성 ≤ 자식 생성 · 끝낸 뒤 확인하고 다시 고르기 최대 3 차례 · claude 의 `taskkill /PID <꼭대기>` · 쓰인 뒤 탄생만 후보 · 여는 이 하나 · 포트 수명 · `kill_agent` 첫 줄의 물러남 · 쓰기 확인 패닉 막이)는 그대로다. ★새 `windows` feature 둘(`Win32_System_IO` · `Wdk_System_Threading`) = 의존성 변경 · §10★. 판 이력 = `git log -- docs/process/S21-chat-ux/trd-t40.md`. 코드 무변경. 사용자 체감이 없는 내부 구현은 이 문서가 골랐다(**[고름]**).
>
> **입력:** 추적 [`docs/tracking.md` T-40](../../tracking.md) · 조사 [`docs/research/claude-interrupt-hook-hang-2026-09-29.md`](../../research/claude-interrupt-hook-hang-2026-09-29.md) · 앞선 TRD [`trd.md`](trd.md) §3-4(ADR-0238). **판독 기준** — 코드 = 1단계 커밋 `9fa9215`(**HEAD**) · 5판의 2·3단계 = `refs/backup/t40-clock-design-20260930`(**백업**). `windows` crate = 이 PC 의 `windows-0.58.0` 소스. 측정 = `.claude/handoff/attachments/` 의 `20260929-t40-spike/` · `20260930-msys-ppid-spike/` · `20260930-t40-provenance-spike/`(`WRITEUP.txt` · `main.log` · `pilot.log` · ★`procs/p1..p6.procs.tsv`(main) · `procs/pilot-p1.procs.tsv`(pilot) — 둘 다 보존★ · `probe/`). claude 내부 = claude 2.1.284 번들 코드 읽기(2026-09-30). MS Learn = `JOBOBJECT_ASSOCIATE_COMPLETION_PORT` · I/O Completion Ports(§3-5).
>
> **앵커:** ADR-0001 · ADR-0004 · ADR-0006 · ADR-0012 · ADR-0175 · ADR-0217 · ADR-0218(★열린 핸들로 신원을 붙든다★) · ADR-0230 · ADR-0238(★결정 2 · 3 · 5 개정★) · ADR-0244 · ADR-0245 · **ADR-0257**(부분 대체 · §9).
>
> 표기: **[고름]** · **[사용자]**. **에피소드** = 끊기 한 판. **스냅숏** = 첫 끊기 때 우리 Job 멤버를 붙든 것. **탄생 기록** = 그 에피소드 동안 가입 알림으로 온 프로세스를 곧바로 열어 붙든 것(순번 `seq`). **사실** = 부모 PID · 생성 시각 · 실행 파일 · 명령줄(붙든 핸들로 한 번 읽는다). **알려진 고리** = 스냅숏 또는 탄생 기록에 있는 프로세스. **훅 실행기** = claude 가 훅마다 띄우는 `…\Git\bin\bash.exe -c "bash …"`.

---

## 0. 결론 (먼저)

| 무엇 | 어디 | 요지 |
|---|---|---|
| **끊기 기록** | `backend/claude/leftover.rs` 의 `GateCell` | 문 자물쇠 구간 ① → (새 에피소드 · 여는 이는 하나) 자물쇠 밖에서 탄생 기록을 켜고 스냅숏 → 구간 ② 에서 연다(그 사이 턴이 닫혔으면 줄을 주지 않는다 · 기록 번호 · 차례가 바뀌었으면 줄만 주고 에피소드는 세우지 않는다). N 초는 줄이 stdin 에 **쓰인 순간**부터 · ★쓰인 직후 라이터가 Job 멤버 번호 명단 W 를 한 번 찍는다 — 후보는 W 밖이어야 한다★ |
| **탄생 기록** | 듣는 스레드(화신당 하나 · 첫 끊기에 뜬다 · ★뜬 뒤에 Job 을 포트에 붙인다★) | Job 가입 알림을 기다려 기록 중이면 곧바로 열어 사실을 읽고 붙든다(끝남 알림 버림 · 결정 6). 기록 중이 아니면 버린다(결정 7). 실패하면 포트를 떼고 그 화신은 기록하지 않는다 |
| **N 초 확인** | 일꾼 스레드 | 쓰인 끊기 + N 에 판. 고르고 → 끝내고(순번 순 · 재확인 + `TerminateProcess` 는 문 자물쇠 안) → 끝남을 확인하고 → 다시 고른다(최대 3 회) |
| **고르기** | 같은 파일 | 후보 = 기록된 탄생 · ★번호가 쓰기 명단 W 에 없음(= 줄이 쓰이기 전부터 살아 있던 것이 아님)★ · 스냅숏 밖 · 산 것. 끝내는 것은 모두를 채운 것: ① 부모가 알려진 고리 · 끝남(결정 11) ② 훅 사본(부모와 실행 파일이 같고 명령줄이 같거나 부모가 `-c "bash …"`)(결정 10) ③ claude 까지 모든 고리가 알려짐 · 끝남 · **부모 생성 ≤ 자식 생성**(결정 9 · 11 · 5) ④ 꼭대기 = 훅 실행기이고 이 에피소드에 claude 가 띄운 **`taskkill /PID <꼭대기> /T /F`** 탄생이 있으며 그 taskkill 이 꼭대기보다 늦게 태어났다(결정 10) |
| **물러남** | 통로 · 세션 · manager | ★`kill_agent` 시작(권한 회수 · `Exiting` 전)에 manager 가 세션 → 통로의 「물러남 시작」을 부른다★ — 통로가 쥔 읽기 전용 `RetiringSignal` 이 선다 · 판 · 재확인이 본다 |
| **OS 조각** | `platform/windows.rs` | 명단(1단계) · 붙들기(`pin_member` — 소속 · 사실 · 끝내기 권한 선택) · 가입 알림 포트(`watch_births` 한 번 — 포트 → 듣는 스레드 기동 → 붙이기 순서를 API 가 강제 · `unwatch_births` · `next`) |
| **고지 · 대체 · 끄기** | 없음 [결정 1·2·3] | 화면은 평소 끊김과 같다 · warn 로그만 |

- **잰 멈춤에 대면(추론 — 기록에 규칙을 대 본 것): 10/10** — 표가 있는 7 번(09-30 A 5 · 백그라운드 p3bg · pilot p1bg)은 사슬 · 생성 순서 · 훅 사본 · `taskkill /PID <꼭대기>`(7/7 · 2 차례씩)까지 표로 서고, 09-29 3 번은 같은 모양으로 본다(§3-0). ★다시 돌릴 수 있는 대조 = `20260930-t40-provenance-spike/r4sim.py` — 표 7 장의 taskkill 에피소드 35 개 전체에 규칙을 대 보면 주인 7 개 + p3bg 2 차례의 24776 만 고르고 다른 것은 하나도 고르지 않는다★. 조건 = 짧게 산 중간 고리를 제때 연다(하네스 4322 개 중 못 연 것 0).
- **구현 순서**(§6): ① 1단계 걷기 · OS 조각 · 물러남 · 쓰기 확인 → ② 순수 코어 → ③ 배선 → ④ 문서. **새 ADR 필요**(§9). **사용자 확인 = 1 건**(§10 — feature 둘).

---

## 1. 사용자 결정 (2026-09-29 · 2026-09-30)

| # | 결정 | 출처 |
|---|---|---|
| **채택안** | ★우리 Job 안의 멈춘 잔여물만 죽이고 claude 는 살린다★ | 조사 §7 |
| **고르기** | 실제로 끊기를 보낸 Esc 의 시각을 적는다. N 초 뒤 턴 끝이 없으면 Job 멤버 중 claude 아님 · 직계 자식 아님 · 적은 시각 뒤에 생성 · 살아 있음 인 것을 죽인다. 고아 · 「bash」 · CPU 는 주 기준이 아니다. ★14판 읽기(문구는 그대로 두고 다시 읽는다)★ — 「적은 시각 뒤에 생성」 = 기록된 탄생 가운데 줄이 쓰인 직후의 멤버 명단(W)에 없던 것 · 「가장 최근 Esc」 = 가장 최근에 쓰인 끊기 · 「claude 아님 · 직계 자식 아님」은 §3-4 가 구조로 지킨다 | 조사 §7 |
| **PID 규칙** | ★PID 부모만으로 고르지 말 것 — Job 명단 + 생성 시각★. ★14판 읽기★ — 후보는 우리 Job 의 가입 알림에서만 · 같은 핸들로 소속 · 부모 연결 · 쓰기 명단 W 는 줄이기만 한다. ★**[고름 — 사용자 결정 아님]** 이 규칙의 「생성 시각」을 고리 두 끝의 상대 순서로 쓴다: 붙든 핸들로 읽은 부모 생성 ≤ 자식 생성(번호 재사용으로 엉뚱한 부모가 끼는 것을 거른다 · 줄이기만 한다) — 사용자에게는 알렸다(§10)★ | 조사 §4 |
| **시계 · N** | ~~시스템 시각과 비교~~ · N = 3 초 · 단조 시각 | 조사 §7 |
| **OS 조각** | Windows 전용 조각은 작게(명단 + 하나 끝내기) · 다른 OS 무동작 · 옮길 수 있게. 붙들기 · 사실 · 알림 포트는 그 조각 안 | 조사 §7 |
| **결정 1 · 2 · 3** | ★고지 없음(「그걸 왜 알려」) · 대체 없음(「너무 복잡해」) · 끄는 수단 없음(「ㅇㅇ 두지 않고」)★ | 2026-09-29 |
| **결정 4** | ★부모가 살아 있으면 건드리지 않는다★(「ㅇㅇ 조건 더해줘 어차피 잘 안일어나는 일이니깐」) — 결정 9 가 조상까지 넓혔다 | 2026-09-29 |
| **결정 5** | ★「끊기 뒤에 태어났나」를 절대 시각 대신 **Job 가입 알림**으로 가른다★. 사용자: 「근데 생성된 절대시간말고 상대시간은 모름?」 · 「알림이 오면 그냥 그거 핸들 보관한다음에 그것들만 이터 돌면 되겠네」. ★「Esc 뒤에 태어났나」를 시각(Esc 시각과 생성 시각의 비교)으로 가르지 않는다★ | 2026-09-30 |
| **결정 6** | ★생성만 기록하고 빠지는 것은 처리하지 않는다★(「생성된것만 기록하고 빠지는건 처리 안하고 그냥 날리면 되는거지」 · 「구지 처리하지 않아도 되는건 처리하지 않아도됨. 종료 이벤트까지 받아서 매번 빼고 그러면 번잡하잖아」) | 2026-09-30 |
| **결정 7** | ★수집은 Esc 할 때만 · 구현은 가장 단순하게★(「어쨋든 esc 할때만 수집하는게 맞아. 그 안에서는 너가 자유롭게하면됨. 나중에 프로세스를 계속 들고있어야되는 이유가 생기면 그쪽으로 가는게 맞겠지만. 그 안에서는 구현 심플한 쪽으로 하면 될듯」) — 기록 · 붙들기는 첫 끊기부터 판까지만 · 포트를 붙인 뒤 에피소드 밖 알림은 꺼내 버린다 | 2026-09-30 |
| **결정 8** | ~~죽은 부모가 Esc 전부터 있던 것일 때만~~(「죽은 부모가 bash 면 esc 전부터가 맞긴하겠구나. 방어 되면 그렇게 진행해.」) — ★결정 11 이 대체(모든 고리가 알려짐 · 끝남)★ | 2026-09-30 |
| **결정 9** | ★죽은 부모에서 claude 까지 타고 올라가 살아 있으면 제외★(「ㅇㅇ 알았어. 어쨋든 부모 쭉 타고올라가서 살아있으면 제외한다는거잖아. 딱히 로직 부담도없겠네 상위부모 탐색만 하면되니」) · claude · 뿌리는 멈추는 자리 | 2026-09-30 |
| **결정 10** | ★깨끗한 훅 표지가 있으면 더한다 · 지저분하면 남는 틈을 받아들이고 진행한다★(메인 계획 · 사용자 수락). 사용자: 「어차피 5분 멈추는것보다는 훨씬 좋은거잖아 이것들이.」 · 표지가 깨끗했다 → 훅 사본 · 훅 사슬(실행기 모양 + claude 의 `taskkill /PID`) | 2026-09-30 |
| **결정 11** | ★3 초 동안 오는 것을 기록하고 삭제되는 것은 빼지 않는다 — 한 번 찍고 비교하는 것이 아니다★. 사용자: 「아니 내가 원래 얘기했던게 3초동안 오는거 기록하고 삭제되는건 빼지 말라고 했는데 한번 캡쳐하고 다음번에 비교하라고 해서 그렇게 진행한건데?」 → 결정 5 · 6 되살림 · 7 그대로 · 8 대체 | 2026-09-30 |

---

## 2. 바뀌는 자리 (기준 = HEAD `9fa9215` · 백업)

| 파일 | 무엇 | 단계 |
|---|---|---|
| `crates/engram-dashboard-agent/Cargo.toml` | `windows` feature 에 **`Win32_System_IO`** · **`Wdk_System_Threading`** · 주석 | ① |
| `…/platform/windows.rs` | **더함:** `pin_member(pid, kill)` · `PinnedMember{exited, facts, terminate_raw, classify, wait_exit}` · `ProcessFacts{ppid, create, image, cmdline}` · `watch_births(start)` · `unwatch_births` · `BirthPort{next}`. **걷음:** `verify_member` · `MemberCheck` · `VerifiedMember`(→ `PinnedMember` · 끝내기 두 메서드는 옮겨 산다). `member_pids`(HEAD `:136-218`) · `GetProcessTimes`(이제 생성 시각에 쓴다)는 그대로 | ① |
| `…/platform/process_group.rs` | `ProcessGroup::new(job, retiring)` · `member_pids` · `pin` · `watch_births` · `unwatch_births` · `retiring` · `Pinned` · `Births` · `RetiringSignal` · **걷음:** `verify` · `Verify` · `ReadyKill` · `root_attached()` · 시험 고쳐 쓰기 | ① |
| `…/platform/process_tree.rs` · `…/platform/mod.rs` | 1단계가 더한 셋과 시험 걷음 · 머리 doc | ① |
| `…/transport/mod.rs` | ★`AgentTransport::begin_retire(&self) {}`(기본 무동작) 더함★ | ① |
| `…/transport/stdio.rs` | `root_attached` 걷음 · ★`retiring: Arc<AtomicBool>` 칸 — `begin_retire()` 와 `shutdown()` 첫 줄이 세운다★ · `process_group()` = 약한 손잡이 + `RetiringSignal` · `InterruptLine` → `InterruptOut` · 시험(HEAD `:589` · `:615` · `:725` · `:768`) | ① |
| `…/transport/input_queue.rs` | 덩이마다 선택적 부를 것 — `push_with` · `pop` 이 쌍을 준다 · `drain` 이 쓰기 성공 · `mark_written` 뒤 **자기 `catch_unwind` 안에서** 부른다(패닉은 error 하고 계속 — §3-8) · 닫힘은 버린다 · 기존 시험(`:391` · `:410`) | ① |
| `…/transport/pty.rs` | **손대지 않는다** — `drain` · `push` 의 모양이 그대로이고 `pop` 을 부르지 않는다. `begin_retire` 는 기본 무동작 | — |
| `…/session.rs` · `…/manager.rs` | ★`Session::begin_retire()`(통로로 넘김) · `kill_agent`(HEAD `manager.rs:2884`)의 첫 줄(권한 회수 앞)과 `tear_down_failed_activation`(HEAD `:2340` — 화신 표식 대조 뒤 첫 줄)에서 부른다★ | ① |
| `…/backend/mod.rs` | `console_wrapper_depth` 와 시험 걷음 | ① |
| `…/backend/claude/leftover.rs` **신설**(백업에서) | 에피소드 · 여는 이 · 스냅숏 · 탄생 기록(듣는 스레드 · 기록 자물쇠) · 쓰기 확인 · 일꾼(패닉 가드) · 판(여러 차례) · 규칙 · 끝내기 확정 · 로그 | ② · ③ |
| `…/backend/claude/mod.rs` | 백업 배선(`GateSlot` · `open_spawn` · decoder · `interrupt_line` → `InterruptOut`) · `leftover_cleaner` · `TurnGate`(HEAD `:750-800` · 시험 `:5625`) 걷고 doc · `// ADR-0238` → `GateSlot` · `started` → `deliver` · HEAD 시험 `:6070-6137` 고쳐 씀 · `control` 주석 | ①(`interrupt_line` 반환 모양만) · ③ |

프론트 · 선 타입 · 프로토콜 · 데몬 · 셸 · `base` · `open_spawn` 시그니처 무변경. `base` 의 1단계 추가분(`ProcessStart` · `process_start` · `process_parent_table`)은 T-40 이 더는 부르지 않지만 기존 공개 함수의 몸통이라 남는다 — `process_creation_time` 을 `backend/codex/mod.rs:1096` · `codex/thread_lock.rs:1236` · `platform/file_holders.rs:190` · daemon `tests/ws_e2e.rs:2445` 가, `child_pids` 를 agent `tests/backend_contract.rs:1431` · daemon `tests/ws_e2e.rs:2469` 가 부른다(`rg` 2026-09-30).

---

## 3. 설계

### 3-0. 실측 — 이 설계가 기대는 사실

**끊기 스파이크(09-29 · `20260929-t40-spike/`):** 대시보드 스폰을 본뜬 하네스 · 실제 훅 · 보낸 뒤 200 ms 끊기 · claude 2.1.284 · 멈춤 3/11 · 끊기 +3 s 에 산 것은 멈춤 잔여물 하나(`Git\usr\bin\bash.exe <훅>.sh` · 깊이 5 · 부모 끝남 · +3 s 멤버 넷 — `main.log:92` `:98` · `pilot.log:26`) · +3 s 에 산 것은 11/11 에서 잔여물뿐 · 뿌리 자식 = conhost + claude(4/4) · `result` 는 보통 끊김과 같다. pilot 의 `followUp=TIMEOUT` 은 하네스 파싱 결함(`20260930-t40-provenance-spike/evidence-0929/pilot-p1.raw.log:72` → `:92`).

**MSYS 부모 스파이크(09-30):** 살아 도는 Git Bash 가 띄운 `sleep` 의 Windows 부모가 7 가지 중 5 가지에서 이미 끝난 fork — 「부모 죽음」은 가르개가 아니다.

**출처 스파이크(09-30 · `20260930-t40-provenance-spike/`):** 하네스가 `assign` 뒤 Job 에 완료 포트를 붙이고(★스폰 때 · 이 설계의 「첫 끊기에 붙임」은 재지 않았다★) 듣는 스레드 하나(동시 실행 수 1 · 50 ms 대기)가 가입 알림마다 곧바로 `PROCESS_QUERY_LIMITED_INFORMATION` 로 열어 부모(`NtQueryInformationProcess` 0) · 생성 시각 · 실행 파일 · 명령줄(60)을 읽고 핸들을 쥐었다(`probe/src/main.rs:373-390` · `:587-610` · `:213` · `:251-298`) — 기록 4322 개 · ★못 연 것 0 · 열었는데 Job 밖 0★(`main.log:49` `:99` `:155` `:201` `:246` `:285`).
- ★**다시 돌릴 수 있는 대조:** `r4sim.py`(같은 폴더 — 표 7 장의 taskkill 에피소드 35 개에 12판 규칙을 대 본다 · 생성 순서로 부모를 찾고 차례 3 번)가 주인 7 개 + p3bg 2 차례의 24776 만 고르고 다른 것은 고르지 않았다(라운드 4 끝내기 안전 리뷰가 돌렸다). 13판에서 더한 좁히기(taskkill 생성 ≥ 꼭대기 생성)는 그 출력의 `tk>=T` 칸이 보여 준다(8/8 참 — 2026-09-30 다시 돌림). 스크립트의 후보 경계는 「첫 taskkill − 5 ms 뒤 생성」이고, 14판의 쓰기 명단 W 는 그보다 이르거나 같은 자리(줄이 쓰인 직후)를 가른다 — 고른 8 개는 그 경계 뒤 262 ms 이상에 태어났다★.
- 멈춤 5/30 + 백그라운드 반복 중 2/4 · 주인은 늘 `"…\Git\usr\bin\bash.exe" C:/…/hooks/<x>.sh` · claude 까지 조상이 모두 끝남 · 이어 보낸 턴 9/9 답함. ★끝내기 → `result` 19–35 ms 는 **주인이 하나인 모양**의 값이다 — p3bg 는 주인(32228)이 +3 s 에 산 fork(24776 · +2756 → +4093 · 그 자식 27524 +3802)를 거느렸고, 하네스가 +10 s 에 끝냈을 때 `result` 는 364 ms 뒤였다(`main.log:145` · `procs/p3.procs.tsv`) — 주인 하나가 파이프의 유일한 주인은 아니다★.
- 훅 실행기 = `"…\Git\bin\bash.exe" -c "<hooks.json 명령>"`(맨 `.sh` 에 claude 가 `bash ` 를 붙인다) · Bash 도구 실행기(포그라운드 · 백그라운드 같음) = `-c "source …/.claude/shell-snapshots/snapshot-bash-… && … eval '<명령>' …"` — 291 대 6 · 모호 0. MSYS fork 는 부모의 명령줄을 가진다(1820/1820 · 43/43). 사건에는 PID 가 없다. 끊기 때 claude 는 훅 실행기마다 `taskkill /PID <실행기> /T /F`(두 차례) · Bash 도구 실행기에는 하지 않았다. 훅과 Bash 도구의 환경 차이(`CLAUDE_PROJECT_DIR` · `AI_AGENT`)는 PEB 읽기가 들어 안 쓴다.

**14판 규칙을 멈춤 10 번에 대면(추론 — 표에 규칙을 대 본 것 · 규칙을 돌려 보지는 않았다):**

| 멈춤 | 사슬(생성 = 줄이 쓰인 때 기준 · 판 = +3 s) | 알려짐 · 끝남 · 생성 순서 · 훅 사본 · taskkill | 결과 |
|---|---|---|---|
| 09-30 p1 · p2 · p4 · p5(`procs/p{1,2,4,5}`) | 주인(+263…+409) ← 훅 bash D(−80…−12) ← 껍데기 `-c "bash …"`(−132…−88) ← 실행기 T(−187…−172) | D 이상 스냅숏 · +3 s 전에 모두 끝남 · 부모가 늘 먼저 · 주인 = D 사본 · `taskkill /PID <T>` 2 건 | 끝낸다 4/4 |
| 09-30 p3t3(`procs/p3`) | 주인(+454) ← D fork(+279 · 250 ms) ← 훅 bash(−6) ← 껍데기 ← 실행기 | D = 탄생 기록 · 나머지 스냅숏 · 모두 끝남 · 주인 = D 사본 · taskkill 2 건 | 끝낸다 |
| 09-30 p3bg(`procs/p3`) | 주인 = 훅 bash(+1432) ← 껍데기(+872 · +1695 끝) ← 실행기(+75 · +1722 끝) · 주인의 fork 24776(+2756 · 산다) | 전부 탄생 기록 · 주인은 껍데기가 exec 한 스크립트(명령줄 다름 → `-c "bash …"` 갈래) · taskkill 2 건 · ★1 차례에 주인 → 끝남 확인 → 2 차례에 24776(부모 = 끝난 주인 · 사본)★ | 끝낸다(2 차례) |
| pilot p1bg(`procs/pilot-p1`) | 주인(+352) ← D 훅 bash(+16 · +389 끝) ← 껍데기(−40) ← 실행기(−100) | D = 탄생 기록 · 나머지 스냅숏 · 주인 = D 사본 · taskkill 2 건 | 끝낸다 |
| 09-29 main 2 · pilot 1 | 주인(+203…+244) ← 깊이 4 훅 bash(끊기 전) ← … | 스냅숏 · +3 s 멤버 넷 · 사본 · taskkill 은 09-30 의 모양으로 본다(표 없음) | 끝낸다 3/3(추론) |

→ **10/10**(표로 선 7 + 모양으로 본 3). 주인이 쓰기 명단 W 밖인 것(§3-2 — 주인은 줄이 쓰이고 263 ms 이상 뒤에 태어났고 W 는 쓰인 직후에 찍는다)과 생성 순서는 10 번 모두에서 선다. 같은 판의 다른 끊기 뒤 탄생(taskkill · 그 conhost · 반복의 `sleep`/fork · `cat`/`jq`)은 부모가 살아 있거나 사본이 아니거나 꼭대기가 훅 실행기가 아니라 빠진다. 실측 판정 = §7 G1.

### 3-1. 흐름 한눈에

```
kill_agent 시작 ─ session.begin_retire() → 통로 retiring = true(권한 회수 · Exiting · shutdown 보다 먼저)

Esc → manager → session → StdioTransport::interrupt → 끊기 줄 함수
  구간 ① — 문 닫힘 → None · 산 에피소드 → 이어 적음 · 여는 중(다른 Esc 가 열고 있음) → 줄만 준다
         · 그 밖 → 표식 + 「여는 이」 표시
  (여는 이만) 자물쇠 밖: 포트 · 듣는 스레드 확보(한 번 — 포트 → 스레드 기동 → Job 붙이기) → 기록 켬(R · 남은 기록은 놓은 뒤 버림) → 스냅숏
  구간 ② — 문 닫힘 · 턴 닫힘 → None · 기록 번호 ≠ R · 전달 · 에피소드 차례 바뀜 → 줄만(에피소드 없음) · 아니면 새 에피소드
  (여는 이 가드: ② 에 닿지 못한 모든 출구 · 에피소드를 안 세운 출구 → opening 내림 · 놓은 뒤 stop(R) · 스냅숏 버림)
  → InterruptOut { 줄, 쓰였을 때 = 멤버 번호 명단 W 찍기 → note_written(gen, W?) } → 큐
라이터: 쓴다 → (catch_unwind) W 찍기(아무 락 없이 · 완전하거나 오류) → note_written → W 가 있을 때만 첫 쓰임 · last_mono → 일꾼 기동
듣는 스레드: 가입 알림 + 기록 중 → 곧바로 열어 사실 → (pid, 생성) 이 새로우면 seq 로 기록

(멈춤) 쓰인 지 N 초 → 판(최대 3 차례 · 차례마다 확인 마감 200 ms 하나):
   후보 = 기록된 탄생 · 번호 ∉ W · 스냅숏 밖 · 산 것 → 규칙 ①–④ → 순번 순으로 끝내기(문 자물쇠 안 재확인 — 거절되면 판 전체를 멈춘다)
   → 끝남 확인 → 기록을 다시 복사해 다시 고른다(끝낸 것은 끝난 고리) → 없으면 멈춤
```

### 3-2. 기록 — 두 구간 · 여는 이 · 스냅숏 · 탄생 기록 · 쓰기 확인

```rust
pub(super) struct GateCell { state: Mutex<GateState>, cleaner: Option<Arc<Cleaner>> }
struct GateState { open: bool, closes: u64, delivers: u64, opening: bool, episode: Option<Episode>, next_gen: u64, worker: bool }
struct Episode { gen: u64, first_mono: Instant, last_mono: Instant, written: bool, w: Option<Arc<[u32]>> /* 쓰기 명단 W — 첫 쓰임 때 · 정렬 */,
                 snap: SnapState /* 인라인: Taken(Arc<Snapshot>) · Failed(사유) · Released */,
                 rec: u64, killed_any: bool, phase: Phase }
struct Mark { closes: u64, delivers: u64, next_gen: u64 }
// 기록 — 또 하나의 잎 · 문 자물쇠와 겹쳐 잡지 않는다 · 독을 견딘다
struct Recorder { active: AtomicU64 /* 0 = 없음 · 쓰기는 자물쇠 안 · 읽기는 어디서나 */, port_failed: AtomicBool, inner: Mutex<RecInner> }
struct RecInner { next_rec: u64, next_seq: u64, births: Vec<Arc<Birth>>, full: bool }
struct Birth { seq: u64, facts: ProcessFacts, pin: Box<dyn Pinned>, killable: bool }
```

- **끊기 줄 함수**(백업 `leftover.rs:889` 를 두 구간으로):
  1. 정리기가 없으면 오늘 그대로.
  2. **구간 ①** — 문 닫힘 → `None` · 산 에피소드 → `Cleaning` 이면 `Waiting` · 그 `gen` 으로 받아들임 · ★`opening` 이 참(다른 Esc 가 여는 중) → 줄만 준다(`on_written` 없음 — 여는 이의 에피소드가 그 턴을 맡는다)★ · 그 밖 → `opening = true` · `Mark`.
  3. **여는 이만** 자물쇠 밖에서: ⓐ 포트 · 듣는 스레드 확보(§3-5 — 포트 → 스레드 기동 → Job 붙이기 · `OnceLock` + 굳은 실패 표시 `port_failed` 를 함께 본다 → `Failed(port)`) ⓑ `R = recorder.start()`(`active = R` · `full` 초기화 · 남아 있던 기록은 꺼내 받아 **놓은 뒤** 버린다) ⓒ 스냅숏(아래). ★기록을 스냅숏 앞에 켠다 — 그 뒤 태어난 것은 기록되거나(명단 뒤) 명단에 든다(명단 앞) · 둘 다면 스냅숏 쪽★.
  4. **구간 ②** — `opening = false` · 문 닫힘 · `closes` 바뀜 → `None`(줄 없음) · ★`recorder.active` 원자 읽기 ≠ R · `delivers` · `next_gen` 바뀜 → **에피소드를 세우지 않고 줄만 준다**(`on_written` 없음 — 놓침 · debug `stale`)★ · 산 에피소드가 있음 → 여는 이가 하나라 닿지 않는 갈래(`debug_assert` + 로그 · 줄만 준다) · 그 밖 → 새 에피소드(찍은 것 · `rec = R` · `written = false` · `Waiting`).
  5. 놓은 뒤: 갈아 끼운 옛 에피소드의 스냅숏을 버리고 그 기록을 끈다 → `InterruptOut { bytes, on_written: Some(W 찍기 + note_written(gen, W)) }`.
  - ★**여는 이 가드(drop 가드)**★: 3 에 들어설 때 세우고, 구간 ② 가 새 에피소드를 세웠을 때만 해제한다. 해제되지 않은 모든 출구(패닉 · 이른 반환 · 줄 없음 · 줄만)에서 — 문 자물쇠를 잡아 `opening = false` 를 내리고 놓은 **뒤에** `recorder.stop(R)` · 찍다 만 스냅숏을 버린다.
- **스냅숏 찍기 [고름]:** `L = member_pids()`(완전할 때만) · `|L| > 256` → `Failed(too_many)` · 뿌리 ∉ L → `Failed(root)` · L 마다 `pin(pid, kill = false)` — 같은 핸들로 우리 Job 이면 붙들고 사실을 읽는다 · 뿌리 못 붙듦 → `Failed(root)` · 부모가 뿌리인 붙든 멤버가 둘 넘음 → `Failed(shape)`(뿌리 자식 = conhost · claude — Q2 · 추론).
- **탄생 기록(듣는 스레드) [고름 — 결정 5 · 6 · 7 · 11]:**
  - **한 알림:** `next(500 ms)` → 가입 알림이 아니면 버림(끝남 = 결정 6) · 가입이면 `active` 원자 읽기 → 0 이면 버림 → 아니면 **자물쇠 밖에서 곧바로** `pin(pid, kill = true)` · ★`Err` 이면 한 번 더 `pin(pid, kill = false)` → 붙들면 **고리 전용**(`killable = false` — 후보는 못 되고 사슬은 잇는다)★ · `Ok(None)`(사라짐 · Job 밖)이면 버림 → 기록 자물쇠 안에서 아직 같은 `active` 이고 ★`(pid, 생성 시각)` 이 새로우면★ `seq` 를 붙여 넣는다(512 넘으면 `full` · 넣지 않음) → 못 넣은 것은 놓은 뒤 버린다.
  - **시작:** 뜬 스레드는 먼저 여는 이의 「붙었나」를 받는다 — `false` 면 곧바로 끝난다(§3-5).
  - **매 차례 · 끝:** 매 알림 · 시간 초과마다 약한 Job 손잡이를 잠깐 올려 살아 있는지 · 종료 표식을 본다(★올린 `Arc` 를 `next` 에 들고 들어가지 않는다★) → 사라졌거나 섰으면 `stop(active)` 로 기록을 놓고 포트를 닫고 끝난다.
  - ★**실패 모드**★: `next` 가 회복할 수 없는 오류이거나 한 차례의 몸통이 패닉하면(차례마다 `catch_unwind`) → `port_failed = true`(굳음 — 이후 에피소드는 `Failed(port)`) · `stop(active)` · 로그 한 번(오류 → warn · 패닉 → error(§3-8)) · `unwatch_births()`. 떼기가 성공하면 끝난다. ★떼기가 실패하면 끝나지 않고 **꺼내 버리기만** 계속한다(기록 안 함 · 약한 손잡이 · 종료 표식을 볼 때까지 — 화신 수명으로 묶인다) — 포트가 붙은 채 아무도 안 꺼내면 커널 큐가 커지기 때문이다(라운드 4 codex)★.
  - **왜 곧바로 여나:** 알림의 PID 는 열어 쥐지 않으면 산 것도 재사용 안 된 것도 보장되지 않는다(MS — §3-5). 곧바로 열어 쥐면 그 뒤로 재사용되지 않고 끝난 부모의 사실도 남는다(하네스 0/4322 놓침 · 가장 짧게 산 중간 고리 = p3t3 의 D 250 ms).
  - **켬 · 끔:** `start()` · `stop(R)` — 끄는 자리 = 판 완료 · 턴 끝 · 새 입력 · 종료 표식 걸음 · 일꾼 기동 실패 · 패닉 가드 · 갈아 끼우기 · 여는 이 가드 · 듣는 스레드의 끝 · 실패 모드. 모두 문 자물쇠를 놓은 뒤 · 꺼낸 기록은 기록 자물쇠를 놓은 뒤 버린다. 판의 복사는 `active == rec` 가 아니면 빈 것을 준다.
- ★**쓰기 확인 — 쓰기 명단 W(확인 라운드 codex BLOCK #1)**★: 라이터가 줄을 쓰고(성공 · `mark_written` 뒤 · 아무 락 없이 · 자기 `catch_unwind` 안) 부를 것을 부른다 → ⓐ 약한 Job 손잡이를 잠깐 올려 **`member_pids()` 를 한 번** 찍고(완전하거나 오류 — 1단계 그대로) 정렬해 `W` 로 만든다(올린 `Arc` 는 곧바로 놓는다) ⓑ `note_written(gen, W 또는 실패)` — 문 자물쇠 안에서 같은 `gen` 이고 `Waiting`/`Cleaning` 이면 `last_mono = max(…, now)` · ★첫 쓰임은 W 가 있을 때만 `written = true` · `w = W`(명단 실패 · 손잡이 사라짐 → 그 쓰기는 첫 쓰임이 되지 못한다 · warn — 같은 에피소드의 뒤 끊기가 W 와 함께 쓰이면 그것이 첫 쓰임이 되고, 그때까지 판이 없다 · 착지 고침 2026-10-01)★ · 첫 쓰임이 아니면 W 를 돌려받아 놓은 뒤 버린다 · 일꾼 몫 → 놓은 뒤 일꾼 기동.
  - ★**왜 안전한가 — 알림이 언제 쌓이는지에 기대지 않는다**★: 후보 C 는 탄생 기록의 핸들로 붙든 것이고 판 때 살아 있다. C 가 W 를 찍기 전에 가입했다면 W 를 찍을 때도 살아 있었고(끝난 프로세스는 되살지 않는다) 명단은 완전하므로 **C 의 번호가 W 에 든다 → 후보가 아니다**. 그래서 후보는 모두 W 뒤 — 곧 줄이 쓰인 뒤 — 에 가입한 것이다. 번호로만 비교하므로 W 에 있던 번호가 끝나고 쓰인 뒤 탄생이 그 번호를 다시 쓰면 빠진다(놓침 · 안전 쪽). 기대는 것 = `member_pids` 의 완전성(1단계 — 할당 수와 목록 수가 다르면 오류) · 한 번 든 프로세스는 Job 에서 빠지지 않는다(MS 문서) · 붙든 핸들의 끝남 판정뿐이다.
  - **알림이 쌓이는 때 · 꺼내는 순서는 효과에만 든다:** 늦게 쌓이거나 늦게 꺼내져도 후보 판정은 그대로 — 짧게 산 고리를 열기 전에 놓칠 수 있을 뿐이다(§8 ③).
  - ★**놓침 창 — 쓰기와 W 사이**★: 줄이 쓰인 뒤 W 를 찍기 전에 태어나 W 때 살아 있던 것은 W 에 들어 **빠진다**(놓침). 틈 = 라이터가 쓰기에서 돌아와 명단 한 번 찍는 시간(짧다 — 추정 · G1 이 쓰기 → W 지연을 잰다). 잰 여유: 주인은 쓰인 뒤 263–454 ms(p3bg 주인 1.4 s · 그 fork 2.8 s — §3-0 표) · 훅 fork 는 taskkill 보다 1 ms 이상 늦고 taskkill 은 claude 가 줄을 읽은 뒤 띄운다(메인 측정). ★13판의 「흐림이 양쪽 다 사라진다」는 틀렸다 — 이 놓침 쪽 틈이 남는다★.
  - **후보 = 번호 ∉ W 인 탄생뿐** — W 에 든 것 · W 를 찍기 전에 끝난 것은 고리로만 쓴다. 쓰이지 않은 줄의 에피소드는 판이 없다. 같은 에피소드의 뒤 끊기의 부를 것도 W 를 찍지만 `note_written` 이 버린다(첫 쓰임 것 하나 — 뒤 끊기 사이의 탄생은 여전히 후보가 될 수 있다).
- **decoder:** 턴 열림(닫힘 → 열림일 때만) · 새 입력(라이브 `started` — 한 구간에서 에피소드를 버리고 `delivers += 1` · 닫혀 있었으면 연다 · B2 실측 claude 2.1.280) · 턴 끝(`open = false` · `episode = None` · `closes += 1`).
- **불변식:** 「산 에피소드면 문이 열려 있다」 · 「`written` 이면 `w` 가 있다」 · 「쓰인 `Waiting`/`Cleaning` 또는 `Spent` 이면 일꾼이 떠 있다」 · 「`Taken` 스냅숏 · 켜진 기록을 가진 에피소드는 `Waiting`/`Cleaning`」 · 「`opening` 은 여는 이의 두 구간 사이에만 참」 · 「`active` 가 켜져 있으면 그것을 쥔 산 에피소드가 있거나 여는 이가 일하는 중이다」.

### 3-3. N 초 확인 · 물러남 · 자물쇠

- **상수:** `INTERRUPT_LEFTOVER_GRACE = 3 s` · `PIN_MAX = 256` · `BIRTH_MAX = 512` · `ANCESTOR_MAX = 16` · `PASS_ROUNDS = 3` · `KILL_CONFIRM = 200 ms` · 듣는 대기 500 ms.
- ★**물러남 시작 [고름 — 라운드 3 codex · 두 번째 지적]**★: `AgentTransport::begin_retire(&self)`(기본 무동작 — pty · codex 통로는 그대로)를 더하고 `StdioTransport` 는 자기 `retiring` 칸(`Arc<AtomicBool>`)을 세운다 · `shutdown()` 첫 줄도 세운다(다른 끝내기 길). `Session::begin_retire()` 가 통로로 넘기고, **`kill_agent`(HEAD `manager.rs:2884`)는 권한 회수 · 의도 · `Exiting` · `session.kill` 보다 먼저**, `tear_down_failed_activation`(HEAD `:2340`)은 화신 표식 대조 뒤 첫 줄에서 부른다(그 밖의 끝내기 길은 `shutdown()` 첫 줄이 세운다). 칸의 주인은 통로 · manager 는 중립 메서드만 부른다 · backend 는 `ProcessGroup::retiring()` 이 주는 읽기 전용 `RetiringSignal`(`is_set()` 만)로 읽는다 — manager 는 backend 의 문을 모른다. 남는 겹침 = 재확인이 표식을 읽은 직후의 끝내기 한 번(§8 ⑲).
- **한 걸음 `next_step(now, retiring)`:** 표식 → 에피소드를 치우고 끝 · 없음 · `Done` · 쓰이지 않은 `Waiting` → 끝 · 쓰인 `Waiting` 전 → 잔다 · 지남 → `Cleaning` + `Ticket` → 판 · `Spent` 전 → 잔다 · 지남 → `Done` + warn(결정 2).
- ★**판의 차례(라운드 3 끝내기 안전 F3) [고름]**★: 한 차례 = 기록 복사(기록 자물쇠 안 `Arc` 복제 · `active == rec` 가 아니면 빈 것) → 후보 · 규칙(§3-4) → `Cleanup` 을 **`seq` 오름차순**(먼저 태어난 조상부터)으로 끝내기 확정 → 그 차례에 끝낸 것들을 ★**하나의 마감(200 ms)** 안에서★ 그 핸들로 끝남 확인(`wait_exit(남은 시간)`) → 다음 차례. 끝낸 것은 다음 차례에서 「끝난 알려진 고리」로 보여, 그것이 거느리던 사본(p3bg 의 24776)이 후보가 된다. `Cleanup` 이 없으면 멈춘다 · **최대 3 차례** — 잰 가장 긴 것이 2 차례(p3bg: 주인 → 그 fork)이고 한 차례를 여유로 둔다(넘으면 warn `rounds_exhausted`). 마감 안에 안 끝난 것은 다음 차례에서 산 것으로 보여 그 아래는 빠진다(놓침).
  - ★**재확인이 거절되면 판 전체를 멈춘다**★(턴 끝 · 새 끊기 · 새 입력 · 종료 표식으로 `still_due` 거짓 — 남은 후보와 다음 차례를 버린다 · debug `superseded`).
  - **판 시간의 상한** = 3 차례 × (확인 마감 200 ms) + 고르기 · 복사 · 끝내기 호출 — 기다림은 판마다 많아야 0.6 s 다.
- **끝내기 확정(5판 그대로):** 문 자물쇠 안에서 단조 시각 · 표식을 읽어 `still_due` = 표식 없음 · 같은 세대 · 쓰임 · `Cleaning` · 쓰임 + N ≤ now → 참이면 탄생 기록의 핸들로 `terminate_raw` 하나(두 시각 읽기로 감쌈) · `note_kill` → 놓은 뒤 가르기 · 확인 · 로그.
- **판 끝:** `Spent`/`Done` · 스냅숏 `Released` · 기록 끔 — 돌려받은 것과 `Ticket` 은 자물쇠를 놓은 뒤 버린다. 완료된 판은 한 번.
- **패닉 가드:** 되감기 → `worker = false` · `Done` · 스냅숏 · 기록 돌려받음 → 놓은 뒤 error(§3-8) · 정상 종료는 가드 해제.
- ★**자물쇠 규칙(정본 — CLAUDE.md 락 순서 줄이 옮긴다)**★
  - **문 자물쇠 `GateCell.state` 는 잎이다: 쥔 채 로그 · 스레드 기동 · emit · 잠 · 다른 자물쇠 · 할당과 해제 · 명단 조회 · 붙들기와 핸들 닫기 · 그 밖의 OS 호출을 하지 않으며, 예외는 ⓐ 단조 시각 읽기 ⓑ 끝내기 확정의 `TerminateProcess` 호출 정확히 하나뿐이다. 원자 읽기(종료 표식 · `recorder.active`)는 허용된다. 스냅숏 · 기록 번호는 밖에서 만들어 옮겨 넣고, 치운 것과 판의 표는 놓은 뒤 버린다.**
  - **기록 자물쇠 `Recorder.inner` 도 잎이다: 쥔 채 필드 · `Vec` 넣기 · `Arc` 복제 · 꺼내기(할당 허용)만 하고, OS 호출 · 핸들 닫기(`Arc<Birth>` 의 마지막 drop) · 로그 · 다른 자물쇠는 하지 않는다. 문 자물쇠와 겹쳐 잡지 않는다. 독을 견딘다.**
  - 잡는 자리: 문 = 끊기 줄 함수 · 쓰기 확인(라이터 — 아무 락 없이 부르고 · W 찍기는 락 밖 · `note_written` 만 문) · decoder · 일꾼 · 판 끝 · 확정 · 패닉 가드 · 여는 이 가드 / 기록 = 듣는 스레드 · `start` · `stop` · 판의 복사.

### 3-4. 고르기 — 규칙

**판의 순서 [고름] — 하나라도 서지 않으면 아무것도 안 끝낸다:** 1 표식 없음(debug `retiring`) → 2 스냅숏 `Taken` 이고 기록이 살아 있음(`active == rec` — 아니면 듣는 스레드가 실패해 기록을 놓은 것)(warn `snapshot_failed`(`cause` = list · root · too_many · shape · port)) → 3 붙든 뿌리가 산다(debug `root_gone` · warn `root_unknown`) → 4 차례들(§3-3) — 후보 0(첫 차례) → warn `no_new_member`(`births` · `full`) · 모두 빠짐 → warn `parent_rule`(규칙별 수) → 5 끝내기 전마다 뿌리 · 표식 재확인(밖) → 6 확정 · 확인(오류 warn · 하나도 못 끝냄 warn `all_failed`).

**후보** = 이 기록(`rec`)의 탄생 중 ★번호 ∉ 쓰기 명단 W(§3-2 — 쓰기 전 탄생을 막는 벽)★ · `killable` · 그 (pid, 생성)이 스냅숏에 없음 · 붙든 핸들이 끝나지 않음.

**규칙(후보 C · 기록된 부모 D). 알려진 고리 = 스냅숏의 붙든 멤버 또는 같은 기록의 탄생. ★모든 고리에서 부모의 생성 시각 ≤ 자식의 생성 시각(붙든 핸들로 읽은 두 커널 도장)이어야 한다 — 아니면 그 고리는 알려지지 않은 것이다★:**

1. **부모(결정 11 · 4):** D 가 알려진 고리가 아니면(유실 · 제때 못 엶 · 순서 어긋남 · 0) `parent_unknown` · D 산다 → `parent_alive` · 오류 → `unknown` · 끝남 → 다음.
2. **훅 사본(결정 10):** C 와 D 의 실행 파일 경로가 같고 ⓘ C 의 명령줄 == D 의 명령줄(fork 사본 — 1820/1820) 또는 ⓘⓘ `is_hook_command(D)`(C 는 그 껍데기가 exec 한 스크립트 — p3bg) → 다음 · 아니면 `not_hook_copy` · 빈 칸 → `unknown`. exec 된 프로그램(`sleep` · `cat` · 서버 · 빌드 자식)은 빠진다.
3. **조상(결정 9 · 11):** `A₁ = D` 에서 적어 둔 부모를 따라 걷는다 · `parent(Aₖ) == 뿌리` 인 `Aₖ`(claude)에서 멈추고 판단하지 않는다 · `A₁ … Aₖ₋₁` 은 알려진 고리이고(생성 순서 포함) 끝났어야 한다 — 산다 → `ancestor_alive` · 알려지지 않음 · 0 · 순환 · 16 초과(`A₁ … Aₖ₋₁` 의 수 — 착지 고침) · k = 1 · 뿌리에 먼저 닿음 → `chain_cut` · 오류 → `unknown`. 꼭대기 **T = Aₖ₋₁**.
4. **훅 사슬(결정 10):** T 의 실행 파일 이름이 `bash.exe` 이고 `is_hook_command(T)` 이며 ★이 기록에 **부모가 claude(= `Aₖ`)이고 실행 파일이 `taskkill.exe` 이고 프로그램 뒤 인자가 정확히 `/PID <T 의 PID> /T /F`(순서 · 대소문자 그대로 · 더 붙은 인자 없음 — 착지 고침)이고 생성이 T 의 생성 이상인** 탄생이 있다★(그 taskkill 이 T 를 가리키는 고리도 부모 · 자식 순서를 지킨다 — 번호 재사용으로 옛 T 를 가리키던 taskkill 을 걸러 낸다) → 끝낼 후보 · 아니면 `not_hook`(taskkill 이 없으면 `no_taskkill`).
   - **`is_hook_command(f)`** = 명령줄이 「첫 토큰 · `-c` · 인자 S」 모양이고 S 가 `bash ` 로 시작하며 `shell-snapshots` 도 줄바꿈(CR · LF)도 담지 않는다(줄바꿈 = 착지 고침 · 양성 · claude 2.1.284 의 `.sh` 훅 감싸기 · 291 대 6 · 음성 표지는 좁히기만).
   - **taskkill 표지:** claude 는 끊기 때 돌던 훅 실행기마다 `taskkill /PID <실행기> /T /F` 를 두 차례 띄운다(WRITEUP B · 표가 있는 멈춤 7/7 에서 꼭대기마다 2 건 — `procs/*.procs.tsv` 대조 2026-09-30). Bash 도구 실행기에는 띄우지 않았다. 양성 · **좁히기만** 한다 — claude 가 끊기 방식을 바꾸면 모두 놓친다(안전).
- **고리마다 훅 사본 검사(선택 — 채택 안 함):** 껍데기 → 실행기 고리는 실행 파일이 달라(`usr\bin` 대 `bin`) 셋째 모양이 필요하고, 규칙 2 · 4 · taskkill 이 사슬을 이미 훅 나무로 묶는다 — 얻는 것이 없다.
- **claude · 래퍼 · 콘솔 호스트:** 스냅숏에 있어 후보가 못 된다 · claude 의 직계 자식(taskkill · 끊기 뒤 뜬 훅 실행기)은 부모 claude 가 살아 1 에서 빠진다 · 뿌리 생존 = claude 생존(`cmd /c` · Q2) · 런처가 끼면(뿌리 → 런처 → claude) 걷기가 런처에서 멈추므로 claude 가 고리가 된다 — claude 가 살아 있으면 `AncestorAlive{claude}` · 끝났으면 T = claude.exe → `not_hook`(어느 쪽이든 모두 놓침 · 안전 · 착지 고침 2026-10-01).
- **모두 AND 거르개**(조사 §7 「고아 여부는 주 기준이 아니다」).

```rust
fn select(c: &Link, esc: &Chain) -> Verdict;   // 순수 — Chain = 스냅숏 + 기록을 (pid, 생성)으로 찾는 보기 · 「끝났나」는 주입
fn is_hook_command(f: &ProcessFacts) -> bool;
fn taskkill_names(f: &ProcessFacts, claude: u32) -> Option<u32>;   // `/PID n /T /F` 의 n
enum Verdict { Cleanup { chain: Vec<u32> }, ParentAlive, ParentUnknown, NotHookCopy, AncestorAlive { pid: u32 }, ChainCut, NotHook, NoTaskkill, Unknown }
```

### 3-5. OS 조각 — 명단 + 붙들기 + 사실 + 가입 알림

```rust
pub(crate) struct ProcessFacts { pub ppid: u32, pub create: u64, pub image: String, pub cmdline: String }
impl JobObjectHandle {
    pub(crate) fn member_pids(&self) -> io::Result<Vec<u32>>;                              // HEAD 그대로
    pub(crate) fn pin_member(&self, pid: u32, kill: bool) -> io::Result<Option<PinnedMember>>;
    pub(crate) fn watch_births(&self, start: impl FnOnce(Arc<BirthPort>) -> io::Result<()>) -> io::Result<()>;
        // 포트를 만든다 → start(포트) = 듣는 스레드 기동 → 그것이 Ok 일 때만 Job 을 붙인다 · 두 번 부르지 않는 것은 정리기의 OnceLock 이 지킨다
    pub(crate) fn unwatch_births(&self) -> io::Result<()>;        // 포트 연결을 뗀다
}
impl PinnedMember { fn exited(&self) -> io::Result<bool>; fn wait_exit(&self, d: Duration) -> io::Result<bool>; fn facts(&self) -> &ProcessFacts;
                    fn terminate_raw(&self) -> io::Result<()>; fn classify(&self, raw: io::Result<()>) -> io::Result<MemberOutcome>; }
impl BirthPort {                                   // Send + Sync — 듣는 스레드와 watch_births(붙이는 동안)가 Arc 로 쥔다 · 마지막 drop = 포트 닫기
    fn next(&self, wait: Duration) -> io::Result<PortEvent>;   // Joined(pid) · Other · Timeout
}
```

- **붙들기 · 사실:** `OpenProcess(QUERY_LIMITED | SYNCHRONIZE [| TERMINATE])` → 87 이면 `Ok(None)` → `IsProcessInJob(h, 우리 Job)`(NULL 금지) 거짓이면 `Ok(None)` → 사실 한 번: 부모 = `NtQueryInformationProcess(h, ProcessBasicInformation(0))` 의 `InheritedFromUniqueProcessId`(결과 구조체는 이 파일의 `repr(C)` — `Win32_System_Kernel` 을 켜지 않는다 · 하네스 `probe/src/main.rs:251-275`) · 생성 = `GetProcessTimes(h)` 의 생성 FILETIME(HEAD 가 이미 쓰는 함수 · 같은 핸들이라 번호가 아니라 그 프로세스의 것) · 명령줄 = `ProcessCommandLineInformation(60)`(64 KiB) · 실행 파일 = `QueryFullProcessImageNameW`. 못 읽은 칸은 빈 칸 · 생성 0(그 고리는 순서 검사를 못 서 알려지지 않은 것으로 친다). 끝났나 = `WaitForSingleObject(h, 0)` · 확인 = `WaitForSingleObject(h, 200 ms)`. 끝내기 = HEAD 의 `terminate_raw` · `LEFTOVER_EXIT_CODE = 0x7440`.
- **가입 알림 포트:** `CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, 1)` → `SetInformationJobObject(job, JobObjectAssociateCompletionPortInformation, {CompletionKey: 1, CompletionPort})`(하네스 `probe/src/main.rs:373-390` 와 같은 호출) · `next` = `GetQueuedCompletionStatus` → 키 1 · 값 6 이면 `Joined(ov as u32)`(역참조 안 함) · 그 밖 `Other` · `WAIT_TIMEOUT` → `Timeout` · 그 밖 거짓 → `Err`. `watch_births` 는 포트를 만들어 `Arc` 하나를 `start` 에 넘기고 제 것은 붙이기가 끝날 때까지 쥔다(그 사이 스레드가 끝나도 포트 핸들 값이 재사용되지 않는다). `unwatch_births` = 같은 연결 호출에 `CompletionPort = NULL`(MS Learn 이 적은 떼기). 상수 `JOB_OBJECT_MSG_NEW_PROCESS = 6`(`Win32_System_SystemServices` 를 켜지 않는다).
- ★**정리기의 포트 확보 — 꺼내는 이가 먼저(확인 라운드 codex BLOCK #2)**★: 화신마다 `OnceLock` 하나 — 처음 새 에피소드를 여는 이가 `(tx, rx)` 한 쌍을 만들고 `watch_births(|port| 듣는 스레드 기동(port, rx, …))` 를 부른 뒤 그 결과를 `tx.send(붙었나)` 로 알린다. 순서는 API 가 강제한다: **포트 → 스레드 기동(포트를 쥐고 `rx` 를 기다린다) → 기동이 `Ok` 일 때만 Job 붙이기**. 그래서 알림이 쌓일 수 있게 되는 순간에는 꺼내 줄 스레드가 이미 떠 있다.
  - 스레드 기동 실패 → **붙이지 않는다** → 굳음(`Failed(port)`) — 쌓일 알림이 없다.
  - 붙이기 실패 → 굳음 · 스레드는 `false` 를 받고 끝난다(붙은 적이 없으니 뗄 것도 없다).
  - 스레드는 `false` 에서만 끝난다 — `true` 이거나 보내는 쪽이 사라졌으면(여는 이가 붙인 뒤 보내기 전에 패닉 — 붙었을 수 있다) 평소대로 돈다(붙지 않았다면 약한 손잡이 · 표식을 볼 때까지 시간 초과만 돈다 — 화신 수명).
  - 두 번째 `watch_births` 는 코드가 막는다(OS 의 거절에 기대지 않는다). ★굳히는 것은 진짜 OS 실패뿐이고 굳은 실패는 `Recorder.port_failed` 원자 칸이다(여는 이의 ⓐ 가 `OnceLock` 값과 함께 본다)★ · 듣는 스레드의 실패 모드 → 굳음(§3-2) · 통로가 이미 사라짐(약한 손잡이)은 굳히지 않는다.
- **MS Learn 이 말하는 것(그대로 기댄다):**
  - 「Processes added to a job at the time a completion port is associated are also reported.」 — ★첫 에피소드에서 기존 멤버가 다시 알려진다(되알림)★ → 살아 있으면 W 에 들어 고리 전용이고 스냅숏의 (pid, 생성)과 같으면 스냅숏 쪽 — 단 그 수만큼 `BIRTH_MAX` 칸과 꺼낼 거리를 먹는다(§8 ㉒ ⓑ · G1 이 잰다).
  - 「it is best to associate a completion port with a job when the job is inactive … you may miss messages for processes whose states change during the association」 — 붙이는 순간의 탄생은 놓칠 수 있다 → 놓침.
  - 「messages are intended only as notifications and their delivery to the completion port is not guaranteed」 — 빠진 가입 알림 → 알려지지 않은 고리 · 후보 아님 → **놓침(오살 아님)**.
  - 「you cannot guarantee that this process is still active or that the identifier has not been recycled … unless you maintain an open handle」 — 곧바로 열어 쥔다 · 열기 전 재사용은 생성 순서가 거른다(§3-7).
  - 「If the job is nested, the message is sent to every I/O completion port associated with any job in the parent job chain」 — 다시 알려지면 (pid, 생성)이 같아 한 번만 기록된다.
  - 포트를 `NULL` 로 주면 연결이 풀린다(Windows 8 이상 — 문서화된 떼기).
- **중립 손잡이 · 통로:** `ProcessGroup { job: Weak (cfg windows), retiring: RetiringSignal }` · `member_pids` · `pin` · `watch_births` · `unwatch_births` · `retiring` · 비Windows 껍데기 = 명단 `Ok(빈)` · 붙들기 `Ok(None)` · 포트 `Err(Unsupported)`. `process_group()` = `ProcessGroup::new(Arc::downgrade(&job), RetiringSignal::of(&self.retiring))`. 붙든 핸들 · 포트 · 표식은 Job 을 붙들지 않는다(듣는 스레드가 매 차례 잠깐 올리는 것 말고 — §8 ⑲).

### 3-6. 시계

벽시계와의 비교 · 「Esc 뒤에 태어났나」를 시각으로 가르기를 쓰지 않는다(결정 5). 「끊기 뒤 탄생」 = 기록을 켠 뒤 온 가입 알림(후보는 쓰기 명단 W 밖) · 「끊기 전부터」 = 스냅숏 · W. 커널 생성 시각은 **한 고리의 두 끝(부모 · 자식)을 서로 비교**하는 데와 규칙 4 의 taskkill ≥ T 에만 쓴다 — [고름] 이고 사용자 결정이 아니다(결정 5 는 Esc 와 시각을 비교하지 않는다는 것만 정했다). 시계가 두 생성 사이에 뒤로 뛰면 참 부모가 「자식보다 늦게」로 읽혀 **놓친다**(안전 쪽). 단조 시각은 N · `waited_ms` · `terminate_max_us` · 확인 대기에만.

### 3-7. PID 재사용 · 조사 §4 사고 규칙 대조

| 틈 | 막는 것 |
|---|---|
| 후보가 우리 것인가 | 우리 Job 의 가입 알림에서만 · 연 핸들로 `IsProcessInJob(우리 Job)` · 부모 연결 · W 는 줄이기만 |
| 알림 뒤 ~ 열기 사이 재사용 | 곧바로 연다(하네스 0/4322) · Job 밖이면 버린다 · Job 안의 다른 새 프로세스면 그 자신의 사실로 기록된다 |
| ★기록된 부모 번호가 엉뚱한 프로세스를 가리킴(사슬 이어 붙이기)★ | 붙든 번호는 붙든 뒤 재사용되지 않는다. 붙들기 **전**의 재사용(스냅숏 전에 끝난 조상 · 열기 전에 끝난 부모)이면 그 번호의 새 주인은 끝난 참 부모보다 — 곧 자식보다 — 늦게 태어났다 → 보통은 **부모 생성 > 자식 생성** 이라 그 고리는 알려지지 않은 것 → 놓침. ★검사는 「부모 ≤ 자식」이라 **같은 눈금**(재사용이 자식 생성과 같은 시각 눈금 안에 일어남)이면 통과한다 — 「엄격히 늦게」가 아니다(ADR-0257 결정 9 「같은 눈금 포함」 · 착지 고침 2026-10-01 · §8 ㉔ K4)★. 남는 것 = 그 같은 눈금 · 두 생성 사이에 시계가 뒤로 뛰어 순서가 뒤집히는 경우(§8 ⑰ ⓑ) |
| 연 뒤 ~ 끝내기 | 같은 핸들(기록 때 끝내기 권한까지) |
| 끊기 뒤 태어나 판 전에 죽은 중간 고리 | 탄생 기록이 붙들어 사실을 남긴다 |
| 재확인과 끝내기 사이 턴 끝 · 새 끊기 · 새 입력 | 문 자물쇠 한 구간 |
| 통로 종료 | `kill_agent` 시작에서 표식이 선다 · 재확인이 읽는다 · 직후 겹침은 해가 없다 |
| taskkill 이 옛 꼭대기를 가리킴 | taskkill 생성 ≥ T 생성이어야 한다 — T 의 번호를 재사용한 뒤의 T 를 옛 taskkill 이 가리키는 모양을 거른다 |
| 두 Esc 가 함께 에피소드를 열려 함 | 여는 이는 하나(`opening` · drop 가드) · 기록 번호 대조(`active == R` — 아니면 에피소드 없이 줄만) |
| 줄이 쓰이기 전 / 뒤의 가입 | ★쓰기 명단 W(§3-2)★ — 쓰기 전에 가입해 판 때 산 것은 W 에 든다(명단 완전성 · 되살지 않음) · W 의 번호를 쓰인 뒤 탄생이 재사용하면 빠진다(놓침) · 쓰기와 W 사이 탄생도 빠진다(놓침) · 명단 실패면 그 쓰기는 첫 쓰임이 못 된다(뒤 끊기의 W 가 첫 쓰임이 될 때까지 판이 없다). 알림이 쌓이는 때에는 기대지 않는다 |
| 알림 유실 · 붙이는 순간 | 알려지지 않은 고리 → 놓침 |
| claude · 래퍼 · 콘솔 호스트 | 스냅숏에 있다 · 걷기는 claude 에서 멈춘다 · 뿌리 자식이 둘 넘으면 `Failed(shape)` |

### 3-8. 로그

에이전트 귀속 = `control.map(|c| c.agent_id)`(ADR-0217) · 없으면 `root_pid`. 전부 자물쇠를 놓은 뒤.

| 사건 | 레벨 | 필드 |
|---|---|---|
| 정리함 | **warn** | `agent` · `root_pid` · `waited_ms` · `terminated`(PID · 부모 · `chain` · 차례) · `rounds` · `terminate_max_us` · `unconfirmed` · `births` · 규칙별 뺀 수 |
| 못 끝냄(§3-4 사유 · `rounds_exhausted`) | **warn**(`reason`) | 사유별 값 |
| 포트 · 듣는 스레드 실패(기동 실패 — 안 붙임 · 붙이기 실패 · 떼었음 · 못 떼어 버리기만 함) · 쓰기 명단 실패 | warn — 한 번 | `agent` · `: {e}` |
| ★잡힌 패닉 — 듣는 스레드 · 쓰기 확인 부를 것 · 일꾼★ | ★**error**★(로깅 규약 — 격리돼도 패닉은 error · 착지 고침 2026-10-01 · 14판은 warn 이라 적었다) · ★릴리스는 `panic = "abort"` 라 잡히는 것은 디버그 · 시험뿐이다★ | `agent` · 메시지 |
| 끝내기 오류 · 정리 뒤 N 초에도 같음 · 일꾼 기동 실패 | warn | `agent` · … |
| 에피소드 열림(첫 에피소드의 되알림 수 포함) · 쓰였다(W 크기 · 쓰기 → W 지연 µs) · 가입 → 엶 지연 · 줄 없음 · `stale`(줄만) · 여는 중에 온 끊기 · 기록 넘침 · `retiring` · `delivered` · `superseded`(판 멈춤) | debug | `agent` · 사유 |

G2 는 `resolved_pid`(`session_file.rs:171-180`)가 `terminated` 에 없는지 본다.

### 3-9. 영향이 없는 것

codex · 터미널 모드(`begin_retire` 기본 무동작) · 프론트 · 선 타입 · 버스(결정 1) · 입력 큐의 사용자 덩이(부를 것 없음) · pty 통로 코드 · 턴 관측 두 지점(ADR-0127) — 무변경.

---

## 4. 불변식 대조

| 원칙 | 이 설계에서 |
|---|---|
| **코어 격리**(ADR-0003) | 전부 `agent` · tauri 0 |
| **백엔드 확장**(ADR-0004) | claude 지식(끊기 줄 · 턴 끝 · `started` · 훅 실행기 · taskkill 모양 · 무엇을 기록 · 정리하나)은 `backend/claude` 에만. 통로는 약한 손잡이 · 읽기 전용 표식 · 「쓰였을 때 부를 것」 · 중립 `begin_retire` 만 · manager 는 `begin_retire` 를 부를 뿐 backend 를 모른다 |
| **플랫폼 중립**(ADR-0230) | OS 갈래 = `platform/windows.rs` · `process_group.rs` · 통로 접근자 · 규칙은 순수 · 부르는 쪽 `cfg!(windows)` 없음 |
| **락 순서**(ADR-0006 · ADR-0231) | 새 잎 둘(문 · 기록) — 겹쳐 잡지 않는다 · 쓰기 확인 · 듣는 스레드 · `begin_retire` 는 아무 락 없이 부른다(`kill_agent` 의 첫 줄은 명부 락을 이미 놓았다) → 기존 락과 간선 없음 |
| **소유권 분할** | Job · `shutdown` · `retiring` 칸은 transport — 밖으로는 `Weak` · 읽기 전용 표식 · 한 번 붙이는 포트. backend 가 에피소드 동안 명단 · 멤버 · 탄생을 붙들고 하나씩 끝낸다 → CLAUDE.md 소유권 줄(④). kill 인과(ADR-0001) 무변경 — `begin_retire` 는 칸 하나를 세울 뿐 |
| **ADR-0238** | `TurnGate` → `GateCell`(결정 3) · 끊기 기록(결정 5) · `InterruptOut`(결정 2) · 여닫기 · 거절 그대로 · 구간 사이 턴 닫힘이면 줄 없음 |
| **바닥 crate**(ADR-0175) | `base` 무변경 |

---

## 5. 시험 (ADR-0012)

**순수 — 모든 OS(가짜 포트 · 시계 · 표식 · 가짜 `Pinned` 는 drop 때 두 자물쇠가 잡혀 있었는지 적는다):**

| 대상 | 경우 |
|---|---|
| 상태기계 · 불변식 | 임의 전이열(① · ② · 쓰기 확인 · `open_turn` · `deliver` · `close_turn` · 걸음 · 판 끝 · 패닉) 뒤 §3-2 불변식 전부(「`written` 이면 `w`」 포함) |
| 두 구간 · 여는 이 | `close_turn` → ② `None` · ★`deliver` · `active ≠ R` · 다른 에피소드 → 줄만 주고 에피소드 없음 · `stop(R)` · 스냅숏 버림(놓은 뒤)★ · 두 Esc 가 엇갈림: 둘째 ① 이 `opening` 을 보고 줄만 준다(기록 · 스냅숏 0) · 여는 이의 ② 뒤 셋째 Esc 는 이어 적음 · ★여는 이 가드: 패닉 · 이른 반환 · 줄 없음 · 줄만 — 모든 출구에서 `opening` 내림 · `stop(R)` · 찍다 만 스냅숏 버림 · 둘 다 자물쇠 밖★ |
| ★쓰기 확인 · 쓰기 명단 W(14판)★ | 쓰이지 않으면 판 없음 · 확인이 W 를 찍은 뒤 `last_mono` · 일꾼 · ★명단 실패 · 손잡이 사라짐 → 첫 쓰임이 서지 않는다(판 없음 · warn)★ · ★쓰기 전에 가입해 산 탄생 → W 에 든다 → 후보 아님(가짜 포트가 그 가입을 **W 뒤에** 늦게 내줘도 — 알림 때에 기대지 않음을 잰다)★ · ★쓰기와 W 사이에 가입한 탄생 → W 에 든다 → 빠짐(놓침 창)★ · W 뒤 가입 → 후보 · W 의 번호가 끝나고 쓰인 뒤 탄생이 재사용 → 빠짐(놓침) · 쓰기 전에 가입해 W 전에 끝난 것 → 고리 전용 · 둘째 끊기의 W 는 버린다(놓은 뒤) · W 는 문 자물쇠 밖에서 찍고 밖에서 버린다 · 부를 것이 패닉 → error · 큐는 계속 쓴다 |
| 스냅숏 | `Failed(list · root · too_many · shape · port)` · 기록을 스냅숏 앞에 켠다 · 이어 적는 끊기는 명단 · 기록 켬 0 |
| 탄생 기록 | 기록 중 가입 → `pin(kill = true)` → `seq` · 아님 → 버림 · 끝남 → 버림 · ★`pin(kill = true)` `Err` → 고리 전용(`killable = false`) · 후보 아님 · 사슬은 잇는다★ · ★(pid, 생성) 같으면 한 번 · pid 같고 생성 다르면 따로★ · 512 넘음 → `full` · ★`start` 가 `full` 을 비우고 남은 기록을 밖에서 버린다 · 판의 복사는 `active ≠ rec` 면 빈 것 · 에피소드가 사는데 기록이 사라짐 → `snapshot_failed(port)` · 기록 자물쇠가 독에 걸려도 돈다★ · 기록 자물쇠 안 OS 호출 · 핸들 닫기 0 · 약한 손잡이 사라짐 · 표식 → 기록 놓고 끝 · 듣는 스레드는 올린 `Arc` 를 쥔 채 기다리지 않는다(가짜 포트가 기다리는 동안의 강한 수를 적는다) |
| ★듣는 스레드 실패 모드(새)★ | `next` 오류 · 차례 몸통 패닉 → `port_failed` 굳음 · `stop(active)` · 로그 한 번(`next` 오류 → warn · 패닉 → error) · `unwatch` → 성공이면 끝 · ★실패면 끝나지 않고 꺼내 버리기만(기록 0) — 약한 손잡이 · 표식에서 끝★ · 다음 에피소드의 ⓐ 가 굳은 칸을 보고 `Failed(port)` |
| ★포트 확보 — 꺼내는 이가 먼저★ | 처음 여는 이만 `watch_births` · 두 번째 부름 없음(코드) · ★가짜 Job 이 부름 순서를 적는다: 포트 → 스레드 기동 → 붙이기★ · ★기동 실패 → 붙이기가 불리지 않는다 · 굳음★ · ★붙이기 실패 → 굳음 · 스레드가 `false` 를 받고 끝난다(떼기 안 부름)★ · 보내는 쪽이 사라짐 → 스레드는 돈다 · 약한 손잡이 사라짐은 굳히지 않음 |
| ★규칙 — 스파이크 모양★ | p1 · p3t3 · pilot 모양 → `Cleanup` · p3bg 모양 → 1 차례 주인 `Cleanup` → 확인 → 2 차례 fork 24776 `Cleanup` · ★taskkill 탄생이 없음 · `/PID` 가 다른 번호 · 부모가 claude 가 아님 · taskkill 생성 < T 생성 → `NoTaskkill`★ · ★부모 생성 > 자식 생성(재사용 흉내) → `ChainCut`/`ParentUnknown`★ · 중간 고리 알림 유실 → `ChainCut` · 반복의 `sleep` → `NotHookCopy` · 반복 bash 산다 → `AncestorAlive` · taskkill → `ParentAlive` · Bash 도구 사슬 전멸 뒤 서브셸 → `NotHook` · 순환 · 17 걸음 · k = 1 · 뿌리 먼저 → `ChainCut` · 런처 모양 → `NotHook` |
| 판의 차례 | `seq` 오름차순으로 끝낸다 · ★한 차례의 확인은 마감 하나(200 ms)를 나눠 쓴다(가짜 시계로 차례 기다림 합 ≤ 200 ms)★ · 마감 안에 안 끝남 → 다음 차례에서 산 것 · 그 아래 빠짐 · 3 차례 넘음 → `rounds_exhausted` · `Cleanup` 없으면 멈춤 · ★재확인 거절(턴 끝 · 새 끊기 · 새 입력 · 표식) → 남은 후보와 다음 차례를 버리고 판 끝(debug)★ |
| `is_hook_command` · `taskkill_names` | 실행기 · 껍데기 → 참 · Bash 도구 · `shell-snapshots` · `node` · `.cmd` · `-lc` · 빈 칸 → 거짓 / `taskkill.exe /PID 35688 /T /F` → 35688 · `/T` 나 `/F` 빠짐 · 다른 실행 파일 → 없음 |
| 종료 표식 · 새 입력 · 뿌리 재확인 · 패닉 가드 · 끝내기 시간 | 11판 그대로 · ★`RetiringSignal` 에 세우는 메서드 없음★ |
| decoder 배선(백업 `decoder_with_gate` `:5692`) | 진행 · `result` · 둘째 `started` → 버림 · 여는 `started` 는 한 구간 · 이어받기 무시 |

**실프로세스 — 시험마다 `#[cfg(windows)]` · 넷 이하(콘솔 호스트 포함) · 도우미 = HEAD `process_group.rs:176`:**

1. 명단(HEAD `:498` · `:456` — 도우미를 `pin_member` 의 `exited()` 로). 2. 붙든 멤버 끝내기 · 종료 코드 · `wait_exit`(HEAD `:346`). 3. 다른 Job · 부모 Job 에만 → `Ok(None)`(`:405` `:425`). 4. `:381` 걷음. 5. `#[ignore]` Git Bash — MSYS fork 자식(`:527`).
6. 사실 — 문 달린 `cmd` X: 부모 = 시험 프로세스 · 생성 ≥ 시험 프로세스의 생성 · 실행 파일 · 명령줄(`QUERY_LIMITED`). 7. 가입 알림 — 시험이 `start` 가 받은 포트를 쥔다 · ★`start` 가 `Err` 면 붙지 않는다(뒤에 띄운 `ping` 이 그 포트에 안 온다 — `Timeout`)★ · `Ok` 면 붙인 뒤 X(되알림)가 `Joined` · X 가 띄운 `ping` `Joined` · 끝남 `Other` · `unwatch_births` 뒤 새 멤버는 안 온다. 8. `#[ignore]` Git Bash — 껍데기 `bash -c "bash x.sh"` 아래 스크립트(exec) · 서브셸(사본) · `sleep`(아님).
9. 통로 — HEAD `:725` 을 시계 없이(100 ms 유지) · ★`begin_retire()` · `shutdown()` 이 `retiring()` 을 세운다★ · 통로를 버리면 명단 빈 · 붙들기 `Ok(None)`. 10. 쓰기 확인 — 쓰인 뒤 한 번 · 닫히면 안 불림 · 부를 것 안 `push` 교착 없음 · 부를 것 패닉 → 다음 덩이도 쓰인다.
11. manager — `kill_agent` 가 `begin_retire` 를 권한 회수보다 먼저 · `tear_down_failed_activation` 이 표식 대조 뒤 첫 줄에서 부른다(가짜 통로가 부름 순서를 적는다).
12. `leftover.rs` 실물 포트 — R(`CREATE_NO_WINDOW` 문 달린 `cmd`)이 띄운 B 가 끊기 뒤 P(ping)를 띄우고 끝남 → P 생존 · `not_hook_copy` · 끊기 전 탄생만 → `no_new_member` · 부모가 산 끊기 뒤 자식 → `parent_alive`. 13. `#[ignore]` MSYS 반복 · 파이프 → 생존.

★끝내는 경로의 실프로세스 시험은 없다★(claude 훅 실행기 모양 아래 사본 고아는 넷을 넘는다) — 순수 시험(스파이크 모양) + §7 G1. `cargo test -p engram-dashboard-agent -- --test-threads=4` · CI 는 `#[ignore]` 를 뺀 전부.

---

## 6. 구현 순서 — 어디서 멈춰도 빌드가 선다

코더 하나 · 순차. **먼저:** `git diff HEAD -- crates/` 가 비어야 한다.

**접점:**

```rust
pub(crate) struct ProcessFacts { pub ppid: u32, pub create: u64, pub image: String, pub cmdline: String }
pub(crate) struct RetiringSignal(Arc<AtomicBool>);                          // is_set() 만 · 만드는 것은 통로
pub(crate) trait Pinned: Send + Sync { fn exited(&self) -> io::Result<bool>; fn wait_exit(&self, d: Duration) -> io::Result<bool>;
    fn facts(&self) -> &ProcessFacts; fn terminate_raw(&self) -> io::Result<()>; fn classify(&self, raw: io::Result<()>) -> io::Result<MemberKill>; }
pub(crate) trait Births: Send + Sync { fn next(&self, wait: Duration) -> io::Result<PortEvent>; }   // Joined(pid) · Other · Timeout
impl ProcessGroup { fn member_pids(&self) -> io::Result<Vec<u32>>; fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>>;
    fn watch_births(&self, start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>) -> io::Result<()>;   // 포트 → start → Ok 면 붙이기
    fn unwatch_births(&self) -> io::Result<()>; fn retiring(&self) -> RetiringSignal; }
// transport
pub trait AgentTransport { /* … */ fn begin_retire(&self) {} }
pub type OnWritten = Box<dyn FnOnce() + Send>;
pub struct InterruptOut { pub bytes: Vec<u8>, pub on_written: Option<OnWritten> }
pub type InterruptLine = Arc<dyn Fn() -> Option<InterruptOut> + Send + Sync>;
// InputQueue: Inner.pending: VecDeque<(Vec<u8>, Option<OnWritten>)> · push(b) = push_with(b, None) · push_with(b, cb)
//             pop() -> Option<(Vec<u8>, Option<OnWritten>)> · drain: write → mark_written → catch_unwind(cb()) (패닉 = error · 계속 — §3-8)
```

**조립(백업 `claude/mod.rs:443-449`):** `GateSlot` → `stream_decoder` → `StdioTransport::open` → `leftover_cleaner`(`process_group()?` · 뿌리 0 이면 `None` · `Cleaner::new(port, clock, root_pid, retiring, agent)` — 포트 · 듣는 스레드는 첫 에피소드에서) → `GateCell::new` · `slot.get_or_init` → `with_interrupt(interrupt_line(gate))` → `start()`.

**백업에서(`leftover.rs`):** 살림 = `INTERRUPT_LEFTOVER_GRACE` · `ProcessPort`(고쳐서) · `LeftoverClock` · `SystemClock` · `Phase` · `Episode` · `GateState` · `Accepted` · `Step` · `Ticket` · 전이들 · `Commit` · `Cleaner` · `GateCell` · `run_worker`(패닉 가드) · `Stop` … `PassReport` · `run_pass`(차례 · 순서 교체 · 확정 구간 `:1173-1191` 모양 그대로) · `log_*`. 걷음 = 시계 · 모양 · 옛 고르기와 그 시험. `claude/mod.rs` = 11판과 같음.

| 단계 | 내용 | 멈춰도 서는 이유 |
|---|---|---|
| ① | 아래 순서 | 바뀌는 이름은 1단계 시험 · HEAD `interrupt_line` · `kill_agent` 첫 줄 말고 아무도 안 부른다 |
| ② | `leftover.rs` + 시험 · `mod leftover;` | 배선 없음 |
| ③ | 배선 · `TurnGate` 걷기 · `started` · 앵커 · HEAD 시험 `:6070-6137` | 처음 동작이 바뀐다 |
| ④ | 새 ADR · ADR-0257 부분 폐기 도장 · ADR-0238 링크 · CLAUDE.md(소유권 · `TurnGate` → `GateCell` · 락 순서에 새 잎 둘 · 「의존성」 feature 둘 · `begin_retire`) · `session-path-ownership.md:160` | `/review doc` |

- **① 의 순서(각 걸음이 홀로 컴파일):** 1. `root_attached` 걷기 — 통로 칸 · 블록 · `ProcessGroup::new(job, root_attached)` → `new(job)` · `detached()` · `root_attached()` 와 ★그것을 단언하던 시험 두 줄(HEAD `process_group.rs:144` · stdio `:768`)을 같은 걸음에서★ 2. `console_wrapper_depth` 걷기 3. stdio `:725` 을 시계 없이 고친 뒤 `process_tree` 셋 걷기 4. `Cargo.toml` feature 둘 + `windows.rs` 의 `pin_member` · 사실 · `watch_births(start)` · `unwatch_births` · `BirthPort{next}` · 시험 6 · 7 · 8 5. `begin_retire`(trait 기본 · `StdioTransport` 칸 · `shutdown` 첫 줄 · `Session` · `kill_agent` 첫 줄 · `tear_down_failed_activation`) + 시험 11 6. `process_group.rs` 의 `new(job, retiring)` · `pin` · `watch_births` · `unwatch_births` · `retiring` · `Pinned` · `Births` · `RetiringSignal` + HEAD 시험(`:346` `:405` `:425` `:456` `:527`)과 도우미(`identity` · `parent_is_dead` · stdio 시험의 `known` · `still_running`)를 `pin` 으로 옮긴 **뒤** `verify` · `Verify` · `ReadyKill` · `verify_member` · `MemberCheck` · `:381` 걷기 + 시험 9 7. `InterruptOut` · 큐 모양(`push_with` · `pop` 쌍 · `drain` 의 `catch_unwind`) + HEAD `interrupt_line` 을 `InterruptOut { bytes, on_written: None }` 으로 · ★stdio 시험 `:589` · `:615`(`pop` 이 쌍을 준다) · `input_queue.rs` 시험 `:391` · `:410`★ · 시험 10 8. 머리 doc.

---

## 7. 검증 계획

**기계 게이트(`/qa`):** agent 시험(`--test-threads=4`) · 워크스페이스 회귀(manager 시험 포함) · `cargo fmt --check` · 코어 격리 `rg` · 생성물 sync.

**GUI 실측(`/qa full` · 실제 대시보드 · claude · 훅):**

| # | 시나리오 | 통과 조건 |
|---|---|---|
| G1 | `UserPromptSubmit` 훅 · 보낸 뒤 약 0.2 초 Esc · **10 회 이상** | 멈춘 회는 N + 1 초 안에 풀린다(기대 10/10 · 못 풀면 `reason`) · `terminated` = 훅 스크립트 bash(와 그 사본) · `chain` 꼭대기 = 훅 실행기 · `rounds` · 멈추지 않은 회는 warn 0 · 다음 턴이 답한다 · `terminate_max_us` 수 ms 이하 · 포트 실패 warn 0 · ★첫 에피소드(첫 끊기에 붙인 포트 — 하네스는 스폰 때 붙였다)도 끝낸다★ · ★적는다: 첫 에피소드의 탄생 수 · `full` · 붙이기 → 열기 지연 · ★쓰기 → W 지연과 첫 주인 탄생까지의 여유(놓침 창 — §8 ㉓)★(§8 ㉒ⓑ)★ |
| G1b | 대기 입력을 두고 Esc | `started` 로 버려지는 비율(놓침 · §8 ⑱) |
| G2 | 로그 대조 | `terminated` 에 claude PID 없음 · 종료 코드 `0x7440` · `snapshot_failed` · `root_unknown` warn 0 |
| G3 · G4 · G5 | E-late · B20 · 다른 에이전트 · MCP · 빌드 | 정리 0 · 약 20 초 · 살아 있다 |
| G6 | Bash 도구 도중 Esc · 데몬 MCP · `run_in_background` 빌드 · MSYS 반복 · 파이프 — 그 중에 훅 멈춤 | 백그라운드 · 도구의 프로세스는 하나도 안 끝난다 · 잔여물만 끝난다 |
| G7 | 멈춘 턴 도중 에이전트 kill | 정리 warn 0 · `retiring` debug |

- 멈춤이 안 나면 표본을 늘리고 「정리 경로 미관측」(PASS 아님). 못 끝냄 warn 의 `reason` 으로 모양을 가른다 — 모두 닫힌 실패다.

---

## 8. 위험 · 알려진 한계

1. **잔여 경합(ADR-0238 결정 7):** 구간 사이 턴 닫힘이면 줄 없음으로 조금 좁혔다.
2. **턴 끝을 놓침:** 다음 `started` 가 버린다(B2).
3. **놓침(오늘처럼 300 초):** 가입 알림 유실 · 붙이는 순간 · 짧은 중간 고리를 열기 전에 끝남 · 시계가 두 생성 사이에 뒤로 뜀 · 기록 넘침 · 부모 · 조상이 산다 · 사슬이 알려지지 않음 · 훅 감싸기 · 끊기 방식(taskkill)이 바뀜 · 명령줄을 못 읽음 · 대기 입력의 `started` · 포트 실패 · 쓰기 명단 실패(그 쓰기로는 판이 없다) · 끝냄 확인이 200 ms 를 넘음 · 3 차례를 넘는 fork 세대 · 쓰기와 W 사이에 태어난 주인(㉓) · W 의 번호를 재사용한 주인. 잰 10 번에서는 없었다(추론).
4. **래퍼 모양:** cmd 와 claude 사이에 런처 → 모두 놓친다.
5. **스냅숏 · 기록의 한계:** 명단 불완전 · 열기 전 재사용 → 생성 순서가 거른다 · 붙드는 수 ≤ 256 + 512 · 쥐는 시간 ≈ N 초 · 중첩 Job 재보고 → 한 번만.
6. **스폰 직후 편입 틈:** 범위 밖. 7. **전제(실측):** 주인 = 훅 스크립트 bash(사본 또는 껍데기가 exec 한 것) · 사슬 전멸 · taskkill 이 꼭대기를 가리킨다. 8. **대체 없음:** 이어 보낸 턴 9/9. 9. **Windows 에서만.** 10. **upstream 이 고치면 잠든다.** 11. **taskkill 과 그 conhost:** 부모 claude 산다 · 사본 아님.
12. **닫힌 실패의 대가:** 스냅숏 · 포트 · 뿌리 · 일꾼 · 패닉 실패는 아무것도 안 끝낸다.
13. **줄이 안 닿음 — 닫혔다(쓰기 확인).** 14. **`TerminateProcess` 가 문 자물쇠 안:** `terminate_max_us`. 15. **부모 번호의 한계:** `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` — 규칙을 다 채워야 끝난다. 16. **뿌리 · claude 생존 틈:** ms · 차례마다 본다.
17. ★**남는 오살 틈**★:
    - ⓐ **창 = (첫 쓰임의 W, 마지막으로 쓰인 끊기 + N + 차례들]** 의 가입 중 훅 사본 · 사슬 전멸 · 꼭대기 = 훅 실행기 · claude 의 taskkill 이 그 꼭대기를 가리킴 — **claude 가 taskkill 한 훅 나무의 스크립트 · 서브셸이 살아남은 것**이다. 끝내는 것은 끊기의 뜻과 맞는다. Bash 도구 사슬(taskkill 없음 · 감싸기 다름) · exec 된 프로그램 · 산 작업 아래의 것은 빠진다. claude 가 Bash 도구를 훅처럼 감싸고 taskkill 까지 하게 바뀌면 들어올 수 있다(⑳).
    - ⓑ **사슬 이어 붙이기(재평가 — 라운드 3 F1):** 부모 번호의 재사용은 생성 순서가 거른다. 남는 것 = 재사용과 **그 두 생성 사이의 시계 뒤로 뛰기**가 겹치거나 ★재사용한 새 주인의 생성이 자식과 **같은 눈금**일 때(검사는 「부모 ≤ 자식」 — 같은 눈금은 통과한다 · ㉔ K4 · 착지 고침 2026-10-01)★, 그리고 엉뚱한 사슬이 규칙을 다 채울 때 — 무시할 만하다(추론).
18. **대기 입력의 `started` 가 멈춘 턴을 버림(놓침):** G1b.
19. **통로 종료 겹침:** `kill_agent` 첫 줄 · `tear_down_failed_activation` · `shutdown` 첫 줄에서 표식이 서므로 겹침은 「재확인이 읽은 직후」 한 번뿐 · 해가 없다. 듣는 스레드는 매 차례 약한 손잡이를 잠깐(µs) 올린다 — 그 순간 통로가 사라지면 Job 핸들 닫기(`KILL_ON_JOB_CLOSE`)가 그만큼 늦는다.
20. **claude 판 따라 모양이 바뀜:** 훅 · Bash 도구 감싸기 · `bash ` 붙이기 · taskkill 방식 · MSYS 명령줄 복사는 문서 없는 내부(2.1.284) — 판마다 출처 스파이크를 다시 돌린다.
21. **포트는 한 번 붙는다 · 꺼내는 이가 먼저:** 듣는 스레드가 뜬 뒤에만 붙인다 — 기동 실패면 붙이지 않는다 · 붙이기 실패면 스레드가 끝난다(§3-5). 에피소드 밖 알림은 듣는 스레드가 버린다 · 스레드가 오류 · 패닉으로 끝나면 떼고 굳힌다(`Failed(port)`) — ★떼기가 실패하면 스레드는 끝나지 않고 약한 손잡이가 죽거나 `retiring` 이 설 때까지 꺼내서 버리기만 한다★(큐가 쌓이지 않는다 · 한 화신 동안).
22. ★**추론 목록**★ — ★**안전을 떠받치는 것(틀리면 엉뚱한 것을 끝낼 수 있다)**★: ⓐ claude 의 직계 자식 중 훅이 아닌 것이 `-c "bash …"` 를 쓰지 않는다(이 설정 하나만 쟀다 — taskkill 표지가 둘째 벽) ⓑ 뿌리 자식 = conhost + claude(claude 를 고르는 모양 — 둘 넘으면 `Failed(shape)`) ⓒ 사슬 이어 붙이기에 시계 뒤로 뛰기가 겹치지 않는다(⑰ ⓑ) — ★재사용한 새 주인이 자식과 같은 생성 눈금인 경우도 잔여다(「부모 ≤ 자식」이 통과시킨다 · ADR-0262 K4)★ ⓓ claude 내부 모양(⑳ — 문서 없음 · 판마다 스파이크). ★「쓰기 전 탄생은 후보가 아니다」는 여기 없다 — 쓰기 명단 W(문서 API 의 완전성 + 붙든 핸들)가 세운다(§3-2)★.
    **효과에만 드는 것(틀리면 놓친다):** ⓔ **첫 끊기에 붙이는 포트는 하네스가 재지 않았다**(하네스는 스폰 때 붙였다 — 붙일 때 이미 있던 멤버를 되알린다는 문서대로면 첫 에피소드는 되알림으로 `BIRTH_MAX` 자리와 큐를 먼저 쓴다 · G1 이 잰다) ⓕ 가입 알림은 가입하는 자리에서 쌓이고 한 스레드가 쌓인 순서로 꺼낸다(늦으면 짧게 산 고리를 열기 전에 놓친다) ⓖ 멈춤 주인이 파이프의 유일한 주인은 아니다(p3bg — 차례가 그것을 겨냥한다) ⓗ `TERMINATE` 권한 열기는 bash 주인에서만 쟀다(실패하면 고리 전용) ⓘ 명령줄 클래스 60 을 `QUERY_LIMITED` 로 읽는 것은 하네스 실측이지 문서 서술이 아니다 ⓙ 다른 Windows 판에서의 권한 ⓚ 쓰기 → W 틈이 주인 탄생보다 훨씬 짧다(㉓). (포트 `NULL` 떼기는 문서 서술이라 뺐다.)
23. ★**놓침 창 — 쓰기와 W 사이(확인 라운드 codex BLOCK #3)**★: 줄이 쓰인 뒤 라이터가 W 를 찍기 전에 가입해 W 때 산 것은 W 에 들어 후보가 못 된다. 틈은 라이터가 쓰기에서 돌아와 명단 호출 하나를 하는 시간이다(짧다 — 추정 · G1 이 로그의 쓰기 → W 지연으로 잰다). 잰 여유: 주인은 쓰인 뒤 263–454 ms(p3bg 주인 1.4 s · 그 fork 2.8 s) · 훅 fork 는 taskkill 보다 1 ms 이상 늦고 taskkill 은 claude 가 줄을 읽은 뒤 띄운다(메인 측정 · §3-0 표). 시험 = §5 「쓰기 확인 · 쓰기 명단 W」 행. ★13판의 「흐림이 양쪽 다 사라진다」는 이 쪽에서 틀렸다 — 이 틈은 남고, 방향은 놓침이다★. ★G1 실측: 쓰기 → W 6–26 µs · 쓰기 → 주인 탄생 320–504 ms★.
24. ★**구현 리뷰가 남긴 알려진 한계 K1–K9(착지 2026-10-01)**★ — 사용자 결정 2026-10-01 「일단 재현 가능한것만 구현하자. 좀 현실적인 사항으로 단순화」 · 「현실적으로 가능한것 위주로 단순화 하자」: 재현되거나 현실적인 지적만 고치고 이론 경로는 **코드 없이** 여기 적는다(정본 = ADR-0262 「알려진 한계」).
    - **K1–K9 의 목록 · 현실성 판단은 ADR-0262 「알려진 한계」에만 둔다** — 같은 목록을 두 곳에 적지 않는다.
    - **짓지 않은 것 · 돌리지 않은 것:** §5 실프로세스 시험 13(`#[ignore]` MSYS 반복 · 파이프)은 위 결정으로 짓지 않았다 · §7 G1b(대기 입력의 `started` — ⑱) · G3–G6 은 돌리지 않았다(실측 = G1 · G2 · G7).
    - **T-40 과 무관한 관측(`docs/tracking.md` T-41):** 새 에이전트의 첫 턴에서 `UserPromptSubmit` 훅 도중의 Esc 는 아무 일도 안 한다 — 문이 첫 `started` · 진행 줄에서야 열린다(G1 중 1 회).

---

## 9. ADR 후보 · 앵커

> ★착지 = **ADR-0262**(2026-10-01) — 아래 대체 · 개정 줄은 그 ADR 이 ADR-0257 · ADR-0238 · ADR-0244 에 부분 폐기 도장과 양방향 링크로 박았다. 코드 앵커 `// ADR-0257` 은 `// ADR-0262` 로 바꿨고 `base` 의 1단계 함수와 `INTERRUPT_LEFTOVER_GRACE`(N = 3 초 — ADR-0257 결정 4 · 두 앵커 나란히)에만 남는다★.

**새 ADR — 「claude 끊기 뒤 잔여물은 첫 끊기부터 판까지 Job 가입 알림으로 기록한 탄생 가운데 줄이 쓰인 직후의 멤버 명단에 없던 것으로서, 끊기 전 스냅숏과 탄생 기록으로 claude 까지 끊김 없이(부모가 먼저 태어난) 알려진 사슬이 모두 끝났고 훅 사본이며 꼭대기가 claude 가 taskkill 한 훅 실행기인 것만 끝낸다 — 벽시계는 쓰지 않는다」**(`/adr` 채번). ADR-0257 부분 대체 · ADR-0238 결정 2 · 3 · 5 개정 · ADR-0244 결정 2 개정 유지.

- **대체되는 것(ADR-0257 줄 번호 = 이 브랜치 HEAD):** `:11` 「맥락」 사고 근거 줄(→ 14판의 고리는 붙든 핸들 사이에서만 잇는다) · `:26` 결정 3 의 수단(「Job 명단 + 생성 시각」 → Job 가입 알림 + 붙든 핸들) · `:27` 결정 4 의 시계(N = 3 초는 그대로) · `:32` 결정 9 의 판정 수단(생성 순서 부모 판정 → 결정 9 · 10 · 11 + 고리마다 부모 ≤ 자식 생성) · `:35`–`:39` 「구현 요지」 전부 · `:42` ADR-0238 결정 3 개정 문구(→ `GateCell`) · `:58`–`:60` 「거부한 대안」 의 「문턱 뒤 생성」 근거 줄 · `:65` 「`push` 실패 되돌리기」(→ 쓰기 확인 · 쓰기 명단 W) · `:71` 「근거」 원자료 줄 · `:76` `TurnGate.state` 잎 줄(→ 잎 둘 — §3-3) · `:77` 닫힌 실패 목록(→ §3-4 판의 차례) · `:78` 한계 ② ③ ⑤ ⑬ · `:80` `base` 줄 · `:81` 코드 앵커 목록(`// ADR-0238` → `GateSlot`).
- **다시 읽고 글은 그대로:** `:25` 결정 2 「적은 시각 뒤에 생성」 = 줄이 쓰인 직후의 멤버 명단(W)에 없던 가입으로 읽는다(사용자 문구는 두고 새 ADR 이 읽는 법을 적는다) · `:28` 결정 5 「OS 조각 작게」 = 조각이 명단 · 붙들기 · 사실 · 포트 · 끝내기로 늘었으나 전부 `windows.rs` 한 곳 · 다른 OS 무동작 · `:77` 「한 에피소드에 판은 한 번」 = 판 한 번이 차례 ≤ 3 을 품는다.
- **그대로:** 결정 1 · 6–8(고지 · 대체 · 끄기) · 결정 9 의 「부모가 살아 있으면 건드리지 않는다」.
- ★**`:55` 「PID 부모 연결로 고르기(「부모 뒤 생성」 검사를 더해도)」 와의 화해**★ — 그 거부는 **Job 밖 PID 까지 부모 번호로 잇던** 사고(남의 `link.exe` · 재사용 21 중 4)에 대한 것이다. 14판은 ① 후보를 우리 Job 의 가입 알림에서만 받고(쓰기 명단 W 밖) ② 고리를 **붙든 핸들(스냅숏 · 같은 기록의 탄생) 사이에서만** 잇고 ③ 부모 ≤ 자식 생성 검사는 후보를 **줄이기만** 한다 — 부모 연결이 후보를 늘리는 자리는 없다. 그래서 거부를 뒤집지 않고 그 안에서 선다.
- **ADR-0238 개정 한 줄 더:** `:66` 락 순서 줄 「새 락 간선 없음」 → 「새 잎 둘(문 · 기록) · 겹쳐 잡지 않는다 · 쓰기 확인은 라이터 스레드에서 명단을 락 밖에서 찍고 문 자물쇠만 잠깐 · 기존 락과 간선 없음」.
- **결정 [사용자]:** §1 결정 5 · 6 · 7 · 9 · 10 · 11 · 8 은 11 이 대체. ★고리의 생성 순서 검사 · 규칙 4 의 taskkill ≥ T 는 [TRD] 고름이고 사용자 결정이 아니다(메인이 알렸다)★.
- **거부한 대안:** 벽시계(5판) · 스냅숏 한 번 찍고 비교(7–10판 — 끊기 뒤 태어나 죽은 고리를 잃는다 · 결정 11) · 6판의 여러 스레드 비우기 · 부모 죽음만 · 죽은 부모가 bash 인가 · 조부모 한 층 · Esc 사이에도 모으기 · 끝남 처리 · [TRD] 판 때 표 · Toolhelp 부모 · 사슬이 끊겨도 끝내기 · `pid_alive` · Esc 마다 다시 찍기 · 문 자물쇠 안 찍기 · 보호 집합 · 래퍼 모양 · 누적 가입 수 · 실행 파일만 비교 · 환경 변수(PEB) · 스트림 사건으로 잇기(PID 없음) · 에피소드 나이 상한 · 포트를 스폰 때 붙이고 늘 듣기(결정 7) · 알림 때 PID 만 적기 · ★「Esc 뒤에 태어났나」를 생성 시각으로 가르기(결정 5)★ · ★쓰기 확인 때 쓰는 쪽이 후보 경계(`next_seq`)를 적기(12판 — 큐에 먼저 쌓였으나 아직 안 꺼낸 알림이 경계 뒤로 섞인다)★ · ★같은 포트에 표지 패킷을 올려 경계를 긋기(13판 — 「가입하는 자리에서 쌓인다」는 문서가 없어 안전의 근거가 못 된다 · 쓰기 명단 W 로 대체)★ · 듣는 스레드를 붙인 뒤에 띄우기(13판까지 — 기동 실패 + 떼기 실패면 아무도 안 꺼내는 큐가 남는다) · 낡은 스냅숏으로 에피소드를 세우기(12판 — 세우지 않고 줄만 돌려준다) · 고리마다 훅 사본 검사(얻는 것 없음) · 끝낸 뒤 확인 없이 한 번에 끝내기(p3bg 의 fork 세대를 놓친다) · manager 가 backend 의 문을 직접 세움(백엔드 확장).
- **앵커:** `pin_member` · 사실 · `watch_births` · `unwatch_births` · `BirthPort` · `ProcessFacts` · `RetiringSignal` · `AgentTransport::begin_retire` · `kill_agent` 첫 줄 · `ProcessGroup` · `StdioTransport::process_group` · `InterruptOut` · 큐의 부를 것 · `leftover.rs` 머리 · `GateCell` · `Recorder` · 듣는 스레드 · 여는 이 · 두 구간 · 스냅숏 · `note_written`(쓰기 명단 W) · `deliver` · 판의 차례 · 끝내기 확정 · `select` · `is_hook_command` · `taskkill_names` · 패닉 가드 · `open_spawn` 조립 · `interrupt_line`. `TurnGate` 의 `// ADR-0238` → `GateSlot`.

---

## 10. 사용자 결정 필요

> 14판 기준: **사용자 확인 1 건**(의존성 변경). 고리의 생성 순서 검사 · taskkill ≥ T 는 [고름] — 사용자에게 알렸다(결정 아님).

1. ★**(확인) 새 `windows` feature 둘**★ — `Win32_System_IO`(완료 포트 — 가입 알림을 받는 유일한 공개 수단 · 결정 5 · 11) · `Wdk_System_Threading`(`NtQueryInformationProcess` — 핸들로 부모 · 명령줄 · 결정 10 · 11). 새 crate 없음 · `Cargo.lock` 무변경.
2. 닫힘: 결정 1–7 · 9 · 10 · 11(§1).

---

## 11. 의존성 · 자리

- ★**새 feature 둘 — `Win32_System_IO` · `Wdk_System_Threading`**★(`windows-0.58.0` — `Win32/System/mod.rs:55` · `IO/mod.rs:34` `CreateIoCompletionPort` · `:70` `GetQueuedCompletionStatus` / `Cargo.toml:398` · `Wdk/System/Threading/mod.rs:34` `NtQueryInformationProcess` · `:192` · `:195`). 켜지 않은 것 = `Win32_System_Kernel`(`repr(C)` 로 대신) · `Win32_System_SystemServices`(상수로).
- **쓰는 API:** JobObjects(`QueryInformationJobObject` · `IsProcessInJob` · `SetInformationJobObject` · `JobObjectAssociateCompletionPortInformation`) · Threading(`OpenProcess` · `GetProcessTimes` · `WaitForSingleObject` · `TerminateProcess` · `QueryFullProcessImageNameW`) · IO · Foundation · Wdk.
- **`base` 무변경** — 1단계 추가분은 기존 공개 함수의 몸통으로 남는다(소비자 = §2 끝 `rg`).
- **1단계 코드의 운명:** 그대로 = `member_pids` · `LEFTOVER_EXIT_CODE` · `MemberOutcome` · `terminate_raw` · `GetProcessTimes` 사용 · 통로의 `Arc` Job 칸 · 바뀜 = `VerifiedMember` → `PinnedMember` · `ProcessGroup::{new, detached}` · `process_group()` · 걷음 = `verify_member` · `MemberCheck` · `Verify` · `ReadyKill` · `root_attached`(둘) · `console_wrapper_depth` · `process_tree` 셋 · 그 시험들 · 더함 = `pin_member` · `ProcessFacts` · `watch_births` · `unwatch_births` · `BirthPort` · `Pinned` · `Births` · `RetiringSignal` · `begin_retire` · `InterruptOut` · 큐의 부를 것.
