# 멀티에이전트 호스팅·오케스트레이션 리서치 (상세 기록)

> **상태:** Stage 1·1.5·2·3 완료. 선택지 제시 — **사용자 결정 대기**.
> **단계:** PRD/컨설 (선택지 → 사용자 결정). 구현 아님.
> **방법:** sonnet Explore 5 + Codex 1 병렬 조사(Stage 1) → 병렬 적대 검증 3(Stage 1.5) → Opus 종합(Stage 2) → Codex+opus 2인 리뷰(Stage 3).
> **날짜:** 2026-06-22 · **plan:** `~/.claude/plans/jaunty-spinning-moth.md`
> **확신도 표기:** (확실) 공식문서·코드 근거 · (가능성 높음) 정황 근거 · (불확실) 미확인/추정.

---

## 0. 왜 (Context)

engram은 에이전트 1개 = `claude` CLI 프로세스 1개(PTY/portable-pty, daemon 아래 spawn) = **process-per-agent**. N개 다중 스폰 시 CLI exe N개만큼 RAM. 사용자 질문: ① "한 세션/프로세스가 여러 에이전트 호스팅(in-exe)"으로 메모리를 줄일 수 있나, ② in-exe/out-exe 조합, ③ 현존 오케스트레이터에서 차용할 알고리즘. 핵심 줄기 → 선택지+트레이드오프+놓친 대안을 사용자 결정용으로 정리.

---

## 1. Stage 1 — 경험적 조사 (원자료)

### A. Claude 1st-party 다중에이전트 메커니즘

