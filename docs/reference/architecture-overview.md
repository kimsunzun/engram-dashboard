# 아키텍처 조감도 — 클라이언트 ~ 서버 전체 (그림 위주)

> **이 문서는 코드 지도(orientation)다.** 단일 출처는 언제나 코드·grep(`// ADR-` 앵커). 여기 line 번호를 박지 않는 건 rot 방지 — 역할·흐름·경계만 그린다. 결정의 *왜*는 `decisions/` ADR, *언제/무엇*은 `process/step-log.md`.
>
> 기준: S16(ADR-0046, 미러 버퍼 제거·뷰 직결 replay) 반영 상태. 2026-07 스냅샷.

용어 규율(혼동쌍 고정): **에이전트**(claude 프로세스) ≠ **클라이언트**(src-tauri 셸) ≠ **데몬**(에이전트 호스팅 서버). **웹뷰**(창=WebView2) ≠ **프론트 컴포넌트**(웹뷰 안 React) ≠ **슬롯**(레이아웃 한 칸). 자세한 정의는 문서 끝 §9.

---

## 1. 큰 그림 — 3 프로세스 + 2 exe

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
│  │  DaemonClient (Rust)  = 무상태 라우터     │  ← ADR-0029/0046          │
│  │   WS 클라이언트 · 프레임 중계만 (버퍼 X)   │                           │
│  └────────────────────────────────────────┘                           │
└───────────────────────────┬──────────────────────────────────────────┘
                            │  WebSocket (127.0.0.1, 토큰 인증)
                            │  ▲ 업링크=JSON 명령  ▼ 다운링크=바이너리 출력
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

**핵심 3분리:**
- **손발/두뇌 분리** — 프론트는 렌더링만(두뇌 아님). 모든 제어는 백엔드측이 쥐고, 사람 클릭은 보조. (§5 불변)
- **클라이언트는 무상태** — 앱은 데몬을 찾아 붙고 프레임을 중계할 뿐, 에이전트를 소유·저장하지 않는다. (ADR-0029/0046)
- **데몬이 진짜 주인** — 에이전트 생사·출력 버퍼(replay)·상태의 단일 출처.

---

## 2. 프로세스 경계와 통신 수단

경계마다 통신 수단이 다르다. 이걸 헷갈리면 흐름을 못 따라간다.

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

| 경계 | 수단 | 방향 | 실는 것 |
|------|------|------|---------|
| 컴포넌트 ↔ agentClient | 함수 호출(TS 인터페이스) | 양방향 | 제어표면(ADR-0011) |
| 프론트 ↔ 클라이언트(Rust) | `invoke` / Tauri `Channel` | 명령↑ / 출력↓ | JSON 명령 / 바이너리 프레임 |
| 클라이언트 ↔ 데몬 | WebSocket | Text↑ / Binary↓ | 명령 JSON / 출력·마커 |
| 데몬 ↔ 에이전트 | PTY(ConPTY) 또는 파이프 | stdin↑ / stdout↓ | raw 바이트 / (json)NDJSON |

---

## 3. 서버측 — 데몬 + core 엔진

### 3.1 crate 계층 (의존 아래→위)

```
protocol ◀── core ◀── daemon          (실행 나오는 건 daemon.exe 하나)
   ▲          ▲          │
   └── discovery ◀────────┘

protocol  : 앱↔데몬 공용 언어(명령·이벤트 타입 + 프레임 codec + ts-rs)  [lib]
core      : 에이전트 엔진(tauri import 0, seam: transport/backend)      [lib]
discovery : 데몬 찾기/띄우기 + default_data_dir 단일결정               [lib]
daemon    : AgentManager 소유 + WS 서버 + 단일인스턴스 + 포트파일       [lib+exe]
```

### 3.2 core 클래스 구조 (소유 관계)

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

★ = **seam(교체점)** 4종: `AgentTransport`(전송) · `AgentBackend`(모델) · `OutputSink`/`StatusSink`(UI). 코어는 이 뒤를 절대 안 본다 → tauri-free · 교체 가능 · headless 테스트.

### 3.3 출력 흐름 (메인: claude → 앱)

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
        │   ② 구독자 스냅샷 뜨고 → 락 놓고 send  ← ADR-0006 (블로킹 중 락 X)
        ▼
 subscribers: OutputSink.send(frame) ★seam   ← 코어의 유일한 출구 (raw만, wire 모름)
        │
        ▼
 데몬 WsOutputSink → 바이너리 WS 프레임 → 클라이언트 → 웹뷰 슬롯
