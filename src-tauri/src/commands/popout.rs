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
use std::time::Instant;

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use uuid::Uuid;

use crate::commands::layout::{RouterSubs, TauriEvents};
use crate::daemon_client::DaemonClient;
use crate::first_paint::NewWindowShow;
use crate::layout::{
    apply, LabelSource, LayoutState, MoveFailed, SlotMove, SpareClaim, WindowHost, WindowPlace,
    MAIN_WINDOW_LABEL,
};
use crate::output_router::OutputRouter;
use crate::spare::pool::Opening;
use crate::state::placement::{self, Fallback};
use crate::webview_env::{Backdrop, WebviewEnv};

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
    //   경계가 아닌 자리에서 패닉한다 — 셸은 아무것도 가두지 않아 그 패닉이 곧 셸 종료다(훅 · ADR-0292).
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
        open_new_window(self.app, label, None, true)
    }

    fn open_at(&self, label: &str, at: &WindowPlace, activate: bool) -> Result<(), String> {
        open_new_window(self.app, label, Some(at), activate)
    }

    // ADR-0299
    fn claim_spare(&self) -> Option<SpareClaim> {
        crate::commands::spare::claim(self.app)
    }

    fn committed(&self, claim: &SpareClaim) {
        crate::commands::spare::commit(self.app, claim);
    }

    fn release_spare(&self, claim: &SpareClaim) {
        crate::commands::spare::release(self.app, claim);
    }

    fn close(&self, label: &str) {
        destroy_window(self.app, label);
    }

    fn is_open(&self, label: &str) -> bool {
        self.app.get_webview_window(label).is_some()
    }

    fn move_window(&self, label: &str, at: &WindowPlace) -> Result<(), MoveFailed> {
        placement::move_window(self.app, label, at)
    }

    fn record_placement(&self, label: &str) {
        if let Some(window) = self.app.get_webview_window(label) {
            crate::state::placement::record(&window.as_ref().window());
        }
    }
}

// 창 포트의 새 창 열기 — 맡긴 예열 창이면 짓지 않고 숨긴 채 놓고, 풀과 무관한 label 이면 새로 짓는다. `at = None` =
//   `open`(계단 자리 · 기본 크기). 새로 짓는 창은 열 때 정한다 — 활성 요청이고 전경이 우리면 지금처럼 포커스를 받으며
//   짓고, 아니면 포커스 없이 지어 올리기만 하는 보이기로 둔다(I30). 맡긴 창의 활성화는 보이기 바로 앞에 정한다(I5).
//   `window_open` 줄 = 예열 창 · 새로 짓기 지연을 가르는 재료(TRD S21-slot-docking §3-9 「로그」 · M14 — 열기 → 보임은
//   첫 화면 풀림 줄의 `reveal_ms` · 짓는 중 기다림은 `spare_claim` 줄의 `wait_ms`).
// ADR-0299
fn open_new_window(
    app: &AppHandle,
    label: &str,
    at: Option<&WindowPlace>,
    activate: bool,
) -> Result<(), String> {
    let started = Instant::now();
    match crate::commands::spare::opening(app, label, activate) {
        Opening::Claimed => {
            let placed = place_claimed(app, label, at)?;
            tracing::info!(
                module = "spare",
                label,
                via = "spare",
                activate,
                place_ms = crate::commands::spare::elapsed_ms(started),
                scale_from = placed.scale_from,
                scale_to = ?placed.scale_to,
                "창 포트 — 새 창을 열었다(window_open)"
            );
            Ok(())
        }
        Opening::Unavailable => {
            tracing::warn!(
                module = "spare",
                label,
                "창 포트 — 맡긴 예열 창이 사라졌다 · 그 label 로 새로 짓지 않는다"
            );
            Err(format!(
                "맡긴 예열 창 {label} 이 사라져 그 label 로 새로 짓지 않는다"
            ))
        }
        Opening::Fresh => {
            let focused =
                crate::first_paint::new_window_show(activate, placement::foreground_is_ours(app))
                    == NewWindowShow::RaiseAndActivate;
            match at {
                Some(at) => open_runtime_window_at(app, label, at, focused, started)?,
                None => {
                    let window = build_window(app, label, None, focused)?;
                    crate::first_paint::hold_built(&window, focused, started);
                }
            }
            tracing::info!(
                module = "spare",
                label,
                via = "fresh",
                activate,
                focused,
                build_ms = crate::commands::spare::elapsed_ms(started),
                "창 포트 — 새 창을 열었다(window_open)"
            );
            Ok(())
        }
    }
}

/// 맡긴 예열 창을 놓은 뒤의 배율 — 다른 배율 모니터로 옮긴 채택을 가르는 로그 몫(I9 · M16).
struct SparePlaced {
    scale_from: f64,
    scale_to: Option<f64>,
}

