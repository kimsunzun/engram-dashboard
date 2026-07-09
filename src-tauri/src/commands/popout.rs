//! 슬롯 팝업 분리(move_slot_to_window) + 빈 창 생성(create_window) invoke 핸들러 — 탭 소유 모델(ADR-0057).
//!
//! ★§5 LLM 제어 표면★: 사람 우클릭(window.__engramLayout.moveSlotToWindow)과 LLM 이 같은 command 를 흔든다.
//!
//! ## 무엇을 하나 (MOVE, not mirror)
//! 원본 슬롯의 agent 를 **새 탭**(새 창 or 지정 기존 창)으로 옮기고, 원본 슬롯을 원본 View 에서 제거한다.
//! agent 자체(데몬 프로세스)는 안 건드린다 — 순수 I/O 표시 표면만 이동(§5 손발/두뇌 분리).
//!
//! ## 라우팅은 일반 메커니즘(ADR-0046 — 하드코딩 whitelist 금지)
//! 새 탭이 그 창의 `tabs` 에 들어가면 OutputRouter.rebuild 가 그 창 label 로 새 View 의 agent 출력을
//! 라우팅한다(각 창 모든 탭 walk — ADR-0057). 라우팅 표는 label-불가지 HashMap 이라 동적 label 도 흡수.
//!
//! ## ★2-phase 롤백 + 기존창 타깃 orphan 방지(§5-3, G4)★
//! WebviewWindowBuilder 는 Windows sync command 에서 호출하면 데드락하므로 `async fn` + 락 밖 빌드다.
//! 락이 풀린 사이 대상 창이 소멸/동시 close 될 수 있어, **기존 창 타깃의 탭 삽입을 phase C 로 이연**하고
//! phase C 에서 `windows.contains_key(to_window)` 재검증 후에만 삽입한다(부재면 롤백). 새 창 타깃은 phase C
//! 에서 새 label 로 창 엔트리 생성(label = PopupCounter 단조라 재사용 충돌 없음). 소스 detach 는 still-ours
//! 가드로 2차 락에서 close.
//!
//! ## label 유일성 (load-bearing)
//! Tauri 창 label 은 재사용 금지(같은 label 재-build 는 에러). 공유 카운터(PopupCounter)로 단조 증가
//! label(`slot-popup-1`, `-2`, …)을 발급한다 — 창을 닫아도 카운터는 안 되돌린다. `create_window`(D-6)도
//! 같은 카운터/prefix 를 재사용한다(G8 — is_popup_label Destroyed 정리 게이트에 걸려야 누수 안 남).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

use crate::commands::layout::{emit_window_tabs, send_subscription_delta, WindowTabsPayload};
use crate::daemon_client::DaemonClient;
use crate::layout::{LayoutState, MAIN_WINDOW_LABEL};
use crate::output_router::OutputRouter;

/// 팝업/런타임 창 label prefix. capabilities/popup.json 의 `"slot-popup-*"` glob 과 짝(변경 시 양쪽 동기).
/// ★의미 확장(ADR-0057/G8)★: "팝업" → "런타임 창"(create_window 포함). prefix 값은 불변(Destroyed 정리
/// 게이트 is_popup_label 재사용 — 다른 label 이면 cleanup 스킵 → 라우팅/구독/Channel 누수).
const POPUP_LABEL_PREFIX: &str = "slot-popup-";

/// ★WebView2 환경 옵션 SSOT — tauri.conf.json 의 `additionalBrowserArgs` 와 문자-단위로 동일해야 한다★.
/// 근거(실측 확인 — ghost windows 버그): 같은 user-data 폴더를 공유하는 모든 WebView 창은 **동일한**
/// WebView2 환경 옵션(additionalBrowserArgs)을 써야 한다. config 창(main·agent-tree)은 이 인자를 주는데
/// 런타임 WebviewWindowBuilder 가 안 주면 환경 옵션 불일치 → 같은 user-data 폴더의 런타임 WebView2 환경
/// 생성이 조용히 실패(build() 는 Ok·창 등록됨·HWND 없음 = 유령 창)한다. 결정·불변식 정본 = ADR-0054.
const WEBVIEW2_BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI --autoplay-policy=no-user-gesture-required";

