# 핸드오프: 데몬 분리 **슬라이스 0 완료**(0-1·0-2·0-5 커밋) — 다음은 **슬라이스 1 = 네트워크 lib 추출**(스코핑 완료) · 19커밋 미푸시 · 워킹트리 클린

> 얽힘 풀기가 끝났다. **crate 는 아직 하나도 쪼개지 않았고**, 다음 슬라이스가 처음으로 crate 를 만든다. 슬라이스 1 스코핑은 **이 문서 안에 있다 — 다시 조사하지 말 것.**

## 한 줄 상태 · 다음 첫 액션

- **상태:** ADR-0129 데몬 3층 분리의 **슬라이스 0(얽힘 풀기) 종료.** 0-1(포트 계약) · 0-2(어휘 이사·순환 해소) · 0-5(팬아웃 포트) 커밋 완료. 0-3은 흡수, **0-4는 사용자 결정으로 슬라이스 1 뒤로 이월**.
- **repo:** `master`, **19커밋 미푸시**, 워킹트리 클린. 데몬·앱 프로세스 0(정리됨).
- **★다음 첫 액션 = 슬라이스 1: 새 crate `engram-dashboard-net` 생성 + 4파일 이동★.** 아래 §슬라이스 1 실행 계획이 정본.
- **워크트리 분담(고정):** `master`(여기) = 데몬 분리 · `../engram-dashboard-wt2`(브랜치 `wt2`) = 주석 정리, **다른 세션 소관 — 건드리지 말 것.**

## 이번 세션 커밋 (7개, 전부 미푸시)

| 커밋 | 내용 |
|---|---|
| `92f05f0` | **슬라이스 0-1** — 네트워크 소유 포트 계약(`FrameSink`/`ConnectionHandler`) 도입 |
| `a91505b` | ADR-0129 "구멍" 서술 보완(팬아웃 = 두 번째 모양) + step-log S18.17 |
| `1ff585d` | **슬라이스 0-2** — `ws.rs`에서 에이전트·메시징 어휘 이사 → **양방향 순환 해소** |
| `c0dd078` | step-log S18.18 |
| `5c1146f` | **슬라이스 0-5** — 팬아웃 포트(`FrameFanout`) + 에이전트行 네트워크 의존 0 |
| `33b33f4` | step-log S18.19 |
| `37880fb` | step-log S18.20 — 슬라이스 0 완료 + **0-4 순서 변경(사용자 결정)** |

## 슬라이스 0이 만든 상태 (기계로 확인된 것)

| 게이트 | 값 |
|---|---|
| `rg "crate::ws::" daemon/src/` | **0** (에이전트行이 네트워크 타입을 전혀 모름) |
| `rg "use crate::" src/frame_port.rs` | **0** (포트 모듈 완전 독립) |
| `rg "use crate::connection_core\|engram_dashboard_core::agent\|AgentEvent" src/ws.rs` | **0 / 0 / 0** |
| `ConnRegistry::new()` in `lib.rs` | **1곳** (조립 함수 내부) |
| 포트 계약의 payload 어휘 | **0** |
| `register_for_test` | **0** (삭제 — 2슬라이스 전 예고대로) |
| `ws.rs` 크기 | 2218 → **1418줄** |

**`{ws, frame_port}` 가 닫힌 부분그래프** = 네트워크行이 먼저 이동 가능. 그게 슬라이스 1의 전제고 이미 성립한다.

## ★슬라이스 1 실행 계획 — 스코핑 완료본(다시 조사하지 말 것)★

**새 crate 이름 = `engram-dashboard-net`** (메인이 정함·보고됨). `transport` 는 코어의 `AgentTransport` seam 과, `wire` 는 프로토콜의 wire 계약과 혼동된다. **모듈명은 그대로 유지**(개명 최소화 = rot 최소화).

**이동 대상 4파일, 총 1,949줄:**

| 파일 | 줄 | crate-외부 의존 |
|---|---|---|
| `frame_port.rs` | 183 | **없음** — 완전 독립 |
| `instance.rs` | 157 | **없음** — 완전 독립 |
| `portfile.rs` | 191 | `protocol::DaemonInfo`(re-export) · `core::agent::platform::{pid_alive_with_start_time, current_process_start_time}` |
| `ws.rs` | 1,418 | `protocol::{AgentCommand, PROTOCOL_VERSION}` ← **0-4 이월분** + `crate::frame_port` |

**격리 게이트 = type-level**(메인 결정, 세션 초 확정). `portfile` 이 protocol·core 에 crate 의존을 지는 건 **허용** — `DaemonInfo` 는 데몬 메타데이터, `pid_alive_*` 는 프로세스 헬퍼로 **에이전트 어휘가 아니다**. 선례 = `discovery` 가 같은 이유로 core 에 full crate 의존.

