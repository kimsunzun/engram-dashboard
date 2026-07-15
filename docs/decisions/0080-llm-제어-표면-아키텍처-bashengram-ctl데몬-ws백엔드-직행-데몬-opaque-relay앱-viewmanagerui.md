# ADR-0080: LLM 제어 표면 아키텍처 — Bash→engram-ctl→데몬 WS(백엔드 직행) + 데몬 opaque-relay→앱 ViewManager(UI)

- 상태: **폐기 (Superseded by ADR-0085)** — 제어 채널 ingress를 engram-ctl CLI(토큰·WS)에서 in-band 출력 마커(M3)로 피벗(보안·속도). 이 ADR은 제안(미확정)에 머문 채 headline 기제가 대체됨. **주의: 폐기 = engram-ctl ingress 한정** — 아래 정의된 UI opaque-relay·권위 2도메인은 ADR-0081(확정)로 존속한다. ~~제안 (2026-07-13, 근거: `/research` medium + Codex 적대리뷰 2회 + claude-code-guide grounding — `/review prd` 통과 시 확정)~~
- 관련: CLAUDE.md §5(LLM-우선 제어) · ADR-0014(데몬 CLI-via-Bash 방향) · ADR-0035(레이아웃 권위=src-tauri ViewManager) · ADR-0068(슬롯 공간 어휘) · ADR-0011(agentClient 제어표면) · PRD `docs/process/S17-llm-control-surface/spec/prd.md` · step-log S17 · 스파이크 `scripts/engram.mjs` · 데몬 WS auth `crates/engram-dashboard-daemon/src/ws.rs`

## 맥락
§5는 "모든 기능이 LLM 제어 가능"을 요구하는데, release 빌드에서 그게 붕괴한다. 제어 핸들(`window.__engramLayout`·`__engramCmd`)은 웹뷰 안에 살아 있으나, 외부 프로세스(스폰된 child claude)가 거기 닿는 유일한 브리지가 `scripts/cdp.mjs`(dev 전용 원격 디버깅 포트)라 release `tauri.conf.json`엔 그 포트가 없어 통째로 죽는다. child claude는 별도 프로세스라 `window.*`에 직접 못 닿는다. 즉 "정식 제어 표면 전까지의 임시 경로"(CDP)가 release에서 사라지는, §5가 예고한 갭이다.

동시에 권위가 두 프로세스로 쪼개져 있다: 백엔드(spawn/kill/list/preset) = 데몬(`AgentManager`, 이미 WS 서버 보유), UI/레이아웃(split/tab/focus) = 앱(`ViewManager`, ADR-0035 — 데몬은 View·slot 무지). 외부 LLM이 UI를 제어하려면 명령이 앱까지 도달해야 한다.

## 결정
**호출:** `child claude의 Bash 도구 → 컴파일 Rust `engram-ctl`(protocol crate 공유) → 데몬 WS(토큰 auth)`. 모델은 텍스트/tool-use로만 작용하며 여기선 Bash 도구가 그 구조화된 행동이다. `engram-ctl`이 WS 클라이언트(portfile 발견·인증·명령 어휘)이고, Bash는 그 프로그램을 실행만 한다(`curl`이 HTTP 하듯).

**권위 2도메인 + 라우팅:**
- 백엔드 = 데몬 직행(`engram-ctl → 데몬 WS → AgentManager`). PC·모바일 이식.
- UI = **데몬 opaque-relay**. 데몬은 UI payload를 **해석하지 않고** 대상 앱 연결로 전달 → 앱이 로컬 Tauri 명령과 **같은 ViewManager 경로**로 적용 → 결과 correlation 회수. 데몬은 UI 무지 유지(ADR-0035 보존), 역할만 "두 클라이언트(CLI↔앱) 사이 opaque 브로커"로 확장.

**명령 모델(2층):** 도메인 층(agent·preset — 전 기기 동일) + 표현 층(레이아웃 — 논리 view-tree 의미 연산, 좌표 금지·ADR-0068 공간 어휘 + 기기별 capability 게이트). `list_commands`가 현재 기기 가용 명령 반환.

