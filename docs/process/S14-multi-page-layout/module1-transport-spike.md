# 모듈① 전송 중계 통일 — 구현 설계 spike (ADR-0036)

**상태:** T1~T4 구현 완료 / **T5~T8 코딩 전 — D1~D5 결정 완료 + 리서치 반영(§7)**
**작성:** 2026-06-27 (dashboard2, opus Plan 에이전트 spike — 실제 코드 확인 기반) · **§7 추가 2026-06-28**(carrier/fan-out 리서치 반영)
**근거:** ADR-0036(전송 중계 통일·phasing 없음·동시성-치명) · ADR-0035(레이아웃 권위=src-tauri) · ADR-0037(전송 의미론=Rust 단독) · TRD rev.5 · **리서치 `docs/research/tauri-channel-multiwindow-carrier-research-2026-06-28.md`**

> 동시성-치명 모듈. 현 TS `wsTransport.ts`(389줄)+`protocolClient.ts`(435줄)의 전송 의미론을 src-tauri Rust로 이전. 코딩은 T1~T8 단위로, 각각 코더→`/review code deep`→QA.

---

## 0. 요약
- src-tauri에 `DaemonClient`(연결·프로토콜 의미론) + `OutputRouter`(ViewManager 라우팅, lock-free snapshot) 신설. 프론트는 `TauriTransport`(얇은 carrier) 위 기존 `ProtocolClient` 인터페이스(ADR-0011) 유지.
- **재사용 Rust 자산:** protocol crate `AgentCommand/AgentEvent/decode_frame/encode_terminal_frame`, discovery crate `read_live_daemon`·동기 tungstenite 패턴, src-tauri `ViewManager`(라우팅 테이블 토대).
- **첫 장애물:** src-tauri Cargo.toml에 tokio·tokio-tungstenite·futures-util·serde_json이 **현재 전부 없음**(ADR-0029에서 제거). 복원이 T1.
- **데몬 불변 확인:** 데몬 WS 서버(`ws.rs`)는 클라 1개든 N개든 동일, ConnId만 관리하고 View 모름 → ADR-0036 "데몬 불변" 코드상 사실.

## 1. 현행 TS가 지키는 동시성 불변식 (Rust 이전 대상)

| TS 위치 | 불변식 | race 시나리오 |
|---|---|---|
| `wsTransport` `openGen` 세대 토큰 | zombie-socket 가드. openSocket 진입마다 ++, await 재개·소켓 생성 직전 myGen!==openGen이면 폐기 | await yield 중 start()/close()가 끼면 재개된 run()이 this.ws hijack |
| `wsTransport` `pendingReject` | detach된 소켓의 resolve/reject가 closure에 갇혀 안 불림 → 밖에서 깨움 | 두 번째 start()가 첫 promise 안 깨우면 호출자 hang |
| `wsTransport` `cleanupSocket` #13133 | 핸들러 delete(null 아님) 후 close — 옛 onclose가 새 소켓 clobber 방지 | 버려진 소켓 onclose가 새 this.ws null화 |
| `wsTransport` `scheduleReconnect` | 지수 백오프 500ms→10s MAX 5 → down. attach-only(read_daemon_info no-spawn) | 명령 부수효과로 데몬 respawn(B-1) |
| `wsTransport` Auth/Hello | 첫 frame=Auth, Hello=인증성공(내부 소비) | Hello를 control로 올리면 오해 |
| ensureReady vs start (ADR-0021) | ensure=attach-only, start=명시 spawn 유일 진입점 | 데몬 끈 뒤 키/resize가 respawn |
| `protocolClient` epoch 가드 | output frame epoch≠st.epoch drop | 옛 세션 잔여가 화면 오염 |
| `protocolClient` seq high-water dedup | seq<=lastDeliveredSeq drop | 재연결 경계 중복 배달 |
| epoch 변경 시 high-water 리셋 | current_epoch 바뀌면 lastDeliveredSeq=-1 | 새 세션 출력 전멸 방지 |
| `resubscribeAll` | connected 재전이 시 epoch=st.epoch + after_seq=last → Resume(tail-only) | epoch=null이면 전체 replay 중복 |
| request_id pending map | side-effect 명령을 randomUUID로 매칭, 전용 reply variant echo | broadcast에 조회응답 편승 |
| connected→끊김 pending reject | 진행 명령 전부 reject(1회성) | promise leak |
| `eventBus` resync 가드 | connected *재*전이만 getAgents+refreshProfiles | 첫 연결 중복/끊긴 동안 변경 누락 |

