# codex 지시서 통로 전수 · 읽는 시점 실측 — 「띄울 때 만들고 띄운 뒤 지우기」가 되나 (2026-10-20)

- **상태:** 조사 완료 · 결정 전(프리셋 지시서 · 스킬을 임시 파일로 두는 안의 입력)
- **방법:** 가짜 모델 서버 + 센티넬(아래 「방법」 — 앞선 보고서 방법을 이어 받았고 재현 요점을 여기 남긴다) · 소스 대조(openai/codex 태그 `rust-v0.156.1` = `b412ff32`, 2026-09-22) · codex 실행 13회(버전 · `--help` 5 · `codex exec` / `exec resume` 7 · `codex app-server` 1) · 실 모델 호출 0
- **대상판:** codex-cli **0.156.1**(npm · Windows 11)
- **질문:** ① `AGENTS.md` / `AGENTS.override.md` 말고 터미널 모드가 따르는 지시서 통로가 있나 — 특히 파일이 작업 폴더 **밖**에 있거나 프로세스 단위인 것 ② `AGENTS.override.md` 를 언제 읽나(시작 · 턴마다 · 압축 뒤 · 이어받기) ③ 스킬 본문은 쓸 때 읽나 · 터미널 모드에 프로세스 단위 스킬 폴더가 있나
- **앞선 보고서(여기서 다시 재지 않은 것은 그쪽이 정본):** [`codex-instruction-injection-2026-09-20.md`](codex-instruction-injection-2026-09-20.md)(통로 · 상한 · cmd 명령줄 벽) · [`external-skill-dir-loading-claude-codex-2026-10-10.md`](external-skill-dir-loading-claude-codex-2026-10-10.md)(스킬 발견 위치 · 이어받기의 `developer_instructions` 무시)
- **확신도 범례:** 확실 = 실측(또는 실측 + 소스 교차) · 가능성 높음 = 소스만 · 불확실 = 미검

## 결론

1. **작업 폴더 밖 · 프로세스 단위 통로가 셋 있다**(전부 터미널 경로 `codex exec` 로 실측):
   - **프로필 v2 파일 `-p <이름>`** → `$CODEX_HOME/<이름>.config.toml` 을 설정 층 하나로 얹는다. 그 안에 `developer_instructions`(여러 줄 · 길이 제한 없이 — cmd 명령줄 벽을 안 탄다) · `model_instructions_file` · 훅을 둘 수 있다. ★**자리는 작업 폴더가 아니라 `$CODEX_HOME`(실 `~/.codex`) 고정이다**★ — 이름은 `[A-Za-z0-9_-]` 만 받아 경로를 못 준다.
   - **`model_instructions_file`(절대경로 아무 데나)** → 시스템 프롬프트(`instructions`)를 **통째로 대체**한다. 설정을 읽을 때 내용을 읽고 **스레드 시작 때 세션 메타에 떠 둔다** — 파일을 지워도 이어받기 · 압축 뒤까지 시작 때 내용이 산다.
   - **SessionStart 훅** → 훅 명령이 아무 파일이나 출력하면 그 글이 developer 메시지로 실린다. **시작 · 이어받기 · 압축 뒤마다 다시 돈다**(그때의 파일 내용). 대가 = 훅 신뢰(`--dangerously-bypass-hook-trust` 또는 저장된 신뢰) · 2,500 토큰 넘으면 잘려 원문이 `%TEMP%\hook_outputs\…` 로 빠진다(`additionalContextLimit = 0` 이면 통째로 실린다 — 58 KB 실측).
