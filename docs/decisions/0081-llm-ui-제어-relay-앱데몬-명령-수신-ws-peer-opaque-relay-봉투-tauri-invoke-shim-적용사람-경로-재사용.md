# ADR-0081: LLM UI 제어 relay: 앱=데몬 명령 수신 WS peer + opaque relay 봉투 + Tauri invoke-shim 적용(사람 경로 재사용)

- 상태: 확정 (2026-07-14, 근거: 실측 지형 + 사용자 fork 결정 2건 + `/review prd` 통과)
- 관련: PRD·TRD `docs/process/S17-llm-control-surface/spec/` · ADR-0080(제어표면 아키텍처·상위) · ADR-0014(CLI-via-Bash) · ADR-0035(ViewManager=UI 권위) · CLAUDE.md §5(LLM-우선 제어) · 구현 앵커(예정): `crates/engram-dashboard-protocol/src/messages.rs`·`crates/engram-dashboard-daemon/src/connection_core.rs`·`src-tauri/src/daemon_client/connection.rs` · step-log S17

## 맥락
release에서 LLM(child claude)이 UI(레이아웃·탭·슬롯)를 제어해야 한다(§5·PRD S17·R3). ADR-0080이 방향을 "engram-ctl → 데몬 WS, UI는 **데몬 opaque-relay**로 앱에 전달"로 정했다. 그러나 실측(2026-07-14, 서브에이전트 지형 조사)에서 데몬엔 **cross-client 라우팅이 없다** — 모든 WS 연결이 대칭 peer이고 broadcast(status/list)만 있으며, "데몬→특정 연결 전달"·"앱의 inbound 명령 수신→ViewManager 적용" 경로가 전무하다. request_id correlation은 이미 있으나(전 side-effect `AgentCommand`), UI relay 자체는 net-new다. 이 ADR은 그 relay의 구체 프로토콜 + 앱 적용 경로를 확정한다.

## 결정
1. **앱 role 등록** — 앱(클라이언트 src-tauri)은 auth 직후 `RegisterRole{UiApp}`를 보내 자신을 "UI 권위 연결"로 등록한다. 데몬은 그 ConnId를 UI-앱 연결로 표시한다(단일 앱 인스턴스 전제 = PRD 범위 → 대상 주소지정 불요).
2. **opaque relay 봉투(single request_id)** — `CLI→데몬 RelayUi{request_id, payload(opaque JSON value)}` → `데몬→앱 UiCommand{request_id, payload}` → `앱→데몬 UiResult{request_id, result}` → `데몬→CLI` 같은 request_id로 결과. ★CLI의 `request_id`를 **end-to-end로 그대로 흘린다**(별도 `relay_id` 네임스페이스 없음).★ 데몬은 **payload를 파싱하지 않고** `request_id→원 ConnId` 라우팅 표만 유지(opaque bridging — ADR-0080 보존). (변형 필드명은 예시 — TRD/구현서 확정.)
3. **앱 적용 = 공유 `ViewCommand` 적용 서비스 + async 상관** — 앱은 relay 받은 UI 명령을 **사람 Tauri 경로와 relay가 공유하는 transport-중립 `ViewCommand` 적용 서비스**로 ViewManager에 적용한다(단일 경로, §5 유지). ★relay는 `daemon_client` 액터 **밖**에서 적용하고 `request_id`로 상관한다★(액터는 소켓 서비스 계속). (사용자 fork 결정 2026-07-14.) ★**개정 2026-07-14 (/review trd)**: 순수 invoke-shim 재진입이 compound 명령(`spawn_into` — 자기 안에서 `client.send_command().await`, reply는 같은 액터 stream 루프에서만 해소)에서 **self-deadlock** → 공유 적용 서비스 + async 상관으로 정밀화.★
4. **request_id correlation 재사용** — 기존 correlation(`PendingMap`→oneshot) 위에 얹는다. relay는 왕복(CLI→데몬→앱→데몬→CLI) 동안 **같은 request_id**를 브리지한다(relay_id 없음).

