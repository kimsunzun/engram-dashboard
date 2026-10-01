# claude fixtures

실측 stdout(`--output-format stream-json`) 줄을 한 줄에 하나씩, 일어난 순서대로 담는다. 기존 픽스처(`claude_text.jsonl`·`claude_tool.jsonl`)와 같이 **CLI 가 낸 줄만** 싣는다 — 우리가 stdin 으로 쓴 줄은 싣지 않고, 아래에 「무엇을 언제 썼나」로 적는다.

## 공통 가공 (턴 도중 입력 픽스처 — `lifecycle_m1` · `cancel_m3` · `drain_m7` · `slash_m13` · `transcript_queued_m5`)

- **원본:** `.claude/handoff/attachments/20260925-midturn-phase0/logs/*.jsonl` 의 `{t,dir,line}` 봉투에서 `dir:"out"` 인 `line` 만 꺼냈다. `meta`·`err`·`in` 은 버렸다. 「줄 범위」는 그 로그 파일의 0 기반 줄 번호다. 각 로그가 무엇을 쟀는지는 `docs/research/mid-turn-phase0-measurements-2026-09-25.md`(M1·M3·M5·M7) · `docs/research/mid-turn-phase0b-measurements-2026-09-26.md`(M13).
- **빼낸 잡음 줄:** `system/hook_started` · `system/hook_response` · `system/thinking_tokens` · `rate_limit_event`. 남은 줄의 순서는 그대로다.
- **`system/init` 손질:** 설치 환경 목록(`slash_commands` · `agents` · `skills` · `plugins` · `mcp_servers`)을 중립 값으로 바꿨다(플러그인 이름은 자리표시). `capabilities`·`tools`·`model`·`claude_code_version` 등 나머지는 실측 그대로다.
- **개인정보 치환:** cwd → `C:\work\proj` · OS 사용자 홈 → `C:\home\user` · 인코딩된 프로젝트 폴더 이름 → `C--work-proj` · 메시징 파이프 이름 → `cc-msg-0000`. uuid·session id 는 난수라 그대로 뒀다.
- claude 2.1.280 · 모델 haiku · 우리 JSON 모드 스폰 인자(Phase 0 보고서 §1).

## 파일

