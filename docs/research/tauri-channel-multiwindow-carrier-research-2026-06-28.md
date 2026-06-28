# 리서치: Tauri 2 멀티윈도우 출력 carrier (Channel vs emit) + fan-out 라우팅 패턴

**상태:** 완료 (cross-family deep — Claude Sonnet 3 + Codex 2 BLIND, opus 교차+적대 검증)
**작성:** 2026-06-28 (dashboard2/master, opus aggregator)
**계기:** 모듈① T5(OutputRouter) 진입 전, D3 carrier(=Tauri Channel) 미검증("멀티윈도우 안전성 context7+QA full 보류") 해소 + fan-out 라우팅/구독 패턴 OSS 근거 확보.
**근거 ADR:** ADR-0036(전송 중계 통일) · ADR-0037(전송 의미론=Rust 단독 가드) · ADR-0006(락 순서)
**우리 버전(검증 기준):** `tauri = 2.11.2`(Cargo.lock) · `@tauri-apps/api = ^2.11.0`(package.json)

> 확신도 범례: **확실**(1차/공식/소스 직접) · **가능성 높음**(2차 수렴 or 소스 추정) · **불확실**(단일 출처/지식 기반).
> 방법: 두 model family(Claude/Codex)가 BLIND 독립 조사 → opus가 클레임 단위 교차 + 핵심/모순 클레임 적대 검증. "만장일치 ≠ 정답" 경계 유지.

---

## 0. 한 줄 결론

D3 carrier = **Tauri `Channel`이 맞다(공식이 "child process output"을 직접 Channel 용례로 명시)** — 단 **raw byte 경로(`Channel<tauri::ipc::Response>`/`InvokeResponseBody::Raw`)로 써야** JSON 팽창을 피한다(`Channel<&[u8]>`는 JSON으로 샘). fan-out = **`ArcSwap<RoutingSnapshot>`(핫패스 락0) + 별도 `Mutex<HashMap<AgentId,usize>>` 구독 ref-count(0→1 subscribe / 1→0 unsubscribe)** — 두 family가 거의 동일 설계로 수렴. 이는 현 TRD/spike 결정을 **확인 + 디테일 보강**이지 번복이 아니다.

---

## 1. Channel carrier — 생명주기·throughput (갈래 A)

### 확인된 사실
- **Channel은 호출 webview에 태생적 바인딩** — JS가 `new Channel(onmessage)`를 만들어 `invoke` 인자로 넘기면 Rust는 그 webview에 묶인 `Channel<T>` 핸들을 받음(`JavaScriptChannelId::channel_on(webview)`). **창마다 별도 Channel = per-window 라우팅이 emit_to 없이 자연 해결.** (확실 — Claude A·B·Codex 3중 수렴, 소스 `crates/tauri/src/ipc/channel.rs`)
- **전송 크기별 3경로:** JSON <8192B = inline `eval` / raw <1024B = `new Uint8Array(...).buffer`(ArrayBuffer) inline eval / 초과 = `ChannelDataIpcQueue` 저장 후 `plugin:__TAURI_CHANNEL__|fetch` 큐잉. (확실 — Codex 소스 + Claude A 수렴. 코드 주석: WebView2 v135에서 8192B JSON eval이 fetch보다 ~2× 빠름)
- **순서 보장 메커니즘 존재 + 우리 버전에서 정상:** Rust가 증가 index 부여, JS가 `#nextMessageIndex`로 out-of-order를 배열 큐잉 후 순차 drain. (확실 — `packages/api/src/core.ts`)
- **공식 권장:** Channel = "fast, ordered data" / "download progress, **child process output**, websocket messages 등 streaming에 내부 사용". 이벤트 시스템은 "**not designed for low latency or high throughput**"라 명시. (확실 — `v2.tauri.app/develop/calling-rust` Channels, `/calling-frontend` Event System)

