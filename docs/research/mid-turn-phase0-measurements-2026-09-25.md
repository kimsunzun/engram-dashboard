# 턴 도중 입력 — Phase 0 벤더 실측 (M1–M8) (2026-09-25)

- **상태:** 실측 완료 · TRD 반영 전. 이 문서는 `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` §3-3 의 표(M1–M8)를 **그대로 과업 목록으로** 삼아 잰 결과다. ★TRD 본문은 이 문서가 고치지 않았다★ — §3 되적기·fixture 박기는 다음 개정의 몫이다(고아 금지: TRD §3-3 에서 이 문서로 링크가 필요하다).
- **잰 날:** 2026-09-25 (KST 03:29–03:40 · 로그 시각은 UTC 2026-09-24T18:29–18:40)
- **버전(우리 앱이 띄우는 설치본):**
  - claude = `2.1.280 (Claude Code)` — PATH 의 `claude` → `%APPDATA%\npm\claude.cmd` → `node_modules\@anthropic-ai\claude-code\bin\claude.exe`. TRD 판독 기준과 **같다**.
  - codex = `codex-cli 0.156.1` — PATH 의 `codex` → `codex.cmd` → node → `codex.exe`. ★TRD 판독 기준(0.154.0)보다 **새것**이다★ — 아래 codex 결과는 전부 0.156.1 실측이다(0.154.0 에서 다시 재지 않았다).
- **방법:** 우리 스폰 인자·와이어 형식을 그대로 흉내 내는 node 하네스가 CLI 를 직접 띄우고, 정해 둔 시점에 stdin 으로 JSON 줄을 쓰고, stdout 의 모든 줄을 **단조 시계(ms, 하네스 기동 기준)** 와 함께 JSONL 로 적는다. 모든 실행은 `scripts/run-detached.ps1` 로 셸 트리 밖에서 돌렸고 완료는 `__EXIT=0` 마커로 판정했다. 앱·데몬·다른 워크트리 프로세스는 건드리지 않았다.
- **하네스·원시 로그 위치(세션 스크래치 — 휘발 가능):** `C:\Users\kimsunzun\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard-wt1\6847046e-28fb-4531-a649-281ac6ab97c0\scratchpad\phase0\`
  - 하네스: `claude_harness.js`(시나리오 M1 · M3 · M3B · M7) · `codex_harness.js`(M6 · M7) · 분석: `sumclaude.js` · `sumcodex.js` · `m7claude.js` · `m7codex.js`
  - 원시 로그: `logs\claude-M1-2026-09-24T18-29-52-786Z.jsonl` · `logs\claude-M3-2026-09-24T18-32-48-267Z.jsonl` · `logs\claude-M3B-2026-09-24T18-38-29-346Z.jsonl` · `logs\claude-M7-2026-09-24T18-35-19-085Z.jsonl` · `logs\codex-M6-2026-09-24T18-32-49-086Z.jsonl` · `logs\codex-M7-2026-09-24T18-35-20-013Z.jsonl`
- **확신도 범례:** 확실(원시 로그에 직접 찍힌 사실, 반복 관측) · 가능성 높음(한두 번 관측 + 코드 판독이 맞물림) · 불확실(추론·단발·외부에서 가를 수 없음)

## 0. 결론 (먼저)

1. **claude 경로(S2)의 전제는 우리 모드에서 선다.** `command_lifecycle` 이 `-p` stream-json 에서 그대로 나오고(M1), `cancel_async_message` 가 `initialize` 없이 받힌다(M3). 큐에 있는 항목의 취소는 `cancelled:true` + 수명주기 `cancelled` 로 닫히고 모델에 안 간다.
2. ★**TRD §3-1 ③ 「아직 안 들어온 uuid 는 취소 예약 → 나중에 `cancelled`」는 턴 도중(접기 경로)에서 틀렸다**★ — 취소를 먼저 보내고 글을 나중에 쓰면 `cancelled:false` 뒤 그 글이 **접혀 전달됐다**(`started`). 예약 취소는 턴 사이 배출(새 턴 시작) 경로에서만 확인된다(코드 판독). 다만 우리 설계는 늘 「글 → 취소」 순서로 쓰고, CLI 는 stdin 을 순서대로 동기 적재하므로(같은 write 로 보내도 `queued` → `cancelled`) ⓒ 경합은 **자연 발생하지 않았다.** 「결말은 수명주기로만」 규칙은 그대로 맞다.
3. ★**M7 은 두 백엔드가 반대로 나왔다**★ — 도구 완료를 보고 넘기기(S1): **claude 0/10**(TRD 예측대로 늘 놓친다) · **codex 10/10**(TRD 예측 「거의 늘 놓친다」와 반대 — 도구 완료 알림 뒤 6–64 ms 동안 대기분 확인이 아직 안 돈다). 결정은 이미 「즉시 넘기기」로 났으므로 설계를 바꾸지는 않는다 — 나중 세분화 때 codex S1 이 실제로 쓸 만한 후보라는 재료다.
4. **codex(0.156.1) 쪽 가정은 전부 섰다**(M6): steer 둘 → 도구 뒤에 `clientId` 단 `userMessage` 둘 · 끊기 시 에코 없음(이력에도 없음) · 턴이 막 끝난 순간 steer → `-32600 "no active turn to steer"` · `thread/items/list` 가 `clientId` 를 싣는다. 턴 끝 「기록만」 갈래는 **못 맞혔다**(두 번 다 같은 턴 후속 샘플링으로 소비됐다).
5. **새로 나온 것 둘** — ① claude `system/init` 은 기동 때가 아니라 **턴이 시작될 때마다** 나온다(첫 입력 전엔 없다) ② codex 는 끊긴 턴의 셸 명령이 **계속 돌아**, 끊기 17 초 뒤 **옛 턴 id 를 단 `item/completed`** 가 다른 턴들이 끝난 뒤에 도착했다(§5-5 도구 계수에 영향).

## 1. 스폰 인자와 와이어 형식 (재현용)

### claude — `ClaudeBackend::build_spec` JSON 모드(`crates/engram-dashboard-agent/src/backend/claude/mod.rs:155-316`)

```
cmd.exe /c claude --permission-mode bypassPermissions -p --input-format stream-json
  --output-format stream-json --replay-user-messages --verbose --session-id <새 uuid> --model haiku
