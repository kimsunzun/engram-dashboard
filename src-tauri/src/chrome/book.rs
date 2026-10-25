//! 창 크롬 장부 — 창마다 마지막으로 입힌 OS 제목과 그 판 · 웹이 보고한 최소 창 크기와 OS 에 입힌 값 · 열기 표시(TRD
//! S26-window-chrome §4-4 · §5-1 · §7-7). Tauri 를 모른다 — 어댑터(`crate::chrome`)가 잎 락으로 쥐고 메인 스레드 일 하나
//! 안에서 [`apply_title`] · [`report_min`] · [`apply_min`] 으로 부른다.
//!
//! ★창 항목은 그 OS 창이 있는 것을 본 뒤에만 만든다★ — 창 확인은 부르는 쪽 몫이고, 없으면 장부를 만지지 않는다.
//! `Destroyed` 가 항목을 잊은 뒤 늦게 온 제목 일이 항목을 되살리지 않게 하려는 것이다. 확인과 적기가 같은 메인 스레드
//! 일 안이고 `Destroyed` 처리기도 메인 스레드라 그 사이에 잊기가 끼어들 수 없다. label 은 한 실행 안에서 다시 쓰이지
//! 않아 묘비를 두지 않는다.
//!
//! ★OS 최소 크기는 보통 크기 창에만 입힌다 — 최대화 · 최소화 창에는 입히지도 걷지도 않는다★. tao 0.35.3 의 최소 크기
//! 세터(`set_min_inner_size`)는 지금 크기를 다시 입혀(`set_inner_size`) OS 가 새 한계를 다시 보게 하는데, 그 세터가
//! 최대화 깃발을 내려 최대화 창을 `SW_RESTORE` 로 원래 크기로 되돌린다 — 숨은 창이면 보이며 활성화한다(tao
//! `platform_impl/windows/window.rs` `set_min_inner_size` · `set_inner_size` → `window_state.rs` `apply_diff` · 소스
//! 독해). 그래서 최대화 · 최소화 동안 온 보고는 적어 두고, 창이 보통 크기가 되면(`Resized` — [`ChromeBook::on_frame`])
//! 입힌다. 최대화 동안 최소 크기를 걷지 않는 것은 그 때문이다 — 최대화 사각형은 작업 영역이고(tao 의 테두리 없는 창
//! `WM_NCCALCSIZE`) 웹이 보고하는 최소 크기(수백 px)가 그보다 작아 끼어들 일이 없다(작업 영역이 그보다 좁은 화면은
//! QA S8 이 잰다).

use std::collections::HashMap;
use std::sync::Mutex;

use engram_dashboard_base::sync;

#[derive(Debug, Default)]
pub(crate) struct ChromeBook {
    windows: HashMap<String, WindowChrome>,
}

#[derive(Debug, Default)]
struct WindowChrome {
    title: Option<TitleMark>,
    min: Option<MinMark>,
    // 열기 표시 — 셸이 그 창을 열 때 세우고([`ChromeBook::mark_opening`]) 그 창의 다음 최소 크기 보고가 거둔다. 넓혀
    //   당기기는 이 표시를 거둔 보고에서만 한다(열 때 한 번 — TRD §7-7 · PRD R20 · AC34).
    opening: bool,
    // OS 창에 마지막으로 입힌 최소 크기 — `None` = 아직 입히지 않았거나 입히다 실패했다.
    applied: Option<MinSize>,
}

impl WindowChrome {
    // 적은 최소 크기가 OS 에 입힌 값과 다르면 그 값 — 입힌 것으로 적는다(못 입혔으면 부르는 쪽이
    //   [`ChromeBook::min_failed`]).
    fn next_min(&mut self) -> Option<MinSize> {
        let want = self.min.as_ref()?.size;
        if self.applied == Some(want) {
            return None;
        }
        self.applied = Some(want);
        Some(want)
    }
}

/// 최소 크기를 입힐지 정할 때의 그 창 크기 상태 — 어댑터가 판정과 같은 메인 스레드 일 안에서 읽는다(모듈 머리).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameState {
    /// 보통 크기(보이든 숨었든 — 숨은 창의 미룬 최대화는 아직 보통 크기다) — 최소 크기를 입힌다.
    Normal,
    /// 최대화 · 최소화 · 못 읽음 — 지금 OS 최소 크기를 건드리지 않는다(모듈 머리).
    Other,
}

/// 최소 크기 판정 — 어댑터가 장부 락 밖에서 [`apply_min`] 으로 입힌다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MinPlan {
    /// 넓혀 당기기 — 보통 크기가 이 폭보다 좁으면 넓히고 작업 영역 안으로 당긴다(`state::placement::fit_min_width`).
    pub(crate) widen: Option<MinSize>,
    /// OS 최소 크기로 입힐 값 — `None` = OS 를 건드리지 않는다.
    pub(crate) set_min: Option<MinSize>,
}

/// [`ChromeBook::on_min_report`] 의 답.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MinReport {
    pub(crate) verdict: MinVerdict,
    pub(crate) plan: MinPlan,
}

/// 웹이 그 창의 제목 줄로 센 최소 창 크기(CSS px = 논리 px · 보통 상태 기준 — TRD S26-window-chrome §7-7). 두 값 다
/// `1..=MIN_SIZE_MAX` 다([`MinSize::new`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MinSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

