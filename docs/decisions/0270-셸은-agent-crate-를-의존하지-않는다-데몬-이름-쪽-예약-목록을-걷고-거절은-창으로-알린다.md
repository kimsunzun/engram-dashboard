# ADR-0270: 셸은 agent crate 를 의존하지 않는다 데몬 이름 쪽 예약 목록을 걷고 거절은 창으로 알린다

- 상태: 확정 (2026-10-02, 근거: 사용자 결정 2026-10-02 (`docs/refactoring/architecture-discussion-2026-09-26.md` 결정 후보 5 · 작업 단위와 순서) + 코드 대조 2026-10-02 (`d5ac725`))
- 관련: Amends ADR-0176 (결정 2의 COMMAND_SPECS 간선과 데몬 이름 쪽 예약 필터 유지와 거부한 대안 중 필터만 없앤다의 근거와 영향의 reserved_names 삭제 금지 경고와 셸 agent 의존 잔존) · Amends ADR-0219 (결정 2의 셸 gate_backend 문과 영향의 셸 거절 팔 가드) · Amends ADR-0175 (영향의 의존 그래프 중 셸이 agent 를 쥐는 줄과 근거의 COMMAND_SPECS 필터 필요 이유) · ADR-0176 결정 4 · 5(주인 선언 · 프리셋 복제 — 아래) · ADR-0167 결정 5(웹뷰가 버스에 올리는 명령이 `tab.next` · `slot.empty` 둘로 남았다 — 2026-08-23) · ADR-0231 · ADR-0237(웹뷰의 `agent.cancelQueuedInput` · `agent.interrupt` 는 창 안 전용 쌍둥이) · ADR-0029(셸 = 데몬 클라이언트) · ADR-0155(등록 반려 계약) · ADR-0138(네이티브 팝업은 필요해지면 얹는다) · ADR-0266(OS 분기 자리) · ADR-0269(`normalize_cwd` → base) · `src-tauri/src/view_commands.rs`(`reserved_names` · `report`) · `src/commands/viewCommandBridge.ts`(`offeredCommands`) · `src-tauri/src/layout/apply.rs`(`gate_backend` · 슬롯 스폰) · `src-tauri/src/layout/commands.rs` · `src/commands/agentCommands.ts` · `crates/engram-dashboard-daemon/src/connection_core.rs`(`refuse_names_i_answer`) · `crates/engram-dashboard-agent/src/commands.rs`(`llm_creation_refusal` · `verb_new`) · `src-tauri/tests/layout_apply.rs` · `src-tauri/tests/layout_commands.rs` · step-log S21

## 맥락

셸은 agent crate 에서 **세 가지만** 쓴다(호스팅 아님, 코드 대조 2026-10-02):

- `COMMAND_SPECS` — 웹뷰가 못 가져갈 예약 이름(`src-tauri/src/view_commands.rs` 의 `reserved_names`). 예약 목록은 두 절반의 합이다 — agent 의 `COMMAND_SPECS`(데몬이 답하는 이름) + 셸 자기 표 `crate::layout::commands::COMMAND_SPECS`(셸이 답하는 이름). agent 가 필요한 것은 앞 절반뿐이다.
- `llm_creation_refusal` — 슬롯 스폰 전 백엔드 정책 거름(`src-tauri/src/layout/apply.rs` 의 `gate_backend`).
- `normalize_cwd` — cwd 표기 정리(`src-tauri/src/layout/commands.rs`).

이름 충돌은 데몬이 이미 막는다 — 데몬이 스스로 답하는 이름을 클라이언트가 등록하면 `CONFLICT` 로 **등록 패킷 통째로** 거절한다(`connection_core.rs` 의 `refuse_names_i_answer`). 셸 명령과 웹뷰 명령은 한 패킷으로 등록되므로, 거절되면 이 셸의 명령이 다음 재연결까지 **0개**가 된다(LLM 이 창 · 탭 · 슬롯을 못 만짐). 지금 셸은 그 거절을 `warn` 로그 한 줄로만 남긴다(`src-tauri/src/daemon_client/connection.rs` — 「셸 명령 등록 거절」).

