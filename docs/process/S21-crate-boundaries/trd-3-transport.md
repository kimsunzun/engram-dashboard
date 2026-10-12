# TRD — 경계 리팩터링 3: transport 를 셸 · 데몬에 붙이고 net 을 걷는다 (S21)

> 상태: **1판(2026-10-12) — 초안 · 리뷰 전.** 코드는 한 줄도 바뀌지 않았다. ★§8 「사용자 결정 목록」이 전부 열려 있다 — 결정 전 구현 진입 금지(CLAUDE.md 「개발 스텝」)★.
> **결정 출처(이 판까지):** 순서 3-1 → 3-2 → 3-3 은 메모 §10 「3. transport」 그대로다. 사용자 지시(2026-10-12) — ① 클라(셸) 하나가 데몬 **여럿**에 붙고 데몬끼리 **서로** 말하는 구조를 지금 3단계에서 세운다 · 원격(다른 PC) 적용 · 시험은 나중 트랙 · 3단계가 내보내는 동작은 셸 하나 + 로컬 데몬 하나 · 다중 데몬 · 데몬끼리는 transport 인메모리 하네스와 한 PC 여러 데몬으로 잰다 ② 보안 층 나누기(조사 `docs/research/transport-security-build-vs-adopt-2026-10-12.md` §0 · §1 · §4 · §5) — transport 는 유지 · 암호화 · 상대 인증은 나중에 Link 층에서 빌린다 · 인가 · 페어링 · Origin · 자원 상한은 우리 몫 · net 의 방어는 옮기고 실측된 구멍은 막거나 명시로 미룬다.
> **범위 = 작업 순서 3 전부** — 3-1 `protocol` 정리 · 3-2 셸 부착 · 3-3 데몬 부착 + net 걷기. PRD 급 출처 = `docs/refactoring/architecture-discussion-2026-09-26.md` §6 결정 후보 7(`:222-229`) · §7 결정 후보 8(`:241-253`) · §10 3.(`:297`).
> **배치 근거:** `docs/README.md` 「새 내용을 어디에 넣나」의 「새 기능 설계 착수 → `process/SN-name/`」. 같은 경계 리팩터링의 형제 TRD(1-1 · 1-3 · 2-1 · 2-2 · 2-3 · A)가 이 폴더에 있다.
> **표기:** 「실측」 = 기준 커밋 `255b2830`(브랜치 `v0.3.3/refactor/transport-attach` HEAD — master 머지 `ba39a080` 와 코드가 같다, 차이는 `docs/research/` 둘)의 **정적 판독**이다. 빌드 · 시험은 돌리지 않았다 · 「제안」 = 이 TRD 의 안(새 이름은 전부 가안) · 「미확인」 = 확인하지 않은 것.
> 앵커: ADR-0177(transport 헌장) · ADR-0178(버전 거절) · ADR-0179(데몬 N 클라 · 클라 N 데몬) · ADR-0180(알림 · 재연결 층) · ADR-0181(요청 시한) · ADR-0182(사건 방출) · ADR-0272(패킷은 남의 구조체를 품지 않는다) · ADR-0129 · ADR-0130(net) · ADR-0135(단일 인스턴스 파일) · ADR-0036(셸 = 데몬 클라 하나) · ADR-0021(재연결은 띄우지 않는다) · ADR-0195(끊긴 동안 명령 즉시 실패) · ADR-0046 · ADR-0231(replay · seq 연속) · ADR-0271 결정 5(정지 클라 임시 거처) · ADR-0291 결정 15(단일 인스턴스 열기 재시도) · ADR-0012 · TRD `docs/process/S21-transport-crate/trd.md`(이하 「transport TRD」).

---

## 0. 결론 (먼저)

```
① 끝 상태: net 삭제. 연결 기계 = transport · 어휘(인증 프레임 포함) = protocol(잎 — 워크스페이스 의존 0) ·
   데몬 전용(판정 · 연결별 줄 · 단일 인스턴스 · daemon.json) = 데몬 link/ · 셸 끝자락 = 셸 daemon_client/link/.
② transport 에 새로 드는 것 셋: 받는 쪽 seam(Listener + Registry::attach — 받은 상대는 다시 걸지 않는다) ·
   ws 수락 강화(수락과 업그레이드 분리 · 업그레이드 시한 · Origin/Host 훅 · 크기 상한) · 끊김 오류에 「보냈나」 칸.
③ 스트림 살림은 지금 자리 그대로 — 두 Wire 의 stream_mark = None. replay→live · gen 펜스 · 화신 표식 · seq 연속은
   데몬 · 셸 replay_flight · 프론트가 계속 진다. transport 의 스트림 층을 켜면 그 불변식이 깨진다(§2-5).
④ 다중 데몬 · 데몬끼리는 「관계마다 Wire · Registry · Listener 따로」 + 영속 데몬 신원 + 인가 seam 으로 자리만
   세운다. 셸 어댑터는 상대 N 을 담고 윗층은 primary 하나를 본다. 원격 · 페어링 · TLS 는 빈 공간(§4).
⑤ 단위 3-1(5) → 3-2(6) → 3-3(7). 어느 커밋에서 멈춰도 빌드 · 회귀 · 게이트가 초록이다(§6 규칙).
```

**지금 하지 않는 것:** 원격 Link(TLS · iroh · VPN) · 페어링 · 중계(relay) · 화면 공유(조사 `screen-sharing-oss-2026-10-12.md` 결론 1 — transport 의 일이 아니다) · 셸 화면 · LLM 의 다중 데몬 표면 · 클라별 되감기 · 다중 클라 입력 권한(ADR-0179 영향 2 · 3) · 제어 평면 HTTP(`/control/*` · `/mcp`)의 강화.

---

## 1. 맥락 · 범위

### 1-1. 왜

- transport 는 독립 crate 로 서서 단독 검증까지 끝났고 소비자가 0 이다(transport TRD §10 「셸을 이 crate 위로 이사 = 붙이는 순서 ②」). 셸은 손으로 짠 WS 클라이언트(`src-tauri/src/daemon_client/connection.rs` 2,050줄 + 동기 정지 클라 `stop.rs` 370줄)를, 데몬은 net 의 서버 행(`crates/engram-dashboard-net/src/ws.rs` 2,248줄)을 쓴다. 같은 일(핸드셰이크 · keepalive · 재연결 · 짝 맞추기)이 세 벌이다.
- net 은 셋을 섞는다 — 데몬 전용 서버 행 · 클라도 쓰는 인증 프레임(`net/src/auth.rs:33`) · 데몬 살림(`instance.rs` · `portfile.rs`). 셸이 데몬 쪽 crate 를 아는 것을 기능 플래그(`net/Cargo.toml` `server`)로 겨우 막는다(메모 §6 사실).
- `protocol` 은 command 의 구조체 넷을 품는다(`protocol/src/messages.rs:4`) — ADR-0272 가 걷기로 했고 착수를 이 단계로 미뤘다(결정 7).

### 1-2. 범위 안

| 단계 | 내용 |
|---|---|
| 3-1 | ADR-0272 실행(명령 칸 넷 = `Box<RawValue>` · `OwnerToken` 처분 · 짝 맞추기 함수를 끝자락으로) · `AuthFrame` 을 protocol 로 · protocol 의존 0 게이트 · 셸의 net 의존 삭제 |
| 3-2 | 셸 끝자락 어댑터(Wire · 핸드셰이크 · 풀이 Dialer · 사건 사상) · 접속 정보 인터페이스 · 정지 명령 클라이언트를 transport 위로 · 셸 어댑터의 상대 N 자리 |
| 3-3 | transport 받는 쪽 seam · 데몬 끝자락 어댑터 · net 의 방어 옮기기와 새 방어 · `instance` · `portfile` 를 데몬으로(열기 재시도 = platform `retry_busy`) · net 삭제 · 데몬 신원 · 데몬끼리 뼈대 |

### 1-3. 범위 밖 — 나중 트랙(§4 「빈 공간」이 채우는 자리)

원격 Link(rustls 키 고정 · iroh · 사용자 VPN) · 페어링 · 원격 주소 발견 · 데몬끼리 어휘(우편 중계 · 목록 합치기) · 되돌아옴 방지 · 셸 화면/LLM 의 데몬 고르기 · 인증 실패 속도 제한 · `daemon.json` ACL · 화면 공유.

---

## 2. 현재 → 목표 구조

### 2-1. 의존 그림

```
지금(255b2830)
  셸 src-tauri ─normal→ net(기본 feature = auth 만) · protocol · command · base · platform · tungstenite(stop.rs)
     daemon_client/connection.rs = 핸드셰이크 · 재연결 · pending · 읽기 루프 · keepalive 없음(:1108 「자동 응답뿐」)
  데몬 ─normal→ net(server: ws · frame_port · instance · portfile) → platform · protocol
     run_accept_loop(daemon/src/lib.rs:337) → net::ws::handle_connection(:390) → frame_port 계약 → agent_conn
  protocol ─→ command(봉투 · 답장 · 등록 항목 · 주인 표지 — messages.rs:4 · ids.rs:36)
  transport = 소비자 0

목표(3-3 뒤)
  셸 ─normal→ transport[ws] · protocol · command · base · platform         (net 0 · tungstenite 직접 의존 0)
     daemon_client/link/ = ShellWire · ShellHandshake · LocalDaemonDialer · EndpointSource · 사건 사상 · 정지
  데몬 ─normal→ transport[ws] · protocol · command · agent · messaging · base · platform   (net 0)
     daemon/src/link/  = DaemonWire · DaemonHandshake · auth(판정 seam) · server(수락 · 연결 포트 구현) · peer(데몬끼리 뼈대)
     daemon/src/conn_port.rs = 옛 net::frame_port 계약(위층 agent_conn · connection_core · command_* 무변경)
     daemon/src/instance.rs · portfile.rs = 옛 net 의 데몬 살림
  protocol = 잎 — handshake::AuthFrame · OwnerToken 사본 · 명령 칸 넷 = Box<RawValue> · (3-3) peer 어휘 뼈대
  net = 삭제
```

