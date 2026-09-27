# 턴 도중 입력 — Phase 0b 벤더 실측 (M9 · M10 · M13) (2026-09-26)

- **상태:** 실측 완료 · TRD 반영 전. `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` §3-3 이 「구현을 막는 측정」으로 남긴 셋(M9 · M10 · M13)과 각 멈춤 조건을 잰 결과다. ★TRD 본문은 이 문서가 고치지 않았다★ — 되적기는 다음 개정의 몫이다(고아 금지: TRD §3-3 에서 이 문서로 링크가 필요하다).
- **소비처:** TRD §3-3(측정 표) · §5-5(codex 처분 — 빚·에코·`-32600`) · §9 P0(멈춤 조건 판정).
- **선행 보고서:** [`mid-turn-phase0-measurements-2026-09-25.md`](mid-turn-phase0-measurements-2026-09-25.md) — 스폰 인자·와이어 형식(§1)은 그대로 재사용했고 여기 되적지 않는다.
- **잰 날:** 2026-09-26 (KST · 로그 시각은 UTC 2026-09-25T15:55–16:02)
- **대상 버전:**
  - codex = `codex-cli 0.156.1` · 모델 `gpt-6-luna` · effort `low`. ★TRD 판독 기준(0.154.0)보다 **새것**이다★ — 아래 codex 결과·소스 줄 번호는 전부 0.156.1 이다.
  - claude = haiku, `claude_harness.js` 경유. ★CLI 버전 문자열은 이번 측정 기록에 남지 않았다★(Phase 0 은 `2.1.280`).
- **목적:** TRD §3-3 의 구현 막는 측정 셋과 멈춤 조건 — M13 적 → P3 전에 멈춤 · M9/M10 적 → P2 전에 멈춤.
- **하네스·원시 로그 위치(저장소 안 · 추적됨):** `.claude/handoff/attachments/20260925-midturn-phase0/`
  - 하네스: `claude_harness.js`(시나리오 M13) · `codex_harness.js`(M9 · M10) · 요약기: `m10codex.js`
  - 원시 로그: `logs/claude-M13-compact-2026-09-25T15-55-09-512Z.jsonl` · `logs/claude-M13-cost-2026-09-25T15-57-33-238Z.jsonl` · `logs/codex-M9-2026-09-25T16-01-30-909Z.jsonl` · `logs/codex-M10-2026-09-25T16-01-58-313Z.jsonl`
  - 래퍼 로그 `run-*.log` 는 `*.log` gitignore 에 걸려 추적되지 않는다.
- **확신도 범례:** 확실(원시 로그에 직접 찍힌 사실, 반복 관측) · 가능성 높음(한두 번 관측 + 코드 판독이 맞물림) · 불확실(추론·단발·외부에서 가를 수 없음)

## 0. 결론 (먼저)

1. **셋 다 녹이다 — 멈춤 조건은 하나도 걸리지 않았다.**
   - **M13(claude 턴 도중 슬래시 명령):** `/compact` 3/3 · `/cost` 3/3. 턴 도중에 쓰면 `queued` 된 뒤 **도구 경계에서 접히지 않고 달리던 턴이 끝날 때까지 붙들렸다가** 새로 `started` 된다. `refused`·`cancelled`·`discarded` 는 한 번도 없었다. → P3 멈춤 없음.
   - **M9(codex 빈 입력 `turn/start`):** `input: []` 가 4/4 받혔다. 진짜 답 못 받은 메시지가 있으면 빈 턴이 **그것 하나에만** 답했다(3/3). → P2 멈춤 없음.
   - **M10(codex 턴 끝 「기록만」):** 4 번 중 3 번 맞혔다. 기록만 된 입력은 `turn/completed` 핸들러에서 곧바로 부른 `thread/items/list` 에 `clientId` 째 실려 있었다(3/3). → P2 멈춤 없음.
