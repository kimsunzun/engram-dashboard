//! window Channel registry + stateless pass-through send helper (ADR-0046 — mirror buffer removed, view-direct replay).
//!
//! ## Role
//! src-tauri no longer mirrors the daemon ring (ADR-0046).
//!
//! ## ★raw byte pitfall (spike §7)★
//! Output frames are carried by `Channel<tauri::ipc::Response>` — `Channel<Vec<u8>>`/`Channel<&[u8]>` get
//! serialized as a JSON array by the blanket `impl<T:Serialize> IpcResponse` and the bytes leak. Always
//! carry it with `Response::new(bytes)` and send it raw.
//!
//! ## ★Concurrency/lock discipline (ADR-0006, load-bearing)★
//! It is `std::sync::Mutex` (not tokio Mutex). The connection task (tokio) does registry lock→send on the hot path, and
//! there is **no** `.await` while the lock is held (`Channel::send` is synchronous). The lock is taken briefly and released at once. With the registry lock
//! being the only one, the lock-order-inversion deadlock surface is 0.
//!
//! ## ★Recover from poison — at all three places that hold the lock (ADR-0231)★
//! Frame pass-through ([`send_to_windows`]) · window registration ([`register_window`]) · window cleanup ([`unregister_window`]) all
//! recover the lock with `PoisonError::into_inner`. This registry is a label → Channel handle map, so there is no half state a panic in the middle
//! could leave. ★Skipping stalls it★ — a view holds behind a seq hole with no deadline, so if the pass-through place drops a frame, that view stalls,
//! and poison does not clear, so even after re-attaching it skips again from the next frame. If only the pass-through place recovers, frames flow,
//! but a new window · reopened window cannot register and never receives output (poison stands on the lock, so it stands at
//! all three). Precedent = the same handling in `view_commands.rs`.
//! When recovering, it leaves one warn and clears the poison mark ([`lock_registry`]) — the pass-through place is called per frame, so
//! if the mark were left, the same warning would repeat every frame.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::output_router::WindowLabel;

/// At window mount the `subscribe_output` invoke inserts, and when a dead window (send Err) is detected the connection task
/// removes it. Shared by both the connection task and the Tauri command through `Arc` → app-level manage.
pub type WindowChannelRegistry =
    Arc<Mutex<HashMap<WindowLabel, tauri::ipc::Channel<tauri::ipc::Response>>>>;

type RegistryMap = HashMap<WindowLabel, tauri::ipc::Channel<tauri::ipc::Response>>;

// ADR-0231: recover the poison but warn once per poisoning — `clear_poison` keeps the next call from
//   repeating the same warning (module header). If two threads see the poison at the same moment there may be two warnings — harmless.
// ADR-0275: do not use the base `sync` — that one does not clear the poison. This own helper that clears the poison is the contract of ADR-0231.
fn lock_registry(registry: &WindowChannelRegistry) -> MutexGuard<'_, RegistryMap> {
    registry.lock().unwrap_or_else(|e| {
        tracing::warn!("출력 Channel 명부 락이 poisoned — 되찾아 계속 쓴다");
        registry.clear_poison();
        e.into_inner()
    })
}

