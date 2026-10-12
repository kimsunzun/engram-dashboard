# 전송 보안 — 자작 transport 유지 vs 기성 라이브러리 채택·감싸기 (2026-10-12)

- 상태: **적대 리뷰 FIX 반영 완료(재판정 미실행)** · 결정 없음(사용자 선택 대기)
- 방법: `/research` medium · 설계-결정 모드 · 수집 4갈래(일반 프로그램 피어 · 앱 수준 Rust RPC/WS 라이브러리 · 보안 채널/원격 계층 · 우리 현황 코드 실측) → 메인 합성 + 출처 대조(grounding) → cross-family 적대 리뷰(codex)
- 계기: 사용자 질문 — "보안 쪽이 잘 돼 있는 라이브러리가 있으면 우리가 구현할 필요가 없지 않나(혹은 감싸서 문제 있으면 교체)". 외부 통신이 되면 고려할 것이 곱절이다.
- 비교 기준: 에이전트 제품이 아니라 **범용 프로그램 기준**(사용자 지시 2026-10-12). 에이전트·터미널 피어는 모양이 가장 가까워 참고로만 남긴다.
- 확신도 범례: **확실**(출처 대조 + 독립 교차확증) · **가능성 높음**(출처 대조, 단일 출처) · **불확실**(미대조·기억·제3자 집계) · **contested**(반증 있음)
- 앞선 조사: `docs/research/reusable-transport-crate-boundary-2026-09-03.md`(연결 살림 축 — 이 문서는 **보안 축**만 다시 본다) · ADR-0177

## 0. 결론 먼저

1. **보안을 통째로 해 주는 라이브러리는 없다.** 보안은 층이 넷이고, 층마다 답이 다르다 — 그래서 "라이브러리 하나 고르기"가 아니라 **층마다 무엇을 빌리고 무엇을 우리가 하나**의 문제다. `가능성 높음`
2. **빌리는 게 확실히 이득인 층 = 암호화·원격(③).** 범용 프로그램 대부분이 원격 구간을 **이미 있는 보안 채널**(SSH · 상호 TLS/인증서 고정 · 종단 암호화 중계 · Tailscale 같은 오버레이)에 맡긴다. 직접 만든 곳을 못 찾았다. 우리 transport 의 `Dialer`/`Link` 접점이 정확히 그 자리라 **감싸서 쓰고 문제 있으면 갈아끼우기**가 구조상 이미 된다. `가능성 높음`
3. **빌려도 결국 우리가 켜고 정해야 하는 층 = 브라우저발 로컬 공격 방어(①) · 인증(②) · 페어링.** Origin 검사를 **기본으로 켜 주는** 라이브러리는 없었다 — Salvo 처럼 허용 목록 옵션을 주는 곳은 있으나 기본 꺼짐이다(적대 리뷰가 찾음 · 메인 대조). 로컬호스트 WS 에 Origin 검사가 없는 것은 이 부류에서 **실제 CVE 가 반복된 실패**다. 단 그 CVE 들은 **인증 없는** 끝점이었고 우리는 첫 프레임 토큰이 있어 같은 경로는 아니다 — Origin 은 우리에게 **2중 방어**다. 우리 `net` 은 이미 막고 있지만 **transport 의 `WsListener` 에는 없다** — 그대로 갈아끼우면 그 한 겹이 빠진다. `확실`
4. **DoS·자원(④)은 반반.** 라이브러리는 프레임 크기 · 시한 정도를 주고, 연결 수 상한 · 속도 제한은 대개 쓰는 쪽 몫이다. 우리 쪽에 실측된 구멍이 있다(§4). `가능성 높음`
5. **앱 수준 RPC 프레임워크(jsonrpsee · tonic · tarpc 등)를 통째로 채택하는 안은 이득이 작다** — wire 형식을 바꿔야 하는데(ADR-0177 결정 5) 정작 인증 · Origin 은 여전히 우리 몫이다. `가능성 높음`

## 1. 층별 지도 — 무엇을 빌리고 무엇을 하나

