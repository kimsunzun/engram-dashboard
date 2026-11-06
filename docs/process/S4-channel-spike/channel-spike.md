# Channel Spike — measuring the Tauri 2.11 Channel (owner: dco23, Opus)

From: ed12 (manager)
Purpose: a measurement for deciding the tauri version pin. The current `tauri = "2.4"` is a caret, so it resolves to 2.11.2.
The LLD states a 2.4 pin because of the concern "2.5+ Channel silent failure", but we verify directly whether that issue actually exists on **Windows WebView2**.
**If the result is good, confirm keeping the latest 2.x; if there is a problem, pin `=2.4`.** (User decision: option 3, measure → option 1, keep)

## Two things to verify

### Verification 1 (required) — integrity of consecutive sends
Does issue #11421 ("a new Channel instance can send only once") also exist on Windows?
Our drain sends on every chunk, so it is fatal if this is broken.

### Verification 2 (if possible) — send return value after the webview is gone
Issue #10901: when the webview is not listening, does send return Err, or silently Ok?
It affects our drain's dead-subscriber detection (removal when send fails). (But we also have an explicit unsubscribe, so it is not fatal)

## Implementation (temporary verification code — clean up later like the spike)

### Backend: temporary test command
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
Register it temporarily in the lib.rs invoke_handler.

### Frontend: receive count
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
Attach a button to App.tsx or similar to trigger it, or run it once on mount. Show the result on screen or in the console.

### Verification 2 (optional)
Create a second WebviewWindow, register the channel in that window → close the window → send to that channel from the backend → observe the return value (Ok/Err). If the implementation is a burden, do only verification 1 and report verification 2 as "not done".

## Run and report

```
npm run tauri dev   # 앱 띄우고 트리거 → 콘솔/화면에서 count, gap 확인
```

Report:
- Verification 1 PASS (count=1000, gap=false): `orch 12 "⟁dco23 Channel spike — 검증1 PASS(1000/1000 무손실), 검증2 <결과>. 최신 2.x 유지 가능"`
- Verification 1 FAIL: `orch 12 "⟁dco23 Channel spike — 검증1 FAIL(N/1000 수신, gap=true). =2.4 핀 필요"` + log

The temporary code (the channel_spike command, the frontend trigger) is cleaned up after verification on ed12's instruction. If stuck, send an interim report within 30 minutes.
Note: this is the first integration test that actually launches the Tauri app — also observe and report build/run environment problems (WebView2 etc.).
