# TRD — 경계 리팩터링 1-1: base 범용 도우미 (S21)

> 상태: **5판 — 결정 반영(2026-10-04).** 코드는 아직 한 줄도 바뀌지 않았다. §4 의 선택 열하나는 전부 정해졌다 — **사용자 위임(2026-10-04 「알아서 진행해」) → 4판 권고안 채택.** 확정된 ADR-0269 · ADR-0268 과 어긋나는 몫은 ADR-0275 로 고쳤다(2026-10-04 · §4-2 — 0274 는 storage 브랜치가 쓴다). 구현은 §6 의 U1 부터(착수 체크리스트 §6-1).
> **개정 기록 — 2026-10-04 · 5판 — 결정 반영:**
> - §4 를 「사용자 선택 대기」에서 「결정」 기록으로 바꿨다 — 고른 갈래가 전부 4판 권고와 같다(C5 (b) · T1 (b) · T2 (a) · C1 (a) · C2 (a) · C3 (a) · C4 (a) · S1 (a) · S2 (e) · N1 (b) · P1 (a)). 갈래별 긴 논의는 걷고 고른 것 · 이유 한 줄 · 거부한 갈래 한 줄씩만 남겼다(거부한 갈래는 ADR-0275 「거부한 대안」의 재료다).
> - **S2=(e) 로 이전 규칙이 바뀌었다** — 쥔 채 로그 금지 자물쇠 열둘(37곳)의 제외 목록은 더 이상 이전 규칙이 아니다. §2-4 에 「(a) 를 고르지 않은 이유」로 짧게 남기고, 단위는 운영 83곳을 옮긴다(제자리 7 — §3-5). U2 13 → **17** · U3a 17 → **24** · U3b 16 → **42**(§5 M3 · §6). codex stdin 소스 개수 시험의 지시(기대값 2 그대로 · matcher 만 넓힌다)는 그대로다.
> - **T1=(b)** — `text` 에서 자르기 두 함수를 뺐다. 사본 셋은 U1 에서 std `floor_char_boundary` · `ceil_char_boundary` 로 제자리 교체한다(§3-2). **C4=(a)** — 가짜 시계 `ManualClock` 은 `time` 안 `test-support` 기능 뒤다(§3-3).
> - 고르지 않은 갈래에 매인 조건문(「S2=(b) 면 …」 · 「C5=a 일 때만」 · 「T1=b 면」 등)을 걷거나 각주로 줄였다. G3(transport 의존 상한 1 → 2)은 없어졌다(§7).
> - §4-2(ADR-0275 에 적을 개정)와 §6-1(U1 착수 체크리스트)을 더했다.
> - (e) 로 넓어진 파일(잔여물 정리 · 대기 입력 명부 · 대기 목록 표 · 래치 · 턴 관측 표 · codex 상태 락 · daemon 넷)을 소스 문자열 시험과 다시 맞댔다 — 깨지는 것은 여전히 codex stdin 개수 시험 하나다(§6 공통 검증 5).
> - ★4판은 커밋된 적이 없다(이 폴더는 미추적)★ — 이 판에서 걷은 상세(제외 감사의 찾은 법 · 자물쇠별 문서 인용 전문 · 중첩 전수 점검 · 교착 판정)는 요약만 남는다(§2-4).
>
> **개정 기록 — 2026-10-03 · 3차 리뷰 반영** (지적마다 코드로 다시 확인한 뒤 고쳤다):
> - **중첩 규칙이 놓친 자리 하나를 더 뺐다 — 첫 제출 래치의 commit 포트 `expected` 칸(`manager.rs:550`).** 래치가 가드를 쥔 채 그 포트를 부른다(`session_id_latch.rs:141-145`) — 문서의 락 순서 「래치 → 포트의 expected 칸 → profiles → store write_lock」(`session_id_latch.rs:23-25` · `manager.rs:523-524`). 제외 자물쇠 열하나 · 36곳 → **열둘 · 37곳** · agent 이전분 34 → **33** · U3a 18 → **17** · M3 47/48 → **46/47**(§2-4 · §5 M3 · §6).
> - **중첩 점검을 전수로 다시 했다** — 제외 자물쇠 열둘마다 문서의 락 순서(`rg '락 순서|lock order'` 33줄 + CLAUDE.md 「락 순서」)와 가드를 쥔 채 부르는 콜백 · 포트를 다 읽었다. 더 걸린 것은 없다. 단 codex 머리의 「상태 → 대기표」(`transport.rs:21`)는 문서가 허용만 하고 실제로 그렇게 잡는 운영 자리가 없다 — 옮기는 쪽에 두고 잠복 간선으로 적었다(§2-4 · §10).
> - 래치 항목의 「`drop(state)` 뒤에 로그」는 래치 **자기** 로그에만 참이다 — commit 포트가 래치를 쥔 채 이미 `warn!` · `info!` · `debug!` 를 낸다(`manager.rs:541-569`). 고쳐 적고 §9 에 올렸다.
> - 낡은 숫자를 맞췄다 — §4 「묶인 것」의 「48 대 49」 · S2 (a) 의 「그 밖의 48곳」 · 「제외 35곳」(§0 · §1-1 · §2-4 · §3-5 · §4 · §5 · §6 · §7 을 grep 으로 다시 셌다).
> - 1-3 U1 이 discovery `Cargo.toml:12-14` 의 「liveness 공유」 사유를 platform 줄로 옮기면 base 줄에 새 사유를 적는 단위가 없었다 — U5 의 손댈 목록에 더했다.
> - **S2 에 갈래 (e)「경고 없이 되찾기만」을 더하고 권고를 (a) → (e) 로 바꿨다(워커 제안 — 결정 아님).** (e) 면 제외 목록이 필요 없고 이전 범위가 46/47 → **83** 이다. 대가 = 확정된 ADR-0269 결정 4 의 개정(§4 S2 · §7).
>
> **개정 기록 — 2026-10-03 · 2차 리뷰 반영** (지적마다 코드로 다시 확인한 뒤 고쳤다):
> - **경고 제외 규칙을 넓은 문구로 다시 훑었다** — 1판 · 2판의 검색어(`쥔 채.{0,40}(로그|IO|emit)|로그도 놓은|잎이다`)가 같은 규율의 다른 표현을 놓쳤다. 새로 든 자물쇠 다섯: codex 상태 락(복구 26곳 — `transport.rs:3012-3015` 「★상태 락을 쥔 채 찍지 않는다★」) · 턴 관측 표(`turn.rs:18` 「잡은 채 외부를 부르지 않는다」 — 검색어가 아니라 사이트별 문서 읽기로 찾았다) · daemon 명령 명부 · 사용량 출구 칸 · 사용량 구독 명부(§2-4). 제외는 그대로 둔다(옛 꼴 · 경고 없음) — **메인 판단 2026-10-03**. 「놓고 경고하고 다시 잡는」 도우미는 거부했다(§4 S2).
> - **운영/시험 가름을 바로잡았다** — `#[cfg(test)]` 함수 둘(codex `Pending::len` `transport.rs:1348` · `input_queue.rs` `queued_bytes` `:262`)을 운영으로 셌었다. 운영 92 → 90 · 시험 전용 25 → 27.
> - 숫자를 전부 다시 셌다 — 이전 범위 80/81 → **48/49**(§5 M3) · U3b 43 → **16** · U2 daemon 8 → **5** · U3a 20 → **18** · 섞인 자물쇠 중 (b) 에 걸리는 것 여섯 → **다섯**(명령 명부가 제외로 빠졌다). S2 의 갈래 글에 제외 규칙을 적었다.
> - **사용량 책 락(`usage_service/mod.rs:606` — 책 락의 복구 사이트는 이 1곳)도 제외로 옮겼다 — 메인 판단 2026-10-03.** 구독 교체 `first_sheets`(`:281-291`)에서 제외한 구독 명부 락 안에서 잡혀, 경고가 그 바깥 락을 쥔 채 찍힌다. 제외 규칙을 「제외한 자물쇠 안에서 잡히는 자물쇠도 제외(중첩)」로 넓혔다. 제외 35 → **36** · M3 48/49 → **47/48** · U2 daemon 5 → **4**(§2-4 · §5 M3 · §6).
> - discovery 의 base 의존이 U5 뒤 실제 쓰임(`time::Clock` 상위 트레이트)을 얻는다는 것을 §3-3 · U5 에 적었다 — 1-3 이 그 의존을 걷지 않게(넘겨줌).
> - 루트 `Cargo.toml:2` · `:12-13` 의 base 서술을 U1 의 손댈 목록 · §7 문서 목록에 더했다 — base `lib.rs:15` · CLAUDE.md 와 같은 문장이라 같은 변경에서 낡는다(1-3 이 자기 몫을 다시 고친다).
>
> **개정 기록 — 2026-10-03 · 1차 리뷰(`/review trd full`) 반영** (지적마다 코드로 다시 확인한 뒤 고쳤다):
> - **S2 의 권고를 (b) → (a) 로 바꿨다(워커 제안 — 결정 아님).** 독을 걷으면(`clear_poison`) 같은 자물쇠를 `.expect` 로 쓰는 이웃이 보던 신호까지 지워진다 — 섞인 자물쇠 일곱을 §2-4 에 실었다(리뷰가 센 넷 + 이 판 실측 셋: stdio `child` · `CommandDeliveries` · stdin 의 `try_lock`). 독을 걷지 않는 변형 (d)(프로세스당 한 번)를 더했다. §6 의 「동작 그대로」는 S2 에 매인 주장으로 고쳤다.
> - 셸 `output_channel.rs` 는 이미 경고 + `clear_poison`(ADR-0231)이고 시험이 `!is_poisoned` 를 단언한다 — U2 의 처분을 S2 에 매었다((b) 가 아니면 제자리).
> - U3b 에 codex 「stdin 블로킹 락 개수」 소스 시험(기대 2)을 넓히라는 지시를 넣었다 — 호출 꼴이 바뀌면 0 이 되어 빨개진다. 소스 문자열 시험 전수를 훑은 결과도 적었다(§6).
> - 사용량 조회의 찾기 명단(`LookupGate::release`)은 1-3 U5 가 platform 으로 옮기므로 1-1 에서 제자리로 뺐다(U3a).
> - 경고 제외 규칙을 「자물쇠 문서가 쥔 채 로그 · IO · 밖 호출을 금한 자리」로 정하고 셋을 더 뺐다(`QueuedInputs` · `InputsPendingTable` · 첫 제출 래치) — 교착 판정도 함께 적었다(§2-4). 이전 범위가 85 → 80(S2=(b)면 81)으로 줄었다.
> - G1 의 게이트 ③ 사본 자리를 4 → 6 으로(`docs/testing-strategy.md` 둘), base 문서 목록에 그 파일의 base 절을 더했다.
> - C5 — (b) 가 확정된 ADR-0269 결정 3-2 를 미루는 개정이라는 것과, (a) 의 「`windows` 를 끈다」가 1-3 U1 과의 순서에 달렸다는 것을 적고 §7 개정 후보에 더했다.
> - §6 의 1-3 겹침 포인터를 고쳤다(daemon `lib.rs:109` · agent `manager.rs:77`) · 겹침 파일에 `usage/process.rs` · `leftover.rs` 를 더했다.
> **범위 = 작업 순서 1-1 중 먼저 가는 몫** — `text` · `time` · `path` · `sync` · `testing` 도우미를 base 에 들이고 흩어진 사본을 갈아 끼운다(`docs/refactoring/architecture-discussion-2026-09-26.md` §10). 손상 사본 치우기(`file`)는 storage P3 착지 뒤로 미룬다(§8).
> **읽는 법:** 각 항목의 출처를 표시한다 — **사용자 결정(날짜)** · **사용자 위임(날짜)**(사용자가 고르기를 맡겨 권고안을 채택한 것) · **메인 판단** · **워커 제안**(이 문서를 쓴 워커의 제안 — 확정이 아니다) · **실측**(돌린 명령을 함께 적는다). §4 의 열하나는 사용자 위임(2026-10-04)으로 확정이다. 본문의 「C5=b」 같은 꼬리표는 그 결정을 가리킨다.
> **문서 배치 규약:** `docs/README.md` 「새 내용을 어디에 넣나」의 「새 기능 **설계 착수** → `process/SN-name/` 새 폴더」 줄. 같은 폴더에 1-3(platform crate) TRD 가 따로 선다.
> **기준 코드:** `0ef6292`(브랜치 `v0.3.3/refactor/crate-boundaries`). master `9406581` 은 이 커밋을 머지한 것이고 `git log master..HEAD` = 0 이라 트리가 같다. storage 브랜치는 `origin/v0.3.3/feat/storage` `5a2cf7f`(2026-10-03 fetch)를 대조했다.
> 앵커: **ADR-0269**(이 단계의 헌장 — base 범용 도우미) · **ADR-0275**(이 TRD 의 결정 기록 — 2026-10-04 에 박았다. 1-1 몫(ADR-0269 · ADR-0268 개정 — §4-2)과 형제 1-3 몫(ADR-0266 · ADR-0262 개정)을 함께 실은 한 ADR 이다) · ADR-0268(로그는 `tracing` 직접) · ADR-0267(command · messaging 은 지금 base 를 의존하지 않는다) · ADR-0266(platform crate — `write_atomic` 의 거처 · base → platform 간선 없음) · ADR-0270 결정 5(`normalize_cwd` → base) · ADR-0175(입주 조건) · ADR-0177(transport — 워크스페이스 의존 0) · ADR-0086(토큰) · ADR-0231(출력 Channel 명부의 poison 경고 선례) · ADR-0262(claude 잔여물 정리의 잎 자물쇠) · `docs/research/clock-seam-placement-2026-10-03.md`.

---

## 0. 이 판의 정본 요약

```
① 들이는 모듈 다섯 — 전부 상태 없는 공개 자유 함수(ADR-0269 결정 2 · S1=a), 예외는 시계 트레이트 하나(결정 3).
     text   = hex_lower                       (UTF-8 자르기는 base 에 안 둔다 — 사본 셋을 std floor/ceil_char_boundary 로 제자리 교체 · T1=b)
     time   = now_epoch_ms + Clock(now 하나 · Send + Sync — C1=a) + SystemClock
              + ManualClock(test-support 기능 뒤 — C4=a)
     path   = normalize_spelling(P1=a)
     sync   = 락 오염 복구(lock · read · write · wait_timeout) — 경고 없이 되찾기만, 독 표시는 남긴다(S2=e)
     testing(test-support 기능 뒤 — N1=b) = wait_until

② 실측이 ADR-0269 의 그림과 다른 곳 — 결정(§4)을 거쳐 ADR-0275 로 고친다(§4-2).
     - 시계 seam 은 넷이 아니라 여섯이다(+ agent LeftoverClock · CaptureClock) → LeftoverClock 은 합친다(C2=a · U6). storage 브랜치가 일곱째를 더한다(P3 뒤 D2).
     - transport 에 base 를 붙이면 transport 의 「워크스페이스 의존 0」 게이트가 깨진다 → transport 몫은 3단계로 미룬다(C5=b).
     - UsageClock 은 깔끔히 맞지 않는다 → 그대로 둔다(C3=a).
     - UTF-8 경계 자르기는 고정된 rustc 1.95 의 std 한 줄(floor/ceil_char_boundary)이다 → base 함수 없음(T1=b).
     - 가짜 시계를 testing 에 두면 입주자 무참조(게이트 ③)와 부딪힌다 → time 안 기능 플래그 뒤(C4=a).
     - hex 루프 셋은 전부 「옮기지 않는다」로 정한 함수(토큰 생성기 둘 · sha256_hex) 안에 있다 → 루프만 바꾼다(T2=a). sha256_hex 의 제외 사유(「sha2 를 끈다」)는 거짓이다.
     - 락 오염 복구 117곳(운영 90) · 운영 빌드는 panic = "abort" 라 그 갈래가 안 닿는다 · 오늘 116곳이 이미 말없이 되찾는다
       → 경고 없는 도우미(S2=e)로 운영 83곳을 옮기고 7곳은 제자리(§3-5). 경고를 달았다면 쥔 채 로그 · IO · 밖 호출을 금한
         자물쇠 열둘(37곳)을 빼야 했다(§2-4 역사 기록 — 그 목록이 리뷰마다 넓어진 것이 (a) 를 고르지 않은 사유다).
     - 같은 자물쇠를 한쪽은 되찾고 다른 쪽은 .expect(또는 try_lock)로 쓰는 자리가 일곱이다 → (e) 는 독을 걷지 않아 그 의미가 그대로다.

③ 단위는 「새 함수 먼저 → 호출부 → 옛 사본 삭제」 순서로, 어디서 멈춰도 빌드가 선다(§6). 동작은 로그까지 그대로다.
```

---

## 1. 범위

### 1-1. 하는 것

