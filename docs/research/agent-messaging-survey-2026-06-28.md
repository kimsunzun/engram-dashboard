# 리서치 — 에이전트 메시징(전통 → 에이전틱) OSS 서베이 + 합치기 판단

**상태:** 설계-결정 모드. cross-family(Claude Sonnet 팬아웃 2 + Codex blind) + opus 적대검증. **medium.**
**날짜:** 2026-06-28 · 작성: dashboard1(wip/a1) · **레퍼런스**(채택=오너 PRD 결정).
**확신도 범례:** 확실 / 가능성높음 / 불확실.

> 목적: 에이전트/컴포넌트 간 메시지를 어떻게 전달하나 — **전통 메시징(MQ·actor·채널)부터 에이전트흐름(A2A·오케스트레이션)까지** 훑고 최종 권고. + "커맨드 버스와 한 버스로 합치나" 러프 판단.

## PART A — 전통 메시징

| 후보 | 메커니즘 | 전달보장 | engram 적합도 | 라이선스/성숙도 |
|---|---|---|---|---|
| **tokio 채널**(mpsc/broadcast/watch) | in-proc async 채널 | mpsc=백프레셔(bounded)/broadcast=Lagged 유실/watch=최신값 | ★ **로컬 in-proc 최적, 의존0, 이미 사용중**. 프로세스 경계 못넘음 | MIT/매우높음 |
| **Ractor**(Rust actor) | Tokio 위 actor, 메일박스(4우선순위레인), supervision, PG 라우팅, 옵션 cluster | at-most-once(추정·불확실) | ★ OTP식 supervision 이식·원격경로. 단 **기본 unbounded 메일박스=백프레셔 X**(PTY 고빈도 출력 주의) | MIT/v0.15·활발 |
| Actix | 자체 Arbiter, Handler<M>, bounded 메일박스 | at-most-once | 로컬 성능 최고나 **원격 불가·개발 정체·Tokio 마찰** | MIT/Apache·정체 |
| **NATS/JetStream** | subject pub/sub, JetStream=영속·재생·ack | core at-most-once / JS at-least-once·exactly-once류 | 원격 확장 시 ★. 현재 로컬엔 **외부서버=오버스펙**. transport seam swap 후보 | Apache-2.0/CNCF·높음 |
| Redis pub/sub | channel pub/sub | at-most-once(Streams 강화) | ✗ 외부서버·**라이선스 변경(RSAL/SSPL/AGPL)**·메시징 전용설계 아님 | 주의/높음 |
| MQTT | topic pub/sub, QoS 0/1/2 | QoS 선택 | △ QoS 장점이나 IoT 추상화 어색·Rust 생태 얇음 | Apache/MIT·중간 |
| RabbitMQ/AMQP | exchange→queue 라우팅, ack | at-least-once | ✗ Erlang 서버 필수·과중 | MPL-2.0/매우높음 |
| ZeroMQ | brokerless 소켓(inproc/ipc/tcp) | at-most-once | △ 로컬 최저지연이나 영속·보장 X·Rust 네이티브 성숙 불확실 | LGPL(crate MIT)/중간 |

## PART B — 에이전틱(LLM 멀티에이전트)

| 후보 | 메커니즘 | engram 적합도 | 성숙도 |
|---|---|---|---|
| **A2A 프로토콜** | Agent Card·task·message·artifact·streaming, JSON-RPC/gRPC/REST | ★ **외부 상호운용 boundary 어댑터**용. 내부 PTY 라우팅엔 부적합 | LF 프로젝트/신생(가능성높음) |
| **LangGraph** | state graph·supervisor·Command handoff·persistence·interrupt | ★ 내부 토폴로지 참조(supervisor+handoff+shared state). Python퍼스트→프로세스/API 경계로 | OSS/성숙(에이전트 생태) |
| **OpenAI Agents SDK / Swarm** | handoff as tool·typed input·history filter·tracing·guardrail | ★ 핸드오프 모델 참조(handoff/라우팅). Python/TS퍼스트 | MIT/현행(Swarm은 실험) |
| MS Agent Framework/AutoGen | conversational message·group chat·workflow | 참고용(.NET/Python). v1.0 초기=성숙도 미달 | MS/신생~성숙 혼재 |
| CrewAI | role/task/crew·위임 | 멘탈모델 참고. Python퍼스트 | OSS/중간 |

**★ 에이전틱이 전통 메시징보다 추가로 요구하는 것(확실):** 타입드 의미 봉투(task·message·artifact·tool_call·observation), capability 발견, identity/role/authority, conversation/task lifecycle, correlation ID, context/memory 정책, artifact/파일 참조, **human approval gate**, cancellation/interrupt, handoff/라우팅 정책, audit/trace/eval 메타, **멱등 replay**(LLM 행동은 비결정적). → 즉 전통 메시징 위에 **"에이전틱 레이어"**를 얹는 구조.

## 교차검증표 (Claude ↔ Codex)

