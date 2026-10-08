# ADR-0286: 데몬 테스트 서버 함수는 데몬 기능 test-support 뒤에 둔다 — 운영 그래프 게이트는 두지 않는다

- 상태: 확정 (2026-10-08, 근거: 사용자 위임 2026-10-07 아래의 메인 결정 · 작업 순서 2-4(`docs/refactoring/architecture-discussion-2026-09-26.md` §10))
- 관련: ADR-0088(데몬 자기 dev 재선언으로 기능을 켜는 모양) · ADR-0269 결정 5 · ADR-0275 결정 5(base · platform 시험 기능 `test-support` — 같은 이름) · ADR-0275 거부한 대안 N1(`test-harness` 이름 기각) · ADR-0271 결정 4(셸 같은 경로 시험이 데몬 `data_dir` 을 부른다) · 메모 `docs/refactoring/architecture-discussion-2026-09-26.md` §8 · §10 · `crates/engram-dashboard-daemon/src/lib.rs` · `crates/engram-dashboard-daemon/Cargo.toml` · `src-tauri/Cargo.toml` · step-log S21 · Amends ADR-0273 (영향의 남은 데몬 경계 항목)

## 맥락

데몬 lib 의 in-process 테스트 서버 함수(`start_test_server` · `start_test_server_with_keepalive` · `start_test_server_with_store` · 핸들 `TestServerHandle`)는 운영 `run()` 이 하나도 부르지 않는데 테스트 표시 없이 공개 API 로 나가 컴파일됐다. 이 함수들만 쓰는 메모리 저장소 둘(`MemProfileStore` · `MemPresetStore`)과, keepalive 주입판의 공개 시그니처 때문에 둔 `KeepaliveConfig` 재수출도 같은 처지였다.

지도 노트(`docs/reference/architecture-map-notes.md:288` — 「테스트 비계가 공개 API에 실려 나간다 … `experiment` 는 feature 게이트가 있고 테스트 서버 헬퍼는 없다」)와 경계 메모 §8 이 이를 적었고, 메모 §10 의 작업 순서 2-4 가 「테스트용 서버 함수를 테스트 전용 플래그 뒤로」를 잡았다. 호출부는 데몬 통합 시험(`tests/ws_e2e.rs`)과 셸 단위 시험 한 곳(`src-tauri/src/daemon_client/tests.rs`)이다. 셸 → 데몬 간선은 dev 의존이다.

## 결정

1. **데몬에 빈 기능 `test-support` 를 두고 위 항목 전부를 그 뒤에 둔다** — 테스트 서버 함수 셋 · `TestServerHandle` · 내부 `start_test_server_inner` · 메모리 저장소 둘 · `KeepaliveConfig` 재수출. 이 기능은 다른 crate 의 기능을 켜지 않는다. 기능이 꺼진 빌드에는 이 항목들이 없으므로 쓰이지 않는 코드 경고도 없다. 이름은 base · platform 의 시험 기능과 같다(ADR-0269 결정 5 · ADR-0275 결정 5).
2. **켜는 자리는 dev 의존 둘이다** — 데몬 자기 재선언(ADR-0088 모양)이 `features = ["test-harness", "test-support"]`, 셸 `src-tauri/Cargo.toml` 의 데몬 dev 의존이 `features = ["test-support"]`. 호출부는 그대로다.
3. **새 CI 게이트를 두지 않는다.** 데몬 밖의 지금 멤버는 전부 데몬으로 가는 정상(운영) 간선을 얻는 순간 이미 막힌다:
   - 셸 — `shell gate 1` 이 빨개진다(데몬 → agent 가 agent 를 셸 정상 그래프로 끌고 온다).
   - cli — `cli gate 1`(직접 워크스페이스 의존 = 정확히 agent · cli · command 이름 집합).
   - base · platform · command · messaging · transport · net — 각자의 depth 1 의존 상한 게이트.
   - agent · protocol — 데몬이 둘을 정상 의존하므로 cargo 의존 순환이 된다.

   그래서 막히지 않은 길은 데몬 자신의 `default` 에 `test-support` 를 넣는 것과 데몬을 정상 의존하는 새 패키지 둘뿐이고, 둘 다 매니페스트에서 보이는 편집이다.

## 거부한 대안

사용자 위임 2026-10-07 아래의 메인 결정이다.

