# TRD — LLM 제어 표면 (engram-ctl)

> 상태: **작성 중.** PRD(요구·`/review prd` 통과 2026-07-14) → 이 문서(어떻게·인터페이스 확정). 굵은 결정은 ADR로 박제, PRD 미결을 여기서 소진.
> 앵커: PRD `./prd.md` · ADR-0080(제어표면 아키텍처) · ADR-0014(CLI-via-Bash) · ADR-0035(ViewManager=UI 권위).

## 현행 지형 (실측 — 무엇이 이미 있고 무엇이 net-new)

**이미 있음 (재사용 — 신규 설계 아님):**
- **request_id correlation** — 전 side-effect `AgentCommand`가 `RequestId(Uuid)` 보유, 데몬이 `Ack`/`Error`/전용 reply(`Spawned`·`AgentList`·`Snapshot` 등)로 echo, 앱 `DaemonClient`가 `PendingMap[request_id]→oneshot`로 매칭. (`crates/engram-dashboard-protocol/src/messages.rs`) → engram-ctl가 그대로 재사용.
- **백엔드 명령 전량** — spawn/kill/interrupt/write/resize/list + lease + profile·preset CRUD + snapshot 모두 이미 wire+dispatch(`connection_core.rs`). engram-ctl는 이걸 **보내기만** 한다.
- **portfile discovery + WS + Auth** — 스파이크 `scripts/engram.mjs`가 증명(daemon.json → WS → Auth → request_id 매칭). Rust 재작성만.

**Net-new (이 TRD가 설계):**
- **UI relay** — 오늘 cross-client 라우팅 없음(모든 연결이 대칭 peer, broadcast만). `CLI → 데몬 → 앱 → ViewManager → 결과 → CLI` 경로가 통째 신설.
- **앱 role 등록** — 데몬이 "어느 연결이 UI 앱인가"를 구분 못 함(연결에 role 없음). 앱이 자기 정체를 알려야 relay 대상 지정 가능.
- **앱 inbound UI-command 핸들러** — 앱은 현재 event 소비 + outbound 명령만. 데몬→앱 명령 수신→ViewManager 적용 경로가 없음.
- **dedup store** — request_id는 있으나 at-most-once dedup 저장소 없음(R6 멱등성 요구).

## 컴포넌트 설계

### 1. engram-ctl (Rust CLI) — 스파이크 정식화
- portfile discovery(discovery crate 경로 재사용) → WS connect → Auth 프레임 → 명령 send → request_id로 reply 매칭 → stdout JSON 봉투(`{v,ok,requestId,result}`/`{error}`) → exit code.
- 백엔드 명령 = 기존 AgentCommand 직송. UI 명령 = relay 봉투(§3).
- CLI 문법 = **noun-verb 그룹형**(`agent`/`view`/`obs` + `list-commands`) — 결정 2026-07-14. 카탈로그 확장 시 동사 충돌·모호 회피.

### 2. 백엔드 명령 경로 — 기존 그대로 + dedup
- 기존 AgentCommand 재사용. 추가 = **dedup store**(§5): 같은 requestId 재시도 at-most-once.

### 3. UI relay (net-new 핵심 — ADR 대상)
- **앱 role 등록:** 신규 `RegisterRole { UiApp }`(auth 직후 앱만 전송). 데몬이 그 ConnId를 "UI 앱 연결"로 표시. (단일 앱 인스턴스 전제 = PRD 범위 → 대상 주소지정 불요.)
  - ★**재연결 재전송(F)**★: 앱은 **첫 auth뿐 아니라 매 재연결마다** `RegisterRole{UiApp}`를 재전송한다(재연결 핸드셰이크에 태움). 안 그러면 재연결 후 데몬에 UI-앱 대상이 없어 모든 UI relay가 잘못 `APP_OFFLINE`로 실패한다. 중복 UiApp 등록은 **last-registration-wins**(새 ConnId가 stale 것을 대체 — 옛 ConnId 표기 폐기).
