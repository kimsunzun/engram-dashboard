# ADR-0253: 사용량 값이 바뀌면 곧바로 밀어낸다 — UsageLimitsUpdated 와 칸마다 revision

- 상태: 확정 (2026-09-27, 근거: 사용자 결정 + TRD §7 #9, 개정 3·4 는 §6 #22)
- 관련: TRD §7 #9 · §6 #22(revision 재설정 순서 가정) · §6 #23(DaemonEvents 일곱째 알림) · `src-tauri/src/daemon_client/events.rs` · `connection.rs` · step-log S21

## 맥락
사용량 값이 바뀌었을 때 언제 화면에 반영할지 — 다음 요청이 올 때까지 대기 맵에 쌓아 둘지, 바뀌는 즉시 밀어낼지 정해야 했다.

## 결정
값이 바뀌면 **곧바로 밀어낸다** — `UsageLimitsUpdated` 이벤트 + **칸(회사)마다 `revision`**. 대상은 **그 회사를 구독한 연결뿐**(연결당 출구 — 전-연결 팬아웃이 아니다). 계기는 `revision`이 오를 때마다(줍기는 회사당 1초로 합쳐 보내고, 끝에는 반드시 한 번 발행을 보장한다). 프론트는 `revision`이 더 작은 갱신은 버리고, **셸 소켓 표식(`socketEpoch`)이 커질 때만 `revision`을 잊는다** — 연결 상태 전이(`connected`를 떠남)만으로는 잊지 않는다(개정 3·4 — TRD §6 #22가 그 두 구멍을 기록한다).

## 거부한 대안
- **다음 요청 때 적용(옛 대기 맵)** — 줍기 결과가 화면에 닿기까지 다음 요청 전(최대 60초)까지 걸린다.

## 근거
push 원천을 도착 즉시 병합하는 참조 선례(t3code·Orca·Codex TUI·CodexMeter — `docs/research/usage-limit-display-ui-survey-2026-09-26.md:89-108`) · 사용자 결정 2026-09-27(「이왕 빨리 전달할 수 있으면 즉각즉각이 맞지싶어」) · TRD §7 #9.

## 영향 / 불변식
`DaemonEvents`의 일곱째 알림(`usage_limits_updated`)이 「여섯 가지 전부 — 넓히지 말 것」(`daemon_client/events.rs:68-70`) 머리 주석의 유일한 예외다 — 순수 1:1 전달(관심 상태·소켓 표식·sink 를 안 쥔다)이라 그 경고 대상이 아니라고 판단했다(TRD §6 #23). `revision` 비교·`socketEpoch` 잊기 규칙을 깨면 재연결 경합에서 오래된 값이 새 값을 덮어쓸 수 있다. 구현 커밋: `7aad8f7`, `00e6683`, `088266e`.