| 클레임 | Claude | Codex | 판정 |
|---|---|---|---|
| 지금은 tokio 채널 in-proc(transport seam 트레이트 뒤) | ✓ | ✓ `Transport` 트레이트 + 전달클래스(ephemeral/durable/state_latest/command_rpc) | **수렴·확실** |
| 원격/영속 미래 타겟 = NATS JetStream | ✓ | ✓ | **수렴·확실** |
| Redis/RabbitMQ/MQTT 배제(현 요구 초과/라이선스) | ✓ | ✓ | **수렴·확실** |
| PTY 에이전트 = Ractor식 supervision | ✓(백프레셔 주의) | ✓(deep adopt보다 small layer) | 수렴·가능성높음 |
| A2A = 외부 boundary만, 내부 버스로 X(아직) | ✓ | ✓ | **수렴·확실** |
| 에이전틱 레이어를 transport 위에 별도로 | ✓(supervisor+handoff) | ✓ `AgentId/Capability/Task/Message/ArtifactRef/Handoff/Approval/RunTrace` | **수렴·확실** |

**불일치 없음.** Codex가 봉투 타입·전달클래스·append-only 로그를 더 구체화.

## ★ 합치기 vs 분리 (러프 교차노트 — 오너가 물은 핵심)

**판정: 논리적으로 분리(control plane / data plane), 물리 전송로는 공유 가능.** (Codex 명시, Claude 일관 — 가능성높음)
- **커맨드 버스**(control plane)는 authorization·priority·cancellation·bounded 스키마·"chat과 혼동 금지" 의미가 필요.
- **에이전트 메시지**(data/message plane)는 replay·context·artifact·느슨한 진화가 필요.
- 성숙 분산설계의 통념 = control plane ↔ data plane 분리. 단 초기엔 **같은 물리 전송로(WS/Tokio/추후 NATS)** 위에 둘을 얹어도 됨 — 분리는 *논리/스키마* 차원.
- 적대검증: "단일머신 대시보드면 한 버스가 더 단순?" → 초기엔 맞지만 auth·취소·스키마혼동 리스크가 커서 seam에서 가르는 게 저위험 over-engineering(§0)에 부합. **분리 유지.**

## 최종 권고 (engram 메시징)

1. **지금:** in-proc = **tokio 채널**을 `Transport`(또는 기존 sink/transport) seam **트레이트 뒤에** 두고 **전달 클래스 명시**(ephemeral/durable/state_latest/command_rpc). 중요한 에이전트 이벤트는 **append-only 로컬 로그**.
2. **에이전트 모델:** PTY 에이전트를 **supervised actor**(Ractor식 개념; 무겁게 Actix 도입보다 small layer). 메일박스는 **bounded 병용**(PTY 고빈도 출력 백프레셔).
3. **에이전틱 레이어(transport 위):** `AgentId·Capability·Task·Message·ArtifactRef·Handoff·Approval·RunTrace` 타입 + supervisor/handoff 토폴로지(LangGraph·OpenAI SDK 패턴 이식). **peer 자유토론 금지**(오케스트레이터 필수).
4. **외부 경계:** A2A 형태를 추적·어댑터로(외부 상호운용). 내부 버스로는 아직 X.
5. **원격 필요 시:** transport seam을 **NATS JetStream**으로 swap. (ADR-0020 전송의미론·ADR-0028 single-push와 일관)
6. **커맨드 버스와는 논리 분리**(위 교차노트).

## 거부 후보 → ADR 거부 대안 후보

Redis(라이선스·설계부적합) · RabbitMQ/AMQP(Erlang서버·과중) · MQTT(IoT 추상화 어색) · Actix(원격불가·정체) · ZeroMQ(영속/보장X·Rust성숙 불확실) · A2A를 *내부* 버스로(아직 신생·내부 PTY 부적합).

## 공백·한계

- 만장일치 ≠ 정답: 양 family가 "tokio now + NATS later + actor supervision + 에이전틱 레이어 분리"에 강수렴 — 잘 정립된 Rust/분산 패턴이라 저위험이나 공통편향 가능성 인지.
- Ractor 전달보장 at-most-once = **문서 미명시(불확실)** — 채택 시 실측 필요.
- 에이전틱 프레임워크 대부분 Python/TS퍼스트 — Rust 이식은 패턴 차용이지 직접 의존 아님.
- A2A·MS Agent Framework 신생 — 성숙도 변동 추적 필요.

## 출처

tokio sync https://docs.rs/tokio/latest/tokio/sync/ · Ractor https://github.com/slawlor/ractor · https://docs.rs/ractor/ · Actix https://docs.rs/actix/ · NATS JetStream https://docs.nats.io/nats-concepts/jetstream · Redis pubsub/license https://redis.io/docs/latest/develop/pubsub/ · MQTT https://mqtt.org/ · RabbitMQ confirms https://www.rabbitmq.com/docs/confirms · A2A https://a2a-protocol.org/latest/specification/ · LangGraph https://github.com/langchain-ai/langgraph · OpenAI Agents SDK handoffs https://openai.github.io/openai-agents-python/handoffs/ · MS Agent Framework https://learn.microsoft.com/en-us/agent-framework/ · CrewAI https://docs.crewai.com/en/concepts/collaboration · actor 라이브러리 비교 https://tqwewe.com/blog/comparing-rust-actor-libraries/
