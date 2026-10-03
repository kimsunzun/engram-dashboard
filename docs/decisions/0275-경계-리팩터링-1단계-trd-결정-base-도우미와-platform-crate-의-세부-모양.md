# ADR-0275: 경계 리팩터링 1단계 TRD 결정 — base 도우미와 platform crate 의 세부 모양

- 상태: 확정 (2026-10-04, 근거: 사용자 위임 2026-10-04 (「알아서 진행해」 → 1단계 TRD 권고안 채택) + 결정 11–12 는 메인 판단(TRD 1-3 이 선택지로 올리지 않은 공통 항목) + 결정 15 는 사용자 「알아서」 2026-10-03 → 메인 권고안 + TRD 실측 `0ef6292` + `/review trd full` 3라운드) — D3 · 결정 12 사용자 확인 대기
- 관련: Amends ADR-0269 (결정 4 복구 경고 철회와 결정 2와 3과 5의 세부와 거부한 대안 sha256_hex 사유) · Amends ADR-0266 (결정 6과 열린 것과 영향 불변식의 시한부 예외) · Amends ADR-0262 (잔여물 끝내기 종료 코드 자리) · TRD `docs/process/S21-crate-boundaries/trd-1-1-base-helpers.md`(§2 · §3 · §4 · §6) · TRD `docs/process/S21-crate-boundaries/trd-1-3-platform-crate.md`(§2-5 · §3-4 · §4 · §7 · §8) · ADR-0267(command · messaging 은 필요해질 때만 base) · ADR-0177(transport 워크스페이스 의존 0) · ADR-0231(출력 Channel 명부의 경고 + 독 걷기) · ADR-0001(kill 인과) · ADR-0175 · ADR-0270 결정 5 · ADR-0271 · `docs/refactoring/architecture-discussion-2026-09-26.md` §10 · `docs/tracking.md` T-47 · step-log S21 · Amends ADR-0268 (영향의 base 락 오염 복구 경고 줄)

## 맥락

0단계가 base 범용 도우미(ADR-0269)와 platform crate(ADR-0266)를 박으면서 세부를 1단계 착수로 미뤘다 — ADR-0269 결정 2 「함수 이름은 가안」 · 결정 3 「세부 모양은 1-1 착수 때」 · 결정 3-4 「`UsageClock` 은 맞으면 합치고 안 맞으면 묻는다」 · ADR-0266 「열린 것」(`windows` feature 를 나눌지)과 게이트 명령의 crate 이름 자리 `<platform>`.

1단계 TRD 두 편(1-1 base 도우미 · 1-3 platform crate)이 `0ef6292` 에서 잰 결과 ADR 의 그림과 다른 곳이 나왔다(TRD 1-1 §0 ② · TRD 1-3 §2-5):

- 「지금 읽기」 시계 seam 은 넷이 아니라 여섯이다(+ agent `LeftoverClock` · `CaptureClock`).
- transport 에 base 를 붙이면 transport 의 「워크스페이스 의존 0」 게이트(ADR-0177)가 깨진다.
- UTF-8 경계 자르기는 고정 컴파일러(1.95.0)의 std 한 줄(`floor_char_boundary` · `ceil_char_boundary`)이다.
- 가짜 시계를 base `testing` 에 두면 `crate::time::Clock` 을 구현해야 해 입주자 무참조(게이트 ③)에 걸린다.
- 락 오염 복구는 117곳(운영 90)이고 그중 116곳이 이미 말없이 되찾는다. 운영 빌드는 `panic = "abort"` 라 그 갈래에 닿지 않는다. 경고를 달면 가드를 쥔 채 찍혀 「쥔 채 로그 금지」 자물쇠를 빼야 하는데, 그 목록이 리뷰마다 넓어졌다.
- `sha256_hex` 를 뺀 사유 「`sha2` 를 끌고 온다」는 거짓이다 — std 손구현이다.
- `windows.rs` 에 claude 잔여물 정리의 도메인 상수 `LEFTOVER_EXIT_CODE` 가 있다 — 「통째로」 옮기면 ADR-0266 결정 3(도메인 지식은 남긴다)과 부딪힌다.
- ADR-0266 결정 6 「한 덩이로」 문구대로 옮기면 platform 공개 API 에 통로 개념(`RetiringSignal`) · 시험 seam 트레이트 · 트레이트 객체가 들어간다.

