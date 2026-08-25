# 핸드오프: ADR-0175 1·2단계 완료 — 다음은 3단계 `replay_flight` 이사

## 한 줄 상태 · 다음 첫 액션

**ADR-0175 5단계 중 1단계(개명)·2단계(`base` 신설)가 CI 초록까지 닫혔다.**
**다음 첫 액션 = 3단계 `replay_flight` 를 셸 안 모듈로 이사**(ADR-0175 결정 3).

## repo 상태

- 브랜치 `v0.3.0/refactor/crate-boundaries`, **작업 트리 깨끗 · 원격과 동기(ahead 0)** · stash 없음.
- 이번 세션 커밋 7개 전부 푸시됨, **최신 CI 초록**(`8a9c482`, run 32878562478).

```
8a9c482  refactor(base): 바닥 crate 신설 — 로깅 + PID 헬퍼 이사   ← CI 초록
acec9e6  docs(cleanup): flaky 2건 백로그 + 죽은 글 5곳 삭제        ← CI 초록
29129c2  docs(handoff): 앞 세션 인계 기록 영속화
5a7b14e  chore(protocol): ts-rs 바인딩 재생성                      ← CI 초록
9269d15  docs(agent): 개명 잔여 133곳 + 리뷰 FIX 7건
d20eb8c  fix(qa): 경로 부재 가드 fail-close
bd38d32  refactor(agent): core → agent 개명 + 모듈 접기
```

## 이번 세션에 한 일

1. **개명 잔여 전수 스윕** — 앞 커밋이 crate *이름 문자열*만 훑어서 맨 `core` 로 적힌 산문이 남아 있었다. 3라운드 헛돈 뒤 전수 열거로 끊었다(133곳/49파일).
2. **`doc` full 리뷰 2인** → FIX 7건 수정.
3. **`base` 신설**(ADR-0175 결정 1) + **`code` deep 리뷰 3인** → FIX 8건 + 선택 1건 수정.
4. QA full(GUI 실측 포함) 1회, CI 초록 4회.

### 확정된 성과

**`discovery → agent` 간선 소멸, `net` 도 agent 를 의존으로조차 모른다.** ADR-0175 가 이 정리의 목표로 명시한 자리다. 둘 다 쓰던 것은 PID 헬퍼뿐이었고 그게 `base` 로 갔다.

## 검증 상태

**돌린 것(실측):**
- `cargo test --workspace -- --test-threads=4` → **2,102 통과 / 0 실패 / 9 무시**, 바이너리 43개. ★ADR §검증 잣대의 합계 그대로★ — 분포만 바뀌었다.
- `cargo test -p engram-dashboard-base -- --test-threads=4` → 19 통과.
- 격리·상한 게이트 **13종** 실측. build · fmt · metadata 클린.
- **GUI 실측 1회 통과**(1단계 시점) — 앱 기동 → 데몬 연결 → spawn → 출력 → kill → 명부 제거. ★1회 통과는 smoke 지 race 없음의 증명이 아니다.★

**검증 안 된 것:**
- ★**flaky 2건 원인 미규명**★ — 아래 「미결」 1번.
- **비-Windows 컴파일 미검증** — 이 머신에 타깃이 없다. `base/src/platform.rs` 의 `#[cfg(not(windows))]` 스텁 5개와 windows-게이트 테스트 7건의 비-Windows 갈래는 **읽기로만** 봤다.
- **agent 의 `windows` feature 최소성은 grep 근거뿐** — `cargo test -p engram-dashboard-agent` 가 base 를 서브그래프에 포함해 feature 가 합쳐지므로 컴파일러가 증명해 주지 않는다.
- **2단계 뒤 GUI 실측 없음** — 순수 crate 경계 이동이고 프론트 무접촉이라 생략했다. 로깅 파일 sink 가 실 데몬 경로에서 여전히 도는지는 미확인.
- 릴리스 바이너리 크기 감소분 미측정(ADR 가 이미 미측정으로 선언).

## ★미결 — 사용자 결정이 필요한 것★