// 웹 셈의 가장 큰 값(세 자리 번호 표식 · 단추 셋 · 칸 최소 상한 1000)보다 넉넉히 크다 — 그 너머는 셈 잘못으로 본다. 입히면
//   창이 화면보다 커진다.
const MIN_SIZE_MAX: u32 = 4096;

impl MinSize {
    /// 둘 중 하나라도 `1..=MIN_SIZE_MAX`(4096) 밖이면 `Err`(그 문구).
    pub(crate) fn new(width: u32, height: u32) -> Result<Self, String> {
        let range = 1..=MIN_SIZE_MAX;
        if !range.contains(&width) || !range.contains(&height) {
            return Err(format!(
                "최소 창 크기 {width}x{height} — 둘 다 1~{MIN_SIZE_MAX} 이어야 한다"
            ));
        }
        Ok(Self { width, height })
    }
}

/// 최소 크기 보고 하나의 차례(TRD S26-window-chrome §7-7) — `epoch` = 그 창의 웹 페이지를 읽은 벽시계 ms(웹
/// `Math.floor(performance.timeOrigin)` — 다시 읽을수록 크다. 같은 ms 에 읽은 두 페이지는 한 페이지로 섞인다 — 다루지
/// 않는다) · `seq` = 그 페이지 안에서 보낼 때마다(재시도 포함) 하나 오르는 순번(1 부터).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MinOrder {
    pub(crate) epoch: u64,
    pub(crate) seq: u32,
}

#[derive(Debug)]
struct MinMark {
    order: MinOrder,
    size: MinSize,
}

/// [`ChromeBook::record_min`] 의 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MinVerdict {
    /// 받아들였고 직전 값과 다르다(그 창의 첫 보고 포함).
    Changed,
    /// 받아들였고 직전 값과 같다.
    Same,
    /// 받아들인 것보다 옛 `epoch` 이거나, 같은 `epoch` 에서 이미 받아들인 `seq` 보다 새 순번이 아니다 — 웹이 시한에 놓은
    /// 옛 시도(다시 읽기 전 페이지의 것 포함)가 늦게 닿았다. 적지 않는다.
    Stale,
}

#[derive(Debug)]
struct TitleMark {
    version: u64,
    // `None` = 그 판을 입히다 실패했다 — 다음 새 판은 글자가 같아도 입힌다.
    text: Option<String>,
}

/// [`ChromeBook::accept_title`] 의 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TitleVerdict {
    /// 새 판이고 글자가 바뀌었다 — 입힌다.
    Set,
    /// 새 판이지만 마지막으로 입힌 글자와 같다 — OS 를 부르지 않는다.
    Same,
    /// 그 창에 이미 받아들인 판보다 새 판이 아니다 — 차례가 뒤바뀌어 늦게 온 옛 알림이다.
    Stale,
}

impl ChromeBook {
    /// 창 `label` 에 판 `version` 의 제목 `title` 을 입힐지 정하고 적는다. 판은 그 창에 받아들인 마지막 판보다 클 때만
    /// 받아들이고(같은 판은 옛 판이다), 받아들이면 `Same` 이어도 판을 적는다 — 뒤에 온 옛 판이 같은 글자 뒤에 숨어
    /// 들어오지 못하게. `Set` 이면 부르는 쪽이 곧 입힌다(못 입혔으면 [`Self::title_failed`]). 항목이 없으면 만든다 —
    /// 그 OS 창이 지금 있는지는 부르는 쪽이 먼저 본다(모듈 머리).
    pub(crate) fn accept_title(&mut self, label: &str, version: u64, title: &str) -> TitleVerdict {
        let window = self.windows.entry(label.to_owned()).or_default();
        let Some(mark) = &mut window.title else {
            window.title = Some(TitleMark {
                version,
                text: Some(title.to_owned()),
            });
            return TitleVerdict::Set;
        };
        if version <= mark.version {
            return TitleVerdict::Stale;
        }
        mark.version = version;
        if mark.text.as_deref() == Some(title) {
            return TitleVerdict::Same;
        }
        mark.text = Some(title.to_owned());
        TitleVerdict::Set
    }

    /// `Set` 으로 받아들인 `label` 의 제목을 못 입혔다 — 판은 두고 글자만 지워, 다음 새 판은 글자가 같아도 입힌다.
    pub(crate) fn title_failed(&mut self, label: &str) {
        if let Some(mark) = self
            .windows
            .get_mut(label)
            .and_then(|window| window.title.as_mut())
        {
            mark.text = None;
        }
    }

    /// 창 `label` 의 최소 크기 보고를 적는다. 받아들인 것보다 옛 `epoch` 는 버리고, 같은 `epoch` 안에서는 받아들인 마지막
    /// `seq` 보다 클 때만 받아들이며, 더 큰 `epoch`(그 창의 페이지를 다시 읽었다)면 순번을 새로 센다 — 그래서 보고가 닿는
    /// 차례와 무관하게 가장 새 페이지가 마지막으로 보낸 값이 남는다. 항목이 없으면 만든다(창 확인은 부르는 쪽 — 모듈 머리).
    pub(crate) fn record_min(&mut self, label: &str, order: MinOrder, min: MinSize) -> MinVerdict {
        let window = self.windows.entry(label.to_owned()).or_default();
        let verdict = match &window.min {
            Some(mark) if (order.epoch, order.seq) <= (mark.order.epoch, mark.order.seq) => {
                return MinVerdict::Stale;
            }
            Some(mark) if mark.size == min => MinVerdict::Same,
            _ => MinVerdict::Changed,
        };
        window.min = Some(MinMark { order, size: min });
        verdict
    }

