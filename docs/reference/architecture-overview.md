# 아키텍처 조감도 — 클라이언트 ~ 서버 전체

> 이 문서는 **코드 지도(orientation)**다. 단일 출처는 언제나 코드·grep(`// ADR-` 앵커) — 여기 line 번호를 안 박는 건 rot 방지다. 결정의 *왜*는 `decisions/` ADR, *언제/무엇*은 `process/step-log.md`.
>
> **두 부분이다.** [PART 1](#part-1--5분-조감도-처음-오는-사람용)은 처음 오는 사람용 5분 조감도, [PART 2](#part-2--심화-레퍼런스-유지보수자용)는 유지보수자용 심화 레퍼런스. 신규자는 **PART 1만 읽어도 시스템 형태가 잡힌다**. 세부를 고치러 왔으면 PART 1의 "읽기 경로"가 PART 2의 해당 지점으로 안내한다.
>
> 기준: S16(ADR-0046, 미러 버퍼 제거·뷰 직결 replay) 반영. 2026-07 스냅샷.

---

# PART 1 — 5분 조감도 (처음 오는 사람용)

## 먼저 알 용어 5개

이것만 알면 아래 그림이 읽힌다. (풀 사전은 PART 2 끝 [§용어 사전](#용어-사전-혼동쌍-고정))

- **에이전트(agent)** = claude 프로세스. 우리가 띄우고 관리하는 대상. "에이전트 재시작" = epoch 교체.
- **클라이언트(client)** = 앱 실행파일(`engram-dashboard.exe`, src-tauri 셸). 데몬에 붙는 손님.
- **데몬(daemon)** = 에이전트 호스팅 서버(`engram-dashboard-daemon.exe`). 생사·출력·상태의 진짜 주인.
- **웹뷰(webview)** = 창(WebView2) · **슬롯(slot)** = 그 창 안 레이아웃 한 칸.
- **replay** = 데몬이 보관한 출력 되감기(리로드·신규 구독 때 과거 복원). **epoch** = 에이전트 재시작 카운터(낡은 프레임 거르는 기준).

## 5분 요약 — 핵심 5문장

1. **앱은 클라이언트 셸일 뿐이다** — 화면을 그리고 명령을 중계할 뿐, 에이전트를 소유·저장하지 않는다.
2. **데몬이 진짜 주인이다** — 에이전트 생사·출력 버퍼(replay)·상태의 단일 출처.
3. **프론트가 뷰별 진도를 소유한다** — replay·중복제거(dedup) 상태는 슬롯마다 프론트가 갖고, 그 사이 Rust 클라이언트는 무상태 프레임 라우터다.
4. **손발/두뇌를 나눈다** — 프론트는 렌더링만(두뇌 아님). 모든 제어는 백엔드측이 쥐고 사람 클릭은 보조다. (불변 원칙 = CLAUDE.md §5)
5. **"왜"의 출처는 여기가 아니다** — 근거·거부한 대안은 코드의 `// ADR-` 앵커와 `decisions/` ADR에 있다. 이 문서는 지도지 진실의 출처가 아니다.

## 큰 그림 — 3 프로세스 + 2 exe

```
┌──────────────────────────────────────────────────────────────────────┐
│  engram-dashboard.exe          (앱 = 클라이언트 셸, src-tauri crate)     │
│  ┌────────────────────────────────────────┐                           │
│  │  WebView2 창                             │   + 시스템 트레이          │
│  │   React 프론트 (src/)                    │   + 창 관리                │
│  │   = 순수 I/O (출력 표시 · 입력 캡처)      │                           │
│  └────────────────────────────────────────┘                           │
│         ▲  invoke(명령)          │  Channel(출력 프레임)                 │
│         │                        ▼                                     │
│  ┌────────────────────────────────────────┐                           │
│  │  DaemonClient (Rust)  = 무상태 라우터     │                           │
│  │   WS 클라이언트 · 프레임 중계만 (버퍼 X)   │                           │
│  └────────────────────────────────────────┘                           │
└───────────────────────────┬──────────────────────────────────────────┘
                            │  WebSocket (127.0.0.1, 토큰 인증)
                            │  업링크=JSON 명령  ·  다운링크=바이너리 출력
                            ▼
┌──────────────────────────────────────────────────────────────────────┐
│  engram-dashboard-daemon.exe   (데몬 = 백엔드 서버, daemon crate)        │
│  ┌────────────────────────────────────────┐                           │
│  │  AgentManager  (core 엔진 소유)          │                           │
│  │   sessions · profiles · reaper           │                           │
│  └────────────────────────────────────────┘                           │
│         │ 각 에이전트 = AgentTransport(PTY/stdio)                        │
└─────────┼──────────────────────────────────────────────────────────────┘
          ▼  PTY / 파이프
   ┌──────────────┐  ┌──────────────┐
   │ claude.exe   │  │ claude.exe   │  ...  (에이전트 N개)
   └──────────────┘  └──────────────┘

   부팅 시: 앱이 daemon.json(포트파일) 읽어 데몬 발견 → 없으면 spawn (discovery crate)
```

결정: DaemonClient 무상태·데몬 단일 주인 = ADR-0029 / ADR-0046.

## 상태는 누가 갖나 — 소유권 지도

시스템을 이해하는 가장 빠른 길: **"이 상태는 누구 것인가"**. 헷갈리면 흐름을 못 따라간다.

| 상태 | 소유자 | 비고 |
|------|--------|------|
| 에이전트 생사·세션 | 데몬 `AgentManager` | 단일 출처 |
| 출력 버퍼(replay) | 데몬 `OutputCore` 링 | 클라이언트는 미러 안 함 |
| 프로필 영속(session-id·epoch) | 데몬 `ProfileRegistry` → agents.json | 세이브데이터 |
| 데몬 발견 정보(포트·토큰) | daemon.json 포트파일 | 휘발(매 기동 재발행) |
| replay 진도·dedup·gen | **프론트 뷰(viewId)** | Rust는 무상태 |
| 레이아웃·테마 | 프론트 Zustand(+장차 localStorage) | 백엔드 불가지 |

결정: 미러 제거 = ADR-0046 · 레이아웃 권위 = ADR-0035 · data_dir 단일결정 = ADR-0024.

## 읽기 경로 — 뭘 고치러 왔나

세부는 PART 2에 있다. 목적별 진입점:

- **출력이 안 나온다/깨진다** → PART 2 [출력 흐름](#출력-흐름-메인-claude--앱) + [프론트 상태기계](#프론트-제어표면--protocolclient-상태기계) + E2E [출력 시나리오](#출력-에이전트--여러-슬롯)
- **리로드하면 이력이 안 돌아온다** → PART 2 [replay 상태기계](#프론트-제어표면--protocolclient-상태기계) + E2E [리로드 시나리오](#리로드--재구독--전체-replay)
- **스폰/kill 생사가 이상하다** → PART 2 [죽음 흐름](#죽음-흐름-종료--정리) + [핵심 불변식](#핵심-불변식-서버--클라이언트) + E2E [스폰 시나리오](#스폰-ui-클릭--에이전트-생성)
- **새 백엔드/전송을 붙인다** → PART 2 [4대 seam](#4대-seam-교체점) + [crate 계층](#crate-계층-의존-아래위)

**여기까지가 조감도다.** 아래 PART 2는 필요할 때 찾아보는 레퍼런스다.

---

# PART 2 — 심화 레퍼런스 (유지보수자용)

> **범례.** ★ = **seam(교체점)** — trait로 구현된 경계, 코어는 이 뒤를 절대 안 본다. 다이어그램의 화살표(→ ▼ ▲)만 "방향"을 뜻한다(산문·표에선 안 쓴다).

## 프로세스 경계와 통신 수단

**경계마다 통신 수단이 다르다.** 이걸 헷갈리면 흐름을 못 따라간다.

```
프론트 컴포넌트 ──(agentClient 인터페이스)──▶ ProtocolClient ──▶ TauriTransport
                                                                     │
   ┌── invoke(명령: JSON) ────────────────────────────────────────────┤
   └── Channel(출력: 바이너리 프레임) ◀────────────────────────────────┤
                                                                     ▼
                                                              DaemonClient(Rust)
                                                                     │
   ┌── WS Text  (명령 JSON: Spawn/Kill/Write/Subscribe…) ─────────────┤
   └── WS Binary(출력 프레임 + replay 마커) ◀──────────────────────────┤
                                                                     ▼
                                                              데몬 WS 서버
                                                                     │
                                                              AgentManager
                                                                     │
   ┌── stdin  (입력) ──────────────────────────────────────────────────┤
   └── stdout (출력) ◀─────────────────────────────────────────────────┤
                                                                     ▼
                                                              claude.exe
```

| 경계 | 수단 | 방향 | 싣는 것 |
|------|------|------|---------|
| 컴포넌트 ↔ agentClient | 함수 호출(TS 인터페이스) | 양방향 | 제어표면 |
| 프론트 ↔ 클라이언트(Rust) | `invoke` / Tauri `Channel` | 명령↑ · 출력↓ | JSON 명령 / 바이너리 프레임 |
| 클라이언트 ↔ 데몬 | WebSocket | Text↑ · Binary↓ | 명령 JSON / 출력·마커 |
| 데몬 ↔ 에이전트 | PTY(ConPTY) 또는 파이프 | stdin↑ · stdout↓ | raw 바이트 / (json)NDJSON |

결정: 제어표면 단일화 = ADR-0011.

## 서버측 — 데몬 + core 엔진

### crate 계층 (의존 아래→위)

**실행 산출은 `daemon.exe` 하나**, 나머지는 그것이 쓰는 라이브러리다.

```
protocol ◀── core ◀── daemon          (실행 나오는 건 daemon.exe 하나)
   ▲          ▲          │
   └── discovery ◀────────┘

protocol  : 앱↔데몬 공용 언어(명령·이벤트 타입 + 프레임 codec + ts-rs)  [lib]
core      : 에이전트 엔진(tauri import 0, seam: transport/backend)      [lib]
discovery : 데몬 찾기/띄우기 + default_data_dir 단일결정               [lib]
daemon    : AgentManager 소유 + WS 서버 + 단일인스턴스 + 포트파일       [lib+exe]
```

### core 클래스 구조 (소유 관계)

**데몬이 `AgentManager` 하나를 소유하고, 그 아래 에이전트마다 세션 조립체가 달린다.**

```
데몬 프로세스
 └─ Arc<AgentManager> ·········································· 관리자
      ├─ sessions: RwLock<HashMap<AgentId, Arc<AgentSession>>>
      ├─ profiles: Arc<ProfileRegistry> ····· 영속(agents.json 세이브)
      ├─ status_sink: Arc<dyn StatusSink> ★seam ··· 상태/목록 출구(control plane)
      └─ Reaper (백그라운드 스레드) ················· 사망 수거

 각 AgentSession = 에이전트 1개  (조립체)
  ├─ id · cwd · epoch(재spawn 카운터) · intent(kill 의도) · encoder(입력 포장)
  │
  ├─ Arc<OutputCore> ································· 출력 두뇌
  │    ├─ seq(순번) · status · finalized(종료 1회 게이트)
  │    ├─ replay 링 (2MB / 4096개 상한)  ← 리로드·신규구독 되감기 원천
  │    ├─ subscribers: Vec<Arc<dyn OutputSink>> ★seam ·· 출력 출구(data plane)
  │    └─ on_terminal 훅 ─────────────▶ Reaper
  │
  └─ Box<dyn AgentTransport> ★seam ················· 연결 손발
       ├─(impl) PtyTransport ······ ConPTY, 터미널 raw 바이트
       └─(impl) StdioTransport ···· 파이프 + Box<dyn OutputDecoder>

 spawn 순간에만 등장 (세션이 오래 안 들고 있음):
  AgentBackend ★seam  (impl: ClaudeBackend / ShellBackend)
    └─ CommandSpec 생성 + encoder/decoder를 세션·transport에 주입
```

★ seam(교체점) 4종: `AgentTransport`(전송) · `AgentBackend`(모델) · `OutputSink`/`StatusSink`(UI). 코어는 이 뒤를 절대 안 본다 → tauri-free · 교체 가능 · headless 테스트. 상세는 아래 [4대 seam](#4대-seam-교체점).

### 출력 흐름 (메인: claude → 앱)

**claude stdout → 펌프 → OutputCore → sink → 앱.** 코어는 raw만 알고 wire는 모른다.

```
claude 프로세스 stdout
        │
        ▼
 Transport 펌프 스레드 (read 루프)
        │   PTY  : raw 바이트 그대로
        │   stdio: OutputDecoder가 NDJSON → OutputEvent 파싱  (★claude 지식 여기까지만)
        ▼
 OutputCore.emit(event)
        │   ① seq 붙여 replay 링에 먼저 저장   ← 구독 타이밍 경쟁에서 유실 방지
        │   ② 구독자 스냅샷 뜨고 → 락 놓고 send  ← 블로킹 중 락 X
        ▼
 subscribers: OutputSink.send(frame) ★seam   ← 코어의 유일한 출구 (raw만, wire 모름)
        │
        ▼
 데몬 WsOutputSink → 바이너리 WS 프레임 → 클라이언트 → 웹뷰 슬롯
```

결정: 락 순서 = ADR-0006 · OutputSink wire 무지 = ADR-0003.

### 입력 흐름 (사용자/LLM → claude)

**입력은 세션이 encoder로 포장해 transport로만 나간다.**

```
입력 (사용자 타이핑 or LLM invoke)
        ▼
 AgentSession.write_input(bytes)
        │  encoder.encode() :  Raw(그대로)  |  ClaudeStreamJson(JSON 포장)
        ▼
 AgentTransport.send_input() ──▶ claude stdin
        │
        └─(json 모드만) 유저 에코를 OutputCore.emit ──▶ 화면에 내 입력 표시
                                                        (PTY는 로컬 에코라 불필요)
```

결정: json 모드 배선 = ADR-0044.

### 죽음 흐름 (종료 → 정리)

**종료는 딱 한 번만 확정되고(finalize 1회), 수거는 Reaper 단일 소비자가 한다.**

```
claude 종료  →  펌프가 EOF 감지
        ▼
 OutputCore.finish()  [finalized.swap 1회 게이트 — 딱 한 번만]
        ├─ status → terminal(Killed/Exited/Failed)
        ├─ StatusSink.status_changed()
        └─ on_terminal 훅: intent · shutting_down 을 "얼려서(freeze-frame)" ReapMsg 발사
                 ▼
        Reaper 스레드 (단일 소비자)
             ① epoch 확인 (낡은 사망 메시지 버림)
             ② 세션 맵에서 제거 → Arc drop(자원 해제)
             ③ 종료 분류: 유저kill=프로필삭제 / 정상 / 크래시=auto_restore 끔
             ④ StatusSink.agent_list_updated() ──▶ 앱 목록 갱신
```

결정: finalize 1회·freeze-frame 수거 = ADR-0019.

## 클라이언트측 — src-tauri 셸 + 프론트

### src-tauri = 무상태 라우터

**미러 버퍼·per-view 커서는 전부 제거됐다.** Rust는 프레임 헤더만 보고 창별 Channel로 중계한다.

```
데몬에서 온 바이너리 프레임/마커
        │
        ▼
 connection.rs main_loop (WS 수신)
        │  decode_frame → {tag, agentId, epoch, seq, payload}
        │  decide_epoch: 낡은 epoch면 드롭
        ▼
 OutputRouter.targets(agentId)  → Arc<[window_label]>   (lock-free, ArcSwap)
        │  (레이아웃 바뀔 때만 rebuild: agentId→[창] 역인덱스)
        ▼
 send_to_windows(registry, labels, bytes)   ← 버퍼 X, 커서 X, raw 그대로
        │  WindowChannelRegistry: window_label → Tauri Channel
        ▼
 각 웹뷰 창의 OutputChannel
```

- **상태 없음:** 진도·dedup·replay는 전부 웹뷰(프론트)가 소유. Rust는 "누구 프레임을 어느 창으로" 라우팅 + single-flight replay 세대만 관리.
- **replay 세대(single-flight):** 프론트가 `request_replay(agentId)` invoke → Rust가 데몬에 Subscribe 발사(진행 중이면 병합) → 완료 시 **tag=255 마커**를 프레임과 **같은 Channel 경로로** 보냄(순서 보존).
- **프론트 직접 Subscribe 금지:** `forward_daemon_command`가 Subscribe/Unsubscribe를 차단(BLOCK-1). 구독은 layout/replay 경로로만.

결정: 무상태 라우터 = ADR-0046 · 프론트 직접 Subscribe 금지 = ADR-0041.

### 프론트 제어표면 + protocolClient 상태기계

**컴포넌트는 `agentClient` 인터페이스에만 의존하고, 구독 키는 agentId가 아니라 viewId(슬롯 id)다.**

```
프론트 컴포넌트/스토어
        │  (agentClient 인터페이스에만 의존 — ptyApi 직접호출 X)
        ▼
 ProtocolClient  (carrier-agnostic, 운영 carrier = TauriTransport 고정)
        │  subs: Map<viewId, SubState>   ← 구독 키 = viewId(슬롯 id), NOT agentId
        │       └ 같은 에이전트를 여러 슬롯에서 독립 진도로 봄
        ▼
 각 SubState = { agentId, phase, buffer[], myGen, epoch, lastDeliveredSeq, attempts }
```

**뷰별 replay 상태기계 (phase):**

```
   subscribeOutput(viewId, agentId)
        │
        ▼
   ┌──────────┐   프레임 들어오면 buffer[]에 쌓음
   │ buffering │   (epoch↑면 버퍼 버리고 재요청 / 오버플로면 재요청)
   └────┬─────┘
        │  tag=255 마커 도착 & 성공 & marker.gen ≥ myGen & epoch 일치
        │      → buffer 정렬·dedup 후 flush
        ▼
   ┌──────────┐   프레임 = 즉시 dedup(seq>lastDeliveredSeq) → onChunk
   │   live    │
   └────┬─────┘
        │  재시도 3회 소진(watchdog 10s / backoff 1s·2s·4s)
        ▼
   ┌──────────┐
   │   error   │   (remount·reconnect 시 buffering으로 리셋)
   └──────────┘
```

- **gen 펜스(핵심):** replay 요청마다 고유 `myGen`(BigInt) 발급. 도착한 마커의 `gen`이 내 `myGen`보다 작으면 **무시**(옛/남의 replay가 dedup 하한선을 오염시키는 것 차단). `gen ≥ myGen`이고 epoch 맞을 때만 buffering→live 전환.
- **팬아웃:** 한 agentId 프레임 → 그 agentId를 보는 **모든 viewId**에 각자 dedup 후 전달.

결정: 구독 키=viewId·gen 펜스 = ADR-0046 · carrier 고정 = ADR-0036 · 제어표면 단일 = ADR-0011.

### 슬롯 렌더 분기

**슬롯은 에이전트 capability를 보고 렌더러를 고른다** — 출력 종류를 가정하지 않는다.

```
ViewLayoutRenderer (레이아웃 트리 → 슬롯)
   mode = renderModeOverride[slotId] ?? (agent.capabilities.output.structured ? 'rich' : 'terminal')
        │
        ├─ 'terminal' → TerminalSlot : tag=0만 받아 xterm.write
        ├─ 'rich'     → RichSlot     : tag=1만 받아 StructuredEvent 파싱 → 칩+마크다운+턴 구분선
        └─ 'dom'      → DomSlot      : ANSI 벗겨 <pre> (CDP innerText 관측용 — LLM 제어, CLAUDE.md §5)

 구독 effect deps = [viewId, agentId, epoch] · reset() 선행 · seq dedup · tag 게이트
```

## 엔드투엔드 흐름 (3 시나리오)

### 스폰 (UI 클릭 → 에이전트 생성)

```
사용자 클릭
  → agentClient.spawnAgent(cwd)
  → ProtocolClient: {SpawnByCwd, request_id} 조립
  → TauriTransport.invoke('forward_daemon_command', cmd)
  → [Rust] BLOCK-1 통과(Subscribe 아님) → DaemonClient.send_command
  → WS Text(AgentCommand) ──▶ 데몬
  → [데몬] AgentManager.spawn_agent: 프로필 upsert → transport 선택 → OutputCore·세션 조립
           → 맵에 넣고 → 펌프 시작 → status_sink 알림
  → WS Text(AgentEvent::Spawned{request_id}) ──▶ [Rust] pending[request_id] resolve
  → invoke 반환 → 프론트 Promise resolve → 컴포넌트 렌더
  (별도로 agent-list-updated 브로드캐스트가 목록 갱신)
```

### 출력 (에이전트 → 여러 슬롯)

```
[데몬] claude stdout → 펌프 → OutputCore.emit → replay 저장 + WsOutputSink
  → WS Binary [tag|agentId|epoch|seq|payload] ──▶ [Rust] connection.rs
  → decode_frame → decide_epoch(낡으면 드롭) → OutputRouter.targets(agentId)=["main","popup"]
  → send_to_windows → 각 창 Channel.send(raw)     ← Rust는 여기까지 무상태 중계
  → [프론트] 각 창 OutputChannel.onmessage → decodeOutputFrame
  → ProtocolClient.handleOutput → 그 agentId 보는 모든 viewId에 팬아웃
        live면: seq dedup → onChunk → 슬롯 렌더
        buffering이면: buffer[]에 적재(마커 기다림)
```

### 리로드 → 재구독 + 전체 replay

```
F5 (웹뷰 리로드)
  → 새 ProtocolClient / TauriTransport 생성 (_state='down')
  → Rust가 'daemon-connection-state: connected' emit → 프론트 Channel 재등록(subscribe_output invoke)
  → 슬롯 mount → subscribeOutput(viewId, agentId)
        SubState{phase:'buffering', myGen:undefined} 생성
  → request_replay(agentId) invoke → [Rust] flight.request_replay → gen 반환(=myGen)
        [Rust]가 데몬에 Subscribe 발사 → 데몬 ring 전체를 Binary로 재전송
  → 프론트: 프레임들 buffering에 쌓임 (watchdog 10s 감시)
  → [Rust] ReplayComplete 수신 → tag=255 성공 마커 인코딩 → 같은 Channel로 전송
  → 프론트 마커 평가: gen ≥ myGen & epoch 일치 → buffer 정렬·dedup·flush → phase=live
  → 이후 프레임은 live 직접 전달
  (사용자: 과거 이력 재생 후 실시간 출력으로 이어짐)
```

> **⚠ 알려진 열린 이슈(다음 세션):** 리로드 시 새 창 Channel로 데몬 replay 재전송이 아직 완전치 않음(Rust측 미검증). 우회 = 에이전트 재배정. (step-log 참조)

## 4대 seam (교체점)

**seam = trait로 끊은 교체 경계.** 코어는 이 뒤를 안 보므로 구현만 갈아끼우면 새 전송·백엔드가 흡수된다. 아래 표의 **위 4개가 코어 seam(★)**, 맨 아래 `(프론트) transport`는 프론트측 carrier 교체점(코어 밖·별개)이다.

| seam(trait) | 무엇을 끊나 | 현재 구현 | 미래 확장 |
|-------------|-------------|-----------|-----------|
| `AgentTransport` | 전송 방식(물리) | PtyTransport / StdioTransport | API transport(껍데기만) |
| `AgentBackend` | 백엔드 프로그램(claude 인자·스키마) | ClaudeBackend / ShellBackend | codex/gemini variant |
| `OutputSink` | 출력이 나가는 wire | 데몬 WsOutputSink / 테스트 sink | 새 전송 경로 |
| `StatusSink` | 상태·목록 알림 | 데몬 broadcast | — |
| (프론트) transport | carrier | TauriTransport 고정 | WsTransport(테스트/직결) |

**설계 지향(LLM-우선 제어 = CLAUDE.md §5):** UI 컴포넌트는 store 액션 호출만, 그 액션을 LLM도 동일하게 부르는 단일 control surface로 모은다. 현 갭 — UI/레이아웃은 아직 프론트(Zustand) 전용, LLM 제어 표면 미비.

## 핵심 불변식 (서버 + 클라이언트)

**변경 금지.** 근거·거부 대안은 각 ADR에 있다.

- **kill 2동사:** `transport.shutdown()`(child.kill+wait → Job terminate → master drop) → `core.join_pump(5s)`. master drop이 reader EOF를 부르고, 그게 pump break → finish로 이어진다. **순서 뒤집으면 hang.** (ADR-0001)
- **finalize 1회:** `finalized.swap`로 종료 전이·알림·수거를 정확히 1회. (ADR-0019)
- **락 순서:** emit은 replay·subscribers 락을 동시 보유 안 함(스냅샷 후 락 놓고 send). subscribe만 예외로 두 락을 순서대로(subscribers→replay) 잡아 replay→live 역전 방지(C4). (ADR-0006)
- **sink 2평면:** `OutputSink`(고빈도·구독단위 출력=data plane) ≠ `StatusSink`(저빈도·전역 상태/목록=control plane). 프론트는 종료를 `status_changed` 아닌 `agent_list_updated`로 판정. (ADR-0005)
- **freeze-frame 수거:** 사망 순간의 intent·shutting_down을 얼려 판정 → 크래시↔kill 오분류 경쟁 차단. (ADR-0019)
- **epoch:** 같은 AgentId 재시작마다 +1. reaper가 낡은 사망 메시지를, 프론트가 낡은 프레임을 거르는 기준. (ADR-0007)
- **백엔드 격리:** claude 전용 인자·JSON 스키마는 `backend/claude.rs`에만. session=encoder 태그만, transport=스키마 모르는 "바보 파이프". (ADR-0004)
- **capability 합성:** `Capabilities::compose(transport, backend)` — input/output/control은 transport, session/model은 backend가 소유(타입으로 강제). (ADR-0030)

## ADR 근거 맵 (더 파려면 여기)

- **0001** kill 2동사 · **0005** finalize/알림 분담 · **0006** 락 순서 · **0007** epoch
- **0002/0030** capability 합성(transport ⊕ backend) · **0003** OutputSink wire 무지
- **0004** 백엔드 격리 · **0044** json 모드 배선 · **0045** 출력 구조화(decoder)
- **0012** 모듈 격리·TDD · **0019** reaper freeze-frame 수거
- **0029** embedded 제거(데몬 단일) · **0036** transport 단일화 · **0035** 레이아웃 권위=src-tauri
- **0011** 제어표면 단일(agentClient) · **0041** 프론트 직접 Subscribe 금지
- **0046** 미러 버퍼 제거·뷰 직결 replay·gen 펜스 (0040 supersede)
- **0024** data_dir 단일 결정

## 용어 사전 (혼동쌍 고정)

- **에이전트(agent)** = claude(추후 codex/API) 프로세스. "에이전트 재시작" = epoch 교체.
- **클라이언트(client)** = src-tauri 셸(앱 exe). 데몬에 붙는 손님. "클라이언트 재시작" = 앱 창 재실행.
- **데몬(daemon)** = 에이전트 호스팅 서버(daemon.exe). "데몬 재시작" = 서버 프로세스 교체.
- **웹뷰(webview)** = 창(WebView2). **프론트 컴포넌트** = 웹뷰 안 React 부품. **슬롯(slot)** = 레이아웃 한 칸(viewId).
- **transport(전송)** = 물리 연결(PTY/파이프/WS). **backend(백엔드)** = 프로그램 지식(claude 인자).
- **OutputSink**(출력 출구, 고빈도) ≠ **StatusSink**(상태 출구, 저빈도).
- **replay** = 데몬 ring 되감기(리로드·신규구독 복원). **gen 펜스** = 옛/남의 replay 무시하는 세대 검사.
- **epoch** = 같은 AgentId 재시작 카운터. 낡은 프레임·사망메시지 거르는 기준.
- **freeze-frame** = 사망 순간의 판정 재료(intent·shutting_down)를 얼려 나중 오분류 차단.
