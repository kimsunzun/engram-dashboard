# 메시징 v1 과잉 구현 감사 — 피어 서베이 (2026-07-28)

> **발단**: 사용자 우려 "규칙이 너무 복잡한데 우리만 과대 구현하는 것 같다" → `/research deep`로 전면 재조사.
> **방법**: 수집 3갈래 병렬(각 15~25회 검색) → 메인 grounding(핵심 주장 원출처 직접 검증 9건) → cross-family(GPT·web_search) 적대 리뷰(30건 적출, 초안 UNSOUND 판정) → 리뷰어 반증 재검증 4건 → 교정 종합.
> **결론 사용처**: 사용자 결정 2026-07-28 — **"과하다" 우려 철회, v1 유지, 학습으로 전환**. (아래 §5)
> 확신도 표기: ●확실(교차확증) ◐가능성 높음 ○불확실.

## 1. 결론 한 줄

"우리만 복잡하다"는 절반만 맞다 — **신생 피어들이 같은 부품(큐·타임아웃·상태 추적·은퇴)을 2025~26에 각자 재발명 중**이라 방향은 과잉이 아니다. 단 개별 축의 "필연" 주장은 성립하지 않고(제품 목표에서 온 **선택**이 여럿), 체감 복잡함의 본체(내부 정밀 기계장치)에는 더 단순한 대안이 실재한다.

## 2. 갈래별 발견

### A. LLM 멀티에이전트 프레임워크 (~25개 서베이)
- 회신 기한+타임아웃 통지: 주요 프레임워크(AutoGen·LangGraph·CrewAI·OpenAI SDK·SK·MetaGPT 등) 부재. 단 **전원 부재는 아님** — 적대 리뷰가 반례 적출(아래 D).
- 메일박스 용량 상한·TTL: 주류 프레임워크엔 부재.
- busy 에이전트로의 비동기 배달: 예상보다 흔함 — AutoGen Core asyncio 큐, MetaGPT 메시지 풀, Letta 비동기 발송+배달 영수증, A2A 푸시. 대화형 프레임워크(CrewAI·핸드오프 계열)는 동기 턴-넘기기 전용.
- ● A2A 스펙(원문 확인): messageId/taskId/contextId + 태스크 상태·history 있음, **기한·TTL·큐 크기 필드 없음**.
- 발신자 신원 인증: 인프로세스 계열 전무, 크로스 조직 프로토콜(A2A Bearer/mTLS)만.

