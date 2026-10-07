# ADR-0279: 백엔드 정책 벽은 데몬의 SpawnByCwd 처리부다 — 셸 슬롯 스폰은 경로를 바꾸지 않는다

- 상태: 확정 (2026-10-06, 근거: 사용자 위임 2026-10-06 「알아서 해 … 단계 착수」 아래의 메인 결정 + `/review trd full` 3라운드 PASS + 코드 대조 `cf693f4` + 착지 `8b48386`(2-1 U1 — `/review code full` 3라운드 PASS · `/qa standard` PASS · CI 초록))
- 관련: Amends ADR-0270 (결정 3 의 정책 벽 경로) · Amends ADR-0219 (결정 5 의 claude 고정 문 축) · ADR-0029(셸 = 데몬 클라이언트) · ADR-0172(즉석 생성은 「마지막 실패」를 쓰지 않는다) · ADR-0012(seam · 단독 하네스) · `crates/engram-dashboard-daemon/src/connection_core.rs`(`SpawnByCwd` 처리부) · `crates/engram-dashboard-agent/src/commands.rs`(`llm_creation_refusal` · `LLM_BACKEND_POLICY` · `verb_new`) · `src-tauri/src/layout/apply.rs`(`spawn_into` · `gate_backend`) · `docs/process/S21-crate-boundaries/trd-2-1-shell-agent-cut.md` §2-1 · §7 · step-log S21

## 맥락

ADR-0270 결정 3.1 은 셸에서 agent 를 걷는 첫 단위로 「슬롯 스폰을 데몬의 `agent.new` 명령 경로로 바꾼다」를 적었다 — 백엔드 정책 판정을 데몬 한 곳에 세우고 셸의 정책 검사(`gate_backend`)를 지우기 위해서다. 2-1 TRD 가 코드에 대 보니(`cf693f4`) 그 경로는 그대로 서지 않는다:

- ★**`agent.new` 는 등록만 한다**★(잠든 상태 — `agent/src/commands.rs:194`). 띄우려면 `agent.spawn {target}` 이 한 번 더 필요하다.
- **데몬 `SpawnByCwd` 처리부는 정책을 안 본다** — `spawn_command_by_cwd` → 없으면 `MISSING_BACKEND` → `CoreProfile::new(.., auto_restore=false)` → `manager.spawn_agent(.., Fresh)`(`connection_core.rs:1455-1500`). 정책을 보는 곳은 오늘 둘이다 — agent `verb_new` · 셸 `gate_backend`.
- **`SpawnByCwd` 를 보내는 경로는 넷이다** — 셸 `DaemonSpawner` 하나를 지나는 셸 경로 둘(Tauri 명령 `spawn_into` · 버스 동사 `agent.spawnInto` — 둘 다 같은 `apply::spawn_into` → `gate_backend` 를 지난다) · 프론트 `slot.createAgentHere`(`src/commands/slotContentCommands.ts`) · 프론트 `agent.spawn`(`src/commands/agentCommands.ts`). 뒤 둘은 `'claude'` 고정이고 셸 정책 검사를 지나지 않는다. 프론트 `agent.spawn` 은 ADR-0219 결정 5 가 「일부러 claude 에 못박은」 문 둘 중 하나다(다른 하나 = agent 버스 `agent.spawn` 의 `create_and_start` — 정책을 안 본다).
- **셸 슬롯 스폰은 「검사 → 데몬 스폰 → 배치」 순이다**(`apply::spawn_into`) — 스폰이 `?` 로 돌아오면 배치에 닿지 않아 레이아웃이 안 바뀐다(시험 `spawn_into_propagates_spawn_failure_untouched`). 데몬이 `Error` 로 답하면 `send_command` 가 그것을 `Err(message)` 로 바꿔 돌려주고, 버스 `agent.spawnInto` 는 그 문구를 `CONFLICT` 로 싣는다.
- **정책 표 `LLM_BACKEND_POLICY` 는 claude · codex 를 다 열어 오늘 거절이 0 이다.**

그래서 ADR-0270 의 뜻(정책 벽 = 데몬)을 지키는 자리를 다시 골라야 했다.

## 결정