```

### 3.4 입력 흐름 (사용자/LLM → claude)

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

### 3.5 죽음 흐름 (종료 → 정리)

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

### 3.6 서버측 핵심 불변식

- **kill 2동사(ADR-0001):** `transport.shutdown()`(child.kill+wait → Job terminate → master drop) → `core.join_pump(5s)`. master drop이 reader EOF를 부르고, 그게 pump break → finish로 이어진다. 순서 뒤집으면 hang.
- **finalize 1회(ADR-0019):** `finalized.swap`로 종료 전이·알림·수거를 정확히 1회.
- **락 순서(ADR-0006):** emit은 replay·subscribers 락을 동시 보유 안 함(스냅샷 후 락 놓고 send). subscribe만 예외로 두 락을 순서대로(subscribers→replay) 잡아 replay→live 역전 방지(C4).
- **sink 2평면:** `OutputSink`(고빈도·구독단위 출력=data plane) ≠ `StatusSink`(저빈도·전역 상태/목록=control plane). 프론트는 종료를 `status_changed` 아닌 `agent_list_updated`로 판정(ADR-0005).
- **freeze-frame 수거(ADR-0019):** 사망 순간의 intent·shutting_down을 얼려 판정 → 크래시↔kill 오분류 경쟁 차단.
- **epoch(ADR-0007):** 같은 AgentId 재시작마다 +1. reaper가 낡은 사망 메시지를, 프론트가 낡은 프레임을 거르는 기준.
- **백엔드 격리(ADR-0004):** claude 전용 인자·JSON 스키마는 `backend/claude.rs`에만. session=encoder 태그만, transport=스키마 모르는 "바보 파이프".
- **capability 합성(ADR-0030):** `Capabilities::compose(transport, backend)` — input/output/control은 transport, session/model은 backend가 소유(타입으로 강제).

---

## 4. 클라이언트측 — src-tauri 셸 + 프론트

### 4.1 src-tauri = 무상태 라우터 (ADR-0046)

미러 버퍼·per-view 커서 전부 제거됨. Rust는 프레임 헤더만 보고 창별 Channel로 중계.

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
- **프론트 직접 Subscribe 금지(ADR-0041):** `forward_daemon_command`가 Subscribe/Unsubscribe를 차단(BLOCK-1). 구독은 layout/replay 경로로만.

### 4.2 프론트 제어표면 + protocolClient 상태기계

```
프론트 컴포넌트/스토어
        │  (agentClient 인터페이스에만 의존 — ptyApi 직접호출 X, ADR-0011)
        ▼
 ProtocolClient  (carrier-agnostic, 운영 carrier = TauriTransport 고정 ADR-0036)
        │  subs: Map<viewId, SubState>   ← 구독 키 = viewId(슬롯 id), NOT agentId
        │       └ 같은 에이전트를 여러 슬롯에서 독립 진도로 봄 (버그 B 해소 ADR-0046)
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

**gen 펜스(핵심):** replay 요청마다 고유 `myGen`(BigInt) 발급. 도착한 마커의 `gen`이 내 `myGen`보다 작으면 **무시**(옛/남의 replay가 dedup 하한선을 오염시키는 것 차단). `gen ≥ myGen`이고 epoch 맞을 때만 buffering→live 전환. (ADR-0046)

**팬아웃:** 한 agentId 프레임 → 그 agentId를 보는 **모든 viewId**에 각자 dedup 후 전달.

### 4.3 슬롯 렌더 분기

```
ViewLayoutRenderer (레이아웃 트리 → 슬롯)
   mode = renderModeOverride[slotId] ?? (agent.capabilities.output.structured ? 'rich' : 'terminal')
        │
        ├─ 'terminal' → TerminalSlot : tag=0만 받아 xterm.write
        ├─ 'rich'     → RichSlot     : tag=1만 받아 StructuredEvent 파싱 → 칩+마크다운+턴 구분선
        └─ 'dom'      → DomSlot      : ANSI 벗겨 <pre> (CDP innerText 관측용, §5 LLM)

 구독 effect deps = [viewId, agentId, epoch] · reset() 선행 · seq dedup · tag 게이트
```

---

## 5. 엔드투엔드 시퀀스 (3 시나리오)

### 5.1 스폰 (UI 클릭 → 에이전트 생성)

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