**웹뷰 레지스트리의 같은 이름 — 등록과 버스 광고를 갈라 본다**(코드 대조 `d5ac725`):

- **레지스트리에는 데몬과 같은 이름이 넷 있다** — `agent.spawn` · `agent.rename` · `agent.cancelQueuedInput` · `agent.interrupt`(`src/commands/agentCommands.ts` · 데몬 선언 = `crates/engram-dashboard-agent/src/commands.rs`). ADR-0176 이 센 것은 앞 둘이다(그 뒤 둘은 ADR-0231 · ADR-0237 이 창 안 전용 쌍둥이로 더했고, 그 자리 주석이 「`help` 를 달지 말 것 — 데몬이 같은 이름을 버스에서 답한다」를 박는다). 셸 자기 표와 같은 이름도 있다 — `tab.create` · `slot.close`.
- ★**그러나 버스에 광고되는 것은 `help` 를 단 명령뿐이다**★ — 웹뷰는 `help.summary` 가 있는 것만 셸에 보고하고(`src/commands/viewCommandBridge.ts` 의 `offeredCommands` · 2026-08-23 부터), 셸도 `help.effect` 가 없거나 설명이 빈 것을 뺀다(`view_commands.rs` 의 `report`). 오늘 `help` 를 단 웹뷰 명령은 `tab.next` · `slot.empty` 둘뿐이다(`rg "^\s*help\s*[:,]" src -g '!*.test.*'`) — ADR-0167 결정 5 가 2026-08-23 에 남긴 그 둘이고, ADR-0176(2026-08-26) 때도 같았다(그날 master 끝 `5780dcc` 에서 `help` 를 단 웹뷰 명령도 그 둘뿐 — `git grep -n -E "^\s*help\s*[:,]" 5780dcc -- 'src/**/*.ts'`). 둘 다 데몬 어휘에도 셸 표에도 없다.
- ★**그래서 오늘 예약 필터가 걸러 내는 이름은 0 이다**★(코드 읽기 — 앱을 띄워 재지는 않았다). 필터가 막는 것은 같은 이름 명령에 누가 `help` 를 다는 날의 사고다. `view_commands.rs` 머리 주석과 `tests/layout_commands.rs::the_registration_packet_never_carries_a_name_the_daemon_answers_itself` 의 「그 사고가 오늘 바로 난다」는 레지스트리에 이름이 있다는 것만 보고 광고 문(`help`)을 빠뜨린 서술이다 — 그 시험은 웹뷰가 이름을 통째로 보고한 가정 상황을 만든다.

백엔드 정책 판정의 정본은 agent 한 곳이고 데몬의 `agent.new` 가 같은 표를 본다(ADR-0219). 셸 쪽은 먼저 거르는 복제다. 단 정책 함수를 부르는 곳은 `agent.new` 처리 한 곳뿐이고(`agent/src/commands.rs` 의 `verb_new`), 셸의 슬롯 스폰은 그 명령을 거치지 않고 통신 메시지 `SpawnByCwd` 를 바로 보내며 데몬은 그 경로에서 정책을 안 본다.

사용자(2026-10-02): 「클라는 데몬이랑 아예 별도의 개념이라서 agent 를 모르는 게 좋음」 · 「데몬에 거절만 하면 딱히 복잡하게 할 필요 없을 듯 … 실행해서 안 되면 다시 빌드하면 되니깐」 · 「빵 하고 뜨면서 클라에서 뜨면 유저도 알지」 · 「망가지지 않는 단위로 잘 그룹지어서 작업하라」 · 「임시 땜빵 때우지 말고」.

## 결정

