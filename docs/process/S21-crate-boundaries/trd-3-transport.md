# TRD — 경계 리팩터링 3: transport 를 셸 · 데몬에 붙이고 net 을 걷는다 (S21)

> 상태: **2판(2026-10-12) — 적대 리뷰(deep · 3인 · 전원 FIX) 반영 · 재리뷰 전.** 코드는 한 줄도 바뀌지 않았다. ★§8-3 「사용자 결정 목록」이 전부 열려 있다 — 결정 전 구현 진입 금지(CLAUDE.md 「개발 스텝」)★. 1판과 다른 점 = 바로 아래 「2판 변경」.
> **결정 출처(이 판까지):** 순서 3-1 → 3-2 → 3-3 은 메모 §10 「3. transport」 그대로다(3-0 은 2판이 제안하는 앞 단계 — D19). 사용자 지시(2026-10-12) — ① 클라(셸) 하나가 데몬 **여럿**에 붙고 데몬끼리 **서로** 말하는 구조를 지금 3단계에서 세운다 · 원격(다른 PC) 적용 · 시험은 나중 트랙 · 3단계가 내보내는 동작은 셸 하나 + 로컬 데몬 하나 · 다중 데몬 · 데몬끼리는 transport 인메모리 하네스와 한 PC 여러 데몬으로 잰다 ② 보안 층 나누기(조사 `docs/research/transport-security-build-vs-adopt-2026-10-12.md` §0 · §1 · §4 · §5) — transport 는 유지 · 암호화 · 상대 인증은 나중에 Link 층에서 빌린다 · 인가 · 페어링 · Origin · 자원 상한은 우리 몫 · net 의 방어는 옮기고 실측된 구멍은 막거나 명시로 미룬다.
> **범위 = 작업 순서 3 전부** — 3-0 transport 보강 · 3-1 `protocol` 정리 · 3-2 셸 부착 · 3-3 데몬 부착 + net 걷기. PRD 급 출처 = `docs/refactoring/architecture-discussion-2026-09-26.md` §6 결정 후보 7(`:222-229`) · §7 결정 후보 8(`:241-253`) · §10 3.(`:297`).
> **배치 근거:** `docs/README.md` 「새 내용을 어디에 넣나」의 「새 기능 설계 착수 → `process/SN-name/`」. 같은 경계 리팩터링의 형제 TRD(1-1 · 1-3 · 2-1 · 2-2 · 2-3 · A)가 이 폴더에 있다.
> **표기:** 「실측」 = 기준 커밋 `13084377`(브랜치 `v0.4.1/refactor/transport-attach` HEAD — 1판 기준 `255b2830` 과 코드가 같다, 차이는 문서뿐)의 **정적 판독**이다. 빌드 · 시험은 돌리지 않았다 · 「제안」 = 이 TRD 의 안(새 이름은 전부 가안) · 「미확인」 = 확인하지 않은 것.
> 앵커: ADR-0177(transport 헌장) · ADR-0178(버전 거절) · ADR-0179(데몬 N 클라 · 클라 N 데몬) · ADR-0180(알림 · 재연결 층) · ADR-0181(요청 시한) · ADR-0182(사건 방출) · ADR-0272(패킷은 남의 구조체를 품지 않는다) · ADR-0129 · ADR-0130(net) · ADR-0135(단일 인스턴스 파일) · ADR-0157(명령 입구 검문) · ADR-0036(셸 = 데몬 클라 하나) · ADR-0021(재연결은 띄우지 않는다) · ADR-0195(끊긴 동안 명령 즉시 실패) · ADR-0046 · ADR-0231(replay · seq 연속) · ADR-0271 결정 5(정지 클라 임시 거처) · ADR-0291 결정 15(단일 인스턴스 열기 재시도) · ADR-0012 · TRD `docs/process/S21-transport-crate/trd.md`(이하 「transport TRD」).

## 2판 변경 — 리뷰 반영

리뷰 셋의 지적은 서로 갈리지 않았다. 각 지적을 코드에서 다시 대조했고(정적 판독), 어긋난 근거는 아래 「반영 안 함 · 부분 반영」에 둔다.

| 묶음 | 지적(요지) | 2판에서 | 결정 영향 |
|---|---|---|---|
| A1 | transport 공유 받는 줄이 손실형 — 1판 대책(유실 → 다시 붙기)이 고리 · 예산 되감기 · 데몬 사이 굶김을 만들고, 데몬 쪽은 명령이 무답으로 사라진다 | §3-0 ① · 3-0 U1 · §2-5 마지막 줄 · R1 | **D11 추천 바뀜** · **D19 신설** |
| A2 | 생애 사건도 같은 손실 줄 | §3-0 ② · 3-0 U1 | — |
| A3 | 받은 상대가 거절 · 시한에서 포기 대기 · 백오프로 빠지고 세대 표가 안 줄어든다 | §3-1 · 3-0 U6 | D1 명세 보강 |
| A4 | 재연결 창에 넣은 명령이 다음 세대에 쓰인다(ADR-0195 위반) | §3-0 ③ · 3-0 U2 | — |
| A5 | 「앞선 것을 다 보낸 뒤 닫기」가 없다(정지 Ack · 오류 프레임 유실) | §3-0 ④ · 3-0 U3 | — |
| A6 | 거절이 전부 종결 — 토큰 불일치도 버튼 없이 앉는다 | §3-0 ⑤ · 3-0 U4 | **D20 신설** |
| B1 | `DaemonWire` In = `AgentCommand` 가 「위층 무변경」과 모순 · 나쁜 명령 처리 셋 소실 · 어휘가 `link/` 로 샌다 | §3-3 「DaemonWire 모양」 · 3-3 U4 · §2-4 | 메인 판단(D 밖) — 원문 통과 |
| B2 | `reply_tag` 가 하나라도 `Some` 이면 답장이 삼켜진다 · 답장이 앞선 사건을 앞지른다 | §3-3 · 3-2 U2 시험 | — |
| C1 · C2 · C8 · C10 | 수락 정책 필수 · 업그레이드 대기 상한과 셈 · loopback 단언 · dial 쪽 크기 | §5 · 3-0 U7 · 3-3 U5 | D9 범위 넓힘 |
| C3 | 인가 입구가 하나가 아니다(3-1 U4 뒤 WS dispatch 는 동사를 못 본다) | §3-4 | **D6 추천 바뀜** |
| C4 | 데몬끼리 인증의 신뢰 수준 · 자격 미정 | §3-3 「데몬끼리」 | D5 명세 |
| C5 | `daemon.json` ACL 은 미래가 아니라 지금 로컬 구멍일 수 있다 | §5 21 | **D22 신설** |
| C6 · C7 · C9 | 비밀이 `Debug` 로 샌다 · 상대가 준 문구 · 보안 로그 없음 | §5 · 3-0 U4 · U5 · U7 · 3-1 U2 | — |
| C11 | Origin 네 값이 틀렸을 가능성 · 토큰이 프론트로 간다 | §5 2 · 3-3 U5 | **D21 신설** |
| D1 | 고칠 ADR 을 링크가 아니라 조항으로 | §6-5 | — |
| D2 | 셸 시험을 두 구동에 다 돌리는 계획이 안 선다(시계) | §6-2 「셸 시험 운명」 · 3-2 U3 | — |
| D3 | 정지 클라 — 동기 호출 · 런타임 · 주 상대 재연결 | 3-2 U5 | — |
| D4 | 게이트 순서 · 빠진 자리 · CLAUDE.md 사본 리뷰 | §2-4 · 3-1 U1/U2 · 3-2 U2 · 3-3 U1/U2/U4 | — |
| D5 | D4 ↔ D11 묶임 | §8-3 묶음 A 머리 | — |
| 추가 1~10 | HTML 을 그린 워커의 일관성 지적 | 1 · 6 = §3-3 · 3-3 U7 / 2 · 10 = §2-4 / 3 = §3-5 / 4 = §5 12 · 14 / 5 = §3-6 · 3-3 U7 / 7 = §5 9 / 8 = §4 / 9 = §6-4 | — |

**단위 순서가 바뀐 것:** 3-0 신설(1판 3-2 U1 · 3-3 U3 를 흡수 — 그 번호는 비워 둔다) · `AuthFrame` golden 을 3-1 U1 → U2 · net gate 4 · 2b 삭제를 3-1 U2 · 3-3 U2 → 3-3 U6.

**반영 안 함 · 부분 반영**

| 지적 | 처리 | 사유(코드 대조) |
|---|---|---|
| C3 「진짜 단일 입구 = `call_daemon_command`」 | 부분 | 그 함수가 덮는 것은 명령 버스 배달 · `/control/agent` · `/control/call` 셋이다(`control/commands.rs:3-7`). 옛 `AgentCommand` 변형(Spawn · Kill · StopDaemon …)은 `connection_core` dispatch 가 직접 처리해(예: `connection_core.rs:1545-1548`) 그 함수를 안 지난다 — 입구는 둘이다(§3-4). |
| A1 「`registry.rs:6-9` · ADR-0180 결정 8 이 굶김을 금한다」 | 부분 | 그 헤더가 금하는 것은 남 탓에 조용한 상대를 **끊는 것**이고 유실은 문서화된 대가다(`transport/src/lib.rs:128-131`). 2판이 고치는 근거는 「유실이 두 소비자에게 받아들일 수 없다」(seq 연속 · 무답 명령)와 1판 규칙이 그 헤더가 막으려던 것(남의 홍수가 내 예산을 태운다)을 도로 만든다는 것이다. |
| D2 「tokio pause/advance 37곳」 | 수치 정정 | `grep -c` = 36줄(주석 포함 · `daemon_client/tests.rs`). 결론은 받는다. |
| C10 「인증 전 작은 상한(선택)」 | 연기 | 업그레이드 뒤 tungstenite 설정을 바꿀 수 있는지 확인하지 않았다(미확인). 16 MiB 단일 상한으로 구멍 5 는 닫힌다 → §4. |
| C11 「3-2 에서 `DaemonInfoDto` 토큰을 뺀다」 | 선택지로만 | 프론트에 그 토큰을 쓰는 곳이 하나 남아 있다 — `src/api/wsTransport.ts:273`(직결 경로 · 운영 아님). 빼면 그 경로가 죽는다 → D21 ① 의 일부로 묻는다. |

---

## 0. 결론 (먼저)

```
① 끝 상태: net 삭제. 연결 기계 = transport · 어휘(인증 프레임 포함) = protocol(잎 — 워크스페이스 의존 0) ·
   데몬 전용(판정 · 연결별 줄 · 단일 인스턴스 · daemon.json) = 데몬 link/ · 셸 끝자락 = 셸 daemon_client/link/.
② ★붙이기 전에 transport 를 고친다(2판 · D19)★ — 3-0 이 소비자 0 인 채로: 상대별 무손실 받는 줄(+ 포화 분류 훅) ·
   세대 도장 · 닫기 전 비우기 · 거절 분류 · Debug 가림 · 받는 쪽 seam(Listener + Registry::attach — 받은 상대는
   다시 걸지 않고 끝나면 스스로 내린다) · ws 수락 강화. 1판의 「유실 → 다시 붙기」 규칙은 걷었다.
③ 스트림 살림은 지금 자리 그대로 — 두 Wire 의 stream_mark = None. replay→live · gen 펜스 · 화신 표식 · seq 연속은
   데몬 · 셸 replay_flight · 프론트가 계속 진다. transport 의 스트림 층을 켜면 그 불변식이 깨진다(§2-5).
④ 다중 데몬 · 데몬끼리는 「관계마다 Wire · Registry · Listener 따로」 + 영속 데몬 신원으로 자리만 세운다. 인가는
   「누구냐(Principal)」만 지금 · 명령별 판정은 입구 둘을 명시하고 미룬다(D6). 원격 · 페어링 · TLS 는 빈 공간(§4).
⑤ 단위 3-0(7) → 3-1(5) → 3-2(5) → 3-3(6). 3-0 과 3-1 은 파일이 안 겹쳐 나란히 가도 된다.
   어느 커밋에서 멈춰도 빌드 · 회귀 · 게이트가 초록이다(§6 규칙).
```

**지금 하지 않는 것:** 원격 Link(TLS · iroh · VPN) · 페어링 · 중계(relay) · 화면 공유(조사 `screen-sharing-oss-2026-10-12.md` 결론 1 — transport 의 일이 아니다) · 셸 화면 · LLM 의 다중 데몬 표면 · 클라별 되감기 · 다중 클라 입력 권한(ADR-0179 영향 2 · 3) · 명령별 인가 정책 · 데몬끼리 중계 어휘 · 제어 평면 HTTP(`/control/*` · `/mcp`)의 강화.

---

## 1. 맥락 · 범위

### 1-1. 왜

- transport 는 독립 crate 로 서서 단독 검증까지 끝났고 소비자가 0 이다(transport TRD §10 「셸을 이 crate 위로 이사 = 붙이는 순서 ②」). 셸은 손으로 짠 WS 클라이언트(`src-tauri/src/daemon_client/connection.rs` 2,050줄 + 동기 정지 클라 `stop.rs` 370줄)를, 데몬은 net 의 서버 행(`crates/engram-dashboard-net/src/ws.rs` 2,248줄)을 쓴다. 같은 일(핸드셰이크 · keepalive · 재연결 · 짝 맞추기)이 세 벌이다.
- net 은 셋을 섞는다 — 데몬 전용 서버 행 · 클라도 쓰는 인증 프레임(`net/src/auth.rs:33`) · 데몬 살림(`instance.rs` · `portfile.rs`). 셸이 데몬 쪽 crate 를 아는 것을 기능 플래그(`net/Cargo.toml` `server`)로 겨우 막는다(메모 §6 사실).
- `protocol` 은 command 의 구조체 넷을 품는다(`protocol/src/messages.rs:4`) — ADR-0272 가 걷기로 했고 착수를 이 단계로 미뤘다(결정 7).

### 1-2. 범위 안

| 단계 | 내용 |
|---|---|
| 3-0 | transport 보강(D19) — 상대별 무손실 받는 줄 · 생애 사건 · 세대 도장 · 보냄 칸 · 닫기 전 비우기 · 거절 분류 · Debug 가림 · 받는 쪽 seam · ws 수락 강화(소비자 0 인 채로) |
| 3-1 | ADR-0272 실행(명령 칸 넷 = `Box<RawValue>` · `OwnerToken` 처분 · 짝 맞추기 함수를 끝자락으로) · `AuthFrame` 을 protocol 로 · protocol 의존 0 게이트 · 셸의 net 의존 삭제 |
| 3-2 | 셸 끝자락 어댑터(Wire · 핸드셰이크 · 풀이 Dialer · 사건 사상) · 접속 정보 인터페이스 · 정지 명령 클라이언트를 transport 위로 · 셸 어댑터의 상대 N 자리 |
| 3-3 | 데몬 끝자락 어댑터 · net 의 방어 옮기기와 새 방어 · `instance` · `portfile` 를 데몬으로(열기 재시도 = platform `retry_busy`) · net 삭제 · 데몬 신원 · 데몬끼리 뼈대 |

