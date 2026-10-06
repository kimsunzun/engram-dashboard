# TRD — 경계 리팩터링 2-1: 셸 → agent 의존을 끊는다 (S21)

> 상태: **4판(2026-10-06) — `/review trd` 2라운드(codex PASS · doc-aware FIX) 반영.** 코드는 아직 한 줄도 바뀌지 않았다.
> **결정 출처:** 사용자 위임(2026-10-06 「알아서 해 … 단계 착수」) → 메인 결정. D1~D7 · §8 의 O1~O5 처리 · 3판의 리뷰 반영(F1 의 「감수한 대가」 포함)이 여기서 온다. 그 밖에 D4 의 「LLM 통지 없음」 · 「warn 로그 유지」는 ADR-0270 결정 4 의 사용자 결정(2026-10-02)이다.
> **개정(2026-10-06, 4판 — 리뷰 2라운드):** ① U3 에 슬롯 계약 주석 두 자리(`connection.rs:61-64` · `:183`)와 `:971` 앵커를 넣었다(N1 · §2-4 · §3-1) ② U3 안 순서 — `:971` 을 맨 뒤에 뒤집는다(N2 · §3-4) ③ U1 시험 ㉢ 에 표 밖 낱말(N3 · §3-2) ④ `layout_commands` 바이너리는 데몬을 **부른다**(`:3033`) — 사실을 바로잡고 「전」 측정을 U2 ① 앞으로(N4 · §2-5 · §3-3 · §9) ⑤ `send_command` 글 불변 시험을 바이트 대조로(N5 · §3-4) ⑥ 표 doc 에 「`refusal` 을 `Some` 으로 바꾸기 전에 ADR-NNNN 재론」(F1 잔여 · §2-1) ⑦ 새 칸이 드는 구조체 · 생성자 자리(잔 지적 · §2-1 · §2-4 · §3-1).
> **개정(2026-10-06, 3판 — 리뷰 1라운드):** ① D4 의 데몬 거절 판정을 문자열 코드 가르기에서 **답 경계의 출처 타입**으로 바꿨다 — 모르는 · 새 데몬 코드도 박스가 뜬다(codex C1 · §2-4 · §3-4 · §7 3) ② D1 의 문은 호출자를 가리지 않는다 — 사람 경로가 「LLM 제어 표면」 정책을 보는 것을 「감수한 대가」로 적고 재론 계기를 달았다 · agent 주석을 사실대로 고치는 일을 U1 에 넣었다(F1 · §2-1 · §7 1) ③ 거절 갈래의 dispatch 수준 시험 — `ConnectionCore` 에 정책 seam(F2 · §2-1 · §3-2) ④ 거부 (a) 에서 철자 사유를 걷었다(F3 · §1-5 · §2-1) ⑤ §7 1 에 ADR-0270 「영향」의 「정책을 보는 문은 데몬 `agent.new` 하나」를 더했다(F4) ⑥ 단위 파일 목록 · 문서 후속 보강(F5 · F6 · §2-5 · §3-1 · §6) ⑦ 잔 지적 넷(문구 대소문자 · 메모 §11 인과 · U1 주석이 커밋마다 참 · 로컬 문구 목록은 C1 로 걷힘).
> **개정(2026-10-06, 2판):** ① D1 거부 사유에서 「슬롯 즉석 에이전트가 영속 프로필이 된다」를 지웠다(§2-1 끝 「바로잡은 사실」 · §8 O1) ② `SpawnByCwd` 가 명부 상한에 닿는다로 바로잡았다(§1-5 · §8 O2) ③ D4 트리거를 데몬이 낸 거절 전부로 넓혔다(§8 O3) ④ O5 를 메모 §11 한 줄로 넘겼다.
> ★**착수 전제 — D1 은 ADR-0270 결정 3.1 의 부분 개정이다 → 새 ADR(ADR-NNNN(예정) · §7)을 U1 착수 전에 박는다**★(CLAUDE.md 「설계 결정 기록」 — 굵은 결정은 결정 즉시).
> **범위 = 작업 순서 2-1 전부**(`docs/refactoring/architecture-discussion-2026-09-26.md` §3 결정 후보 5 · §10 2-1) — 슬롯 스폰의 백엔드 정책 벽을 데몬에 세우고, 셸의 정책 검사와 예약 목록의 데몬 절반을 걷어 셸의 agent 운영 의존을 지우고, 등록 거절을 OS 메시지 박스로 알린다.
> **배치 근거:** `docs/README.md` 「새 내용을 어디에 넣나」의 「새 기능 **설계 착수** → `process/SN-name/`」. 같은 단계의 형제(1-1 · 1-3 TRD)가 이 폴더에 있다.
> **표기:** 「실측」 = 기준 커밋 `cf693f4`(master — 브랜치 `v0.3.3/refactor/crate-boundaries` 의 HEAD `a65ffa7` 와 코드가 같다. 차이는 `docs/decisions/` 두 파일뿐)에서 잰 것과 그 명령 · 「결정(메인 2026-10-06)」 = 위 「결정 출처」의 것 · 「ADR」 = 확정 ADR 본문 · 「제안」 = 이 TRD 의 안(새 이름은 전부 가안).
> 앵커: **ADR-0270**(이 단계의 헌장 — 결정 3.1 을 ADR-NNNN(예정)이 부분 개정) · ADR-0219(정책 표와 그것을 보는 문 · 결정 5 claude 고정 문) · ADR-0029(셸 = 데몬 클라이언트) · ADR-0172(즉석 생성은 「마지막 실패」를 쓰지 않는다) · ADR-0155(등록 반려 계약) · ADR-0012(seam · 단독 하네스) · ADR-0006(락 밖 외부 호출) · ADR-0175(셸 dev 그래프의 agent) · step-log S21.

---

## 0. 결론 (먼저)

```
① 정책 벽 = 데몬. 데몬의 SpawnByCwd 처리부가 프로필을 만들기 전에 agent 의 llm_creation_refusal 을 부른다(D1).
     슬롯 스폰 경로 · 왕복 수 · 배치 순서는 그대로다. 셸 agent.spawnInto 와 프론트의 SpawnByCwd 두 경로가 함께 덮인다.
     그 문은 호출자를 가리지 않는다 — 사람 경로도 같은 표를 본다(감수한 대가 · 오늘 영향 0).
② 셸은 낱말 오탈자 그물(parse_backend)만 남기고, 예약 목록은 셸 자기 표 절반만 남기고, agent 의존을 지운다(D2 · D3).
     셸 시험은 agent 이름을 부르지 않게 고쳐 쓰고, 정책 주인의 단언은 데몬 · agent 시험으로 옮긴다(D5).
③ 게이트 — 셸 운영 그래프(normal · build)에 agent 0줄 + 짝(데몬 ≥ 1줄). dev 그래프는 일부러 뺀다(D6).
④ 데몬이 등록 · 차분에 Error 로 답하면 OS 메시지 박스 — 코드가 무엇이든(모르는 코드 포함), 셸 로컬 실패는 빼고.
     가름은 문자열이 아니라 답 경계에서 남기는 출처 타입이다. tauri-plugin-dialog Rust API · 새 좁은 포트 ·
     같은 (자리, 문구)는 셸 프로세스당 한 번(D4).
⑤ 단위 셋 U1 → U2 → U3. 어느 커밋에서 멈춰도 빌드 · 회귀가 초록이다. U2 · U3 은 /qa full(GUI).
```

**결정 출처:** 사용자 위임(2026-10-06 「알아서 해 … 단계 착수」) 아래의 메인 결정이다(머리말).

**지금 하지 않는 것:** 슬롯 스폰을 `agent.new` 로 옮기기(D1 이 거부) · 셸 dev 그래프에서 agent 걷기(셸 → (dev) 데몬 → agent — 작업 순서 2-4) · 등록 거절 뒤 재시도 · 스폰 패킷에 호출자 축 넣기(§2-1 「감수한 대가」의 재론 계기 때) · 프론트 `CreateProfile` 경로의 정책(범위 밖 — 메모 §11 · §8 O5).

---

## 1. 현황 실측 (`cf693f4`)

### 1-1. 셸이 쓰는 agent 심볼 — 넷(운영 둘 · 시험 둘)

| 자리 | 심볼 | 쓰임 | 처분 |
|---|---|---|---|
| `src-tauri/src/layout/apply.rs:36` · `:558-572` | `commands::llm_creation_refusal` | `gate_backend` = `parse_backend`(`:553`) + 정책 | D2 — 정책 몫 · import 삭제, 그물은 남김 |
| `src-tauri/src/view_commands.rs:196-210` | `commands::COMMAND_SPECS` | `reserved_names` = agent 절반 ∪ 셸 표 절반 | D3 — agent 절반 삭제 |
| `src-tauri/tests/layout_apply.rs:37` | `llm_creation_refusal` · `AgentNewArgs` · `LLM_BACKEND_POLICY` | `every_creation_door_reads_one_backend_policy`(`:1408`) | D5 |
| `src-tauri/tests/layout_commands.rs:2686` | `COMMAND_SPECS` | `the_registration_packet_never_carries_a_name_the_daemon_answers_itself`(`:2685`) | D5 |

`normalize_cwd` 는 이미 base `path::normalize_spelling` 이다(1-1 — `layout/commands.rs:38` · `:1127`). 그 밖의 쓰임 0(`rg engram_dashboard_agent src-tauri` → 위 넷 + `apply.rs:559` doc 링크). 매니페스트 = `src-tauri/Cargo.toml:71`(주석 `:64-70`) · `Cargo.lock` 의 셸 항목 `dependencies` 에도 이 이름이 있다 → 의존을 지우면 lock 도 바뀐다(CI `--locked`).

### 1-2. 슬롯 스폰 경로

- **셸** `apply::spawn_into`(`:574-644`): 0) 낱말 그물 + 정책(`:586-589`) · 탭/슬롯 가드(`:590-596`) → 1) `spawner.spawn_by_cwd`(`:598` · 락 밖) → 2) 배치(`:600-` · 한 임계구역). ★1) 이 `?` 로 돌아오면 2) 에 닿지 않는다 — 레이아웃 불변★(시험 `spawn_into_propagates_spawn_failure_untouched` — `tests/layout_apply.rs:1603`).
- **셸 문은 지금도 호출자를 가리지 않는다** — Tauri 명령 `spawn_into`(`src-tauri/src/commands/layout.rs:492-519`)도 버스 동사 `agent.spawnInto`(`layout/commands.rs:1100-1153`)도 같은 `apply::spawn_into` → `gate_backend` 를 지난다. 그래서 프론트 `agent.spawnInto`(`src/commands/tabCommands.ts:238`)는 사람이 불러도 오늘 이미 정책 표를 본다.
- **운영 포트** `DaemonSpawner`(`commands/layout.rs:84-113`) → `AgentCommand::SpawnByCwd`. ★그 `match` 의 `AgentEvent::Error` 팔은 닿지 않는다★ — `send_command` 가 `Error` 를 이미 `Err(message)` 로 바꿔 돌려준다(`daemon_client/protocol_state.rs:29-37` · `connection.rs:971`). 그래서 데몬 문구가 접두 없이 그대로 호출자에 간다 — 버스 `agent.spawnInto` 는 그 문구를 `CONFLICT` 로 싣는다(`layout/commands.rs:1152` · `:1159-1161`).
- **데몬** `SpawnByCwd`(`crates/engram-dashboard-daemon/src/connection_core.rs:1455-1500`): `spawn_command_by_cwd`(`:654-661`) → 없으면 `MISSING_BACKEND`(`:628` · `:1466-1469`) → `CoreProfile::new(.., auto_restore=false)`(`:1470-1476`) → `manager.spawn_agent(.., Fresh)`(`:1483`). ★정책을 안 본다★. 즉석 생성이라 「마지막 실패」를 쓰지 않는다(`:1477-1482` · ADR-0172).
- **`SpawnByCwd` 를 보내는 곳** — 셸 `DaemonSpawner`(위 두 셸 경로) · 프론트 `protocolClient.spawnAgent`(`src/api/protocolClient.ts:992-994`)를 부르는 `slot.createAgentHere`(`src/commands/slotContentCommands.ts:80`)와 프론트 `agent.spawn`(`src/commands/agentCommands.ts:104`) — 뒤 둘은 `'claude'` 고정이고 셸 정책 검사를 지나지 않는다. 프론트 `agent.spawn` 은 ADR-0219 결정 5 가 「일부러 claude 에 못박은」 문 둘 중 하나다(다른 하나 = agent 버스 `agent.spawn` 의 `create_and_start` — `agent/src/commands.rs:967-1007` · 정책을 안 본다).
- **정책을 보는 곳(오늘)** — agent `verb_new`(`crates/engram-dashboard-agent/src/commands.rs:1078`) · 셸 `gate_backend`. 표 `LLM_BACKEND_POLICY`(`:645`)는 claude · codex 를 다 열어 **오늘 거절 0**이다.