// 맡긴 예열 창을 숨긴 채 놓는다([`spare_spot`] · 위치 → 크기 — 까닭 = [`placement::place`]). 보이지 않는다 — 보이기는
//   확정 뒤 첫 화면이다. 못 놓으면 그 창을 거두고 `Err` 다(창 포트 `open_at` 계약 — 떠 있는 창이 없다 · 그 `Destroyed` 가
//   맡김을 끝내고 다시 짓는다). 창이 이미 없으면 거둘 것 없이 `Err` 다 — 그 label 로 새로 짓지 않는다(I20).
// ADR-0299
fn place_claimed(
    app: &AppHandle,
    label: &str,
    at: Option<&WindowPlace>,
) -> Result<SparePlaced, String> {
    let Some(window) = app.get_webview_window(label) else {
        return Err(format!(
            "맡긴 예열 창 {label} 이 없다 — 그 label 로 새로 짓지 않는다"
        ));
    };
    let window = window.as_ref().window();
    let placed = window
        .scale_factor()
        .map_err(|e| format!("배율을 못 읽었다: {e}"))
        .and_then(|scale_from| {
            let monitors = match at.and_then(|at| at.pos) {
                Some(_) => placement::monitors(app),
                None => Vec::new(),
            };
            let (pos, size) = spare_spot(label, at, &monitors, scale_from);
            placement::move_steps(&window, Some(pos), Some(size))
                .map_err(|failed| format!("{:?}: {}", failed.step, failed.error))?;
            Ok(SparePlaced {
                scale_from,
                scale_to: window.scale_factor().ok(),
            })
        });
    placed.map_err(|reason| {
        tracing::warn!(
            module = "spare",
            label,
            error = %reason,
            "맡긴 예열 창을 놓지 못해 거둔다"
        );
        let gone = match try_destroy_window(app, label) {
            Ok(()) => "거뒀다".to_string(),
            Err(e) => {
                crate::commands::spare::abandon_claimed(app, label);
                format!("거두기도 실패: {e}")
            }
        };
        format!("맡긴 예열 창 자리를 못 입혔다({reason}) — {gone}")
    })
}

// 맡긴 예열 창의 자리 — 물리 바깥 왼쪽 위 · 논리 안쪽 크기. 새로 짓는 창([`open_runtime_window_at`] · [`build_window`])과
//   같은 셈이다 — `at` 이 어느 모니터에 걸치면 그 착지, 아니면 label 의 계단 자리를 그 창의 지금 배율 `scale` 로 푼 것(Tauri
//   의 논리 위치 세터와 같은 풀이) · 크기 = `at.size`(없거나 쓸 수 없으면 기본). 돌려받은 예열 창은 앞선 놓기가 옮기고 크기를
//   바꿨을 수 있어 `open`(`at = None`)도 계단 자리 · 기본 크기를 다시 입힌다(I18).
fn spare_spot(
    label: &str,
    at: Option<&WindowPlace>,
    monitors: &[placement::MonitorArea],
    scale: f64,
) -> ((i32, i32), (f64, f64)) {
    let landed = at.and_then(|at| {
        placement::resolve_place(
            label,
            at,
            POPOUT_DEFAULT_SIZE,
            monitors,
            Fallback::DefaultPlace,
        )
    });
    if let Some((bounds, landing)) = landed {
        return ((landing.x, landing.y), (bounds.w(), bounds.h()));
    }
    let size = at
        .and_then(|at| placement::usable_size(at.size))
        .unwrap_or(POPOUT_DEFAULT_SIZE);
    let (x, y) = cascade_position(label);
    (
        ((x * scale).round() as i32, (y * scale).round() as i32),
        size,
    )
}

// lib.rs Destroyed arm 이 main 과 구분하는 데 쓴다.
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
//
// ★숨긴 채 만든다 — 보이기는 부르는 쪽 몫이다★: 자리 · 최대화를 입힌 뒤 첫 화면에 보이거나
//   (`first_paint::hold_window`), 런타임 복원 수락이 모델에 넣은 뒤 보이기로 정하면 첫 화면에 보인다
//   (`first_paint::reveal_held` · TRD S21-storage §6-7 ② ④).
pub(crate) fn build_runtime_window(
    app: &AppHandle,
    label: &str,
    at: Option<(LogicalPosition<f64>, LogicalSize<f64>)>,
) -> Result<WebviewWindow, String> {
    build_window(app, label, at, true)
}

/// 예비 창(TRD S21-slot-docking §3-9) — 보통 팝아웃과 같은 빌더 · URL(예비 표시 없음)로 숨긴 채 · 포커스 없이
/// 짓는다(tao 의 보이기가 활성화하지 않게). 모델에 넣지 않고 자리도 적지 않는다 — 풀에 적기는 부르는 쪽
/// (`commands::spare` 의 예비 창 짓기)이다.
// ADR-0295
pub(crate) fn build_spare_window(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    build_window(app, label, None, false)
}

/// 팝아웃 기본 크기(논리) — 자리 없이 여는 런타임 창과, 크기를 고르지 않은 도킹 새 창.
pub(crate) const POPOUT_DEFAULT_SIZE: (f64, f64) = (720.0, 500.0);

// 도킹의 새 창(창 포트 `open_at`) — `at` 이 어느 모니터에 걸치면 빌더에 첫 자리를 주고 만든 뒤 그 자리를 다시
//   정확히 입힌다(위치 먼저 — 그 까닭 = [`placement::place`] · [`build_runtime_window`] 머리). 안 걸치거나 위치가 없으면
//   label 의 계단 자리다. 크기가 없거나 쓸 수 없는 값이면 기본 크기다. `activate` = 포커스를 받으며 짓는다(아니면 올리기만
//   하는 보이기 — `first_paint::hold_built`) · `opened` = 창 포트가 열기를 시작한 때.
// TRD S21-slot-docking §1-2 「적용 서비스」
fn open_runtime_window_at(
    app: &AppHandle,
    label: &str,
    at: &WindowPlace,
    activate: bool,
    opened: Instant,
) -> Result<(), String> {
    let monitors = match at.pos {
        Some(_) => placement::monitors(app),
        None => Vec::new(),
    };
    match placement::resolve_place(
        label,
        at,
        POPOUT_DEFAULT_SIZE,
        &monitors,
        Fallback::DefaultPlace,
    ) {
        Some(landed) => {
            let window = build_window(app, label, Some(placement::first_place(landed)), activate)?;
            let (bounds, landing) = landed;
            // 고른 자리를 못 입혔으면 그 창을 거두고 실패로 답한다 — 남기면 도킹이 엉뚱한 자리의 창에 탭을 붙인다(창 포트
            //   `open_at` 계약).
            let placed = placement::move_steps(
                &window.as_ref().window(),
                Some((landing.x, landing.y)),
                Some((bounds.w(), bounds.h())),
            );
            if let Err(failed) = placed {
                let gone = match try_destroy_window(app, label) {
                    Ok(()) => "거뒀다".to_string(),
                    Err(e) => format!("거두기도 실패: {e}"),
                };
                return Err(format!(
                    "새 창 자리를 못 입혔다({:?}: {}) — {gone}",
                    failed.step, failed.error
                ));
            }
            crate::first_paint::hold_built(&window, activate, opened);
        }
        None => {
            let (w, h) = placement::usable_size(at.size).unwrap_or(POPOUT_DEFAULT_SIZE);
            let (x, y) = cascade_position(label);
            let window = build_window(
                app,
                label,
                Some((LogicalPosition::new(x, y), LogicalSize::new(w, h))),
                activate,
            )?;
            crate::first_paint::hold_built(&window, activate, opened);
        }
    }
    Ok(())
}