/// 팝업/런타임 창 label 발급용 단조 카운터. app-level 공유(app.manage). ★재사용 금지 불변식★: fetch_add
/// 로 단조 증가만 하고 창을 닫아도 되돌리지 않는다(닫힌 label 재-build 에러 회피). AtomicU64 라 락 없이 안전.
#[derive(Default)]
pub struct PopupCounter(pub AtomicU64);

impl PopupCounter {
    /// 다음 유일 label 발급(`slot-popup-N`). N 은 1 부터 단조 증가.
    fn next_label(&self) -> String {
        let n = self.0.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{POPUP_LABEL_PREFIX}{n}")
    }
}

/// 이 label 이 팝업/런타임 창인지(prefix 매칭). lib.rs Destroyed arm 이 main/agent-tree 와 구분하는 데 쓴다.
pub fn is_popup_label(label: &str) -> bool {
    label.starts_with(POPUP_LABEL_PREFIX)
}

/// 런타임 창 URL 을 만든다. ★URL 키 = `?window=<label>`(ADR-0057/§3-3)★: 팝업 페이지는 "고정 뷰"가
/// 아니라 "이 창의 활성 탭"을 그린다(활성 탭은 백엔드 `windows[label].active` 가 권위). 프론트는 이 슬라이스
/// 밖(스테이지 4)이라 stale 하지만, Rust 측 일관성 위해 새 키로 발급한다.
fn window_url(label: &str) -> String {
    format!("index.html#/popup?window={label}")
}

/// 대각 cascade 위치(창이 겹쳐 뜨는 것 방지). label 순번 N 으로 오프셋. 8개마다 wrap.
fn cascade_position(label: &str) -> (f64, f64) {
    let n: u32 = label
        .strip_prefix(POPUP_LABEL_PREFIX)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let step = (n.saturating_sub(1) % 8) as f64;
    (140.0 + step * 72.0, 110.0 + step * 60.0)
}

/// WebviewWindowBuilder 로 런타임 창을 빌드(★락 밖에서만 호출 — 데드락 회피★). config 창과 동일한
/// WebView2 환경 옵션 필수(ghost windows 버그, ADR-0054). 실패 시 Err(빌드 문자열).
fn build_runtime_window(app: &AppHandle, label: &str) -> Result<(), String> {
    let (x, y) = cascade_position(label);
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(window_url(label).into()))
        .title(format!("Engram — {label}"))
        .inner_size(720.0, 500.0)
        .position(x, y)
        .additional_browser_args(WEBVIEW2_BROWSER_ARGS)
        .build()
        .map(|_| ())
        .map_err(|e| format!("런타임 창 생성 실패: {e}"))
}

/// OS 창을 destroy(닫기). Destroyed 이벤트 → lib.rs Destroyed arm → cleanup_popup_window 가 잔여 정리.
/// ★창 닫힘 = 백엔드 단일 소스(§5-2/G2)★: 프론트로 별도 view:closed 를 안 쏜다(이중 발화·재진입 방지).
/// registry 는 여기선 안 건드린다(Destroyed→cleanup 이 정리) — 그래서 인자로도 안 받는다(F5).
pub fn destroy_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        if let Err(e) = w.destroy() {
            tracing::warn!(label, "destroy_window 실패(창 이미 닫힘일 수 있음): {e}");
        }
    } else {
        // 창이 이미 없음(경합) — no-op. 모델은 이미 정리됨.
        tracing::debug!(label, "destroy_window: OS 창 없음(이미 닫힘) — no-op");
    }
}