| 파일 | 줄 | 원본 · 줄 범위 | 보여 주는 것 |
|---|---|---|---|
| `lifecycle_m1.jsonl` | 17 | `claude-M1-2026-09-24T18-29-52-786Z.jsonl` 9–31 | 턴 A(`f53b8bb3…`, `sleep 20`) 도중 B(`44fcee51…`)를 씀(A 의 `tool_use` 뒤) → B `command_lifecycle` `queued` → `tool_result` → B 의 `isReplay` 되울림 → B `started` → 답 "DONE-A PINEAPPLE" → B `completed` → `result` → A `completed`. 한가할 때 쓴 A 도 `queued`→`started` 를 낸다. |
| `cancel_m3.jsonl` | 34 | `claude-M3-2026-09-24T18-32-48-267Z.jsonl` 9–29 (trial a) · 52–73 (trial c2) | **a:** 도구 중 B(`6d3edeb9…`) 씀 → `queued` → 1 s 뒤 `cancel_async_message` 씀 → `cancelled` 줄이 `control_response {cancelled:true}` 보다 먼저 → B 는 모델에 안 감. **c2:** 취소(`635fe486…`)를 먼저 씀 → `control_response {cancelled:false}` → 0.7 s 뒤 B 씀 → `queued` → 되울림 → `started` → 답에 WORDC2(전달됨). |
| `drain_m7.jsonl` | 19 | `claude-M7-2026-09-24T18-35-19-085Z.jsonl` 9–32 (trial 1) | `tool_result` 를 받은 즉시 B(`df5a5b43…`)를 씀(그 `tool_result` 줄 바로 뒤) → `queued` 가 18 ms 뒤 → 접히지 못하고 A 의 `result`·`completed` 뒤 새 턴으로 `started` → 두 번째 `result`. |
| `slash_m13.jsonl` | 36 | `claude-M13-compact-2026-09-25T15-55-09-512Z.jsonl` 9–40 (trial m1) · 43–59 (trial i1) | **m1:** 도구 중 `/compact`(`317a496f…`) 씀 → `queued` → A `result`·`completed` 뒤 `started` → `status compacting` → `init` → `compact_boundary` → 요약 user 줄 → **우리가 안 보낸 uuid** 의 `<local-command-stdout>Compacted` 되울림 → 우리 uuid 의 `<command-name>/compact` 되울림 → `result`(빈 문자열) → `completed`. **i1:** 한가할 때 `/compact`(`6cef0ba9…`) → `Compacted` 되울림이 **두 번**(앞선 compact 것의 중복). |
| `transcript_queued_m5.jsonl` | 11 | M1 세션 transcript(`~/.claude/projects/<인코딩된 cwd>/561e36c6-….jsonl`) 0–2 · 17–20 · 24–25 · 27–28 | ★로그 폴더가 아니라 CLI 가 남긴 transcript 에서 꺼냈다 — 손으로 지은 줄은 없다★. 접힌 B 가 `user` 줄이 아니라 `attachment{type:"queued_command", prompt, source_uuid, commandMode:"prompt"}` 한 줄로 남고, 그 뒤 `queue-operation {operation:"remove", reason:"absorbed_mid_turn"}` → 다음 `assistant`. 빠진 줄(환경·모델·스킬·지시문·세션 정보·`prompt_snapshot` 첨부 등 — 환경 정보가 실려 있거나 수십 KB)이 있어 **`parentUuid` 사슬이 중간에 끊긴다.** |
| `result_error_handbuilt.jsonl` | 8 | ★**전부 손으로 지음**★ — `lifecycle_m1.jsonl` 의 A 턴 `result`·`command_lifecycle` 줄을 본떴다 | `is_error:true` 인 `result` 는 한 번도 캡처되지 않았다(TRD §7-1 「우편의 오류 뒤 멈춤」 행). 턴 둘: ① `subtype:"success"` + `is_error:true` + `result:"API Error: 500 …"` + `api_error_status:500` ② `subtype:"error_during_execution"` + `is_error:true` + `result` 필드 없음. 각 턴 = `queued` → `started` → `result` → `completed`. uuid 는 `aaaaaaaa-0000-4000-8000-…` 가짜 값이고 `usage` 등 나머지 필드는 M1 성공 줄 그대로다. ★실제 오류 줄의 `terminal_reason`·`stop_reason`·필드 구성은 모른다★ — 실측이 생기면 갈아 끼운다. |

기존 `claude_text.jsonl` · `claude_tool.jsonl` · `claude_transcript.jsonl` 은 이 작업이 건드리지 않았다.

## 글자 스트리밍 픽스처 (`partial_stream_p1` — ADR-0240)

- **원본:** B1 채취 로그 `claude-main-2026-09-27T15-17-20-649Z.jsonl`(`{t,dir,chunk,nl,raw}` 봉투 · `dir:"out"` 인 `raw` 만 꺼냈다). ★원본 로그는 저장소 밖(채취한 워커의 스크래치)이라 남지 않는다★. 「줄 범위」는 그 로그의 `out` 줄만 센 1 기반 번호다.
- **스폰:** 우리 JSON 모드 인자 그대로 — `--permission-mode bypassPermissions -p --input-format stream-json --output-format stream-json --replay-user-messages --verbose --include-partial-messages --session-id <sid>` · env `MAX_THINKING_TOKENS=8000` · 제어 채널 없음 · `extra_args` 없음(그래서 모델 = CLI 기본값). claude 2.1.280 · 모델 claude-opus-5-5.
- **가공:** 위 「공통 가공」과 같다 — 잡음 줄 넷을 뺐고 · `system/init` 설치 환경 목록을 중립 값으로 바꿨고 · 개인정보를 같은 자리표시로 바꿨다. `system/task_started` · `system/task_notification` 은 실측 그대로 남겼다(decoder 가 건너뛴다).
- **뺀 턴:** T4(긴 답 — `out` 줄 104–694)를 통째로 뺐다. 링 압박 수치(흘린 델타 수 · 글자 수)는 B1 반환이 싣는다 — 이 픽스처로 재지 않는다.

