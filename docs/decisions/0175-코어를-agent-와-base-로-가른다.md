# ADR-0175: 코어를 agent 와 base 로 가른다

- 상태: 확정 (2026-08-25, 근거: 사용자 결정 + trd full 리뷰 2인)
- 관련: ADR-0151(crate 판정 기준 — §영향의 「덩어리별 판정」 기대와 일부 갈린다, 아래 §거부한 대안) · ADR-0003(코어 격리) · ADR-0155(command crate) · ADR-0046(replay single-flight) · ADR-0174(셸 lib 테스트 하네스) · step-log S21 · Amends ADR-0151 (바닥 crate 구성)

## 맥락

`engram-dashboard-core`(30파일 · 20,790줄 · 프로덕션 10,423줄, 실측 2026-08-25)는 **데몬의 에이전트 런타임**인데 성격이 전혀 다른 소비자 넷이 통째로 의존한다. 실제로 만지는 표면은 서로 겹치지 않는다:

| 소비자 | distinct 심볼 | 무엇 |
|---|---|---|
| `daemon` | **94** | `agent::*` 91 · `persistence` 2 · `logging` 1 |
| `src-tauri`(셸) | **13** | `logging` 4 · `replay_flight` 8 · `agent::commands::COMMAND_SPECS` 1 |
| `net` | **2** | 둘 다 `agent::platform::*`(PID liveness) |
| `discovery` | **1** | `agent::platform::pid_alive_with_start_time` |

즉 셸·net·discovery가 닿는 것은 **바닥 인프라뿐**인데 그것 때문에 에이전트 런타임 전체를 의존한다. 코어에 **optional 의존이 0건**(feature-gate도 0)이라 `portable-pty`·Job Object·`tracing-subscriber`가 클라 빌드에 그대로 딸려 들어간다.

역방향 사례도 있다 — `replay_flight`(860줄)는 코어에 있는데 **셸만 쓴다**(데몬은 컴파일 의존 0, 산문 주석 1줄뿐: `daemon/src/connection_core.rs:1494`). 그 파일 헤더가 대는 존치 사유는 *"이 위치 덕에 headless 에서 단위테스트가 실행된다 — src-tauri 테스트는 미실행"*인데, **ADR-0174(2026-08-24)가 셸 테스트 타깃을 세우면서 죽은 전제**다.

## 결정

1. **`engram-dashboard-base` 신설**(잎 crate) — `logging`(692줄) + `platform`의 PID 판정 헬퍼(`process.rs` 325줄). 로깅이 딸고 있던 `regex`·`tracing-subscriber`, PID 헬퍼가 딸고 있던 `windows`가 함께 이사한다.
2. **`engram-dashboard-core` → `engram-dashboard-agent` 개명.** 남는 내용 = 에이전트 런타임 전부(수명·펌프·backend·transport·도메인 모델·`commands`·`types`·`persistence`) + **Job Object 래퍼**(`platform/windows.rs` — 소비자가 `agent` 안뿐이라 남긴다). 안쪽 `agent/` 모듈은 crate 루트로 접어 올린다(`engram_dashboard_agent::agent::types` 겹침 방지).
3. **`replay_flight` 는 셸 안의 모듈로 이사**한다(crate 로 승격하지 않는다). `src-tauri/src/daemon_client/replay_flight.rs` 의 4줄짜리 재수출이 실물로 바뀌므로 소비자 import 경로는 안 바뀐다. 헤더의 죽은 존치 사유와 데몬 주석의 경로도 같은 커밋에서 고친다.
4. **셸 → `agent::commands::COMMAND_SPECS` 간선은 이번에 남긴다.** 다만 방치가 아니라 **같은 브랜치의 뒤 단계**에서 없앤다 — 웹뷰가 답하는 명령에 `ui.` 접두를 붙여 이름 충돌 자체를 없앤 뒤(결정 5), 그 상수를 읽는 필터를 삭제한다.
5. **명령 이름 접두는 웹뷰가 답하는 것에만 `ui.` 를 붙인다.** 데몬(`agent.*`)·셸(`tab.*`·`slot.*`)은 그대로 둔다.
6. **crate 판정에 무게 기준을 더한다(사용자 결정 2026-08-25):** *lib 은 무게를 서로 비슷하게 맞추고, **파일 하나짜리 lib 은 만들지 않는다.*** 이 조항이 아래 거부한 대안 3건의 공통 근거다.

