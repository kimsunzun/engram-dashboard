//! 슬롯 팝업 분리(pop-out) invoke 핸들러 — 슬롯의 agent 를 런타임 생성 OS 창으로 **이동(detach)** 한다.
//!
//! ★§5 LLM 제어 표면★: 사람 우클릭(window.__engramLayout.popOutSlot)과 LLM 이 같은 command 를 흔든다.
//!
//! ## 무엇을 하나 (MOVE, not mirror)
//! 원본 슬롯의 agent 를 **새 View** 로 옮기고, 그 View 를 **새로 만든 팝업 창**에 바인딩한 뒤, 원본
//! 슬롯을 메인에서 제거한다. agent 자체(데몬 프로세스)는 안 건드린다 — 순수 I/O 표시 표면만 이동(§5).
//!
//! ## 라우팅은 일반 메커니즘 재사용(ADR-0046 — 하드코딩 whitelist 금지)
//! 팝업 창 label 을 `window_bindings` 에 넣으면 OutputRouter.rebuild 가 그 label 로 새 View 의 agent
//! 출력을 라우팅한다. 라우팅 표는 label-불가지 HashMap 이라 동적 label 도 바인딩만 되면 흡수된다 —
//! main/agent-tree 라우팅과 완전히 직교(ADR-0035: active_view_id 는 main 전용, 팝업은 binding 경유).
//!
//! ## ★async fn 필수 (load-bearing)★
//! `WebviewWindowBuilder::build()` 는 Windows 에서 **sync command** 안에서 호출하면 데드락한다
//! (docs.rs/tauri — 창 생성이 메인 스레드 이벤트 루프와 상호작용). 그래서 pop_out_slot 은 `async fn`.
//!
//! ## label 유일성 (load-bearing)
//! Tauri 창 label 은 재사용 금지(같은 label 재-build 는 에러). 그래서 공유 카운터(PopupCounter)로 단조
//! 증가 label(`slot-popup-1`, `-2`, …)을 발급한다 — 창을 닫아도 카운터는 안 되돌린다(닫힌 창 label 재사용 X).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

use crate::daemon_client::DaemonClient;
use crate::layout::LayoutState;
use crate::output_router::OutputRouter;

/// 팝업 창 label prefix. capabilities/popup.json 의 `"slot-popup-*"` glob 과 짝(변경 시 양쪽 동기).
const POPUP_LABEL_PREFIX: &str = "slot-popup-";

/// ★WebView2 환경 옵션 SSOT — tauri.conf.json 의 `additionalBrowserArgs` 와 문자-단위로 동일해야 한다★.
/// 근거(실측 확인 — ghost windows 버그): 같은 user-data 폴더를 공유하는 모든 WebView 창은 **동일한**
/// WebView2 환경 옵션(additionalBrowserArgs)을 써야 한다. config 창(main·agent-tree)은 이 인자를 주는데
/// 런타임 WebviewWindowBuilder 가 안 주면 환경 옵션 불일치 → 같은 user-data 폴더의 런타임 WebView2 환경
/// 생성이 조용히 실패(build() 는 Ok·창 등록됨·HWND 없음 = 유령 창)한다. 그래서 런타임 창 생성자는 반드시
/// 이 상수를 config 문자열과 동일하게 쓴다. ★wry 의 더 넓은 기본값(추가로 msSmartScreenProtection 비활성)은
/// 금지★ — config 창과 새 불일치를 만들어 버그를 재발시킨다. tauri.conf.json 값 변경 시 여기도 함께 갱신할 것.
const WEBVIEW2_BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI --autoplay-policy=no-user-gesture-required";

/// `layout:updated` / `view:list-updated` 이벤트명(commands/layout.rs 와 동일 어휘 — 팝업 창도 같은
/// listen 경로로 자기 View 를 그린다). 문자열 상수는 여기 재정의(모듈 간 pub 노출 최소화).
const EVT_LAYOUT_UPDATED: &str = "layout:updated";
const EVT_VIEW_LIST_UPDATED: &str = "view:list-updated";

/// 팝업 창 label 발급용 단조 카운터. app-level 공유(app.manage) → 여러 pop_out_slot 호출이 같은 카운터를
/// 본다. ★재사용 금지 불변식★: fetch_add 로 단조 증가만 하고 창을 닫아도 되돌리지 않는다(닫힌 label 재-build
/// 에러 회피). AtomicU64 라 락 없이 동시 호출 안전.
#[derive(Default)]
pub struct PopupCounter(pub AtomicU64);