2. ★**설계에 걸리는 놀라운 것 둘**★
   - **M9 대조군:** 답 못 받은 것이 **없는데** 빈 턴을 열면 모델이 **직전 답을 한 번 더 말했다**(`READY-7`). 이중 전달은 아니지만 사용자 눈에 중복 답이 보인다 → 빈 턴을 여는 빚은 **그 항목이 정말 답을 못 받았을 때만** 세워야 한다.
   - **M10 늦은 실패:** `-32600 "no active turn to steer"` 가 `turn/completed` 보다 **0.1 ms 먼저** 왔다(`thread/status/changed` idle 보다도 먼저). ★`-32600` 이 늘 `turn/completed` 뒤에 온다고 가정하지 말 것★ — Phase 0 M6 에서는 뒤였다.
3. **M10 — 기록만 된 입력의 에코는 `turn/completed` 전에 온다.** `item/started userMessage`(clientId) 가 `turn/completed` 보다 4.5–4.8 ms 앞서고, 끝나는 턴의 id 를 단다(3/3). Phase 0 M6 이 못 맞혀 미확인으로 남겼던 근거(「기록된 입력은 완료 알림 전에 에코된다」)가 이번에 섰다. 탐침도 같은 항목을 싣는다 — 에코와 탐침 어느 쪽으로도 판정이 선다.

## 1. M13 — claude 턴 도중 `/compact` · `/cost`

**방법:** `claude_harness.js` 시나리오 M13, 여섯째 인자 `slashCmd`(기본 `/compact`). 턴 A 가 Bash `sleep 12` 를 부르게 하고, 그 `tool_use` 약 2 s 뒤 슬래시 명령을 사용자 줄로 쓴다(턴 도중 2 회) · 따로 한가할 때 1 회. 명령마다 한 실행.

**증거 — `/compact` 턴 도중**(`logs/claude-M13-compact-…jsonl`):

```
tool_use(Bash sleep 12) → +~2 s 우리가 /compact 씀 → queued (+1 ms)
tool_result → A result/success → A completed
→ /compact started (A completed 1 ms 뒤)
→ system/status compacting → compact_result success (14–18 s)
→ system/init → compact_boundary
→ user "This session is being continued..."
→ isReplay "<local-command-stdout>Compacted</local-command-stdout>"  (우리가 안 보낸 uuid)
→ isReplay (우리 uuid) "<command-name>/compact</command-name>"
→ result success, result ""  → completed  (result 가 completed 0.6 ms 앞)
```

**증거 — `/cost` 턴 도중**(`logs/claude-M13-cost-…jsonl`): `queued` 13230.4 → A `completed` 27644.8 까지 붙들림 → `started` 27645.7 → `completed` 30014.9.

**결과:**

| 질문 | 결과 | 확신도 |
|---|---|---|
| 턴 도중 슬래시 명령이 받히나 | **예** — `/compact` 2/2 · `/cost` 2/2 가 `queued` | 확실 |
| 도구 경계에서 접히나 | **아니다** — 달리던 턴이 `completed` 된 뒤 1 ms 안팎에 새로 `started` | 확실 |
| 한가할 때 | `queued` → `started` 1 ms 안 · 꼬리는 턴 도중과 같다(`/cost` 는 ~1.3 s 에 `completed`) | 확실 |
| `refused`·`cancelled`·`discarded` | **한 번도 없음**(6/6) | 확실 |
| 끝 순서 | `result`(빈 문자열) → `completed` 0.6 ms — M1 이 턴 연 명령에서 본 순서와 같다 | 확실 |

- ★**`/compact` 는 우리가 안 보낸 uuid 의 `isReplay` 줄을 하나 더 낸다**★ — `<local-command-stdout>Compacted</local-command-stdout>`. 되울림 대조를 「내가 보낸 uuid 」로만 하면 이 줄은 모르는 줄로 떨어진다.
- ★**놀라운 것 — 한가할 때의 `/compact` 가 앞선 compact 의 「Compacted」 stdout 줄을 다시 되울렸다**★(uuid `6905adf2`). 중복 되울림을 새 사건으로 세지 말 것.
- 비용: `/compact` 실행 `total_cost_usd` ≈ $0.128.

