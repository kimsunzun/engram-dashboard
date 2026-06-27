# ADR-0036: 전송 중계 통일 — src-tauri 단일 데몬 클라이언트 + 출력 라우터 (창=Tauri IPC)

- 상태: 확정 (2026-06-27, 근거: `/research` deep 보고서 + 사용자 결정)
- 관련: ADR-0035(레이아웃 권위=src-tauri — 이 결정의 권위 짝)·ADR-0020(단일 프로토콜 + transport-중립 dispatch core·swappable carrier — 본 ADR이 carrier 토폴로지를 개정)·ADR-0029(daemon-only·앱=데몬 클라이언트)·ADR-0011(agentClient 제어표면)·ADR-0028(이벤트버스 단일 push) · `docs/research/multi-window-layout-authority-topology-research-2026-06-27.md` · step-log S14

## 맥락
ADR-0035로 레이아웃 권위가 src-tauri로 오면서 토폴로지 불일치가 드러났다. 현행은 **각 창(WebView)이 데몬에 WS 직접 연결**(`wsTransport.ts`가 브라우저 WebSocket으로 데몬 attach; src-tauri는 데몬 위치만 알려줄 뿐 데이터 경로 밖). 그런데 레이아웃은 src-tauri 경유, 에이전트 출력만 창↔데몬 직결이면 ① 토폴로지 비일관 ② 라우팅 중복(src-tauri가 이미 "어느 창이 어느 에이전트를 보는지" 레이아웃 테이블을 쥐는데 각 창이 재유도) ③ 같은 에이전트를 N창이 보면 데몬 구독 N개·출력 N중복(원격 데몬이면 네트워크로 N배).

## 결정
**src-tauri가 데몬과 단일 WS 연결을 쥐고, 자기가 소유한 레이아웃 라우팅 테이블(ADR-0035)로 각 창에 출력을 fan-out한다.** 창은 src-tauri하고만 IPC(레이아웃 + 에이전트 둘 다) — 데몬 직결 0. 데몬엔 클라이언트 1개로 보이고, 에이전트당 데몬 구독은 1회(중복 제거, 로컬 fan-out). 입력(write/spawn/kill/resize)도 창→invoke→src-tauri→데몬. 프론트 carrier는 `WsTransport(데몬 직결)` → **`TauriTransport`(src-tauri 경유)**로 교체(ADR-0011/0020 transport seam 활용 — `agentClient`=`ProtocolClient` 인터페이스는 불변). 데몬과의 단일 WS 연결은 src-tauri가 쥔다. **단, 프로토콜 의미론(재연결·epoch·seq dedup·resubscribe)을 어디까지 Rust로 옮길지**(전면 Rust 이전 vs 얇은 프레임 라우터 + JS 의미론 유지)는 **Phase B spike에서 확정** — 본 ADR은 *토폴로지*(단일 연결·src-tauri 중계·창=IPC)만 못 박고, 그 구현 메커니즘은 미정으로 남긴다.

## 거부한 대안
- **창마다 데몬 WS 직결(현행)** — N창 = N 인증/재연결/discover 중복 + 출력 N중복 전송(원격이면 네트워크 N배) + src-tauri가 이미 가진 라우팅 테이블을 각 창이 재유도. 유일 장점(고대역 출력 직통·창별 실패 격리)은 (a) 로컬 IPC라 relay 한 홉 비용 미미 (b) 원격에선 오히려 직결이 대역폭 낭비 (c) 정말 병목이면 per-pane 직통 서브채널을 후속으로 추가 가능(Codex 권고: "중앙 유지, 프로파일링 병목 시 옵션 추가"). 비용 대비 이득 없음.

## 근거
`/research` deep — 단일 권위 + 단일 연결 멀티플렉싱이 관행: **LSP**(에디터↔서버 단일 연결로 다중 문서 multiplex, 문서마다 연결 안 만듦), **Chromium**(browser process가 단일 권위 + renderer들에 Mojo fan-out), **Wayland**(compositor 단일 소켓). src-tauri가 레이아웃(ADR-0035)을 소유 = 라우팅 테이블 보유 → 출력 라우팅이 자연 합성(데몬은 "누가 보는지" 모르니 라우팅 못 함). 원격 데몬(ADR-0029) 대역폭에서 중계가 명백히 유리(출력 1회 전송 → 로컬 fan-out).

## 영향 / 불변식
- **단일 choke point:** 모든 트래픽(레이아웃·에이전트 I/O)이 src-tauri를 지난다. 창↔데몬 직결 금지.
- **데몬 불변:** 데몬은 클라 수를 모르고 1 연결만 본다. 데몬 코어/프로토콜 변경 없음(에이전트만).
- **carrier 교체(ADR-0020 개정):** 프론트 daemon carrier = `WsTransport` 직결 → `TauriTransport`(src-tauri IPC). `ProtocolClient`(ADR-0011) 인터페이스·단일 프로토콜은 불변 — transport seam이 정확히 이걸 위해 존재(ADR-0020). 데몬 단일 연결은 src-tauri 소유. **프로토콜 의미론의 Rust 이전 범위는 Phase B spike에서 확정**(토폴로지만 확정, 메커니즘 미정).
- **출력 라우팅 정책:** src-tauri `OutputRouter`가 `ViewManager` 조회로 "agentId→그 에이전트를 띄운 창"에만 전달. 안 띄운 창엔 안 보냄.
- **메시지 정책(락/게이팅)은 데몬:** 전송 중계는 정책을 enforce하지 않는다 — 에이전트 메시지 락 등은 데몬이 단일 choke point로 enforce(src-tauri는 캐시 기반 조기거부 *최적화*만 가능, 권위는 데몬).
- **단계 분리:** 이건 전송층 재설계라 S14 레이아웃 기능과 별도 구현 단계(TRD가 phasing 규정). ADR-0035(권위)와 짝이지만 구현 순서는 TRD.