impl PopupCounter {
    /// 다음 유일 label 발급(`slot-popup-N`). N 은 1 부터 단조 증가.
    fn next_label(&self) -> String {
        let n = self.0.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{POPUP_LABEL_PREFIX}{n}")
    }
}

/// 슬롯을 팝업 창으로 분리(MOVE). detach 흐름:
///   ① 원본 슬롯 agent 읽기 → ② create_view(새 View) → ③ assign_agent(새 View 슬롯 ← agent)
///   → ④ 팝업 창 생성 + bind_window(창→새 View) → ⑤ close_slot(원본 메인 슬롯) → rebuild+emit.
///
/// ★async fn(위 모듈 주석)★: WebviewWindowBuilder 데드락 회피.
///
/// ★배정 검증 안 함(ADR-0035)★: assign 은 참조 문자열만 저장(데몬 실재 확인 0). 빈 슬롯(agent 없음)을
/// pop-out 하려 하면 Err — 프론트 메뉴가 이미 enabled 가드로 막지만 백엔드도 방어한다.
///
/// invalid view_id/slot_id → Err(no-op). 창 생성 실패 → Err(부분 상태 롤백은 아래 참조).
#[tauri::command]
pub async fn pop_out_slot(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    counter: State<'_, Arc<PopupCounter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    let label = counter.next_label();

    // ── 1차 락 구간: 원본 agent 읽기 → 새 View + assign + bind(창 생성 전) ─────────────────────
    // ★창 생성 전에 새 View 를 먼저 만든다★: 팝업 창이 mount 되면 곧장 `?view=<new_view>` 로 get_view 를
    //   pull 하므로, 창 build 시점엔 이미 새 View 가 존재해야 한다(create→build 순서 불변).
    // ★assign 이 원본 슬롯을 안 건드림★: 여기선 새 View 슬롯에만 agent 를 넣고, 원본 close 는 창 생성
    //   *성공 후* 별도 락 구간에서 한다 — 창 생성이 실패하면 원본이 그대로 남아 사용자가 슬롯을 잃지 않는다.
    // ★agent_id 를 락 밖으로 반출★(FIX-1 MOVE 원자성): 창 build 로 락이 풀린 사이 원본 슬롯이 다른 agent
    //   로 재배정될 수 있다 — 2차 락에서 close 전에 이 값과 재조회 결과를 대조해 "옮긴 그 agent 그대로일 때만"
    //   원본을 닫는다(엉뚱한 agent 삭제 방지). 그래서 여기서 (new_view_id, agent_id) 둘 다 반환한다.
    let (new_view_id, agent_id) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;

        // ① 원본 슬롯 agent 읽기(빈 슬롯이면 거부 — pop-out 대상 없음).
        let agent_id = mgr
            .slot_agent(view_id, slot_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "빈 슬롯은 팝업으로 분리할 수 없음(agent 미배정)".to_string())?;

        // ② 새 View 생성(빈 슬롯 하나). create_view 는 이 View 를 active 로 만든다 — 하지만 팝업은
        //    active(main) 가 아니라 window_bindings 로 라우팅되므로 active 여부는 팝업엔 무관하다.
        //    (원본 View 가 active 였다면 아래 close_slot 후에도 원본이 main 에 계속 그려지도록, 새 View 를
        //     active 로 둔 채 두지 않고 곧바로 원본 active 상태를 복원한다.)
        let prev_active = mgr.active_view_id;
        let new_view_id = mgr.create_view(Some(format!(
            "Popup {}",
            &label[POPUP_LABEL_PREFIX.len()..]
        )));
        // create_view 가 active 를 새 View 로 바꿨으니 원래 active 로 되돌린다(팝업은 main active 를 안 뺏음).
        // ★핵심(ADR-0035)★: 팝업 View 는 main 탭이 아니다 — active_view_id 는 main 창 전용 개념이라
        //   팝업 View 가 active 로 남으면 main 이 팝업 내용을 그려버린다. 원본 active 복원으로 직교 유지.
        let _ = mgr.switch_view(prev_active);

        // ③ 새 View 의 (유일) 슬롯에 원본 agent 배정.
        // ★F3-minor 가드★: assign_agent 가 실패하면(현재는 방금 만든 유효 View·슬롯이라 불가) 방금 만든
        //   new_view 가 agent 없이 고아로 남을 수 있다 — 지금은 harmless(assign 실패 경로 없음)라 주석만.
        //   assign 이 실낼 여지가 생기면 여기서 close_view(new_view_id) 롤백을 넣어야 한다.
        let new_slot = {
            let v = mgr
                .views
                .iter()
                .find(|v| v.id == new_view_id)
                .expect("방금 만든 View 존재");
            crate::layout::tree::first_slot_id(&v.layout)
        };
        // assign 엔 clone 을 넘기고, 원본 agent_id 는 2차 락 재검증(FIX-1)용으로 반출한다.
        mgr.assign_agent(new_view_id, new_slot, agent_id.clone())
            .map_err(|e| e.to_string())?;

        // ④-bind: 팝업 창 label 을 새 View 에 바인딩(창 생성 전에 미리 — 창 mount replay 가 곧장 라우팅되게).
        // ADR-0046 / ADR-0035
        mgr.bind_window(label.clone(), new_view_id)
            .map_err(|e| e.to_string())?;

        // 라우팅 표 재계산(락 안 — FIX-1). 새 agent 가 팝업 label 로 라우팅되도록. 델타 송신은 아래 락 밖.
        // agent 는 원본(main) 과 팝업 양쪽에 잠깐 보이므로(원본 close 전) 여기선 Unsubscribe 델타가 안 난다.
        let _delta = router.rebuild(&mgr);
        (new_view_id, agent_id)
    }; // ← 락 드롭

    // ── ④-build: 팝업 창 생성(★락 밖 — WebviewWindowBuilder 는 절대 락 보유 중 호출 금지, 데드락★) ──
    // URL 은 index.html#/popup?view=<new_view_id> — 프론트 HashRouter 의 /popup 라우트가 이 view 를 그린다.
    // dev(localhost:1420)에선 Tauri 가 WebviewUrl::App 을 devUrl 로 리라이트해 같은 React 앱을 로드한다.
    let url = format!("index.html#/popup?view={new_view_id}");
    // ★위치 cascade★: 위치를 지정하지 않으면 팝업이 매번 같은 기본 자리에 겹쳐 생성돼 여러 개를 띄워도
    //   화면상 1개처럼 보인다(+큰 main 창 뒤로 가려짐). label 순번 N 으로 대각 오프셋을 줘 서로 안 겹치게
    //   한다. 8개마다 wrap. FOLLOW-UP(범위 밖): 다중 모니터 좌표 클램프(화면 경계 밖으로 나가는 것 방지).
    let popup_n: u32 = label[POPUP_LABEL_PREFIX.len()..].parse().unwrap_or(1);
    let step = (popup_n.saturating_sub(1) % 8) as f64;
    let pos_x = 140.0 + step * 72.0;
    let pos_y = 110.0 + step * 60.0;
    // ★additional_browser_args 필수(ghost windows 버그 수정)★: config 창과 동일한 WebView2 환경 옵션을 줘야
    //   같은 user-data 폴더에서 런타임 WebView2 환경 생성이 성공한다(불일치 시 창이 유령으로 뜸). 상수 정본 =
    //   WEBVIEW2_BROWSER_ARGS(위 정의 — tauri.conf.json 과 동기).
    let build_result = WebviewWindowBuilder::new(&app, label.clone(), WebviewUrl::App(url.into()))
        .title(format!("Engram — {label}"))
        .inner_size(720.0, 500.0)
        .position(pos_x, pos_y)
        .additional_browser_args(WEBVIEW2_BROWSER_ARGS)
        .build();

    if let Err(e) = build_result {
        // ★부분 상태 롤백★: 창 생성 실패 시 방금 만든 View + 바인딩을 되돌린다(원본 슬롯은 아직 안 닫았으니
        //   그대로 안전). close_view 가 window_bindings 도 retain 으로 정리한다.
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.unbind_window(&label);
        let _ = mgr.close_view(new_view_id);
        let delta = router.rebuild(&mgr);
        drop(mgr);
        for a in delta.to_unsubscribe {
            client.unsubscribe(a);
        }
        return Err(format!("팝업 창 생성 실패: {e}"));
    }

    // ── 2차 락 구간: 원본 슬롯 제거(detach 완료) → rebuild → emit ─────────────────────────────
    // 창이 성공적으로 떴으니 이제 원본 메인 슬롯을 닫는다(MOVE 완성 — 원본에서 사라짐). 형제 승격/root 리셋은
    // close_slot 의 기존 의미(트리 규칙) 그대로.
    let (layout, list, delta) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        // ★FIX-1 MOVE 원자성 재검증★: 창 build 로 락이 풀린 사이 원본 슬롯이 *다른* agent 로 재배정됐을 수
        //   있다 — 그 상태로 close_slot 을 무조건 돌리면 엉뚱한 agent 를 화면에서 지운다(severe). 그래서 close
        //   직전에 원본 슬롯 agent 를 다시 읽어 "우리가 옮긴 그 agent 그대로일 때만" 닫는다. 재배정/제거됐으면
        //   close 를 스킵한다 — 팝업 창은 이미 그 agent 를 보여주고 있으므로 MOVE 의 표시 쪽은 성립한다.
        let still_ours = matches!(
            mgr.slot_agent(view_id, slot_id),
            Ok(Some(ref a)) if *a == agent_id
        );
        if still_ours {
            // 원본 슬롯 닫기(MOVE 완성). 이미 사라졌으면(경합) 무해하게 Err → 무시하고 계속(창은 이미 떴음).
            let _ = mgr.close_slot(view_id, slot_id);
        } else {
            // ★잔여★: 진짜 동시(같은 슬롯) pop-out 2건은 둘 다 close 전에 agent 를 읽어 같은 agent 를 보이는
            //   팝업 2개를 만들 수 있다 — 이 재검증은 *엉뚱한 agent 삭제*(severe)만 막고, 드문 동일-agent
            //   중복 팝업(milder)은 남는다(리뷰 범위 밖 — 완전 해소 시도 안 함).
            tracing::warn!(
                view = %view_id,
                slot = %slot_id,
                agent = %agent_id,
                "원본 슬롯이 창 생성 중 재배정/제거됨 — MOVE 의 close 스킵(팝업 창은 그대로 유지)"
            );
        }
        // 원본 View(닫은 뒤)와 새 View 양쪽 레이아웃이 바뀔 수 있다 — 원본 View 스냅샷을 main emit 용으로.
        let layout = mgr.snapshot(view_id).ok();
        let list = list_payload(&mgr);
        let delta = router.rebuild(&mgr);
        (layout, list, delta)
    }; // ← 락 드롭

    // 델타 송신(락 밖, ADR-0006): 원본만 보이던 agent 는 이제 팝업에도 보이므로 net Unsubscribe 는 없다.
    // 혹시 있으면(엣지) 정리. 0→1 Subscribe 는 layout 이 안 보냄 — 뷰 주도 request_replay 가 형성(ADR-0046).
    for a in delta.to_unsubscribe {
        client.unsubscribe(a);
    }

    // emit(락 밖, ADR-0006): 원본 View 레이아웃 갱신 + 탭 목록 갱신을 main 창에 반영.
    if let Some(snap) = layout {
        if let Err(e) = app.emit(EVT_LAYOUT_UPDATED, &snap) {
            tracing::warn!("[popout] {EVT_LAYOUT_UPDATED} emit 실패: {e}");
        }
    }
    if let Err(e) = app.emit(EVT_VIEW_LIST_UPDATED, &list) {
        tracing::warn!("[popout] {EVT_VIEW_LIST_UPDATED} emit 실패: {e}");
    }

    tracing::info!(label = %label, view = %new_view_id, "슬롯 팝업 분리 완료(detach)");
    Ok(())
}