env += MAX_THINKING_TOKENS=8000
```

- **뺀 것:** control endpoint 가 있을 때만 붙는 `--mcp-config` · `--append-system-prompt-file` · `--settings` · `--disallowedTools SendMessage` · `--allowedTools …`. 전부 MCP·프라이밍·권한 선언이라 명령 큐·접기·취소 기계와 무관하다고 봤다(불확실 — 우리 `--settings` 조각에 훅이 실리는 날엔 아래 「접기 전 지연」이 달라질 수 있다).
- **더한 것:** `--model haiku`(extra_args 자리). 실측 모델 = `claude-haiku-4-5-20251001`. 큐·접기·수명주기·취소는 CLI 안의 기계라 모델과 무관하다 — 모델이 영향을 주는 것은 지시를 따르느냐(판정에 안 썼다)와 응답 지연뿐이다.
- **사용자 설정은 그대로 탔다** — 우리 앱이 띄우는 claude 와 같은 조건이다(사용자 `settings.json` 훅 · 설치 플러그인 훅 · 전역 지시문). 이 점이 아래 M3 의 「접기 전 지연 ~0.57 s」에 걸린다.
- stdin 사용자 줄(`wrap_user_turn` 과 같은 키 순서): `{"type":"user","message":{"role":"user","content":[{"type":"text","text":…}]},"uuid":"<uuid>"}\n`
- 취소 줄(TRD §5-4 의 모양 그대로): `{"type":"control_request","request_id":"cancel:<uuid>","request":{"subtype":"cancel_async_message","message_uuid":"<uuid>"}}\n`
- 작업 폴더 = 스크래치의 `cwd-claude`(저장소 밖) · 매 실행 새 세션 id. `--resume` 은 안 썼다.

### codex — app-server 모드(`backend/codex/mod.rs:807-815` · `thread_open` `:125-152` · 통로 핸드셰이크 `transport.rs:827-852`)

```
cmd.exe /c codex app-server --stdio -c model=gpt-6-luna -c model_reasoning_effort=low -c notify=[]
```

- `-c` 셋은 extra_args 자리(우리 argv 가 허용하는 통로)다. 모델을 싼 것으로 · 추론을 낮게 · **사용자 `config.toml` 의 `notify`(데스크톱 앱 실행)가 턴마다 돌지 않게** 막았다. steer·대기 입력 기계는 app-server 쪽이라 모델과 무관하다.
- 핸드셰이크: `initialize {clientInfo:{name:"engram-dashboard",version:"0.1.0"}}` → `initialized` 알림 → `thread/start {cwd, approvalPolicy:"on-request", sandbox:"workspace-write"}`. 서버 요청은 우리 `Reader::refuse` 와 같이 `-32601` 로 거절하게 했다(이번 실행에선 한 건도 안 왔다).
- 봉투에 `jsonrpc` 칸 없음(우리 `request_line` 과 같다). `turn/start` 에 설계대로 `clientUserMessageId` 를 실었다(오늘 코드는 안 싣는다 — `protocol.rs:537`).
- `turn/steer`: `{"threadId":…,"clientUserMessageId":<uuid>,"input":[{"type":"text","text":…}],"expectedTurnId":<활성 턴 id>}` — 0.156.1 스키마(`app-server-protocol/src/protocol/v2/turn.rs` `TurnSteerParams`)와 같다.
- 스레드는 사용자 `CODEX_HOME` 에 새로 생겼다(기존 스레드는 안 건드렸다 — 아래 §10).

## 2. M1 — claude `command_lifecycle` 이 우리 모드에서 나오나

**설정:** 첫 줄 A(uuid `f53b8bb3…`) = 「Bash 로 `sleep 20` 을 돌리고 DONE-A 로 답하라」 → `tool_use` 를 본 뒤 3 s 에 B(uuid `44fcee51…`) = 「답에 PINEAPPLE 을 넣어라」.

**증거**(`logs\claude-M1-…jsonl`, t = ms):

```
4010.3   >> A 씀
6923.1   {"type":"command_lifecycle","command_uuid":"f53b8bb3-…","state":"queued","uuid":…,"session_id":…}
6924.4   command_lifecycle started f53b8bb3
7580.3   system/init  (capabilities 포함)
9680.5   assistant tool_use Bash {"command":"sleep 20"}
12681.2  >> B 씀 (도구 도는 중)
12682.1  command_lifecycle queued 44fcee51        ← 쓴 뒤 0.9 ms
32271.3  user tool_result
32842.6  user isReplay uuid=44fcee51 ("timestamp":"…18:30:05.469Z" = 쓴 시각)
32843.7  command_lifecycle started 44fcee51      ← 도구 결과 뒤 572 ms
35239.6  assistant text "DONE-A PINEAPPLE"
35663.3  command_lifecycle completed 44fcee51
35666.1  result/success
35667.2  command_lifecycle completed f53b8bb3
```

**결과: 녹색.** 줄이 나오고 순서는 `queued` → `started` → `completed`. 종결 줄의 `result` 대비 위치: **접혀 들어간 B 의 `completed` 는 `result` 바로 앞(3 ms)**, 턴을 연 A 의 `completed` 는 `result` 바로 뒤(1 ms). 필드명은 `command_uuid`(우리 uuid) · 줄 자체의 `uuid` 는 CLI 가 새로 뽑은 값이다. 전달된 뒤 모델이 B 를 반영했다(PINEAPPLE).
- 한가할 때 보낸 A 에도 수명주기가 나온다(`queued`→`started` 가 1 ms 안) — TRD §5-4 의 「Direct 로 보낸 입력의 `started` 는 모르는 id 라 버린다」가 실제로 필요한 갈래다.
- M3·M3B·M7 에서 uuid 32 개 전부 같은 모양으로 반복 관측.

**확신도:** 확실. **TRD 「빨가면」:** 해당 없음 — §4-3 claude 권고(S2)를 유지한다.

## 3. M2 — `isReplay` 되울림은 쓸 때 오나 접을 때 오나

**결과: 쓸 때는 절대 안 온다. 소비될 때 온다** — 그런데 **소비 경로에 따라 `started` 와의 순서가 뒤집힌다:**

| 경로 | 되울림 위치 | 관측 |
|---|---|---|
| 턴 도중 접힘(도구 경계) | `started` **바로 앞**(대개 0.4–1.1 ms · 취소가 끼어든 tb2 만 44 ms) | M1 (+572 ms 에 replay → +1 ms started) · M3 c2 · M3B tb2–tb4 |
| 경계를 놓쳐 턴 끝에 새 턴으로 배출 | `started` **뒤 약 1.5–2.1 s**(그 사이 `system/init`) | M7 10 회 전부 (예: trial 1 started +1799.5 · replay +3588.7, tool_result 기준) |
| 취소됨 | **안 온다** | M3 a · c1 · b1–b4 · M3B tb1 |

- 되울림 줄의 `timestamp` 칸은 **쓴(적재된) 시각**이지 소비 시각이 아니다(M1: 18:30:05.469Z = B 를 쓴 순간, 줄은 20 s 뒤 도착).
- 접힌 입력에도 `user` 줄(되울림)은 **온다**.

**확신도:** 확실(세 경로 모두 반복 관측). **TRD 「빨가면」:** 수치만 적는다 — ★단 §5-7 누산기는 두 순서를 **다** 받아야 한다★: 접기 경로에선 되울림이 `Delivered` 보다 먼저 오므로 「대기 uuid 억제」가 필요하고, 새 턴 경로에선 `Delivered` 뒤에 오므로 uuid dedup 이 받는다. TRD 설계는 둘 다 흡수한다.

## 4. M3 — `cancel_async_message` (우리 모드 · `initialize` 없이)

**받히나:** 받힌다. 「not supported in this context」 오류는 한 번도 안 왔다. 응답 봉투(원문):

```
{"type":"control_response","response":{"subtype":"success","request_id":"cancel:6d3edeb9-…","response":{"cancelled":true}}}
```

결과값은 `response.response.cancelled` 에 중첩돼 있다. 성공 취소면 수명주기 `cancelled` 줄이 **응답보다 먼저**(같은 ms) 나온다.

**경합 셋 — 시도한 것과 맞힌 것:**

| 시도 | 설정 | 관측(t = ms) | 판정 |
|---|---|---|---|
| **ⓐ 큐에 있을 때** (trial a) | 도구 중 B 씀 → `queued` 확인 1 s 뒤 취소 | 13827.9 취소 씀 → 13829.1 `LC cancelled` → 13829.1 응답 `true` · 답 "DONE-a"(단어 없음) | **맞힘** — `true` + `cancelled`, 모델에 안 감 |
| ⓐ 변형 — 같은 write 로 글+취소 (c1) | 두 줄을 한 번에 씀 | 35671.1 씀 → 35672.0 `queued` → 35672.2 `cancelled` → 35672.3 `true` | ⓒ 를 노렸으나 **ⓐ 로 떨어짐** — CLI 가 줄을 순서대로 **동기 적재**한다 |
| ⓐ 변형 — 도구 결과 뒤 +0/150/300/450/540 ms 에 취소 (b1–b4 · tb1) | B 는 도구 중에 적재됨 | 다섯 번 모두 `cancelled` + `true`, 모델에 안 감(b1 은 CLI 가 바빠 13 ms 늦게 처리) | **ⓐ** — 도구 결과 뒤 **540 ms 까지도** 아직 큐에서 뺄 수 있었다 |
| **ⓑ 접는 중** (tb2) | 도구 결과 +580 ms 에 취소 | 28505.4 B 되울림(=접기 진행 중) → 28515.4 취소 씀 → 28549.3 `LC started` → 28560.9 응답 `false` · 답에 단어 있음 | **선 위에서는 맞힘** — 되울림과 `started` 사이에 보냈고 결과는 `false` → `started`(전달). 단 CLI 안에서 `isFoldInFlight` 갈래를 탔는지 「이미 꺼냄」 갈래를 탔는지는 **밖에서 못 가른다**(응답이 `started` 뒤에 나왔다) |
| 너무 늦음 (tb3 · tb4) | +620 · +660 ms | `started` 뒤에 취소 도착 → `false` | 「이미 꺼냄」 — 결말 `started` 와 일치 |
| **ⓒ 쓴 직후 CLI 가 아직 안 읽음** | 자연 발생 불가(위 c1) — 대신 **취소를 먼저, 글을 0.7 s 뒤에** 보내 예약 취소 기계를 직접 때림 (c2) | 55629.5 취소 → 55630.1 `false` → 56330.3 B 씀 → 56331.1 `queued` → 66873.9 되울림 → 66874.6 **`started`** · 답 "DONE-c2 WORDC2" | ★**예약 취소가 안 걸렸다 — 전달됐다**★ |

- **ⓒ 가 왜 안 걸렸나(코드 판독, 가능성 높음):** 예약 표식을 확인하는 `consumeCancelPending` 의 호출자는 **턴 사이 배출기**(새 턴 시작 직전 · 바이너리 @217253965 · @217254615)와 배치 정산뿐이고, 턴 도중 **접기 경로**(@209592877 부근)는 표식을 보지 않는다. 즉 설명문의 「caught by a pending cancel just before dispatch」는 **새 턴 배출** 얘기다.
- **접기 전 지연의 정체(코드 판독, 가능성 높음):** 도구 결과 직후 접기는 대기 명령을 먼저 스냅숏(`getCommandsByMaxPriority`)한 뒤, 명령마다 **「prompt.submit at the fold」 비동기 처리**(`ah` → `Ed`, @209442306)를 기다리고, 끝에 **아직 큐에 있는 것만** 남긴다(`Oi` 필터 = `getCommandQueue()` 대조, @209441195). 그래서 이 구간(여기선 ~0.55–0.66 s) 동안의 취소는 `true` 로 먹고 이중 전달이 없다. 이 구간이 길었던 원인은 **설치된 플러그인의 `UserPromptSubmit` 훅으로 추정**한다(불확실 — 훅 실행 이벤트가 stream 에 안 찍혀 직접 확인 못 했다). 훅이 없는 환경에선 이 창이 짧아질 수 있다.
- **부작용 하나:** 너무 늦은 취소(ⓑ·「이미 꺼냄」)와 ⓒ 는 CLI 에 **예약 취소 표식을 남긴다**(`markCancelPending` — 결과가 비었고 접는 중이 아니면 무조건). 같은 uuid 를 다시 쓰지 않는 한 무해하다(가능성 높음).

**결과: 녹색(취소 기능) · TRD §3-1 ③ 서술은 빨강.** ⓐ 확실 · ⓑ 선 위 결말만 확실(내부 갈래 불확실) · ⓒ 자연 경합은 발생 불가(확실) / 예약 취소는 턴 도중엔 안 걸림(가능성 높음 — 1 회 관측 + 코드).
**TRD 「빨가면」:** 해당 없음(claude 취소는 codex 등급으로 떨어지지 않는다) — S5 이전을 다시 열 이유 없음. 다만 §3-1 ③ 행과 §0-결정 2 의 괄호(「stdin 을 아직 안 읽었으면 취소 예약만 걸고 false 를 답한 뒤 나중에 `cancelled` 를 낸다」)를 **「턴 사이 배출 경로에서만 — 턴 도중 접기에선 예약이 안 걸려 `started` 로 끝난다」**로 고쳐야 한다. 결말을 수명주기로만 판정하는 규칙은 이 관측에서도 옳은 답을 냈다(`false` 뒤 `started` → Delivered).

## 5. M4 — `system/init` 은 언제 오고 `capabilities` 를 싣나

**결과:**
- **기동 때는 안 온다.** 첫 입력 전 4 s 동안 stdout 에는 `system/hook_started`·`hook_response`(SessionStart 훅)만 있었다(M1 0–4010 ms).
- **턴이 시작될 때마다 한 번** 온다 — 턴을 연 명령의 `started` 뒤 **0.58–0.86 s**(M7 첫 5 턴: +749 · +587 · +612 · +855 · +584 ms). 실행별 `init` 수 = `result` 수(M1 1/1 · M3 7/7 · M7 20/20 · 접힌 입력에는 안 온다).
- `capabilities` = `["interrupt_receipt_v1","interrupt_cancel_queued_v1","msg_lifecycle_v1","mcp_read_resource_v1","mcp_tool_ui_meta_v1"]` — `msg_lifecycle_v1` 있음.

**확신도:** 확실. **TRD 「빨가면」:** 버전 대조로 바꿀 필요는 없다. 다만 §5-4 「탐지」에 한 가지 틈이 있다 — 깃발이 **첫 턴의 `started` 뒤 ~0.6–0.9 s** 에야 선다. 그 사이에 친 글은 한가함으로 오분류돼 오늘 동작(보낸 자리 말풍선)으로 간다. 수명주기 줄은 CLI 가 첫 입력을 읽은 직후에 오지만 우리가 쓴 시각(M1 4010.3 ms) 기준으로는 약 2.9 s 뒤다(6923.1 ms — CLI 기동 포함. 정정 2026-09-25 — TRD 2판 작성 중 M1 로그 대조). 그래서 **첫 `command_lifecycle` 줄을 탐지 신호로 겸하면** 그 틈이 좁아질 뿐 사라지지는 않는다(설계 판단은 TRD 몫).

## 6. M5 — 접힌 입력의 transcript 모양

**결과(M1 세션 `561e36c6-….jsonl`):** 접힌 B 는 `user` 줄이 아니라 **`attachment` 한 줄**로만 남는다:

```
{"type":"attachment", "attachment":{"type":"queued_command",
  "prompt":[{"type":"text","text":"When you reply, also include the word PINEAPPLE."}],
  "source_uuid":"44fcee51-376b-4de7-9532-e9608f9de0a0", "commandMode":"prompt",
  "timestamp":"2026-09-24T18:30:05.469Z"}, "uuid":"c1d33099…", "parentUuid":"f158675a…"}