// `focused = false` = 만들되 포커스를 가져가지 않는다(도킹 새 창의 `FocusPolicy::Keep` · 예비 창). `true` 면 숨은 동안에도
//   포커스가 이 창으로 간다(wry 가 만들자마자 웹뷰에 넣는다 — `state::placement` 의 main 짓기 주석) — 보이기 전과 같은
//   때다.
//
// ★늘 숨긴 채 짓는다 — 빌더에 `visible(true)` 를 주지 말 것(`crate::first_paint` 머리 — 첫 프레임의 비침)★.
fn build_window(
    app: &AppHandle,
    label: &str,
    at: Option<(LogicalPosition<f64>, LogicalSize<f64>)>,
    focused: bool,
) -> Result<WebviewWindow, String> {
    let ((x, y), (w, h)) = match at {
        Some((at, size)) => ((at.x, at.y), (size.width, size.height)),
        None => (cascade_position(label), POPOUT_DEFAULT_SIZE),
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
            .visible(false)
            .focused(focused),
        Backdrop::for_window(app, label),
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
// close_tab/close_window 경유 destroy) lib.rs Destroyed arm 이 이걸 부른다. titlebar close 는 닫기 요청
// ([`close_requested_popup_window`])이 모델을 먼저 지웠으므로 여기서는 재계산 · Channel 떼기만 남는다.
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

    // 1) 모델 정리 — ViewManager 락 한 번 안(아래 함수). 닫기 요청이 먼저 지웠으면 0.
    let Some(dropped_views) = drop_window_in_model(label, state, router, client) else {
        return;
    };

    // 2) 출력 Channel registry 에서 이 label 제거(누수 방지 — 죽은 webview Channel 이 남지 않게). Tauri
    //   부분이라 별도 락(ViewManager 무관) — 코어(모델·라우팅) 밖이라 락 밖 유지 OK(F1).
    //   ★poison 이어도 뺀다(ADR-0231)★ — 옛 건너뛰기는 죽은 Channel 을 남겼다(근거 정본 = `output_channel`
    //   모듈 헤더).
    crate::output_channel::unregister_window(registry, label);

    tracing::info!(
        label,
        dropped_views,
        "런타임 창 소멸 정리 완료(모델 몫 · Channel)"
    );
}

/// 팝아웃의 닫기 요청(`lib.rs` 의 `CloseRequested`) — 창 정리의 모델 몫([`drop_window_in_model`] · 묘비 포함)을
/// 소멸을 기다리지 않고 지금 한다. 뒤이은 `Destroyed` 정리([`cleanup_popup_window`])가 같은 몫을 다시 해도 바뀌는
/// 것은 없다. main 은 무동작이다(main 의 닫기 요청은 숨기기 — 부르는 쪽 갈래).
/// TRD S21-slot-docking §1-2 「닫히는 창」
pub(crate) fn close_requested_popup_window(
    label: &str,
    state: &LayoutState,
    router: &OutputRouter,
    client: &DaemonClient,
) {
    // 0 = 모델이 아직 그 창을 몰랐다(새 창 옮기기의 창 열기와 모델 확정 사이) — 묘비만 선다.
    if let Some(dropped_views) = drop_window_in_model(label, state, router, client) {
        tracing::info!(
            label,
            dropped_views,
            "팝아웃 닫기 요청 — 모델에서 먼저 지우고 label 에 묘비를 단다"
        );
    }
}

// 팝아웃 창 정리의 모델 몫 — 창의 모든 탭 View 드롭 + windows 엔트리 제거 + 라우팅 표 재계산 + 구독 정리 발화 +
// 사용량 관심 재계산 + label 묘비, ★전부 같은 락 안★. 닫기 요청과 `Destroyed` 가 둘 다 부르고 두 번째는 바뀌는
// 것이 없다(창이 이미 없으면 재계산만 · 묘비는 집합). 답 = 모델에서 지운 탭 수(창이 이미 없었으면 0) · `None` =
// 정리하지 않음(main · 락 오염). 창(`AppHandle`) 없이 서므로 셸 단위 시험이 이것을 직접 부른다.
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
) -> Option<usize> {
    // main 은 닫지 않는다(ADR-0057 ④) — 묘비가 서면 main 으로의 도킹이 막힌다.
    if label == MAIN_WINDOW_LABEL {
        return None;
    }
    let Ok(mut mgr) = state.0.lock() else {
        tracing::warn!(
            label,
            "팝아웃 창 정리(닫기 요청 · 소멸): lock poisoned — 정리 스킵"
        );
        return None;
    };
    let dropped_views = mgr.windows.get(label).map_or(0, |w| w.tabs.len());
    // 창이 이미 모델에서 지워졌으면 rebuild 만.
    let delta = crate::output_router::cleanup_window_core(&mut mgr, router, label);
    // 묘비는 모델 삭제와 같은 락 안 — 이 락 뒤의 도킹 · 새 창 확정은 그 창을 「없음」이나 묘비로 만나 무변경이다
    //   (TRD S21-slot-docking §1-2 「닫히는 창」 (c)).
    mgr.mark_closed(label);
    // 이 창이 마지막이던 agent 는 1→0 → Unsubscribe(락 안 발화 — F1).
    for a in delta.to_unsubscribe {
        client.unsubscribe(a);
    }
    // ★모델이 이 창을 쥐고 있었는지로 가르지 않는다(TRD S21 usage-limit-slot §1-7 「창 소멸」)★ — 숨김 표시는
    //   모델과 따로 선다: 새 창으로 슬롯 옮기기는 OS 창을 먼저 열고 모델 창을 나중에 더하므로, 그 틈에 숨겨졌다
    //   닫힌 창은 모델에 없어도 표시가 남는다. label 은 다시 쓰이지 않아(`PopupCounter`) 여기서 안 지우면 샌다.
    client.usage_window_destroyed(label, &mgr);
    Some(dropped_views)
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
        assert!(!is_popup_label("x-slot-popup-1"));
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

    // ── 맡긴 예열 창의 자리(TRD S21-slot-docking §3-9 「`open_at`(맡긴 label)」 · I18) ──

    const SCREEN: placement::MonitorArea = placement::MonitorArea {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
        scale: 1.0,
    };

    #[test]
    fn a_claimed_spare_opened_without_a_place_gets_the_cascade_and_the_default_size_again() {
        assert_eq!(
            spare_spot("slot-popup-3", None, &[SCREEN], 1.5),
            ((426, 345), POPOUT_DEFAULT_SIZE),
            "계단 자리(284, 230)를 그 창의 배율로 푼다"
        );
    }

    #[test]
    fn a_claimed_spare_lands_where_a_fresh_window_would() {
        let at = WindowPlace {
            pos: Some((300, 200)),
            size: Some((800.0, 600.0)),
        };
        assert_eq!(
            spare_spot("slot-popup-3", Some(&at), &[SCREEN], 2.0),
            ((300, 200), (800.0, 600.0)),
            "모니터에 걸친 자리는 물리 그대로 — 배율로 풀지 않는다"
        );
    }

    #[test]
    fn a_claimed_spare_whose_place_is_off_every_monitor_takes_the_cascade_and_the_asked_size() {
        let off = WindowPlace {
            pos: Some((-5000, -5000)),
            size: Some((640.0, 480.0)),
        };
        assert_eq!(
            spare_spot("slot-popup-1", Some(&off), &[SCREEN], 1.0),
            ((140, 110), (640.0, 480.0))
        );
        let unusable = WindowPlace {
            pos: None,
            size: Some((0.0, 480.0)),
        };
        assert_eq!(
            spare_spot("slot-popup-1", Some(&unusable), &[SCREEN], 1.0),
            ((140, 110), POPOUT_DEFAULT_SIZE),
            "쓸 수 없는 크기는 기본 크기"
        );
    }
}

