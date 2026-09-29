# claude 훅 도중 끊기가 약 300 초 멈춘다 — Windows 에서 살아남은 훅 자식이 출력 파이프를 쥔다 (2026-09-29)

- **상태:** 조사 완료(light · 적대 리뷰 없음) · 결정 = 사용자 2026-09-29(§7) · TRD·구현 전 · 추적 = `docs/tracking.md` T-40. 수집자 보고는 메인이 대조했고, 메인이 줄을 직접 연 것만 ✓ 로 표시했다 — 표시 없는 `파일:줄`·서술은 수집자 판독뿐이다(가능성 높음 상한).
- **날짜:** 2026-09-29
- **버전:** claude `2.1.283` — 증상 세션과 실험 로그 전부(`system/init` 의 `claude_code_version`). Git Bash = MSYS2 `msys-2.0.dll` 3.6.6.
- **왜:** 직전 세션 F2 GUI 실측에서 보낸 직후 Esc 가 약 300 초 풀리지 않았다(`docs/process/step-log.md` S21 챗 화면 「구현 마무리 · GUI 실측」 줄 — 1 회 관측 · 다음 세션 논의로 넘겼다).
- **방법:** ① 우리 스폰 인자 그대로 띄운 stream-json(`-p`) 하네스로 훅 조합을 바꿔 가며 끊기(실험 1) ② 대화형 TUI 파일럿 1 회(실험 2) ③ 고아만 죽이는 하네스(실험 3) ④ upstream 이슈 조회 ⑤ 피어 조사 2 라운드(수집자 → 메인 스팟 대조).
- **출처(세션 스크래치 — 커밋하지 않음 · 휘발):** `C:\Users\<user>\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard-wt1\8717a9e4-353a-4963-8d10-3e638811be32\scratchpad\`
  - 실험 1 = `hookrepro\driver.cjs` · `matrix.cjs` · `logs\` · 실험 3 = `hookrepro\driver2.cjs` · `logs2\` · 실험 2 = `hookrepro-tty\tty.cjs` · `logs\`
  - 피어 라운드 2 판독본 = `peers\`(클론 7 개) · `cca\`(claude-code-action 클론) · `ext\`(VS Code 확장 번들) · `opcode.rs` · 라운드 1 의 Zed 어댑터 사본 = `acp.ts`
  - 라운드 1 의 t3code · vibe-kanban · paseo 는 로컬 참조 클론(`I:\Engram_Workspace\opensource\`)이다 — `파일:줄` 은 그 HEAD(`t3code 5378f87f9` · `vibe-kanban 78580443` · `paseo bbe3f17`) 기준이고, 이 기록을 쓰며 ✓ 줄의 내용을 다시 맞췄다.
- **확신도 범례:** 확실(실측 반복 · 또는 1차 자료 + 메인 대조) · 가능성 높음(단발 실측 · 판독 · 둘이 맞물린 해석) · 불확실(추론뿐). 주장마다 **실측 / 판독 / 추론** 을 함께 적는다.

## 0. 한 줄 결론

> ★**Windows 의 claude 는 훅 도중 끊기를 받으면 훅을 `taskkill /T /F` 로 죽이는데, 그 틈에 Git Bash 가 fork 한 자식이 살아남아 훅의 출력 파이프를 약 300 초 쥐고, CLI 는 그 파이프가 닫힐 때까지 시한 없이 기다린다 — 그래서 우리 모드(stream-json `-p`)에서 끊긴 턴의 `result` 가 약 300 초 늦는다.**★ 그 잔여물만 죽이면 약 0.2–0.4 초 안에 턴 끝이 오고 claude 는 산 채 다음 턴을 받는다(3/3). 대시보드가 자기 Job 안의 그 잔여물만 죽이기로 했다(사용자 2026-09-29 · §7).

## 1. 증상 (실측 · 직전 세션 F2 GUI 실측)

- 채팅(stream-json) 모드에서 보낸 지 약 0.2 초 만에 Esc → 끊기 응답은 왔는데 `result` 는 약 300 초 뒤에 왔다. 그 턴의 `hook_cancelled` 지속 = 300461 ms · 훅 = `handoff-trigger.sh`(사용자 전역 플러그인 훅).
- 그동안 Esc 는 무시됐고(「중단하는 중…」 동안 — ADR-0244) 대기 입력이 멈췄다.
- 에이전트를 죽이고 다시 열자 회복했고 대화는 그대로였다.
- 같은 세션에서 훅 도중의 다른 끊기 4 번은 0.6–0.8 초에 끝났다 — 매번이 아니라 경합이다(§2 ⑦).

## 2. 원인

| # | 주장 | 근거 | 확신도 |
|---|---|---|---|
| ① | CLI 는 훅 취소를 `taskkill /PID <bash> /T /F` 로 한다 — **두 번**(둘째는 약 1.5 초 뒤) | 실측(실험 1) | 확실 |
| ② | 트리 걷기 도중·뒤에 만들어진 Git Bash fork 자식이 살아남는다 — 부모 죽음 · 스레드 2 개 대기 · CPU 약 0.09 초 · 자식 없음. 약 300 초 뒤 `bash.exe: *** fatal error in forked process - WFSO timed out after longjmp` 를 남기고 끝난다 | 실측(실험 1 · 실험 3 에서 고른 고아의 모양은 §3-3) | 확실(살아남는 모양) · 가능성 높음(생성 시점 = 트리 걷기 도중·뒤) |
| ③ | 훅 스크립트는 자기 줄에서 막혀 있지 않았다 | 실측(실험 1) | 확실 |
| ④ | ★CLI 는 훅의 stdout/stderr 파이프가 **전부** 닫힐 때(자식 `close` 사건)까지 기다리고 상한이 없다★ · 선언한 훅 timeout 타이머는 맨 위 셸이 끝나면 지워진다 — 그래서 선언 timeout 이 이 멈춤을 끊지 못한다 | 실험 3(고아만 죽이면 곧바로 `result`) · 실험 1 B·C(살아남은 자식이 끝날 때까지 기다림) · upstream #85250 collaborator 서술(§5)이 맞물린다 | 가능성 높음 |
| ⑤ | 300 초 = MSYS fork 핸드셰이크 대기 상한 | **추론** — 오류 문구 + 시각뿐. MSYS 런타임 소스는 읽지 않았고 300 초를 적은 외부 출처도 없다 | 불확실 |
| ⑥ | 멈춤이 없어도 끊기는 살아남은 훅 자식을 기다린다(`sleep 3` → 약 3 초 · `sleep 20` → 약 20 초) | 실측(실험 1 B · C · B20 · C20) | 확실 |
| ⑦ | 경합이다 — 훅 도중 끊기마다 나는 것이 아니다 | **추론** — 실험 1 E 는 7 번 중 2 번 · 증상 세션의 다른 끊기 4 번은 0.6–0.8 초 · E-late(3 초 뒤 끊기)는 멈춤 없음 | 가능성 높음 |
| ⑧ | 고아 자체는 claude-on-Windows 일반이고, 눈에 보이는 300 초 턴 멈춤은 우리가 쓰는 `-p` stream-json 에서 드러난다(TUI 는 아니다) | 실측 n=1(실험 2) | 불확실(잠정) |

## 3. 실험

### 3-1. 실험 1 — 비대화형 (stream-json `-p`)

**설정:** 대시보드 스폰 인자 그대로 + `--model haiku --include-hook-events`. 끊기 줄 = `{"type":"control_request","request_id":"interrupt:<uuid>","request":{"subtype":"interrupt"}}`.

| 경우 | 설정 | 끊기 → `result` (ms) |
|---|---|---|
| A | 훅 없음 | 11 · 10 |
| B | `cat` 으로 stdin 읽기 → `sleep 3` | 3237 · 3125 |
| C | `sleep 3`(stdin 안 읽음) | 3028 · 3037 |
| B20 / C20 | `sleep 20` | 20109 / 20021 |
| D | `handoff-trigger.sh` 하나만 | 11 번 · 6–365(멈춤 없음) |
| **E** | 실제 환경(`UserPromptSubmit` 훅 4 개) · 보낸 뒤 200 ms 끊기 | 300619 · 1407 · 492 · 502 · 548 · 300284 · 491 → ★**7 번 중 2 번 멈춤**★ |
| E-late | 실제 환경 · 보낸 뒤 3 초 끊기 | 7 · 10 |
| F / F4 | 무해한 fork 다발 훅(F = 하나 · F4 = 여럿) | 8 번 · 멈춤 없음 |

- 멈춘 두 번의 메커니즘 = §2 ①–④. 멈춤 없는 B · C 계열이 ⑥ 이다.

### 3-2. 실험 2 — 대화형 TUI (파일럿 1 회 뒤 멈춤)

| 항목 | 관측 |
|---|---|
| 설정 | 실제 환경 · Enter 뒤 199 ms 에 Esc |
| 프롬프트 복귀 | 250 ms 이하 — 메시지는 버려지고 transcript 기록 없음 |
| 이어 보낸 「say ok」 | 1.5 초에 답 |
| 고아 | 같은 모양 — `wiki-preconsult.sh` 의 bash 가 약 300 초 살았고 stderr 에 fork 오류 |

- **해석(n=1 · 잠정):** 고아는 claude-on-Windows 일반이고, 눈에 보이는 300 초 턴 멈춤은 `-p` stream-json(대시보드가 쓰는 모드)에서 드러나며 TUI 에서는 아니다. TUI 에서 제출 전 훅 도중의 Esc 는 다른 코드 경로일 수 있다.

### 3-3. 실험 3 — 고아만 죽이기

**설정:** 감지 = `control_response` 가 왔는데 10 초 동안 `result` 없음. 고르기 = 살아 있고 · claude 가 아니고 · 기록해 둔 부모가 죽은 프로세스. 죽이기 = `taskkill /PID x /F`(`/T` 없음). ★**이 고르기(PID 부모 기록)는 채택안이 아니다 — §4**★.

| 시행 | 고른 것 | 죽이기 시작 → `result` | 이어 보낸 턴 |
|---|---|---|---|
| E r12 | bash `wiki-preconsult.sh` | 247 ms | ok · 2399 ms |
| E r20 | bash `handoff-trigger.sh` | 223 ms | ok · 2078 ms |
| Emcp r1 | bash 2 개 | 약 420 ms | ok · 2069 ms |

- **3/3 풀렸다** · `claude.exe` 는 살아 있었고 같은 프로세스가 다음 턴을 받았다(실측 · n=3).
- 멈춤 없는 18 번: 끊기 뒤 394–918 ms 에 `result`.
- MCP stdio 서버(부모 `claude.exe` 가 살아 있다)는 고르지 않았다.
- 고른 고아의 모양: `"C:\Program Files\Git\usr\bin\bash.exe" <hook>.sh`(fork 사본이라 부모와 명령줄이 같다) · 부모 bash 죽음 · 스레드 5 개 전부 대기 · CPU 16–31 ms.

## 4. ★사고 기록★ — 실험 정리가 남의 프로세스를 죽였다

- **무엇이 일어났나(실측):** 실험 워커의 끝 정리가 **PID 부모 연결**로 프로세스를 골라 강제 종료했고, 그 안에 우리와 무관한 프로세스가 들었다 — MSVC `link.exe`(pid 16924 · 약 05:46:29Z) · `vctip.exe`(pid 10432 · 약 05:40:47Z)(다른 워크트리의 빌드였을 가능성이 높다) · 주인을 모르는 `sleep.exe` 13 개 · `conhost.exe` 2 개 · `bash.exe` 2 개.
- **고르기 자체도 안전하지 않았다(실측):** PID 부모 연결은 「부모보다 뒤에 만들어졌나」 검사를 더해도 **PID 재사용** 때문에 다른 순간에 남의 프로세스를 골랐다 — 턴 전 모의 실행 21 번 중 4 번 · `result` 시점 18 번 중 3 번.
- ★**규칙 — 「PID 부모만으로 고르지 말 것 — Job 명단 + 생성 시각」**★: Windows 에서는 부모 PID 만으로 프로세스를 고르지 않는다. 커널이 추적하는 소속(Job Object 명단)과 프로세스별 생성 시각을 쓴다. §7 의 고르기가 이 규칙 위에 서 있다.

## 5. upstream 이슈 (존재 확인 2026-09-29)

| 이슈 | 상태 · 날짜 | 내용 |
|---|---|---|
| [anthropics/claude-code#96476](https://github.com/anthropics/claude-code/issues/96476) | 열림 · 2026-09-23 | 훅 timeout 이 Git Bash 런처만 죽이고, 고아가 된 손자가 stdout 파이프를 쥐어 세션이 멈춘다 · 고아를 죽이면 곧바로 풀렸다 · 2.1.283 에 대한 댓글이 있다 |
| [anthropics/claude-code#96945](https://github.com/anthropics/claude-code/issues/96945) | 열림 · 2026-09-25 | 고아 자손이 훅 stdout 을 쥐는 동안 도구 호출이 풀리지 않는다 · 맨 Python 훅에서도 난다(bash 만의 문제가 아니다) |
| [anthropics/claude-code#77078](https://github.com/anthropics/claude-code/issues/77078) | 열림 | 훅 프로세스가 SUSPENDED 로 남는다 · 사용자 감시기(ResumeThread · CPU 0 으로 오래된 훅 프로세스 죽이기)가 13–61 분 멈춤을 12–16 초로 줄였다 |
| [anthropics/claude-code#67888](https://github.com/anthropics/claude-code/issues/67888) | 닫힘 | claude 의 정리 `taskkill /PID x /T /F` 자체가 매달리고 쌓인다 |
| [anthropics/claude-code#85250](https://github.com/anthropics/claude-code/issues/85250) | 열림 · 라벨 reproduced | Anthropic collaborator(2026-08-25): 「completion of the hook batch waits on the hook's output pipe reaching EOF rather than on the timeout」 · 선언 timeout 이 배치를 끝내야 한다. ★CHANGELOG 2.1.284 까지 수정 없음★ |
| MSYS/Cygwin 「WFSO timed out after longjmp」 | — | 옛 cygwin 메일링 리스트 보고(fork 도중 외부 kill · 긴 멈춤). ★**300 초의 출처는 없다**★ |

## 6. 피어 처리 방식 — 끊기 뒤 `result` 가 안 올 때

★**claude 를 살려 둔 채 자손만 죽이는 피어는 없다** — 그렇게 하는 것은 사용자 감시기(#77078)뿐이다.★

### 6-1. 라운드 1 (✓ = 메인이 줄을 직접 대조)

| 피어 | 처리 | 근거 |
|---|---|---|
| Zed `claude-agent-acp` | 30 초 유예 뒤 턴을 로컬에서 「cancelled」 로 닫는다 · 죽이지 않는다 · 「a new session may be required」 를 로그에 남긴다 | `acp-agent.ts:354` ✓(`DEFAULT_FORCE_CANCEL_GRACE_MS = 30_000`) · `:6868` ✓ · 계기 이슈 #680 |
| t3code(`pingdotgg/t3code`) | `interruptTurn` → `stopSessionInternal` → `query.close()` · 주석 「interrupt() can acknowledge while resumed background tasks keep the CLI alive」 ✓ · 커밋 `4d12e5222`(#5891 · 2026-08-23). 다음 메시지 = 멈춘 세션을 본 reactor 가 `startProviderSession` → `ProviderService` 가 영속된 `resumeCursor` 로 `--resume` | `apps/server/src/provider/Layers/ClaudeAdapter.ts:5289-5296` ✓ · `apps/server/src/orchestration/Layers/ProviderCommandReactor.ts:815` ✓ · `apps/server/src/provider/Layers/ProviderService.ts:1459` ✓ |
| vibe-kanban(`BloopAI/vibe-kanban`) | `control_request` 끊기 · 컨테이너 정지는 5 초 기다린 뒤 「Graceful shutdown timed out … force killing」 → `kill_process_group`(`command_group` · Windows 는 Job Object — crate 문서로만, 소스는 안 읽었다) · README 가 서비스 종료(sunsetting)를 알린다 | `crates/local-deployment/src/container.rs:1438` ✓ |
| paseo(`getpaseo/paseo`) | 낙관적 `turn_canceled` · 3 초 시한의 끊기 · 늦게 온 결과는 버린다(「Suppressing stale non-success result…」) · 트리 죽이기(`tree-kill` = Windows 에서 `taskkill /T /F`) | `packages/server/src/server/agent/providers/claude/agent.ts:3790` ✓ |
| opcode | 죽이기만 한다(`taskkill /F` · `/T` 없음) | 수집자 판독 |
| 공식 Python SDK | 끊기 응답 시한 60 초뿐 · `close()` 는 terminate → kill 을 5 초 간격으로 올린다 · 직계 자식만 | 수집자 판독 |
| 공식 TS SDK | 상한이 문서에 없다 · 열린 이슈 [#425](https://github.com/anthropics/claude-agent-sdk-typescript/issues/425) · [#429](https://github.com/anthropics/claude-agent-sdk-typescript/issues/429) · [#444](https://github.com/anthropics/claude-agent-sdk-typescript/issues/444) · [#352](https://github.com/anthropics/claude-agent-sdk-typescript/issues/352) · [#279](https://github.com/anthropics/claude-agent-sdk-typescript/issues/279) | `anthropics/claude-agent-sdk-typescript` |

### 6-2. 라운드 2 (✓ 외에는 수집자 판독)

| 피어 | 처리 | 근거 |
|---|---|---|
| 공식 VS Code 확장 2.1.284 | `interruptClaude` = `query.interrupt()` · 타이머 없음 · 닫기 → `claude.exe` pid 만 SIGKILL(win32 2 초 + 5 초) · `taskkill` 0 회 | 번들 판독 · `taskkill` 출현 수 0 ✓ |
| claude-code-action(`anthropics/claude-code-action`) | 끊지 않는다 · 잡의 `timeout-minutes` 에 기댄다 | `base-action/src/run-claude-sdk.ts:190-210` |
| Claude Desktop Code | 15 분 유휴 재활용 → `taskkill /T /F` | 제3자 역공학 · #68625 |
| claudecodeui(`siteboon/claudecodeui`) | 로컬에서 닫는다 | — |
| happy(`slopus/happy`) | 중단 = 죽이기 + resume | — |
| sculptor(`imbue-ai/sculptor`) | 끊기 → 5 초 → 프로세스 그룹 SIGTERM → 2 초 → SIGKILL → 2 초(POSIX 만) | — |
| emdash(`generalaction/emdash`) | `claude-agent-acp` 를 거친다(30 초) | — |
| crystal(`stravu/crystal`) | `taskkill /F /T` + 종료 뒤 PPID 훑기 — 부모가 죽은 고아는 놓친다 | — |
| agentapi(`coder/agentapi`) · claude-squad(`smtg-ai/claude-squad`) · JetBrains | 대화형 PTY | — |
| Cline v3.40 | execa timeout 10 분 | — |

## 7. 결정 (사용자 · 2026-09-29)

- **판단:** claude 의 일반 결함이지만 우리 모드에서 드러난다 → 대시보드가 완화한다.
- **채택:** 우리 Job Object 안의 멈춘 잔여물만 죽이고 claude 는 살린다(재시작이 아니다).
- **고르기:**
  - 실제로 끊기를 보낸 Esc 에서 시각을 적는다 — 「중단하는 중…」 동안의 되풀이 Esc 는 무시되므로(ADR-0244) 적히지 않고, 새 턴을 위한 뒤 Esc 는 덮어쓴다 · 따로 지우는(reset) 일은 필요 없다.
  - N 초 뒤, **그 끊기의 턴 끝이 안 왔고 그리고 가장 최근 Esc 에서 N 초 이상 지났으면**: Job 멤버 중 claude 가 아니고 · claude 의 직계 자식이 아니고 · 적어 둔 시각 뒤에 만들어졌고 · 아직 살아 있는 것을 죽인다.
  - 고아 여부 · 「bash」 인지 · CPU 상태는 주 기준이 아니다 — 정당한 고아(예: dev 서버)가 있고, 한가한 프로세스는 얼어 보인다.
- **시계:** 커널의 생성 시각을 같은 시계(시스템 시각)에서 뽑은 타임스탬프와 비교한다.
- **N = 3 초:** 정상은 1.4 초 이하로 쟀다 · 잘못 발화해도 맞는 것은 Esc 뒤에 생긴 프로세스뿐이다. 상수는 한 곳에 두고, 실측에서 세게 검증한다.
- **OS 조각:** Windows 전용 조각은 작게 둔다(Job 멤버 명단 + 하나 끝내기) · 다른 OS 는 무동작. 사용자가 나중에 별도 플랫폼 모듈을 둘 계획이 있어(아직 구상 단계) 그 OS 조각을 그리로 옮길 수 있게 짠다.
- **일정:** 이 브랜치 `v0.3.3/feat/chat-ux` 에 master 머지 전에 넣는다 → 최종 실테스트 → 머지.
- **TRD 에서 물을 것(미결):** ① 정리가 일어났을 때 화면에 고지할지 ② 그래도 턴 끝이 안 오면 대체(재시작)를 둘지.

## 8. 한계

- light 조사 · 적대 리뷰 없음. ✓ 가 없는 `파일:줄`·서술은 수집자 판독뿐이다(가능성 높음 상한).
- ★300 초의 출처를 모른다★ — MSYS 런타임 소스를 읽지 않았고 외부 출처도 없다(§2 ⑤).
- 실험 2(TUI)는 파일럿 1 회 뒤 멈췄다 — TUI 쪽 서술은 전부 잠정이다.
- 실험 3 은 3 번이다 — 풀림은 3/3 이지만 표본이 작다. 그 실험의 고르기는 채택안이 아니며(§4), §7 의 고르기는 아직 재지 않았다.
- 경합 빈도의 근거는 실험 1 E(2/7)와 증상 세션뿐이다.
- Windows 에서만 쟀다.
- vibe-kanban 의 Windows Job Object 는 crate 문서로만 봤고, TS SDK 의 상한은 「문서에 없다」는 부재 확인이다.
- 원시 로그·하네스는 세션 스크래치에만 있고 커밋하지 않았다(휘발).
