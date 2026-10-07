//! 슬롯 팝업 분리(move_slot_to_window) invoke 껍데기 + 런타임 창 label·빌드·destroy(= 적용 서비스의 OS
//! 창 포트·label 발급 포트 어댑터, `crate::layout::apply`) — 탭 소유 모델(ADR-0057).
//!
//! ★§5 LLM 제어 표면★: 사람 우클릭(프론트 레지스트리의 `slot.popout` → `viewStore.moveSlotToWindow`
//! invoke)과 LLM(명령 버스의 `slot.popout`)이 **같은 적용 서비스**에 떨어진다 — 이 파일에 MOVE 로직은
//! 없다(`apply::move_slot_to_window`).
//! 여기 남는 것은 Tauri 세계로의 번역뿐이다: 창 빌드·destroy·존재 확인, label·탭 이름 발급, 창 정리.
//!
//! ★두 경로가 **백엔드에서만** 합류한다 — 그 앞은 아직 다르다★: 프론트는 invoke 전에 그 슬롯의 렌더 모드
//! 오버라이드를 지운다(`src/store/viewStore.ts` 의 `clearRenderMode` — 슬롯이 사라지므로 누수 방지). 버스
//! 경로엔 그 단계가 없어 죽은 slot id 의 오버라이드가 웹뷰에 남는다. 같은 부류가 셋 더 있다 —
//! `slot.close`·`slot.assignAgent`·`layout.setSlotContent` 도 프론트 쪽에서 같은 정리를 하고 버스 쪽에서는
//! 안 한다. 해소는 웹뷰 소유 상태를 백엔드로 올리는 후속 스텝 몫이다(여기서 흉내내지 말 것 — 셸은 그
//! 상태를 갖고 있지 않다).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use uuid::Uuid;

use crate::commands::layout::{RouterSubs, TauriEvents};
use crate::daemon_client::DaemonClient;
use crate::layout::{apply, LabelSource, LayoutState, SlotMove, WindowHost, MAIN_WINDOW_LABEL};
use crate::output_router::OutputRouter;
use crate::webview_env::WebviewEnv;

// 팝업/런타임 창 label prefix. capabilities/popup.json·usage-links.json 의 `"slot-popup-*"` glob 과 짝(변경 시
// 함께 동기).
// ★의미 확장(ADR-0057/G8)★: "팝업" → "런타임 창"(create_window 포함). prefix 값은 불변(Destroyed 정리
// 게이트 is_popup_label 재사용 — 다른 label 이면 cleanup 스킵 → 라우팅/구독/Channel 누수).
const POPUP_LABEL_PREFIX: &str = "slot-popup-";

// 팝업/런타임 창 label 발급용 단조 카운터. app-level 공유(app.manage). ★재사용 금지 불변식★: fetch_add
// 로 단조 증가만 하고 창을 닫아도 되돌리지 않는다(닫힌 label 재-build 에러 회피).
#[derive(Default)]
pub struct PopupCounter(pub AtomicU64);

impl PopupCounter {
    fn next_label(&self) -> String {
        let n = self.0.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{POPUP_LABEL_PREFIX}{n}")
    }
}

impl LabelSource for PopupCounter {
    fn next_label(&self) -> String {
        PopupCounter::next_label(self)
    }

    // label 형식(`slot-popup-<n>`)을 아는 쪽이 짓는다 — "slot-popup-3" → "Popup 3".
    // ★바이트 슬라이싱 금지★: 이건 공개 포트라 어떤 `&str` 이든 들어올 수 있고, 인덱스로 자르면 문자
    //   경계가 아닌 자리에서 패닉한다 — 릴리즈는 `panic = "abort"` 라 받아 줄 그물이 없다.
    fn tab_name(&self, label: &str) -> String {
        format!(
            "Popup {}",
            label.strip_prefix(POPUP_LABEL_PREFIX).unwrap_or(label)
        )
    }
}

// 적용 서비스의 OS 창 포트(`crate::layout::apply`) — 창 빌드·destroy·존재 확인은 이 파일이 소유한 Tauri 기법이다.
pub(crate) struct TauriWindowHost<'a> {
    pub app: &'a AppHandle,
}