**데몬 crate 에 남는 것:** `ws_e2e.rs`(2,774줄 — 조립부 `start_test_server_inner` 를 타므로) · `test_doubles.rs`(`#[cfg(test)]`, 새 crate 의 trait 을 지역 타입에 구현 = 허용) · 나머지 전부.

**예상 작업:** 새 crate + `Cargo.toml` + 워크스페이스 멤버 추가 · `pub(crate)` → `pub` 승격(경계 넘는 것: `ConnFrameSink::new`·`handle_connection`·`CONN_TX_CAP` 등 실측 필요) · `use` 경로 수정 · `ws.rs` 자체 테스트는 파일과 함께 이동.

**★슬라이스 1 리뷰어에게 반드시 알릴 것★:** **네트워크 lib 이 `protocol::{AgentCommand, PROTOCOL_VERSION}` 을 import 한 상태로 존재한다.** ADR-0129 결정 1의 "에이전트 어휘 타입 0" 불변식이 **이 구간만 의도적으로 미충족**이다(0-4 이월, 근거 = step-log S18.20). **회귀로 적출하지 말 것** — 안 알리면 리뷰가 여기서 멈춘다.

## 그 다음 순서

1. **0-4 (인증 이사)** — 슬라이스 1 직후. 그때는 네트워크 lib 이 discovery 무의존이라 `discovery → net`(핸드셰이크 타입)이 순환이 아니다. 지금 하면 순환(§미결 참조).
2. **슬라이스 2** — 에이전트 시스템 lib 추출(`connection_core`·`agent_conn`·`status_fanout`·`control/`·`messaging_host`·`experiment/`). **주의: 모듈 순환 3개**(`connection_core ↔ status_fanout`, `messaging_host → status_fanout → connection_core → messaging_host`)가 있어 **이 셋은 crate 경계의 같은 쪽에 둬야 한다**(전부 행 내부라 합법).
3. **슬라이스 3** — 얇은 조립 바이너리. **여기서 반드시 할 것(코드에 기록됨, `lib.rs` 파생 지점):** 조립 바이너리가 레지스트리와 팬아웃을 **둘 다 들면 짝 어긋남이 crate 경계에서 되살아난다.** 정공법 = 네트워크 crate 가 투영을 내주는 것 — `impl ConnRegistry { pub fn fanout(&self) -> Arc<dyn FrameFanout> }`. 파생을 또 베끼거나 인자 2개로 되돌리지 말 것. 또한 `run_accept_loop` 이 네트워크 수락 + 에이전트行 조립을 겸하고 있어 여기서 분해된다.

## 검증 상태

**재실행 명령(워크스페이스 루트):**
```bash
cargo build
cargo test -p engram-dashboard-core -p engram-dashboard-protocol \
           -p engram-dashboard-discovery -p engram-dashboard-messaging \
           -p engram-dashboard-daemon          # 1148 통과 / 0 실패
cargo fmt --check
rg "^\s*use tauri" crates/engram-dashboard-core/src/                                          # 0
rg "engram_dashboard_(core|daemon|protocol|discovery)" crates/engram-dashboard-messaging/src/  # 0
npx tsc --noEmit && npm test                   # 클린 / 41파일 634
# GUI 실측 (Windows 전용, 포트 9223) — 절차·함정은 아래 do-not 참조
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223" npm run tauri dev
```

**★검증 안 된 것★:**
- **슬라이스 1은 착수 0줄.** 위 계획은 **읽기 조사 기반**이고 컴파일로 검증된 바 없다. 가시성 승격 목록은 실측해야 확정된다.
- **포화-계속 테스트의 변이 탐지는 확률적**(HashMap 순회 순서 의존, K=20 반복). 거짓 실패는 불가(정상 코드는 순서 무관 통과). **K 는 탐지 노브지 튜닝 대상 아님(ADR-0038).**
- **`write_handle.abort()` 는 미검증이고 제거 금지** — "송신단 드롭이 자기종료를 보장"은 조건부이고, `read_task` 가 코어 등록과 `subs` 기록 사이에서 **패닉**하면 조건이 깨진다(그 구간엔 `await` 이 없어 취소는 못 끼어들지만 패닉은 끼어든다. release 는 `panic=abort` 라 디버그·테스트 한정).
- **진행 중 소켓 쓰기는 종료 신호로 중단 불가**(HEAD 동일, 선존) — recv 팔의 `sink_half.send` 가 `select!` 바깥이라 그동안 종료 신호도 keepalive 틱도 못 폴링한다.
- **정리가 코어 등록 직후 미기록 구독을 놓칠 수 있음**(선존, 위 abort 건과 같은 뿌리 — 코드에 양방향 교차참조됨).
- **`on_connect` 과잉 푸시 행은 로그에 안 잡힌다**(잡으려면 타임아웃 = 동작 변경).