- **relay 봉투(opaque, single request_id):** `CLI → 데몬`: `RelayUi { request_id, payload(opaque JSON value) }`. `데몬 → 앱`: `UiCommand { request_id, payload }`. `앱 → 데몬`: `UiResult { request_id, result }`. `데몬 → CLI`: 같은 request_id로 결과 반환. ★**request_id는 CLI가 준 것을 end-to-end로 그대로 흘린다**(별도 `relay_id` 네임스페이스 없음 — 데몬은 `request_id → 원 ConnId`만 저장).★ 데몬은 **payload 미파싱**, 라우팅 표만 유지(opaque bridging). payload는 봉투 안 **opaque JSON value**(wire가 raw value를 허용하는 곳에서 JSON-in-JSON 문자열 회피).
- **★비블로킹 relay dispatch(B)★:** 데몬 `RelayUi` 핸들러는 `dispatch` 안에서 앱 왕복을 **await하지 않는다** — `ws.rs`가 연결당 명령을 한 번에 하나씩 읽어 dispatch하므로(read/dispatch가 순차) await하면 그 CLI 연결의 명령 stream이 head-of-line 블록된다. 대신: (1) 라우팅 표에 엔트리 등록 + (2) 앱 연결로 `UiCommand` enqueue + (3) **즉시 `DispatchFlow` 반환**. 상관은 나중에 앱 연결 read 경로로 `UiResult`가 도착하면 비동기로 완료된다.
- **★relay 라우팅 표 + 수명 경계(D)★:** 엔트리 = `{ request_id → 원 ConnId, deadline }`. evict 트리거 = (1) `UiResult` 수신, (2) **어느 쪽 엔드포인트든 연결 cleanup**(데몬 연결정리 경로가 그 ConnId로 키된 relay 엔트리를 sweep — 원-CLI 측·앱 측 **양쪽**), (3) timeout. 모르는/중복 request_id의 `UiResult` → drop(기존 "모르는 request_id 무시" 방어 미러).
- **앱 적용 경로 = (A) 공유 `ViewCommand` 적용 서비스 + async 상관** (결정 2026-07-14, /review trd 정밀화) — 앱이 relay 받은 UI 명령을 **사람 Tauri 경로와 공유하는 transport-중립 `ViewCommand` 적용 서비스**로 ViewManager에 적용한다(단일 경로 — §5 "사람 클릭도 같은 핸들" 정합). ★단, 그 공유 경로는 **Tauri command 엔트리를 액터 위에서 재-invoke하는 방식이 아니다**★: relay 인바운드 핸들러는 적용을 **`daemon_client` 액터 태스크 밖(spawn된 태스크)에서** 돌리고 결과를 `request_id → oneshot`로 액터에 되돌린다 → 액터는 소켓 서비스를 계속해 재진입 데드락을 피한다. (B) ViewManager 직접 호출 거부 = 2경로 동기화·lock-ordering 리스크(§5 위반).
- **★Opaque 결합 가드(H)★:** `protocol` crate의 relay 봉투는 **opaque payload만** 나른다. UI 명령 enum + payload→`ViewCommand` dispatch 맵은 **오직 `src-tauri`**에 산다. `engram-ctl`·데몬 relay·`core`는 UI 명령 enum을 **import하지 않는다**(core는 tauri-free 유지, ADR-0003). 격리 게이트 `rg "use tauri"` → 0줄 리마인더를 **engram-ctl에도 확장**(engram-ctl은 UI 의미 무지 — opaque 봉투만).
- **ADR 대상:** "앱 = 데몬으로부터 명령을 수신하는 WS peer"는 새 능력 → ADR 신설(또는 ADR-0080 확장). 거부 대안 = broadcast+앱필터 / 앱-소유 별도 엔드포인트(ADR-0080에서 이미 거부).

### 4. 관찰 primitive — 기존 Subscribe/epoch/seq/replay 위
- `wait --until` · `output tail --until message-done`(턴 경계 계약 = PRD) · `events poll --cursor` · `list_commands`.
- **events poll cursor = §5 결정**(데몬 monotonic 이벤트 커서 + bounded in-memory ring 저널 + 밀려나면 RESET→`agent list` full 재조정).

### 5. PRD 미결 소진 — 결정 (실측 그라운딩 2026-07-14)

- **토큰 주입 + discoverability = spawn env 오버레이 (인프라 기존).** `CommandSpec.env: Vec<(String,String)>`가 이미 `profile.env → build_spec → transport pty cmd.env(k,v)`로 흐른다(`manager.rs:254-260` · `backend/claude.rs` · `transport/pty.rs:74`). 주입 seam = `manager.rs` spawn 직전(build_command_spec 전). **비영속 오버레이**(profile.env에 안 넣음 — agents.json 평문 저장 금지, CLAUDE.md). 주입 항목:
  - **제어 토큰** = env var(예 `ENGRAM_CONTROL_TOKEN`) = 현행 마스터 토큰(MVP; per-child = R7 보류/T-11).
  - **engram-ctl discoverability = PATH prepend**(전용 env var 아님) — LLM Bash가 `engram-ctl`을 이름으로 자연 해소, 별도 CLI 인지 불요(env var면 `$ENGRAM_CTL` 명시 호출 필요 = UX 열등). 번들 dir을 PATH 앞에.
