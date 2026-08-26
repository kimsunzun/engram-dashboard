# 검증 절차 (결정·미룬 정리)

> **상태:** 착수 가능 · **정본 색인:** [`../todo.md`](../todo.md)

> **★flaky 기록은 두 건이다 — 서로 다른 테스트다.★** 첫째 = 아래 첫 항목(2026-08-25 · 브랜치 `v0.3.0/refactor/crate-boundaries` · `buffered_command_…` / `subscribe_emits_…`). 둘째 = 맨 아래 「flaky 2건」 절(2026-08-26 · 브랜치 `v0.3.0/fix/frontend-epoch-filter` · `stdio_…` 둘). 하나를 다른 하나로 읽지 말 것.

- **CI에서만 깨지는 테스트 2건 — 기록만 하고 단언은 손대지 않는다**(사용자 결정 2026-08-25). 둘 다 브랜치 `v0.3.0/refactor/crate-boundaries`의 GitHub CI에서만 났고 로컬 재현은 전패했다. ★원인 미상인 단언을 약화하면 진짜 회귀가 있을 때 그것까지 덮는다★. ★sleep·매직넘버 지연으로 맞추는 것도 금지★(ADR-0038). **지금 안 하는 이유:** 재현 수단이 없어 무엇을 고칠 대상으로 삼을지가 안 선다.
  - **`buffered_command_on_disconnect_is_drained_not_executed_post_reconnect`**(`src-tauri/src/daemon_client/tests.rs` · CI run **32848074505**) — 실패 문구 `B 도 끊김 Err: Ok(Ack { request_id: … })`. 끊김으로 Err에 깨어나야 할 버퍼 명령 B가 그냥 성공했다. ★그 단언 위 주석은 이미 「갈래(미전송/전송됨/송신 실패)는 race」라 자인하지만 **B가 끊김보다 먼저 성공하는 갈래는 그 목록에 없다**★ — 재발하면 그것을 정당한 넷째 갈래로 받을지부터 정한다.
  - **`subscribe_emits_ack_then_replay_then_complete_in_order`**(`crates/engram-dashboard-daemon/src/connection_core.rs` · CI run **32850418777**) — 실패 문구 `마지막은 Text(ReplayComplete) 여야 함: Binary([0, 212, …])`. `ReplayComplete` 자리에 tag-0 종단 프레임이 왔다. CLAUDE.md 「핵심 불변식」의 replay→live 순서를 지키는 단언이라, 사실이면 실결함이다.
  - **로컬 재현 전패(실측 2026-08-25):** `cargo test -p engram-dashboard --test lib_unit` ×10 → 매번 228 통과·0 실패 · `cargo test -p engram-dashboard-daemon --lib` ×12 → 매번 351 통과·0 실패 · 워크스페이스 회귀 초록.
  - ★**같은 커밋을 재실행하니 CI가 초록으로 돌았다 — 그 초록은 실패를 설명하지 않는다**★. ★**개명(ADR-0175)도 원인 후보에서 빠지지 않았다 — 재현이 안 됐을 뿐이다**★. 최근 CI 약 20건을 전 브랜치로 훑어도 실패는 이 브랜치에만 있었다. 둘 다 동시성 순서 불변식을 지키는 단언이라는 것이 공통점이다.
  - **재발 시 다음 수순:** 위 두 run과 초록 재실행 run의 러너 로그를 나란히 놓고 스레드 수·소요 시간·`--test-threads` 유무부터 대조한다(로컬만 4로 낮추고 CI는 안 낮춘다 — 그 갈림은 의도된 것이고 정본은 CLAUDE.md 「빌드·검증 명령」). 그 다음이 두 테스트를 CI에서 반복 실행해 실패율을 재는 것이며, 단언 수정은 원인이 선 뒤에만 한다.