### 2-2. 이사 표 (파일 · 모듈 단위)

| 지금 | 무엇 | 갈 곳 | 단계 |
|---|---|---|---|
| `net/src/auth.rs:33` | `AuthFrame` | `protocol/src/handshake.rs`(가안) — 3-1 동안 net 은 재수출만 | 3-1 |
| `protocol/src/messages.rs:1010` · `:1074` | 짝 맞추기 두 함수 | 셸 `daemon_client/protocol_state.rs`(둘 — 지금 재수출 자리 `:28`) · 데몬 `agent_conn.rs`(받는 쪽 하나 — 지금 `:433`) → 3-2 · 3-3 에서 각 Wire 의 `request_tag` · `reply_tag` 로 | 3-1 |
| `protocol/src/ids.rs:36` | `From<command::RequestId>` | 셸 · 데몬 끝자락 | 3-1 |
| command `OwnerToken`(`command/src/envelope.rs:38` — protocol 이 품음) | 주인 표지 | protocol 사본(D17) | 3-1 |
| 셸 `connection.rs`(핸드셰이크 `:418-:520` · 재연결 `:784-:850` · pending · 쓰기) | 연결 기계 | transport `Registry`/`Peer` + 셸 `link/` | 3-2 |
| 셸 `daemon_client/mod.rs:105` `DaemonDiscovery` | 발견 seam | `link/endpoint.rs` `EndpointSource` 로 넓힘(§3-6) | 3-2 |
| 셸 `daemon_client/stop.rs` | 동기 정지 클라(ADR-0271 결정 5 임시) | `link/stop.rs` — transport 위 | 3-2 |
| `net/src/frame_port.rs` | 연결 포트 계약(`ConnId` · `FrameSink` · `FrameFanout` · `ConnectionHandler` · 공장) | 데몬 `conn_port.rs` | 3-3 |
| `net/src/ws.rs` | 수락 뒤 서버 행 | 연결 기계 = transport · Origin/업그레이드 = transport `ws` 수락 설정 · 판정 = 데몬 `link/auth.rs` · 연결별 줄 · dispatch · bypass · 팬아웃 = 데몬 `link/server.rs` | 3-3 |
| `net/src/instance.rs` · `portfile.rs` | 단일 인스턴스 가드 · daemon.json | 데몬 `instance.rs` · `portfile.rs`(열기 재시도 = platform `fs::retry_busy`) | 3-3 |
| 데몬 `lib.rs:337` · `:540` | 수락 루프 · `127.0.0.1:0` 바인드 | 데몬 `link/server.rs`(transport `Listener` 위) | 3-3 |
| transport `ws.rs:192` `WsListener` | 받는 쪽 구체 타입 | `Listener` 구현 + 수락 강화 | 3-3 |

### 2-3. 지우는 것

net crate 전체(6 파일 3,601줄 · 워크스페이스 멤버 · `Cargo.lock` 항목) · 셸 `tungstenite = "0.26"`(`src-tauri/Cargo.toml:98`) · 셸 `tokio-tungstenite` 운영 의존(→ dev — 시험의 가짜 서버가 쓴다) · 셸 `connection.rs` 의 핸드셰이크 · 재연결 · pending 부분 · protocol → command 간선 · net 게이트 1~5 와 그 사본(CLAUDE.md 「네트워크 행 격리 게이트」 · `.claude/skill-bindings/qa.md:280-285` · `docs/testing-strategy.md` §net).

### 2-4. 게이트 — 걷는 것과 대신 세우는 것

| 걷는 것(`ci.yml`) | 대신 | 단계 |
|---|---|---|
| net gate 1 — net 이 daemon · messaging 을 안 부른다(`:1140`) | ★끝자락 어휘 게이트(제안)★ `test -d crates/engram-dashboard-daemon/src/link && rg "engram_dashboard_(agent|messaging)" crates/engram-dashboard-daemon/src/link/` → 0줄 — 「네트워크 행은 에이전트 어휘를 모른다」(ADR-0129)를 데몬 안 모듈로 잇는다. 셸 쪽 짝 `src-tauri/src/daemon_client/link/` 에 `tauri::` · `engram_dashboard_agent` 0 | 3-3 · 3-2 |
| net gate 2a · 2b(`:1156` · `:1192`) | 없음 — platform 심볼은 `instance` · `portfile` 와 함께 데몬으로 간다(데몬은 원래 platform 을 진다) | 3-3 U2 |
| net gate 3 — 의존 상한 3(`:1210`) | transport gate 1(이미) + ★protocol 의존 상한 = 정확히 1줄(자기 자신)★ — ADR-0272 영향의 「의존 0 게이트」 | 3-1 U5 |
| net gate 4 — auth 어휘 재유입 금지(`:1231`) | protocol 의 `AuthFrame` golden(바이트 고정 시험 — `net/src/auth.rs` 의 `GOLDEN` 을 옮긴다) | 3-1 U2 |
| net gate 5a · 5b(`:361` · `:365`) | 없음(transport 4a~4d 가 있다) | 3-3 U6 |
| platform gate 7 의 net 두 줄(`:1281-1282`) | net 줄 삭제 · 「1줄 이상」 짝을 `-p engram-dashboard-transport`(tokio 가 무조건 의존)로 | 3-3 U6 |
| messaging 이름 게이트의 `net`(`:538`) | 이름에서 뺀다(사본 다섯 — 메모 §11 의 같은 처리) | 3-3 U6 |
| platform 4f · 4h 명단의 `net/src/instance.rs` · `portfile.rs`(`:924-925` · `:1023`) | 새 경로로 | 3-3 U2 |
| — (신설) | 셸 운영 그래프에 net 0줄 + 짝(데몬 ≥ 1줄 — 2-1 D6 꼴) | 3-1 U2 → 3-3 U6 에서 걷음 |
| — (신설) | 셸 직접 normal 의존에 `tungstenite` · `tokio-tungstenite` 0줄(`cargo tree -p engram-dashboard -e normal --depth 1`) | 3-2 U5 |

### 2-5. 스트림 살림 — 누가 무엇을 지나 (replay→live 화해)

| 일 | 지금 | 3단계 뒤(제안 — D10 ①) |
|---|---|---|
| 링 · seq 찍기 | 데몬 `OutputCore` | 그대로 |
| replay→live 순서 | subscribers lock 안에서 replay 를 연결당 FIFO 큐(`net/src/ws.rs:52` 4608칸)에 `try_send` | 같은 lock 안 `FrameSink::try_send` → `Peer::notify`(await 없음 — `transport/src/peer.rs:248`) → 감독 단일 writer FIFO. 같은 성질 |
| 화신 표식(epoch) 거름 | 셸 `frame_relay.rs` | 그대로 — transport `StreamMark.generation` 은 쓰지 않는다 |
| single-flight Subscribe · gen 펜스 마커 | 셸 `replay_flight.rs` | 그대로 |
| seq 연속(dedup · 구멍 붙듦) | 프론트 뷰(`HELD_MAX_*`) | 그대로 |
| transport 스트림 층(`stream.rs`) | — | **안 쓴다** — `ShellWire` · `DaemonWire` 의 `stream_mark` = `None` · `resume_stream` 을 부르지 않는다 |
| 공유 Inbound 포화 유실 | 없음(셸은 읽기 루프에서 Channel 로 직행) | 셸: `Dropped{Inbound}` = 연결 오염 → `reconnect_now()`(Live 면 끊고 다시 건다 — `machine.rs:272-285`) → 다시 붙은 뷰는 전량 replay. `frame_relay.rs` 의 `TooShort` 와 같은 규칙. 데몬: §3-1 · D11 |

**스트림 층을 켜면 깨지는 셋(확실 — 코드 판독):**
1. **셸은 무상태 통과 라우터라 같은 에이전트의 replay 를 창마다 전량 다시 받는다**(`replay_flight.rs:3-7`). transport 의 「`seq <= last` 면 버린다」(transport TRD §4-3 1)가 셸 한 곳에서 돌면 새로 연 창의 replay 가 통째로 버려진다.
2. **구멍을 보면 transport 가 `resume_request` 로 Subscribe 를 스스로 낸다**(`wire.rs` `resume_request` doc) — `replay_flight.rs:36-43` 「추적을 잃은 채 wire 로 나간 Subscribe 를 만드는 진입점을 두지 않는다 — 있으면 gen 펜스 붕괴」를 정면으로 어긴다.
3. **계약이 다르다** — transport 는 재요청 뒤 `resume_timeout`(2초 — `policy.rs:208`)이 지나면 잘림을 신고하고 붙든 것을 내보낸다 · 프론트는 구멍 뒤를 「시한 없이」 붙든다(CLAUDE.md 「replay→live」).

