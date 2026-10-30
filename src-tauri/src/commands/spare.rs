//! 예열 창의 셸 쪽 창 관리(TRD S21-slot-docking §3-9): 창 역할 질의([`window_role`]) · 창 포트의 맡기기 · 확정 ·
//! 되물림 몫([`claim`] · [`commit`] · [`release`] — 창 놓기는 `commands::popout` 의 창 포트) · 다시 짓기 완료(맡긴 창의
//! 보임 · 창 `Destroyed` · 안전 시한) · 예열 창 짓기 · 앱 종료 표시([`on_app_exiting`]). 상태 기계는 `crate::spare` 에
//! 있고 여기는 Tauri 어댑터다.
//!
//! ★락 차례 = 레이아웃 → 예열 창 풀 → 첫 화면 문★(`SpareState` 문서) — 풀 락을 쥔 채 잡는 것은 첫 화면 문의 잎 락
//! 하나뿐이고, 쥔 채 레이아웃 락 · emit · 창 호출 · 창 짓기를 하지 않는다. 이 파일의 창 호출 · emit 은 전부 두 락 밖이다.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::commands::popout::PopupCounter;
use crate::layout::apply;
use crate::layout::{LayoutState, SpareClaim};
pub(crate) use crate::spare::elapsed_ms;
use crate::spare::pool::Opening;
use crate::spare::wire::{SpareAdopted, WindowRoleReply, EVT_SPARE_ADOPTED};
use crate::spare::{CommitPort, SpareState};

/// 부른 창의 지금 역할(동기) — `invoke('window_role', { adoptable })`. 웹뷰가 마운트 때 `spare:adopted` 듣기를 건 뒤
/// 묻고 `spare` 면 대기 모드로 그린다(TRD §3-9). `adoptable` = 그 듣기를 걸었다 — `false` 인 풀의 창은 풀에서 빼 닫고
/// `unknown` 이다(I28). 채택 뒤 보임을 기다리는 창이면 그 채택 번호도 싣는다(I26). 웹뷰 안 배선이라 버스 명령표에 올리지
/// 않는다.
// ADR-0299
#[tauri::command]
pub fn window_role(
    app: AppHandle,
    window: Window,
    adoptable: bool,
    spare: State<'_, Arc<SpareState>>,
    layout: State<'_, LayoutState>,
) -> WindowRoleReply {
    let label = window.label();
    let answer = spare.answer_role(label, adoptable, || {
        apply::list_windows(&layout).is_ok_and(|windows| windows.iter().any(|w| w == label))
    });
    if answer.close {
        // 동기 명령은 메인 스레드에서 돈다 — 부른 웹뷰를 그 IPC 처리기 안에서 거두지 않고 작업 스레드로 넘긴다. 곧바로
        //   다시 짓지 않는다 — 풀에서 먼저 뺐으므로 그 `Destroyed` 는 풀 항목을 못 찾는다(다음 맡기기가 짓는다 · I16).
        let label = label.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            crate::commands::popout::destroy_window(&app, &label)
        });
    }
    answer.reply
}

/// 예열 창 풀의 창인가 — 트레이 보이기 · 숨기기 · `--hidden` 과 자리 기록이 빼는 창(TRD §3-9).
pub(crate) fn is_spare_window(app: &AppHandle, label: &str) -> bool {
    app.try_state::<Arc<SpareState>>()
        .is_some_and(|spare| spare.is_spare(label))
}

/// 창 `Destroyed` 의 풀 갈래(TRD §3-9 「다시 짓기」) — 풀의 창이면 풀에서 빼고 다시 짓고, 보임을 기다리던 채택 창이면
/// 다시 짓기 완료다(둘 다 종료 중 · 풀 꺼짐이면 짓지 않는다 · 판정 = `SpareState::on_destroyed`). 닫는 창의 풀 항목을
/// 지우는 곳은 여기 하나다. 레이아웃 락 밖에서 부른다(`lib.rs`).
// ADR-0295 · ADR-0299
pub(crate) fn on_spare_window_destroyed(app: &AppHandle, label: &str) {
    let Some(spare) = app.try_state::<Arc<SpareState>>() else {
        return;
    };
    if spare.on_destroyed(label) {
        spawn_spare_build(app);
    }
}