**멈춤 조건(M13 적 → P3 전에 멈춤):** 걸리지 않음.

## 2. M9 — codex 빈 입력 `turn/start`

**방법:** `codex_harness.js` 시나리오 M9(대조군) · 변형 A 는 M10 시나리오가 맞힐 때마다 이어서 돈다(로그의 `m9summary` 줄). 보낸 것 = `turn/start {threadId, input: []}`, `clientUserMessageId` 없음.

**결과:**

| 질문 | 결과 | 확신도 |
|---|---|---|
| 빈 `input` 이 받히나 | **예** 4/4 — 응답 ~50 ms `{"turn":{"id":…,"items":[],"itemsView":"notLoaded","status":"inProgress"}}` → `turn/started` → 샘플링 → `turn/completed` completed | 확실 |
| 사용자 항목·에코가 생기나 | **없다** — `userMessage` 항목도 에코도 없음 | 확실 |
| 변형 A — 진짜 답 못 받은 메시지가 있을 때 | 빈 턴의 유일한 `agentMessage` 가 **바로 그 메시지에** 답했다(PINEAPPLE2/3/4) 3/3 | 확실 |
| 변형 B(대조) — 답 못 받은 것이 없을 때 | 모델이 **직전 답을 되풀이**(`READY-7`) — 사용자에게 중복 답이 보인다 | 가능성 높음(1 회) |

- **소스(0.156.1):** 빈 입력은 `start_or_steer` 의 idle 갈래를 탄다 — `core/src/session/turn_input.rs:290-365`.
- ★**설계 함의**★: 빈 턴은 이중 전달을 만들지 않지만 **헛빚이면 보이는 중복 답**을 만든다. 빚은 「그 항목이 정말 답을 못 받았다」가 확인됐을 때만 세운다(TRD §5-5).
- 로그: `logs/codex-M9-2026-09-25T16-01-30-909Z.jsonl`(B) · `logs/codex-M10-2026-09-25T16-01-58-313Z.jsonl`(A, `m9summary` 줄).

**멈춤 조건(M9 적 → P2 전에 멈춤):** 걸리지 않음.

## 3. M10 — codex 턴 끝 「기록만」 갈래

**방법:** 도구 없는 턴(「reply with exactly: ACK-k」). 그 턴의 **첫** `thread/tokenUsage/updated` 핸들러 안에서 d ms 바쁜 대기 후 steer. `turn/completed` 핸들러에서 곧바로 `thread/items/list {limit:40, desc}` + `limit:2` desc 커서 걷기(앞 턴 id 가 든 페이지가 나올 때까지). d 는 적응형(시작 `M10_D0`=1.0 ms). 맞힘 판정 = steer 가 턴 id 로 수락 · 답은 ACK-k 하나뿐 · 두 번째 샘플링 없음 · 다음 빈 턴이 PINEAPPLEk 에 답함.

**시도:**

| d (ms) | steer 시점(tokenUsage 기준) | 결과 |
|---|---|---|
| 1.0 | +1.4 ms | 늦음 → `-32600 "no active turn to steer"` |
| 0.53 | +0.6 ms | **맞힘** |
| 0.41 | +0.5 ms | **맞힘** |
| 0.36 | +0.4 ms | **맞힘** |

- 이른 쪽 가장자리는 **안 봤다**(3 맞힘에서 멈췄다). 창은 tokenUsage 를 받은 뒤 대략 (0, 1.4) ms 안이다 — ★추론이고 불확실★. Phase 0 M6 T4 는 ~+0 ms 에 보낸 steer 가 같은 턴의 후속 샘플링으로 들어갔다.