실행 순서(같은 브랜치) = ① 개명 ② `base` 신설 ③ `replay_flight` 이사 ④ `ui.` 접두 ⑤ 간선 제거 + 충돌 시 앱 표면 고지.

## 거부한 대안

- **`engram-dashboard-logging` 과 `engram-dashboard-platform` 을 각각 독립 crate 로**(blind 리뷰어 GPT 권고 · doc-aware 리뷰어도 「덩어리별 판정」으로 같은 방향) — **사용자 결정으로 기각**: *"기능 추가할 때마다 모듈 만들 순 없다. 과해지면 그때 분리하자."* 그리고 로깅만 떼면 **파일 하나짜리 lib** 이 되어 결정 6 에 걸린다. ★**이 기각은 ADR-0151 §영향의 기대와 갈린다**★ — 거기 *"덩어리별로 따로 판정한다 — 셋을 뭉쳐 하나에 넣을 근거는 없고, 목적 이름을 가진 작은 crate 여러 개가 나올 수 있다(`bevy_core` 실패도 피한다)"*라고 적혀 있고, 이 결정은 둘을 한 자루에 담는다. **기각 근거 = 사용자 판단이지 실측이 아니다(자평: 약함).** 그래서 되열릴 수 있는 자리로 명시해 둔다 — 되열 조건 = `base` 에 세 번째 입주자가 들어오려 할 때.
- **`engram-dashboard-agent-command` 신설**(선언부 110줄만 떼어 셸의 `agent` 의존을 0 으로 — GPT 권고안 2) — 결정 6(파일 하나짜리 lib 금지)에 걸려 기각. 같은 목적을 결정 4·5(접두 + 필터 삭제)가 crate 를 늘리지 않고 달성한다.
- **`replay_flight` 를 독립 crate 로**(doc-aware 리뷰어 권고 — ADR-0151 §영향이 *"재생 상태기계 610줄"* 로 이름 지어 최상위 바닥 crate 후보로 지목했고, 리뷰어가 ADR 시점 커밋에서 줄 수를 대조해 동일 대상임을 확인했다) — 결정 6 에 걸려 기각(프로덕션 366줄 단일 파일). ★**소비자가 하나라서 기각한 것이 아니다**★ — ADR-0151 결정 1 이 *"지금 실제 소비자가 있는지는 묻지 않는다"* 로 그 축을 이미 폐기했다. 기각 축은 **무게**다.
- **쪼개지 않고 cargo feature 로 무거운 의존을 optional 로 돌린다**(옵션 C) — 기각. ① 소유권 경계의 대안이 아니라 빌드 최적화일 뿐이다(GPT) ② **셸이 `daemon` 을 dev-dependency 로 물고 있어**(`src-tauri/Cargo.toml:97`) feature 가 합쳐지므로 `cargo test -p engram-dashboard` 에서는 이득이 사라진다(doc-aware 리뷰어 실측) ③ 9,277줄 crate 내부에 `#[cfg(feature)]` 를 뿌려야 해 침습도가 crate 를 만드는 것보다 크다.
- **반대로 crate 를 합친다**(옵션 D — `command` 를 코어로 되돌리는 등) — 기각. 두 리뷰어가 일치했고, `command` 의 존재 이유는 ADR-0155 가 이미 박았다(독립적으로 쓸 수 있고 순환을 막는다). **단 `discovery` 접기(T-10)는 이 기각에 들지 않는다** — 별건으로 살아 있고, GPT 실측으로 T-10 설명보다 실제 사용 범위가 커졌다(`daemon/src/lib.rs:62,428` · `daemon/src/control/priming.rs:288`).
- **`base` 대신 `-core`·`-common`·`-util`·`-sys`** — `-core` 는 Rust 관용 1순위지만(`tracing-core`·`futures-core`) 그 관용의 뜻이 *"그 추상의 최소 커널"* 이라 로깅+PID 라는 가로지르는 인프라에는 안 맞고, 개명으로 이름이 비더라도 옛 기록 전체가 어느 쪽을 가리키는지 흐려진다. `-common`·`-util` 은 잡동사니 이름. `-sys` 는 Rust 에서 C FFI 바인딩을 뜻하는 예약 관행.
- **`daemon.` · `shell.` 접두까지 대칭으로 붙인다** — 기각 근거 둘: ① `agent.spawn` 은 LLM·사용자가 실제로 치는 이름이라 개명하면 기존 사용·문서·키바인딩이 어긋난다(웹뷰 쪽은 **이미 버스 등록에서 걸러지고 있어** 대외 비용 0 이다) ② 데몬이냐 셸이냐는 **위치**고 위치는 바뀐다 — `tab.create` 를 나중에 데몬으로 옮기면 `shell.` 접두가 거짓이 되어 또 개명해야 한다. `ui.` 는 위치가 아니라 범주("붙어 있는 UI 가 답한다")라 이사를 통과한다.
- **`webview.` 접두** — WebView2 라는 기술을 이름에 박아, 모바일 클라이언트가 붙는 순간 거짓이 된다.

