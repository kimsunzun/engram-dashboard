//! window Channel registry + 무상태 통과 send 헬퍼 (ADR-0046 — 미러 버퍼 제거, view-direct replay).
//!
//! ## 역할 (미러 제거 후)
//! src-tauri 는 더는 데몬 ring 을 미러하지 않는다(ADR-0046). 이 파일은 두 가지만 남는다:
//! - **`WindowChannelRegistry`** = `window_label → 출력 Channel`(Tauri 타입 보관). 창 mount 시
//!   `subscribe_output` invoke 가 insert, dead window(send Err) 감지 시 connection task 가 remove.
//! - **`send_to_windows`** = 원본 frame(또는 replay 경계 마커) bytes 를 targets∩registered 창 Channel 로
//!   그대로 통과시키는 순수 fan-out(버퍼·cursor 없음). 죽은 Channel 은 send 실패 시 같은 lock 안에서 제거.
//!
//! ## ★raw byte 함정(spike §7)★
//! 출력 프레임은 `Channel<tauri::ipc::Response>` 로 운반한다 — `Channel<Vec<u8>>`/`Channel<&[u8]>` 는
//! blanket `impl<T:Serialize> IpcResponse` 가 JSON 배열로 직렬화해 바이트가 샌다. 반드시
//! `Response::new(bytes)` 로 실어 raw 로 보낸다.
//!
//! ## ★동시성/락 규율(ADR-0006, load-bearing)★
//! `std::sync::Mutex` 다(tokio Mutex 아님). connection task(tokio)가 핫패스에서 registry lock→send 하는데
//! 락 보유 중 `.await` 가 **없다**(`Channel::send` 는 동기). 락은 짧게 잡았다 즉시 푼다. 미러가 사라져 옛
//! "buffer 락 ⊃ registry 락 중첩" 자체가 없다 — 이제 registry 락 하나뿐이라 순서 역전 데드락 표면이 0이다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::output_router::WindowLabel;

/// window label → 그 창의 출력 Channel. 창 mount 시 `subscribe_output` invoke 가 insert,
/// dead window(send Err) 감지 시 connection task 가 remove 한다. connection task 와 Tauri command
/// 양쪽이 `Arc` 로 공유 → app-level manage.
pub type WindowChannelRegistry =
    Arc<Mutex<HashMap<WindowLabel, tauri::ipc::Channel<tauri::ipc::Response>>>>;

/// ★무상태 통과 fan-out(ADR-0046)★. `bytes`(원본 binary frame 또는 replay 경계 마커)를 `labels`
/// (= `router.targets(agent)`) 중 **registry 에 실제 등록된** 창 Channel 로 그대로 보낸다. 미러/cursor 없음
/// — 데몬이 보낸 그대로 통과시키고, 진도/dedup 판정은 전부 웹뷰 뷰 단위가 한다(진도 상태 유일 거처 = 뷰).
///
/// ## ★ADR-0006 락 규율(load-bearing)★
/// registry 락을 한 번 잡아 (a) 등록된 각 label 로 `Channel::send`(동기 — `.await` 0) (b) send Err 인
/// dead label 을 같은 lock 안에서 remove 한다(소멸 webview 는 send 가 Err — spike §7 D6, 절대 unwrap 금지).
/// 미등록 label(미mount 창)은 조용히 skip — 그 창은 mount 시 자기 뷰가 replay 를 재요청한다(뷰 주도).
///
/// ## ★bytes 소유★
/// `Response::new` 는 `Vec<u8>` 소유가 필요하다. 한 frame 을 여러 창에 fan-out 하므로 창당 `to_vec()`
/// 로 복제해 싣는다(대개 창 1개라 복제 1회). marker/frame 공통 경로 — 마커도 binary frame 과 **같은 이
/// 함수**로 흘려 Channel 순서를 보존한다(app.emit 경유 금지 — 순서 붕괴, ADR-0046).
pub fn send_to_windows(registry: &WindowChannelRegistry, labels: &[WindowLabel], bytes: &[u8]) {
    if labels.is_empty() {
        return;
    }
    let mut dead: Vec<WindowLabel> = Vec::new();
    {
        // ★락 across await 없음★: 이 블록 안 `.await` 0 — Channel::send 는 동기.
        let Ok(mut reg) = registry.lock() else {
            tracing::warn!("registry lock poisoned — 출력 frame 통과 스킵");
            return;
        };
        for label in labels {
            if let Some(channel) = reg.get(label) {
                if channel
                    .send(tauri::ipc::Response::new(bytes.to_vec()))
                    .is_err()
                {
                    // 소멸 webview — registry 에서 제거 대상(절대 unwrap 금지, spike §7 D6).
                    dead.push(label.clone());
                }
            }
            // 미등록 label(미mount 창)은 skip — 그 창 mount 시 뷰가 replay 를 재요청한다(뷰 주도).
        }
        for label in &dead {
            reg.remove(label);
        }
    } // ← registry lock drop
    if !dead.is_empty() {
        tracing::debug!(dead = ?dead, "dead window Channel 제거");
    }
}
