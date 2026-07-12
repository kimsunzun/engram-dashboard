# 멀티 에이전트 오케스트레이션 아키텍처 — OSS 서베이 + 옵션셋 (설계-결정용 자료)

> **상태:** 조사 자료(결정 아님). 다음 세션이 PRD/TRD 결정에 쓰는 옵션셋·트레이드오프·거부 후보 모음. 임의 채택 금지 — 결정권은 사용자.
> **방법:** `/research design-decision deep` — 주계열 수집자 5명 병렬(sonnet, 층별 by-candidate) → 메인 grounding → Codex(cross-family blind) 적대 리뷰. 앵커 = ADR-0014(제안).
> **날짜:** 2026-07-12 · **작성:** 메인 오케스트레이터(Opus)
> **확신도 범례:** 확실(1차/공식 문서 직접 확인) · 가능성 높음(2차 다수 지지 or 일부 미검증) · 불확실(미검증·추정).
> **열화 라벨:** deep의 *cross-family 병렬 수집*은 미실행(단일 family 주계열 수집 + Codex 적대 리뷰 omission 렌즈로 부분 백스톱). 이 주제는 omission-critical(안 다룬 후보가 결론을 가를 수 있음)이라, 누락 가능성은 §8 한계에 명시.
> **적대 리뷰 반영(2026-07-12):** Codex(cross-family blind, effort high) 리뷰 = 초판 BLOCK, findings 12건(high 3) 전부 반증 근거 동반. 이 v2에서 반영 — 네이티브 baseline(A0) 신설 · 프로세스 트리 격리 축 추가 · "Claude Code Workflow" 귀속 교정(공개 OSS 아님) · Temporal 로컬 dev server 뉘앙스 · §5 deadlock/ACK confident-wrong 교정 · §6 순서 내부모순 교정. 방향(옵션셋)은 유지, 세부·근거 강화.

---

## 0. TL;DR (옵션 요약 — 결정은 §6)

- engram은 이미 오케스트레이션 하부의 **절반을 갖고 있다**: reaper(사망 감지·분류) + epoch(재구독 안전) + S9 복원 사다리(resume→fresh→Failed) + persistence(예약 필드 `RestartPolicy`/`restart_count`/`failed_reason`) + WS 이벤트버스(fanout) + command registry 골격(§5 진입점). 진짜 새로 짜야 하는 건 **런타임 자동재시작 실행 경로 · 에이전트간 주소지정/메시징 · 태스크 그래프 · 오케스트레이터 브레인(§5 LLM)**.
- 성숙 OSS 결론: **감독(Layer A)의 정직한 baseline = 프레임워크 없이 tokio + 기존 PTY/reaper 위에 supervisor state machine 직접 구현(A0)** — Erlang/OTP·Ractor는 *패턴 차용* 대상이지 채택 대상 아님(OS 프로세스≠in-VM actor). 여기에 **프로세스 트리 격리**(Unix process group/cgroups · Windows Job Object) 축이 핵심인데 engram은 이미 Job Object 보유(ADR-0001). **내구성 엔진(Layer B, Temporal/Restate/Inngest)은 현 규모에 과함**(별도 런타임 + Rust SDK pre-1.0) — 로컬 "태스크 저널" 패턴만 차용. **조율(Layer C)은 "코드가 흐름 소유·LLM은 각 단계" 중앙 결정론 모델이 §5와 정합**(가장 가까운 작동 참조 = Claude Code 자체 Workflow/subagent 툴 — 공개 OSS 아닌 내장 기능, 아래 §4 귀속 주의). **통신(Layer D, A2A)은 로컬 단일 데몬엔 과함** — 기존 sink/bus/protocol seam + bounded `tokio::mpsc`로 충분(단 §5 신뢰성 한계 유의).
- 큰 방향 후보 3개(§6): **(1) 감독 우선 최소증분**(reaper+예약필드 위에 OnCrash 자동재시작) · **(2) 중앙 오케스트레이터(Workflow형)** · **(3) 메시징 우선(에이전트간 파이프)**. 서로 배타 아님 — 순서 문제.

