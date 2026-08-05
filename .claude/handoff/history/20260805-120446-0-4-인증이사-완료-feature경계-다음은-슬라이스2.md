# 핸드오프: **0-4(인증 이사) 완료 + auth feature 경계** — ADR-0129 결정 1 불변식 **완전 충족** · 다음은 **슬라이스 2(에이전트 시스템 lib)** · **푸시 완료(미푸시 0)** · 워킹트리 클린

> **"에이전트 어휘 0"이 달성됐다.** 슬라이스 1이 의도적으로 이월했던 마지막 예외(`ws.rs`의 `AgentCommand::Auth`/`PROTOCOL_VERSION`)가 사라졌다. 0-4와 feature 경계는 커밋·푸시·기록까지 닫혔으니 **다시 손대지 말 것** — 다음 단위는 슬라이스 2다.

## 한 줄 상태 · 다음 첫 액션

- **상태:** ADR-0129 3층 분리 중 **슬라이스 1 + 0-4 + feature 경계 종료.** `/review code` 3인(doc-aware · cross-family blind · 실측 검증자) + light 1인, 전원 FIX → 전부 반영. **GUI 실측 PASS**(cdp).
- **repo:** `master`, **미푸시 0**(`93c1a53`까지 origin 반영), 워킹트리 **클린**. 데몬·앱 프로세스 0(정리됨).
- **★다음 첫 액션 = 슬라이스 2(에이전트 시스템 lib 추출)★.** 착수 0줄 — **범위 조사부터** 한다(0-4도 조사 없이 들어갔으면 생산 지점 5개를 몰랐다).
- **워크트리 분담(고정):** `master`(여기) = 데몬 분리 · `../engram-dashboard-wt2`(브랜치 `wt2`) = 주석 정리, **다른 세션 소관 — 건드리지 말 것.**

## 이번 세션 커밋 4개 (전부 푸시됨)

| 커밋 | 내용 |
|---|---|
| `db503c9` | standing 문서 4종 진실 복원(멤버 7 반영 + net 격리 게이트 주입) |
| `8d45672` | step-log S18.22 |
| `818cc0c` | **0-4 — 인증 핸드셰이크 이사(protocol → net), 어휘 0 달성** |
| `93c1a53` | **auth를 feature 경계 뒤로 + 게이트5 + ADR-0129 note** |

전말 = step-log **S18.22 · S18.23**.

## 지금 상태 (기계로 확인된 것)

| 사실 | 값 |
|---|---|
| `net/src/` 의 에이전트 어휘 | **0** (게이트4가 강제 — positive control로 비공허 확인) |
| 격리 게이트 | 코어 tauri · messaging · net 1·2·3·4·5a·5b (정본 = `net/src/lib.rs` 헤더) |
| `net` 기본 feature | **`default = []`** — 데몬만 `features = ["server"]` 옵트인 |
| discovery 정상 의존 폐포 | **122 → 110 crate**, tokio 계열 0줄 |
| 전 멤버 회귀 | `cargo test --workspace --exclude engram-dashboard` 전 타깃 ok(net 31) |

## ★슬라이스 2 — 착수 전 반드시 볼 것★

**대상:** `connection_core` · `agent_conn` · `status_fanout` · `control/` · `messaging_host` · `experiment/`.

- **★모듈 순환 3개★** — `connection_core ↔ status_fanout`, `messaging_host → status_fanout → connection_core → messaging_host`. **이 셋은 crate 경계의 같은 쪽에 둬야 한다**(전부 행 내부라 합법).
- **★`frame_port`를 먼저 검토하라★(93c1a53이 매니페스트에 박아둠)** — `frame_port`는 **계약이지 서버가 아니고 tokio를 안 쓴다**(`rg -n "tokio" crates/engram-dashboard-net/src/frame_port.rs` → 0줄). 그런데 지금 `server` feature에 묶여 있어서, 슬라이스 2가 `ConnectionHandler`/`ConnectionHandlerFactory`를 **구현**하려면 서버 행 전체(async 런타임 + Win32 + net→core/protocol 간선)를 지게 된다. **오늘 discovery에서 없앤 짐이 슬라이스 2에서 되살아나는 경로다.** 착수 시 `frame_port`를 `server` 밖으로(또는 `port` feature로) 빼는 걸 **먼저** 결정할 것 — 슬라이스 2가 `server`를 켜고 난 뒤엔 되돌리기가 공개 API 변경이다.
- **슬라이스 3 승계(코드에 기록됨, `daemon/src/lib.rs`의 `DaemonWiring` 주변):** 조립 바이너리가 레지스트리와 팬아웃을 **둘 다 들면 짝 어긋남이 crate 경계에서 되살아난다.** 정공법 = 네트워크 crate가 투영을 내주는 것(`impl ConnRegistry { pub fn fanout(&self) -> Arc<dyn FrameFanout> }`). 파생을 또 베끼거나 인자 2개로 되돌리지 말 것. `run_accept_loop` 분해도 슬라이스 3.