### 1-3. 예약 이름 · 등록 거절

- `reserved_names()`(`view_commands.rs:204`) → 다리 생성(`:370-371`) → `report`(`:436-481`)가 빼서 `refused` 에 담고, 그것은 로그 한 줄(`commands/view_bus.rs:53`)뿐이다.
- 등록 패킷 = 셸 표 선언 + 웹뷰 선언 **한 패킷**(`daemon_client/inbound.rs:183-187`). 명부는 한 패킷 안 같은 이름을 오류 없이 접는다(`crates/engram-dashboard-command/src/roster.rs:130` · 시험 `:624`).
- 데몬 거절 `refuse_names_i_answer`(`connection_core.rs:1160-1190`) = **데몬이 답하는 이름만** · 패킷 통째 · `CONFLICT`. 등록(`:1806`) · 차분(`:1839`) 둘 다 이것을 지나고, 데몬 시험 `registering_a_name_this_daemon_answers_is_refused_as_a_conflict`(`:4159`)가 두 갈래를 잰다. 명부 자기 거절도 있다 — 남의 이름 = `CONFLICT`(`roster.rs:248`) · 크기 · 상한 = `INVALID_ARGUMENT`(`:283-320` · 상한값 `:82-100`). 거절은 `reply_roster`(`connection_core.rs:1234-1251`)가 `Error{message: "{CODE}: {message}"}` 로 싣고(`command/src/error.rs:307-310`), wire `Error` 에는 코드 칸이 없다(`protocol/src/messages.rs:698-701`).
- **셸의 처리 자리 둘** — 등록 `register_own_commands`(`daemon_client/connection.rs:1924-1976` · 결말 태스크 `:1961-1976` · warn `:1969`) · 차분 `push_delta`(`commands/view_bus.rs:63-100` · warn `:96`).
- ★**답 경계에서 출처가 사라진다**★ — 연결 루프가 데몬의 `Error` 답을 `reply_outcome`(`protocol_state.rs:29-37`)으로 `Err(message)` 로 접어 대기 슬롯에 넣는데(`connection.rs:971`), 같은 슬롯의 `Err` 에는 셸 로컬 실패도 들어간다 — 끊김 `SENT_OUTCOME_UNKNOWN`(`:677`) · 미전송 `UNSENT_ON_DISCONNECT`(`:702` · `:1535`) · 송신 · 직렬화 실패(`:1361` · `:1377`) · request_id 없음(`:1343`) · 중복 번호 `PENDING_SUPERSEDED`(`:1799`). `send_command`(`daemon_client/mod.rs:804-831`)도 미연결 · 채널 끊김 · 답 유실을 같은 `Err(String)` 으로 낸다. 그래서 지금 두 자리는 「데몬이 거절했다」와 「셸이 못 보냈다 · 못 받았다」를 타입으로 못 가르고, 등록 자리의 「거절」 warn 은 끊김에도 찍힌다(`Err(_)` 팔 `:1974` 는 사실상 안 닿는다 — §8 O4). → D4 는 그 경계에서 출처를 남긴다(§2-4).
- 연결 태스크의 바깥 포트는 `DaemonEvents` 하나다(`daemon_client/events.rs:75` — 「그보다 넓히지 말 것」 `:68` · 머리 `:3-8` 「`AppHandle` 을 쥐던 유일한 사유는 emit … 그래서 emit 만 포트로 끊는다」). 연결 태스크는 `AppHandle` 을 쥐지 않는다.

### 1-4. 의존 그래프 (실측 `cargo tree`)

- 셸 직접 워크스페이스 의존(normal · build) = agent · base · command · discovery · net · protocol. 2-1 뒤 agent 만 빠진다.
- `cargo tree --locked -p engram-dashboard -e normal,build --target all --all-features --prefix none | rg "^engram-dashboard-agent "` → **지금 1줄**(직접 간선 하나 — 다른 normal 경로 없음). 같은 꼴 `-p engram-dashboard-daemon` → 1줄.
- dev 그래프 — 셸 → (dev) 데몬(`src-tauri/Cargo.toml:108`) → agent. 2-1 뒤에도 남는다(ADR-0270 「영향」 · 작업 순서 2-4).
- `cargo tree -i engram-dashboard-agent` 는 대상이 그래프에 없으면 **rc=101 로 죽는다**(실측 `-p engram-dashboard-net -e normal,build`).
- net 을 셸이 쓰는 모양(기본 feature)으로 보면 워크스페이스 의존 0, `server` 를 켜면 {platform, protocol}(실측 `--depth 1` 두 조합 · `net/Cargo.toml:31-41`). → `src-tauri/Cargo.toml:82` 의 「forward 폐포는 {net, agent, protocol}」은 낡았다.

### 1-5. 기록과 어긋난 사실

| 기록 | 실제(실측 `cf693f4`) |
|---|---|
| 메모 §3 · ADR-0270 「맥락」 — 셸이 agent 에서 「세 가지」를 쓴다 | 둘(운영). `normalize_cwd` 는 1-1 이 base 로 옮겼다 — 셸 `Cargo.toml:64` 주석은 이미 「둘」 |
| ADR-0270 결정 3.1 · 메모 §3 · §10 — 「슬롯 스폰을 데몬 `agent.new` 명령 경로로」 | `agent.new` 는 등록만 한다(잠든 상태 — `commands.rs:194`). 띄우려면 `agent.spawn {target}` 이 한 번 더 필요하다. → D1 |
| 조사 입력 — 「`agent.new` 의 `backend` 는 PascalCase 라 셸이 lowercase wire 낱말과 갈린다(손 JSON 필요)」 | **철자는 갈리지 않는다** — 명령 표가 부르기 전에 열거 칸의 대소문자를 선언 철자로 옮긴다(`command/src/table.rs:130-131` → `coerce::enum_words_to_declared_spelling` · `coerce.rs:101-109`). 셸이 인자 JSON 을 손으로 지어야 하는 이유는 하나 — agent 의 인자 타입(`AgentNewArgs`)을 못 쓴다(2-1 의 목적) |
| ADR-0270 「맥락」 · 메모 §3 — 거절 처리 자리 = `connection.rs` 하나 | 둘(차분 `view_bus.rs:96`) |
| 메모 §3 줄 번호 `view_commands.rs:204` · `apply.rs:597` · `:585-620` · `layout/commands.rs:35` · agent `commands.rs:1077` | `:205` · `:598` · `:574-644` · `:38`(base import) · `:1078` |
| 메모 §11 둘째 줄 — 셸 `Cargo.toml` 주석 「`COMMAND_SPECS` 하나만 남았다(실제 셋)」 | 실제 둘이고 주석(`:64`)은 이미 고쳐졌다. 2-1 이 의존째 지운다 |
| agent `commands.rs:600-614` — 정책을 묻는 「LLM 제어 표면이 셋」 중 ③ 프론트 `humanOnly` | 2026-09-22 에 걷혔다(`src/commands/agentCommands.ts:41`). 정책을 보는 문은 지금 둘이다 |
| 조사 입력의 표 — `SpawnByCwd` 는 명부 상한에 「닿지 않는다」 | **닿는다**(메인 확인 2026-10-06 · §8 O2) — 즉석 신규 등록 갈래가 `check_roster_capacity` 를 본다(`agent/src/manager.rs:1148-1151`). 다만 `CONFLICT` 같은 타입 코드가 아니라 문자열 오류로 나간다 |

---

## 2. 결정 (메인 2026-10-06) · 설계

| | 결정 | 한 줄 이유 |
|---|---|---|
| **D1** | 데몬 `SpawnByCwd` 처리부가 스폰 전에 정책을 본다 | 벽 하나 = 데몬(ADR-0270 의 뜻) · 슬롯 동작과 왕복 수 불변 · 프론트 `SpawnByCwd` 경로도 덮는다 |
| **D2** | 셸은 낱말 그물(`parse_backend`)만 남긴다 | 그물은 agent 가 필요 없다(protocol enum 에 묻는다) |
| **D3** | 예약 목록의 agent 절반만 지우고 셸 표 절반은 남긴다 | 셸 · 웹뷰 같은 이름은 데몬이 못 본다(§2-3 코드) |
| **D4** | 데몬이 등록 · 차분에 `Error` 로 답하면(코드 무관 · 셸 로컬 실패 제외) 두 자리에서 OS 메시지 박스 — 출처는 답 경계의 타입으로 · tauri-plugin-dialog Rust API · 새 좁은 포트 | ADR-0270 결정 4 · 어느 코드의 거절이든 셸 명령이 데몬에 안 오른다 · OS 분기를 셸에 들이지 않는다 |
| **D5** | 셸 시험은 agent 를 부르지 않게 고쳐 쓰고 정책 주인의 단언은 데몬 · agent 로 | 셸에 agent dev 의존을 두지 않는다 |
| **D6** | 게이트 「셸은 agent 를 의존하지 않는다」(normal · build 0줄 + 짝) | ADR-0270 불변식의 기계 벽 · `-i` 함정 회피 |
| **D7** | 지나가며 — 낡은 주석 둘 · ADR-0219 는 새 ADR 의 링크로 | — |

### 2-1. D1 — 정책 벽은 데몬 `SpawnByCwd` 처리부

- **고른 것:** 데몬이 `SpawnByCwd` 를 받으면 프로필을 만들기 전에 정책(운영 = agent `llm_creation_refusal`)을 부르고, 거절이면 `Error` 로 답한다 — 셸이 이미 그대로 띄우는 오류 경로다(§1-2).
- **이유:** ADR-0270 의 뜻(정책 벽은 데몬)을 지키면서 슬롯 동작 · 왕복 수 · 배치 순서가 그대로다. 프론트가 직접 보내는 `SpawnByCwd` 두 경로(`slotContentCommands.ts:80` · `agentCommands.ts:104`)도 함께 덮는다. 셸이 처음으로 버스 호출(`AgentCommand::Command`)을 내는 생산자가 되지 않는다(셸 Rust 에 그 생산자 0 — 조사 · `messages.rs:645` 주석).
- **거부:**
  - (a) 슬롯 스폰을 `agent.new` + `agent.spawn {target}` 으로 — `agent.new` 는 등록만 해(잠든 상태) 왕복이 둘이고, 둘째가 실패하면 잠든 고아가 남고, 셸은 agent 의 인자 타입(`AgentNewArgs`)을 못 쓰므로 인자 JSON 을 손으로 지어야 하고, 프론트의 `SpawnByCwd` 슬롯 경로는 여전히 정책을 비켜 간다.
  - (b) 만들고 띄우기를 한 번에 하는 새 데몬 동사 — D1 보다 얻는 것 없이 표면만 는다.