- **write(stdin) ⟂ lease = per-call 자동 획득 (결정 (a), 2026-07-14).** engram-ctl `write`는 lease가 **비어 있으면 그 단일 호출 범위로 자동 획득**한다(한 호출 안에서 acquire→write→release). 사람(또는 다른 보유자)이 쥐고 있으면 → `LEASE_DENIED`(정직한 실패). ★이 설계가 per-connection lease 증발과 디커플링★: engram-ctl은 호출마다 새 WS 연결을 여니 per-connection lease는 호출 사이에 사라진다 — 그래서 LLM `write` 경로는 **호출 간 lease 보유에 의존하지 않는다**. (명시적 사람식 보유를 위한 standalone `input acquire`/`release`는 여전히 존재할 수 있으나, LLM write는 거기에 의존 안 함.)
- **dedup store = 데몬-전역 맵(request_id → 캐시 결과), 수명 = 바운드(재시도 창 규모 — 세션/데몬 lifetime 아님).** ★핵심: engram-ctl은 per-call 스폰(호출마다 새 WS 연결)이라 per-connection dedup 불가 → **반드시 데몬-전역**.★
  - **단일 request_id로 상관+dedup 통합(C):** at-most-once는 **호출자가 안정 키를 줘야** 성립한다(새 engram-ctl 호출 = 새 uuid → dedup 안 됨). engram-ctl은 **재사용 가능한 `--request-id <k>`**(옵션)를 노출해 **상관과 dedup을 한 정체성으로** 굴린다(옛 별도 `--idempotency-key` 대체). 있으면 데몬이 그 request_id로 dedup(at-most-once, 캐시 결과·`ALREADY_APPLIED`), 없으면 engram-ctl이 fresh uuid 생성 → at-least-once(재시도 이중적용 가능). PRD R6 스탠스 정합.
  - **수명 = 바운드(D — "데몬 lifetime 무한 누적" 아님):** 재시도 창 위 **TTL**(초~분 규모 — 세션 아님) 또는 **명시 크기의 LRU 캡**. 재시작 리셋(PRD)은 유지되나 그것이 상한은 아니다.
  - **커밋 타이밍(D — critical):** relay UI 명령은 dedup 엔트리를 **성공 `UiResult`(적용 후)에만 커밋**한다 — **relay 전 사전 캐싱 금지**(사전 캐싱 = timeout 시 거짓 "성공" 캐시 → PRD "UI 성공은 ViewManager 적용 후에만 확정" 위반). in-flight 중복(같은 request_id가 첫 relay 진행 중 재도착) → **같은 pending 결과에 coalesce**(이중 적용 안 함).
- **순서 = 앱측 공유 `ViewCommand` 적용 서비스 + async 상관 (신규 큐 불요, 재진입 데드락 회피).** ★옛 주장("ViewManager Mutex만으로 순서 충분·신규 큐 불요, re-entrancy 없음")은 **compound 명령에서 거짓 → 폐기**★: `spawn_into`(`commands/layout.rs`) 같은 합성 UI command는 자기 안에서 `client.send_command(...).await`를 부르는데, 그 reply는 **같은 `daemon_client` 액터의 stream 루프 안에서만** 해소된다. relay 인바운드 핸들러를 그 액터에서 **인라인으로 돌리며 그런 Tauri command를 await하면 → self-deadlock**(액터가 자기 reply를 못 꺼냄). 대신: 사람 Tauri 경로와 relay가 **transport-중립 `ViewCommand` 적용 서비스**를 공유하되(§5 단일 경로 유지 — Tauri command 엔트리 재-invoke 아님), relay는 그 적용을 **액터 밖(spawn 태스크)에서** 돌리고 `request_id → oneshot`로 상관해 액터를 소켓 서비스에 자유로 둔다. ViewManager 단일 Mutex는 여전히 적용 순서를 직렬화하나(RMW 임계구역), **재진입 안전성**은 이 async 상관 설계가 준다. `LayoutState(Arc<Mutex<ViewManager>>)`(`layout/mod.rs:29`)·PRD read-your-writes는 그대로 유효.
- **stale-ref 에러 코드(초기 닫힌 집합) + 확실성 인코딩(G):** `STALE_REF`(주소지정 ID 사라짐/닫힘) · `APP_OFFLINE`·`APP_DISCONNECTED`·`TIMEOUT`(라우팅, PRD) · `LEASE_DENIED`(write lease 미보유) · `UNSUPPORTED`(capability 게이트 밖) · `ALREADY_APPLIED`(dedup 히트).
  - ★**확실성(certainty) 인코딩**★: `APP_OFFLINE` = **미적용 확정**(certain-not-applied — 대상 앱 없음). `APP_DISCONNECTED`/`TIMEOUT` = **불명**(unknown — 적용됐을 수도 → LLM이 read로 재조정). retryable 플래그는 **불명 상태의 안전 재실행을 함의해선 안 된다**(certain-not-applied만 안전 재시도).
  - **주소지정 selector 타이핑(R5 2-모드):** `byId`(안정 id) vs live-relative(공간/방향 등 실시간 상대) 선택자를 타입으로 구분 — 정확한 enum·메시지·retryable 플래그 = 구현.