## 검증 상태

**재실행 명령(워크스페이스 루트):**
```bash
cargo build
cargo test --workspace --exclude engram-dashboard   # 전 멤버 회귀 (루트 bare cargo test 는 0xc0000139 로 불가)
cargo test -p engram-dashboard-net                  # 게이트5a feature 0개 → 6 passed
cargo test -p engram-dashboard-net --all-features   # 게이트5b 서버 행 → 31 passed
cargo fmt --check
# 격리 게이트 정본 = crates/engram-dashboard-net/src/lib.rs 헤더 (게이트 1~5)
rg "^\s*use tauri" crates/engram-dashboard-core/src/                                          # 0
rg "engram_dashboard_(core|daemon|protocol|discovery)" crates/engram-dashboard-messaging/src/  # 0
npx tsc --noEmit && npm test                        # 클린 / 41파일 634
```

**★검증 안 된 것★:**
- **슬라이스 2는 착수 0줄** — 모듈 순환 3개는 알려져 있지만 **실제 이사 범위·경계면은 미조사**.
- **src-tauri 핸드셰이크 테스트 6건은 컴파일만 되고 실행 안 된다**(선존 `0xc0000139`, step-log 494/532/761). 그중 둘이 "컴파일 상수를 보내고 daemon.json을 에코하지 않는다"를 지키는 테스트다 — **초록 CI를 이 생산 지점의 커버리지로 오독하지 말 것.**
- **JS 발신자 둘(`src/api/wsTransport.ts:272` · `scripts/engram.mjs:72`)은 daemon.json의 `protocol_version`을 에코한다** — 러스트 발신자가 지키는 불변식이 JS에선 무효. 선존, 테스트 0건.
- **`scripts/engram.mjs`는 인증 프레임을 손조립하는 다섯 번째 생산 지점**이고 테스트가 없다(`auth.rs` 헤더가 열거).
- **`dep:windows`의 비Windows 해소** — 리눅스 타깃 `cargo tree`/`metadata`로 해석은 확인(windows crate 부재 + 양성 대조), **실제 컴파일은 툴체인 부재로 미측정.**
- **`protocol::` 일반 심볼 allowlist 게이트는 여전히 부재** — 게이트4는 `AgentCommand`/`PROTOCOL_VERSION` 두 이름만 막는다(ADR-0129 note가 유예로 기록).
- **cdp 실측 1회 통과 = smoke**(존재 증거)지 race-free 증명이 아니다.
- **`write_handle.abort()` 미검증·제거 금지**(선존) — `net/src/ws.rs`. read_task가 코어 등록과 `subs` 기록 사이에서 **패닉**하면 "송신단 드롭이 자기종료를 보장"이 깨진다.
- **진행 중 소켓 쓰기는 종료 신호로 중단 불가**(선존) · **정리가 코어 등록 직후 미기록 구독을 놓칠 수 있음**(선존, 같은 뿌리).

## 실패한 접근 (do-not)