TRD 는 갈림길을 1-1 에 열하나(C1–C5 · T1 · T2 · S1 · S2 · N1 · P1) · 1-3 에 셋(D1–D3) 사용자 선택으로 올렸다. ★**사용자는 고르기를 위임했다(2026-10-04 「알아서 진행해」). 아래 결정은 결정 11 · 12 · 15 를 빼고 전부 그 위임 아래 TRD 권고안을 그대로 채택한 것이다 — 결정과 기각의 이유는 TRD 의 것이고 사용자 본인의 판단이 아니다**★(TRD 1-1 §4 · TRD 1-3 §7 — 둘 다 위임 뒤 권고안과 글자 대조로 같음을 적었다). 결정 11–12 · 15 는 출처가 다르다(아래 문단 · 그 항목).

★**사용자 확인 대상 = D3(결정 10)과 결정 12.**★ 결정 10 만 D3 이다. 결정 11–12 는 D3 이 아니고 선택지로 올린 적도 없다 — TRD 1-3 §7 끝 「공통 — 안과 무관해 묻지 않던 것」이라 위임 범위 밖이고 메인 판단이다. 그중 결정 12 는 2026-09-29 사용자 결정의 모양을 일부 바꾼다: ADR-0262 「[구현] 고른 것」의 「windows crate 와 io 만 · 통째로 옮길 수 있게」(사용자 결정 2026-09-29)는 실질 — `windows.rs` 가 `windows` crate 와 io 만 쓰는 잎이다 — 이 그대로이고, 바뀌는 것은 「통째로」 중 상수 하나(`LEFTOVER_EXIT_CODE`)가 인자로 나가는 것이다. D3 은 위임 아래, 결정 12 는 메인 판단으로 내린 결정이라 둘 다 사용자 본인의 확인을 거치지 않았다.

## 결정

**base 도우미 — ADR-0269 · ADR-0268 을 고친다**(정본 = TRD 1-1 §4-2 표):