★「sink 와 셸 중계는 seq 를 건너뛰지 않는다」(CLAUDE.md 「핵심 불변식」)는 transport 의 공유 Inbound 가 새로 깰 수 있는 유일한 자리다★ — 위 표 마지막 줄이 그것을 막는다(R1).

---

## 3. 지금 고정할 seam — 다중 데몬 · 데몬끼리 (안 · 대가 · 권고, 미결정)

### 3-1. 받는 쪽 seam · 받은 상대 (D1)

- **사실:** `link.rs` 에 `Listener` 가 없고 `Registry` 는 들어오는 연결을 모른다(`transport/src/lib.rs:142-145`) · `WsListener::accept` 가 TCP 수락과 WS 업그레이드를 한 자리에서 해 느린 상대 하나가 다른 수락을 막는다(`ws.rs:29-30` · `:217-227`) · 재연결은 건 상대에게만 뜻이 있다 · `Handshake` 는 이미 방향 대칭(`link.rs:150-158`).
- **① `Listener` trait + `Registry::attach`(권고)** — `Listener::accept() -> Accepted{tx, rx, info: LinkInfo}`(`LinkInfo` = 원격 주소 · Origin · 나중 TLS 상대 키 칸) · `Registry::attach(id, accepted, handshake, policy) -> Peer` · 받은 상대는 Dialing 을 건너뛰고 Handshaking 에서 시작 · Live 를 잃으면 Backoff 없이 Closed · `reconnect_now` 무시 · 명부에서 스스로 내림. 수락 루프는 소비자(데몬)가 돌린다 — 상한 · 속도 정책이 소비자 몫(ADR-0177 결정 4). `Registry::new` 시그니처 그대로. 대가: 기계(`machine.rs`)에 「받은 상대」 모드 하나 · 수락 루프가 소비자마다 한 벌.
- ② `Registry::new(.., listeners)` — 수락까지 crate 안. transport TRD §12-7 이 예고한 시그니처 변경이고 상한 · 속도를 crate 정책 값으로 들여야 한다.
- ③ 데몬이 Registry 밖에서 Link 를 직접 돌린다 — ADR-0179 결정 3(같은 명부 · 같은 감독) 위반.
- `Handshake::start(&self)` 에 문맥 인자가 없어도 된다 — 받는 쪽은 연결마다 핸드셰이크 값을 새로 만들어(`Arc<dyn Handshake>` 를 attach 마다) `LinkInfo` 를 그 안에 담는다. trait 은 안 바뀐다.

### 3-2. 상대 신원 (D3)

- **사실:** `PeerId` = 소비자가 붙이는 불투명 문자열(`transport/src/event.rs:19-23` — 「나중에 화면에 그대로 뜬다」) · 데몬에는 영속 신원이 없다(`Hello` = 버전 · 능력뿐 — `protocol/src/messages.rs:433-438` · `DaemonInfo` = pid · 주소 · 토큰 — `protocol/src/discovery.rs`) · 토큰 = 데이터 폴더를 읽는 누구나의 공유 비밀(ADR-0179 영향 4).
- **① 영속 `DaemonId` + `Hello` 의 덧붙임 칸(권고)** — 데몬 상태 폴더에 처음 한 번 UUID v4 를 만들어 둔다(데몬 `DataLayout` 이 자리 소유) · `Hello{.., daemon_id: Option<String>}`(`#[serde(default)]` — 옛 셸 호환 · `PROTOCOL_VERSION` 그대로 · TS 바인딩 한 칸) · `PeerId` 는 이름표로 남긴다(셸 = 접속 정보의 키 · 데몬이 받은 연결 = `conn-<ConnId>`) · 키는 나중에 **신원에 묶는다**(신원 ≠ 키 — 키를 바꿔도 신원과 그것을 가리키는 영속 기록이 산다). 대가: 데이터 폴더 파일 하나 · 그 파일이 깨지면 새 신원(base `file` 규칙).
- ② 키 쌍을 지금 만들고 지문 = 신원 — 지금 쓰지 않는 암호 의존을 들이고, 키 회전이 신원 회전이 된다.
- ③ 이름표만 — 셸이 같은 데몬을 두 주소로 보거나 데몬끼리 서로를 가리킬 재료가 없다.

### 3-3. Wire 를 관계마다 가르나 (D2)

- **사실:** `Wire` 는 `Out`/`In` 이 갈린 제네릭이고 `decode` 에 상대 인자가 없다(`wire.rs:20-60`) · `Registry<W>` 는 Wire 하나 · ★셸↔데몬 링크는 이미 양방향 요청이다★ — 데몬→셸 `AgentEvent::CommandRequest{envelope}`(`messages.rs:608`)에 셸이 `AgentCommand::CommandOutcome{reply}`(`:333`)로 답하고, 그 짝은 데몬 `command_delivery` 가 기한 · 자리 표로 진다(transport pending 이 아니다).
- **① 관계마다 Wire · Registry · Listener(포트) 따로(권고)** — 셸↔데몬 = `ShellWire`/`DaemonWire`(지금 어휘) · 데몬↔데몬 = `PeerWire`(새 어휘 — protocol `peer` 모듈 뼈대). 각 Wire 는 자기 관계 안에서 양방향(요청 · 답장 · 알림의 합집합)이다. 데몬 쪽 `DaemonWire::request_tag` 는 전부 `None` — 데몬→셸 요청의 짝은 지금처럼 `command_delivery` 가 쥔다. 대가: 데몬이 포트 둘(데몬끼리 Listener 는 3단계에 기본 꺼짐 · `daemon.json` 에 칸 하나는 그때) · Inbound 두 줄. 원격 때 바인드 주소 · TLS 가 관계마다 달라지기 쉬워 갈라 두는 쪽이 자연스럽다.
- ② 합집합 Wire 하나 + `AuthFrame` 에 역할 칸 — 패킷 이름 공간이 겹치면 `decode` 가 못 가른다 · 셸이 데몬끼리 어휘를 타입으로 안다.
- ③ 한 포트 + 수락 뒤 첫 프레임 엿보기로 Registry 고름 — 핸드셰이크가 다시 읽게 프레임을 되돌려 놓는 Link 감싸기가 든다.

### 3-4. 인가 seam (D6 · D7)

- **사실:** 토큰 하나 = 전권 — 첫 프레임 판정(`net/src/ws.rs:331-360`) 뒤 명령마다 검사가 없다 · 인증 세 조각(프레임 모양 · 판정 · 비밀값 나누기)이 net 한 곳에 섞여 있다(메모 §6 열린 질문).
- **① `Principal` + 판정 둘(권고)** — 연결마다 `Principal`(`LocalClient` · `PeerDaemon(DaemonId)` · 나중 `Paired(client id)`) · `admit(attempt, &LinkInfo) -> Result<Principal, Refusal>`(데몬 핸드셰이크가 부른다) · `allow(&Principal, &AgentCommand) -> Result<(), Refusal>`(데몬 dispatch 입구 **한 곳** · 3단계 구현은 전부 허용). 나중 정책은 구현만 갈아끼운다. 대가: 명령마다 함수 호출 하나.
- ② `admit` 만 — 명령별 인가가 생기는 날 dispatch 입구를 그때 찾아 넓힌다.
- ③ 없음.
- **자리(D7):** ① 데몬 `link/auth.rs` 한 모듈 — 판정 trait + 지금 구현(토큰 · 상수시간 비교 · 버전) · 프레임 모양은 protocol(권고 — 메모 §6 메인 의견) ② 인증 crate — 소비자가 데몬 하나뿐 ③ transport 안 — ADR-0177 결정 4(인증 자료는 밖) 위반.

### 3-5. Link 교체 자리 — TLS · iroh (D8 과 함께 봄)

- **사실:** 전송 · 시간이 dyn(`link.rs:99-155`) · `Registry` 는 Dialer 를 하나 받는다(`registry.rs:80`) · `WsDialer` 는 tungstenite 기본 feature 라 `wss://` 가 dial 에서 실패한다(`ws.rs:33-34`) · TLS 는 바이트 스트림 **아래**에, iroh 는 QUIC 스트림 위에 메시지 경계를 따로 세워야 한다(보안 조사 §3-2 · §5 D).
- **① 주소 scheme 으로 가르는 합성 Dialer + 관계별 Listener(권고)** — `ws://`(3단계) · 나중 `wss://`(ws 어댑터 안 스트림 층 — tokio-rustls · 키 고정) · `iroh://`(새 Dialer/Listener 구현). 상대 키는 `LinkInfo` 칸으로 핸드셰이크 · `admit` 에 올라간다. `Registry` · `Peer` · 감독은 안 바뀐다.
- ② 상대별 Dialer — `Registry::add` 시그니처 변경.
- ③ 프레임 위 암호(Noise 손구현) — 보안 조사 §5 E(암호 자작 금기)에 가깝다.
- **Origin · Host 검사는 WS 업그레이드(HTTP)에서만 보인다** — 그래서 ws 어댑터 수락 설정에 산다(D8 ① · 값은 소비자가 준다). 데몬이 자기 수락을 짜고 `transport::ws::split_link`(`ws.rs:237` 공개)로 Link 를 만드는 ② 도 된다 — 대가 = 수락 강화가 transport 시험망 밖에 선다.

