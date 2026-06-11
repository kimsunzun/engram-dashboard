# ChatGPT 응답

검토 결론부터 말하면 방향은 맞지만, 그대로 구현 시작하면 dev/HMR, 팝업 resize, Channel cleanup에서 결함이 날 가능성이 큽니다. 특히 문서의 Channel 선택 자체는 맞지만, cleanup 방식과 “팝업 resize 후 subscribe”의 보장 수준은 보강해야 합니다. 문서가 현재 subscribeOutput, TerminalSlot cleanup, initEventBus, 팝업의 resize → subscribe 순서를 핵심 패턴으로 삼고 있기 때문에 이 부분을 집중적으로 봤습니다. 

frontend-integration-lld

1. Channel<PtyEvent>를 invoke 파라미터로 전달하는 패턴

판정: 맞습니다. Tauri v2 문서는 Rust command 인자로 tauri::ipc::Channel<T>를 받고, 프론트엔드에서 new Channel<T>()를 만든 뒤 onmessage를 할당하고 invoke 인자로 넘기는 예제를 공식 패턴으로 제시합니다. 즉 현재 문서의 const channel = new Channel<PtyEvent>(); channel.onmessage = onChunk; invoke(..., { agentId, channel }) 구조는 Tauri v2 계열에서 올바른 방향입니다. 
Tauri
+1

다만 인자 이름 변환은 확인해야 합니다. Tauri 문서 예제는 Rust 쪽 on_event: Channel<...>를 프론트엔드에서 onEvent로 전달합니다. 따라서 Rust command가 subscribe_agent_output(agent_id, channel)라면 프론트엔드는 { agentId, channel }이 맞지만, Rust 인자가 on_event라면 { onEvent }처럼 camelCase로 넘겨야 합니다. 현재 문서의 agentId, sinkId camelCase 방침은 맞는 쪽입니다. 
Tauri

수정 권고: Channel 생성은 아래처럼 생성자 콜백을 써도 됩니다. API 문서상 new Channel<T>(onmessage?) 형태도 지원됩니다. 
Tauri

TypeScript
const channel = new Channel<PtyEvent>((event) => {
  onChunk(event)
})

하지만 핵심은 생성 방식보다 해제 방식입니다. 문서의 channel.onmessage = null as unknown as ...는 타입을 속이는 방식이고, Tauri Channel memory leak 이슈의 workaround는 delete onProgress.onmessage 쪽으로 제시되어 있습니다. 해당 이슈 설명에 따르면 onmessage 콜백이 window 쪽 callback table에 남아 closure를 붙잡을 수 있었고, 임시 workaround로 delete onProgress.onmessage가 제시되었습니다. 
GitHub

따라서 cleanup은 다음처럼 쓰는 편이 더 안전합니다.

TypeScript
function clearChannel(channel: Channel<PtyEvent> | null) {
  if (!channel) return
  delete (channel as any).onmessage
}
2. atob(data_b64) → Uint8Array 변환 안전성

판정: 기능적으로는 안전하지만, 고빈도 PTY 스트림에는 최선이 아닙니다. atob()은 base64를 “raw bytes가 들어 있는 binary string”으로 디코딩합니다. 그래서 charCodeAt(i) & 0xff로 Uint8Array를 만드는 방식은 바이트 손상 없이 동작합니다. 다만 binary string을 한 번 만들고 다시 byte array를 만드는 구조라 메모리 할당이 2단계로 생기며, PTY 출력처럼 짧은 chunk가 매우 자주 오는 경우 WebView2 메모리 압박을 키울 수 있습니다. MDN도 binary data라면 atob()보다 Uint8Array.fromBase64()가 byte array를 바로 만들기 때문에 더 다루기 쉽다고 설명합니다. 
MDN
+1

다만 Uint8Array.fromBase64()는 “Baseline 2025” API라서, 사용자의 Windows WebView2 Runtime 버전에 따라 항상 있다고 가정하면 위험합니다. MDN 호환성 표 기준으로 Edge 140 이상에서 지원되는 흐름이므로, 배포 대상의 WebView2 Runtime이 그보다 낮을 수 있으면 feature detection + fallback이 필요합니다. 
MDN

권장 구현은 이 정도입니다.

TypeScript
export function decodeBase64Bytes(dataB64: string): Uint8Array {
  const fromBase64 = (Uint8Array as any).fromBase64
  if (typeof fromBase64 === 'function') {
    return fromBase64(dataB64)
  }

  const binary = atob(dataB64)
  const bytes = new Uint8Array(binary.length)

  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i)
  }

  return bytes
}