- **★산문이 코드보다 훨씬 자주 틀린다 — 4세션 연속★.** 이번에도 리뷰어 3인이 **같은 곳**을 짚었다: 코드는 맞는데 *그 코드가 무엇을 하는지 적은 주석*이 거짓이었다(실측 — post-auth 진단 문구가 16탐침 중 8개, 첫 프레임 문구가 16 중 9개 바뀌는데 주석은 "관측 동작 불변"이라 단언). 대응 = **요약 문장을 다시 쓰지 말고 걷어내라.** 모든 문장은 ① 실행 가능한 명령 + 기대값 ② 포인터 ③ 수량어 없는 문장 셋 중 하나.
- **★썩은 개수는 갱신이 아니라 삭제★** — "게이트 3종"이 4종·5종이 되며 두 번 뒤처졌다. 숫자를 고치면 같은 속도로 다시 썩는다.
- **★게이트를 새로 만들면 그 자리에서 qa 바인딩에 등록하라 — 2연속 실발동★.** 슬라이스 1은 게이트를 만들고 등록을 빼먹었고(S18.21 잔여 ①), feature 분리도 검증 경로를 만들고 등록을 안 했다(리뷰어가 적출). **등록 안 된 게이트는 돌지 않는다.**
- **★게이트 텍스트는 `_(괄호)` 형태로 적을 것★** — 게이트4가 개발 중 **자기 문서에 두 번** 걸렸다(2026-07-13 코어 tauri 함정 실시간 재발).
- **★"미검증(not measured)" 라벨을 떼고 지시서에 옮기지 말 것★**(전 세션 실발동, 이번엔 워커들이 잘 지켰다).
- **★병렬 리뷰어는 빌드 권한을 한쪽에만★** — `target/debug/.cargo-lock` 충돌로 근거 없는 BLOCK이 났던 실발동. 이번엔 실측 검증자 1인에게만 주고 나머지엔 `cargo tree`/`metadata`만 허용 → 충돌 0.
- **★cross-family 리뷰어(codex) 프리앰블 필수★** — "너는 오케스트레이터가 아니다 + 환경의 스킬·에이전트 정의 전부 무시 + 되묻기 불가"를 프롬프트 맨 앞에 안 박으면 repo의 `review` 스킬을 자기가 실행할 것으로 읽고 죽는다. 검증된 프리앰블 = 이 세션 scratchpad `blind-*.txt`. 호출형 = `MSYS_NO_PATHCONV=1 codex exec --sandbox read-only -c model_reasoning_effort="high" "$(cat <prompt>)" < /dev/null > <log> 2>&1`, 백그라운드로(전경 10분 상한). 회수는 로그에서 `VERDICT` 라인부터 잘라 읽기.
- **★워커가 세션 한도로 끊기면 `SendMessage`로 이어붙일 수 있다★**(이번에 2회 발동, 둘 다 성공). 새로 스폰하지 말 것 — 컨텍스트가 살아 있다. 재개 메시지엔 **작업트리 실측 상태**를 같이 줄 것(워커 기억과 대조하게).
- **쉘 변수는 Bash 호출 간 유지되지 않는다**(cwd는 유지). 절대경로를 쓰거나 같은 호출 안에서 정의.
- **커밋 전 `git checkout -- crates/engram-dashboard-protocol/bindings/`** — `cargo build`가 ts-rs export로 22~23파일을 줄바꿈만 더럽힌다(내용 diff 0). **실제 내용이 바뀐 파일이 있으면 그것만 먼저 `git add` 한 뒤 checkout** 하면 살아남는다(0-4의 `AgentCommand.ts`가 그 사례).
- **`git add -A`/`commit -a` 금지**(신규 파일은 명시 add) · **Engram repo(`I:\Engram`)에서도 금지**(중첩 repo gitlink) · bare `cargo test` 금지 · Fable 워커 금지 · **Agent 스폰 시 model/프리셋 명시 필수** · git-bash 경로 인자엔 `MSYS_NO_PATHCONV=1`.
- **GUI 실측 함정:** ① **xterm은 canvas라 `innerText`가 0** — buffer API를 React fiber로 잡아 읽거나 `shot` + Read ② `cdp.mjs eval` 스크립트가 길면 조용히 깨진다 — 쪼갤 것 ③ **cwd 인자를 셸 경유로 넘기면 백슬래시가 먹힌다**(이번에 `agents.json`에 이상한 이름 프로필이 하나 생겼다 — 앱 결함 아님) ④ 자기가 띄운 **PID 트리째** 죽이고(`taskkill /PID <pid> /T /F`), **무관한 `node.exe`를 죽이지 말 것**, **공유 데몬 불가침**.
- **다른 세션이 `docs/`를 편집하면 vite watcher가 앱을 리로드한다** — cdp 실측 중 콘솔 warning으로 나타난다. 결함 아님.