### 3-6. 접속 정보 인터페이스 (3-2 범위 — 메인 판단 가능, D 목록 밖)

- **사실:** `DaemonDiscovery`(`daemon_client/mod.rs:105-115`) = `ensure_spawn`(띄울 수 있음) · `read_live`(띄우지 않음 — ADR-0021) → `DaemonInfo`(토큰 포함) · 재연결마다 daemon.json 을 다시 읽어 옮겨 간 포트를 따라간다(`connection.rs:813-850`) · transport 는 주소와 핸드셰이크를 `add` 때 고정하고 시도마다 같은 `dialer.dial(&addr)` → `handshake.start()` 를 **한 태스크에서 차례로** 부른다(`peer.rs:540` · `:1324` · `:1342`).
- **제안:** `EndpointSource` — `ensure() -> Result<Endpoint>` · `current() -> Option<Endpoint>` · `Endpoint{key, address, credential(로그 · Debug 금지 newtype), expected_daemon_id: Option}` · 운영 구현 = 로컬 데이터 폴더. `Address` = 논리 주소(가안 `engram-local:<data_dir>` — transport 는 주소를 해석하지 않는다는 계약 그대로). 셸 `LocalDaemonDialer` 가 dial 마다 `current()` 로 실주소를 풀고(없으면 마지막으로 안 주소 — 지금 동작), 풀어 낸 자격을 **같은 상대의 핸드셰이크와 공유하는 칸**에 둔다. 처음 연결만 `ensure()`(띄울 수 있음)를 `Registry::add` **밖**에서 부른다 — 재연결은 dialer 안 `current()` 뿐이라 ADR-0021 이 구조로 선다.
- **거른 안:** transport 를 넓혀 `Dialer` 가 문맥을 돌려주고 `Handshake::start(ctx)` 가 받는다 — 「지금 확정」 trait 변경 · daemon.json 감시로 `Registry::add` 를 갈아끼운다 — 세대 · 예산이 끊긴다.
- **원격 대비:** 같은 trait 의 다른 구현(페어링 저장소 · 원격 주소)이 들어온다 — 메모 §4 「나중: 데몬 주소와 토큰을 어디서 얻나를 인터페이스로」의 실물.

---

## 4. 빈 공간 — 3단계가 비워 두는 칸

| 빈칸 | 3단계에 두는 모양 | 채우는 것 · 트랙 | 전제 | 연결 관리를 안 고쳐도 되는 이유 |
|---|---|---|---|---|
| TLS(키 고정 · mTLS) | scheme 합성 Dialer 에 `ws://` 하나 · `LinkInfo.peer_key` 칸 비어 있음 | `wss://` 어댑터 · 원격 | 데몬 키 · 페어링 · rustls 판 확정(RUSTSEC-2024-0399 판 범위 — 조사 §3-2) | TLS 는 Link 아래 스트림 층 · 위(감독 · 명부 · Wire)는 프레임만 본다 |
| iroh(QUIC · NAT) | 같음 | `iroh` Dialer/Listener · 원격 | 메시지 경계 규칙 · 중계 인프라 결정(조사 §3-2) | Link 가 프레임 단위 trait — WS 가정이 그 위로 새지 않는다(Origin 은 ws 어댑터 안) |
| 사용자 VPN | 데몬 Listener 바인드 주소가 설정값(3단계 = `127.0.0.1:0` 고정 기본) | 바인드 설정 · 원격 | 원격 Listener 는 TLS 와 함께(평문 원격 금지) | 바인드는 수락 루프 인자 하나 |
| 페어링 · 처음 믿기 | `EndpointSource` · `admit` · `Principal::Paired` 자리 | 페어링 저장소 + 판정 구현 · 원격 | 위협 모형(ADR-0179 영향 4 · `client-identity-survey`) | 자격은 `Endpoint.credential` 과 핸드셰이크 구현 안 — transport 는 모른다 |
| 명령별 인가 정책 | `allow()` 전부 허용 · 입구 한 곳 | 정책 표 · 다중 클라/원격 | 클라 신원 | dispatch 입구가 하나로 정해져 있다 |
| 클라 신원(다중 클라) | `Principal` 에 칸 · 토큰은 공유 그대로 | 원격과 함께(ADR-0179 영향 4) | 페어링 | `Principal` 은 ConnId 옆 표 — 연결 기계와 무관 |
| 셸 다중 데몬 표면 | 어댑터 상대 N · `DaemonKey` 타입 · 윗층 `primary()` 하나(D4 ①) | 에이전트 → 데몬 길찾기 · 레이아웃 영속의 데몬 키 · LLM 표면(`daemon.*`) · 다중 데몬 트랙 | 「앱 전체」 알림 접기(ADR-0180 결정 8) | 명부는 처음부터 N · `AgentId` 는 UUID 라 출력 길찾기는 이미 데몬 무관 |
| 데몬끼리 어휘 | `PeerWire` 뼈대(식별 · ping) · Listener 기본 꺼짐(D5) | 우편 중계 · 목록 합치기 · 데몬끼리 트랙 | 되돌아옴 방지 · 상대 부재 처리 조사(transport TRD §12-14) | 관계가 Wire · Registry 단위로 갈려 셸↔데몬을 안 건드린다 |
| 인증 실패 속도 제한 | 없음(명시 연기) | `admit` 구현이 `LinkInfo.remote` 로 센다 · 원격 | 원격 Listener | 판정이 `admit` 한 곳 |
| `daemon.json` ACL | 없음(명시 연기 — 조사 §4 구멍 7) | 데몬 쓰기 쪽 · 원격 또는 보안 트랙 | 플랫폼별 ACL(platform) | 파일 쓰기 쪽 일 |
| transport 스트림 층 | 안 씀(D10 ①) | 셸이 창별 스트림 키를 갖게 되면 재론 | replay_flight · 프론트 계약 개정 | Wire 칸은 그대로 — `stream_mark` 를 `Some` 으로 바꾸는 날 결정 |
| 재연결 「다시 연결」 UI | `Peer::reconnect_now` 와 사건이 셸까지 온다 | 팝업 + 버튼 + LLM 명령 · 셸 UI 트랙(ADR-0180 결정 7) | — | 입구는 transport 에 이미 있다 |

---

## 5. 보안 — 옮기는 것 · 새로 막는 것 · 미루는 것