**라우팅:** 현재 없음 — 각 창이 데몬에 N개 WS 직결, 자기 구독 agentId만. ADR-0036이 없애려는 N중복.

## 2. Rust 재배치 설계

### 배치
```
src-tauri/src/
  daemon_client/         ← 신규 ★동시성-치명
    mod.rs               DaemonClient (연결 핸들 + 명령 API)
    connection.rs        재연결·generation 가드·Auth/Hello (wsTransport 이전)
    protocol_state.rs    epoch·seq dedup·resubscribe·pending (protocolClient 이전)
  output_router.rs       ← 신규 ★lock-free snapshot(arc-swap)
  layout/                ← 모듈②(완료), OutputRouter가 의존
  commands/agent.rs      ← 신규 invoke: spawn/kill/write/resize/subscribe
src/api/tauriTransport.ts ← 신규 Transport 구현(얇은 carrier)
```

### DaemonClient — 단일 연결 actor
- 런타임 tokio multi-thread(데몬처럼 tokio-tungstenite 0.26 + futures-util split). discovery의 동기 tungstenite는 fire-and-forget용이라 상시 연결엔 부적합 → 비동기.
- **단일 연결 task(actor)**가 `WebSocketStream`·`pending: HashMap<RequestId,oneshot>`·`subs: HashMap<AgentId,SubState>`를 단독 소유(Mutex 없이, protocolClient self-contained 상태와 동형). invoke는 `cmd_tx.send(req{cmd,reply:oneshot})` 후 await.
- generation 가드(openGen)는 **task 재시작**으로 자연 대체(옛 task는 채널 닫혀 폐기) — ★단 가정, 검증 필요(§6 리스크 1).
- 상태(connected/reconnecting/down)·control 이벤트는 `app.emit` broadcast, request reply는 oneshot.

### OutputRouter — lock-free (FIX-F6)
```rust
pub struct OutputRouter { table: ArcSwap<RoutingTable> }  // arc-swap: 읽기 lock-free
struct RoutingTable { by_agent: HashMap<AgentId, Vec<WindowSink>> }
```
- 읽기(핫패스): `table.load()` → by_agent.get → 각 WindowSink 전달. 락 0.
- 쓰기(저빈도): ViewManager mutation 후(락 드롭 뒤, emit 단계) 테이블 재계산 → `table.store`. ADR-0006 위반 없음.

### TauriTransport (프론트)
- `Transport` 인터페이스 구현, 두뇌는 Rust. `send`→invoke('agent_command'), `onMessage`→listen+출력 Channel, state→listen('daemon-connection-state'), start/ensure/close→invoke. `clientFactory`는 `new WsTransport()`→`new TauriTransport()`. agentClient=ProtocolClient 싱글톤 유지(ADR-0011). ensure/start 분리는 Rust로 이전.

## 3. 불변식 매핑 (Rust 보존 + race 위험)