2. **`AGENTS.override.md`(작업 폴더)는 스레드를 열 때 한 번 읽고 그 프로세스가 기억한다.** 턴마다 다시 읽지 않고(고쳐도 지워도 무시), **압축 뒤에도 기억한 내용을 다시 넣는다**(파일이 이미 바뀌었어도). 디스크를 다시 읽는 순간은 **이어받기(새 프로세스)** 와 — 소스상 — 같은 프로세스의 새 스레드(`/new` · `/clear` 등) · 작업 폴더나 신뢰 수준이 바뀔 때다. ★**이어받을 때 파일이 없으면 「앞서 준 AGENTS.md 지시는 더 이상 적용되지 않는다」는 글을 모델에게 덧붙인다**★ — 지운 것이 조용히 무시되는 게 아니라 **명시적으로 철회된다.**
3. **전역 `$CODEX_HOME/AGENTS.md` 는 반대로 매 요청 다시 읽는다** — 바뀌면 대체 블록, 지우면 제거 블록이 그 자리에서 붙는다. 사용자의 모든 codex 세션이 같이 보는 파일이라 에이전트별 통로가 아니다.
4. **스킬:** 목록(이름 · 설명 · 경로)만 싣고 **본문은 쓸 때 디스크에서 읽는다**(시작 요청에 본문 센티넬 0). 목록은 **매 요청 다시 만든다** — 정션을 지우면 다음 요청에서 그 스킬이 목록에서 빠지고 `$이름` 멘션은 조용히 버려진다. **터미널 모드에 프로세스 단위 스킬 폴더는 이 판에도 없다**(설정 키 · 인자 · 환경변수 0 — 앞선 보고서 결론 유지).
5. **「띄울 때 만들고 띄운 뒤 지우기」 판정**
   - **지시서(`AGENTS.override.md`) = 조건부로 된다.** 조건 둘 — ① **스레드 시작이 끝난 뒤** 지울 것(TUI 는 띄운 뒤 비동기로 스레드를 연다 · 그 순간을 밖에서 아는 신호는 찾지 못했다) ② **이어받기(그리고 새 스레드) 전마다 같은 내용으로 다시 만들 것** — 같은 내용이면 블록이 더 붙지 않는다(실측). ①을 못 지키면 그 에이전트는 프리셋 없이 시작하고, ②를 못 지키면 이어받은 순간 철회 글이 붙는다.
   - **스킬(정션) = 안 된다.** 세션 내내 있어야 한다 — 지운 다음 요청부터 목록에서 빠진다.
   - **작업 폴더를 아예 안 건드리는 길 = 프로필 v2 + `developer_instructions`.** 단 ★**이어받을 때도 같은 프로필을 줘야 압축 뒤 살아남는다**★ — 안 주면 압축이 프리셋을 지운다(실측 R5).

## 통로 표 — 0.156.1 터미널 모드

`작업 폴더 밖` = 그 통로의 **파일**을 작업 폴더 밖에 둘 수 있나. 「이어받기」 = 새 프로세스로 `resume`. 「압축 뒤」 = 같은 프로세스 안에서 압축한 다음 요청(로컬 압축 실측 · 원격 압축은 소스 — 「한계」).