## 근거

- **소비자별 심볼 실측**(위 §맥락 표) — `rg -o "engram_dashboard_core::[A-Za-z0-9_:]+" | sort -u` + 별칭·중괄호 전개 보강. `net` 은 정확히 2개(`portfile.rs:86` 운영 + 인라인 테스트 2곳), `discovery` 는 1개(`lib.rs:1012`) — **둘 다 PID 헬퍼뿐이라 결정 1 로 `agent` 의존이 0 이 된다.** 이것이 이번 정리의 확정된 성과다.
- **코어 내부 응집 실측** — `logging` 은 나가는 참조 0 · 들어오는 것 1줄(`transport/stdio.rs:30`), `platform` 은 나가는 참조 0 · 들어오는 것 2줄, `replay_flight` 는 **들어오는 것도 나가는 것도 0**. 셋 다 떼어내도 나머지가 흔들리지 않는다. 반면 수명·펌프 ↔ transport ↔ backend 는 **모듈 수준 순환**(12,685줄)이라 한 crate 안에 유지한다(두 리뷰어 일치).
- **가시성 실측** — `logging`·`platform` 에 `pub(crate)`/`pub(super)` 항목 **0건**. crate 경계를 넘어도 승격할 것이 없다.
- **`replay_flight` 가 wire 계약이 아님** — 그 파일 헤더가 명시: 경계 마커 tag 255 는 데몬 codec 에 없는 **셸↔웹뷰 Channel 내부 계약**이다. 데몬은 그 태그를 모른다.
- **trd full 리뷰 2인**(2026-08-25) — doc-aware(Claude) `BLOCK` · blind(GPT) `FIX`. 원안(`base` 에 로깅+플랫폼 통째, `-core`/`-common` 만 이름 대안으로 검토)의 결함 다수가 여기서 나왔고 위 결정에 반영됐다. 남은 대립 1건(`replay_flight` crate 여부)은 사용자가 결정 6 으로 판정했다.
- **`COMMAND_SPECS` 필터가 지금 필요한 이유** — 데몬은 *자기가 답하는 이름*이 하나라도 실린 등록 패킷을 **통째로 반려**한다(`connection_core.rs:675` `refuse_names_i_answer` — 겹친 것만 빼 주지 않는다). 프론트 레지스트리에 `agent.spawn`·`agent.rename` 이 실재하므로(`src/commands/agentCommands.ts:41,76`), 필터를 먼저 없애면 매 부팅 등록이 반려돼 **셸의 창·탭·슬롯 이름이 통째로 명부에 못 오른다.** 그래서 접두(결정 5)가 간선 제거(결정 4)보다 **먼저**다.

## 영향 / 불변식