    /// 창 `label` 을 셸이 막 열었다 — 그 창의 다음 최소 크기 보고가 넓혀 당기기를 한다(TRD §7-7 — 창 포트의 새로 짓기 ·
    /// 맡긴 예열 창의 확정 · 부팅 복원 창 짓기 · 사본 수락의 숨긴 팝아웃 짓기). 항목이 없으면 만든다(창 확인은 부르는 쪽 —
    /// 모듈 머리).
    pub(crate) fn mark_opening(&mut self, label: &str) {
        self.windows.entry(label.to_owned()).or_default().opening = true;
    }

    /// 최소 크기 보고 하나 — 적고([`Self::record_min`]) 무엇을 입힐지 정한다. 옛 순번(`Stale`)이면 아무것도 입히지 않고
    /// 열기 표시도 두지 않는다. 받아들인 보고는 열기 표시를 거두고, 넓혀 당기기는 그 표시가 있었고 지금 보통 크기일 때만이다
    /// (최대화 · 최소화 동안 연 창은 넓히지 않는다 — 열 때 한 번). OS 최소 크기는 보통 크기일 때 입힌 값과 다르면 입힌다
    /// (그 밖은 [`Self::on_frame`] 이 보통 크기가 될 때 입힌다 — 모듈 머리).
    pub(crate) fn on_min_report(
        &mut self,
        label: &str,
        order: MinOrder,
        min: MinSize,
        state: FrameState,
    ) -> MinReport {
        let verdict = self.record_min(label, order, min);
        if verdict == MinVerdict::Stale {
            return MinReport {
                verdict,
                plan: MinPlan::default(),
            };
        }
        let window = self.windows.entry(label.to_owned()).or_default();
        let opening = std::mem::take(&mut window.opening);
        let normal = state == FrameState::Normal;
        MinReport {
            verdict,
            plan: MinPlan {
                widen: (opening && normal).then_some(min),
                set_min: if normal { window.next_min() } else { None },
            },
        }
    }

    /// 창 `label` 의 크기 상태가 바뀌었다(`Resized` — 최대화 · 최소화에서 돌아옴 포함) — 보통 크기가 됐고 적은 최소 크기를
    /// 아직 OS 에 입히지 않았으면 그 값을 입히게 한다(넓혀 당기기는 하지 않는다). 항목을 만들지 않는다.
    pub(crate) fn on_frame(&mut self, label: &str, state: FrameState) -> MinPlan {
        let set_min = match (state, self.windows.get_mut(label)) {
            (FrameState::Normal, Some(window)) => window.next_min(),
            _ => None,
        };
        MinPlan {
            widen: None,
            set_min,
        }
    }

    /// 판정이 입히라고 한 OS 최소 크기를 못 입혔다 — 입힌 값을 잊어 다음 보고 · 크기 바뀜이 다시 입힌다.
    pub(crate) fn min_failed(&mut self, label: &str) {
        if let Some(window) = self.windows.get_mut(label) {
            window.applied = None;
        }
    }

    /// 창 `label` 의 항목을 잊는다 — 그 OS 창의 `Destroyed`. 없으면 무동작.
    pub(crate) fn forget(&mut self, label: &str) {
        self.windows.remove(label);
    }
}

/// 최소 크기 보고 하나의 몸통 — `window` = 그 label 의 OS 창의 지금 크기 상태(`None` = 창이 없다 · 부르는 쪽이 같은 메인
/// 스레드 일 안에서 읽는다 — 모듈 머리). 범위 밖이면 `Err` 이고 장부를 만지지 않는다(순번도 적지 않는다). 창이 없으면 적지
/// 않고 `Ok(None)`. 옛 순번이면 적지 않고 빈 판정이다. 판정은 [`ChromeBook::on_min_report`] — 입히기는 부르는 쪽이 장부
/// 락 밖에서 [`apply_min`] 으로 한다.
pub(crate) fn report_min(
    book: &Mutex<ChromeBook>,
    label: &str,
    width: u32,
    height: u32,
    order: MinOrder,
    window: Option<FrameState>,
) -> Result<Option<MinReport>, String> {
    let min = MinSize::new(width, height)?;
    let Some(state) = window else {
        tracing::debug!(
            label,
            width,
            height,
            "OS 창이 없어 최소 크기 보고를 적지 않는다"
        );
        return Ok(None);
    };
    let report = sync::lock(book).on_min_report(label, order, min, state);
    if report.verdict == MinVerdict::Stale {
        tracing::debug!(
            label,
            width,
            height,
            epoch = order.epoch,
            seq = order.seq,
            "옛 순번의 최소 크기 보고 — 버린다"
        );
    }
    Ok(Some(report))
}