- **기존 `test-harness` 재사용** — 그 기능은 데몬 자신의 하네스 몫(`handle_send` 의 송신 도중 hook · CLI 강제 노브 · `experiment` 모듈)에 더해 agent(`insert_test_session` 등)와 messaging(`set_accept_hook_for_test` 등 하네스 관찰 seam)의 같은 이름 기능까지 켠다 — 대부분이 운영 코드 안의 seam 이다. 셸 dev 의존에 걸면 셸 테스트 빌드 안에서 그것들이 함께 켜진다. 이름도 뜻이 갈린다 — ADR-0275 거부한 대안 N1 이 `test-harness` 를 시험 기능 이름으로 기각한 이유가 「agent · daemon · messaging 이 「운영 코드 안의 seam 을 연다」는 다른 뜻으로 쓴다」이다.
- **base · platform 꼴 운영 그래프 게이트 추가** — 그 꼴은 선다: `cargo tree -p engram-dashboard -e normal,build,features -i engram-dashboard-daemon --target all | rg 'feature "test-support"'` 는 0줄이고, `-e` 에 `dev` 를 넣은 짝은 1줄이다(실측 2026-10-08 — 셸 정상 그래프에 데몬이 없어도 데몬이 셸의 resolve 안에 있으므로 `cargo tree` 는 「nothing to print」로 rc=0 이다). 그러나 결정 3 이 지금 멤버를 이미 다 막아 더 덮는 것이 적고, 게이트를 더하면 qa 바인딩 줄도 더해야 한다(바인딩 편집 = 사용자 승인 대상).
- **테스트 함수를 별도 test-util crate 로 옮기기** — `start_test_server_inner` 가 데몬의 비공개 조립 함수 · 타입(`build_daemon_wiring_with_store` · `run_accept_loop` · `UsageInputs`)을 부르므로, 바깥 crate 로 옮기면 그것들을 공개 API 로 넓혀야 한다 — 테스트 비계를 공개 API 에서 빼려는 목적과 거꾸로다. 그리고 셸 → 데몬 dev 간선은 어차피 남는다 — 셸 시험이 실제 데몬 조립을 띄우고(메모 §8) 셸 같은 경로 시험이 데몬 `data_dir` 을 부른다(ADR-0271 결정 4).

## 근거

- 코드 사실: 운영 `run()` 은 이 항목을 하나도 부르지 않는다(공유 도우미 `generate_token` · `build_daemon_wiring_with_store` · `run_accept_loop` · `UsageInputs` 는 기능 밖에 그대로 둔다). 하네스 bin 도 부르지 않는다.
- 다른 패키지 중 데몬을 의존하는 매니페스트는 셸 dev 의존 하나다(`rg "engram-dashboard-daemon" --glob '**/Cargo.toml'`).
- 기능 뒤로 옮겨도 시험은 하나도 빠지지 않는다 — 워크스페이스 회귀가 구현 전후 같다(실측 2026-10-08 · 결과 줄 59 · 4233 통과 · 0 실패 · 31 무시).
- 기능 없는 데몬 lib 에 대고 `start_test_server` · `TestServerHandle` 을 부르는 코드는 `E0425`(「configured out」)로, `KeepaliveConfig` 는 「private」로 컴파일이 실패하고, 기능을 켠 lib 에서는 같은 코드가 선다(실측 2026-10-08 — rustc 로 두 lib 메타데이터에 대고 직접 컴파일).

## 영향 / 불변식

- **셸 → 데몬 dev 간선은 남는다.** 셸 테스트 빌드는 여전히 데몬과 그 밑의 agent 를 컴파일한다 — CI `shell gate 1` 이 `-e` 에서 `dev` 를 빼는 이유가 그대로다.
- **한 cargo 호출의 기능 합치기 주의는 그대로 걸린다** — lib 과 시험 타깃을 한 호출로 지으면(`--all-targets` 등) dev 의존이 켠 `test-support` 가 그 호출의 데몬 lib 에 합쳐진다(데몬 `Cargo.toml` 「운영 빌드 주의」 — self dev 의존의 `test-harness` 와 같은 위험군). 배포용 빌드는 그렇게 짓지 않는다.
- **다른 패키지의 시험이 테스트 서버를 쓰려면 자기 데몬 dev 의존에 `features = ["test-support"]` 를 단다.** 운영 의존이나 데몬 `default` 에 넣지 않는다. **새 패키지가 데몬을 운영 의존하거나 데몬 `default` 에 `test-support` 를 넣으면 이 결정을 다시 연다(그때 base · platform 꼴 게이트를 데몬 자신과 그 새 패키지를 대상으로 더한다).**
- `KeepaliveConfig` 는 운영 코드(accept loop)도 쓰므로 기능이 꺼진 빌드는 같은 타입을 비공개로 들인다 — 기능 뒤에 있는 것은 공개 재수출뿐이다.
- 찾는 법 = `rg "ADR-0286"`.