그리고 xterm.write(bytes) 자체는 맞습니다. xterm.js API는 write(data: string | Uint8Array)를 지원하고, Uint8Array는 PTY raw bytes로 취급되며 UTF-8로 해석됩니다. 
Xterm.js

추가 반론: 현재 구조에서 base64 자체가 꼭 최선은 아닙니다. Tauri Channel payload가 serde object라면 Vec<u8>를 그대로 보내면 JSON number array가 되어 base64보다 비대해질 수 있으므로 base64 선택은 합리적입니다. 하지만 출력량이 큰 경우에는 Rust 쪽 chunk size 제한, 프론트엔드 write queue, drop/backpressure 정책이 필요합니다. xterm.js의 write는 비동기 처리이며 buffer 반영이 즉시 되지 않고 callback으로 처리 완료를 알 수 있습니다. 
Xterm.js

3. StrictMode double-effect에서 cancelled flag 패턴

판정: 방향은 맞지만 현재 코드에는 cleanup race와 leak 구멍이 있습니다. React 공식 문서도 StrictMode에서 effect가 개발 모드에 한해 setup → cleanup → setup을 한 번 더 실행한다고 설명하고, cleanup이 setup을 대칭적으로 되돌려야 한다고 합니다. 또한 async 응답 순서 race를 막기 위해 ignore flag를 cleanup에서 true로 바꾸는 패턴도 공식 문서에 나옵니다. 
React
+1

문제는 현재 문서 코드에서 cleanup이 먼저 실행되고, 그 뒤 subscribeOutput().then(...)이 늦게 도착하는 경우입니다. 이때 cancelled branch에서 unsubscribeOutput만 호출하고 result.channel.onmessage를 지우지 않습니다. 문서가 “Channel memory leak 방지”를 요구사항으로 적어둔 것과 충돌합니다. 

frontend-integration-lld

현재 코드의 취약 지점:

TypeScript
if (cancelled) {
  ptyApi.unsubscribeOutput(agentId, result.sinkId)
  return
}

여기에 channel cleanup이 들어가야 합니다.

TypeScript
useEffect(() => {
  if (!agentId || !terminal) return

  let sinkId: SinkId | null = null
  let channel: Channel<PtyEvent> | null = null
  let cancelled = false

  ptyApi.subscribeOutput(agentId, (event) => {
    if (cancelled) return

    const bytes = decodeBase64Bytes(event.data_b64)
    terminal.write(bytes)
  }).then((result) => {
    if (cancelled) {
      clearChannel(result.channel)
      void ptyApi.unsubscribeOutput(agentId, result.sinkId).catch(console.error)
      return
    }

    sinkId = result.sinkId
    channel = result.channel
  }).catch((err) => {
    if (!cancelled) console.error('subscribeOutput failed', err)
  })

  return () => {
    cancelled = true

    clearChannel(channel)

    if (sinkId) {
      void ptyApi.unsubscribeOutput(agentId, sinkId).catch(console.error)
    }
  }
}, [agentId, terminal])

추가로 terminal이 effect 내부에서 쓰이는데 dependency가 [agentId]뿐이면 stale terminal closure가 생길 수 있습니다. terminal instance가 절대 바뀌지 않는 설계라면 ref로 고정하고, 바뀔 수 있다면 dependency에 넣어야 합니다. React 문서는 effect 안에서 쓰는 reactive value는 dependency에 포함해야 한다고 설명합니다. 
React

4. Tauri Event listener를 App level에서 1회 등록하는 패턴과 HMR

판정: production 구조로는 맞지만, HMR/dev에서는 현재 설계만으로 중복 등록 위험이 있습니다. Tauri 문서는 listen이 반환하는 unlisten을 보관해 실행 컨텍스트가 사라질 때 반드시 호출하라고 합니다. 페이지 reload나 URL navigation 때는 자동 해제되지만, SPA router에는 적용되지 않는다고 명시합니다. Vite HMR은 “항상 전체 page reload”가 아니므로 SPA router와 비슷하게 module/effect만 갈리는 상황에서 중복 listener가 남을 수 있습니다. 
Tauri

현재 문서의 initEventBus()는 await listen(...)만 하고 unlisten handle을 저장하지 않습니다. 이건 HMR뿐 아니라 테스트, Storybook, root 재생성, StrictMode에서 App.tsx effect 안 호출 시 중복을 만들 수 있습니다. 

frontend-integration-lld

권장 패턴은 “singleton + unlisten 저장 + HMR dispose”입니다.

TypeScript
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

type EventBusState = {
  promise: Promise<UnlistenFn[]> | null
}