/// 최소 크기를 입히는 창 하나의 OS 조작 — [`apply_min`] 이 창 없이 시험되게 끊은 자리. 실물 = `crate::chrome` 의 Tauri 창.
pub(crate) trait MinWindow {
    /// 보통 크기가 `min_w`(논리 px)보다 좁으면 그 폭으로 넓히고 작업 영역 안으로 당긴다(`state::placement::fit_min_width`).
    /// `Ok(true)` = 옮겼다 · `Ok(false)` = 넓힐 일이 없었다.
    fn widen_to(&self, min_w: u32) -> Result<bool, String>;
    /// OS 최소 크기(논리 안쪽 px)를 입힌다.
    fn apply_min_size(&self, min: MinSize) -> Result<(), String>;
}

/// 판정 `plan` 을 창 `label` 에 입힌다 — 장부 락 밖에서 부른다(잎 락 — 쥔 채 OS 를 부르지 않는다). ★넓혀 당기기가
/// 먼저다★ — OS 최소 크기를 먼저 입히면 OS 가 왼쪽 끝을 두고 오른쪽으로 넓혀 작업 영역 밖으로 민 뒤에 당기게 된다. 못 한
/// 일은 warn 을 남기고, 못 입힌 OS 최소 크기는 장부에 적는다([`ChromeBook::min_failed`]).
pub(crate) fn apply_min(
    book: &Mutex<ChromeBook>,
    label: &str,
    plan: MinPlan,
    window: &impl MinWindow,
) {
    if let Some(min) = plan.widen {
        match window.widen_to(min.width) {
            Ok(true) => tracing::info!(
                module = "chrome",
                label,
                min_width = min.width,
                "연 창이 최소 폭보다 좁아 넓혀 당겼다"
            ),
            Ok(false) => {}
            Err(e) => tracing::warn!(
                module = "chrome",
                label,
                min_width = min.width,
                "연 창을 최소 폭으로 넓히지 못했다(OS 최소 크기만 걸린다): {e}"
            ),
        }
    }
    if let Some(min) = plan.set_min {
        match window.apply_min_size(min) {
            // QA 재료(S8 — 최대화 동안 입히지 않는 것 · 보통이 되면 입히는 것을 로그로 가른다).
            Ok(()) => tracing::debug!(
                module = "chrome",
                label,
                width = min.width,
                height = min.height,
                "OS 최소 창 크기를 입혔다"
            ),
            Err(e) => {
                tracing::warn!(
                    module = "chrome",
                    label,
                    width = min.width,
                    height = min.height,
                    "OS 최소 창 크기를 입히지 못했다(다음 보고 · 크기 바뀜이 다시 입힌다): {e}"
                );
                sync::lock(book).min_failed(label);
            }
        }
    }
}