1. **셸(클라)은 agent crate 를 의존하지 않는다**(사용자 2026-10-02).
2. **데몬 이름에 대한 벽은 데몬의 등록 거절 하나로 둔다**(사용자 2026-10-02 — 「데몬에 거절만 하면」). 예약 목록 중 agent 의 `COMMAND_SPECS` 를 읽는 절반을 걷는다. CI 대조 스크립트는 두지 않는다. **셸 자기 표에서 뽑는 절반은 남긴다**(메인 판단 2026-10-02) — agent crate 가 필요 없는 셸 자기 선언이고, 그 절반이 막는 셸 · 웹뷰 같은 이름은 데몬 거절이 보지 못한다(아래 「거부한 대안」 둘째 항목 — 코드).
3. **작업 순서(작업 순서 2-1) — 단위마다 앱이 돌아가는 채로 끊고 임시 우회를 두지 않는다**(사용자 2026-10-02 — 「망가지지 않는 단위로 잘 그룹지어서 작업하라」 · 「임시 땜빵 때우지 말고」):
   1. **슬롯 스폰을 데몬의 `agent.new` 명령 경로로 바꾼다.** 슬롯 스폰은 「검사 → 데몬 스폰 → 배치」 순이라 데몬이 거절해도 레이아웃이 안 바뀐다(`layout/apply.rs` — 스폰은 락 밖 · 배치는 그 뒤 단일 임계구역). 거부 문구는 데몬을 한 번 다녀온다.
   2. **셸의 정책 검사(`gate_backend`)를 지운다.** 1 뒤라야 정책 구멍이 없다.
   3. **예약 목록의 데몬 절반(agent `COMMAND_SPECS` 읽기)을 지운다.** 이 단위로 셸 → agent 운영 간선이 닫힌다(`normalize_cwd` 는 결정 5 가 먼저 옮긴다). 웹뷰 레지스트리의 같은 이름 넷은 `help` 가 없어 버스에 오르지 않으므로(위 「맥락」) 이 단위가 앱을 깨지 않는다 — **같은 이름을 먼저 없애는 단위는 두지 않는다**(메인 판단 2026-10-02 · 아래 「거부한 대안」).
4. **거절을 클라이언트가 OS 메시지 박스로 띄운다**(사용자 2026-10-02). 띄우는 쪽 = 셸. 플랫폼마다 다른 창이므로 OS 분기는 셸이 아니라 OS 의존 코드 자리(결정 후보 1)나 Tauri 의 대화상자 기능 뒤에 둔다 — 구현 때 고른다. **LLM 통지는 두지 않는다** — 원인은 로그에 남고 에이전트가 로그를 뒤지면 나온다(사용자). 지금의 `warn` 로그 한 줄은 그대로 남긴다. 결정 3 의 단위 3 뒤 같은 이름 명령에 누가 `help` 를 달면 그 박스가 뜨는 것이 이 벽이 의도한 모양이다(「실행해서 안 되면 다시 빌드하면 되니깐」).
5. **`normalize_cwd` 는 base 로 옮긴다**(결정 후보 4 — `path::normalize_spelling` · 작업 순서 1-1 이 선행).

## 거부한 대안

