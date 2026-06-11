# Gemini 응답

> 모델: Gemini 3.5 Thinking | 검토 대상: `frontend-integration-lld.md` | 일시: 2026-06-11

제시해주신 프론트엔드 통합 LLD(Low-Level Design) 문서를 **Adversarial(반론 및 결함 탐색) 관점**에서 날카롭게 검토했습니다.

결론부터 말씀드리면, **전반적인 패턴은 실무 관점에서 훌륭히 설계되었으나, 성능(Performance)과 타이밍(Race Condition) 측면에서 시스템을 죽이거나 UI를 얼려버릴 수 있는 몇 가지 치명적인 지뢰**가 숨어 있습니다.

요청하신 6가지 질문과 추가 위험 사항에 대해 구체적인 분석과 해결책을 제시합니다.

---

## 1. Tauri v2.4 Channel 패턴 검토

**질문:** `Channel<PtyEvent>`를 `invoke` 파라미터로 전달하고 `new Channel<T>()` 생성 후 `onmessage`를 할당하는 패턴이 올바른가?

### ⚠️ 결함 분석

Tauri v2에서 `new Channel()`을 생성하고 `invoke`로 넘기는 흐름 자체는 API 스펙상 **올바릅니다.** 하지만 제공된 LLD 코드에는 두 가지 결함이 있습니다.

**타입 안전성 결함:** `new Channel<PtyEvent>()`로 생성했다면, Tauri 내부 메커니즘에 의해 이벤트 페이로드가 내부적으로 풀립니다. 백엔드에서 `channel.send(PtyEvent)`를 하면 프론트엔드 `onmessage` 콜백에는 `PtyEvent` 객체가 **그대로** 들어옵니다. 즉, 래퍼 코드의 파라미터 타입은 맞지만 백엔드와 매핑이 정확해야 합니다.

**`onmessage = null`로 메모리 누수가 완전히 해결되지 않음:** GitHub #13133 이슈를 의식해 `channel.onmessage = null` 처리를 한 것은 좋으나, 이것은 JS 런타임 내의 클로저 참조만 끊을 뿐입니다. Tauri 내부의 IPC 리스너(Window 이벤트 레벨)는 백엔드에서 해당 채널의 `Sender`가 명시적으로 Drop되기 전까지 해제되지 않고 메모리에 남아있을 수 있습니다.

### 🛠️ 개선안

프론트엔드에서 수동으로 무력화하기보다, 백엔드(`unsubscribe_agent_output`)에서 해당 `sinkId`에 대응하는 `Sender`를 명시적으로 파괴(Drop)하여 채널을 닫아버리는 흐름을 확실히 보장해야 합니다.

---

## 2. Base64 디코딩 성능 및 WebView2 안정성

**질문:** `atob(data_b64) → Uint8Array` 변환이 안전한가? 더 나은 방법은?

### 💥 치명적인 결함 (Performance Bottleneck)

이 설계에서 **가장 먼저 수정해야 할 거대한 지뢰**입니다.

`atob`와 `Uint8Array.from(..., c => c.charCodeAt(0))`의 조합은 대량의 PTY 스트림(예: 빌드 로그 출력, 대용량 파일 `cat`)이 발생할 때 **CPU 점유율을 폭발시키고 UI 스레드를 얼려버리는 주범**이 됩니다.

- Base64는 데이터 크기를 33% 증가시킵니다.
- Rust에서 문자열 인코딩 → IPC 전송 → JS에서 `atob` 디코딩 → JS 루프 돌며 바이트 배열 변환이라는 **4중 오버헤드**가 발생합니다.

### 🛠️ 개선안

2026년 현재 Tauri v2.x 환경에서는 **Raw Binary(바이트 배열)를 IPC와 Channel을 통해 직접 전송할 수 있습니다.** Base64 문자열을 쓸 이유가 전혀 없습니다.

백엔드 타입을 `Vec<u8>`로 변경하고, 프론트엔드 타입을 `Uint8Array`로 직송하세요.

만약 굳이 문자열을 써야 한다면, 브라우저 표준인 `TextDecoder`를 쓰거나 최신 웹 표준인 `Uint8Array.fromBase64()`(최신 WebView2/Chromium 지원)를 사용해야지, 절대 `atob` 루프를 돌리면 안 됩니다.

```typescript
// 추천: 백엔드에서 바로 바이트 배열로 쏘고, 프론트엔드는 그대로 xterm에 주입
export interface PtyEvent {
  agent_id: string
  seq:      number
  data:     Uint8Array // data_b64가 아님!
}

// TerminalSlot.tsx 안에서 디코딩 없이 즉시 write
terminal.write(event.data)
```