| 층 | 무엇을 막나 | 범용 프로그램 관행 | 빌릴 수 있나 | 우리 현황 |
|---|---|---|---|---|
| ① 로컬 브라우저발 공격 | 사용자가 연 악성 웹페이지가 `127.0.0.1` 포트로 붙음(Origin) · DNS 리바인딩(Host) | Origin/Host 허용 목록 · SameSite 쿠키. 안 한 곳이 CVE | **△** — 훅이나 기본 꺼진 옵션(Salvo)만 준다 · 값은 우리가 정한다 | `net` = Origin 허용 목록 있음(Host 검사 없음) · transport = 없음 |
| ② 인증 | 포트에 닿은 남이 명령을 보냄 | 서버가 난수 비밀 생성 · 상수시간 비교 · 새 기기는 페어링 | **✕** — 직접 짠다(원시 부품만 빌림) | 256비트 CSPRNG 토큰 · 첫 프레임 1초 · 상수시간 비교 |
| ③ 암호화·원격 | 네트워크 도청 · 중간자 · NAT 너머 접속 | **기존 보안 채널에 맡김**(SSH · mTLS/인증서 고정 · E2E 중계 · Tailscale) | **○** — 이 층이 빌리는 자리 | 없음(평문 `ws://`) — 지금은 loopback 전용이라 문제 아님 |
| ④ DoS·자원 | 연결 폭주 · 거대 프레임 · 느린 상대 | 프레임 상한 · 시한 · 연결 상한 | **△** — 일부만 | §4 의 구멍들 |
| (페어링) | 새 기기를 처음 믿는 절차 | QR · 페어링 URL · 기기 ID 승인 | **✕** | 없음 |

## 2. 범용 프로그램은 어떻게 하나 (피어)

### 2-1. 범용 프로그램

| 프로그램 | 로컬 인증 | 원격 구간 | 로컬 강화 | 출처 · 확신도 |
|---|---|---|---|---|
| Syncthing | 기기 ID = 인증서 SHA-256, 승인 목록과 대조 | TLS 종단간 — **중계는 암호문만 본다** | GUI 는 기본 localhost | docs.syncthing.net/users/security — `확실`(메인 대조) |
| Docker Engine | 소켓 파일 권한 | 상호 TLS(`--tlsverify`) 또는 `ssh://` | 기본은 로컬 소켓만, TCP 끔 | 수집자 기억 — `불확실` |
| VS Code Remote Tunnels | 계정 인증(GitHub/MS) | 벤더 터널 + 안쪽 SSH | — | 2차 출처 — `불확실` |
| code-server | 비밀번호 → 쿠키 | TLS 선택 · 리버스 프록시 권장 | 4.10.1 에서 **Origin 검사 추가**(CVE-2023-26114) | wiz.io — `확실`(메인 대조) |
| Jupyter Server | 난수 토큰 → 쿠키 · XSRF | TLS 선택 · SSH 포워딩 | Host/Origin 검사 | blog.jupyter.org · CVE-2022-29241 — `가능성 높음` |
| Home Assistant | 액세스 토큰 | Nabu Casa 중계(SNI 라우팅, TLS 종단간) | — | support.nabucasa.com — `가능성 높음` |
| OBS WebSocket | 챌린지-응답(비밀번호 자체는 안 보냄) | 없음 | 비밀번호 선택 | 프로토콜 문서(v4.5) — `가능성 높음` |
| Chrome DevTools | **인증 없음**(포트에 닿으면 전권) | SSH 포워딩만 | Chrome 111 = 모르는 Origin 거부 · 136 = 기본 프로필에서 포트 무시(쿠키 탈취 악용 때문) | developer.chrome.com — `확실`(메인 대조, 136 부분) |
| Redis | AUTH/ACL | TLS 선택(6.0+) | 보호 모드 · `POST`/`Host:` 를 QUIT 로(브라우저 교차 프로토콜 공격 대응) | lwn.net — `가능성 높음` |