| 파일 | 줄 | 원본 · 줄 범위 | 보여 주는 것 |
|---|---|---|---|
| `partial_stream_p1.jsonl` | 140 | 위 로그 `out` 1–103 · 695–744 | 매 턴 `message_start` → `content_block_start` → 델타들 → **그 블록만 담은 완결 `assistant` 줄**(같은 id) → `content_block_stop` → `message_delta` → `message_stop` → `result`. **T1** 한글 글만(델타 셋). **T2** 생각 → 글 → 도구(`input_json_delta` 일곱) → `tool_result` → 새 `message_start` → 글. **T3** 생각 → 글(델타 열다섯). **T5** A(`d24e1139…` — `sleep 8`)의 `tool_use` 줄(112) 2 s 뒤 B(`05a41fe4…`)를 씀 → B `queued`(116) → `tool_result` → B 되울림 → B `started` → 새 `message_start`(123) → 답 "DONE-A PINEAPPLE". **한가 구간** = 각 `result` 뒤 다음 입력을 쓰기 전: T1 뒤 32.0 s · T2 뒤 6.0 s · T3 뒤 6.0 s(그 뒤 T4 는 뺐다) · T5 뒤 10 s + stdin 닫고 끝날 때까지(16.4 s). 네 구간 모두 `command_lifecycle completed` 한 줄뿐이고 **최상위 `stream_event` 0 줄**이다(뺀 T4 뒤 6.0 s 도 같다). 이 세션의 transcript 에는 `stream_event` 줄이 **없었고** 완결 `assistant` 글 줄은 남아 있었다(`--resume` 이어받기도 성공 — B1 반환). |

## 끊기 픽스처 (`interrupt_s1` — ADR-0238)

- **원본·방법:** B2 스파이크(`docs/research/claude-interrupt-spike-2026-09-28.md`) 채취 — 보고서 §6 에 원 줄이 있다.

| 파일 | 줄 | 원본 · 줄 범위 | 보여 주는 것 |
|---|---|---|---|
| `interrupt_s1.jsonl` | 202 | B2 채취 로그 `claude-s1-2026-09-27T15-37-24-325Z.jsonl`(`{t,dir,chunk,line}` · 0 기반) `out` 11–223 · 226–291 · 311–324 | **S1a**(1–139) 글 흐르는 중 끊기(134 뒤에 씀) → `control_response {still_queued:[]}`(135) → 잘린 완결 `assistant`(136 · `aborted:true` · 글 블록 하나 = 흘린 927 자 · `content_block_stop` 없음) → 합성 사용자 줄 `[Request interrupted by user]`(137) → `result` `error_during_execution`·`is_error:true`·`terminal_reason:"aborted_streaming"`(138) → 턴을 연 명령 `cancelled`(139). **S1b**(140–188) Bash 90 초 루프 도는 중 끊기(182 뒤) → 응답(183) → `task_notification status:"stopped"`(184) → `tool_result` `is_error:true`(185) → `[Request interrupted by user for tool use]`(186) → `result` `aborted_tools`(187) → `cancelled`(188). **뒤 턴**(189–202) 평범한 `success`. 가공 = 공통 가공 + 스크래치 경로 → `C:\work\proj`·`C:/work/logs` · 경로를 품은 `input_json_delta` 11 줄(원본 251–261)을 한 줄로 합침(35→25 · 이은 인자 = 완결 `tool_use` 입력). claude 2.1.280 · 모델 claude-opus-5-5 · B1 과 같은 스폰 인자. |