| 모듈 | 무엇 | 흡수 대상(§2 실측) |
|---|---|---|
| `text` | 바이트 → 소문자 hex | daemon 세 루프(T2=a). ★UTF-8 자르기 사본 셋은 base 로 오지 않는다 — std 로 제자리 교체(T1=b · U1)★ |
| `time` | 지금 epoch ms · 「지금 읽기」 트레이트 · 가짜 시계(기능 뒤) | agent `now_millis` 세 벌 · daemon `command_delivery` `Clock`(운영 · 가짜 둘 다) · discovery `Clock` 의 now 부분 · agent `LeftoverClock` 의 `mono_now`(C2=a · U6) |
| `path` | 친 경로의 철자만 고르기 | agent `normalize_cwd`(셸이 가져다 씀) |
| `sync` | 락 오염 복구 — 경고 없음 · 독 표시는 남김(S2=e) | §2-4 의 운영 사이트 **83곳**(운영 90 − 제자리 7 — §3-5 · §5 M3) |
| `testing` | `wait_until(timeout, cond) -> bool` | 같은 계약의 사본 11벌 |

- **base 입주 규칙 문서도 함께 고친다** — ADR-0269 영향: base `lib.rs` 헤더의 입주 조건 ①을 결정 7 의 규칙으로 바꾸고, 「셋째 입주자를 받기 전에 ADR-0175 재론」 문단을 결정 1 이 답한 것으로 고친다(같은 변경에서). CLAUDE.md 「백엔드 모듈 맵」 base 항목도 같다. 루트 `Cargo.toml` 의 멤버 주석 둘(`:2` 「base(바닥 인프라 — 로깅·PID 판정 …)」 · `:12-13` 「로깅 + PID 판정만 담는 잎 crate … 셋째 입주자는 ADR 재론을 거친다」)도 같은 문장이라 함께 낡는다 — U1 에서 고친다(2차 리뷰). ADR-0269 결정 8(새 범용 도우미를 base 로 옮기도록 다음 세션을 유도하는 서술)도 이 자리에서 쓸 수 있다 — 쓸지 · 언제 쓸지는 오케스트레이터가 정한다.

### 1-2. 안 하는 것 (사유는 §8)

- `file::set_aside_corrupt` 통일 — storage P3 가 master 에 착지한 뒤(사용자 「알아서」 2026-10-03 → 메인 권고 적용 · 메모 §10).
- `write_atomic` — base 가 아니라 platform 으로 간다(ADR-0266 결정 8 · 1-3, 역시 P3 뒤).
- messaging 의 XML 이스케이프 · command 하네스(`src/testing.rs`)의 락 오염 복구 3곳 — 두 crate 는 지금 base 를 의존하지 않는다(ADR-0267 결정 1).
- 로그 래퍼 — 없다(ADR-0268 결정 1).
- agent codex 의 `clip` · `truncate`(글자 수 자르기) — 마스킹 문 뒤에 일부러 숨긴 비공개다. 옮기면 그 문이 뚫린다(§2-1).
- transport 시계와 transport 의 base 의존 — 3단계(transport 부착 TRD)로 미룬다(C5=b · §8 D4). transport 의 UTF-8 자르기는 1-1 에서 std 로 바꾸지만(U1) 그것으로 transport 가 base 를 의존하게 되지는 않는다.
- daemon `UsageClock` — 그대로 둔다(C3=a · §8 D6).
- 락 오염 복구의 제자리 7곳(§3-5) · 시험 전용 24곳(§5 M3).

---

## 2. 현황 실측 (`0ef6292`)

> 메모(`architecture-discussion-2026-09-26.md`)의 줄 번호는 여러 곳이 밀렸다 — 아래가 지금 값이다. 밀린 목록은 §10.

### 2-1. `text`

**UTF-8 경계 자르기 — 사본 셋, 모양 둘** (실측: `rg -n "is_char_boundary" crates src-tauri/src`)

| 자리 | 남기는 쪽 | 동작 | 반환 |
|---|---|---|---|
| `crates/engram-dashboard-transport/src/ws.rs:147-156` `clamp_close_reason` | 앞 | `len <= 123` 이면 그대로, 아니면 123 에서 문자 경계까지 **내려가** 자른다 | 빌린 `&str` |
| `crates/engram-dashboard-daemon/src/experiment/record.rs:178-186` `cap_response` | 앞 | 같은 루프 · 상한 4096 | 새 `String`(`saturation_pilot` 실험 바이너리만 부른다) |
| `crates/engram-dashboard-agent/src/output_core.rs:1059-1062` `push_diagnostic` 안 | 뒤 | `cut = len - CAP` 에서 문자 경계까지 **올라가** `drain(..cut)` — 제자리 수정 | 없음(`String` 을 고친다) |

- 그 밖의 `is_char_boundary` 셋은 자르기가 아니다 — messaging `service.rs:3804-3807`(구획 검증) · agent `backend/claude/mod.rs:5441`(시험 전제).
- **★std 한 줄로 같은 동작이 된다(실측)★** — `str::floor_char_boundary` · `str::ceil_char_boundary` 가 이 워크스페이스의 고정 컴파일러(`rust-toolchain.toml` = 1.95.0)에서 기능 플래그 없이 컴파일된다(스크래치 파일을 `rustc --edition 2021` 로 컴파일 · `"가나다"` 에서 4 → 3 · 6 확인, 2026-10-03). 앞 남김 = `&s[..s.floor_char_boundary(max)]`, 뒤 남김 자리 = `s.ceil_char_boundary(s.len() - max)`. 위 두 루프와 결과가 같다(`floor` 는 `max >= len` 이면 `len` 을 돌려준다). → **결정 T1=(b)**: base 함수 없이 이 둘로 제자리 교체(§3-2 · U1).
- **뒤 남김은 사본이 하나뿐이다** — ADR-0269 결정 7(「지금 여러 곳에서 쓰이거나 복사돼 있으면」)로는 입주 근거가 안 선다. 결정 2 의 표가 `keep_tail_bytes` 를 적은 것은 사용자 「다 옮겨」(2026-09-26)를 따른 것이다 — T1=(b) 로 ADR-0275 가 두 함수를 그 표에서 뺀다(§4-2).
- **안 옮기는 것(제자리):** agent `backend/codex/decoder.rs:1479` `clip` → `:1488` `truncate`(`char_indices().nth` — 글자 수 · `…` 를 붙임) · `backend/codex/transport.rs:1537` `clip`(글자 수 · `…(잘림)`). 둘 다 소스 문자열 시험이 「문이 정해진 둘뿐」을 잰다(`decoder.rs:3486` · `transport.rs:5028`). agent `usage/process.rs:616` `tail_line` 은 바이트를 손실 복원(`from_utf8_lossy`)하고 마스킹을 먼저 한다 — 계약이 다르다. `output_core.rs` · `session.rs` 의 `terminal_tail` 은 `Vec<u8>` 이다.

**바이트 → 소문자 hex — 루프 셋, 전부 「옮기지 않는다」 함수 안** (실측: `rg -n '02x\}' crates src-tauri/src`)

| 자리 | 감싼 함수 | ADR-0269 의 판정 |
|---|---|---|
| `crates/engram-dashboard-daemon/src/lib.rs:77-80` | `generate_token`(:73 — WS 클라이언트 토큰) | 토큰 생성기 공유 금지 · 루프만 범용 |
| `crates/engram-dashboard-daemon/src/control/mod.rs:146-149` | `gen_token`(:142 — 제어 채널 토큰) | 같음 |
| `crates/engram-dashboard-daemon/src/experiment/record.rs:192-195` | `sha256_hex`(:189) | 「`sha2` 를 끌고 온다」로 제외 — ★사실이 아니다★(아래) |

- 셋 다 `write!(s, "{b:02x}")` 루프로 같다. 소비자는 daemon 하나다.
- **ADR-0086 본문에는 공유 금지 문장이 없다** — `grep -n "공용\|공유" docs/decisions/0086-*.md` → 0줄. 「공용화 금지 · 혼용 금지(ADR-0086 §맥락)」는 코드 주석(`daemon/src/lib.rs:584-585` · `control/mod.rs:140-141`)에 있고, 가리키는 것은 **두 토큰을 섞지 말라**는 것이다(인코딩 루프가 아니라). 그러니 생성기 둘은 따로 두고 그 안의 루프만 `hex_lower` 를 부르는 것은 그 주석과 부딪히지 않는다(워커 판단).
- **`sha256_hex` 는 `sha2` 를 끌지 않는다** — std 만으로 손구현한 SHA-256 이다(`record.rs:247-251` 주석 「워크스페이스에 sha2 가 없다」 · `grep -n sha2 crates/*/Cargo.toml src-tauri/Cargo.toml` → 0줄). 그래도 함수 자체는 사본이 하나라 결정 7 로 입주하지 않는다 — 제외는 서되 사유가 바뀐다(ADR-0275 — §4-2).
- → **결정 T2=(a)**: 감싼 함수 셋은 제자리, 안의 루프만 `hex_lower` 로(U1).

### 2-2. `time`

**지금 epoch ms** (실측: `rg -n "UNIX_EPOCH" crates src-tauri/src`)

| 자리 | 모양 | 쓰는 곳 |
|---|---|---|
| agent `profile.rs:20-25` `now_millis` | `i64` · 1970 전 = 0 | `:244` · `:819`(`last_active`) |
| agent `persistence/mod.rs:25-30` `now_millis` | 같음 | `:89` 손상 사본 이름(`agents.json.corrupt-<ms>`) **하나뿐** |
| agent `persistence/presets.rs:22-27` `now_millis` | 같음 | `:81` 손상 사본 이름 **하나뿐** |
| 셸 `src-tauri/src/settings/store.rs:122-125` `copy_aside` 안 | `u128` · 1970 전 = 0 | 손상 사본 — ★storage P3 가 이 파일을 고친다 → 1-1 에서 손대지 않는다★ |
| storage 브랜치 `src-tauri/src/state/saver.rs`(`5a2cf7f`) `SystemClock::wall_ms` | `u64` · 1970 전 = 0 | 아직 master 에 없다 |

- 나머지 `UNIX_EPOCH` 자리는 ms 가 아니다 — 나노초 임시 폴더 이름(discovery `lib.rs:2620` · daemon `tests/ws_e2e.rs:2341` · 셸 `settings/tests.rs:24`) · 초(daemon `bin/saturation_pilot.rs:1703,1713` · `usage_service/clock.rs:51` 의 부호 있는 내림 초) · base `logging/mod.rs:155`(로그 파일 이름 — `logging` 안에 남는다: 입주자 무참조).
- **반환 타입이 세 갈래다**(i64 · u128 · u64). 1-1 이 바꾸는 것은 agent 의 i64 셋뿐이다.

**시계 seam — ADR-0269 는 넷으로 셌는데 지금 여섯이다** (실측: `rg -n "trait (Clock|UsageClock|\w*Clock)\b" crates src-tauri/src`)

| 자리 | 모양 | 경계 | 운영 구현 | 가짜 |
|---|---|---|---|---|
| daemon `command_delivery.rs:176` `Clock` | `now() -> Instant` | `Send + Sync` | `SystemClock`(:181) | `tests::ManualClock`(:2425 — `Mutex<Instant>` + `advance`) · `connection_core.rs:4403` 도 쓴다 |
| discovery `lib.rs:431` `Clock` | `now()` + 막는 `sleep(Duration)` | 없음 · `&dyn Clock` 로 받는다(:485) | `RealClock`(:1047) | `FakeClock`(:1742 — `RefCell<Instant>` · `sleep` 이 now 를 민다 · `slept` 를 센다) |
| transport `clock.rs:20` `Clock` | `now()` + `sleep -> BoxFuture<'static, ()>` | `Send + Sync + 'static` · `Arc<dyn Clock>` | `SystemClock`(tokio sleep) | `testing::ManualClock`(:99 — 시한 순서로 잠꾸러기를 깨운다 · oneshot) |
| daemon `usage_service/clock.rs:9` `UsageClock` | `mono() -> Duration`(기점 보유) + `wall() -> i64`(부호 있는 epoch 초) | `Send + Sync + 'static` | `OsUsageClock` | `ManualUsageClock`(두 축을 따로 민다) · `usage_service/mod.rs:1251` · `schedule.rs:370` |
| ★agent `backend/claude/leftover.rs:953` `LeftoverClock`★ | `mono_now() -> Instant` + 막는 `sleep` + 스레드 `spawn` | `Send + Sync` · `pub(super)` | `SystemClock`(:961) | `FakeClock`(:4664 — 원자 정수 · sleep 이 민다) · `SkipClock`(:6586 — **실시간** + 건너뛴 몫) |
| ★agent `backend/codex/thread_lock.rs:261` `CaptureClock`★ | `child_alive()` + 막는 `sleep` — **지금 읽기가 없다** | 없음 | `ChildClock` | `FakeClock`(:895) |

- `LeftoverClock` 은 「지금 읽기」 seam 이다 — ADR-0269 결정 3 이 세지 않은 다섯째. `mono_now` 호출 22곳(`rg -n "mono_now\(" crates/engram-dashboard-agent/src | wc -l`). 그 계약에 「★문 자물쇠 안에서도 부른다★ — 다른 자물쇠를 잡거나 기다리지 않는다」가 박혀 있다(:954) — CLAUDE.md 「핵심 불변식」 락 순서의 잔여물 정리 조항과 같은 것이다. → **결정 C2=(a)** — 상위 트레이트 꼴로 합친다(U6).
- `CaptureClock` 은 시간을 읽지 않는다 — 합칠 몫이 없다(워커 판단, 질문 아님).
- **storage 브랜치가 일곱째를 더한다** — `src-tauri/src/state/saver.rs`(`5a2cf7f`) `pub trait Clock: Send + 'static { now() -> Instant; wall_ms() -> u64 }`. `Sync` 가 없다. P3 착지 뒤 단위(§8 D2)의 몫이다.
- **`UsageClock` 은 깔끔히 맞지 않는다**(ADR-0269 결정 3-4 의 조건 — 「안 맞으면 그때 사용자에게 묻는다」):
  - `mono()` 는 `Instant` 가 아니라 **시계가 쥔 기점부터의 `Duration`** 이고, 그 값이 `usage_service/book.rs` 의 도메인 타입으로 흘러간다(`book.rs:54` `pub mono: Duration` · `owes_at(mono)` · `reject_deadline(mono, …)` 등). 시험은 `Duration::MAX` · `Duration::ZERO` 끝값을 쓴다(`book.rs:2133,2756` · `clock.rs:176`) — `Instant` 로는 그 끝값을 만들 수 없다.
  - `wall()` 은 부호 있는 epoch **초**(1970 전 음수 · 양끝 포화)라 `now_epoch_ms()`(ms · 1970 전 0)로 바꿀 수 없다.
  - 사용량: `.mono()` 11곳 · `.wall()` 13곳(daemon src).
  - → **결정 C3=(a)** — 그대로 둔다(결정 3-4 의 「안 맞으면 그때 묻는다」에 대한 답 · 사유는 `usage_service/clock.rs` 머리에 · U5 · §8 D6).

### 2-3. `path`