---

## 1. 조사 축 & engram 제약 (판정 기준)

**제약(불변 — CLAUDE.md):** ① 교체성(추상 인터페이스 위 구현, swappable) ② §5 LLM-우선 제어(모든 기능이 LLM 호출 핸들, 사람 UI는 보조, 프론트=순수 I/O) ③ crate 경계(core=tauri import 0 · daemon=AgentManager 소유·에이전트 호스트 · protocol=wire 계약) ④ 로컬 소규모(분산 클러스터 아님) ⑤ Rust 코어. 에이전트 = OS 프로세스(PTY claude/codex), in-VM 객체 아님.

**층(ADR-0014 구조):** A 감독·내결함성 · B 내구성 실행 · C 조율 프레임워크(패턴 차용) · D 인터-에이전트 통신. + E engram 기존 자산 그라운딩.

---

## 2. Layer A — 감독·재시작 (A0 네이티브 baseline · Erlang/OTP · Ractor · Actix)

**핵심 발견:** engram의 사망 감지는 이미 OTP monitor의 등가물(`reaper.rs` pump EOF → `done_tx` → 분류)을 갖췄다. 빌릴 것은 *재시작 정책·meltdown 임계값*이지 actor 런타임이 아니다. **정직한 정답은 A0** — 프레임워크 없이 tokio + 기존 PTY/reaper 위에 supervisor를 직접 짜는 것이고, OTP/Ractor는 그 supervisor에 넣을 *패턴*의 출처다.

| 후보 | 핵심 메커니즘 | 재시작 모델 | 성숙도/라이선스 | engram 적합 |
|---|---|---|---|---|
| **A0 — tokio + hand-rolled supervisor (baseline)** | `tokio::process`(child wait·`kill_on_drop`) + `JoinSet` + bounded `mpsc` + engram reaper/epoch/S9 사다리 위에 supervisor state machine 직접 구현 | OTP 패턴을 직접 코딩(아래 매핑). 프레임워크 abstraction 0 | engram이 이미 태반 보유(portable-pty·reaper·epoch), Rust std/tokio(MIT) | **★ 기본 채택 후보.** 프레임워크가 PTY OS 프로세스를 직접 감독하지 못하므로(전부 in-VM actor 가정), 어차피 supervisor는 직접 짜게 된다 — OTP/Ractor는 패턴만 공급 |
| **Erlang/OTP supervision tree** | link/monitor + supervisor behaviour, let-it-crash, process 격리 | 전략 4종(one_for_one/one_for_all/rest_for_one/simple_one_for_one) + intensity/period meltdown → 부모 에스컬레이션. restart 타입 permanent/transient/temporary | 30년+ 실전, Apache-2.0 | **패턴 1순위(차용).** intensity/period → meltdown 임계값, transient → fresh-fallback 연속실패 포기, one_for_one 기본, rest_for_one → 미래 파이프라인 체인 |
| **Ractor (Rust)** (+ ractor-supervisor) | **core ractor가 이미 supervision 관계·link·실패전파·`Actor::handle_supervisor_evt` 제공**(core가 감독함 — 무감독 아님). 별도 `ractor-supervisor`는 *재사용 가능한 재시작 전략*을 얹는 것 | ractor-supervisor: OneForOne/OneForAll/RestForOne + `ChildSpec{restart,backoff_fn,reset_after}` + meltdown(max_restarts/max_window). **DynamicSupervisor**가 engram 동적 spawn/kill과 동형 | ractor v0.15.x(2.1k★, Meta 사용, MIT). supervisor crate v0.1.x — young | **차용(직접 채택 보류).** `backoff_fn:(count)->Option<Duration>` + DynamicSupervisor·meltdown 개념을 A0 supervisor에 이식. actor=tokio task ≠ OS 프로세스라 직접 쓰면 ADR-0001 kill 인과가 복잡 |
| **Actix** | `Supervised` trait + `Supervisor<A>` 단일 actor 감시 | 전략 enum 없음·restart-intensity/backoff 없음(무한 재시작 기본) | v0.13.5(2024)·릴리스 저빈도(유지보수는 지속·abandoned 아님), MIT/Apache | **비권장.** *거부 근거 = `Supervisor`에 restart-intensity/backoff 정책 부재*(검증 가능한 API 사실)이지 유지보수 상태가 아니다. 단일 actor 감시라 N-agent 트리엔 수동 래핑 필요 |