- **ADR:** ADR-0270 결정 3.1 부분 개정 → ADR-NNNN(예정) §7.
- ★**감수한 대가 — 이 문은 호출자를 가리지 않는다**★(메인 결정 2026-10-06 · 리뷰 F1): 정책 이름은 「LLM 제어 표면」이지만 데몬 `SpawnByCwd` 는 누가 보냈는지 모른다. 그래서 2-1 뒤 **사람 경로도 같은 표를 본다** — 새로 닿는 것은 claude 고정 두 문(`slot.createAgentHere` · 프론트 `agent.spawn` — 뒤의 것은 ADR-0219 결정 5 의 「일부러 claude 에 못박은」 문)이고, 프론트 `agent.spawnInto` 는 **새 노출이 아니다**(셸 문이 이미 호출자를 가리지 않고 표를 본다 — §1-2). 그 결과 같은 이름의 두 문이 갈린다 — 프론트 `agent.spawn` 은 표를 보고 agent 버스 `agent.spawn`(`create_and_start`)은 안 본다. **오늘 영향 0** — 표가 아무것도 안 닫고 claude 가 열려 있다. ★**재론 계기: 어느 백엔드든 LLM 에게 닫히거나 선언이 빠지는(`NO_POLICY_DECLARED`) 날 — 그때 스폰 패킷에 호출자 축을 더한다.**★
- ★**바로잡은 사실 — 「영속 프로필이 되느냐」는 (a) 와 D1 을 가르는 축이 아니다. 거부 사유로 다시 꺼내지 말 것**★(메인 결정 2026-10-06 · §8 O1): 지금 `SpawnByCwd` 도 즉석 프로필을 명부에 올리고 디스크에 쓴다 — `spawn_agent` → `register_for_spawn` → `upsert_preserving_hierarchy`(`agent/src/manager.rs:1161`) → `store.save`(`agent/src/profile.rs:387-395`) · protocol doc `messages.rs:113-115` 「spawn 경로가 그 프로필을 registry 에 등록·persist 한다」. 두 경로 모두 `auto_restore = false` 다(`connection_core.rs:1470-1476` · agent `commands.rs:1114`). 철자도 사유가 아니다(§1-5).

**설계(제안):**

- **자리** = `MISSING_BACKEND` 판정(`connection_core.rs:1466-1469`) 바로 뒤 · `CoreProfile::new`(`:1470`) 앞. ★프로필을 만들기 전이어야 한다★ — 뒤에 두면 거절된 즉석 프로필이 명부 · 디스크에 남는다(위 「바로잡은 사실」의 사슬).
- **정책 seam = `ConnectionCore` 의 칸 하나**(가안 `llm_policy: fn(&str) -> Option<&'static str>`) — `ConnectionCore::new`(`:1096-1107`)가 `llm_creation_refusal` 로 채운다. 시험은 `#[cfg(test)]` 빌더(가안 `with_llm_policy`)로 닫는 가짜를 꽂는다 — 운영 생성자 시그니처는 그대로라 호출자(`agent_conn.rs:587` · 시험 조립 `connection_core.rs:2751`)를 안 건드린다. 구조체 리터럴은 `new` 안 한 곳(`:1108-1119`)뿐이다. ★구조체 doc(`:1067-1071`)은 「이 struct 는 연결마다 새로 만들어지고 필드는 전 연결이 공유하는 핸들의 clone 이다 — 공유 핸들이 아닌 값을 넣으면 연결마다 별개인 상태가 생긴다」를 경고한다★ — 새 칸은 상태 없는 순수 함수 포인터라 연결마다 달라질 상태가 없다. 그 사실을 칸 doc 에 적고 구조체 doc 의 경고에 예외 한 줄을 단다(U1). 오늘 표가 아무것도 안 닫아 실물 입력으로는 거절 갈래에 닿을 수 없어서 seam 이 필요하다(ADR-0012). 판정 · 문구는 순수 도우미 하나(가안 `by_cwd_refusal(kind, policy) -> Option<String>`)가 진다. `spawn_command_by_cwd` 와 그 시험(`:4934`)은 건드리지 않는다.
- **낱말** = wire enum 직렬화(serde — `#[serde(rename_all = "lowercase")]`, `protocol/src/domain.rs:209-215`)로 얻는다. 손으로 적지 않는다(셸 시험 `wire_word` 와 같은 수법 — `tests/layout_apply.rs:1369`). 표 비교는 대소문자를 무시한다(`commands.rs:675-680`).
- **문구** = 지금 셸 문구의 틀 그대로 — 「`backend '{word}' 는 아는 낱말이지만 이 표면으로는 지금 만들지 않는다 — {reason} 스폰 안 함.`」(`apply.rs:568-570`). ★다른 것은 `{word}` 의 대소문자 하나다★ — 셸은 친 철자를 다듬어(`trim`) 그대로 실었고(`Codex` 도 통과한다 — protocol 역직렬화가 대소문자를 무시한다, `domain.rs:235-245`), 데몬은 enum 을 직렬화해 늘 lowercase(`codex`)를 싣는다. 오탈자 문구(「를 모른다」)와는 그대로 갈린다.
- **「마지막 실패」를 쓰지 않는다** — 활성화도 즉석 생성도 일어나지 않았다(ADR-0172 결정 1 · 4 — 안 한 일에는 기록할 실패가 없다).
- **와이어 · 카탈로그** — `PROTOCOL_VERSION` · 셸 `CATALOG_VERSION` 그대로(패킷 모양도 오늘 동작도 안 바뀐다).
- **앵커** `// ADR-0270` · `// ADR-NNNN`.
- **agent 주석을 사실대로(U1 · 커밋마다 참이게):** ① 정책 구획 머리(`commands.rs:593` 「LLM 제어 표면의 백엔드 생성 정책」 · `:600-614` 「LLM 제어 표면이 셋」) · 표 doc(`:629` 「LLM 제어 표면이 어느 백엔드를 만들 수 있나」) · 판정 함수 doc(`:668-673` 「사람이 쓰는 문에서는 그대로 만들어진다 · 막는 것은 사람이 아닌 호출자뿐」)에 **데몬 `SpawnByCwd` 문은 호출자를 가리지 않고 이 판정을 적용한다**(사람 경로도 막힌다 — 위 「감수한 대가」)를 적는다. ★표 doc(`:629`)에는 한 줄을 더한다 — 「어느 줄의 `refusal` 을 `Some` 으로 바꾸거나 선언 없는 낱말을 들이기 전에 ADR-NNNN 을 다시 연다 — 데몬 `SpawnByCwd` 문이 사람 경로도 막는다」★(재론 계기를 그 편집이 일어나는 자리에 둔다). ② 「문」 목록은 U1 커밋에서 **데몬 `SpawnByCwd` 문을 더하고 셸 `gate_backend` 문은 「2-1 U2 가 걷는다 — 그때까지 데몬 문보다 먼저 같은 표를 본다」로 남긴다** — U1 시점에 셸 문이 아직 살아 있으므로. U2 커밋이 그 줄을 지운다. ③(걷힌 `humanOnly`)은 U1 에서 사실대로 고친다. `:1073-1077` 「형제 문의 자리」도 같은 규칙.

### 2-2. D2 — 셸은 낱말 그물만

- `gate_backend`(`apply.rs:558-572`) 삭제 · 0) 단계(`:586-589`)가 `parse_backend` 를 직접 부른다 · import(`:36`) 삭제. `parse_backend` 는 protocol enum 에만 묻는다(agent 불필요).
- **거절 문구가 데몬을 한 번 다녀온다** — 거절은 1) 단계의 `?`(`:598`)에서 돌아와 2) 배치에 닿지 않는다 → 레이아웃 불변(§1-2). 탭/슬롯 가드(`:590-596`)는 여전히 스폰 전이다.
- 머리 주석(`:531-556`)의 「두 축」 서술을 「셸 = 오탈자 그물 · 정책 = 데몬(ADR-NNNN)」으로 고친다. 「두 거절을 한 문구로 뭉치지 말 것」은 그대로 산다(데몬 문구 ≠ 그물 문구).
- 버스 오류 코드 그대로 — 지금의 정책 거절도 2-1 뒤의 데몬 거절도 `not_applied` → `CONFLICT`(§1-2).

### 2-3. D3 — 예약 목록: 셸 표 절반은 남긴다 (코드로 판단)

- agent 절반(`view_commands.rs:205`)만 지운다.
- **셸 표 절반은 할 일이 남아 있다(코드)** — ① 셸 표 선언과 웹뷰 선언이 한 패킷으로 간다(`inbound.rs:183-187`) ② 명부는 한 패킷 안 같은 이름을 오류 없이 접는다(`roster.rs:130` · 시험 `:624`) ③ 데몬 거절은 데몬이 답하는 이름만 본다(`connection_core.rs:1160-1190`). 그래서 웹뷰 레지스트리의 `tab.create` · `slot.close` 에 누가 `help` 를 달면 그 절반 말고는 막는 것이 없고 거절 박스도 안 뜬다. ADR-0270 결정 2 · 그 「거부한 대안」 둘째와 같은 결론이다.
- `reserved_names` 는 남고 doc · 모듈 머리(`view_commands.rs:8-15` · `:196-203`)를 고친다 — 「데몬 이름의 벽 = 데몬 거절 + 거절 박스(U3)」. 「데몬 어휘를 전부 덮지는 못한다(`mail.*`)」 문장은 뜻을 잃는다(셸은 데몬 이름을 더는 안 본다).
- 오늘 걸러 내는 이름은 0 이다(코드 읽기 — `help` 를 단 웹뷰 명령은 `tab.next` · `slot.empty` 뿐 · ADR-0270 「맥락」) → 앱 동작 불변. GUI 로 확인한다(§5-1).

### 2-4. D4 — 거절을 OS 메시지 박스로

- **고른 것:** 등록(`connection.rs:1969`) · 차분(`view_bus.rs:96`) **두 자리 모두** · tauri-plugin-dialog 의 **Rust API** · `DaemonEvents` 를 넓히지 않는 **새 좁은 포트** + 시험 대역 · 지금의 warn 로그는 매번 그대로(라벨도 그대로 — §8 O4) · LLM 통지 없음(사용자 결정 — ADR-0270 결정 4).
- **트리거 = 데몬이 이 등록 · 차분에 `Error` 로 답한 것 전부(메인 결정 2026-10-06 · §8 O3 · 리뷰 C1)** — 그 답은 「적용되지 않았다」이고 결과가 같다(등록이면 셸 명령이 0 개, 차분이면 그 차분이 데몬에 안 오른다). 오늘 실물은 `CONFLICT`(데몬 이름 · 남의 이름)와 `INVALID_ARGUMENT`(이름 128 B · 설명 4096 B · 주인당 512 · 전체 4096 — §1-3)이고, ★**모르는 · 새 코드도 똑같이 띄운다**★ — 코드 추가는 허용된 확장이다(`command/src/error.rs` 의 `ErrorCode` doc 「코드 추가는 additive」). **빼는 것은 셸 로컬 실패뿐**이다(§1-3 의 목록).

**출처를 답 경계에서 타입으로 남긴다(리뷰 C1 — 문자열 코드 가르기를 걷었다):**