- 정의 = agent `commands.rs:1143-1153` `pub fn normalize_cwd(raw) -> String` — 첫 글자가 `"` 또는 `'` 이고 같은 글자로 끝나며 안이 비지 않았으면 **한 겹만** 벗기고, 그 뒤 `\` → `/`. 파일시스템을 보지 않는다.
- 부르는 곳 = agent `commands.rs:1106`(등록 공통부) · 셸 `src-tauri/src/layout/commands.rs:38`(import) · `:1127`(`verb_spawn_into`). 셸 시험 `tests/layout_commands.rs:2121` 은 주석에서 이름만 부른다.
- 시험 = agent `commands.rs:2091`(벗기기 · 뒤집기 다섯) · `:2106`(안 벗기는 다섯) — 함수와 함께 base 로 옮길 수 있다. `:2114` 이후의 「두 문이 같은 철자」 시험은 등록 경로를 재므로 agent 에 남는다.
- 문서 주석이 도메인을 말한다(「사람 · LLM 이 친 cwd」 · 앞의 빈 값 검문 `reject_blanks` · ADR-0122) — base 판은 그 말을 빼고 계약만 남기고, 도메인 사유는 agent 호출 자리 주석으로 옮긴다(워커 제안).
- 셸 → agent 간선은 이것으로 닫히지 않는다 — `COMMAND_SPECS` · `llm_creation_refusal` 이 남는다(2-1 몫 · ADR-0270).

### 2-4. `sync` — 락 오염 복구

**집계 = 117곳**(실측 — 셈법은 §10). ADR-0269 「근거」의 94(`ebfdafc`)는 같은 줄 클로저 67 + `PoisonError::into_inner` 27 이었고, 지금 그 두 셈은 67 · 27(줄 수 — 그중 1줄은 `output_channel.rs:18` 주석)로 같다. 117 은 여러 줄에 걸친 꼴과 다른 꼴까지 넣은 수다.

| 꼴 | 수 |
|---|---|
| `.lock()` + `unwrap_or_else(\|x\| x.into_inner())` | 77 |
| `.lock()` + `unwrap_or_else(PoisonError::into_inner)` | 26 |
| `match m.lock() { Ok(g) => g, Err(p) => p.into_inner() }` | 6 |
| `.lock().unwrap_or_else(\|e\| { warn; clear_poison; e.into_inner() })` | 1 |
| `Condvar::wait_timeout(..).unwrap_or_else(\|p\| p.into_inner())` | 4 |
| `RwLock::read` / `write` + 클로저 | 1 / 1 |
| `try_lock` 의 `TryLockError::Poisoned` 갈래 | 1 |

| crate | 운영 | 시험 전용 | 비고 |
|---|---|---|---|
| agent | 68 | 19 | 운영 중 `backend/codex/transport.rs` 42 · `transport/input_queue.rs` 7 · `backend/claude/leftover.rs` 3 · `transport/stdio.rs` 3 · `manager.rs` · `reaper.rs`(read 1 · write 1) · `session.rs` · `transport/pty.rs` 각 2 · `inputs_pending.rs` · `queued_input.rs` · `session_id_latch.rs` · `turn.rs` · `usage/process.rs` 각 1 |
| daemon | 8 | 5 | 운영 = `agent_conn.rs:213` · `command_delivery.rs:1008` · `command_roster.rs:401` · `usage_service/mod.rs:571,606,648` · `usage_service/reject_store.rs:276` · `usage_service/watch.rs:205` |
| 셸 | 13 | 0 | `view_commands.rs` 7(:403,410,455,527,552,758,790) · `settings/mod.rs` 3(:478,482,486) · `daemon_client/mod.rs:88` · `output_channel.rs:41` · `ui_settings.rs:768` |
| base | 1 | 0 | `logging/mod.rs:129` |
| command | 0 | 3 | `src/testing.rs` |
| messaging · net · protocol · discovery · transport | 0 | 0 | — |

- 「운영/시험」 가름은 스크립트 추정(파일의 첫 `#[cfg(test)] mod … {` 뒤 · `tests/` · `testing.rs` · `fakes.rs`)에 손 정정을 더했다 — daemon `usage_service/clock.rs:114` 는 `#[cfg(test)]` 가짜라 시험 쪽으로 셌다. ★2차 리뷰 때 감싼 함수의 속성까지 보도록 스크립트를 고쳐 둘을 더 시험 쪽으로 옮겼다★ — codex `transport.rs:1349`(`#[cfg(test)] fn len` `:1348` — `Pending` 의 시험용 창) · `transport/input_queue.rs:263`(`#[cfg(test)] fn queued_bytes` `:262`). 그래서 운영 92 → 90 · 시험 전용 25 → 27 이다. 감싼 `impl` 이나 모듈에 붙은 속성은 여전히 손으로 본 것뿐이라 오차 가능성은 남는다.
- **이미 있는 선례 둘:** agent `usage/process.rs:636` 의 `fn lock<T>(&Mutex<T>) -> MutexGuard<T>` — 자유 함수 꼴 그대로다(부르는 곳 = `StderrTail` 셋 `:696 · 985 · 1002` + 찾기 명단 `LookupGate::release` `:352`). 셸 `output_channel.rs:38-45` — **경고 + `clear_poison`** 으로 「poisoning 한 번에 warn 한 번」(ADR-0231 앵커). 그 시험 `:145-148` 이 되찾은 뒤 `!registry.is_poisoned()` 를 단언한다 — **독을 걷지 않는 도우미로 옮기면 빨개진다** → S2=(e) 의 도우미는 독을 걷지 않으므로 이 자리는 제자리다(§3-5 · U2). 그 밖의 사이트는 전부 **말없이** 되찾고 독 표시를 남긴다 — 117곳 중 116곳이다(S2=(e) 의 근거).
- **★운영 빌드에서는 이 갈래가 안 닿는다★** — 워크스페이스 `[profile.release]` 가 `panic = "abort"`(`Cargo.toml:34-35`)라 패닉이 프로세스를 끝내고 독이 생기지 않는다. 그 사실을 코드 주석이 이미 적는다(`command_roster.rs:396` · `command_delivery.rs:1003`). 그러니 복구 경고를 달았더라도 **debug · 시험 빌드의 신호**였다 — S2=(e) 가 잃는 것이 그것뿐이라는 근거다(§4).
- **★S2=(e) 의 이전 규칙 — 운영 90곳 중 83곳을 옮기고 7곳은 제자리★**(목록 · 사유 = §3-5). 도우미가 로그 · 다른 락 · 밖 호출을 하지 않으므로, 문서가 「쥔 채 로그 · IO · 밖 호출」을 금한 자물쇠(잔여물 정리 잎 둘 — CLAUDE.md 「핵심 불변식」 락 순서 · 대기 입력 명부 · codex 상태 락 등)의 사이트도 옮긴다 — 도우미가 더하는 동작이 없어 옮겨도 그 자물쇠들의 규율이 바뀌지 않는다(무엇을 언제 잡는지는 부르는 쪽 그대로).
- **역사 기록 — 경고를 다는 갈래(S2 (a) ~ (d))였다면 필요했던 제외 목록(채택 안 함 · ADR-0275 의 거부 사유):** 그 갈래들은 경고가 **가드를 쥔 채** 찍힌다(독 걸린 가드를 되찾은 뒤라야 돌려줄 수 있다). 그래서 자물쇠 문서가 「쥔 채 로그 · IO · 밖 호출」을 금한 자물쇠와, 그런 자물쇠 **안에서** 잡히는 자물쇠(중첩)는 옮기지 못하고 옛 꼴로 둬야 했다 — 열둘 · 복구 37곳(메인 판단 2026-10-03):
  - agent(33) — 잔여물 정리 잎 둘(`leftover.rs:402` `Recorder.inner` · `:1713` `GateCell.state` — CLAUDE.md 락 순서) · 대기 입력 명부(`queued_input.rs:405` — 「★이 가드를 쥔 채 emit·IO·다른 락을 잡지 말 것★」) · 대기 목록 표(`inputs_pending.rs:47` — 「명부·표는 잎」) · 첫 제출 래치(`session_id_latch.rs:150` — 머리 `:23`) · 그 commit 포트의 `expected` 칸(`manager.rs:550` — 중첩 · 3차 리뷰) · 턴 관측 표(`turn.rs:145` — 머리 `:18-21` 「leaf」) · codex 상태 락 26(`backend/codex/transport.rs` — `SharedState` `:1079` · 규율 `:3010-3015` 「★상태 락을 쥔 채 찍지 않는다★」 · 사이트 = `lock.lock()` 25 + 라이터 `wait_timeout` `:2192`).
  - daemon(4) — 명령 명부(`command_roster.rs:401` — `detach` 문서 `:313-314`) · 사용량 출구 칸(`agent_conn.rs:213` — `usage_service/watch.rs:28-29`) · 사용량 구독 명부(`usage_service/watch.rs:205` — `:146-147`) · 사용량 책 락(`usage_service/mod.rs:606` — 구독 명부 안에서 잡힌다 · 중첩).
  - **그 목록의 이력이 (a) 를 고르지 않은 주된 사유다** — 1판 = 잔여물 정리 잎 둘 → 1차 리뷰 = 같은 규율의 문구로 셋 더 → 2차 리뷰 = 다른 표현(굵은 글씨 · 영어 `leaf`)으로 다섯 더 + 중첩 규칙(책 락) → 3차 리뷰 = 중첩이 놓친 `expected` 칸. 리뷰마다 넓혀야 했고 지키는 게이트가 없었다 — 자물쇠 문서에 「쥔 채 로그 금지」를 새로 적거나 제외 자물쇠 안에서 옮긴 자물쇠를 잡게 고치면 말없이 어긋난다(잠복 자리 하나 = codex 머리가 허용해 둔 「상태 → 대기표」 · §9). 그 규율이 코드에서 이미 다 지켜지지도 않는다(§9 의 둘). 교착 때문은 아니었다 — 운영 구독자(base `logging/mod.rs:417-421`)의 파일 sink 는 자기 `LOG_FILE` 자물쇠만 잡는다(코드 읽기). 걸린 것은 규율 일관이었다.
  - ★「놓고 경고하고 다시 잡는」 도우미도 거부했다★ — 그 자물쇠 자신은 비우지만 부른 쪽이 쥔 바깥 자물쇠 아래서는 여전히 찍힌다(예: codex `Announcer.order` → 상태 락 — `transport.rs:1112-1115`).
  - 4판이 실었던 상세(찾은 법의 `rg` 정규식 · 자물쇠별 문서 인용 전문 · 자물쇠마다의 중첩 전수 점검 · 「경계에 있지만 빼지 않는」 넷 — `command_delivery.rs:1008` · `daemon_client/mod.rs:88` · `view_commands.rs` · `reaper.rs`)는 이 판에서 걷었다. 실측 명령의 요약은 §10. ★경고를 다시 달고 싶어지면 이 감사를 83곳 전체에 대해 처음부터 다시 한다★(ADR-0275 에 적는다).
- **섞인 자물쇠 — 같은 자물쇠를 한쪽은 되찾고 다른 쪽은 `.expect`(또는 `try_lock`)로 쓴다** (1차 리뷰 넷 + 2판 실측 셋. 찾은 법 = 복구 사이트의 필드 이름으로 같은 crate 운영 코드의 `.lock()/.read()/.write()` + `.expect(`/`.unwrap(` 를 맞대는 스크래치 스크립트 뒤 손 확인 — 필드 이름이 같은 남의 구조체는 걸러 냈다). 지금 독 표시는 **사라지지 않는다** — 그래서 아래 「다른 쪽」의 동작은 그 사이에 복구가 돌았는지와 무관하다. ★S2=(e) 의 도우미도 독을 걷지 않으므로 이 무관함이 1-1 뒤에도 그대로다★ — 이 표는 (b)(`clear_poison`)를 거부한 근거로 남긴다(마지막 칸 = (b) 였다면).

| 자물쇠 | 되찾는 쪽 | 다른 쪽 | 독을 걷었다면((b) — 채택 안 함) |
|---|---|---|---|
| agent PTY `child` | `transport/pty.rs:261-264`(감시 스레드) · `:330-333`(펌프) | `shutdown()` 2단계 `:422` `.expect("child poisoned")` — ADR-0001 kill 인과 경로 | 지금은 독이면 shutdown 이 패닉해 4단계(Job terminate) · 5단계(master drop)를 건너뛴다. 걷으면 그 패닉 여부가 감시 · 펌프의 복구가 먼저 돌았나에 달린다 |
| agent stdio `child` | `transport/stdio.rs:378-381`(펌프) | `shutdown()` `:482` `.expect("child poisoned")` | 같은 꼴(Job terminate 를 건너뛰나가 순서에 달린다) |
| agent `sessions` RwLock | `reaper.rs:52-55`(write) · `:154-156`(read) | `manager.rs` 운영 구획의 `.expect("sessions poisoned")` 7곳(`:843 · 2011 · 2839 · 2921 · 2938 · 2954 · 2972` — 시험 모듈 `:3060~` 의 여섯은 뺐다) | reaper 한 번이 매니저의 fail-fast 신호를 지운다 |
| daemon `CommandRoster.inner` | `command_roster.rs:400-405` `lock_for_cleanup`(「정리 경로 전용」) | `lock()` `:390-391` `.expect` — 일부러 갈라 둔 짝이다 | 정리 한 번이 「오염된 표를 계속 쓰지 않는다」를 지운다 |
| daemon `CommandDeliveries.inner` | `command_delivery.rs:1003-1008` `lock_for_cleanup`(소멸자 전용) | `lock()` `:990-991` `.expect` — 위와 같은 짝(그 주석이 「표마다 따로 서야 한다」) | 같음 — ★리뷰가 못 센 것(2판 실측)★ |
| agent 찾기 명단 `LookupGate` | `usage/process.rs:352` `release` → `:636` 도우미 | `claim` `:345-348` 이 독을 「도는 중 → 모름」으로 읽는다(시험 `:1742`) | 도는 중이던 찾기 하나의 `release` 가 그 판정을 「평소」로 되돌린다 — 1-1 에서는 제자리(U3a · 1-3 U5) |
| agent stdin(stdio · codex) | `stdio.rs:194` · codex `transport.rs:1555 · 2751-2753` | `shutdown` 의 `try_lock`(`stdio.rs:496` · codex `:4246`)이 독을 「건너뜀」으로 읽는다 | shutdown 이 stdin 을 거두게 된다(무해 쪽이지만 동작이 바뀐다) — 2판 실측 |

- **storage P3 와 겹치는 파일:** 셸 `settings/mod.rs`(3곳) — storage 브랜치가 이 파일을 고친다(`git diff <merge-base> origin/v0.3.3/feat/storage --stat` → `settings/mod.rs | 36`). storage 브랜치는 복구 사이트 1곳(`state/` — `self.cell.lock().unwrap_or_else(PoisonError::into_inner)`)을 더한다.
- **`clear_poison` 은 이 컴파일러에 있다** — `Mutex::clear_poison` · `RwLock::clear_poison`(셸 `output_channel.rs:43` 이 이미 쓴다). 1-1 의 도우미는 쓰지 않는다(S2=e).
- **시험이 독 상태를 단언하는 자리**(`rg -n "is_poisoned" crates src-tauri/src`): 9곳 — 전부 복구 **전** 전제(「전제: 락이 poisoned」 · 「시험이 poison 갈래를 탔다」)이거나 `!is_poisoned` 이다. S2=(e) 의 도우미는 독을 걷지 않아 그 전제와 재는 범위가 그대로다(읽기 — 돌려 보지 않았다). 셸 `output_channel.rs:145-148` 의 `!is_poisoned` 는 그 자리의 독 걷기(ADR-0231)를 재므로 그 자리는 제자리다(§3-5 · U2). (b) 였다면 단언은 안 깨지되 재는 범위가 줄었다 — 예: `leftover.rs:3903` 「기록 자물쇠가 독에 걸려도 켜기 · 넣기 · 복사 · 끄기가 돈다」가 첫 연산만 독 갈래를 탄다.

### 2-5. `testing` — 시험 대기

**같은 계약 `fn(timeout, cond) -> bool`(마감 뒤 마지막 한 번 더 본다) — 사본 11벌** (실측: `rg -n "fn (wait_until|poll_until|…)\b" crates src-tauri`)

| 자리 | 클로저 | 폴링 |
|---|---|---|
| agent `tests/activation.rs:53` · `transport_smoke.rs:65` · `session_smoke.rs:85` · `stdio_smoke.rs:135` · `headless.rs:79` | `Fn` | 50 ms |
| agent `tests/backend_contract.rs:703` | `FnMut` | 50 ms |
| agent `tests/reaper.rs:54` | `Fn` | 25 ms |
| daemon `tests/control_send.rs:76` · `tests/mcp_manager_lifecycle.rs:64` | `Fn` | 20 ms |
| daemon `tests/ws_e2e.rs:2404` `poll_until`(이름만 다름 · 모듈 안) | `FnMut` | 50 ms |
| daemon `src/messaging_host.rs:1153`(`#[cfg(windows)]` 시험 모듈 안) | `Fn` · **시한 인자 없음**(150 × 20 ms 고정) | 20 ms |

- ADR-0269 의 「9개 파일(agent 7 · daemon 2)」은 맞다 — 위 11벌 중 앞 9벌이다.
- **계약이 다른 변형 다섯(제자리 제안):** agent `platform/windows.rs:746`(`Option<T>` 를 돌려줌 · 10 s 뒤 패닉) · `transport/input_queue.rs:620`(5 s · 패닉) · `backend/codex/mod.rs:1251`(60 s · 패닉 전에 캡처를 안 거치는 stderr 에 먼저 쓴다 — 실측 사유가 주석에 있다) · daemon `usage_service/mod.rs:1417`(상한 `BOUND` · 패닉) · agent `usage/process.rs:1056` `eventually`(5 s · 패닉). 라벨 + 패닉 + 각자의 폴링(5 ms 등)이라 `assert!(wait_until(..), "{what}")` 로 접으면 폴링이 바뀐다.
- **기능 플래그 선례:** `test-support`(command · transport — 하네스 도우미 모듈을 연다) · `test-harness`(agent · daemon · messaging — 운영 코드 안의 seam 을 연다). 둘 다 dev-dependency 로만 켜고, 워크스페이스는 `resolver = "2"`(`Cargo.toml:9`)라 dev 그래프의 기능이 운영 빌드로 합쳐지지 않는다. → **결정 N1=(b)** — base 도 같은 뜻이므로 `test-support`.
- **운영 그래프 확인 명령(실측 — 지금은 base 에 기능이 없어 `feature "default"` 만 나온다):** `cargo tree --locked -p engram-dashboard-daemon -e normal,features -i engram-dashboard-base --target all`. 같은 꼴로 `-e normal,dev,features -p engram-dashboard-agent -i engram-dashboard-command` 를 돌리면 dev 쪽에서만 `feature "test-support"` 가 보인다(실측 — 기존 선례가 그렇게 갈린다).

---

## 3. 설계 (§4 의 결정을 반영 — 꼬리표가 없는 세부는 워커 제안)

### 3-1. 배치 · 입주 규칙