| 통로 | 파일 자리(작업 폴더 밖?) | 덧붙임/대체 | 시작 때 | 턴마다 | 이어받기 | 압축 뒤 | 근거 | 확신도 |
|---|---|---|---|---|---|---|---|---|
| `AGENTS.override.md` > `AGENTS.md`(프로젝트 루트~작업 폴더 층마다) | **X** — 작업 폴더~프로젝트 루트 | 덧붙임(user · 전역 AGENTS.md 와 한 블록) | 스레드 열 때 읽어 기억 | **안 읽는다** | 다시 읽는다 — 바뀜 = 대체 블록 · 없음 = **철회 블록** · 같음 = 블록 없음 | **기억한 내용**을 다시 넣는다(디스크 무관) | 실측 R1~R3 · R6~R7 · A | 확실 |
| `project_doc_fallback_filenames` | X — 이름만(경로 거절) · AGENTS.md 가 없을 때만 | 덧붙임 | 위와 같음 | 위와 같음 | 위와 같음 | 위와 같음 | 앞선 보고서 실측 · 소스 | 확실 |
| 전역 `$CODEX_HOME/AGENTS.override.md` > `AGENTS.md` | **O** — 단 `$CODEX_HOME` = 사용자 전체 공용 | 덧붙임(위 블록의 앞부분) | 읽는다 | **매 요청 다시 읽는다** — 바뀜 = 대체 · 지움 = 제거 블록 | 다시 읽는다 | 지금 디스크 내용 | 실측 R1~R3 · A t2·t3 + 상류 시험 | 확실 |
| `developer_instructions`(`-c`) | 파일 없음(값) — cmd 벽(8,191자 · 줄바꿈 잘림)을 탄다 | 덧붙임(developer · codex 자기 블록 앞) | 실린다 | 고정(한 번) | ★새 값은 그 자리에 안 실린다(옛 값이 기록에 남는다)★ | ★**지금 프로세스 설정값**을 다시 넣는다★ — 이어받을 때 안 줬으면 사라진다 | 실측 R2 · R5 · A t3 + 앞선 보고서 | 확실 |
| **프로필 v2 `-p <이름>`** 의 `developer_instructions` | **O** — `$CODEX_HOME/<이름>.config.toml`(경로 지정 불가) | 위와 같음 | 실린다(여러 줄 실측) | 위와 같음 | 위와 같음 — 프로필 값도 `-c` 와 똑같이 그 자리에선 버려진다 | 위와 같음(이어받을 때 준 **새** 값이 압축 뒤 나타났다) | 실측 R1 · R2 | 확실 |
| `model_instructions_file` | **O** — 절대경로 아무 데나 | **대체**(`instructions` 칸) | 설정 읽을 때 내용 읽음 · 세션 메타에 사본 | 고정 | 키를 주면 그 파일(지금 내용) · ★**안 주면 시작 때 사본**(파일 삭제 · 변경 무관)★ | 칸이 달라 압축과 무관 | 실측 R1 · R2 · R3 · R5 · 롤아웃 `session_meta` | 확실 |
| `instructions`(설정 키) | 값 | 대체 | — | — | (소스: 위와 같은 우선순위) | — | 앞선 보고서 · 소스 | 가능성 높음 |
| **SessionStart 훅**(`[[hooks.SessionStart]]` — 프로필 · `-c` · `config.toml`) | **O** — 명령이 읽는 파일은 아무 데나 | 덧붙임(developer · `hooks.additional_context`) | 첫 턴 직전에 돈다 | 안 돈다 | `source=resume` 으로 다시 돈다(그때 파일 내용) | `source=compact` 로 다시 돈다 | 실측 R1 · R2 · R4 | 확실 |
| UserPromptSubmit 훅 | O(위와 같음) | 덧붙임(developer) | — | **사용자 차례마다** 돈다 → 매번 다시 쌓인다 | — | — | 소스(`hook_runtime.rs` 의 문맥 주입 경로) | 가능성 높음 · 미측 |
| 레거시 `profile = "x"` + `[profiles.x]` | — | — | ★**`developer_instructions` 칸이 없다**★ — `model_instructions_file` 만 있다 | | | | 소스 `config/src/profile_toml.rs` | 확실(소스) |
| `~/.codex/rules/*.rules` | — | **지시서 아님** — execpolicy 명령 승인 규칙(`codex exec --ignore-rules` 도움말) | | | | | `--help` · `docs/execpolicy.md` | 확실 |
| `compact_prompt` · `experimental_compact_prompt_file` | — | 지시서 아님 — 압축 요약을 만드는 프롬프트 | | | | | 소스 | 가능성 높음 |
| managed developer instructions | — | requirements(관리자) 층 전용 · 사용자가 못 단다 | | | | | 소스 `world_state/managed_developer_instructions.rs` | 가능성 높음 |
| 「thread instructions」 제공자 | — | 확장 API 내부 칸 · 이 판의 CLI · 설정 · app-server 인자에 노출 없음 | | | | | 소스 `agents_md_manager.rs` | 가능성 높음 |

- **`CODEX_HOME` 을 에이전트마다 바꾸는 길**은 앞선 보고서가 이미 막았다(인증 · 이어받기 조회가 따라 옮겨진다).
- **신뢰:** 작업 폴더가 **`untrusted` 로 명시돼 있으면** 프로젝트 AGENTS.md 를 아예 안 읽는다(`agents_md.rs:62` · 신뢰 미지정은 읽는다 — `config_toml.rs:578`).
- ★**지운 임시 파일의 내용도 디스크에는 남는다**★ — 롤아웃(`$CODEX_HOME/sessions/…jsonl`)이 `session_meta` 에 시스템 프롬프트 사본을, `world_state` 항목에 AGENTS.md 전문을 적는다(R1~R3 롤아웃 실측). 「지우기」가 덜어 주는 것은 **작업 폴더 오염**뿐이다.

## `AGENTS.override.md` 읽는 시점 — 실측

**R = `codex exec` / `exec resume`(터미널 경로) · A = `codex app-server` 한 프로세스(같은 프로세스의 여러 턴 · 압축).** A 는 TUI 와 같은 경로다 — TUI 는 `-c` · `-p` · `--dangerously-bypass-hook-trust` · `--strict-config` 중 하나라도 있으면 공유 데몬을 안 쓰고 **자기 프로세스 안에 app-server 를 띄워** 스레드를 연다(`tui/src/daemon_startup.rs:17-44` · 데몬 자동 기동 기능은 아직 실험 단계 · 기본 꺼짐).