impl WindowHost for TauriWindowHost<'_> {
    fn open(&self, label: &str) -> Result<(), String> {
        build_runtime_window(self.app, label, None).map(drop)
    }

    fn close(&self, label: &str) {
        destroy_window(self.app, label);
    }

    fn is_open(&self, label: &str) -> bool {
        self.app.get_webview_window(label).is_some()
    }

    fn record_placement(&self, label: &str) {
        if let Some(window) = self.app.get_webview_window(label) {
            crate::state::placement::record(&window.as_ref().window());
        }
    }
}

// lib.rs Destroyed arm 이 main/agent-tree 와 구분하는 데 쓴다.
pub fn is_popup_label(label: &str) -> bool {
    label.starts_with(POPUP_LABEL_PREFIX)
}

// ★URL 키 = `?window=<label>`(ADR-0057/§3-3)★: 팝업 페이지는 "고정 뷰"가 아니라 "이 창의 활성 탭"을
// 그린다(활성 탭은 백엔드 `windows[label].active` 가 권위).
fn window_url(label: &str) -> String {
    format!("index.html#/popup?window={label}")
}

// 대각 cascade 위치(창이 겹쳐 뜨는 것 방지).
fn cascade_position(label: &str) -> (f64, f64) {
    let n: u32 = label
        .strip_prefix(POPUP_LABEL_PREFIX)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let step = (n.saturating_sub(1) % 8) as f64;
    (140.0 + step * 72.0, 110.0 + step * 60.0)
}

// WebviewWindowBuilder 로 런타임 창을 빌드(★락 밖에서만 호출 — 데드락 회피★). 웹뷰 환경은 공통 마무리
// (`WebviewEnv::finish`)가 붙인다 — 빠뜨리면 유령 창(ADR-0054).
//
// `at` = 빌더에 줄 첫 자리 — `None` 이면 label 순번의 계단 자리 · 기본 크기. 저장된 자리를 정확히 놓는 것은 부르는
//   쪽(`state::placement`)이 만든 뒤에 한다: tao 는 빌더의 논리 위치를 모니터를 열거 순으로 훑어 처음 들어맞는 것의
//   배율로 풀어(tao 0.35 `platform_impl/windows/window.rs` `init`), 배율이 다른 모니터가 섞이면 다른 모니터에 뜰 수 있다.
pub(crate) fn build_runtime_window(
    app: &AppHandle,
    label: &str,
    at: Option<(LogicalPosition<f64>, LogicalSize<f64>)>,
) -> Result<WebviewWindow, String> {
    build_window(app, label, at, true)
}

/// [`build_runtime_window`] 와 같되 숨긴 채 만든다 — 보이기는 부르는 쪽 몫이다(런타임 복원 수락이 모델에 넣은
/// 뒤 보인다 — TRD S21-storage §6-7 ② ④).
pub(crate) fn build_hidden_runtime_window(
    app: &AppHandle,
    label: &str,
    at: Option<(LogicalPosition<f64>, LogicalSize<f64>)>,
) -> Result<WebviewWindow, String> {
    build_window(app, label, at, false)
}

fn build_window(
    app: &AppHandle,
    label: &str,
    at: Option<(LogicalPosition<f64>, LogicalSize<f64>)>,
    visible: bool,
) -> Result<WebviewWindow, String> {
    let ((x, y), (w, h)) = match at {
        Some((at, size)) => ((at.x, at.y), (size.width, size.height)),
        None => (cascade_position(label), (720.0, 500.0)),
    };
    // 웹뷰 환경 없이 만들지 않는다 — 마무리를 건너뛴 창은 다른 창과 환경이 갈린다(ADR-0054).
    let env = app
        .try_state::<Arc<WebviewEnv>>()
        .ok_or_else(|| "런타임 창 생성 실패: 웹뷰 환경이 등록돼 있지 않다".to_owned())?;
    env.finish(
        WebviewWindowBuilder::new(app, label, WebviewUrl::App(window_url(label).into()))
            .title(format!("Engram — {label}"))
            .inner_size(w, h)
            .position(x, y)
            .visible(visible),
    )
    .build()
    .map_err(|e| format!("런타임 창 생성 실패: {e}"))
}