1. **`sync` = 경고 없이 되찾기만**(S2=(e) · ADR-0269 결정 4 개정 · ADR-0268 「영향」의 base 락 오염 복구 경고 줄 개정). 자유 함수 넷(`lock` · `read` · `write` · `wait_timeout`)이고 몸은 각각 `unwrap_or_else(PoisonError::into_inner)` 한 줄이다. 독 표시는 남기고(`clear_poison` 을 부르지 않는다) 로그 · 다른 락 · 밖 호출을 하지 않는다 — 도우미가 더하는 동작이 없어 「쥔 채 로그 · IO · 밖 호출 금지」 자물쇠의 사이트를 옮겨도 그 자물쇠들의 규율이 바뀌지 않는다(무엇을 언제 잡는지는 부르는 쪽 그대로). 예외 하나 = 셸 `output_channel.rs`(ADR-0231 의 경고 + 독 걷기 — 제자리).
2. **`sync` 흡수 범위 = 운영 83곳**(ADR-0269 결정 2 표 `sync` 행 · 「근거」의 57곳 — 「1-1 착수 때 다시 센다」의 답). 실측 117곳 = 운영 90 + 시험 전용 27. 제자리 = 운영 7(base `logging` · `leftover.rs` 의 `try_lock` · 찾기 명단 도우미 · 셸 `settings/mod.rs` 3 — storage P3 뒤 · 셸 `output_channel.rs`) · command 3(ADR-0267) · 시험 전용 24.
3. **transport 몫은 3단계(transport 부착 TRD)로 미룬다**(C5=(b) · ADR-0269 결정 3-2 · 3-5 의 transport 몫). 1-1 에서 transport 는 base 를 의존하지 않고 transport 게이트 ①(정확히 1줄 · ADR-0177)도 그대로다. 결정 3-2 의 「가짜 하나가 둘을 같은 상태로 구현」은 discovery 몫만 1-1 에 선다. 결정 3-5 는 사실로 남고 적용은 3단계다. 대가 = transport 시계 seam 이 사용자 규칙 「시간은 공용이면 무조건 합친다」(2026-09-26)와 어긋난 채 3단계까지 남는다 — transport `clock.rs` 머리 주석에 적는다(TRD 1-1 §4-1 C5).
4. **`text` = `hex_lower` 하나**(T1=(b) · T2=(a) · ADR-0269 결정 2 표 `text` 행). `truncate_bytes` · `keep_tail_bytes` 는 만들지 않는다 — UTF-8 자르기 사본 셋은 std `floor_char_boundary` · `ceil_char_boundary` 로 제자리 교체한다. hex 루프 셋은 감싼 함수(토큰 생성기 둘 · `sha256_hex`)를 제자리에 두고 안의 루프만 `hex_lower` 를 부른다.
5. **가짜 시계 `ManualClock` 은 `time` 안 `#[cfg(any(test, feature = "test-support"))]` 뒤에 둔다**(C4=(a) · N1=(b) · ADR-0269 결정 3-1 · 결정 5 · 결정 2 표 `testing` 행). `testing` 은 `wait_until` 하나다. 기능 이름 = `test-support`(command · transport 와 같은 뜻 · 같은 이름).
6. **시계 seam 은 여섯이다 — `LeftoverClock` 을 base `Clock` 의 하위 트레이트로 합친다**(C2=(a) · ADR-0269 결정 3 의 seam 목록 · 「맥락」 표의 「네 벌」). `mono_now` → `now` · 가짜는 원자 정수 그대로 · 별도 단위(U6). `CaptureClock` 은 지금 읽기가 없어 합칠 몫이 없다. storage 브랜치의 saver `Clock` 은 P3 뒤의 몫이다.
7. **`UsageClock` 은 그대로 둔다**(C3=(a) · ADR-0269 결정 3-4 의 「안 맞으면 묻는다」에 대한 답 — 맞지 않음을 실측으로 확인했다). 사유는 `usage_service/clock.rs` 머리에 적는다.
8. **`sha256_hex` 의 제외 사유를 고친다**(ADR-0269 「거부한 대안」 `sha256_hex` 줄 — 사실 정정). 「`sha2` 를 끌고 온다」 → 「사본이 하나라 결정 7 에 못 미친다」. 결론(입주 안 함)은 그대로다.
9. **세부 확정**(개정 아님 — ADR-0269 결정 2 「가안」 · 결정 3 「1-1 착수 때」를 채운다): `Clock: Send + Sync`(C1=(a)) · `sync` = 자유 함수(S1=(a)) · 이름 `hex_lower` · `normalize_spelling`(P1=(a)) 확정 · `wait_until` 흡수 대상은 9개 파일이 아니라 사본 11벌.

**platform crate — ADR-0266 · ADR-0262 를 고친다**(정본 = TRD 1-3 §8):