## 정지 조건

- **ADR 본문(결정 텍스트) 수정 = 사용자 승인.** (note 추가는 이번 세션 승인받아 1건 추가.)
- **슬라이스 경계·계획 변경 = 사용자 결정.** 그 밖의 내부 구현은 **메인이 정하고 보고.**
- **wire JSON 변경이 필요해지면 멈추고 재론** — ADR-0129 전제(프론트 무변경)가 깨지면 비용 계산이 달라진다.
- **프로덕션 코드에 test-only 훅 추가 = 사용자 결정.**
- **리뷰 정면 대립은 메인이 근거로 갈랐다면 올리지 않는다** — 못 가릴 때만. (이번 세션 사례: blind가 "상위 계층 분류기로 3분류 복원"을 요구했으나 ADR-0129 결정 1과 정면 충돌 + 그 문자열 의존처가 `ws.rs` 자신뿐임을 실측 → 메인이 기각. 반대로 doc-aware가 blind를 이긴 사례도 1건.)
- **`.gitattributes` 추가 임의 금지.**

## ★사용자 판정 대기 2건★ (S18.22에서 이월 — 아직 미해결)

문서 리뷰에서 리뷰어 2인이 **정면 대립(cut vs keep)** 한 것. **보수측(=유지)으로 잠정 채택해 커밋**했고, cut 쪽을 고르면 되돌리는 게 한 커밋이다:

1. **격리 게이트 명령 텍스트를 세 문서(`CLAUDE.md`·`qa.md`·`testing-strategy.md`)에서 지우고 `net/src/lib.rs` 헤더만 정본으로 남길지** — blind: "중복이라 rot한다" / doc-aware: "qa 바인딩은 문자 그대로 실행돼야 게이트가 돈다(포인터로 바꾸면 조용히 안 돎)".
2. **`docs/reference/architecture-overview.md`의 net 설명 문단을 통째 지울지.**

## ★사용자 방향 확정 2026-08-05 — wire 타입을 빌드 타임으로 당긴다★

**결정:** 생성된 wire 타입 바인딩을 프론트가 **실제로 물게 한다**(현재는 생성만 하고 안 쓴다). 근거 = 오류가 런타임에서 **빌드 타임으로 당겨진다.** 이 프로젝트에서 그 차이가 큰 이유는 하나다 — **그 JS를 짜는 주체가 매번 갈리는 LLM 세션**이고, 세션은 런타임 실패를 못 보지만(앱을 띄워 클릭하지 않는다) 컴파일 실패는 반드시 본다. ADR-0129가 crate 벽을 세운 논리("문서·리뷰로만 지킨 경계는 세션 교체를 못 견딘다")와 **동일한 논리**다.

**★슬라이스 2와 섞지 말 것★** — 축이 다르다(축 1 = 프론트↔데몬 계약 / 축 2 = 데몬 내부 3층 분리). 별 단위로 잡고, 굵은 설계면 PRD/TRD·ADR 단계부터.