- 모듈 = 파일 하나씩: `src/text.rs` · `src/time.rs` · `src/path.rs` · `src/sync.rs` · `src/testing.rs`. 머리마다 `// ADR-0269`.
- **`testing` 은 `#[cfg(any(test, feature = "test-support"))] pub mod testing;`** — transport 선례(`any(test, feature = "test-support")`)와 같다. 그러면 base 자기 시험이 기능 없이 그 모듈을 컴파일해, CI 의 base 시험 스텝(`ci.yml:276-278` — 기능 인자 없음)을 고치지 않아도 된다. 기능 이름 = `test-support`(N1=b) — `time::ManualClock` 도 같은 기능 뒤다(C4=a).
- **입주자 무참조(게이트 ③)를 지킨다** — `text` · `time` · `path` · `sync` · `testing` 어느 것도 다른 입주자를 부르지 않는다. 걸리는 자리:
  - 가짜 시계를 `testing` 에 두면 `crate::time::Clock` 을 구현해야 해 게이트 ③ 에 걸린다 → `time` 안 같은 기능 뒤에 둔다(C4=a · §3-3). `testing` 은 `time` 을 부르지 않는다.
  - `sync` 는 로그를 내지 않는다(S2=e) — `logging` 도 `tracing` 도 부르지 않는다.
  - base `logging` 자기 복구 사이트(`logging/mod.rs:129`)는 `sync` 를 쓰지 않는다(게이트 ③ · §3-5).
- **새 외부 의존 0** — 새 모듈 다섯은 전부 std 만 쓴다. serde · tokio · futures 를 들이지 않는다(ADR-0269 결정 6).
- **오류 타입:** 1-1 의 함수는 전부 실패하지 않는다 — `std::io::Result` 를 쓸 자리가 없다. 그 규칙은 `file`(P3 뒤)에서 쓰인다.
- 1-3 이 `platform` 모듈을 base 에서 내보낸다(ADR-0266 결정 4) — 그때 게이트 ③ 의 이름에서 `platform` 이 빠진다(§7).

### 3-2. `text`

```rust
/// 바이트를 소문자 16진 문자열로 — 바이트 하나당 두 글자.
pub fn hex_lower(bytes: &[u8]) -> String
```

- 루프 셋(§2-1)을 이것으로 바꾼다 — 감싼 함수(토큰 생성기 둘 · `sha256_hex`)는 제자리에 그대로 둔다(T2=a).
- **UTF-8 자르기는 base 에 두지 않는다(T1=b)** — `truncate_bytes` · `keep_tail_bytes` 는 만들지 않는다. 사본 셋을 U1 에서 std 로 제자리 교체한다(손 루프만 바꾸고 함수 이름 · 시그니처 · 상한 상수는 그대로):
  - transport `ws.rs:147-156` `clamp_close_reason`(앞 남김 · 빌림) — `while … is_char_boundary` 루프 → `&reason[..reason.floor_char_boundary(MAX_CLOSE_REASON_BYTES)]`. transport 는 이 교체로도 base 를 의존하지 않는다(C5=b).
  - daemon `experiment/record.rs:178-186` `cap_response`(앞 남김 · 새 `String`) — 같은 루프 → `s[..s.floor_char_boundary(RESPONSE_CAP_BYTES)].to_string()`.
  - agent `output_core.rs:1059-1062` `push_diagnostic` 안(뒤 남김 · 제자리 수정) — `cut` 을 올리는 루프 → `let cut = buf.ceil_char_boundary(buf.len() - DIAGNOSTIC_CAP_BYTES);` 뒤 `buf.drain(..cut)` 그대로. 그 위의 「★문자 경계로 밀어 올린다★」 주석은 남긴다(왜 경계인가는 그대로 참이다).
  - 셋 다 손 루프와 결과가 같다(§2-1 — `floor` 는 `max >= len` 이면 `len`). 상한 아래에서 일찍 돌아가는 분기는 남겨도 지워도 결과가 같다 — 남긴다(바뀐 줄을 줄인다 · 워커 제안).

### 3-3. `time`

```rust
/// 벽시계 epoch 밀리초. 1970 전이면 0 (agent 세 벌의 동작 그대로).
pub fn now_epoch_ms() -> i64

/// 「지금 읽기」 — 시험이 시간을 갈아 끼우는 seam(ADR-0269 결정 3-1).
pub trait Clock: Send + Sync {          // C1=a
    fn now(&self) -> Instant;
}
pub struct SystemClock;                 // now() = Instant::now()

/// 시험용 가짜 — 기점 + 원자 정수 오프셋, 자물쇠 없음(C4=a).
#[cfg(any(test, feature = "test-support"))]
pub struct ManualClock { /* … */ }      // new() · advance(d) · impl Clock
```

- 반환 타입 `i64` = agent 가 영속 필드(`last_active` 등)에 쓰는 타입과 같다. storage 의 `u64` · settings 의 `u128` 은 P3 뒤 단위(§8 D2)에서 맞춘다.
- 기다리기가 필요한 쪽은 **자기 트레이트가 이것을 상위 트레이트로 둔다**(ADR-0269 결정 3-2) — 예: discovery `pub trait Clock: engram_dashboard_base::time::Clock { fn sleep(&self, d: Duration); }`. 가짜 하나가 두 트레이트를 같은 상태로 구현한다(sleep 이 now 를 미는 연동 유지).
- **★discovery 는 base 의존을 지키고, U5 뒤에는 그 의존을 실제로 쓴다★(2차 리뷰 — 1-3 과의 정합).** 오늘(`0ef6292`) discovery 가 base 에서 부르는 것은 `platform::pid_alive_with_start_time` 하나다(`discovery/src/lib.rs:1043` · `Cargo.toml:15` 의 `engram-dashboard-base`). 1-3 이 `platform` 을 base 밖으로 옮기면 그 쓰임은 사라지지만, U5 가 discovery `Clock` 을 `engram_dashboard_base::time::Clock` 의 하위 트레이트로 바꾸면서 discovery → base 간선이 다시 실제 쓰임을 얻는다. ★그러니 1-3 은 discovery 의 base 의존을 걷지 않는다★ — 1-3 이 U5 보다 먼저 착지하면 그 사이 잠시 쓰임 없는 의존이 되는데, 워크스페이스에 쓰임 없는 의존을 잡는 게이트가 없어(`rg 'unused_crate_dependencies|machete|udeps' Cargo.toml crates/*/Cargo.toml .github/workflows/ci.yml` → 0줄) 빌드 · CI 는 그대로 선다. 1-3 TRD 쪽 정정은 그 문서의 몫이다(넘겨줌).
- **`dyn Sub` 에서 상위 트레이트의 `now()` 를 부를 때 base 트레이트를 `use` 해야 하는지는 컴파일로 확인하지 않았다** — 필요하면 호출 파일에 `use engram_dashboard_base::time::Clock as _;` 한 줄이다.
- 가짜 시계 `ManualClock`(원자 정수 오프셋, 자물쇠 없음 · `new()` · `advance(d)`)은 `time` 안 `#[cfg(any(test, feature = "test-support"))]` 뒤에 둔다(C4=a) — `testing` 에 두면 게이트 ③ 에 걸린다. 자물쇠 없는 꼴인 이유 = `LeftoverClock` 의 「문 자물쇠 안에서도 부른다 — 다른 자물쇠를 잡지 않는다」 계약에 가짜도 들어설 수 있게(지금 그 가짜들이 원자 정수다 · 워커 제안).
- **합치는 것:**
  - daemon `command_delivery` — base `Clock` · `SystemClock` 을 그대로 쓰고, 시험 가짜 `tests::ManualClock`(`:2425` — `Mutex<Instant>`)도 base `ManualClock` 으로 바꾼다(`connection_core.rs:4403` 의 시험도 같다). 결정 3-3 · U5. daemon 의 dev 의존에 `test-support` 를 켜는 줄은 U4 가 이미 더한다.
  - discovery — 결정 3-2 대로 `Clock: engram_dashboard_base::time::Clock` + 막는 `sleep`. 가짜 `FakeClock`(`:1742`)은 sleep 이 now 를 미는 연동 · `slept` 셈을 쥐어 자기 자리에 남고, `RefCell` · `Cell` 을 `Sync` 인 꼴(원자 정수 또는 `Mutex`)로 바꾼다(C1=a). base `ManualClock` 을 품지 않는다 — discovery 에 dev 의존 기능을 새로 켜지 않으려는 것이다(워커 제안). U5.
  - agent `LeftoverClock` — `trait LeftoverClock: base::time::Clock { sleep; spawn }` · `mono_now` 22곳 → `now`. 가짜 `FakeClock` · `SkipClock` 은 원자 정수 그대로 자기 자리(C2=a · U6).
- **안 합치는 것(머리에 사유 한 줄):** transport `clock.rs`(C5=b — 「3단계 transport 부착 때 base `time::Clock` 의 하위 트레이트로 — ADR-0275」 · U5) · daemon `usage_service/clock.rs` `UsageClock`(C3=a — §2-2 의 계약 차이 · U5) · agent `CaptureClock`(지금 읽기가 없다 — 주석 불요).

### 3-4. `path`

```rust
/// 사람이 친 경로의 철자만 고른다 — 감싼 따옴표 한 겹(안이 빈 짝 · 두 겹은 그대로)을 벗기고 `\` 를 `/` 로.
/// 파일시스템을 보지 않는다(실재 · 상대경로 판정은 부르는 쪽 몫).
pub fn normalize_spelling(raw: &str) -> String     // P1=a
```

- agent `normalize_cwd` 는 지운다 — agent 등록부와 셸 둘 다 base 를 직접 부른다(ADR-0270 결정 1 — 셸은 agent 를 모른다). 얇은 agent 판을 남기면 셸이 그쪽을 계속 부를 길이 열린다.

### 3-5. `sync`

```rust
pub fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T>
pub fn read<T: ?Sized>(l: &RwLock<T>) -> RwLockReadGuard<'_, T>
pub fn write<T: ?Sized>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T>
pub fn wait_timeout<'a, T>(cv: &Condvar, g: MutexGuard<'a, T>, d: Duration)
    -> (MutexGuard<'a, T>, WaitTimeoutResult)
```

- 꼴 = 자유 함수(S1=a — `usage/process.rs:636` 이 이미 그 꼴이다). 몸은 각각 `….unwrap_or_else(PoisonError::into_inner)` 한 줄이다 — **경고 없음 · 독 표시는 남긴다 · `clear_poison` 을 부르지 않는다**(S2=e). `#[track_caller]` 도 필요 없다(위치를 나를 로그가 없다).
- **계약(모듈 머리에 적는다 · `// ADR-0269` · `// ADR-0275`):**
  1. 독을 걷지 않는다 — 같은 자물쇠를 `.expect` · `try_lock` 으로 쓰는 이웃이 보는 신호를 지우지 않는다(§2-4 섞인 자물쇠 표).
  2. 로그 · 다른 락 · 밖 호출을 하지 않는다 — 도우미가 더하는 동작이 없어 「쥔 채 로그 · IO · 밖 호출 금지」 자물쇠의 사이트를 옮겨도 그 자물쇠들의 규율이 바뀌지 않는다(무엇을 언제 잡는지는 부르는 쪽 그대로 · §2-4).
  3. 운영 빌드는 `panic = "abort"` 라 이 갈래에 닿지 않는다 — debug · 시험 빌드의 복구다.
  - ★경고를 더하고 싶어지면 2가 깨진다★ — 83곳 전체의 「쥔 채 로그 금지」 감사를 다시 해야 한다(§2-4 역사 기록 · ADR-0275).
- **옮기는 꼴:** §2-4 의 꼴 중 클로저 · 함수 포인터 · `match` 갈래 · `wait_timeout` · `read` / `write`. `match` 꼴 6곳(`pty.rs:263,332` · `stdio.rs:317,380` · `command_roster.rs:403` · `command_delivery.rs:1010`)은 갈래 뒤에 붙은 후처리(예: `stdio.rs:317` 의 `.take()`)를 도우미 결과에 그대로 잇는다. 조건변수 `wait_timeout` 운영 3곳 = `input_queue.rs:178 · 254` · codex `transport.rs:2192`(`:10176` 은 시험 모듈).
- **제자리 7곳(운영 90 − 83):**

  | 자리 | 사유 | 「왜 base `sync` 를 안 쓰나」 주석 |
  |---|---|---|
  | base `logging/mod.rs:129` | 같은 crate 의 `sync` 를 부르면 입주자 무참조(게이트 ③)에 걸린다 | U2 에 한 줄 |
  | agent `leftover.rs:1662-1665` `GateCell` 의 `Debug`(`try_lock`) | `WouldBlock` 갈래가 있는 다른 계약이다 — 자물쇠를 기다리지 않는다 | U3a 에 한 줄 |
  | agent `usage/process.rs:636` 지역 도우미(찾기 명단 `LookupGate::release` `:352` 몫) | 1-3 U5 가 찾기 명단을 platform 으로 옮기고 platform 은 base 를 모른다(ADR-0266). `StderrTail` 을 부르는 셋(`:696 · 985 · 1002`)은 `sync::lock` 으로 옮기고, 남는 이 도우미는 `release` 안으로 접는다(워커 제안) | U3a 에 한 줄 |
  | 셸 `settings/mod.rs:478 · 482 · 486`(3) | storage P3 가 이 파일을 고친다 — 1-1 은 손대지 않는다. P3 뒤 D2 에서 셋 다 옮긴다(S2=e 라 `:486` 의 「잎」 규율도 걸리지 않는다) | 없음(파일을 안 건드린다) |
  | 셸 `output_channel.rs:38-45` | ADR-0231 — 경고 + `clear_poison`(「poisoning 한 번에 warn 한 번」)이 그 자리의 계약이다. 옮기면 그 동작을 잃고 시험 `:145-148` 의 `!is_poisoned` 가 빨개진다 | U2 에 한 줄 — 「ADR-0231 — 독을 걷는 자기 도우미, base `sync` 는 독을 걷지 않는다」 |

- 그 밖에 옮기지 않는 것: command `src/testing.rs` 3곳(시험 전용 · ADR-0267) · 시험 전용 24곳(agent 19 · daemon 5 — §5 M3).
- 주석은 위 제자리 자리에만 단다. 옮긴 83곳에는 따로 주석을 달지 않는다 — 4판의 「제외한 자물쇠 정의에 한 줄」 지시는 제외 목록과 함께 없어졌다.

### 3-6. `testing`

```rust
/// `cond` 가 참이 될 때까지 20 ms 마다 본다. 시한이 지나면 마지막으로 한 번 더 보고 그 값을 돌려준다.
pub fn wait_until(timeout: Duration, cond: impl FnMut() -> bool) -> bool
```

- `FnMut` 로 받는다 — `Fn` 사본도 그대로 들어간다.
- 폴링 20 ms(§5 M2) — 사본들은 20 · 25 · 50 ms 로 갈린다. 50 ms 사본(실 PTY · 실 셸 시험)은 조건을 더 자주 본다 — 자식 프로세스를 더 띄우지는 않는다(`--test-threads=4` 사유와 무관).
- `messaging_host.rs:1153` 은 `wait_until(Duration::from_secs(3), cond)` 로 — 횟수 대신 시각(150 × 20 ms ≈ 3 s).
- 쓰는 crate(agent · daemon)는 `[dev-dependencies] engram-dashboard-base = { path = "…", features = ["test-support"] }` 를 더한다(정상 의존과 dev 의존에 같은 crate 를 기능만 달리 적는 것은 command 선례 그대로 — agent `Cargo.toml:41`).

---

## 4. 결정 (2026-10-04)

> **출처(열하나 공통): 사용자 위임(2026-10-04 「알아서 진행해」) → 권고안 채택.** 4판 §4 의 워커 권고를 그대로 골랐다 — 위임 뒤 대조한 결과 열하나 모두 4판 권고와 같은 글자다. 확정된 ADR-0269 와 어긋나는 몫은 ADR-0275 로 고친다(§4-2). 거부한 갈래는 ADR-0275 「거부한 대안」의 재료다. 갈래별 긴 논의는 4판에 있었고(커밋 전) 이 판은 결론만 싣는다.

### 4-1. 결정 기록

| id | 질문 | 결정 |
|---|---|---|
| C5 | transport 에 base 를 붙이나 | **(b)** 이번엔 빼고 3단계(transport 부착 TRD) 때 본다 |
| T1 | UTF-8 자르기를 base 함수로 두나 | **(b)** std `floor_char_boundary` · `ceil_char_boundary` 로 제자리 교체 |
| T2 | hex 루프 셋을 `hex_lower` 로 바꾸나 | **(a)** 바꾼다(감싼 함수는 그대로) |
| C1 | base `Clock` 의 경계 | **(a)** `Send + Sync` |
| C2 | agent `LeftoverClock` 도 합치나 | **(a)** 합친다 — 별도 단위(U6) |
| C3 | daemon `UsageClock` | **(a)** 그대로 둔다 |
| C4 | 가짜 시계의 자리 | **(a)** `time` 안 · 기능 플래그 뒤 |
| S1 | `sync` 의 꼴 | **(a)** 자유 함수 |
| S2 | 복구 경고의 단위 · 독을 걷나 | **(e)** 경고 없이 되찾기만(독 표시는 남긴다) |
| N1 | `testing` 기능 이름 | **(b)** `test-support` |
| P1 | `normalize_cwd` 의 새 이름 | **(a)** `normalize_spelling` |

