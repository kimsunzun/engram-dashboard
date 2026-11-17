//! Applying a `ui.language` change to the shell-made text — automatic tab names · OS window titles (the layout) and the tray
//! menu. The settings write calls it through `SettingsEvents::ui_language_changed`; the Tauri side supplies the ports. [`current`] is the
//! one Tauri-facing helper here: the effective language for shell text built after boot.
//!
//! - ★One hook at a time, and it reads the value itself★ — [`LanguageHook::apply`] runs under one mutex and reads the effective
//!   setting there instead of trusting the value it was called for. Two writes whose hooks run in the reverse order therefore
//!   both apply the last stored value (the later hook may apply the same value twice — harmless).
//! - ★Order inside: layout (under its lock: set the language, bump, collect the windows) → release → announce → tray★. The tray
//!   is touched only after the layout lock is released.
//! - ★The tray relabel is posted, not waited for★ — the production port hands it to the main thread and returns, so a settings
//!   reply (human command or LLM bus) is never held by a busy main thread. Last value still wins: the posts are made in gate
//!   order, each carries the value read under the gate, and the main thread runs them first in, first out.
//! - The hook is never called on the main thread (ADR-0373) — its callers run on the blocking pool because the write waits for the
//!   disk, and the ordering above relies on it too: on the main thread a post runs in place, ahead of posts already queued.
// ADR-0373

use std::sync::{Arc, Mutex};

use engram_dashboard_base::sync;
use tauri::{Manager, Runtime};

use crate::layout::apply::{self, LayoutEvents};
use crate::layout::LayoutState;
use crate::settings::SettingsService;
use crate::ui_text::Lang;

/// The tray menu's text — production = the four item handles (`tray`), tests = a fake. Failure is the implementation's to log.
pub(crate) trait TrayMenuText: Send + Sync {
    /// Relabels the menu in `lang`, or hands that to the thread that owns the menu without waiting for it — calls must take effect
    /// in call order.
    fn relabel(&self, lang: Lang);
}

/// One per shell (managed in the builder) — every settings write path must reach the same instance, or two hooks run at once.
#[derive(Default)]
pub(crate) struct LanguageHook {
    gate: Mutex<()>,
}

impl LanguageHook {
    /// Applies the effective `ui.language` of `settings` to the layout and the tray. A failed layout lock is logged and the tray
    /// is still relabelled.
    pub(crate) fn apply(
        &self,
        settings: &SettingsService,
        layout: &LayoutState,
        events: &dyn LayoutEvents,
        tray: &dyn TrayMenuText,
    ) {
        let _one_at_a_time = sync::lock(&self.gate);
        let lang = Lang::from_settings(settings);
        if let Err(e) = apply::language_changed(layout, events, lang) {
            tracing::warn!(
                module = "settings",
                lang = lang.code(),
                "could not apply the new UI language to tab names and window titles: {e}"
            );
        }
        tray.relabel(lang);
    }
}

