# ADR-0030: capability 산출 = transport(물리) ⊕ backend(프로그램) 합성

- 상태: 확정 (2026-06-21 dashboard11, 근거: 코더→reviewer-deep(Blocker 0)→QA 실측[shell resume=false / claude resume=true, 실제 WS→데몬→프론트 IPC 경로])
- 관련: ADR-0002(capability 매트릭스 — 본 ADR이 산출 위치를 구체화) · ADR-0004(AgentTransport seam + backend 지식 격리) · CLAUDE.md §2(capability 매트릭스)·§3(backend 격리) · `crates/engram-dashboard-core/src/agent/types.rs`(TransportCaps/BackendCaps/Capabilities::compose)·`agent/backend/mod.rs`(backend_caps dispatch)·`agent/session.rs`(compose)
- 범위: 에이전트 capability(기능 매트릭스)를 **누가 결정하는가**. 값 자체가 아니라 산출 책임의 분할(seam).

## 맥락

`PtyTransport::capabilities()`가 `session.resume = true`를 **backend(claude vs shell) 무관하게 하드코딩**했다. 그 결과 범용 셸 에이전트도 `resume=true`로 나왔다(부정확 — 셸엔 `--resume`이 없다). 원인은 capability를 **transport 한 곳에서만** 산출했기 때문이다. transport(PtyTransport)는 어떤 프로그램을 띄웠는지(claude/shell) 모르도록 격리돼 있다(ADR-0004) — 그래서 backend-종속 capability(resume 등)를 정확히 채울 수 없다.

ADR-0002의 capability 매트릭스를 보면 capability는 사실 **두 출처**에서 온다:
- **물리 채널이 정하는 것:** resize(PTY는 됨, API는 무의미)·interrupt·raw 입력·terminal-bytes 출력 → *transport*가 안다.
- **프로그램이 정하는 것:** resume(claude `--resume` 됨, shell 안 됨)·model 선택/옵션(API 전용) → *backend*가 안다.

한 곳(transport)에서 둘 다 채우려니 backend-종속 값이 틀렸다.

## 결정

**capability를 두 출처의 합성으로 산출한다. 소유권을 타입으로 강제한다.**

- `TransportCaps { input, output, control }` — 물리 채널 caps. transport가 반환(`AgentTransport::capabilities() -> TransportCaps`). PTY = raw 입력·terminal-bytes·resize·interrupt.
- `BackendCaps { session, model }` — 프로그램 caps. backend가 반환(`AgentBackend::capabilities() -> BackendCaps`). claude `session.resume=true`, shell `session.resume=false`, codex/gemini stub `false`(CLI spike 전 보수값).
- `Capabilities::compose(TransportCaps, BackendCaps) -> Capabilities` — `AgentSession`이 둘을 합쳐 최종 5영역 Capabilities를 만든다. session은 spawn 시 `backend::backend_caps(&AgentCommand)`로 BackendCaps를 받아 보유.

**왜 타입 분리인가:** transport 타입엔 session/model 필드가 없고 backend 타입엔 input/output/control 필드가 없다 → 출처 혼입(transport가 resume을 채우는 등)이 **컴파일 단계에서 불가능**. 매트릭스(§2)와 1:1: resize=transport / resume=backend / model=backend.

## 거부한 대안

- **transport가 backend kind를 인자로 받아 전부 산출** — ADR-0004 격리(transport는 claude/codex 모름)를 깬다. transport에 backend 지식이 새면 백엔드 추가마다 transport를 고쳐야 함.
- **transport가 full Capabilities 반환 + session이 session/model만 덮어쓰기** — transport가 여전히 무의미한 session/model 값을 만들어 흘리고 session이 조용히 버린다(혼입을 타입이 못 막음, drop 실수 여지). 합성 타입 분리가 더 안전.
- **capability를 profile/AgentCommand에 정적 테이블로** — 산출 로직이 backend 구현과 떨어져 drift. backend가 자기 caps를 선언하는 게 응집도 높음.

## 근거

- reviewer-deep 적대적 리뷰: Blocker/Major 0. compose 5영역 1:1 매핑·소유권 타입 분리·resume 정확화 모두 의도대로 확인. 지적은 Minor(spec/caps dispatch가 별도 호출이라 미래 variant 추가 시 둘 다 갱신 필요 — `backend_for` 단일 진입이라 현재는 exhaustive match가 강제)·Nit(BackendCaps Copy 미파생으로 clone)뿐.
- QA 실측(실제 앱 WS→데몬→프론트 ProtocolClient): shell 에이전트 `capabilities.session.resume=false`, claude `=true`로 정확히 분기. 이전 하드코딩 true가 아님을 실증.
- core unit+통합 76 통과(claude resume=true·shell resume=false 회귀 테스트 + compose 소유권 합성 테스트 포함).

## 영향 / 불변식

- **capability는 `Capabilities::compose` 로만 생성한다.** transport는 TransportCaps만, backend는 BackendCaps만 반환 — 어느 한쪽이 상대 영역을 채우려 하면 타입 에러. 이 분리를 깨면 부정확(과거 shell resume=true)이 재발한다.
- **session/model = backend 소유, input/output/control = transport 소유.** 새 capability 필드 추가 시 이 기준으로 영역을 고른다.
- backend variant 추가 시 `backend_for`(dispatch)에 분기를 더하면 `build_command_spec`·`backend_caps`·`needs_session`이 같은 command를 보므로 caps와 실행 backend가 자동 정합(단일 진입 유지가 불변식).
- wire/protocol `Capabilities`(ts-rs 미러)는 5영역 그대로 — TransportCaps/BackendCaps는 core 내부 합성 전용(ts-rs 미부착, wire 비노출).
- codex/gemini의 BackendCaps는 dispatch 미연결이라 현재 죽은 코드(보수 stub). CLI spike 후 variant 연결 시 실측값으로 교체.