## 실패한 접근 (do-not)

- **★cross-family 리뷰어(codex)에게 "너는 오케스트레이터가 아니다 + 환경의 스킬·에이전트 정의 전부 무시 + 되묻기 불가"를 프롬프트 맨 앞에 박지 않으면 실패한다 — 실발동★.** 안 박으면 repo 의 `review` 스킬을 자기가 실행할 것으로 읽고 "cross-family 리뷰어가 없다"며 되물음을 시도하다 죽는다(비대화 모드라 되묻기 불가). 검증된 프리앰블은 `scratchpad/blind-*.txt` 참조. 호출형 = `MSYS_NO_PATHCONV=1 codex exec --sandbox read-only -c model_reasoning_effort="high" -o <out> "$(cat <prompt>)" < /dev/null`.
- **★코더 세션 한도로 2회 중단됨 — 지시서를 "값싼 것 먼저" 순으로 배치할 것★.** 중단되면 앞의 값싼 성과는 디스크에 남는다. 중단 후엔 **반영 여부를 grep 으로 확인**하고 재개(1회는 "아무것도 안 반영"이었고 1회는 일부 반영이었다 — 후자에서 메인이 "전부 미반영"으로 오보했다).
- **★리뷰어가 트리를 읽는 동안 코더를 붙이지 말 것★**(공유 트리 변형 금지). 변이 테스트도 리뷰어 가동 중엔 금지.
- **★"~뿐" 류 열거를 쓰기 전에 어휘 범위 전체를 확인할 것 — 이 세션 지배적 결함★.** 코드가 아니라 **코드에 대한 산문**이 계속 틀렸다: 거짓 보장 6건, 거짓 문장을 고치며 다른 거짓 문장 삽입, 못 대는 확률 수치. **열거가 3라운드 연속 한 칸 짧으면 열거를 규칙으로 바꿔라**(규칙은 한 칸 짧을 수 없다).
- **★메인 자신의 실패: 디렉토리 범위 grep 을 crate 전체 명제로 2회 보고★** — "팬아웃이 어느 시그니처에도 없다"(에이전트行엔 있다) · "레지스트리를 파라미터로 받는 함수 없음"(`handle_connection` 이 받는다). **두 번 다 그 과잉 일반화가 전이 운반자를 가렸다.** 두 문장은 `lib.rs` 에 *"둘 다 거짓이니 인용 금지"* 로 박제됨.
- **테스트 이름이 몸통이 검증 안 하는 걸 주장 금지** — 관측 불가한 속성은 이름에서 빼라("한 번만 인코딩"이 실제 그렇게 처리됨).
- **GUI 실측 함정 3개:** ① `daemon_start` 는 프로세스 기동일 뿐 — 연결은 별개라 `daemon_connection_state` 가 `down` 이면 **`daemon_connect` 를 명시 호출**해야 붙는다(회귀 아님) ② **xterm 은 캔버스 렌더라 `innerText` 가 0** — 출력 확인은 `shot` + Read 로 ③ `cdp.mjs eval` 스크립트가 길면 조용히 깨진다 — 단계를 쪼갤 것.
- **실측 teardown:** 런처 트리를 PID 로 죽인다(`CommandLine` 매칭으로 식별 — `tauri.js dev` / `vite/bin`). **무관한 `node.exe` 를 죽이지 말 것**(codex 런타임이 섞여 있다). 데몬은 자기가 띄운 것만.
- **실행 중 데몬이 바이너리를 잡아 `cargo build` 가 os error 5 면 — 죽이거나 파일을 옮기지 말고** 패키지 스코프로 좁히거나 보고할 것(코더가 1회 잠긴 exe 를 rename 해 우회했다).
- bare `cargo test` 금지(멤버별 `-p`) · Fable 워커 금지 · **Agent 스폰 시 model/프리셋 명시 필수** · git-bash 경로 인자엔 `MSYS_NO_PATHCONV=1` · **커밋 전 `git checkout -- crates/engram-dashboard-protocol/bindings/`**(cargo build 가 줄바꿈만 더럽힌다) · **`git add -A`/`commit -a` 금지**(신규 파일은 명시 add) · ws_e2e case09 flaky(임계값 튜닝 금지 — ADR-0038)

## 정지 조건