**프로세스 트리 격리(Layer A 핵심 축 — 초판 과소평가, Codex 지적):** engram 에이전트는 OS 프로세스라 "직속 자식 감독"과 "자손 트리 전체 격리"가 다르다. tokio child kill은 **이식성 있는 트리 격리를 안 준다** — Unix는 process group/`setsid`/cgroups, Windows는 **Job Object(kill-on-job-close)**가 필요. **engram은 이미 Windows Job Object 보유**(`TerminateJobObject` — ADR-0001 kill 인과). 따라서 Layer A 요건 = ① Windows=Job Object(있음) ② Unix=process group 격리(claude/codex가 손자 프로세스를 fork하면 필요 — 현 구현 확인 권장). systemd/supervisord류 OS 감독자는 데몬 자체 재시작·cgroup 격리엔 유효하나 이식성(Windows 주 타깃)으로 채택은 거부, 단 그 *격리 모델*은 위 요건으로 흡수.

**engram 매핑(A0에 이식할 OTP/Ractor 패턴):** intensity/period→`restart_count`+시간창→Failed terminal · `Transient`→S9 fresh-fallback 연속실패 포기 · `backoff_fn`→epoch 교체 전 지수 backoff(250ms→×2→30s cap) · DynamicSupervisor→런타임 agent 관리 · `one_for_one` 기본. 확신도: OTP=확실, Ractor core 감독 존재=확실, 버전 숫자=가능성 높음(채택 시 재확인).

**함정/방어:** restart storm(intensity 관대하면) · one_for_all 폭발반경(engram은 one_for_one) · backoff 없는 즉시재시작(반드시 지수 backoff) · supervisor meltdown/데몬 크래시 시 OS 자식 orphan(트리 격리로 방어 — BEAM과 달리 자동 아님, Job Object/process group가 그 역할).

---

## 3. Layer B — 내구성 실행 (Temporal · Restate · Inngest)

**핵심 발견:** 세 엔진 **전부 현 engram 규모엔 과하다** — 공통 이유 ① 별도 서버 프로세스 필수(임베드 불가) ② Rust SDK 전부 pre-1.0/alpha(장기 의존 리스크) ③ 분산 서비스 오케스트레이션용이라 로컬 소규모 PTY 관리와 목적 불일치.

| 후보 | 메커니즘 | 인프라 무게 | Rust SDK | engram 정당성 |
|---|---|---|---|---|
| **Temporal** | 이벤트소싱 + 결정론적 replay(워커 재시작 시 히스토리 대조, 완료단계 skip) | 프로덕션은 무거움(서버 4종 + Cassandra/PG + 옵션 ES). **단 로컬 dev server는 단일 프로세스+임베디드 스토리지 존재**(`temporal server start-dev`) — "무조건 무거움"은 프로덕션 HA 한정. 결정론 강제 전파성 큼 | Rust SDK Public Preview(pre-1.0 — 단 pre-1.0≠alpha) | **불필요.** dev server가 있어도 결정론 강제 + 워크플로 엔진 개념 부담이 로컬 PTY 관리엔 과함 |
| **Restate** | 저널링 + 스냅샷(저널 hit 시 코드 미실행, 결정론 강제 없음) | 중간(단일 Rust 바이너리, 외부DB 불요). 단 **임베드 불가** — 런타임이 클라이언트↔서비스 엔드포인트 사이에 위치(전송은 HTTP 콜백 외 streaming/Lambda 등도 지원, 단 in-process 라이브러리는 아님) | v0.10 pre-1.0, MIT | **불필요(현재).** 실제 durable 필요 시 최유력 후보 |
| **Inngest** | HTTP 콜백 step memoize | 외부 서버 + HTTP 워커 | Alpha(axum만) | **부적합.** 서버리스/웹훅용 |