## 거부한 대안
- **broadcast + 앱 필터** — 데몬이 UI 명령을 전 연결에 broadcast하고 앱이 자기 것만 필터. 시끄럽고(다른 연결·창이 다 봄) 대상 지정이 아니라 필터라 다중 창·비-앱 연결로 명령이 샌다.
- **앱-소유 별도 엔드포인트** — 앱이 자체 WS/IPC 엔드포인트를 열어 CLI가 직결. 2차 엔드포인트·discovery·auth 중복 + 모바일(원격 데몬)에서 앱 직결 불가 → ADR-0080에서 이미 거부, 여기서 재확인.
- **ViewManager 직접 호출**(daemon_client 태스크가 공유 적용 경로를 건너뛰고 ViewManager 직접 호출) — 빠르나 사람 경로와 LLM 경로가 갈려 **2 코드 경로를 동기화**해야 하고 lock-ordering 리스크. §5 "사람 클릭도 같은 핸들"(단일 control surface) 위반. fork에서 (A) 공유 적용 경로 채택으로 거부. (반대 극인 "Tauri command 엔트리를 액터 위에서 그대로 재-invoke"도 compound 명령 self-deadlock으로 기각 — 결정 #3 개정 참조. 채택안은 그 중간: **공유 `ViewCommand` 적용 서비스** + relay는 액터 밖 async 상관.)
- **CDP/webview 핸들**(현 임시 경로 `cdp.mjs eval`·`window.__*`) — release exe에서 원격 디버깅 포트·웹뷰 핸들이 죽음(PRD 배경의 붕괴 지점). 이 프로젝트의 애초 동기라 재도입 불가.

## 근거
- **실측(2026-07-14):** request_id correlation·백엔드 명령·portfile+WS는 이미 존재(재사용) / cross-client 라우팅·앱 inbound 핸들러·relay 봉투는 net-new (`messages.rs`·`connection_core.rs`·`ws.rs`·`daemon_client/connection.rs`).
- **§5 정합:** 공유 `ViewCommand` 적용 서비스는 사람 UI 클릭과 정확히 같은 ViewManager 적용 경로를 태워 "사람 클릭 = LLM 제어의 같은 핸들"을 구조로 만든다(단일 control surface, 유지보수 1곳). relay는 그 서비스를 액터 밖에서 부를 뿐이라 재진입 없이 같은 경로를 공유한다.
- **ADR-0080 보존:** 데몬이 payload를 안 읽어 opaque-relay 불변식 유지. R7(child 스코프) 보류로 세밀 인가 없음 → opaque가 깨끗.
- **PRD 통과:** `/review prd` full 3라운드 통과(2026-07-14) — 요구·범위·계약 확정 위에 이 relay를 얹는다.

## 영향 / 불변식
- **앱 = 데몬 명령 수신 WS peer(신규 능력).** 지금까지 앱은 event 소비 + outbound 명령만 했다 — 이제 `daemon_client` 연결에 **inbound UI-command 핸들러**가 생긴다. 이 인바운드 경로는 반드시 **사람 경로와 공유하는 `ViewCommand` 적용 서비스**를 거쳐 ViewManager로 간다 — ViewManager 직접 호출 금지(§5·2경로 방지).
- **★relay 적용은 액터 밖(비블로킹).★** relay 인바운드 핸들러는 적용을 `daemon_client` 액터 태스크 **밖(spawn 태스크)에서** 돌리고 `request_id→oneshot`로 상관한다 — 액터에서 인라인으로 compound Tauri command를 await하면 그 command 자신이 부르는 `client.send_command().await`가 같은 액터에서만 해소돼 **self-deadlock**(결정 #3 개정). 데몬 `RelayUi` dispatch도 앱 왕복을 await하지 않고 즉시 반환한다(연결당 순차 dispatch head-of-line 블록 방지 — TRD §3).
- **데몬 opaque 유지(ADR-0080):** 데몬은 UI payload를 파싱하지 않는다(opaque JSON value). relay 표는 `request_id→원 ConnId`만. UI 의미를 데몬에 넣으면 ADR-0080·0035 위반. UI 명령 enum + payload→`ViewCommand` dispatch 맵은 오직 `src-tauri` — `engram-ctl`·데몬 relay·`core`는 UI enum import 금지(core tauri-free, ADR-0003).
- **★relay 라우팅 표 수명 바운드 + 연결 cleanup sweep.★** 표 엔트리 = `{request_id→원 ConnId, deadline}`. evict = (1) `UiResult` 수신 (2) 어느 쪽 엔드포인트든 연결 cleanup(그 ConnId로 키된 엔트리 sweep — 원-CLI·앱 양쪽) (3) timeout. dedup store도 **바운드**(TTL/LRU — 데몬 lifetime 무한 아님). 모르는/중복 request_id `UiResult` → drop.
- **★dedup는 UiResult(적용 후)에만 커밋.★** relay UI 명령의 dedup 엔트리는 **성공 `UiResult` 후에만** 커밋한다(사전 캐싱 금지 — timeout 시 거짓 성공 캐시 = PRD "UI 성공은 ViewManager 적용 후 확정" 위반). in-flight 중복은 같은 pending에 coalesce.
- **★RegisterRole 재연결 재전송 + last-wins.★** 앱은 첫 auth뿐 아니라 **매 재연결마다** `RegisterRole{UiApp}`를 재전송한다(재연결 핸드셰이크에 태움) — 안 그러면 재연결 후 데몬에 UI-앱 대상이 없어 UI relay가 잘못 `APP_OFFLINE`로 실패. 중복 UiApp 등록 = **last-registration-wins**(새 ConnId가 stale 대체).
- **request_id 왕복 보존:** CLI→데몬→앱→데몬→CLI 전 구간 **같은 request_id** 유지. 앱 role 미등록 시 `RelayUi`는 정직한 에러(대상 없음 = `APP_OFFLINE`, 미적용 확정)로 실패.
- **단일 앱 인스턴스 전제:** 다중 앱이면 대상 모호(PRD 범위 밖·후속). role 등록이 1개 초과일 때 정책은 TRD 미정.
- 구현 시 코드 앵커 `// ADR-0081` = relay 봉투 dispatch(`connection_core`)·앱 inbound 핸들러(`daemon_client`)·공유 `ViewCommand` 적용 서비스(`src-tauri`)·`RegisterRole`.
