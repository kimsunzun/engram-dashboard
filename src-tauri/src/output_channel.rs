//! window Channel registry + 무상태 통과 send 헬퍼 (ADR-0046 — 미러 버퍼 제거, view-direct replay).
//!
//! ## 역할
//! src-tauri 는 더는 데몬 ring 을 미러하지 않는다(ADR-0046).
//!
//! ## ★raw byte 함정(spike §7)★
//! 출력 프레임은 `Channel<tauri::ipc::Response>` 로 운반한다 — `Channel<Vec<u8>>`/`Channel<&[u8]>` 는
//! blanket `impl<T:Serialize> IpcResponse` 가 JSON 배열로 직렬화해 바이트가 샌다. 반드시
//! `Response::new(bytes)` 로 실어 raw 로 보낸다.
//!
//! ## ★동시성/락 규율(ADR-0006, load-bearing)★
//! `std::sync::Mutex` 다(tokio Mutex 아님). connection task(tokio)가 핫패스에서 registry lock→send 하는데
//! 락 보유 중 `.await` 가 **없다**(`Channel::send` 는 동기). 락은 짧게 잡았다 즉시 푼다. registry 락
//! 하나뿐이라 순서 역전 데드락 표면이 0이다.
//!
//! ## ★poison 은 되찾는다 — 락을 쥐는 자리 셋 모두(ADR-0231)★
//! 프레임 통과([`send_to_windows`]) · 창 등록([`register_window`]) · 창 정리([`unregister_window`])가 모두
//! `PoisonError::into_inner` 로 락을 되찾는다. 이 명부는 label → Channel 핸들 map 이라 중간에 난 panic 이
//! 남길 반쪽 상태가 없다. ★건너뛰면 멈춘다★ — 뷰는 seq 구멍 뒤를 시한 없이 붙들므로 통과 자리가 프레임을
//! 버리면 그 뷰가 멈추고, poison 은 풀리지 않아 다시 붙어도 다음 프레임부터 또 건너뛴다. 통과 자리만
//! 되찾으면 프레임은 흐르는데 새 창·다시 연 창이 등록을 못 해 출력을 영영 못 받는다(poison 은 락에 서므로
//! 셋 모두에 선다). 선례 = `view_commands.rs` 의 같은 처분.
//! 되찾을 때 warn 한 번을 남기고 poison 표시를 지운다([`lock_registry`]) — 통과 자리는 프레임마다 불리므로
//! 표시를 남겨 두면 같은 경고가 프레임마다 되풀이된다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::output_router::WindowLabel;

/// 창 mount 시 `subscribe_output` invoke 가 insert, dead window(send Err) 감지 시 connection task 가
/// remove 한다. connection task 와 Tauri command 양쪽이 `Arc` 로 공유 → app-level manage.
pub type WindowChannelRegistry =
    Arc<Mutex<HashMap<WindowLabel, tauri::ipc::Channel<tauri::ipc::Response>>>>;

type RegistryMap = HashMap<WindowLabel, tauri::ipc::Channel<tauri::ipc::Response>>;

// ADR-0231: poison 을 되찾되 poisoning 한 번에 warn 한 번 — `clear_poison` 으로 다음 호출이 같은 경고를
//   되풀이하지 않게 한다(모듈 헤더). 두 스레드가 같은 순간 poison 을 보면 경고가 둘일 수 있다 — 무해.
fn lock_registry(registry: &WindowChannelRegistry) -> MutexGuard<'_, RegistryMap> {
    registry.lock().unwrap_or_else(|e| {
        tracing::warn!("출력 Channel 명부 락이 poisoned — 되찾아 계속 쓴다");
        registry.clear_poison();
        e.into_inner()
    })
}

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
        // ADR-0231: poison 이어도 이 프레임을 건너뛰지 않는다(모듈 헤더).
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

/// 창의 출력 Channel 을 등록한다. 같은 label 재등록(창 reload)은 덮어쓴다(옛 Channel 은 drop — 이미 죽은
/// webview 라 무해). poison 이어도 되찾아 등록한다(모듈 헤더 — ADR-0231).
pub fn register_window(
    registry: &WindowChannelRegistry,
    label: WindowLabel,
    channel: tauri::ipc::Channel<tauri::ipc::Response>,
) {
    lock_registry(registry).insert(label, channel);
}

/// 창의 출력 Channel 을 뺀다(누수 방지 — 죽은 webview Channel 이 남지 않게). poison 이어도 되찾아 뺀다(모듈
/// 헤더 — ADR-0231).
pub fn unregister_window(registry: &WindowChannelRegistry, label: &str) {
    lock_registry(registry).remove(label);
}

#[cfg(test)]
mod tests {
    //! TRD §7-1 「셸 중계」 행의 poison 항목 — 락을 쥐는 자리 셋이 poison 을 되찾아 제 일을 한다.
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tauri::ipc::{Channel, InvokeResponseBody, Response};

    use super::*;

    // 받은 raw 바이트를 적재하는 Channel — 실 webview 없이 `Channel::send` 의 결말을 본다.
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