| # | 보호 | 지금 | 목표 | 단계 | 시험 |
|---|---|---|---|---|---|
| 1 | loopback 바인드 | 데몬 `lib.rs:540` `127.0.0.1:0` | 데몬 수락 루프 인자(기본 loopback) | 3-3 U5 | 실소켓 — 바인드 주소 단언 · `ws_e2e` |
| 2 | Origin 허용 목록(없음 = 허용 · 밖 = 403) | `net/src/ws.rs:158-163` · `:264-283` | transport ws 수락 설정 훅 + 데몬이 같은 네 값(D8 ①) — ★네 값은 미실측 설계값 그대로 옮긴다(`ws.rs:155-157` 자인)★ | 3-3 U3 · U4 | 실소켓 — 허용 · 없음 · 거부(403) 셋 |
| 3 | 첫 프레임 인증 1초 | `ws.rs:124` · `:331` | `DaemonHandshake`(Await → Auth) + 받은 상대 정책 `connect_timeout = 1s` | 3-3 U4 | 인메모리(`ManualClock`) — 무프레임 1초 거절 · 실소켓 한 건 |
| 4 | 상수시간 비교 | `ws.rs:292` | 데몬 `link/auth.rs` | 3-3 U4 | 단위(`ws.rs:898` 시험 이사) |
| 5 | 토큰을 로그 · 오류에 안 싣는다 | `ws.rs:338` · 셸 `connection.rs:436` · `:445` | `Endpoint.credential` newtype(Display/Debug 가림) · 데몬 판정 | 3-2 U2 · 3-3 U4 | 단위 — `format!("{:?}")` 에 비밀 없음 |
| 6 | 버전 불일치 거절 문구(ADR-0178) | `handshake_error_frame` → Error 프레임 + 맨 close(`ws.rs:565`) | `Reject{last: Error 프레임, reason}` → 닫기 코드 4001 · 셸은 예산 안 씀(ADR-0180 결정 5) | 3-2 U2 · 3-3 U4 | 실소켓 `ws_reject` 꼴 · GUI 팝업 |
| 7 | 쓰기 시한 | 데몬 없음(`ws.rs:579-587` 자인 구멍) | transport `write_deadline` 5s(데몬 새 동작) | 3-3 U5 | transport `ws_write_deadline` + 어댑터 인메모리 |
| 8 | keepalive 20s/50s | `ws.rs:126-128` · 셸은 능동 ping 없음 | transport 정책 — 셸도 능동 ping(새 동작) | 3-2 U4 · 3-3 U5 | transport 인메모리(이미) |
| 9 | 느린 소비자 끊기 | `CONN_TX_CAP` 4608 + `close_signal`(`ws.rs:52`) | `outbound_queue` 4608 + `Disconnect`(D16) | 3-3 U4 | 어댑터 — 포화 → 끊김 |
| 10 | 연결별 받는 줄 상한 · 포화 처분 | `ws.rs:78` · `:96` · `:109` · `on_inbound_saturated` | 데몬 `link/server.rs`(D11 ①) | 3-3 U4 | `ws.rs` 시험 이사 |
| 11 | 단일 인스턴스 · daemon.json | `net/src/instance.rs` · `portfile.rs` | 데몬 · 열기 재시도 = `retry_busy`(ADR-0291 결정 15 · 4 × 100 ms) | 3-3 U2 | 시험 이사 · 바쁨 갈래 |
| 12 | **새** 업그레이드 시한 | 없음(`ws.rs:323` — 조사 §4 구멍 2 · 코드 순서 판독) | ws 수락: TCP 수락 즉시 연결 태스크로 · 업그레이드 시한(가안 1s) | 3-3 U3 | 실소켓 — TCP 만 열고 침묵 → 시한 안에 닫힘 |
| 13 | **새** 느린 핸드셰이크가 다른 수락을 막음 | transport `ws.rs:29-30` | 수락과 업그레이드 분리(12 와 같은 변경) | 3-3 U3 | 실소켓 — 멈춘 상대 하나 + 둘째 수락 성공 |
| 14 | **새** 연결 수 상한 | 없음(구멍 3) | 데몬 수락 루프 — 핸드셰이크 중 상한 · 전체 상한(가안 8 · 64 — ★미검 · 메인이 정해 보고하는 내부 상수★) | 3-3 U4 | 인메모리 — 넘친 수락 즉시 닫힘 |
| 15 | **새** 메시지 상한 | tungstenite 기본 64 MiB(구멍 5 · `ws.rs:85`) | 받는 쪽 `max_message_size` = `max_frame_size` = 16 MiB — transport 가 이미 내보내는 쪽에서 16 MiB 를 거절한다(`ws.rs:138-142`)는 사실과 맞춘다 · ★정당한 가장 큰 메시지 미측정★ | 3-3 U3 | 실소켓 — 초과 → 끊김 |
| 16 | **새** Host 검사(DNS 리바인딩) | 없음(구멍 8) | 업그레이드에서 `Host` ∈ {`127.0.0.1:<port>` · `localhost:<port>`} 또는 없음(D9) | 3-3 U3 · U4 | 실소켓 — 다른 Host → 403 |
| 17 | 인증 실패 속도 제한 | 없음(구멍 4) | **연기** — loopback · 256비트 토큰이라 무차별 대입 이득이 없고 전역 계수는 자기 DoS 가 된다 · 원격 Listener 와 함께 | 원격 | — |
| 18 | 암호화 | 없음(구멍 1) | **연기** — §4 TLS · 지금은 loopback 전용 | 원격 | — |
| 19 | `daemon.json` ACL | 없음(구멍 7) | **연기** — §4 | 원격/보안 | — |
| 20 | 제어 평면 HTTP Host 검사 | `/control/*` 미확인(조사 §4 구멍 8) | **범위 밖** — transport 와 별개 서버. `docs/tracking.md` 에 항목(오케스트레이터) | — | — |

---

## 6. 작업 단위

**규칙(사용자 2026-10-02 「망가지지 않는 단위로」 · 「임시 땜빵 금지」 · 전역 규율 「중간 어디서 멈춰도 빌드가 서게」):** 단위마다 워크스페이스 빌드 · 회귀 · ★모든 게이트가 초록인 채 끊는다★. 갈 곳을 먼저 세우고 옛 자리를 지운다. 타입을 바꾸는 단위는 그 타입의 호출부를 **같은 커밋**에서 다 맞춘다(자료구조만 갈아엎고 호출부를 다음 커밋에 미루지 않는다). 게이트 사본(CI · qa · CLAUDE.md · testing-strategy · crate 헤더)은 게이트를 바꾸는 그 커밋에서 함께 고친다. 각 단위 앞뒤로 `cargo test --workspace -- --test-threads=4` 결과 줄 · 통과 · 무시를 견준다(줄이 줄면 타깃 소실 — 초록이어도 멈춘다 · 3단계는 net 의 결과 줄 둘(lib · doc-tests — 미확인 셈)이 의도적으로 사라진다).

### 6-1. 3-1 — protocol 정리

| 단위 | 범위 | 시험 | 게이트 · QA |
|---|---|---|---|
| **U1** | wire golden 먼저 — 명령 버스 변형 여섯(`RegisterCommands` · `UpdateCommands` · `Command` · `CommandOutcome` · `CommandRequest` · `CommandReply` — `messages.rs:268-337` · `:608-656`)과 `AuthFrame` 의 지금 바이트를 protocol 시험으로 고정. 코드 무변경 | 단위(바이트 문자열 리터럴 — 다른 타입과 round-trip 대조 금지 · `net/src/auth.rs` golden 의 방식) | `cargo test -p engram-dashboard-protocol` · `/qa quick` |
| **U2** | `protocol::handshake::AuthFrame`(같은 serde) · net `auth.rs` = 재수출(net 의 protocol 의존이 optional 에서 무조건으로 — net gate 3 은 `--all-features` 라 3줄 그대로 · net 기본 feature 의 async 0 은 protocol 에 tokio 가 없어 그대로) · 소비자 전환(셸 `connection.rs:24` · `stop.rs:22` · `tests.rs:45` · `tests/stop_smoke.rs:186` · 데몬 `agent_conn.rs:808`(시험) · `tests/ws_e2e.rs:24`) · ★셸의 net 의존 삭제(`src-tauri/Cargo.toml:79-85`) + `Cargo.lock`★ · 셸 → net 0 게이트 | U1 golden 이 그대로 · 셸 `lib_unit` · `daemon_client_*` | net gate 5a/5b · platform gate 7 · 새 셸 게이트 · `--locked` · `/qa standard` |
| **U3** | `OwnerToken` protocol 사본(newtype — 안쪽 문자열 그대로 직렬화 = 같은 바이트, D17 ①) + 셸 · 데몬 끝자락 변환 함수 · 그 필드를 쓰는 모든 생성 · 해체 자리 한 커밋 | U1 golden · 명부 시험(`command_roster`) · 셸 등록 시험(`layout_commands`) | `/qa standard` |
| **U4** | 명령 칸 넷(봉투 · 답장 · 등록 · 등록 변경)을 `Box<RawValue>`(serde_json `raw_value`) · `#[ts(type = ..)]` 덮어쓰기 유지(D18 ① — 생성물 바이트 불변) · 짝 맞추기 두 함수를 셸 `protocol_state.rs` 로(본문 이사 — `Command`/`CommandReply` 갈래는 받은 즉시 봉투를 구조체로 풀어 번호를 읽는다 · ADR-0272 결정 4) · 데몬은 받는 쪽 하나(`agent_conn.rs:433`) · 짝 고정 시험(`messages.rs` 의 「한 쌍」 시험)을 셸로 · `From<command::RequestId>` 를 끝자락으로 · 「패킷은 풀렸는데 봉투가 깨진」 처리(ADR-0272 결정 6 — 셸: 요청 번호 없음으로 · 데몬: `Error{request_id: None}` 답 · 가안) · 데몬 중계(풀기 · 다시 싸기) | U1 golden(바이트 그대로가 요점) · 깨진 봉투 갈래 단위 · `control_send` · `mail_gate` · `ws_e2e` | 생성물 sync(바뀌지 않아야 한다) · `/qa standard` · `/review code full` |
| **U5** | protocol → command 간선 삭제(`protocol/Cargo.toml:8`) · protocol 의존 상한 게이트(정확히 1줄) · 문서(ADR-0272 영향의 `OwnerToken` 닫힘 · 메모 §11 `CommandListEntry`≈`RosterEntry` 는 보고 4단계로) | — | 새 게이트 · `/qa standard` |

- **U2 를 3-1 에 둔 이유:** 셸이 net 에서 쓰는 것이 `AuthFrame` 하나뿐이라(`src-tauri/Cargo.toml:79` 주석) 프레임이 protocol 로 오는 순간 셸 → net 간선이 걷힌다. 3-2 의 셸 작업이 net 을 아예 모른 채 시작한다.
- **U4 가 가장 넓다** — 데몬(`connection_core` · `command_delivery` · `command_roster` · `agent_conn`) · 셸(`inbound.rs` · `view_commands.rs` · `commands/agent.rs:248`) 의 생성 · 해체 자리를 한 커밋에서 맞춘다. 번지면 칸 하나씩(봉투 → 답장 → 등록 둘) 커밋을 나눈다 — 칸마다 golden 이 바이트를 잰다.

### 6-2. 3-2 — 셸 부착