---

## 3. React StrictMode와 cancelled 플래그 경쟁 조건

**질문:** StrictMode double-effect에서 `cancelled` 플래그 패턴에 경쟁 조건이 있는가?

### ⚠️ 결함 분석

프론트엔드 JS 사이드에서는 `cancelled` 플래그 덕분에 UI 상태가 꼬이지는 않습니다. 하지만 **백엔드(Rust) 사이드에서 심각한 자원 낭비와 비동기 순서 뒤틀림(Race Condition)이 발생할 수 있습니다.**

```
[Effect 1 실행] ➡️ 백엔드 subscribe 호출 (Rust 동작 시작)
[Effect 1 클린업] ➡️ cancelled = true
[Effect 2 실행] ➡️ 백엔드 subscribe 호출 (Rust 동작 시작)
... 약간의 시간차 후 ...
[Effect 1 Promise 완료] ➡️ cancelled 확인 ➡️ 백엔드 unsubscribe 호출
[Effect 2 Promise 완료] ➡️ 정상 등록
```

Rust 백엔드 입장에서는 아주 짧은 밀리초(ms) 사이에 동일 Agent에 대해 **구독 → 구독 → 해제** 요청이 들어옵니다.

만약 Rust의 비동기 태스크 스케줄링으로 인해 **Effect 1의 해제(unsubscribe) 요청이 Effect 2의 구독(subscribe) 요청보다 아주 미세하게 늦게 처리된다면?** 정작 유지되어야 할 Effect 2의 구독이 해제되어 버리는 대참사가 날 수 있습니다.

또한 PTY 스냅샷(Replay) 데이터가 두 번 요청되므로 백엔드 자원이 낭비됩니다.

### 🛠️ 개선안

가장 확실한 방법은 프론트엔드에서 동일 `agentId`에 대한 구독 요청이 단시간 내에 중복 발생하지 않도록 **상태 기반 락(Lock)을 걸거나, AbortController 패턴을 지원하도록 백엔드와 통신 구조를 맞추는 것**입니다. 최소한 백엔드에서 `unsubscribe`를 처리할 때 `sinkId`를 명확히 대조하므로 대참사는 면할 수 있겠으나, 백엔드 구조가 완전히 **멱등성(Idempotency)**을 보장하는지 확인해야 합니다.

---

## 4. HMR/Dev Reload 시 Event Listener 중복 위험

**질문:** App level에서 1회 등록하는 패턴이 HMR 발생 시 중복 등록될 위험이 있는가? 해결책은?

### 🎯 정확한 지적입니다

Vite HMR 환경에서 `initEventBus()`가 포함된 파일이 재평가(Re-evaluate)되거나 전역 컨텍스트가 유지된 채 모듈이 새로 로드되면, **Tauri의 전역 이벤트 리스너가 중복 등록됩니다.** 결과적으로 이벤트를 한 번 받았을 때 Zustand 스토어 로직이 2번, 3번 중복 실행되는 현상이 발생합니다.

### 🛠️ 해결책

Tauri의 `listen` 함수는 이벤트를 해제할 수 있는 `unlisten` 함수(Promise)를 반환합니다. 이를 이용해 **멱등성을 보장하는 전역 초기화 패턴**을 적용해야 합니다.

```typescript
// src/lib/eventBus.ts
import { listen, UnlistenFn } from '@tauri-apps/api/event'

let unlisteners: UnlistenFn[] = []

export async function initEventBus() {
  // HMR 시 기존 리스너가 있다면 모두 해제
  if (unlisteners.length > 0) {
    for (const unlisten of unlisteners) unlisten()
    unlisteners = []
  }

  const u1 = await listen<{ id: string; status: AgentStatus }>('agent-status-changed', (e) => {
    useAgentStore.getState().onStatusChanged(e.payload.id, e.payload.status)
  })
  
  const u2 = await listen<AgentInfo[]>('agent-list-updated', (e) => {
    useAgentStore.setState({ agents: e.payload })
  })

  unlisteners.push(u1, u2)
}
```

---

## 5. 팝업 창의 resize → subscribe 실행 순서 보장

**질문:** Rust 쪽에서 보면 resize 전에 subscribe가 먼저 도착할 수 있는가? (async 순서 보장 여부)

### ⚙️ 비동기 보장 여부