### 1-3. 범위 밖 — 나중 트랙(§4 「빈 공간」이 채우는 자리)

원격 Link(rustls 키 고정 · iroh · 사용자 VPN) · 페어링 · 원격 주소 발견 · 데몬끼리 어휘(우편 중계 · 목록 합치기) · 되돌아옴 방지 · 셸 화면/LLM 의 데몬 고르기 · 명령별 인가 정책 · 인증 실패 속도 제한 · 화면 공유. `daemon.json` ACL 은 1판의 「연기」에서 결정으로 올렸다(D22).

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
| transport `ws.rs:192` `WsListener` | 받는 쪽 구체 타입(무검사 `accept` — `:217`) | `Listener` 구현 + 수락 정책 필수(무검사 경로 삭제) | 3-0 |

### 2-3. 지우는 것

net crate 전체(6 파일 3,601줄 · 워크스페이스 멤버 · `Cargo.lock` 항목) · 셸 `tungstenite = "0.26"`(`src-tauri/Cargo.toml:98`) · 셸 `tokio-tungstenite` 운영 의존(→ dev — 시험의 가짜 서버가 쓴다) · 셸 `connection.rs` 의 핸드셰이크 · 재연결 · pending 부분 · protocol → command 간선 · net 게이트 1~5 와 그 사본(CLAUDE.md 「네트워크 행 격리 게이트」 · `.claude/skill-bindings/qa.md:280-285` · `docs/testing-strategy.md` §net).

### 2-4. 게이트 — 걷는 것과 대신 세우는 것

「걷는 때」 = 그 게이트가 CI 에서 사라지거나 기대값이 바뀌는 단위 · 「세우는 때」 = 대신 세우는 게이트가 처음 도는 단위. ★net 은 3-3 U6 까지 살아 있으므로 net 을 겨누는 게이트는 그때까지 남긴다★(1판은 gate 4 · 2b 를 먼저 걷었다).

| 걷는 것(`ci.yml`) | 걷는 때 | 대신 세우는 것 | 세우는 때 |
|---|---|---|---|
| net gate 1 — net 이 daemon · messaging 을 안 부른다(`:1140`) | 3-3 U6 | **끝자락 어휘 게이트 ①** — `test -d crates/engram-dashboard-daemon/src/link && rg "engram_dashboard_(agent\|messaging)" crates/engram-dashboard-daemon/src/link/` → 0(ADR-0129 「네트워크 행은 에이전트 어휘를 모른다」를 모듈로 잇는다) · 셸 짝 `src-tauri/src/daemon_client/link/` 에 `tauri::` · `engram_dashboard_agent` 0 | 3-3 U4 · 3-2 U2 |
| net gate 2a — platform 심볼 정확히 다섯(`:1156`) | 3-3 U2 기대값 5 → 0(net 의 platform 간선이 끊긴다) · 3-3 U6 삭제 | 없음 — 그 다섯은 `instance` · `portfile` 와 함께 데몬으로 간다 | — |
| net gate 2b — 에이전트 런타임 재유입 0(`:1192`) | **3-3 U6**(2b 는 platform 축이 아니다 · 2a 와 함께 걷어 CLAUDE.md 「하나만 남기지 말 것」을 지킨다) | 끝자락 어휘 게이트 ① | 3-3 U4 |
| net gate 3 — 의존 상한 정확히 3(`:1210`) | 3-3 U2 기대값 3 → 2 · 3-3 U6 삭제 | transport gate 1(이미) + **protocol 의존 상한 = 정확히 1줄**(ADR-0272 영향의 「의존 0 게이트」) | 3-1 U5 |
| net gate 4 — `(A)gentCommand\|(P)ROTOCOL_VERSION` 0(`:1231`) | **3-3 U6**(3-1 U2 뒤 net `auth.rs` 는 재수출뿐이라 그대로 초록) | **끝자락 어휘 게이트 ②** — 같은 패턴을 `crates/engram-dashboard-daemon/src/link/` 에 → 0(§3-3 「DaemonWire 모양」 — `link/` 는 어휘를 공장 콜백으로만 받는다) + `AuthFrame` golden 을 protocol 로 | 3-3 U4 · 3-1 U2 |
| net gate 5a · 5b(`:361` · `:365`) | 3-3 U6 | 없음(transport 4a~4d) | — |
| platform gate 7 의 net 두 줄(`:1281-1282`) | 3-3 U6 | 「1줄 이상」 짝을 `-p engram-dashboard-transport`(tokio 가 무조건 의존)로 | 3-3 U6 |
| messaging 이름 게이트의 `net`(`:538`) | 3-3 U6 | 이름에서 뺀다(사본 다섯 — 메모 §11 의 같은 처리) | 3-3 U6 |
| platform 4f · 4h 명단의 `net/src/instance.rs` · `portfile.rs`(`:924-925` · `:1023`) | 3-3 U2 | 새 경로로 | 3-3 U2 |
| — | — | 셸 운영 그래프에 net 0줄 + 짝(데몬 ≥ 1줄 — 2-1 D6 꼴) | 3-1 U2(3-3 U6 에서 걷음) |
| — | — | 셸 직접 normal 의존에 `tungstenite` · `tokio-tungstenite` 0줄(`cargo tree -p engram-dashboard -e normal --depth 1`) | 3-2 U5 |
| — | — | **transport `test-support` 운영 그래프 게이트** — base · platform 줄과 같은 꼴(데몬 · 셸 · cli 의 normal 그래프에 0줄 + 짝 = 셸 dev ≥ 1줄). ★`-i engram-dashboard-transport` 꼴은 대상이 그래프에 없으면 rc=101 로 죽는다(CLAUDE.md 4i 주의) — 3-0 엔 소비자 그래프에 transport 가 없으므로 붙는 단위에서 세운다★ | 3-2 U2(셸) · 3-3 U4(데몬 · cli) |

★게이트 사본(CLAUDE.md 「빌드·검증 명령」 · qa 바인딩 · testing-strategy · crate 헤더)을 바꾸는 단위(3-1 U2 · U5 · 3-2 U2 · U5 · 3-3 U2 · U4 · U6)는 그 커밋에서 `/review doc` 을 거친다★ — CLAUDE.md 는 load-bearing 문서다(「구현 실행 규약」).

### 2-5. 스트림 살림 — 누가 무엇을 지나 (replay→live 화해)

| 일 | 지금 | 3단계 뒤(제안 — D10 ①) |
|---|---|---|
| 링 · seq 찍기 | 데몬 `OutputCore` | 그대로 |
| replay→live 순서 | subscribers lock 안에서 replay 를 연결당 FIFO 큐(`net/src/ws.rs:52` 4608칸)에 `try_send` | 같은 lock 안 `FrameSink::try_send` → `Peer::notify`(await 없음 — `transport/src/peer.rs:248`) → 감독 단일 writer FIFO. 같은 성질 |
| 화신 표식(epoch) 거름 | 셸 `frame_relay.rs` | 그대로 — transport `StreamMark.generation` 은 쓰지 않는다 |
| single-flight Subscribe · gen 펜스 마커 | 셸 `replay_flight.rs` | 그대로 |
| seq 연속(dedup · 구멍 붙듦) | 프론트 뷰(`HELD_MAX_*`) | 그대로 |
| transport 스트림 층(`stream.rs`) | — | **안 쓴다** — `ShellWire` · `DaemonWire` 의 `stream_mark` = `None` · `resume_stream` 을 부르지 않는다 |
| 받는 줄 포화 | 없음(셸은 읽기 루프에서 Channel 로 직행) | ★유실을 없앤다(3-0 U1 · D11 ①)★ — 상대별 무손실 줄. 셸 줄이 차면 그 상대의 소켓 읽기만 서고(TCP 배압) 데몬 나가는 큐가 차면 D13 규칙(끊고 전량 replay)으로 푼다 — seq 를 건너뛰는 길이 없다. 1판의 「`Dropped{Inbound}` → `reconnect_now`」는 걷었다(§3-0 ①) |

**스트림 층을 켜면 깨지는 셋(확실 — 코드 판독):**
1. **셸은 무상태 통과 라우터라 같은 에이전트의 replay 를 창마다 전량 다시 받는다**(`replay_flight.rs:3-7`). transport 의 「`seq <= last` 면 버린다」(transport TRD §4-3 1)가 셸 한 곳에서 돌면 새로 연 창의 replay 가 통째로 버려진다.
2. **구멍을 보면 transport 가 `resume_request` 로 Subscribe 를 스스로 낸다**(`wire.rs` `resume_request` doc) — `replay_flight.rs:36-43` 「추적을 잃은 채 wire 로 나간 Subscribe 를 만드는 진입점을 두지 않는다 — 있으면 gen 펜스 붕괴」를 정면으로 어긴다.
3. **계약이 다르다** — transport 는 재요청 뒤 `resume_timeout`(2초 — `policy.rs:208`)이 지나면 잘림을 신고하고 붙든 것을 내보낸다 · 프론트는 구멍 뒤를 「시한 없이」 붙든다(CLAUDE.md 「replay→live」).

★「sink 와 셸 중계는 seq 를 건너뛰지 않는다」(CLAUDE.md 「핵심 불변식」)는 transport 의 손실형 공유 줄이 새로 깰 수 있던 유일한 자리다★ — 2판은 다시 붙기로 메우지 않고 유실 자체를 없앤다(R1).

---

## 3. 지금 고정할 seam (안 · 대가 · 권고, 미결정)

### 3-0. transport 보강 — 붙이기 전에 고칠 것 (D19 · D11 · D20 — 2판 신설)

1판은 transport 표면을 거의 그대로 쓴다고 봤다. 리뷰가 짚은 다섯을 코드에서 확인했고(정적 판독) 어느 것도 어댑터로 메울 수 없다 — 유실 · 순서가 crate 안에서 이미 정해진다.

① **공유 받는 줄이 손실형이다(A1 · D11).** 감독은 메시지도 사건도 한 줄에 `try_send` 하고 차면 세기만 한다(`peer.rs:1224-1246` · `note_inbound_drop :1254`). 1판의 대책(`Dropped` → `reconnect_now`)이 만들거나 남기는 것:
- **셸 replay 고리** — replay 한 번(링 4096)이 512칸 줄을 넘기면 유실 → 다시 붙기 → 다시 replay. `ReconnectNow` 는 예산(`retries` · `flaps`)을 0 으로 되돌려(`machine.rs:272-292`) 그 고리는 포기로도 안 끝난다. 백오프 중 `ConnectFailed` 사건이 유실돼도 같은 규칙이 예산을 되감아 죽은 데몬을 영영 포기하지 못한다.
- **굶김** — 셸 명부에 데몬이 N 이면 시끄러운 데몬이 줄을 채워 조용한 데몬의 메시지가 사라지고, 1판 규칙이 그 조용한 쪽을 다시 붙인다 — `registry.rs:6-9` 가 막으려던 「남의 홍수가 내 예산을 태운다」.
- **데몬 쪽 무답 소실** — 줄에서 사라진 명령에 `Refused` 답이 없다. frame_port 구현 의무 2(`net/src/frame_port.rs:195-199`)와 net 의 「조용히 버리지 않는다」가 깨지고 보낸 쪽은 마감까지 매달린다.
- **신고 지연** — `Dropped` 는 감독이 깰 때만 나간다(`flush_drop_reports` — `peer.rs:612` · `:647` · `:688`). Live 에서 조용하면 keepalive 주기(20 s)까지, 포기 대기(`park`) 중이면 제어가 올 때까지다.

선택지(D11 — 2판에서 다시 썼다):
- **① 상대별 유계 줄 + 배압 + 포화 분류 훅(2판 권고)** — 감독은 자기 줄의 자리를 기다리는 동안에도 제어 · keepalive · 쓰기를 계속 돌리고(select 한 갈래), 자리가 없으면 그 상대의 소켓 읽기만 선다(TCP 배압). `Inbound::recv` 는 상대 줄을 돌아가며 합친다. 줄이 찼을 때 소비자가 고르는 동기 훅 `on_saturated(&PeerId, &W::In) -> Wait | Refuse | Bypass`(기본 `Wait`) — net `on_inbound_saturated`(`frame_port.rs:186-207`) 계약을 crate 의 seam 으로 옮긴 것이고 데몬은 지금 의미(`Refused` 답 · 취소 우선)를 그대로 구현한다. 대가: `Inbound` 표면 변경 · 「지금 확정」 다시 열기(R9).
- ② 상대별 줄 + 배압만(훅 없음) — 줄이 찬 동안 뒤에 온 취소가 앞을 못 지른다(net 의 bypass 소실 — `net/src/ws.rs:799-900`).
- ③ 공유 줄을 await — 무손실이지만 한 상대가 막으면 전부 선다(ADR-0180 결정 8).
- ④ 1판 — 손실형 그대로 + 어댑터 대처(위 넷이 남는다).
- 1판 추천(① 어댑터가 공유 줄을 즉시 비운다) → 2판 추천 ①. 사유: 즉시 비우기는 소비자 태스크가 밀리는 순간 유실로 돌아가고, 그 유실을 셸이 다시 붙기로 메우는 것이 위 고리다.

② **생애 사건도 그 줄을 탄다(A2).** `Connected` · `Disconnected` · `Closed` 가 `emit` → `try_send`(`peer.rs:1224`). 명세: 사건은 ① 의 상대별 줄에 메시지와 같은 순서로 실린다(무손실) · 어댑터는 상대마다 상태 기계를 두고 끝은 `Peer::state()` watch(`peer.rs:210`)로도 확인한다 · `on_disconnect` 는 ConnId 마다 정확히 한 번 · 이미 닫은 ConnId 앞으로 온 메시지는 버린다 · 셈(연결 상한)은 사건이 아니라 RAII 가드로(§5 14).

③ **재연결 창의 낡은 명령(A4).** `notify` · `request` 는 `is_live` 를 본 뒤 큐에 넣는다(`peer.rs:226-252`). `ReconnectNow` 는 그 순간의 큐만 비우고 곧장 dial 하며(`machine.rs:281-292` · `fail_pending peer.rs:1151-1162`), dial · 핸드셰이크(`peer.rs:534-590`)는 큐를 안 읽는다 → 그 창에 들어온 것이 세대 G+1 에 쓰인다. ADR-0195 와 셸의 지킴이(`connection.rs:1514-1547` `reject_foreign_command`)가 깨진다. 명세: 넣을 때 본 세대를 `PeerCmd` 에 도장으로 찍고 감독은 쓰기 직전 지금 세대와 다르면 「안 보냄」으로 깨운다(Connected 때 큐를 비우는 안보다 창이 0). 셸 시험 둘(`tests.rs:3029` · `:3505`)을 transport 시험으로 옮긴다.