| 실행 | 그때 디스크 | 요청에 실린 것 |
|---|---|---|
| R1 새 대화 | override = A · 전역 = A | 블록 하나: 전역 A + override A |
| R2 이어받기(새 프로세스) | override → **B** · 전역 → B | 옛 블록(A) 그대로 + **「이 AGENTS.md 지시가 앞서 준 것을 모두 대체한다」 + B 블록** |
| R2 같은 턴 안 압축 뒤 | — | 옛 블록 둘 다 사라지고 **B 블록 하나**가 다시 들어감(이 프로세스가 읽은 것) |
| R3 이어받기 | override · 전역 **삭제** | **「앞서 준 AGENTS.md 지시는 더 이상 적용되지 않는다」** 블록 |
| R5 이어받기 + 압축 | 삭제 그대로 | 압축 뒤 AGENTS 블록 **0**(철회 글도 함께 사라짐) |
| R6 이어받기 | override = C 새로 만듦 | C 블록(앞에 지시가 없던 상태라 대체 문구 없이) |
| R7 이어받기 | **지웠다가 같은 내용으로 다시 만듦** | ★**새 블록 0**★ — 기록된 사본과 글이 같으면 아무것도 안 붙인다(파일 시각 무관) |
| A thread/start 직후 **override 삭제** → t1 | 없음 | ★**t1 에 override A2 가 실렸다**★ — 스레드를 열 때 읽었다 |
| A t2 | override = B2 로 다시 만듦 · 전역 → B2 | 대체 블록 = **전역 B2 + override A2**(전역만 새로 읽고 프로젝트는 기억) |
| A 압축(`thread/compact/start`) → t3 | override B2 · 전역 B2 | 다시 넣은 블록 = **전역 B2 + override A2** — 디스크가 B2 인데도 기억한 A2 |

- **소스:** 프로젝트 문서는 스레드 시작 때(`core/src/session/session.rs:1436-1443`) 읽고, 매 요청 경계의 갱신(`core/src/session/mod.rs:3754`)은 **환경 선택(작업 폴더 등) · 신뢰 수준이 같으면 기억한 것을 쓴다**(`core/src/agents_md_manager.rs` 의 `refresh_repository`). 전역 파일 제공자는 부를 때마다 디스크를 읽는다(`codex-rs/codex-home/src/instructions/mod.rs`). 대체 · 철회 문구와 「같으면 무블록」 = `core/src/context/world_state/agents_md.rs` 의 `render_diff`. 상류 시험 `core/tests/suite/agents_md_refresh.rs`(`live_global_removal_preserves_repository_instructions`)가 같은 동작을 박아 둔다.
- **압축 뒤 재주입의 출처:** 압축은 기준점(직전 턴 문맥)을 지우고 다음 요청이 **지금 프로세스의 상태로** 첫 문맥을 통째로 다시 만든다(`core/src/session/mod.rs:4490-4530` · 원격 압축도 같은 함수를 쓴다 — `core/src/compact_remote_v2.rs:323-333`). 그래서 developer 지시 · AGENTS 블록은 「기록에 있던 것」이 아니라 **「지금 프로세스가 아는 것」**으로 바뀐다 — R2 에서 그 자리에선 버려졌던 이어받기 때 값(B)이 압축 뒤 나타났고, R5 에서 아무것도 안 준 이어받기는 압축 뒤 지시가 0 이 됐다. 앞선 보고서의 「압축이 developer 메시지를 전부 지운다」(JSON · 이어받은 프로세스)는 이 규칙으로 설명된다 — 그 프로세스에 `developerInstructions` 를 안 줬다.
- **지우는 시점의 함정(소스):** TUI 는 띄운 뒤 **비동기로** 스레드를 연다(`tui/src/app/startup.rs:16` `spawn_startup_thread_start`). 그리고 세션을 만들 때 **롤아웃 자리를 잡는 것이 AGENTS.md 를 읽는 것보다 먼저**다(`session.rs:1156` → `1436`) — 「롤아웃 파일이 생겼다」는 「읽었다」의 증거가 아니다. 첫 턴이 시작됐다면 스레드는 이미 열렸다(턴은 스레드 뒤에만 온다).
- **새 스레드(소스):** `/new` · `/clear` 같은 같은 프로세스의 새 스레드는 세션을 새로 만들어 디스크를 다시 읽는다(`session.rs:1860-1868` 의 시작 출처 `Startup`/`Clear`) — 그때 파일이 없으면 프리셋 없이 시작한다. 미측.

## 작업 폴더 밖 통로 — 실측