**측정된 현재 상태(이 세션 대화 중 실측 — 전부 재확인 가능):**
- **wire 명령 union에 TS 타입이 아예 없다** — `src/api/transport.ts:78` = `send(payload: unknown)`. **두 벌 문제가 아니라 영 벌 문제다**(세션 중 내가 "손 미러가 있다"고 말한 것은 오류였고 아래 혼동 쌍이 원인).
- **유일한 검문소 = 셸의 serde** — `src-tauri/src/daemon_client/mod.rs:532` `send_command(cmd: AgentCommand)`. 운영 경로(`TauriTransport` 고정, ADR-0036)는 여기를 통과하므로 모양이 틀리면 런타임에 거절된다.
- **직결 경로는 그 검문소마저 우회** — `src/api/wsTransport.ts:270-274`(auth 프레임 `JSON.stringify` 손조립) · `scripts/engram.mjs:72`. 둘 다 `daemon.json`의 `protocol_version`을 **에코**한다(러스트 발신자는 컴파일 상수를 보낸다 = "Fix C" 불변식이 JS에선 무효, 테스트 0건).
- **생성물 23개 중 소비 1개** — `protocol/bindings/StructuredEvent`만 `src/components/slot/structuredAccumulator.ts`가 import. 나머지 22개는 매 `cargo test`마다 재생성돼 dirty로 뜨고 아무도 안 읽는다(커밋 전 `git checkout --` 대상이 계속 생기는 원인).
- **★선례가 같은 repo에 있다 — 발명할 게 없다★** — 레이아웃 타입은 `src-tauri/bindings/`(8개)를 `src/api/layoutTypes.ts`가 **파사드로 재수출**하고 컴포넌트가 거기서 가져온다(ADR-0035, 레이아웃 권위 = src-tauri). bindings가 `tsconfig include("src")` 밖이라 상대경로를 한 곳에 모은 것까지 주석에 적혀 있다. **wire 타입도 같은 모양으로 옮기면 된다.**
- **★혼동 쌍 미등록 — `AgentCommand`★:** 러스트 = **WS wire 명령**(`Spawn`/`Kill`/`ListAgents`…) / 프론트 `src/api/types.ts:100` = **에이전트 실행 명령**(`{kind:'Claude'|'Shell'}`, 러스트 대응 이름은 `protocol::AgentSpawnCommand` — `domain.rs:174`). **같은 이름이 경계 양쪽에서 다른 뜻**이라 세션이 갈리면 엉뚱한 쪽을 고친다. CLAUDE.md 「혼동 쌍 — 고정 용어」 등록 후보.
- 프론트가 손 미러하는 건 wire 명령이 아니라 **도메인 타입**들이다(`AgentProfile`·`AgentSpawnCommand`·`Preset` — `src/api/types.ts` 주석이 "wire ~ 미러"로 표기).

**거부한 대안:** 생성기를 걷어낸다(잡음·가짜 안전망 제거, 검사는 런타임 한 곳으로 정직하게 남김) — 사용자가 빌드 타임 쪽을 택했으므로 기각. **단 지금 상태(생성하고 안 씀)는 비용만 내고 이득 0 + "안전망 있다"는 착각까지 주므로 유지 금지** — 어느 쪽이든 지금보다 낫다.

## 미결 (착수를 막지 않음)

