# 프론트엔드 통합 LLD 검토 — fable

**검토자:** fable (pane 8), 2026-06-11
**대상:** `frontend-integration-lld.md` (교차 기준: `backend-lld-stage1.md` 개정판)
**방법:** adversarial + 백엔드 확정본과의 계약(contract) 일치 검증. 확신 수준: [확실] / [가능성 높음] / [불확실]

---

## 종합 판정: 조건부 GO

구조(ptyApi 래퍼 층, 명시적 unsubscribe, cancelled flag, 앱 레벨 이벤트 버스)는 백엔드 M2 반영을 올바르게 받았고 패턴 선택도 타당하다. 그러나 **Critical 2건은 첫 실행에서 즉시 드러나는 확정 버그**이고, 백엔드와의 타입 계약 불일치가 2건 더 있다. 이들을 반영한 뒤 구현 시작을 권한다.

---

## Critical — 구현 착수 전 수정 필수

### C1. AgentStatus의 serde 표현 불일치 — 상태 표시가 영원히 안 맞는다 [확실]

프론트 §1은 internally-tagged 형태를 기대한다:

```ts
{ type: 'Exited', code: 0 }
```

그러나 백엔드 §3 `AgentStatus`는 `#[derive(Serialize)]` 기본값 = **externally tagged**다. 실제 wire 산출물:

```json
"Running"                      // unit variant → 그냥 문자열
{ "Exited": { "code": 0 } }    // struct variant → 중첩 객체
```

`status.type`은 항상 `undefined` — AgentTree 색상, 상태 분기 전부 불발. **수정:** 백엔드 enum에 `#[serde(tag = "type")]`을 붙이거나(internally tagged — unit/struct variant 모두 `{"type": "Running"}` 형태로 나와 프론트 타입과 일치), 프론트 타입을 external tagging으로 맞추거나. **권장: 백엔드에 `#[serde(tag = "type")]`** — 프론트 discriminated union이 관용적이고 switch 친화적. 어느 쪽이든 한 줄 수정이지만 **두 문서 중 하나는 반드시 고쳐야 하며**, 이는 백엔드 "확정본"의 재수정을 의미한다 — 변경 통제 절차(아래 부록 참조)에 태워라.

추가 불일치: 프론트 union에 `Starting`이 없다. 백엔드 §9는 Starting 제거를 선언했지만 **§3 enum 코드에는 `Starting`이 잔존**한다(개정 누락). 백엔드 §3에서 variant를 실제로 지워 양쪽을 §9 기준으로 통일하라.

### C2. 재구독 시 terminal reset 부재 — 출력이 중복으로 쌓인다 [확실]

subscribe는 항상 replay(seq 0..N)부터 재생한다. 그런데 §4 effect는 구독 전에 `terminal.reset()`/`clear()`를 호출하지 않는다. 따라서:

- 슬롯의 agentId 교체: 이전 agent 출력 위에 새 agent replay가 이어 붙는다.
- 같은 agent 재구독(HMR, 컴포넌트 remount): 기존 화면 내용 + replay 전체가 한 번 더 — **화면 2배 중복**.

StrictMode 이중 실행은 cancelled flag가 write를 막아주지만, **remount/agent 전환은 정상 경로라서 flag로 안 막힌다.** **수정:** subscribe 직전(effect 본문 첫머리)에 `terminal.reset()` 후 구독. §4 구현 패턴과 §9 규칙표에 명시.

---

## Major — 착수 전 결정/수정 필요

### M1. 멀티 창 resize 충돌 정책이 없다 [확실 — 정책 부재가]

PTY는 agent당 1개, cols/rows도 1쌍이다. §6 팝업 패턴은 "팝업이 열리면 팝업 크기로 resize"인데, 메인 창이 같은 agent를 더 큰 슬롯에서 보고 있으면 **메인 쪽 xterm.js cols와 PTY cols가 어긋나 줄바꿈이 전부 깨진다.** 이건 터미널 공유의 고전 문제로, 정책 없이는 회피 불가. 선택지:

- (a) **최소 크기 룰(tmux 방식):** 구독 중인 모든 창의 min(cols), min(rows)로 PTY 설정. 견고하지만 backend에 구독자별 크기 추적 추가 필요.
- (b) **마지막 포커스 창 우선:** 포커스된 창만 resize 권한. 단순, 비포커스 창은 깨진 표시 허용.
- (c) **팝업 = 보기 전용(읽기만), 크기는 메인이 지배.**