/// 제목 일 하나의 몸통 — 메인 스레드에서 돈다. `window` = 그 label 의 OS 창(없으면 `None` — 닫혔다). 창이 없으면 장부를
/// 만지지 않고 끝낸다. 판정은 `book` 락 안이고, `set_title` 은 그 락을 놓은 뒤 부른다(잎 락 — 쥔 채 OS 를 부르지
/// 않는다 · TRD S26-window-chrome §5-1). 실패는 warn 을 남기고 장부에 적는다([`ChromeBook::title_failed`]).
pub(crate) fn apply_title<W>(
    book: &Mutex<ChromeBook>,
    label: &str,
    version: u64,
    title: &str,
    window: Option<W>,
    set_title: impl FnOnce(&W, &str) -> Result<(), String>,
) {
    let Some(window) = window else {
        tracing::debug!(label, version, "OS 창이 없어 제목을 입히지 않는다");
        return;
    };
    let verdict = sync::lock(book).accept_title(label, version, title);
    match verdict {
        TitleVerdict::Set => {}
        TitleVerdict::Same => return,
        TitleVerdict::Stale => {
            tracing::debug!(label, version, "옛 판의 제목 — 버린다");
            return;
        }
    }
    if let Err(e) = set_title(&window, title) {
        tracing::warn!(
            label,
            version,
            "OS 창 제목을 입히지 못했다(다음 판이 다시 입힌다): {e}"
        );
        sync::lock(book).title_failed(label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const POPUP: &str = "slot-popup-1";

    // ── accept_title ──

    #[test]
    fn the_first_title_of_a_window_is_set() {
        let mut book = ChromeBook::default();
        assert_eq!(
            book.accept_title(POPUP, 3, "빈 칸 - Engram:2"),
            TitleVerdict::Set
        );
    }

    #[test]
    fn an_older_or_equal_version_is_stale_whatever_its_text() {
        let mut book = ChromeBook::default();
        book.accept_title(POPUP, 5, "a - Engram:2");

        assert_eq!(
            book.accept_title(POPUP, 4, "b - Engram:2"),
            TitleVerdict::Stale
        );
        assert_eq!(
            book.accept_title(POPUP, 5, "b - Engram:2"),
            TitleVerdict::Stale
        );
        assert_eq!(
            book.accept_title(POPUP, 6, "b - Engram:2"),
            TitleVerdict::Set,
            "버린 옛 판은 적히지 않았다 — 다음 새 판은 마지막으로 입힌 a 와 견준다"
        );
    }

    #[test]
    fn a_newer_version_with_the_same_text_skips_the_os_but_moves_the_version() {
        let mut book = ChromeBook::default();
        book.accept_title(POPUP, 5, "a - Engram:2");

        assert_eq!(
            book.accept_title(POPUP, 7, "a - Engram:2"),
            TitleVerdict::Same
        );
        assert_eq!(
            book.accept_title(POPUP, 6, "b - Engram:2"),
            TitleVerdict::Stale,
            "7 을 적었다 — 늦게 온 6 이 같은 글자 뒤로 끼어들지 못한다"
        );
    }

    #[test]
    fn windows_keep_their_own_versions() {
        let mut book = ChromeBook::default();
        book.accept_title("main", 9, "a - Engram:1");

        assert_eq!(
            book.accept_title(POPUP, 4, "b - Engram:2"),
            TitleVerdict::Set
        );
    }

    #[test]
    fn a_failed_title_is_set_again_by_the_next_version_even_with_the_same_text() {
        let mut book = ChromeBook::default();
        book.accept_title(POPUP, 5, "a - Engram:2");
        book.title_failed(POPUP);

        assert_eq!(
            book.accept_title(POPUP, 5, "a - Engram:2"),
            TitleVerdict::Stale
        );
        assert_eq!(
            book.accept_title(POPUP, 6, "a - Engram:2"),
            TitleVerdict::Set
        );
    }

    #[test]
    fn a_forgotten_window_starts_over() {
        let mut book = ChromeBook::default();
        book.accept_title(POPUP, 5, "a - Engram:2");
        book.forget(POPUP);

        assert_eq!(
            book.accept_title(POPUP, 1, "a - Engram:2"),
            TitleVerdict::Set
        );
        book.forget("never-seen");
    }

    // ── apply_title(메인 스레드 일) ──

    struct FakeWindow;

    // 부른 제목 · 그때 장부 락이 놓여 있었나.
    fn run(
        book: &Mutex<ChromeBook>,
        version: u64,
        title: &str,
        window: Option<FakeWindow>,
        fail: bool,
    ) -> Vec<(String, bool)> {
        let calls = RefCell::new(Vec::new());
        apply_title(book, POPUP, version, title, window, |_, text| {
            calls
                .borrow_mut()
                .push((text.to_owned(), book.try_lock().is_ok()));
            if fail {
                Err("창 핸들 무효(시험)".to_owned())
            } else {
                Ok(())
            }
        });
        calls.into_inner()
    }

    #[test]
    fn the_os_title_is_set_after_the_book_lock_is_released() {
        let book = Mutex::new(ChromeBook::default());

        let calls = run(&book, 1, "a - Engram:2", Some(FakeWindow), false);

        assert_eq!(calls, [("a - Engram:2".to_owned(), true)]);
    }

    #[test]
    fn only_new_versions_with_new_text_reach_the_os() {
        let book = Mutex::new(ChromeBook::default());
        run(&book, 5, "a - Engram:2", Some(FakeWindow), false);

        assert!(run(&book, 4, "b - Engram:2", Some(FakeWindow), false).is_empty());
        assert!(run(&book, 6, "a - Engram:2", Some(FakeWindow), false).is_empty());
        assert_eq!(
            run(&book, 7, "b - Engram:2", Some(FakeWindow), false).len(),
            1
        );
    }

    // `Destroyed` 가 잊은 뒤 늦게 온 일 — 창이 없으니 항목을 되살리지 않는다(TRD S26-window-chrome §4-4).
    #[test]
    fn a_late_job_after_forget_finds_no_window_and_leaves_no_entry() {
        let book = Mutex::new(ChromeBook::default());
        run(&book, 5, "a - Engram:2", Some(FakeWindow), false);
        book.lock().unwrap().forget(POPUP);

        assert!(run(&book, 9, "b - Engram:2", None, false).is_empty());

        assert!(
            book.lock().unwrap().windows.is_empty(),
            "잊은 창의 항목이 되살아났다"
        );
    }

    #[test]
    fn a_failed_set_is_retried_by_the_next_version_with_the_same_text() {
        let book = Mutex::new(ChromeBook::default());

        assert_eq!(
            run(&book, 5, "a - Engram:2", Some(FakeWindow), true).len(),
            1
        );

        assert!(run(&book, 5, "a - Engram:2", Some(FakeWindow), false).is_empty());
        assert_eq!(
            run(&book, 6, "a - Engram:2", Some(FakeWindow), false),
            [("a - Engram:2".to_owned(), true)]
        );
    }

    // ── 최소 크기 보고 ──

    const EPOCH: u64 = 1_761_000_000_000;

    fn nth(seq: u32) -> MinOrder {
        MinOrder { epoch: EPOCH, seq }
    }

    fn size(width: u32, height: u32) -> MinSize {
        MinSize::new(width, height).unwrap()
    }

    fn min_of(book: &Mutex<ChromeBook>, label: &str) -> Option<MinSize> {
        book.lock()
            .unwrap()
            .windows
            .get(label)
            .and_then(|w| w.min.as_ref())
            .map(|m| m.size)
    }

    // 보통 크기 창의 보고 하나 — `open` = 그 OS 창이 있다. `Ok(true)` = 적었고 직전 보고와 달랐다.
    fn changed(
        book: &Mutex<ChromeBook>,
        label: &str,
        width: u32,
        height: u32,
        order: MinOrder,
        open: bool,
    ) -> Result<bool, String> {
        report_min(
            book,
            label,
            width,
            height,
            order,
            open.then_some(FrameState::Normal),
        )
        .map(|report| report.is_some_and(|r| r.verdict == MinVerdict::Changed))
    }

    #[test]
    fn a_min_report_is_recorded_per_window_and_says_whether_it_changed() {
        let book = Mutex::new(ChromeBook::default());

        assert_eq!(changed(&book, POPUP, 286, 64, nth(1), true), Ok(true));
        assert_eq!(changed(&book, POPUP, 286, 64, nth(2), true), Ok(false));
        assert_eq!(changed(&book, POPUP, 295, 64, nth(3), true), Ok(true));
        assert_eq!(changed(&book, "main", 120, 64, nth(1), true), Ok(true));

        assert_eq!(
            min_of(&book, POPUP),
            Some(MinSize {
                width: 295,
                height: 64
            })
        );
        assert_eq!(
            min_of(&book, "main"),
            Some(MinSize {
                width: 120,
                height: 64
            })
        );
    }

    #[test]
    fn an_out_of_range_min_report_is_refused_and_keeps_the_last_one_and_its_seq() {
        let book = Mutex::new(ChromeBook::default());
        changed(&book, POPUP, 286, 64, nth(1), true).unwrap();

        for (i, (w, h)) in [
            (0, 64),
            (286, 0),
            (MIN_SIZE_MAX + 1, 64),
            (286, MIN_SIZE_MAX + 1),
        ]
        .into_iter()
        .enumerate()
        {
            let seq = 10 + i as u32;
            assert!(
                changed(&book, POPUP, w, h, nth(seq), true).is_err(),
                "{w}x{h}"
            );
        }
        assert_eq!(min_of(&book, POPUP), Some(size(286, 64)));
        assert_eq!(
            changed(&book, POPUP, MIN_SIZE_MAX, MIN_SIZE_MAX, nth(2), true),
            Ok(true),
            "거절한 보고의 순번(10~13)은 적지 않았다 — 2 가 받아들여진다"
        );
        assert_eq!(min_of(&book, POPUP), Some(size(MIN_SIZE_MAX, MIN_SIZE_MAX)));
    }

    // 웹이 시한에 놓은 시도가 더 새 시도 뒤에 셸에 닿은 경우(TRD S26-window-chrome §7-7).
    #[test]
    fn an_older_or_equal_seq_of_the_same_epoch_is_dropped() {
        let book = Mutex::new(ChromeBook::default());
        changed(&book, POPUP, 295, 64, nth(2), true).unwrap();

        assert_eq!(changed(&book, POPUP, 286, 64, nth(1), true), Ok(false));
        assert_eq!(changed(&book, POPUP, 286, 64, nth(2), true), Ok(false));
        assert_eq!(min_of(&book, POPUP), Some(size(295, 64)));

        assert_eq!(changed(&book, POPUP, 286, 64, nth(3), true), Ok(true));
        assert_eq!(min_of(&book, POPUP), Some(size(286, 64)));
    }

    #[test]
    fn record_min_tells_changed_same_and_stale_apart() {
        let mut book = ChromeBook::default();

        assert_eq!(
            book.record_min(POPUP, nth(1), size(286, 64)),
            MinVerdict::Changed
        );
        assert_eq!(
            book.record_min(POPUP, nth(2), size(286, 64)),
            MinVerdict::Same
        );
        assert_eq!(
            book.record_min(POPUP, nth(2), size(295, 64)),
            MinVerdict::Stale
        );
        assert_eq!(
            book.record_min(POPUP, nth(3), size(286, 64)),
            MinVerdict::Same,
            "버린 295 는 적히지 않았다"
        );
    }

    // 같은 창의 페이지를 다시 읽으면 웹의 순번이 1 부터 다시 간다.
    #[test]
    fn a_new_epoch_restarts_the_seq() {
        let book = Mutex::new(ChromeBook::default());
        changed(&book, POPUP, 295, 64, nth(5), true).unwrap();
        let reloaded = |seq| MinOrder {
            epoch: EPOCH + 1,
            seq,
        };

        assert_eq!(changed(&book, POPUP, 286, 64, reloaded(1), true), Ok(true));
        assert_eq!(changed(&book, POPUP, 120, 64, reloaded(1), true), Ok(false));
        assert_eq!(min_of(&book, POPUP), Some(size(286, 64)));
    }

    // 페이지 A 의 시도가 시한에 놓인 채, 다시 읽은 페이지 B 의 보고가 먼저 받아들여지고 A 의 것이 늦게 닿은 경우.
    #[test]
    fn a_late_report_of_an_older_epoch_after_a_newer_one_is_dropped() {
        let book = Mutex::new(ChromeBook::default());
        let page_a = |seq| MinOrder { epoch: EPOCH, seq };
        let page_b = |seq| MinOrder {
            epoch: EPOCH + 5_000,
            seq,
        };
        changed(&book, POPUP, 295, 64, page_a(1), true).unwrap();

        assert_eq!(changed(&book, POPUP, 286, 64, page_b(1), true), Ok(true));
        assert_eq!(
            changed(&book, POPUP, 120, 64, page_a(9), true),
            Ok(false),
            "옛 페이지의 순번이 더 커도 새 페이지 값을 덮지 못한다"
        );
        assert_eq!(min_of(&book, POPUP), Some(size(286, 64)));
        assert_eq!(
            book.lock()
                .unwrap()
                .record_min(POPUP, page_a(10), size(120, 64)),
            MinVerdict::Stale
        );
    }

    #[test]
    fn windows_keep_their_own_seqs() {
        let book = Mutex::new(ChromeBook::default());
        changed(&book, "main", 286, 64, nth(9), true).unwrap();

        assert_eq!(changed(&book, POPUP, 295, 64, nth(1), true), Ok(true));
    }

    // `Destroyed` 가 잊은 뒤 늦게 온 보고 — 창이 없으니 항목을 되살리지 않는다(모듈 머리).
    #[test]
    fn a_min_report_without_the_window_leaves_no_entry() {
        let book = Mutex::new(ChromeBook::default());

        assert_eq!(changed(&book, POPUP, 286, 64, nth(1), false), Ok(false));

        assert!(book.lock().unwrap().windows.is_empty());
    }

    #[test]
    fn forgetting_a_window_drops_its_min_report_and_seq_too() {
        let book = Mutex::new(ChromeBook::default());
        changed(&book, POPUP, 286, 64, nth(5), true).unwrap();
        book.lock().unwrap().forget(POPUP);

        assert_eq!(min_of(&book, POPUP), None);
        assert_eq!(changed(&book, POPUP, 286, 64, nth(1), true), Ok(true));
    }

    // ── 최소 크기 입히기 · 넓혀 당기기 판정(TRD S26-window-chrome §7-7) ──

    use FrameState::{Normal, Other};

    fn plan(book: &mut ChromeBook, seq: u32, min: MinSize, state: FrameState) -> MinPlan {
        book.on_min_report(POPUP, nth(seq), min, state).plan
    }

    fn set_only(min: MinSize) -> MinPlan {
        MinPlan {
            widen: None,
            set_min: Some(min),
        }
    }

    #[test]
    fn a_normal_window_gets_each_new_min_once() {
        let mut book = ChromeBook::default();

        assert_eq!(
            plan(&mut book, 1, size(286, 64), Normal),
            set_only(size(286, 64))
        );
        assert_eq!(
            plan(&mut book, 2, size(286, 64), Normal),
            MinPlan::default(),
            "같은 값은 다시 입히지 않는다"
        );
        assert_eq!(
            plan(&mut book, 3, size(295, 64), Normal),
            set_only(size(295, 64))
        );
    }

    // 최대화 · 최소화 창에 OS 최소 크기를 입히면 tao 가 원래 크기로 되돌린다(모듈 머리) — 보통 크기가 될 때 입힌다.
    #[test]
    fn a_maximized_or_minimized_window_gets_its_min_when_it_is_normal_again() {
        let mut book = ChromeBook::default();

        assert_eq!(plan(&mut book, 1, size(286, 64), Other), MinPlan::default());
        assert_eq!(book.on_frame(POPUP, Other), MinPlan::default());
        assert_eq!(book.on_frame(POPUP, Normal), set_only(size(286, 64)));
        assert_eq!(
            book.on_frame(POPUP, Normal),
            MinPlan::default(),
            "한 번 입힌 값은 크기가 바뀔 때마다 다시 입히지 않는다"
        );
    }

    #[test]
    fn maximizing_and_restoring_leave_an_applied_min_alone() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        assert_eq!(
            book.on_frame(POPUP, Other),
            MinPlan::default(),
            "최대화로 걷지 않는다"
        );
        assert_eq!(book.on_frame(POPUP, Normal), MinPlan::default());
    }

    #[test]
    fn a_frame_change_without_a_report_does_nothing_and_makes_no_entry() {
        let mut book = ChromeBook::default();

        assert_eq!(book.on_frame(POPUP, Normal), MinPlan::default());

        assert!(book.windows.is_empty());
    }

    // 넓혀 당기기는 열 때 한 번이다 — 사람이 창을 끌어 좁은 모니터로 옮기는 동안은 OS 최소 크기만 걸린다(PRD R20 · AC34).
    #[test]
    fn only_the_first_report_after_an_opening_mark_widens() {
        let mut book = ChromeBook::default();
        assert_eq!(
            plan(&mut book, 1, size(286, 64), Normal).widen,
            None,
            "표시 없는 보고는 넓히지 않는다"
        );

        book.mark_opening(POPUP);

        assert_eq!(
            plan(&mut book, 2, size(286, 64), Normal),
            MinPlan {
                widen: Some(size(286, 64)),
                set_min: None,
            },
            "같은 값이어도 연 창은 넓혀 본다 — OS 최소 크기는 이미 입혔다"
        );
        assert_eq!(plan(&mut book, 3, size(295, 64), Normal).widen, None);
    }

    #[test]
    fn an_opening_mark_spent_by_a_maximized_report_never_widens() {
        let mut book = ChromeBook::default();
        book.mark_opening(POPUP);

        assert_eq!(plan(&mut book, 1, size(286, 64), Other), MinPlan::default());
        assert_eq!(
            plan(&mut book, 2, size(295, 64), Normal),
            set_only(size(295, 64)),
            "열 때 최대화였던 창은 나중에 원래 크기로 돌아와도 넓혀 당기지 않는다"
        );
    }

    #[test]
    fn a_stale_report_keeps_the_opening_mark_for_the_next_one() {
        let mut book = ChromeBook::default();
        plan(&mut book, 5, size(286, 64), Normal);
        book.mark_opening(POPUP);

        assert_eq!(
            plan(&mut book, 4, size(120, 64), Normal),
            MinPlan::default()
        );
        assert_eq!(
            plan(&mut book, 6, size(286, 64), Normal).widen,
            Some(size(286, 64))
        );
    }

    #[test]
    fn a_failed_min_is_applied_again_by_the_next_frame_change() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);

        book.min_failed(POPUP);

        assert_eq!(book.on_frame(POPUP, Normal), set_only(size(286, 64)));
        book.min_failed("never-seen");
    }

    #[test]
    fn forgetting_a_window_drops_its_opening_mark_and_applied_min() {
        let mut book = ChromeBook::default();
        plan(&mut book, 1, size(286, 64), Normal);
        book.mark_opening(POPUP);
        book.forget(POPUP);

        assert_eq!(
            plan(&mut book, 1, size(286, 64), Normal),
            set_only(size(286, 64)),
            "입힌 값 · 열기 표시를 함께 잊는다"
        );
    }

    #[test]
    fn a_report_without_the_window_neither_plans_nor_spends_anything() {
        let book = Mutex::new(ChromeBook::default());

        assert_eq!(report_min(&book, POPUP, 286, 64, nth(1), None), Ok(None));
        assert_eq!(
            report_min(&book, POPUP, 0, 64, nth(2), None),
            Err(format!(
                "최소 창 크기 0x64 — 둘 다 1~{MIN_SIZE_MAX} 이어야 한다"
            )),
            "범위 검사가 창 확인보다 먼저다"
        );
    }

    // ── apply_min(입히기 차례) ──

    // 부른 일 · 그때 장부 락이 놓여 있었나.
    struct FakeMinWindow<'a> {
        book: &'a Mutex<ChromeBook>,
        calls: RefCell<Vec<(String, bool)>>,
        widen: Result<bool, String>,
        set: Result<(), String>,
    }

    impl<'a> FakeMinWindow<'a> {
        fn new(book: &'a Mutex<ChromeBook>) -> Self {
            Self {
                book,
                calls: RefCell::new(Vec::new()),
                widen: Ok(true),
                set: Ok(()),
            }
        }

        fn log(&self, call: String) {
            let free = self.book.try_lock().is_ok();
            self.calls.borrow_mut().push((call, free));
        }
    }

    impl MinWindow for FakeMinWindow<'_> {
        fn widen_to(&self, min_w: u32) -> Result<bool, String> {
            self.log(format!("widen {min_w}"));
            self.widen.clone()
        }

        fn apply_min_size(&self, min: MinSize) -> Result<(), String> {
            self.log(format!("min {}x{}", min.width, min.height));
            self.set.clone()
        }
    }

    #[test]
    fn widening_goes_before_the_os_min_and_both_run_outside_the_book_lock() {
        let book = Mutex::new(ChromeBook::default());
        let window = FakeMinWindow::new(&book);

        apply_min(
            &book,
            POPUP,
            MinPlan {
                widen: Some(size(286, 64)),
                set_min: Some(size(286, 64)),
            },
            &window,
        );

        assert_eq!(
            window.calls.into_inner(),
            [
                ("widen 286".to_owned(), true),
                ("min 286x64".to_owned(), true)
            ]
        );
    }

    #[test]
    fn a_failed_widen_still_applies_the_os_min() {
        let book = Mutex::new(ChromeBook::default());
        let mut window = FakeMinWindow::new(&book);
        window.widen = Err("모니터를 못 읽었다(시험)".to_owned());

        apply_min(
            &book,
            POPUP,
            MinPlan {
                widen: Some(size(286, 64)),
                set_min: Some(size(286, 64)),
            },
            &window,
        );

        assert_eq!(window.calls.into_inner().len(), 2);
    }

    #[test]
    fn a_failed_os_min_is_forgotten_so_the_next_frame_change_applies_it_again() {
        let book = Mutex::new(ChromeBook::default());
        let planned = {
            let mut book = book.lock().unwrap();
            book.on_min_report(POPUP, nth(1), size(286, 64), Normal)
                .plan
        };
        let mut window = FakeMinWindow::new(&book);
        window.set = Err("창 핸들 무효(시험)".to_owned());

        apply_min(&book, POPUP, planned, &window);

        assert_eq!(
            book.lock().unwrap().on_frame(POPUP, Normal),
            set_only(size(286, 64))
        );
    }

    #[test]
    fn an_empty_plan_touches_nothing() {
        let book = Mutex::new(ChromeBook::default());
        let window = FakeMinWindow::new(&book);

        apply_min(&book, POPUP, MinPlan::default(), &window);

        assert!(window.calls.into_inner().is_empty());
    }
}
