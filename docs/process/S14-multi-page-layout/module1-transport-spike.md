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

## 참조 코드
`src/api/wsTransport.ts`(이전 원본) · `protocolClient.ts`(이전 원본) · `crates/engram-dashboard-daemon/src/ws.rs`(서버 actor 대칭) · `src-tauri/src/layout/manager.rs`(라우팅 소스) · `src/api/wsTransport.test.ts`·`protocolClient.test.ts`(Rust 이식 명세서) · `crates/engram-dashboard-protocol`(`encode_terminal_frame` → Response/Raw 적재).
