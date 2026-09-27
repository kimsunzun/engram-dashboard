# ADR-0236: control 이 다른 데몬 모듈을 쓰는 것을 받아들인다 — 잎 성질을 내려놓고 제어 평면 분리는 나중 리팩터링에서 다룬다

- 상태: 확정 (2026-09-27, 근거: 사용자 결정 2026-09-27 + master `control/` 간선 실측 2026-09-27)
- 관련: Amends ADR-0130 (재개 조건 2와 재론 트리거) · Amends ADR-0151 (control 잎 긍정 증거) · ADR-0129(보류 중인 응용 층 분리 목표 모양 — 이 결정도 꺼내지 않는다) · ADR-0154(명령 배달) · ADR-0155(명령 버스 도구) · ADR-0231(입력 임대 거절 문구) · step-log S21

## 맥락
ADR-0130 은 데몬 crate 를 더 쪼개지 않기로 하면서 재개 조건 ② 로 「`control/` 에서 다른 최상위 데몬 모듈로 나가는 production 간선이 생긴다」를 두었고, `/qa` 바인딩의 「ADR-0130 재론 트리거」가 그 알림이었다(게이트가 아니라 「멈추고 사용자에게 물어라」). ADR-0151 영향절은 여기에 뜻을 하나 더 얹었다 — `control/` 이 나가는 간선 0 인 잎이라는 것이 제어 평면만 떼어낼 수 있다는 **긍정 증거**이고, 트리거 ② 가 깨지면 그 증거가 사라진다.

2026-09-27 이 브랜치(`v0.3.2/feat/json-midturn-queue`)의 `/qa standard` 가 트리거에서 `control/agent.rs` · `control/commands.rs` → `connection_core::INPUT_LOCKED_REFUSAL`(입력 임대 거절 문구를 WS 입구와 같은 글자로 쓰려는 공유 — ADR-0231)을 보고했다. 그 간선만 걷으면 잎이 돌아온다고 보고 상수를 `control/` 로 옮겼으나, 옮긴 뒤에도 트리거가 production 매치를 냈다. 확인해 보니 `control/` 은 **S20(2026-08-19~20)부터 이미 잎이 아니었다.** 트리거가 울린 것도 처음이 아니다 — 2026-08-23 에 todo 항목이 「재론할 시점인지 판단이 필요하다」로 적었고(출처는 아래 「근거」의 판단 보류 기록), 그 뒤 QA 곁 관측(step-log S21)과 qa 스킬 피드백에도 「상시 매치」로 남았다. 판단만 내려지지 않은 채 ADR-0130 · ADR-0151 · `/qa` 바인딩 본문은 잎을 전제로 남아 있었다. 이 ADR 이 그 보류된 판단을 닫는다.

## 결정
1. **`control/` 이 다른 최상위 데몬 모듈을 production 에서 쓰는 것을 받아들인다** — 잎 성질을 되살리지 않는다. 지금 그 대상은 `command_delivery` 와 `connection_core` 둘이고, 이 브랜치가 더한 거절 문구 간선도 그대로 둔다(상수를 옮긴 수정은 커밋 전에 되돌렸다).
2. **ADR-0130 재개 조건 ② 는 관측됐지만 재론하지 않는다** — ADR-0129 의 목표 모양을 지금 꺼내지 않는다. 제어 평면을 떼는 일은 데몬을 나누는 리팩터링을 할 때 그 자리에서 고려한다 [사용자] — 「그냥 놔두고 control 자체를 때는건 나중에 리팩토링할때 고려해야되는데 현재는 단계가 아님 내려놓아」. 재개 조건 ③(production 순환)도 이미 성립해 있다 — `connection_core` → `control/`(2026-07-21 `7c47947` 부터)와 `control/` → `connection_core`(`sanitize_for_log` — `9df137f` 부터)가 서로를 부른다. 같은 간선 묶음이라 이 결정이 함께 덮는다 [메인 — 사용자가 받아들인 간선과 같은 묶음 · 사용자에게 보고].
3. **재론 트리거 ② 를 걷는다** — 결정 1 의 귀결이다. 잎이 아닌 채로 트리거를 두면 매 QA 가 같은 매치로 울려 알림이 뜻을 잃는다. `/qa` 바인딩의 그 블록과, 그것을 로컬 몫으로 세던 자리(바인딩 「CI 미커버」 · CLAUDE.md 「CI」 절 · `docs/testing-strategy.md`)를 같은 변경에서 걷는다.

## 거부한 대안
- **거절 문구 상수만 `control/` 로 옮긴다**(2026-09-27 코더 착지 — `cargo check -p engram-dashboard-daemon --all-targets` · `cargo fmt --check` 녹 · 커밋 전 되돌림) [사용자 — 「내려놓아」] — 옮긴 뒤에도 트리거가 production 매치를 냈다(코드 실측): S20 간선(`command_delivery` 3종 · `sanitize_for_log`)이 남아 잎이 돌아오지 않는다. 얻는 것 없이 「여기 둬야 잎이 유지된다」는 틀린 주석만 남는다.
- **S20 간선까지 걷어 잎을 되살린다**(명령 배달 연결과 로그 정리 함수까지 옮기거나 뒤집는 리팩터) [사용자 — **거부가 아니라 미룸**] — 위 결정 2 인용. `command_delivery` 간선은 기능 경로다 — 에이전트가 대시보드 소유 명령(`tab.create` 등)을 제어 입구로 부르는 길이 그것을 탄다(`9df137f`). **재론 조건** = 제어 평면 분리를 다루는 리팩터링 착수(ADR-0151 영향절의 되열기 사항 ①–③ 과 함께 판단한다).