**차용할 최소 패턴 — "로컬 태스크 저널":** 각 오케스트레이션 스텝을 `JournalEntry{task_id, step_id, status, result, ts}` append-only로 로컬(SQLite/NDJSON) 기록 → 데몬 재시작 시 완료 스텝 skip·미완 재개. 기존 `agents.json` persistence 계층 위 추가 레이어. 결정론 강제 없음(Restate 저널 패턴). **진짜 엔진이 필요해지는 시점:** 여러 에이전트 DAG(A→B→C·조건분기·saga rollback)가 데몬/클라 재시작을 넘어 진행 보장돼야 하고 원격/고부하가 낄 때 — 그때 Restate 재평가(SDK 1.0 도달 후). 확신도: 엔진 특성=확실, engram fit=가능성 높음.

---

## 4. Layer C — 조율 프레임워크 (패턴 차용, Python 중심)

**핵심 발견:** 조율 *결정권을 누가 갖나*가 축이다 — 코드(결정론) ↔ LLM(유연·비결정). engram §5(데몬=결정론 호스트, LLM=핸들 발행 두뇌)엔 **코드-소유 중앙 오케스트레이터**가 정합. 가장 가까운 작동 참조 = **Claude Code 자체의 Workflow/subagent 툴**.

> **귀속 주의(Codex 지적):** "Claude Code Workflow"는 **공개 OSS 프레임워크가 아니라 Claude Code(Anthropic)의 내장 기능**이다 — 이 프로젝트가 실제로 그 위에서 돌아 1차(first-hand) 관찰이나, 외부 독자는 버전·repo로 재현할 수 없다. 아래 팬아웃 cap 수치(동시16/총1000/중첩1-depth)는 그 툴 자체 명세 기준이며, 공개 인용 가능한 값이 아니다. 따라서 이 행의 ★평가는 *설계 판단*이지 검증된 벤치마크가 아니다. 공개 인용이 필요하면 Anthropic Claude Code subagents / Agent SDK 문서를 근거로 삼는다.

| 모델 | 조율 결정권 | 결정론 | 팬아웃 cap | engram 적합 |
|---|---|---|---|---|
| **Claude Code Workflow/Task**(내장 툴·비-OSS) | **스크립트(코드)**, LLM은 각 단계만 | 최고 | 툴 명세상 동시16/총1000/중첩1-depth(공개 인용 불가) | **판단상 최적합** — 데몬=스크립트, agent 프로세스=서브에이전트(★는 검증 벤치 아닌 판단) |
| **LangGraph** | 라우팅 함수(코드+LLM) | 높음 | 명시 cap 없음(그래프 구조가 cap, 사이클엔 END 필수) | ★★★★ — 조건부 라우팅 + 체크포인트 보조 차용 |
| **CrewAI** Sequential | 순서 고정(코드) | 높음 | max_iter | ★★★ — 역할 선언 관용구만 |
| **AG2/AutoGen** SelectorGroupChat | LLM | 중간 | max_turns | ★★ — termination-marker 패턴만 |
| **OpenAI Agents SDK**(Swarm 후계) | LLM(handoff=제어 완전 이전) | 낮음 | 불명(Guardrail이 방어선) | ★★ — Guardrail 병렬검증만, handoff는 충돌 |

**즉시 차용 권장 패턴(Claude Code Workflow에서):** ① **Schema-validated 구조화 출력**(도구 계층 검증·재시도 — protocol crate 응답에 정합) ② **Adversarial verify**(발견마다 N skeptic 팬아웃 — `/review`가 이미 구현, 오케스트레이션 QA에 확장) ③ **Loop-until-dry + 전역 dedup**(재시작/fresh-fallback 체인) ④ **1-depth 중첩 cap**(데몬이 서브에이전트 스폰 시 fork-bomb 방지). **차용 비추:** 피어 handoff(LLM이 다음 에이전트 결정) = 데몬 결정론 라우팅과 충돌. 확신도: Claude Code 수치·패턴=확실, 타 프레임워크 조율모델=확실·일부 cap 수치=불확실.