| 불변식 | Rust 보존 | race 위험 |
|---|---|---|
| zombie 가드(openGen) | task 재시작(옛 task 채널 닫힘) + 명시 generation | ★높음 — "자연해결" 가정 틀리면 최위험. tokio abort 타이밍·in-flight await 재개 정밀 테스트. TS `[Blocker-1]` 2케이스 반드시 이식 |
| 재연결 백오프 | task 내 tokio sleep 루프 + read_live_daemon(no-spawn) MAX5→down | 중 |
| ensure/start 분리 | DaemonClient connect()/ensure() 분리 | 중 — 단일 클라라 다중 WebView 동시 ensure 사라짐(이득), discovery ensure_lock 불필요해질 수도(D5) |
| epoch 가드 | 연결 task SubState | 낮음(순차) |
| seq dedup | SubState last_delivered_seq, replay_from 기준 안 씀(버그B 가드) | 낮음 |
| epoch 변경 high-water 리셋 | current_epoch 바뀌면 -1 | 낮음 |
| resubscribe resume(버그A) | connected 재전이 시 subs 순회 Subscribe{epoch:Some,after_seq:last} | 중 |
| replay→live 순서 | **데몬 소유(ws.rs 단일 writer FIFO)** — 클라 seq dedup만 | 낮음 |
| finalize 1회(ADR-0005) | **데몬 소유 — 클라 무관** | 없음 |
| kill 인과(ADR-0001) | **데몬 소유 — 클라는 Kill 전송+Ack** | 없음 |
| 락 순서(ADR-0006) | OutputRouter 핫패스 락 0(arc-swap), 테이블 갱신은 ViewManager 락 드롭 후 | 중 — 갱신을 락 안에 넣으면 위반 |
| pending 매칭 | 연결 task HashMap | 낮음 |
| 끊김 시 pending reject | task 종료/재시작 시 전부 Err | 중 — task 교체 시 옛 pending 정리 누락 |

**race 핵심 3곳:** ① task 교체 중 in-flight 명령/구독(Blocker-1 이식 단언) ② OutputRouter 갱신↔ViewManager 락 순서 ③ 재연결 resubscribe와 동시 unsub/재구독.

## 4. 코더 단위 분해 (T1~T8)
```
T1 deps복원 → T2 connect/handshake → T3 protocol_state(epoch/dedup/pending)
                    │                        │
                    │                  T4 reconnect+resubscribe(★Blocker-1)
                    ▼                        │
              T5 OutputRouter(arc-swap)◄─layout(②)┘
                    │
              T6 commands/agent.rs invoke
                    │
              T7 TauriTransport+clientFactory 교체
                    │
              T8 React 정리(slotStore삭제·PopupPage·eventBus)+InProc mock(D4)
```

| T | 내용 | 격리 하네스(실 데몬 없이) |
|---|---|---|
| T1 | Cargo.toml tokio·tokio-tungstenite·futures-util·serde_json·arc-swap 복원 + AppState manage 자리 | cargo build |
| T2 | DaemonClient 연결 task: discover→WS→Auth→Hello, connect/ensure 분리 | integration bin(daemon `tests/ws_e2e.rs` start_test_server 재사용) 또는 mock WS 서버 |
| T3 | protocol_state: epoch·dedup·resubscribe·pending **순수 함수 분리** | headless unit — `protocolClient.test.ts` 21케이스 1:1 이식, SubState 직접 주입 |
| T4 | 재연결 백오프 + generation 가드 + read_live_daemon | headless unit + tokio `time::pause()` — `wsTransport.test.ts` Blocker-1·hot-swap·소진→down 이식 |
| T5 | OutputRouter arc-swap + 갱신 트리거 | headless unit — ViewManager mutation→테이블 재계산→load 검증, 핫패스 락 0 단언 |
| T6 | invoke(spawn/kill/write/resize/subscribe)→DaemonClient | integration bin |
| T7 | TauriTransport + clientFactory 교체 | vitest invoke mock |
| T8 | React 정리 + InProc mock 처리(D4) | vitest + cargo test 루트 회귀 + cdp.mjs 실측 |

**TS 테스트 2파일(40+케이스)이 Rust 이식의 명세서.** 격리 토대 이미 존재: discovery Clock/DaemonReader trait 주입, daemon start_test_server.

## 5. 사용자 결정 필요 (D1~D5)

