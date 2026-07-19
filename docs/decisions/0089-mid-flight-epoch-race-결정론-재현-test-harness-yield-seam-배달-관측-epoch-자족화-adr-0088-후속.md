# ADR-0089: mid-flight epoch race 결정론 재현 — test-harness yield seam + 배달 관측 epoch 자족화 (ADR-0088 후속)

- 상태: 확정 (2026-07-20, 근거: 사용자 승인(S17 단계 1 follow-up 결정 3종 일괄 착수) + /review code deep 3렌즈(doc-aware·동시성·cross-family Codex) 2라운드 PASS + /qa full PASS)
- 관련: ADR-0088(배달 정확성 검증 — 이 결정이 그 Stage 1의 예고된 follow-up) · ADR-0086 §F5(epoch 경쟁 design-accepted — 이 결정은 그 수용을 바꾸지 않고 *관측*만 결정론화) · ADR-0006(락 규율 — hook 발화·drop 위치가 이 규율을 따름) · ADR-0012(격리 하네스)

## 맥락
ADR-0088 Stage 1 오라클 5(epoch 회전 배달)는 두 공백을 정직하게 남겼다: ① **진짜 mid-flight epoch race**(resolve가 epoch 0을 보고 그 직후 재시작으로 epoch 1이 된 뒤 write가 도착)는 `handle_send`의 resolve↔write 사이에 외부가 끼어들 yield 지점이 프로덕션에 없어 **결정론적으로 재현 불가** — 순차 교체만 검증됐다. ② `DeliveryObservation`이 수신자 epoch을 담지 않아 "정확히 어느 incarnation이 받았나"를 **레코드만으로 단정 불가**(캡처 버퍼 대조로 간접 확인만). 사용자가 이 follow-up 착수를 승인했다(2026-07-19~20).

## 결정
1. **`handle_send`에 test-hookable yield seam** — `ControlRegistry`에 `#[cfg(feature = "test-harness")]` 게이트된 `mid_send_hook`(RwLock\<Option\<Arc\<dyn Fn()\>\>\>)을 두고, `handle_send`가 `write_stdin_observed` 호출 **직전**(resolve↔write 간극의 최후 지점)에 발화한다. 발화는 Arc clone 후 락 밖 호출(ADR-0006), setter는 구 hook을 락 해제 후 drop. 운영 빌드에는 통째로 미컴파일.
2. **daemon `test-harness` feature + self-dev-dependency** — daemon이 자기 자신을 `[dev-dependencies]`로 feature와 함께 참조해, `cargo test -p engram-dashboard-daemon`이 플래그 없이 hook을 켠다. 운영 그래프(normal edges)에는 feature가 유니피케이션되지 않음을 cargo tree로 실증. **운영 주의 1건 문서화:** `cargo build --release --all-targets`는 dev-feature를 유니피케이션하므로 릴리즈 바이너리를 그 형태로 만들지 않는다(Cargo.toml 주석).
3. **배달 관측 epoch 자족화** — `WriteOutcome`에 `epoch: u32`(write를 집행한 세션 incarnation의 by-construction 값), `DeliveryObservation`에 `to_epoch: Option<u32>`(성공=`Some(outcome.epoch)` = **착지 epoch**, 실패=`None` = "완결 write 부재 → 입증할 incarnation 없음" — 0바이트 이동 주장이 아님). `is_delivered()`에 epoch 조건을 넣지 않는다(관측 축이지 유효성 게이트 아님).

## 거부한 대안
- **비결정 상태 유지(순차 교체 검증만)** — race가 design-accepted(F5)니 관측도 간접으로 충분하다는 입장. 거부: 스케줄러 의존 재현은 회귀를 못 잡고, 오라클 5 docstring이 "이 테스트는 race가 안전하다고 주장하지 않는다"고 남긴 공백이 영구화된다. 결정적 커버리지가 follow-up으로 예고돼 있었고 사용자가 착수를 골랐다.
- **epoch pinning(resolve 시점 epoch에 배달 고정)** — 거부: ADR-0086 §F5가 명시 거부한 방향 — 메일은 논리 에이전트를 향하므로 pinning하면 재시작 중 유실이 생긴다. 이 결정은 배달 시맨틱을 1비트도 바꾸지 않는다(seam은 관측 전용).
- **to_epoch에 resolve-시점 스냅샷 epoch 기록** — 거부: race 관측의 목적이 "실제로 어느 incarnation에 착지했나"라 resolve 스냅샷을 실으면 레코드 자족성이 무너진다(간접 대조로 회귀). 착지 세션의 `self.epoch`이 유일하게 옳은 출처(write와 같은 Arc).
- **테스트 실행에 `--features` 플래그 요구(self-dev-dep 대신)** — 거부: `cargo test -p engram-dashboard-daemon` 맨 호출이 조용히 hook 없는 빌드로 돌아 신규 오라클이 상시 실패하거나 조용히 skip되는 각. self-dev-dep은 플래그 없이 dev 그래프에만 feature를 켠다.
- **hook을 catch_unwind로 감싸기** — 거부(관찰만 기록): observer(운영 존재)와 달리 hook은 테스트 전용·운영 미컴파일이라 panic 전파가 곧 테스트 실패 신호로 정상. 감싸면 오히려 실패가 묻힌다.

## 근거
- 신규 오라클 `stage1_lifecycle_mid_flight_epoch_race_lands_on_new_incarnation_deterministic`: resolve=epoch 0 → hook이 같은 AgentId의 epoch 1 세션으로 교체 → write 착지 → 레코드 1건·`is_delivered`·`to_epoch==Some(1)`·epoch-1 버퍼에만 바이트. TDD 판별력 실증: hook 발화를 무력화하면 `to_epoch=Some(0)`으로 실패(resolve-시점 배선 회귀도 같은 단언이 잡음).
- 리뷰: doc-aware + 동시성 렌즈 + cross-family(Codex) 2라운드 — fire 지점 무락(발화 시 보유 락 0)·guard-lifetime 정확·운영 그래프 feature 미유입(cargo tree normal edges 공란)·feature-off byte-identical 실증. Codex R1 FIX 5건(–all-targets 문서화·구 hook 락내 drop·실패 주석 과대주장·to_epoch None 단언 누락·테스트 hook Arc 순환) 전부 반영 후 R2 PASS.
- QA full: 전 멤버 회귀 + 격리 + 프론트 + GUI 실측(spawn→출력→kill 스모크, 신규 바이너리) PASS.

## 영향 / 불변식
- **seam은 관측 전용 — 배달 시맨틱 불변.** feature OFF에서 `handle_send`는 변경 전과 동작 동일(유일한 프로덕션 코드 이동은 setter의 구 hook drop 위치이며 test-harness 빌드 한정). epoch pinning을 이 seam 위에 얹으려는 시도는 F5 위반.
- **`to_epoch`는 착지 epoch — resolve 스냅샷으로 되돌리면 자족성 회귀.** 성공 레코드의 epoch 출처는 write를 집행한 세션의 `self.epoch`뿐이다.
- **릴리즈 바이너리를 `--all-targets`로 만들지 않는다**(dev-feature 유니피케이션 — daemon Cargo.toml 주석이 앵커).
- 코드 앵커: `crates/engram-dashboard-daemon/src/control/{ingress.rs(fire 지점·to_epoch),registry.rs(mid_send_hook)}` · `crates/engram-dashboard-core/src/agent/{types.rs(WriteOutcome.epoch),session.rs(write_input_observed)}` — 각 `// ADR-0088` 앵커 인접(이 ADR은 그 follow-up 결정의 *왜*).