| 단위 | 범위 | 시험 | 게이트 · QA |
|---|---|---|---|
| **U1**(transport) | `RequestError::Disconnected` 에 「보냈나」 칸(D12 ① — `fail_pending`(`peer.rs:1151-1162`)이 대기 슬롯(보냄)과 큐(안 보냄)를 이미 따로 훑으므로 거기서 가른다) · `Superseded` doc 이 「승계」라 적는 자리를 lib 헤더(둘 다 실패)와 맞춤(`pending.rs:29`) · 헤더 게이트 ① 문장 고침(메모 §11 `:332`) | transport 인메모리(보냄 · 안 보냄 갈래) | transport gate 1~4d · `/qa standard` |
| **U2** | 셸 `daemon_client/link/` — `ShellWire`(Out = `AgentCommand` · In = `ShellIn{Event(AgentEvent) · Output(Vec<u8>)}` — 출력은 해독하지 않고 통과 · `request_tag`/`reply_tag` = 3-1 U4 의 두 함수 · `stream_mark` = `None` · `resume_request` 는 crate 가 부를 길이 없음을 시험으로 고정) · `ShellHandshake`(Send(Auth) → Hello = Done · Error = Reject · 그 밖 프레임은 버리고 Await — `connection.rs:2020-2050` 과 같다) · `LocalDaemonDialer` + `EndpointSource`(§3-6) · `TransportEvent` → `ConnectionState`/`DaemonEvents` 사상 · `RequestError` → 지금 글(`SENT_OUTCOME_UNKNOWN` · `UNSENT_ON_DISCONNECT` 등) 사상 · `DaemonKey` 타입. **아무도 안 부른다** | transport `MemoryNetwork` + `ManualClock` — 핸드셰이크 셋 · 거절은 예산 안 씀 · 포트 이동 따라가기 · 마지막 주소로 · 오류 글 바이트 대조 | 셸 끝자락 게이트(§2-4) · `/qa standard` |
| **U3** | 새 연결 구동(가안 `connection_v2`)을 `DaemonClient` 생성자 선택으로 — 시험 생성자 먼저. `daemon_client/tests.rs` 의 가짜 WS 서버 시험을 두 구동에 다 돌린다(매개화). 운영은 아직 옛것 | 기존 실소켓 스위트 전량(바이트 그대로라 가짜 서버가 그대로 선다) · 공유 Inbound `Dropped` → `reconnect_now` 시험 | `lib_unit` · `daemon_client_pending` · `daemon_client_replay` · `/qa standard` |
| **U4** | 운영 생성자(`new_real_with_owned_runtime`)를 새 구동으로 · 정책 주입(§7 — D14 · D15) · `REPLY_TIMEOUT`(`commands/agent.rs:277`) 걷기 · 옛 구동 삭제 · `daemon_client_pending` 을 M1(겹친 번호 = 양쪽 실패 — transport TRD §4-1)로 다시 씀 · 셸 `tokio-tungstenite` → dev | 위 전량 + 다시 쓴 pending | `/qa full`(GUI §6-4 ①~⑤) · `/review code full` |
| **U5** | 정지 클라를 transport 위로(`link/stop.rs`) — 연결이 있으면 그 `Peer` 로 `StopDaemon` 알림 · 없으면 한 번 쓰는 상대(재연결 0 · 짧은 시한)로 알림 뒤 끊김 관측 = `DaemonClosed` · 셸 `tungstenite` 삭제 · `stop_smoke` 다시 겨눔(T-52 — 알려진 실패 · 원인 미규명. 고쳐지면 기록, 아니면 그대로 둔다) · 셸 직접 의존 tungstenite 0 게이트 | `stop_smoke --ignored`(실 데몬) · 트레이 GUI | `/qa full` |
| **U6** | 상대 N 자리(D4 ①) — 어댑터가 `Registry` 하나에 상대 N 을 담고 윗층(`router` · `layout` · 명령)은 `primary()` 하나를 본다 · 가짜 데몬 둘을 붙이는 시험(사건이 상대 표식으로 갈리고 하나가 죽어도 다른 하나가 산다 · 출력은 `AgentId` 로 길찾기) | 실소켓 가짜 서버 둘 · 같은 PC 실 데몬 둘(`ENGRAM_DATA_DIR` 둘 — `#[ignore]` 수동) | `/qa standard` |

- **U3 · U4 를 가른 이유(전역 규율 「쪼개는 건 착수 시점」):** 2,050줄 연결 루프를 한 커밋에 갈아 끼우면 중간 상태가 없다. 새 구동을 옆에 세워 같은 시험을 두 번 돌리고, 운영 전환은 생성자 한 줄이다. 되돌릴 지점 = U3 커밋.
- **시계 함정(가능성 높음):** transport `SystemClock::now()` = `std::time::Instant`(`clock.rs:16` · `:34-35`) · `sleep` = tokio. 셸 시험 다수가 `tokio::time::pause`/`advance` 로 시간을 흘린다(`tests.rs:1330` 등) — 새 구동 시험은 `ManualClock` 을 꽂는다. 옛 시험을 그대로 돌리면 시한 계산이 멈춘 시계와 갈린다.

### 6-3. 3-3 — 데몬 부착 · net 걷기

| 단위 | 범위 | 시험 | 게이트 · QA |
|---|---|---|---|
| **U1** | 데몬 `conn_port.rs` = `pub use engram_dashboard_net::frame_port::*` · 데몬 소스 · 시험의 net 경로를 전부 이쪽으로(`agent_conn.rs:42` · `connection_core.rs:83` · `command_delivery.rs:106` · `command_roster.rs:41` · `status_fanout.rs:32` · `control/commands.rs:29` · `usage_service/{mod.rs:43, watch.rs:22}` · `test_doubles.rs:18` · `tests/{control_agent.rs:31, mail_gate.rs:119-154}`). 기계적 · 동작 무변경 | 기존 전량 | `/qa quick` |
| **U2** | `instance` · `portfile` 를 데몬으로(`data_dir.rs:17` `DAEMON_FILE` 포함) · 열기 재시도 = `retry_busy`(★동작 차이 — `is_busy` 는 접근 거부도 잠김으로 쳐 최악 400 ms 늦게 같은 오류가 돌아온다 · 지금은 공유 위반만 다시 한다 `instance.rs:242`★) · net 의 platform 간선 삭제 → net gate 2a 삭제 · gate 3 기대값 3 → 2 · platform 4f/4h 명단 경로 · ADR-0135 앵커 | 시험 이사 · 바쁨 갈래 · 같은 경로 시험(셸 사본 — ADR-0271 결정 4) | net gate 3 · platform 4f/4h · `/qa standard` |
| **U3**(transport) | `Listener` trait · `Registry::attach` · 기계의 받은 상대 모드(§3-1 ①) · `LinkInfo` · ws 수락 강화(분리 · 업그레이드 시한 · Origin/Host 훅 · 크기 설정 — 값은 전부 소비자 인자) · 양끝 인메모리 통로 + `MemoryListener`(하네스 — 지금 `MemoryNetwork` 는 먼 끝이 대본이라 두 명부를 서로 잇지 못한다 `testing.rs:349-416`) · 데몬은 아직 안 씀 | 인메모리 — 받은 상대 손실 = Closed · 다시 안 건다 · 두 명부 양방향 요청 · 실소켓 — §5 12 · 13 · 15 · 16 · Origin 403 | transport gate 1~4d(4c 에 새 실소켓 스위트 등록) · `/qa standard` · `/review code full` |
| **U4** | 데몬 `link/` — `DaemonWire`(In = `AgentCommand` · Out = `DaemonOut{Event · Output(이미 짠 바이트)}` · `request_tag` 전부 `None` · `stream_mark` `None`) · `DaemonHandshake` · `auth.rs`(판정 seam · 토큰 · 상수시간 · 버전) · `Principal` + `allow()` 입구(D6 ①) · `server.rs` = `conn_port` 계약을 transport 위에 구현(수락 루프 · 연결 상한 · `ConnId` ↔ `PeerId` · 연결별 받는 줄 · dispatch · bypass · 포화 처분(net 상수 64 · 16 MiB · 8 · 10 s 그대로) · `FrameSink` = `Peer::notify` · `FrameFanout` = 상대별 알림(D13)) · 아직 수락 루프에 안 꽂음 | `net/src/ws.rs` 시험 22개 중 의미가 남는 것을 어댑터로 이사 · 인메모리 다중 상대(셸 둘이 붙어 팬아웃 · 하나 포화가 다른 하나를 안 끊음) · 인가 가짜(거절 판정이 명령을 막는다) | 데몬 끝자락 게이트 · `/qa standard` · `/review code full` |
| **U5** | `run_accept_loop`(`lib.rs:337`)와 `start_test_server*`(`:854-871`)를 어댑터로 전환 · 받은 상대 정책(`connect_timeout` 1 s · `outbound_queue` 4608 · keepalive 20/50) | `ws_e2e` · `mail_gate` · `control_agent` · 셸 dev 의존 시험(데몬 시험 서버를 쓴다) 전량 | `/qa full`(GUI §6-4 전부) |
| **U6** | `conn_port.rs` 를 재수출에서 자기 정의로(net `frame_port.rs` 그대로 옮김) · net crate 삭제(멤버 · `Cargo.lock` · 데몬 `Cargo.toml:99`) · §2-4 의 걷는 게이트 · 대신 세우는 게이트 · 사본 다섯 · CLAUDE.md 「백엔드 모듈 맵」(net 항목 삭제 · transport 항목 신설 — 메모 §11 `:345`) · 「네트워크 행 격리 게이트」 절 → 「끝자락 게이트」 | 워크스페이스 결과 줄 대조(net 몫이 사라진다) | `/qa standard` · `/review doc`(load-bearing 문서) |
| **U7** | 데몬 신원(D3 ① — `DaemonId` 파일 · `Hello.daemon_id`) · 데몬끼리 뼈대(D5 — protocol `peer` 모듈: `PeerAuth{daemon_id, credential, protocol_version}` · `Identify` 요청/답 · `Ping` 요청/답 · `PeerWire` · 데몬 `link/peer.rs` · 둘째 Listener(기본 꺼짐)) | 인메모리 두 명부(양방향 요청 · 한쪽 죽음 · 신원 대조 실패 = 거절) · D5 ② 면 같은 PC 실 데몬 둘(`#[ignore]`) | 생성물 sync(`Hello` 한 칸) · `/qa standard` |