- **D1 (가장 큼) dedup/epoch 가드 위치:** (A) Rust 단독 — ProtocolClient handleOutput 죽이고 Rust가 깨끗한 청크만 emit(단일 진실원·라우팅 전 1회, 단 ProtocolClient 껍데기화→ADR-0011 정체성 충돌) / (B) Rust 1차 + JS 방어적 2차(기존 보존·점진, 의미론 두 곳) / (C) JS 단독·Rust raw relay(라우팅 위해 agentId 부분디코드 필요 + N창 가드 N회 → ADR-0036 이점 반감).
- **D2 ProtocolClient 두께:** D1 종속. A면 거의 사라지고 TauriTransport가 곧 클라 / B·C면 거의 그대로 + carrier만.
- **D3 라우팅 출력 carrier:** Tauri Channel(바이너리 효율, 창 생명주기 결합 복잡) vs emit_to(단순, JSON 팽창). 고대역 출력 직결 → §6 성능.
- **D4 InProc mock 테스트:** `protocolClient.test.ts` InProc describe + MockTransport(ADR-0020 흔적). 폐기 / Rust 이식 / TauriTransport 재작성.
- **D5 `commands/discovery.rs` ensure_lock:** 단일 DaemonClient면 다중 WebView 동시 ensure 사라져 불필요할 수 있음. 제거 vs 보존.

## 6. 리스크·미지수
1. **연결 task 교체 zombie/hijack 차단(T4)** — task 모델이 openGen 가드를 "구조적 해결"한다는 건 가정. tokio abort 타이밍·oneshot drop 순서에서 새 race 가능. Blocker-1 이식 통과가 안전 게이트.
2. **OutputRouter 갱신↔ViewManager 락 순서(ADR-0006)** — arc-swap 핫패스는 OK, 갱신 트리거를 락 안에 넣으면 위반+데드락.
3. **재연결 resubscribe N창 구독 union** — 에이전트당 구독 1회(중복제거)면 "구독 ref-count" 필요(마지막 창 닫힐 때만 Unsubscribe). TS에 없던 신규 동시성 표면.
- **고대역 relay 오버헤드 실측:** 데몬→src-tauri WS(1회)→Router→창(IPC) 한 홉 추가. D3 emit_to면 JSON/base64 팽창 부활 위험. `cdp.mjs eval`로 throughput 측정(ADR-0036 "로컬 IPC 미미"는 가정, QA full 검증).
- **미지수(코드 미확인):** Tauri 2 `ipc::Channel` 멀티윈도우 per-window 생성/정리 안전성(context7 확인) · src-tauri tokio multi-thread 재도입과 Tauri 런타임 상호작용.

## 7. 리서치 반영 (2026-06-28) — T5~T8 확정 디테일

> cross-family deep 리서치(보고서: `docs/research/tauri-channel-multiwindow-carrier-research-2026-06-28.md`)가 D3 carrier와 fan-out 설계의 미지수(§6)를 닫음. **현 버전 기준: `tauri = 2.11.2` / `@tauri-apps/api ^2.11.0`.**

### D3 carrier — Channel 확정 + ★raw byte 함정
- carrier 이원화 확정: **고대역 출력 = `Channel` (per-window) / 레이아웃 control = `emit`**(저빈도 JSON OK, 모듈② 현행 유지). 공식이 "child process output"을 Channel 용례로 직접 명시.
- **★ 터미널 바이트는 `Channel<tauri::ipc::Response>`(`Response::new(bytes)`) 또는 `InvokeResponseBody::Raw`로 보낸다.** `Channel<Vec<u8>>`/`Channel<&[u8]>`는 blanket `impl<T:Serialize> IpcResponse`가 **JSON 배열로 직렬화**(공식 4096B 예제의 `Channel<&[u8]>`조차 JSON으로 샘). protocol crate `encode_terminal_frame` 바이트를 Response/Raw에 실어 보내면 정합.
- Channel은 호출 webview에 태생 바인딩 → 창마다 Channel = per-window 라우팅 자연 해결(emit_to 불필요).