**C5 = (b).** 이유: 둘 다 확정 결정을 고치는 개정이지만 (b) 는 미루는 개정이라 transport 의 게이트 · 의존 그래프가 그대로다 — transport 는 소비자가 0 이고 3단계에서 의존 · 게이트를 어차피 다시 연다(ADR-0267 과 같은 결). `ws.rs` 의 자르기 사본은 T1=(b) 로 base 없이 사라진다.
- 거부 (a) 붙인다 — transport 게이트 ①(직접 워크스페이스 의존 정확히 1줄 · ADR-0177)을 2줄로 고치는 개정 + `tracing` · `tracing-subscriber` · `regex`(1-3 U1 전이면 `windows` 까지)가 소비자 0 인 재사용 lib 에 실린다.
- 거부 (c) 시계만 붙이고 자르기는 std — (a) 의 대가를 그대로 지고 얻는 것이 시계 하나다.
- 받아들인 대가: 「시간은 공용이면 무조건 합친다」(사용자 2026-09-26)와 어긋난 transport 시계가 3단계까지 남는다 — transport `clock.rs` 머리에 적는다(U5 · §8 D4).

**T1 = (b).** 이유: 감쌀 몫이 없는 std 한 줄이다(ADR-0269 가 `dunce::canonicalize` 를 뺀 사유와 같다) · 뒤 남김은 사본 하나라 결정 7 에도 못 미친다 · C5 와 무관하게 `ws.rs` 까지 한 번에 정리된다.
- 거부 (a) base `text::truncate_bytes` · `keep_tail_bytes` — 이름이 뜻을 말해 주지만 몸이 std 한 줄이고 뒤 남김은 입주 근거(결정 7)가 안 선다.

**T2 = (a).** 이유: ADR-0269 가 「인코딩 루프만 범용」이라고 따로 적은 몫이 정확히 이것이고, 「생성기를 섞지 말라」는 코드 주석과 부딪히지 않는다(§2-1 — ADR-0086 본문엔 공유 금지 문장이 없다).
- 거부 (b) `hex_lower` 를 1-1 에서 뺀다 — 같은 루프 셋이 남고 결정 2 의 표와 어긋난다.

**C1 = (a).** 이유: 「지금 읽기」 seam 대다수(daemon · leftover · transport)가 이미 이 경계이고, 바뀌는 것은 discovery 시험 가짜 하나다.
- 거부 (b) 경계 없음 — 쓰는 자리마다 경계를 적어야 하고 경계 없는 `dyn Clock` 이 생길 수 있다.
- 거부 (c) 읽기 둘(`now` + 벽시계 ms) — 결정 3-1(「지금 읽기 하나」)을 고치는데 1-1 의 소비자는 벽시계를 안 쓴다.

**C2 = (a), 별도 단위(U6).** 이유: 사용자 규칙 「시간은 공용이면 무조건 합친다」의 적용 대상이고, 바뀌는 것은 상위 트레이트와 이름(`mono_now` → `now`)뿐이다. 가짜는 「자물쇠를 잡지 않는다」 계약 때문에 원자 정수 그대로 자기 자리.
- 거부 (b) 둔다 — 규칙에 어긋난 seam 이 남는다.

**C3 = (a).** 이유: 깔끔히 맞지 않는다(§2-2 — `mono` 의 `Duration` 이 book 도메인으로 흐르고 `Duration::MAX` 끝값 시험이 있다 · `wall` 은 부호 있는 epoch 초). 결정 3-4 의 「안 맞으면 그때 묻는다」에 대한 답이다. 사유는 `usage_service/clock.rs` 머리에 적는다(U5).
- 거부 (b) 상위 트레이트만 붙인다 — 아무도 `now()` 를 안 부르는 껍데기 합치기다.
- 거부 (c) `mono` 를 `Instant` 로 — ADR-0250 · 사용량 TRD §3 #28 의 계약 변경 · 끝값 시험 재작성이라 1-1 범위를 넘는다.

**C4 = (a).** 이유: 게이트 ③(입주자 무참조)이 그대로 선다 — `testing` 에 두면 `crate::time::Clock` 을 구현해야 해 걸린다. 대가 = 기능 플래그 하나가 두 모듈(`time` · `testing`)에 걸친다.
- 거부 (b) `testing` 에 두고 게이트 ③ 이 `testing` 을 뺀다 — 게이트에 예외가 생기고 정규식이 길어진다.
- 거부 (c) base 에 두지 않는다 — 결정 3-1 · 5 를 고치고 daemon 의 `Mutex<Instant>` 가짜가 남는다.

**S1 = (a).** 이유: 결정 2(「상태 없는 공개 함수」)와 같고 `usage/process.rs:636` 이 이미 그 꼴이다.
- 거부 (b) 확장 트레이트 `m.lock_or_recover()` — 파일마다 `use` 한 줄 · std `lock` 과 다른 이름 · ADR-0269 「거부한 대안」(도우미를 트레이트 뒤에)과 결이 달라 설명이 필요하다.

**S2 = (e).** 이유: 오늘 117곳 중 116곳이 이미 말없이 되찾아 동작이 로그까지 그대로이고(순수 중복 제거), 경고를 달면 가드를 쥔 채 찍혀 쥔 채 로그 금지 자물쇠 열둘(37곳)을 빼야 했는데 그 목록은 리뷰마다 넓어졌고 지키는 게이트가 없다(§2-4 역사 기록). 잃는 것 = debug · 시험 빌드에서만 나는, 오늘도 116곳에 없는 신호(운영은 `panic = "abort"`). 이전 범위가 46 → 83 으로 넓어져 ADR-0269 의 목적(흩어진 사본을 한 곳으로)을 더 채운다.
- 거부 (a) 복구마다 경고(독은 둔다) — 결정 4 의 문구 그대로지만 제외 목록(열둘 · 37곳)을 지고 가야 하고, 독이 든 뒤 모든 잠금이 경고를 쏟는다.
- 거부 (b) 독 한 번에 한 번(`clear_poison`) — 섞인 자물쇠 일곱(§2-4)에서 이웃(`.expect` · `try_lock`)의 동작이 「그 사이 복구가 돌았나」에 매인다(PTY · stdio `child` 의 shutdown 은 ADR-0001 kill 인과 경로) · 제외 자물쇠와 섞여 한 crate 안에서 걷는 자물쇠와 안 걷는 자물쇠가 갈린다.
- 거부 (c) 사이트마다 한 번 — 매크로가 필요해 결정 2(「상태 없는 공개 함수」)에서 벗어난다 · 제외 목록은 그대로 필요하다.
- 거부 (d) 프로세스당 처음 한 번 — 정적 깃발을 쥐어 결정 2 와 결이 어긋나고 둘째 독부터 기본 레벨에서 안 보인다 · 제외 목록은 그대로 필요하다.
- 거부 「놓고 경고하고 다시 잡는」 도우미 — 부른 쪽이 쥔 바깥 자물쇠 아래서는 여전히 찍힌다(§2-4).
- 받아들인 대가: 확정된 ADR-0269 결정 4 를 고친다(ADR-0275) · 나중에 경고를 다시 달려면 83곳 전체의 제외 감사를 처음부터 다시 한다 · 셸 `output_channel.rs` 는 ADR-0231 의 경고 + 독 걷기를 지켜 제자리다(§3-5).

**N1 = (b).** 이유: command · transport 가 「하네스 도우미 모듈을 연다」는 같은 뜻으로 쓰는 이름이다 — 같은 뜻에 같은 이름.
- 거부 (a) `testing` — 모듈 이름과 같지만 기존 선례와 갈린다.
- 거부 (c) `test-harness` — agent · daemon · messaging 이 「운영 코드 안의 seam 을 연다」는 다른 뜻으로 쓴다.

**P1 = (a).** 이유: ADR-0269 결정 2 · ADR-0270 결정 5 가 이미 적은 가안이라 두 ADR 의 문구가 그대로 맞는다.
- 거부 (b) `normalize_typed_path` — 대상이 이름에 드러나지만 두 ADR 의 문구와 어긋난다.
- 거부 (c) `tidy_typed_path` — 같음.

**묶인 것의 귀결:** C5=(b) · T1=(b) → transport 는 base 를 의존하지 않은 채 `ws.rs` 만 std 로 바뀐다(G3 없음). C1=(a) · C4=(a) → base `ManualClock` 은 `Send + Sync` 를 만족하는 원자 정수 꼴. S2=(e) → 셸 `output_channel.rs` 는 제자리이고 섞인 자물쇠의 처분이 필요 없다 · 제외 규칙 대신 제자리 7곳(§3-5).

### 4-2. ADR-0275 에 적은 개정 (Amends ADR-0269 · ADR-0268)

> **박았다(2026-10-04)** — ADR-0275 는 1-1 몫(이 표 — ADR-0269 개정 + ADR-0268 「영향」의 base 락 오염 복구 경고 줄 개정)과 형제 1-3 몫(ADR-0266 · ADR-0262 개정 — TRD 1-3 §8)을 함께 실은 **한 ADR** 이다. 번호 = **0275**(0274 는 `origin/v0.3.3/feat/storage` 가 쓴다 — 2026-10-04 원격 `git ls-tree` 로 확인. 이 브랜치의 마지막은 0273). 박은 것은 오케스트레이터 · `/adr`(결정 날조 금지 — 아래 거부한 대안 · 이유는 §4-1 과 이 TRD 의 실측에서 왔다). 상태 근거 = 사용자 위임 2026-10-04(「알아서 진행해」 → 권고안 채택) + 이 TRD 의 실측(`0ef6292`). ★ADR-0269 본문에도 「ADR-0275 가 결정 2 · 3 · 4 · 5 와 거부한 대안 한 줄을 고쳤다」는 개정 링크를 단다★ — 옛 ADR 만 읽는 세션이 낡은 문구(경고 로그 · transport 하위 트레이트 · `testing` 의 가짜)를 따라가지 않게(CLAUDE.md 「설계 결정 기록」 — 상태는 양방향). 착수 순서: U1 전에 박는다(U1 이 T1 · 이름 확정에 기댄다 — §6-1).

| # | ADR-0269 의 자리 | 무엇이 바뀌나 | 거부한 대안 | 이유 |
|---|---|---|---|---|
| 1 | **결정 4**(S2=e) | 「복구할 때 `tracing::warn!` 로 경고 로그를 남기는」 도우미 → **경고 없이 되찾기만 하는 도우미(독 표시는 남긴다 · `clear_poison` 없음)**. 「`sync` 가 `logging` 을 부르지 않아 무참조가 선다」는 더 강하게 참이 된다(로그 자체가 없다). 예외 하나 = 셸 `output_channel.rs`(ADR-0231 의 경고 + 독 걷기 — 제자리). ADR-0268 「영향」의 「base 의 락 오염 복구 경고도 `tracing::warn!` 을 직접 부른다」 줄도 같은 결정으로 낡는다(Amends ADR-0268) | (a) 복구마다 경고 · (b) 독 한 번에 한 번(`clear_poison`) · (c) 사이트마다 한 번(매크로) · (d) 프로세스당 한 번 · 놓고 경고하고 다시 잡는 도우미 | 운영 빌드는 `panic = "abort"` 라 신호가 debug · 시험 빌드뿐이다 · 경고는 가드를 쥔 채 찍혀 쥔 채 로그 금지 자물쇠 열둘(37곳)을 빼야 했고 그 목록이 리뷰 세 번마다 넓어졌으며 지키는 게이트가 없다 · 117곳 중 116곳이 이미 말없이 되찾는다 · (b) 는 섞인 자물쇠 일곱의 이웃(`.expect` · `try_lock`) 의미를 바꾼다 · (c) · (d) 는 결정 2 의 「상태 없는 함수」에서 벗어난다. 대가 = 경고를 다시 달려면 83곳 전체의 감사를 다시 한다 |
| 2 | **결정 2 표 `sync` 행 · 「근거」의 57곳** | 흡수 대상 = 실측 117곳(운영 90 · 시험 전용 27) 중 **운영 83곳**. 제자리 = 운영 7(base `logging` · `leftover.rs` `try_lock` · 찾기 명단 도우미 · 셸 `settings/mod.rs` 3(P3 뒤 D2) · 셸 `output_channel.rs`) · command 3(ADR-0267) · 시험 전용 24 | — (수치 확정 — 「근거」가 「1-1 착수 때 다시 센다」고 미뤄 둔 값) | §2-4 · §5 M3 의 셈 |
| 3 | **결정 3-2 · 3-5 의 transport 몫**(C5=b) | transport 가 base 트레이트를 상위로 둔 자기 트레이트를 갖는 일을 **3단계(transport 부착 TRD)로 미룬다**. 1-1 에서 transport 는 base 를 의존하지 않고 transport 게이트 ①(정확히 1줄 · ADR-0177)도 그대로다. 결정 3-2 의 「가짜 하나가 둘을 같은 상태로 구현」은 discovery 몫만 1-1 에 선다. 결정 3-5(tokio 로 잠 · `tokio::time::pause` 는 std `Instant` 를 안 멈춘다)는 사실로 남고 적용은 3단계다 | (a) 지금 붙인다 · (c) 시계만 붙이고 자르기는 std | transport 는 소비자 0 이고 3단계가 의존 · 게이트를 어차피 다시 연다 · (a) 는 게이트 기대값 변경 + `tracing` · `tracing-subscriber` · `regex`(· `windows`) 발자국 · ADR-0267(필요할 때만 연결)과 같은 결 · transport 의 자르기는 std 로 base 없이 해소된다. 대가 = 「시간은 공용이면 무조건 합친다」와 어긋난 seam 이 3단계까지 남는다(transport `clock.rs` 머리에 적는다) |
| 4 | **결정 2 표 `text` 행**(T1=b) | `truncate_bytes` · `keep_tail_bytes` 를 뺀다 → `text` = `hex_lower` 하나. 사본 셋은 std `floor_char_boundary` · `ceil_char_boundary` 로 제자리 교체한다. 「맥락」 표의 「UTF-8 경계 자르기」 행은 사실로 남는다 | (a) base 함수 둘(안은 std 한 줄) | 고정 컴파일러 1.95 의 std 한 줄이라 감쌀 몫이 없다(「거부한 대안」 `dunce::canonicalize` 와 같은 사유) · 뒤 남김은 사본 하나라 결정 7 미달 |
| 5 | **결정 3-1 · 결정 5 · 결정 2 표 `testing` 행**(C4=a · N1=b) | 가짜 `Clock` 의 자리를 `testing` → **`time` 안 `#[cfg(any(test, feature = "test-support"))]`** 로. `testing` 행은 `wait_until` 하나. 기능 이름 = `test-support`(command · transport 와 같은 뜻 · 같은 이름) | (b) `testing` 에 두고 게이트 ③ 에 예외 · (c) base 에 두지 않는다 / 기능 이름 `testing` · `test-harness` | 결정 5 와 결정 6 · 게이트 ③(입주자 무참조)이 부딪힌다 — `testing` 의 가짜는 `crate::time::Clock` 을 구현해야 한다. `test-harness` 는 이 워크스페이스에서 「운영 코드 안의 seam 을 연다」는 다른 뜻이다 |
| 6 | **결정 3 의 seam 목록 · 「맥락」 표의 「네 벌」**(C2=a) | seam 은 여섯이다(+ agent `LeftoverClock` · `CaptureClock`). **`LeftoverClock` 을 base `Clock` 의 하위 트레이트로 합친다**(`mono_now` → `now` · 가짜는 원자 정수 그대로). `CaptureClock` 은 지금 읽기가 없어 합칠 몫이 없다. storage 브랜치의 saver `Clock` 은 P3 뒤 D2 의 몫이다 | (b) 둔다 | 사용자 규칙 「시간은 공용이면 무조건 합친다」(2026-09-26) · 바뀌는 것이 이름과 상위 트레이트뿐이다. 대가 = ADR-0262 의 불변식이 무거운 파일(잔여물 정리)을 기계적 개명이나마 건드린다 — 별도 단위(U6) |
| 7 | **결정 3-4**(C3=a) | `UsageClock` 「보류 — 1-1 때 깔끔히 맞으면 합치고 안 맞으면 묻는다」 → 맞지 않음을 실측으로 확인, **그대로 둔다**(사용자 위임). 사유는 `usage_service/clock.rs` 머리에 | (b) 상위 트레이트만 붙인다 · (c) `mono` 를 `Instant` 로 | `mono` 의 `Duration` 이 book 도메인 타입으로 흐르고 `Duration::MAX` · `ZERO` 끝값 시험이 있다 · `wall` 은 부호 있는 epoch 초다 · (b) 는 아무도 `now()` 를 안 부르는 껍데기 · (c) 는 ADR-0250 · 사용량 TRD 계약 변경이라 1-1 범위를 넘는다 |
| 8 | **「거부한 대안」 `sha256_hex` 줄** | 사유 「`sha2` 를 끌고 온다」 → **「사본이 하나라 결정 7 에 못 미친다」**. 결론(입주 안 함)은 그대로. 안의 hex 루프는 `hex_lower` 를 부른다(T2=a) | — (사실 정정) | std 손구현이다 — `record.rs:247-251` 주석 · `grep -n sha2 crates/*/Cargo.toml src-tauri/Cargo.toml` → 0줄 |
| 9 | **세부 확정**(개정 아님 — 결정 2 「함수 이름은 가안」 · 결정 3 「세부 모양은 1-1 착수 때 정한다」를 채운 것. ADR-0275 에 한 줄씩 적어 둔다) | `Clock: Send + Sync`(C1=a) · `sync` = 자유 함수 넷(S1=a) · 이름 `hex_lower` · `normalize_spelling` 확정(T2=a · P1=a) · `wait_until` 흡수 대상은 9개 파일이 아니라 사본 11벌(+ daemon `ws_e2e.rs` `poll_until` · `messaging_host.rs`) | C1 (b) 경계 없음 · (c) 읽기 둘 / S1 (b) 확장 트레이트 / P1 (b) `normalize_typed_path` · (c) `tidy_typed_path` | §4-1 의 각 줄 |