- **웹뷰의 같은 이름(`agent.spawn` · `agent.rename`)을 먼저 없애는 단위를 맨 앞에 둔다.** 처음엔 이 순서였다 — ADR-0176 결정 2 의 「필터만 없애면 앱이 깨진다(자평: 강함)」를 전제로 했기 때문이다. 기각 = 코드 — 그 이름들은 `help` 가 없어 버스에 오르지 않고 오늘 필터가 걸러 내는 이름은 0 이라(위 「맥락」) 단위 3 이 앱을 깨지 않는다. 그 정리는 ADR-0176 결정 5(프리셋 복제)와 식별자 변경 비용(그 ADR 「거부한 대안」 첫 항목 사유 ③)에 걸려 2-1 의 목적(셸 → agent 간선)과 무관한 범위를 끌고 온다(메인 판단). ★ADR-0176 의 그 「강함」 근거는 그날 코드로도 서지 않았다★(위 「맥락」 — `5780dcc` 대조). ★ADR-0175 「근거」의 「`COMMAND_SPECS` 필터가 지금 필요한 이유」(필터를 먼저 없애면 매 부팅 등록이 반려된다)도 같은 전제를 실었다★ — 광고 문 `help` 는 `85aa6ab`(2026-08-23)부터 있었고 ADR-0175(2026-08-25)는 그 뒤라, 그 근거도 그날 코드로 서지 않았다(`85aa6ab` 은 ADR-0175 커밋의 조상이다).
- **예약 목록을 통째로 걷는다(셸 자기 절반까지).** 기각 = 코드(메인 판단) — 그 절반은 agent 를 의존하지 않아 결정 1 의 목적과 무관하다(코드). ★그리고 그 절반을 대신할 벽이 없다★ — 셸 명령과 웹뷰 명령은 한 주인의 한 패킷으로 등록되고, 데몬의 등록 거절(`refuse_names_i_answer`)은 데몬이 답하는 이름만 본다. 명부는 한 패킷 안의 같은 이름을 오류 없이 하나로 접는다(`crates/engram-dashboard-command/src/roster.rs` 의 `register` — 이름을 키로 한 `BTreeMap` 에 차례로 넣어 뒤가 앞을 덮는다 · 시험 `duplicate_names_in_one_packet_count_once`, `roster.rs:622`). 그래서 웹뷰의 `tab.create` · `slot.close` 에 누가 `help` 를 달면 셸 표의 같은 이름과 말없이 겹치고, 결정 4 의 거절 박스도 뜨지 않는다(코드).
- **셸 정책 검사를 그냥 지운다(슬롯 스폰 경로를 그대로 둔 채).** 기각 = 코드 — 정책을 보는 곳은 `agent.new` 하나뿐이고 슬롯 스폰은 `SpawnByCwd` 로 그 문을 비켜 가므로, 먼저 지우면 슬롯 스폰에 정책 구멍이 난다.
- **명령이 자기 주인을 선언하게 해서 필터 · 간선을 한꺼번에 없앤다(ADR-0176 결정 4 의 「진짜 결함」).** 지금 고르지 않는다 = 별개 주제이고 범위가 크다(메인 판단). **기각 근거 자평: 약함**(서술 — 범위를 재지 않았다). 결정 2 가 데몬 절반만 걷으므로 「뺄셈으로 웹뷰 몫을 구한다」는 셸 자기 절반에 남는다 — 그 결함은 그대로 다음 주제다.
- **CI 대조 스크립트로 충돌을 미리 잡는다.** 기각 = 사용자 결정(「실행해서 안 되면 다시 빌드하면 되니깐」).
- **거절을 LLM 에게도 통지한다.** 기각 = 사용자 결정(원인은 로그에 남는다).
- **거절을 `warn` 로그 한 줄로만 남긴다(현행).** 기각 = 사용자 결정(「빵 하고 뜨면서 클라에서 뜨면 유저도 알지」).

## 근거

- **사용자 결정 2026-10-02** — 위 「맥락」 인용.
- **코드 대조(2026-10-02 · `d5ac725`)** — 셸의 agent 심볼 셋 · 예약 목록의 두 절반(`reserved_names` 본문) · 레지스트리의 같은 이름(데몬과 넷 · 셸 표와 둘) · 버스 광고 문 = `help`(`offeredCommands` · `report`) · `help` 를 단 웹뷰 명령 = `tab.next` · `slot.empty` · 데몬의 패킷 통째 거절 · 거절 처리 = `warn` 한 줄 · 정책 호출처 하나 · 슬롯 스폰이 `SpawnByCwd` 직송.
- ★**검증 상태: 「오늘 필터가 걸러 내는 이름 = 0」은 코드 읽기다**★ — 앱을 띄워 등록 패킷을 재지 않았다. 2-1 착수 때 셸 로그의 「예약 이름이라 뺀 것」(`ReportOutcome.refused`)으로 확인할 수 있다.

## 영향 / 불변식