### T5 OutputRouter — arc-swap snapshot (open item 해소)
- `ArcSwap<RoutingSnapshot { by_agent: HashMap<AgentId, Arc<[WindowId]>> }>` — **핫패스(프레임마다) `load()` 락0**, 레이아웃 변경 시에만 `store(Arc::new(..))`. arc-swap 공식 "routing table read with every packet" 용례.
- **Pitfall(테스트로 단언):** `load()` Guard를 `.await` 너머로 보유 금지(슬롯 고갈) → async 경유 시 `load_full()`. (ADR-0006 핫패스 락0 = 이미 §2 FIX-F6.)
- 기각 대안: `RwLock`(읽기 경합), `broadcast`(느린 수신자 `Lagged` **유실** → 무손실 터미널에 위험), `left-right`/`evmap`(eventual consistency 복잡), `watch`(snapshot 배포엔 OK·핫패스 lookup 부적합).

### 구독 union ref-count (§6 리스크3 "신규 동시성 표면" 해소)
- 핫패스와 **분리**: `Mutex<HashMap<AgentId, usize>>` (레이아웃/창 수명 이벤트에만 변경). **0→1 전이 = `Subscribe` / 1→0 = `Unsubscribe`.**
- `SubscriptionGuard { agent_id, owner }` + `Drop`으로 슬롯/창 수명에 묶음. **★ async Drop 금지** — `Drop`은 await 불가 → unsubscribe를 **DaemonClient cmd_tx로 enqueue**(actor 모델과 정합, 직접 await X).