### ★ raw byte 함정 (적대 검증으로 확정 — 구현 직결)
- `InvokeResponseBody`는 `Json(String)` + `Raw(Vec<u8>)` 둘 다 있어 raw 경로는 **실재**. **그러나** `impl<T: Serialize> IpcResponse`(blanket)가 JSON으로 직렬화하므로, **공식 4096-byte 예제의 `Channel<&[u8]>`조차 `&[u8]: Serialize`라 JSON 배열(`[104,101,...]`)로 나간다** — base64보다도 비대. (확실 — Codex 적대검증, `crates/tauri/src/ipc/mod.rs` blanket impl + docs "Serialize 반환=JSON, optimized array buffer는 `tauri::ipc::Response` 필요")
- **결론:** 터미널 바이트는 `Channel<tauri::ipc::Response>`(`Response::new(bytes)`) 또는 `InvokeResponseBody::Raw`로 보내야 raw 경로를 탄다. **단순 `Channel<Vec<u8>>`/`Channel<&[u8]>`는 안 됨.**
- 처음부터 우리는 protocol crate `encode_terminal_frame`로 바이트 프레임을 만든다 → 그걸 Response/Raw로 실어 보내면 자연스럽다.

### 생명주기 버그 (우리 버전 2.11.2 기준)
| Issue | 증상 | 우리 영향 |
|---|---|---|
| #13133 | Channel `onmessage`가 `window`에 영구 잔류 → 메모리 누수 | **해소**(v2.5.0 fix, PR #13136). 단 프론트 정리는 `delete channel.onmessage`(null 아님) — 기존 CLAUDE.md micro-rule과 일치 (확실) |
| #12065 | `pendingMessageIds`를 **문자열 정렬** → 순서 역전, `onmessage` 미발화 | **해소**(@tauri-apps/api 2.2.0 + tauri 2.2.0 fix, PR #12069) (가능성 높음→확실, CHANGELOG 확인) |
| #15583 | **webview 소멸 시 백엔드 `js_event_listeners` 미정리** → stale 누적 | **미해결(2.11.2 포함)** — 창 닫을 때 `onCloseRequested`에서 명시 `unlisten` 필수 (확실) |
| 2.11.3 deadlock | `send_channel_data_handler`/`send_channel_data` 데드락 | **우리는 2.11.2 = 한 패치 아래.** Channel 본격 사용 전 **2.11.3+ 업글 검토** (가능성 높음 — release notes, Codex) |
| 대용량 큐 잔류 | `ChannelDataIpcQueue` insert 후 창이 fetch 전에 닫히면 pending payload 잔류 가능 | 운영 정리 필요(close/unsubscribe 시 큐 정리는 자동 아님) (가능성 높음 — Codex 소스 추정) |

- **dead window send:** `Channel::send`는 `Result<()>` 반환, 내부 최종 `webview.eval(...)?` → 소멸 webview면 `Err`. **정확한 에러 타입은 공식 미문서화 → `send` `Err`로 감지해 registry에서 제거하는 게 현실적.** 절대 `unwrap()` 금지. (가능성 높음 — 두 family 수렴, 소스 추정)

---

## 2. Channel vs emit/emit_to (갈래 B)

- **이벤트 payload = 항상 JSON 문자열** → 바이트엔 부적합(JSON 배열 팽창). `emit`=전 창 브로드캐스트, `emit_to(label)`=타깃, `emit_filter`=클로저 선택. (확실 — 3중 수렴 + 공식)
- **`emit` 후 JS 필터는 성능뿐 아니라 정확성/보안도 깨짐** — "보는 창에만" 요구에 안 맞음(모든 창이 payload 수신). (확실 — Codex 강조)
- 이벤트 라우팅 버그들(낮은 우리 영향 — 우리는 carrier로 Channel 채택, emit은 레이아웃 control용만):
  - #8916 unlisten 오동작(early v2, beta.3, PR #8930 closed) — *핸드오프의 "2.6.0 미만" 표기는 부정확*(beta.3 이슈). 우리 2.11.2 무관. (확실)
  - #10182 `emit_to`가 label 무시하고 전체 발송(beta.23, "not planned" 종료) — stable 2.x 영향 불확실, 우리는 emit_to를 출력에 안 씀 (가능성 높음)
  - #11561 AnyLabel 매칭 실패(2.0.6, PR #11581) — 수신 타깃 타입 명시로 회피 (확실)
