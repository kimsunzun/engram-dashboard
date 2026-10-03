# 시간 seam(`Clock`)을 한 곳에 모으나 — 피어 서베이

- **상태:** medium · 설계-결정 모드 · 2026-10-03 · 적대 리뷰 BLOCK → 결론 정정(아래) · 결정 대기(ADR-0269 결정 3 — 리뷰 F2)
- **방법:** 수집자 1(피어 코드 · `gh api` 원문 + 문서) → 메인 grounding(zed · Materialize · quinn 원문 대조) → cross-family 적대 리뷰 1회.
- **확신도 범례:** 확실(독립 교차확증) · 가능성 높음(단일 출처 grounding) · 불확실(미검증).

## 왜 조사했나

ADR-0269 결정 3 은 「시간은 공용이면 무조건 합친다」(사용자 2026-09-26)로 `Clock` 네 벌을 base `time` 으로 합치기로 했다. 그런데 네 벌의 모양이 다르다(리뷰 F2 — 코드 대조): 데몬 `command_delivery.rs` = `now()` 하나 · discovery `lib.rs` = `now()` + 막는 `sleep` · transport `clock.rs` = `now()` + 비동기 `sleep`(tokio) · 데몬 `usage_service/clock.rs` `UsageClock` = `mono() -> Duration` + `wall() -> i64`. 통째로 합치면 base 가 tokio 를 끈다.

## 발견

1. **seam 을 공용 crate 에 두는 것은 흔하다 — 단 「워크스페이스 전체에 Clock trait 하나」는 아니다** (가능성 높음). 큰 워크스페이스는 기존 util · 런타임 crate 에 둔다: zed `scheduler`(`crates/scheduler/src/clock.rs` — 메인 원문 대조) · Materialize `mz_ore::now`(`src/ore/src/now.rs` — 메인 원문 대조) · TiKV `tikv_util::time` · Firecracker `utils::time`. 그래도 zed 는 seam 이 **둘**이다(`clock` crate 의 `SystemClock` · `scheduler::Clock`).
2. **비동기 sleep 을 실행기(런타임)가 쥐는 피어가 있다** (가능성 높음 — 빈도는 세지 못했다). ★「거의 안 둔다」는 과장이었다★ — `now` 옆에 sleep 을 두는 trait 도 있다: `cruxi_clock::Clock`(막는 sleep) · `async_speed_limit::clock::Clock`(비동기 sleep — TiKV 계열)(적대 리뷰 반례). tokio `time::pause`/`start_paused` · zed gpui `advance_clock`(테스트 디스패처) · fuchsia-async 가짜 시간 실행기. 예외 = quinn `Runtime` trait(`new_timer` · `spawn` · `now` 를 한 묶음 — 메인 원문 대조) — 이것은 시계가 아니라 「비동기 런타임 추상」이다.
3. **「지금 읽기」와 「기다리기」를 가른 피어가 여럿 있다** (가능성 높음 — 「보통」이라 할 빈도 근거는 없다). zed `Clock` = `utc_now` · `now` 만, 타이머는 `scheduler.timer()` · Materialize `NowFn` = 벽시계 밀리초 클로저 하나 · quanta · mock_instant = now 만. 부분 예외 = Firecracker `MockClock` + `MockTimer`(비차단 `timerfd` 읽기를 흉내 낸다 — 막는 sleep 이 아니다 · 적대 리뷰 정정).
4. **단조 시간과 벽시계는 자주 따로다** — zed 는 한 trait 에 둘 다 · Materialize 는 벽시계만 · TiKV · Firecracker 는 함수 · 타입으로 가른다.
5. **sans-IO 대안** — quinn-proto 는 매 호출에 `now: Instant` 를 인자로 받는다(trait 없음 · 검색 결과만 — 불확실).
6. **tokio 가짜 시간의 함정** — `tokio::time::pause` 는 `tokio::time::Instant` 만 멈추고 `std::time::Instant` 는 계속 간다(tokio 문서). 섞으면 결정성이 조용히 깨진다.

## 우리 네 벌에 비추면