### B. 동제약 피어 (에이전트 = 별도 프로세스·장수 세션)
- ● **naive 주입(턴 무시 stdin)은 커뮤니티 스스로 문서화한 실패**: "~70-80% reliable", "message clobbering" (agent-orchestrator#853 원문 확인). 그 재설계는 3층 주입·epoch 펜싱·append-only JSONL 인박스 채택 — 우리와 같은 방향 수렴.
- ● **Claude Code Agent Teams**(공식 문서 확인): 파일 메일박스 + 자동 배달 + TeammateIdle 훅. 상관 id·기한·그룹(수신자별 1통씩 보내라)·세션 영속(resume 시 유령에게 발신) 전부 없음. 손상 항목 1개가 그 메일박스 배달 전체를 막던 실사고 → v2.1.207 수정. **단순함의 대가가 실재한다는 실증.**
- ● LangGraph Agent Server: `multitask_strategy` 기본값 enqueue(수신자별 FIFO) — 원문 확인. (단 서버 endpoint 기본값이지 코어 그래프 기본값 아님 — 리뷰 지적 반영)
- ● 턴/idle 감지는 실사고 다발 지점: GitHub Copilot SDK 포스트모템 2건(고정 60s idle 타임아웃이 진행 중 세션 오살 — 6~8일 장애 / 오케스트레이터 턴 종료를 전체 완료로 오판 ~30%). 우리 busy 밸브·중첩 Task 미검증 항목과 정확히 같은 문제 계열.
- 오프라인 배달: fail-fast(AutoGen UndeliverableException) 또는 전송 회피(AgentMail=이메일). "무기한 파킹+스폰 시 자동 배달"을 1급으로 파는 곳 없음(Temporal이 비-LLM 천장).

### C. 성숙 메시징 인프라 기준선
- 보편 최소셋: 채널당 FIFO류 순서 + 회신 상관 **패턴**(native 필드는 절반: email Message-ID/In-Reply-To·AMQP correlation_id·FIPA reply-with·Matrix relates_to) + 발신자향 실패 신호(bounce/DSN/DLX/DLQ).
- 계열 분화: **TTL·용량 상한 = 큐/브로커 계열 표준**(Postfix 5d·SQS 4d·Kafka 7d·AMQP max-length) vs **액터 계열은 의도적 무제한**(Erlang 메일박스 unbounded — OOM 리스크 문서화).
- 기한 감시: 인프라가 **평문 발송**에 거는 곳 없음. 있는 곳은 전부 opt-in 요청-응답 래퍼(Erlang gen_server:call·Akka ask·NATS request/reply·gRPC deadline) — 우리 reply_by도 opt-in이라 구조적으로 이 부류.
- 봉투 수준 발신자 인증: 희소(XMPP JID 스탬프·Matrix 서명·SQS SigV4). SMTP는 SPF/DKIM이 후행 볼트온.

### D. 적대 리뷰 (GPT · 30건 적출 · 초안 UNSOUND) + 반증 재검증
초안의 "전원 부재" 보편 부정문들이 반례로 격추됨. 반례를 직접 재검증한 결과:
- ● **OpenClaw**: 서브에이전트 비동기 실행 + `runTimeoutSeconds` + 초과 시 부모에 "timed out" 상태 푸시 + 실행 이력 + 60분 자동 아카이브·stale-run 은퇴. (큐 상한 20·오버플로 정책 주장은 ○미확인 — 문서 리다이렉트)
- ● **AgentScope**(Alibaba): `agent_send`에 `timeout_seconds`(기본 30s·최대 600s) + 태스크 상태 영속·조회. MsgHub 동적 그룹(늦은 참가자에 과거 방송 미전달 = 발송 시점 멤버십).
- ● **MCP Agent Mail**(2k★): 코딩 에이전트용 메일함 MCP 서버 — 스레딩·회신 상관·ack_required. (쿼터·TTL 강제는 ○미발견 — 리뷰어도 과장)
- 뉘앙스: 피어들의 타임아웃은 전부 **위임-태스크(부모→자식 런) 타임아웃** 형태. **임의 피어 메시지에 절대 기한 + 회신 빚 장부 + 통지 빚 은퇴**라는 우리 조합은 여전히 미발견 — 리뷰어 표현: "unusual하나 unique/necessary까진 미입증".
- 방법 교정 수용: FIPA "발신자가 감시" 표현은 과장(스펙은 감시 주체 미지정) · "modal solution" 단정은 표본 부족 · ESSENTIAL 판정은 계보→필연 비약 · 툴 개수는 복잡도 반박 근거 아님(복잡함은 내부 상태기계에 있음).

## 3. 교정된 축별 판정 (3분류)

| 분류 | 축 | 근거 |
|---|---|---|
| **필연에 가까움** (동제약 피어 수렴 + 실패 실증) | 파킹 큐·FIFO / busy 게이트 / 유계(형태 불문) / 발신자 신원 스탬프 | ● naive 주입 실패 문서화 + Agent Teams 사고 + 전 피어 수렴 |
| **제품 목표에서 온 선택** (증거가 강제하진 않음) | request 계약 전반 / reply_by / 그룹 / 이력 링 깊이 / notice 레인 | ◐ 목표(잊는 LLM 외부 기억·자기 각성·표류 없는 조회)를 인정하면 정당 — 목표 자체가 근거지 피어가 근거 아님 |
| **정밀 기계장치** (더 단순한 대안 실재) | mark-and-sweep 은퇴(대안: cap 도달 시 반려) / in-flight 회계 / busy 30분 밸브 / notice +1 허용 | 정확성 비용 — 축소 실후보였으나 §5 결정으로 유지 |

## 4. 한계
OpenClaw 큐 상세 ○ · MS Durable Agents ○미검증 · 교정 후 보고서 2차 적대 리뷰 미실시(1차 30건 반영으로 갈음) · Slack/Discord 내부 비공개 · 학술 서베이 표(arxiv 2504.16736 등) 미대조.

## 5. 사용자 결정 (2026-07-28)
- **"과하다" 우려 철회 — v1 그대로 유지**, 축소 없음.
- 대신 **사용자 학습 플랜**으로 전환: 시스템을 깊이 학습한 뒤 이상 지점을 직접 판별하기로.
- MCP Agent Mail = 직접 피어 → 참조 조사 가치 백로그 적립.

## 출처 (핵심만 — 갈래별 전체 목록은 수집 로그)
A2A spec: a2a-protocol.org/latest/specification · Agent Teams: code.claude.com/docs/en/agent-teams · agent-orchestrator#853: github.com/AgentWrapper/agent-orchestrator/issues/853 · LangGraph: docs.langchain.com/langsmith/agent-server-api/thread-runs/create-background-run · OpenClaw: docs.openclaw.ai/tools/subagents · AgentScope: java.agentscope.io/v1/en/docs/harness/subagent.html, doc.agentscope.io/tutorial/task_pipeline.html · MCP Agent Mail: github.com/Dicklesworthstone/mcp_agent_mail · Copilot SDK 포스트모템: github.com/github/gh-aw/issues/39310, github.com/github/copilot-sdk/issues/1275 · 인프라: postfix.org·rabbitmq.com/docs/maxlength·docs.aws.amazon.com(SQS)·erlang.org·doc.akka.io·fipa.org/specs/fipa00061·xmpp.org/extensions/xep-0160·spec.matrix.org · NATS/gRPC/ROS2: docs.nats.io·grpc.io/docs/guides/deadlines·docs.ros.org(QoS)