- **`core`의 `protocol` dev-dependency가 죽은 것으로 보인다** — `crates/engram-dashboard-core/Cargo.toml:33`이 `[dev-dependencies]`로 선언하고 주석이 사유를 적어놨는데(S15 B8 opaque replay 단위테스트가 실 codec으로 프레임 조립), **지금 쓰는 코드가 0줄**이다(`rg -n "engram_dashboard_protocol" crates/engram-dashboard-core/` → 매치 없음). 슬라이스 1이 걷어낸 죽은 `windows` 의존과 같은 부류. **확정은 한 줄** — 지우고 `cargo test -p engram-dashboard-core` 통과하면 죽은 것.
- **`docs/reference/architecture-overview.md:166`이 그 dev-only 간선을 평범한 "의존"으로 그린다** — 그래프만 보면 코어가 런타임에 wire 타입을 안다고 읽혀 ADR-0003 격리와 정반대다. dev 표시를 붙이거나 간선을 빼야 한다. (2026-08-05 커밋 `db503c9`에서 간선 감사를 했는데 `[dependencies]`/`[dev-dependencies]` 구분을 안 봤다 — 감사 기준의 공백.)
- **`net/src/lib.rs:83`이 `ADR-0129 Note A`를 가리키는데 ADR에 그런 라벨이 없다** — 새 note 안에서 지시 대상을 고정해 뒀다. 코드 포인터를 고칠지 ADR에 라벨을 달지는 미정(후자는 기존 note 수정 = 사용자 승인 사안).
- **`bin/engram-send.rs`(1,528줄)는 직교** — `engram_dashboard_*` 의존 0. 어느 crate의 `[[bin]]`에 둘지만 정하면 된다(조립 crate가 자연스러움).
- **MCP 제어 평면(`control/mcp_server.rs`, axum, 자체 포트)** — ADR-0129가 "제어 평면"을 에이전트 시스템에 배정했다. 슬라이스 2 대상.
- **`src-tauri` 교차 의존:** `src-tauri/src/daemon_client/tests.rs`가 `engram_dashboard_daemon::start_test_server()`를 직접 부른다. **조립 crate 이름이 바뀌면 같은 PR에서 고쳐야 한다.**
- **`net → core` 존폐**(process liveness 헬퍼 2개) — blind가 계층 냄새로 지적, 메인이 비차단 판정. **사용자 판단 대기**(이월).
- **선존 stale 주석:** `connection_core.rs:3/:5`·`:494/:496` · `docs/reference/logging-conventions.md`의 `ws.rs` 줄 참조 · `tests/ws_e2e.rs`의 `ws.rs` 줄 참조(파일이 crate를 옮겨 경로도 틀려짐).
- **허수 테스트 1건(선존):** `subscribe_control_order_ack_then_complete` — 프레임 3개를 새 채널에 넣고 FIFO만 단언, 프로덕션 dispatch 무접촉.
- **ws_e2e `case09` flaky**(부하 민감, 임계값 튜닝 금지 — ADR-0038). 이번엔 통과.
- **`agents.json`에 cdp 실측이 남긴 프로필 1건** — `I:Engramappsengram-dashboard(2)`. 사용자 데이터라 안 지웠다, 트리에서 제거하면 된다.
- **스킬 피드백 적립됨:** review(cross-family CLI 프리앰블 / 병렬 리뷰어 빌드 락 / 델타 재리뷰 인원 규정 부재) · implement(코더 세션 한도 중단 시 재개 절차 — 이번에 `SendMessage` 재개로 2회 해결, 절차화 후보).
- 승계분: `agents.json` 동명 5건 · 접미사 `(1)` 미검증 2건 · `reply_failed` 조회 표면 부재 · `profiles_cb` 주입 잔존 · `~/.claude/settings.local.json` 허용 규칙 38개 무시 · settings.json 하드링크(DevMode OFF) · `--disallow-mcp` 존폐 · 실측 ⑧ 프로덕션 훅(수신자별 인과, 사용자 결정 필요).

## 참조 (읽어야 할 것만)

- **결정 정본:** `docs/decisions/0129-*.md` — 목표 모양 · 거부한 대안 3개 · **§영향/불변식 아래 note 3건**(2026-08-04 구멍은 두 모양 / 2026-08-05 Note A 격리 게이트 / **2026-08-05 최신 note = 0-4 완료 + feature 경계 + 옛 note 정정**)
- **흐름:** `docs/process/step-log.md` **S18.23**(이번 세션 전말) · **S18.22**(문서 진실 복원 + 미해결 쟁점 2건) · **S18.20**(0-4 착지점 = 네트워크 lib 이라는 사용자 결정 + 거부한 대안 2개)
- **경계 정본:** `crates/engram-dashboard-net/src/lib.rs` 헤더 — 소유 범위 · **격리 게이트 1~5 + 기대값** · feature 경계 · 단독 검증
- **feature 결정 근거:** `crates/engram-dashboard-net/Cargo.toml`의 `[features]` 주석 — 왜 `default = []`인지 · 게이트5가 **못** 잡는 것 · **슬라이스 2 착수 시 `frame_port` 먼저 검토**
- **코드 — 슬라이스 2의 무대:** `crates/engram-dashboard-daemon/src/`(`connection_core.rs` · `agent_conn.rs` · `status_fanout.rs` · `control/` · `messaging_host.rs` · `experiment/`)
- **코드 — 슬라이스 3 승계:** `crates/engram-dashboard-daemon/src/lib.rs`의 `DaemonWiring` 주변 — 짝 어긋남 규칙 · **"★다음 두 문장은 둘 다 거짓이니 인용하지 말 것★"**(디렉토리 범위 grep을 crate 전체 명제로 일반화한 사례 2건이 박제돼 있다)