// Destroyed 이벤트 → lib.rs Destroyed arm → cleanup_popup_window 가 잔여 정리.
// ★창 닫힘 = 백엔드 단일 소스(§5-2/G2)★: 프론트로 별도 view:closed 를 안 쏜다(이중 발화·재진입 방지).
// registry 는 여기선 안 건드린다(Destroyed→cleanup 이 정리) — 그래서 인자로도 안 받는다(F5).
pub fn destroy_window(app: &AppHandle, label: &str) {
    if let Err(e) = try_destroy_window(app, label) {
        tracing::warn!(label, "destroy_window 실패(창 이미 닫힘일 수 있음): {e}");
    }
}

/// [`destroy_window`] 와 같되 실패를 돌려준다 — 거두기를 다시 해 보거나 남은 창을 알려야 하는 쪽(런타임 복원 수락)이
/// 쓴다. OS 창이 없으면 `Ok`.
pub(crate) fn try_destroy_window(app: &AppHandle, label: &str) -> Result<(), String> {
    match app.get_webview_window(label) {
        Some(w) => w.destroy().map_err(|e| e.to_string()),
        None => {
            tracing::debug!(label, "destroy_window: OS 창 없음(이미 닫힘) — no-op");
            Ok(())
        }
    }
}

// 슬롯을 다른 창의 새 탭으로 MOVE — 로직·락 규율·롤백은 전부 적용 서비스가 소유한다.
//
// ★async fn 필수★: WebviewWindowBuilder 데드락 회피(새 창 타깃은 서비스의 phase B 에서 빌드된다).
#[tauri::command]
pub async fn move_slot_to_window(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    counter: State<'_, Arc<PopupCounter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
    to_window: Option<String>,
) -> Result<SlotMove, String> {
    apply::move_slot_to_window(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        &TauriWindowHost { app: &app },
        &**counter,
        view_id,
        slot_id,
        to_window,
    )
}

// ★창 Destroyed 정리(수명/누수 임계 — 멀티탭, G1)★. 팝업/런타임 창이 닫히면(titlebar close·강제 destroy·
// close_tab/close_window 경유 destroy) lib.rs Destroyed arm 이 이걸 부른다.
//
// ★현 버그 수정(G1)★: 옛 코드는 단일 바인딩 하나만 정리해 멀티탭 팝업을 강제 종료하면 나머지 탭이 잔류
// + Unsubscribe 누락.
//
// ★이 함수는 command 가 아니다★ — Rust 이벤트 핸들러(on_window_event)에서 직접 호출. State 대신 이미
// 손에 쥔 Arc 참조들을 인자로 받는다(lib.rs 가 app.state 로 꺼내 넘김). `_app` 은 현재 미사용(향후 즉시
// emit 여지로 시그니처 통일).
pub fn cleanup_popup_window(
    _app: &tauri::AppHandle,
    label: &str,
    state: &LayoutState,
    router: &OutputRouter,
    registry: &crate::output_channel::WindowChannelRegistry,
    client: &DaemonClient,
) {
    // main 은 절대 정리 대상 아님(불변식 4 — hide only, Destroyed 안 남). 방어적으로 스킵.
    if label == MAIN_WINDOW_LABEL {
        return;
    }

    // 1) 모델 정리 — ViewManager 락 한 번 안(아래 함수).
    if !drop_window_in_model(label, state, router, client) {
        return;
    }

    // 2) 출력 Channel registry 에서 이 label 제거(누수 방지 — 죽은 webview Channel 이 남지 않게). Tauri
    //   부분이라 별도 락(ViewManager 무관) — 코어(모델·라우팅) 밖이라 락 밖 유지 OK(F1).
    //   ★poison 이어도 뺀다(ADR-0231)★ — 옛 건너뛰기는 죽은 Channel 을 남겼다(근거 정본 = `output_channel`
    //   모듈 헤더).
    crate::output_channel::unregister_window(registry, label);

    tracing::info!(label, "런타임 창 정리 완료(탭 전부 드롭·구독·Channel)");
}

