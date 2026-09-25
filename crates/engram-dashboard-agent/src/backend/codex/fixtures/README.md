# codex fixtures

실측 `codex app-server --stdio` stdout 줄(JSON-RPC 알림·응답)을 한 줄에 하나씩, 일어난 순서대로 담는다. `claude/fixtures` 관례와 같이 **서버가 낸 줄만** 싣는다 — 우리가 보낸 요청은 싣지 않고, 아래 「응답 id → 요청」 표로 적는다.

## 공통 가공

- **원본:** `.claude/handoff/attachments/20260925-midturn-phase0/logs/codex-*.jsonl` 의 `{t,dir,line}` 봉투에서 `dir:"out"` 인 `line` 만 꺼냈다. `meta`·`in` 은 버렸다. 「줄 범위」는 그 로그 파일의 0 기반 줄 번호다. 각 로그가 무엇을 쟀는지는 `docs/research/mid-turn-phase0-measurements-2026-09-25.md`(M6·M7) · `docs/research/mid-turn-phase0b-measurements-2026-09-26.md`(M9·M10).
- **빼낸 잡음 줄:** `mcpServer/startupStatus/updated` · `skills/changed` · `account/updated` · `account/rateLimits/updated` · `remoteControl/status/changed`(기계 이름이 실려 있다) · `thread/started`. 핸드셰이크 응답(`initialize` id 0 · `thread/start` id 1)은 범위 밖이라 없다 — 모든 파일이 첫 `turn/start` 응답에서 시작한다.
- **개인정보 치환:** cwd → `C:\work\proj` · OS 사용자 홈 → `C:\home\user`. 스레드·턴·항목 id 는 서버 난수라 그대로 뒀다.
- codex-cli 0.156.1 · `-c model=gpt-6-luna -c model_reasoning_effort=low -c notify=[]`(Phase 0 보고서 §1).
- **손으로 지은 줄은 없다** — 전부 실측이다.

## 파일

| 파일 | 줄 | 원본 · 줄 범위 | 보여 주는 것 |
|---|---|---|---|
| `steer_m6.jsonl` | 50 | `codex-M6-2026-09-24T18-32-49-086Z.jsonl` 15–133 · 188 | **T1:** 도구(`Start-Sleep 20`) 중 steer 둘 → 각 `{turnId}` 수락 → 도구 `item/completed` → `tokenUsage` → `clientId` 단 `userMessage` 에코 둘(친 순서) → 답 "DONE-1 MANGO LEMON" → `turn/completed` → 그 핸들러에서 보낸 steer 가 `-32600 "no active turn to steer"`. **T2:** 도구 중 steer Z 수락 → `turn/interrupt` → `turn/completed(interrupted)`, Z 에코 없음. **T3:** 도구 없는 턴의 `agentMessage` `item/completed` 에서 steer W → 같은 턴의 후속 샘플링으로 소비(에코 → 두 번째 답). **마지막 줄:** T2 의 `commandExecution` `item/completed`(옛 턴 id · status `completed`) — 끊기 17 s 뒤, 원본에선 T4 가 끝난 **뒤** 도착. ★T4(원본 134–187)는 뺐고, 크기 때문에 이 파일만 `item/agentMessage/delta` 줄도 뺐다(`item/completed` 가 전체 본문을 싣는다)★. |
| `record_only_m10.jsonl` | 47 | `codex-M10-2026-09-25T16-01-58-313Z.jsonl` 15–39 (trial 1) · 43–67 (trial 2) · 73–86 (trial 2 뒤 빈 턴) | **trial 1(늦음):** 첫 `tokenUsage` 뒤 1.4 ms 에 steer → `-32600` 이 `thread/status idle`·`turn/completed` **보다 먼저** 온다. **trial 2(맞힘 — 「기록만」):** 0.6 ms 에 steer → `{turnId}` → `userMessage` 에코(`item/started`)가 `turn/completed` 앞 → 답은 "ACK-2" 하나, 두 번째 샘플링 없음 → `turn/completed` 핸들러의 `thread/items/list` 탐침(limit 40 · limit 2 페이지 둘)이 steer 항목을 `clientId` 째 싣는다. **빈 턴(M9 변형 A):** `turn/start {input:[]}` → 기록만 된 메시지에 답 "PINEAPPLE2". |
| `empty_turn_m9.jsonl` | 24 | `codex-M9-2026-09-25T16-01-30-909Z.jsonl` 14–32 · 37–49 | 평범한 턴("READY-7") 뒤 **답 못 받은 메시지가 없을 때** `turn/start {input:[]}` → 받힘(`items:[]` `inProgress`) → `userMessage` 없음 → 모델이 직전 답 "READY-7" 을 되풀이(M9 변형 B). |
| `tool_end_m7.jsonl` | 19 | `codex-M7-2026-09-24T18-35-20-013Z.jsonl` 15–42 (trial 1) | 도구 `commandExecution` `item/completed` 를 받은 즉시 steer → `{turnId}` → `tokenUsage` → `clientId` 단 `userMessage` 에코 → 다음 샘플링 전에 같은 경계로 들어감 → 답 "OK-1" 하나 → `turn/completed`. |

## 응답 id → 요청

| 파일 | id | 요청 |
|---|---|---|
| `steer_m6` | 2 · 7 · 11 | T1 · T2 · T3 `turn/start`(`clientUserMessageId` 실음) |
| | 3 · 4 | T1 `turn/steer` X · Y(도구 중) |
| | 5 | T1 `turn/steer`(옛 턴 id, `turn/completed` 핸들러 안) → `-32600` |
| | 6 · 10 | T1 · T2 뒤 `thread/items/list {limit:40, sortDirection:"desc"}` |
| | 8 · 9 | T2 `turn/steer` Z · `turn/interrupt` |
| | 12 | T3 `turn/steer` W(`agentMessage` `item/completed` 핸들러 안) |
| `record_only_m10` | 2 · 6 | trial 1 · 2 `turn/start` |
| | 3 · 7 | trial 1 · 2 `turn/steer`(첫 `tokenUsage` 핸들러 안, 1.0 · 0.53 ms 바쁜 대기 뒤) |
| | 4 · 8 | `turn/completed` 핸들러의 `thread/items/list {limit:40, desc}` |
| | 5 · 9 · 10 | `thread/items/list {limit:2, desc}` — trial 1 페이지 0 · trial 2 페이지 0 · 1(`cursor`) |
| | 12 | `turn/start {input:[]}`(빈 턴 — `clientUserMessageId` 없음). id 11(안정화 뒤 목록 조회, 원본 70 줄)은 뺐다 |
| `empty_turn_m9` | 2 · 4 | 평범한 `turn/start` · 빈 `turn/start {input:[]}`. id 3(목록 조회 응답, 원본 34 줄)은 뺐다 |
| `tool_end_m7` | 2 · 3 | `turn/start` · `turn/steer`(도구 `item/completed` 핸들러 안) |