1. **flaky 테스트 2건.** `daemon_client::tests::buffered_command_on_disconnect_is_drained_not_executed_post_reconnect`(셸) · `connection_core::tests::subscribe_emits_ack_then_replay_then_complete_in_order`(데몬). 둘 다 동시성 순서 불변식 자리, CI 에서만 죽고 **로컬 재현 0**(lib_unit 10회 · daemon --lib 12회 · 워크스페이스 전회귀). **기록만 했고 단정은 안 건드렸다** — `docs/backlog.md` 「검증 절차」 절이 정본. ★재실행 초록은 원인을 설명하지 않는다★ · ★개명이 원인일 가능성은 배제되지 않았다★ · ★sleep·매직넘버 금지(ADR-0038)★.
2. **4단계 `ui.` 접두의 적용 범위** — 미결. 3단계가 끝나야 그 결과를 반영해 정할 수 있다.
3. **맨 `core` 잔여를 막는 CI 게이트 신설** — 보류 결정. `base` 가 섰으니 이제 잔여 집합이 확정됐다, 세울 수 있는 시점이다.
4. **`base` 에 세 번째 입주자** — 들어오려 하면 그 자리에서 멈추고 ADR-0175 「거부한 대안」 첫 항목을 다시 연다. 게이트가 아니라 규약이라 자동으로 안 걸린다.

## ★안 한 것 (요청대로 모아 둔다)★

- **3·4·5단계 전부 미착수.**
- **`daemon` 의 `tracing-subscriber` 잔여** — `daemon/Cargo.toml:124` 에 production 의존으로 선언돼 있는데 **사용 0줄**(실측). 이번 범위 밖이라 안 뗐다. `CLAUDE.md` 「의존성」에 사실대로 적어 뒀다.
- **ADR-0175 본문의 오류 2종** — ① §근거가 `transport/stdio.rs:30` 을 가리키는데 실제는 `:24`(모듈 접기 전 번호) ② §근거·§영향이 옛 정규식 모양 `…::[A-Za-z0-9_:]+` 를 인용하는데 그 패턴의 결함이 이번에 고쳐졌다. **사실 오류라 정정 가능하지만 손대지 않았다.**
- **`docs/reference/architecture-overview.md` 에 `base` crate 절이 없다** — 다른 crate 는 절이 있는데 base 만 빠졌고, 의존 그래프 다이어그램도 ADR 목표 그래프와 대조 안 했다.
- **`protocol/bindings/AgentProfile.ts` 의 `cwd` 서술 모순 의혹** — 생성물은 "정규화된 cwd", agent 의 같은 이름 필드는 "raw". 타입이 서로 달라(wire 미러 vs 내부) 자동으로 모순은 아니다. 미확인.
- **`docs/reference/logging-conventions.md:53` 의 거짓 서술** — "`mask_secrets` 호출처 0" 이라 적었는데 실제로 둘이 부른다(`agent/src/transport/stdio.rs:179` · `src-tauri/src/ui_settings.rs:417`). 뒤따르는 결론은 참이라 의도를 못 가려 보고만 했다.
- **`CLAUDE.md` 의 선재 자기모순 1곳** — 「이 네 줄이 각 통합 스위트를 도는 유일한 경로다 — 워크스페이스 회귀가 이 패키지를 통째로 빼기 때문」이 거짓이다(제외는 걷혔고 같은 줄 뒷부분은 그걸 반영한다). 선재 드리프트라 안 건드렸다.

## 실패한 접근 / do-not

