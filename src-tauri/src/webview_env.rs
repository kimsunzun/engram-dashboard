//! 웹뷰 환경 — 이 셸 프로세스의 WebView 창이 쓰는 데이터 폴더와 브라우저 인자([`WebviewEnv`]) (TRD S21-storage §4).
//!
//! - ★같은 데이터 폴더의 창은 환경 옵션이 같아야 한다★ — 어긴 창은 `build()` 가 `Ok` 인데 OS 창이 안 생긴다
//!   (ADR-0054 의 유령 창). 그래서 창을 만드는 코드는 전부 [`WebviewEnv::finish`] 를 지나고, 그 값은 셸에 하나인
//!   인스턴스에서 나온다. 설정이 선언한 창(main · agent-tree)도 예외가 아니다 — Tauri 가 만들지 않게 두고
//!   (`tauri.conf.json` 의 `"create": false`) 부팅 단계 ⑧ 이 `from_config` 로 만들어 이 마무리에 넘긴다
//!   (`state::placement`).
//! - ★환경 옵션 넷(폴더 · 브라우저 인자 · 확장 · 스크롤바 모양)을 마무리가 정한다★ — wry 0.55.1 `webview2` 의
//!   `create_environment` 가 환경에 싣는 것이 이 넷이다. 셋은 늘 같은 값으로 박는다. 폴더만 빌더가 지우지 못해,
//!   기본 폴더로 물러날 때 설정 창 선언이 적은 `dataDirectory` 가 남는다 — 그래서 이 모듈 시험이 설정을 본다.
//! - ★폴더는 프로세스당 한 번 정한다★ — `<데이터 루트>\webview` 에 쓸 수 있으면 그 폴더, 못 쓰면 폴더를 넘기지 않아
//!   Tauri 기본 폴더(`LocalData/<identifier>`)로 물러나고 warn 한다(멈추지 않는다 — TRD §6 D8). 창마다 다시 묻지
//!   않는다: 창 사이에 답이 바뀌면 한 프로세스의 창들이 다른 폴더를 받는다. 데이터 루트 전체를 못 쓰는 경우의 사유는
//!   창이 뜬 뒤 데몬 연결 실패로 창 안에 보인다(ADR-0134 결정 4 · ADR-0135).
//! - ★확인하는 폴더와 넘기는 폴더는 같은 절대 경로다★ — 데이터 루트는 상대 경로일 수 있다(`ENGRAM_DATA_DIR` 을 그대로
//!   쓴다). WebView2 가 상대 경로를 무엇에 대어 푸는지는 확인하지 않았다 — 상대 경로를 넘기면 확인한 폴더와 쓰는
//!   폴더가 갈릴 수 있다.
// ADR-0054

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tauri::webview::ScrollBarStyle;
use tauri::{Manager, Runtime, WebviewWindowBuilder};

use crate::discovery::DataLayout;

/// WebView2 브라우저 인자 — 이 값이 유일한 출처다. 설정 창도 이 값을 받으므로 `tauri.conf.json` 에는 적지 않는다
/// (적으면 이 모듈 시험이 걸린다 — 설정 JSON 은 주석을 못 달아 사유를 여기 둔다).
///
/// ★wry 기본값(`msSmartScreenProtection` 까지 끄는 것)으로 바꾸지 말 것★ — 불변식은 「표준값」이 아니라 「같은 폴더
/// 안에서 서로 같다」이다(ADR-0054 거부한 대안).
pub(crate) const WEBVIEW2_BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI --autoplay-policy=no-user-gesture-required";

type WriteCheck = fn(&Path) -> Result<(), String>;

/// 이 프로세스의 웹뷰 환경 — 셸에 하나다(`Arc` 로 나눠 쥔다 · 빌더에서 manage).
///
/// ★`Clone` 을 달지 말 것★ — 결정 칸이 둘로 갈리면 사본마다 따로 정해 창마다 다른 폴더를 받을 수 있다.
pub(crate) struct WebviewEnv {
    dir: PathBuf,
    check: WriteCheck,
    chosen: OnceLock<Option<PathBuf>>,
}

impl WebviewEnv {
    pub(crate) fn resolve() -> Self {
        Self::new(DataLayout::resolve().webview_dir(), fs_check)
    }