/// ★Stateless pass-through fan-out (ADR-0046)★. Sends `bytes` (the original binary frame or a replay boundary marker) as they are to the window Channels among `labels`
/// (= `router.targets(agent)`) that are **actually registered in the registry**. No mirror/cursor
/// — it passes through exactly what the daemon sent, and all progress/dedup decisions are made per webview view (the only home of progress state = the view).
///
/// ## ★ADR-0006 lock discipline (load-bearing)★
/// Take the registry lock once and (a) `Channel::send` to each registered label (synchronous — `.await` 0) (b) remove, inside the same lock, the
/// dead labels whose send is Err (send to a destroyed webview is Err — spike §7 D6, never unwrap).
/// An unregistered label (a window not yet mounted) is silently skipped — when that window mounts, its own view re-requests the replay (view-driven).
///
/// ## ★bytes ownership★
/// `Response::new` needs an owned `Vec<u8>`. A frame is fanned out to several windows, so it is
/// copied with `to_vec()` per window (usually one window, so one copy). Common path for marker/frame — a marker also flows through **this same
/// function** as a binary frame to preserve Channel order (no going through app.emit — order collapse, ADR-0046).
pub fn send_to_windows(registry: &WindowChannelRegistry, labels: &[WindowLabel], bytes: &[u8]) {
    if labels.is_empty() {
        return;
    }
    let mut dead: Vec<WindowLabel> = Vec::new();
    {
        // ADR-0231: even if poisoned, this frame is not skipped (module header).
        let mut reg = lock_registry(registry);
        for label in labels {
            if let Some(channel) = reg.get(label) {
                if channel
                    .send(tauri::ipc::Response::new(bytes.to_vec()))
                    .is_err()
                {
                    dead.push(label.clone());
                }
            }
        }
        for label in &dead {
            reg.remove(label);
        }
    }
    if !dead.is_empty() {
        tracing::debug!(dead = ?dead, "dead window Channel 제거");
    }
}

/// Registers a window's output Channel. Re-registering the same label (window reload) overwrites (the old Channel is dropped — harmless since it is an already dead
/// webview). Even if poisoned, it recovers and registers (module header — ADR-0231).
pub fn register_window(
    registry: &WindowChannelRegistry,
    label: WindowLabel,
    channel: tauri::ipc::Channel<tauri::ipc::Response>,
) {
    lock_registry(registry).insert(label, channel);
}

/// Removes a window's output Channel (to prevent leaks — so no dead webview Channel remains). Even if poisoned, it recovers and removes (module
/// header — ADR-0231).
pub fn unregister_window(registry: &WindowChannelRegistry, label: &str) {
    lock_registry(registry).remove(label);
}

#[cfg(test)]
mod tests {
    //! Poison item of the TRD §7-1 「셸 중계」 row — the three places that hold the lock recover the poison and do their job.
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tauri::ipc::{Channel, InvokeResponseBody, Response};

    use super::*;

    // A Channel that stores the raw bytes it receives — see the outcome of `Channel::send` without a real webview.
    fn recording_channel() -> (Channel<Response>, Arc<Mutex<Vec<Vec<u8>>>>) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Raw(bytes) = body {
                sink.lock().unwrap().push(bytes);
            }
            Ok(())
        });
        (channel, got)
    }

    fn poisoned_registry() -> WindowChannelRegistry {
        let registry: WindowChannelRegistry = Arc::new(Mutex::new(HashMap::new()));
        let r = registry.clone();
        let _ = std::thread::spawn(move || {
            let _guard = r.lock().unwrap();
            panic!("registry 락을 쥔 채 panic — 락을 poison 시킨다");
        })
        .join();
        assert!(registry.is_poisoned(), "전제: 락이 poisoned");
        registry
    }

    #[test]
    fn a_poisoned_registry_still_registers_passes_frames_and_unregisters() {
        let registry = poisoned_registry();
        let (channel, got) = recording_channel();
        let main = "main".to_string();

        register_window(&registry, main.clone(), channel);
        assert!(
            !registry.is_poisoned(),
            "되찾은 뒤 poison 표시를 지워 다음 호출이 경고를 되풀이하지 않는다"
        );
        send_to_windows(&registry, std::slice::from_ref(&main), b"frame");
        assert_eq!(
            got.lock().unwrap().as_slice(),
            &[b"frame".to_vec()],
            "poison 이어도 등록된 창으로 프레임이 나간다"
        );

        unregister_window(&registry, &main);
        send_to_windows(&registry, std::slice::from_ref(&main), b"after");
        assert_eq!(
            got.lock().unwrap().len(),
            1,
            "poison 이어도 정리가 Channel 을 뺐다"
        );
    }
}