10. **OS 층만 platform 으로 간다**(D3=B · ADR-0266 결정 6 「한 덩이로」 개정). platform = `windows.rs`(잎 — `LEFTOVER_EXIT_CODE` 만 뺀다) + 그 겉의 강한 주인 `GroupOwner` · 약한 손잡이 `GroupRef` · 구체 값 타입 다섯(이름 가안). 공개 API 에 트레이트 객체가 없다. 중립 손잡이 `ProcessGroup` · `RetiringSignal` · 시험 seam 트레이트(`Pinned` · `Births`)는 agent 에 `cfg` 없는 어댑터로 남는다 — `agent/src/transport/process_group.rs`(손잡이를 내주고 물러남 칸을 쥔 것이 통로라서 통로 밑이다). agent `platform/` 폴더는 사라진다. 결정 6 하위 항목의 「약한 손잡이 `Weak<JobObjectHandle>`」는 「platform `GroupRef`(agent `ProcessGroup` 이 감싼다)」로 읽는다. 결정 6 의 실질(강한 주인 = 통로 · 밖으로는 약한 손잡이 · 읽기 전용 표시 · 한 번 붙이는 포트 · kill 인과 무변경)은 그대로다.
11. **새 손잡이를 짓는다**(선택지로 올리지 않은 공통 항목 · 메인 판단 · ADR-0266 결정 6 「새 손잡이를 따로 짓지 않는다」 개정). 강한 주인 `GroupOwner` 는 `Clone` 이 아니다 — 타입이 강제하는 것은 「무리당 강한 주인 하나」까지이고, 「그 주인이 통로다」는 배치가 지킨다(사용량 조회의 트리 손잡이도 자기 무리의 주인이다). Windows 밖은 아무것도 안 하는 주인이다. pty · stdio · codex 통로의 `#[cfg(windows)]` Job 칸은 `GroupOwner` 뒤로 들어간다. 약한 손잡이는 통로가 `downgrade()` 의 `GroupRef` 에 `RetiringSignal::of(&self.retiring)` 를 얹어 agent 에서 만든다.
12. **`LEFTOVER_EXIT_CODE` 는 agent 로 돌아가고 `terminate_raw` 는 끝 코드를 인자로 받는다**(선택지로 올리지 않은 공통 항목 · 메인 판단 · 사용자 확인 대기 · ADR-0262 「[구현] 고른 것」의 「통째로 옮길 수 있게」와 ADR-0266 결정 3 의 충돌 — 결정 3 이 이긴다). 상수 `0x7440`(ADR-0262 결정 6)은 agent 잔여물 정리 쪽에 둔다. 어댑터가 넘길지 seam 트레이트에 인자를 붙여 leftover 가 넘길지는 1-3 U3 이 정한다(TRD 1-3 §9).
13. **`windows` 바인딩을 기능별 cargo feature 로 나누지 않는다**(D1=(a) · ADR-0266 「열린 것」을 닫는다). platform 의 `windows` 의존 하나에 합집합 feature 이고, platform 의 cargo feature 는 `test-support` 하나다.
14. **crate 이름 = `engram-dashboard-platform`**(D2 · ADR-0266 「영향」 게이트 명령의 `<platform>` 자리).
15. **시한부 예외 하나 — 셸 `src-tauri/src/fsutil.rs:107` 의 `cfg!(windows)`**(ADR-0266 「영향」 불변식 · 결정 8 의 시점). 원자적 쓰기 통일은 1-3 안이지만 storage P3 가 master 에 착지한 뒤로 가고, 그때까지 이 줄이 알려진 운영 예외로 남아 불변식 게이트의 기대 명단에 든다. ★출처는 D1–D3 이 아니라 TRD 1-3 의 범위 결정(사용자 「알아서」 2026-10-03 → 메인 권고안)이다★.

## 거부한 대안

자평 규칙 = 기각 근거가 정량 · 실측 · 코드 중 하나에 걸리지 않으면 **약함**.