    fn new(dir: PathBuf, check: WriteCheck) -> Self {
        Self {
            dir,
            check,
            chosen: OnceLock::new(),
        }
    }

    /// 창에 넘길 데이터 폴더(절대 경로) — `None` = 넘기지 않는다(Tauri 기본 폴더). 처음 부를 때 쓰기 확인을 한 번 하고,
    /// 그 뒤로는 같은 답을 돌려준다(동시에 불려도 확인은 한 번이다).
    pub(crate) fn chosen_dir(&self) -> Option<&Path> {
        self.chosen
            .get_or_init(|| choose(&self.dir, self.check))
            .as_deref()
    }

    /// 창 빌더에 웹뷰 환경 옵션(폴더 · 브라우저 인자 · 확장 · 스크롤바 모양)을 붙인다 — 창을 만드는 모든 코드가
    /// `build()` 앞에서 지난다. 앞서 빌더에 붙은 값(설정 창이 `from_config` 로 받은 것 포함)은 덮는다 — 폴더만 예외다
    /// (모듈 머리).
    ///
    /// 폴더는 OS 를 가리지 않고 넘긴다 — 그 값을 쓰느냐는 Tauri 의 플랫폼 구현 몫이다(CLAUDE.md 「플랫폼 중립」).
    pub(crate) fn finish<'a, R: Runtime, M: Manager<R>>(
        &self,
        builder: WebviewWindowBuilder<'a, R, M>,
    ) -> WebviewWindowBuilder<'a, R, M> {
        let builder = builder
            .additional_browser_args(WEBVIEW2_BROWSER_ARGS)
            .browser_extensions_enabled(false)
            .scroll_bar_style(ScrollBarStyle::Default);
        match self.chosen_dir() {
            Some(dir) => builder.data_directory(dir.to_path_buf()),
            None => builder,
        }
    }
}

// 절대 경로로 바꾼 뒤 확인한다(모듈 머리) — 바꾸지 못하면 못 쓰는 폴더와 같이 물러난다.
fn choose(dir: &Path, check: impl FnOnce(&Path) -> Result<(), String>) -> Option<PathBuf> {
    let (dir, checked) = match std::path::absolute(dir) {
        Ok(absolute) => {
            let checked = check(&absolute);
            (absolute, checked)
        }
        Err(e) => (
            dir.to_path_buf(),
            Err(format!("절대 경로로 바꾸지 못했다: {e}")),
        ),
    };
    match checked {
        Ok(()) => {
            tracing::info!(module = "webview_env", dir = %dir.display(), "웹뷰 데이터 폴더 결정");
            Some(dir)
        }
        Err(reason) => {
            tracing::warn!(
                module = "webview_env",
                dir = %dir.display(),
                %reason,
                "웹뷰 데이터 폴더에 쓸 수 없어 Tauri 기본 폴더로 창을 만든다"
            );
            None
        }
    }
}

/// 폴더를 만들지 않는다(만들어 봤으면 되돌린다) — 폴더는 창을 만들 때 Tauri 가 만든다.
fn fs_check(dir: &Path) -> Result<(), String> {
    crate::discovery::check_data_dir_writable(dir).map_err(webview_reason)
}