**관찰·결과:** 셸은 스트림을 턴 간 못 잡으므로 유한 primitive(`wait --until`·`events poll --cursor`·`output tail --after-seq`) + 정직 JSON 봉투(`{v,ok,requestId,result}`/`{error{code,retryable}}`). UI 성공은 ViewManager 적용 후에만 확정.

**프로비저닝:** 데몬(부모)이 child spawn 시 제어 채널을 물려준다(ephemeral env 오버레이 — `profile.env` 금지, agents.json 평문 저장이라). MVP 인증 = **현행 단일 마스터 토큰**(전권) — child별 스코프는 비목표(아래).

**범위(MVP):** 노출 대상 = **동작이 이미 권위에 존재하는 커맨드만**. 신규 백엔드/`ViewManager` 연산을 만들지 않고 release-safe 명령 경로만 짓는다 — 백엔드 = `AgentManager` 전 wire 명령(spawn/kill/interrupt/write/resize/list + input lease + profile/preset CRUD + snapshot, `manager.rs`/`connection_core.rs`), UI = `ViewManager` 전 명령(tabs/windows/slots/content/popout + read 조회 get_view/list_tabs/list_windows/snapshot, `layout/manager.rs`/`commands/layout.rs`). §5 갭 중 권위에 안 붙은 것(테마 변경〔v1 명시 거절〕·렌더모드 override·레이아웃 저장/복원〔미구현〕·트리 네비게이션〔read-only〕)은 명시 보류(신규 배선 필요). **MVP 전제 = 단일 클라이언트(앱) 인스턴스:** 데몬에 연결된 UI 권위 앱은 하나로 가정한다 — 다중 앱 인스턴스 타깃팅은 MVP 밖(보류). 이 전제 덕에 UI 명령의 대상이 유일하게 확정되고, 아래 opaque-relay 라우팅에 앱 선택 로직이 필요 없다.

## 거부한 대안
- **MCP over HTTP** — 데몬이 스폰한 자기 로컬 child claude와 통신하는데 HTTP+JSON-RPC+핸드셰이크 ceremony는 과다(부모-자식이 같은 머신). 토큰 우려는 Claude Code tool-search 기본 defer로 약화됐으나, 로컬 단일 앱엔 MCP의 상호운용 가치가 낭비. **로컬 stdio MCP은 거부 아니라 보류** — 명령 카탈로그가 커지거나 "넓은 Bash 대신 좁은 툴 권한/타입 스키마"가 중요해지면 재검토(cross-family 리뷰 반론 기록).
- **chat-text 파싱**(claude가 명령을 대화 텍스트로 뱉고 watcher가 파싱) — 취약·환각. tool-use가 정식 구조화 채널이라 텍스트 파싱은 열등.
- **CDP-in-release**(release 빌드에도 원격 디버깅 포트 개방) — 아무 로컬 프로세스나 붙어 임의 JS/invoke 실행 가능한 보안 노출 + 원래 걷어내려던 임시 경로 연장. (Chrome 136 하드닝 근거는 Chrome 것이라 WebView2에 미검증.)
- **UI용 앱-소유 별도 엔드포인트**(앱이 자체 제어 서버를 여는 방식) — 2차 엔드포인트·discovery·auth 중복. 데몬 opaque-relay가 기존 데몬↔앱 WS를 재사용해 비용↓, 모바일에선 원격 데몬이 폰 앱에 닿는 유일 경로라 relay가 이식성도 우위.

**child별 권한 스코프(R7) 인가 배치 — 보류(비목표, 알면서 수용한 위험), 분석 보존.** 로컬 단일 PC = 단일 신뢰경계라 MVP는 세밀 인가를 아무도 하지 않고 현행 단일 마스터 토큰 전권을 허용한다 — 이때 prompt-injection된 child 하나가 공유 마스터 토큰으로 형제 agent kill·write·전 UI 조작(child↔child 횡이동)이 가능하다는 위험을 **알고도 수용**한다("single PC"는 외부 경계만 덮고 내부 오염 child는 못 덮음; 전제 = 로컬·신뢰 콘텐츠). 재도입 = 모바일/원격, 상세·사용자 재확인 = `docs/tracking.md` T-11(2026-07-14 노트). 검토한 배치안: **(lean, 향후 유력 해)** 데몬은 per-child 토큰으로 **신원 도장**만(UI 의미 무지 유지 = ADR-0035 보존), 인가(스코프)는 UI 권위 소유자 `ViewManager`가 — opaque-relay와 무모순. **(a, 거부 후보)** 데몬 coarse 게이트 병행 = 인가가 데몬/클라 양쪽에 분산. **(b, 거부 후보)** 클라 직결 엔드포인트 = 모바일 부적합(폰 앱은 원격 데몬 경유가 유일 경로). MVP 라우팅을 relay-through-daemon(child→데몬→`ViewManager`)으로 유지하면 lean/(a)가 열린 채 남고 foreclose되는 건 (b)뿐이라 무해 — ADR-0080이 이미 그 모양이다.

