//! window Channel registry — window label → 출력 Channel 매핑(S14 모듈① T6b, ADR-0036).
//!
//! ## 왜 output_router.rs 가 아니라 여기인가 (불변식)
//! `output_router.rs` 는 **Tauri 런타임 의존 0**(headless 단독 테스트)이 불변식이다. registry 는
//! `tauri::ipc::Channel`(Tauri 타입)을 들어야 하므로 거기 두면 그 불변식이 깨진다 — 그래서 Tauri 에
//! 의존하는 이 별도 모듈(src-tauri 측)에 둔다. router(순수 라우팅 계산)와 registry(Tauri Channel 보관)는
//! 역할이 다르다: router 가 `agent_id → [window_label]` 을 주면, connection task 가 그 label 들로 이
//! registry 에서 실제 Channel 을 찾아 fan-out 한다.
//!
//! ## ★raw byte 함정(spike §7)★
//! 출력 프레임은 `Channel<tauri::ipc::Response>` 로 운반한다 — `Channel<Vec<u8>>`/`Channel<&[u8]>` 는
//! blanket `impl<T:Serialize> IpcResponse` 가 JSON 배열로 직렬화해 바이트가 샌다. 반드시
//! `Response::new(bytes)` 로 실어 raw 로 보낸다.
//!
//! ## ★동시성(load-bearing)★
//! `std::sync::Mutex` 다(tokio Mutex 아님). connection task(tokio)가 라우팅 핫패스에서 lock→get→send
//! 하는데, `Channel::send` 는 **동기**라 락 보유 중 `.await` 가 없다(ADR-0006 미위반). insert(창 mount)·
//! remove(dead window)도 동기. 락은 짧게 잡았다 즉시 푼다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::output_router::WindowLabel;

/// window label → 그 창의 출력 Channel. 창 mount 시 `subscribe_output` invoke 가 insert,
/// dead window(send Err) 감지 시 connection task 가 remove 한다. connection task 와 Tauri command
/// 양쪽이 `Arc` 로 공유 → app-level manage.
pub type WindowChannelRegistry =
    Arc<Mutex<HashMap<WindowLabel, tauri::ipc::Channel<tauri::ipc::Response>>>>;