// ★bind_window #[tauri::command] 제거(Fix 4)★: 임의 창 label 을 임의 view_id 에 바인딩하는 공개 command
//   는 dead code 였고(프론트 호출자 0 — pop_out_slot 은 ViewManager.bind_window 를 직접 부름) invoke_handler
//   에 노출된 채라 `{label:"main", arbitrary_view}` 같은 호출로 라우팅을 오염시킬 수 있었다. §5 상 노출 제어
//   표면은 pop_out_slot 하나면 충분하므로 command 를 삭제한다. 매니저 메서드 ViewManager::bind_window 는
//   pop_out_slot 이 계속 쓰므로 그대로 두고(아래 tests 도 그 메서드를 검증), command wrapper 만 지운다.

/// ★창 Destroyed 정리(수명/누수 임계)★. 팝업 창이 닫히면(정상 close 또는 프로그램 destroy) lib.rs 의
/// on_window_event Destroyed arm 이 이걸 부른다: 그 label 에 바인딩된 **팝업 View 를 통째로 닫고**(Fix 2A —
/// 아니면 "Popup N" View 가 고아로 쌓여 switch_view 로 main 에 되살아나거나 탭 바에 유령 탭이 남는다) →
/// rebuild(라우팅 표에서 그 label·view 빠짐) → 그 label 의 출력 Channel 을 registry 에서 제거(누수 방지).
/// 반환된 Unsubscribe 델타는 호출자가 락 밖에서 데몬에 보낸다(그 agent 가 더는 어느 창에도 안 보이면 데몬
/// 구독 정리). View 목록이 바뀌었으니 view:list-updated 를 main 창에 emit 한다(ADR-0006: 락 해제 후 emit).
///
/// ★이 함수는 command 가 아니다★ — Rust 이벤트 핸들러(on_window_event)에서 직접 호출한다. 그래서 State
/// 대신 이미 손에 쥔 Arc 참조들을 인자로 받는다(lib.rs 가 app.state 로 꺼내 넘김). emit 용 AppHandle 도 인자로.
pub fn cleanup_popup_window(
    app: &tauri::AppHandle,
    label: &str,
    state: &LayoutState,
    router: &OutputRouter,
    registry: &crate::output_channel::WindowChannelRegistry,
    client: &DaemonClient,
) {
    // 1) 바인딩된 팝업 View 닫기 + 라우팅 표 재계산 + view:list 페이로드 구성(전부 같은 락 안).
    let (delta, list) = {
        let Ok(mut mgr) = state.0.lock() else {
            tracing::warn!(
                label,
                "cleanup_popup_window: ViewManager lock poisoned — 정리 스킵"
            );
            return;
        };
        // ★Fix 2A: 백킹 View 를 닫는다★. 이 label 에 바인딩된 view_id 를 먼저 읽고, 있으면 close_view 로
        //   View 자체를 제거한다(close_view 가 retain 으로 이 label 바인딩도 함께 정리 — 언바인딩 중복 불필요).
        //   바인딩이 이미 사라졌으면(경합/LLM 이 먼저 close_view) 종전대로 unbind_window 만 no-op 호출한다.
        match mgr.window_bindings.get(label).copied() {
            Some(v) => {
                let _ = mgr.close_view(v); // View 제거 + 이 label 바인딩 retain 정리
            }
            None => mgr.unbind_window(label), // ADR-0046: stale binding 방어(없으면 no-op)
        }
        let delta = router.rebuild(&mgr);
        let list = list_payload(&mgr);
        (delta, list)
    }; // ← 락 드롭

    // 2) 데몬 구독 정리(락 밖, ADR-0006): 이 창이 마지막이던 agent 는 1→0 → Unsubscribe.
    for a in delta.to_unsubscribe {
        client.unsubscribe(a);
    }

    // 2b) View 목록 변경을 main 창 탭 바에 반영(락 밖 emit, ADR-0006). 팝업 View 제거로 탭 목록이 바뀐다.
    if let Err(e) = app.emit(EVT_VIEW_LIST_UPDATED, &list) {
        tracing::warn!(
            label,
            "cleanup_popup_window: {EVT_VIEW_LIST_UPDATED} emit 실패: {e}"
        );
    }

    // 3) 출력 Channel registry 에서 이 label 제거(누수 방지 — 죽은 webview Channel 이 남지 않게).
    //    subscribe_output 이 insert 한 대응물. (죽은 Channel 은 send 시 어차피 Err 로 제거되나, 명시 제거로
    //    확정한다 — 라벨 재사용은 없으므로 잔존 엔트리는 순수 누수.)
    if let Ok(mut reg) = registry.lock() {
        reg.remove(label);
    } else {
        tracing::warn!(
            label,
            "cleanup_popup_window: registry lock poisoned — Channel 제거 스킵"
        );
    }

    tracing::info!(label, "팝업 창 정리 완료(바인딩·구독·Channel)");
}