1. ★**데몬의 `SpawnByCwd` 처리부가 스폰(`manager.spawn_agent`) 전에 백엔드 정책(운영 = agent `llm_creation_refusal`)을 부르고, 거절이면 `Error` 로 답한다.**★ 자리 = `MISSING_BACKEND` 판정 바로 뒤 · `CoreProfile::new` 앞. ★경계는 `spawn_agent` 다★ — 즉석 프로필이 명부 · 디스크에 오르는 곳이 거기다(아래 「영향」 첫 항목). `CoreProfile::new` 는 메모리 값만 만들므로 그 앞에 둔 것은 배치 선택이지 불변식이 아니다. 셸이 이미 그대로 띄우는 오류 경로다(위 「맥락」).
2. **셸 슬롯 스폰의 경로 · 왕복 수 · 배치 순서는 그대로다.** 슬롯 스폰을 `agent.new` 로 옮기지 않는다 — ADR-0270 결정 3.1 의 「`agent.new` 명령 경로로 바꾼다」를 이 결정이 대신한다. 결정 3.2 · 3.3 의 순서(정책 벽 → 셸 검사 삭제 → 예약 목록)는 그대로이고, 3.2 의 「셸의 정책 검사(`gate_backend`)를 지운다」는 **정책 몫**이며 낱말 그물(`parse_backend`)은 셸에 남는다.
3. **정책을 보는 데몬 문은 둘이다** — `agent.new`(`verb_new`) · `SpawnByCwd` 처리부. 정책의 집은 여전히 `LLM_BACKEND_POLICY` 한 곳이다.
4. ★**이 문은 호출자를 가리지 않는다 — 지금 스폰 패킷에 호출자 축을 더하지 않는다**★(감수한 대가 — 아래 「영향」). ★**재론 계기 = 어느 백엔드든 LLM 에게 닫히거나 선언이 빠지는(`NO_POLICY_DECLARED`) 날 — 그때 스폰 패킷에 호출자 축을 더한다.**★

## 거부한 대안

- **(a) 슬롯 스폰을 `agent.new` + `agent.spawn {target}` 두 번으로 바꾼다(ADR-0270 결정 3.1 의 글 그대로).** 기각 = 코드 — `agent.new` 는 등록만 해(잠든 상태 — `agent/src/commands.rs:194`) 왕복이 둘이고, 둘째가 실패하면 잠든 고아가 남고, 셸은 agent 의 인자 타입(`AgentNewArgs`)을 못 쓰므로(2-1 의 목적) 인자 JSON 을 손으로 지어야 하고, 프론트의 `SpawnByCwd` 슬롯 경로(`slot.createAgentHere` · 프론트 `agent.spawn`)는 여전히 정책을 비켜 간다.
  - ★**이 대안을 기각하는 사유로 다음 둘을 다시 꺼내지 말 것 — 코드로 서지 않는다**★(아래 「근거」의 바로잡은 사실):
    - 「`agent.new` 의 `backend` 는 PascalCase 라 셸이 lowercase wire 낱말과 갈린다」 — 철자는 갈리지 않는다. 리뷰 1라운드(F3)에서 이 사유를 걷었다.
    - 「슬롯 즉석 에이전트가 영속 프로필이 된다」 — `SpawnByCwd` 도 즉석 프로필을 명부에 올리고 디스크에 쓴다. 「영속 프로필이 되느냐」는 (a) 와 이 결정을 가르는 축이 아니다.
- **(b) 만들고 띄우기를 한 번에 하는 새 데몬 동사를 둔다.** 기각 = 이 결정보다 얻는 것 없이 표면만 는다(메인 판단). **기각 근거 자평: 약함**(서술 — 정량 · 실측 · 코드 근거가 없다). 사용자 확인 2026-10-06 — 약함 표시대로 확정.

## 근거