- ★**개명·이사를 crate 이름 문자열 grep 으로 끝내지 말 것**★ — 맨 낱말로 적힌 산문이 안 잡혀 이번에 3라운드를 헛돌았다. **커밋 전에 전수 열거를 돌린다.** 분류 규칙 = 다른 뜻(원칙 「코어 격리」·`OutputCore` 같은 식별자)으로 읽어 거짓이 될 때만 바꾼다.
- ★**`#[ts(export)]` 타입의 doc 주석을 고치면 생성물 `.ts` 가 밀린다**★ — 주석만 바꿔도 바인딩 재생성이 필요하다. 이번에 CI 가 잡았다(2차 실패의 실체).
- ★**CI 에 `-- --test-threads=4` 를 붙이지 말 것**★ — CLAUDE.md 가 "CI 는 이 플래그를 쓰지 않으며 그것이 의도다, 드리프트 수정 하지 말 것"이라고 못 박는다. **이번 세션 메인이 지시서에 잘못 넣었고 코더가 거부한 게 옳았다**(리뷰가 확인). 로컬 명령에만 붙는다.
- ★**0 기대 게이트를 짝 없이 세우지 말 것**★ — 옛 이름이 사라져 매치가 0이 되는 것과 위반이 없어 0인 것은 다르다. net 게이트 2a/2b 가 그 짝의 형태다.
- ★**게이트 정규식의 자기일치**★ — 0 기대 게이트를 그 게이트 문자열이 적힌 파일에 겨누면 영구 FAIL 한다. 이 저장소는 `(a)gent` 괄호 형태로 막는다(현재 세 곳). **새 게이트를 만들 때가 가장 위험하다.**
- **역사 기록의 옛 이름을 고치지 말 것** — `docs/decisions/` 본문 · `docs/process/` · `docs/research/` · `.claude/handoff/history/` 는 append-only.

## 정지 조건 (멈추고 물어야 하는 자리)

- `base` 에 **세 번째 입주자**를 넣고 싶어질 때 → ADR-0175 「거부한 대안」 첫 항목 재론.
- 4단계 `ui.` 접두의 **적용 범위**를 정해야 할 때.
- flaky 2건의 **단정을 고치려 할 때** → 원인 미규명 상태에서 느슨하게 하면 진짜 회귀를 가린다.

## 3단계 착수 정보 (ADR-0175 결정 3)

`replay_flight`(860줄, 프로덕션 366줄)를 `src-tauri/src/daemon_client/replay_flight.rs` 의 **4줄짜리 재수출을 실물로** 바꾼다. crate 로 승격하지 않는다(결정 6 — 파일 하나짜리 lib 금지).

- **소비자는 셸뿐이다** — 데몬은 컴파일 의존 0, 산문 주석 1줄(`daemon/src/connection_core.rs:1494`)뿐. 그 경로도 같이 고친다.
- **헤더의 죽은 존치 사유를 걷는다** — *"이 위치 덕에 headless 에서 단위테스트가 실행된다"* 는 ADR-0174 가 셸 테스트 타깃을 세우면서 죽은 전제다.
- ★**침묵 축**★ — 그 단위테스트 494줄이 셸 `lib_unit` 타깃 아래로 들어간다. `[[test]] lib_unit` 선언이 사라지면 실패가 아니라 **침묵으로 증발**한다(ADR-0174).
- 합격선은 이번과 같다 — **워크스페이스 총 통과 2,102 불변**.

## 참조 (읽어야 할 것만)

- `docs/decisions/0175-코어를-agent-와-base-로-가른다.md` — 이 작업의 정본. 결정 3 이 다음 단계다.
- `crates/engram-dashboard-base/src/lib.rs` 헤더 — 입주 조건 3개 + 격리 게이트 3종의 정본.
- `crates/engram-dashboard-net/src/lib.rs` 헤더 — net 게이트 기대값·명령 텍스트의 정본(이번에 다섯 사본을 일치시켰다).
- `docs/backlog.md` 「검증 절차」 — flaky 2건 기록.
- `CLAUDE.md` 「빌드·검증 명령」 — 재실행 명령 정본.

## 이어가기 프롬프트

```
engram-dashboard 리팩토링 이어간다. /handoff load 로 맥락 받아라.
ADR-0175 1·2단계는 CI 초록까지 끝났다 — 다시 돌리지 말고 3단계
(replay_flight 를 셸 모듈로 이사)로 바로 들어가라. 착수 전에 핸드오프의
「안 한 것」과 「미결」을 나한테 한 번 읽어 주고, 4단계 ui. 접두 범위는
나한테 물어라.
```