/// ★빈 새 창 생성(create_window — D-6)★. 모델에 빈 창(빈 탭 1개) 추가 → 락 밖에서 웹뷰 빌드. 성공 시
/// 새 창 label 반환. 빌드 실패 시 모델 롤백(close_window). command wrapper 는 commands/layout.rs.
///
/// ★create_window count 노트(ADR-0056/§4-2)★: 창 수를 늘리므로 보이는 슬롯 상한(≤16) 근접을 로그로 남긴다
/// (하드 블록 아님 — 초과 레이아웃은 프론트 onContextLoss→DOM graceful degrade + ADR-0056 재검토 트리거).
pub async fn create_empty_window(
    app: &AppHandle,
    state: &LayoutState,
    router: &Arc<OutputRouter>,
    counter: &Arc<PopupCounter>,
    client: &Arc<DaemonClient>,
) -> Result<String, String> {
    let label = counter.next_label();

    // ── phase A(락): 모델에 빈 창 엔트리 생성(빈 탭 1개) ──────────────────────────
    {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.create_window(&label).map_err(|e| e.to_string())?;
        // 빈 슬롯뿐이라 라우팅 델타는 없지만 계약상 rebuild(표 재계산).
        let delta = router.rebuild(&mgr);
        send_subscription_delta(client, delta);
        // ADR-0056 상한 근접 로그(하드 블록 아님).
        let n_windows = mgr.windows.len();
        if n_windows >= 3 {
            tracing::info!(
                windows = n_windows,
                "create_window: 창 수 증가 — 보이는 슬롯 상한(≤16, ADR-0056) 근접 주의"
            );
        }
    } // ← 락 드롭

    // ── phase B(락 밖): 웹뷰 빌드(WebviewWindowBuilder 데드락 회피) ────────────────
    if let Err(e) = build_runtime_window(app, &label) {
        // 빌드 실패 → 모델 롤백(방금 만든 빈 창 제거).
        let delta = {
            let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
            let _ = mgr.close_window(&label);
            router.rebuild(&mgr)
        };
        send_subscription_delta(client, delta);
        return Err(e);
    }

    // 창 mount 시 프론트가 list_tabs(label) pull + window:tabs-updated listen 으로 초기 렌더(§3-3).
    // 별도 emit 은 불필요(read-only pull 로 자기 창 활성 탭 확정). 진단 로그만.
    tracing::info!(label = %label, "빈 새 창 생성 완료(create_window)");
    Ok(label)
}

