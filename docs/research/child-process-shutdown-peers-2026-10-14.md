# 자식 프로세스 종료 방식 — 피어 서베이 (2026-10-14)

- **상태:** 조사 완료 · 결정 = C(Engram 이 띄우는 claude 자식 env 에 `DISABLE_AUTOUPDATER=1` — 사용자 2026-10-20) · 구현 2026-10-30(`crates/engram-dashboard-agent/src/backend/claude/mod.rs` 의 `NO_AUTOUPDATE_ENV` — 에이전트 스폰 · 사용량 조회 둘 다 · 프로필에 같은 키가 있으면 그 값이 이긴다) · ★그 env 가 실제로 업데이트를 막는지는 실 claude 로 재지 않았다★(문서 근거 — 아래 §3)
- **방법:** research 스킬 medium — 수집자 4갈래(멀티플렉서 소스 · 에이전트 매니저 소스 · Windows 콘솔/터미널 문서 · claude 업데이터 문서/이슈) → 메인 grounding(로컬 클론 `파일:줄` 재열람 5건 · `gh issue view` 1건) → cross-family 적대 리뷰(결과는 아래 「적대 리뷰」 절)
- **확신도 범례:** 확실 = 독립 출처 둘 이상 또는 코드 직접 확인 · 가능성 높음 = 단일 출처로 지지 · 불확실 = 근거 약함 · 모름 = 못 찾음
- **계기:** Engram 이 claude 를 강제 종료할 때 claude 의 백그라운드 자동 업데이트가 중간에 끊겨 설치가 깨진다는 사용자 관찰. 질문 = 「Engram 이 대응해야 하나, claude 자체 문제인가」 · 「다른 곳은 어떻게 끄나」.

## 결론