// 창 소멸 정리의 모델 몫 — 창의 모든 탭 View 드롭 + windows 엔트리 제거 + 라우팅 표 재계산 + 구독 정리 발화 +
// 사용량 관심 재계산, ★전부 같은 락 안★. `false` = 락 오염(정리 스킵). 창(`AppHandle`) 없이 서므로 셸 단위
// 시험이 이것을 직접 부른다.
//   ★F1 REAL 동시성 버그 수정★: 옛 코드는 델타(cleanup_window_core rebuild)를 락 안에서 계산하고
//   `to_unsubscribe` 발화를 락 드롭 뒤에 했다 → 계산~발화 사이 다른 command(assign_agent/spawn/move)가
//   그 agent 를 재추가하면 stale 1→0 unsubscribe 가 방금 형성된 라이브 구독을 죽인다
//   (`output_router::rebuild` 의 호출 계약 위반 — 정본은 그 함수 주석이고 ★ADR-0006 에 「델타 enqueue
//   는 락 안」 조항은 없다★). 이제 적용 서비스(`SubscriptionSync::resync`)·move_slot_to_window 와
//   일관되게 발화도 락 안이다(unsubscribe 는 동기 try_send — await/network 0, lifecycle 락 독립 →
//   데드락 없음).
pub(crate) fn drop_window_in_model(
    label: &str,
    state: &LayoutState,
    router: &OutputRouter,
    client: &DaemonClient,
) -> bool {
    let Ok(mut mgr) = state.0.lock() else {
        tracing::warn!(label, "cleanup_popup_window: lock poisoned — 정리 스킵");
        return false;
    };
    // 창이 이미 모델에서 지워졌으면 rebuild 만.
    let delta = crate::output_router::cleanup_window_core(&mut mgr, router, label);
    // 이 창이 마지막이던 agent 는 1→0 → Unsubscribe(락 안 발화 — F1).
    for a in delta.to_unsubscribe {
        client.unsubscribe(a);
    }
    // ★모델이 이 창을 쥐고 있었는지로 가르지 않는다(TRD S21 usage-limit-slot §1-7 「창 소멸」)★ — 숨김 표시는
    //   모델과 따로 선다: 새 창으로 슬롯 옮기기는 OS 창을 먼저 열고 모델 창을 나중에 더하므로, 그 틈에 숨겨졌다
    //   닫힌 창은 모델에 없어도 표시가 남는다. label 은 다시 쓰이지 않아(`PopupCounter`) 여기서 안 지우면 샌다.
    client.usage_window_destroyed(label, &mgr);
    true
}

// ── 테스트: label 발급·prefix 판정(창 생성 자체는 running app 필요라 GUI 검증) ──────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_counter_monotonic_never_reuses() {
        let c = PopupCounter::default();
        let a = c.next_label();
        let b = c.next_label();
        assert_eq!(a, "slot-popup-1");
        assert_eq!(b, "slot-popup-2");
        assert_ne!(a, b, "label 재사용 금지 — 단조 증가");
    }

    #[test]
    fn is_popup_label_matches_prefix_only() {
        assert!(is_popup_label("slot-popup-1"));
        assert!(is_popup_label("slot-popup-42"));
        assert!(!is_popup_label("main"));
        assert!(!is_popup_label("agent-tree"));
    }

    #[test]
    fn window_url_uses_window_key() {
        // ★URL 키 = ?window=<label>(ADR-0057)★ — 옛 ?view=<id> 아님.
        assert_eq!(
            window_url("slot-popup-3"),
            "index.html#/popup?window=slot-popup-3"
        );
    }

    #[test]
    fn cascade_position_offsets_by_label_index() {
        assert_eq!(cascade_position("slot-popup-1"), (140.0, 110.0));
        // 9번째는 wrap(8개마다).
        assert_eq!(cascade_position("slot-popup-9"), (140.0, 110.0));
    }
}