/// 맡긴 예열 창이 첫 화면에 보였다(알림 또는 그 시한 — `first_paint` 의 맡긴 창 보이기) — 다시 짓기 완료다. 메인
/// 스레드에서 불린다(짓기는 작업 스레드로 넘긴다).
// ADR-0299
pub(crate) fn on_adopted_window_shown(app: &AppHandle, label: &str) {
    if app
        .try_state::<Arc<SpareState>>()
        .is_some_and(|spare| spare.on_shown(label))
    {
        spawn_spare_build(app);
    }
}

// ── 창 포트의 예열 창 몫(TRD §3-9 「포트」 · 「맡기기」 · 「확정」) ─────────────────────────────

/// 창 포트 `claim_spare` 의 실물 — 맡길 예열 창(짓는 중이면 기다린 뒤)을 맡긴다. 맡길 것이 없고 풀이 바라면 짓기를
/// 띄운다(I16). 풀이 등록돼 있지 않으면 `None`. ★메인 스레드에서 부르지 말 것★(I15 — `SpareState::claim`).
// ADR-0299
pub(crate) fn claim(app: &AppHandle) -> Option<SpareClaim> {
    let spare = app.try_state::<Arc<SpareState>>()?;
    let reply = spare.claim();
    if reply.build {
        spawn_spare_build(app);
    }
    let wait_ms = reply
        .waited
        .map(|waited| u64::try_from(waited.as_millis()).unwrap_or(u64::MAX));
    match &reply.claim {
        Some(claim) => tracing::info!(
            module = "spare",
            label = claim.label,
            generation = claim.generation,
            wait_ms = ?wait_ms,
            "창 포트 — 예열 창을 맡겼다(spare_claim)"
        ),
        None => tracing::debug!(
            module = "spare",
            build = reply.build,
            wait_ms = ?wait_ms,
            "창 포트 — 맡길 예열 창이 없어 새로 짓는다"
        ),
    }
    reply.claim
}

/// 창 포트가 `label` 을 열(놓을) 때 — 풀이 그 label 을 어떻게 보나(뜻 = `SpareState::opening`). 풀이 등록돼 있지 않으면
/// `Fresh`.
pub(crate) fn opening(app: &AppHandle, label: &str, activate: bool) -> Opening {
    app.try_state::<Arc<SpareState>>()
        .map_or(Opening::Fresh, |spare| spare.opening(label, activate))
}

/// 창 포트 `committed` 의 실물 — 확정(`SpareState::commit`)을 Tauri 창 쪽 일([`TauriCommitPort`])과 묶는다. 락 밖에서
/// 부른다.
// ADR-0299
pub(crate) fn commit(app: &AppHandle, claim: &SpareClaim) {
    let (Some(spare), Some(layout)) = (
        app.try_state::<Arc<SpareState>>(),
        app.try_state::<LayoutState>(),
    ) else {
        tracing::warn!(
            module = "spare",
            label = claim.label,
            "예열 창 상태 · 레이아웃이 등록돼 있지 않아 맡긴 창을 확정하지 못했다"
        );
        return;
    };
    // 들인 창의 열기 표시는 들임 알림(`spare:adopted`)보다 먼저다 — 알림 · 탭을 받은 페이지가 번호 표식 · 창 단추를 그리며
    //   처음 보내는 최소 크기 보고가 넓혀 당긴다(TRD S26-window-chrome §7-7). 대기 중 페이지는 창 번호가 없어 보고하지
    //   않는다(`TitleRow`) — 그래서 날아가던 대기 중 보고가 이 표시를 먼저 거둘 일이 없다(같은 페이지라 보고 순번으로는 대기
    //   중 보고와 들인 뒤 보고를 가를 수 없다). 확정이 닫기로 끝나면 그 창의 `Destroyed` 가 표시를 잊는다.
    crate::chrome::mark_opening(app, &claim.label);
    spare.commit(&layout, claim, &TauriCommitPort { app });
}

/// 창 포트 `release_spare` 의 실물 — 맡김표가 지금의 맡김이면 숨긴 채 풀에 돌려준다(아니면 무동작 + warn).
pub(crate) fn release(app: &AppHandle, claim: &SpareClaim) {
    if let Some(spare) = app.try_state::<Arc<SpareState>>() {
        spare.release(claim);
    }
}