R1 한 번에 모든 통로를 켰다(프로필 `eng1` = `developer_instructions` 두 줄 + `model_instructions_file`(바깥 절대경로) + SessionStart 훅(`type <바깥>\hook.md`) · 전역 AGENTS.md · 작업 폴더 override · 정션 스킬). `--strict-config` 를 함께 줘 프로필의 세 키가 이 판의 정식 키임을 확인했다.

| 실행 | `instructions` | developer(묶음) | 훅 developer | 비고 |
|---|---|---|---|---|
| R1 새 대화 | MIF-A | PROF-DEV-A | HOOK-A | 스킬 설명 실림 · 본문 0 |
| R2 이어받기 · 파일 전부 B 로 · 같은 프로필 | **MIF-B**(키를 줬다) | PROF-DEV-A **만**(B 버림) | **HOOK-B 추가**(resume) | — |
| R2 압축 뒤 | MIF-B | **PROF-DEV-B**(A 사라짐) | **HOOK-B**(compact 로 다시 돎) | 사용자 말 두 줄 + 요약만 남음 |
| R3 이어받기 · 프로필 없이 | ★**MIF-A**★(시작 때 사본 — 파일은 B, 직전 이어받기는 B 였는데도) | 기록의 B | 없음 | — |
| R5 이어받기 · 프로필 없이 · 압축 | MIF-A | **0** | 0 | 시스템 프롬프트만 산다 |

- **R4 훅 크기:** 58,326자 파일을 같은 훅 둘로 냈다. 기본값 = **10,102자로 잘림**(머리 · 꼬리 보존) + 「Full hook output saved to: `C:\Users\…\AppData\Local\Temp\hook_outputs\<세션>\<id>.txt`」 · `additionalContextLimit = 0` = **58,326자 통째**(머리 · 꼬리 센티넬 둘 다). 문턱 기본 2,500 토큰의 정본 = `config/src/hook_config.rs:175-184`.
- **훅의 성질(소스):** 기능 `hooks` 는 안정 · 기본 켬. Windows 에선 `cmd.exe /C "<명령>"` 으로 돈다(`hooks/src/engine/command_runner.rs:435`) — **훅 명령도 cmd 를 탄다**(명령은 짧은 `type <경로>` 라 명령줄 벽은 안 걸린다). SessionStart 대기 출처는 시작 · 이어받기 · `/clear` · 압축 넷이고(`hooks/src/events/session_start.rs:24-38`) 압축 뒤 실행 지점은 `core/src/session/turn.rs:637`. 신뢰는 훅 정의(명령 포함)의 해시로 매긴다(`hooks/src/engine/discovery.rs:767-791`) — 에이전트마다 명령이 다르면 저장 신뢰로는 매번 새 승인이 필요하고, 프로세스 단위로 건너뛰는 길이 `--dangerously-bypass-hook-trust` 다(그 플래그는 매 실행 경고 두 줄을 낸다 · TUI 를 내장 모드로 돌린다). ADR-0216 이 걷어낸 「훅 기계장치」는 세션 id 수확용이었고 이 통로와 목적이 다르다.

## 스킬

| 실행 | 정션 | 요청 |
|---|---|---|
| R1 · A t1 | 있음 | 목록에 `engram-probe`(설명 센티넬) · **본문 센티넬 0** |
| R2 | 있음 · 본문만 B 로 바꿈 | 목록 그대로 · 본문 0(아무도 안 불렀다) |
| R3 · A t2 | **지움**(원본 폴더는 그대로) | **새 스킬 목록 블록이 덧붙고 `engram-probe` 가 빠졌다** · 같은 요청의 `$engram-probe` 는 **조용히 버려졌다**(본문 0 · 오류 없음) |

- **본문은 쓸 때 읽는다:** 목록 안내문이 「스킬을 쓰기로 했으면 그 `SKILL.md` 를 끝까지 연다」고 모델에게 시킨다(`ext/skills/src/catalog_prompt.rs:28`) — 모델이 셸로 정션 경로를 연다. `$이름` 멘션은 그 순간 codex 가 읽어 넣는다(앞선 보고서). 어느 쪽이든 **그 시점에 링크가 살아 있어야** 한다.
- **목록은 매 요청 새로 만든다:** 스킬 목록이 「world state」 한 칸이라 요청마다 지금 디스크로 다시 만들고 바뀌면 블록을 덧붙인다(`ext/skills/src/world_state.rs`). 지운 다음 요청에서 바로 빠진 것이 그 결과다.
- **프로세스 단위 스킬 폴더:** `[skills]` 설정은 `bundled` · `include_instructions` · `max_context_tokens` · `config`(이미 발견된 것 켜고 끄기)뿐이고 루트를 더하는 키 · 인자 · 환경변수가 없다(`config/src` 의 `SkillsConfig`) — 앞선 보고서 결론 그대로. 바깥 루트는 app-server `skills/extraRoots/set` 뿐이다.