## 근거
- **사용자 결정 2026-09-27** — 결정 2 인용. 그 앞에 사용자가 선택지 셋(기록만 · 상수만 옮김 · 전부 걷음)을 받았고, 마지막에 둘(내려놓기 / 되살리기)로 좁혀 「내려놓기」(= 기록만)를 골랐다.
- **master `645d55d` 의 `control/` production 간선(실측 2026-09-27 — `git grep` 뒤 각 파일 `#[cfg(test)]` 시작 줄로 테스트분 제외)**:
  - `command_delivery` — `LocalCommands`(`control/catalog.rs:36` · `control/commands.rs:27`) · `CommandBus`(`control/mcp_server.rs:1050` · `:1254`) · `ENTRANCE_CONTROL`(`control/mcp_server.rs:1202`). 처음 든 커밋 = `2f8d8f2`(2026-08-19) · `9df137f`(2026-08-20, S20 「에이전트가 대시보드 소유 명령을 부를 수 있다」).
  - `connection_core` — `sanitize_for_log`(`control/mcp_server.rs:41` · `control/catalog.rs:357` · `control/registry.rs:246`). 처음 든 커밋 = `9df137f`, 그 뒤 `75bdfda`(2026-09-19, codex 제어 평면 입구).
  - 이 목록은 그날의 스냅숏이다 — 되열 때는 아래 「영향」의 명령으로 다시 잰다.
- **master `645d55d` 의 `control/` ⇄ `connection_core` production 순환(실측 2026-09-27 — `connection_core.rs` 의 `#[cfg(test)]` 모듈은 `:2032` 부터)**:
  - `connection_core` → `control/` — `ControlRegistry`(`connection_core.rs:66`) · `mcp_server::MessagingSlot`(`:856` · `:874`) · `catalog::merge`(`:1621`) · `agent::RosterBroadcast` 구현(`:1960`). 처음 든 커밋 = `7c47947`(2026-07-21).
  - `control/` → `connection_core` — 위 `sanitize_for_log` 간선. 처음 든 커밋 = `9df137f`(2026-08-20) — 그날부터 순환이다.
- **판단 보류 기록** — todo 항목 「ADR-0130 재론 트리거가 발화했다(2026-08-23 실측)」가 — 2026-08-23 `docs/backlog.md`(`d0a2074`)에 처음 적혔고 `docs/todo/observed-only.md`(`ed0b033`)로 옮겨졌다가 이 ADR 과 같은 변경에서 지웠다(읽는 법 = `git show 5e7d522:docs/todo/observed-only.md`) — 같은 간선(`catalog.rs:36` · `commands.rs:27` · `mcp_server.rs:41`)을 적고 판단을 미뤘다. ADR-0130 · ADR-0151 · `/qa` 바인딩 본문에는 반영되지 않았다. ADR-0151 영향절의 「`control/` 은 나가는 간선 0 인 leaf」는 그 ADR 이 쓰인 2026-08-17 에는 참이었고 이틀 뒤 S20 이 깼다.

## 영향 / 불변식
- **ADR-0130 재개 조건 ① 은 그대로다**(ADR-0151 이 판정 기준을 이미 갈았다). **③ 은 이 묶음에 대해 관측됐고 결정 2 가 덮는다** — 조건 자체는 살아 있어 다른 순환이 생기면 여전히 재론 대상이고, 여전히 단발 명령이 없다.
- **ADR-0151 영향절의 「`control/` 이 잎이라는 긍정 증거」는 2026-08-19 부터 사실이 아니다.** 보류를 붙드는 것은 여전히 사용자 판단이고(ADR-0151 결정 5 + 이 ADR 결정 2), 되열기 사항 ①–③ 에 「`control/` 이 쓰는 데몬 모듈 간선을 끊는 공수」가 더해진다.
- **되열 때 간선을 재는 법** = ADR-0130 §근거 ③ 의 재확인 명령. 그 절의 「살아 도는 사본(`/qa` 바인딩)과 명령줄이 같아야 한다」는 사본이 걷혀 대상이 없어진다. ★그 명령은 `-U` 와 `[^;]*` 때문에 여러 줄에 걸친 doc 주석까지 문다★(2026-09-27 이 브랜치 트리 실측 — `control/catalog.rs:150-160` · `control/mcp_server.rs:1051-1065` 가 주석인데 매치됐다) — 다시 쓸 때는 `#[cfg(test)]` 분할과 함께 주석 줄을 걸러 읽는다. 또 셸에서 그대로 옮겨 치면 치환이 깨져 **0 줄로 나와 통과처럼 읽힌** 적이 있다(위 「근거」의 지운 todo 항목) — 매치 유무보다 명령이 실제로 돌았는지를 먼저 본다.
- 코드 동작은 바꾸지 않았다 — 옛 사유를 적던 주석 두 곳(`control/agent.rs` 의 `RosterBroadcast` 포트 doc · `connection_core.rs` 의 `RosterFanout` doc)이 이 ADR 을 가리키도록 고쳤다. `// ADR-` 앵커 줄은 달지 않는다.