- **고치지 않는 것:** 결정 1 · 6 · 7 · 8 · 9 · 「영향」(게이트 ③ 에 모듈 이름을 더한다는 문장 포함)은 이 결정들과 부딪히지 않는다. ADR-0177(transport 게이트)은 C5=(b) 라 바뀌지 않는다. ADR-0231(출력 Channel 명부의 경고 + 독 걷기)은 그대로 서고 base `sync` 의 예외로 남는다.

---

## 5. 메인 판단 (2026-10-04 · ADR-0269 결정 7 — 묻지 않고 보고)

> **출처: 메인 판단(2026-10-04).** 아래 다섯은 ADR-0269 결정 7(입주 판단은 메인이 하고 사용자에게 묻지 않고 보고한다)의 몫이라 사용자 위임(§4)이 아니라 메인이 정했다. 4판까지 워커 제안이던 안을 그대로 채택했다 — 내용은 바뀌지 않았다.

- **M1 — 시험 임시 폴더 도우미:** 「태그 + PID + 시각으로 고유한 임시 폴더 경로」가 9벌 있다(base `logging/mod.rs:448` · discovery `lib.rs:2619` · net `portfile.rs:94` · `instance.rs:349` · 셸 `settings/tests.rs:20` · `settings/store.rs:1193` · agent `backend/claude/session_file.rs:232` · `persistence/mod.rs:162` · `persistence/presets.rs:134`). 결정 7 로는 입주 대상이다. **1-1 에 넣지 않는다**(메인 판단 2026-10-04) — net 이 `testing` 을 부르면 net 의 「base 심볼 정확히 2줄」 게이트가 움직이고(시험 모듈도 `src/` 안이다), 셸 둘은 storage 가 고치는 파일이며, base `logging` 의 사본은 게이트 ③ 때문에 못 쓴다. P3 · 1-3 뒤에 따로.
- **M2 — `wait_until` 폴링 20 ms 고정**(사본 20 · 25 · 50 ms). 인자로 받지 않는다.
- **M3 — `sync` 이전 범위 = 운영 사이트 83곳(S2=e).** 시험 전용(agent 19 · daemon 5)은 그대로 둔다 — 옮겨도 얻는 것이 없다(경고가 없어 시험 출력도 같다). 옮기기로 하면 그 단위에 더하면 된다 — 제외 규칙이 없어 막는 것이 없다.
  - 셈: 117 = 운영 90(agent 68 · daemon 8 · 셸 13 · base 1) + 시험 전용 27(agent 19 · daemon 5 · command 3). 운영 중 제자리 7 = base `logging` 1 · `leftover.rs:1664` `try_lock` 1 · 찾기 명단 도우미(`usage/process.rs:636`) 1 · 셸 `settings/mod.rs` 3 · 셸 `output_channel.rs` 1(§3-5) → 90 − 7 = **83**(agent 66 · daemon 8 · 셸 9).
  - 단위별: U2 17(daemon 8 · 셸 9) · U3a 24(agent 의 codex 통로 밖) · U3b 42(codex `transport.rs` — 상태 락 26 포함) · §6.
  - 4판까지의 수(1판 85 → 2판 80/81 → 3판 47/48 → 4판 46/47)는 경고를 다는 갈래의 제외 목록이 리뷰마다 넓어진 기록이다. 4판 46 과 83 의 차 37 = 쥔 채 로그 금지 자물쇠 열둘의 복구 사이트(agent 33 · daemon 4 — §2-4 역사 기록).
- **M4 — 패닉 변형 다섯은 제자리**(§2-5) — 라벨 · 패닉 · 폴링이 각자 근거를 가진다.
- **M5 — `now_epoch_ms` 의 persistence 두 벌도 1-1 에서 바꾼다** — 그 두 파일의 쓰임은 손상 사본 이름 하나뿐이라 P3 뒤 `set_aside_corrupt` 가 그 시각을 인자로 받게 되면 다시 움직이지만, 지금 바꿔도 storage 브랜치와 겹치지 않는다(storage 는 `crates/` 를 안 고친다 — `git diff --stat <merge-base> origin/v0.3.3/feat/storage -- crates` → 0).

---

## 6. 작업 단위

> **지시서 공통 문구(코더에게):** 「중간 어디서 멈춰도 빌드가 서게 순서를 짜라」 — **base 에 새 함수를 먼저 더하고(옛 사본과 공존) → 호출부를 파일 단위로 갈아 끼우고 → 옛 사본을 마지막에 지운다.** 옛 자료구조를 먼저 지우는 순서는 금지. 주석은 `/code-conventions` 의 주석 규약 파일을 주입한다.
>
> **되돌릴 지점:** 코드 트리는 출발점 `0ef6292` 그대로다(미추적은 이 TRD 폴더뿐). U1 전에 이 TRD 를 로컬 커밋해 출발점으로 삼고(§6-1), 단위마다 게이트 초록 뒤 로컬 커밋으로 스냅숏을 남긴다.
>
> **단위마다 검증(공통):**
> 1. `cargo test --workspace -- --test-threads=4` 를 단위 **전후로** 돌려 **`test result:` 줄 수와 통과 총계를 둘 다** 견준다 — 타깃 소실은 실패가 아니라 침묵이다(CLAUDE.md 「빌드·검증 명령」). 기대: 줄 수 = 그대로(base 새 시험은 `src/` 안 단위 시험으로만 둔다 — `tests/` 에 파일을 더하면 +1) · 통과 = 전 + base 새 시험 수(옮긴 시험은 이쪽 −N · base +N 으로 상쇄). ★출발 수치는 재지 않았다★ — 마지막 기록은 CLAUDE.md 의 2026-10-01(`cac9ee0`) 59줄 · 3822 통과 · 30 무시다. U1 착수 때 잰다(§6-1 의 명령).
> 2. `cargo test -p engram-dashboard-base -- --test-threads=4` · `cargo fmt --check` · base 게이트 ①②③(§7) · 바뀐 crate 의 `-p` 시험(플래그 규칙은 CLAUDE.md 「병렬은 테스트 바이너리마다 걸린다」).
> 3. 셸이 바뀌는 단위는 `cargo test -p engram-dashboard --test lib_unit` 과 해당 통합 타깃(`--test layout_commands` 등)을 따로 돈다.
> 4. 소스가 바뀌므로 커밋 전 `/qa`(범위는 바인딩이 정한다). **동작은 로그까지 그대로다** — S2=(e) 의 도우미는 독 표시를 건드리지 않고 로그를 내지 않으며, 옮기는 83곳은 이미 말없이 되찾던 자리다(117곳 중 경고를 내는 유일한 자리 `output_channel.rs` 는 제자리). T1 의 std 교체는 손 루프와 결과가 같다(§2-1). 운영 빌드는 `panic = "abort"` 라 복구 갈래에 아예 안 닿는다. 그래서 GUI 실측이 필요한 동작 변화는 없다 — 단 U1 의 셸 `layout/commands.rs` 는 `agent.spawnInto` 의 cwd 철자에 닿는다(동작 동일 · 회귀망 = 옮긴 시험 + `layout_commands`).
> 5. **소스 문자열 시험(실측 — `rg -n 'include_str!\(' crates src-tauri/src` 의 원천 파일 읽기 21곳 + `settings/tests.rs:1321` 의 폴더 훑기 하나를 다 읽었다):** 1-1 의 편집에 **깨지는** 것은 codex `transport.rs` 의 `the_production_blocking_stdin_locks_are_counted`(`:7329-7375`) 하나다 — U3b. 닿지만 안 깨지는 것: `manager.rs:3953` 의 핸드셰이크 실패 갈래 순서 시험(U3b 가 고치는 `:2751-2753` 이 그 갈래 안이다 — 재는 바늘은 `tracing::warn!(…headline())` 과 `deliver_link(` 의 순서뿐) · `manager.rs:6374` `squashed_production_body`(바늘 `self.sessions.write()` 는 `.expect` 쪽이라 U3a 가 안 고친다) · `profile.rs:1167`(U1 의 `now_millis` 교체가 바늘 `p.epoch` · `replace_session_id(` 를 건드리지 않는다). ★5판 재대조(S2=e 로 넓어진 몫):★ 원천을 읽는 시험이 있는 파일 중 새로 손대는 것은 codex `transport.rs`(상태 락 26곳)와 `manager.rs`(`:550`)뿐이고, 그 시험들의 바늘(`clip(` · `s.turn = TurnState::Idle` · `.close_turn_items(` · `ChildGuard(Some(child))` · `record_session_id(` · `hydrate_history(` · `HANDSHAKE_BUDGET` · `s.closed = true;` · `self.pending.close()` / `manager.rs` 의 `_reservation` · `spawn_fresh_settled` · `note_activation_result` · `.with_session_id_latch(` 등)은 락 복구 꼴을 재지 않는다. 잔여물 정리 · 대기 입력 명부 · 대기 목록 표 · 래치 · 턴 관측 표 · daemon 넷(`command_roster.rs` · `agent_conn.rs` · `usage_service/{watch,mod}.rs`)에는 원천을 읽는 시험이 없다. CI · qa 바인딩의 `rg` 게이트에는 락 복구 꼴을 훑는 줄이 없다(`grep -n 'into_inner\|lock()\|poison' .github/workflows/ci.yml .claude/skill-bindings/qa.md` → 0).

| id | 범위(사이트 수) | 건드리는 파일 | 선행 |
|---|---|---|---|
| **U1** | base 입주 규칙 문서(헤더 · Cargo `description`) + `text::hex_lower`(루프 셋) + `time::now_epoch_ms`(사본 셋) + `path::normalize_spelling`(정의 1 · 부르는 곳 2 · 시험 둘 이사) + UTF-8 자르기 셋을 std 로(T1=b) | §6-1 이 정본(파일 · 줄) — 요약: base `lib.rs` · `Cargo.toml` · 새 `text.rs` · `time.rs` · `path.rs` / agent `commands.rs` · `profile.rs` · `persistence/{mod,presets}.rs` · `output_core.rs` / daemon `lib.rs` · `control/mod.rs` · `experiment/record.rs` / transport `ws.rs` / 셸 `layout/commands.rs` · `tests/layout_commands.rs`(주석) / 게이트 ③ 6곳 · CLAUDE.md · `docs/testing-strategy.md` · 루트 `Cargo.toml` `:2` · `:12-13` | ADR-0275 박제 |
| **U2** | `sync` 모듈(함수 넷) + daemon 운영 **8** + 셸 운영 **9** = **17** · 제자리 주석 둘 | base `lib.rs` · `Cargo.toml`(`description`) · 새 `sync.rs` · `logging/mod.rs`(`:129` 제자리 주석 — 게이트 ③. ★그 주석에 `crate::` · `super::` + `sync` 철자를 쓰지 않는다 — 게이트 ③ 이 주석까지 잡는다★) / daemon `command_delivery.rs`(`:1008` — `lock_for_cleanup`) · `command_roster.rs`(`:401` — `lock_for_cleanup`) · `agent_conn.rs`(`:213` — `slot`) · `usage_service/mod.rs`(`:571` `save_lock` · `:606` `desk` · `:648` 조회 가드 칸) · `usage_service/reject_store.rs`(`:276`) · `usage_service/watch.rs`(`:205` — `lock`) / 셸 `view_commands.rs`(7 — `:403 · 410 · 455 · 527 · 552 · 758 · 790`) · `daemon_client/mod.rs`(`:88`) · `ui_settings.rs`(`:768`) · `output_channel.rs`(제자리 + 「ADR-0231 — 독을 걷는 자기 도우미, base `sync` 는 독을 걷지 않는다」 한 줄 주석) / 게이트 ③ 6곳(`+sync`). ★`lock_for_cleanup` 두 짝(`command_roster` · `command_delivery`)은 이름 · 「정리 경로 전용」 문서를 그대로 두고 몸만 `sync::lock` 으로 — `.expect` 쪽 `lock()` 과 일부러 갈라 둔 짝이고 (e) 는 독을 걷지 않아 그 가름이 그대로 선다★ | U1(base 헤더) |
| **U3a** | agent `sync` — codex 통로 밖 운영 **24** · 제자리 주석 둘 | `transport/input_queue.rs` 7(`wait_timeout` `:178 · 254` 포함 — `:263` 은 `#[cfg(test)] fn queued_bytes` 라 안 바꾼다) · `transport/stdio.rs` 3(`:194` stdin · `:317` · `:380`) · `transport/pty.rs` 2(`:263 · 332` — `match` 꼴) · `reaper.rs` 2(`read` `:154-156` · `write` `:52-55`) · `session.rs` 2 · `manager.rs` 2(`session_id_sink` 의 `expected` 칸 `:550` · `lock_name_allocation` `:1061`) · `backend/claude/leftover.rs` 2(`Recorder.inner` `:402` · `GateCell.state` `:1713`) + 제자리 주석 1(`:1662-1665` `Debug` 의 `try_lock`) · `queued_input.rs` 1(`:405`) · `inputs_pending.rs` 1(`:47`) · `session_id_latch.rs` 1(`:150`) · `turn.rs` 1(`:145` `TurnObservations::lock`) · `usage/process.rs`(복구 사이트 0 — `StderrTail` 을 부르는 셋 `:696 · 985 · 1002` 를 `sync::lock` 으로 바꾸고, 찾기 명단 몫만 남는 지역 도우미 `fn lock`(`:636`)은 `LookupGate::release`(`:352`) 안으로 접어 「1-3 U5 로 간다 · platform 은 base 를 모른다(ADR-0266)」 한 줄 주석 — 워커 제안). ★잔여물 정리 잎 둘 · 대기 입력 명부 · 대기 목록 표 · 래치 · 턴 관측 표는 「쥔 채 로그 · 다른 락 · 밖 호출 금지」 자물쇠다 — (e) 의 도우미는 로그도 다른 락도 밖 호출도 없어 그 규율 안에 든다(§3-5 계약 2). 자물쇠 문서는 고치지 않는다★ | U2 |
| **U3b** | agent `sync` — `backend/codex/transport.rs` 운영 **42** + ★소스 개수 시험 넓히기★ | 그 파일 하나. 운영 구획 = `:4284`(시험 모듈 시작) 앞. ★바꾸는 42곳 = 상태 락 26(`SharedState` `:1079` — `lock.lock()` 25: `:499 · 1115 · 1144 · 1176 · 1572 · 2000 · 2175 · 2230 · 2348 · 2461 · 2638 · 2972 · 3176 · 3254 · 3334 · 3399 · 3524 · 3589 · 3645 · 3715 · 3921 · 3949 · 4079 · 4171 · 4227` + 라이터 `wait_timeout` `:2192`) · `Announcer.order` 1(`:1112`) · `Pending.entries` 5(`:1298 · 1316 · 1328 · 1333 · 1338` — `:1349` 는 `#[cfg(test)] fn len`) · `stdin` 2(`:1555 · 2752`) · `child` 2(`:3889 · 4233`) · `stdout` · `stderr` · `open_params` · `link_sink` · `writer_handle` · `decoder` 각 1(`:3999 · 4009 · 4038 · 4052 · 4073 · 4092`)★. 「★상태 락을 쥔 채 찍지 않는다★」(`:3010-3015`) 규율은 그대로 지켜진다 — 도우미가 찍지 않는다. ★`tests::the_production_blocking_stdin_locks_are_counted`(`:7329-7375`)는 `stdin` 바로 뒤의 `.lock()` 을 세어 **정확히 2** 를 기대한다(교착 가드 — `write_line` `:1555` 과 핸드셰이크 실패 갈래 `:2751-2753`). U3b 가 그 둘을 새 꼴로 바꾸면 0 이 되어 빨개진다. **matcher 를 새 호출 꼴로 넓히고 기대값 2 는 그대로 둔다** — `sync::lock(` 뒤 `&` 를 건너뛴 `stdin`(`write_line` 은 `&Mutex` 를 받아 `sync::lock(stdin)` 이 된다 · S1=a). 옛 `.lock()` 꼴도 계속 센다(되돌아오면 그것도 블로킹이다). 「숫자만 고치지 말 것」은 그 시험 주석의 지시다 — 기대값을 0 으로 내리면 셋째 블로킹 자리가 생겨도 초록인 눈먼 가드가 된다. 같은 파일 `:2741-2750` 의 주석(「블로킹으로 이 락을 잡는 자리는 둘」)은 그대로 참이다★ | U2 |
| **U4** | `testing` 모듈(`test-support` 기능) + `wait_until` + 사본 11벌 + dev 의존 + 운영 그래프 게이트(§7 G2) | base `lib.rs` · `Cargo.toml`(`[features] test-support = []`) · 새 `testing.rs` / agent `Cargo.toml` · `tests/{activation,transport_smoke,session_smoke,stdio_smoke,headless,backend_contract,reaper}.rs` / daemon `Cargo.toml` · `tests/{control_send,mcp_manager_lifecycle,ws_e2e}.rs` · `src/messaging_host.rs` / `ci.yml` · CLAUDE.md · qa 바인딩 · 게이트 ③ 6곳(`+testing`) | U1 |
| **U5** | `time::Clock`(`Send + Sync`) + `SystemClock` + `ManualClock`(`test-support` 뒤 · C4=a) + daemon `command_delivery` + discovery + 안 합치는 시계 둘의 머리 주석 | base `time.rs` · `Cargo.toml`(`description`) / daemon `command_delivery.rs`(`Clock` `:176` · `SystemClock` `:181` 삭제 → base · 시험 `ManualClock` `:2425` → base) · `connection_core.rs`(`:4403` 시험) · `usage_service/clock.rs`(머리 주석 — C3=a: 「base `time::Clock` 과 합치지 않는다 — `mono` 의 `Duration` 계약 · 부호 있는 epoch 초 · ADR-0275」) / discovery `lib.rs`(`Clock` `:431` → base 하위 트레이트 + 막는 `sleep` · 가짜 `FakeClock` `:1742` 의 `RefCell` · `Cell` 을 `Sync` 인 꼴로 — C1=a) · `Cargo.toml:12-15`(base 의존 주석 — 사유를 「base `time::Clock` 상위 트레이트」로. 1-3 U1 이 그 줄의 「liveness 판정 공유」 사유를 platform 줄로 옮기므로 이 단위가 안 적으면 base 줄에 사유가 없다 — 1-3 이 먼저 착지하면 덧붙이고, 1-1 이 먼저면 「liveness」 사유 곁에 더한다 · 3차 리뷰) / transport `src/clock.rs`(머리 주석 — C5=b: 「3단계(transport 부착)에서 base `time::Clock` 의 하위 트레이트로 — ADR-0275」 · 코드는 안 바꾼다) | U4(`test-support` 기능) · U2(`command_delivery.rs` 겹침) |
| **U6** | `LeftoverClock` 을 base `Clock` 의 하위 트레이트로(C2=a) — `mono_now` 22곳 → `now` | agent `backend/claude/leftover.rs` 하나(`LeftoverClock` `:953` · 운영 `SystemClock` `:961` · 가짜 `FakeClock` `:4664` · `SkipClock` `:6586` 은 원자 정수 그대로 자기 자리 — base `Clock` 을 함께 구현한다). ★「문 자물쇠 안에서도 부른다 — 다른 자물쇠를 잡거나 기다리지 않는다」(`:954`) 계약은 `now` 로 옮겨 적는다★ | U5 |