```

- 위치: 도구 결과 `user` 줄 → `prompt_snapshot` 첨부 → **`queued_command`** → `queue-operation {"operation":"remove","reason":"absorbed_mid_turn"}` → 다음 `assistant`.
- `source_uuid` = 우리 uuid, `prompt` = 우리 본문 배열 — 복원 seed 가 사용자 말풍선을 만들 재료가 다 있다.
- **취소된 입력은 본문을 남기지 않는다** — `queue-operation` 의 enqueue/dequeue 만 남는다(M3 세션: enqueue 14 · dequeue 13, `queued_command` 는 전달된 c2 하나뿐).
- **경계를 놓쳐 새 턴이 된 입력은 평범한 `user` 줄**이다(M7 세션: `user` 텍스트 20 = A 10 + B 10, `queued_command` 0).

**확신도:** 확실(모양) · 복원 순서 영향은 판독하지 않았다. **TRD 「빨가면」:** 모양이 맞았다 → §9 P3 의 「복원 seed 에 그 줄을 사용자 말풍선으로 옮기는 조각」이 그대로 필요하다(오늘 `consume_line` 은 `attachment` 를 버린다).

## 7. M6 — codex `turn/steer` (0.156.1)

**설정:** 한 스레드에서 네 턴. T1 = `Start-Sleep -Seconds 20` 도중 steer 둘(X=MANGO, Y=LEMON) + `turn/completed` 를 받는 **그 핸들러 안에서** 옛 턴 id 로 steer 한 번 · T2 = 같은 명령 도중 steer Z(PAPAYA) 1.5 s 뒤 `turn/interrupt` · T3/T4 = 도구 없는 턴에서 샘플링 끝(각각 `agentMessage` `item/completed` · `thread/tokenUsage/updated`)을 받는 순간 steer W · 매 턴 뒤 `thread/items/list {limit:40, sortDirection:"desc"}`.

**증거**(`logs\codex-M6-…jsonl`):

```
T1  5939.1  >> turn/steer X      → 5943.5 {"turnId":"01a0d4b1-12c5…"}   (수락 = 전달 아님)
    6938.5  >> turn/steer Y      → 6939.4 {"turnId":…}
    24830.6 item/completed commandExecution
    24835.7 thread/tokenUsage/updated
    24837.8 item/started {"type":"userMessage","clientId":"bb2c04bc…"(X),"content":[{"type":"text","text":"…MANGO…","text_elements":[]}]}
    24843.0 item/started userMessage clientId=fa5e629a (Y)
    25988.3 item/completed agentMessage "DONE-1 MANGO LEMON"
    26023.1 turn/completed
    26023.2 >> turn/steer (옛 턴 id, turn/completed 핸들러 안) → 26026.5 {"code":-32600,"message":"no active turn to steer"}