(b)가 구현비 대비 합리적이다. 어느 쪽이든 **결정 없이 §6대로 구현하면 두 창이 서로 resize를 빼앗는 핑퐁**이 된다 (각 창의 ResizeObserver가 상대 창의 resize 결과로 다시 fit → resize → ...). 핑퐁 차단만이라도: "자기 창이 포커스일 때만 resizePty 호출" 가드를 §5에 추가하라.

### M2. `get_agent_snapshot` 반환 타입이 백엔드와 불일치 [확실]

프론트 §2는 `{ seq, data_b64 }[]`를 기대하지만 백엔드 §8은 `Vec<PtyChunk>`, `PtyChunk = { seq, data: Vec<u8> }` — wire에서는 **`data`가 숫자 배열**로 나온다. base64가 아니다. **수정:** 백엔드 command 층에서 `data_b64`로 변환해 반환하도록 명시하거나(권장 — M4 wire format 결정과 일관), 프론트 타입을 `{ seq, data: number[] }`로. 참고: subscribe가 replay를 자동 전송하므로 이 command의 용도 자체가 디버깅 외엔 없다 — **프론트에서 사용하지 않는다고 명시**하는 것도 답이다.

### M3. 창 닫힘 정리가 send-실패 감지 단일 의존 — 그런데 그 감지를 못 믿어 2.4로 고정했다 [가능성 높음]

팝업 §6의 cleanup은 effect 기준인데, **창 자체가 닫히면 effect cleanup의 unsubscribe invoke가 완주한다는 보장이 없다** (webview 파괴 타이밍). 그러면 백엔드 §12(c)의 send-실패 감지가 유일한 정리 경로다. 그런데 백엔드 G-2가 인용한 고정 사유가 바로 "**2.5 Channel silent failure**" — send가 조용히 성공 처리되는 계열의 버그다. 2.4에서 죽은 webview에 대한 send가 확실히 Err를 주는지는 **여전히 미실측** [불확실]. 보강책:

1. 팝업에서 `getCurrentWindow().onCloseRequested` 훅으로 unsubscribe를 명시 호출 (effect cleanup보다 먼저, 확실한 시점).
2. 백엔드 스파이크 검증 항목에 "창 강제 종료 후 send 반환값"을 유지 (백엔드 검토에서 요청한 항목 — 여전히 유효).
3. 보험: 백엔드에 sink별 연속 실패 카운트가 아니라, **창 라벨→sink 매핑을 commands 층에서 추적하고 `WindowEvent::Destroyed`에서 일괄 unsubscribe** (이전 백엔드 검토 M2-②, 미채택 상태 — 팝업 도입 시점인 지금이 채택 적기다).

### M4. 에러 처리 전략 부재 + 종료된 agent로의 입력 가드 없음 [확실]

- §2 ptyApi 전체와 §4 effect에 `.catch`가 하나도 없다. agent가 죽은 뒤 키 입력마다 `write_stdin` → `NotFound` → unhandled rejection이 키스트로크 단위로 쏟아진다.
- `agent-status-changed`로 Exited/Killed를 받았을 때 TerminalSlot이 무엇을 하는지(입력 차단, "process exited" 표시, 슬롯 닫기?)가 설계에 없다. 이건 프론트 LLD의 정확한 스코프인데 비어 있다.

**수정:** ① ptyApi 레벨 공통 에러 처리(최소 console.warn + NotFound는 무시 가능 분류), ② status가 terminal 상태로 바뀌면 `onData` 핸들러 가드 + 슬롯 오버레이 표시를 §4에 추가.

### M5. HMR/dev reload 시 이벤트 리스너 중복 — Q4에 대한 답 [확실]

위험 실재한다. 두 경로를 구분하라:

- **전체 페이지 reload:** JS 컨텍스트가 리셋되므로 중복 없음. 안전.
- **Vite HMR 모듈 교체:** `eventBus.ts` 모듈이 재평가되고 `initEventBus()`가 다시 불리면 이전 콜백이 살아 있는 채 **리스너가 누적**된다 — status 변경 1회에 store 갱신 N회.