/// ★슬롯을 다른 창의 새 탭으로 MOVE(move_slot_to_window)★. §5-3 2-phase 롤백(G4).
///   `to_window` 지정 → 그 기존 창 새 탭으로(phase C 삽입·재검증). 미지정 → 새 팝업 창 생성.
/// 반환 = `{ window, tab }`(호출자가 옮겨간 창·탭을 안다, G4). 빈 슬롯이면 Err.
///
/// ★async fn 필수★: WebviewWindowBuilder 데드락 회피(새 창 타깃은 phase B 에서 빌드).
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
) -> Result<MoveResult, String> {
    // 새 창 타깃이면 label 을 미리 발급(phase B 빌드에 필요). 기존 창 타깃이면 그 label.
    let is_new_window = to_window.is_none();
    let target_label = to_window.clone().unwrap_or_else(|| counter.next_label());

    // ── phase A(락): 소스 agent → 임시 View(아직 어느 창 tabs 에도 안 넣음 — orphan 방지) ──────────
    // ★agent_id 를 락 밖으로 반출★(MOVE 원자성): 창 build 로 락이 풀린 사이 원본 슬롯이 다른 agent 로
    //   재배정될 수 있다 — 2차 락에서 close 전에 이 값과 재조회 결과를 대조해 "옮긴 그 agent 그대로일 때만"
    //   원본을 닫는다(엉뚱한 agent 삭제 방지).
    //
    // ★owner-less tmp_view 가 phase B(언락) 동안 views 에 있어도 안전한 이유(F3 — BLOCK-1 해소)★:
    //   prepare_detached_view 가 만든 tmp_view 는 `views` 에는 있으나 `view_owner`/`windows[*].tabs`
    //   어디에도 없다(불변식 1·2 의 "모든 View 는 owner 1개"를 phase B 동안 일시 위배). 그럼에도 이 상태는
    //   안전하다: ① 이 view id 는 이 op 만 손에 쥔다(다른 command 는 uuid 를 모르니 건드릴 수 없음) ② 어느
    //   창 tabs 에도 없어 rebuild 라우팅 순회(창→tabs walk)에 안 걸린다(구독/출력 영향 0) ③ 소스 agent 는
    //   소스 슬롯이 아직 살아있어 그 경유로 계속 표시된다(사용자 화면 손실 없음) ④ 종점은 항상 둘 중 하나 —
    //   phase C attach(owner 부여 → 불변식 복구) 또는 rollback drop_detached_view(views 에서도 제거).
    //   ⚠️ 다음 세션 주의: rebuild/정리 로직에 "views 전체를 순회하며 owner 를 요구/가정"하는 코드를 넣을
    //   때는 이 일시 owner-less 창(phase B)을 전제로 깔아야 한다(무조건 owner 있음 가정 = 이 op 중 패닉).
    let (tmp_view, agent_id) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        // ① 원본 슬롯 agent 읽기(빈 슬롯이면 거부).
        let agent_id = mgr
            .slot_agent(view_id, slot_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "빈 슬롯은 다른 창으로 옮길 수 없음(agent 미배정)".to_string())?;
        // ② 임시 View 생성(agent 담김, 창 미배정 — phase C 에서 삽입).
        let name = if is_new_window {
            format!(
                "Popup {}",
                &target_label[POPUP_LABEL_PREFIX.len().min(target_label.len())..]
            )
        } else {
            "Tab".to_string()
        };
        let tmp_view = mgr
            .prepare_detached_view(view_id, slot_id, name)
            .map_err(|e| e.to_string())?;
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta);
        (tmp_view, agent_id)
    }; // ← 락 드롭

    // ── phase B(락 밖): 새 창 타깃이면 웹뷰 빌드 / 기존 창 타깃이면 존재 확인만 ─────────────────
    if is_new_window {
        if let Err(e) = build_runtime_window(&app, &target_label) {
            rollback_detached(&state, &router, &client, tmp_view);
            return Err(e);
        }
    } else {
        // 기존 창 타깃: 창이 실제 존재하는지 확인(부재면 롤백). 빌드 없음.
        if app.get_webview_window(&target_label).is_none() {
            rollback_detached(&state, &router, &client, tmp_view);
            return Err(format!("대상 창 없음: {target_label}"));
        }
    }

    // ── phase C(락): 임시 View 를 타깃 창 탭으로 삽입(★기존창 재검증★) + 소스 슬롯 close ───────────
    let (src_tabs, tgt_tabs, src_layout) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;

        // 삽입: 새 창 = 창 엔트리 생성 / 기존 창 = windows.contains_key 재검증 후 tabs 삽입(G4).
        let inserted = if is_new_window {
            mgr.attach_view_as_new_window(&target_label, tmp_view)
        } else {
            // ★재검증(G4)★: phase B 언락 중 대상 창이 소멸/동시 close 됐을 수 있음 → 존재할 때만 삽입.
            mgr.insert_tab_into(&target_label, tmp_view)
        };
        if let Err(e) = inserted {
            // 삽입 실패(재검증 실패 등) → 임시 View 롤백 + (새 창이면) 이미 뜬 창 destroy. 소스 유지.
            // ★F7 nit — 이 롤백은 실질적으로 기존 창 insert_tab_into 실패(phase B 언락 중 대상 창 소멸)만
            //   가드한다★: 새 창 경로(is_new_window)의 attach_view_as_new_window 는 fresh label(PopupCounter
            //   단조 — 재사용 충돌 없음) + 방금 만든 tmp_view 에 대해 실패 불가라 사실상 dead 분기다. 그래도
            //   is_new_window 일 때 destroy 를 남기는 건 방어(미래에 attach 가 실패 가능해질 경우 유령 창 방지).
            let _ = mgr.drop_detached_view(tmp_view);
            let delta = router.rebuild(&mgr);
            // ★F1/F2 일관★: 롤백 델타의 to_unsubscribe 발화도 락 안(drop 전). registry 는 안 건드리니
            //   destroy_window(OS)만 락 밖으로.
            send_subscription_delta(&client, delta);
            drop(mgr);
            if is_new_window {
                destroy_window(&app, &target_label);
            }
            return Err(format!("탭 삽입 실패(롤백): {e}"));
        }

        // 소스 슬롯 close(MOVE 완성 — still-ours 가드). 창 build 로 락이 풀린 사이 재배정됐으면 스킵.
        // ★F4 — MOVE→COPY 열화는 의도된 best-effort★: phase B(언락) 동안 소스 슬롯이 다른 agent 로
        //   재배정되면(Ok(Some(other))) still_ours=false → close 스킵. 즉 "재배정된 엉뚱한 agent 를 지우지
        //   않는 것"이 최우선이고, 그 대가로 원래 agent 가 타깃 탭 + 소스 슬롯 양쪽에 남는다(MOVE 가 사실상
        //   COPY 로 열화). 이 중복은 불변식 5(같은 agent 두 View 허용, 진도 독립·ADR-0046)로 무해하므로
        //   엄격 롤백(타깃 되돌리기) 대신 이대로 둔다.
        // ★load-bearing★: 소스 View 자체가 gap 중 소멸(탭/창 닫힘)했으면 slot_agent 가 `Err`(ViewNotFound/
        //   SlotNotFound)를 준다 → `matches!(_, Ok(Some(..)))` 가 실패 → still_ours=false → close 스킵.
        //   이 `Err→스킵`이 이미-사라진 소스를 다시 close 하려다 나는 오작동/패닉을 막는다(수정 금지).
        let still_ours = matches!(
            mgr.slot_agent(view_id, slot_id),
            Ok(Some(ref a)) if *a == agent_id
        );
        if still_ours {
            let _ = mgr.close_slot(view_id, slot_id);
        } else {
            tracing::warn!(
                view = %view_id, slot = %slot_id, agent = %agent_id,
                "원본 슬롯이 창 생성 중 재배정/제거됨 — MOVE 의 close 스킵(대상 탭은 그대로 유지)"
            );
        }

        // 양 창 탭바 + 소스 View 레이아웃 페이로드(락 안 복사).
        let src_owner = mgr.owner_of(view_id).cloned();
        let src_tabs = src_owner
            .as_deref()
            .and_then(|l| mgr.list_tabs(l).ok())
            .map(WindowTabsPayload::from);
        let tgt_tabs = mgr
            .list_tabs(&target_label)
            .ok()
            .map(WindowTabsPayload::from);
        let src_layout = mgr.snapshot(view_id).ok();

        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta);
        (src_tabs, tgt_tabs, src_layout)
    }; // ← 락 드롭

    // emit(락 밖, ADR-0006): 소스 View 레이아웃 + 양 창 탭바.
    if let Some(snap) = src_layout {
        if let Err(e) = app.emit_layout(&snap) {
            tracing::warn!("[move_slot] layout:updated emit 실패: {e}");
        }
    }
    if let Some(t) = &src_tabs {
        emit_window_tabs(&app, t);
    }
    if let Some(t) = &tgt_tabs {
        emit_window_tabs(&app, t);
    }

    tracing::info!(window = %target_label, view = %tmp_view, "슬롯 MOVE 완료(detach)");
    Ok(MoveResult {
        window: target_label,
        tab: tmp_view,
    })
}