- **S2 (a) 복구마다 경고(독은 둔다)** — ADR-0269 결정 4 의 문구 그대로. 기각 = 경고가 가드를 쥔 채 찍혀 「쥔 채 로그 · IO · 밖 호출 금지」 자물쇠 열둘(복구 37곳)을 빼야 했고, 그 목록이 리뷰마다 넓어졌으며(1판 둘 → 1차 +3 → 2차 +5 와 중첩 규칙으로 +1 → 3차 +1) 지키는 게이트가 없다 · 운영 빌드는 `panic = "abort"`(`Cargo.toml:34-35`)라 신호가 debug · 시험 빌드뿐이다 · 117곳 중 116곳이 이미 말없이 되찾는다(TRD 1-1 §2-4).
- **S2 (b) 독 한 번에 경고 한 번(`clear_poison`).** 기각 = 같은 자물쇠를 `.expect` · `try_lock` 으로 쓰는 이웃이 일곱 자리 있다(TRD 1-1 §2-4 표 — 예: PTY · stdio `child` 의 `shutdown()` `.expect` 는 ADR-0001 kill 인과 경로). 걷으면 그 이웃의 동작이 「그 사이 복구가 돌았나」에 매인다.
- **S2 (c) 사이트마다 한 번(매크로) · (d) 프로세스당 처음 한 번(정적 깃발).** 기각 = 결정 2 의 「상태 없는 공개 함수」에서 벗어나고, 경고를 다는 갈래라 위 제외 목록(37곳)이 그대로 필요하다 · (d) 는 둘째 독부터 기본 레벨에서 안 보인다.
- **「놓고 경고하고 다시 잡는」 도우미.** 기각 = 그 자물쇠 자신은 비우지만 부른 쪽이 쥔 바깥 자물쇠 아래서는 여전히 찍힌다(예: codex `Announcer.order` → 상태 락 — `backend/codex/transport.rs:1112-1115`).
- **C5 (a) transport 에 지금 base 를 붙인다 · (c) 시계만 붙이고 자르기는 std.** 기각 = transport 게이트 ①(정확히 1줄 · ADR-0177)을 2줄로 고치는 개정이 따르고, 소비자 0 인 재사용 lib 에 `tracing` · `tracing-subscriber` · `regex`(1-3 전이면 `windows` 까지)가 실린다 · 3단계가 의존 · 게이트를 어차피 다시 연다(ADR-0267 과 같은 결) · (c) 는 그 대가를 그대로 지고 얻는 것이 시계 하나다.
- **T1 (a) base `text::truncate_bytes` · `keep_tail_bytes`.** 기각 = 몸이 고정 컴파일러 1.95.0 의 std 한 줄이라 감쌀 몫이 없다(스크래치 컴파일 실측 — `"가나다"` 에서 4 → 3 · 6 · ADR-0269 가 `dunce::canonicalize` 를 뺀 사유와 같다) · 뒤 남김은 사본이 하나라 결정 7 미달이다.
- **T2 (b) `hex_lower` 를 1-1 에서 뺀다.** 기각 = 같은 루프 셋이 남고 결정 2 표와 어긋난다.
- **C1 (b) `Clock` 에 경계를 두지 않는다.** 기각 = 쓰는 자리마다 경계를 적어야 하고 경계 없는 `dyn Clock` 이 생길 수 있다. 고른 쪽의 사실 = 기존 seam 대다수(daemon · leftover · transport)가 이미 `Send + Sync` 이고 바뀌는 것은 discovery 시험 가짜 하나다. **기각 근거 자평: 약함**(서술 — 코드 사실은 고른 쪽의 비용이 작다는 것이지 (b) 의 해를 보이지 않는다).
- **C1 (c) 읽기 둘(`now` + 벽시계 ms).** 기각 = 결정 3-1(「지금 읽기 하나」)을 고치는데 1-1 의 소비자는 벽시계를 안 쓴다.
- **C2 (b) `LeftoverClock` 을 둔다.** 기각 = 사용자 규칙 「시간은 공용이면 무조건 합친다」(2026-09-26)에 어긋난 seam 이 남는다 · 바뀌는 것은 상위 트레이트와 이름(`mono_now` 22곳)뿐이다.
- **C3 (b) `UsageClock` 에 상위 트레이트만 붙인다 · (c) `mono` 를 `Instant` 로.** 기각 = (b) 는 아무도 `now()` 를 안 부르는 껍데기 합치기다(쓰임은 `.mono()` 11 · `.wall()` 13) · (c) 는 ADR-0250 · 사용량 TRD 의 계약 변경이고 `Duration::MAX` · `ZERO` 끝값 시험(`usage_service/book.rs`)을 다시 써야 해 1-1 범위를 넘는다.
- **C4 (b) 가짜를 `testing` 에 두고 게이트 ③ 에 예외를 단다.** 기각 = 게이트에 예외가 생기고 정규식이 길어진다. **기각 근거 자평: 약함**(서술 — 예외가 무엇을 놓치게 하는지는 재지 않았다).
- **C4 (c) 가짜를 base 에 두지 않는다.** 기각 = 결정 3-1 · 5 를 고치고 daemon 의 `Mutex<Instant>` 가짜(`command_delivery.rs:2425`)가 남는다.
- **S1 (b) 확장 트레이트 `m.lock_or_recover()`.** 기각 = 파일마다 `use` 한 줄 · std `lock` 과 다른 이름 · ADR-0269 「거부한 대안」(도우미를 트레이트 뒤에)과 결이 달라 설명이 필요하다. **기각 근거 자평: 약함**(서술 — 고른 쪽의 선례 `usage/process.rs:636` 만 코드 사실이다).
- **N1 기능 이름 (a) `testing` · (c) `test-harness`.** 기각 = (a) 는 기존 선례(command · transport 의 `test-support`)와 갈린다 · (c) 는 agent · daemon · messaging 이 「운영 코드 안의 seam 을 연다」는 다른 뜻으로 쓴다.
- **P1 (b) `normalize_typed_path` · (c) `tidy_typed_path`.** 기각 = ADR-0269 결정 2 · ADR-0270 결정 5 가 이미 적은 가안과 어긋난다. **기각 근거 자평: 약함**(서술 — 이름 고르기).
- **D3 A — 결정 6 문구대로 `windows.rs` + `process_group.rs` 를 통째로 platform 으로.** 품은 가장 작다. 기각 = platform 공개 API 에 통로 개념 `RetiringSignal`(통로의 물러남 칸 읽기 — OS 와 무관) · 시험 seam 트레이트 · 트레이트 객체(`Box<dyn Pinned>` · `Arc<dyn Births>`)가 들어가고 `ProcessGroup::detached` 를 `test-support` 공개 생성자로 열어야 한다 — ADR-0266 결정 2(판정 = OS 의존) · 결정 5(가짜용 트레이트는 쓰는 쪽)에 어긋난다. A 도 `GroupOwner` 를 새로 지어야 해 「새 손잡이를 따로 짓지 않는다」 문구는 어느 쪽이든 고쳐야 했다(TRD 1-3 §3-4 「왜 A 가 아닌가」).
- **`GroupOwner` 를 「`JobObjectHandle` 의 겉만 바꾼 것」으로 읽어 결정 6 을 개정 없이 둔다.** 기각 = 공개 타입의 이름 · 모양 · 내부(`Arc` 를 안으로 넣는다)가 바뀐다.
- **`windows.rs` 를 상수째 통째로 옮긴다.** 기각 = 도메인 값(`LEFTOVER_EXIT_CODE` — `windows.rs:53` · `terminate_raw` 가 박아 쓴다 `:405-406`)이 platform 에 들어가 ADR-0266 결정 3 을 어긴다.
- **D1 (b) 기능별 feature(`process` · `group` · `file-holders` · `spawn` …) · 기본 비움 · (c) 지금은 (a), 조건이 서면 (b).** 기각 = (b) 는 platform 안 `#[cfg(feature = …)]` · CI 기능 조합 게이트(net 5a · 5b 꼴) · 기능별로 갈리는 시험을 늘리고, 얻는 것은 `-p` 단독 빌드의 컴파일 시간뿐이다(워크스페이스 빌드에선 feature 가 합쳐지고 agent 가 전부 켠다 — ADR-0175 옵션 C 와 같은 관찰) · 그 이득을 볼 「PID 만 쓰는 소비자」 net · discovery 가 3-3 · 2-2 에서 사라진다 · (c) 는 지켜볼 대상이 남지 않는다. **기각 근거 자평: 약함** — 컴파일 비용은 재지 않았고(TRD 1-3 §9), net 이 사라지는 3-3(결정 후보 7)은 ADR 이 아니라 transport TRD 로 미룬 계획이다.
- **D2 `engram-dashboard-os` · `engram-dashboard-sys`.** 기각 = `-os` 는 `std::os` 와 뜻이 겹쳐 「std 확장」으로 읽힐 수 있다 · `-sys` 는 ADR-0175 가 기각한 이름이다(Rust 에서 `-sys` 는 C FFI 바인딩 crate 관례). **기각 근거 자평: 약함**(서술 · 관례 — 이름 고르기).
- **`fsutil.rs` 의 분기를 지금 platform 으로 옮긴다.** 기각 = storage P3 가 같은 파일을 크게 바꾼다(`write_atomic_unless` · `copy_atomic` · `copy_aside` · `retry_denied` 신설 — `origin/v0.3.3/feat/storage` 대조) · P3 앞에서 옮기면 P3 착지 때 같은 파일을 다시 맞춰야 한다.