④ **닫기 전 비우기가 없다(A5).** 데몬은 `StopDaemon` Ack 를 넣고 닫는다(`connection_core.rs:1545-1548`) · binary 는 Error 프레임 뒤 닫는다(`agent_conn.rs:384-389`) — frame_port 의 `Frame::Close` 는 앞선 프레임 뒤에 선다(FIFO · `agent_conn.rs:181`). transport `Peer::close` 는 제어 줄로 가 매 바퀴 먼저 들리고(`peer.rs:282-283` · `:692`) 정리가 큐를 거절한다(`teardown :1177-1185`) → 그 Ack · Error 가 사라진다. 명세: `Peer::close_after_flush(code, reason)` — 데이터 줄에 닫기 표지를 넣어 앞선 것을 다 쓴 뒤 닫는다(상한 = 쓰기 시한).

⑤ **거절이 전부 종결이다(A6 · D20 · C7).** 핸드셰이크 `Reject` → `GaveUp` 고정(`machine.rs:361-368`). 1판 셸 핸드셰이크는 데몬 Error 를 전부 Reject 로 보내 토큰 불일치(「auth failed」 — `net/src/ws.rs:340-345`)도 종결이 된다. 지금 셸은 그것을 실패 한 번으로 세고 다음 시도에 daemon.json 을 다시 읽는다(`connection.rs:813-850` · `:891-894`). daemon.json 은 제자리에 다시 쓰이므로(ADR-0135 — 부분 내용이 보인다) 낡은 자격이 거절당할 길이 있고, 1판이면 그 뒤 「다시 연결」 버튼(D14 — 아직 없음)도 없이 앉는다. 명세: `Reject` 에 종류 칸 — `Retryable`(예산을 쓰고 다음 dial 이 접속 정보를 다시 푼다) · `Terminal`(버전 불일치 — ADR-0180 결정 5 그대로). 함께, 상대가 준 문구를 믿지 않는다: `DecodeFailed.reason` 은 `err.to_string()`(`peer.rs:911` — 값이 섞일 수 있다) 대신 분류 문자열 · `Rejected.reason`(팝업 문안 — `event.rs:170`)은 길이 상한 · 제어 문자 제거 · LLM 표면에는 「상대가 준 데이터」로 표시.

**자리(D19):** ① 새 단계 3-0 에 전부(권고 — 소비자 0 인 지금 고치면 깨지는 쪽이 없다 · 「지금 확정」 표면을 한 번만 다시 연다 · 3-1 과 파일이 안 겹쳐 나란히 간다 · 1판 3-2 U1 · 3-3 U3 를 흡수) ② 3-0 은 셸이 쓰는 것(①~⑤)만 · 받는 쪽 seam · ws 수락 강화는 1판대로 3-3 U3(그 설계를 쓰는 데몬 어댑터 바로 앞에서 · 표면을 두 번 연다) ③ 1판대로 각 단계 안(3-2 가 손실형 줄 위에서 시작한다 — 기각 사유 = 위 ①).

### 3-1. 받는 쪽 seam · 받은 상대 (D1)

- **사실:** `link.rs` 에 `Listener` 가 없고 `Registry` 는 들어오는 연결을 모른다(`transport/src/lib.rs:142-145`) · `WsListener::accept` 가 TCP 수락과 WS 업그레이드를 한 자리에서 해 느린 상대 하나가 다른 수락을 막는다(`ws.rs:29-30` · `:217-227`) · 재연결은 건 상대에게만 뜻이 있다 · `Handshake` 는 이미 방향 대칭(`link.rs:150-158`) · ★지금 기계는 핸드셰이크 중 거절을 `GaveUp` 으로 앉히고(`machine.rs:361-368` — `park` 가 제어를 기다린다) 시한 · 통로 실패를 Backoff → 명부의 Dialer 로 다시 건다(`:375-380`)★ · 세대 표는 지우지 않는 map 이다(`registry.rs:58-63` — 「소비자가 쓴 이름표 수만큼 자란다」).
- **① `Listener` trait + `Registry::attach`(권고)** — `Listener::accept() -> Accepted{tx, rx, info: LinkInfo}`(`LinkInfo` = 원격 주소 · Origin · 나중 TLS 상대 키 칸) · `Registry::attach(id, accepted, handshake, policy) -> Peer`. 받은 상대의 명세(A3):
  - Dialing 을 건너뛰고 Handshaking 에서 시작 · ★Live 가 아닌 모든 끝(거절 · 핸드셰이크 시한 · 통로 손실 · Live 뒤 손실)은 `Closed` + 명부에서 스스로 내림★ — Backoff · `GaveUp` 대기 없음 · `reconnect_now` 무시.
  - ★받은 상대의 이름표는 세대 표에 안 든다★ — 연결마다 새 이름표(`conn-<ConnId>`)라 넣으면 연결 수만큼 영원히 자란다. 받은 상대는 세대 1 에서 시작하는 자기 계수기를 쓴다(ConnId 는 단조라 같은 이름표가 다시 오지 않는다).
  - 시험: 인증 실패 N 번 뒤에도 진짜 셸이 붙는다 · 명부 · 세대 표 크기가 실패 수와 무관.
  - 수락 루프는 소비자(데몬)가 돌린다 — 상한 · 속도 정책이 소비자 몫(ADR-0177 결정 4). `Registry::new` 시그니처 그대로. 대가: 기계에 「받은 상대」 모드 하나 · 수락 루프가 소비자마다 한 벌.
- ② `Registry::new(.., listeners)` — 수락까지 crate 안. transport TRD §12-7 이 예고한 시그니처 변경이고 상한 · 속도를 crate 정책 값으로 들여야 한다.
- ③ 데몬이 Registry 밖에서 Link 를 직접 돌린다 — ADR-0179 결정 3(같은 명부 · 같은 감독) 위반.
- `Handshake::start(&self)` 에 문맥 인자가 없어도 된다 — 받는 쪽은 attach 마다 핸드셰이크 값을 새로 만들어 `LinkInfo` 를 그 안에 담고, `admit` 이 낸 `Principal` 도 그 값 안에 **동기로** 적는다(§3-4). trait 은 안 바뀐다.

### 3-2. 상대 신원 (D3)

- **사실:** `PeerId` = 소비자가 붙이는 불투명 문자열(`transport/src/event.rs:19-23` — 「나중에 화면에 그대로 뜬다」) · 데몬에는 영속 신원이 없다(`Hello` = 버전 · 능력뿐 — `protocol/src/messages.rs:433-438` · `DaemonInfo` = pid · 주소 · 토큰 — `protocol/src/discovery.rs`) · 토큰 = 데이터 폴더를 읽는 누구나의 공유 비밀(ADR-0179 영향 4).
- **① 영속 `DaemonId` + `Hello` 의 덧붙임 칸(권고)** — 데몬 상태 폴더에 처음 한 번 UUID v4 를 만들어 둔다(데몬 `DataLayout` 이 자리 소유) · `Hello{.., daemon_id: Option<String>}`(`#[serde(default)]` — 옛 셸 호환 · `PROTOCOL_VERSION` 그대로 · TS 바인딩 한 칸) · `PeerId` 는 이름표로 남긴다(셸 = 접속 정보의 키 · 데몬이 받은 연결 = `conn-<ConnId>`) · 키는 나중에 **신원에 묶는다**(신원 ≠ 키). 대가: 데이터 폴더 파일 하나 · 그 파일이 깨지거나 지워지면 새 신원(base `file` 규칙) — ★그때 데몬은 경고 로그를 크게 남긴다(C4)★.
- ② 키 쌍을 지금 만들고 지문 = 신원 — 지금 쓰지 않는 암호 의존을 들이고, 키 회전이 신원 회전이 된다.
- ③ 이름표만 — 셸이 같은 데몬을 두 주소로 보거나 데몬끼리 서로를 가리킬 재료가 없다.
- **대조(추가 5):** 셸 `ShellHandshake` 가 Hello 에서 `Endpoint.expected_daemon_id` 와 `Hello.daemon_id` 를 비교한다 — 기대가 `None` 이면 안 본다 · 둘 다 있고 다르면 `Reject{Terminal}`(「다른 데몬」 — 팝업 문안). 로컬 구현은 3단계에서 늘 `None` 이다(daemon.json 이 정본이고 거기에 id 칸이 없다) → 대조는 인메모리 시험으로만 선다. 칸과 대조는 `Hello.daemon_id` 와 함께 3-3 U7 에 든다(1판은 3-2 에 칸만 두어 U7 까지 죽은 칸이었다).

### 3-3. Wire 를 관계마다 가르나 (D2)

- **사실:** `Wire` 는 `Out`/`In` 이 갈린 제네릭이고 `decode` 에 상대 인자가 없다(`wire.rs:20-60`) · `Registry<W>` 는 Wire 하나 · ★셸↔데몬 링크는 이미 양방향 요청이다★ — 데몬→셸 `AgentEvent::CommandRequest{envelope}`(`messages.rs:608`)에 셸이 `AgentCommand::CommandOutcome{reply}`(`:333`)로 답하고, 그 짝은 데몬 `command_delivery` 가 기한 · 자리 표로 진다(transport pending 이 아니다).
- **① 관계마다 Wire · Registry · Listener 따로(권고)** — 셸↔데몬 = `ShellWire`/`DaemonWire`(지금 어휘) · 데몬↔데몬 = `PeerWire`(새 어휘 — protocol `peer` 모듈 뼈대). 각 Wire 는 자기 관계 안에서 양방향이다. 대가: 관계마다 Inbound 하나 · 데몬끼리 Listener 의 실물(둘째 포트 · `daemon.json` 칸)은 **D5 ② 일 때만** 3단계에 든다(기본 꺼짐) — D5 ① 이면 데몬끼리는 인메모리뿐이라 포트는 하나다. 원격 때 바인드 주소 · TLS 가 관계마다 달라지기 쉬워 갈라 두는 쪽이 자연스럽다.
- ② 합집합 Wire 하나 + `AuthFrame` 에 역할 칸 — 패킷 이름 공간이 겹치면 `decode` 가 못 가른다 · 셸이 데몬끼리 어휘를 타입으로 안다.
- ③ 한 포트 + 수락 뒤 첫 프레임 엿보기로 Registry 고름 — 핸드셰이크가 다시 읽게 프레임을 되돌려 놓는 Link 감싸기가 든다.

**DaemonWire 모양(B1 · B2 — 메인 판단 가능, D 목록 밖):**
- 1판은 In = `AgentCommand` 로 Wire 가 해독하게 했다. 그러면 ① 위층 계약(`on_text(&str)` · `on_inbound_saturated(&str)` · `Frame::Text(String)` · `broadcast_text` — `net/src/frame_port.rs:79` · `:170` · `:203`)과 「위층 무변경」이 모순이고 ② 기본 해독 실패 정책(`DropFrame`)이 지금 동작 셋을 조용히 버린다 — 깨진 명령에 Error 답 + 연결 유지(`agent_conn.rs:351-372` · `ws_e2e` case21 `:1447`) · 두 번째 Auth 에 「already authenticated」 · binary 에 Error + 닫기(`:384-389`) ③ `AgentCommand` 어휘가 `link/` 로 들어와 net gate 4 의 축이 사라진다.
- **제안:** 원문 통과 — In = `RawIn{Text(String) | Binary(Vec<u8>)}`(해독 실패 0 · 바이트 길이를 같이 실어 `CONN_RX_MAX_BYTES` 회계를 잇는다) · 해독은 지금처럼 `agent_conn` · `link/` 는 어휘를 공장 콜백(net `handshake_error_frame` 꼴 — `frame_port.rs:241`)으로만 받는다 → 끝자락 어휘 게이트 ②(§2-4).
- ★`DaemonWire::request_tag` · `reply_tag` 는 모든 입력에 `None`★ — `reply_tag` 가 하나라도 `Some` 이면(예: 셸의 `CommandOutcome`) `route()` 가 대기 표에서 못 찾고 `UnmatchedReply` 로 삼킨다(`peer.rs:929-950`). 원문 통과면 구조로 성립한다.
- ★셸 쪽 순서 변경(B2)★ — `ShellWire::reply_tag` 가 `Some` 인 답장은 받는 줄을 건너 대기자를 바로 깨운다(`peer.rs:938-942`). 지금 셸 읽기 루프는 프레임 순서대로 사건을 내고 답장을 푼다 — 붙인 뒤엔 답장 직전에 온 사건(예: 목록 갱신)이 답장보다 늦게 위층에 보일 수 있다. 3-2 U2 에 그 순서를 재는 시험을 두고, 그 순서에 기대는 호출자를 찾아 적는다(미확인 — 아직 안 찾았다).

**데몬끼리(D5 와 함께 — C4 · 추가 6):**
- **신뢰:** 3단계의 `PeerDaemon(DaemonId)` 는 **주장된 · 검증 안 된** id 이고 신뢰 수준은 `LocalClient` 와 같다. 자격 = 받는 데몬의 `daemon.json` 토큰(같은 PC 데이터 폴더를 읽을 수 있음 = 지금 셸과 같은 믿음). ★`DaemonId` 를 키로 한 영속 신뢰(허용 목록 등)를 만들지 않는다 — 페어링으로 키에 묶기 전까지★. 각 Listener 는 다른 관계의 인증 프레임(셸 `Auth` ↔ `PeerAuth`)을 거절한다.
- **거는 방향:** 3단계엔 자동 발견이 없다 — 설정(시험 하네스)이 「누가 누구에게 건다」를 정하고 한쪽만 건다. 양쪽이 동시에 걸면 `DaemonId` 가 작은 쪽이 건 연결을 남긴다(가안 · 대칭 깨기 — D5 ② 일 때만 실물 시험).
- **되돌아옴:** 3단계 어휘(`Identify` · `Ping`)는 남에게 넘기는 것이 없어 고리가 생길 수 없다 — 중계 어휘가 드는 날의 전제로 §4 에 둔다.

### 3-4. 인가 seam (D6 · D7)