**A-1. Claude Code Task 서브에이전트 — 별도 OS 프로세스 (process-per-agent), 완전 격리 (확실)**
- 실제 스폰 명령(issue #19045 확인): `claude --output-format stream-json --verbose --input-format stream-json --model <m> --resume <SESSION-ID> --disallowedTools ...`. 부모 세션이 `spawn()`으로 자식 `claude` 생성.
- 동시성: 공식은 "순차"라 하나 단일 응답에서 다수 Task 호출 시 빠른 연쇄 생성. 공식 `maxParallelAgents` 없음(feature request #15487). 2분 내 24개 프로세스 생성 사례 보고.
- 메모리: 고아 프로세스(부모 종료 후 미종료) 각 ~400MB.
- 컨텍스트: `--resume <sid>`로 기존 세션 읽되 실행 완전 격리. 부모↔자식 실시간 공유 없음.
- 제어: LLM이 Task 도구 호출 → 런타임이 spawn. 외부 직접 제어 API 없음.

**A-2. Claude Agent SDK (TS/Py) — CLI subprocess 래퍼, 단일 SDK 프로세스가 N세션 구동 (확실)**
- 내부: `query()`가 Claude Code CLI 바이너리를 subprocess로 스폰(네이티브 바이너리 optional dependency 번들). **다이렉트 API 호출 아님.**
- N-세션: 호출자가 `query()` 여럿 병렬 → 각 독립 subprocess. (11+ 동시 호출 시 `MaxListenersExceededWarning` 버그→수정, 즉 다수 동시 세션 설계 지원)
- 세션 재개 3종: `resume: "<uuid>"`(특정 과거 세션), `continue: true`(최근 이어받기), `forkSession: true`+`resume`(과거 세션 fork→새 sid 발급=브랜치).
- 세션 ID 주입: `query({ options: { sessionId: "custom-uuid" } })`.
- **Pre-warming:** `startup()` API로 subprocess 미리 초기화 → 첫 쿼리 지연 제거. `WarmQuery` 핸들, `AsyncDisposable`(자동 해제).
- 세션 저장: 기본 JSONL(`.claude/sessions/`). `sessionStore` 어댑터로 외부 백엔드(DB) 연결. `persistSession: false`로 비저장.
- 파일 체크포인팅(실험): `rewindFiles(userMessageId)`로 특정 메시지 시점 파일 롤백.
- 스트리밍 입력: `prompt: AsyncIterable<SDKUserMessage>` → 세션 중 메시지 스트림. 세션 도중 `setModel()`/`setPermissionMode()` 가능.
- 메모리: 동시 세션당 subprocess — SDK도 process-per-agent. SDK 프로세스 자체는 경량 orchestrator.

**A-3. Agent View / Teams / 멀티에이전트 UI**
- Agent View: 서브에이전트 실행 현황 시각화(실행/완료/실패). (불확실: 공식 스크린샷 미확인)
- **Teams 모드:** 팀원 = **별도 Claude Code instance**(확실). `~/.claude/teams/{team}/config.json`에 session IDs + tmux pane IDs 저장. split-pane은 *표시 방식*이지 프로세스 1개가 아님.
- claude.ai: 별도 멀티에이전트 기능 없음(단일 대화). 멀티에이전트는 Claude Code/SDK 영역.

**A-4. 헤드리스/프로그래매틱 제어**
- `claude -p "prompt"` non-interactive 단발(매 호출 독립 프로세스). `--output-format stream-json`(JSONL) / `text`.
- `--session-id <uuid>`(우리가 sid 통제 — engram S9와 동일), `--resume <sid>`, `--input-format stream-json`(stdin JSON 멀티턴). 종료코드 0/1/2.

**A 결론:** Anthropic 1st-party(Task + SDK) **모두 process-per-agent**. "단일 프로세스 N에이전트"를 1st-party가 공식 제공하지 않음 → engram 현 설계 = Anthropic 자체 아키텍처와 동일 계층. SDK가 더하는 가치 = 프로세스 관리 추상화(pre-warming·fork·sessionStore·AsyncDisposable) — engram 이식 레퍼런스.
출처: code.claude.com/docs/en/agent-sdk/typescript, .../agent-sdk/subagents, .../agent-teams, claude.com/blog/building-agents-with-the-claude-agent-sdk, github.com/anthropics/claude-code/issues/{4182,19045,15487}, docs.anthropic.com/.../sdk-sessions

### B. 서드파티 Claude 에이전트 매니저/UI

| 도구 | 프로세스 모델 | 격리 | 메모리 | 영속/재개 | 스택 |
|---|---|---|---|---|---|
| **claude-squad** (smtg-ai) | tmux 세션 1 + CLI 1 per agent | git worktree 1:1 | 절감 없음 | `r`키 일시정지 재개(상세 불확실) | Go, tmux, gh |
| **Crystal** (stravu→Nimbalyst) | node-pty, CLI 1 per agent, tmux 미사용 | worktree 1:1(`worktreeManager.ts`) | **lazy init**(열 때만 spawn)+비활성 렌더 중단; 프로세스는 여전히 1/agent | **SQLite**(sessions/outputs/messages) → 재시작 후 scrollback 복원 | Electron+React19+Zustand+XTerm+SQLite |
| **Conductor** (conductor.build) | CLI 1 per agent, 자체 번들 실행 | workspace=브랜치+터미널(worktree 추정, 불확실) | 절감 없음 | 불확실 | macOS 전용, 내부 미공개 |
| **vibe-kanban** (BloopAI) | `command-group`로 CLI subprocess spawn, 1/agent | worktree 1:1 | 절감 없음 | 불확실 | Rust 50%+TS 46%, React |
| **ccmanager** (kbwo) | PTY, CLI 1 per agent, tmux 회피 | worktree 생성/삭제/병합 UI, 세션데이터 복사 | 절감 없음 | worktree 간 대화이력 복사 | TypeScript |

- **공통(확실):** RAM 근본 해결 도구 없음 — 전부 process-per-agent. CLI 자체가 독립 프로세스라 외부 매니저가 멀티플렉싱 불가. RAM은 클라이언트(매니저)가 아니라 **에이전트 런타임 계층**에서만 해결.
- 부분 완화: Crystal lazy init(불필요 선제 spawn 방지).
- engram 차용 후보: lazy spawn, SQLite 세션 영속(현 replay 버퍼 메모리 한정), ccmanager 4단계 상태머신+훅(§5), worktree 자동 생성/정리.
출처: github.com/{smtg-ai/claude-squad, stravu/crystal, kbwo/ccmanager, BloopAI/vibe-kanban}, conductor.build

### C1. 사용자 지목 named tools

**Gastown** (github.com/gastownhall/gastown, Steve Yegge, 2025) — 다중 코딩 에이전트(20~30개) 동일 코드베이스 병렬 운용.
- 조율 = 계층 supervisor: **Mayor**(작업 분해·배치) · **Polecat**(워커, 세션은 일시적이나 정체성·이력 영속) · **Witness**(장비별 헬스모니터 — 정체 감지→nudge/handoff) · **Deacon**(교차 장비 감시 데몬).
- 호스팅: process-per-agent. 컨텍스트: **Hooks**(worktree 상태) · **Beads**(git 이슈 데이터) · **Seance**(이전 세션 검색·복구 → 재시작 후 컨텍스트 무손실).
- 차용: **외부 헬스모니터+컨텍스트 전달 재시작(Witness)** = engram reaper/StatusSink에 직접 대응.

**multi-agent-coordinator** — 공식 제품명 아님(불확실). 후보 ① issue #20095 "Conference Room Mode"(라우터가 입력을 다수 세션에 분배, feature request) ② Managed Agents multi-agent API(오케스트레이터가 Task로 서브에이전트 스폰, 격리 컨텍스트 병렬, 스레드 영속). 가장 가능성 높은 해석 = Task 기반 오케스트레이터-워커.

**claude-flow → ruflo** (github.com/ruvnet/ruflo, 상표로 리네임, Reuven Cohen, v3.5) — 100+ 전문 에이전트 메타-하네스.
- 조율 = **Queen-워커 스웜**: Queen이 토폴로지·합의 관리, 역할별 에이전트(coder/tester/reviewer/architect/security), stream-json chaining, 의존성 그래프 병렬 실행.
- 호스팅: Claude Code 인스턴스를 MCP+27 훅 위에 조율, process-per-agent. 내결함 명시 없음(불확실).
- 메모리: **AgentDB**(공유 벡터 DB)+SONA+ReasoningBank.
- 차용: capability 기반 동적 역할 라우팅.
출처: github.com/gastownhall/gastown, steve-yegge.medium.com/welcome-to-gas-town, github.com/anthropics/claude-code/issues/20095, platform.claude.com/docs/en/managed-agents/multi-agent, github.com/ruvnet/ruflo

### C2. 일반 오케스트레이션 프레임워크 (내구성·재개 초점)

| 프레임워크 | 조율 모델 | 내구성/재개 | 컨텍스트 공유 | 차용 패턴 |
|---|---|---|---|---|
| **LangGraph** | StateGraph(노드/엣지), Supervisor 노드 | **super-step 체크포인트** `thread_id`별 스냅샷(SQLite/PG), 크래시 후 동일 id 자동 재개, time-travel | TypedDict+reducer 공유 상태 | `thread_id=AgentSession.id` |
| **Temporal** | Workflow(결정론)+Activity(외부I/O), Child Workflow | **이벤트 히스토리 replay**: 완료 Activity 스킵→중단점 재개 | Workflow 로컬+Signal/Query | "외부 닿는 건 다 Activity" 경계 |
| **Restate** | 서비스 핸들러(HTTP)+Virtual Object(키별 상태) | 저널 replay(이벤트 소싱), **단일 바이너리+내장 KV(RocksDB)** | Virtual Object 키별 KV | 단일 바이너리 사이드카 |
| **AutoGen(AG2)** | GroupChat 대화, speaker selection | 약함(메모리 내 히스토리), 외부 체크포인터 플러그인(불확실) | 대화 히스토리 | UserProxy=오케스트레이터 역할 분리 |
| **CrewAI** | Agent+Task+Crew, Sequential/Hierarchical | 약함(Flows 일부), 프로덕션은 LangGraph로 이주 경향 | 태스크 출력 체이닝 | Role 기반 에이전트 명세 |
| **MS Agent Framework** | Orchestrator-Agent(AutoGen+SK), v1.0 GA 2026-04 | 내장 없음, 외부(Cosmos/Durable Task) 위임 | 메시지 스트림+공유 메모리 | 에이전트 타입 분류 |
| **A2A 프로토콜** | 에이전트=네트워크 엔드포인트(HTTP+JSON-RPC+SSE) | 프로토콜 자체 내구성 없음, Task 상태만 | Task 멀티파트 메시지 | **Agent Card**(capability JSON 광고) |

- **추천 Top 3 (engram pain=재개 불가):** ①Restate/Temporal replay(세션 spawn을 Activity로 래핑→재개; **Restate 우선** — 단일 바이너리 사이드카, 데스크탑 네이티브에 운영부담 최소, Rust SDK 존재 성숙도 불확실) ②LangGraph thread_id=세션id 체크포인트(SQLite 1파일, Rust 직접 구현 쉬움) ③A2A Agent Card로 capability 매트릭스 표준화.
출처: docs.langchain.com/.../persistence, docs.temporal.io/workflow-execution, spheron.network/blog(temporal-restate), devblogs.microsoft.com/agent-framework, arxiv.org/pdf/2504.16736(A2A survey), latenode.com(LangGraph vs AutoGen vs CrewAI)

### C3. Rust-fit supervision (OTP/actor)

- **OTP 재시작 전략:** `one_for_one`(죽은 것만 — AI 에이전트는 독립이라 **기본값**) / `one_for_all`(하나 죽으면 전체) / `rest_for_one`(죽은 것+이후만 — 파이프라인 의존).
- **Restart Intensity `{MaxR, MaxT}`**(차용 핵심): MaxT초 내 MaxR회 초과 → supervisor 자신 종료=에스컬레이션. crash-loop을 숫자로 정의. "let it crash"(복구 시도 말고 죽여 재시작). supervisor tree 계층.
- **Ractor vs plain-tokio 판정: plain-tokio + 경량 wrapper 권장.** Ractor(2.1k★, v0.15) `ractor_supervisor`는 초기 수준, 외부 OS 프로세스 감시 미지원, in-process actor와 PTY 프로세스 임피던스 미스매치. tokio 대안(task-supervisor·Taskvisor·**ProcessKit-rs**(외부 프로세스 전용))이 backoff·crash-loop 이미 구현. → **OTP 개념을 AgentManager에 직접 이식**이 의존성 대비 이득. 진짜 actor mesh(원격 분산) 시점에 재평가.
- engram 매핑: AgentManager=root supervisor, AgentSession=child spec, reaper=monitor+exit 분류, epoch=child 재시작 슬롯. 권고값: MaxR=5/MaxT=120s, backoff 1s×2 상한 60s jitter±20%.
출처: erlang.org/doc/apps/stdlib/supervisor.html, github.com/slawlor/ractor, docs.rs/ractor-supervisor, github.com/ZelAnton/ProcessKit-rs, users.rust-lang.org(Taskvisor)

### 호스팅 축 매트릭스 (Codex 추론 — FACT/REASONING/UNCERTAIN 라벨)

| 모델 | RAM | 결함 격리 | 보안/샌드박스 | 구현 복잡도 |
|---|---|---|---|---|
| **A. process-per-agent** (현행) | 최고(프로세스당 런타임 세금) | 강(1 crash=1 agent) | 최강(OS 프로세스 경계) | CLI엔 최저(자연) |
| **B. in-process 멀티** | 최저(daemon+task, per-agent=대화상태) | 약(panic·deadlock·event-loop starvation이 전체 영향) | 약(주소공간·자격증명 공유) | 중~고(취소·task supervision·shared-state 규율) |
| **C. hybrid worker pool** | 중(K agents/worker, 세금 분산) | 중(1워커 crash=K agents) | 중(워커/trust tier별) | 최고(워커 프로토콜·라우팅·부분실패) |

- **핵심 통찰(확정·정제):** "backend의 실행 substrate가 최소 호스팅 경계를 결정한다." PTY CLI=프로세스 경계 강제 / HTTP API=경계 없음(async task) / daemon화 CLI(JSON-RPC·LSP식)=worker-pool 가능. 스레드는 라이브러리형 에이전트에만 유효. tmux도 CLI 병합은 못 함.
- **메모리 규모(UNCERTAIN, 추정):** idle async task ~10KB~수백KB / in-process+대화상태 ~0.5~10+MB / **idle Node형 AI CLI ~80~300+MB RSS** / active CLI ~200MB~1GB+. → 20 idle: process-per-agent 수 GB vs in-process API 수십~수백 MB.
- **Codex 권고:** `capabilities.hosting` 도입, CLI=process-per-agent 유지(정직한 경계), **API(codex_api)부터 in-process(최대 이득)**, worker pool은 미래. AgentManager 함의: ①AgentHandle를 ProcessHandle에서 분리 ②supervision hosting-aware ③kill 계층화(cancel/terminate/close_session) ④output 정규화(terminal_stream vs structured_events) ⑤pooling은 PTY CLI 말고 API/SDK부터.

---

## 2. Stage 1.5 — 다투던 전제, 데이터로 정정 (병렬 적대 검증)

원래 Stage 1 요약이 "CLI는 한 세션 다중 에이전트 불가"로 **과일반화**. 병렬 검증(Advocate/Adversary/Codex)이 정정:

**정정 명제(Codex 판정: 원결론 over-stated):** process-per-agent는 **"CLI 바이너리가 단일 세션만 노출하고 multiplexed 진입점이 없을 때"만** 강제. 이미 뜬 CLI 프로세스 둘의 사후 병합만 불가(OS 사실). **API/SDK substrate는 한 프로세스가 N agent loop(async task)** — 실증:

| 방식 | 동일 프로세스? | 메커니즘 |
|---|---|---|
| Claude Agent SDK (`agents` param) | 예(가능성 높음) | 단일 프로세스 내 async `Agent` 도구 호출, 격리는 컨텍스트 윈도우 단위 |
| **Claude Code Agent Teams** | **아니오(확실)** | CLI 인스턴스별 별도 OS 프로세스, tmux pane 분리 |
| AutoGen v0.4 | 예(확실) | 단일 Python 프로세스, asyncio 이벤트 루프 |
| CrewAI | 예(확실) | 단일 프로세스 내 Agent 객체 + LLM API 호출 |
| LangGraph | 예(확실) | 단일 프로세스 내 그래프 노드(함수/async task) |

**용어 정정(혼선 원인 — "session"이 둘을 뭉갬):**
- **Agent Teams = 다중 세션/다중 프로세스** (확실; "each teammate is a separate Claude instance", session ID + tmux pane). → engram 현 모델과 동류, 메모리 절감 대상 아님.
- **Subagents = 단일 세션** ("work within a single session"). SDK `agents`=in-process 객체(가능성 높음). CLI Task 경로 subprocess 여부 **불확실(공식 미확인)**.

**경계(Adversary 데이터):**
- 사후 병합 불가 = OS 사실(PID별 메모리/fd/시그널 소유, 병합 API 없음).
- CLI가 daemon/mux 모드를 노출하면 단일 프로세스 다중 세션 가능 — 실존: **tmux server**(세션=서버 내 구조체), **Zellij**, **lspmux/karellen-lsp-mcp**(LSP 멀티세션 데몬). claude CLI 해당 모드 지원 **불확실**.
- 공동 호스팅 비용: 결함 격리(Chrome site isolation = 렌더러 crash 격리·OS sandbox·Spectre 차단 위해 프로세스 분리), 단 Erlang VM은 단일 OS 프로세스 내 수천 경량 프로세스 격리(런타임 격리는 OS 격리 없이도 가능, 메모리/sandbox는 없음).
- **가장 좁은 정확한 명제:** "process-per-agent는 ① 에이전트 간 OS 수준 sandbox/자격증명/메모리 격리가 요구되거나 ② CLI가 multiplexed 진입점을 안 줄 때만 강제. CLI daemon 모드가 있거나(tmux/LSP) 신뢰된 동일 사용자 에이전트를 async 런타임으로 격리하면 단일 프로세스 다중 호스팅 가능."

**결론:** "한 세션 다중 에이전트"의 *메모리* 이득은 **API substrate에서만**. CLI(claude_console)는 정직하게 process-per-agent.
출처: code.claude.com/docs/en/{agent-teams, agent-sdk/subagents}, microsoft.github.io/autogen, tao-of-tmux(server), chromium process-model & site-isolation, codeberg.org/p2502/lspmux, github.com/karellen/karellen-lsp-mcp, moldstud.com(Erlang isolation)

---

## 3. engram 현 seam 매핑 (코드 확인 — 리뷰 정정 반영)

- `Capabilities { input, output, control, session, model } = compose(TransportCaps, BackendCaps)` (domain.rs / types.rs:141). **2-source 머지**: transport=input/output/control, backend=session/model. → 호스팅 모델은 **transport substrate가 결정**하므로 `TransportCaps` 안 필드가 맞다(새 top-level 차원으로 두면 compose 시그니처+wire struct+ts-rs+PROTOCOL_VERSION bump). `ControlCaps`에 `cancel`/`graceful_shutdown` 필드는 *존재*하나 API용으로 *세팅된 건 아님*.
- `AgentTransport`(start/send_input/resize/interrupt/shutdown/capabilities) = 자원 수명 책임. **`transport/api.rs`는 no-op 껍데기**(`pub struct ApiTransport;` 무상태, 모든 caps=false, cancel=false 확인). **trait 경계만 존재 — 구현은 greenfield**(HTTP client·stream task·cancel token·세션 상태 전부 신규). "stub 채우기"가 아님.
- kill 인과 2동사(ADR-0001: shutdown→master drop→reader EOF→pump break→core.finish→done_tx, 이후 join_pump 5s)는 **PTY 전제**. in-process API는 **master fd 없음** → 누가 pump인지·done 신호를 무엇이 만드는지·`join_pump` 동기 5s 대기가 공유 tokio runtime에서 합법인지 **재설계 필요**.
- 결함 격리: reaper(catch_unwind per pump, reaper.rs)는 **per-process/per-thread 결함 도메인** 전제. async task 공유 runtime이면 한 task panic/deadlock이 executor starvation으로 전체 pump·reaper를 멈출 수 있음 — 현 supervision 미방어.

---

## 4. Stage 2 — 융합 선택지 (사용자 결정용, phased)

메모리(substrate)와 제어(orchestration)는 별개 축. in-process는 격리/finalize retrofit 비용이 커서 단계적 제시.

**옵션 0 — 호스팅 불변 + 실무 가드** _(최저비용 baseline, near-term)_
maxParallelAgents 상한+큐(issue #15487 방증) + lazy spawn(Crystal) + warm pool(SDK `startup`). 이득: 아키텍처 손 안 대고 메모리 폭주 *실무* 완화. 비용: agent당 근본 RAM 그대로. 다른 옵션은 이 대비 추가비용 정당화 필요.

**옵션 1 — API backend in-process (hosting = TransportCaps 필드)**
호스팅 모델을 `TransportCaps` 안 필드로(새 차원 아님 — compose 2-source·ADR-0002 정합). `codex_api` transport를 async task로 신규 구현. 이득: API 에이전트 task당 ~MB 가능 → 다수 동시 저메모리(**수치 미측정·UNCERTAIN**; CLI 대비 기능·auth·세션 의미 다를 수 있음). **미해결 retrofit:** master fd 부재로 ADR-0001 kill 인과·finalize-once·join_pump 안 맞음 → cancel→synthetic terminal→core.finish + done 생성자 재정의. 격리 per-process→per-task 약화. 거부대안: "CLI도 in-process" → OS상 불가.

**옵션 1b — engram 자체 out-of-process API worker pool** _(리뷰 제안 — 격리 보존형)_
별도 `engram-worker` 바이너리가 K개 API agent 호스팅(daemon은 이미 프로세스 spawn). 이득: RAM 이득 + **프로세스 격리 유지**(옵션1 finalize/starvation 회피), 벤더 의존 없이 지금 구현 가능. 비용: IPC/worker 프로토콜·재스케줄·부분실패. 거부대안: "in-process 단일 프로세스" → 격리 상실.

**옵션 2 — cwd 팀 오케스트레이션 + lifecycle 제어** _(RAM과 별개 축, §5)_
cwd로 묶은 논리 "팀"을 supervisor가 라우팅·컨텍스트 공유(Gastown Witness/Seance). **mixed CLI+API 이종 backend 한 팀**(통합 status/output/kill/restart). 팀 lifecycle을 LLM 핸들로: start/pause-all/cancel-all/kill-all/restart-failed/restart-team/drain/archive. 이득: "같은 폴더 여러 에이전트 한 곳 제어" UX + LLM-우선. **RAM 안 줄임** → 옵션 1/1b와 결합해야 메모리까지. 거부대안: "cwd 팀=CLI 한 프로세스" → claude multiplexed 세션 없음(불확실).

**옵션 3 — 벤더 daemon-mux 호스팅** _(미래 게이트, 비-액션)_
claude/codex가 daemon·multiplexed-session 노출 시 K agents/worker. enum만 예약(저위험·§0), 구현 보류. 벤더 지원 불확실.

### 가로지르는 차용 (정정 반영)
- **supervision:** reaper `RestartRecord`(OTP restart-intensity {MaxR,MaxT}+backoff)는 **ADR-0019가 retire한 "런타임 자동재시작"을 재오픈**하는 결정 — 예약 필드 활성화 아님(ADR-0016 예약은 부팅복원/가드/Failed 한정, profile.rs:55). 채택 시 ADR-0019 번복 ADR 필요.
- **resumability("처음부터" 통증):** S9 `--session-id` = LangGraph `thread_id` 동형. 출력 replay SQLite 영속(Crystal)으로 콜드부팅 복원.
- **capability 노출:** A2A Agent Card 형식으로 capability 매트릭스 LLM 조회(§5). §5는 "LLM이 모든 UI 동작 호출 가능"까지 요구 → 팀 lifecycle 동사 전부 tool/command 스키마화 필요.

### 결정해야 할 갈림길 (사용자)
1. **메모리 접근:** 옵션 0(가드만) / 옵션 1(in-process, 단순·격리약함·retrofit 큼) / 옵션 1b(out-of-proc pool, 격리보존·IPC비용).
2. **팀/제어(옵션 2)** 지금 vs 나중.
3. **supervision 자동재시작**(=ADR-0019 재오픈) 할지.

---

## 5. Stage 3 — 2인 적대 리뷰 (완료: 둘 다 FIX, 수렴 — 에스컬레이션 불요)

- **Codex(User/완결성 렌즈) FIX:** mixed CLI+API 팀·팀 lifecycle·crash/resume·§5 깊이 미흡 / 옵션0 승격 / 메모리 수치 UNCERTAIN / "다음 주에 뭐부터?"의 phased 권고 부재. → 반영.
- **opus(Tester/breaker 렌즈, doc-aware) FIX (코드 확인):** ① kill 인과/finalize/join_pump retrofit "명명만 됨"(manager.rs:462-483, reaper.rs:178-198, session.rs) ② per-task 격리 약화 미반영(reaper catch_unwind는 executor starvation 미방어) ③ "stub 채우기" 과장 — api.rs 무상태·caps=false ④ HostingCaps는 새 차원 아니라 TransportCaps 필드(compose 2-source, types.rs:141) ⑤ 누락 대안=engram 자체 out-of-proc worker pool(옵션 1b) ⑥ RAM 수치 UNCERTAIN 미전파 ⑦ RestartRecord=ADR-0019 재오픈(profile.rs:55). → 전부 반영.
- **불일치 없음** → 사용자 에스컬레이션 불요.

### 잔여 UNCERTAIN (측정·확인 필요)
- codex_api 실제 메모리(stream/세션/tool 상태 포함 실측).
- CLI Task-tool 경로 내부 subprocess 여부(공식 미확인).
- claude/codex daemon-mux 모드 지원 여부.

---

## 6. 통합 출처

- **Anthropic:** code.claude.com/docs/en/{agent-sdk/typescript, agent-sdk/subagents, agent-teams}, claude.com/blog/building-agents-with-the-claude-agent-sdk, docs.anthropic.com/.../sdk-sessions, platform.claude.com/docs/en/managed-agents/multi-agent, github.com/anthropics/claude-code/issues/{4182,15487,19045,20095}
- **서드파티 매니저:** github.com/{smtg-ai/claude-squad, stravu/crystal, kbwo/ccmanager, BloopAI/vibe-kanban}, conductor.build
- **named tools:** github.com/gastownhall/gastown, steve-yegge.medium.com/welcome-to-gas-town, github.com/ruvnet/ruflo
- **프레임워크:** docs.langchain.com/.../persistence, docs.temporal.io/workflow-execution, spheron.network/blog(temporal-restate), devblogs.microsoft.com/agent-framework, arxiv.org/pdf/2504.16736
- **Rust supervision:** erlang.org/doc/apps/stdlib/supervisor.html, github.com/slawlor/ractor, docs.rs/ractor-supervisor, github.com/ZelAnton/ProcessKit-rs
- **호스팅 경계:** tao-of-tmux(server), chromium process-model & site-isolation, codeberg.org/p2502/lspmux, moldstud.com(Erlang isolation)

---

## 7. 후속

- **위치/발견:** `docs/research/`에 거주 — 해당 주제(호스팅·오케스트라·메모리) 착수 시 참조. tracking 미등재(사용자 결정 2026-06-24: 액션 보류항목이 아니라 *참조 자료*라서). 같은 줄기 참조: T-9(claude 풀링)·`control-surface-and-fleet.md`.
- **사용자 결정 후** 굵은 선택 → ADR(거부 대안 포함). supervision 자동재시작 채택 시 **ADR-0019 번복 ADR** 필요. hosting=TransportCaps 필드 채택 시 ADR-0002/0030 연장.
- **다음 세션 TODO(미편입):** "오케스트라 관점 5축 정리"(토폴로지·에이전트 수명·두뇌(제어주체)·컨텍스트 공유·supervision) — 대화에서 도출했으나 doc 본문 미반영. 다음에 별도 섹션으로.