// ADR-0373: the language is read where the text is made, never cached in a global.
/// The effective `ui.language` now — for the shell text built or shown later than boot (the tray at startup, the refusal box when
/// shown, a new window's `<html lang>`). The settings service is managed in the builder, so the default is not reached in production.
pub(crate) fn current<R: Runtime, M: Manager<R>>(manager: &M) -> Lang {
    manager
        .try_state::<Arc<SettingsService>>()
        .map_or_else(Lang::default, |settings| Lang::from_settings(&settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::apply::WindowTabsPayload;
    use crate::layout::{ViewSnapshot, MAIN_WINDOW_LABEL};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::Duration;

    fn settings(tag: &str) -> SettingsService {
        let dir = std::env::temp_dir().join(format!(
            "engram-u3-language-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let svc = SettingsService::load_from_dir(&dir);
        svc.enable_writes();
        svc
    }

    #[derive(Default)]
    struct Titles(Mutex<Vec<(String, String, u64)>>);

    impl LayoutEvents for Titles {
        fn layout_updated(&self, _snapshot: &ViewSnapshot) {}
        fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
            sync::lock(&self.0).push((tabs.label.clone(), tabs.title.clone(), tabs.version));
        }
    }

    /// Records each relabel and whether the layout lock was free at that moment. The first call can be held: it signals
    /// `entered` and waits for `go` before it records.
    struct Tray {
        layout: LayoutState,
        seen: Mutex<Vec<(Lang, bool)>>,
        hold_first: Mutex<Option<(Sender<()>, Receiver<()>)>>,
    }

    impl Tray {
        fn new(layout: &LayoutState) -> Self {
            Tray {
                layout: layout.clone(),
                seen: Mutex::default(),
                hold_first: Mutex::default(),
            }
        }

        fn langs(&self) -> Vec<Lang> {
            sync::lock(&self.seen)
                .iter()
                .map(|(lang, _)| *lang)
                .collect()
        }

        fn always_after_the_layout_lock(&self) -> bool {
            sync::lock(&self.seen).iter().all(|(_, free)| *free)
        }
    }

    impl TrayMenuText for Tray {
        fn relabel(&self, lang: Lang) {
            let layout_free = self.layout.0.try_lock().is_ok();
            let hold = sync::lock(&self.hold_first).take();
            if let Some((entered, go)) = hold {
                entered.send(()).unwrap();
                go.recv().unwrap();
            }
            sync::lock(&self.seen).push((lang, layout_free));
        }
    }

    fn main_title(layout: &LayoutState) -> String {
        let mgr = sync::lock(&layout.0);
        WindowTabsPayload::from(mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap()).title
    }

    #[test]
    fn the_hook_applies_the_stored_language_to_the_layout_and_the_tray() {
        let svc = settings("apply");
        let layout = LayoutState::new();
        let (titles, tray) = (Titles::default(), Tray::new(&layout));
        let hook = LanguageHook::default();

        svc.set("ui.language", "ko").unwrap();
        hook.apply(&svc, &layout, &titles, &tray);

        assert_eq!(sync::lock(&layout.0).language(), Lang::Ko);
        assert_eq!(main_title(&layout), "빈 칸 - Engram:1");
        assert_eq!(tray.langs(), [Lang::Ko]);
        assert!(
            tray.always_after_the_layout_lock(),
            "the tray is touched only after the layout lock is released"
        );
        let announced = sync::lock(&titles.0).clone();
        assert_eq!(
            announced,
            [(
                MAIN_WINDOW_LABEL.to_string(),
                "빈 칸 - Engram:1".to_string(),
                1
            )]
        );
    }

    // The layout starts in Korean. Write A (ko) then write B (en) — B's hook runs first, A's last. Had a hook applied its own
    //   write's value, the end would be A's Korean; reading the stored value ends on B's English.
    #[test]
    fn hooks_that_run_in_reverse_order_end_on_the_last_stored_value() {
        let svc = settings("reverse");
        let layout = LayoutState::new();
        sync::lock(&layout.0).set_language(Lang::Ko);
        let (titles, tray) = (Titles::default(), Tray::new(&layout));
        let hook = LanguageHook::default();

        svc.set("ui.language", "ko").unwrap();
        svc.set("ui.language", "en").unwrap();
        hook.apply(&svc, &layout, &titles, &tray); // B's hook
        hook.apply(&svc, &layout, &titles, &tray); // A's hook, late

        assert_eq!(sync::lock(&layout.0).language(), Lang::En);
        assert_eq!(main_title(&layout), "Empty slot - Engram:1");
        assert_eq!(tray.langs(), [Lang::En, Lang::En]);
        assert!(tray.always_after_the_layout_lock());
        let titles = sync::lock(&titles.0).clone();
        assert_eq!(
            titles.len(),
            1,
            "only the first hook changed a name: {titles:?}"
        );
    }

    // Hook A (ko) is held inside the tray step, inside the gate. Then en is stored and hook B starts on another thread. B must
    //   wait for A: without the gate B would run to the end now, and A's late Korean relabel would land after B's English.
    // The bounded wait only limits detection — correct code passes whatever the timing.
    #[test]
    fn a_second_hook_waits_for_the_first_so_the_tray_ends_on_the_stored_value() {
        let svc = Arc::new(settings("serial"));
        let layout = LayoutState::new();
        let titles = Arc::new(Titles::default());
        let tray = Arc::new(Tray::new(&layout));
        let hook = Arc::new(LanguageHook::default());
        let (entered_tx, entered) = mpsc::channel();
        let (go, go_rx) = mpsc::channel();
        *sync::lock(&tray.hold_first) = Some((entered_tx, go_rx));

        svc.set("ui.language", "ko").unwrap();
        let a = {
            let (svc, layout, titles, tray, hook) = (
                svc.clone(),
                layout.clone(),
                titles.clone(),
                tray.clone(),
                hook.clone(),
            );
            std::thread::spawn(move || hook.apply(&svc, &layout, titles.as_ref(), tray.as_ref()))
        };
        entered.recv().unwrap();

        svc.set("ui.language", "en").unwrap();
        let (b_done_tx, b_done) = mpsc::channel();
        let b = {
            let (svc, layout, titles, tray, hook) = (
                svc.clone(),
                layout.clone(),
                titles.clone(),
                tray.clone(),
                hook.clone(),
            );
            std::thread::spawn(move || {
                hook.apply(&svc, &layout, titles.as_ref(), tray.as_ref());
                b_done_tx.send(()).unwrap();
            })
        };
        assert!(
            b_done.recv_timeout(Duration::from_millis(500)).is_err(),
            "hook B finished while hook A was still inside the gate"
        );

        go.send(()).unwrap();
        a.join().unwrap();
        b.join().unwrap();

        assert_eq!(tray.langs(), [Lang::Ko, Lang::En]);
        assert!(tray.always_after_the_layout_lock());
        assert_eq!(sync::lock(&layout.0).language(), Lang::En);
        assert_eq!(main_title(&layout), "Empty slot - Engram:1");
    }
}