1. **Windows 에서 콘솔 이벤트(`CTRL_CLOSE_EVENT` · `ClosePseudoConsole`)를 먼저 보내고 기다리는 멀티플렉서·에이전트 매니저는 찾지 못했다.** Windows 경로가 있는 7곳(herdr · zellij · ttyd · orca · paseo · t3code · vibe-kanban)의 프로세스 단계는 강제 종료(`TerminateProcess` · `taskkill /T /F` · Job Object · node-pty `kill()`)다(tmux 는 Windows 경로 없음 · cline 은 자식 PTY 를 직접 관리하지 않음). 단 **프로세스 강제 종료 앞에 프로토콜 수준 끊기를 두는 곳은 있다** — vibe-kanban 은 claude 에 stdin JSON `Interrupt` 를 보내고 최대 5초 기다린 뒤 그룹을 죽이고, paseo · t3code 는 SDK `close`/stdin 닫기를 먼저 한다. — 가능성 높음(코드 재열람 5곳은 확실)
2. **콘솔 닫기를 먼저 하는 선례는 터미널 앱 쪽에 있다.** Windows Terminal 은 탭을 닫을 때 pseudoconsole 을 놓아 `CTRL_CLOSE_EVENT` 에 맡기고 `TerminateProcess` 를 먼저 부르지 않는다. — 가능성 높음(소스 요약 + 리뷰어 독립 확인)
3. **claude 자식 env 에 자동 업데이트 끄기를 주입하는 피어는 없다**(5개 에이전트 매니저 클론 grep 0건). — 가능성 높음
4. **claude 업데이터는 Engram 과 무관하게 Windows 에서 설치를 깨뜨린 보고가 여럿이다.** npm 전역 설치에서 실행 파일이 사라져 새 실행이 실패(#65478 · #90233 — 후자는 열린 상태) · 네이티브 설치에서 versions 저장소 → `.local\bin\claude.exe` 복사본이 같은 크기 다른 해시로 손상(#86295). 원인은 세 보고 모두 **추정**이고 강제 종료가 원인이라고 적힌 곳은 없다. — 가능성 높음(보고 존재) · 원인 불확실
5. **Engram 의 종료 순서는 피어와 같은 부류다.** `child.kill`(=`TerminateProcess`) → `TerminateJobObject` → master drop(`ClosePseudoConsole`) — zellij · ttyd 와 같은 「강제 먼저, 콘솔 닫기 나중」. 다른 점은 Job Object 로 손자까지 확실히 거둔다는 것이다(vibe-kanban 도 Job Object · orca/paseo 는 `taskkill /T` 로 같은 효과). — 확실(Engram 코드) · 피어 쪽 가능성 높음
6. **그래서 「터미널은 안전하고 Engram 만 깨뜨린다」는 근거가 없다.** 일반 터미널은 `CTRL_CLOSE_EVENT` 로 약 5초를 주지만, claude 업데이터가 그 신호를 받아 교체를 끝내는지는 **모른다**. Engram 의 강제 종료가 손상 확률을 얼마나 올리는지도 **모른다**(실측 없음).

## 1. Windows OS · 터미널 앱의 종료 동작

| 항목 | 내용 | 출처 | 확신도 |
|---|---|---|---|
| `CTRL_CLOSE_EVENT` | 콘솔이 닫히면 붙은 모든 프로세스에 보낸다. 타임아웃 = `SPI_GETHUNGAPPTIMEOUT`(기본 5000ms) 뒤 시스템이 종료. `CTRL_C`·`CTRL_BREAK` 는 타임아웃 없음 | learn.microsoft.com/en-us/windows/console/handlerroutine | 가능성 높음 |
| `ClosePseudoConsole` | 아직 붙은 클라이언트에 `CTRL_CLOSE_EVENT` 를 보낸다. Windows 11 24H2(26100)부터 즉시 반환, 이전은 끝날 때까지 대기(출력 파이프를 같은 스레드에서 읽으면 교착) | learn.microsoft.com/en-us/windows/console/closepseudoconsole | 가능성 높음 |
| Windows Terminal | 탭을 닫을 때 pseudoconsole 핸들을 놓아 `CTRL_CLOSE_EVENT` 에 맡긴다. `TerminateProcess` 를 부르지 않는다 | microsoft/terminal `src/cascadia/TerminalConnection/ConptyConnection.cpp` `Close()` (요약 도구로 읽음 — 줄 단위 미확인) | 가능성 높음 |
| VS Code(node-pty) | node-pty Windows `kill()` = ConPTY 경로면 입력 소켓 destroy + 네이티브 kill, winpty 경로면 콘솔 프로세스 목록에 `process.kill`. VS Code 가 그 앞에 유예를 두는지 | microsoft/node-pty `src/windowsPtyAgent.ts` | node-pty 가능성 높음 · VS Code 모름 |

## 2. 피어별 종료 순서

### 멀티플렉서

| 피어(체크아웃) | Unix | Windows | 근거 |
|---|---|---|---|
| herdr `065ef9d` (v0.9.1) | SIGHUP → SIGTERM → SIGKILL, 단계마다 250ms(20ms 폴링), 자손 PID 전체 | Hangup 무시, Terminate·Kill 둘 다 PID마다 `TerminateProcess`(Toolhelp 로 자손 열거) → 실질 「250ms 뒤 트리 강제 종료」. 패인용 Job Object 없음(상태 명령 전용만) | `src/pane.rs:1508-1559` · `src/platform/windows.rs:2197-2210` (메인 재열람 확인) |
| zellij `bf8d23a` | 패인 닫기 = SIGHUP 1회 · 서버가 SIGTERM 받으면 자식에 SIGTERM 3회 → SIGKILL | 패인 닫기 = 직계 자식 `TerminateProcess` 1회 → `ClosePseudoConsole`. 자손·Job 없음 | `zellij-server/src/os_input_output_windows.rs:370-382` (메인 재열람 확인) · `pty.rs:1801-1828` |
| tmux `5eaf557` (얕은 클론) | PTY master close 만 — SIGHUP 은 커널에 맡김 | 해당 없음 | `window.c:1120-1134` |
| ttyd `2922cb8` | 프로세스 그룹에 SIGHUP(`-s` 로 변경 가능) | 신호 무관 `TerminateProcess` → `ClosePseudoConsole` | `src/pty.c:160-164` (메인 재열람 확인) · `src/pty.c:102-110` |

### 에이전트 매니저

| 피어(체크아웃) | 순서 | Windows | 근거 |
|---|---|---|---|
| orca `df7460af` (낡은 얕은 클론) | 에이전트면 자손 정리 → node-pty `kill()` → 5초 뒤 `forceKill` 최대 2회(250ms) | PID 신원 확인 후 `taskkill /pid X /T /F`(5초 타임아웃) → node-pty `kill()`. `kill()` 을 이미 냈으면 5초 뒤 강제를 건너뜀(「ConPTY 이중 닫기 위험」 주석) | `src/main/daemon/session-termination-controller.ts:55-90,154-173` · `src/main/windows-process-tree-kill.ts:13-28` |
| paseo `bbe3f173` | claude = SDK `input.end()`·`query.close()`·`interrupt()` 뒤 tree-kill(2초+2초) · codex = stdin 닫기 → SIGTERM → 2초 → SIGKILL → 1초 · 셸 PTY = Windows 는 `kill()` → 2초 → `kill()` → 1초 | tree-kill 은 Windows 에서 `taskkill /T /F`(라이브러리 미열람) | `packages/server/src/utils/tree-kill.ts:33-54` · `.../terminal/terminal.ts:1467-1525` |
| t3code `5378f87f` | claude = SDK `query.close()`(주석: stdin 닫기 → SIGTERM → SIGKILL) · codex app-server = SIGTERM → 2초 → SIGKILL · 터미널 = SIGTERM → 1초 → SIGKILL | Windows 분기 없음(Node 시맨틱 — SIGTERM = `TerminateProcess`) | `apps/server/src/terminal/Manager.ts:1616-1660` |
| vibe-kanban `78580443` | claude 에 stdin JSON `Interrupt` → 최대 5초 대기 → 프로세스 그룹 종료(Unix SIGINT → 2초 → SIGTERM → 2초 → SIGKILL) | 신호 사다리를 건너뛰고 바로 `child.kill()` — command-group Windows 그룹 = Job Object(라이브러리 미열람) | `crates/utils/src/process.rs:5-31` (메인 재열람 확인 — 사다리가 `#[cfg(unix)]` 안) |
| cline `25ef0939` | claude·codex 를 자식 PTY 로 직접 관리하지 않음 | — | — |

**공통 패턴:** 구조화 경로(SDK · app-server · stdin JSON)가 있으면 프로토콜 수준 interrupt/close 나 stdin 닫기를 먼저 하고, 그다음 트리 강제 종료로 간다. 유예 1~5초 · 강제 뒤 대기 1~2초. Windows 에서 `CTRL_C_EVENT` / `CTRL_CLOSE_EVENT` 로 정중히 끝내려는 곳은 없다(zellij 의 SIGINT 대응 `GenerateConsoleCtrlEvent` 는 종료가 아니라 사용자 Ctrl+C 전달용).

## 3. claude 자동 업데이터

| 항목 | 내용 | 출처 | 확신도 |
|---|---|---|---|
| 동작 | 시작 시 + 실행 중 주기적으로 확인, 백그라운드 다운로드·설치, 다음 시작 때 적용. 네이티브 설치만 자동(WinGet 등 패키지 매니저 설치는 기본 꺼짐) | code.claude.com/docs/en/setup | 가능성 높음 |
| 끄는 법 | `DISABLE_AUTOUPDATER=1` = 백그라운드 확인만 끔(`claude update`·`claude install` 은 동작) · `DISABLE_UPDATES` = 수동까지 전부 끔 | 같은 문서 | 가능성 높음 |
| 설치 구조 | Windows 네이티브 = `%USERPROFILE%\.local\bin\claude.exe` + `%USERPROFILE%\.local\share\claude`. macOS/Linux 는 versions 디렉터리 + symlink 전환 | 같은 문서 | 가능성 높음 |
| Windows 교체 결함(npm) | `claude.exe` → `claude.exe.old.<ts>` 이름 변경 뒤 새 파일 쓰기(실행 중 이미지를 덮어쓸 수 없어서) — 쓰기가 실패하면 실행 파일이 없는 상태가 남아 새 실행이 실패. #65478 은 동시 세션까지 죽는다고 적지만 그 교체 순서 자체를 「추정」이라 밝히고, stale 봇이 not planned 로 닫았다(메인테이너 결정·수정 아님). #90233(열림 · `update_apply_exe_locked`)은 실패마다 ~240MB `.old` 누수 + 간헐적으로 `claude.exe` 없음을 보고 | anthropics/claude-code#65478 · #90233 (`gh` 로 메인 확인) | 보고 존재 = 가능성 높음 · 실행 중 세션까지 죽는지 = 불확실(보고 갈림) |
| Windows 손상(네이티브) | 백그라운드 업데이트 뒤 `%USERPROFILE%\.local\bin\claude.exe` 가 versions 저장소 원본과 같은 크기 · 다른 SHA256 → 즉시 `0xC0000005`, `claude update` 로 자가 복구 불가 — 즉 Windows 네이티브는 symlink 가 아니라 **복사**로 보인다. 원인은 추정 | anthropics/claude-code#86295 (2.1.229 · not planned · `gh` 로 메인 확인) | 가능성 높음(보고) · 원인 불확실 |
| 기타 보고 | 실행 중 바이너리가 디렉터리를 잠가 스테이징 교체가 EBUSY 로 실패(#67909 — 미러 요약만 읽음) · VS Code 확장 활성 시 EBUSY 조용한 실패(#61676 — 검색 스니펫) | 각 이슈 | 불확실 |
| 동시 인스턴스 잠금 | `.update.lock`(5분) — 비공식 소스 사본에서만 | tangled.org 사본 | 불확실 |
| 종료 신호 처리 | 업데이터가 `CTRL_CLOSE_EVENT` 를 받으면 교체를 끝내는지 / 별도 자식으로 도는지 | — | 모름 |

## 4. Engram 현황 (비교 기준)

- 종료 = `transport.shutdown()` → `child.kill`(=`TerminateProcess`) + wait → `TerminateJobObject` → master drop(ADR-0001). graceful 단계 없음.
- 데몬이 죽어도 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 로 트리 전체가 OS 수준에서 즉시 종료(`crates/engram-dashboard-agent/src/platform/windows.rs:156`).
- claude JSON 모드에는 끊기 줄(`InterruptLine`)이 이미 있다. 종료 인과(`begin_retire` → `shutdown` → `join_pump` — CLAUDE.md 「핵심 불변식」)에는 끊기 단계가 없다(코드로 재확인은 안 함).
- claude 자식 env 에 업데이트 관련 값을 넣지 않는다(저장소 grep 0건).

## 5. 선택지 (결정은 사용자)

| 선택지 | 효과 | 비용 · 위험 | 피어 선례 |
|---|---|---|---|
| A. 그대로 둔다 | — | 업데이트 창과 겹친 종료는 계속 깨질 수 있음 | 9곳 전부 Windows 에서 즉시 강제 |
| B. 강제 전에 콘솔 닫기(`ClosePseudoConsole`) 먼저 + 유계 대기 | 일반 터미널(Windows Terminal)과 같은 조건이 됨 | 종료가 수 초 늦어짐 · 24H2 이전엔 `ClosePseudoConsole` 이 **무기한** 막힐 수 있어 출력 파이프를 동시에 비우거나 먼저 닫아야 하고, 24H2 부터는 즉시 반환이라 반환이 클라이언트 종료를 뜻하지 않음 → 대기는 프로세스 종료 관측 + 상한 타이머로 따로 세워야 함 · **업데이터가 그 신호로 교체를 끝내는지 모름** — 효과 미검증 | 매니저 쪽 없음 · 터미널 앱(Windows Terminal)이 이 모양 |
| B'. 강제 전에 프로토콜 끊기(claude JSON 모드 interrupt) + 유계 대기 | 진행 중 턴을 정리할 틈을 줌 | 터미널 모드 claude 엔 적용 불가 · 업데이터에 대한 효과는 B 보다 더 불확실(업데이터는 턴과 무관) | vibe-kanban(5초) · paseo · t3code |
| C. claude 자식 env 에 `DISABLE_AUTOUPDATER=1` 기본 주입 | Engram 이 띄운 claude 의 백그라운드 확인이 사라짐 | **같은 설치본을 쓰는 다른 claude(일반 터미널 등)는 여전히 업데이트한다** — 공유 설치본의 교체 창이 없어지지는 않음 · 사용자 동작 변경(Engram 안에선 업데이트를 사용자가 직접) · 「백엔드 확장」 원칙상 `backend/claude` 안에서만 | 없음 |
| D. 문서 안내만(설정 권장) | 사용자 선택에 맡김 | 모르는 사용자는 계속 겪음 | — |

## 6. 후속 — 「업데이트 중」을 알 수 있나 (2026-10-14)

사용자 방향 = 「업데이트 중인 것만 알면 종료를 막는다 · 방법이 없으면 Engram 에서 자동 업데이트를 끈다」. 수집자 2갈래(공식 표면 · 설치본 2.1.284 의 내장 JS 문자열) → 메인이 문자열 추출본에서 락 코드를 재확인.

**결론: 문서화된 기계 판독 신호는 없다. 비공식 흔적은 있지만 설치 방식(npm 전역 · 네이티브 · Homebrew/WinGet)마다 다르고, 같은 버전 안에서도 락 위치가 조건부로 갈리며, 네이티브 락은 오탐이 난다.**

| 표면 | 「진행 중」 신호 | 확신도 |
|---|---|---|
| CLI(`claude doctor` · `update` · `install`) | 없음 — doctor 는 「마지막 시도 결과」를 사람용 텍스트로만 | 가능성 높음 |
| 설정 · env | 제어용만(`DISABLE_AUTOUPDATER` · `DISABLE_UPDATES` · `autoUpdatesChannel` · `minimumVersion` …) | 가능성 높음 |
| 훅 · OTel · 상태줄 입력 JSON | 업데이트 이벤트·필드 없음 | 가능성 높음 |
| SDK / stream-json | 못 찾음(system/init 필드 전수 대조는 안 함) | 불확실 |

**비공식 흔적(2.1.284 내장 JS · 문자열 추출 · 메인 재확인 표시)**

| 설치 방식 | 흔적 | 어느 프로세스인지 | 확신도 |
|---|---|---|---|
| npm 전역(이 PC) | 업데이트 락 = 내용이 `${process.pid}` 인 파일, mtime 5분(`K=300000`) 지나면 stale 로 지우고 다시 잡음. **위치가 조건부**: 어떤 플래그(`F()`)가 서면 `Ae.state("update-lock")`(위치 모름), 아니면 `<configDir>/.update.lock`(`~/.claude`) | pid 로 구분됨 | 코드 = 확실(메인 재확인) · 실제 어느 위치를 타는지 = 모름 |
| npm 전역 | 락을 잡은 뒤(`Xe()` → 설치 함수 → `finally` 해제) `npm install -g @anthropic-ai/claude-code@<ver>`(또는 bun) 자식 프로세스를 띄운다. Windows · 번들 실행 · 실행 파일이 `node_modules/@anthropic-ai/` 아래일 때만 그 전에 실행 파일을 `.old.<ms>` 로 이름 바꾼다(pid 없음 · 실패해도 설치는 계속). 실패 코드 `update_apply_exe_locked` | 자식의 부모 pid 로 구분 | 가능성 높음(락 → 자식 순서는 리뷰어 확인) |
| 네이티브 | 버전별 락 — 기본 `~/.local/state/claude/locks/<ver>.lock`(`XDG_STATE_HOME` 을 따름), 형식은 pid JSON `{pid, version, execPath, acquiredAt}` 또는 mtime 기반으로 갈림. **실행 중인 claude 도 실행 파일이 versions 디렉터리 아래면 자기 버전 락을 잡는다** → 다른 버전 락을 잡은 pid 가 업데이트 중인 프로세스라는 보장이 없다(다른 버전으로 도는 평범한 세션일 수 있음 — 오탐). 설치 중 별도 임시 락도 있다 · staging `~/.cache/claude/staging/<ver>` · 런처 교체 `claude.exe.new.<pid>.…` → `claude.exe.old.<ms>.<pid>` | 런처 교체 파일 이름은 pid 로 구분 · 락만으로는 오탐 | 가능성 높음 |
| 공통 | 완료 뒤 `~/.claude/.last-update-result.json`(진행 중 신호 아님) | — | 가능성 높음 |

적대 리뷰(codex · high · 레벨 2) = **FIX**, 4건 반영: 네이티브 락 「평생 락 → 업데이트 판정」 오탐(high) · 락 경로·형식 조건부(med) · `.old` 이름 바꾸기 조건(med) · 「공식 신호 없음」을 「문서화된 기계 판독 신호 없음」으로 좁히고 Homebrew/WinGet 도 설치 방식에 있음(low–med). 리뷰어도 `F()` 의 조건은 확정하지 못했다.

**감지 방식의 약점**
- 문서화되지 않은 minified 내부에 기댄다 — 버전이 바뀌면 조용히 안 맞는다(실패 모드 = 지금 상태로 돌아감).
- 설치 방식 둘 × 락 위치 분기를 다 따라가야 한다.
- 데몬 크래시 · 재부팅 · 작업 관리자 종료는 못 막는다(Job Object 가 트리를 죽인다).

## 한계 · 공백

- 업데이터의 종료 신호 처리(선택지 B 의 효과)를 확인하지 못했다 — 실측 재현이 필요하다.
- VS Code 가 node-pty `kill()` 앞에 유예를 두는지 확인하지 못했다.
- herdr Windows 에서 `ClosePseudoConsole` 이 250ms 대기보다 먼저 일어나는지 · herdr 서버 종료 시 모든 패인이 drop 되는지 확인하지 못했다.
- orca · tmux · cline 클론은 얕거나 낡았다(원격과 다를 수 있다).
- #65478 은 npm 전역 설치 보고다. 네이티브 설치의 Windows 교체 방식은 공식 자료에 없다.
- 동시 인스턴스끼리의 업데이트 경합 전용 보고는 찾지 못했다.

## 적대 리뷰

cross-family 리뷰어(codex · effort high · 레벨 2 · web_search) 판정 = **FIX**. 지적 6건 전부 반영했다.

| # | 지적 | 판정 | 반영 |
|---|---|---|---|
| 1 | 「9곳 전부 첫 단계부터 강제」는 범위 과장 — vibe-kanban 은 stdin `Interrupt` + 5초 대기, tmux·cline 은 Windows 대상 아님 | 수용(코드 근거) | 결론 1 을 「콘솔 이벤트 먼저는 못 찾음 · 프로토콜 끊기 먼저는 있음」으로 좁힘 · 선택지 B' 추가 |
| 2 | Windows Terminal 이 콘솔 닫기 먼저인데 「선례 없음」 결론에서 빠짐 | 수용 | 결론 2 신설 |
| 3 | #65478 의 교체 순서는 보고자 추정이고 실행 중 세션 생존은 #90233 과 갈림 | 수용(`gh` 로 #90233 확인) | 3절 표 수정 · 확신도 강등 |
| 4 | `ClosePseudoConsole` 은 24H2 이전 **무기한** 대기 가능 · 24H2 부터 즉시 반환 → 「수 초」는 틀림 | 수용(MS 문서) | 선택지 B 비용 수정 |
| 5 | `DISABLE_AUTOUPDATER` 주입은 Engram 자식만 끄고 공유 설치본의 교체 창은 남음 | 수용(논리) | 선택지 C 효과·비용 수정 |
| 6 | 누락 — #90233(열림) · #86295(네이티브 손상) · #65478 은 봇이 닫음 | 수용(`gh` 로 셋 다 확인 · 닫은 주체 `github-actions[bot]`) | 결론 4 · 3절 표에 추가 |

리뷰어는 「supervisor 아래에서 claude 를 돌릴 때의 공식 종료 보장」을 찾지 못했다고 보고했다(기권 — 반박 아님).