**안티패턴(전 프레임워크 공통 경고):** 피어 groupchat 프리포올(오류 증폭) · 사이클 END 누락(무한루프) · State explosion(공유 상태 비대) · max_turns/max_iter 누락(비용 폭발).

---

## 5. Layer D — 인터-에이전트 통신 (A2A · actor 메시징 · pub-sub)

**핵심 발견:** 에이전트간 메시징은 engram에 **아직 없는 관심사**(현재 에이전트끼리 대화 안 함). A2A는 로컬엔 과함 — 기존 seam + bounded 채널로 충분.

- **A2A(Google/Linux Foundation):** cross-vendor·조직경계 상호운용용(agent card `/.well-known`, Task 상태머신, 복수 protocol binding). **로컬 단일 데몬엔 fit 판단상 과함**(서명·에이전트카드·전송 오버헤드가 내부 데몬엔 이득 없음 — "overkill"은 확정 사실 아닌 적합도 판단이며, A2A의 경계/상호운용 가치는 별개로 인정). 단 **Task lifecycle 상태머신 패턴**(contextId+taskId+명시적 FAILED/CANCELED)은 이미 engram `AgentStatus`/`Disposition`/`epoch`에 상당 반영. 외부 조직 에이전트 협업이 제품 요구가 되면 재검토. Apache-2.0. 확신도: 가능성 높음(2차 소스, 스펙 원문 미파싱).
- **Actor 메시징 메커니즘:** 전달=at-most-once(Akka 공식·로컬도 손실 가능) · 순서=동일 송수신쌍 FIFO만 · 백프레셔=bounded mailbox. exactly-once 원하면 ACK+고유ID+멱등. engram `OutputSink` fanout이 이미 "mailbox 수신자" 동형.
- **pub-sub/bus:** engram 이벤트버스(ADR-0028 single-push)는 토픽 1개 degenerate pub-sub. `AgentCommand::Subscribe{agent_id,epoch,after_seq}`가 이미 "agent별 토픽 구독" 논리 계약 보유.

