//! window Channel registry + stateless pass-through send helper (ADR-0046 — mirror buffer removed, view-direct replay). src-tauri no longer mirrors the daemon ring.
//! Raw bytes (spike §7): output frames ride `Channel<tauri::ipc::Response>` — `Channel<Vec<u8>>`/`Channel<&[u8]>` get serialized as a JSON array by the blanket `impl<T:Serialize> IpcResponse` and the bytes leak. Always carry `Response::new(bytes)` and send raw.
//! Concurrency/lock discipline (ADR-0006, load-bearing): `std::sync::Mutex` (not tokio's). The connection task does registry lock→send on the hot path with no `.await` while holding the lock (`Channel::send` is synchronous); the lock is taken briefly and released at once. The registry lock being the only one, the lock-order-inversion deadlock surface is 0.
//! Recover from poison at all three lock-holding places (ADR-0231): frame pass-through ([`send_to_windows`]) · window registration ([`register_window`]) · window cleanup ([`unregister_window`]) all recover with `PoisonError::into_inner`. The registry is a label → Channel handle map, so a panic leaves no half state. Skipping stalls: a view holds behind a seq hole with no deadline, so a dropped frame at the pass-through stalls it, and poison does not clear, so even re-attached it skips again from the next frame. Recovering only at the pass-through lets frames flow but a new · reopened window cannot register and never receives output (poison stands on the lock at all three). Precedent: the same handling in `view_commands.rs`. On recovery it leaves one warn and clears the poison mark ([`lock_registry`]) — the pass-through is called per frame, so a left mark would repeat the warning every frame.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::output_router::WindowLabel;

/// `subscribe_output` inserts at window mount, and the connection task removes a dead window (send Err). Shared by the connection task and the Tauri command through `Arc` → app-level manage.
pub type WindowChannelRegistry =
    Arc<Mutex<HashMap<WindowLabel, tauri::ipc::Channel<tauri::ipc::Response>>>>;

type RegistryMap = HashMap<WindowLabel, tauri::ipc::Channel<tauri::ipc::Response>>;

// ADR-0231: recover the poison but warn once per poisoning — `clear_poison` keeps the next call from repeating the warning (module header); two threads seeing the poison at once may warn twice — harmless.
// ADR-0275: do not use the base `sync` — it does not clear the poison; this own helper clearing it is the ADR-0231 contract.
fn lock_registry(registry: &WindowChannelRegistry) -> MutexGuard<'_, RegistryMap> {
    registry.lock().unwrap_or_else(|e| {
        tracing::warn!("output Channel registry lock is poisoned — recovering it and continuing");
        registry.clear_poison();
        e.into_inner()
    })
}

/// Stateless pass-through fan-out (ADR-0046): sends `bytes` (the original binary frame or a replay boundary marker) as they are to the window Channels among `labels` (= `router.targets(agent)`) that are actually registered. No mirror/cursor — it passes through exactly what the daemon sent, and all progress/dedup decisions are per webview view (the only home of progress state).
/// ADR-0006 lock discipline (load-bearing): take the registry lock once and (a) `Channel::send` to each registered label (synchronous — 0 `.await`) (b) remove inside the same lock the dead labels whose send is Err (a send to a destroyed webview is Err — spike §7 D6, never unwrap). An unregistered label (a window not yet mounted) is silently skipped — when it mounts its own view re-requests the replay (view-driven).
/// bytes ownership: `Response::new` needs an owned `Vec<u8>`, and a frame fans out to several windows, so it is copied with `to_vec()` per window (usually one). A marker also flows through this same function as a binary frame to preserve Channel order (going through app.emit would collapse order, ADR-0046).
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
        tracing::debug!(dead = ?dead, "removed dead window Channels");
    }
}

/// Registers a window's output Channel. Re-registering the same label (window reload) overwrites (the old Channel is dropped — its webview is already dead). Even if poisoned it recovers and registers (module header — ADR-0231).
pub fn register_window(
    registry: &WindowChannelRegistry,
    label: WindowLabel,
    channel: tauri::ipc::Channel<tauri::ipc::Response>,
) {
    lock_registry(registry).insert(label, channel);
}

/// Removes a window's output Channel (no dead webview Channel remains — leak prevention). Even if poisoned it recovers and removes (module header — ADR-0231).
pub fn unregister_window(registry: &WindowChannelRegistry, label: &str) {
    lock_registry(registry).remove(label);
}

#[cfg(test)]
mod tests {
    // Poison item of the TRD §7-1 「shell relay」 row — the three lock-holding places recover the poison and do their job.
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
            panic!("panic while holding the registry lock — poisons the lock");
        })
        .join();
        assert!(registry.is_poisoned(), "precondition: the lock is poisoned");
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
            "after recovering, the poison mark is cleared so the next call does not repeat the warning"
        );
        send_to_windows(&registry, std::slice::from_ref(&main), b"frame");
        assert_eq!(
            got.lock().unwrap().as_slice(),
            &[b"frame".to_vec()],
            "even when poisoned, frames go out to the registered window"
        );

        unregister_window(&registry, &main);
        send_to_windows(&registry, std::slice::from_ref(&main), b"after");
        assert_eq!(
            got.lock().unwrap().len(),
            1,
            "even when poisoned, cleanup removed the Channel"
        );
    }
}
