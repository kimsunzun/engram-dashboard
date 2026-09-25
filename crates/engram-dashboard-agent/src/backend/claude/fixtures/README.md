# claude fixtures

실측 stdout(`--output-format stream-json`) 줄을 한 줄에 하나씩, 일어난 순서대로 담는다. 기존 픽스처(`claude_text.jsonl`·`claude_tool.jsonl`)와 같이 **CLI 가 낸 줄만** 싣는다 — 우리가 stdin 으로 쓴 줄은 싣지 않고, 아래에 「무엇을 언제 썼나」로 적는다.

## 공통 가공 (턴 도중 입력 픽스처 — `lifecycle_m1` · `cancel_m3` · `drain_m7` · `slash_m13` · `transcript_queued_m5`)

- **원본:** `.claude/handoff/attachments/20260925-midturn-phase0/logs/*.jsonl` 의 `{t,dir,line}` 봉투에서 `dir:"out"` 인 `line` 만 꺼냈다. `meta`·`err`·`in` 은 버렸다. 「줄 범위」는 그 로그 파일의 0 기반 줄 번호다. 각 로그가 무엇을 쟀는지는 `docs/research/mid-turn-phase0-measurements-2026-09-25.md`(M1·M3·M5·M7) · `docs/research/mid-turn-phase0b-measurements-2026-09-26.md`(M13).
- **빼낸 잡음 줄:** `system/hook_started` · `system/hook_response` · `system/thinking_tokens` · `rate_limit_event`. 남은 줄의 순서는 그대로다.
- **`system/init` 손질:** 설치 환경 목록(`slash_commands` · `agents` · `skills` · `plugins` · `mcp_servers`)을 중립 값으로 바꿨다(사내 플러그인 이름이 실려 있었다). `capabilities`·`tools`·`model`·`claude_code_version` 등 나머지는 실측 그대로다.
- **개인정보 치환:** cwd → `C:\work\proj` · OS 사용자 홈 → `C:\home\user` · 인코딩된 프로젝트 폴더 이름 → `C--work-proj` · 메시징 파이프 이름 → `cc-msg-0000`. uuid·session id 는 난수라 그대로 뒀다.
- claude 2.1.280 · 모델 haiku · 우리 JSON 모드 스폰 인자(Phase 0 보고서 §1).

## 파일

| 파일 | 줄 | 원본 · 줄 범위 | 보여 주는 것 |
|---|---|---|---|
| `lifecycle_m1.jsonl` | 17 | `claude-M1-2026-09-24T18-29-52-786Z.jsonl` 9–31 | 턴 A(`f53b8bb3…`, `sleep 20`) 도중 B(`44fcee51…`)를 씀(A 의 `tool_use` 뒤) → B `command_lifecycle` `queued` → `tool_result` → B 의 `isReplay` 되울림 → B `started` → 답 "DONE-A PINEAPPLE" → B `completed` → `result` → A `completed`. 한가할 때 쓴 A 도 `queued`→`started` 를 낸다. |
| `cancel_m3.jsonl` | 34 | `claude-M3-2026-09-24T18-32-48-267Z.jsonl` 9–29 (trial a) · 52–73 (trial c2) | **a:** 도구 중 B(`6d3edeb9…`) 씀 → `queued` → 1 s 뒤 `cancel_async_message` 씀 → `cancelled` 줄이 `control_response {cancelled:true}` 보다 먼저 → B 는 모델에 안 감. **c2:** 취소(`635fe486…`)를 먼저 씀 → `control_response {cancelled:false}` → 0.7 s 뒤 B 씀 → `queued` → 되울림 → `started` → 답에 WORDC2(전달됨). |
| `drain_m7.jsonl` | 19 | `claude-M7-2026-09-24T18-35-19-085Z.jsonl` 9–32 (trial 1) | `tool_result` 를 받은 즉시 B(`df5a5b43…`)를 씀(그 `tool_result` 줄 바로 뒤) → `queued` 가 18 ms 뒤 → 접히지 못하고 A 의 `result`·`completed` 뒤 새 턴으로 `started` → 두 번째 `result`. |
| `slash_m13.jsonl` | 36 | `claude-M13-compact-2026-09-25T15-55-09-512Z.jsonl` 9–40 (trial m1) · 43–59 (trial i1) | **m1:** 도구 중 `/compact`(`317a496f…`) 씀 → `queued` → A `result`·`completed` 뒤 `started` → `status compacting` → `init` → `compact_boundary` → 요약 user 줄 → **우리가 안 보낸 uuid** 의 `<local-command-stdout>Compacted` 되울림 → 우리 uuid 의 `<command-name>/compact` 되울림 → `result`(빈 문자열) → `completed`. **i1:** 한가할 때 `/compact`(`6cef0ba9…`) → `Compacted` 되울림이 **두 번**(앞선 compact 것의 중복). |
| `transcript_queued_m5.jsonl` | 11 | M1 세션 transcript(`~/.claude/projects/<인코딩된 cwd>/561e36c6-….jsonl`) 0–2 · 17–20 · 24–25 · 27–28 | ★로그 폴더가 아니라 CLI 가 남긴 transcript 에서 꺼냈다 — 손으로 지은 줄은 없다★. 접힌 B 가 `user` 줄이 아니라 `attachment{type:"queued_command", prompt, source_uuid, commandMode:"prompt"}` 한 줄로 남고, 그 뒤 `queue-operation {operation:"remove", reason:"absorbed_mid_turn"}` → 다음 `assistant`. 빠진 줄(환경·모델·스킬·지시문·세션 정보·`prompt_snapshot` 첨부 등 — 개인정보·사내 정보가 실려 있거나 수십 KB)이 있어 **`parentUuid` 사슬이 중간에 끊긴다.** |
| `result_error_handbuilt.jsonl` | 8 | ★**전부 손으로 지음**★ — `lifecycle_m1.jsonl` 의 A 턴 `result`·`command_lifecycle` 줄을 본떴다 | `is_error:true` 인 `result` 는 한 번도 캡처되지 않았다(TRD §7-1 「우편의 오류 뒤 멈춤」 행). 턴 둘: ① `subtype:"success"` + `is_error:true` + `result:"API Error: 500 …"` + `api_error_status:500` ② `subtype:"error_during_execution"` + `is_error:true` + `result` 필드 없음. 각 턴 = `queued` → `started` → `result` → `completed`. uuid 는 `aaaaaaaa-0000-4000-8000-…` 가짜 값이고 `usage` 등 나머지 필드는 M1 성공 줄 그대로다. ★실제 오류 줄의 `terminal_reason`·`stop_reason`·필드 구성은 모른다★ — 실측이 생기면 갈아 끼운다. |

기존 `claude_text.jsonl` · `claude_tool.jsonl` · `claude_transcript.jsonl` 은 이 작업이 건드리지 않았다.