**engram 매핑(차용):** 에이전트간 메시징 필요 시 → `AgentManager`=named registry + `WriteStdin`=라우팅 수단, "agent-to-agent bridge `OutputSink`"(A 출력→B stdin, 코어는 수신자 모름·ADR-0003 유지). **핵심 위험(신뢰성 층위 구분 — Codex 지적):** bounded `tokio::mpsc`는 **backpressure만** 주지 end-to-end 전달 보장이 아니다. 큐 경계 ACK는 *큐 수용*만 증명하지 PTY write 성공·에이전트 파싱·태스크 완료를 증명 못 한다. 진짜 신뢰성 메시징은 ① framing ② **수신자 레벨** 메시지 ID/ACK ③ retry 규칙 ④ 멱등성 or persistence가 있어야 성립. **unbounded 채널 금지**(OOM — tokio #4321). seq dedup + epoch가 순서 방어. 확신도: tokio/actor 메커니즘=확실.

**함정/방어:** **deadlock(A↔B 순환 대기)는 supervision으로 방어 안 됨(Codex 교정)** — supervision은 *종료*를 감지·재시작할 뿐 무한 상호 대기하는 살아있는 프로세스를 감지/해소하지 못한다. 필요한 건 timeout·cancellation·bounded request lifetime·순환 감지. · unbounded queue OOM · message storm(agent가 agent spawn하면 N² fanout — 소규모라 현재 무해, 오케스트레이터 패턴 추가 시 fan-out 제어 필요) · silent drop(SinkError를 상위에 통보 안 하면 데이터 유실).

---

## 6. 큰 방향 옵션셋 (배타 아님 — 순서 문제) — **결정 = 사용자**

engram 기존 자산(§7) 위에서 3개 진입 방향. 각 = 최소증분·거부 대안·선행조건.

### 옵션 1 — 감독 우선(최소 증분): 런타임 자동재시작 (권장 시작점)
- **무엇:** 예약 필드(`RestartPolicy::OnCrash`/`restart_count`/`failed_reason`)를 실동작으로. reaper `KeepDisableAutoRestore` 처분 뒤 OnCrash면 backoff 후 재시작(OTP intensity/period+transient 차용). ADR-0019 §후속2 미이행분.
- **차용:** OTP restart 타입·meltdown 임계값 · ractor-supervisor `backoff_fn`. **새로 짤 것:** 자동재시작 태스크(사다리 resume→fresh→Failed는 이미 있음), meltdown 카운터, `SetRestartPolicy` command(§5).
- **거부 대안:** Ractor 직접 채택(OS 프로세스 불일치 → ADR-0001 복잡) · Actix(감독 빈약).
- **선행:** 없음(코어 안정 상태). **가장 작고 되돌리기 쉬움.**

### 옵션 2 — 중앙 오케스트레이터(Workflow형): 태스크 위임·그래프
- **무엇:** 데몬 안에 "코드가 흐름 소유·LLM은 각 단계" 결정론 오케스트레이터(Claude Code Workflow 모델). SpawnProfile/WriteStdin/구독을 조합해 태스크 그래프 실행, 결과 취합. §5 LLM 두뇌가 command 핸들로 조율.
- **차용:** Workflow의 schema-validated 출력·adversarial verify·loop-until-dry·1-depth cap. **새로 짤 것:** 태스크 그래프/DAG 상태, "태스크 결과 반환"(현재 종료만 있고 결과 채널 없음), 하위 출력→오케스트레이터 내부 채널.
- **거부 대안:** 피어 handoff(Swarm)·groupchat(AG2) = 비결정성·오류증폭 → 데몬 모델 충돌.
- **선행:** 없음 — **기존 SpawnProfile/WriteStdin/구독 API로 바로 착수 가능**(옵션3 선행 불필요). 옵션1(감독)이 있으면 더 튼튼. 태스크 상태 영속 필요 시 §3 로컬 저널 차용.

### 옵션 3 — 메시징 우선: 에이전트간 파이프
- **무엇:** `AgentCommand`에 `SendMessage{from,to,payload}` variant + agent-to-agent bridge sink. A 출력을 B 입력으로.
- **차용:** actor named-registry·bounded mpsc+ACK · A2A Task 상태머신 개념(전체 채택 X). **새로 짤 것:** 주소지정·라우팅 레이어·메시지 스키마.
- **거부 대안:** A2A 전체 채택(로컬 overkill) · unbounded 채널(OOM).
- **선행:** 없음이나, 오케스트레이터(옵션2)와 함께라야 값이 큼(메시징만 있고 조율 없으면 용처 적음).

**순서 제언(자료 — 결정은 사용자 · Codex 교정 반영):** **1 → 2 → (3 필요 시)**. 초판의 "1→3→2"는 내부모순이었다 — 중앙 오케스트레이터(옵션2)는 기존 spawn/stdin/output API로 에이전트를 **직접** 지휘할 수 있어 일반 peer 메시징(옵션3)을 선행할 필요가 없다(오히려 옵션3 선행은 조기 프로토콜·신뢰성 작업을 유발). 옵션3은 에이전트간 직접 파이프가 제품 요구가 될 때 추가한다. 단 §5 원칙상 각 단계에 LLM command 핸들을 **함께** 낸다("기능 먼저, 제어 나중" = 위반).

---

## 7. engram 기존 자산 지도 (build-on vs 새로) — 코드 그라운딩

**BUILD ON (있음):**
- 감독: `reaper.rs`(사망분류 단일소비자) + `epoch`(재구독 안전, `manager.rs:195`) + S9 사다리 `restore_one`/`fallback_fresh`(`manager.rs:358-423`) + 예약필드 `RestartPolicy`/`restart_count`/`failed_reason`(`profile.rs:159-169`) + `TerminationIntent`/`Disposition`.
- 메시징: WS `ConnRegistry` fanout(`ws.rs`) + `AgentCommand`/`AgentEvent` 확장가능 enum(`protocol/messages.rs`) + `OutputSink` 임의 구독자 주입 + `WriteStdin`.
- 위임: `SpawnProfile`/`SpawnByCwd` + `AgentProfile{cwd,extra_args,env}` + command registry `window.__engramCmd`(§5 진입, ADR-0022/0055 골격).

**새로(home 없음):** 에이전트간 주소지정 · 태스크 그래프/DAG(영속 없음) · 오케스트레이터 브레인(현재 사람이 오케스트레이터) · 런타임 자동재시작 실행 · 인터-에이전트 메시지 프로토콜 · 오케스트레이션 상태 영속 · per-agent heartbeat(종료 EOF만, 주기 probe 없음).

**§5 LLM 제어 갭:** 현재 LLM 경로 = `window.__engramCmd.{list,run}`(골격) + `invoke`(AgentCommand) + WS 직결. **빠진 핸들:** `agent.send_message` · 태스크그래프 조작 · `SetRestartPolicy`(현재 `SetProfileAutoRestore`만) · supervisor 정책 설정. ADR-0022/0055가 흡수 방향이나 미착수.

**관련 기존 문서(중복 조사 회피 — 반드시 확인):** `docs/research/control-surface-and-fleet.md`(fleet 제어표면) · `docs/research/llm-control-surface-message-command-scope-2026-06-28.md`(메시지시스템 스코프 draft) · ADR-0022(command registry 방향)·0028(이벤트버스)·0019(reaper·자동재시작 게이트)·0055(command registry 확정).

---

## 8. 쟁점·한계·안 다룬 것 (정직 라벨)

- **cross-family 병렬 수집 미실행** — 단일 family 주계열 5명 + Codex 적대 리뷰(omission 렌즈, web_search 능동 탐침)로 부분 백스톱. Codex가 §8 미커버 후보를 실제 탐침한 판정:
  - **Ray · Orleans(virtual actor) · NATS** — 로컬-데몬 권고를 **뒤집지 않음**(분산 런타임/브로커 경계를 더할 뿐). 화면 밖 처리 확인.
  - **systemd/supervisord/s6(OS 프로세스 감독자)** — 채택은 이식성(Windows)으로 거부하되 그 *프로세스 트리 격리 모델*은 §2에 흡수(초판 과소평가 → v2 반영).
  - **네이티브 tokio baseline(A0)** — 초판 누락(high). v2에서 Layer A 표에 명시 추가 — 프레임워크가 PTY OS 프로세스를 직접 감독 못 하므로 A0가 실제 기본 채택 후보.
  - **tmux/zellij 세션 다중화** — 지속 터미널세션 소유·재attach가 *제품 요구*가 될 때만 유의(현재 요구 아님). 미深堀.
- **버전 숫자·SDK 성숙도**는 시간민감(가능성 높음) — 채택 시점에 재확인 필수(라이브러리 버전 = confident-wrong 위험 범주).
- **A2A 전달보장·Restate 아키텍처 세부**는 2차 소스 기반(원문 미파싱) — load-bearing 아니나 채택 전 원문 확인.
- **미검증 세부:** ractor-supervisor `RestForOne` 내부구현 · Inngest exactly-once 엄밀수준 · Unix side 손자프로세스 fork 시 engram process-group 격리 현황(§2 — 코드 확인 권장).
- **적대 리뷰 잔여:** Codex 초판 BLOCK의 high 3·med 5는 v2에서 반영·교정. 방향(옵션셋·기존자산 활용·프레임워크 아닌 패턴 차용)은 리뷰가 뒤집지 못함(반증 없음) → 방향은 **가능성 높음**, 개별 기술 세부는 위 미검증분 제외 대체로 확실.

---

## 9. 후속 (자료→결정 경로)
- 사용자가 §6 옵션 방향 선택 → 해당 방향 TRD + ADR-0014를 **확정**으로 갱신(또는 후속 ADR)하고 거부 대안(§6)을 ADR에 박제.
- 옵션1(감독)이 최소증분·독립이라 파일럿 후보. 선택 시 `/implement`로 ADR-0019 §후속2(OnCrash) 구현.
- 미커버(§8) 중 tmux/zellij·OS 프로세스 감독자는 옵션1 착수 전 짧은 보강 조사 가치.