- **사실:** 토큰 하나 = 전권 — 첫 프레임 판정(`net/src/ws.rs:331-360`) 뒤 명령마다 검사가 없다 · ★명령 입구가 둘이다★ — 명령 버스 배달 · `/control/agent` · `/control/call` 은 `call_daemon_command` 한 곳(`control/commands.rs:3-7` · ADR-0157 검문)으로 들어오고, 옛 `AgentCommand` 변형(Spawn · Kill · StopDaemon …)은 `connection_core` dispatch 가 직접 친다 · 3-1 U4 뒤 WS 의 `Command` 봉투는 원문(`Box<RawValue>`)이라 WS dispatch 에서는 동사가 안 보인다 · `/mcp` 는 별도 서버 · 인증 세 조각(프레임 모양 · 판정 · 비밀값 나누기)이 net 한 곳에 섞여 있다(메모 §6 열린 질문).
- ① `Principal` + `admit` + `allow` 를 입구 둘에 — `call_daemon_command` 에 `Principal` 인자(제어 라우트는 `LocalClient` 고정) · `connection_core` dispatch 에 하나 · 3단계 구현은 전부 허용. 대가: ADR-0157 입구와 그 호출부(배달 1단계 · 라우트 둘) 시그니처가 지금 바뀐다 — 늘 허용하는 판정 하나를 위해.
- **②' `admit` + `Principal` 보관만(2판 권고)** — 연결마다 `Principal`(`LocalClient` · `PeerDaemon(DaemonId)` · 나중 `Paired`)을 `admit(attempt, &LinkInfo) -> Result<Principal, Refusal>` 이 그 attach 의 핸드셰이크 값 안에 동기로 적고, 어댑터가 Live 때 ConnId 옆 표로 옮긴다 · ★없으면 거부 · 닫기★ · 명령별 `allow` 는 짓지 않고 **입구 둘을 이 TRD 와 ADR(E)에 이름으로 박는다** — 정책이 드는 날 두 곳에 꽂는다.
- ③ 없음.
- **1판 추천 ① → 2판 추천 ②'.** 사유: 1판 ① 이 기대던 「dispatch 입구 한 곳」이 코드에 없다. 둘에 꽂는 새 ① 은 늘 허용하는 판정 하나를 위해 ADR-0157 입구를 지금 연다 — 입구 이름만 박으면 나중 비용이 같다(CLAUDE.md 「판단 기준」 위험도 × 기간).
- **자리(D7):** ① 데몬 `link/auth.rs` 한 모듈 — 판정 trait + 지금 구현(토큰 · 상수시간 비교 · 버전) · 프레임 모양은 protocol(권고 — 메모 §6 메인 의견) ② 인증 crate — 소비자가 데몬 하나뿐 ③ transport 안 — ADR-0177 결정 4(인증 자료는 밖) 위반.

### 3-5. Link 교체 자리 — TLS · iroh (3단계 결정 아님)

- **사실:** 전송 · 시간이 dyn(`link.rs:99-155`) · `Registry` 는 Dialer 를 하나 받는다(`registry.rs:80`) · `WsDialer` 는 tungstenite 기본 feature 라 `wss://` 가 dial 에서 실패한다(`ws.rs:33-34`) · TLS 는 바이트 스트림 **아래**에, iroh 는 QUIC 스트림 위에 메시지 경계를 따로 세워야 한다(보안 조사 §3-2 · §5 D).
- 방향(가안): 주소 scheme 으로 가르는 합성 Dialer + 관계별 Listener — `ws://`(3단계) · 나중 `wss://`(ws 어댑터 안 스트림 층 — tokio-rustls · 키 고정) · `iroh://`(새 구현). 상대 키는 `LinkInfo` 칸으로 핸드셰이크 · `admit` 에 올라간다. 대안 = 상대별 Dialer(`Registry::add` 시그니처 변경) · 프레임 위 암호(Noise 손구현 — 보안 조사 §5 E 암호 자작 금기에 가깝다).
- ★D 번호가 없는 이유(추가 3)★ — 3단계는 `ws://` 하나와 셸의 논리 주소 풀이 Dialer(§3-6)만 짓는다. 셋 중 고르는 일은 원격 트랙 TRD 의 몫이고, 3단계가 미리 박는 것은 `LinkInfo` 의 상대 키 칸 하나다(§4) — 어느 쪽을 골라도 그 칸은 같다.
- **Origin · Host 검사는 WS 업그레이드(HTTP)에서만 보인다** — 그래서 ws 어댑터 수락 설정에 산다(D8 ① · 값은 소비자가 준다 · 값 자체는 D21). 데몬이 자기 수락을 짜고 `transport::ws::split_link`(`ws.rs:237` 공개)로 Link 를 만드는 ② 도 된다 — 대가 = 수락 강화가 transport 시험망 밖에 선다.

### 3-6. 접속 정보 인터페이스 (3-2 범위 — 메인 판단 가능, D 목록 밖)

- **사실:** `DaemonDiscovery`(`daemon_client/mod.rs:105-115`) = `ensure_spawn`(띄울 수 있음) · `read_live`(띄우지 않음 — ADR-0021) → `DaemonInfo`(토큰 포함) · 재연결마다 daemon.json 을 다시 읽어 옮겨 간 포트를 따라간다(`connection.rs:813-850`) · transport 는 주소와 핸드셰이크를 `add` 때 고정하고 시도마다 같은 `dialer.dial(&addr)` → `handshake.start()` 를 **한 태스크에서 차례로** 부른다(`peer.rs:540` · `:1324` · `:1342`).
- **제안:** `EndpointSource` — `ensure() -> Result<Endpoint>` · `current() -> Option<Endpoint>` · `Endpoint{key, address, credential: Secret}`(`Secret` = `Debug` · `Display` 가림 · `serde(transparent)` — `AuthFrame` · `DaemonInfo` 와 같은 타입, §5 5) · 3-3 U7 이 `expected_daemon_id: Option<DaemonId>` 를 더한다(§3-2) · 운영 구현 = 로컬 데이터 폴더. `Address` = 논리 주소(가안 `engram-local:<data_dir>` — transport 는 주소를 해석하지 않는다는 계약 그대로). 셸 `LocalDaemonDialer` 가 dial 마다 `current()` 로 실주소를 풀고(없으면 마지막으로 안 주소 — 지금 동작), 풀어 낸 자격을 **같은 상대의 핸드셰이크와 공유하는 칸**에 둔다 — `Reject{Retryable}`(§3-0 ⑤) 뒤 다음 dial 이 그 칸을 새로 채운다. 처음 연결만 `ensure()`(띄울 수 있음)를 `Registry::add` **밖**에서 부른다 — 재연결은 dialer 안 `current()` 뿐이라 ADR-0021 이 구조로 선다.
- **거른 안:** transport 를 넓혀 `Dialer` 가 문맥을 돌려주고 `Handshake::start(ctx)` 가 받는다 — 「지금 확정」 trait 변경 · daemon.json 감시로 `Registry::add` 를 갈아끼운다 — 세대 · 예산이 끊긴다.
- **원격 대비:** 같은 trait 의 다른 구현(페어링 저장소 · 원격 주소)이 들어온다 — 메모 §4 「나중: 데몬 주소와 토큰을 어디서 얻나를 인터페이스로」의 실물.

---

## 4. 빈 공간 — 3단계가 비워 두는 칸

