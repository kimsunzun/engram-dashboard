# codex fixtures

실측 `codex app-server --stdio` stdout 줄(JSON-RPC 알림·응답)을 한 줄에 하나씩, 일어난 순서대로 담는다. `claude/fixtures` 관례와 같이 **서버가 낸 줄만** 싣는다 — 우리가 보낸 요청은 싣지 않고, 아래 「응답 id → 요청」 표로 적는다.

## 공통 가공

- **원본:** 로그 파일의 `{t,dir,line}` 봉투에서 `dir:"out"` 인 `line` 만 꺼냈다. `meta`·`in` 은 버렸다. 「줄 범위」는 그 로그 파일의 0 기반 줄 번호다. 로그 폴더는 둘이다:
  - `codex-M*.jsonl` = `.claude/handoff/attachments/20260925-midturn-phase0/logs/`. 각 로그가 무엇을 쟀는지는 `docs/research/mid-turn-phase0-measurements-2026-09-25.md`(M6·M7) · `docs/research/mid-turn-phase0b-measurements-2026-09-26.md`(M9).
  - `codex-B5*.jsonl` = `.claude/handoff/attachments/20260928-chat-ux/logs/`(S21 chat-ux B5 채취 · 무엇을 재나 = `docs/process/S21-chat-ux/trd.md` §4-7 ⑨). ★원 로그가 아니라 **가린 사본**이다★ — 원 로그 전체를 아래 치환에 한 번 통과시켰고 줄 수 · 줄 번호는 원 로그와 같다(`meta`·`in` 도 남아 있다). 원 로그는 저장소에 없다. 드라이버 · 가공기 = 같은 폴더의 `b5codex.cjs` · `b5fixture.cjs`(`log` 방식이 사본을, `fixture` 방식이 이 폴더의 줄을 짓는다).
- **빼낸 잡음 줄:** `mcpServer/startupStatus/updated` · `skills/changed` · `account/updated` · `account/rateLimits/updated` · `remoteControl/status/changed`(기계 이름이 실려 있다) · `thread/started`. 핸드셰이크 응답(`initialize` id 0 · `thread/start` id 1)은 범위 밖이라 없다 — 모든 파일이 첫 `turn/start` 응답에서 시작한다.
- **개인정보 치환:** cwd → `C:\work\proj` · cwd 밖 대상 폴더(승인 채취) → `C:\work\outside` · OS 사용자 홈 → `C:\home\user`. 가린 로그 사본에는 잡음 줄이 남아 있어 더 가렸다 — 기계 이름 → `HOST` · `installationId` → `00000000-0000-0000-0000-000000000000` · `planType` → `"redacted"`. `b5fixture.cjs` 는 사용자 이름 · 기계 이름 · 설치 id · 채취 세션 경로 · 메일 주소가 하나라도 남으면 쓰지 않고 실패한다. 스레드·턴·항목 id 는 서버 난수라 그대로 뒀다.
- codex-cli 0.156.1 · `-c model=gpt-6-luna -c model_reasoning_effort=low -c notify=[]`(Phase 0 보고서 §1 · B5 채취도 같다).
- **승인 채취 조건(Windows · B5 채취 셋째에서 배웠다):** cwd 밖 대상이 `%TEMP%` 아래면 codex 가 그곳을 `workspace-write` 의 쓰기 가능 루트로 잡아 **승인 요청 없이** 쓴다(첫 채취에서 패치가 그대로 적용됐다). 그래서 ① `-c sandbox_workspace_write.exclude_tmpdir_env_var=true` 를 더하거나 대상을 `%TEMP%` 밖에 둔다 ② 자식 env 에서 `TMPDIR` 를 뺀다(Git Bash 가 내보낸다 — 데몬 스폰에는 없다) ③ 그 채취를 믿기 전에 `thread/start` 응답의 `sandbox`(`writableRoots` · `excludeTmpdirEnvVar`)를 본다. B5 채취는 셋 다 `TMPDIR` 를 뺐고, ① 은 채취 셋째(`refuse_u2a`)에만 더했다.
- **손으로 지은 줄은 없다** — 전부 실측이다.