**해결:** ① `listen`이 반환하는 `UnlistenFn`을 모듈 변수에 보관, ② `initEventBus`를 idempotent로 (이미 등록돼 있으면 skip), ③ `import.meta.hot?.dispose(() => unlisten())` 등록. 세 개 다 몇 줄이다.

---

## Minor — 2단계 중 처리 가능

1. **base64 디코딩 핫패스 성능 (Q2 관련):** `Uint8Array.from(atob(s), c => c.charCodeAt(0))`은 정확하지만 chunk마다 문자 단위 콜백이라 고속 출력에서 느리다. WebView2(Chromium)의 `Uint8Array.fromBase64()`를 feature-detect해 우선 사용, 없으면 atob 폴백. [가능성 높음 — fromBase64 가용성은 WebView2 런타임 버전 의존]
2. **debounce 미취소:** §5 cleanup이 `observer.disconnect()`만 한다. pending debounce가 unmount 후 발화해 stale agentId로 resize 호출. cleanup에서 `debounced.cancel()`.
3. **`xterm.onBinary` 미배선:** `onData`(string)만 연결돼 있다. 바이너리 paste 경로(onBinary)도 writeStdin에 연결.
4. **§2 중복 import:** `invoke`와 `Channel` 둘 다 `@tauri-apps/api/core` — 한 줄로.
5. **초기 fetch와 event의 경합:** 시작 시 `fetchAgents()` 응답이 그 사이 도착한 `agent-list-updated`를 덮어쓸 수 있다. 실용적 완화: 초기화 시 fetch 1회 후 이후엔 event만 신뢰(현 설계가 대체로 그러함 — 명시만 하라).
6. **subscribe 실패 시 channel 잔존:** invoke가 reject되면 만들어 둔 Channel 객체의 onmessage 정리를 .catch에서도 수행.
7. **`channel.onmessage = null as unknown as ...`:** 타입 우회 대신 no-op 함수 할당(`channel.onmessage = () => {}`)이 SDK 타입과 충돌 없이 같은 효과. (#13133 대응 효과는 동일)
8. **`agent-status-changed` payload 형식이 백엔드 문서에 없다:** 프론트는 `{ id, status }`를 가정하는데 백엔드 LLD 어디에도 emit payload 정의가 없다. StatusSink → commands emit 시의 JSON 형태를 백엔드 §9에 1줄 명시해 계약을 닫아라.

---

## 검토 요청 질문 6건 — 직접 답변

1. **Channel을 invoke 파라미터로 — 올바른가?** 그렇다. `new Channel<T>()` 생성 → `onmessage` 할당 → invoke 인자로 전달이 Tauri v2 공식 패턴이고, §2처럼 **invoke 전에 onmessage를 할당**하는 순서도 맞다(역순이면 초기 메시지 유실 가능). 인자명 camelCase(`agentId`) → Rust snake_case 자동 매핑도 v2 기본 동작과 일치. [확실]
2. **atob → Uint8Array 안전한가?** 정확성은 안전하다(atob는 Latin-1 바이너리 문자열을 정확히 복원, WebView2에서 표준 지원). 문제는 성능 — Minor 1 참조. terminal.write가 Uint8Array를 직접 받으므로 변환 후 바로 write하는 흐름은 옳다. [확실]
3. **cancelled flag 패턴 — 경쟁 조건?** 패턴 자체는 건전하다. 검증한 경로: cleanup이 pending invoke보다 먼저(then에서 즉시 unsubscribe ✓), 메시지가 cancel 후 도착(write 가드 ✓), StrictMode 이중 실행(각 effect가 자기 channel만 ✓). 남은 구멍 2개: ① unsubscribe invoke 실패 시 unhandled rejection(.catch 필요), ② **terminal reset 부재로 remount 시 중복 출력 — 이건 flag로 안 막힌다(C2).** [확실]
4. **HMR 중복 등록 위험?** 있다 — M5 참조. 페이지 reload는 안전, HMR 모듈 재평가가 위험. UnlistenFn 보관 + idempotent guard + `import.meta.hot.dispose`. [확실]
5. **resize → subscribe 순서 보장?** 보장된다 — invoke promise는 Rust handler 완료 후 resolve되므로 `.then` 체인이면 resize가 Rust 쪽에서 완료된 뒤에 subscribe가 도착한다. 부수 이점: resize가 유발하는 ConPTY 전체 repaint가 replay buffer에 들어가므로 후발 attach의 화면 충실도 문제(백엔드 검토 Minor 8)를 상당 부분 자연 해소한다 — 이 효과를 §6에 명시할 가치가 있다. 단 **멀티 창 크기 충돌(M1)은 별개 문제로 남는다.** [확실]
6. **놓친 것 / v2 pitfall:** C1(serde tag — 최대 함정), C2(terminal reset), M1(resize 정책), M3(창 닫힘 정리), M4(에러/종료 가드), Minor 3(onBinary)·8(event payload 미정의). Tauri 특화로는: invoke 인자 camelCase 자동 변환을 모르고 snake_case로 보내는 실수(현 설계는 올바름), `listen`의 UnlistenFn 미보관(M5), Channel onmessage를 invoke 후에 다는 실수(현 설계는 올바름).

---

## 추가 질문: "이 설계 기준으로 프론트엔드 통합 구현 시작해도 되는가?"

**C1·C2 반영 후 시작 가능.** 둘 다 수정량은 작지만(serde attribute 1줄 + reset 1줄) 안 고치면 첫 실행에서 상태 표시 전멸 + 화면 중복이라는 확정 버그다. 나머지는 병행 결정 가능.

**위험한 미결정 사항 (우선순위순):**

| # | 미결정 | 왜 위험한가 |
|---|---|---|
| 1 | AgentStatus wire 표현 (C1) — 백엔드 재수정 필요 | 양 문서가 서로 다른 계약을 "확정"하고 있음 |
| 2 | 멀티 창 resize 정책 (M1) | 정책 없이 구현하면 resize 핑퐁 — 사후 수정은 양쪽 코드 변경 |
| 3 | 창 닫힘 시 구독 정리의 실측 (M3) | 2.4 고정 사유와 정리 메커니즘이 같은 불확실성 위에 있음 — 스파이크로 못박기 전엔 [불확실] |
| 4 | terminal 상태(Exited 등) UX (M4) | 입력 가드 없으면 에러 스팸, 표시 정책은 사용자 결정 사안 |
| 5 | `agent-status-changed` payload 형식 (Minor 8) | 백엔드 문서에 부재 — 구현자 추측에 맡겨짐 |
| 6 | get_agent_snapshot 포맷/용도 (M2) | 안 쓸 거면 명시, 쓸 거면 타입 통일 |

---

## 부록: 백엔드 "확정본"의 잔재 — 프론트가 미러링할 때 혼동 유발 [확실]

프론트 검토 중 발견한 백엔드 문서 자체 결함. 프론트는 "확정본 기준 미러"가 원칙인데 기준 문서가 신구 혼재 상태다:

1. **PtyEvent 정의가 2개** — §3에 구버전(`chunk: PtyChunk`)과 신버전(`data_b64`)이 공존. 구버전 삭제 필요.
2. **`Starting` variant 잔존** — §9는 제거 선언, §3 enum엔 그대로 (C1에서 전술).
3. **§12 워크스루 3종이 전부 구버전** — (a) `app_handle.emit`(C1 위반 표현)·"남은 batch"(C2로 폐기된 개념), (b) master.take() 누락·"drain_handle.detach()"(std에 없는 API)·G-1 completion channel 미반영, (c) "Channel drop → 자동 제거"(M2로 보강 전 서사).
4. **§16 결정표 "0.9.x 이슈 확인됨" vs §2 "출처 미확인"** — 자기모순.
5. **§14 테스트 코드가 구 API** — `PtyManager::new()`(StatusSink 인자 누락), `Box::new(MpscSink)`(Arc로 변경됨), `event.chunk.data`(data_b64로 변경됨).
6. **§17 검토 질문이 구버전 그대로** — Q5가 이미 폐기된 `Mutex<PtyManager>`를 묻는 등.
7. **§10 스레드 표 "5초 타임아웃"** — G-1의 completion channel 방식 미반영.
8. (재지적) **kill_agent의 5초 대기를 async command 안에서 수행** — `spawn_blocking` 권고(백엔드 검토 M1 후반)가 여전히 미반영. tokio worker 1개가 kill마다 최대 5초 점유.

2단계 코드 검증은 "확정 스펙 대비 적합성"으로 진행되므로, 기준 문서의 신구 혼재는 검증 자체를 오염시킨다. **2단계 착수 전 백엔드 문서 정리 1회전을 강권한다.**