### 2-2. 같은 모양의 피어(참고)

| 피어 | 스택 | 인증 | 원격 | 로컬 강화 | 출처 |
|---|---|---|---|---|---|
| zellij web | axum + rustls | UUIDv4 토큰 SHA-256 해시 저장 · HttpOnly SameSite=Strict 쿠키 · 읽기 전용 토큰 | rustls | CSP · SameSite | 로컬 클론 `zellij-client/src/web_client/` — `가능성 높음` |
| paseo | express + ws | bcrypt 비밀번호 · 상수시간 비교 | 중계 + NaCl box 종단 암호화 + 페어링 | Host 허용 목록(DNS 리바인딩) | 로컬 클론 `packages/server/src/server/auth.ts` — `가능성 높음` |
| t3code | Effect node | 일회용 만료 페어링 → HMAC 서명 쿠키 · DPoP | SSH · Tailscale 패키지 | 포트별 쿠키 이름 | 로컬 클론 `apps/server/src/auth/` — `가능성 높음` |
| orca | Electron 자체 RPC | QR 페어링 토큰 회전 | 중계 + tweetnacl box | 모름 | 로컬 클론 `src/shared/e2ee-crypto.ts` — `가능성 높음` |
| herdr | `interprocess` 로컬 소켓 | 소켓 권한(추정) | SSH | 네트워크 미노출 | 로컬 클론 `Cargo.toml` — `가능성 높음` |
| ttyd | libwebsockets | Basic 인증 · 프록시 헤더 | 내장 TLS · 클라 인증서 | `--check-origin` 있으나 **기본 꺼짐** | 로컬 클론 `src/server.c` — `가능성 높음` |

### 2-3. 이 부류의 사고 (로컬호스트 WS · 포트 노출)

- **CVE-2026-44211** Cline `kanban` — `127.0.0.1:3484` WS 에 Origin 검증 없음 → 악성 웹사이트가 에이전트 터미널 탈취. 2.13.0 이하. advisories.gitlab.com — `확실`(메인 대조)
- **CVE-2023-26114** code-server — WS 핸드셰이크 Origin 검증 없음, CVSS 9.3, 4.10.1 수정. wiz.io — `확실`(메인 대조)
- **CVE-2025-24964** Vitest — WS Origin 검사 없음 → 테스트 파일 저장·재실행으로 RCE. db.gcve.eu — `가능성 높음`
- **CVE-2026-59723** Cline Hub — `/browser` Origin 검사 없음 · 127.0.0.1 바인드 시 방 비밀 없음. opencve.io — `가능성 높음`
- **CVE-2016-10517** Redis — 브라우저가 보낸 HTTP 를 명령으로 일부 해석. lwn.net — `가능성 높음`
- Chrome 원격 디버깅 포트 — App-Bound Encryption 뒤 쿠키 탈취에 악용 → 136 에서 제한. developer.chrome.com — `확실`(메인 대조)
- 미대조: CVE-2026-77359 · CVE-2026-77318(Jupyter MCP 서버 DNS 리바인딩) · CVE-2026-86049(Jupyter 토큰 로그 노출) — `불확실`

## 3. 라이브러리 후보

### 3-1. 앱 수준 RPC / WS (②·④ 층 후보)