- **mixed-version:** engram-ctl↔데몬 = 기존 `PROTOCOL_VERSION` handshake(Auth 불일치 거부)로 커버. relay payload(앱이 파싱하는 UI 명령 JSON) = 봉투 `v` 필드로 버전, 앱이 미지원 `v`면 정직한 에러. MVP 단일 v, 필드로 예약.
- **부분적용 원자성(다중 명령 의도) = 열림(hard).** 하나의 논리 의도가 여러 engram-ctl 호출로 쪼개졌을 때 중간 실패 원자성 — MVP는 각 명령 개별 정직 결과 + LLM이 read로 재조정(트랜잭션 미도입). 배치/트랜잭션은 후속.

## 결정 맵 (fork = 사용자 / 내부 = 메인 결정·보고 / ADR = 박제)

| 항목 | 종류 | 상태 |
|---|---|---|
| engram-ctl CLI 문법 | FORK-1 | ✔ noun-verb(agent/view/obs) |
| UI relay 앱 적용 경로 | FORK-2 / ADR | ✔ (A) 공유 `ViewCommand` 적용 서비스 + async 상관(§3) |
| "앱 = 데몬 명령 수신 WS peer" | ADR | ✔ ADR-0081 박제 |
| relay 봉투 wire 변형·라우팅 표 | 내부 | ✔ 설계(§3 — single request_id·opaque value·비블로킹·수명 바운드) |
| 앱 role 등록 방식 | 내부 | ✔ `RegisterRole{UiApp}` + 재연결 재전송·last-wins(§3) |
| 토큰 주입 + discoverability | 내부 | ✔ spawn env 오버레이(토큰=env var · ctl=PATH prepend) §5 |
| write(stdin) ⟂ lease | 내부 | ✔ per-call 자동 획득(결정 (a) 2026-07-14) §5 |
| dedup store | 내부 | ✔ 데몬-전역 맵 + `--request-id`(상관+dedup 통합) + 바운드 수명 + 적용후 커밋 §5 |
| events poll cursor | 내부 | ✔ monotonic 커서 + ring 저널 + RESET fallback §5 |
| 순서 | 내부 | ✔ 공유 `ViewCommand` 적용 서비스 + async 상관(옛 "Mutex만으로 충분·큐 불요" 폐기 — compound self-deadlock) §5 |
| stale-ref 에러 코드 | 내부 | ✔ 초기 닫힌 집합 + 확실성 인코딩 §5 |
| mixed-version | 내부 | ✔ PROTOCOL_VERSION + payload `v` §5 |
| 부분적용 원자성(다중명령 의도) | 내부/설계 | 열림(hard) — 후속(배치/트랜잭션) |

## 결정 완료 (2026-07-14)
- **FORK-1** = noun-verb 그룹형(`agent`/`view`/`obs` + `list-commands`).
- **FORK-2** = (A) 사람 Tauri 경로와 공유하는 transport-중립 `ViewCommand` 적용 서비스(단일 경로, §5) + relay는 액터 밖 적용·async 상관(/review trd 정밀화 — 순수 invoke-shim 재진입이 compound `spawn_into`에서 self-deadlock).
- **ADR-0081** 박제(UI relay 아키텍처: 앱=데몬 명령 수신 WS peer + opaque relay 봉투 + 공유 적용 서비스; 개정 2026-07-14 /review trd).
- **내부 미결 전부 소진**(§5): 토큰/discoverability(spawn env 오버레이) · write⟂lease(per-call 자동 획득) · dedup(데몬-전역 + `--request-id` 통합 + 바운드 수명 + 적용후 커밋) · events cursor(ring 저널 + RESET) · 순서(공유 적용 서비스 + async 상관) · stale-ref 코드(+확실성 인코딩) · mixed-version. 남은 열림 = 부분적용 원자성(hard, 후속).
- **다음(인터페이스 상세 표 — 구현 세션 이월):** engram-ctl 서브커맨드별 인자·결과 스키마 + 관찰 계약(tail start-cursor 결속, done/pending/terminal/reset 결과, next-cursor, poll 배치 shape) + relay wire 변형 정의는 **인터페이스 상세 표**로 구현 세션에 이월한다 — 그 뒤의 **설계 결정은 이 리뷰(/review trd)로 잠금**(표는 그 설계를 스키마로 옮기는 작업). 이어 모듈 경계(DDD) → 구현+TDD. step-log(흐름) 갱신은 이 TRD 커밋 시.