- **바뀌는 자리 = 셋, 그 밖의 호출자는 그대로:**
  1. `connection.rs:971` — 데몬의 짝 답을 접지 않고 그대로 슬롯에 넣는다(`Ok(ev)` — `Error` 도 `Ok(AgentEvent::Error{..})` 로). 그러면 대기 슬롯(`CommandReply` = `oneshot::Sender<Result<AgentEvent, String>>` — `:65`, **타입은 그대로**)의 `Err` 에는 **셸 로컬 실패만** 남는다 — §1-3 의 로컬 송신 자리 일곱(`:677` · `:702` · `:1343` · `:1361` · `:1377` · `:1535` · `:1799`)은 손대지 않는다. ★그 계약을 적은 주석 두 자리를 함께 고친다★ — `CommandReply` doc(`:61-64` — 지금 「`Err(msg)` = 데몬 Error 또는 연결 끊김」)과 `SendCommand` doc(`:183` — 「send/끊김 실패 시 Err」)에 「**데몬 답을 슬롯에 넣는 자리는 `:971` 하나이고 `Error` 도 `Ok` 로 넣는다 · 슬롯의 `Err` 는 셸 로컬 실패뿐이다 · `Error` 를 `Err` 로 접지 말 것 — 접으면 거절 박스가 조용히 죽는다**」를 적는다. `:971` 에는 `// ADR-NNNN` 앵커를 단다.
  2. `protocol_state.rs:29-37` `reply_outcome(ev)` → 슬롯 결과를 출처 타입으로 바꾸는 함수 하나(가안 `classify_reply(Result<AgentEvent, String>) -> Result<AgentEvent, CommandFailure>`) · `enum CommandFailure { Daemon { message: String }, Local(String) }`(가안) · `Display` = 지금과 같은 글(`Daemon` = 데몬 문구 원문 · `Local` = 로컬 문구). 판정: `Ok(Error{message,..})` → `Daemon` · 그 밖의 `Ok(ev)` → `Ok(ev)` · `Err(s)` → `Local(s)`. **문자열을 읽지 않는다** — 코드 접두가 있든 없든 모르는 코드든 데몬이 답했으면 `Daemon` 이다.
  3. `DaemonClient` 에 출처를 지키는 형제 하나(가안 `send_command_with_origin(cmd) -> Result<AgentEvent, CommandFailure>` — `mod.rs:804-831` 의 본문을 옮기고, 미연결 · 채널 끊김 · 답 유실 · request_id 없음(`:806`)은 `Local`) · 기존 `send_command` 는 그것을 불러 `CommandFailure` 를 `String` 으로 바꾼다(`to_string`). ★기존 호출자(`commands/agent.rs` 의 여섯 · `commands/layout.rs:100` · 시험)가 받는 글은 바이트 단위로 같다★ — 지금도 데몬 `Error` 는 `Err(message)` 로, 로컬 실패는 그 상수 글로 왔다.
- **두 자리:** 등록 결말 태스크(`connection.rs:1961-1976`)는 `outcome_rx` 의 결과를 같은 함수로 가른다 — `Daemon` → warn(지금 라벨 그대로) + 박스 · `Local` → warn(같은 라벨 — §8 O4) 만 · `Ok` → info. 차분 자리(`view_bus.rs:94-100`)는 `send_command_with_origin` 을 불러 같은 규칙.
- **안 바뀌는 것:** wire(데몬 `reply_roster` · `Error` 모양) · 대기 슬롯 타입 · `register_pending` · `src-tauri/tests/daemon_client_pending.rs`(로컬 `Err` 만 다룬다).
- **고른 모양의 대가와 다른 안:** 슬롯의 `Err` 가 「로컬뿐」인 것은 타입이 아니라 배치(데몬 답을 받는 곳이 `:971` 하나)가 지킨다 — 누가 그 자리에서 다시 `Error` 를 `Err` 로 접으면 박스가 조용히 죽는다. 그것을 시험 ㉢(실 소켓 왕복으로 데몬 `Error` → 박스)이 잡는다. 슬롯 `Err` 를 새 타입(가안 `LocalFailure`)으로 바꿔 컴파일러가 지키게 하는 안도 있으나 로컬 송신 일곱 자리 · `register_pending` · 시험 두 파일(`daemon_client/tests.rs:3125` · `:3639` · `tests/daemon_client_pending.rs`)로 번져 고르지 않았다(최소 변경 — 리뷰 C1 의 「keep it minimal」).

**Rust API 실측(박스):**