/// move_slot_to_window 반환(G4) — 옮겨간 창 label + 새 탭 View id.
#[derive(serde::Serialize, Clone)]
pub struct MoveResult {
    pub window: String,
    pub tab: Uuid,
}

/// phase A 임시 View 롤백(창 삽입 전이라 tabs 갱신 불필요). 소스 슬롯은 유지(사용자가 슬롯 안 잃음).
///
/// ★F2 REAL 동시성 버그 수정★: 옛 코드는 rebuild 델타를 락 안에서 계산하고 `send_subscription_delta`
///   발화를 `drop(mgr)` 뒤(락 밖)에 했다 → F1 과 같은 클래스(계산~발화 사이 재추가로 stale 1→0
///   unsubscribe). 이제 발화도 락 안(drop 전) — send_subscription_delta 는 동기 try_send(await/network 0)라
///   ADR-0006 위반 아님. tmp_view 는 orphan(view_owner 없음)이라 이 rebuild 델타에 to_subscribe 는 안 나오고
///   (라우팅 순회는 windows→tabs walk 인데 tmp_view 는 어느 tabs 에도 없음), drop 으로 to_unsubscribe 가
///   나올 수 있어(0→1 이 애초에 안 나갔으니 대개 no-op) 발화를 락 안으로 옮기는 게 안전.
fn rollback_detached(
    state: &LayoutState,
    router: &Arc<OutputRouter>,
    client: &Arc<DaemonClient>,
    tmp_view: Uuid,
) {
    let Ok(mut mgr) = state.0.lock() else {
        tracing::warn!("rollback_detached: lock poisoned — 롤백 스킵");
        return;
    };
    mgr.drop_detached_view(tmp_view);
    let delta = router.rebuild(&mgr);
    // ★락 안 발화(F2)★ — F1 과 동일 이유. drop(mgr) 은 이 스코프 끝에서 자동.
    send_subscription_delta(client, delta);
}

// AppHandle 확장(layout:updated emit 을 popout 에서도 — commands/layout.rs 상수 재정의 회피).
trait EmitLayout {
    fn emit_layout(&self, snap: &crate::layout::ViewSnapshot) -> tauri::Result<()>;
}
impl EmitLayout for AppHandle {
    fn emit_layout(&self, snap: &crate::layout::ViewSnapshot) -> tauri::Result<()> {
        use tauri::Emitter;
        self.emit("layout:updated", snap)
    }
}