/// 맡긴 창을 놓다 실패해 거두려 했으나 그 거두기마저 실패했다 — 그 창의 `Destroyed` 가 오지 않으면 맡김이 끝나지 않아
/// 그 실행의 예열이 멈춘다. 사라진 것으로 치고 풀에서 뺀 뒤 다시 짓는다(숨은 창 하나는 남는다 — 새로 지은 창의 같은
/// 실패와 같다).
// ADR-0299
pub(crate) fn abandon_claimed(app: &AppHandle, label: &str) {
    tracing::warn!(
        module = "spare",
        label,
        "맡긴 예열 창을 거두지 못해 사라진 것으로 치고 풀에서 뺀다"
    );
    on_spare_window_destroyed(app, label);
}

/// 확정 ③ ④ 뒤의 Tauri 창 쪽 일(`crate::spare::CommitPort`) — 레이아웃 락 · 풀 락 밖에서 불린다.
struct TauriCommitPort<'a> {
    app: &'a AppHandle,
}

impl CommitPort for TauriCommitPort<'_> {
    fn adopted(&self, label: &str, generation: u32) {
        if let Err(e) = self
            .app
            .emit_to(label, EVT_SPARE_ADOPTED, SpareAdopted { generation })
        {
            tracing::warn!(
                module = "spare",
                label,
                generation,
                "{EVT_SPARE_ADOPTED} emit 실패 — 그 창은 첫 화면 시한에 보인다: {e}"
            );
        }
    }

    fn reveal_deadline(&self, label: &str, after: Duration) {
        let Some(spare) = self
            .app
            .try_state::<Arc<SpareState>>()
            .map(|spare| spare.inner().clone())
        else {
            return;
        };
        let app = self.app.clone();
        let label = label.to_owned();
        tauri::async_runtime::spawn(complete_when_overdue(spare, label, after, move || {
            spawn_spare_build(&app)
        }));
    }

    fn destroy(&self, label: &str) -> Result<(), String> {
        crate::commands::popout::try_destroy_window(self.app, label)
    }
}

// 안전 시한(I8) — `after` 뒤 그 label 의 보임 기다림이 아직 서 있으면 끝내고 짓는다. 먼저 온 완료(보임 · `Destroyed`)가
//   있었으면 무동작이다(label 마다 한 번 — `SpareState::on_reveal_overdue`).
fn complete_when_overdue(
    spare: Arc<SpareState>,
    label: String,
    after: Duration,
    build: impl FnOnce() + Send + 'static,
) -> impl Future<Output = ()> + Send + 'static {
    async move {
        tokio::time::sleep(after).await;
        if spare.on_reveal_overdue(&label) {
            tracing::info!(
                module = "spare",
                label,
                "채택한 창이 안전 시한까지 보이지 않았다(UI 숨기기 · 보이기 실패) — 다시 짓는다"
            );
            build();
        }
    }
}

/// 앱 종료 요청 — 이 뒤로 사라지는 예열 창은 다시 짓지 않는다(`lib.rs` 의 `run` 사건).
pub(crate) fn on_app_exiting(app: &AppHandle) {
    if let Some(spare) = app.try_state::<Arc<SpareState>>() {
        spare.begin_exit();
    }
}

// ── 예열 창 짓기(TRD §3-9 「부팅」 · 「다시 짓기」) ─────────────────────────────────────
//
// 미리 하나 지어 ★숨긴 채★ 기다린다 — 화면 밖 대기 자리를 두지 않는다(사용자 결정 2026-10-19). 숨긴 최상위 창은
// 작업 표시줄 · Alt+Tab 에 나오지 않는다.
// ★숨긴 채 기다린 창의 첫 보이기 모습은 잰 적이 없다★ — 스파이크 S8 의 미리 만든 창(깜빡임 0)은 화면 밖에 보이는
// 채였다. 새면 증상 = 그 창을 처음 보일 때 검정 · 흰 · 지난 그림이 몇 프레임 보인다.
// ★작업 스레드에서 짓는다★ — 동기 IPC 처리기 안에서 지으면 Windows 에서 교착이고(스파이크), 메인 스레드의 사건
// 처리기(`Destroyed`)도 부르는 쪽이다.

/// 예열 창 하나를 작업 스레드에서 짓는다 — 풀에 예열 창(짓는 중 포함)이 이미 있거나 · 보임을 기다리는 채택 창이
/// 있거나 · 앱이 끝나는 중 · 풀이 꺼졌으면 짓지 않고 팝아웃 번호도 받지 않는다(`SpareState::build`).
pub(crate) fn spawn_spare_build(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || build_spare(&app));
}