- **결정 출처** — 사용자 위임(2026-10-06 「알아서 해 … 단계 착수」) 아래의 메인 결정이다. 설계는 `/review trd full` 3라운드를 거쳐 PASS 했다(`docs/process/S21-crate-boundaries/trd-2-1-shell-agent-cut.md`). 「감수한 대가」는 리뷰 F1 이 올린 것을 메인이 받아들인 것이다(2026-10-06).
- **왜 이 자리인가** — ADR-0270 의 뜻(정책 벽은 데몬)을 지키면서 슬롯 동작 · 왕복 수 · 배치 순서가 그대로다. 프론트가 직접 보내는 `SpawnByCwd` 두 경로(`slot.createAgentHere` · 프론트 `agent.spawn`)도 함께 덮는다. 셸이 처음으로 버스 호출(`AgentCommand::Command`)을 내는 생산자가 되지 않는다(셸 Rust 에 그 생산자 0 — 2-1 조사 · 재확인 `rg "AgentCommand::Command\b" src-tauri/src` → doc 주석 한 줄뿐(2026-10-06). `protocol/src/messages.rs:645` 주석이 적는 것은 웹뷰(`src/`) 쪽 0 건이다).
- **코드 대조(`cf693f4` — 이 ADR 의 줄 번호 = `cf693f4`)** — `agent.new` 는 등록만 한다 · `SpawnByCwd` 처리부는 정책을 안 본다 · `SpawnByCwd` 송신 경로 넷(셸 둘 · 프론트 둘) · 정책 호출처 둘(`verb_new` · `gate_backend`) · 셸 슬롯 스폰은 스폰 실패 시 배치에 닿지 않는다 · 정책 표가 오늘 아무것도 안 닫는다(위 「맥락」).
- ★**바로잡은 사실 — 철자**★: 명령 표가 부르기 전에 열거 칸의 대소문자를 선언 철자로 옮긴다(`command/src/table.rs:130-131` → `coerce::enum_words_to_declared_spelling` · `coerce.rs:101-109`). 셸이 인자 JSON 을 손으로 지어야 하는 이유는 하나 — agent 의 인자 타입(`AgentNewArgs`)을 못 쓴다는 것이다.
- ★**바로잡은 사실 — 즉석 프로필도 등록 · 저장된다**★: 지금 `SpawnByCwd` 도 `spawn_agent` → `register_for_spawn` → `upsert_preserving_hierarchy`(`agent/src/manager.rs:1161`) → `store.save`(`agent/src/profile.rs:387-395`)로 즉석 프로필을 명부에 올리고 디스크에 쓴다(protocol doc `messages.rs:113-115` 「spawn 경로가 그 프로필을 registry 에 등록·persist 한다」). 두 경로 모두 `auto_restore = false` 다(`connection_core.rs:1470-1476` · agent `commands.rs:1114`).
- ★**검증 상태 — 정책 거절 갈래는 런타임으로 못 잰다**★: 오늘 표가 아무것도 안 닫아 실물 입력으로는 그 갈래에 닿을 수 없다. 그래서 정책 판정을 seam 뒤에 두고 시험이 닫는 가짜를 꽂는다(ADR-0012 · TRD §2-1 설계). 이 결정 시점에 코드는 바뀌지 않았다 — 착지 = 2-1 U1 `8b48386` — 거절 갈래는 `spawn_by_cwd_refuses_a_backend_the_policy_closes`(seam 에 닫는 가짜) · 운영 배선은 `the_production_core_asks_the_agent_backend_policy` · 표 선언 범위는 `every_wire_backend_declares_an_llm_policy` 가 잰다.

## 영향 / 불변식

- ★**불변식: 정책 호출은 `manager.spawn_agent` 보다 먼저다**★ — 즉석 프로필은 그 안(`register_for_spawn` → `upsert_preserving_hierarchy` → `store.save`)에서 명부에 오르고 디스크에 쓰이므로, 그 뒤에 두면 거절된 즉석 프로필이 명부 · 디스크에 남는다(위 「근거」의 등록 · 저장 사슬). `CoreProfile::new` 는 메모리 값만 만든다 — 그 앞에 둔 것은 배치 선택이다(결정 1).
- **거절은 「마지막 실패」를 쓰지 않는다** — 활성화도 즉석 생성도 일어나지 않았다(ADR-0172 결정 1 · 4).
- **거절 문구가 데몬을 한 번 다녀온다** — 데몬 문구는 접두 없이 호출자에 가므로 혼자 서야 한다. 셸 문구의 틀(「`backend '{word}' 는 아는 낱말이지만 이 표면으로는 지금 만들지 않는다 — {reason} 스폰 안 함.`」)을 그대로 쓰고, `{word}` 는 wire enum 직렬화로 얻는다(늘 lowercase). 버스 오류 코드는 지금처럼 `CONFLICT` 다.
- **와이어 · 카탈로그 세대는 그대로다** — `PROTOCOL_VERSION` · 셸 `CATALOG_VERSION`(패킷 모양도 오늘 동작도 안 바뀐다).
- ★**감수한 대가 — 이 문은 호출자를 가리지 않는다**★: 정책 이름은 「LLM 제어 표면」이지만 데몬 `SpawnByCwd` 는 누가 보냈는지 모른다. 그래서 **사람 경로도 같은 표를 본다** — 새로 닿는 것은 claude 고정 두 문(`slot.createAgentHere` · 프론트 `agent.spawn` — 뒤의 것은 ADR-0219 결정 5 의 「일부러 claude 에 못박은」 문)이다. 프론트 `agent.spawnInto` 는 **새 노출이 아니다**(셸 문이 이미 호출자를 가리지 않고 표를 본다). 그 결과 같은 이름의 두 문이 갈린다 — 프론트 `agent.spawn` 은 표를 보고 agent 버스 `agent.spawn`(`create_and_start`)은 안 본다. **오늘 영향 0** — 표가 아무것도 안 닫고 claude 가 열려 있다. 재론 계기는 위 결정 4.
- **재론 계기를 편집 자리에 둔다** — agent 정책 표 doc 에 「어느 줄의 `refusal` 을 `Some` 으로 바꾸거나 선언 없는 낱말을 들이기 전에 ADR-0279 를 다시 연다 — 데몬 `SpawnByCwd` 문이 사람 경로도 막는다」를 달았다(U1 `8b48386` — agent `commands.rs` 의 `LLM_BACKEND_POLICY` doc).
- **ADR-0270 을 고친다** —
  - 결정 3.1 「슬롯 스폰을 데몬의 `agent.new` 명령 경로로 바꾼다」 → 「데몬 `SpawnByCwd` 처리부가 스폰 전에 정책 표를 본다 · 슬롯 스폰 경로는 그대로」.
  - 「영향」의 「(ADR-0219 를 고친다 — …) 정책을 보는 문은 데몬 `agent.new` 하나가 된다」 → **데몬의 두 문** — `agent.new`(`verb_new`) · `SpawnByCwd` 처리부.
  - 「영향」의 코드 앵커 「슬롯 스폰의 `agent.new` 경로」와 GUI 완료 조건 「슬롯 스폰 경로 교체 뒤 레이아웃 불변」도 이 뜻(`SpawnByCwd` 처리부의 정책 벽 · 경로 불변)으로 읽는다.
  - 결정 3.2 · 3.3 의 순서는 그대로다(위 결정 2).
  - 「거부한 대안」 「셸 정책 검사를 그냥 지운다(슬롯 스폰 경로를 그대로 둔 채)」는 그대로 기각이다 — 그 대안의 뜻은 **데몬 벽 없이** 셸 검사만 지우는 것이다. 이 결정의 끝 모양(경로 그대로 · U2 에서 셸 검사 삭제)이 그 괄호와 겉으로 같지만, 가르는 축은 경로가 아니라 「`SpawnByCwd` 처리부의 벽이 셸 검사 삭제보다 먼저 서느냐」다(결정 2 의 순서). 그 기각 사유의 「정책을 보는 곳은 `agent.new` 하나뿐 · 슬롯 스폰이 `SpawnByCwd` 로 비켜 간다」는 U1 착지 뒤 낡았다(데몬 문 둘 — 결정 3).