- 잠긴 판 = `tauri-plugin-dialog` 2.7.1(`Cargo.lock:4226-4227`) · 직접 의존(`src-tauri/Cargo.toml:48`) · 등록(`src-tauri/src/lib.rs:44`).
- 소스 = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tauri-plugin-dialog-2.7.1/src/` — `DialogExt::dialog()`(`lib.rs:83-91` · `Manager` 를 구현하는 `AppHandle` 등) → `Dialog::message(text)`(`:173`) → `MessageDialogBuilder::{title, kind, buttons, show, blocking_show}`(`:283-367`) · `MessageDialogKind::{Info, Warning, Error}`(`models.rs:10-17`).
- `show(f)` 는 막지 않는다 — 안에서 `run_on_main_thread` 로 창을 띄우고 기다림은 따로 띄운 스레드가 진다(`desktop.rs:215-245`). `blocking_show` 는 「main thread 에서 쓰지 말 것」(`lib.rs:351-358`) → 쓰지 않는다.
- **권한(capability) 불필요 — 확신: 확실(소스 독해)** — `dialog:allow-message` 는 JS 쪽 IPC 명령 `message` 를 여닫는 권한이고(`permissions/autogenerated/commands/message.toml:6-13`), Rust API 는 IPC 를 거치지 않고 `show_message_dialog` 를 바로 부른다. `src-tauri/capabilities/*.json` 은 그대로(`dialog:allow-open` 만).
- **OS 분기 없음** — 창의 OS 차이는 플러그인(rfd 0.16) 안이다. ADR-0270 결정 4 「Tauri 의 대화상자 기능 뒤」와 맞고 셸에 `cfg` 가 들지 않는다(platform 게이트 4f · 4h 명단 불변).

**박스 쪽 설계(제안 — 이름 가안):**

- 새 파일 `src-tauri/src/daemon_client/refusal.rs`:
  - 포트 `RefusalNotice: Send + Sync { fn show(&self, site: RefusalSite, daemon_message: &str); }` · `RefusalSite = Register | Update`. `DaemonEvents` 와 별개다(그 doc `events.rs:68` 이 넓히기를 금한다 — 그 파일 머리 `:3-8` 의 「emit 만 포트로 끊는다」에 형제 포트 한 줄을 더한다).
  - 운영 어댑터 `TauriRefusalBox(AppHandle)` — `app.dialog().message(..).title(..).kind(MessageDialogKind::Warning).show(|_| {})`. 실패는 삼킨다(`DaemonEvents` 와 같은 계약).
  - 시험 대역 `RecordingRefusals`(`#[cfg(test)]`) — (자리, 문구)를 기록한다.
  - 거르개 `RefusalAlerts { port, shown: Mutex<HashSet<(RefusalSite, String)>> }` · `surface(site, daemon_message: &str)` — **`CommandFailure::Daemon` 일 때만 불린다**(판정은 위 출처 타입이 이미 했다 — 여기서 문자열을 읽지 않는다):
    1. **되풀이 억제** — 같은 (자리, 데몬 문구)를 이 프로세스에서 이미 띄웠으면 안 띄운다. 판정은 락 안 · 포트 호출은 락을 놓은 뒤(ADR-0006).
    2. info 로그 한 줄(띄움 / 눌러 둠) — GUI 실측이 박스 결정을 로그로도 센다(§5-2).
  - 소유 — `DaemonClient`(`daemon_client/mod.rs`)가 `Arc<RefusalAlerts>` 칸을 하나 얻어 연결 태스크에 넘긴다(`events` 와 같은 길). ★그 구조체 리터럴은 넷이고 넷 다 칸을 채운다★ — `events` 를 채우는 자리와 같다: `new_with_events`(`:239` · 리터럴 `:246` · 시험) · `new_with_router`(`:275` · `:281` · 시험) · `new_with_router_and_timeout`(`:300` · `:307` · 시험) · `new_real_with_owned_runtime`(`:326` · `:337` · 운영). 운영은 `TauriRefusalBox(app.clone())`, 시험 셋은 대역(또는 아무것도 안 하는 대역 — `NoDaemonEvents` 와 같은 꼴). 데몬 쪽 `ConnectionCore` 는 U3 에서 칸이 늘지 않는다(U1 의 정책 칸뿐). 차분 자리는 접근자(가안 `client.refusal_alerts()`)로 같은 거르개를 쓴다.
- **박스 글(가안)** — 셸 사용자 대면 문자열 관례(한국어 하드코딩 — `tray/core.rs:35-41`)를 따른다. 제목 「Engram — 명령 등록 거절」. 등록 = 「데몬이 이 셸의 명령 등록을 통째로 거절했습니다. 다음 재연결까지 LLM 이 창 · 탭 · 슬롯 명령을 쓸 수 없습니다.」 · 차분 = 「데몬이 화면 명령 명단 갱신을 거절했습니다. 바뀐 화면 명령은 데몬에 오르지 않았습니다(셸 명령은 그대로입니다).」 둘 다 데몬 문구 원문(코드 접두 포함 — 겹친 이름 · 넘친 크기 · 상한 수치가 거기 실린다)과 「원인이 된 명령 선언을 고친 뒤 다시 빌드하세요.」를 붙인다. 코드별 문구는 두지 않는다 — 원인은 데몬 문구가 이미 말한다.
- **되풀이 억제 — 키 = (자리, 데몬 문구) · 수명 = 셸 프로세스(이 TRD 의 판단):**
  - ① 거절은 같은 빌드 · 같은 이름이면 결정적이다 — 재연결마다 같은 패킷이 같은 문구로 거절되므로 다시 띄워도 새 정보가 없고, 데몬 재기동 · 절전 복귀처럼 재연결이 잦을 때 창이 쌓인다. 「연결마다 한 번」은 바로 이 반복을 못 막는다.
  - ② 데몬 문구가 원인을 싣는다 — 겹친 이름 전부(`connection_core.rs:1162-1187`) · 넘친 크기 · 상한 수치(`roster.rs:283-320`). 그래서 다른 사고는 다른 키라 새로 뜬다. 상한 문구는 명부에 남은 수를 실어 같은 사고라도 수가 바뀌면 다시 뜰 수 있다 — 상태가 바뀐 것이라 받아들인다.
  - ③ 자리를 키에 넣는 이유 — 두 거절의 무게가 다르다(등록 거절 = 셸 이름까지 0 · 차분 거절 = 화면 몫 차분만 빠짐). 그리고 두 자리를 GUI 로 따로 잴 수 있다(§5-2).
  - 대가 — 같은 사고의 두 번째부터는 warn 로그만 남는다(이미 한 번 띄웠고 원인 로그는 매번 찍힌다).

### 2-5. D5 — 시험 재배치

| 지금(셸) | 무엇을 재나 | 옮길 자리 | 단위 |
|---|---|---|---|
| `every_creation_door…` ① | wire 어휘 전량이 정책 표에 선언돼 있나 | 데몬 `connection_core.rs` 시험(두 crate 를 다 보는 자리) — `wire_slot` 식 완전 `match` 로 빈칸 방지 | U1 |
| ② | `agent.new` 가 광고하는 어휘 == 표가 여는 집합 | agent `commands.rs` 시험(둘 다 agent 안) — `advertised_new_backends` 도우미째 | U2 |
| ③ 열린 팔 | 열린 백엔드가 포트까지 간다 | 셸에 이미 있다 — `spawn_into_forwards_a_known_backend`(`layout_apply.rs:1315`) | 그대로 |
| ③ 닫힌 팔 | 닫힌 백엔드는 스폰 전에 거절 · 사유를 싣는다 · 오탈자 문구와 갈린다 | 데몬 — `ConnectionCore` 에 닫는 가짜 정책을 꽂은 dispatch 수준 시험(§3-2 ㉡) | U1 |
| `layout_commands.rs` `the_registration_packet_never_carries_a_name_the_daemon_answers_itself` | 셸이 데몬 이름을 패킷에서 뺀다 | 데몬 쪽 단언은 이미 있다(`connection_core.rs:4159` — 등록 · 차분). 셸 시험은 셸 몫으로 고쳐 쓴다 | U2 |
| `layout_commands.rs:629-634` `no_two_declarations_claim_the_same_name` | 「이 바이너리에 링크된 **전 crate**」의 선언 이름 겹침(doc 이 코어 `agent.*` 와의 겹침을 이유로 든다) | 남긴다 · doc 범위를 고쳐 쓴다(아래) | U2 |

- **셸이 지키는 것(U2):**
  - 새 시험(가안 `spawn_into_passes_a_daemon_refusal_through_without_placing`) — 가짜 `Spawner` 가 거절 문구로 `Err` · 낱말 `Some("codex")` → `calls() == 1` · 문구 그대로 · 탭 수 불변 · resync 0. 기존 `spawn_into_propagates_spawn_failure_untouched`(`:1603`)는 낱말 `None` 이라 「낱말이 데몬까지 갔다가 거절되어 돌아온다」를 재지 않는다.
  - `layout_commands` 고쳐 쓰기(가안 `the_registration_packet_carries_shell_names_once_and_leaves_daemon_names_to_the_daemon`) — 웹뷰가 셸 표 이름 전량(`COMMAND_SPECS` — 셸 자기 표, 그 파일이 이미 import · `:59`) · `tab.next` · 데몬이 답하는 꼴의 이름 문자열 하나(`"agent.spawn"`)를 보고 → `refused` = 셸 표 이름 · 패킷에 `tab.next` 와 `agent.spawn` 이 실린다(벽 = 데몬 — ADR-0270 결정 2) · 셸 표 이름은 표에서 온 한 번뿐. 시험 다리는 실 `reserved_names()` 를 쓴다(`:2628`).
  - **`no_two_declarations_claim_the_same_name` 의 doc(`:629-630`)** — 그 시험은 링커 수집(`command` 의 `duplicate_command_names` — `spec.rs:96`)이라 **이 바이너리에 링크된 crate 의 선언만** 본다. ★이 파일은 데몬을 부른다★ — `the_webview_deadline_fits_inside_the_daemon_seat`(`:3031-3033`)가 `engram_dashboard_daemon::command_delivery::CommandDeliveries::DEFAULT_DEADLINE` 을 읽는다(3판의 「데몬을 부르지 않는다 · rg 0」은 잘린 검색 출력을 읽은 오류였다). 그래서 2-1 뒤에도 데몬은 링크되고, agent 선언이 이 바이너리에 남아 보이는지는 **그 우연한 참조가 agent 의 선언 객체까지 끌어오느냐에 달린다 — 불확실**(셸 lib 은 더는 agent 를 부르지 않는다). doc 을 「셸 표와 이 바이너리에 링크된 crate 사이의 겹침만 잰다 · agent 선언이 보이는 것은 데몬 dev 경로의 우연이다 · 셸 · 데몬 이름 겹침의 벽은 데몬 등록 거절이다(ADR-0270 결정 2) + 거절 박스(U3)」로 고쳐 쓴다. **측정(커밋하지 않는 일회성) — 「전」은 U2 단계 ① 앞(또는 U1 머리)에서, 「후」는 U2 단계 ⑤ 뒤에서** 같은 바이너리로 `spec_of("agent.new")` 가 `Some` 인지 잰다. 결과를 U2 커밋 본문에 적는다.
- ★**셸에 agent dev 의존을 두지 않는다**★ — 셸 시험은 agent 를 이름으로 부르지 않는다. 데몬 dev 의존이 agent 를 컴파일에 끌어오지만(§1-4) 그것을 쓰지 않는다.
- **갈 곳에 먼저 세우고 지운다** — U1 이 데몬 몫을, U2 가 agent 몫을 먼저 더한 뒤 셸 시험을 지운다(회귀망 공백 0).

### 2-6. D6 — 게이트 「셸은 agent 를 의존하지 않는다」

```bash
# 셸 운영 그래프(normal · build)에 agent 가 없다 → 0줄
cargo tree --locked -p engram-dashboard -e normal,build --target all --all-features --prefix none | rg "^engram-dashboard-agent "
# 짝 — 눈먼 게이트 방지 → 1줄 이상
cargo tree --locked -p engram-dashboard-daemon -e normal,build --target all --all-features --prefix none | rg "^engram-dashboard-agent "
```

- **판정** — 앞 줄은 매치 0(rg 종료코드 1) · 뒤 줄은 1줄 이상 · `cargo tree` 종료코드가 0 이 아니면 FAIL(discovery async 반입 게이트 `ci.yml:1163-1180` 과 같은 꼴).
- **`-i` 를 쓰지 않는다** — 대상이 그래프에 없으면 rc=101 로 죽어(§1-4) 「없다」가 FAIL 로 읽힌다.
- **dev 를 일부러 뺀다** — 셸 → (dev) 데몬 → agent 가 2-4 까지 남는다.
- **짝이 막는 것** — agent 개명 · 접두 변경 · 패턴 오타로 앞 줄이 눈먼 채 통과하는 것.
- **`^…␣` 앵커(뒤 공백)** — 빼면 같은 접두의 다른 이름이 걸린다(discovery 게이트 주석과 같은 이유).
- 매니페스트 grep 이 아니라 **해석된 그래프**다 — 다른 의존이 agent 를 끌어오는 전이 경로까지 잡는다. ADR-0270 「영향」 불변식(「`src-tauri` 의 매니페스트에 `engram-dashboard-agent` 가 없다(운영 의존)」)의 기계 벽이다.
- **등록할 곳** — `.github/workflows/ci.yml` `gates` 잡(제안 = `platform gate 3`(`:761`) 뒤) · `.claude/skill-bindings/qa.md` standard 의 새 항목 · CLAUDE.md 「빌드·검증 명령」 새 줄 + 「백엔드 모듈 맵」 src-tauri 항목 한 구절 · `docs/testing-strategy.md` §1 src-tauri 절.

### 2-7. D7 — 지나가며

- `src-tauri/Cargo.toml:64-71` — 주석과 의존 줄 삭제(「뒤 단계에서 닫힌다」는 2-1 이 닫는다).
- `src-tauri/Cargo.toml:79-82` net 주석 「forward 폐포는 {net, agent, protocol} … 셋 다 이미 이 셸의 의존이다」 → 「기본 feature(이 셸이 쓰는 조합)로는 워크스페이스 의존 0 · `server` 를 켜면 {platform, protocol}」(§1-4 실측).
- ADR-0219 의 「문」 · 「재는 자리 셋」 서술은 본문을 고치지 않는다 — ADR-NNNN 이 링크로 진다(`/adr link` · §7).

---

## 3. 작업 단위

**원칙(사용자 2026-10-02 — 「망가지지 않는 단위로」 · 「임시 땜빵 금지」):** 단위마다 워크스페이스가 빌드되고 회귀가 초록인 채 끊는다. 갈 곳을 먼저 세우고 옛 자리를 지운다. U1 뒤 U2 전까지 슬롯 경로에 셸 문과 데몬 문이 함께 서는데, 그것은 땜빵이 아니라 순서다(데몬 문이 영구 벽이고 셸 문은 U2 가 걷는다). 그 사이의 주석 · 문서도 커밋마다 참이게 쓴다(§2-1 「agent 주석」).

### 3-1. 단위 표

| 단위 | 범위 | 건드리는 파일 | 리뷰 · QA |
|---|---|---|---|
| **U1** | D1 + 데몬 시험(D5 ① · ③ 닫힌 팔) | daemon `src/connection_core.rs`(정책 seam 칸과 그 doc · 구조체 doc `:1067-1071` 예외 한 줄 · `new` 리터럴 `:1108-1119` · 처리부 · 도우미 · 시험) · agent `src/commands.rs`(주석만 — `:593` · `:600-614` · `:629` · `:668-673` · `:1073-1077`) | `/implement standard` · `/review code full` · `/qa standard` |
| **U2** | D2 · D3 · D5(② · 셸 고쳐 쓰기 · 중복 이름 시험 doc) · D6 · D7 · agent 의존 삭제 | agent `src/commands.rs`(시험 · `:600-614` 의 셸 문 줄 삭제) · 셸 `src/layout/apply.rs` · `src/view_commands.rs` · `tests/layout_apply.rs` · `tests/layout_commands.rs`(`:629-634` 포함) · `Cargo.toml` · 루트 `Cargo.lock` · 프론트 `src/commands/agentCommands.ts:46-51`(주석 — 「형제 문 둘」 · 「재는 자리 = `layout_apply.rs`」) · `docs/reference/structure/agent-backend.md`(`:98` · `:107` · `:209` · `:217`) · `docs/reference/structure/agent-backend.html`(`:605` · `:616` — `.md` 의 짝) · `ci.yml` · `qa.md` · `CLAUDE.md` · `docs/testing-strategy.md` | `/implement standard` · `/review code full`(+ doc 렌즈 — load-bearing 문서) · `/qa full`(GUI) |
| **U3** | D4 + GUI 실측 | 셸 `src/daemon_client/{protocol_state.rs(`reply_outcome` → 출처 분류 · 그 시험 `:433-450`), connection.rs(`:971` + `// ADR-NNNN` · 슬롯 계약 주석 `:61-64` · `:183` · 결말 태스크 `:1961-1976`), mod.rs(`send_command` `:804-831` · 형제 · `DaemonClient` 칸 하나 + 리터럴 넷 `:246` · `:281` · `:307` · `:337`), refusal.rs(새), events.rs(머리 `:3-8`), tests.rs}` · `src/commands/view_bus.rs` | `/implement standard` · `/review code full` · `/qa full`(GUI) |

**순서 U1 → U2 → U3.** U1 이 U2 보다 앞이어야 한다 — 거꾸로면 셸 검사를 지운 순간 슬롯 스폰에 정책 구멍이 난다(ADR-0270 「거부한 대안」 셋째). U3 의 코드는 U2 와 무관하지만 GUI 실측의 트리거(§5-2)가 D3(셸이 데몬 이름을 거르지 않음)에 기대므로 U2 뒤다. 파일 겹침은 agent `commands.rs`(U1 주석 · U2 시험과 셸 문 줄)뿐이다 — 그래도 **한 코더씩 직렬**을 권한다(U2 의 Cargo.lock · CLAUDE.md 를 U3 이 다시 만질 수 있다).

### 3-2. U1 — 데몬 벽

- **수용 기준:** ① 정책이 닫은 백엔드의 `SpawnByCwd` 는 프로필 생성 · 명부 등록 · 디스크 저장 · 스폰 **어느 것도 없이** `Error`(§2-1 문구)로 답한다 ② 열린 백엔드는 지금과 같다(`ws_e2e.rs` `case40_ws_spawn_by_cwd` 초록) ③ `MISSING_BACKEND` 가 정책보다 먼저다(빈 칸 = 그 문구 그대로) ④ 셸은 아직 안 건드린다 ⑤ agent 주석이 이 커밋 시점의 사실을 말한다(셸 문은 아직 산다 · 데몬 문은 호출자를 안 가린다 — §2-1).
- **시험(제안 · 같은 파일 시험 모듈):**
  - ㉠ `every_wire_backend_declares_an_llm_policy` — protocol wire 어휘 × agent 표(셸 ① 이사 · `wire_slot` 식 완전 `match`).
  - ㉡ **dispatch 수준 거절 시험**(리뷰 F2 — 본보기 = `spawn_by_cwd_without_a_backend_is_refused`, `connection_core.rs:4822-4847`) — `test_core()`(`:2628`)에 닫는 가짜 정책을 꽂고 `SpawnByCwd{backend: Some(Codex)}` 를 dispatch → 답이 **정확히 하나** `Error{request_id: Some(req)}` · `core.manager.agent_snapshots()`(`manager.rs:892`) · `list_agents()` 둘 다 빔 · 문구에 사유가 있고 「를 모른다」가 없고 `MISSING_BACKEND` 와 다르다.
  - ㉢ 운영 기본값 = 실 정책 — `ConnectionCore::new` 로 만든 코어의 정책이 wire 낱말마다 **그리고 표 밖 낱말 하나(`"no-such-backend"`)에** `llm_creation_refusal` 과 같은 답을 낸다. wire 낱말은 오늘 전부 `None` 이라 「늘 연다」 가짜를 못 잡는다 — 표 밖 낱말이 `Some(NO_POLICY_DECLARED …)`(fail-closed — `commands.rs:666` · `:675-680`)로 같아야 비로소 잡힌다. 상수는 비공개라 이름으로 부르지 않고 `llm_creation_refusal("no-such-backend")` 의 값과 견준다.
  - ㉣ (선택) 호출 자리 소스 순서 — 운영 구획(`connection_core.rs:2533` 의 `"mod tests {"` 자르기 선례)의 `SpawnByCwd` 갈래에서 정책 호출이 `CoreProfile::new(` 보다 앞. ㉡ 이 거절 시 프로필 · 명부가 비어 있음을 이미 재므로 순서는 ㉡ 이 잡는다 — ㉣ 은 값싼 보강일 뿐이다.
- **게이트:** `cargo fmt --check` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` · `cargo test -p engram-dashboard-agent -- --test-threads=4`(주석만 바뀌어도 doc 링크 확인) · CI.
- **critical 이 아닌 이유:** 스폰 **앞**에 판정 하나를 더할 뿐이고 kill 인과 · finalize · 락 순서 · replay 어느 불변식도 지나지 않는다. 오늘 정책이 아무것도 안 닫아 런타임 동작이 같다(열린 갈래 = 지금 코드 그대로). 그래서 GUI 도 필요 없다 — 열린 갈래의 실 스폰은 CI `case40` 이 잰다.
- **위험:** 판정을 `CoreProfile::new` 뒤에 두면 거절된 즉석 프로필이 디스크에 남는다(㉡ 이 잡는다) · seam 의 운영 배선이 빠지면 벽이 열린다(㉢) · 문구를 바꾸면 버스 호출자가 받는 글이 달라진다(낱말 대소문자 말고는 지금 셸 문구 그대로) · 사람 경로도 표를 본다(감수한 대가 · 오늘 영향 0 — §2-1).

### 3-3. U2 — 셸 끊기

- **단위 안 순서(어디서 멈춰도 빌드가 선다):** ⓪ 중복 이름 시험의 「전」 측정(§2-5 — U1 머리에서 해도 된다) ① agent 쪽에 §2-5 표의 ② 시험을 도우미 `advertised_new_backends` 와 함께 먼저 세운다(정책 주인 쪽) ② 셸 `apply.rs` — `gate_backend` · import 삭제 · 0) 단계 = `parse_backend` · 머리 주석 ③ 셸 `view_commands.rs` — agent 절반 삭제 · 머리 · doc ④ 셸 시험 — `layout_apply.rs` 의 `every_creation_door…` 와 그것만 쓰는 도우미(`WIRE_BACKENDS` · `wire_slot` · `wire_word` · `advertised_new_backends` — 구획 머리 주석째 `:1349-1396`) · import(`:37`) 삭제 · `:1308-1313` doc 의 그 시험 언급 고침 · 새 거절 통과 시험 · `layout_commands.rs` 고쳐 쓰기와 `:629-630` doc ⑤ `src-tauri/Cargo.toml` 의존 삭제(D7 주석 둘 포함) · `Cargo.lock` 재생성 · 중복 이름 시험의 「후」 측정(§2-5) ⑥ D6 게이트 · agent `commands.rs` 「문」 목록의 셸 문 줄 삭제 · 프론트 · 구조 문서 · 나머지 문서. ①~④ 동안은 운영 의존이 남아 있어 쓰지 않는 의존일 뿐 빌드가 선다. ★④ 가 ⑤ 보다 앞이어야 한다★ — 통합 시험은 그 패키지의 의존만 보므로 시험이 agent 를 부르는 채 의존을 지우면 컴파일이 깨진다.
- **수용 기준:** ① D6 두 줄 = 0줄 · 1줄 이상 ② 셸 소스 · 시험에 `engram_dashboard_agent` 0(`rg engram_dashboard_agent src-tauri` → 0줄) ③ `agent.spawnInto` 가 claude · codex 를 지금처럼 띄워 배치하고, 오탈자는 스폰 전에 거절한다 ④ 등록 패킷에서 걸러 내는 이름이 오늘 0 이다(GUI 로그 — §5-1) ⑤ 버스 오류 코드 그대로(`CONFLICT`) ⑥ 「셸 `gate_backend` 문」을 가리키는 글이 코드 · 문서에 남지 않는다(`rg gate_backend` → 날짜 박힌 스냅숏 `architecture-map-notes.md` 와 옛 기록만).
- **시험:** §2-5 표의 U2 몫.
- **게이트:** `cargo build` · `cargo fmt --check` · `cargo test -p engram-dashboard --test layout_apply` · `--test layout_commands` · `--test lib_unit` · `cargo test -p engram-dashboard-agent -- --test-threads=4` · `npm test` · `npx tsc --noEmit`(프론트 주석만이라도 그 파일을 지난다) · D6 두 줄 · 생성물 sync(바인딩이 바뀌지 않아야 한다) · CI(`--locked` 가 새 `Cargo.lock` 을 본다).
- **QA = full(GUI)** — ADR-0270 「영향」이 「슬롯 스폰 경로 교체 뒤 레이아웃 불변 · 셸 명령 등록(로그의 `refused` 로 「오늘 걸러 내는 이름 0」 실측)」을 완료 조건으로 박았다(§5-1).
- **위험:** 셸 시험을 먼저 지우면 회귀망 공백(① · U1 이 먼저 선다) · `Cargo.lock` 을 빠뜨리면 CI `--locked` 가 빨개진다 · CLAUDE.md · qa · testing-strategy 의 게이트 사본이 서로 어긋난다(한 단위에서 함께 고친다) · `agent-backend.md` 와 `.html` 짝이 갈린다(같은 커밋) · 데몬 문구가 오탈자 문구로 읽히면 호출자가 없는 오탈자를 고치려 든다(U1 ㉡ 이 잰다).

### 3-4. U3 — 거절 박스

- **수용 기준:** ① 데몬이 등록 · 차분에 `Error` 로 답하면 코드와 무관하게(`CONFLICT` · `INVALID_ARGUMENT` · **모르는 코드**) 자리마다 박스 하나(GUI) ② 같은 (자리, 문구)는 프로세스에서 한 번 · warn 은 매번(라벨 그대로) ③ 셸 로컬 실패(끊김 · 미연결 · 송신 실패 등)에는 박스가 없다 — 박스는 warn 팔이 아니라 **출처 타입**(§2-4)을 따른다 ④ 기존 `send_command` 호출자가 받는 글이 바이트 단위로 같다 ⑤ 박스가 연결 태스크 · 명령 경로를 막지 않는다(`show` 는 비동기 · 락 밖) ⑥ 셸에 `cfg` 가 들지 않고 capability 는 그대로.
- **시험(제안):**
  - ㉠ 출처 분류(`protocol_state.rs` · 옛 `reply_outcome_splits_ok_and_err` `:433-450` 를 고쳐 쓴다) — `Ok(Error{"CONFLICT: …"})` · `Ok(Error{"INVALID_ARGUMENT: …"})` · ★`Ok(Error{"BRAND_NEW_CODE: …"})`(모르는 코드)★ · `Ok(Error{"접두 없는 글"})` → 넷 다 `Daemon` · `Err(SENT_OUTCOME_UNKNOWN)`(상수를 이름으로 부른다) → `Local` · `Ok(Ack)` → `Ok` · `Display` 가 옛 글과 같다.
  - ㉡ 억제 — 같은 (자리, 문구) 두 번 → 1회 · 자리만 다름 → 2회 · 문구만 다름 → 2회 · 대역이 거르개를 다시 불러도 교착 없음(락 밖 호출).
  - ㉢ 등록 자리(실 소켓 왕복 — `daemon_client/tests.rs` 의 가짜 서버) — 네 갈래: **`CONFLICT`** 로 답 → 대역이 `(Register, 문구)` 를 정확히 한 번 받고 재연결 뒤 같은 답에는 늘지 않는다 · **`INVALID_ARGUMENT`**(예: 「a command name may be at most 128 bytes …」 꼴) → 한 번 · ★**모르는 코드**(`"BRAND_NEW_CODE: …"`) → 한 번★ · **답 전에 끊음**(`SENT_OUTCOME_UNKNOWN`) → 0 회(warn 은 찍힌다 — O4). 이 갈래가 「`:971` 에서 누가 다시 `Error` 를 `Err` 로 접는 회귀」도 잡는다(§2-4 「대가와 다른 안」).
  - ㉣ `send_command` 호환 — 기존 `daemon_client/tests.rs` 의 `send_command` 시험은 고치지 않고 초록이어야 한다. ★다만 `send_command_resolves_err_on_matching_error`(`:2528-2542`)의 단언은 지금 `contains("거부")` 라 수용 기준 ④(바이트 단위로 같다)를 못 잰다 → 바이트 대조로 조인다★: 가짜 서버가 보내는 글은 `"데몬측 거부(테스트)"`(`ReplyBehavior::ErrorEcho` · `:2469-2472`)이므로 `assert_eq!(result.err().as_deref(), Some("데몬측 거부(테스트)"))`. (`assert_eq!(result, Err(…))` 는 컴파일되지 않는다 — `AgentEvent` 가 `PartialEq` 를 derive 하지 않는다, `protocol/src/messages.rs:429`.) 이 조임은 U3 단계 ① 에서 먼저 한다 — 뒤 단계들이 그 글을 바꾸지 않았음을 매 단계 잰다.
  - 차분 자리 `view_bus.rs` 는 Tauri 어댑터라(그 파일 머리 「로직이 없다」) 판단은 ㉠ · ㉡ 이 재고 배선 한 줄은 GUI 가 잰다.
- **단위 안 순서 — 중간 커밋마다 컴파일만이 아니라 동작이 맞다(리뷰 N2):**
  1. `CommandFailure` + `classify_reply` + 시험 ㉠ · ㉣ 의 조임 — 아무도 안 부른다. `reply_outcome` 과 `:971` 은 그대로.
  2. `send_command_with_origin` 신설 · `send_command` 가 그것에 맡긴다 — 지금 슬롯에는 데몬 `Error` 가 `Err` 로 들어오므로 분류 결과는 `Local` 이고, `send_command` 가 돌려주는 글은 그대로다(㉣ 이 잰다).
  3. 등록 결말 태스크(`connection.rs:1961-1976`)와 `view_bus.rs:94-100` 을 분류 결과로 돌린다 — 이 시점엔 데몬 거절도 아직 `Err` → `Local` 로 와서 박스 갈래가 안 서고, warn 은 지금과 같다(동작 불변).
  4. ★`:971` 을 마지막에 뒤집는다★(`Ok(ev)` 그대로 · 슬롯 계약 주석 `:61-64` · `:183` · 앵커) — 이 순간부터 데몬 거절이 `Daemon` 으로 분류된다. 박스 포트가 아직 없으므로 `Daemon` 도 warn 만 낸다.
  5. 박스 포트 · 거르개 · 운영 어댑터 · `DaemonClient` 칸(리터럴 넷) · `events.rs:3-8` 머리 · 시험 ㉡ · ㉢ — 박스가 선다.
  - **`:971` 을 먼저 뒤집으면 틀린 이유:** 그 커밋에서 `send_command` 는 아직 슬롯 결과를 그대로 돌려주므로(`mod.rs:827-830` — `Ok(result) => result`) 데몬 `Error` 가 `Ok(AgentEvent::Error{..})` 로 호출자에 간다. `commands/agent.rs` 의 `expect_ack`(`:292-305`) · `expect_ack_or_spawned`(`:309-319`)는 `Ok(other)` 를 warn 하고 **성공으로** 넘기므로 데몬이 거절한 명령이 프론트에 성공으로 보이고, 등록 결말 태스크는 `Ok(Ok(_))` 팔(`connection.rs:1963`)로 가서 「셸 명령 등록 완료」를 찍는다. 컴파일은 되지만 동작이 틀린 중간 커밋이다.
- **게이트:** `cargo fmt --check` · `cargo test -p engram-dashboard --test lib_unit` · `--test daemon_client_pending`(슬롯 타입이 그대로임을 확인) · CI · GUI(§5-2). 단계 1~4 의 각 커밋도 `lib_unit` 초록(㉣ 포함)이어야 한다.
- **위험:** `blocking_show` 를 쓰면 연결 태스크 또는 메인 스레드가 멈춘다(`show` 만) · 락을 쥔 채 포트를 부르면 대역 · 어댑터의 재진입에 교착(ADR-0006) · 창이 숨은(트레이) 상태에서 박스가 앞에 뜨는지(미검 — §5-2) · `:971` 이 다시 접히면 박스가 조용히 죽는다(㉢ 이 잡는다).

### 3-5. 단위마다 검증 — 회귀 수 대조

1-3 TRD §5-2 의 명령 그대로다(`cargo test --workspace -- --test-threads=4` 를 앞뒤로 돌려 결과 줄 수 · 통과 · 실패 · 무시를 견준다 · 줄 수가 줄면 타깃 소실 — 초록이어도 멈춘다).

| 단위 | 결과 줄 | 통과 합계 | 분포 이동 |
|---|---|---|---|
| U1 | 그대로(기존 파일에 더한다) | + 데몬 새 시험 셋(㉣ 을 넣으면 넷) | — |
| U2 | 그대로 | − 1(`every_creation_door…`) + 1(셸 거절 통과) + 1(agent ②) · `layout_commands` 는 고쳐 쓰기라 ± 0 | 정책 단언 셸 → 데몬(U1) · agent |
| U3 | 그대로 | + `lib_unit` 새 시험 수(㉠ 은 고쳐 쓰기라 ± 0) | — |

**기준선은 U1 착수 직전에 잰다** — 마지막 기록(CLAUDE.md 「빌드·검증 명령」 = 결과 줄 59 · 3822 통과 · 30 무시, `cac9ee0`)은 1-1 · 1-3 머지로 낡았다. 이 TRD 는 돌려 보지 않았다.

### 3-6. U1 착수 체크리스트

0. **전제 둘.** ① ADR-NNNN(예정)을 박는다(§7 — 채번 · 링크 · 도장 = `/adr`). ② 되돌릴 지점 — 이 TRD 를 로컬 커밋해 출발점을 만든다(코드 트리 = `cf693f4`).
1. **출발 수치** — §3-5 명령을 돌려 결과 줄 · 통과 · 실패 · 무시를 적는다(터미널이 죽으면 `scripts/run-detached.ps1` — qa 바인딩 「분리 실행」).
2. **손댈 파일(정확히):** daemon `src/connection_core.rs` — import 한 줄(`:24` 옆 · `llm_creation_refusal`) · 구조체 칸 하나(`:1072-1092`)와 그 doc · 구조체 doc(`:1067-1071`)의 「공유 핸들만」 경고에 예외 한 줄(상태 없는 순수 함수 포인터) · `new` 의 채움(리터럴 `:1108-1119` — 유일) · `#[cfg(test)]` 빌더 · 도우미(`spawn_command_by_cwd` `:654-661` 옆 · `// ADR-0270` · `// ADR-NNNN`) · 처리부(`:1466-1469` 와 `:1470` 사이) · 시험(§3-2 ㉠~㉢ · 선택 ㉣ — `spawn_by_cwd_without_a_backend_is_refused` `:4822` 옆) · agent `src/commands.rs` 주석(`:593` · `:600-614` · `:629` · `:668-673` · `:1073-1077` — §2-1 「agent 주석」 규칙). 구조 문서 · 프론트 주석은 U2 다.
3. **순서:** 구조체 칸 + `new` 채움(아무도 안 씀 · 빌드 초록) → 도우미 + 시험 ㉠ · ㉢ → 처리부 한 줄 + ㉡(+ ㉣) → agent 주석.
4. **게이트 · 수치:** §3-2 게이트 → §3-5 대조(기대 = 결과 줄 그대로 · 통과 + 셋 또는 넷).
5. `/review code full` → `/qa standard` → 게이트 초록 뒤 커밋(`S21: <타입>(daemon): …` — 스텝 번호는 step-log 를 잇는다 · 끝에 Co-Authored-By 트레일러).

---

## 4. 의존 그래프 — 2-1 뒤

| crate | 직접 워크스페이스 의존(normal · build) 지금 | 2-1 뒤 |
|---|---|---|
| 셸(`engram-dashboard`) | agent · base · command · discovery · net · protocol | base · command · discovery · net · protocol |
| 셸 dev | + daemon(→ agent) | 그대로(2-4) |
| daemon | agent 포함 | 그대로 — `llm_creation_refusal` 은 이미 쓰는 `engram_dashboard_agent::commands`(`connection_core.rs:24`) 옆이다 |

바뀌지 않는 게이트: 셸 `test-support` 누수 게이트 둘(base · platform — `-p engram-dashboard -e normal,build,features -i …` → 0줄)은 base · platform 이 셸 그래프에 남으므로(base 직접 · platform 은 discovery 경유) 그대로 선다(`-i` 대상이 사라지지 않는다).

---

## 5. GUI 실측

절차(기동 인자 · 환경변수 · PID · teardown)는 `/qa` 바인딩 §full 이 갖는다. 앱은 `scripts/` 런처로 띄운다(셸에서 직접 띄우지 않는다). 로그는 `RUST_LOG=info` 로 켠다(아래 판정이 info 줄을 본다).

### 5-1. U2 — 슬롯 스폰 · 등록

1. **착수 전 한 번 + U2 뒤 한 번** — 부팅 로그에 「셸 명령 등록 완료」(`connection.rs:1963`)가 있고 「웹뷰 명령 일부를 등록에서 뺐다」(`view_bus.rs:53`)가 없다 → 「오늘 걸러 내는 이름 0」 실측(ADR-0270 「근거」의 미검 항목).
2. **슬롯 스폰** — `node scripts/cdp.mjs eval "window.__engramCmd.run('agent.spawnInto', { cwd: '<scratch>/ws-2-1', backend: 'claude' })"` · 같은 꼴 `'codex'`(로컬 codex CLI 필요) → 새 탭에 배치 · id 반환. `backend: 'codx'` → 「를 모른다 … 스폰 안 함」 · 탭 수 불변(`list_tabs` 로 앞뒤 비교 — qa 조리법 B 의 직통 invoke).
3. 정책 거절 갈래는 오늘 GUI 로 못 닿는다(표가 아무것도 안 닫는다) — U1 시험 ㉡ 이 진다.

### 5-2. U3 — 거절 박스를 일부러 띄우는 법 (소스를 고치지 않는다)

1. **차분 자리 · `CONFLICT`** — 웹뷰 보고를 CDP 로 바꿔 데몬이 답하는 이름을 싣는다:
   ```bash
   node scripts/cdp.mjs eval "window.__TAURI__.core.invoke('report_view_commands', { commands: [{ name: 'agent.rename', help: { summary: '2-1 GUI 탐침', effect: 'write' } }] })"
   ```
   main 창의 보고가 이 목록으로 갈린다 → U2 뒤 셸은 `agent.rename` 을 거르지 않는다(D3) → `UpdateCommands` → 데몬 `CONFLICT` → **박스 ①(차분)**. 로그 = warn 「웹뷰 명령 차분 등록 실패」 + info(띄움 · Update). `agent.rename` 을 고른 까닭 = 웹뷰 레지스트리에 실재하는 데몬 같은 이름이라(ADR-0270 「맥락」) 「누가 그 이름에 `help` 를 단 날」의 사고를 그대로 흉내 낸다.
2. **등록 자리** — 같은 세션에서 연결을 다시 맺는다:
   ```bash
   node scripts/cdp.mjs eval "(async () => { const i = window.__TAURI__.core.invoke; await i('daemon_close'); await i('daemon_connect'); return 'ok' })()"
   ```
   새 연결의 전량 등록에 다리가 쥔 `agent.rename` 이 실린다 → 패킷 통째 `CONFLICT` → **박스 ②(등록)**. 로그 = warn 「셸 명령 등록 거절」 + info(띄움 · Register).
3. **억제** — 2 를 한 번 더 → warn 은 또 찍히고 박스는 없다(info · 눌러 둠).
4. **`INVALID_ARGUMENT` 갈래 — 값싸게 일으킨다(코드 읽기 · 미검)** — 데몬 이름이 아닌 128 바이트 넘는 이름을 싣는다:
   ```bash
   node scripts/cdp.mjs eval "window.__TAURI__.core.invoke('report_view_commands', { commands: [{ name: 'probe.' + 'x'.repeat(200), help: { summary: '2-1 GUI 탐침', effect: 'write' } }] })"
   ```
   셸 `report` 는 길이를 안 본다(`view_commands.rs:436-481` — 표식 · 예약 · 설명만) → 데몬 이름이 아니라 `refuse_names_i_answer` 를 지난다 → 명부가 길이를 먼저 잰다(`roster.rs:182-184` · 등록도 `:127`) → `INVALID_ARGUMENT: a command name may be at most 128 bytes …` → **박스 ③(차분 · 새 키)**. 이어 2 → **박스 ④(등록)**. 이 갈래가 GUI 에서 안 서면 시험 ㉢ 의 `INVALID_ARGUMENT` 갈래만으로 받는다(시험 전용). 모르는 코드 갈래는 실 데몬이 내지 않으므로 시험 전용이다(㉠ · ㉢).
5. **로컬 실패는 박스가 없다(코드 읽기 · 미검)** — `node scripts/cdp.mjs eval "(async () => { const i = window.__TAURI__.core.invoke; await i('daemon_close'); await i('report_view_commands', { commands: [{ name: 'probe.local', help: { summary: '2-1 GUI 탐침', effect: 'write' } }] }); return 'ok' })()"` → 차분이 미연결로 끝난다(`Local`) → warn 「웹뷰 명령 차분 등록 실패」만 · 박스 없음 · info 도 없음. 그 뒤 `daemon_connect`.
6. **복구** — 창 새로고침(웹뷰가 진짜 목록을 다시 보고) 뒤 2 → 「셸 명령 등록 완료」.
7. **박스 관측** — `cdp.mjs shot` 은 웹뷰만 찍는다(qa 조리법 A). OS 창은 PowerShell 로 화면 전체를 찍거나(`System.Drawing` `CopyFromScreen`) 제목으로 최상위 창을 찾는다(UI Automation) — 둘 다 이 저장소에 선례 없음(미검). 박스 결정 자체는 거르개의 info 로그(§2-4)가 센다.
8. 창을 트레이로 숨긴 상태에서 1 을 한 번 더(다른 이름 — 예 `agent.interrupt`) → 박스가 앞에 뜨는지 본다(미검 항목).

---

## 6. 문서 후속 (이 TRD 는 고치지 않는다 — 각 단위가)

1. **CLAUDE.md** — 「빌드·검증 명령」에 D6 두 줄(U2) · 「백엔드 모듈 맵」 src-tauri 항목에 「agent crate 를 의존하지 않는다(ADR-0270 · 게이트 = 「빌드·검증 명령」)」(U2). load-bearing 이라 `/review doc` 렌즈를 함께 건다.
2. **`.claude/skill-bindings/qa.md`** — standard 의 새 항목(D6 · 판정 규칙 · `-i` 함정 · dev 제외 사유)(U2).
3. **`docs/testing-strategy.md`** §1 src-tauri 절 — 게이트 한 줄(정본 = qa)(U2).
4. **`docs/reference/structure/agent-backend.md`**(`:98` · `:107` · `:209` · `:217`) **와 그 짝 `agent-backend.html`**(`:605` · `:616`) — 정책 문 목록(셸 `gate_backend` → 데몬 `SpawnByCwd` · 호출자를 안 가린다)(U2 · 같은 커밋).
5. **코드 주석** — agent `commands.rs:593` · `:600-614` · `:629`(+ 「`refusal` 을 `Some` 으로 바꾸기 전에 ADR-NNNN 재론」) · `:668-673` · `:1073-1077`(U1 — 셸 문 줄은 U2 가 지운다) · daemon `connection_core.rs:1067-1071` 구조체 doc(U1) · 셸 `daemon_client/connection.rs:61-64` · `:183` 슬롯 계약 · `:971` 앵커(U3) · 셸 `apply.rs:531-556` · `view_commands.rs:8-15` · `:196-203` · `tests/layout_apply.rs:1308-1313` · `tests/layout_commands.rs:629-630` · `src-tauri/Cargo.toml:64-71` · `:79-82` · 프론트 `src/commands/agentCommands.ts:46-51`(U2) · 셸 `daemon_client/events.rs:3-8` 머리(형제 포트 `refusal.rs` 한 줄 — U3).
6. **메모 `docs/refactoring/architecture-discussion-2026-09-26.md`** — §3 결정 후보 5 의 「순서: 슬롯 스폰을 데몬의 `agent.new` 명령 경로로 바꾼 뒤」 · §10 2-1 의 「① 슬롯 스폰을 `agent.new` 경로로 → ② 셸 검사 제거 순서 엄수」는 ADR-NNNN 이 서면 낡는다(오케스트레이터) · §11 둘째 줄(셸 `Cargo.toml` 주석) 처리 표시(U2 뒤).
7. **날짜 박힌 스냅숏** — `docs/reference/architecture-map-notes.md:134` · `:209`(`gate_backend`)는 그 지도를 다시 뽑을 때.
8. **step-log** — 착지 항목(커밋 · 사용자 결정 · 이 TRD 링크)은 오케스트레이터.

---

## 7. ADR-NNNN(예정)에 박을 것

> 채번 · `Amends` 링크 · 개정당하는 ADR 의 도장 = `/adr`. 거부한 대안은 메인이 준 것만 옮긴다(CLAUDE.md 「결정 날조 금지」). ★`docs/decisions/` 는 다른 작업이 동시에 쓰는 중이라 이 TRD 는 손대지 않았다★.

1. **ADR-0270 부분 개정(D1)** — 개정당하는 글:
   - 결정 3.1 「슬롯 스폰을 데몬의 `agent.new` 명령 경로로 바꾼다」 → 「데몬 `SpawnByCwd` 처리부가 스폰 전에 정책 표를 본다 · 슬롯 스폰 경로는 그대로」.
   - 「영향」의 「(ADR-0219 를 고친다 — …) 정책을 보는 문은 데몬 `agent.new` 하나가 된다」 → **데몬의 두 문** — `agent.new`(`verb_new`) · `SpawnByCwd` 처리부.
   - 「영향」의 코드 앵커 「슬롯 스폰의 `agent.new` 경로」와 GUI 완료 조건 「슬롯 스폰 경로 교체 뒤 레이아웃 불변」도 이 뜻으로 읽힌다.
   - 결정 3.2 · 3.3 의 순서(정책 벽 → 셸 검사 삭제 → 예약 목록)는 그대로이고, 3.2 의 「셸의 정책 검사(`gate_backend`)를 지운다」는 정책 몫이며 낱말 그물(`parse_backend`)은 남는다(D2).
   - 거부 = §2-1 (a) · (b) · 이유 = §2-1. 근거에는 §2-1 끝의 「바로잡은 사실」(`SpawnByCwd` 도 즉석 프로필을 등록 · 저장한다 · 철자는 명령 표가 맞춘다)을 한 줄 싣는다 — 같은 사유가 다시 오르지 않게.
   - ★**감수한 대가**★ — 이 문은 호출자를 가리지 않아 사람 경로(`slot.createAgentHere` · 프론트 `agent.spawn` — ADR-0219 결정 5 의 claude 고정 문)도 「LLM 제어 표면」 정책을 본다 · 프론트 `agent.spawn` 과 agent 버스 `agent.spawn` 이 그 점에서 갈린다 · 프론트 `agent.spawnInto` 는 셸 문 때부터 이미 그랬다 · 오늘 영향 0. **재론 계기 = 어느 백엔드든 LLM 에게 닫히거나 선언이 빠지는 날 — 스폰 패킷에 호출자 축을 더한다.** ADR-0219 결정 5 에 링크한다.
2. **ADR-0219 「영향」 링크(D7)** — 정책을 보는 문 ② = 셸 `gate_backend` → 데몬 `SpawnByCwd`(호출자를 안 가린다). 재는 자리 셋 중 `src-tauri/tests/layout_apply.rs::every_creation_door_reads_one_backend_policy` → 데몬 · agent 시험(§2-5). 결정 5 의 claude 고정 문 중 프론트 `agent.spawn` 이 정책을 보게 됐다(위 1 「감수한 대가」). 본문은 고치지 않는다.
3. **ADR-0270 결정 4 의 「구현 때 고른다」를 채운다(D4 — 개정 아님)** — tauri-plugin-dialog Rust API · `DaemonEvents` 를 넓히지 않는 새 포트 · 두 자리 · 트리거 = 데몬이 등록 · 차분에 `Error` 로 답한 것 전부(코드 무관 · 모르는 코드 포함) · 셸 로컬 실패 제외 · ★가름은 답 경계의 출처 타입이다 — 문자열 코드를 읽지 않는다(그 이유 = 코드 추가는 허용된 확장)★ · 되풀이 억제 키 (자리, 문구) · 셸 프로세스 수명과 그 이유(§2-4).
4. **ADR-0270 「영향」 불변식의 기계 벽(D6)** — 게이트 꼴 · `-i` 를 안 쓰는 이유 · dev 를 빼는 이유.

---

## 8. 열린 것 — 처리 기록

1판이 올린 일곱 가지다. O1~O5 는 메인이 2026-10-06 에 처리했고(머리말 「결정 출처」), O6 · O7 은 쓴 그대로 받아들여졌다. 3판의 리뷰 반영은 머리말 「개정」이 적는다.

- **O1 — D1 거부 사유 「슬롯 즉석 에이전트가 영속 프로필이 된다」 → 지웠다.** 코드로 서지 않는다. 바로잡은 사실은 §2-1 끝 한 곳에만 적고, ADR-NNNN 근거도 그것을 싣는다(§7 1).
- **O2 — 「`SpawnByCwd` 는 명부 상한에 닿지 않는다」 → 바로잡았다**(§1-5 — `manager.rs:1148-1151`).
- **O3 — D4 트리거 → 넓혔다.** 데몬이 `Error` 로 답하면 코드와 무관하게 띄우고 셸 로컬 실패만 뺀다. 3판에서 가름을 출처 타입으로 바꿨다(§2-4 · §3-4 · §5-2 4 · 5).
- **O4 — 등록 자리의 「거절」 warn 이 끊김에도 찍힌다(선재) → 그대로 둔다.** 라벨은 안 고치고, 박스는 warn 팔이 아니라 출처 타입을 따른다(§2-4 · §3-4 수용 기준 ③).
- **O5 — 프론트 `agentlist.createCodex` → `CreateProfile` 이 LLM 백엔드 정책을 안 거친다(범위 밖 · 오늘 영향 0) → 메모 §11 로 넘겼다**(`docs/refactoring/architecture-discussion-2026-09-26.md` §11 의 그 줄 — 미처리). agent `commands.rs:600-614` 의 「문 ③」 글은 U1 이 그 주석을 지날 때 사실대로 고친다(§2-1 「agent 주석」).
- **O6 — `DaemonSpawner` 의 `AgentEvent::Error` 팔은 죽은 코드다(확실)** — §1-2. 데몬 문구가 접두 「spawn 실패:」 없이 나가므로 D1 의 문구가 혼자 서야 한다(§2-1 이 지금 셸 문구의 틀을 고른 이유). 팔을 지울지는 별건. U3 뒤에도 `send_command` 가 데몬 `Error` 를 `Err` 로 돌려주므로 그대로 죽은 팔이다.
- **O7 — ADR-0270 결정 3 은 단위 셋(경로 교체 · 셸 검사 · 예약 목록)을 적었는데 이 TRD 의 U2 는 뒤 둘을 한 단위로 묶는다.** 순서(3.2 → 3.3)는 U2 안에서 지키고 각 단계가 빌드를 세운다 — 개정이 아니라 묶음이라고 읽었다.

---

## 9. 미검

- **기준선 회귀 수** — 이 판에서 돌리지 않았다(§3-5 · U1 착수 직전).
- **박스 실물** — Windows 에서 rfd 가 띄우는 창의 모양 · 창이 숨은(트레이) 상태에서 앞에 뜨는지(§5-2 8).
- **OS 창 캡처 수단**(§5-2 7) — 이 저장소에 선례가 없다.
- **`daemon_close` → `daemon_connect` 로 새 연결의 전량 등록이 나가는지**(§5-2 2) — 코드 읽기(`register_own_commands` 는 매 연결 뒤 불린다 — `connection.rs:943`)이고 돌려 보지 않았다.
- **GUI 의 `INVALID_ARGUMENT` · 로컬 실패 갈래**(§5-2 4 · 5) — 코드 읽기다. 앞쪽이 GUI 에서 안 서면 시험 ㉢ 으로만 받는다.
- **2-1 뒤 `layout_commands` 바이너리에 agent 선언이 보이는지**(§2-5) — 불확실 · 미측정. 그 바이너리는 데몬을 부르므로(`:3033`) 데몬은 링크된다 — agent 선언까지 딸려 오느냐가 갈린다. 「전」은 U2 ① 앞(또는 U1 머리) · 「후」는 U2 ⑤ 뒤에 일회성으로 잰다(「후」를 이 판에서 재려면 코드를 고쳐야 해 하지 않았다).
- **정책 거절 갈래의 실물 동작** — 표가 아무것도 안 닫아 런타임으로 못 잰다(seam 시험으로만 — U1 ㉡).
- **「오늘 걸러 내는 이름 0」** — 코드 읽기다(§5-1 1 이 잰다).
- **codex 실 스폰 GUI**(§5-1 2) — 로컬 codex CLI 가 있어야 한다.