**결과(TRD 질문별):**

| # | 질문 | 결과 | 확신도 |
|---|---|---|---|
| ① | 탐침 시점에 `clientId` 째 실려 있나 | **예** — `turn/completed` 핸들러의 탐침이 0.9–1.4 ms 뒤 돌아왔고 steer 한 `userMessage`+`clientId` 가 index 0 에 있었다(3/3) | 확실 |
| ② | 에코가 오나 · 언제 | **예, `turn/completed` 전** — `item/started userMessage`(clientId) 가 `turn/completed` 4.5–4.8 ms 앞, `item/completed` 는 ~2.3 ms 뒤 · steer 성공 응답 ~2.4 ms 뒤 · 끝나는 턴의 id 를 단다 | 확실 |
| ③ | 탐침이 「없음」이라 한 뒤 에코가 오나 | 해당 없음 — 탐침이 「없음」을 낸 적이 없다 | — |
| ④ | 턴 id · 위치 | 끝난 턴 **자기** id(steer 응답과 같다) · desc index 0(마지막 `agentMessage` 뒤) · 턴 안 오름차순 = U, agentMessage, W | 확실 |
| ⑤ | `limit:2` 페이지 걷기 | page 0 = [W, agentMessage](찾음) · page 1 = [U, 앞 턴의 agentMessage] → 2 페이지 만에 턴을 지남 · 걷기 1.9–3.5 ms | 확실 |

- **기록만이 턴 끝을 늘린다:** tokenUsage → `turn/completed` 가 8.1–8.5 ms(기록만 없을 때 3.3–3.6 ms).
- ★**`turn/completed` 페이로드로는 가를 수 없다**★ — `itemsView "summary"` 는 `agentMessage` 만 싣는다. 기록만 된 항목은 에코나 탐침으로만 보인다.
- ★**놀라운 것 — 늦은 실패의 `-32600` 이 `turn/completed` 보다 0.1 ms 먼저 왔다**★(`thread/status/changed` idle 보다도 먼저). `-32600` 을 받은 시점에 그 턴의 `turn/completed` 가 아직 안 왔을 수 있다.
- **소스(0.156.1):**
  - 마지막 대기 입력 확인 — `core/src/session/turn.rs:555-567`
  - 창이 닫히는 곳 — `on_task_finished` 의 `active_turn.task.take()`, `core/src/tasks/mod.rs:640-650`
  - `TurnComplete` 전에 기록 — `take_pending_input_for_turn_state` `:655` · `run_hooks_and_record_inputs` `:673`
  - task 가 `None` 이 된 뒤 steer 거절 — `turn_input.rs:634-638`
- 로그: `logs/codex-M10-2026-09-25T16-01-58-313Z.jsonl`.

**멈춤 조건(M10 적 → P2 전에 멈춤):** 걸리지 않음.

## 4. 재현

- **시나리오:**
  - claude: `claude_harness.js` 시나리오 `M13` + 여섯째 인자 `slashCmd`(기본 `/compact`) → 로그 이름 `claude-M13-<cmd>-<stamp>.jsonl`.
  - codex: `codex_harness.js` 시나리오 `M9`(대조군) · `M10`(사냥 + 맞힘마다 M9 변형 A). 보조 함수 `spin`·`pick`·`emptyTurnStart`·`walkItems`. 환경변수 `M10_MAX`(40) · `M10_HITS`(3) · `M10_D0`(1.0 ms).
  - 요약: `node m10codex.cjs <log> [verbose]`.
- ★**ESM 함정**★ — 저장소 `package.json` 이 `"type":"module"` 이라 저장소 안에서 `node <harness>.js` 를 돌리면 `require is not defined` 로 죽는다. **저장소 밖으로 `.cjs` 로 복사해서** 거기서 돌린다.
- **실행:** 셸 트리 밖에서 `scripts/run-detached.ps1` 로 띄우고, 완료는 `__EXIT` 마커로 판정한다(선행 보고서와 같다).