## 근거
- OSS 서베이(`/research` medium): CLI→데몬-소켓(tmux ISC/zellij MIT)이 성숙 패턴이고, MCP의 실제 값어치는 "모르는 것들끼리 interop"이라 단일 인하우스 앱엔 과함.
- Codex 적대리뷰(repo 실측): 데몬 WS는 이미 토큰 auth 보유(`ws.rs`), `engram.mjs`가 release-safe 경로를 증명, 단 `AgentCommand`엔 layout 명령 없음·`ViewManager`는 src-tauri라 UI는 별도 경로 필요 — opaque-relay로 해소. Node 스파이크보다 컴파일 Rust CLI가 릴리즈 산출물로 우위.
- claude-code-guide grounding: 모델은 텍스트/tool-use로만 작용, 스톡 claude 툴 = built-in(Bash)+MCP. 즉 "MCP 없는 네이티브 도구"는 없고, 비-MCP 구조화 행동 = Bash 도구.

## 영향 / 불변식
- **데몬 UI 무지 유지(ADR-0035):** relay는 UI payload를 파싱하지 않는 opaque 통로여야 한다. `AgentCommand`에 `LayoutNode` 명령을 넣으면 이 경계가 깨진다 — UI 봉투는 데몬이 모르는 별도 opaque 타입. **R7 보류로 이 경계가 깨끗해진다:** MVP가 세밀 인가를 아무도 하지 않으므로 "데몬 opaque-relay(UI 의미 무지) ↔ child별 스코프" 모순이 소멸(= `/review prd` BLOCK을 보류로 해소, T-11).
- **권위 소유자에서 실행:** 백엔드=데몬, UI=앱 ViewManager. 사람 클릭(invoke)이든 LLM(engram-ctl→WS→relay)이든 마지막엔 같은 핸들로 수렴(§5 단일 control surface).
- **좌표 비노출:** 표현 명령은 논리 트리 의미 연산만. 픽셀·창번호를 명령 어휘에 넣으면 기기 이식성이 깨진다(ADR-0068).
- **release 게이트:** CDP/devtools 비의존. `engram-ctl`은 데몬 WS(빌드 무관 생존)에만 의존.
- **비목표(보류, 알면서 수용한 위험):** child별 권한 스코프(R7) = `docs/tracking.md` T-11(모바일/원격 단계 재도입, 2026-07-14 사용자 재확인). MVP = 현행 단일 마스터 토큰 전권 — injection된 child의 child↔child 횡이동 위험을 수용하고 보류 유지(전제 = 로컬·신뢰 콘텐츠).
- **동시성 메커니즘 = TRD-level, ViewManager 단일 권위 위에서 해소:** requestId dedup(at-most-once)·result correlation·timeout 재조정 등은 아키텍처 결정이 아니라 TRD 수준 문제이고, `ViewManager` 단일 권위의 순차 적용(ADR-0035) 위에서 풀린다 — 단일 권위가 순서를 확정하므로 분산 합의가 필요 없다.
- **미확정(TRD):** portfile ACL, event journal, 명령 카탈로그 노출 어휘·발견(대상 연산은 위 "범위(MVP)"로 확정), 브로커 라우팅(앱 identity/lease·다중창·offline), stale-ref 의미론(닫힌/kill된 view·slot 주소지정), R6 robustness(멱등성·부분적용 원자성·순서/동시성·mixed-version 협상). 보안 판단은 담당 부서.