/// 이 label 이 팝업 창인지(prefix 매칭). lib.rs Destroyed arm 이 main/agent-tree 와 구분하는 데 쓴다.
pub fn is_popup_label(label: &str) -> bool {
    label.starts_with(POPUP_LABEL_PREFIX)
}

// ── 내부 헬퍼(commands/layout.rs list_payload 와 동형 — 재노출 대신 로컬 정의로 결합도 낮춤) ─────────
/// view:list-updated 페이로드 구성(main 창 탭 바 갱신용).
fn list_payload(mgr: &crate::layout::ViewManager) -> ViewListPayload {
    ViewListPayload {
        views: mgr.view_metas(),
        active_view_id: mgr.active_view_id,
    }
}

/// view:list-updated 페이로드(commands/layout.rs ViewListPayload 와 동형 wire — emit 페이로드).
#[derive(serde::Serialize, Clone)]
struct ViewListPayload {
    views: Vec<crate::layout::ViewMeta>,
    active_view_id: Uuid,
}

// ── 테스트: 라벨 발급·바인딩/언바인딩 라우팅 로직(창 생성 자체는 running app 필요라 GUI 검증) ──────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::ViewManager;

    /// 새 agent uuid + 문자열.
    fn agent() -> (Uuid, String) {
        let id = Uuid::new_v4();
        (id, id.to_string())
    }

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
    fn bind_window_routes_agent_to_popup_label() {
        // 새 View 를 만들고 그 슬롯에 agent 배정 → 팝업 label 로 바인딩 → router.targets 가 그 label 포함.
        let mut mgr = ViewManager::new();
        let v1 = mgr.active_view_id;
        let new_view = mgr.create_view(None);
        let _ = mgr.switch_view(v1); // active=main(v1), new_view 는 바인딩 전용

        let (aid, astr) = agent();
        let slot = {
            let v = mgr.views.iter().find(|v| v.id == new_view).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        mgr.assign_agent(new_view, slot, astr).unwrap();

        // 바인딩 전: 어느 창에도 안 보임(비활성·미바인딩).
        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert!(router.targets(aid).is_empty(), "바인딩 전엔 라우팅 대상 0");

        // 바인딩 후: 팝업 label 로 라우팅.
        mgr.bind_window("slot-popup-1".into(), new_view).unwrap();
        router.rebuild(&mgr);
        let t: Vec<String> = router.targets(aid).iter().cloned().collect();
        assert_eq!(
            t,
            vec!["slot-popup-1".to_string()],
            "바인딩된 팝업 label 로 라우팅"
        );
    }

    #[test]
    fn unbind_window_removes_routing_and_unsubscribes() {
        // 바인딩된 팝업이 유일 소비자였던 agent → unbind 후 rebuild 델타에 Unsubscribe(1→0).
        let mut mgr = ViewManager::new();
        let v1 = mgr.active_view_id;
        let new_view = mgr.create_view(None);
        let _ = mgr.switch_view(v1);

        let (aid, astr) = agent();
        let slot = {
            let v = mgr.views.iter().find(|v| v.id == new_view).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        mgr.assign_agent(new_view, slot, astr).unwrap();
        mgr.bind_window("slot-popup-1".into(), new_view).unwrap();

        let router = OutputRouter::new();
        router.rebuild(&mgr); // 0→1: 팝업으로 보이기 시작

        // unbind → rebuild: 그 agent 가 어느 창에도 안 보임 → 1→0 Unsubscribe + label 라우팅 사라짐.
        mgr.unbind_window("slot-popup-1");
        let delta = router.rebuild(&mgr);
        assert_eq!(
            delta.to_unsubscribe,
            vec![aid],
            "팝업 닫힘 → 1→0 Unsubscribe"
        );
        assert!(
            router.targets(aid).is_empty(),
            "unbind 후 라우팅 대상 0(stale binding 없음)"
        );
    }

    #[test]
    fn slot_agent_reads_assigned_ref() {
        // pop_out_slot ①단계: 원본 슬롯의 agent 를 정확히 읽는다(빈 슬롯은 None).
        let mut mgr = ViewManager::new();
        let view = mgr.active_view_id;
        let slot = crate::layout::tree::first_slot_id(
            &mgr.views.iter().find(|v| v.id == view).unwrap().layout,
        );
        assert_eq!(mgr.slot_agent(view, slot).unwrap(), None, "빈 슬롯은 None");
        mgr.assign_agent(view, slot, "agent-x".into()).unwrap();
        assert_eq!(
            mgr.slot_agent(view, slot).unwrap(),
            Some("agent-x".to_string()),
            "배정된 agent 참조를 읽음"
        );
    }

    #[test]
    fn bind_window_invalid_view_is_err_noop() {
        let mut mgr = ViewManager::new();
        let before = mgr.window_bindings.len();
        assert!(mgr
            .bind_window("slot-popup-9".into(), Uuid::new_v4())
            .is_err());
        assert_eq!(
            mgr.window_bindings.len(),
            before,
            "invalid view → 바인딩 안 함"
        );
    }
}