- **ADR-0219 를 고친다(본문은 고치지 않는다)** —
  - 결정 5 의 claude 고정 문 중 프론트 `agent.spawn` 이 정책 표를 보게 됐다(위 「감수한 대가」). agent 버스 `agent.spawn`(`create_and_start`)은 그대로 안 본다.
  - 정책을 보는 문 ② = 셸 `gate_backend` → 데몬 `SpawnByCwd`(호출자를 안 가린다).
  - 「영향」의 재는 자리 셋 중 `src-tauri/tests/layout_apply.rs::every_creation_door_reads_one_backend_policy` → 데몬 · agent 시험으로 옮긴다(TRD §2-5).
- **낡았던 서술(U1 `8b48386` 이 고쳤다)** — agent `commands.rs` 정책 구획 머리 「LLM 제어 표면의 백엔드 생성 정책」 · 「LLM 제어 표면이 셋」(③ 프론트 `humanOnly` 는 2026-09-22 에 걷혔다) · 표 doc 「LLM 제어 표면이 어느 백엔드를 만들 수 있나」 · 판정 함수 doc 「사람이 쓰는 문에서는 그대로 만들어진다 · 막는 것은 사람이 아닌 호출자뿐」 — 데몬 `SpawnByCwd` 문이 호출자를 가리지 않고 이 판정을 적용한다는 것이 빠져 있었다. 「문」 목록에 데몬 `SpawnByCwd` 문을 더했고, 셸 `gate_backend` 문은 「2-1 U2 가 걷는다 — 그때까지 데몬 문보다 먼저 같은 표를 본다」로 남겼다(U2 가 그 줄을 지운다).
- **범위 밖(지금 하지 않는 것)** — 스폰 패킷의 호출자 축(재론 계기 때) · 프론트 `CreateProfile` 경로의 정책(오늘 영향 0 — `docs/refactoring/architecture-discussion-2026-09-26.md` §11).
- **코드 앵커 = `// ADR-0279`** — 데몬 `SpawnByCwd` 처리부의 정책 호출(호출 자리는 `// ADR-0270 · ADR-0279` 한 줄) · 거절 문구 도우미 `by_cwd_refusal` · 정책 seam 필드 `llm_policy`.
- **`MISSING_BACKEND` 가 정책보다 먼저인 이유** — 백엔드 칸이 빈 요청(`backend: None`)에서 「어느 칸을 채우라」는 `MISSING_BACKEND` 문구를 정책 거절이 가리지 않게 하려는 것이다. 시험 `spawn_by_cwd_without_a_backend_is_refused` 가 그때 정책 호출이 0 회임을 단언한다.