- **의존 그래프(목표):** `base`·`command`·`messaging` = 잎 · `protocol → command` · `agent → command, base` · `net → base, protocol` · `discovery → base, protocol, net` · `daemon → agent, base, protocol, discovery, messaging, net, command` · `src-tauri → base, command, protocol, discovery, net`(+ 결정 4 가 닫힐 때까지 `agent` 1심볼).
- **`base` 입주 조건(헤더에 박는다):** ① 소비자가 둘 이상 ② 도메인 지식 0 ③ 이 crate 안에서 서로를 참조하지 않을 것. ★**세 번째 입주자가 생기면 위 「거부한 대안」 첫 항목을 다시 연다**★ — 그때가 `bevy_core` 경로에 들어서는 지점이다.
- **개명이 조용히 깨뜨리는 게이트(같은 커밋에서 함께 고친다):**
  - `.github/workflows/ci.yml:399` — 메시징 격리 정규식이 crate 이름 알파벳을 손으로 박는다. 새 이름을 안 더하면 벽에 구멍이 난다.
  - `ci.yml:481-498` — net 의 core 심볼 allowlist(정확히 2줄). 개명 후 「0줄 기대」로 고치면 **어떤 위반으로도 깨질 수 없는 죽은 게이트**가 된다 → `engram_dashboard_agent::` 로 재조준하되 ★**기대값은 2 그대로**★다. net 이 무는 심볼 수는 개명으로 변하지 않았다 — 모듈 접기로 경로만 `::agent::platform::` → `::platform::` 으로 짧아졌을 뿐이다. **0 기대로 내리고 `engram_dashboard_base::` allowlist 를 새로 세우는 것은 결정 1 의 `base` 가 실제로 서서 PID 헬퍼가 그리로 이사한 뒤**다(같은 문장이 `ci.yml` 의 그 게이트 주석에도 박혀 있다).
  - `ci.yml:271-272` — 생성물 sync 게이트의 `crates/engram-dashboard-core/bindings/` 경로.
  - **`.claude/skill-bindings/qa.md`** — 이 파일의 **코어 격리 게이트 두 자리**(standard 블록의 `use tauri` 줄 + 맨 끝 「코어 격리 불변식」 절)가 CI 와 달리 경로 부재 분기가 없어, 개명 후 **없는 경로를 훑고 통과로 읽힌다**. ADR-0003 을 지키는 로컬 유일 게이트다.
    - ★**여기에 줄 번호 목록을 박지 않는다**★ — 개명 커밋이 그 게이트 앞에 경로 존재 가드(4-pre)를 끼워 넣으며 뒤 자리가 한 줄씩 밀렸다. 자리는 세지 말고 찾을 것: `rg -n "engram-dashboard-agent|engram_dashboard_agent" .claude/skill-bindings/qa.md`.
    - ★**그 문자열 grep 이 전부가 아니다**★ — 개명 커밋이 이 파일에서 실제로 고친 자리는 **12줄**이다(`git show bd38d324 --numstat -- .claude/skill-bindings/qa.md` → 13 추가 / 12 삭제. 추가가 한 줄 더 많은 것은 순수 신설인 4-pre 경로 부재 가드 때문이다 — 실측 2026-08-25). 그중 옛 crate 이름 문자열은 **7줄**뿐이고, 나머지 **5줄**은 이름 grep 에 안 걸리는 세 갈래다: 이름 없이 「core」로만 부르던 산문 **3줄** · 메시징 격리 정규식이 손으로 박아 둔 이름 알파벳 **1줄** · 그 신설 가드를 설명하려 함께 고친 판정 산문 **1줄**(이 한 줄은 개명 수정이 아니라 가드 문서다). 다음 개명도 같은 사각을 만난다.
  - 새 crate 는 게이트를 **세 곳**(`ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `qa.md`)에 등록해야 한다. 이름 접두 `engram-dashboard-` 를 지켜야 상한 게이트가 본다(ADR-0151 「개명 함정」).
- **`replay_flight` 이사 뒤 침묵 축:** 그 단위테스트 494줄이 셸 `lib_unit` 타깃 아래로 들어간다. **ADR-0174 의 세 다리 중 `[[test]] lib_unit` 선언이 사라지면 실패가 아니라 침묵으로 증발**한다 — 이사가 그 지붕 아래 물량을 늘린다는 사실을 함께 안다.
- **검증 잣대:** 워크스페이스 회귀의 **총 통과 개수가 그대로**여야 한다(2,102 통과·0 실패, 실측 2026-08-25). 타깃별 분포는 바뀌지만 합계가 어긋나면 무언가 사라진 것이다. 개명 커밋의 diff 는 "이름 말고 바뀐 게 있나 → 없다"로 검토된다.
- **남는 무검증/미측정:** 셸 exe 크기 감소분은 재지 않았다(링커가 미사용 심볼을 얼마나 걷는지 미측정). `daemon` dev-dependency 때문에 `cargo test -p engram-dashboard` 는 결정 4 가 닫혀도 `agent` 전량을 컴파일한다 — 릴리스 빌드만 이득이다.