## 방법 — 재현 요점

ADR-0216 이 「방법의 서술이 어디에도 남지 않았다」고 적은 공백을 메우려고 요점을 남긴다(스크립트 자체는 세션 스크래치라 남지 않는다).

- **가짜 서버:** node `http` 로 `127.0.0.1:<포트>` 에서 `POST …/responses` 를 받아 본문을 파일로 떨군다. 답은 SSE 세 사건 — `response.created {response:{id}}` → `response.output_item.done {item:{type:"message",role:"assistant",content:[{type:"output_text",text}]}}` → `response.completed {response:{id,usage:{input_tokens,output_tokens,total_tokens,…}}}`. `GET …/models` 는 빈 목록.
- **설정(스크래치 `CODEX_HOME`):** `model_provider = "fake"` · `[model_providers.fake] base_url = "http://127.0.0.1:<포트>/v1"` · `wire_api = "responses"` · 재시도 0 · `[projects.'<작업 폴더>'] trust_level = "trusted"`. 작업 폴더는 `git init`(프로젝트 루트 표식). 모델 이름이 카탈로그에 없으면 경고 한 줄 뒤 기본 메타데이터로 돈다(무해).
- **같은 턴 안 압축 일으키기:** `model_auto_compact_token_limit = 200000` 을 두고, 프롬프트에 표식이 있으면 서버가 **없는 도구 호출**(`function_call` · 이름 `probe_noop`) + `total_tokens = 900000` 으로 답한다 → codex 가 도구 오류를 돌려주고 「후속 필요 + 한도 초과」로 **턴 안 압축**(로컬 — 요약 요청이 `You are performing a CONTEXT CHECKPOINT COMPACTION…` 으로 온다) → 그다음 요청이 압축 뒤 문맥이다. 마지막 입력이 `function_call_output` 이면 평범한 글로 답해 무한 반복을 막는다.
- ★**외부 전송 덫**★ — `HTTP(S)_PROXY` · `ALL_PROXY` 를 `127.0.0.1:<포트+1>` 의 기록용 프록시로 돌리고 `NO_PROXY=127.0.0.1,localhost`. 프록시는 `CONNECT` 를 적고 403 으로 끊는다. **모델 요청 말고도 codex 가 혼자 나가려 한다** — 8 프로세스에서 `chatgpt.com` 15 · `github.com` 8 · `api.github.com` 7 · `ab.chatgpt.com` 7(전부 TLS 이전 거절 → 페이로드 0). 덫 없이 「외부 전송 0」을 말하면 모델 요청만 본 것이다.
- **실 `~/.codex` 불변 확인:** 세션 jsonl 125 · `session_index.jsonl` 509줄 · `history.jsonl` 150줄 — 전 · 후 같음. 스크래치 `CODEX_HOME` 이름은 `h1` · `h2`(앞선 보고서의 `codexhome` 충돌 경고를 따름).

## 한계 · 미측

- **대화형 TUI 미측** — PTY 를 몰 수단(node-pty 등)이 이 PC 에 없어 `codex exec` 와 `app-server` 로 대신 쟀다. 근거는 코드 경로가 같다는 소스 판독(위 내장 모드 조건)이다.
- **원격 압축(OpenAI 제공자 · 실제 우리 사용 형태) 미측** — 가짜 제공자는 로컬 압축만 탄다. 원격 압축도 같은 재주입 함수를 쓰는 것은 소스로만 확인했다(가능성 높음).
- **`/new` · `/clear` · 작업 폴더 변경 · 하위 에이전트의 재읽기** — 소스만.
- **TUI 가 스레드를 연 순간을 밖에서 아는 신호** — 찾지 않았다. 「지우기」를 하려면 이 신호가 먼저 필요하다.
- **UserPromptSubmit 훅 주입 · 훅 신뢰를 저장해 쓰는 길** — 미측.
- **모델이 통로별 지시를 얼마나 다르게 따르는지** — 실 모델 호출이 필요해 안 돌렸다(앞선 보고서와 같은 공백).