- **함의:** 레이아웃 control(`layout:updated`/`view:list-updated`)은 emit으로 충분(저빈도·JSON OK, 현 모듈② 이미 그러함). **고대역 출력만 Channel.** carrier 이원화가 맞다.

---

## 3. fan-out 라우팅 + 구독 ref-count (갈래 C) — 두 family 거의 동일 수렴

### 라우팅 테이블 = `arc-swap`
- **arc-swap은 "read often, update rarely" 라우팅 테이블의 공식 명시 용례** — `RwLock<Arc<T>>` 대비 락+refcount 경합 제거. 경합 시 std RwLock 대비 ~2.5× 읽기 유리. (확실 — docs.rs + vorner blog, 두 family 수렴)
- **권장 shape(두 family 거의 동일 제시):**
  ```rust
  struct RoutingSnapshot { by_agent: HashMap<AgentId, Arc<[WindowId]>> }
  // 핫패스(프레임마다): 락0
  let routes = routing.load();
  if let Some(windows) = routes.by_agent.get(&frame.agent_id) {
      for w in windows.iter() { send_to_window(*w, &frame); }
  }
  // 레이아웃 변경(드물게): routing.store(Arc::new(new_snapshot));
  ```
- **Pitfall(확실, 두 family 수렴):** ① `load()` Guard를 `.await` 너머로 들고 가지 말 것(슬롯 고갈) → async 경유 시 `load_full()`. ② 엔트리 단위 잦은 갱신이면 전체 클론 비용 → 우리는 레이아웃 변경 시만 갱신이라 무관. ③ Sized만 — trait object는 박싱.
- 대안 기각: `left-right`/`evmap`(eventual consistency·`publish()` 복잡), `watch`(snapshot 배포엔 OK·핫패스 lookup엔 부적합), `broadcast`(느린 수신자 `Lagged`로 **메시지 유실** → 무손실 터미널 출력에 위험). (확실 — 두 family 수렴)

### 구독 union ref-count
- **핫패스와 분리**: ref-count map은 레이아웃/창 수명 이벤트에만 변하므로 `Mutex<HashMap<AgentId,usize>>`로 충분. **0→1 전이 = backend subscribe / 1→0 = unsubscribe.** (가능성 높음 — 두 family 수렴)
- `SubscriptionGuard { agent_id, owner: Arc<...> }` + `Drop`으로 창/슬롯 수명에 묶음.
- **★ async Drop 주의(확실):** `Drop`은 `.await` 불가 → unsubscribe가 async면 **Drop에서 manager task로 unsubscribe 명령을 enqueue**(직접 await 금지). 우리 DaemonClient actor 모델(cmd_tx)과 자연 정합.

### OSS prior art (패턴 차용 — 복붙 아님)
| 시스템 | 언어/라이선스 | 메커니즘 | 우리 시사 |
|---|---|---|---|
| **wezterm** | Rust/MIT | `Mux::notify`: `RwLock<HashMap<id, Box<dyn Fn(Notif)->bool>>>`에 PaneOutput 브로드캐스트, 구독자가 false 반환 시 `retain`으로 lazy 제거. 클라측 필터 | 구독자 적으면 write-lock notify+retain 단순. 단 우리는 고빈도라 arc-swap 라우팅이 더 맞음 |
| **zellij** | Rust/MIT | Screen 중앙 상태머신: PtyBytes→owning pane→**클라별 render push**. `Arc<Mutex<HashMap<ClientId,ClientSender>>>` | "한 번 파싱, 클라별 뷰 push" — 우리는 파싱을 webview xterm에 위임하니 raw frame 라우팅이 더 단순 |
| **tmux** | C/ISC | 서버가 PTY 소유, 클라 `TAILQ`, **보는 pane만** diff push, `server_client_lost`로 detach 정리 | "안 보는 창엔 안 보냄" = 우리 OutputRouter 목표와 동일. 수용기준 5 |
| **ra-multiplex/lspmux** | Rust/MIT | `HashMap<RequestId,(ClientId,OriginalId)>`로 id 재작성→응답 역라우팅. notification은 라우팅 불가분 drop | per-agent 라우팅 테이블의 전형. 우리 request_id pending map과 동형 |