- **순서:** U1 → U2 → (U3a ∥ U3b) → U4 → U5 → U6. 어느 단위 뒤에서 멈춰도 워크스페이스는 빌드되고 초록이다 — 각 단위가 새 함수를 더한 뒤 그 단위 안에서 옛 사본까지 지우기 때문이다.
- **크기(사이트 수 기준 — 파일 겹침이 축):** U3b 는 한 파일 42곳의 기계적 교체 + 시험 하나라 한 워커에 묶는다(쪼개면 같은 파일이 겹친다). U3a 는 파일 열둘에 24곳 — 파일이 서로 안 겹쳐 둘로 갈라도 되지만 각 몫이 작아 묶는 쪽을 제안한다(워커 제안).
- **파일 겹침(병렬 판단용):**
  - base `lib.rs` · `Cargo.toml` · 게이트 ③ 줄(6곳) — U1 · U2 · U4 가 만지고 U5 는 `description` 만 → **직렬.**
  - daemon `command_delivery.rs` — U2(복구 1곳) · U5(시계 · 가짜) → 직렬.
  - agent `backend/claude/leftover.rs` — U3a(복구 2곳 · 주석) · U6(시계) → 직렬.
  - U3a 와 U3b 는 파일이 겹치지 않는다 → 병렬 가능(같은 crate 라 한 워크트리에서 동시 빌드는 서로 막는다 — 워크트리 격리 여부는 오케스트레이터 몫).
  - agent · daemon `Cargo.toml` — U4 만.
  - discovery `lib.rs` · `Cargo.toml` — U5 만. transport — U1(`ws.rs`) · U5(`clock.rs` 주석).
- **형제 TRD(1-3 platform)와 겹치는 파일 — 1-1 과 1-3 을 동시에 돌리지 말 것을 권한다:** base `lib.rs` · `Cargo.toml` · 게이트 ③(1-3 이 `platform` 을 뺀다) · agent `transport/pty.rs` · `transport/stdio.rs` · `backend/codex/transport.rs`(1-3 = Job 핸들 분기 · 1-1 = U3) · agent `usage/process.rs`(1-3 = U3 · U5 — 찾기 명단을 포함한 사용량 조회의 OS 층 · 1-1 = U3a 의 `StderrTail` 셋 + 제자리 주석) · agent `backend/claude/leftover.rs`(1-3 = U3 · 1-1 = U3a · U6) · agent `manager.rs`(1-3 = OS 규칙 :77 · 1-1 = U3a) · discovery `lib.rs`(1-3 = `wmi_spawn` 이사 · 1-1 = U5 — ★1-3 은 discovery 의 base 의존(`Cargo.toml:15`)을 걷지 않는다: U5 뒤 `time::Clock` 이 그 쓰임이다 · §3-3★) · discovery `Cargo.toml:12-15`(1-3 U1 = 「liveness 공유」 사유를 platform 줄로 · 1-1 U5 = base 줄에 `time::Clock` 사유 — 3차 리뷰) · 루트 `Cargo.toml` 멤버 주석 `:2` · `:12-13`(1-1 = U1 이 입주 규칙 문장을 고친다 · 1-3 = `platform` 이 빠진 몫을 다시 고친다 — 넘겨줌) · daemon `lib.rs`(1-3 = 실행 파일 이름 분기 :109 · 1-1 = U1 hex) · agent `persistence/{mod,presets}.rs` · daemon `usage_service/reject_store.rs`(1-3 의 `write_atomic` — 단 그쪽은 P3 뒤).

### 6-1. U1 착수 체크리스트

0. **전제 둘.** ① ADR-0275 를 먼저 박는다 — ★2026-10-04 에 박았다(1-3 몫과 한 ADR)★(오케스트레이터 · `/adr` — §4-2. U1 은 그중 4번(T1 — `text` 표에서 자르기 둘을 뺀다)과 9번(이름 확정)에 기대고, base 머리가 ADR-0269 를 가리키므로 개정이 먼저 서야 앵커가 맞는다). ② 되돌릴 지점 — 이 TRD 폴더가 미추적이다(`?? docs/process/S21-crate-boundaries/`). 착수 전에 로컬 커밋해 출발점을 만든다(코드 트리는 `0ef6292` 그대로).
1. **출발 수치(코드 손대기 전).** 워크스페이스 루트에서:
   ```
   cargo test --workspace -- --test-threads=4 > <scratch>/u1-before.txt 2>&1
   awk '/^test result:/{n++; for(i=1;i<NF;i++){if($(i+1)~/^passed/)p+=$i; if($(i+1)~/^failed/)f+=$i; if($(i+1)~/^ignored/)g+=$i}} END{print n" 줄 · "p" 통과 · "f" 실패 · "g" 무시"}' <scratch>/u1-before.txt
   ```
   결과 줄 수 · 통과 총계(+ 실패 · 무시)를 그대로 적는다. 마지막 기록(CLAUDE.md 2026-10-01 `cac9ee0` = 59 · 3822 · 0 · 30)과 다르면 차이만 적는다 — 출발점이 그 뒤의 master 다. 터미널이 죽으면 `scripts/run-detached.ps1`(qa 바인딩 「분리 실행」)로 돌린다.
2. **손댈 파일(정확히):**
   - base — `crates/engram-dashboard-base/src/lib.rs`(머리: 입주자 목록에 `text` · `time` · `path` · 입주 조건 ①을 ADR-0269 결정 7 로 · `:15-19` 「셋째 입주자를 받기 전에 ADR-0175 재론」 문단을 결정 1 이 답한 것으로 · `pub mod text; pub mod time; pub mod path;` · 게이트 ③ `:40` · `// ADR-0269`) · `Cargo.toml:5`(`description`) · 새 `src/text.rs`(`hex_lower` + 단위 시험) · 새 `src/time.rs`(`now_epoch_ms` 하나 — `Clock` 은 U5 · 단위 시험) · 새 `src/path.rs`(`normalize_spelling` + agent 에서 옮겨 온 시험 둘 · 도메인 말을 뺀 계약 문서).
   - agent — `src/commands.rs`(`normalize_cwd` `:1143-1153` 삭제 · 부르는 곳 `:1106` → base · 시험 `:2091` · `:2106` 을 base `path.rs` 로 이사 · `:2114` 이후 「두 문이 같은 철자」 시험은 남김 · 「사람 · LLM 이 친 cwd」 · ADR-0122 같은 도메인 사유는 호출 자리 주석으로) · `src/profile.rs`(`now_millis` `:20-25` 삭제 · `:244` · `:819` → `now_epoch_ms`) · `src/persistence/mod.rs`(`:25-30` 삭제 · `:89`) · `src/persistence/presets.rs`(`:22-27` 삭제 · `:81`) — 세 파일의 `use std::time::{SystemTime, UNIX_EPOCH};`(`profile.rs:12` · `mod.rs:11` · `presets.rs:12`)가 쓰임을 잃으면 함께 지운다 · `src/output_core.rs:1059-1062`(`ceil_char_boundary` — §3-2).
   - daemon — `src/lib.rs:77-80`(`generate_token` 안 루프 → `hex_lower`) · `src/control/mod.rs:146-149`(`gen_token` 안 루프) · `src/experiment/record.rs:178-186`(`cap_response` → `floor_char_boundary`) · `:192-195`(`sha256_hex` 안 루프 → `hex_lower`). 토큰 생성기 둘의 「공용화 금지 · 혼용 금지」 주석(`lib.rs:584-585` · `control/mod.rs:140-141`)은 그대로 — 생성기는 섞지 않는다.
   - transport — `src/ws.rs:147-156`(`clamp_close_reason` → `floor_char_boundary`). `Cargo.toml` 은 안 건드린다(base 의존 없음 — C5=b).
   - 셸 — `src-tauri/src/layout/commands.rs:38`(import → `engram_dashboard_base::path::normalize_spelling`) · `:1127`(`verb_spawn_into`) · `src-tauri/tests/layout_commands.rs:2121`(주석의 이름). 셸은 이미 base 를 의존한다(`src-tauri/Cargo.toml:61`).
   - 문서 · 게이트 — 게이트 ③ 정규식 6곳을 `(crate|super)::(logging|platform|text|time|path)` 로: base `lib.rs:40` · `CLAUDE.md:249` · `.github/workflows/ci.yml:644` · `.claude/skill-bindings/qa.md:138` · `docs/testing-strategy.md:50` · `:155`. `CLAUDE.md` 「백엔드 모듈 맵」 base 항목(입주자 · 입주 조건 · 「셋째 입주자를 받기 전에 ADR-0175 … 다시 연다」 문장) · 「빌드·검증 명령」 base 시험 줄 설명. `docs/testing-strategy.md:43`(단위 시험 목록) · `:46` · `:138`. 루트 `Cargo.toml:2` · `:12-13`(base 서술 · 「셋째 입주자는 ADR 재론」). ★CLAUDE.md · qa 바인딩 · testing-strategy 는 load-bearing 문서다 — 그 단위의 리뷰에 doc 렌즈를 함께 건다★. ★새 모듈 머리 · 주석에 `crate::` · `super::` 뒤에 입주자 이름을 붙인 철자를 쓰지 않는다 — 게이트 ③ 정규식은 주석까지 잡는다(예: 「`crate::time` 을 부르지 않는다」라고 적으면 그 줄이 걸린다)★.
3. **순서(멈춰도 빌드가 서게):** base 새 모듈 셋 + 시험 → 호출부를 파일 단위로(agent → daemon → transport → 셸) → 옛 사본(`now_millis` 셋 · `normalize_cwd`) 삭제 → 문서 · 게이트 ③.
4. **게이트(U1 뒤):** base ①(`cargo tree -p engram-dashboard-base --depth 1 --prefix none -e normal,dev,build --target all --all-features | rg "^engram-dashboard" | sort -u` → 정확히 1줄) · ②(`rg "^\s*use tauri" crates/engram-dashboard-base/src/` → 0줄) · ③(새 정규식 → 0줄 · 경로 존재 먼저 — qa 바인딩 4b-pre) · transport 게이트 ①(같은 꼴 `-p engram-dashboard-transport` → 정확히 1줄 — `ws.rs` 를 건드렸으니 다시 본다) · `cargo fmt --check` · `cargo test -p engram-dashboard-base -- --test-threads=4` · `cargo test -p engram-dashboard-agent -- --test-threads=4` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` · `cargo test -p engram-dashboard-transport` 와 `--all-features`(CI transport 게이트 4a · 4b 와 같은 두 조합) · `cargo test -p engram-dashboard --test lib_unit` · `--test layout_commands`.
5. **수치(U1 뒤):** 1번 명령을 다시 돌려 `<scratch>/u1-after.txt` 와 견준다 — 기대: 결과 줄 수 = 전과 같음 · 통과 = 전 + base 새 단위 시험 수(옮긴 `normalize_cwd` 시험 둘은 agent −2 · base +2 로 상쇄) · 실패 0 · 무시 같음. 줄 수가 줄면 타깃 소실이다 — 초록이어도 멈춘다.
6. `/qa`(소스 변경) → 게이트 초록 뒤 커밋(`S21: refactor(base): …` — 스텝 번호는 step-log 를 잇는다 · 끝에 Co-Authored-By 트레일러).

---

## 7. 게이트 · 문서 변경 목록 (이 TRD 는 편집하지 않는다 — 목록만)

| id | 무엇 | 자리 | 언제 |
|---|---|---|---|
| G1 | 입주자 무참조 정규식에 새 모듈 이름을 더한다 — `(crate\|super)::(logging\|platform)` → `(…\|text\|time\|path)` (U1) → `+sync`(U2) → `+testing`(U4). 모듈을 만드는 단위에서 그 이름을 더한다(게이트가 실재 입주자만 말하게). 1-3 이 `platform` 을 뺀다 | `crates/engram-dashboard-base/src/lib.rs:40` · `CLAUDE.md:249` · `.github/workflows/ci.yml:644` · `.claude/skill-bindings/qa.md:138` · `docs/testing-strategy.md:50` · `:155`(1차 리뷰가 찾은 사본 둘 — 6곳) | U1 · U2 · U4 |
| G2 | **새 게이트(제안):** base 의 시험 기능이 운영 그래프에 없다 — `cargo tree --locked -p engram-dashboard-daemon -e normal,features -i engram-dashboard-base --target all \| rg 'feature "test-support"'` → 0줄, `-p engram-dashboard`(셸) 도 같음. 짝으로 눈먼 게이트 방지: `-e normal,dev,features -p engram-dashboard-agent` → 1줄 이상(기능 이름을 바꾸면 0 기대가 조용히 통과하므로 — transport 게이트 ⑤와 같은 근거). 같은 기능 뒤의 `time::ManualClock`(C4=a)도 이 게이트가 함께 덮는다 | `ci.yml` 새 스텝 · `CLAUDE.md` 「빌드·검증 명령」 · qa 바인딩 | U4 |
| — | **바뀌지 않는 것:** base 게이트 ①(정확히 1줄) · ②(`use tauri` 0줄) · transport 게이트 ①(직접 워크스페이스 의존 정확히 1줄 — C5=b 라 4판의 G3(1 → 2줄)은 없어졌다 · 3단계 TRD 가 다시 본다) · net 의 base 심볼 정확히 2줄(net 은 1-1 에서 base 를 새로 안 부른다) · command · messaging 상한 게이트(ADR-0267) · transport 게이트 ③(순수 층 셋은 시계를 인자로 받는다) · base CI 시험 스텝 `ci.yml:276-278`(`testing` 이 `cfg(any(test, …))` 라 기능 인자가 필요 없다) | | |

문서(같은 변경에서):

- base `lib.rs` 헤더 — 입주자 목록 · 입주 조건 ①을 ADR-0269 결정 7 로 · 「셋째 입주자 재론」 문단 정리 · 게이트 ③ 이름 · `// ADR-0269` 앵커(U1).
- base `Cargo.toml` `description`(U1 · 이후 단위마다 모듈 추가).
- 루트 `Cargo.toml` 멤버 주석 — `:2` 「base(바닥 인프라 — 로깅·PID 판정, 잎 crate, ADR-0175)」 · `:12-13` 「ADR-0175 결정 1: 로깅 + PID 판정만 담는 잎 crate … 셋째 입주자는 ADR 재론을 거친다」(U1 — base `lib.rs:15` 의 재론 문단 · CLAUDE.md base 항목과 같은 문장이라 같은 변경에서 낡는다. 2차 리뷰). ★1-3 이 `platform` 을 base 에서 빼며 「PID 판정」 몫을 다시 고친다★ — 1-1 은 입주 규칙 · 새 입주자 몫만 고치고 나머지는 1-3 에 넘긴다.
- `CLAUDE.md` 「백엔드 모듈 맵」 base 항목(입주자 · 입주 조건) · 「빌드·검증 명령」 base 시험 줄 설명(U1) · G2(U4).
- `docs/testing-strategy.md` base 절 — `:43` 단위 시험 목록(`logging` · `platform` 뒤에 새 모듈) · `:46` · `:138` 의 base 시험 줄 설명은 CLAUDE.md 그 줄의 사본이라 같은 변경에서 함께 고친다(U1 · 모듈이 느는 단위마다). 그 두 줄의 플래그 근거(「`platform` 의 자식 PID 테스트가 실 `cmd.exe` 를 띄운다」)는 1-1 이 바꾸지 않는다 — `platform` 을 base 에서 빼는 1-3 의 몫이다.
- 제자리로 남기는 사본에는 「왜 base 를 안 쓰나」 한 줄 — `sync` 제자리 넷(base `logging` · `leftover.rs` 의 `try_lock` · 찾기 명단 도우미 · 셸 `output_channel.rs` — §3-5. 셸 `settings/mod.rs` 는 파일을 안 건드려 주석 없음) · 안 합치는 시계 둘(transport `clock.rs` — C5=b · daemon `usage_service/clock.rs` — C3=a · 둘 다 U5).
- **ADR-0275**(오케스트레이터 · `/adr` — U1 전에 · ★2026-10-04 에 박았다★): §4-2 의 표가 정본이다. ADR-0269 · ADR-0268 본문에 개정 링크를 달았다.