const KEY = '__engram_event_bus__'

function getState(): EventBusState {
  const g = globalThis as any
  if (!g[KEY]) {
    g[KEY] = { promise: null } satisfies EventBusState
  }
  return g[KEY]
}

export function initEventBus() {
  const state = getState()

  if (state.promise) {
    return state.promise
  }

  state.promise = Promise.all([
    listen<{ id: string; status: AgentStatus }>('agent-status-changed', (e) => {
      useAgentStore.getState().onStatusChanged(e.payload.id, e.payload.status)
    }),

    listen<AgentInfo[]>('agent-list-updated', (e) => {
      useAgentStore.setState({ agents: e.payload })
    }),
  ])

  return state.promise
}

export async function disposeEventBus() {
  const state = getState()
  const unlisteners = await state.promise
  unlisteners?.forEach((unlisten) => unlisten())
  state.promise = null
}

if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    void disposeEventBus()
  })
}

또 하나 중요한 Tauri v2 pitfall이 있습니다. v2에서 JavaScript event.listen()은 target을 지정하지 않으면 사실상 모든 event listener를 듣는 쪽에 가깝고, WebviewWindow.listen은 해당 target으로 emit된 이벤트만 받는 구조입니다. 멀티 창 구조라면 listen을 무심코 쓰면 main/popup이 모두 같은 이벤트를 받을 수 있습니다. 상태 이벤트가 main 전용이면 Rust에서 emit_to("main", ...)를 쓰거나, 프론트에서 getCurrentWebviewWindow().listen(...)로 target을 좁히는 쪽을 검토해야 합니다. 
Tauri

5. 팝업 창에서 resize → subscribe 순서 보장 여부

프론트엔드 JS 수준에서는 await resizePty(); await subscribeOutput();이면 subscribe invoke는 resize invoke 응답 이후에 시작됩니다. 즉 네트워크/IPC 요청을 “프론트에서 발행하는 순서”는 보장할 수 있습니다.

하지만 Rust 쪽 실제 PTY resize 적용 완료가 subscribe보다 먼저라는 보장은 Rust command 구현에 달려 있습니다. resize_pty command가 session mutex를 잡고 PTY 크기를 실제로 변경한 뒤 return한다면 안전합니다. 반대로 resize_pty가 내부 task/channel에 resize 요청만 넣고 바로 return한다면, frontend에서 await resizePty()를 했어도 subscribe의 replay가 resize 적용보다 먼저 시작될 수 있습니다. Tauri Channel 자체는 ordered streaming에 적합하지만, 서로 다른 두 command인 resize_pty와 subscribe_agent_output 사이의 내부 side effect 완료까지 보장해 주는 것은 아닙니다. Tauri 문서는 Channel이 streaming/ordered data에 적합하다고 설명하지만, 이건 하나의 channel 메시지 흐름에 대한 이야기입니다. 
Tauri

더 큰 구조적 결함은 같은 PTY를 main과 popup이 동시에 resize할 수 있다는 점입니다. 문서는 main과 popup이 같은 agent에 독립적으로 subscribe하는 구조를 제안합니다. 

frontend-integration-lld

 그런데 PTY size는 subscriber별 속성이 아니라 PTY session의 전역 속성입니다. main이 120x40, popup이 200x60이면 마지막 resizePty 호출자가 이깁니다. 두 창이 모두 ResizeObserver를 돌리면 서로 계속 PTY 크기를 덮어쓸 수 있습니다.

권장 정책 중 하나를 반드시 정해야 합니다.

활성 뷰 하나만 resize owner: popup이 열리면 main은 해당 agent에 대해 resize 송신 중지.

focused window만 resize owner: focus/visibility 기반으로 owner를 넘김.

popup은 mirror 전용: popup은 subscribe만 하고 resize하지 않음.

backend command 통합: subscribe_agent_output(agent_id, cols, rows, channel)로 resize와 sink 등록/replay를 같은 critical section에서 처리.

프론트엔드만으로 가장 현실적인 보강은 이 정도입니다.

TypeScript
await waitNextAnimationFrame()
fitAddon.fit()

const cols = terminal.cols
const rows = terminal.rows

if (cols <= 0 || rows <= 0) return

await ptyApi.resizePty(agentId, cols, rows)
const sub = await ptyApi.subscribeOutput(agentId, onChunk)

다만 이것도 Rust가 resize를 동기 적용하고 return한다는 전제가 있어야 합니다. 그 전제가 확인되지 않으면 resize_and_subscribe 또는 subscribeOutput(agentId, cols, rows, channel)로 합치는 것이 정답입니다.