// 실패 사유만 꺼낸다 — 그 오류의 문구(「쓰기 가능한 위치에 압축을 풀어 주세요」)는 데이터 루트를 두고 쓴 조치라 웹뷰 폴더에
//   맞지 않는다. 경로는 로그의 `dir` 칸이 싣는다.
fn webview_reason(e: crate::discovery::DiscoveryError) -> String {
    match e {
        crate::discovery::DiscoveryError::DataDirUnwritable { reason, .. } => reason,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tauri::utils::config::{ScrollBarStyle, WindowConfig};

    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engram-webview-env-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ── 결정 ──

    #[test]
    fn a_writable_dir_is_chosen() {
        let dir = std::env::temp_dir().join("webview");
        let mut seen = None;
        let chosen = choose(&dir, |dir| {
            seen = Some(dir.to_path_buf());
            Ok(())
        });
        assert_eq!(chosen, Some(dir.clone()));
        assert_eq!(seen, Some(dir));
    }

    #[test]
    fn an_unwritable_dir_falls_back_to_the_default() {
        assert_eq!(
            choose(Path::new("R/webview"), |_| Err("원인".to_owned())),
            None
        );
    }

    #[test]
    fn a_relative_dir_is_checked_and_handed_over_as_the_same_absolute_path() {
        let mut seen = None;
        let chosen = choose(Path::new("R/webview"), |dir| {
            seen = Some(dir.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert!(chosen.is_absolute(), "{}", chosen.display());
        assert!(chosen.ends_with("R/webview"), "{}", chosen.display());
        assert_eq!(seen, Some(chosen), "확인한 폴더를 그대로 넘긴다");
    }

    #[test]
    fn a_dir_that_cannot_be_made_absolute_falls_back_without_a_check() {
        assert_eq!(choose(Path::new(""), |_| panic!("확인하지 않는다")), None);
    }

    #[test]
    fn the_choice_is_made_once_and_every_window_gets_it() {
        static CHECKS: AtomicUsize = AtomicUsize::new(0);
        fn counting_fail(_: &Path) -> Result<(), String> {
            CHECKS.fetch_add(1, Ordering::SeqCst);
            Err("원인".to_owned())
        }
        let env = WebviewEnv::new(PathBuf::from("R/webview"), counting_fail);
        let answers: Vec<_> = (0..3)
            .map(|_| env.chosen_dir().map(Path::to_path_buf))
            .collect();
        assert_eq!(answers, vec![None, None, None]);
        assert_eq!(CHECKS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn concurrent_first_calls_check_once() {
        static CHECKS: AtomicUsize = AtomicUsize::new(0);
        fn counting_ok(_: &Path) -> Result<(), String> {
            CHECKS.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(20));
            Ok(())
        }
        let dir = std::env::temp_dir().join("webview");
        let env = WebviewEnv::new(dir.clone(), counting_ok);
        let answers: Vec<_> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..4)
                .map(|_| s.spawn(|| env.chosen_dir().map(Path::to_path_buf)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(answers.iter().all(|a| a.as_deref() == Some(dir.as_path())));
        assert_eq!(CHECKS.load(Ordering::SeqCst), 1);
    }

    // ── 운영 확인(discovery) 그대로 ──

    #[test]
    fn real_check_chooses_an_existing_dir() {
        let root = scratch("ok");
        let dir = root.join("webview");
        std::fs::create_dir(&dir).unwrap();
        assert_eq!(
            WebviewEnv::new(dir.clone(), fs_check).chosen_dir(),
            Some(dir.as_path())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_fallback_reason_is_the_cause_without_the_data_root_advice() {
        let reason = webview_reason(crate::discovery::DiscoveryError::DataDirUnwritable {
            path: "R/webview".into(),
            reason: "접근이 거부되었다(시험)".into(),
        });
        assert_eq!(reason, "접근이 거부되었다(시험)");
    }

    #[test]
    fn real_check_falls_back_when_a_file_sits_in_the_folder_spot() {
        let root = scratch("file");
        let dir = root.join("webview");
        std::fs::write(&dir, b"not a folder").unwrap();
        assert_eq!(WebviewEnv::new(dir, fs_check).chosen_dir(), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── 설정 창의 환경 옵션 ──

    fn environment_options(
        window: &WindowConfig,
    ) -> (Option<&str>, Option<&Path>, bool, &ScrollBarStyle) {
        (
            window.additional_browser_args.as_deref(),
            window.data_directory.as_deref(),
            window.browser_extensions_enabled,
            &window.scroll_bar_style,
        )
    }

    // 폴더 칸은 이 시험이 유일한 벽이다 — 설정 창 선언이 `dataDirectory` 를 적으면 마무리가 기본 폴더로 물러날 때 그 값을
    //   지우지 못해 설정 창만 다른 폴더를 받는다. 나머지 셋은 마무리가 덮어 갈리지 않지만, 적힌 값이 실제와 다르게 읽히지
    //   않게(출처 하나) 함께 기본값을 요구한다.
    #[test]
    fn config_windows_leave_every_environment_option_at_the_default() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let defaults = WindowConfig::default();
        for value in conf["app"]["windows"].as_array().unwrap() {
            let window: WindowConfig = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(
                environment_options(&window),
                environment_options(&defaults),
                "창 {}",
                window.label
            );
        }
    }
}
