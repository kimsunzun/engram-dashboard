# ADR-0191: 백엔드가 자기 통로를 만들어 넘긴다 — 가르는 switch 는 한 곳뿐이다

- 상태: 확정 (2026-09-10, 근거: 사용자 결정 + 2 인 적대 리뷰 지적 + 기존 선례)
- 관련: Amends ADR-0189 (통로의 거처는 정했으나 생성 경로를 안 정했다) · ADR-0004(백엔드 지식 격리) · ADR-0044/0030(통로 모양·caps 주입) · ADR-0012(모듈 격리 하네스) · `docs/process/S21-codex-backend/trd-phase2a.md` §5·§8 · step-log S21

## 맥락

ADR-0189 는 codex app-server 통로를 `crates/engram-dashboard-agent/src/backend/codex/` 안의 `AgentTransport` 구현체로 두기로 했다. **그런데 누가 그것을 만드나는 정하지 않았다.**

- 오늘 통로를 만드는 자리는 `crates/engram-dashboard-agent/src/manager.rs:79-99` 의 `select_transport` 이고, `TransportShape` 두 갈래 match 로 **조립점이 직접** `StdioTransport`/`PtyTransport` 를 만든다.
- ★**그 함수의 주석이 스스로 못 박는다**★ — `manager.rs:75-76`: 「이 함수는 어느 backend 도 이름으로 모른다: 모양은 `backend::transport_shape` 가 신고한 것을 그대로 받는다(ADR-0004)」. 오늘 그 파일에 `codex` 문자열은 **0 회**다.
- 그래서 셋째 갈래를 여기 더하면 **그 자리에서 그 불변식이 깨진다** — 갈래가 `backend/codex/` 의 타입을 이름으로 부르게 된다.

2 인 적대 리뷰가 각각 이 지점을 짚었다(blind = BLOCK 「교체성 추상화가 장식적이다」 · doc-aware = FIX 「격리 누수」).

## 결정

**백엔드를 가르는 단일 switch 가 통로까지 정해서 한 번에 주입한다.**

- **그 switch 는 이미 있고 하나뿐이다** — `backend_for(c)`(`crates/engram-dashboard-agent/src/backend/mod.rs:282`).
- **통로를 만드는 실물 코드는 각 백엔드 폴더 안에 산다.** 조립점은 만들어진 것을 받아 세션에 꽂을 뿐 ★**그 실제 타입을 모른다**★.
- **같은 switch 를 아스펙트마다 다시 타지 않는다.** 오늘 조립점은 모양·인코더·정제기·턴 판정기·우편 자격을 각각 따로 물어 같은 switch 를 여러 번 탄다(`manager.rs:1037-1043`). 이것을 한 번으로 접고, 통로가 그 묶음에 함께 실린다. ★사용자 결정 — 「이왕 하나 있는 거 알뜰히 이용한다」★
- **claude 경로의 값과 순서는 그대로 둔다** — 동작 변화 0 이 이 변경의 수용 조건이다.

## 거부한 대안

- **중앙 `TransportShape` match 에 셋째 갈래를 더한다** — 변경은 가장 작지만 **가르는 자리가 두 곳으로 갈린다**(`backend_for` + `select_transport`), 그리고 조립점이 백엔드 타입을 이름으로 알게 되어 위 주석과 ADR-0004 를 그 자리에서 깬다. 넷째 백엔드마다 중앙 두 곳을 고쳐야 한다. ★게다가 codex 는 통로가 **둘**이다★ — Phase 1 의 PTY 대화 화면이 그대로 살고 app-server 가 더해진다(ADR-0189). 「백엔드당 모양 값 하나」 전제가 거기서 깨지고, 선언 표 트립와이어(`backend/mod.rs:779`)는 `AgentCommand::Codex` **variant 하나에 튜플 하나**를 대는 모양이라 둘째 모드가 그 검사를 아예 안 탄다(doc-aware 리뷰 F4).
- **아스펙트별 배달부 함수를 하나 더 늘린다**(`output_decoder` 패턴 그대로 `open_transport` 추가) — 방향은 맞고 격리도 지켜지지만, **같은 switch 를 또 타는 오늘의 모양을 그대로 둔다.** 사용자 결정으로 기각 — 한 번에 정해 주입하는 쪽으로 접는다.

## 근거

- ★**선례가 이미 이 repo 안에 있다**★ — `backend/mod.rs:477` 의 `pub fn output_decoder(c) { backend_for(c).output_decoder(c) }`. 그 doc 이 이렇게 적는다: 「판정도 decoder 실물도 `AgentBackend::output_decoder` 가 소유하고 이 함수는 dispatch 뿐이다 … **새 backend 는 자기 폴더에서 그 메서드를 구현하면 되고 이 함수는 손대지 않는다**(교체성)」. 같은 형태가 `resume_transcript_events` 에도 있다(`backend/mod.rs:489`).
- CLAUDE.md 「백엔드 확장」 — 「에이전트 백엔드 전용 코드는 없앨 수 없고 **한 곳에 모을 수 있을 뿐이다 — 그 한 곳이 `backend`**」.

## 영향 / 불변식

- **ADR-0189 의 「조립점 주입」 문구는 이 형태로 충족된다** — 주입은 그대로 일어나고, 주입값을 **만드는 주체**가 백엔드로 옮겨질 뿐이다.
- ★**`output.structured` 를 통로 구현체가 하드코딩하는 것은 여전히 금지**★ — `manager.rs:70-73` · `transport/stdio.rs:47-50`(ADR-0044/0030). 이 ADR 은 그 값을 **없애는 것이 아니라 만드는 자리를 옮기는 것**이다. (doc-aware 리뷰 F5 가 지적한 「caps 를 누가 채우나가 네 갈래로 적혀 있다」를 이 결정이 닫는다.)
- **caps 소유권 분할(ADR-0030) 유지** — transport 가 자기 것을 신고하고 backend 가 자기 것을 신고하는 분담은 안 바뀐다.
- **claude 동작 변화 0** — 기존 트립와이어(`backend/mod.rs:779`)와 기존 테스트가 그 그물이다.
- **이 결정을 다시 열 트리거** — 다른 도구가 같은 계열 프로토콜을 쓰게 되어 통로 구현체가 특정 도구에 안 묶이게 되면(ADR-0189 의 재론 트리거와 같은 사건) 그 구현체는 `transport/` 로 올라가고, 그때 이 주입 경로가 그것을 어떻게 받는지를 다시 연다.