- **U1 이 맨 앞인 이유:** 위층 30여 자리가 `net::frame_port` 를 이름으로 부른다. 재수출 한 겹을 먼저 넣으면 U4~U6 이 위층을 한 줄도 안 건드리고, net 삭제(U6)는 재수출을 정의로 바꾸는 한 파일이 된다.
- **U3 이 U4 앞:** 어댑터가 쓸 transport 표면이 먼저 선다. U3 만으로는 데몬 동작이 바뀌지 않는다.
- **U4 가 이 단계의 본체다** — net 의 연결별 받는 줄 · bypass(`ws.rs:799-900`)는 transport 에 대응물이 없고(공유 Inbound 한 줄 — `transport/src/lib.rs:128-131`) 어댑터가 그 의미를 잇는다(D11 ①).

### 6-4. GUI 실측 (`/qa full` — 절차는 qa 바인딩 §full)

① 연결 · 명령 왕복(에이전트 띄우기 · 끄기) ② 데몬 재시작 → 옮겨 간 포트로 다시 붙음(ADR-0021 — 띄우지 않는다) ③ 버전 불일치 데몬 → 거절 팝업 · 재연결 예산을 안 태움(ADR-0178 · 0180 결정 5) ④ 창 여럿 + 팝아웃에서 replay → live · seq 연속(출력이 많은 에이전트 하나를 붙인 채 새 창) ⑤ 트레이 데몬 끄기 · 앱 종료 ⑥(3-3) 같은 PC 데몬 둘 + 셸 하나 — D4 ① 의 수동 실측(primary 만 화면에 · 둘째는 로그로).

### 6-5. ADR — 써야 하거나 고칠 것 (목록만 · 채번 · 도장 = `/adr`)

| 시점 | 새 ADR(가칭) | 고치거나 잇는 ADR |
|---|---|---|
| 3-1 착수 전 | **A** 「net 을 걷는다 — 연결 기계 = transport · 어휘(인증 프레임 포함) = protocol · 데몬 전용 = 데몬」(결정 후보 7 · D7) | ADR-0272 개정(D17 · D18 · 짝 맞추기의 실제 자리) · ADR-0129 개정(인증 프레임 소유) · ADR-0177 링크(결정 4 의 「밖」) |
| 3-2 착수 전 | **B** 「셸 부착 — 접속 정보 인터페이스 · 논리 주소 + 풀이 Dialer · 정책 주입 값 · 끊김 오류의 보냄 칸 · 정지 클라」(D12 · D14 · D15) | ADR-0036 개정(재연결 · pending 소유 → transport) · ADR-0271 결정 5 닫힘 · 링크: ADR-0021 · 0195 · 0178 · 0180 결정 4 · 0181 「적용 시 30초」 |
| 3-3 착수 전 | **C** 「받는 쪽 seam — Listener · Registry::attach · 받은 상대는 다시 걸지 않는다」(D1 · transport TRD §12-7 닫힘) · **D** 「관계 · 신원 — 관계마다 Wire/Registry/Listener · 영속 데몬 신원 · 다중 데몬 · 데몬끼리 깊이」(D2 · D3 · D4 · D5) · **E** 「전송 보안 층 · 인가 seam」(D6 · D8 · D9 · §5) | ADR-0129 폐기(A 가 대체) · ADR-0130 폐기(대상 소멸) · ADR-0179 링크(결정 3 의 실물 · 영향 4 자리) · ADR-0135 링크(자리 이동) · ADR-0291 결정 15 링크 · ADR-0177 링크(결정 1 마지막 걸음 · 결정 6 팬아웃 = D13) · D10 · D11 · D16 은 B 또는 C 에 |

---

## 7. 정책 기본값 맞추기 (D14 · D15 · D16)

| 값 | transport 기본(`policy.rs:191-211`) | 지금 | 안 | 권고 |
|---|---|---|---|---|
| 재연결 횟수 | 3 | 셸 5(`connection.rs:90`) · 프론트 직결 경로 5(`src/api/wsTransport.ts:383` — 운영 아님) · 산문 「5회」(`agentClient.ts:191` · `daemonControl.ts:92`) | ① 3-2 는 5 를 주입하고 「다시 연결」 팝업이 착지하는 날 3 ② 3 지금 ③ 3 + 팝업 · 버튼을 3-2 에 | **①** — ADR-0180 이 3 을 떠받치는 전제로 버튼(결정 7)을 들었고 그 UI 가 아직 없다(`git grep "다시 연결" src` → 0 · 실측). ② 는 더 빨리 포기하는 것만 남는다 · ③ 은 UI 범위가 3-2 에 든다 |
| 요청 시한 | 8 s | `forward_daemon_command` 30 s(`commands/agent.rs:277`) · 나머지 상한 없음 | ① 30 s 공용 주입 · spawn 실소요를 3-2 QA 에서 재 기록 · 8 s 는 따로 ② 8 s 지금(ADR-0181 결정 1) ③ 명령별 | **①** — ADR-0181 이 「적용 시 30초를 함께 정한다」 · 「8초의 유일한 실질 위험 = spawn 실소요 미측정」을 적었다. ① 은 「공용 하나」를 성립시키되(없던 상한을 얻는 핸들러가 생긴다) 위험을 재고 내린다 |
| 백오프 | 500 ms · ×2 · 상한 10 s · 지터 0.2 | 같음 · 지터 없음(`connection.rs:93-96`) | 그대로 받는다 | 지터는 새 동작(N 데몬일 때 동시 재연결 흩기) |
| 붙어 버틴 시간(`live_dwell`) | 30 s | 없음(성공 즉시 리셋) | 그대로 | 붙자마자 끊기는 데몬에 영원히 매달리지 않는다 |
| 연결 · 핸드셰이크 시한 | 10 s | 10 s(`connection.rs:84`) | 그대로 · 데몬이 받은 상대는 1 s(§5 3) | — |
| keepalive | 20 s / 50 s | 데몬 같음 · 셸 능동 ping 없음 | 그대로 | 셸이 데몬 죽음을 스스로 안다(transport TRD §6) |
| 쓰기 시한 | 5 s | 셸 없음(미확인 — 쓰기 시한 상수를 못 찾음) · 데몬 없음 | 그대로 | 데몬 새 동작 — R7 |
| 나가는 큐 | 512 | 셸 512 · 데몬 4608(`net/src/ws.rs:52` = 링 4096 + 512) | ① 데몬 4608 유지 ② 링 이하 | **①** — replay 한 번이 4096 을 미는데 512 면 포화 → `Disconnect` → 다시 replay 가 되풀이된다. ADR-0177 근거의 「4608 > 4096 이 거꾸로」는 큐가 링을 다 담아야 한다는 net 주석(`ws.rs:50-51`)과 정면으로 갈려 근거를 다시 봐야 한다(미확인 — 그 근거의 원문 대조 안 함) |
| 공유 받는 줄 | 512 | 셸 없음 · 데몬 연결별 64칸 | 셸 512 + `Dropped` = 재동기(§2-5) · 데몬은 어댑터가 즉시 비운다(D11) | — |
| 디코드 실패 | 프레임 버림 | 셸: 모르는 tag = 자리채움 · 짧은 프레임 = 끊음(`frame_relay.rs`) | 출력은 Wire 가 해독하지 않고 통과 → `frame_relay` 규칙 그대로 · JSON 실패는 버림(지금 `wait_for_hello` 와 읽기 루프도 버린다) | — |

---

## 8. 위험 · 모르는 것 · 사용자 결정 목록

### 8-1. 위험