## 파일

시험이 읽는 곳은 `include_str!` 로 찾는다(`rg "fixtures/" crates/engram-dashboard-agent/src/backend/codex`) — 목록을 여기 적지 않는다. 지금 아무 시험도 읽지 않는 것은 `refuse_u2a.jsonl` 하나다(우리 거절 귀속 시험이 읽을 자리 — TRD §4-7 ⑨). `steer_m6` · `empty_turn_m9` 는 「도구 끝 결과가 하나도 없다」는 회귀 확인으로만 읽힌다. `record_only_m10.jsonl` 은 그것을 읽던 후속 턴 빚 시험과 함께 걷었다(ADR-0235).

| 파일 | 줄 | 원본 · 줄 범위 | 보여 주는 것 |
|---|---|---|---|
| `steer_m6.jsonl` | 50 | `codex-M6-2026-09-24T18-32-49-086Z.jsonl` 15–133 · 188 | **T1:** 도구(`Start-Sleep 20`) 중 steer 둘 → 각 `{turnId}` 수락 → 도구 `item/completed` → `tokenUsage` → `clientId` 단 `userMessage` 에코 둘(친 순서) → 답 "DONE-1 MANGO LEMON" → `turn/completed` → 그 핸들러에서 보낸 steer 가 `-32600 "no active turn to steer"`. **T2:** 도구 중 steer Z 수락 → `turn/interrupt` → `turn/completed(interrupted)`, Z 에코 없음. **T3:** 도구 없는 턴의 `agentMessage` `item/completed` 에서 steer W → 같은 턴의 후속 샘플링으로 소비(에코 → 두 번째 답). **마지막 줄:** T2 의 `commandExecution` `item/completed`(옛 턴 id · status `completed`) — 끊기 17 s 뒤, 원본에선 T4 가 끝난 **뒤** 도착. ★T4(원본 134–187)는 뺐고, 크기 때문에 이 파일만 `item/agentMessage/delta` 줄도 뺐다(`item/completed` 가 전체 본문을 싣는다)★. |
| `empty_turn_m9.jsonl` | 24 | `codex-M9-2026-09-25T16-01-30-909Z.jsonl` 14–32 · 37–49 | 평범한 턴("READY-7") 뒤 **답 못 받은 메시지가 없을 때** `turn/start {input:[]}` → 받힘(`items:[]` `inProgress`) → `userMessage` 없음 → 모델이 직전 답 "READY-7" 을 되풀이(M9 변형 B). |
| `tool_end_m7.jsonl` | 19 | `codex-M7-2026-09-24T18-35-20-013Z.jsonl` 15–42 (trial 1) | 도구 `commandExecution` `item/completed` 를 받은 즉시 steer → `{turnId}` → `tokenUsage` → `clientId` 단 `userMessage` 에코 → 다음 샘플링 전에 같은 경계로 들어감 → 답 "OK-1" 하나 → `turn/completed`. |
| `tool_fail_u2.jsonl` | 25 | `codex-B5A-2026-09-28T05-15-42-947Z.jsonl` 16–44 | 0 아닌 종료 명령(`exit 7`) 한 턴(S21 chat-ux B5 채취 첫째 · 2026-09-28) — `item/started` `commandExecution`(`inProgress`) → 같은 id 의 `item/completed` **`status: failed` · `exitCode: 7`** · `aggregatedOutput: null` → 답 "DONE-A" → `turn/completed(completed)`. 승인 요청 없음. |
| `interrupt_fail_b5.jsonl` | 11 | `codex-B5B-2026-09-28T05-16-55-018Z.jsonl` 14–33 | 끊긴 뒤 실패하는 명령(B5 채취 둘째 · 2026-09-28) — `item/started` `commandExecution`(`Start-Sleep -Seconds 8; exit 5`) → 2 s 뒤 `turn/interrupt` → `tokenUsage` → `{}` → `turn/completed(interrupted)` → 약 7 s 뒤(명령이 제 시간에 끝난 자리 · 다음 턴 없이 한가한 채로) **같은 id · 옛 턴 id** 의 `item/completed` **`failed` · `exitCode: 5`** · `aggregatedOutput: null`. 그 뒤 32 s 동안 줄 없음(원본 35 줄 `meta`). 같은 조건의 앞선 채취(저장소에 없음)도 같은 모양이었다. |
| `refuse_u2a.jsonl` | 79 | `codex-B5C-2026-09-28T05-19-49-528Z.jsonl` 15–111 | 우리 거절 두 턴(B5 채취 셋째 · 2026-09-28). **T1 명령:** `thread/status/changed(waitingOnApproval)` → `item/started` `commandExecution`(`source: agent` · `processId: null`) → 서버 요청 `item/commandExecution/requestApproval`(`itemId` = 시작 item id · `approvalId` 없음) → (우리 `-32601`) → `serverRequest/resolved` → 같은 id 의 `item/completed` **`failed` · `exitCode: null` · `aggregatedOutput: null`** → 답 → `turn/completed(completed)`. **T2 파일 변경:** `item/started` `fileChange`(`inProgress`) → `waitingOnApproval` → `item/fileChange/requestApproval`(`itemId` = 시작 item id · `reason`·`grantRoot` 는 `null`) → (우리 `-32601`) → `serverRequest/resolved` → 같은 id 의 `item/completed` **`declined`** → 답 → `turn/completed(completed)`. 두 턴 다 `error` 알림 없음. ★채취 조건이 다른 파일과 다르다★ — `-c sandbox_workspace_write.exclude_tmpdir_env_var=true` 를 더했다(위 「승인 채취 조건」 · 그 응답의 `sandbox` = `writableRoots: []` · `excludeTmpdirEnvVar: true`). 그것 없이 뜬 첫 채취는 패치가 승인 없이 적용돼 버렸다(저장소에 없음). **이어받은 이력**(기록만 · `codex-B5R-2026-09-28T05-21-02-206Z.jsonl` — 새 app-server 로 `thread/resume` → `thread/items/list` desc): 거절된 명령 item 은 **없고** 파일 변경은 `declined` 그대로다. |

