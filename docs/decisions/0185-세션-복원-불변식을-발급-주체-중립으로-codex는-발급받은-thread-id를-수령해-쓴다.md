# ADR-0185: 세션 복원 불변식을 발급 주체 중립으로 — codex는 발급받은 thread id를 수령해 쓴다

- 상태: 확정 (2026-09-09, 근거: 사용자 결정 — 「고치고 진행」)
- 관련: Amends ADR-0008 (sid 발급 주체) · ADR-0082(resume 조기종료 종점) · CLAUDE.md 「세션 복원」 · `docs/reference/architecture-overview.md` 「세션 복원 / 활성화」 · `docs/process/S21-codex-backend/trd.md` §0(Phase 2 범위) · `crates/engram-dashboard-agent/src/profile.rs:165,629` · Amends ADR-0076 (우리 쪽 sid 발급 전제) · Amends ADR-0082 (살아남는 상위 결정 중 sid 발급 조항)

## 맥락
Phase 2가 codex를 상주 JSON 서버(`codex app-server`)로 띄우면 세션 복원이 열린다. 그런데 **codex는 스레드 id를 클라이언트가 지정하게 해주지 않는다** — `ThreadStartParams` 27개 속성에 id·threadId·sessionId·name·title이 하나도 없고, `Thread.id`는 codex가 UUIDv7로 발급한다고 문서가 적는다. 그 파라미터를 넣자는 요청(`session_id` + `--session-id` 플래그, 구현까지 제시)이 상류에 올라왔으나 OpenAI 메인테이너가 닫았다(openai/codex#15767).

문제는 우리 쪽 문장이다. CLAUDE.md 「세션 복원」과 ADR-0008의 근거가 「복원 정확성은 **우리가 통제하는** sid에만 의존한다」로 적혀 있어, 글자 그대로면 codex는 이 원칙 아래 들어올 수 없고 Phase 2 전체가 원칙 위반으로 읽힌다.

## 결정
1. **불변식을 발급 주체 중립으로 다시 적는다** — 「복원은 **프로필에 저장된 backend sid 단독**에 의존한다. 그 sid를 누가 발급하는지는 백엔드가 정한다.」 claude는 우리가 발급해 `--session-id`로 넘기고(ADR-0008 그대로), codex는 스레드 생성 응답에서 받아 저장한다.
2. **codex sid의 수령 지점은 기존 `observe_session_id`다**(`profile.rs:629`) — **새 필드·새 매핑·새 파일을 만들지 않는다.** 그 함수가 이미 「백엔드가 정한 sid를 관측해 옛 값을 이력으로 밀고 즉시 persist」를 한다. ★**단 그 지점까지 가는 배선은 없다 — 이 결정은 Phase 2에 셋을 요구한다**★:
   - ① **수령 배선** — 스레드 생성 응답 → `observe_session_id`. 지금 그 함수의 유일한 생산 호출자는 claude `/clear` watcher 하나이고(`daemon/src/lib.rs:288`), ★**그 watcher는 codex의 수령 경로가 될 수 없다**★ — `backend::session_id_source`가 **우리가 발급한 기준 sid**(`expected_sid: Uuid`)를 요구하는 파일 폴러이기 때문이다(`backend/mod.rs:348-353`). codex의 id는 JSON-RPC 응답으로 온다. 즉 ★**이 항목 하나가 수령을 켠다 — ②도 추적기도 필요하지 않다**★.
   - ② **`needs_session()` 쪼개기** — 이 한 플래그가 *우리 쪽 발급 · watcher 부착 · resume 가능* 셋을 겸한다. ★**이것을 「수령을 막는 원인」으로 적지 말 것**★ — codex에서 watcher가 안 붙는 1차 원인은 그 앞줄(`manager.rs:1075`)의 `sid`가 `None`이라서고(`:955-962`), `if needs`(`:1076`)는 평가되기도 전이다. 쪼개야 하는 이유는 따로다: Phase 2가 `restore_one`이 codex를 resume 가능으로 보게 하려면 이 플래그를 켜야 하는데, 켜면 **발급도 함께 켜져** codex가 쓰지 않는 uuid가 심긴다.
   - ③ ★**활성화 입구의 가드**★ — `commands.rs:521`·`connection_core.rs:1126`이 저장된 sid **하나만** 보고 `SpawnMode::Resume`을 유도하며 `needs_session()`을 보지 않는다(부팅 복원 `manager.rs:1376`은 본다). ③이 빠진 채 sid를 심으면 `spawn_agent`이 sid를 안 넘기고(`manager.rs:954-962`) `build_spec`이 `_mode`를 무시해(`backend/codex/mod.rs:93`) **새 대화가 열린다.** ★**그리고 그것이 새 대화라는 표식이 wire에 하나도 없다**★ — `AgentSpawnOk`(`commands.rs:807-814`)도 `AgentEvent::Spawned`(`protocol/src/messages.rs:468-471`)도 resume 여부를 안 싣는다. ★**`resumed` 를 grep 해 찾지 말 것**★ — 그 이름은 `commands.rs:1090` 의 `#[cfg(test)]` 테스트 더블 지역변수 하나뿐이다. 그리고 `commands.rs:519-520` 의 주석은 **두 입구가 서로 다른 규칙을 쓰는** 위험을 적은 것이라 이 갭을 덮지 않는다 — 두 입구는 같은 규칙을 쓰고 있고, 그 규칙이 `needs_session()` 을 안 보는 것이 여기서 문제다.
3. 원칙 문서 두 곳을 고친다 — CLAUDE.md 「세션 복원」 첫 문장 · `docs/reference/architecture-overview.md` 「세션 복원 / 활성화」 첫 문장.

## 거부한 대안
- **codex만 원칙 예외로 박기** — 사용자가 이 안을 보고 「고친다」를 택했다. 예외를 박으면 **코드에 없는 갈래를 문서가 만든다**: `backend_session_id`도 `observe_session_id`도 이미 발급 주체를 모르는 중립 구조라, 「codex는 예외」는 문장에만 존재하는 divergence다. 그리고 셋째 백엔드가 같은 길을 요구하면 예외가 규칙이 된다.
- **우리 id(E)를 새로 만들고 `E → codex thread id(T)` 매핑을 디스크에 신설** — 적대 리뷰가 4안으로 올렸다. **그 매핑이 이미 있다**: 우리 id = `AgentId`, 백엔드 id = `backend_session_id`(`profile.rs:165`), 둘의 매핑 = 프로필 레코드 자체. ★**단 4안의 대가로 지적된 「T를 받고 커밋하기 전 크래시하면 복원 불가」는 이 결정에서도 그대로 남는다**★ — `observe_session_id`는 *불린 뒤*에만 원자적이고(`mutate_if` → `store.save`), 열려 있는 창은 **응답 수령 → 호출 사이**라 그 함수가 지지 못한다. 즉 4안을 기각한 사유는 크래시 창이 아니라 **매핑이 이미 프로필 레코드라는 것 하나뿐**이다. 그 창은 아래 「영향」이 Phase 2 요구사항으로 진다.
- **원칙에 「서버 발급 + 프로토콜 응답 수령 + 우리가 보관」 갈래를 덧붙이기** — 적대 리뷰가 **합리화**로 판정했다(통제의 정의를 넓히는 것). 실제로는 갈래를 더할 자리가 없다 — 원래 불변식이 답하던 질문은 「복원이 무엇에 의존하나」였고 거기 발급 주체는 들어 있지 않았다. 이 결정은 원칙을 넓히는 것이 아니라 원래 뜻을 적는 쪽이다.
- **Phase 2를 하지 않기** — resume·구조화 챗·우편이 통째로 막힌다. 셋의 뿌리가 하나(상주 JSON 스트림)라 하나만 포기하는 선택지가 아니다 — 우편이 Phase 1에서 꺼진 사유도 「턴을 관측할 수 없다」로 같은 뿌리다(TRD §4-3).

## 근거
- **스키마 + 상류 거절** — `ThreadStartParams`에 id 계열 속성 0개(codex 자신이 내보내는 JSON 스키마) + 그 파라미터를 넣자는 issue가 메인테이너 판정으로 닫힘(openai/codex#15767). 「아직 없다」가 아니라 **「넣지 않기로 했다」**라서, 기다리는 선택지가 없다.
- **코드가 이미 중립이다** — `backend_session_id: Option<Uuid>`(`profile.rs:165`)에 발급 주체를 적는 칸이 없고, `observe_session_id`(`profile.rs:629`)가 claude `/clear` 추적에 쓰이며 이미 「백엔드가 바꾼 sid를 받아 즉시 persist」를 한다. **이 라운드가 바꾼 것은 문서뿐이다.** ★**단 「그래서 코드 변경이 0이다」로 읽지 말 것**★ — 자료구조가 발급 주체 중립인 것과 배선이 서 있는 것은 다르고, 그 차이가 결정 2의 셋이다.
- **ADR-0008의 기각 전제가 codex엔 없다** — 그 ADR이 「claude 자동 sid에 의존」을 버린 사유는 *그 sid를 알 방법이 없다*였다(`sessions/<pid>.json` 폴링뿐). codex는 스레드 생성 **응답으로** id를 준다 — 특정 불가라는 전제가 성립하지 않는다.

## 영향 / 불변식
- **불변식(개정판) — 여기까지가 지금 성립한다:** 복원은 프로필에 저장된 backend sid **단독**에 의존한다. 발급 주체는 백엔드가 정한다.
- **Phase 2 요구사항(아직 불변식이 아니다):** 응답으로 받은 sid는 **첫 턴을 허용하기 전에** persist한다 — 그 사이가 복원 불가 구간이다. ★**이것을 강제하는 것은 지금 아무것도 없다**★ — `turn/start`라는 이름이 이 저장소의 코드·문서에 하나도 없고 근거는 스키마 읽기뿐이다. 성립한 불변식으로 인용하지 말 것.
- **claude 경로는 한 줄도 바뀌지 않는다** — ADR-0008의 `--session-id` 발급 · 추적 파일 best-effort 등급 · 3s 조기종료 윈도 · 콜드부팅 복원 정책 전부 유효하다. 이 ADR이 개정한 것은 **발급 주체가 우리여야 한다는 조항 하나**다.
- **새 매핑·새 필드를 만들지 말 것** — 만들면 프로필 레코드와 이중 출처가 되고, 어느 쪽이 정본인지 다음 세션이 못 가린다.
- **미검(이 ADR 확정 시점):** app-server로 **실제 스레드를 만들어 id를 받아 본 적이 없다.** `initialize` 왕복만 실측했다(Windows stdio, stdin 닫으면 종료코드 0). 스레드 생성 응답의 실제 필드 모양은 Phase 2의 첫 게이트로 확인한다 — 거기서 어긋나면 결정 2가 흔들린다(결정 1은 스키마·상류 거절 위에 서므로 그대로다).
- **ADR-0076의 「sid 발급 단일 권위점」은 claude 축의 말이다** — codex는 우리가 발급하지 않으므로 `spawn_agent`이 sid를 심지 않고(`needs_session()` = false), 대신 **수령 단일점이 `observe_session_id`다**. ★**「codex Fresh가 `new_session_id`로 엉뚱한 uuid를 심는다」는 함정은 `needs_session()`을 true로 뒤집는 순간 열린다**★ — 지금은 `spawn_agent`이 구조적으로 도달하지 못한다(`manager.rs:954-962`의 `if needs`). 그런데 Phase 2는 `restore_one`이 codex를 resume 가능으로 보게 하려고 그 플래그를 켜야 하고, 한 플래그가 셋을 겸하므로 **켜기 전에 쪼개야 한다**(결정 2 ②). 쪼개지 않고 켜면 codex가 쓰지 않는 uuid가 프로필에 실리고 추적기가 없는 파일을 폴링한다(`docs/process/S21-codex-backend/trd.md:537`).
- ★**`backend/codex/mod.rs:141-144`(및 `:56-58`)의 주석은 이 결정과 반대를 적고 있다**★ — 「호출자가 sid를 못 정하므로 무손실 복원이 성립하지 않는다(실측)」. 결정 1은 발급 주체가 복원 성립과 무관하다고 정하므로 그 문장은 낡았다. **이 라운드는 문서만 바꿨으니 그 주석은 그대로 남아 있다 — Phase 2가 `// ADR-0185` 앵커와 함께 고친다.** 그 자리가 Phase 2 세션이 가장 먼저 읽는 텍스트라 방치하면 결정이 뒤집힌 채 전달된다.
- **코드 앵커가 아직 0이다** — `rg "ADR-0185" crates/ src/ src-tauri/` → 0줄. CLAUDE.md가 이름한 두 발견 표면 중 하나가 비어 있다는 뜻이고, 채우는 것은 위 주석 수정과 같은 라운드(Phase 2)다.
- **개정 전 프레이밍이 남아 있는 문서 둘 — 둘의 처분이 다르다.**
  - `docs/process/S21-codex-backend/trd.md`(§0 resume 유보 사유 · M2 「claude 계약이 codex엔 성립하지 않는다」 · §413) — **Phase 0·1 시점 기록이라 고치지 않는다.** Phase 2 TRD가 이 ADR을 입력으로 받아 그 프레이밍을 대체한다.
  - ★`docs/reference/structure/agent-backend.md:140,146`★ — 이쪽은 **시점 기록이 아니라 이 축의 주제별 상세 정본**이고(ADR-0179) 조감도가 독자를 여기로 내려보낸다. 그런데 `capabilities().session.resume`을 「무손실 복원 가능 여부」로 발급 주체에 묶어 설명한다. **다음 라운드에서 이 ADR 포인터를 달아야 한다** — 안 달면 더 새 문서가 개정 전 불변식으로 이 축을 설명한다.