- **ADR 본문(결정 텍스트) 수정 = 사용자 승인.** (이번 세션 ADR-0129 에 note 추가 — 승인받음.)
- **슬라이스 경계·계획 변경 = 사용자 결정.** (0-4 순서 변경이 그 사례.) 그 밖의 내부 구현(코드 구조·자료구조·테스트 전략·리뷰어 판정 충돌 해소·커밋 시점)은 **메인이 정하고 보고** — 사용자가 이 세션에서 2회 지적한 항목이다.
- **wire JSON 변경이 필요해지면 멈추고 재론** — ADR-0129 전제(프론트 무변경)가 깨지면 비용 계산이 달라진다.
- **리뷰 정면 대립은 메인이 근거로 갈랐다면 올리지 않는다** — 못 가릴 때만. (이번 세션 cross-family 가 same-family 를 3회 이겼고, 전부 메인이 코드를 직접 읽어 판정했다.)
- **프로덕션 코드에 test-only 훅 추가 = 사용자 결정.** (`#[cfg(test)]` 접근자는 예외 — 메인 판단으로 승인한 선례 있음: `contains`·`register_for_test`.)
- `.gitattributes` 추가 임의 금지 · **Engram repo(`I:\Engram`)에서 `git add -A`/`commit -a` 금지**(중첩 repo gitlink).

## 미결 (착수를 막지 않음)

- **0-4 가 지금 불가한 이유(재조사 불필요):** 인증은 데몬↔클라이언트 **공유 계약**이다 — `discovery::build_auth_command`(StopDaemon 발사)와 `src-tauri/src/cli.rs` 가 각자 만든다. `daemon → discovery` 가 이미 있어(경로 헬퍼 `default_data_dir`·`find_install_root`) 지금 옮기면 순환. **거부한 대안 2개** = ① 작은 핸드셰이크 crate 신설(영구 멤버 추가 + `AgentCommand` variant 제거로 전수 매칭 변경) ② 프로토콜 영구 존치 + 불변식 축소(ADR 본문 수정).
- **`bin/engram-send.rs`(1,528줄)는 직교** — `engram_dashboard_*` 의존 0. 어느 crate 의 `[[bin]]` 에 둘지만 정하면 된다(조립 crate 가 자연스러움).
- **MCP 제어 평면(`control/mcp_server.rs`, axum, 자체 포트)은 네트워크 lib 으로 안 간다** — ADR-0129 가 "제어 평면"을 에이전트 시스템에 배정했다.
- **`src-tauri` 교차 의존:** `src-tauri/src/daemon_client/tests.rs` 가 `engram_dashboard_daemon::start_test_server()` 를 직접 부른다. **조립 crate 이름이 바뀌면 같은 PR 에서 고쳐야 한다.**
- **선존 stale 주석:** `connection_core.rs:3/:5/:494/:496`(슬라이스 0-1 이후 `agent_conn.rs` 를 가리켜야 함) · `logging-conventions.md` 의 `ws.rs:344-350/:416/:478` · `tests/ws_e2e.rs` 의 `ws.rs:604/:610`. 전부 HEAD 부터 stale.
- **허수 테스트 1건(선존):** `subscribe_control_order_ack_then_complete` — 프레임 3개를 새 채널에 넣고 FIFO 만 단언, 프로덕션 dispatch 무접촉. 이번 이동과 무관.
- **스킬 피드백 적립됨:** review(cross-family CLI 프리앰블 필요 — 안 박으면 죽는다 / 델타 재리뷰의 인원 규정 부재) · implement(코더 세션 한도 중단 시 재개 절차 부재).
- 승계분: `agents.json` 동명 5건 · 접미사 `(1)` 미검증 2건 · `reply_failed` 조회 표면 부재 · `profiles_cb` 주입 잔존 · `~/.claude/settings.local.json` 허용 규칙 38개 무시 · settings.json 하드링크(DevMode OFF) · `--disallow-mcp` 존폐(권고 = 플래그·env seam·테스트 3건 함께 삭제) · 실측 ⑧ 프로덕션 훅(수신자별 인과, 사용자 결정 필요).

## 참조 (읽어야 할 것만)

- **결정 정본:** `docs/decisions/0129-*.md` — 목표 모양 · 거부한 대안 3개 · **결정 1 아래 2026-08-04 note(구멍은 두 모양)** · 영향/불변식(ADR-0029 embedded 부활 아님)
- **흐름:** `docs/process/step-log.md` **S18.17~S18.20** — 슬라이스별 무엇/왜 + 알려진 잔여 + 반증된 접근
- **코드 — 슬라이스 1의 무대:** `crates/engram-dashboard-daemon/src/frame_port.rs`(포트 정본, 독립) · `ws.rs`(네트워크行 전부) · `instance.rs`·`portfile.rs`(clean) · `lib.rs` 의 `DaemonWiring` 주변(짝 어긋남 규칙 + 슬라이스 3 승계가 여기 적혀 있다)
- **격리 하네스:** `test_doubles.rs` — 더블은 네트워크 정책을 흉내내지 않는다(기록만). 슬라이스 2에서 에이전트行 테스트가 이걸 쓴다.