| 후보 | 인증 | TLS | Origin/Host | 크기·연결·속도 상한 | wire 고정? | 유지보수 · 권고 | 라이선스 |
|---|---|---|---|---|---|---|---|
| tokio-tungstenite 0.30.0(지금 씀) | 없음(콜백) | feature | 없음(콜백에서 직접) | 메시지 64 MiB · 프레임 16 MiB 기본 · 연결·속도 없음 | **아니오** — 임의 프레임 | 2026-07-11 · RUSTSEC-2023-0065(≤0.20.0, 핸드셰이크 헤더 DoS — 이미 지난 판) | MIT |
| axum 0.8.9 ws | 미들웨어 | hyper/rustls 바깥 | WebSocketUpgrade 문서에 없음 | 64 MiB/16 MiB | 아니오 | h2 권고 이력 | MIT(미확인) |
| fastwebsockets 0.10.0 | 없음 | 문서 없음 | 없음 | 문서 없음 | 아니오 | 2026-10-05 · 퍼징 · Autobahn 통과 · Windows 미실측(docs.rs 가 기본 타깃만 짓는 정책이라 문서 타깃으로는 판정 못 함 — 적대 리뷰) | Apache-2.0 |
| Salvo(WebSocketUpgrade) | 프레임워크 미들웨어 | 프레임워크 | **`allowed_origins()` · `check_origin()` — 기본 꺼짐**(문서가 「검사 없으면 방문한 아무 사이트나 인증된 WS 를 연다」고 경고) | 미확인 | 아니오 — 앱 메시지 그대로 | 미확인 | 미확인 |
| tower-mcp(참고) | — | — | Origin · Host 검증 + 세션 상한을 명시적으로 갖춤 | 세션 상한 | 예(MCP) — 패턴 참고용 | 미확인 | 미확인 |
| jsonrpsee 0.26.1 | 못 찾음 | 서버 파일에 없음 | 미확인 | 연결 100 · 본문 10 MiB · 배치 무제한 기본 · ping 기본 꺼짐 | **예** — JSON-RPC 2.0 | 2026-09-30 | MIT |
| tonic 0.14.6 | 인터셉터 | `tls_config` | 없음 | 동시 스트림 · 연결당 동시성 · 연결 수명 | **예** — gRPC/protobuf | 2026-09-24 · RUSTSEC-2024-0376(accept 오류로 서버 종료 DoS, ≥0.12.3 수정) | MIT(미확인) |
| tarpc 0.38.0 | 못 찾음 | 옛 문서에 feature | 없음 | 미확인 | **예** — 자체 serde 프레이밍 | 2026-08-12 | MIT |
| ezsockets 0.7.2 | 없음 | rustls/native-tls | 문서 없음 | 미확인 | 미확인 | 2026-05-18 · 자동 재연결 | MIT |
| bevy_simplenet 0.16.0 | 서버 쪽 클라 인증 | 선택 | 미확인 | 미확인 | 자체 채널 API | **2025-04-26 이후 정체** | MIT/Apache-2.0 |
| zenoh | TLS · mTLS · 사용자 · ACL | TLS/QUIC | — | 전송 허용 목록 · ACL | **예** — 자체 프로토콜 | 미확인 | Apache-2.0/EPL-2.0 |

출처: tungstenite 설정 = doc.servo.org(tungstenite `protocol/mod.rs`) · axum/tonic/fastwebsockets = docs.rs · jsonrpsee 기본값 = GitHub `server/src/server.rs` · 판·날짜 = crates.io API · 권고 = rustsec.org. **RUSTSEC-2023-0065 · RUSTSEC-2024-0376 은 메인이 원문 대조** — 나머지 판·날짜는 수집자 단일 출처 `가능성 높음`. h2 권고(RUSTSEC-2024-0332 · RUSTSEC-2025-0034)는 검색 요약만 — `불확실`.

### 3-2. 보안 채널 · 원격 계층 (③ 층 후보 — 우리 `Dialer`/`Link` 아래)

