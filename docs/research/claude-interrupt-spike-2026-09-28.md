# claude stream-json 끊기(interrupt) 스파이크 — B2 (S1–S8) (2026-09-28)

- **상태:** 실측 완료 · TRD 반영 전. `docs/process/S21-chat-ux/trd.md` §3-5 의 표(S1–S8)와 「S1 기록 추가」를 **그대로 과업 목록으로** 삼아 잰 결과다. ★TRD·ADR 본문은 이 문서가 고치지 않았다★ — TRD §3-5 의 「결과 = 새 보고서 `…-2026-09-2x.md`」 자리를 이 파일로 잇는 것은 다음 개정의 몫이다(고아 금지). 메인의 추가 지시(잘린 완결 `assistant` 줄이 블록 하나인가 여럿인가)는 §6 S5 에 함께 적었다.
- **잰 날:** 2026-09-28 (KST 00:37–00:46 · 로그 시각은 UTC 2026-09-27T15:37–15:46)
- **버전:** `claude --version` = `2.1.280 (Claude Code)` — PATH 의 `claude` → `%APPDATA%\npm\claude.cmd` → `…\@anthropic-ai\claude-code\bin\claude.exe`. 모델 = CLI 기본값(`system/init` 의 `model` = `claude-opus-5-5[1m]`).
- **스폰 인자:** `backend/claude/mod.rs` `build_spec` 의 JSON 모드를 그대로 — `cmd.exe /c claude --permission-mode bypassPermissions -p --input-format stream-json --output-format stream-json --replay-user-messages --verbose --include-partial-messages --session-id <uuid>` · env `MAX_THINKING_TOKENS=8000` · 제어 채널 없음(`--mcp-config` 등 없음) · `extra_args` 없음. B1(`partial_stream_p1`)과 같은 조건이다. 부모 Claude Code 세션의 환경변수(`CLAUDE*` · `AI_AGENT` 등)는 지웠다 — 데몬이 띄우는 claude 는 그것을 물려받지 않는다.
- **끊기 줄:** ADR-0238 결정 1 의 모양 그대로 — `{"type":"control_request","request_id":"interrupt:<uuid v4>","request":{"subtype":"interrupt"}}\n`. `reason` · `cancel_queued` 는 싣지 않았다. ★모든 끊기는 하네스가 **직접** 썼다 — 우리 턴 열림 문(아직 없다)을 거치지 않았다★. 문이 그 순간 열려 있었을지는 §9 S8 이 decoder 로 다시 잰다.
- **방법:** 우리 스폰 인자·와이어 형식을 흉내 내는 node 하네스가 CLI 를 직접 띄우고, 정해 둔 시점(대개 **그 줄을 받은 stdout 처리기 안에서 동기로**)에 stdin 으로 JSON 줄을 쓰고, stdout 의 모든 줄을 **단조 시계(ms, 하네스 기동 기준)** 와 함께 `{t,dir,chunk,line}` JSONL 로 적는다(Phase 0 하네스를 이어 썼다). 하네스는 이 워커 셸의 백그라운드로 돌렸고 출력은 파일로만 받았다. 앱·데몬·다른 워크트리 프로세스는 건드리지 않았다. 제품 코드는 한 줄도 바꾸지 않았다.
- **S8 decoder:** 작업 트리의 `engram-dashboard-agent` · `-command` · `-base` 를 스크래치로 **복사한 스냅숏**(B1 의 미커밋 `claude/mod.rs` 포함 — sha256 `8fc220d6…`)에 작은 바이너리를 붙여 `backend::output_decoder`(라이브 decoder) + `backend::turn_classifier` 를 **실물 그대로** 불렀다. 턴 열림 문은 아직 코드가 없으므로 TRD §3-4 의 정의(한 줄의 사건 중 `Progress` 가 하나라도 있으면 열림 · `result` 줄에서 닫힘)를 그 위에 흉내 냈다. 빌드는 `scripts/run-detached.ps1` 로 돌렸다(`__EXIT=0`).
- **하네스·원시 로그 위치(세션 스크래치 — 휘발):** `C:\Users\<user>\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard-wt1\3bacb436-cde6-47e8-a754-31810509c430\scratchpad\b2\`
  - 하네스 `harness.cjs`(시나리오 `s1` · `s3` · `s4` · `s5` · `s6` · `s7`) · 요약 `sum.cjs` · 픽스처 가공 `mkfixture.cjs` · 문 추적 `s8ws\gate-trace\src\main.rs`
  - 원시 로그: `logs\claude-s1-2026-09-27T15-37-24-325Z.jsonl` · `…-s3-…T15-41-22-719Z` · `…-s4-…T15-41-53-339Z` · `…-s5-…T15-44-47-625Z` · `…-s6-…T15-42-33-918Z` · `…-s7-…T15-43-28-073Z`
  - ★S1 만 저장소에 남는다★ — 픽스처 `crates/engram-dashboard-agent/src/backend/claude/fixtures/interrupt_s1.jsonl`(202 줄). 아래 「픽스처 N」은 그 파일의 **1 기반** 줄 번호다. 그 밖의 시행은 원시 로그의 **0 기반** 줄 번호(「s3:43」 식)로 적고, 필요한 줄은 이 문서에 발췌했다.
- **확신도 범례:** 확실(원시 로그에 직접 찍힌 사실, 반복 관측) · 가능성 높음(한두 번 관측 + 벤더 스키마 판독이 맞물림) · 불확실(추론·단발)

## 0. 결론 (먼저)

> ★**멈춤 — TRD 가 예상하지 못한 것 하나(표시)**★: 끊긴 턴마다 CLI 가 `result` **앞에** 합성 사용자 줄 `{"type":"user", …, "content":[{"type":"text","text":"[Request interrupted by user]"}]}` 을 낸다(도구 중이면 `…for tool use]`). `isReplay` 가 없고 uuid 는 CLI 가 새로 뽑는다. **관측한 끊긴 턴 14 개 전부**에서 나왔고 transcript 에도 남는다. 오늘 decoder 는 이것을 `Structured{user}` 로 내고 누산기는 **사용자 말풍선**으로 그린다(`structuredAccumulator.ts:224-244` — uuid 가 대기 명부에도 「본 것」에도 없으니 그대로 그린다). 그래서 TRD §3-4 의 「누산기가 중단 행과 구분선을 그린다 — codex 끊김과 같은 모양 · 프론트·선 타입 무변경」은 성립하지 않는다 — 중단 행 위에 「[Request interrupted by user]」 말풍선이 하나 더 선다(이어받기 이력에도). ★턴 관측·문·오류 뒤 멈춤에는 영향이 없다★(턴 안 · `result` 앞의 진행 줄이다). 지시서의 멈춤 조건 둘째에 걸리므로 **B3 의 claude 부분은 메인 판정 전에 시작하지 않는다.** 상세 = §10-1.

| # | 결과 | 판정 |
|---|---|---|
| **S1** | 글 중 · 도구 중 · 턴 열리기 전 · 한가 뒤 새 턴 — 끊은 **14 번 모두** `result` 가 왔다(끊기 뒤 9 ms – 1.0 s · `init` 전 끊기는 그 턴의 `init` 을 기다린다) | ✅ 통과 |
| **S2** | 끊긴 `result` = `subtype:"error_during_execution"` · `is_error:true` · **`terminal_reason:"aborted_streaming"`**(글·생각·도구 입력 흘리는 중) \| **`"aborted_tools"`**(도구 실행 중) — 14/14. `subtype:"interrupted"` 는 한 번도 안 나왔다 | ✅ `terminal_reason` 이 가른다 → **④ 예비 표식은 짓지 않는다** |
| **S3** | 응답 `still_queued:["<B>"]` → A 의 `result` 뒤 **곧바로 B `started`**(새 턴) → B 답 → `completed`. B 에 `cancelled`/`discarded` 없음 | ✅ 통과 |
| **S4** | `started` 직후 · `system/init` 전 끊기 3 번(세션 첫 명령 포함) + 되울림 직후 1 번 — **4/4 멈췄다**. 2.1.280 의 순서는 `queued → started → init → 되울림`(되울림이 init **뒤**) | ✅ 통과 — **문이 여는 지점은 그대로(`started` 의 `Delivered`)** · 늦출 필요 없음 |
| **S5** | 잘린 완결 `assistant` 줄은 **열린 글 블록일 때만** 온다 — 블록 **하나**(`aborted:true` · 흘린 글과 같은 본문 · `content_block_stop` 없이 · `result` 앞). 열린 도구 입력·생각 블록은 완결 줄이 **아예 안 온다** | 기록 — **single-block** |
| **S6** | 첫 입력 전 · 끝난 턴 뒤 한가 끊기 — `control_response {success, still_queued:[]}` 한 줄뿐. 뒤이은 턴은 정상 | ✅ 통과 |
| **S7** | A 의 `result` 뒤 +0 · +300 · +800 · +1500 ms 끊기 — **4/4 에서 B 의 턴이 끊겼다**(`aborted_streaming` · 오류 행 · 멈춤 없음 — S2 판정으로 가른다). A `result` → B `started` 틈 = 2.3–3.6 ms | 기록 — 받아들인다(TRD 조건 충족) |
| **S8** | 로그 여섯(끊기 14 번 포함)을 실 decoder 에 먹임 — 문을 여는 줄은 **전부 그 턴의 `command_lifecycle started`**, 닫는 줄은 전부 그 턴의 `result`. 턴 밖에서 여는 줄 0 | ✅ 통과 |

- **출구 조건(§3-5): 충족.** S1 · S2 · S3 · S4 · S6 · S8 통과, S5 · S7 기록. 기대는 불변식 셋(턴 관측 두 지점·30 분 fail-open / 오류 뒤 멈춤 / 대기 입력 한 줄기) 모두 이 실측으로 선다 — 단 **오류 뒤 멈춤은 B3 의 `result` 분류가 들어와야** 선다(오늘 코드는 끊긴 `result` 를 `Failed` + `Ended(Clean)` 로 읽는다 — §9).
- **S1 기록 추가:** stream-json 끊기도 **도는 Bash 도구를 죽인다** — 끊은 초 뒤로 틱이 하나도 안 늘었고, 도구의 `bash.exe` 둘이 +3 s 에 없었다. `system/task_notification status:"stopped"`. 그 도구의 `tool_result` = `is_error:true` · 본문 = **문자열**(배열 아님) 「The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.」 — 대화형 CLI 기록(TRD §11 ⑪)과 **같다**. 상세 = §2-2.
- **메인 추가 지시 — 판정: single-block.** 관측한 완결 `assistant` 줄 23 개(잘린 것 1 포함)가 **전부 블록 하나**다. B3 이 `content[i]` 를 블록 번호 i 로 맞출 필요는 이 실측에선 없다. 상세 = §6.

## 1. 공통 — 끊긴 턴의 꼬리 모양 (14/14 · 확실)

끊는 순간 무엇이 돌고 있었든 꼬리는 같은 순서였다:

```
>> control_request interrupt 씀
   control_response {"subtype":"success","request_id":"interrupt:<우리 uuid>","response":{"still_queued":[…]}}   ← +1–112 ms (첫 입력 전 한가 끊기만 385 ms)
   (글 블록이 열려 있었을 때만) assistant {… "aborted":true, content:[열린 글 블록 하나]}
   (도구가 돌고 있었을 때만) system/task_notification status:"stopped" · user tool_result is_error:true
   user {"content":[{"type":"text","text":"[Request interrupted by user]"}]}   ← 도구 중이면 "…for tool use]" · isReplay 없음 · 새 uuid
   result {"subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_streaming"|"aborted_tools", "errors":["[ede_diagnostic] …"], …}
   command_lifecycle cancelled <그 턴을 연 명령의 uuid>                        ← result 뒤 1–7 ms