### 정리·생명주기 규약 (T6/T7)
- **dead window:** `Channel::send`는 `Result<()>` — 소멸 webview면 `Err`(에러 타입 미문서화). **`Err` 감지 시 라우팅 registry에서 해당 채널 제거**(절대 `unwrap()` 금지).
- **창 close:** `onCloseRequested`에서 명시 `unsubscribe` + `unlisten` — **#15583(webview 소멸 시 백엔드 리스너 미정리)이 2.11.2에서 미해결**.
- **대용량 큐:** `ChannelDataIpcQueue` 잔류 가능(창이 fetch 전 닫힘) → close/unsubscribe 시 정리 확인.
- **프론트 채널 정리:** `delete channel.onmessage`(null 아님 — #13133, 기존 micro-rule 유지).

### ⚠️ 버전 (의존성 — 결정 보고 대상)
- **#13133(콜백 누수, v2.5.0 fix)·#12065(순서 역전, 2.2.0 fix) = 우리 2.11.2에서 이미 해소.**
- **tauri 2.11.3에서 channel-data 데드락 수정 — 우리는 2.11.2(한 패치 아래).** T5~T7 Channel 배선 전 **2.11.3+ 업글 검토**(의존성 변경 = 사용자 보고).

### 미검증(실측 영역 — QA full)
- raw Channel 실제 throughput·한 홉 오버헤드는 문서/소스 추정 → `cdp.mjs eval` throughput 실측으로 ADR-0036 "로컬 IPC 미미" 가정 확정(수용기준 §3 trd / spike §6).

## 8. T5 OutputRouter — TRD 상세 (2026-06-29)

> 통합 표면 매핑(Explore) + carrier 리서치(§7) 반영. **순수 내부 결정은 확정(아래), 굵은 갈림길 2개(F-A/F-B)는 사용자 결정 대기.**

### 확정된 seam·타입 (코드 실측)
- **입력 seam:** `src-tauri/src/daemon_client/connection.rs:668` `Message::Binary(bytes)` 자리(현 `// TODO(T5)`). 흐름: `protocol::decode_frame(bytes) → DecodedFrame{agent_id, epoch, seq, payload:&[u8]}` → `protocol_state::decide_output(&mut sub, epoch, seq)` → **`Deliver`면 `router.route(agent_id, payload, seq)`**, `Drop*`면 무시. (`main_loop`에 `router: &OutputRouter` Arc 주입 — 재연결 넘어 app-level 공유.)
- **라우팅 소스:** `layout/manager.rs` `ViewManager{views, active_view_id, window_bindings:HashMap<label,view_id>, version}` + `LayoutNode::Slot{agent_id:Option<String>}`. `agent_id → [window_label]` = 각 View 트리에서 해당 agent 슬롯 탐색 → (View==active_view_id면 "main") + (window_bindings에서 그 view_id에 바인딩된 label들).
- **출력 carrier:** **per-window `Channel<tauri::ipc::Response>` 1개**가 그 창의 모든 agent 출력을 운반(프레임에 agent_id 태그). 창은 T6 invoke(`subscribe_output(channel)`)로 등록. (§7 raw byte: `Response`/`Raw`로 적재.)
- **★타입 브리지(코더 주의):** 프레임 `agent_id`=`AgentId`(protocol, UUID newtype) ↔ 슬롯 `agent_id`=`String`. 라우팅 키를 한쪽으로 정규화(권장: `AgentId` → 슬롯 저장 시점에 동일 문자열). 기존 spawn 경로의 String↔AgentId 변환 재사용.

### 내부 결정 (확정 — 보고용, 사용자 결정 아님)
- **D1 WindowId = Tauri window label(String)** — `window_bindings` 키와 동일, 별도 numeric 레지스트리 불필요. `RoutingSnapshot{ by_agent: HashMap<AgentId, Arc<[String]>> }`.
- **D2 rebuild-always** — 레이아웃 변경은 저빈도라 매 변경 시 snapshot 전체 재계산 후 `ArcSwap::store`. version-cache 분기 불채택(복잡도 대비 무이득).
- **D3 rebuild 호출 = ViewManager 락 *보유 중*(layout mutation과 같은 critical section)에서 `router.rebuild(&mgr)`** → 반환 delta는 **unlock 후** T6가 cmd_tx로 송신. **(★rev 2026-06-29 — `/review code deep` 3인 수렴: 옛 "emit_after_unlock(락 밖) 호출"은 `load(prev)→delta→store`가 비원자라 동시 rebuild 시 델타 drift[중복 Subscribe·누락 Unsubscribe]+ABA[낡은 store가 새 store 덮음]. 기존 ViewManager 락으로 RMW 직렬화 + `&mgr` 현재성 보장.)** ADR-0006 위반 아님 — 본문=순수 계산 + lock-free `ArcSwap::store`, 락 안 외부 I/O 0(emit/DaemonClient/network 0), 송신만 unlock 후. 별도 rebuild mutex·콜백/역참조 불채택(기존 락 재사용으로 충분).
- **D4 carrier = per-window Channel(태그)** — §7 리서치 확정(per-(window,agent) 채널 폭증 기각 / emit 브로드캐스트 JSON·정확성 기각).
- **D6 정리** — `Channel::send` `Err`→해당 window sink 제거 + rebuild(절대 unwrap 금지) · 창 close `onCloseRequested`→명시 unsubscribe + unlisten(#15583 2.11.3 미해결).

### 굵은 갈림길 — 결정 완료 (2026-06-29, 사용자)
- **F-A 슬라이싱 = ① T5 단독 먼저.** OutputRouter 코어(snapshot 재계산 + route)만, headless unit(ViewManager mutation→table→load·핫패스 락0 단언)으로 격리 검증 → 이후 T6(invoke+channel 등록)·T7·T8 각각 코더→`/review code deep`→`/qa`.
- **F-B 구독 union 소스 = ① layout 파생.** 별도 카운터 없음 — snapshot rebuild마다 현재 agent union 집합을 직전과 diff해 0→1 `Subscribe`/1→0 `Unsubscribe`. **라우팅 snapshot 재계산과 같은 트리 순회에 piggyback(단일 패스: 라우팅표 + 구독 diff 동시 산출).** 근거: SSOT=ViewManager(ADR-0035), 출력 소비자=View뿐 + 데몬이 출력 보관(Unsubscribe 후 재구독=`Resume(after_seq)` tail 리플레이라 유실0)이라 ②(명시 ref-count)는 현재 무이점. **async Drop 직접 await 금지(설사 정리 경로 쓰더라도 cmd_tx enqueue).**
  - **확장 메모(YAGNI — 지금 안 함):** 비-화면 출력 소비자(예: §5 백엔드 LLM이 다른 agent 출력을 프로그램 소비 = 목표 ⑤ 메시징 트랙, a1·연기)가 생기면 "①의 레이아웃 파생 집합 ∪ 별도 구독자 집합"으로 확장. ②를 통째로 까지 않음.
  - **개념 분리(혼동 방지):** 출력 구독(T5, 렌더 대상) ≠ 포커스(`focused_slot_id`, 키 입력 대상) ≠ agent↔agent 메시징(목표 ⑤, data-plane). T5는 출력 구독만.

## 9. T6 — 배선 TRD (2026-06-29, 통합 표면 매핑 반영)

> T5(라우팅 코어, headless) 위에 **실 IPC 배선**. 7+ 파일 통합 — 자체 `/review code deep` 라운드 필요. forks 대부분 내부 결정(보고용), 아래 확정. **동시성-치명 인접**(connection task ↔ Tauri Channel ↔ layout lock).

### T6 시퀀스 (코더 단위)
1. **`ConnectionCommand` variant 채움**(`connection.rs:~131`, 현 `__Placeholder`): `SendCommand{cmd: AgentCommand, reply: oneshot}` + `Subscribe{agent_id, epoch, after_seq, reply}` + `Unsubscribe{agent_id, reply}`. (Fork B = B1)
2. **`DaemonClient::send_command`**(`mod.rs:~357`, 현 TODO): `cmd_tx.send` 후 oneshot await 래퍼 노출. **끊김 시 pending oneshot drain**(T4 reconnect 정리와 정합 — 확인 필요).
3. **AppState 등록**(`lib.rs` setup): `Arc<DaemonClient>` · `Arc<OutputRouter>` · **window Channel registry** `Arc<Mutex<HashMap<WindowLabel, Channel<Response>>>>`. (Fork A = A1)
4. **`commands/agent.rs`(신설)**: invoke spawn/kill/write/resize → `ConnectionCommand::SendCommand` 빌드+reply await. + **`subscribe_output(label, channel)`** invoke = 창 mount 시 registry insert. (Fork D/E = D1/E1)
5. **`commands/layout.rs` 전 mutation 커맨드 수정**: critical section **락 안**에서 `router.rebuild(&mgr)` 호출(FIX-1/D3), **unlock 후** delta→`send_command(Subscribe/Unsubscribe)`. (Fork C = C1 확정)
6. **`connection.rs` main_loop 배선**: (a) cmd_rx arm(`:686`) — variant를 wire 인코딩해 `sink.send` + reply 처리. (b) Binary arm(`:668`) — `decode_frame → decide_output → Deliver면 router.targets(agent_id) → registry 의 각 창 Channel 로 fan-out`(`Response::new(bytes)` raw, §7).

### 확정 내부 결정 (forks — 보고용)
- **A=A1** window Channel registry = `Arc<Mutex<HashMap<WindowLabel, Channel<Response>>>>` in AppState. connection task가 Arc clone 보유, route 시 lock→send. 창 close→deregister + send `Err`→registry remove(dead window). (per-(win,agent) 폭증·emit JSON 기각)
- **B=B1** ConnectionCommand 3 variant + oneshot reply. (Box<dyn Any> 타입소거 기각)
- **C=C1** rebuild 락 안 / delta 송신 unlock 후 (FIX-1·D3 확정).
- **D/E=D1/E1** 창당 Channel 1개 mount 시 `subscribe_output` 등록, close 시 deregister + #15583 unlisten.

### ★T6 미해결 — 코더 진입 전 해소 필요
- **(G1) Subscribe epoch/after_seq 출처:** delta는 "agent X 구독해라"만 안다. epoch/after_seq는 **protocol_state `SubState`**(연결 task 소유, T3)에서 와야 함 — 신규=epoch None(전체 replay)/재구독=tail-only(`resubscribe_params` 재사용). delta→Subscribe 변환을 **연결 task 안에서** SubState 조회로 채울지, layout 커맨드가 채울지 결정 필요(전자가 정합 — SubState는 task 소유).
- **(G2) `Channel::send` from tokio task:** connection task(tokio 런타임)가 Tauri `Channel::send`를 호출해도 안전한지 실측(Channel은 Send+Sync, 내부 webview.eval 마샬링 — 가능성 높음이나 미검증). `cdp.mjs`로 첫 출력 도달 실측 = T6 GUI 게이트.
- **(G3) registry ↔ connection task 수명:** registry는 AppState(Tauri) 소유, connection task는 DaemonClient 소유 — task에 registry Arc를 어떻게 주입하나(start_connection 인자 추가). layout label("main"/"slot-popup") ↔ window_bindings label 일치 확인.

## 10. T6b 완료 + 잔여(T7로 이연) (2026-06-29)

**완료:** 출력 평면 배선 — Binary arm fan-out(`decode_frame`→`decide_output` 가드→`router.targets`→창 `Channel<Response>`) · SubscribeAck epoch 갱신(`apply_subscribe_ack`) · Subscribe/Unsubscribe/Fire(fire-and-forget) · **`router.current_agents()` 기반 resubscribe**(C1+C2) · **`mark_delivered` 분리**(fan_out 1+창 성공 후에만 high-water 전진, C4) · layout 6 mutation rebuild+delta(락 안 rebuild/락 밖 송신) · `subscribe_output` invoke · window Channel registry(`output_channel.rs`). `cargo test --workspace` **148 green**.

**검증:** `/review code deep` 2라운드(opus doc-aware ×2 + Codex blind). 1R C1~C4 적출 → 수정 → 2R 재검증 = **C1~C4 닫힘**(데이터 손상/오염 0). ADR-0006(fan_out 락 보유 중 await 0·rebuild 락안/송신 락밖)·ADR-0037(가드 라우팅 전 1회) 정상.

**★잔여 = T7 영역** (현재 **dead path** — 프론트가 아직 `wsTransport` 직결, 이 경로 미사용이라 화면 안 깨짐):
- **N1** 멀티창/늦은 mount: 같은 agent를 여러 창에 동시 표시 시 per-agent 단일 seq로 창별 진도 못 챙김 + 미배달 frame 재배달 트리거 부재.
  - **근본(리서치 2026-06-29, `research/study-notes/`):** 우리는 출력을 **seq 큐 소비 모델**(per-agent high-water + dedup)로 다루나, 터미널 업계 표준은 **state-render 모델**(tmux/Zellij/VS Code — 서버 중앙 보관 + attach 시 redraw, 출력에 단일 consumed cursor 없음). → **seq(데몬↔게이트웨이 전송 무손실)와 렌더(창 채우기) 분리**가 N1 해소 방향.
  - **새 창 채우기 갈림(T7 PRD/TRD에서 결정):** (A) 데몬 재요청(업계 표준·단순·2홉 loopback) vs (B) src-tauri 게이트웨이 캐시(2계층 적합·비표준·동기화 비용). 리서치 결론: 중앙 보관+재요청이 표준, 게이트웨이 캐시는 web 렌더러 편의일 뿐 authoritative store 아님.
- **N5** subs 운영 중 정리(retain은 재연결 시에만 — 연결 유지 중 누수 잔존). **N6** resubscribe 중 동시 layout mutation → 중복 Subscribe(데몬 Subscribe 멱등성 확인 필요). **N4** fan_out Binary 배선 통합 테스트 부재(seam 도입 시 headless 가능 / 또는 T7 GUI 실측).

**T7 = `TauriTransport`+`clientFactory` 교체** 시 위 전송/렌더 분리·A/B를 PRD/TRD로 결정 + GUI 실측(G2 = 실제 창 출력 도달, `cdp.mjs`).

## 참조 코드
`src/api/wsTransport.ts`(이전 원본) · `protocolClient.ts`(이전 원본) · `crates/engram-dashboard-daemon/src/ws.rs`(서버 actor 대칭) · `src-tauri/src/layout/manager.rs`(라우팅 소스) · `src/api/wsTransport.test.ts`·`protocolClient.test.ts`(Rust 이식 명세서) · `crates/engram-dashboard-protocol`(`encode_terminal_frame` → Response/Raw 적재).