1. **seq 줄기에 구멍(R1 · 가장 큼)** — transport 공유 Inbound 가 차면 출력 프레임이 사라지고(`transport/src/lib.rs:128-131` 「그 사이 도착한 패킷은 실제로 사라진다」) 뷰는 구멍 뒤를 붙든 채 멈춘다. 막는 것 = §2-5 마지막 줄(`Dropped` → 재동기) + 그 시험(3-2 U3). 스트림 층을 켜는 실수도 같은 결말이다(D10).
2. **데몬 받는 쪽 의미 손실(R2)** — net 의 연결별 받는 줄 · 포화 처분(`Refused` 답장 의무 · `Bypass` 취소 우선)은 transport 에 없다. 어댑터가 공유 줄을 즉시 비우지 못하면 명령이 조용히 사라져 호출자가 마감까지 매달린다. 3-3 U4 의 시험 이사가 그 회귀망이다.
3. **연결 루프 교체의 크기(R3)** — 셸 2,050줄 루프 + 5,978줄 시험 · 데몬 2,248줄 서버 행. 쪼갬(3-2 U3/U4 · 3-3 U1/U4/U5)과 시계 함정(§6-2)이 대책이다.
4. **replay 폭주(R4)** — 데몬 상대 정책을 기본값(512)으로 두면 replay 마다 끊김 · 재replay 고리(D16).
5. **요청 시한(R5)** — 30 s 로 두면 지금 상한이 없던 핸들러가 30 s 에 실패하기 시작한다 · 8 s 로 두면 느린 spawn 을 오판할 수 있다(실소요 미측정).
6. **Origin 값 · Host 검사(R6)** — 네 Origin 은 미실측 설계값 · Host 검사가 정당한 클라(`[::1]` 등)를 막을 수 있다 — 우리 클라는 `127.0.0.1` 로 건다(`connection.rs:435`)는 것만 확인했다.
7. **데몬 쓰기 시한 5 s 새로(R7)** — 셸 읽기가 5 s 멈추면 끊고 전량 replay. 셸 읽기는 자기 런타임 태스크라 가능성은 낮다(미검).
8. **겹친 번호 동작 변경(R8)** — transport 는 둘 다 실패(M1). `daemon_client_pending` 은 다시 쓴다 — 사용자 결정 2026-09-06 그대로라 새 결정은 아니다.
9. **「지금 확정」 표면을 다시 연다(R9)** — `Registry::attach` · `Listener` · `RequestError` 칸 하나. 소비자가 0 에서 출발하므로 깨지는 쪽이 없다 — 3단계가 그 표면을 고칠 마지막 값싼 때다.

### 8-2. 모르는 것 (미확인)

정당한 가장 큰 메시지(§5 15) · spawn 실소요(D15) · 핸들 쪽 큐 포화(`overflow` — `peer.rs:175` 「감독이 걷어 가 처리한다」)가 끊기까지의 지연 · ADR-0177 근거의 「4608 > 4096」 원문 · net 결과 줄 둘이 정확히 몇인지 · T-52(`stop_smoke`) 원인 · WebView2 가 실제로 보내는 Origin.

### 8-3. 사용자 결정 목록 — 권고안이 첫째

**묶음 A — 구조**(A 의 답이 뒤를 바꾼다: D2 ② 면 D5 의 뼈대가 셸↔데몬 Wire 안으로 들어가고 D6 의 `Principal` 이 Auth 의 역할 칸에서 나온다 · D3 ③ 이면 D5 는 「서로 누군지 모르는」 연결 시험까지만 된다)

1. **D1 받는 쪽 seam** — ① `Listener` + `Registry::attach`(시그니처 그대로 · 수락 정책은 데몬) ② `Registry::new` 가 Listener 를 받아 수락까지(시그니처 변경 · 상한을 crate 정책으로) ③ 데몬이 명부 밖에서(ADR-0179 결정 3 위반).
2. **D2 관계 가르기** — ① 관계마다 Wire · Registry · Listener(포트 둘 · 어휘 격리) ② 합집합 Wire + Auth 역할 칸(포트 하나 · 셸이 데몬끼리 어휘를 안다) ③ 한 포트 + 첫 프레임 엿보기(Link 감싸기가 든다).
3. **D3 데몬 신원** — ① 영속 `DaemonId` + `Hello` 칸(파일 하나 · 키는 나중에 묶음) ② 키 쌍 지금(안 쓰는 암호 의존 · 키 회전 = 신원 회전) ③ 이름표만(데몬끼리 · N 데몬의 재료 없음).
4. **D4 셸 다중 데몬 깊이** — ① 어댑터 N · 윗층 primary · 가짜 데몬 둘 시험(구조가 실물로 선다 · 윗층 무변경) ② 타입 · seam 만(값쌈 · 셸 쪽 N 은 미검) ③ 화면 · LLM 까지(3단계 범위를 넘는다).
5. **D5 데몬끼리 깊이** — ① `PeerWire` 뼈대 + 인메모리 두 명부 시험(실 프로세스 없음) ② ① + 실 둘째 Listener(기본 꺼짐) + 같은 PC 실 데몬 둘 시험(실물 확인 · 포트 · daemon.json 칸이 지금 든다) ③ 나중 트랙(3단계는 seam 만).

**묶음 B — 보안**(D6 ① 이면 D9 의 속도 제한이 `admit` 구현 하나로 나중에 들어온다)

6. **D6 인가 seam** — ① `Principal` + `admit`/`allow`(입구 한 곳 · 지금 전부 허용) ② `admit` 만(명령별 인가 날에 입구를 찾음) ③ 없음.
7. **D7 인증 자리** — ① 데몬 안 한 모듈 · 프레임은 protocol(소비자 하나에 맞음) ② 인증 crate(소비자 하나) ③ transport 안(ADR-0177 결정 4 위반).
8. **D8 Origin · Host 검사 자리** — ① transport ws 수락 설정(값은 데몬 · transport 시험망 안) ② 데몬 자기 수락 + `split_link`(시험망 밖).
9. **D9 새 방어 범위** — ① 업그레이드 시한 · 수락 분리 · 연결 상한 · 16 MiB · Host 검사 지금, 속도 제한 · TLS · ACL 연기(실측 구멍을 닫는다 · 원격 몫은 원격 때) ② 옮기기만(동작 변화 0 · 구멍 그대로) ③ 속도 제한까지(loopback 에서 이득 적고 자기 DoS 위험).

**묶음 C — 스트림 · 큐**(D10 ② 를 고르면 D11 · D13 과 §2-5 가 다시 써진다)

10. **D10 transport 스트림 층** — ① 안 쓴다(불변식 그대로 · transport 기능 일부 놀림) ② 쓴다(replay_flight · 프론트 계약을 다시 — 3단계 범위 밖 크기).
11. **D11 데몬 받는 줄** — ① 어댑터가 공유 Inbound 를 즉시 연결별 줄로(net 의미 그대로 · 어댑터 코드 큼) ② transport 에 상대별 Inbound 선택지(crate 표면 증가) ③ 공유 줄 유실을 받아들임(명령 소실).
12. **D12 끊김 오류의 「보냈나」** — ① `RequestError::Disconnected` 에 칸 하나(지금 셸이 가르는 두 글 유지 · transport 표면 변경) ② 한 글로 합침(「안 나갔다」와 「결과 모름」을 사용자가 못 가른다).
13. **D13 팬아웃 포화** — ① 그 연결을 끊는다(조용한 유실 결함을 끊김 + 전량 replay 로 · transport TRD §12-2) ② 버리고 신고(연결 유지 · 셸 화면이 어긋날 수 있다).

**묶음 D — 정책 값**(§7 — 서로 독립)

14. **D14 재연결 횟수** — ① 5 주입 → 팝업 버튼 착지 때 3 ② 3 지금 ③ 3 + 버튼을 3-2 에.
15. **D15 요청 시한** — ① 30 s 공용 + spawn 실측 → 8 s 는 따로 ② 8 s 지금 ③ 명령별.
16. **D16 데몬 나가는 큐** — ① 4608 유지 ② 링 이하.

**묶음 E — protocol**(3-1)

17. **D17 `OwnerToken`** — ① protocol 사본(같은 바이트 · 타입 검사 남음) ② 원문 칸(등록 패킷 머리가 불투명해짐).
18. **D18 원문 칸의 TS** — ① `#[ts(type)]` 덮어쓰기 유지(생성물 불변 · 손 문자열이 drift 위험으로 남음 — `messages.rs:1789` 가 이미 지키는 그것) ② `unknown`/JSON(ADR-0272 영향 그대로 · 프론트 소스 import 0 이라 영향 작음).

---

## 9. 문서 후속 · 미검

- **문서 후속(각 단위가 고친다 — 이 TRD 는 안 고친다):** CLAUDE.md(「백엔드 모듈 맵」 net · transport · protocol 항목 · 「네트워크 행 격리 게이트」 · 「빌드·검증 명령」 net 줄 · 「의존성」 `tungstenite` 줄 — U6 · 3-2 U5) · `.claude/skill-bindings/qa.md`(`:11` 예외 문단 · `:84-85` · `:280-285`) · `docs/testing-strategy.md` §net · `crates/engram-dashboard-transport/src/lib.rs` 헤더(받는 쪽 seam 한계 문장 `:142-145` · 게이트 ① 문장) · `transport/src/ws.rs` 「알려진 한계」 · `src-tauri/Cargo.toml:79-85` · `:94-98` 주석 · 메모 §10 3. 착지 표시 · §11 처리 표시(`:327` · `:332` · `:345`) · `docs/tracking.md`(제어 평면 Host 검사 항목 · T-52 결과) · step-log(오케스트레이터).
- **미검(이 판):** 빌드 · 시험을 하나도 돌리지 않았다 · 기준선 회귀 수(3-1 U1 착수 직전에 잰다 · CLAUDE.md 마지막 기록 = 결과 줄 60 · 4371 통과 · 31 무시) · §8-2 전부 · `RawValue` 가 외부 태그 enum 안에서 바이트를 지키는지(ADR-0272 영향 — serde-rs/json#497 · 3-1 U1 golden 이 잰다) · `Host` 헤더 실제 값(tungstenite 클라가 보내는 꼴) · net 시험 22개 중 어댑터로 옮길 수 · 같은 PC 데몬 둘의 데이터 폴더 격리(`ENGRAM_DATA_DIR` — `stop_smoke` 선례만 확인).
