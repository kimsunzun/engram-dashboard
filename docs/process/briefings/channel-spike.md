# Channel Spike — Tauri 2.11 Channel 실측 (담당: dco23, Opus)

발신: ed12 (매니저)
목적: tauri 버전 핀 결정을 위한 실측. 현재 `tauri = "2.4"`가 caret이라 2.11.2로 resolve됨.
LLD는 "2.5+ Channel silent failure" 우려로 2.4 고정을 명시했으나, 그 이슈가 **Windows WebView2**에 실제 존재하는지 직접 검증한다.
**결과 양호 시 최신 2.x 유지 확정, 문제 시 `=2.4` 핀.** (사용자 결정: 3번 실측 → 1번 유지)

## 검증 대상 2가지

### 검증 1 (필수) — 연속 send 무결성
이슈 #11421("새 Channel 인스턴스가 1회만 전송 가능")이 Windows에도 있는가?
우리 drain은 매 chunk마다 send하므로 이게 깨지면 치명적.

### 검증 2 (가능하면) — webview 소멸 후 send 반환값
이슈 #10901: webview가 안 듣고 있을 때 send가 Err를 반환하는가, 아니면 조용히 Ok인가?
우리 drain의 dead subscriber 감지(send 실패 시 제거)에 영향. (단 우리는 명시적 unsubscribe도 있어 치명적이진 않음)

## 구현 (임시 검증 코드 — spike처럼 나중 정리)

### 백엔드: 임시 test command
```rust
use tauri::ipc::Channel;

#[tauri::command]
fn channel_spike(on_event: Channel<String>) -> Result<(), String> {
    // 1000회 연속 send — 전부 도착하는지 (검증1)
    for i in 0..1000 {
        on_event.send(format!("msg-{i}"))
            .map_err(|e| format!("send {i} failed: {e}"))?;
    }
    Ok(())
}
```
lib.rs invoke_handler에 임시 등록.

### 프론트: 수신 카운트
```ts
import { Channel, invoke } from '@tauri-apps/api/core'

const ch = new Channel<string>()
let count = 0
let lastSeq = -1
let gap = false
ch.onmessage = (msg) => {
  const n = parseInt(msg.split('-')[1])
  if (n !== lastSeq + 1) gap = true   // 순서/누락 감지
  lastSeq = n; count++
}
await invoke('channel_spike', { onEvent: ch })
// 잠시 후 count, gap 출력 — count===1000 && !gap 이면 검증1 PASS
console.log({ count, gap, lastSeq })
```
App.tsx 등에 버튼 하나 달아 트리거하거나, 마운트 시 1회 실행. 화면이나 콘솔에 결과 표시.

### 검증 2 (선택)
두 번째 WebviewWindow를 만들어 그 창에서 channel 등록 → 창 close → 백엔드에서 그 channel로 send → 반환값(Ok/Err) 관찰. 구현 부담되면 검증1만 하고 검증2는 "미실시"로 보고.

## 실행 & 보고

```
npm run tauri dev   # 앱 띄우고 트리거 → 콘솔/화면에서 count, gap 확인
```

보고:
- 검증1 PASS(count=1000, gap=false): `orch 12 "⟁dco23 Channel spike — 검증1 PASS(1000/1000 무손실), 검증2 <결과>. 최신 2.x 유지 가능"`
- 검증1 FAIL: `orch 12 "⟁dco23 Channel spike — 검증1 FAIL(N/1000 수신, gap=true). =2.4 핀 필요"` + 로그

임시 코드(channel_spike command, 프론트 트리거)는 검증 후 ed12가 정리 지시. 막히면 30분 내 중간보고.
주의: 이건 Tauri 앱을 실제로 띄우는 첫 통합 테스트다 — 빌드/실행 환경 문제(WebView2 등)도 같이 관찰해서 보고.