// ── 테스트: 팝아웃 닫기 요청의 모델 몫(TRD S21-slot-docking §1-2 「닫히는 창」) — 창 없이 선다 ──────────
// 라우팅 표 · 구독 재동기는 운영과 같은 실물(`OutputRouter` · `RouterSubs`)이고 데몬에는 붙지 않는다(해제 발화는
// 창구가 닫혀 no-op — 발화 횟수는 `daemon_client` 시험이 잰다). `lib.rs` 의 창 사건 배선은 화면 실측 몫이다.
#[cfg(test)]
mod close_request_tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    use engram_dashboard_protocol::DaemonInfo;
    use tokio::runtime::Handle;

    use super::*;
    use crate::daemon_client::DaemonDiscovery;
    use crate::layout::dock::{DockAnchor, DockSide, DOCK_NEW_SHARE};
    use crate::layout::manager::{
        DockError, DockGuard, DockMove, DockTo, FocusPolicy, FrameMeasures,
    };
    use crate::layout::{
        tree, DockDest, DockRequest, LayoutError, LayoutEvents, OnEmptied, SlotContent,
        SubscriptionSync, ViewManager, ViewSnapshot, WindowBounds, WindowPlacement,
        WindowTabsPayload,
    };

    const POPUP: &str = "slot-popup-7";

    struct NoDaemon;

    impl DaemonDiscovery for NoDaemon {
        fn ensure_spawn(&self, _timeout: Duration) -> Result<DaemonInfo, String> {
            Err("이 시험은 연결하지 않는다".to_string())
        }
        fn read_live(&self) -> Option<DaemonInfo> {
            None
        }
    }

    struct NoEvents;

    impl LayoutEvents for NoEvents {
        fn layout_updated(&self, _snapshot: &ViewSnapshot) {}
        fn window_tabs_updated(&self, _tabs: &WindowTabsPayload) {}
        fn settled(&self) {}
    }

    struct Rig {
        state: LayoutState,
        router: Arc<OutputRouter>,
        client: Arc<DaemonClient>,
    }

    impl Rig {
        fn new() -> Self {
            Rig {
                state: LayoutState::new(),
                router: Arc::new(OutputRouter::new()),
                client: Arc::new(DaemonClient::new(Handle::current(), Arc::new(NoDaemon))),
            }
        }

        fn subs(&self) -> RouterSubs<'_> {
            RouterSubs {
                router: &self.router,
                client: &self.client,
            }
        }

        // `lib.rs` 의 `CloseRequested` 팝아웃 갈래.
        fn close_requested(&self, label: &str) {
            close_requested_popup_window(label, &self.state, &self.router, &self.client);
        }

        // `lib.rs` 의 `Destroyed` 정리 중 모델 몫.
        fn destroyed(&self, label: &str) -> Option<usize> {
            drop_window_in_model(label, &self.state, &self.router, &self.client)
        }

        // 런타임 크래시 사본 수락의 모델 몫(`state::restore` — 팝아웃 없는 사본) — 지금 팝아웃은 항목째 빠진다.
        fn adopt_crash_copy(&self) {
            let mut mgr = self.state.0.lock().unwrap();
            mgr.adopt_restored(ViewManager::new()).unwrap();
            self.subs().resync(&mgr);
        }

        // `window` 에 탭 하나를 더한다(없으면 그 창째 만든다).
        fn add_tab(&self, window: &str) -> Uuid {
            let mut mgr = self.state.0.lock().unwrap();
            let view = if mgr.windows.contains_key(window) {
                mgr.create_tab(window, None).unwrap()
            } else {
                mgr.create_window(window).unwrap()
            };
            self.subs().resync(&mgr);
            view
        }

        fn main_tab(&self) -> Uuid {
            self.state.0.lock().unwrap().windows[MAIN_WINDOW_LABEL].active
        }

        fn first_slot(&self, view: Uuid) -> Uuid {
            tree::first_slot_id(&self.state.0.lock().unwrap().views[&view].layout)
        }

        // 탭 `view` 의 첫 슬롯에 `agent` 를 앉힌다 — 라우팅 표도 맞춘다.
        fn seat(&self, view: Uuid, agent: Uuid) {
            let slot = self.first_slot(view);
            let mut mgr = self.state.0.lock().unwrap();
            mgr.assign_agent(view, slot, agent.to_string()).unwrap();
            self.subs().resync(&mgr);
        }

        fn routed(&self, agent: Uuid) -> Vec<String> {
            self.router.targets(agent).to_vec()
        }

        fn fingerprint(&self) -> (u64, Vec<String>, HashMap<Uuid, String>) {
            let mgr = self.state.0.lock().unwrap();
            assert_owned(&mgr);
            let mut windows: Vec<_> = mgr.windows.keys().cloned().collect();
            windows.sort();
            (mgr.version, windows, mgr.view_owner.clone())
        }
    }

    // ADR-0057 ① ② — 창 없는 View(롤백이 못 거둔 임시 View 포함)가 남지 않았다.
    fn assert_owned(mgr: &ViewManager) {
        let tabs: usize = mgr.windows.values().map(|w| w.tabs.len()).sum();
        assert_eq!(mgr.views.len(), tabs, "어느 창의 탭도 아닌 View 가 남았다");
        for (label, w) in &mgr.windows {
            for view in &w.tabs {
                assert_eq!(mgr.view_owner.get(view), Some(label));
            }
        }
    }

    // pop-out 이 락을 놓은 틈 — 창을 확인(`is_open` → 열려 있음)하거나 연(`open`) 바로 뒤에 사용자가 그 창을 닫는다.
    //   `adopts_on_open` 이면 연 바로 뒤의 일이 닫기가 아니라 런타임 크래시 사본 수락이다.
    struct ClosingHost<'a> {
        rig: &'a Rig,
        adopts_on_open: bool,
        closed: Mutex<Vec<String>>,
        placed: Mutex<Vec<String>>,
        // 한 번 맡길 예열 창(TRD S21-slot-docking §8-1 ⑬).
        spare: Mutex<Option<SpareClaim>>,
        // 창 포트가 불린 차례 — `"claim:"` · `"open:"` · `"committed:"` · `"release:"` · `"close:"` · `"place:"` + label.
        journal: Mutex<Vec<String>>,
        // 확정 · 되물림이 돌려준 맡김표.
        returned: Mutex<Vec<SpareClaim>>,
    }

    impl<'a> ClosingHost<'a> {
        fn new(rig: &'a Rig) -> Self {
            ClosingHost {
                rig,
                adopts_on_open: false,
                closed: Mutex::new(Vec::new()),
                placed: Mutex::new(Vec::new()),
                spare: Mutex::new(None),
                journal: Mutex::new(Vec::new()),
                returned: Mutex::new(Vec::new()),
            }
        }

        fn adopting(rig: &'a Rig) -> Self {
            ClosingHost {
                adopts_on_open: true,
                ..ClosingHost::new(rig)
            }
        }

        fn with_spare(rig: &'a Rig, claim: SpareClaim) -> Self {
            ClosingHost {
                spare: Mutex::new(Some(claim)),
                ..ClosingHost::new(rig)
            }
        }

        fn note(&self, entry: String) {
            self.journal.lock().unwrap().push(entry);
        }

        fn user_closes(&self, label: &str) {
            assert!(
                self.rig.state.0.try_lock().is_ok(),
                "창 포트는 락 밖에서 불려야 한다 — 닫기 요청 처리기가 같은 락을 잡는다"
            );
            self.rig.close_requested(label);
        }
    }

    // 새 창 · 재사용 옮기기도 같은 틈이다 — 열거나 옮긴 바로 뒤에 닫힌다.
    impl WindowHost for ClosingHost<'_> {
        // ★모델에 들기 전이어야 한다★ — 새 창을 여는 세 자리가 모두 창을 먼저 열고 모델에 넣는다(TRD S21-slot-docking
        // §3-9 「자리별 차례」 · ADR-0299).
        fn open(&self, label: &str) -> Result<(), String> {
            let mgr = self
                .rig
                .state
                .0
                .try_lock()
                .expect("창 포트는 락 밖에서 불려야 한다");
            assert!(
                !mgr.windows.contains_key(label),
                "open({label}) 은 창이 모델에 들기 전에 불려야 한다"
            );
            drop(mgr);
            self.note(format!("open:{label}"));
            if self.adopts_on_open {
                self.rig.adopt_crash_copy();
            } else {
                self.user_closes(label);
            }
            Ok(())
        }
        fn open_at(&self, label: &str, _at: &WindowPlace, _activate: bool) -> Result<(), String> {
            self.open(label)
        }
        fn close(&self, label: &str) {
            assert!(
                self.rig.state.0.try_lock().is_ok(),
                "close 는 락 밖에서 불려야 한다 — destroy 의 `Destroyed` 처리기가 같은 락을 잡는다"
            );
            self.note(format!("close:{label}"));
            self.closed.lock().unwrap().push(label.to_string());
        }
        fn is_open(&self, label: &str) -> bool {
            self.user_closes(label);
            true
        }
        fn move_window(&self, label: &str, _at: &WindowPlace) -> Result<(), MoveFailed> {
            self.user_closes(label);
            Ok(())
        }
        fn record_placement(&self, label: &str) {
            self.note(format!("place:{label}"));
            self.placed.lock().unwrap().push(label.to_string());
        }
        fn claim_spare(&self) -> Option<SpareClaim> {
            let claim = self.spare.lock().unwrap().take()?;
            self.note(format!("claim:{}", claim.label));
            Some(claim)
        }
        fn committed(&self, claim: &SpareClaim) {
            self.note(format!("committed:{}", claim.label));
            self.returned.lock().unwrap().push(claim.clone());
        }
        fn release_spare(&self, claim: &SpareClaim) {
            assert!(
                self.rig.state.0.try_lock().is_ok(),
                "release_spare 는 락 밖에서 불려야 한다"
            );
            self.note(format!("release:{}", claim.label));
            self.returned.lock().unwrap().push(claim.clone());
        }
    }

    // 빈 새 창이 열린 바로 뒤 · 모델에 넣기 전에 사용자가 닫는다(`Destroyed` 정리도 같은 모델 몫 — 묘비) — 넣기가 묘비를
    //   만나 연 창을 거두고, OS 창 없는 팝아웃 항목이 모델 · `window.list` 에 남지 않는다(남으면 다음 부팅에 빈 팝업으로
    //   복원된다). TRD S21-slot-docking §8-1 ⑬ · §3-9 「자리별 차례」
    #[tokio::test]
    async fn a_close_before_the_insert_refuses_a_created_window_and_closes_it() {
        let rig = Rig::new();
        let host = ClosingHost::new(&rig);
        let before = rig.fingerprint();

        let fresh = "slot-popup-1";
        let err = apply::create_window(&rig.state, &rig.subs(), &host, &PopupCounter::default())
            .unwrap_err();
        assert_eq!(err, LayoutError::WindowClosed(fresh.into()).to_string());
        assert_eq!(*host.closed.lock().unwrap(), [fresh], "방금 연 창을 거둔다");
        assert!(
            host.placed.lock().unwrap().is_empty(),
            "모델에 없는 창의 자리는 적지 않는다"
        );
        assert_eq!(rig.fingerprint(), before, "모델 무변경");
        assert!(!apply::list_windows(&rig.state)
            .unwrap()
            .iter()
            .any(|w| w == fresh));

        // 그 창의 `Destroyed` 도 바꾸는 것이 없다.
        assert_eq!(
            rig.destroyed(fresh),
            Some(0),
            "모델이 그 창을 가진 적이 없다"
        );
        assert_eq!(rig.fingerprint(), before);
    }

    // 같은 틈의 예열 창 판 — 넣기가 묘비로 거절되면 맡긴 창은 닫지 않고 그 맡김표 그대로 되물린다(확정 · 자리 기록
    //   없음). TRD S21-slot-docking §8-1 ⑬
    #[tokio::test]
    async fn a_close_before_the_insert_refuses_a_prewarmed_window_and_gives_it_back() {
        let rig = Rig::new();
        let claim = SpareClaim {
            label: "slot-popup-4".into(),
            generation: 3,
        };
        let spare = claim.label.clone();
        let host = ClosingHost::with_spare(&rig, claim.clone());
        let before = rig.fingerprint();

        let err = apply::create_window(&rig.state, &rig.subs(), &host, &PopupCounter::default())
            .unwrap_err();

        assert_eq!(err, LayoutError::WindowClosed(spare.clone()).to_string());
        assert_eq!(
            *host.journal.lock().unwrap(),
            [
                format!("claim:{spare}"),
                format!("open:{spare}"),
                format!("release:{spare}"),
            ],
            "닫기 · 확정 · 자리 기록 없음"
        );
        assert_eq!(
            *host.returned.lock().unwrap(),
            [claim],
            "맡김표 그대로 한 번"
        );
        assert_eq!(rig.fingerprint(), before, "모델 무변경");
        assert!(!apply::list_windows(&rig.state)
            .unwrap()
            .iter()
            .any(|w| *w == spare));
    }

    // 빈 새 창이 열린 바로 뒤 · 모델에 넣기 전에 크래시 사본 수락이 창 묶음을 갈아끼운다 — 그 수락은 모델이 아직 모르는
    //   그 창을 지우지 않으므로(묘비도 없다) 넣기가 서고, 창은 남아 모델의 창이 된다(정한 결말 — 리뷰 2026-10-21).
    //   TRD S21-slot-docking §3-9 「자리별 차례」
    #[tokio::test]
    async fn a_crash_copy_adopted_during_the_open_keeps_the_created_window_in_the_model() {
        let rig = Rig::new();
        let old_popout = rig.add_tab(POPUP);
        let host = ClosingHost::adopting(&rig);

        let label =
            apply::create_window(&rig.state, &rig.subs(), &host, &PopupCounter::default()).unwrap();

        assert_eq!(label, "slot-popup-1");
        assert!(
            host.closed.lock().unwrap().is_empty(),
            "연 창을 거두지 않는다"
        );
        assert_eq!(*host.placed.lock().unwrap(), [label.as_str()]);
        let mgr = rig.state.0.lock().unwrap();
        assert_owned(&mgr);
        assert!(
            !mgr.windows.contains_key(POPUP),
            "수락이 옛 팝아웃을 지웠다"
        );
        assert!(!mgr.views.contains_key(&old_popout));
        let tabs = mgr.list_tabs(&label).expect("새 창은 모델의 창이다");
        assert_eq!(tabs.tabs.len(), 1, "빈 탭 하나");
        assert_eq!(mgr.owner_of(tabs.active), Some(&label));
    }

    #[tokio::test]
    async fn a_close_request_drops_the_popout_now_and_the_later_destroyed_changes_nothing() {
        let rig = Rig::new();
        let shared = Uuid::new_v4();
        let only_popout = Uuid::new_v4();
        rig.seat(rig.main_tab(), shared);
        let q1 = rig.add_tab(POPUP);
        rig.seat(q1, only_popout);
        let q2 = rig.add_tab(POPUP);
        rig.seat(q2, shared);
        assert_eq!(rig.routed(only_popout), [POPUP]);

        rig.close_requested(POPUP);
        let after = rig.fingerprint();
        {
            let mgr = rig.state.0.lock().unwrap();
            assert!(!mgr.windows.contains_key(POPUP));
            for q in [q1, q2] {
                assert!(!mgr.views.contains_key(&q), "그 창의 탭이 전부 빠진다");
            }
        }
        assert!(
            rig.routed(only_popout).is_empty(),
            "그 창에만 보이던 에이전트는 라우팅에서 빠진다"
        );
        assert_eq!(rig.routed(shared), [MAIN_WINDOW_LABEL]);

        // 뒤이은 `Destroyed` — 같은 몫을 다시 해도 바뀌는 것이 없다.
        assert_eq!(rig.destroyed(POPUP), Some(0), "닫기 요청이 이미 지웠다");
        assert_eq!(rig.fingerprint(), after);
        assert!(rig.routed(only_popout).is_empty());
        assert_eq!(rig.routed(shared), [MAIN_WINDOW_LABEL]);
    }

    #[tokio::test]
    async fn the_model_part_answers_how_many_tabs_it_dropped() {
        let rig = Rig::new();
        rig.add_tab(POPUP);
        rig.add_tab(POPUP);
        assert_eq!(rig.destroyed(POPUP), Some(2));
        assert_eq!(rig.destroyed(POPUP), Some(0), "두 번째는 지울 것이 없다");
        // 새 창 옮기기의 창 열기와 모델 확정 사이 — 모델이 아직 모르는 창에도 묘비가 선다.
        let fresh = "slot-popup-99";
        assert_eq!(rig.destroyed(fresh), Some(0));
        let main = rig.main_tab();
        let slot = rig.first_slot(main);
        let mut mgr = rig.state.0.lock().unwrap();
        let (tmp, _) = mgr.prepare_detached_view(main, slot, "tmp".into()).unwrap();
        assert_eq!(
            mgr.attach_view_as_new_window(fresh, tmp),
            Err(LayoutError::WindowClosed(fresh.into()))
        );
    }

    #[tokio::test]
    async fn after_a_close_request_docking_into_that_window_is_refused_unchanged() {
        let rig = Rig::new();
        let source = rig.add_tab(MAIN_WINDOW_LABEL);
        let q = rig.add_tab(POPUP);
        rig.close_requested(POPUP);

        let before = rig.fingerprint();
        let mut mgr = rig.state.0.lock().unwrap();
        let version = mgr.version;
        for (to, want) in [
            (
                DockTo::Seam {
                    target: q,
                    anchor: DockAnchor::Outer,
                    side: DockSide::Left,
                    share: DOCK_NEW_SHARE,
                },
                DockError::TabNotFound(q),
            ),
            (
                DockTo::TabBar {
                    window: POPUP.into(),
                    index: None,
                },
                DockError::WindowNotFound(POPUP.into()),
            ),
            (
                DockTo::NewWindow {
                    label: POPUP.into(),
                    seed: None,
                    measures: FrameMeasures::default(),
                },
                DockError::WindowClosed(POPUP.into()),
            ),
        ] {
            assert_eq!(
                mgr.dock(
                    source,
                    to,
                    &DockGuard::NONE,
                    FocusPolicy::Reveal,
                    DockMove::default()
                ),
                Err(want)
            );
            assert_eq!(mgr.version, version);
        }
        // 팝아웃(`slot.popout`)의 새 창 phase C 도 묘비 앞에서 멈춘다.
        let slot = tree::first_slot_id(&mgr.views[&source].layout);
        let (tmp, _) = mgr
            .prepare_detached_view(source, slot, "tmp".into())
            .unwrap();
        assert_eq!(
            mgr.attach_view_as_new_window(POPUP, tmp),
            Err(LayoutError::WindowClosed(POPUP.into()))
        );
        mgr.drop_detached_view(tmp);
        drop(mgr);
        // 임시 View 를 만들고 거둔 만큼 `version` 은 오른다 — 창 · 탭 소유만 본다.
        let after = rig.fingerprint();
        assert_eq!((after.1, after.2), (before.1, before.2));
    }

    #[tokio::test]
    async fn a_close_request_after_the_check_rolls_a_popout_into_that_window_back() {
        let rig = Rig::new();
        let agent = Uuid::new_v4();
        let src = rig.main_tab();
        rig.seat(src, agent);
        let slot = rig.first_slot(src);
        rig.add_tab(POPUP);
        let host = ClosingHost::new(&rig);

        let out = apply::move_slot_to_window(
            &rig.state,
            &rig.subs(),
            &NoEvents,
            &host,
            &PopupCounter::default(),
            src,
            slot,
            Some(POPUP.into()),
        );
        assert!(out.is_err(), "{out:?}");
        {
            let mgr = rig.state.0.lock().unwrap();
            assert_owned(&mgr);
            assert!(!mgr.windows.contains_key(POPUP), "닫힌 창이 모델에 남았다");
            assert_eq!(
                mgr.slot_content(src, slot),
                Ok(SlotContent::Agent {
                    agent_id: agent.to_string()
                }),
                "옮기려던 슬롯은 출처에 남는다"
            );
        }
        assert!(
            host.closed.lock().unwrap().is_empty(),
            "기존 창 타깃은 창을 닫지 않는다"
        );
        assert_eq!(rig.routed(agent), [MAIN_WINDOW_LABEL]);
    }

    #[tokio::test]
    async fn a_close_request_after_the_build_rolls_a_popout_into_a_new_window_back() {
        let rig = Rig::new();
        let agent = Uuid::new_v4();
        let src = rig.main_tab();
        rig.seat(src, agent);
        let slot = rig.first_slot(src);
        let labels = PopupCounter::default();
        let fresh = "slot-popup-1";
        let host = ClosingHost::new(&rig);

        let out = apply::move_slot_to_window(
            &rig.state,
            &rig.subs(),
            &NoEvents,
            &host,
            &labels,
            src,
            slot,
            None,
        );
        assert!(out.is_err(), "{out:?}");
        let after = rig.fingerprint();
        assert!(
            !after.1.iter().any(|w| w == fresh),
            "OS 창 없는 모델 창이 남았다"
        );
        assert_eq!(
            rig.state.0.lock().unwrap().slot_content(src, slot),
            Ok(SlotContent::Agent {
                agent_id: agent.to_string()
            }),
            "옮기려던 슬롯은 출처에 남는다"
        );
        assert_eq!(*host.closed.lock().unwrap(), [fresh], "방금 연 창을 거둔다");
        assert!(
            host.placed.lock().unwrap().is_empty(),
            "모델에 없는 창의 자리는 적지 않는다"
        );
        assert_eq!(rig.routed(agent), [MAIN_WINDOW_LABEL]);

        // 그 창의 `Destroyed` 도 바꾸는 것이 없다.
        assert_eq!(
            rig.destroyed(fresh),
            Some(0),
            "모델이 그 창을 가진 적이 없다"
        );
        assert_eq!(rig.fingerprint(), after);
    }

    #[tokio::test]
    async fn a_close_request_for_main_changes_nothing_and_main_still_takes_tabs() {
        let rig = Rig::new();
        let q = rig.add_tab(POPUP);
        rig.add_tab(POPUP);
        let before = rig.fingerprint();

        rig.close_requested(MAIN_WINDOW_LABEL);
        assert_eq!(
            rig.destroyed(MAIN_WINDOW_LABEL),
            None,
            "main 은 정리하지 않는다"
        );
        assert_eq!(rig.fingerprint(), before);
        // main 에 묘비가 서지 않았다.
        rig.state
            .0
            .lock()
            .unwrap()
            .dock(
                q,
                DockTo::TabBar {
                    window: MAIN_WINDOW_LABEL.into(),
                    index: None,
                },
                &DockGuard::NONE,
                FocusPolicy::Keep,
                DockMove::default(),
            )
            .unwrap();
    }

    fn dock_to_new_window(rig: &Rig, host: &ClosingHost, view: Uuid) -> Result<(), DockError> {
        apply::dock_tab(
            &rig.state,
            &rig.subs(),
            &NoEvents,
            host,
            &PopupCounter::default(),
            DockRequest {
                view,
                to: DockDest::NewWindow {
                    at: WindowPlace {
                        pos: Some((300, 200)),
                        size: None,
                    },
                },
                guard: DockGuard::NONE,
                focus: FocusPolicy::Reveal,
                mv: DockMove::default(),
                on_emptied: OnEmptied::Close,
            },
        )
        .map(drop)
    }

    // 도킹 새 창이 열린 바로 뒤 사용자가 닫는다 — phase C 가 묘비를 만나 연 창을 거두고 탭은 출처에 남는다.
    #[tokio::test]
    async fn a_close_request_before_the_attach_closes_the_new_dock_window_and_keeps_the_tab() {
        let rig = Rig::new();
        let agent = Uuid::new_v4();
        let src = rig.add_tab(MAIN_WINDOW_LABEL);
        rig.seat(src, agent);
        let host = ClosingHost::new(&rig);
        let before = rig.fingerprint();

        let fresh = "slot-popup-1";
        assert_eq!(
            dock_to_new_window(&rig, &host, src),
            Err(DockError::WindowClosed(fresh.into()))
        );
        assert_eq!(*host.closed.lock().unwrap(), [fresh], "방금 연 창을 거둔다");
        assert!(
            host.placed.lock().unwrap().is_empty(),
            "모델에 없는 창의 자리는 적지 않는다"
        );
        assert_eq!(rig.fingerprint(), before, "탭은 출처에");
        assert_eq!(rig.routed(agent), [MAIN_WINDOW_LABEL]);
    }

    // 재사용으로 옮긴 바로 뒤 사용자가 그 창을 닫는다 — 놓기는 그대로 끝나고(다시 판정하지 않는다) 모델은 닫기가 남긴
    // 그대로다. 연 창이 없으니 닫을 것도 없다.
    #[tokio::test]
    async fn a_close_request_during_a_reuse_move_lets_the_drop_finish() {
        let rig = Rig::new();
        let q = rig.add_tab(POPUP);
        rig.state
            .0
            .lock()
            .unwrap()
            .observe_window_placement(
                POPUP,
                WindowPlacement {
                    minimized: false,
                    maximized: false,
                    bounds: WindowBounds::new(10.0, 20.0, 640.0, 480.0),
                },
            )
            .unwrap();
        let host = ClosingHost::new(&rig);

        assert_eq!(dock_to_new_window(&rig, &host, q), Ok(()));
        assert!(host.closed.lock().unwrap().is_empty());
        let mgr = rig.state.0.lock().unwrap();
        assert_owned(&mgr);
        assert!(!mgr.windows.contains_key(POPUP), "닫기 요청이 지웠다");
    }
}