T2  33147.5 >> turn/steer Z → 수락 · 34648.9 >> turn/interrupt → 34674.0 turn/completed status "interrupted"
    (Z 의 userMessage 에코 없음 · items/list 에도 Z 없음)
    51852.1 item/completed commandExecution (T2 의 턴 id · status "completed") ← 끊기 17 s 뒤, T3·T4 가 끝난 뒤
T3  40381.3 item/completed agentMessage → 같은 핸들러에서 steer W → 40459.3 userMessage 에코 → 두 번째 agentMessage "…A COMET…" → 42144.6 turn/completed
T4  47658.8 tokenUsage/updated → 같은 핸들러에서 steer W → 47688.7 에코 → 두 번째 agentMessage "…COMET…" → 49228.8 turn/completed
```

**결과(항목별):**

| TRD M6 질문 | 결과 | 확신도 |
|---|---|---|
| 도구 중 steer 둘 → 각자 `clientId` 단 `userMessage` 둘이 도구 뒤에 오나 | **예** — 친 순서대로, 도구 완료 뒤·다음 샘플링 전에 | 확실 |
| 끊기 시 에코 없나 | **없다** — 에코도 없고 `thread/items/list` 에도 없다(조용히 버려진다) | 확실 |
| 턴이 막 끝날 때 −32600 | **예** — `turn/completed` 를 받은 그 핸들러에서 보낸 steer 가 `-32600 "no active turn to steer"` | 확실 |
| `thread/items/list` 가 steer 로 든 항목의 `clientId` 를 싣나 | **싣는다** — X · Y · W 둘 모두, `turn/start` 로 연 첫 입력에도 | 확실 |
| 턴 끝에 「기록만」 된 입력이 `turn/completed` 전에 에코되나 | **그 갈래를 못 맞혔다** — 샘플링 끝 두 지점(T3·T4)에서 보낸 steer 가 둘 다 **같은 턴의 후속 샘플링**으로 소비됐다(에코 → 모델이 반영 → `turn/completed`). 기록만 되는 창은 `tokenUsage/updated` 보다도 뒤라는 뜻이고, 이번 방법으로는 닿지 않았다 | 불확실(갈래 미관측) |

- `turn/start` 가 `clientUserMessageId` 를 받아 첫 입력 에코에도 `clientId` 를 단다(0.156.1 확인 — 오늘 우리 코드는 안 싣는다).
- steer 성공 응답(`{"turnId"}`)은 1–5 ms 안에 와 에코보다 **훨씬 먼저** 온다 — TRD §5-5 「성공 응답은 무동작(전달 판정은 에코가 한다)」가 맞는 선택이다.
- ★**새로 본 것 — 끊긴 턴의 도구가 계속 돈다**★: `turn/interrupt` 응답·`turn/completed(interrupted)` 뒤에도 셸 명령은 끝까지 돌았고(20 s), 그 `item/completed`(**옛 턴 id**, status `completed`)가 **다음 두 턴이 끝난 뒤** 도착했다. 우리 디코더가 이것을 새 턴의 도구로 세면 §5-5 의 도구 계수가 어긋난다.

**TRD 「빨가면」:** §5-5 처분 중 고칠 것은 둘 — ① 도구 계수는 **턴 id 로 가려야** 한다(현재 턴이 아닌 `item/*` 는 계수에서 뺀다 — 「턴 경계에서 0」만으로는 늦게 오는 `completed` 가 음수를 만든다) ② 「정상 `turn/completed` 인데 넘긴 항목의 에코가 없음 = `Dropped{Unknown}`」 갈래는 이번에 **발생 조건을 못 만들었다** — 이중 전달 없이 선다는 근거(「기록된 입력은 완료 알림 전에 에코된다」)는 여전히 미확인이다.

## 8. M7 — 도구 **완료**를 보고 넘길 때(S1) 그 경계를 놓치는 비율

**설정:** 턴마다 「짧은 명령(2 s)을 돌리고 OK-k 로 답하라」. 도구 완료 줄(claude = `tool_result` `user` 줄 · codex = `commandExecution` `item/completed`)을 받는 **그 stdout 핸들러 안에서 곧바로** B 를 넘긴다(claude = stdin 사용자 줄 · codex = `turn/steer`). 한 세션에서 10 회.

**판정 기준:**
- claude: B 의 `started` 가 도구 뒤 첫 `assistant` 보다 앞이면 적중(같은 경계에 접힘).
- codex: B 의 `userMessage` 에코가 도구 뒤 첫 `agentMessage`/`reasoning` 시작보다 앞이고 **후속 샘플링이 없으면** 적중. ★모델이 KIWI 를 말했는지는 기준으로 쓰지 않았다★ — codex 는 10 회 모두 적중했는데도 답은 "OK-k" 뿐이었다(지시 「exactly」 와 부딪친 모델 선택). rollout 파일에서 KIWI 사용자 메시지가 `OK-k` 답보다 앞선 이력에 들어 있음을 trial 1–3 에서 따로 확인했다(4–10 은 에코 순서로만 판정).

**claude — 0/10 적중** (`m7claude.js`, tool_result 시각 기준 ms):

| trial | 씀 | `queued` | `started` | 도구 뒤 첫 assistant | `result` 수 |
|---|---|---|---|---|---|
| 1 | +0.3 | +18.0 | +1799.5 | +1302.1 | 2 |
| 2 | +0.0 | +19.2 | +1440.1 | +951.5 | 2 |
| 3 | +0.1 | +14.4 | +1538.8 | +952.4 | 2 |
| 4 | +0.1 | +30.8 | +2219.4 | +1533.9 | 2 |
| 5–10 | +0.0–0.2 | +11.7–14.1 | +1386–1658 | +914–1171 | 2 |

→ 우리가 0.3 ms 안에 썼는데도 CLI 는 **12–31 ms 뒤에야** 그 줄을 읽었고(그 사이 접기 스냅숏이 이미 찍혔다), B 는 매번 `result` 뒤 새 턴으로 나갔다(`started` = 첫 `result` + 1.5 ms).

**codex — 10/10 적중** (`m7codex.js`, 도구 `item/completed` 기준 ms):

| trial | steer 씀 | `tokenUsage/updated` | B 에코 | 다음 샘플링 시작 | 도구 뒤 답 |
|---|---|---|---|---|---|
| 1 | +0.4 | +9.2 | +12.1 | +978 | 1 ("OK-1") |
| 2 | +0.1 | +6.6 | +8.8 | +1098 | 1 |
| 5 | +0.2 | +54.7 | +63.9 | +1253 | 1 |
| 6 | +0.1 | +53.7 | +57.9 | +1060 | 1 |
| 3·4·7–10 | +0.0–0.3 | +6.1–9.9 | +8.3–13.5 | +954–1078 | 1 |

→ 도구 완료 알림 뒤 대기분 확인까지 **6–64 ms** 가 비어 있고(확인은 `tokenUsage/updated` 뒤 2–9 ms), 그 안에 들어온 steer 는 같은 경계에 들어갔다.

**확신도:** claude 확실(10/10 같은 모양 + 코드 판독 일치) · codex 가능성 높음 — 하네스는 0.1–0.4 ms 에 반응했다. **우리 데몬의 반응 지연**(펌프 → 통로 → 쓰기)이 이 창(최소 6 ms)보다 짧은지는 재지 않았고, 도구가 여럿 병렬이거나 모델이 느리면 창 모양이 달라질 수 있다(불확실). 두 M7 은 동시에 돌았다(다른 프로세스 — 반응 경로는 각 하네스의 동기 핸들러라 간섭은 작다고 본다).
**TRD 「빨가면」:** §4-2 S1 칸을 실측으로 바꾼다 — **claude = 「✕ 0/10 놓침」(판독 확인) · codex = 「○ 10/10 맞음(창 6–64 ms)」(판독과 반대).** 결정(두 백엔드 즉시 넘기기)은 바뀌지 않는다. 다만 나중 세분화 때 codex 의 「보관 → 도구 완료 시 넘김」은 **한 걸음 늦지 않는다**는 재료가 된다 — §0.4 · §4-2 판정 칸 「D1 의 『바로』를 놓친다」는 codex 에 대해선 틀렸다.

## 9. M8 — codex 버전을 어디서 읽나 (하한 0.140.0 판정)

**결과:**
- `initialize` 응답 `userAgent` = `engram-dashboard/0.156.1 (Windows 10.0.26200; x86_64) unknown (engram-dashboard; 0.1.0)`.
  - 모양 = `<originator>/<codex 버전> (<OS> <OS 버전>; <arch>) <터미널> (<clientInfo.name>; <clientInfo.version>)`. ★앞머리 `originator` 가 **우리 `clientInfo.name`** 이다★ — codex 가 `initialize` 에서 그것을 전역 originator 로 박는다(0.156.1 `app-server/src/request_processors/initialize_processor.rs:124-175`). 단 env `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` 가 있으면 그 값이 앞머리가 된다(같은 파일 주석).
  - 버전은 첫 `/` 뒤 첫 공백 전(`login/src/auth/default_client.rs` `get_codex_user_agent` — `CARGO_PKG_VERSION`). ★끝의 괄호에 **우리 버전(0.1.0)** 도 들어 있다★ — 「문자열 안 첫 semver」로 찾으면 맞지만 「마지막 semver」로 찾으면 우리 버전을 읽는다.
- **더 깨끗한 출처가 이미 손에 있다:** `thread/start` 응답 `result.thread.cliVersion` = `"0.156.1"`. 우리 `protocol.rs` 의 `Thread.cli_version` 이 이미 읽는 칸이고, `thread/resume` 도 같은 `Thread` 를 준다(스키마 판독 — resume 응답은 이번에 안 쟀다).

**확신도:** 확실(실측 문자열 + 소스). **TRD 「빨가면」:** 해당 없음 — 판정 가능. 권고 순서 = `thread.cliVersion` → 없으면 `userAgent` 의 `^[^/]+/(\d+\.\d+\.\d+)` → 둘 다 실패면 보관 모드(TRD §5-5 그대로).

## 10. 뒷정리 · 비용

- **프로세스:** 하네스가 띄운 자식 6 개(claude 4 · codex 2)는 전부 stdin 을 닫자 스스로 종료했다(로그 `exit:0`, 강제 종료 0 회). 종료 뒤 `Get-CimInstance Win32_Process` 로 하네스·`app-server --stdio`·세션 id 를 품은 명령줄을 훑어 **남은 것 0**. 03:29 이후 생긴 `node_repl.exe` 둘은 부모가 이 측정 전부터 떠 있던 다른 `codex.exe app-server`(03:19 · 전날 23:26 기동)라 이번 것이 아니다.
- **남긴 흔적(버려도 되는 새 세션):** claude transcript 4 개 = `%USERPROFILE%\.claude\projects\C--Users-kimsunzun-AppData-Local-Temp-claude-I--Engram-apps-engram-dashboard-wt1-6847046e-28fb-4531-a649-281ac6ab97c0-scratchpad-phase0-cwd-claude\` · codex rollout 2 개 = `%USERPROFILE%\.codex\sessions\2026\09\25\rollout-…-01a0d4b1-1240-….jsonl` · `…-01a0d4b3-6048-….jsonl`. 기존 사용자 세션은 안 건드렸다.
- **비용:** claude(haiku) 누적 약 $0.28(결과 32 턴) · codex(gpt-6-luna, low) 턴 14 개(M6 4 · M7 10 — 액수 미집계). 재시도 0 회.

## 11. 요약표

| M# | 결과 | 녹/적 | 영향받는 TRD 절 |
|---|---|---|---|
| M1 | `command_lifecycle` 우리 모드에서 나옴 · `queued`→`started`→`completed` · 접힌 입력 `completed` 는 `result` 직전, 턴 연 입력은 직후 | 녹 | §3-1 행 2(미실측 → 실측) · §4-3 유지 |
| M2 | 되울림은 소비 때만 — 접기 경로 = `started` 직전 · 새 턴 경로 = `started` 뒤 ~1.5–2 s · 취소분은 안 옴 · `timestamp` 는 쓴 시각 | 녹(수치) | §3-1 행 10 · §5-7(두 순서 모두 흡수 확인) |
| M3 | 받힘 · 봉투 `response.response.cancelled` · ⓐ `true`+`cancelled`(도구 결과 뒤 540 ms 까지) · ⓑ 선 위 `false`→`started` · ⓒ 자연 경합 없음 / **예약 취소는 턴 도중 안 걸려 전달됨** | 녹(기능) · **적(§3-1 ③ 서술)** | §3-1 행 4·5 · 머리 블록 결정 2 괄호 · §5-4 (판정 규칙은 유지) |
| M4 | init 은 기동 때 없음 · 턴 시작마다 `started` +0.6–0.9 s · `msg_lifecycle_v1` 있음 | 녹(틈 하나) | §3-1 행 8 · §5-4 탐지(첫 턴 초반 오분류 틈) |
| M5 | `attachment{type:"queued_command", prompt, source_uuid, commandMode}` · 취소분은 본문 없음 · 놓친 입력은 평범한 `user` | 녹 | §3-1 행 11 · §5-4 이어받기 복원 · §9 P3 |
| M6 | steer 둘 → 에코 둘 · 끊기 = 에코·이력 없음 · 막 끝난 턴 −32600 · items/list 에 `clientId` · 「기록만」 갈래 미관측 · **끊긴 턴 도구가 늦게 `item/completed`** | 녹 + 새 사실 | §3-2 · §5-5(도구 계수 턴 id 가림 · `Dropped{Unknown}` 갈래 근거 미확인) |
| M7 | claude 0/10(놓침) · codex 10/10(맞음, 창 6–64 ms) | claude 판독 확인 · **codex 판독과 반대** | §0.4 · §4-2 S1 칸 · 판독 근거 줄 |
| M8 | `userAgent` = `<clientInfo.name>/<버전> (…) … (<name>; <우리 버전>)` · `thread.cliVersion` 이 더 깨끗함 | 녹 | §5-5 하한 판정 |

## 12. TRD 가정과 어긋난 것 (인용)

1. §3-1 ③ — 「**아직 안 들어온 uuid 면 「취소 예약」(`markCancelPending`)을 걸고 `cancelled:false` 로 답한다** — 나중에 들어오는 순간(「caught by a pending cancel just before dispatch」) `cancelled` 로 닫힌다」 → 턴 도중(접기)에서는 닫히지 않고 **전달**됐다(M3 c2). 예약은 새 턴 배출에서만 본다(코드). 같은 서술이 머리 블록 결정 2 에도 있다.
2. §0.4 · §4-2 S1 codex 칸 — 「**✕ 거의 늘 놓친다 — 도구 끝나자마자 같은 프로세스 안에서 대기분을 확인한다**」 및 판독 근거 「codex 의 `has_pending_input` 은 도구 완료 알림을 우리에게 보내는 그 프로세스 안에서 곧바로 돈다」 → 10/10 맞았다. 완료 알림과 확인 사이에 6–64 ms 가 있다(M7).
3. §3-1 행 1 — 「**그 사이에 I/O 양보가 없다 — 접기 직전의 `await dc(D,kg)` 는 빈 async 함수다**」 → 접기 안에 대기 명령마다 비동기 「prompt.submit at the fold」 처리(`ah`/`Ed`)가 있고 여기선 ~0.55–0.66 s 걸렸다. ★결론(S1 은 claude 에서 놓친다)은 그대로다★ — 스냅숏이 그 대기보다 **앞**에서 찍히기 때문이다(M7 0/10). 달라지는 것은 「경계 직전까지 취소」의 창이 생각보다 넓다는 것(M3 b1–b4·tb1).
4. §5-5 도구 계수 — 「**턴 경계에서 0 으로**」만으로는 부족하다: 끊긴 턴의 `commandExecution` `item/completed` 가 다음 턴들 뒤에 옛 턴 id 로 온다(M6 T2).
5. 판독 기준 버전 — TRD 는 codex **0.154.0** 소스를 읽었고 설치본은 **0.156.1** 이다. M6·M7·M8 은 0.156.1 실측이다.