/// ★창 Destroyed 정리(수명/누수 임계 — 멀티탭, G1)★. 팝업/런타임 창이 닫히면(titlebar close·강제 destroy·
/// close_tab/close_window 경유 destroy) lib.rs Destroyed arm 이 이걸 부른다. 그 창의 **모든 탭 View 를
/// 통째로 드롭**(views + view_owner) + windows 엔트리 제거 → rebuild **1회** → 그 델타의 to_unsubscribe
/// (어느 창에도 안 남은 agent)를 데몬에 발화 → 출력 Channel 제거.
///
/// ★현 버그 수정(G1)★: 옛 코드는 단일 바인딩 하나만 정리해 멀티탭 팝업을 강제 종료하면 나머지 탭이 잔류
/// + Unsubscribe 누락. 이제 Tauri-free 코어 `cleanup_window_core`(output_router.rs)가 close_window 로 tabs
/// 전부 순회 드롭 + rebuild(마지막 1회 — 락 1구간)해 델타를 반환하고, 이 핸들러가 그 델타의 to_unsubscribe
/// 를 **락 안에서** 발화한다(F1 — 발화를 락 밖으로 미루면 재추가로 stale 1→0 이 라이브 구독을 죽인다).
/// 코어(모델·라우팅)는 headless 단독 테스트 가능(G1 필수, TRD §8 스테이지1) — Tauri 부분(registry.remove)만
/// 이 핸들러에 남는다.
///
/// ★이 함수는 command 가 아니다★ — Rust 이벤트 핸들러(on_window_event)에서 직접 호출. State 대신 이미
/// 손에 쥔 Arc 참조들을 인자로 받는다(lib.rs 가 app.state 로 꺼내 넘김). `_app` 은 현재 미사용(향후 즉시
/// emit 여지로 시그니처 통일).
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

    // 1) 창의 모든 탭 View 드롭 + windows 엔트리 제거 + 라우팅 표 재계산 + 구독 정리 발화 — ★전부 같은 락 안★.
    //   ★F1 REAL 동시성 버그 수정★: 옛 코드는 델타(cleanup_window_core rebuild)를 락 안에서 계산하고
    //   `to_unsubscribe` 발화를 락 드롭 뒤에 했다 → 계산~발화 사이 다른 command(assign_agent/spawn/move)가
    //   그 agent 를 재추가하면 stale 1→0 unsubscribe 가 방금 형성된 라이브 구독을 죽인다(ADR-0006 §5-1
    //   "델타 enqueue 는 락 안" 위반). 이제 create_empty_window/move_slot_to_window 가 이미 락 안에서
    //   send_subscription_delta 하는 것과 일관되게, 발화도 락 안(unsubscribe 는 동기 try_send — await/network
    //   0, lifecycle 락 독립 → 데드락 없음).
    {
        let Ok(mut mgr) = state.0.lock() else {
            tracing::warn!(label, "cleanup_popup_window: lock poisoned — 정리 스킵");
            return;
        };
        // Tauri-free 코어(G1 멀티탭 드롭 + rebuild 델타). 창이 이미 모델에서 지워졌으면 rebuild 만.
        let delta = crate::output_router::cleanup_window_core(&mut mgr, router, label);
        // 이 창이 마지막이던 agent 는 1→0 → Unsubscribe(락 안 발화 — F1). ADR-0046: to_unsubscribe 만 wire.
        for a in delta.to_unsubscribe {
            client.unsubscribe(a);
        }
    } // ← 락 드롭

    // 2) 출력 Channel registry 에서 이 label 제거(누수 방지 — 죽은 webview Channel 이 남지 않게). Tauri
    //   부분이라 별도 락(ViewManager 무관) — 코어(모델·라우팅) 밖이라 락 밖 유지 OK(F1).
    if let Ok(mut reg) = registry.lock() {
        reg.remove(label);
    } else {
        tracing::warn!(
            label,
            "cleanup_popup_window: registry lock poisoned — Channel 제거 스킵"
        );
    }

    tracing::info!(label, "런타임 창 정리 완료(탭 전부 드롭·구독·Channel)");
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
