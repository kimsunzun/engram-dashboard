# ADR-0286: 데몬 테스트 서버 함수는 데몬 기능 test-support 뒤에 둔다 — 운영 그래프 게이트는 두지 않는다

- 상태: 확정 (2026-10-08, 근거: 사용자 위임 2026-10-07 아래의 메인 결정 · 작업 순서 2-4(`docs/refactoring/architecture-discussion-2026-09-26.md` §10))
- 관련: ADR-0088(데몬 자기 dev 재선언으로 기능을 켜는 모양) · ADR-0269 결정 5 · ADR-0275 결정 5(base · platform 시험 기능 `test-support` — 같은 이름) · ADR-0273 「영향」(남은 데몬 경계 — 이 결정이 그 항목을 처리한다) · ADR-0271 결정 4(셸 같은 경로 시험이 데몬 `data_dir` 을 부른다) · 메모 `docs/refactoring/architecture-discussion-2026-09-26.md` §8 · §10 · `crates/engram-dashboard-daemon/src/lib.rs` · `crates/engram-dashboard-daemon/Cargo.toml` · `src-tauri/Cargo.toml` · step-log S21

## 맥락

데몬 lib 의 in-process 테스트 서버 함수(`start_test_server` · `start_test_server_with_keepalive` · `start_test_server_with_store` · 핸들 `TestServerHandle`)가 테스트 표시 없이 공개 API 로 나가 있었다. 운영 `run()` 은 이 중 하나도 부르지 않는데 데몬 lib 를 링크하는 모든 빌드에 실렸다. 이 함수들만 쓰는 메모리 저장소 둘(`MemProfileStore` · `MemPresetStore`)과, keepalive 주입판의 공개 시그니처 때문에 둔 `KeepaliveConfig` 재수출도 같은 처지였다.

지도 노트(`docs/reference/architecture-map-notes.md:288` — 「테스트 비계가 공개 API에 실려 나간다 … `experiment` 는 feature 게이트가 있고 테스트 서버 헬퍼는 없다」)와 경계 메모 §8 이 이를 적었고, 메모 §10 의 작업 순서 2-4 가 「테스트용 서버 함수를 테스트 전용 플래그 뒤로」를 잡았다. 호출부는 데몬 통합 시험(`tests/ws_e2e.rs`)과 셸 단위 시험 한 곳(`src-tauri/src/daemon_client/tests.rs`)이다. 셸 → 데몬 간선은 dev 의존이다.

## 결정

1. **데몬에 빈 기능 `test-support` 를 두고 위 항목 전부를 그 뒤에 둔다** — 테스트 서버 함수 셋 · `TestServerHandle` · 내부 `start_test_server_inner` · 메모리 저장소 둘 · `KeepaliveConfig` 재수출. 이 기능은 다른 crate 의 기능을 켜지 않는다. 기능이 꺼진 빌드에는 이 항목들이 없으므로 쓰이지 않는 코드 경고도 없다. 이름은 base · platform 의 시험 기능과 같다(ADR-0269 결정 5 · ADR-0275 결정 5).
2. **켜는 자리는 dev 의존 둘이다** — 데몬 자기 재선언(ADR-0088 모양)이 `features = ["test-harness", "test-support"]`, 셸 `src-tauri/Cargo.toml` 의 데몬 dev 의존이 `features = ["test-support"]`. 호출부는 그대로다.
3. **새 CI 게이트를 두지 않는다.** 데몬을 정상(운영) 간선으로 의존하는 패키지가 없다 — 셸의 간선은 dev 뿐이고 cli 패키지는 데몬을 의존하지 않는다. 그래서 「`test-support` 가 운영 그래프에 실린다」로 가는 길은 누가 데몬 `default` 에 넣는 것 하나뿐이다. 셸 정상 그래프에 대고 `cargo tree -i engram-dashboard-daemon` 을 돌리면 대상이 그래프에 없어 rc=101 로 죽는다 — base · platform 게이트와 달리 그 꼴이 서지 않는다.

## 거부한 대안

사용자 위임 2026-10-07 아래의 메인 결정이다.

- **기존 `test-harness` 재사용** — 그 기능은 agent · messaging 의 시험 seam(`insert_test_session` · 송신 도중 hook)까지 켠다. 셸 dev 의존에 걸면 셸 테스트 빌드 안에서 그 seam 들이 함께 켜진다.
- **base · platform 과 같은 운영 그래프 게이트 추가** — 결정 3 의 이유다. 데몬으로 가는 운영 간선이 없어 잴 대상이 없고, `-i engram-dashboard-daemon` 꼴은 rc=101 로 죽는다.
- **테스트 함수를 별도 test-util crate 로 옮기기** — 함수 넷에 비해 장치가 크다. 그리고 셸 → 데몬 dev 간선은 어차피 남는다 — 셸 시험이 실제 데몬 조립을 띄우고(메모 §8) 셸 같은 경로 시험이 데몬 `data_dir` 을 부른다(ADR-0271 결정 4).

## 근거

- 코드 사실: 운영 `run()` 은 이 항목을 하나도 부르지 않는다(공유 도우미 `generate_token` · `build_daemon_wiring_with_store` · `run_accept_loop` · `UsageInputs` 는 기능 밖에 그대로 둔다). 하네스 bin 도 부르지 않는다.
- 데몬을 의존하는 매니페스트는 셸 dev 의존 하나다(`rg "engram-dashboard-daemon" --glob '**/Cargo.toml'`).
- 기능 뒤로 옮겨도 시험은 하나도 빠지지 않는다 — 워크스페이스 회귀가 구현 전후 같다(실측 2026-10-08 · 결과 줄 59 · 4233 통과 · 0 실패 · 31 무시).
- 기능 없는 데몬 lib 에 대고 `start_test_server` · `TestServerHandle` 을 부르는 코드는 `E0425`(「configured out」)로, `KeepaliveConfig` 는 「private」로 컴파일이 실패하고, 기능을 켠 lib 에서는 같은 코드가 선다(실측 2026-10-08 — rustc 로 두 lib 메타데이터에 대고 직접 컴파일).

## 영향 / 불변식

- **셸 → 데몬 dev 간선은 남는다.** 셸 테스트 빌드는 여전히 데몬과 그 밑의 agent 를 컴파일한다 — CI `shell gate 1` 이 `-e` 에서 `dev` 를 빼는 이유가 그대로다.
- **한 cargo 호출의 기능 합치기 주의는 그대로 걸린다** — lib 과 시험 타깃을 한 호출로 지으면(`--all-targets` 등) dev 의존이 켠 `test-support` 가 그 호출의 데몬 lib 에 합쳐진다(데몬 `Cargo.toml` 「운영 빌드 주의」 — self dev 의존의 `test-harness` 와 같은 위험군). 배포용 빌드는 그렇게 짓지 않는다.
- **다른 패키지의 시험이 테스트 서버를 쓰려면 자기 데몬 dev 의존에 `features = ["test-support"]` 를 단다.** 운영 의존이나 데몬 `default` 에 넣지 않는다 — 결정 3 이 게이트 없이 기대는 규율이 이것 하나다. 데몬을 운영 의존하는 패키지가 생기면 결정 3 의 전제가 무너지므로 base · platform 같은 게이트를 다시 본다.
- `KeepaliveConfig` 는 운영 코드(accept loop)도 쓰므로 기능이 꺼진 빌드는 같은 타입을 비공개로 들인다 — 기능 뒤에 있는 것은 공개 재수출뿐이다.
- 찾는 법 = `rg "ADR-0286"`.