프론트엔드 코드에서 `ptyApi.resizePty(...).then(() => ptyApi.subscribeOutput(...))` 처럼 체이닝을 걸었기 때문에, **프론트엔드가 첫 번째 IPC 요청의 응답을 받은 후에 두 번째 요청을 보내므로 순서는 100% 보장**됩니다. Rust가 요청을 거꾸로 처리할 일은 없습니다.

### 🚨 진짜 문제는 따로 있습니다: DOM 렌더링 타이밍 (Layout Race)

설계서의 코드를 보면 `PopupPage.tsx`가 마운트되자마자 `fitAddon.fit()`을 호출합니다.

```typescript
useEffect(() => {
  fitAddon.fit() // 💥 위험!
  ptyApi.resizePty(...)
}, [])
```

새 팝업 창(WebviewWindow)이 열리는 시점에는 **DOM 요소가 완전히 배치되거나 화면에 렌더링이 완료되지 않았을 확률이 매우 높습니다.** 크기가 0이거나 불완전한 상태에서 `fit()`을 호출하면 터미널의 `cols`와 `rows`가 비정상적인 값(예: 0 또는 아주 작은 값)으로 계산되어 백엔드로 전송됩니다.

### 🛠️ 개선안

터미널 컨테이너가 확실히 DOM에 붙고 크기가 잡힌 후(`ResizeObserver`의 첫 번째 호출 또는 `requestAnimationFrame`)에 `fit()`과 `resizePty`가 일어나도록 유도해야 합니다.

---

## 6. 전체 구조에서 놓친 것 및 Tauri v2.4 Pitfall

### ① 팝업 창의 에이전트 상태(Status) 동기화 누락

LLD 설계 사상에서 "팝업 창은 Zustand를 공유하지 못하므로 독립적으로 PTY 스트림만 구독한다"고 했습니다. 하지만 만약 **팝업 창이 열려 있는 동안 백엔드의 Agent가 종료되거나(Exited) 실패하면(Failed), 팝업 창은 그 사실을 어떻게 알 수 있을까요?**

스트림이 그냥 끊기는 것만으로는 원인을 알 수 없습니다. 팝업 창 내부의 `App.tsx` 또는 `PopupPage.tsx`에서도 **`agent-status-changed` 전역 이벤트를 리슨**하고 있어야 UI에 "프로세스 종료" 레이오버를 띄울 수 있습니다.

### ② 백프레셔(Backpressure) 통제 불능

백엔드 PTY가 제어할 수 없을 정도로 빠른 속도로 데이터를 쏟아낼 때, Tauri IPC 채널과 React 스레드는 이 과부하를 버티지 못합니다. xterm.js 자체는 최적화되어 있지만, JS 스트림 수신 레이어에서 큐가 밀릴 수 있습니다. 대용량 로그 수신 시 프론트엔드가 먹통이 되는 것을 방지하기 위해 프론트엔드나 백엔드 단에서 **청크 배치(Chunk Batching)나 디바운싱 메커니즘**이 고려되어야 합니다.

---

## ⚖️ 최종 판정: 지금 이 설계로 구현을 시작해도 되는가?

**결론: 조건부 승인 (Conditional GO)**

아래 명시된 "치명적 위험 사항" 2가지를 전면 수정하는 조건 하에 구현에 착수하셔도 좋습니다. 구조적 뼈대(API 래퍼 분리, Clean-up 패턴 정의 등)는 매우 탄탄합니다.

### 🚨 즉시 수정해야 할 미결정 위험 사항 (Critical Risks)

| 위험 요소 | 영향도 | 해결 방안 |
|---|---|---|
| 1. Base64 직렬화 오버헤드 | **상(High)** — PTY 대량 출력 시 UI 프리징 유발 | 백엔드와 `Channel` 통신 시 문자열 변환을 제거하고 `Uint8Array` 및 `Vec<u8>` Raw Binary 직송 구조로 즉시 변경할 것. |
| 2. 팝업 마운트 직후 `fit()` 호출 | **중(Medium)** — 초기 터미널 깨짐 및 크기 오작동 | 컨테이너의 크기가 실제로 확보된 것을 `ResizeObserver`로 감지한 후 첫 `resizePty`를 날리도록 타이밍 수정할 것. |
| 3. HMR 이벤트 리스너 누수 | **중(Medium)** — 개발 환경에서 오작동 및 메모리 누수 | `initEventBus` 실행 시 기존 `unlisten` 함수를 먼저 실행하는 방어 코드 적용할 것. |
| 4. 팝업 창 Status 감지 부재 | **하(Low)** — 프로세스 종료 시 UI 대응 불가 | 팝업 창 내부에서도 `agent-status-changed` 이벤트를 구독하여 화면에 반영할 것. |