```

- 응답(`control_response`)은 **언제나 `result` 앞**이다 — 벤더 스키마 설명(「on a clean interrupt this receipt is written before the interrupted turn result」)과 맞는다.
- 끊긴 턴 뒤에 `message_delta` · `message_stop` · 열린 블록의 `content_block_stop` 은 **오지 않는다.**
- 턴을 연 명령의 `command_lifecycle` 은 `completed` 가 아니라 **`cancelled`** 로, `result` **뒤**에 온다. 우리 번역표로는 `Dropped{Unknown}` 이고 이미 `Delivered` 로 묘비가 선 id 라 **무동작**이다(`queued_input.rs:320-332` 의 `Some(_) => {}`) · 턴 분류기는 `None`(`claude/mod.rs:566`). 문을 다시 열지 않는다(§9).
- 벤더 스키마(설치본 `claude.exe` 안의 zod 설명 — 판독 · 가능성 높음): `interrupt_receipt_v1` = 「응답 `still_queued` = 이 끊기를 **살아남는** 비동기 사용자 메시지의 uuid — **These WILL run**」. ADR-0238 결정 1 · 6 의 뜻(대기분은 다음 턴에 돈다)과 같다. 요청의 `reason`(선택 · `@internal`)은 싣지 않았는데도 도구는 「사용자 거절」 모양으로 닫혔다.

## 2. S1 — 글 흐르는 중 · 도구 도는 중 끊기

**설정(`s1` 로그 · 한 프로세스):** ① S1a = 「인쇄술의 역사 1500 단어 에세이 · 도구 없이」 → 첫 `text_delta` 2.5 s 뒤 끊기 ② S1b = 「Bash 로 `for i in $(seq 1 90); do echo "<표식> tick $i"; echo "tick $i $(date +%s)" >> <틱 파일>; sleep 1; done` 을 전경으로」 → `tool_use` 뒤 틱 파일이 5 줄이 되자 끊기 ③ 뒤 턴 = 「Reply with exactly: AFTER-OK」.

### 2-1. S1a — 글 흐르는 중 (픽스처 1–139)

```
t(ms)      픽스처  줄
10129.5    2       command_lifecycle started 753cc4e5
11092.1    3       system/init
14530.5    5       user isReplay=true (A 되울림)
14531.0    7       stream_event content_block_start #0 thinking   … 생각 ~106 s
120400.4   85      stream_event content_block_start #1 text
120401.3–122656.7  86–134   text_delta 49 줄
122902.2   —       >> interrupt:f453a7a6 씀   (첫 글 델타 2.5 s 뒤)
122905.0   135     control_response {"subtype":"success","request_id":"interrupt:f453a7a6-…","response":{"still_queued":[]}}
122937.0   136     assistant msg_011CfUC9… {"stop_reason":null,"aborted":true, content:[{type:"text", 927 자}]}
122940.0   137     user {"content":[{"type":"text","text":"[Request interrupted by user]"}], "uuid":"f1ab3f56-…"}
122951.4   138     result {"subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_streaming","stop_reason":null,"num_turns":2,"errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null"]}
122953.7   139     command_lifecycle cancelled 753cc4e5
```

**판정: 통과** — 끊은 뒤 49 ms 에 `result`.

### 2-2. S1b — Bash 도구 도는 중 (픽스처 140–188) · S1 기록 추가

```
t(ms)      픽스처  줄
127964.6   141     command_lifecycle started f127a5e7
140151.1   178     assistant tool_use Bash {"command":"for i in $(seq 1 90); …"}
152469.0   182     system/task_started {"task_type":"local_bash","is_backgrounded":false}
155964.4   —       (프로세스 조회) 표식을 가진 bash.exe 2 개 — pid 44372 → 자식 4104
155964.5   —       >> interrupt:deae228f 씀   (틱 파일 6 줄)
155972.0   183     control_response {… "still_queued":[]}
155975.5   184     system/task_notification {"status":"stopped","summary":"Run 90-second tick loop writing to log file"}
155988.4   185     user tool_result {"is_error":true,"content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","tool_use_id":"toolu_01EG…"}
                            + 줄 수준 "tool_use_result":"User rejected tool use" · "tool_result_meta":[{"id":"toolu_01EG…","non_execution_kind":"user-rejected"}]