---

## 8. 미룬 것

| id | 무엇 | 사유 · 트리거 |
|---|---|---|
| D1 | `file::set_aside_corrupt` 통일(agent `persistence/{mod,presets}.rs` · 셸 `settings/store.rs` `set_aside`) | storage P3 가 그 동작 규칙을 바꿨다(ADR-0274 — 원격 브랜치) · 셸 `fsutil.rs` · `settings/` 에 파일 쓰기를 더한다 → P3 가 master 에 착지한 뒤(메모 §10 진행 방식) |
| D2 | P3 착지 뒤 쓸어 담기 — 셸 `settings/mod.rs` 복구 3곳 · `settings/store.rs:122` epoch ms(`u128`) · storage 의 `state/saver.rs` `Clock`(`now` + `wall_ms`) · `SystemClock::wall_ms`(`u64`) · `state/` 복구 1곳 | 같은 트리거. saver `Clock` 은 base `Clock` 의 `Send + Sync`(C1=a)에 맞춰 `Sync` 를 더해야 한다 · 셸 `settings/mod.rs` 3곳은 S2=(e) 라 셋 다 옮긴다(`:486` 의 「잎」 규율도 걸리지 않는다) |
| D3 | `write_atomic` 여러 벌 → platform | ADR-0266 결정 8 · 1-3 · P3 뒤 |
| D4 | transport 시계와 transport 의 base 의존(C5=b) — 자르기는 U1 에서 std 로 끝난다 | 3단계(transport 부착) TRD — ADR-0275 #3 |
| D5 | messaging XML 이스케이프 · command `testing.rs` 복구 3곳 | ADR-0267 — 필요해질 때 연결 |
| D6 | `UsageClock`(C3=a) | 합칠 계기가 생기면(예: book 의 시간 타입을 다시 볼 때) |

---

## 9. 지나가며 본 것 (메모 §11 적립 후보 — 오케스트레이터가 옮긴다)

| 무엇 | 위치 | 처리 시점 |
|---|---|---|
| `sha256_hex` 제외 사유 「`sha2` 를 끈다」는 거짓 — std 손구현이다 | `daemon/src/experiment/record.rs:189,247-251` · ADR-0269 「거부한 대안」 | ADR-0275(§4-2 #8) |
| 시계 seam 이 넷이 아니라 여섯(+ storage 일곱) | §2-2 | ADR-0275(#6) · U6 · D2 |
| `normalize_cwd` 의 `\` → `/` 는 모든 OS 에서 돈다 — POSIX 에서는 `\` 가 파일 이름에 쓸 수 있는 글자다(분기는 없어 platform 감은 아니다) | `agent/src/commands.rs:1143` | macOS 이식 때(ADR-0230) |
| 시험 임시 폴더 도우미 9벌 | §5 M1 | P3 · 1-3 뒤 |
| base `logging/mod.rs:155` 도 epoch 를 읽는다(로그 파일 이름) — 게이트 ③ 때문에 `time` 을 못 부른다 | base | 그대로(사실 기록) |
| 락 오염 복구의 경고는 운영 빌드에서 안 닿는다(`panic = "abort"`) | `Cargo.toml:34-35` | S2=(e) 의 근거(ADR-0275 #1) |
| 첫 제출 래치 머리 「래치 뮤텍스를 쥔 채 … 로그도 놓은 뒤에 쓴다」는 래치 자기 로그에만 참이다 — commit 포트가 래치를 쥔 채 `warn!` · `info!` · `debug!` 를 내고, 그 아래 명부 · 저장소도 로그를 낸다(저장소는 디스크 쓰기까지) | `session_id_latch.rs:23` · `manager.rs:541-569` · `profile.rs:717` · `persistence/mod.rs:101-106` | 래치 문서를 고칠 때(1-1 밖 — S2=(e) 의 도우미는 로그를 안 내 이 어긋남에 보태지 않는다) |
| 대기 입력 명부 「★이 가드를 쥔 채 emit·IO·다른 락을 잡지 말 것★」인데 봉인 뒤 `Queued` 를 바꿔 적는 갈래가 그 가드(와 replay 락)를 쥔 채 `debug!` 를 낸다 | `queued_input.rs:399` · `output_core.rs:558-570` | 그 자리를 고칠 때(1-1 밖) — 경고를 다는 갈래가 문서 규율에 기대는 한계의 실례(ADR-0275 #1 의 근거) |
| codex 머리가 「락 순서 = 상태 → 대기표」를 허용하지만 그렇게 잡는 운영 자리는 없다(소멸자는 일부러 놓은 뒤 잡는다) | `backend/codex/transport.rs:21` · `:2966` | 1-1 엔 무관(S2=e) — 복구 경고를 다시 달면 잠복 간선이 된다(§2-4) |

---

## 10. 근거 · 검증 상태

**실측 명령(전부 `0ef6292`, 2026-10-03):**

- UTF-8 자르기: `rg -n "is_char_boundary|floor_char_boundary|ceil_char_boundary" crates src-tauri/src --glob '*.rs'`.
- std 경계 함수: 스크래치 `fcb.rs`(`"가나다".floor_char_boundary(4)` · `ceil_char_boundary(4)`) → `rustc --edition 2021` 컴파일 · 실행 출력 `3 6`(rustc 1.95.0).
- hex: `rg -n '02x\}|:02x' crates src-tauri/src --glob '*.rs'` · `grep -n sha2 crates/*/Cargo.toml src-tauri/Cargo.toml`(0줄).
- epoch ms: `rg -n "fn (now_epoch_ms|now_ms|…|now_millis)\b"` · `rg -n "UNIX_EPOCH" crates src-tauri/src` · `rg -n "now_millis\(\)"`.
- 시계: `rg -n "trait (Clock|UsageClock|\w*Clock)\b" crates src-tauri/src` · `rg -n "impl\b.*\b(Clock|UsageClock|\w+Clock) for\b"` · `rg -n "mono_now\(" crates/engram-dashboard-agent/src | wc -l`(22) · `rg -n "\.mono\(\)"`(11) · `rg -n "\.wall\(\)"`(13).
- 락 오염: ADR-0269 셈법 `git grep -h -o -E '\.lock\(\)\s*\.unwrap_or_else\(\|[a-z_]+\|\s*[a-z_]+\.into_inner\(\)\)' HEAD -- crates src-tauri/src | wc -l`(67) · `rg -c 'PoisonError::into_inner' crates src-tauri/src`(합 27). 117 은 스크래치 파이썬 스크립트(여러 줄 정규식 — `.lock|read|write|wait_timeout(...)` 뒤 `unwrap_or_else` 의 클로저 · 함수 포인터 꼴 109 + `match` 갈래 6 + 블록 클로저 1 + `try_lock` 1)로 셌다. 같은 109 는 `rg -U -o` 로도 재현된다.
- 독 단언: `rg -n "is_poisoned|clear_poison" crates src-tauri/src`.
- 섞인 자물쇠(1차 리뷰 뒤): 스크래치 파이썬 스크립트 — 운영 구획의 복구 사이트에서 받는 쪽 필드 이름을 뽑아, 같은 crate 운영 코드에서 그 이름 뒤 `.lock()/.read()/.write()` + `.expect(`/`.unwrap(` 를 찾는다. 필드 이름만 같은 남의 구조체(codex `child` · `stdout` 대 stdio · `self.inner` 대 `connection_core` 등)는 손으로 걸렀다. `try_lock` 이웃은 `rg -n 'try_lock\(\)' crates/engram-dashboard-agent/src` 로 따로 봤다. 일치 비교라 다른 이름으로 같은 자물쇠를 쥐는 자리(복제한 `Arc` 를 다른 변수 이름으로 받는 꼴)는 못 잡는다 — PTY 감시 스레드의 `watcher_child` 는 손으로 찾았다.
- 쥔 채 로그 금지 자물쇠 · 중첩(4판까지 — S2=(e) 로 이전 규칙에서 빠졌다 · §2-4 역사 기록): 넓힌 문구 `rg -n '찍지 않는다|놓은 뒤 (찍|부른)|락 밖에서|잠금 밖에서|쥔 채.{0,40}(로그|IO|emit|찍|보내)|로그도 놓은|잎이다' crates/engram-dashboard-agent/src crates/engram-dashboard-daemon/src src-tauri/src`(74줄)을 하나씩 읽고 남은 운영 사이트마다 자물쇠 정의 · 획득 도우미 문서를 읽었다(굵은 글씨 · 영어 `leaf` 가 정규식을 빗나간다) · 문서의 락 순서 `rg -n '락 순서|lock order|Lock order' crates/engram-dashboard-agent/src crates/engram-dashboard-daemon/src src-tauri/src --glob '*.rs'`(33줄)로 제외 자물쇠가 바깥인 간선을 뽑아 가드 구간과 그 안의 콜백 · 포트 · 시계를 읽었다 · codex 상태 락 사이트 = `rg -n -U '(\.lock\(\)|wait_timeout\([^;]*?\))\s*\.unwrap_or_else\(' crates/engram-dashboard-agent/src/backend/codex/transport.rs` 중 `:4284`(시험 모듈 시작) 앞 줄 · 각 자리의 `lock` 이 `SharedState` 에서 풀린 것을 앞 25줄에서 확인 · 운영 구독자 `rg -n 'set_global_default|\.try_init\(\)|\.init\(\)|registry\(\)' crates src-tauri/src`. 운영/시험 재분류 = 스크래치 스크립트(복구 꼴마다 감싼 `fn` 위 `#[cfg(test)]` · `#[cfg(any(test`) — codex `:1348` · `input_queue.rs:262` 를 눈으로 확인했다. 전부 읽기다.
- 소스 문자열 시험: `rg -n 'include_str!\(' crates src-tauri/src --glob '*.rs'`(생성물 · 문서 제외 21곳) · `rg -n 'CARGO_MANIFEST_DIR' crates src-tauri` 로 원천 파일을 읽는 시험을 찾고 각 시험의 바늘을 읽었다.
- 시험 대기: `rg -n "fn (wait_until|poll_until|wait_for|eventually|spin_until|wait_while)\b" crates src-tauri --glob '*.rs'`.
- 운영 그래프: `cargo tree --locked --offline -p engram-dashboard-daemon -e normal,features -i engram-dashboard-base --target all` · `… -p engram-dashboard-agent -e normal,dev,features -i engram-dashboard-command`.
- storage 겹침: `git fetch` 뒤 `git diff --stat $(git merge-base HEAD origin/v0.3.3/feat/storage) origin/v0.3.3/feat/storage -- crates src-tauri/src` · 같은 diff 의 `+` 줄 grep.
- transport 게이트: `crates/engram-dashboard-transport/src/lib.rs:31-85` · `.github/workflows/ci.yml:686-760` 읽기.

**5판 재대조(2026-10-04 · 코드 트리 `0ef6292` 그대로):**

- ADR 번호: 원격 브랜치마다 `git ls-tree --name-only <브랜치> docs/decisions/ | grep -E '027[4-9]'` → `origin/v0.3.3/feat/storage` 의 0274 하나뿐 · 이 브랜치의 마지막은 0273 → 새 번호 0275.
- 소스 문자열 시험(S2=(e) 로 넓어진 몫): 위 21곳 중 codex `transport.rs`(`:5029 · 5631 · 6641 · 6702 · 7076 · 7360`) · `manager.rs`(`:3960 · 4289 · 4319 · 4348 · 4439 · 4473 · 6250 · 6374 · 6512`) · daemon `command_delivery.rs:4779` · `connection_core.rs:2532 · 3166` · `profile.rs:1167` 의 `split/find/contains/matches` 바늘을 다시 뽑아 락 복구 꼴과 맞댔다 — 락 복구를 재는 것은 `:7360`(stdin 개수) 하나.
- `match` 꼴 6곳: `rg -n -U --pcre2 'Err\((\w+)\)\s*=>\s*\1\.into_inner\(\)' crates src-tauri/src --glob '*.rs'` · `wait_timeout` = 운영 3(`input_queue.rs:178 · 254` · codex `:2192`) + 시험 1(codex `:10176`) — `rg -n -U 'wait_timeout\([^;]*?\)\s*\.unwrap_or_else'`.
- std 경계 함수의 MSRV: `grep -rn rust-version Cargo.toml crates/*/Cargo.toml src-tauri/Cargo.toml` → 0줄(어느 crate 도 컴파일러 하한을 낮게 선언하지 않는다 — 고정은 `rust-toolchain.toml` 1.95.0 하나).
- base 의존: `grep -n engram-dashboard-base crates/*/Cargo.toml src-tauri/Cargo.toml` → agent · daemon · discovery · net(optional) · 셸(`src-tauri/Cargo.toml:61`). transport 는 없다.

**메모 줄 번호가 밀린 곳(메모 → 지금):** agent `output_core.rs:716` → `:1059-1062` · agent `commands.rs:763` → `:1143` · discovery `lib.rs:425` → `:431` · daemon `tests/control_send.rs:70` → `:76` · daemon `lib.rs:65` → `:73-82` · daemon `control/mod.rs:141` → `:142-151`.

**미검(이 문서의 주장 중 돌려서 확인하지 않은 것):**

- 워크스페이스 회귀의 출발 수치(`0ef6292`) — 재지 않았다(U1 착수 때 — §6-1).
- `dyn` 하위 트레이트에서 상위 트레이트 메서드를 부를 때 `use` 가 필요한지 — 컴파일 안 함.
- S2=(e) 를 돌려 보지 않았다 — 83곳이 전부 그 도우미로 컴파일된다는 것, `output_channel.rs` 를 제자리로 둬 `:145-148` 이 초록으로 남는다는 것 모두 읽기다. (e) 는 로그를 늘리지 않아 로그 캡처 시험과 겹칠 것이 없다.
- T1 의 std 교체 — `fcb.rs` 실측은 함수 동작이다. 세 자리를 바꾼 뒤의 동작은 U1 의 시험이 잰다.
- 쥔 채 로그 금지 감사(4판까지)의 완전성 — 읽기였고 문구가 다른 규율을 놓쳤을 수 있다. S2=(e) 로 이 감사는 이전 규칙에서 빠졌다 — 복구 경고를 다시 달 때만 다시 문제가 된다(§2-4 · ADR-0275 #1).
- 섞인 자물쇠 표(§2-4)는 필드 이름 일치로 찾았다 — 다른 이름으로 받은 같은 자물쇠는 손으로 찾은 것뿐이라 빠진 것이 있을 수 있다.
- 운영/시험 가름(§2-4)은 스크립트 추정 + 손 정정이다.
- storage 브랜치 사실은 `5a2cf7f` 시점이다 — P3 착지 때 다시 잰다.