| 빈칸 | 3단계에 두는 모양 | 채우는 것 · 트랙 | 전제 | 연결 관리를 안 고쳐도 되는 이유 |
|---|---|---|---|---|
| TLS(키 고정 · mTLS) | `ws://` 하나 · `LinkInfo.peer_key` 칸 비어 있음 · Dialer 모양은 원격 TRD(§3-5) | `wss://` 어댑터 · 원격 | 데몬 키 · 페어링 · rustls 판 확정(RUSTSEC-2024-0399 판 범위 — 조사 §3-2) | TLS 는 Link 아래 스트림 층 · 위(감독 · 명부 · Wire)는 프레임만 본다 |
| iroh(QUIC · NAT) | 같음 | `iroh` Dialer/Listener · 원격 | 메시지 경계 규칙 · 중계 인프라 결정(조사 §3-2) | Link 가 프레임 단위 trait — WS 가정이 그 위로 새지 않는다(Origin 은 ws 어댑터 안) |
| 사용자 VPN | 데몬 Listener 바인드 주소가 인자(3단계 = `127.0.0.1:0` · `ws://` 는 바인드 뒤 loopback 단언 — §5 1) | 바인드 설정 · 원격 | 원격 Listener 는 TLS 와 함께(평문 원격 금지 — 단언이 코드로 막는다) | 바인드는 수락 루프 인자 하나 |
| 중계(relay) | 없음 | 원격(iroh 중계 또는 자체) | NAT 실측 · 인프라 결정 | 중계는 Link 구현 아래 일 |
| 원격 주소 발견 | `EndpointSource` 의 로컬 구현 하나 | 페어링 저장소 · 원격 주소 구현 · 원격 | 페어링 | 같은 trait 의 다른 구현 |
| 페어링 · 처음 믿기 | `EndpointSource` · `admit` · `Principal::Paired` 자리 | 페어링 저장소 + 판정 구현 · 원격 | 위협 모형(ADR-0179 영향 4 · `client-identity-survey`) | 자격은 `Endpoint.credential` 과 핸드셰이크 구현 안 — transport 는 모른다 |
| `DaemonId` → 키 묶기 | 주장된 · 검증 안 된 id · 그 id 로 영속 신뢰 금지(§3-3) | 데몬 키 쌍 + 지문 대조 · 페어링과 함께 | 데몬 키 보관 자리 | 신원 ≠ 키(§3-2 ①) — 키는 신원 옆 칸 |
| 명령별 인가 정책 | `allow` 없음 · 입구 둘(`call_daemon_command` · `connection_core` dispatch)을 이름으로 박음(D6 ②') | 정책 표 · 다중 클라/원격 | 클라 신원 | `Principal` 이 ConnId 옆에 이미 있다 |
| 클라 신원(다중 클라) | `Principal` 에 칸 · 토큰은 공유 그대로 | 원격과 함께(ADR-0179 영향 4) | 페어링 | `Principal` 은 ConnId 옆 표 — 연결 기계와 무관 |
| 셸 다중 데몬 표면 | 어댑터 상대 N · `DaemonKey` 타입 · 윗층 `primary()` 하나(D4 ①) | 에이전트 → 데몬 길찾기 · 레이아웃 영속의 데몬 키 · LLM 표면(`daemon.*`) · 다중 데몬 트랙 | 「앱 전체」 알림 접기(ADR-0180 결정 8) · 상대별 받는 줄(D11 ①) | 명부는 처음부터 N · `AgentId` 는 UUID 라 출력 길찾기는 이미 데몬 무관 |
| 데몬끼리 어휘 | `PeerWire` 뼈대(식별 · ping) · 실 Listener 는 D5 ② 일 때만(기본 꺼짐) | 우편 중계 · 목록 합치기 · 데몬끼리 트랙 | 되돌아옴 방지 · 상대 부재 처리 조사(transport TRD §12-14) | 관계가 Wire · Registry 단위로 갈려 셸↔데몬을 안 건드린다 |
| 되돌아옴 방지 | 없음 — 3단계 어휘가 남에게 넘기지 않는다(§3-3) | 중계 어휘와 함께(홉 수 · 본 메시지 기억) | 중계 어휘 설계 | 어휘 층 일 — 연결 기계는 홉을 모른다 |
| 인증 실패 속도 제한 | 없음(명시 연기) | `admit` 구현이 `LinkInfo.remote` 로 센다 · 원격 | 원격 Listener | 판정이 `admit` 한 곳 |
| 인증 전 작은 메시지 상한 | 없음 — 16 MiB 단일(§5 15) | 원격 Listener 와 함께 | 업그레이드 뒤 설정 변경 가능 여부(미확인) | 크기 설정은 ws 어댑터 안 |
| 화면 공유 | 없음 | 별도 트랙(조사 `screen-sharing-oss-2026-10-12.md` 결론 1) | — | transport 의 일이 아니다 |
| transport 스트림 층 | 안 씀(D10 ①) | 셸이 창별 스트림 키를 갖게 되면 재론 | replay_flight · 프론트 계약 개정 | Wire 칸은 그대로 — `stream_mark` 를 `Some` 으로 바꾸는 날 결정 |
| 재연결 「다시 연결」 UI | `Peer::reconnect_now` 와 사건이 셸까지 온다 | 팝업 + 버튼 + LLM 명령 · 셸 UI 트랙(ADR-0180 결정 7) | — | 입구는 transport 에 이미 있다 |

---

## 5. 보안 — 옮기는 것 · 새로 막는 것 · 미루는 것

| # | 보호 | 지금 | 목표 | 단계 | 시험 |
|---|---|---|---|---|---|
| 1 | loopback 바인드 | 데몬 `lib.rs:540` `127.0.0.1:0` · 비-loopback 금지는 산문뿐 | 수락 루프 인자(기본 loopback) + ★`ws://` Listener 가 바인드 직후 `local_addr` 이 loopback 이 아니면 거부(C8)★ · 데몬끼리 Listener 도 `127.0.0.1:0` 고정 · 켜는 스위치는 데몬 설정 한 칸(D5 ② 일 때) | 3-0 U7 · 3-3 U5 | 단위 — 비-loopback 바인드 거부 · 실소켓 바인드 주소 단언 |
| 2 | Origin | 허용 목록(없음 = 허용 · 밖 = 403 — `net/src/ws.rs:154-163` · `:264-283`) · ★네 값은 미실측 설계값(`:155-157` 자인)★ | ★D21★ — 사실: `useHttpsScheme` 미설정이라 릴리스 WebView2 의 Origin 은 `http://tauri.localhost` 일 가능성 높음(목록에 없음 — 미확인) · `tauri.localhost` 는 모든 Tauri 앱이 함께 쓴다 · 운영 클라(셸 tungstenite)는 Origin 을 안 보낸다(가능성 높음 — 미확인) · 토큰이 프론트로 간다(`src-tauri/src/commands/discovery.rs:39-47` → `src/api/daemonControl.ts:61`) | 3-0 U7(훅) · 3-3 U5(값 · 실측) | 실소켓 — 허용 · 없음 · 거부(403) · `/qa full` 에서 받은 업그레이드의 Origin 유무를 로그로 잰다 |
| 3 | 첫 프레임 인증 1초 | `ws.rs:124` · `:331` | `DaemonHandshake`(Await → Auth) + 받은 상대 `connect_timeout = 1s` — attach 부터 잰다(12 의 업그레이드 시한과 **직렬**) | 3-3 U4 | 인메모리(`ManualClock`) — 무프레임 1초 거절 · 실소켓 한 건 |
| 4 | 상수시간 비교 | `ws.rs:292` | 데몬 `link/auth.rs` | 3-3 U4 | 단위(`ws.rs:898` 시험 이사) |
| 5 | 비밀이 로그 · 오류 · `Debug` 에 안 실린다(C6) | 판정 쪽은 지킨다(`ws.rs:338` · 셸 `connection.rs:436` · `:445`) · ★그러나 `AuthFrame` 이 `derive(Debug)` 로 토큰을 찍고(`net/src/auth.rs:32-40`) `DaemonInfo` 도 같다(`protocol/src/discovery.rs:18-26`) · transport `Frame` Debug 는 본문을(`frame.rs:10`) `HandshakeStep::Send` 는 그 프레임을(`link.rs:188-191`) 찍는다★ | `Secret` newtype(`Debug` · `Display` = 가림 · `serde(transparent)` — 바이트 같음)을 `AuthFrame.token` · `DaemonInfo.token` · `Endpoint.credential` · `PeerAuth.credential` 에 · transport `Frame` · `HandshakeStep` Debug 는 갈래 + 길이만 | 3-0 U5 · 3-1 U2 · 3-2 U2 · 3-3 U7 | 단위 — `format!("{:?}")` 에 비밀 · 본문 없음 · golden 바이트 불변 |
| 6 | 버전 불일치 거절 문구(ADR-0178) | `handshake_error_frame` → Error 프레임 + 맨 close(`ws.rs:565`) | `Reject{Terminal, last: Error 프레임}` → 닫기 코드 4001 · 셸은 예산 안 씀(ADR-0180 결정 5) · 토큰 불일치는 `Retryable`(D20 ①) | 3-0 U4 · 3-2 U2 · 3-3 U4 | 실소켓 `ws_reject` 꼴 · GUI 팝업 |
| 7 | 쓰기 시한 | 데몬 없음(`ws.rs:579-587` 자인 구멍) | transport `write_deadline` 5s(데몬 새 동작) | 3-3 U5 | transport `ws_write_deadline` + 어댑터 인메모리 |
| 8 | keepalive 20s/50s | `ws.rs:126-128` · 셸은 능동 ping 없음 | transport 정책 — 셸도 능동 ping(새 동작) | 3-2 U4 · 3-3 U5 | transport 인메모리(이미) |
| 9 | 느린 소비자 끊기 | `CONN_TX_CAP` 4608 + `close_signal`(`ws.rs:52`) | `outbound_queue` 4608(D16) + 포화 = `Disconnect`(D13) | 3-3 U4 | 어댑터 — 포화 → 끊김 |
| 10 | 연결별 받는 줄 상한 · 포화 처분 | `ws.rs:78` · `:96` · `:109` · `on_inbound_saturated` | transport 상대별 줄 + 포화 분류 훅(D11 ①) · 데몬이 net 상수(64 · 16 MiB)와 `Refused` · `Bypass` 의미로 구현 | 3-0 U1 · 3-3 U4 | transport 인메모리 · `ws.rs` 시험 이사 |
| 11 | 단일 인스턴스 · daemon.json | `net/src/instance.rs` · `portfile.rs` | 데몬 · 열기 재시도 = `retry_busy`(ADR-0291 결정 15 · 4 × 100 ms) | 3-3 U2 | 시험 이사 · 바쁨 갈래 |
| 12 | **새** 업그레이드 시한 · 수락 분리 | 없음(`ws.rs:323` — 조사 §4 구멍 2) · 느린 핸드셰이크가 다른 수락을 막음(transport `ws.rs:29-30`) | TCP 수락 즉시 연결 태스크로 · 업그레이드 시한(가안 1 s)은 Listener 안에서 **TCP 수락부터** 잰다 · 끝나면 3 의 인증 시한이 attach 부터 이어 잰다 — ★두 시한은 겹치지 않고 더해진다(최악 2 s · 지금 net = 업그레이드 무한 + 1 s)★(추가 4) | 3-0 U7 | 실소켓 — TCP 만 열고 침묵 → 시한 안에 닫힘 · 멈춘 상대 하나 + 둘째 수락 성공 |
| 14 | **새** 연결 수 상한(C2) | 없음(구멍 3) · 1판 안은 업그레이드 대기를 못 셌다 | ★세 칸 · 셈의 시작점이 다르다★ — 업그레이드 대기 `max_pending_upgrades`(Listener 설정 · TCP 수락 → 업그레이드 끝) · 인증 대기(attach → Live) · 전체(Live). 셈 = 상대 수명에 묶인 RAII 가드(사건으로 세지 않는다 — §3-0 ②) · 인증에 실패한 상대는 닫히는 즉시 칸을 놓는다 · 넘치면 **새것을** 즉시 닫는다(가안 8 · 8 · 64 — ★미검 · 메인이 정해 보고하는 내부 상수★). 대가: loopback 은 OS 사용자 모두가 함께 쓴다 — 같은 PC 의 다른 사용자가 대기 칸을 채우면 셸이 못 붙는다(로컬 DoS · 알려진 한계로 받아들인다) | 3-0 U7 · 3-3 U4 | 인메모리 — 넘친 수락 즉시 닫힘 · 실패한 상대가 칸을 놓음 |
| 15 | **새** 메시지 상한(C10) | tungstenite 기본 64 MiB(구멍 5 · net `ws.rs:85`) · transport `WsDialer` 도 기본(`ws.rs:63`) | 받는 쪽 · 거는 쪽 둘 다 `max_message_size` = `max_frame_size` = 16 MiB — 내보내는 쪽 거절(`ws.rs:138-142`)과 맞춘다 · ★정당한 가장 큰 메시지 미측정★ | 3-0 U7 | 실소켓 — 초과 → 끊김(양방향) |
| 16 | **새** Host 검사(DNS 리바인딩) | 없음(구멍 8) | 업그레이드에서 `Host` ∈ {`127.0.0.1:<port>` · `localhost:<port>`} 또는 없음(D9) | 3-0 U7 · 3-3 U5 | 실소켓 — 다른 Host → 403 |
| 17 | **새** 수락 정책 필수(C1) | transport 무검사 `WsListener::accept`(`ws.rs:217`) | 무검사 경로 삭제 · `AcceptPolicy` 에 `Default` 없음 · 허용 정책은 이름이 위험을 말한다(가안 `AcceptPolicy::permissive_unsafe()` · `test-support` 뒤) | 3-0 U7 · 3-3 U5 | 3-3 U5 `ws_e2e` 다섯 — 나쁜 Origin → 403 · 나쁜 Host → 403 · 침묵 TCP → 닫힘 · 16 MiB 초과 → 닫힘 · 1 s 안에 Auth 없음 → 닫힘(`start_test_server` · `run_accept_loop` `lib.rs:337` 를 거친다) |
| 18 | **새** 보안 로그(C9) | net 은 거절마다 `tracing::warn`(`ws.rs:326` · `:340`) · transport 는 tracing 이 없다(`transport/Cargo.toml:46`) | Listener 가 거절 갈래 + 원격 주소를 소비자에게 돌려준다(`AcceptRefusal{class, remote}`) · 데몬이 인증 실패 · 상한 거절 · 신원 재생성을 토큰 없이 로그 | 3-0 U7 · 3-3 U4 | 단위 — 거절 갈래마다 보고 · 로그에 토큰 없음 |
| 19 | **새** 상대가 준 문구(C7) | `DecodeFailed.reason` = `err.to_string()`(`peer.rs:911`) · `Rejected.reason` 이 팝업 문안(`event.rs:170`) | §3-0 ⑤ — 분류 문자열 · 길이 상한 · 제어 문자 제거 · LLM 표면에 데이터 표시 | 3-0 U4 | 단위 — 긴 문구 · 제어 문자 · 값 섞인 해독 오류 |
| 20 | **새** 연결의 신원(C3) | 없음 | `Principal` 을 attach 의 핸드셰이크 값에 동기로 · 없으면 거부 · 닫기(D6 ②') | 3-3 U4 | 인메모리 — `Principal` 없는 Live 는 닫힘 |
| 21 | `daemon.json` ACL(C5) | ★지금 로컬 구멍일 수 있다★ — 릴리스 데이터 폴더 = exe 폴더 `data/`(`data_dir.rs:62-63` · `:91-99`) · 상속 ACL 이 다른 로컬 사용자에게 읽기를 줄 수 있다(미확인 — `icacls` 로 잰다) · 토큰 = 전권 | ★D22★ | 3-3 U2(실측 · ① 이면 구현) | `icacls` 실측 기록 · (① 이면) 단위 |
| 22 | 인증 실패 속도 제한 | 없음(구멍 4) | **연기** — loopback · 256비트 토큰이라 무차별 대입 이득이 없고 전역 계수는 자기 DoS 가 된다 · 원격 Listener 와 함께 | 원격 | — |
| 23 | 암호화 | 없음(구멍 1) | **연기** — §4 TLS · 지금은 loopback 전용(1 이 단언) | 원격 | — |
| 24 | 데몬끼리 인증(C4) | 없음 | §3-3 「데몬끼리」 — 주장된 id · `LocalClient` 급 · 자격 = 받는 쪽 토큰 · 관계 밖 인증 프레임 거절 | 3-3 U7 | 인메모리 — 셸 `Auth` 를 데몬끼리 Listener 가 거절 · 그 역 |
| 25 | 제어 평면 HTTP Host 검사 | `/control/*` 미확인(조사 §4 구멍 8) | **범위 밖** — transport 와 별개 서버. `docs/tracking.md` 에 항목(오케스트레이터) | — | — |

(13 은 2판에서 12 에 합쳤다 — 번호는 1판 참조를 위해 비워 둔다.)

---

## 6. 작업 단위

**규칙(사용자 2026-10-02 「망가지지 않는 단위로」 · 「임시 땜빵 금지」 · 전역 규율 「중간 어디서 멈춰도 빌드가 서게」):** 단위마다 워크스페이스 빌드 · 회귀 · ★모든 게이트가 초록인 채 끊는다★. 갈 곳을 먼저 세우고 옛 자리를 지운다. 타입을 바꾸는 단위는 그 타입의 호출부를 **같은 커밋**에서 다 맞춘다(자료구조만 갈아엎고 호출부를 다음 커밋에 미루지 않는다). 게이트 사본(CI · qa · CLAUDE.md · testing-strategy · crate 헤더)은 게이트를 바꾸는 그 커밋에서 함께 고치고 `/review doc` 을 거친다(§2-4). 각 단위 앞뒤로 `cargo test --workspace -- --test-threads=4` 결과 줄 · 통과 · 무시를 견준다(줄이 줄면 타깃 소실 — 초록이어도 멈춘다 · 3단계는 net 의 결과 줄 둘(lib · doc-tests — 미확인 셈)이 의도적으로 사라진다).

### 6-0. 3-0 — transport 보강 (D19 ① 기준 — ② 면 U6 · U7 이 3-3 U3 자리로 돌아간다)

| 단위 | 범위 | 시험 | 게이트 · QA |
|---|---|---|---|
| **U1** | 상대별 무손실 받는 줄 · 생애 사건을 같은 줄에 · 포화 분류 훅(§3-0 ① ② · D11 ①) — `Inbound` 표면 · 감독 select 에 줄 자리 갈래 · `Dropped{Inbound}` 갈래 정리 · lib 헤더 「공유 줄 포화는 연결을 안 끊는다」(`lib.rs:128-131`) · registry 헤더(`:6-9`) 다시 씀 | 인메모리 — 시끄러운 상대 + 조용한 상대(조용한 쪽 손실 0) · 줄이 찬 동안 keepalive · close 가 들림 · 훅 `Refuse` · `Bypass` · 생애 사건 손실 0 · replay 한 번(4096)이 512칸 줄에서 손실 0 | transport gate 1~4d · `/qa standard` · `/review code full` |
| **U2** | 세대 도장 · 보냄 칸(§3-0 ③ · D12 ① — 1판 3-2 U1) — `PeerCmd` 에 세대 · 쓰기 직전 대조 · `RequestError::Disconnected{sent}`(`fail_pending` `peer.rs:1151-1162` 가 대기 슬롯(보냄)과 큐(안 보냄)를 이미 따로 훑는다) · `Superseded` doc 을 lib 헤더와 맞춤(`pending.rs:29`) · 헤더 게이트 ① 문장(메모 §11 `:332`) | ADR-0195 두 시험(`tests.rs:3029` · `:3505`)의 transport 판 · 보냄 · 안 보냄 갈래 | 같음 · `/qa standard` |
| **U3** | 닫기 전 비우기(§3-0 ④) — `Peer::close_after_flush` | 인메모리 — 앞선 알림 N 개 뒤 닫기 → N 개 다 쓰고 닫힘 · 쓰기 시한이 상한 | `/qa standard` |
| **U4** | 거절 분류 · 상대 문구(§3-0 ⑤ · D20 · §5 19) — `Reject` 종류 칸 · 기계의 `Retryable` 갈래(예산을 쓴다) · `DecodeFailed.reason` 분류 · `Rejected.reason` 가림 | 인메모리 — `Retryable` 은 예산을 쓰고 다시 dial · `Terminal` 은 포기 · 긴 문구 · 제어 문자 | `/qa standard` |
| **U5** | `Debug` 가림(§5 5) — `Frame` · `HandshakeStep` Debug 는 갈래 + 길이만(transport 시험 기능 게이트는 소비자가 붙는 3-2 U2 · 3-3 U4 에서 — §2-4) | 단위 — `{:?}` 에 본문 없음 | `/qa standard` |
| **U6** | 받는 쪽 seam(§3-1 ① · D1 — 1판 3-3 U3 앞 절반) — `Listener` trait · `Registry::attach` · 받은 상대 모드(Live 아닌 모든 끝 = `Closed` + 스스로 내림 · 세대 표 밖) · `LinkInfo` · 양끝 인메모리 통로 + `MemoryListener`(지금 `MemoryNetwork` 는 먼 끝이 대본이라 두 명부를 못 잇는다 `testing.rs:349-416`) | 인메모리 — 받은 상대 손실 = `Closed` · 다시 안 건다 · 인증 실패 N 번 뒤 진짜 연결 · 명부 · 세대 표 크기 고정 · 두 명부 양방향 요청 | `/qa standard` · `/review code full` |
| **U7** | ws 수락 강화(§5 1 · 2(훅) · 12 · 14 · 15 · 16 · 17 · 18 — 1판 3-3 U3 뒤 절반) — `AcceptPolicy`(필수 · `Default` 없음) · 무검사 `accept` 삭제 · 업그레이드 대기 상한 · 시한 · Origin/Host 훅 · 양방향 크기 · loopback 단언 · `AcceptRefusal` 보고 — 값은 전부 소비자 인자 | 실소켓 — §5 12 · 15 · 16 · Origin 403 · 비-loopback 거부 · 대기 상한 | transport gate 4c 에 새 실소켓 스위트 등록 · `/qa standard` |

- **U1 이 가장 넓다** — `Inbound` 가 바뀌면 transport 시험 하네스(`testing.rs`)가 같이 바뀐다. 한 커밋으로 안 서면 「상대별 줄(공유 줄 앞단)」 → 「배압」 → 「훅」으로 나눈다 — 커밋마다 무손실을 늘리기만 한다.
- **3-0 과 3-1 은 파일이 안 겹친다**(transport vs protocol · 셸 · 데몬) — 나란히 가도 된다. 3-2 U2 는 3-0 U1~U5 뒤 · 3-3 U4 는 3-0 전부 뒤.

### 6-1. 3-1 — protocol 정리

| 단위 | 범위 | 시험 | 게이트 · QA |
|---|---|---|---|
| **U1** | wire golden 먼저 — 명령 버스 변형 여섯(`RegisterCommands` · `UpdateCommands` · `Command` · `CommandOutcome` · `CommandRequest` · `CommandReply` — `messages.rs:268-337` · `:608-656`)의 지금 바이트를 protocol 시험으로 고정. ★`AuthFrame` golden 은 net 에 둔다★ — 타입이 아직 net 에 있어 protocol 시험이 부르려면 protocol → net dev 의존이 들고, 그 간선은 U5 의 의존 상한(`-e normal,dev,build`)에 걸린다. 코드 무변경 | 단위(바이트 문자열 리터럴 — 다른 타입과 round-trip 대조 금지 · `net/src/auth.rs` golden 의 방식) | `cargo test -p engram-dashboard-protocol` · `/qa quick` |
| **U2** | `protocol::handshake::AuthFrame`(같은 serde) + net `auth.rs` 의 golden(`:47`)을 함께 옮김 · `AuthFrame.token` · `DaemonInfo.token` = `Secret`(§5 5 — `serde(transparent)` 라 golden 그대로) · net `auth.rs` = 재수출(net 의 protocol 의존이 optional 에서 무조건으로 — net gate 3 은 `--all-features` 라 3줄 그대로 · ★net gate 4 는 재수출뿐이라 그대로 초록★) · 소비자 전환(셸 `connection.rs:24` · `stop.rs:22` · `tests.rs:45` · `tests/stop_smoke.rs:186` · 데몬 `agent_conn.rs:808`(시험) · `tests/ws_e2e.rs:24`) · ★셸의 net 의존 삭제(`src-tauri/Cargo.toml:79-85`) + `Cargo.lock`★ · 셸 → net 0 게이트 | golden 이 그대로 · `{:?}` 에 토큰 없음 · 셸 `lib_unit` · `daemon_client_*` | net gate 4 · 5a/5b · platform gate 7 · 새 셸 게이트 · `--locked` · `/qa standard` · `/review doc` |
| **U3** | `OwnerToken` protocol 사본(newtype — 안쪽 문자열 그대로 직렬화 = 같은 바이트, D17 ①) + 셸 · 데몬 끝자락 변환 함수 · 그 필드를 쓰는 모든 생성 · 해체 자리 한 커밋 | U1 golden · 명부 시험(`command_roster`) · 셸 등록 시험(`layout_commands`) | `/qa standard` |
| **U4** | 명령 칸 넷(봉투 · 답장 · 등록 · 등록 변경)을 `Box<RawValue>`(serde_json `raw_value`) · `#[ts(type = ..)]` 덮어쓰기 유지(D18 ① — 생성물 바이트 불변) · 짝 맞추기 두 함수를 셸 `protocol_state.rs` 로(본문 이사 — `Command`/`CommandReply` 갈래는 받은 즉시 봉투를 구조체로 풀어 번호를 읽는다 · ADR-0272 결정 4) · 데몬은 받는 쪽 하나(`agent_conn.rs:433`) · 짝 고정 시험(`messages.rs` 의 「한 쌍」 시험)을 셸로 · `From<command::RequestId>` 를 끝자락으로 · 「패킷은 풀렸는데 봉투가 깨진」 처리(ADR-0272 결정 6 — 셸: 요청 번호 없음으로 · 데몬: `Error{request_id: None}` 답 · 가안) · 데몬 중계(풀기 · 다시 싸기) | U1 golden(바이트 그대로가 요점) · 깨진 봉투 갈래 단위 · `control_send` · `mail_gate` · `ws_e2e` | 생성물 sync(바뀌지 않아야 한다) · `/qa standard` · `/review code full` |
| **U5** | protocol → command 간선 삭제(`protocol/Cargo.toml:8`) · protocol 의존 상한 게이트(정확히 1줄) · 문서(ADR-0272 영향의 `OwnerToken` 닫힘 · 메모 §11 `CommandListEntry`≈`RosterEntry` 는 보고 4단계로) | — | 새 게이트 · `/qa standard` · `/review doc` |

- **U2 를 3-1 에 둔 이유:** 셸이 net 에서 쓰는 것이 `AuthFrame` 하나뿐이라(`src-tauri/Cargo.toml:79` 주석) 프레임이 protocol 로 오는 순간 셸 → net 간선이 걷힌다. 3-2 의 셸 작업이 net 을 아예 모른 채 시작한다.
- **U4 가 가장 넓다** — 데몬(`connection_core` · `command_delivery` · `command_roster` · `agent_conn`) · 셸(`inbound.rs` · `view_commands.rs` · `commands/agent.rs:248`) 의 생성 · 해체 자리를 한 커밋에서 맞춘다. 번지면 칸 하나씩(봉투 → 답장 → 등록 둘) 커밋을 나눈다 — 칸마다 golden 이 바이트를 잰다.

### 6-2. 3-2 — 셸 부착

| 단위 | 범위 | 시험 | 게이트 · QA |
|---|---|---|---|
| ~~U1~~ | → 3-0 U2 로 옮겼다(번호는 1판 참조를 위해 비워 둔다) | — | — |
| **U2** | 셸 `daemon_client/link/` — `ShellWire`(Out = `AgentCommand` · In = `ShellIn{Event(AgentEvent) · Output(Vec<u8>)}` — 출력은 해독하지 않고 통과 · `request_tag`/`reply_tag` = 3-1 U4 의 두 함수 · `stream_mark` = `None` · `resume_request` 는 crate 가 부를 길이 없음을 시험으로 고정) · `ShellHandshake`(Send(Auth) → Hello = Done · Error = `Reject` — 「auth failed」 = `Retryable` · 버전 불일치 = `Terminal`(D20 ①) · 그 밖 프레임은 버리고 Await — `connection.rs:2020-2050` 과 같다) · `LocalDaemonDialer` + `EndpointSource`(§3-6) · `TransportEvent` → `ConnectionState`/`DaemonEvents` 사상 · `RequestError` → 지금 글(`SENT_OUTCOME_UNKNOWN` · `UNSENT_ON_DISCONNECT` 등) 사상 · `DaemonKey` 타입 · transport `test-support` 운영 그래프 게이트(셸 몫 — §2-4). **아무도 안 부른다** | transport `MemoryNetwork` + `ManualClock` — 핸드셰이크 셋 · 거절 둘(예산 씀 · 안 씀) · 포트 이동 따라가기 · 마지막 주소로 · 오류 글 바이트 대조 · ★답장이 앞선 사건을 앞지르는 순서(§3-3 B2)★ | 셸 끝자락 게이트 · transport 시험 기능 게이트(§2-4) · `/qa standard` · `/review doc` |
| **U3** | 새 연결 구동(가안 `connection_v2`)을 `DaemonClient` 생성자 선택으로 — 시험 생성자 먼저 · ★새 구동의 시험은 `ManualClock` + 가짜 서버로 따로 세운다 — 옛 시험을 두 구동에 돌리지 않는다(아래 「셸 시험 운명」 — 1판의 매개화는 시계 때문에 안 선다)★ · 운영은 아직 옛것 | 운명표의 「옮김」 · 「다시 씀」 몫 · 옛 스위트는 옛 구동에 그대로 | `lib_unit` · `daemon_client_pending` · `daemon_client_replay` · `/qa standard` |
| **U4** | 운영 생성자(`new_real_with_owned_runtime`)를 새 구동으로 · 정책 주입(§7 — D14 · D15) · `REPLY_TIMEOUT`(`commands/agent.rs:277`) 걷기 · 옛 구동 삭제 + 운명표 「퇴역」 몫을 사유 줄과 함께 지움 · `daemon_client_pending` 을 M1(겹친 번호 = 양쪽 실패 — transport TRD §4-1)로 다시 씀 · 셸 `tokio-tungstenite` → dev | 위 전량 + 다시 쓴 pending | `/qa full`(GUI §6-4 ①~⑤) · `/review code full` |
| **U5** | 정지 클라를 transport 위로(`link/stop.rs`). ★`send_stop` 은 동기다★ — 트레이 워커(`tray/mod.rs:217`)와 앱 종료(`tray/actions.rs:185`)가 부른다. 명세: ① 셸 tokio 런타임 핸들에 태스크로 띄우고 동기로 시한까지 기다린다(런타임 스레드 위에서 부르면 실패로 단언) ② ★주 상대를 먼저 닫는다★ — 안 닫으면 데몬이 꺼진 뒤 주 상대가 재연결을 시도하고 띠를 띄운다 · 의도된 끊김이라 띠 없음 · 다시 켜면 `add` 로 올린다 ③ 일회용 상대(재연결 0 · 짧은 시한)로 `request(StopDaemon)` — Ack = 성공 · 그 뒤 끊김 = `DaemonClosed`. `notify` 로 보내면 Ack 가 `UnmatchedReply` 로 떨어진다(`StopDaemon` 은 `request_id` 를 갖는다 — `messages.rs:1019`) · 셸 `tungstenite` 삭제 · `stop_smoke` 다시 겨눔(T-52 — 알려진 실패 · 원인 미규명. 고쳐지면 기록, 아니면 그대로 둔다) · 셸 직접 의존 tungstenite 0 게이트 | 인메모리 — 주 상대가 재연결을 안 함 · Ack 대기 · 시한 · `stop_smoke --ignored`(실 데몬) · 트레이 GUI | `/qa full` · `/review doc` |
| **U6** | 상대 N 자리(D4 ① — ★D11 ① 이 전제★: 받는 줄이 상대별이어야 시끄러운 데몬이 조용한 데몬을 굶기지 않는다) — 어댑터가 `Registry` 하나에 상대 N 을 담고 윗층(`router` · `layout` · 명령)은 `primary()` 하나를 본다 · 가짜 데몬 둘을 붙이는 시험(사건이 상대 표식으로 갈리고 하나가 죽어도 다른 하나가 산다 · 출력은 `AgentId` 로 길찾기) | 실소켓 가짜 서버 둘 · 같은 PC 실 데몬 둘(`ENGRAM_DATA_DIR` 둘 — `#[ignore]` 수동 · GUI ⑥) | `/qa standard` |

- **U3 · U4 를 가른 이유(전역 규율 「쪼개는 건 착수 시점」):** 2,050줄 연결 루프를 한 커밋에 갈아 끼우면 중간 상태가 없다. 새 구동을 옆에 세우고 운영 전환은 생성자 한 줄이다. 되돌릴 지점 = U3 커밋.

**셸 시험 운명**(`daemon_client/tests.rs` — 시험 91 · 5,978줄 · 묶음 경계는 머리 주석의 정적 판독이고 U3 착수 때 시험 이름으로 확정한다):

| 묶음 | 예(줄) | 운명 | 사유 |
|---|---|---|---|
| 핸드셰이크 바이트 · 순서 | 케이스 ① ② · Fix C(`:212-289` · `:479`) | 옮김 — 새 구동 + 같은 가짜 WS 서버 | 바이트가 그대로라 가짜 서버가 선다 |
| 발견 · 띄우기 정책 | 케이스 ③ ④ · 보강 둘(`:289-403`) | 옮김 — `EndpointSource` 가짜 위로 | ADR-0021 구조 |
| 손으로 짠 생애 · 세대 가드 · 취소 경쟁 | Fix B · (a)~(e) · T4 `reconnect_guard` · bump · read_live/핸드셰이크 창 close(`:524-960` · `:1583-1960`) | 퇴역 — 사유 줄에 대응 transport 시험 이름을 적는다 | 그 기계(`lifecycle`)가 지워진다 · 같은 성질은 transport 감독 시험이 잰다(예: `close_is_heard_while_a_write_is_parked` `peer.rs:2953`) |
| 백오프 · 재연결 · 소진 · 포트 이동 | `:964-1580`(tokio `pause`/`advance` 36줄 대부분) | 다시 씀 — `ManualClock` + `MemoryNetwork` | `SystemClock::now()` = `std::time::Instant`(`clock.rs:34-35`)라 tokio 멈춘 시계와 갈린다 |
| ADR-0195 | `:3029` · `:3505` | transport 판(3-0 U2) + 셸 어댑터 한 건(끊긴 동안 명령 즉시 실패 글) | 지킴이가 transport 세대 도장으로 간다 |
| 그 밖(pending · usage · replay · 명령 글) | 나머지 | 옮김 | 바이트 · 글 대조 |

### 6-3. 3-3 — 데몬 부착 · net 걷기

| 단위 | 범위 | 시험 | 게이트 · QA |
|---|---|---|---|
| **U1** | 데몬 `conn_port.rs` = `pub use engram_dashboard_net::frame_port::*` · 데몬 소스 · 시험의 `frame_port` 경로를 전부 이쪽으로 — ★손 명단을 쓰지 않는다★(1판 명단은 `control/catalog.rs:469` 를 빠뜨렸다) · 찾는 법 = `git grep -n engram_dashboard_net -- crates/engram-dashboard-daemon`(지금 47줄 — `frame_port` 아닌 것 = `lib.rs` 의 `ws` · `instance` · `portfile` 줄은 U2 · U5 몫) · 기계적 · 동작 무변경 | 기존 전량 | `/qa quick` |
| **U2** | `instance` · `portfile` 를 데몬으로(`data_dir.rs:17` `DAEMON_FILE` 포함) · 열기 재시도 = `retry_busy`(★동작 차이 — `is_busy` 는 접근 거부도 잠김으로 쳐 최악 400 ms 늦게 같은 오류가 돌아온다 · 지금은 공유 위반만 다시 한다 `instance.rs:242`★) · net 의 platform 간선 삭제 → net gate 2a 기대값 5 → 0 · gate 3 3 → 2 · ★2b 는 그대로★ · platform 4f/4h 명단 경로 · ADR-0135 앵커 · ★`daemon.json` ACL 실측(`icacls` — D22) · ① 이면 만들 때 소유자 전용 DACL(platform)★ | 시험 이사 · 바쁨 갈래 · 같은 경로 시험(셸 사본 — ADR-0271 결정 4) · ACL 실측 기록 | net gate 2a · 2b · 3 · platform 4f/4h · `/qa standard` · `/review doc` |
| ~~U3~~ | → 3-0 U6 · U7 로 옮겼다 | — | — |
| **U4** | 데몬 `link/` — `DaemonWire`(원문 통과 — §3-3 「DaemonWire 모양」 · `request_tag` · `reply_tag` · `stream_mark` 전부 `None`) · `DaemonHandshake` · `auth.rs`(판정 seam · 토큰 · 상수시간 · 버전) · `Principal` 보관(D6 ②' — 없으면 닫기) · `server.rs` = `conn_port` 계약을 transport 위에 구현(수락 루프 · 상한 셋의 RAII 가드 · `ConnId` ↔ `PeerId` · 상대마다 상태 기계(§3-0 ②) · 포화 분류 훅 = net 의미(`Refused` 답 · `Bypass` · 상수 64 · 16 MiB · 8 · 10 s 그대로) · `FrameSink` = `Peer::notify` · `Frame::Close` = `close_after_flush` · `FrameFanout` = 상대별 알림(D13) · 거절 로그) · 아직 수락 루프에 안 꽂음 | `net/src/ws.rs` 시험 22개 중 의미가 남는 것을 어댑터로 이사 · 인메모리 다중 상대(셸 둘이 붙어 팬아웃 · 하나 포화가 다른 하나를 안 끊음) · 깨진 명령 Error + 유지 · 두 번째 Auth · binary Error + 닫기 · 정지 Ack 가 닫기 앞 · `Principal` 없는 Live 닫힘 | 끝자락 어휘 게이트 ① ② · transport 시험 기능 게이트(데몬 · cli 몫) · `/qa standard` · `/review code full` · `/review doc` |
| **U5** | `run_accept_loop`(`lib.rs:337`)와 `start_test_server*`(`:854-871`)를 어댑터로 전환 · `KeepaliveConfig` 재수출(`lib.rs:63` — `ws_e2e.rs:22` · `:1578` 이 쓴다)을 transport keepalive 정책으로 · 받은 상대 정책(`connect_timeout` 1 s · `outbound_queue` 4608 · keepalive 20/50) · 수락 정책 값(Origin — D21 · Host · 크기 · 상한) | `ws_e2e` · `mail_gate` · `control_agent` · 셸 dev 의존 시험(데몬 시험 서버를 쓴다) 전량 + ★`ws_e2e` 다섯(§5 17)★ | `/qa full`(GUI §6-4 전부 · Origin 유무 실측) |
| **U6** | `conn_port.rs` 를 재수출에서 자기 정의로(net `frame_port.rs` 그대로 옮김) · net crate 삭제(멤버 · `Cargo.lock` · 데몬 `Cargo.toml:99`) · net gate 1 · 2a · 2b · 3 · 4 · 5 함께 삭제 · §2-4 의 대신 세우는 게이트 · 사본 다섯 · CLAUDE.md 「백엔드 모듈 맵」(net 항목 삭제 · transport 항목 신설 — 메모 §11 `:345`) · 「네트워크 행 격리 게이트」 절 → 「끝자락 게이트」 | 워크스페이스 결과 줄 대조(net 몫이 사라진다) | `/qa standard` · `/review doc` |
| **U7** | 데몬 신원(D3 ① — `DaemonId` 파일 · `Hello.daemon_id` · 재생성 경고 로그) · `Endpoint.expected_daemon_id` + `ShellHandshake` 대조(§3-2) · 데몬끼리 뼈대(protocol `peer`: `PeerAuth{daemon_id, credential: Secret, protocol_version}` · `Identify` · `Ping` · `PeerWire` · 데몬 `link/peer.rs` · 관계 밖 인증 프레임 거절) · ★D5 ② 일 때만★ 둘째 Listener(기본 꺼짐 · `127.0.0.1:0` · 데몬 설정 한 칸) + `daemon.json` 칸 + 거는 방향 · 동시 걸기 대칭 깨기(§3-3) | 인메모리 두 명부(양방향 요청 · 한쪽 죽음 · 신원 대조 실패 = 거절 · 셸 `Auth` 거절 · 그 역) · D5 ② 면 같은 PC 실 데몬 둘(`#[ignore]`) | 생성물 sync(`Hello` 한 칸) · `/qa standard` |

- **U1 이 맨 앞인 이유:** 위층 30여 자리가 `net::frame_port` 를 이름으로 부른다. 재수출 한 겹을 먼저 넣으면 U4~U6 이 위층을 한 줄도 안 건드리고, net 삭제(U6)는 재수출을 정의로 바꾸는 한 파일이 된다.
- **3-0 이 U4 앞:** 어댑터가 쓸 transport 표면(상대별 줄 · 훅 · attach · 수락 정책)이 먼저 선다.
- **U4 가 이 단계의 본체다** — net 의 연결별 받는 줄 · bypass(`ws.rs:799-900`)의 의미를 3-0 U1 의 훅 위에 잇는다(D11 ①).

### 6-4. GUI 실측 (`/qa full` — 절차는 qa 바인딩 §full)

① 연결 · 명령 왕복(에이전트 띄우기 · 끄기) ② 데몬 재시작 → 옮겨 간 포트로 다시 붙음(ADR-0021 — 띄우지 않는다) ③ 버전 불일치 데몬 → 거절 팝업 · 재연결 예산을 안 태움(ADR-0178 · 0180 결정 5) ④ 창 여럿 + 팝아웃에서 replay → live · seq 연속(출력이 많은 에이전트 하나를 붙인 채 새 창) ⑤ 트레이 데몬 끄기 · 앱 종료(끈 뒤 재연결 · 띠 없음) ⑥(3-2 U6) 같은 PC 데몬 둘 + 셸 하나 — D4 ① 의 수동 실측(primary 만 화면에 · 둘째는 로그로) ⑦(3-3 U5) 받은 업그레이드의 Origin 유무 · Host 값을 로그로(D21 · R6).

### 6-5. ADR — 써야 하거나 고칠 것 (목록만 · 채번 · 도장 = `/adr`)

| 시점 | 새 ADR(가칭) | 고치는 조항(링크가 아니라 개정) |
|---|---|---|
| 3-0 · 3-1 착수 전 | **A** 「net 을 걷는다 — 연결 기계 = transport · 어휘(인증 프레임 포함) = protocol · 데몬 전용 = 데몬」(결정 후보 7 · D7) · **C** 「transport 보강 · 받는 쪽 seam — 상대별 무손실 줄 · 세대 도장 · 닫기 전 비우기 · 거절 분류 · Listener/attach · 받은 상대는 다시 걸지 않는다」(D19 · D11 · D20 · D1 · transport TRD §12-7 닫힘) | ADR-0272(D17 · D18 · 짝 맞추기의 실제 자리) · ADR-0129(인증 프레임 소유 · net gate 4 의 축 → 끝자락 어휘 게이트 ②) · ADR-0177 결정 3(받는 줄이 상대별 · 무손실 — 「유계 큐와 배압 신호」를 받는 쪽으로 넓힘) · ADR-0195(지킴이 = transport 세대 도장) · ADR-0180 결정 5(D20 ① 이면 「붙었다가 거절 = 즉시 팝업」을 버전 불일치로 좁힘) |
| 3-2 착수 전 | **B** 「셸 부착 — 접속 정보 인터페이스 · 논리 주소 + 풀이 Dialer · 정책 주입 값 · 정지 클라」(D14 · D15) | ADR-0036(재연결 · pending 소유 → transport) · ADR-0271 결정 5 닫힘 · ADR-0181 결정 1(D15 ① 이면 8 s 대신 30 s 주입 · spawn 실측 뒤 재론) · ADR-0180 결정 4(D14 ① 이면 3 회 대신 5 회 주입 · 버튼 착지 때 3) · 링크: ADR-0021 · 0178 |
| 3-3 착수 전 | **D** 「관계 · 신원 — 관계마다 Wire/Registry/Listener · 영속 데몬 신원 · 다중 데몬 · 데몬끼리 깊이 · 데몬끼리 신뢰 수준」(D2 · D3 · D4 · D5) · **E** 「전송 보안 층 · 연결 신원 — Principal · 입구 둘 · Origin · ACL · 상한」(D6 · D8 · D9 · D21 · D22 · §5) | ADR-0129 폐기(A 가 대체) · ADR-0130 폐기(대상 소멸) · ADR-0177 결정 3 · 9(스트림 층을 두 소비자가 안 쓴다 — D10 ①) · ADR-0177 「근거」의 「큐 상한이 되감기 링보다 크면 안 되는 이유」(`:52`)를 D16 ① 과 맞게(§7) · ADR-0177 결정 6(팬아웃 포화 = D13) · ADR-0157(D6 ① 이면 입구에 `Principal` 인자 · ②' 면 링크) · 링크: ADR-0179 결정 3 · 영향 4 · ADR-0135 · ADR-0291 결정 15 · D13 · D16 은 C 또는 D 에 |

---

## 7. 정책 기본값 맞추기 (D14 · D15 · D16 · D20)

| 값 | transport 기본(`policy.rs:191-211`) | 지금 | 안 | 권고 |
|---|---|---|---|---|
| 재연결 횟수 | 3 | 셸 5(`connection.rs:90`) · 프론트 직결 경로 5(`src/api/wsTransport.ts:383` — 운영 아님) · 산문 「5회」(`agentClient.ts:191` · `daemonControl.ts:92`) | ① 3-2 는 5 를 주입하고 「다시 연결」 팝업이 착지하는 날 3 ② 3 지금 ③ 3 + 팝업 · 버튼을 3-2 에 | **①** — ADR-0180 이 3 을 떠받치는 전제로 버튼(결정 7)을 들었고 그 UI 가 아직 없다(`git grep "다시 연결" src` → 0 · 실측). ② 는 더 빨리 포기하는 것만 남는다 · ③ 은 UI 범위가 3-2 에 든다 · D20 ① 이면 토큰 거절도 이 예산을 쓴다 |
| 요청 시한 | 8 s | `forward_daemon_command` 30 s(`commands/agent.rs:277`) · 나머지 상한 없음 | ① 30 s 공용 주입 · spawn 실소요를 3-2 QA 에서 재 기록 · 8 s 는 따로 ② 8 s 지금(ADR-0181 결정 1) ③ 명령별 | **①** — ADR-0181 이 「적용 시 30초를 함께 정한다」 · 「8초의 유일한 실질 위험 = spawn 실소요 미측정」을 적었다. ① 은 「공용 하나」를 성립시키되(없던 상한을 얻는 핸들러가 생긴다) 위험을 재고 내린다 |
| 백오프 | 500 ms · ×2 · 상한 10 s · 지터 0.2 | 같음 · 지터 없음(`connection.rs:93-96`) | 그대로 받는다 | 지터는 새 동작(N 데몬일 때 동시 재연결 흩기) |
| 붙어 버틴 시간(`live_dwell`) | 30 s | 없음(성공 즉시 리셋) | 그대로 | 붙자마자 끊기는 데몬에 영원히 매달리지 않는다 |
| 연결 · 핸드셰이크 시한 | 10 s | 10 s(`connection.rs:84`) | 그대로 · 데몬이 받은 상대는 1 s(§5 3 — 업그레이드 시한 뒤에 이어 잰다 · §5 12) | — |
| keepalive | 20 s / 50 s | 데몬 같음 · 셸 능동 ping 없음 | 그대로 | 셸이 데몬 죽음을 스스로 안다(transport TRD §6) |
| 쓰기 시한 | 5 s | 셸 없음(미확인 — 쓰기 시한 상수를 못 찾음) · 데몬 없음 | 그대로 | 데몬 새 동작 — R7 |
| 나가는 큐 | 512 | 셸 512 · 데몬 4608(`net/src/ws.rs:52` = 링 4096 + 512) | ① 데몬 4608 유지 ② 링 이하 | **①** — replay 한 번이 4096 을 미는데 512 면 포화 → `Disconnect` → 다시 replay 가 되풀이된다. ADR-0177 근거(`:52` — 원문 대조함, 1판의 「미확인」 닫힘)는 「큐 > 링이면 끊기는 순간 가장 오래된 것이 이미 링에서 밀려났을 수 있다」를 든다 — 그것은 **잘림**이고 replay 머리가 마커로 실려 받아들여진다(CLAUDE.md 「replay→live」). 큐 < 링은 replay 한 번을 못 담아 **고리**가 된다. 2판 판정 = 큐 ≥ 링 → ADR-0177 근거 문단 개정(§6-5) |
| 받는 줄 | 512(공유 한 줄) | 셸 없음 · 데몬 연결별 64칸 + 16 MiB | 상대별 줄(D11 ① — 크기는 정책 값 · 셸 가안 512 · 데몬 = net 상수) · 공유 줄 유실은 없앤다(§2-5) | — |
| 핸드셰이크 거절 | 전부 종결(`machine.rs:361`) | 셸: 토큰 불일치 = 실패 한 번(`connection.rs:891-894`) · 버전 불일치는 같은 길(팝업은 ADR-0180 결정 5 미착지) | D20 | **D20 ①** — `Retryable` / `Terminal` |
| 디코드 실패 | 프레임 버림 | 셸: 모르는 tag = 자리채움 · 짧은 프레임 = 끊음(`frame_relay.rs`) | 셸: 출력은 Wire 가 해독하지 않고 통과 → `frame_relay` 규칙 그대로 · JSON 실패는 버림(지금 `wait_for_hello` 와 읽기 루프도 버린다) · 데몬: 원문 통과라 transport 해독 실패 0(§3-3) | — |

---

## 8. 위험 · 모르는 것 · 사용자 결정 목록

### 8-1. 위험

1. **seq 줄기에 구멍(R1 · 가장 큼)** — 1판은 transport 공유 줄 유실(`transport/src/lib.rs:128-131`)을 다시 붙기로 메우려 했고 그 대책이 고리 · 예산 되감기 · 굶김을 만들었다(§3-0 ①). 2판은 유실 자체를 없앤다(3-0 U1). 남는 위험 = 배압이 데몬 나가는 큐를 채워 D13 끊김 → 전량 replay 가 잦아지는 것(셸 소비가 느릴 때) — 3-0 U1 · 3-2 U3 에서 replay 한 번이 손실 0 · 끊김 0 인지 잰다. 스트림 층을 켜는 실수도 같은 결말이다(D10).
2. **데몬 받는 쪽 의미 손실(R2)** — net 의 연결별 받는 줄 · 포화 처분(`Refused` 답 의무 · `Bypass` 취소 우선)을 3-0 U1 의 훅이 crate 로 옮긴다. 훅을 안 고르면(D11 ②) 줄이 찬 동안 취소가 앞을 못 지른다. 회귀망 = 3-3 U4 의 시험 이사.
3. **교체의 크기(R3)** — 셸 2,050줄 루프 + 5,978줄 시험 · 데몬 2,248줄 서버 행 · 2판은 3-0(transport 일곱 단위)을 더했다. 쪼갬(3-0 U1 · 3-2 U3/U4 · 3-3 U1/U4/U5) · 시험 운명표 · 시계(`ManualClock`)가 대책이다.
4. **replay 폭주(R4)** — 데몬 상대 정책을 기본값(512)으로 두면 replay 마다 끊김 · 재replay 고리(D16).
5. **요청 시한(R5)** — 30 s 로 두면 지금 상한이 없던 핸들러가 30 s 에 실패하기 시작한다 · 8 s 로 두면 느린 spawn 을 오판할 수 있다(실소요 미측정).
6. **Origin 값 · Host 검사(R6)** — 네 Origin 은 미실측이고 릴리스 값이 목록에 없을 가능성이 높다(D21) · Host 검사가 정당한 클라(`[::1]` 등)를 막을 수 있다 — 우리 클라는 `127.0.0.1` 로 건다(`connection.rs:435`)는 것만 확인했다.
7. **데몬 쓰기 시한 5 s 새로(R7)** — 셸 읽기가 5 s 멈추면 끊고 전량 replay. 2판 배압(R1)이 그 가능성을 키운다 — 셸 줄이 차 소켓 읽기가 서면 데몬 쓰기가 시한에 닿는다(미검 · 3-0 U1 시험이 잰다).
8. **겹친 번호 동작 변경(R8)** — transport 는 둘 다 실패(M1). `daemon_client_pending` 은 다시 쓴다 — 사용자 결정 2026-09-06 그대로라 새 결정은 아니다.
9. **「지금 확정」 표면을 다시 연다(R9)** — 2판에서 넓어졌다: `Inbound` · `Listener` · `Registry::attach` · `RequestError` 칸 · `Reject` 종류 · `close_after_flush` · `PeerCmd` 세대 · `AcceptPolicy`. 소비자 0 이라 깨지는 쪽이 없다 — 3-0 이 그 표면을 고칠 마지막 값싼 때다.
10. **답장 순서 변경(R10 · 새)** — 답장이 받는 줄을 건너 앞선 사건을 앞지른다(§3-3 B2). 그 순서에 기대는 셸 호출자가 있는지 모른다 — 3-2 U2 시험 + 호출자 조사.
11. **로컬 DoS(R11 · 새)** — 수락 상한(§5 14)은 같은 PC 의 다른 OS 사용자가 채울 수 있다. 알려진 한계로 받아들인다(원격 · 다중 사용자 트랙에서 재론).

### 8-2. 모르는 것 (미확인)

정당한 가장 큰 메시지(§5 15) · spawn 실소요(D15) · 핸들 쪽 큐 포화(`overflow` — `peer.rs:175` 「감독이 걷어 가 처리한다」)가 끊기까지의 지연 · net 결과 줄 둘이 정확히 몇인지 · T-52(`stop_smoke`) 원인 · WebView2 가 실제로 보내는 Origin · 셸 tungstenite 클라가 Origin · Host 를 어떻게 보내는지 · 배포 폴더의 상속 ACL(`icacls` — D22) · 답장 순서에 기대는 셸 호출자(R10) · tungstenite 가 업그레이드 뒤 크기 설정을 바꿀 수 있는지(§4) · 셸 시험 묶음 경계(§6-2 운명표).

### 8-3. 사용자 결정 목록 — 권고안이 첫째

표시: **[신설]** = 2판에 새로 든 결정 · **[추천 바뀜]** = 1판과 권고가 다름 · **[보강]** = 선택지 · 권고는 같고 명세가 늘었다.

**묶음 0 — 단계**(D19 ② 면 D1 · D8 · D9 의 구현이 3-3 으로 돌아간다)

- **D19 transport 보강 자리 [신설]** — ① 3-0 에 전부(소비자 0 인 지금 · 표면을 한 번 연다 · 3-1 과 병행) ② 3-0 = 셸 몫만 · 받는 쪽 seam · 수락 강화는 3-3 U3(표면을 두 번 연다) ③ 1판대로 각 단계 안(3-2 가 손실형 줄 위에서 시작 — 기각 사유 §3-0 ①).

**묶음 A — 구조**(A 의 답이 뒤를 바꾼다: D2 ② 면 D5 의 뼈대가 셸↔데몬 Wire 안으로 들어가고 D6 의 `Principal` 이 Auth 의 역할 칸에서 나온다 · D3 ③ 이면 D5 는 「서로 누군지 모르는」 연결 시험까지만 된다 · ★D4 ① 은 D11 ① 이 전제다★ — 받는 줄이 상대별이 아니면 시끄러운 데몬이 조용한 데몬을 굶긴다)

- **D1 받는 쪽 seam [보강]** — ① `Listener` + `Registry::attach`(시그니처 그대로 · 받은 상대 = Live 아닌 모든 끝에서 `Closed` + 스스로 내림 · 세대 표 밖) ② `Registry::new` 가 Listener 를 받아 수락까지(시그니처 변경 · 상한을 crate 정책으로) ③ 데몬이 명부 밖에서(ADR-0179 결정 3 위반).
- **D2 관계 가르기 [보강]** — ① 관계마다 Wire · Registry · Listener(어휘 격리 · 둘째 포트는 D5 ② 일 때만) ② 합집합 Wire + Auth 역할 칸(포트 하나 · 셸이 데몬끼리 어휘를 안다) ③ 한 포트 + 첫 프레임 엿보기(Link 감싸기가 든다).
- **D3 데몬 신원** — ① 영속 `DaemonId` + `Hello` 칸(파일 하나 · 대조는 셸 핸드셰이크 · 키는 나중에 묶음) ② 키 쌍 지금(안 쓰는 암호 의존 · 키 회전 = 신원 회전) ③ 이름표만(데몬끼리 · N 데몬의 재료 없음).
- **D4 셸 다중 데몬 깊이** — ① 어댑터 N · 윗층 primary · 가짜 데몬 둘 시험(구조가 실물로 선다 · 윗층 무변경) ② 타입 · seam 만(값쌈 · 셸 쪽 N 은 미검) ③ 화면 · LLM 까지(3단계 범위를 넘는다).
- **D5 데몬끼리 깊이 [보강]** — ① `PeerWire` 뼈대 + 인메모리 두 명부 시험(실 프로세스 · 둘째 포트 없음) ② ① + 실 둘째 Listener(기본 꺼짐) + 같은 PC 실 데몬 둘 + 거는 방향 · 대칭 깨기 ③ 나중 트랙(seam 만). 공통: 주장된 id · `LocalClient` 급 신뢰 · 자격 = 받는 쪽 토큰 · id 로 영속 신뢰 금지.

**묶음 B — 보안**(D6 ②' 이면 D9 의 속도 제한이 `admit` 구현 하나로 나중에 들어온다 · D20 은 D14 의 예산을 쓴다)

- **D6 인가 seam [추천 바뀜 ① → ②']** — ②' `admit` + `Principal` 보관 · 명령 입구 둘을 이름으로만 박음(권고) ① `Principal` + `allow` 를 입구 둘(`call_daemon_command` · `connection_core` dispatch)에 지금(ADR-0157 입구 시그니처 변경) ③ 없음.
- **D7 인증 자리** — ① 데몬 안 한 모듈 · 프레임은 protocol(소비자 하나에 맞음) ② 인증 crate(소비자 하나) ③ transport 안(ADR-0177 결정 4 위반).
- **D8 Origin · Host 검사 자리 [보강]** — ① transport ws 수락 설정(필수 정책 · `Default` 없음 · 값은 데몬 · transport 시험망 안) ② 데몬 자기 수락 + `split_link`(시험망 밖).
- **D9 새 방어 범위 [보강 · 범위 넓힘]** — ① 업그레이드 시한 · 수락 분리 · 상한 셋(대기 · 인증 · 전체 — RAII) · 양방향 16 MiB · Host 검사 · loopback 단언 · 수락 정책 필수 · 거절 보고 지금, 속도 제한 · TLS 연기 ② 옮기기만(동작 변화 0 · 구멍 그대로) ③ 속도 제한까지(loopback 에서 이득 적고 자기 DoS 위험).
- **D20 인증 실패 분류 [신설]** — ① 토큰 불일치 = `Retryable`(예산을 쓰고 접속 정보를 다시 푼다 — 지금 셸 동작) · 버전 불일치 = `Terminal`(ADR-0180 결정 5 를 버전으로 좁힘) ② 둘 다 `Terminal`(1판 · 결정 5 문자 그대로 — 낡은 자격이 버튼 없이 앉는다) ③ 둘 다 `Retryable`(버전 불일치에 예산을 태운다 — 결정 5 위반).
- **D21 Origin 정책 [신설]** — ① 릴리스 = Origin 달린 업그레이드 전부 거부 · dev 빌드만 목록 · 3-2 에서 `DaemonInfoDto` 토큰 걷기(운영 클라는 Origin 을 안 보낸다 — 가능성 높음 · 브라우저발 접속을 통째로 막는다 · 대가 = `wsTransport.ts` 직결 경로가 릴리스에서 죽는다) ② 1판 네 값 그대로(미실측 · 릴리스 Origin 이 목록에 없을 가능성) ③ 실측 뒤 목록 확정(3-3 U5 가 실측을 기다린다).
- **D22 `daemon.json` ACL [신설 · 1판 「연기」에서 올림]** — ① 만들 때 소유자 전용 DACL(platform 한 함수 · 배포판은 아무 폴더에나 풀려 상속 ACL 이 위치마다 다르므로 한 PC 실측이 답이 못 된다 · `icacls` 는 적용 확인용) ② `icacls` 실측 뒤 남이 읽으면 ① · 아니면 조건부 구멍으로 기록 ③ 1판대로 원격 때.

**묶음 C — 스트림 · 큐**(D10 ② 를 고르면 D11 · D13 과 §2-5 가 다시 써진다)

- **D10 transport 스트림 층** — ① 안 쓴다(불변식 그대로 · transport 기능 일부 놀림 · ADR-0177 결정 3 · 9 개정) ② 쓴다(replay_flight · 프론트 계약을 다시 — 3단계 범위 밖 크기).
- **D11 받는 줄 [추천 바뀜 — 1판 ①(어댑터가 공유 줄을 즉시 비움) → 2판 ①(transport 변경)]** — ① transport 상대별 무손실 줄 + 배압 + 포화 분류 훅(net 의미를 crate seam 으로) ② 상대별 줄 + 배압만(취소 우선 소실) ③ 공유 줄 await(한 상대가 전부를 막는다) ④ 1판(손실형 + 어댑터 대처 · 셸은 다시 붙기 — 고리 · 굶김).
- **D12 끊김 오류의 「보냈나」 [자리 이동 3-2 U1 → 3-0 U2]** — ① `RequestError::Disconnected` 에 칸 하나(지금 셸이 가르는 두 글 유지 · transport 표면 변경) ② 한 글로 합침(「안 나갔다」와 「결과 모름」을 사용자가 못 가른다).
- **D13 팬아웃 포화** — ① 그 연결을 끊는다(조용한 유실 결함을 끊김 + 전량 replay 로 · transport TRD §12-2) ② 버리고 신고(연결 유지 · 셸 화면이 어긋날 수 있다).

**묶음 D — 정책 값**(§7 — 서로 독립 · D14 는 D20 과 예산을 함께 쓴다)

- **D14 재연결 횟수** — ① 5 주입 → 팝업 버튼 착지 때 3(ADR-0180 결정 4 개정) ② 3 지금 ③ 3 + 버튼을 3-2 에.
- **D15 요청 시한** — ① 30 s 공용 + spawn 실측 → 8 s 는 따로(ADR-0181 결정 1 개정) ② 8 s 지금 ③ 명령별.
- **D16 데몬 나가는 큐 [근거 확인]** — ① 4608 유지(ADR-0177 근거 문단 개정 — §7) ② 링 이하(replay 고리).

**묶음 E — protocol**(3-1)

- **D17 `OwnerToken`** — ① protocol 사본(같은 바이트 · 타입 검사 남음) ② 원문 칸(등록 패킷 머리가 불투명해짐).
- **D18 원문 칸의 TS** — ① `#[ts(type)]` 덮어쓰기 유지(생성물 불변 · 손 문자열이 drift 위험으로 남음 — `messages.rs:1789` 가 이미 지키는 그것) ② `unknown`/JSON(ADR-0272 영향 그대로 · 프론트 소스 import 0 이라 영향 작음).

---

## 9. 문서 후속 · 미검

- **문서 후속(각 단위가 고친다 — 이 TRD 는 안 고친다):** CLAUDE.md(「백엔드 모듈 맵」 net · transport · protocol 항목 · 「네트워크 행 격리 게이트」 · 「빌드·검증 명령」 net 줄 · transport 시험 기능 줄 · 「의존성」 `tungstenite` 줄 — 3-1 U2 · U5 · 3-2 U2 · U5 · 3-3 U2 · U4 · U6, 각각 `/review doc`) · `.claude/skill-bindings/qa.md`(`:11` 예외 문단 · `:84-85` · `:280-285`) · `docs/testing-strategy.md` §net · `crates/engram-dashboard-transport/src/lib.rs` 헤더(공유 줄 포화 문단 `:128-131` · 받는 쪽 seam 한계 `:142-145` · 게이트 ① 문장) · `registry.rs` 헤더(`:6-9`) · `transport/src/ws.rs` 「알려진 한계」 · `src-tauri/Cargo.toml:79-85` · `:94-98` 주석 · 메모 §10 3. 착지 표시 · §11 처리 표시(`:327` · `:332` · `:345`) · `docs/tracking.md`(제어 평면 Host 검사 항목 · T-52 결과 · R11 로컬 DoS) · step-log(오케스트레이터).
- **미검(이 판):** 빌드 · 시험을 하나도 돌리지 않았다 · 기준선 회귀 수(3-0 U1 · 3-1 U1 착수 직전에 잰다 · CLAUDE.md 마지막 기록 = 결과 줄 60 · 4371 통과 · 31 무시) · §8-2 전부 · 3-0 U1 의 감독 select(줄 자리 대기 중 제어 · keepalive · 쓰기를 돌리는 것)가 지금 루프 구조에 그대로 얹히는지(가능성 높음 — 설계 판독만) · `RawValue` 가 외부 태그 enum 안에서 바이트를 지키는지(ADR-0272 영향 — serde-rs/json#497 · 3-1 U1 golden 이 잰다) · `Host` 헤더 실제 값(tungstenite 클라가 보내는 꼴) · net 시험 22개 중 어댑터로 옮길 수 · 같은 PC 데몬 둘의 데이터 폴더 격리(`ENGRAM_DATA_DIR` — `stop_smoke` 선례만 확인).