- **불변식: `src-tauri` 의 매니페스트에 `engram-dashboard-agent` 가 없다**(운영 의존). 「셸 = 데몬 클라이언트」(ADR-0029)가 의존 그래프에서도 보인다. 셸 실행 파일이 agent crate 전체(PTY · Windows 프로세스 관리 · 명령 수집 등)를 안고 가지 않는다. 테스트 전용 의존(셸 → 데몬 dev-dependency)은 별건이다 — 데몬이 agent 를 끌어오므로 `cargo test -p engram-dashboard` 는 여전히 agent 를 컴파일한다(ADR-0175 「남는 무검증」). 셸 시험 둘이 agent 경로를 직접 부른다 — `tests/layout_apply.rs`(`llm_creation_refusal` · `LLM_BACKEND_POLICY`) · `tests/layout_commands.rs`(`COMMAND_SPECS`). 이 둘은 단위 2 · 3 에서 함께 고친다(시험이 agent 를 계속 쓰면 dev-dependency 로 따로 선언해야 한다).
- **ADR-0176 을 고친다** — 결정 2(「`COMMAND_SPECS` 간선과 예약 이름 필터를 그대로 둔다」)의 데몬 쪽이 이 결정으로 닫힌다. 결정 4(주인 선언)와 결정 5(프리셋 복제)는 그대로 다음 주제다 — 웹뷰의 같은 이름 명령은 남는다. 그 「거부한 대안」 중 「필터만 없앤다」의 근거(「앱이 깨진다 · 강함」)는 서지 않는다(위 「맥락」). ★같은 전제를 실은 그 「영향」 두 줄도 이 결정이 닫는다★ — 「`reserved_names`를 지우지 말 것 — 지우면 매 부팅 등록이 반려된다」(데몬 절반은 결정 3 의 단위 3 이 지운다 · 셸 자기 절반은 남는다) · 「셸의 `engram-dashboard-agent` 의존은 남는다 … 0으로 만드는 게이트를 지금 세우지 않는다」(위 불변식이 그 반대다).
- **ADR-0219 를 고친다** — 결정 2(정책 표를 보는 셸 `gate_backend` 문)와 영향(「셸 `gate_backend` 의 거절 팔은 … 다음 백엔드를 위한 가드로 남긴다」)이 셸 쪽 문을 잃고, 정책을 보는 문은 데몬 `agent.new` 하나가 된다(그 결정 2 의 다른 문인 프론트 `humanOnly` 게이트는 2026-09-22 에 이미 걷혔다 — `src/commands/agentCommands.ts` 주석). 회귀망 `src-tauri/tests/layout_apply.rs::every_creation_door_reads_one_backend_policy` 를 같은 변경에서 고친다.
- **ADR-0175 영향의 의존 그래프(`src-tauri → … + 결정 4 가 닫힐 때까지 agent 1심볼`)가 닫힌다** — 그 결정 4 의 목적(간선 소멸)을 이 결정이 접두 없이 이룬다. 그 「근거」의 「`COMMAND_SPECS` 필터가 지금 필요한 이유」(필터를 먼저 없애면 매 부팅 등록이 반려된다)도 서지 않는다(위 「거부한 대안」 첫 항목).
- **작업은 망가지지 않는 단위로 묶는다**(사용자 2026-10-02) — 결정 3 의 세 단위가 그 모양이다. 단위마다 빌드 · 회귀 초록 · 앱이 돈다.
- **거절 메시지 박스는 OS 분기를 셸에 들이지 않는다**(결정 4 · 결정 후보 1 의 불변식). 띄우는 실물은 구현 때 고른다.
- **GUI 실측이 완료 조건이다**(작업 순서 2-1) — 슬롯 스폰 경로 교체 뒤 레이아웃 불변 · 단위 3 뒤 셸 명령 등록(로그의 `ReportOutcome.refused` 로 「오늘 걸러 내는 이름 0」을 실측) · 거절 박스.
- **낡는 서술:** 셸 `Cargo.toml` 주석 「agent 에서 `COMMAND_SPECS` 하나만 남았다」(실제 셋 — 메모 §11) · `view_commands.rs` 머리의 「예약 이름 — 이 필터가 등록 패킷을 지킨다」(이미 광고 문 `help` 를 빠뜨렸고, 데몬 이름 쪽이 빠진다) · `reserved_names` 주석의 「데몬 어휘를 전부 덮지는 못한다(`mail.*`)」(데몬 이름은 더 덮지 않는다 — 벽은 데몬 거절이다) · `tests/layout_commands.rs` 의 「그 사고가 오늘 바로 난다」.
- **코드 앵커 = `// ADR-0270`** — 등록 거절 박스 · 슬롯 스폰의 `agent.new` 경로 · `reserved_names`(셸 자기 절반만 남는 이유).