## 응답 id → 요청

| 파일 | id | 요청 |
|---|---|---|
| `steer_m6` | 2 · 7 · 11 | T1 · T2 · T3 `turn/start`(`clientUserMessageId` 실음) |
| | 3 · 4 | T1 `turn/steer` X · Y(도구 중) |
| | 5 | T1 `turn/steer`(옛 턴 id, `turn/completed` 핸들러 안) → `-32600` |
| | 6 · 10 | T1 · T2 뒤 `thread/items/list {limit:40, sortDirection:"desc"}` |
| | 8 · 9 | T2 `turn/steer` Z · `turn/interrupt` |
| | 12 | T3 `turn/steer` W(`agentMessage` `item/completed` 핸들러 안) |
| `empty_turn_m9` | 2 · 4 | 평범한 `turn/start` · 빈 `turn/start {input:[]}`. id 3(목록 조회 응답, 원본 34 줄)은 뺐다 |
| `tool_end_m7` | 2 · 3 | `turn/start` · `turn/steer`(도구 `item/completed` 핸들러 안) |
| `tool_fail_u2` | 2 | `turn/start` |
| `interrupt_fail_b5` | 2 · 3 | `turn/start` · `turn/interrupt`(명령 `item/started` 2 s 뒤 · 답 `{}`) |
| `refuse_u2a` | 2 · 3 | T1 · T2 `turn/start` |
| | 0 · 1(`method` 실음) | ★응답이 아니라 **서버 요청**이다★(T1 명령 승인 · T2 파일 변경 승인) — 우리 답 `{"id":0 또는 1,"error":{"code":-32601,…}}` 은 싣지 않았다(리더가 그 줄을 보고 스스로 거절한다) |
