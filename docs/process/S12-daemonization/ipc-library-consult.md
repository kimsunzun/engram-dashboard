# S12 데몬화 — IPC 라이브러리 실조사 + 아키텍처 판단 (consult 교차검증)

날짜: 2026-06-14. 방법: `/consult` — GPT·Gemini·Claude-Opus 3종 블라인드 교차검증 + 전용 judge 판정. 원자료: `I:\Engram\agents\web-runner\shared\20260614-042121-consult-daemon-ipc-lib\`.

## TL;DR — 구현 보류 결정

**턴키(turnkey) 라이브러리는 없다. 핵심 불변식(replay→live 순서·seq dedup·epoch 재구독)은 어떤 성숙 라이브러리도 대신 못 하므로 커스텀 구현이 불가피하고, 규모는 MVP 1~3주 + production-ready 3~6주+다.** 사용자 지시("적당한 성숙 라이브러리가 없어 구현이 커지면 하지 말 것")에 따라 **착수 보류** — 아래 결정 지점을 사용자가 정한 뒤 진행.

3종 모델 + judge가 이 점에 **만장일치**: 라이브러리는 "전송선(transport)"만 대줄 뿐, 정확성은 데몬 session core의 책임. (각 라이브러리 공식 문서가 명시한 한계로 검증됨 — 추측 아님.)

## 합의된 결론 (judge가 비판 검증 후 "옳음" 판정)

1. **replay 위치 = 데몬 자체 보유** (현 OutputCore/ring buffer가 source of truth). broker(JetStream/Centrifugo)에 위임 X — PTY 바이트는 휘발성·세션수명 한정이라 외부 영속 broker 동기 약함. broker recovery는 size/TTL/limit 초과 시 실패(Centrifugo recovered:false) → 어차피 자체 fallback 필요. (durable 필요해지면 그때 daemon이 seq 붙은 frame을 JetStream에 *mirror*, authoritative ordering은 데몬 유지.)
2. **경로 = B-contract-first** (JS↔데몬 WS 직결). 처음부터 데몬이 WS 서버를 열고 envelope를 B(모바일 직결) 기준으로 고정. 초기엔 Tauri 앱이 localhost로 붙는 형태로 시작(A의 안정성 + 단일 WS로 A/B 흡수). **tarpc/jsonrpsee로 A를 따로 구현하지 말 것**(JS 미도달 → B 전환 시 폐기). §5(LLM-우선 단일 control surface)와도 정합.
3. **커스텀 구현 필요성 = YES (불가피).** 단 OutputCore에 이미 seq/replay/epoch/subscribe 로직이 있어 0부터는 아님 — "와이어로 노출"이 핵심 작업.

## 갈린 쟁점 — judge 판정

| 쟁점 | 판정 | 근거 |
|---|---|---|
| **1순위 라이브러리** | **raw `tokio-tungstenite` + 자체 얇은 프로토콜** (조건부) — Claude 입장 채택 | 핵심 불변식을 어차피 자체 구현해야 하고, Socket.IO는 프로토콜 lock-in이라 "교체 가능" seam 원칙과 마찰. "얇은 WS + 기존 OutputCore 노출"이 라이브러리 모델 역매핑보다 깨끗. **단 사용자 결정 지점**(아래). |
| **socketioxide CSR** | Claude 가장 정확 / Gemini 오류 | socketioxide GitHub feature 목록에 Connection State Recovery **부재**(judge가 WebSearch로 실확인). JS Socket.IO는 4.6.0~ CSR 있으나 socketioxide(별도 Rust 구현)엔 없음. 있어도 best-effort라 불변식 ①(누락 0) 보장 못 함 → 이 프로젝트엔 **무의미**. |
| **공수 추정** | GPT 구간추정이 현실적 / Gemini 과소추정 | Gemini "2~3주"는 데몬 프로세스 생명주기·backpressure 누락. 현실: **MVP 1~3주, production-ready 3~6주+**(하드닝이 본체). 데몬 관리(단일 인스턴스·좀비·포트·Job Object 데몬 이전)는 별도 공수. |

## 탈락 (3종 + judge 일치)
- **tarpc** — JS/브라우저 미도달(Rust↔Rust 전용). 경로 A 구간만 가능한데 B 전환 시 폐기.
- **tonic/gRPC + grpc-web** — 브라우저에서 client/bidi 스트리밍 불가, PTY 입력 못 실음.
- **jsonrpsee 재연결 래퍼** — 공식이 "구독 메시지 손실, 어떤 게 손실됐는지 알 수 없음" 명시 → 불변식 ①②와 정면 충돌.
- **NATS / nats.ws** — async-nats(Rust 서버)는 훌륭하나 **브라우저 클라 nats.ws가 2026-05-08 아카이브** → 모바일 WS attach 1차 타겟(JS)에 치명적.

## ★ 사용자 결정 필요 지점 (착수 전)

1. **라이브러리: raw WS vs Socket.IO+socketioxide.** judge 판정은 raw WS(lock-in 회피·의존성 최소·seam 교체성). 단 "자동 재연결·바이너리 프레이밍·네임스페이스 기성 편의"가 더 가치 있다고 보면 Socket.IO+socketioxide도 합리적. **양쪽 다 transport seam 뒤에 숨기면 후회 비용 낮음** — 나중에 교체 가능.
2. **모바일 원격 직결의 TLS·인증 신뢰경계** (3종 다 약하게 다룸). loopback 토큰은 localhost 전용. 원격 단계 설계를 미리 envelope에 반영할지.
3. **backpressure 정책** (GPT만 명시). 느린/끊긴 클라가 붙어 있을 때 ring buffer 상한·드롭/블록 정책 — 고-throughput PTY에서 메모리 안전 핵심.

## 모델별 결정적 오류 (참고)
- **Gemini:** CSR/재연결 혼동(Q1 "수동 상태머신 불요" ↔ Q5 "100% 커스텀" 자가모순), 공수 과소추정, 1순위 불명확(Socket.IO·Axum 동급 병기).
- **GPT:** 사실 오류 0, Centrifugo Windows 바이너리 "모름"으로 정직. 약점: Socket.IO 1순위 권하며 lock-in 인정 → seam 원칙과 미묘한 충돌.
- **Claude:** 사실 오류 0(검증한 모든 주장 실측 일치), 가장 정확. 약점: 모바일 인증/backpressure 얕음, 공수 숫자 회피.

## 2차 OSS 선례 조사 (2026-06-14, 별도 research) — "공용화 있나?" 검증

사용자 의문("데몬 예제는 비슷한 부분 많을 텐데 왜 공수가 높나, 오픈소스 데몬 확인해봐"). 1차 출처 조사 결론:
- **턴키 Rust 크레이트 없음(확실).** WezTerm `codec`/`mux`=`publish=false`(내부코드), `zellij-server`=Zellij 전용, Eternal Terminal=C++. 데몬 본체를 끼우는 통짜 라이브러리는 어디에도 없음.
- **재사용 가능한 "배관"만:** `portable-pty`(이미 사용), `tokio-tungstenite`(WS), `interprocess`(크로스플랫폼 IPC, zellij 사용). JS: `@xterm/addon-attach`(WS↔터미널 파이프, **재연결 로직 없음**), `@xterm/addon-serialize`(클라 스냅샷, 우린 서버 replay라 선택).
- **replay→live 알고리즘 = 보편 패턴, 우리가 이미 보유.** Eternal Terminal `BackedWriter`: `재전송량 = serverSeq − clientLastSeq`, 최신 N개 역순→reverse 재전송 → gap·dup 구조적 방지. **OutputCore가 동형 구현.** ET/VS Code가 우리 패턴이 표준임을 교차검증. → 발명 아닌 복제 수준, de-risk됨.
- **진짜 공수 = 알고리즘 아니라 인프라(끼울 라이브러리 없음):** ① 데몬 생명주기(부팅·crash 재기동·고아 PTY/Job Object 정리, `KILL_ON_JOB_CLOSE` 결합 재설계) ② 프로세스 경계 직렬화+backpressure(in-process Arc/RwLock→WS 프레이밍, 느린 클라가 데몬 안 막게) ③ 단절 중 버퍼링 정책 ④ Windows 데몬화(선례 대부분 Unix domain socket 전제 → named pipe/localhost TCP 직접).
- **공수 추정 갱신:** replay 동시성이 de-risk되어 변수 감소. ~5~8 집중 세션, 최대 변수는 Windows 데몬 생명주기(replay 아님).
- 아키텍처만 빌릴 것: VS Code "out-of-process pty host + 재연결 시 서버 캐시 scrollback push" 모델(Engram과 1:1).

## 설계 방향 확정 (2026-06-14)

### 제안 아키텍처 ↔ Engram 현실 대조 (외부 LLM 제안이 "처음부터"를 가정했으나 실제론 S10에서 완료)
| 제안 단계 | Engram 현실 |
|---|---|
| Tauri 없는 core 분리 | **완료** — `pty/` tauri import 0(불변식), `examples/{headless,transport_smoke,session_smoke}.rs`가 Tauri 없이 AgentManager 직접 구동 = standalone 증명 |
| Command/Event 프로토콜 분리 | **완료(중립 seam)** — `OutputEvent`/`InputEvent`/`CommandSpec`/`Capabilities`(types.rs), `AgentCommand`(profile.rs). 단 별도 crate 아님 |
| Tauri IPC를 transport adapter로 격리 | **완료** — `OutputSink`/`StatusSink` trait(=제안의 EventSink) + `AgentTransport` trait + `ChannelOutputSink`/`TauriStatusSink`(lib.rs) |
| daemon transport 추가 | **남은 진짜 일** |
| embedded/daemon 두 모드 | sink 주입형이라 저렴 |

주의(제안이 단순화한 것, 우리 것이 더 맞음): (1) 제안 `EventSink` 단일 통합 vs 우리 `OutputSink`(고-throughput)/`StatusSink`(저빈도) 분리 — 처리량 특성 달라 유지. (2) 제안 `AgentEvent::Output{Vec<u8>}` = 바이트 고정 vs 우리 `OutputEvent` 종류 불가지(§2 "터미널 강제 금지") — 제안 받으면 퇴행. 받아들일 것: **ts-rs 자동 TS 타입 생성**(현재 api/types.ts 수동 미러 → drift 감소, 데몬 필수 아닌 개선).

### 두-모드 토글 (확정 방향)
프론트에 `AgentClient` 인터페이스(canonical `AgentCommand`/`AgentEvent` 프로토콜). 토글이 구현체만 교체:
- **Embedded:** in-process 어댑터 → 같은 프로세스 AgentManager(직렬화 0). = known-good 폴백.
- **Daemon:** WebSocket 클라 → socket → 데몬 AgentManager. **B-direct(JS↔WS 직결)**, relay 금지(모바일 무도달+이중직렬화로 폐기됨).
- 코어(AgentManager)는 두 모드 동일. 토글은 startup/config 선택(**라이브 핫스왑 아님** — 사용자 "실시간 변경 불요" 확인. 모드 전환 시 에이전트 재시작).
- 효용: embedded가 항상 폴백 → 데몬 버그 나도 안 막힘(babysitting 불요). 단 daemon 경로 정확성(생명주기·재연결)은 한 번 만들고 검증 필요 — 토글이 면제 안 함.

### 구현 phasing (사용자 설계 — 경계 기반 격리 테스트)
1. **seam 캡슐화 + 프론트 부착, 기존(embedded) 회귀 0 확인** — workspace lib crate `engram-core` 추출(코드 Tauri-free라 *이동*) + `engram-protocol`(AgentCommand/AgentEvent) + 프론트 `AgentClient` 인터페이스. embedded가 그 인터페이스 통해 그대로 동작 확인.
2. **데몬 단독 구성 + 데이터 전송 격리 테스트** — 데몬 바이너리 + WS 서버 + standalone 테스트 클라(프론트 없이 spawn/output/replay/재연결/epoch 검증). = `transport_smoke.rs` 철학을 socket 경계로 확장.
3. **그 하네스를 영구 보존** — 데몬 버그 나오면 그 부분만 인큐베이팅 테스트(재연결·slow consumer·데몬 kill/respawn 같은 nasty 케이스 포함해야 phase 4가 깨끗).
4. **접합부만 연결** — 프론트 `AgentClient`를 embedded-impl→daemon(WS)-impl로 스왑. phase 2 하네스가 프로토콜 동작을 충분히 검증했으면 접합 안전.

**linchpin:** 모든 격리 테스트가 검증하는 대상 = `engram-protocol` 계약. 이 계약을 먼저 정확히 못박는 게 "경계 두면 알아서 깨끗"이 성립하는 전제. envelope 초안: `subscribe{sessionId,epoch?,afterSeq?}` / `unsubscribe` / `input{sessionId,epoch,data}` / `list` / `spawn{spec}` / `kill{sessionId}` + 이벤트 `output`/`status`/`exited`/`error`.

## 다음
S12 설계 문서(engram-protocol 계약 + AgentClient 인터페이스 + ET recover 패턴 + 데몬 생명주기 + 하네스 계약) consult 재검증 → phase 1 착수(저위험·기계적, 회귀 0 게이트).