155990.4   186     user {"content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}
155997.4   187     result {"subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_tools","stop_reason":"tool_use","num_turns":3}
155998.9   188     command_lifecycle cancelled f127a5e7
+0 … +16.1 s  —    틱 파일 줄 수 = 6 에서 한 번도 안 늘었다(1 s 간격 17 표본)
+3 s · +16 s  —    표식을 가진 프로세스 0 개
```

- **Bash 도구는 멈췄고 프로세스는 죽었다(확실).** 마지막 틱 `tick 6 1790523600`(= 15:40:00 UTC)은 끊은 그 초(`tool_result` 의 `timestamp` = 15:40:00.311Z)다. 대화형 Claude Code 의 Esc 와 같다(TRD §9 대조).
- **`tool_result`:** `is_error:true` · `content` 는 **문자열**. 대화형 CLI 기록의 본문과 같은 문장이다. 줄 수준에 `tool_use_result:"User rejected tool use"` · `tool_result_meta[].non_execution_kind:"user-rejected"` 가 함께 온다(구조화된 표지 — T-37 에 쓸 만하다. 이번 표시 규칙(TRD §11 ⑪)은 여기에 기대지 않는다).
- **판정: 통과** — 끊은 뒤 33 ms 에 `result`.

### 2-3. 뒤 턴 (픽스처 189–202)

끊기 둘 뒤의 평범한 턴이 `started → init → 되울림 → 글 → result success(terminal_reason:"completed") → completed` 로 끝났다. 끊기가 CLI 에 남기는 상태는 없었다.

## 3. S2 — 끊긴 `result` 가 끊김을 가르나

14 개 끊긴 `result` 전부:

| 끊는 순간 | 수 | `subtype` | `is_error` | `terminal_reason` | `stop_reason` |
|---|---|---|---|---|---|
| 글 흐르는 중 · 생각 흐르는 중 · 도구 **입력** 흐르는 중 · 턴 열리기 전 | 12 | `error_during_execution` | `true` | **`aborted_streaming`** | `null` |
| 도구 **실행** 중(S1b · S3) | 2 | `error_during_execution` | `true` | **`aborted_tools`** | `"tool_use"` |

- 정상 턴의 `result` 는 전부 `terminal_reason:"completed"` 였다.
- ★`subtype` 은 끊김을 가르지 못한다★ — `error_during_execution` 은 진짜 실행 오류와 같은 낱말이다. `terminal_reason` 이 가른다. 그래서 TRD §3-4 의 `interrupted(result)` 두 칸 중 **`terminal_reason` 칸이 실제로 일하는 칸**이고 `subtype == "interrupted"` 칸은 이 설치본에선 한 번도 안 맞았다(남겨도 해가 없다 — 옛 CLI 대비).
- ★오늘 코드로는 이 `result` 가 오류다★ — `is_error:true` 라 `Error(RESULT_FAILURE_DETAIL …)` + `MessageDone` 을 내고, 분류기는 `[Failed, Ended(Clean)]` 로 적는다(§9 추적의 `signals=`). B3 의 분류가 들어오기 전까지 Esc 한 번이 오류 뒤 멈춤을 세운다 — TRD 가 예상한 그대로다.
- 기존 시험 둘(`result_interrupted_subtype_*` — `subtype:"interrupted"`)은 **손으로 지은 모양**이다. 실측 모양(`error_during_execution` + `terminal_reason`)은 `interrupt_s1.jsonl` 픽스처 138 · 187 에 있다.
- **판정: 통과 — ④ 예비 표식(`interrupt_sent`)은 짓지 않는다**(ADR-0238 결정 5 의 조건 「두 칸 모두 못 가를 때만」이 성립하지 않는다).

## 4. S3 — 도구 중 B 를 대기시킨 뒤 끊기 (`s3` 로그)

**설정:** A = Bash `for i in $(seq 1 40); do echo tick $i; sleep 1; done` → `tool_use` 3 s 뒤 B(「Reply with exactly: B-RAN-S3」) 씀 → B `queued` 1 s 뒤 끊기.

```
t(ms)     s3:줄  줄
16235.2   40     >> B 씀 (6b15c7c8)
16239.6   41     command_lifecycle queued 6b15c7c8
17239.8   42     >> interrupt:154ce007 씀
17247.5   43     control_response {"subtype":"success","request_id":"interrupt:154ce007-…","response":{"still_queued":["6b15c7c8-41e0-4c9d-a5cc-f3b48654d405"]}}
17258.7   44     user tool_result is_error:true "The user doesn't want to proceed with this tool use. …"
17260.5   45     user "[Request interrupted by user for tool use]"
17266.0   46     result error_during_execution · is_error:true · terminal_reason:"aborted_tools"
17268.1   47     command_lifecycle cancelled 013fa71d   (A)
17269.4   48     command_lifecycle started 6b15c7c8     (B — 새 턴)
17927.4   49     system/init
20657.0   51     user isReplay=true uuid=6b15c7c8 "Reply with exactly: B-RAN-S3"
20685.0   55     assistant text "B-RAN-S3"
21171.0   59     result success · terminal_reason:"completed"
21171.7   60     command_lifecycle completed 6b15c7c8
```

- B 는 `still_queued` 에 있었고, A 의 `result` 3.4 ms 뒤 새 턴으로 `started` 됐고, 답했다. B 에 `cancelled` · `discarded` 는 없다.
- 이 시행에선 도구가 아직 `task_started` 를 내기 전이었는데도 `terminal_reason` 은 `aborted_tools` 였다(도구 호출 단계에 들어간 뒤면 그렇게 적는 것으로 보인다 · 단발).
- **판정: 통과** — ADR-0238 결정 1 · 6 의 뜻(도는 턴만 멈추고 대기분은 다음 턴)이 CLI 동작과 같다.

## 5. S4 — 새 턴이 열린 직후 · `system/init` 전 끊기 (`s4` 로그)

**설정:** A = 「1 부터 150 까지 영어 단어로 한 줄씩」(안 끊기면 수십 초 걸린다) → **A 의 `command_lifecycle started` 줄을 받은 처리기 안에서 동기로** 끊기 씀(S4a1 = 세션 첫 명령 · S4a2 · S4a3 = 뒤 턴). S4b = **A 의 되울림 줄**을 받은 처리기 안에서 끊기.

```
시행   started   >>끊기    응답      init      되울림     [Request…]  result(aborted_streaming)
S4a1   8457.4    8457.6    8468.6    8506.8    8546.9     8547.4      8572.0      (s4:12–19 · 세션 첫 명령)
S4a2   13595.7   13596.0   13707.9   14554.4   14588.2    14588.6     14594.3     (s4:24–31)
S4a3   19606.2   19606.3   19674.4   20210.6   20235.4    20235.7     20240.4     (s4:36–43)
S4b    25253.4   27310.4   27311.9   26365.2   27310.3    27315.7     27319.9     (s4:48–57 · 되울림 뒤 끊기 — 그 사이 message_start · thinking 블록 시작만 옴)
```

- **네 번 모두 멈췄다** — `assistant` 줄도 글 델타도 없이 `result` 가 왔다. `system/init` **전에** 닿은 끊기(S4a1–a3 — 응답이 init 보다 먼저 왔다)도 무시되지 않았다. 세션 **첫** 명령(S4a1)도 같다.
- **순서:** 2.1.280 은 `queued → started → system/init → 되울림`이다 — ★되울림은 init **뒤**에 온다★. TRD §3-5 S4 의 전제(「되울림이 init 보다 먼저」)는 이 설치본과 다르지만, 문을 여는 **가장 이른** 줄은 되울림이 아니라 `started`(`Delivered` — TRD §3-4 의 진행 신호)이고 바로 그 자리의 끊기가 먹혔으므로 결론은 같다.
- 벤더 스키마 판독(가능성 높음): 첫 명령의 「prewait window」에 닿은 끊기는 **걸쇠(latch)** 로 잡혀 「그 일을 싣고 무장하는 첫 턴이 이미 끊긴 채 시작한다」고 적혀 있다(SDK issue #429 의 수정으로 보인다). 뒤 턴(S4a2 · a3)도 같은 결말이었다.
- **`system/init` 은 턴마다 온다**(TRD §12 1 라운드 low 1 의 기록 요청): 로그 여섯 모두 `init` 수 = `result` 수(3/3 · 2/2 · 4/4 · 3/3 · 2/2 · 8/8). `started → init` 간격 = 49 ms – 5.7 s(대개 0.5–1.1 s).
- **판정: 통과 — 문이 여는 지점을 늦추지 않는다**(ADR-0238 결정 3 의 ★보정★ 조건이 성립하지 않는다 · 「마지막 `result` 뒤 init 을 봤나」 비트도 필요 없다).

## 6. S5 — 끊김 직전의 잘린 `assistant` 줄 (+ 메인 추가 지시: 블록 하나냐 여럿이냐)

### 6-1. 글 블록이 열려 있을 때(S1a · 픽스처 85–138)

- 열린 블록 = `#1 text`(`#0 thinking` 은 이미 자기 완결 줄 · `content_block_stop` 으로 닫혔다).
- 끊은 뒤 **완결 `assistant` 줄 하나**가 온다(픽스처 136): `content` 길이 **1** = 그 글 블록뿐 · `message.stop_reason:null` · 줄 수준 **`"aborted":true`** · 본문 927 자 = **흘린 델타 49 줄을 이은 글과 한 글자도 다르지 않다**(`startsWith` 참 · 꼬리 0 자).
- 위치: 그 블록의 **`content_block_stop` 은 끝내 오지 않는다**(앞도 뒤도 아니다) · **`result` 보다 앞**(34.8 ms 뒤 줄 → 14.4 ms 뒤 `result`).
- decoder(ADR-0240): 완결 줄이 오는 순간 `partial.open == Some(1)` 이고 1 은 흘린 블록이므로 글을 버린다 — 잃는 글이 없다(흘린 것이 전부다). §9 추적에서 이 줄은 사건을 내지 않았다.

### 6-2. 도구 입력 블록이 열려 있을 때 — 흘린 글 블록 뒤(S5a1 · S5a2 · `s5` 로그)

**설정:** 「도구를 쓰기 전에 한 문장으로 알리고, Write 도구로 80 줄 넘는 시를 파일에 써라」 → 같은 메시지 안에서 글 델타를 본 뒤 `tool_use` 블록이 열리고 `input_json_delta` 12 줄을 받은 처리기 안에서 끊기.

```
S5a1 (s5:72–99)
38768.6  72  assistant msg_011CfUChZ2i… [thinking]          ← #0 완결 줄
38769.1  73  content_block_stop #0
38769.3  74  content_block_start #1 text
38770.3–38970.9  75–79  text_delta 5 줄 "poem-S5a1.txt 파일에 시를 작성하겠습니다."
39160.4  80  assistant msg_011CfUChZ2i… [text]              ← #1 완결 줄 (content 1 개)
39160.6  81  content_block_stop #1
39161.1  82  content_block_start #2 tool_use
39161.4–39649.5  83–94  input_json_delta 12 줄
39649.8  95  >> interrupt 씀
39654.0  96  control_response {… "still_queued":[]}
39663.1  97  user "[Request interrupted by user]"
39707.5  98  result error_during_execution · terminal_reason:"aborted_streaming"
39710.8  99  command_lifecycle cancelled 7e3624cf
```

S5a2(`s5:114–140`)도 같은 모양이다 — #1 글 완결 줄(121) → `content_block_stop #1`(122) → `#2 tool_use` 시작(123) → 델타 12 → 끊기(136) → 응답 · `[Request…]` · `result aborted_streaming`. **열린 `tool_use` 블록의 완결 줄은 두 번 모두 오지 않았다** — 흘리던 도구 입력은 어디에도 남지 않는다(`ToolCall` 사건 없음 · 파일도 안 써졌다). 도구가 아직 실행되지 않았으므로 `terminal_reason` 은 `aborted_tools` 가 아니라 `aborted_streaming` 이다.

### 6-3. 생각 블록이 열려 있을 때(S5b · `s5:148–160` · S4b)

S5b = 메시지 첫 블록 `#0 thinking` 에서 `thinking_delta` 3 줄 뒤 끊기 → 응답 · `[Request…]` · `result aborted_streaming`. **완결 줄 없음.** S4b(생각 블록이 막 열린 순간)도 같다.

### 6-4. 판정 — **single-block**

- 관측한 완결 `assistant` 줄 **23 개 전부 `content` 길이 1** 이다(로그 여섯 전수 — 생각 6 · 글 11 · 도구 6 · 잘린 것 1 포함).
- 잘린 완결 줄은 **열린 블록이 글일 때만** 오고, 그 줄이 싣는 것은 그 글 블록 하나다. 앞서 닫힌 블록(흘린 글 · 생각)을 다시 싣지 않고, 열린 도구 입력 · 생각 블록은 싣지도 않는다(완결 줄이 아예 없다).
- 그래서 B3 이 `content[i]` 를 블록 번호 i 로 맞출 근거는 이 실측에 없다. 불확실로 남는 것: 한 메시지 안에서 **글 블록이 둘 이상 연달아 열린 상태**의 끊김은 만들지 못했다(한 번에 열린 블록은 하나뿐이라 구조상 드물다).

## 7. S6 — 턴이 없을 때 끊기 (`s6` 로그)

```
s6:9   4014.1   >> interrupt 씀 (첫 입력 전 — 세션에 턴이 한 번도 없었다)
s6:11  4399.1   control_response {"subtype":"success","request_id":"interrupt:bec5b7a2-…","response":{"still_queued":[]}}
                … 8 s 동안 다른 줄 없음(SessionStart 훅 응답 한 줄만 — 잡음)
s6:13–29        턴 1 「Reply with exactly: HELLO-S6」 → started → init → 되울림 → 글 → result success → completed
s6:31  27426.8  >> interrupt 씀 (끝난 턴 뒤 5 s 한가)
s6:32  27490.5  control_response {"subtype":"success","request_id":"interrupt:fbd32172-…","response":{"still_queued":[]}}
                … 10 s 동안 다른 줄 없음
s6:34–48        턴 2 「Reply with exactly: AFTER-S6」 → 정상 success
```

- 한가 끊기는 응답 한 줄만 낸다 — `assistant` · `user` · `stream_event` 없음. 뒤이은 턴은 끊기지 않았다(첫 입력 전 끊기가 걸쇠로 첫 턴을 죽이지 않았다 — 걸쇠는 그 순간 대기 중이던 일에만 걸린다는 스키마 설명과 맞는다).
- ★CLI 는 한가 끊기를 거절하지 않는다★ — 늘 `success` 다. 「끊을 턴이 없다」를 말해 줄 수 있는 것은 우리 턴 열림 문뿐이다(TRD §3-4 가 가정한 그대로).
- **판정: 통과.**

## 8. S7 — A 의 `result` 직후 끊기 · 잔여 경합의 실물 크기 (`s7` 로그)

**설정:** A = Bash `sleep 3` → **A 의 `tool_result` 를 받은 처리기 안에서** B(「1 부터 60 까지 영어 단어로」) 씀(Phase 0 M7 — 접기를 놓쳐 새 턴이 된다) → A 의 `result` 를 받은 뒤 +지연에 끊기. 「문」 칸 = 끊기를 쓴 순간까지 받은 줄로 §9 의 흉내 문이 가졌을 값.

| 시행 | A `result` | B `started` | 끊기 씀 | 그때 문 | 응답 | B `result` | B 모양 |
|---|---|---|---|---|---|---|---|
| S7-0 (`s7:49–59`) | 14311.6 | 14314.2 | 14311.8 | **닫힘** | 14354.0 | 14846.4 `aborted_streaming` | 되울림 · `[Request…]` · 글 0 · `cancelled` |
| S7-300 (`s7:92–102`) | 28741.1 | 28743.5 | 29041.6 | 열림(B) | 29067.6 | 29488.0 `aborted_streaming` | 같음 |
| S7-800 (`s7:134–144`) | 43162.4 | 43164.7 | 43961.9 | 열림(B) | 43962.8 | 43968.6 `aborted_streaming` | 같음 |
| S7-1500 (`s7:177–187`) | 59761.9 | 59765.5 | 61263.4 | 열림(B) | 61316.9 | 61429.0 `aborted_streaming` | 같음 |

- **넷 다 B 의 턴이 끊겼다.** B 의 글은 되울림(사용자 말풍선)으로 보이고, 답은 한 글자도 없고, `result` 는 `aborted_streaming` 이다 — S2 의 판정으로 **중단 턴**(오류 행 · 오류 뒤 멈춤 없음)이 된다. 대기 명부: B 는 `started`(`Delivered`)로 닫혀 말풍선이 받음 자리에 서고, 뒤이은 `cancelled` 는 묘비 위 무동작이다. → **TRD 의 받아들임 조건(오류 행 · 멈춤 없음) 충족.**
- **A `result` → B `started` 틈 = 2.6 · 2.4 · 2.3 · 3.6 ms.** 대기 중인 B 가 있으면 문은 A 의 끝에서 거의 즉시 B 로 다시 열린다. 그래서 A 의 끝을 노린 늦은 Esc 는 사실상 B 를 끊는다(S7-300 · 800 · 1500 은 문이 **B 의 턴**으로 열려 있어 우리 구현도 보냈을 것이다 — 그 순간의 뒷단에서 보면 정당한 끊기다).
- S7-0 은 문이 닫힌 순간(A `result` 0.2 ms 뒤 · B `started` 전)에 썼다 — **우리 구현이라면 CONFLICT 로 거절해 B 가 돌았을 것**이다. 하네스가 직접 쓴 줄은 CLI 가 B 의 턴이 시작된 **뒤**에 처리해(응답이 B `started` 40 ms 뒤) B 를 끊었다. 즉 「줄이 A 의 턴에 읽혔지만 CLI 에 닿기 전에 B 가 열렸다」는 TRD §3-4 의 잔여 경합이 **실제로 B 를 끊는 모양**이 이것이다(벤더 줄에 턴 id 가 없다).
- **판정: 기록 — 받아들인다.**

## 9. S8 — 문 궤적 (실 decoder · `gate-trace`)

로그 여섯을 한 줄씩 실 decoder 에 먹이고, 그 줄의 사건 중 분류기가 `Progress` 로 읽은 것이 있으면 열고 `result` 줄에서 닫았다.

| 로그 | 연 줄(전부) | 닫은 줄(전부) | 턴 밖에서 연 줄 |
|---|---|---|---|
| S1 픽스처 | 2 · 141 · 190 = 각 턴의 `command_lifecycle started` | 138 · 187 · 201 = 각 턴의 `result` | 0 |
| s3 | 12(A) · 48(B `started` — A `result` 뒤 새 턴) | 46 · 59 | 0 |
| s4 | 12 · 24 · 36 · 48 | 19 · 31 · 43 · 57 | 0 |
| s5 | 12 · 103 · 144 | 98 · 139 · 159 | 0 |
| s6 | 15 · 36 (끊기 둘은 문이 닫힌 채 씀) | 28 · 47 | 0 |
| s7 | A 넷 · B 넷 — 모두 `started` | 모두 `result` | 0 |

- **문을 여는 줄은 예외 없이 그 턴의 `command_lifecycle started`**(→ `Delivered`)였다 — 되울림 · `init` · 첫 글보다 앞이다. 끝난 뒤 오는 `cancelled`(→ `Dropped`) · 한가 끊기의 `control_response` · `system/*` 는 열지 않았다.
- 합성 `[Request interrupted…]` 줄은 `Structured{user}` → `Progress` 지만 **언제나 그 턴의 `result` 앞**이라 이미 열린 문을 건드리지 않는다.
- 끊기를 쓴 순간의 문: S1 · S3 · S4 · S5 는 전부 **열림**(우리 구현도 보냈다) · S6 둘은 **닫힘**(우리 구현은 CONFLICT) · S7 은 §8 표.
- 오늘 분류기가 끊긴 `result` 에 매긴 신호는 전부 `[Failed, Ended(Clean)]` 이다 — B3 전에는 오류 뒤 멈춤이 선다(§3).
- 픽스처 꼬리의 줄별 사건(오늘 decoder 그대로): 끊기 응답(135 · 183) → **사건 0**(TRD §3-4 「번역하지 않는다 — 이미 `None`」 확인) · 잘린 완결 줄(136) → **사건 0**(흘린 글이라 버림) · 합성 줄(137 · 186) → `Structured{kind:"user"}`(그 줄의 uuid 가 실린다) · 도구 결과(185) → `Structured{kind:"user"}` · 끊긴 `result`(138 · 187) → `[Usage?, Error("claude stream-json result reported failure (subtype=error_during_execution)"), MessageDone]` · 뒤이은 `cancelled`(139 · 188) → `QueuedInput(Dropped{cause: Unknown})`.
- **판정: 통과.**

## 10. TRD §3-4 · ADR-0238 과 어긋나거나 새로 나온 것

1. ★**합성 사용자 줄 `[Request interrupted by user]` / `…for tool use]` — TRD 가 예상하지 못했다(멈춤 사유)**★ — 끊긴 턴 14/14 에서 `result` 앞에 오고(§1) transcript 에도 `user` 줄로 남는다(S1 세션 transcript 의 0 기반 21 · 33 번째 줄 — 확인했다). 오늘 경로로는 라이브에서도 이어받기 이력에서도 **사용자 말풍선**이 된다. TRD §3-4 「codex 끊김과 같은 모양 · 프론트·선 타입 무변경」 · U8(중단 줄 강조)의 화면이 이 말풍선 하나만큼 달라진다. 도구 중 끊기면 화면 순서는 「붉은 `Error` 도구 행 → `[Request interrupted by user for tool use]` 말풍선 → 중단 행」이 된다(TRD §11 ⑪ 의 모양 + 말풍선). 불변식에는 영향이 없다(§9). **어떻게 보일지(그대로 둔다 · decoder 가 거른다 · 다르게 그린다)는 사용자 체감이라 메인·사용자 판정이 필요하다** — 거른다면 무엇으로 알아보나(벤더 문자열 · `isReplay` 없는 글 전용 `user` 줄 · 위치)도 함께 갈린다. 이 문서는 고르지 않는다.
2. **S4 의 전제 순서가 다르다(멈춤 아님)** — 2.1.280 은 되울림이 init **뒤**다. 문을 여는 줄은 `started` 이고 거기서의 끊기가 먹혔다 — 보정 불필요(§5).
3. **끊긴 `result` 의 `subtype` 은 `interrupted` 가 아니라 `error_during_execution` 이다(TRD 가 `terminal_reason` 칸으로 예상했다)** — B3 은 픽스처 138 · 187 로 `TurnEnd{Interrupted}` 를 박는 것이 실측 근거가 된다. 손으로 지은 `subtype:"interrupted"` 시험 둘은 옛 CLI 대비로만 남는다.
4. **턴을 연 명령의 `cancelled` 가 `result` 뒤에 온다(무해)** — 묘비 위 무동작 · 분류기 `None` · 문 안 열림(§1 · §9). TRD 는 이 줄을 적지 않았지만 번역표(`claude/mod.rs:1330`의 「`cancelled` → 모름(끊기·…)」)가 이미 덮는다.
5. **흘리던 도구 입력은 흔적 없이 사라진다** — 열린 `tool_use` 블록은 완결 줄도 `content_block_stop` 도 없이 끝난다(§6-2). 화면에는 도구 행이 아예 없다(흘린 글 · 중단 행만).
6. **잔여 경합은 좁지 않다** — 대기 중인 B 가 있으면 A 의 끝과 B 의 시작 사이 문이 닫힌 틈이 2–4 ms 뿐이라, A 의 꼬리를 노린 늦은 Esc 는 사실상 B 를 끊는다(§8). 결말은 TRD 가 받아들인 모양(중단 턴 · 오류 아님 · 멈춤 불변)이다.
7. (부수) 마지막 턴이 끊긴 채 stdin 을 닫으면 프로세스가 **종료 코드 1** 로 끝났다(s4 · s5 · s7 — 마지막 턴이 성공인 s1 · s3 · s6 은 0). 우리 kill 경로는 종료 코드에 기대지 않는다 — 기록만.

## 11. 픽스처

- `crates/engram-dashboard-agent/src/backend/claude/fixtures/interrupt_s1.jsonl` — 202 줄. 원본 = `claude-s1-2026-09-27T15-37-24-325Z.jsonl` 의 `dir:"out"` 줄 11–223 · 226–291 · 311–324(0 기반). `fixtures/README.md` 의 공통 가공(잡음 줄 넷 제거 · `system/init` 설치 환경 목록을 B1 과 같은 중립 값으로 · 개인정보 자리표시)에 더해: 스크래치 경로 → `C:\work\proj`(cwd) · `C:/work/logs`(틱 파일) · ★경로를 품은 `input_json_delta` 11 줄(원본 251–261)을 한 줄 `/work/logs/` 로 합쳤다★(이어 붙인 인자가 완결 `tool_use` 의 `input` 과 같음을 확인했다 · decoder 는 이 델타를 읽지 않는다 — 도구 입력 델타 35 → 25 줄). `system/status` · `system/task_started` · `system/task_notification` 은 실측 그대로다.
- 실 decoder 로 다시 먹여 문 궤적이 원본 로그와 같음을 확인했다(연 줄 2 · 141 · 190 · 닫은 줄 138 · 187 · 201).
- ★`fixtures/README.md` 는 이 작업이 고치지 않았다★(다른 워커의 미커밋 변경이 있다) — 표 한 줄은 메인이 더한다.