## 5. 뒷정리 · 비용

- **프로세스:** codex 쪽 남은 프로세스 없음. claude 쪽도 없음 — `/compact` 실행의 자식(36388)은 stdin 이 닫히자 스스로 0 으로 끝났고, `/cost` 실행은 래퍼 로그에 `__EXIT=0` 이 찍힌 뒤 자식 PID 29624 가 목록에서 사라진 것을 메인이 `tasklist` 로 확인했다(2026-09-26).
- **남긴 흔적(버려도 되는 새 스레드):** codex 스레드 2 개 = `%USERPROFILE%\.codex\sessions\2026\09\26\`. 기존 사용자 세션은 안 건드렸다.
- **비용:** codex 2 실행(M9 ~9 s · M10 ~30 s) · 턴 9 개 · 5 시간 창의 4%. claude `/compact` 실행 ≈ $0.128(`/cost` 실행 액수는 기록 없음).

## 6. 한계

- **한 세션 · 한 모델 · 한 버전이다.** codex 는 `0.156.1` · `gpt-6-luna` 한 스레드씩, claude 는 haiku 한 세션씩이다. TRD 판독 기준 `0.154.0` 에서 다시 재지 않았다.
- **M10 창의 이른 가장자리는 못 봤다** — 3 맞힘에서 멈췄으므로 (0, 1.4) ms 는 위쪽만 실측이고 아래쪽은 추론이다.
- **정산 기한(settlement-deadline) 갈래는 안 탔다** — 탐침이 늘 제때 항목을 찾아 기한이 지나는 경우가 생기지 않았다. 「탐침 없음 → 나중 에코」(③)도 같은 이유로 미관측이다.
- **사용자 codex hooks 설정을 들여다보지 않았다.** 대기 입력 훅은 기록 경로 위에서 돈다 — 느린 `UserPromptSubmit`·`Stop` 훅은 창을 **넓힐** 뿐 창 안에서 일어나는 일은 바꾸지 않는다(소스 판독 · 가능성 높음).
- M9 대조군(중복 답)은 1 회 관측이다.

## 7. 요약표

| M# | 결과 | 녹/적 | 영향받는 TRD 절 |
|---|---|---|---|
| M13 | `/compact`·`/cost` 턴 도중 3/3씩 받힘 · 도구 경계에서 안 접히고 턴 끝까지 붙들림 · 거절·취소·버림 0 · `/compact` 는 남의 uuid `isReplay` 한 줄 더(한가할 때 앞 compact 줄 재되울림) | 녹 | §3-3 · §9 P0(P3 멈춤 없음) |
| M9 | `input: []` 4/4 받힘 · 에코·항목 없음 · 답 못 받은 것 있으면 그것에만 답(3/3) · **없으면 직전 답 되풀이** | 녹 + 설계 함의 | §3-3 · §5-5(빚은 진짜 미응답일 때만) · §9 P0 |
| M10 | 3/4 맞힘(창 (0, 1.4) ms 추정) · 탐침에 `clientId` 째 index 0 · **에코가 `turn/completed` 4.5–4.8 ms 전** · 끝난 턴 id · `limit:2` 2 페이지 · **`-32600` 이 `turn/completed` 보다 먼저 올 수 있음** | 녹 + 새 사실 | §3-3 · §5-5(에코 근거 섬 · `-32600` 순서 가정 제거) · §9 P0(P2 멈춤 없음) |

> **정정(2026-09-26, M16):** 벤더의 마지막 대기분 확인은 같은 턴 안에서 `run_turn` 을 다시 도는 작업 고리 `core/src/tasks/regular.rs:120` 이다 — 위 §3 의 `core/src/session/turn.rs:555-567` 가리킴은 부정확하다. 근거·실측 = [M16 보고서](mid-turn-m16-measurements-2026-09-26.md).