| 후보 | 지키는 것 | 상대 인증 | NAT·중계 | 끼우는 모양 | 성숙도 · 감사 · 권고 | 라이선스 |
|---|---|---|---|---|---|---|
| rustls + tokio-rustls | TLS 기밀성·무결성 | CA/PKI · mTLS · 커스텀 검증기로 키 고정 | 없음(도달성은 우리가) | WS 아래 바이트 스트림 | **Cure53 감사(2020)** · RUSTSEC-2024-0336(무한 루프, 0.23.5 등 수정) · RUSTSEC-2024-0399(조각난 ClientHello 패닉 — **0.23.13~0.23.17 영향 · 0.23.18 수정** · tokio-rustls `LazyConfigAcceptor` 영향 · `TlsAcceptor` 무영향, 메인 대조) · 감사는 2020 판 대상이지 지금 판의 감사가 아니다 | Apache-2.0/MIT/ISC(미확인) |
| iroh 1.3.0 | QUIC + TLS 1.3 · 중계는 암호문만 | **공개키로 상대를 인증** — ★그 상대를 받아 줄지(인가)는 앱 몫이라고 문서가 명시★(적대 리뷰) | 홀펀칭 + 중계 폴백 · 기본 프리셋은 **공용 인프라** — 운영은 전용 인프라를 권고(그 문서) → 중계·메타데이터·가용성 결정이 따로 든다 | QUIC 스트림·데이터그램 → 우리 JSON·바이너리를 나르려면 **메시지 경계를 따로 정해야** 한다(WS 프레이밍 가정 수정) | 1.3.0 = 2026-09-28(메인 대조) · **감사 못 찾음** · 권고 미확인 · Linux/macOS/**Windows/Android/iOS** 지원 명시(docs.iroh.computer/compatibility — 메인 대조) | Apache-2.0/MIT(메인 대조) |
| snow(Noise) | AEAD · 상호 인증 | 정적 키 · PSK | 없음 | 바이트 스트림 위에 직접 구동 | **정식 감사 없음(README)** · CVE-2024-58265(논스 증가 DoS, 0.9.5 수정) | Apache-2.0/MIT |
| russh | SSH 채널 | 호스트 키 · 사용자 키 | 없음 | 채널 · tokio | 0.62.4 · CVE 번호는 제3자 집계만 — `불확실` | 미확인 |
| libp2p | Noise/TLS 1.3 | 키에서 나온 peer id | 중계 · 홀펀칭 | 다중화 스트림 | 감사 언급은 포럼 수준 · RUSTSEC-2022-0009 | 미확인 |
| tsnet / libtailscale 내장 | WireGuard 오버레이 | Tailscale 신원(제어 평면) | Tailscale DERP · **계정 필요** | Go 런타임 또는 C 라이브러리 · Rust 래퍼 0.1.0 | 미성숙 래퍼 | BSD-3(libtailscale) |
| 사용자의 Tailscale/WireGuard 에 기대기 | 오버레이 | 사용자 네트워크 | 사용자 네트워크가 해결 | 데몬은 tailnet IP 에 바인드할 뿐 | 우리 코드 0 | — |

**메인 관측(추론):** 가장 싼 원격 경로 = **rustls(인증서 고정 또는 mTLS) + 사용자 VPN**. NAT 를 우리가 풀어야 하면 = **iroh**. 둘 다 transport 의 `Dialer`·(새) `Listener` 자리에 끼우고 위 연결 살림은 그대로 쓴다.

### 3-3. 페어링 (어느 층도 대신 안 해 줌)

- Syncthing: 기기 ID(인증서 해시) 교환 · QR 선택 — 그 ID 가 고정 신원이다. `가능성 높음`
- t3code: 일회용 만료 페어링 그랜트 → 서명 쿠키. `가능성 높음`(로컬 클론)
- orca · paseo: QR 페어링 → 중계 위 종단 암호화. `가능성 높음`(로컬 클론)
- OpenClaw: `openclaw qr` 가 QR + 설정 코드 · **Tailscale/공개 호스트의 평문 `ws://` 에선 거부하고 `wss://` 를 요구**. docs.openclaw.ai — `가능성 높음`
- magic-wormhole 의 SPAKE2(짧은 코드) — 출처 미확보 · `spake2` crate 0.4.0 · 감사 못 찾음 — `불확실`

## 4. 우리 현황 (코드 실측 · 2026-10-12)

**지금 운영 경로(`net` + 데몬 + 셸 `daemon_client`)가 가진 것:** loopback 바인드(`127.0.0.1:0`) · 256비트 CSPRNG 토큰 · 첫 프레임 인증 1초 시한 · **상수시간 비교**(메인 대조 `crates/engram-dashboard-net/src/ws.rs:291`) · Origin 허용 목록(Origin 없는 요청은 통과 · 네 허용값은 **미실측 설계값** — 그 파일이 자인) · ping 20초/무소식 50초.

**구멍(원격이 되면 커지는 순):**
1. **암호화 없음** — 셸은 평문 `ws://`. 원격이면 토큰이 평문으로 지나간다. transport 도 `wss://` 를 안 켰다.
2. **WS 업그레이드에 시한 없음** — `accept_hdr_async` 가 인증 시한(1초) **앞**에서 시한 없이 돈다 → TCP 만 열고 아무것도 안 보내는 상대가 태스크를 무한히 쥔다(메인 대조 `ws.rs:323` → `:331` 순서. **실측 아님**, 코드 순서 판독).
3. **연결 수 상한 없음** — 수락 루프가 연결마다 태스크를 상한 없이 띄운다.
4. **인증 실패 속도 제한 없음** — 실패는 그 소켓만 닫는다.
5. **메시지 상한이 tungstenite 기본 64 MiB 그대로** — 그 파일이 「동작 변경이라 별건」으로 남겼다.
6. **`net` 은 쓰기 시한이 없다**(알려진 구멍) — transport 는 5초로 있다.
7. **토큰은 파일 위치로만 지킨다** — `daemon.json` ACL 미설정 · 부팅마다 회전 · 에이전트 제어 토큰은 환경변수 · 기본 ACL 파일.
8. Host 헤더 검사 없음(DNS 리바인딩) — 제어 HTTP `/mcp` 는 rmcp 기본 `allowed_hosts` 에 기댄다고 주석만 있고 `/control/*` 는 검사 못 찾음.

**transport crate 는 `net` 보다 연결 살림(쓰기 시한 · 유계 큐)은 낫고 가장자리 보안은 약하다:** 인증은 소비자의 `Handshake` 몫(의도) · Origin 검사 없음 · `WsListener::bind` 는 아무 주소나 받음 · 한 상대의 느린 핸드셰이크가 다른 수락을 막음(그 파일 자인).

## 5. 제약 적합도 (후보 × 우리 제약)

제약 출처 = CLAUDE.md 「아키텍처 원칙」(교체 가능 추상 · 코어 격리 · 플랫폼 중립) · ADR-0177 결정 5(wire 보존) · 라이선스 MIT/Apache 선호(확정 판단은 담당 부서 몫).

| 안 | wire 보존 | 교체 가능 seam | Windows | 보안 이득 | 비용 |
|---|---|---|---|---|---|
| **A. transport 유지 + ③ 층만 기성(rustls 지금 · iroh/SSH/VPN 은 원격 때) + ①② 는 `net` 것을 옮겨 옴** | ○ | ○ (`Dialer`/`Listener` 그대로) | ○(rustls) · iroh 미확인 | 암호화·상대 인증을 감사된 부품에 맡김 | Listener 접점 · Origin/시한/상한 이식 |
| B. tonic 채택 | ✕ (gRPC) | △ | ○ | TLS·동시성 상한 | wire 전면 교체 · 인증·Origin 여전히 우리 몫 |
| C. jsonrpsee 채택 | ✕ (JSON-RPC 봉투) | △ | 미확인 | 연결·본문 상한 | 같음 + 기본값이 느슨함 |
| D. iroh 를 링크로 통째 사용 | △ (내용은 그대로, 메시지 경계는 새로 정함) | ○ | ○(문서상 — 미실측) | 암호화 · 키 인증 · NAT(인가는 여전히 우리) | 로컬 전용에도 QUIC 를 강제 · 감사 없음 · 중계 인프라 결정 |
| E. 아무것도 안 빌림(TLS 도 직접) | ○ | ○ | ○ | — | 암호 구현 자작 = 금기 |

**거부 후보(→ ADR 거부 대안 후보):** jsonrpsee · tonic · tarpc · irpc · zenoh · capnp-rpc = 자기 봉투/전송 강요(wire 보존 위반) · bevy_simplenet = 2025-04 이후 정체 + 자체 채널 API · tsnet 내장 = Go 런타임 · 미성숙 래퍼 · 계정 필요 · snow = 감사 없음 + NAT 없음(TLS 로 충분할 때 고를 이유 약함).

## 6. 쟁점 · 한계

- **iroh 의 감사 · RustSec 이력 미확인** — D 와 A 의 원격 단계가 여기에 기댄다(플랫폼 지원은 문서로 확인됨 · 실측은 아님).
- **페어링 선례 보강 필요** — magic-wormhole 프로토콜(짧은 코드 + SPAKE2)이 성숙한 설계 비교 대상이다. Rust 구현 crate 는 EUPL 이라 선호 라이선스 밖(확정 판단은 담당 부서) — 프로토콜 설계만 참고 후보.
- fastwebsockets 의 Windows 동작은 미실측(제외 사유가 아니라 열린 항목).
- **Docker · PostgreSQL 행은 수집자 기억** — 결론(원격은 기존 채널에 맡김)은 다른 행만으로도 선다.
- **DoS 축은 피어 문서가 거의 없다** — 피어 비교가 안 되는 축이라 §4 의 구멍은 우리 판단으로만 정한다.
- **russh CVE 번호 · h2 권고 일부는 제3자 집계 · 검색 요약** — 결론에 안 쓴다.
- **우리 코드의 「업그레이드 시한 없음」은 코드 순서 판독이지 실측이 아니다.**
- 미조사: Plex/Jellyfin · Steam Remote Play · Coder · Cloudflare Tunnel · boringtun · remoc · actix-ws.

## 7. 적대 리뷰 결과

- 리뷰어: codex(cross-family) · effort high · 레벨 2~3 · 웹 검색 켬. ★입력에서 §4(우리 코드의 파일:줄 · 구멍 세부)는 빼고 요약만 보냈다★(내부 정보 외부 반출 최소화).
- 판정: **FIX** — 8건. 메인이 핵심 셋을 원문 대조(RUSTSEC-2024-0399 판 범위 · iroh 플랫폼 표 · Salvo Origin 옵션) → 셋 다 리뷰어가 맞다.
- 반영:
  1. (높음 · 누락) Salvo 의 Origin 허용 목록 — 결론 3 · §1 · §3-1 고침. 「기본으로 켜 주는 곳 없음」은 유지(Salvo 도 기본 꺼짐).
  2. (높음 · 틀린 판) RUSTSEC-2024-0399 = 0.23.13 이 **도입**, 0.23.18 이 수정 · API 범위 — §3-2 고침.
  3. (중간 · 근거 없는 제외) fastwebsockets 의 Windows 제외 사유 철회 — §3-1 · §5 고침.
  4. (높음 · 논리 공백) iroh 는 인증만 하고 인가는 앱 몫 — §3-2 · §5 고침.
  5. (중간 · 과한 불확실) iroh Windows/Android/iOS 지원 명시 — §3-2 · §6 고침.
  6. (중간 · 누락) iroh 운영은 전용 중계 인프라 권고 — §3-2 고침.
  7. (중간 · 과장) CVE 들은 인증 없는 끝점 — 우리에게 Origin 은 2중 방어 — 결론 3 고침.
  8. (중간 · 논리 공백) iroh 에서 「wire 보존」은 메시지 경계를 새로 정해야 한다 — §3-2 · §5 고침.
- 리뷰어 대조(지지): tungstenite 64 MiB/16 MiB 기본 · axum 0.8.9 같은 기본 · RUSTSEC-2023-0065 · RUSTSEC-2024-0376 · code-server 4.10.1 Origin · Cline kanban · rustls Cure53 2020(단 지금 판의 감사 아님).
- 리뷰어가 더한 후보: Salvo · tower-mcp(패턴 참고) · magic-wormhole 프로토콜(페어링).
- 결론 1·2·5 와 추천 방향(A)은 리뷰가 때리지 않았다 — 통과는 증명이 아니라 반증이 안 나왔다는 뜻이다.