// 기다리는 동안 이 창에는 보임을 바꾸는 창 호출을 하지 않는다 — tao 보임 깃발(거짓)이 OS 보임과 맞은 채다.
// ★보일 때는 Tauri `show()` 로★ — 날 Win32 로 보이면 깃발이 거짓으로 남아 트레이 숨기기가 아무것도 하지 않는다(바뀐
//   깃발이 없으면 tao 0.35.3 `apply_diff` 가 돌아간다). 포커스 없이 지은 창이라 tao 는 활성화하지 않고 보인다
//   (`SW_SHOWNOACTIVATE`).
// ADR-0295
fn build_spare(app: &AppHandle) {
    let (Some(spare), Some(labels)) = (
        app.try_state::<Arc<SpareState>>(),
        app.try_state::<Arc<PopupCounter>>(),
    ) else {
        tracing::warn!(
            module = "spare",
            "예열 창 상태 · 팝아웃 번호가 등록돼 있지 않아 예열 창을 짓지 못했다"
        );
        return;
    };
    spare.build(&**labels, |label| {
        crate::commands::popout::build_spare_window(app, label).map(drop)
    });
}

// ── 시험: 안전 시한(TRD S21-slot-docking §8-1 ⑫ 「다시 짓기」) — 창 없이 선다 ──────────
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;

    use super::*;
    use crate::layout::LabelSource;
    use crate::spare::{CommitOutcome, PaintGate, SpareBuild, SpareSeams};

    struct OpenGate;

    impl PaintGate for OpenGate {
        fn hold_unarmed(&self, _label: &str) {}
        fn cancel(&self, _label: &str) {}
        fn reveal_if_held(
            &self,
            _label: &str,
            _generation: u32,
            _activate: bool,
            _opened: Option<Instant>,
        ) -> bool {
            true
        }
    }

    struct NoWindows;

    impl CommitPort for NoWindows {
        fn adopted(&self, _label: &str, _generation: u32) {}
        fn reveal_deadline(&self, _label: &str, _after: Duration) {}
        fn destroy(&self, _label: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Labels(AtomicU32);

    impl LabelSource for Labels {
        fn next_label(&self) -> String {
            format!("slot-popup-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1)
        }
    }

    // 채택해 보임을 기다리는 창 하나.
    fn adopted() -> (Arc<SpareState>, String) {
        let spare = Arc::new(SpareState::new(SpareSeams::new(Arc::new(OpenGate))));
        let SpareBuild::Built { label } = spare.build(&Labels::default(), |_| Ok(())) else {
            panic!("지어야 한다");
        };
        let claim = spare.claim().claim.expect("맡긴다");
        let layout = LayoutState::new();
        layout.0.lock().unwrap().create_window(&label).unwrap();
        assert_eq!(
            spare.commit(&layout, &claim, &NoWindows),
            CommitOutcome::Adopted { revealed: true }
        );
        (spare, label)
    }

    fn counter() -> (Arc<AtomicU32>, impl FnOnce() + Send + 'static) {
        let built = Arc::new(AtomicU32::new(0));
        let sink = built.clone();
        (built, move || {
            sink.fetch_add(1, Ordering::SeqCst);
        })
    }

    #[tokio::test(start_paused = true)]
    async fn a_window_still_unrevealed_at_the_safety_deadline_is_completed_and_rebuilt() {
        let (spare, label) = adopted();
        let (built, build) = counter();
        let after = Duration::from_secs(3);
        let deadline = tokio::spawn(complete_when_overdue(spare.clone(), label, after, build));
        tokio::time::sleep(after - Duration::from_millis(1)).await;
        assert_eq!(built.load(Ordering::SeqCst), 0, "시한 전에는 짓지 않는다");
        deadline.await.unwrap();
        assert_eq!(built.load(Ordering::SeqCst), 1);
        assert_eq!(spare.awaiting_reveal(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_safety_deadline_after_the_reveal_builds_nothing() {
        let (spare, label) = adopted();
        let (built, build) = counter();
        assert!(spare.on_shown(&label), "보임이 먼저 짓는다");
        complete_when_overdue(spare, label, Duration::from_secs(3), build).await;
        assert_eq!(built.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn the_safety_deadline_after_the_window_is_gone_builds_nothing() {
        let (spare, label) = adopted();
        let (built, build) = counter();
        assert!(spare.on_destroyed(&label), "사라짐이 먼저 짓는다");
        complete_when_overdue(spare, label, Duration::from_secs(3), build).await;
        assert_eq!(built.load(Ordering::SeqCst), 0);
    }
}