### 5.2 출력 (에이전트 → 여러 슬롯)

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

### 5.3 리로드 → 재구독 + 전체 replay

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

> ⚠️ 알려진 열린 이슈(다음 세션): 리로드 시 새 창 Channel로 데몬 replay 재전송이 아직 완전치 않음(Rust측 미검증). 우회 = 에이전트 재배정. (step-log 참조)

---

## 6. 4대 seam (교체점) 요약

| seam(trait) | 무엇을 끊나 | 현재 구현 | 미래 확장 |
|-------------|-------------|-----------|-----------|
| `AgentTransport` | 전송 방식(물리) | PtyTransport / StdioTransport | API transport(껍데기만) |
| `AgentBackend` | 백엔드 프로그램(claude 인자·스키마) | ClaudeBackend / ShellBackend | codex/gemini variant |
| `OutputSink` | 출력이 나가는 wire | 데몬 WsOutputSink / 테스트 sink | 새 전송 경로 |
| `StatusSink` | 상태·목록 알림 | 데몬 broadcast | — |
| (프론트) transport | carrier | TauriTransport 고정 | WsTransport(테스트/직결) |

**설계 지향:** UI 컴포넌트는 store 액션 호출만, 그 액션을 LLM도 동일하게 부르는 단일 control surface로 모은다(§5). 현 갭 — UI/레이아웃은 아직 프론트(Zustand) 전용, LLM 제어 표면 미비.

---

## 7. 상태(state)는 누가 갖나 — 소유권 지도

| 상태 | 소유자 | 비고 |
|------|--------|------|
| 에이전트 생사·세션 | 데몬 `AgentManager` | 단일 출처 |
| 출력 버퍼(replay) | 데몬 `OutputCore` 링 | 클라이언트는 미러 안 함(ADR-0046) |
| 프로필 영속(session-id·epoch) | 데몬 `ProfileRegistry`→agents.json | 세이브데이터 |
| 데몬 발견 정보(포트·토큰) | daemon.json 포트파일 | 휘발(매 기동 재발행) |
| replay 진도·dedup·gen | **프론트 뷰(viewId)** | Rust는 무상태 |
| 레이아웃·테마 | 프론트 Zustand(+장차 localStorage) | 백엔드 불가지(ADR-0035) |

---

## 8. ADR 근거 맵 (더 파려면 여기)

- **0001** kill 2동사 · **0005** finalize/알림 분담 · **0006** 락 순서 · **0007** epoch
- **0002/0030** capability 합성(transport ⊕ backend) · **0003** OutputSink wire 무지
- **0004** 백엔드 격리 · **0044** json 모드 배선 · **0045** 출력 구조화(decoder)
- **0012** 모듈 격리·TDD · **0019** reaper freeze-frame 수거
- **0029** embedded 제거(데몬 단일) · **0036** transport 단일화 · **0035** 레이아웃 권위=src-tauri
- **0011** 제어표면 단일(agentClient) · **0041** 프론트 직접 Subscribe 금지
- **0046** 미러 버퍼 제거·뷰 직결 replay·gen 펜스 (0040 supersede)
- **0024** data_dir 단일 결정

---

## 9. 용어 사전 (혼동쌍 고정)

- **에이전트** = claude(추후 codex/API) 프로세스. "에이전트 재시작" = epoch 교체.
- **클라이언트** = src-tauri 셸(앱 exe). 데몬에 붙는 손님. "클라이언트 재시작" = 앱 창 재실행.
- **데몬** = 에이전트 호스팅 서버(daemon.exe). "데몬 재시작" = 서버 프로세스 교체.
- **웹뷰** = 창(WebView2). **프론트 컴포넌트** = 웹뷰 안 React 부품. **슬롯** = 레이아웃 한 칸(viewId).
- **transport(전송)** = 물리 연결(PTY/파이프/WS). **backend(백엔드)** = 프로그램 지식(claude 인자).
- **OutputSink**(출력 출구, 고빈도) ≠ **StatusSink**(상태 출구, 저빈도).
- **replay** = 데몬 ring 되감기(리로드·신규구독 복원). **gen 펜스** = 옛/남의 replay 무시하는 세대 검사.
- **epoch** = 같은 AgentId 재시작 카운터. 낡은 프레임·사망메시지 거르는 기준.
- **freeze-frame** = 사망 순간의 판정 재료(intent·shutting_down)를 얼려 나중 오분류 차단.