6. 놓친 것과 Tauri v2.4 pitfall

가장 큰 누락은 Tauri v2 capabilities/permissions입니다. 팝업이 별도 WebviewWindow라면 그 window label도 capability에 포함되어야 command invoke가 됩니다. Tauri v2 capability는 window/webview의 IPC 접근을 제어하며, matching capability가 없으면 IPC layer 접근 자체가 없다고 문서화되어 있습니다. 
Tauri

두 번째는 stdin의 binary 경로입니다. 문서의 terminal.onData → TextEncoder().encode(data)는 일반 UTF-8 입력에는 괜찮습니다. 하지만 xterm.js에는 onBinary가 따로 있고, 문서는 non-UTF-8 binary message를 backend로 보낼 때 이 이벤트를 사용하라고 설명합니다. 마우스 reporting 같은 일부 escape/input은 onData만으로 충분하지 않을 수 있습니다. 
Xterm.js

TypeScript
useEffect(() => {
  if (!agentId || !terminal) return

  const dataDisp = terminal.onData((data) => {
    void ptyApi.writeStdin(agentId, new TextEncoder().encode(data))
  })

  const binaryDisp = terminal.onBinary((data) => {
    const bytes = new Uint8Array(data.length)
    for (let i = 0; i < data.length; i++) {
      bytes[i] = data.charCodeAt(i) & 0xff
    }
    void ptyApi.writeStdin(agentId, bytes)
  })

  return () => {
    dataDisp.dispose()
    binaryDisp.dispose()
  }
}, [agentId, terminal])

세 번째는 고빈도 output에 Tauri Event를 쓰지 않는 것입니다. 이 문서는 PTY output에 Channel을 쓰고 status/list update에 Event를 쓰기 때문에 방향은 좋습니다. Tauri 문서도 Event system은 큰 데이터 전송에 적합하지 않고, Channel은 ordered data와 streaming operation에 적합하다고 설명합니다. 
Tauri
 여기에 더해 Tauri issue에는 backend→frontend event를 계속 emit할 때 memory usage가 크게 증가했다는 open issue도 있으므로, PTY output을 Event로 바꾸면 안 됩니다. 
GitHub

네 번째는 seq 처리 정책입니다. PtyEvent에 seq가 있는데 현재 프론트 설계는 seq를 거의 쓰지 않습니다. 

frontend-integration-lld

 StrictMode, reconnect, replay, popup attach가 있는 구조라면 최소한 subscriber 단위로 lastSeqRef를 두고 중복 replay를 drop해야 합니다.

TypeScript
const lastSeqRef = useRef<number>(-1)

function onChunk(event: PtyEvent) {
  if (event.seq <= lastSeqRef.current) return
  lastSeqRef.current = event.seq

  terminal.write(decodeBase64Bytes(event.data_b64))
}

다섯 번째는 xterm open/fit timing입니다. xterm 문서는 open(parent) 호출 시 parent element가 visible dimensions를 가져야 DOM 측정이 가능하다고 설명합니다. 따라서 popup 첫 render 직후 fitAddon.fit()을 바로 호출하면 0 cols/rows 또는 stale size가 나올 수 있습니다. useLayoutEffect, requestAnimationFrame, offsetWidth/offsetHeight > 0 guard를 같이 써야 합니다. 
Xterm.js

구현 시작 여부

조건부로 시작해도 됩니다. 단, 아래 항목은 구현 시작 전에 설계에 반영하는 것이 좋습니다.

위험	심각도	조치
Channel cleanup 누락	높음	cancelled branch와 normal cleanup 양쪽에서 delete channel.onmessage
EventBus HMR 중복	높음	singleton + unlisten 저장 + import.meta.hot.dispose
popup/main resize 충돌	높음	resize owner 정책 결정
resize → subscribe Rust 적용 순서	중간~높음	Rust resize_pty가 동기 적용 후 return하는지 확인. 아니면 command 통합
seq 미사용	중간	replay/live 중복 방지용 lastSeq 적용
stdin onBinary 누락	중간	terminal.onBinary 처리 추가
WebView2 Uint8Array.fromBase64 호환성	낮음~중간	feature detection + atob fallback
Tauri v2 capabilities	중간	main/popup window label 모두 command permission 포함

최종 판단은 “LLD의 골격은 승인 가능, 현재 코드 스니펫은 그대로 승인 불가”입니다. 위 보강 중 특히 Channel cleanup, EventBus HMR, resize owner 세 가지를 먼저 고치면 프론트엔드 통합 구현을 시작해도 됩니다.