## 근거

- **결정 출처 = 사용자 위임(2026-10-04 「알아서 진행해」) → TRD 권고안 채택.** TRD 1-1 §4(열하나 — 4판 권고와 같은 글자) · TRD 1-3 §7(셋 — 3판 권고와 같은 글자). 결정 11–12 는 TRD 1-3 §7 끝 「공통 — 안과 무관해 묻지 않던 것」이다 — 선택지로 올리지 않아 위임 범위 밖이고 메인 판단이다. 결정 15 는 TRD 1-3 의 범위 결정(사용자 「알아서」 2026-10-03 → 메인 권고안 · TRD 1-3 §1)이다. ADR-0266 결정 6 자체도 처음부터 메인 판단(사용자 위임)이었다(TRD 1-3 §8).
- **「복구할 때 경고」는 사용자 결정이 아니었다** — 메모 §1 후보 4 의 락 오염 복구 줄(`docs/refactoring/architecture-discussion-2026-09-26.md`)에서 사용자 태그(2026-09-26)가 덮는 것은 「base `sync` 모듈로 모은다」뿐이고, 「복구할 때 경고 로그를 남기는」 문구는 메인이 덧붙인 풀이다. 그 풀이가 기댄 「후보 3 과 같은 결」의 후보 3 은 사용자가 번복했다(2026-10-03 「로그 감싸지 마」 · ADR-0268). 그래서 S2=(e) 는 사용자의 명시 결정을 뒤집지 않는다.
- **실측(`0ef6292`, 2026-10-03 · 5판 재대조 2026-10-04)** — 명령 전문은 TRD 1-1 §10 · TRD 1-3 §2. 요지: 락 오염 복구 117곳(운영/시험 가름은 스크립트 추정 + 손 정정) · `panic = "abort"`(`Cargo.toml:34-35`) · std 경계 함수 컴파일(rustc 1.95.0 · 출력 `3 6`) · `grep -n sha2 crates/*/Cargo.toml src-tauri/Cargo.toml` → 0줄 · 시계 트레이트 `rg` 여섯 · `mono_now` 22곳 · `UsageClock` `.mono()` 11 · `.wall()` 13 · `LEFTOVER_EXIT_CODE`(`agent/src/platform/windows.rs:53`).
- **리뷰 = `/review trd full` 3라운드** — 지적마다 코드로 다시 확인한 뒤 TRD 를 고쳤다(각 TRD 머리의 개정 기록). 1차 = 1-1 S2 권고 (b) → (a)(섞인 자물쇠 일곱) · 1-3 불변식 게이트 정규식 결함 셋 · `GroupOwner` 신설은 어느 안이든 결정 6 개정. 2차 = 1-1 제외 자물쇠 다섯 + 중첩 규칙 · 운영/시험 재분류 · 1-3 약한 손잡이에 물러남 표시를 얹는 경로 · discovery 의 base 의존 유지. 3차(1-1) = 래치 commit 포트 `expected` 칸 제외 → 갈래 (e) 추가 · 권고 (a) → (e).
- **고칠 글이 없는 것(확인함)** — ADR-0269 결정 1 · 6 · 7 · 8 · 9 · 「영향」 · ADR-0177 · ADR-0231(base `sync` 의 예외로 그대로 선다) · ADR-0175 · ADR-0218(TRD 1-1 §4-2 · TRD 1-3 §8).
- **경로 · 사실만 낡는 것 — 개정이 아니다**(옛 ADR 본문은 고치지 않는다 · TRD 1-3 §8): ADR-0262 「OS 조각」 · 「[구현] 고른 것」의 `platform/windows.rs` · `process_group.rs` → platform `group/windows.rs` · agent `transport/process_group.rs` · ADR-0230 「관련」 · 「영향」의 `backend/mod.rs:44`(`console_command`) → platform `shell::console_command`(이름 가안) · ADR-0218 「관련」의 `crates/engram-dashboard-agent/src/platform` → platform crate · ADR-0266 「맥락」 재실측 목록에 「창 없이 띄우기」(`CREATE_NO_WINDOW`) 세 벌이 없다 · ADR-0266 「영향」의 「`windows` feature 목록이 crate 별로 옮겨 간다」 — agent 는 예제 둘 때문에 dev-dependency 로 남긴다 · ADR-0266 결정 6 의 `Weak<JobObjectHandle>` 줄은 `117-118`.
- **미검** — S2=(e) 를 돌려 보지 않았다(83곳이 컴파일된다는 것 · `output_channel.rs` 시험이 초록으로 남는다는 것은 읽기) · 워크스페이스 출발 수치(1-1 U1 · 1-3 U0 착수 때 잰다) · `windows` feature 를 안 쪼갤 때의 컴파일 비용 · agent 어댑터 약 100줄은 어림 · 「쥔 채 로그 금지」 감사(4판까지)의 완전성.