- **스크린샷을 창 전체로 찍는 것 — 좁히지 않기로 결정**(사용자 2026-08-21). cdp 스크립트에 영역 지정이 아예 없고(포맷 인자만 받는다), 낭비의 실측 근거는 한 세션이 찍은 세 장 중 한 장이 무정보였던 것뿐이며, 여러 장을 한 장으로 합치려면 이 저장소에 없는 이미지 도구가 필요하다.
- **`/qa` 바인딩 전반 정리 — 미룸.** 사용자가 그 파일이 어수선하다고 판정했고, 다른 실측·qa 항목과 묶어 한 번에 손보기로 했다(사용자 결정 2026-08-21). 무엇을 어떻게 정리할지는 그때 정한다 — 여기 작업 목록을 미리 박지 않는다.
- **doc 단계 적대 리뷰의 컷 사이드 지적 — 미조정.** 블라인드 리뷰어가 `step-log.md` 이번 세션 블록·`qa.md`의 레시피·이 파일의 개명 보류 항목을 표면 간 중복을 근거로 더 쳐내자 주장했다. load-bearing 쪽과 정면 충돌하는 축이라 이번엔 판정 없이 최소안으로 닫았다(사용자 결정 2026-08-21).

## flaky 2건 — ★게이트가 깨져 있다. 제일 먼저★

- **무엇:** `crates/engram-dashboard-agent/tests/stdio_smoke.rs` 의 두 테스트가 CI 에서만 죽는다 — `stdio_decoder_routes_bytes_and_flushes_on_eof` · `stdio_no_decoder_passes_terminal_bytes_through`.
- **상태:** ★**살아 있다**★. 가장 최근 CI 가 이것으로 빨갛다(2026-08-26, 브랜치 `v0.3.0/fix/frontend-epoch-filter`, run **32936570502**). master 는 초록이라 직전 세션이 이걸 못 보고 끝냈다.
- ★**위 「CI에서만 깨지는 테스트 2건」과 다른 테스트다 — 합치지 말 것**★. 저쪽은 `buffered_command_on_disconnect_…` · `subscribe_emits_ack_…`(동시성 순서 단언)이고, 이쪽은 stdio 두 건이다. **이쪽은 이번이 첫 관측**이라 「같은 테스트 두 번째」 정지 조건은 아직 발화하지 않았다.
- ★**원인이 잡혀 있다 — 이쪽은 「원인 미규명」이 아니다**★(실측 2026-08-26, CI 로그):
  ```
  open: Io(Custom { kind: Other, error: Error {
      code: HRESULT(0x80070005), message: "Access is denied." } })
  ```
  둘 다 `StdioTransport::open` 에서 **자식 프로세스 생성이 거부**된다(`stdio_smoke.rs:328` · `:490`). 띄우려는 것은 `cmd.exe /c echo …` 로, 테스트 대상 코드가 아니라 **하네스가 쓰는 껍데기 프로세스**다. 즉 단언이 틀린 게 아니라 **프로세스를 못 띄운 것**이다.
- **같은 코드가 2분 전 master 에서 초록이었다** — 그 사이 커밋은 문서 하나뿐이라(`714ab09`) 코드가 동일하다. 회귀가 아니라 간헐이다.
- **아직 모르는 것:** 러너 부하·병렬도와 상관이 있는지, Defender 류가 물었는지. ★추측으로 메우지 말 것★ — 다음 수순은 재발 시 같은 run 의 병렬도·소요 시간을 초록 run 과 대조하는 것이다.
- **왜 급한가:** 미뤄 둘 항목이 아니라 **깨진 게이트**다. 이 상태로 다른 작업에 들어가면 push 마다 뜨는 빨강이 내 변경 탓인지 이 둘 탓인지 구별되지 않는다. 하필 명부 정합은 런타임 재현이 아직 한 번도 안 된 결함이라 그 오염이 특히 비싸다.
- **★착수 규율★:**
  - **재실행 버튼을 누르지 말 것 — 로그부터.** (위 원인 블록이 그 로그를 뜬 결과다.)
  - **`sleep` 로 덮지 말 것**(ADR-0038 — 추측·매직넘버 금지). 트리거·발화 규약은 `../reference/debugging-conventions.md`.
  - 로컬은 초록이고 CI 만 빨갛다 → **로컬 재현이 안 될 것을 전제로** 접근한다. 로컬/CI 차이의 정본은 `CLAUDE.md` 「빌드·검증 명령」의 `--test-threads=4` 예외 항목(로컬만 4로 돈다 — CI 는 의도적으로 안 쓴다).
- **정본:** 이 문서. 로컬/CI 의 `--test-threads` 갈림은 `../../CLAUDE.md` 「빌드·검증 명령」이 정본이고, 디버깅 절차는 `../reference/debugging-conventions.md` 다.