| 우리 seam | 모양 | 피어 형 | 합치기 |
|---|---|---|---|
| 데몬 `command_delivery` `Clock` | `now()` | zed `Clock` · quanta | base `time` 의 「지금 읽기」로 |
| discovery `Clock` | `now()` + 막는 `sleep` | Firecracker(부분) | 「지금 읽기」만 base 로 · 막는 sleep 은 discovery 에 남긴다 |
| transport `Clock` | `now()` + 비동기 `sleep`(tokio) | quinn `Runtime`(런타임 추상) · 나머지 피어는 런타임이 쥔다 | 「지금 읽기」만 base 로 · 비동기 sleep 은 transport 에 남긴다(base 가 tokio 를 끌지 않게) |
| 데몬 `UsageClock` | `mono()` + `wall()` | zed `Clock`(mono + wall) | 「지금 읽기」(단조) + 벽시계 밀리초(`now_epoch_ms`)로 표현 |

**함의(적대 리뷰 뒤 정정):** 피어 증거는 「어디까지 합치나」의 보편 경계를 세우지 못한다 — 갈라 둔 피어도 묶어 둔 피어도 있다. 「지금 읽기만 base · 기다리기는 쓰는 쪽」은 **피어에서 도출한 결론이 아니라 설계 선택**이고, 그렇게 가려면 아래 넷을 먼저 정해야 한다(적대 리뷰 · 코드 대조):

- **가짜 시간 상태 공유** — transport 의 수동 시계(`crates/engram-dashboard-transport/src/testing.rs`)와 discovery 의 가짜(`lib.rs` 의 시험 시계)는 sleep 이 `now` 를 앞으로 민다. 둘을 갈라도 가짜 하나가 두 seam 을 같은 상태로 구현해야 한다(예: 쓰는 쪽 sleep trait 이 base `Clock` 을 상위 trait 으로 둔다).
- **`UsageClock` 은 같은 계약이 아니다** — 「그 시계가 만들어진 뒤 경과」(원점 보유) + 부호 있는 epoch **초**(1970 이전 포함)이고 가짜가 두 축을 따로 움직인다(`usage_service/clock.rs`). `Instant` + `now_epoch_ms` 로 바로 옮겨지지 않는다.
- **tokio 시간 영역** — transport 는 `std::time::Instant` 를 돌려주면서 tokio 로 잔다(`clock.rs`). `tokio::time::pause` 는 std `Instant` 를 안 멈춘다 — 시험 전략이 수동 시계라 지금은 문제없지만, 합칠 때 시계 영역을 적어야 한다.
- **trait 과 구현의 분리** — transport trait 시그니처는 `Duration` · `BoxFuture` 만 쓰고 tokio 는 구현에만 있다. 「통째로 합치면 base 가 tokio 를 끈다」는 구현까지 옮길 때의 말이다(적대 리뷰 정정).

## 적대 리뷰

cross-family(codex · effort high · web_search) 1회 → **BLOCK**(결론이 증거를 넘었다). 반영: 빈도 주장 강등 · 반례 둘 추가 · Firecracker 정정 · 함의를 「설계 선택 + 먼저 정할 넷」으로 다시 썼다. 스팟 재검증 통과: zed `scheduler::Clock`(`utc_now` · `now` 만) · Materialize `NowFn`(epoch 밀리초 `u64`) · quinn `Runtime`(`now` + `new_timer`) · tokio `pause` 문서. 리뷰어 판단: 「지금 읽기만 합치고 기다리기는 쓰는 쪽」은 피어 결론으로는 아니고, 위 넷을 정하면 가능한 설계다.

## 한계 · 공백

- GitHub 코드 검색이 도중에 한도에 걸렸다 — deno · nushell · wezterm · rust-analyzer · databend 는 한 문구 검색 0 건뿐이라 seam 부재를 확인한 것이 아니다. cargo · helix 는 못 봤다.
- Fuchsia API 이름 · Firecracker 모양은 요약으로만 봤다.
- zed `scheduler::Clock` 이 gpui 밖에서 얼마나 쓰이는지는 재지 않았다.