## 영향 / 불변식

- **`sync` 계약**(모듈 머리에 `// ADR-0269` · `// ADR-0275`) — 독을 걷지 않는다 · 로그 · 다른 락 · 밖 호출을 하지 않는다 · 운영 빌드는 이 갈래에 닿지 않는다. ★**경고를 다시 달고 싶어지면 83곳 전체의 「쥔 채 로그 금지」 감사를 처음부터 다시 한다**★ — 도우미가 찍기 시작하면 그 자물쇠들의 규율이 말없이 깨진다.
- **base 게이트 ③ 의 이름 알파벳**은 모듈을 만드는 단위에서 늘린다(`text|time|path` → `+sync` → `+testing`)고, 1-3 이 `platform` 을 뺀다. `time::ManualClock` 은 `testing` 과 같은 기능 뒤라 「시험 기능이 운영 그래프에 없다」 게이트(base = TRD 1-1 §7 G2 · platform = TRD 1-3 §4-2 ③)가 함께 덮는다.
- **transport 는 1-1 뒤에도 base 를 의존하지 않는다** — `ws.rs` 만 std 로 바뀐다. transport `clock.rs` 머리에는 「3단계(transport 부착)에서 base `time::Clock` 의 하위 트레이트로 — ADR-0275」를, daemon `usage_service/clock.rs` 머리에만 「합치지 않는다 — ADR-0275」 사유를 적는다.
- **discovery 는 1-3 뒤에도 base 를 쓴다** — 1-1 이 discovery `Clock` 을 base `time::Clock` 의 하위 트레이트로 만든다. 1-3 은 그 의존을 걷지 않는다.
- **kill 인과(ADR-0001) 순서 그대로** — `shutdown()` 의 `child.kill + wait` → `group.terminate(1)` → master drop. 새 Job 칸도 구조체의 **마지막 필드**에 둔다(drop 때 `KILL_ON_JOB_CLOSE` 가 도는 시점 유지 — 재는 시험이 없어 리뷰가 지킨다). 이 단위(1-3 U3)는 `/qa full` 이다.
- **잔여물 정리 락 규칙(ADR-0262 결정 8)** — platform 의 `terminate_raw` · `classify` · `is_gone` 은 로그도 락도 쓰지 않고, `terminate_raw` 는 끝 코드를 인자로 받아도 OS 호출 정확히 하나로 남는다. `GroupRef` 는 Job 을 붙들지 않는다.
- ★**약한 손잡이에서 물러남 표시를 빼지 말 것**★ — `downgrade()` 만 내주면 잔여물 정리의 듣는 스레드와 끝내기 앞 재확인이 멈춤 조건을 잃는다.
- **CLAUDE.md 「핵심 불변식」의 `ProcessGroup` · `RetiringSignal` 은 이름이 그대로**이고 파일 자리만 agent `transport/process_group.rs` 로 바뀐다.
- **착수 순서** — 이 ADR 은 1-1 U1 · 1-3 U0 전에 박는다 · 1-3 은 1-1 머지 뒤 · 원자적 쓰기 통일과 손상 사본 치우기 통일은 storage P3 착지 뒤.
- **코드 앵커 = `// ADR-0275`** — base `sync` 모듈 머리 · transport `clock.rs` 머리 · daemon `usage_service/clock.rs` 머리 · platform `GroupOwner` · `GroupRef` · agent `transport/process_group.rs` 머리(지금의 `// ADR-0262` 와 함께 — TRD 1-3 §6 11).