**공통 교훈:** 성숙 멀티플렉서는 출력을 N번 재전송하지 않고 **소스에서 한 번 처리 → 보는 클라에만** 보낸다. 우리 OutputRouter(arc-swap snapshot으로 "보는 창에만 fan-out")는 이 관행과 정합 — 단 파싱은 각 webview xterm이 하므로 우리는 raw frame을 나르기만 하면 됨.

---

## 4. 교차검증표 (Claude ↔ Codex)

| 클레임 | Claude | Codex | 판정 |
|---|---|---|---|
| Channel이 호출 webview에 바인딩 → per-window 자연 | ✓ | ✓ | **수렴·확실** |
| 이벤트=JSON, 고대역 부적합 / Channel=streaming 권장 | ✓ | ✓ | **수렴·확실(공식)** |
| Channel raw byte 지원 | A: **미지원(오류)** / B: 지원 | 지원(단 blanket=JSON) | **모순→해소:** 지원. A가 #13405(이벤트 FR)와 혼동. raw는 `Response`/`Raw` 명시 필요 |
| Channel 순서 보장(우리 버전) | B: #12065 패치버전 불확실 | 2.2.0 fix(적대검증) | **해소·확실:** 우리 2.11 fix됨 |
| #15583 webview 소멸 리스너 미정리 미해결 | ✓ | ✓ | **수렴·확실(우리 영향)** |
| arc-swap = 라우팅 테이블 적합 | ✓ | ✓ | **수렴·확실** |
| 구독 ref-count(0→1/1→0)+async Drop 주의 | ✓ | ✓ | **수렴·가능성 높음** |
| broadcast는 Lagged 유실로 출력엔 위험 | ✓ | ✓ | **수렴·확실** |

---

## 5. 공백 / 한계
- **실측 미수행:** Channel raw 경로의 실제 PTY MB/s throughput·한 홉 추가 오버헤드는 문서/소스 기반 추정. ADR-0036 "로컬 IPC 미미" 가정은 여전히 `cdp.mjs eval` throughput 실측(QA full)으로 닫아야 함.
- **dead-window send 에러 타입** 공식 미문서화 — `send` `Err` 감지로 우회(타입 단언 의존 금지).
- **2.11.3 deadlock** 우리 미적용 — Channel 본격 사용 전 업글 검토 필요(미결).
- **tauri-conduit 등 3rd-party binary IPC 벤치(~11×)**는 환경·버전 의존, 방법론 미검증(불확실) — 1차 후보 아님, 실측 후에만 고려.
- wezterm 정확한 client fanout 자료구조는 Codex pass에서 미회수(Claude C는 Mux notify 확인) — 우리 설계 영향 없음.

## 6. 산출 → 설계 반영(요지)
1. **carrier 이원화 확정:** 출력=`Channel<Response>`(raw) per-window / 레이아웃 control=emit. (D3 확인 + raw 디테일 보강)
2. **OutputRouter:** `ArcSwap<RoutingSnapshot{by_agent}>` 핫패스 락0, 레이아웃 변경 시 store. (spike open item "snapshot 갱신 메커니즘" 해소)
3. **구독 ref-count:** `Mutex<HashMap<AgentId,usize>>` 0→1/1→0, `SubscriptionGuard` Drop→manager enqueue(async Drop 회피). (spike §6 리스크3 "신규 동시성 표면" 해소)
4. **정리 규약:** send `Err`→채널 제거 · 창 close `onCloseRequested`→unsubscribe+unlisten(#15583) · 대용량 큐 잔류 정리.
5. **버전:** Channel 배선 전 tauri **2.11.3+** 업글 검토(데드락 수정).
