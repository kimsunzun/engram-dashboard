//! The native approval dialog — one question, three choices (approve / deny / deny and stop asking), deny the default, approve and the
//!   third choice enabled only a second after the dialog appears (it shows topmost without the focus, so a click aimed beneath must not
//!   act) — shown with [`ask_approval`], closable from another thread through a [`DialogCloser`]. All text comes from the caller; this module only shows it.
//!
//! Windows = a Task Dialog (`TaskDialogIndirect`). ★It is loaded at run time from comctl32 v6 under an activation context★ — the
//!   function exists only in comctl32 v6, which a process gets only through a manifest; the daemon and the test executables carry none, and
//!   a static import would stop every such executable before `main` with 0xC0000139 (the shell's `src-tauri/build.rs` records that
//!   failure). Chosen over a crate (`rfd`): `rfd` imports the function statically (same failure), always makes the first button the
//!   default, and has neither topmost nor a way to close the dialog from outside.
//!
//! Windows only for now: outside Windows [`ask_approval`] is `Unsupported` without trying (no OS code); the behaviour defined for other
//!   OSes (a native dialog where a display exists, [`ApprovalAnswer::NoDesktop`] otherwise) is built with T-118 / T-119.

// ADR-0266, ADR-0394

use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(windows)]
use windows::core::{HRESULT, PCWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, S_OK, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::Controls::{
    TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TASKDIALOG_NOTIFICATIONS, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_CALLBACK_TIMER, TDF_NO_SET_FOREGROUND, TDM_ENABLE_BUTTON, TDN_CREATED, TDN_DESTROYED,
    TDN_TIMER,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SendMessageW, SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    WM_CLOSE,
};

#[cfg(windows)]
use crate::win::to_wide;

/// The text of one question. Shown as given: no hyperlinks are enabled, so markup such as `<a href=…>` stays literal text. In the three
///   button labels `&` marks the access key (the Win32 button rule) — write `&&` for a literal `&`.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalQuestion<'a> {
    /// The window caption.
    pub title: &'a str,
    /// The main instruction — the bold line at the top.
    pub heading: &'a str,
    /// The text under it.
    pub body: &'a str,
    pub approve: &'a str,
    pub deny: &'a str,
    pub deny_and_stop: &'a str,
}

/// How a question ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAnswer {
    Approve,
    Deny,
    DenyAndStop,
    /// The user closed the window without choosing (Esc, the close box, Alt+F4). What that means is the caller's decision.
    Closed,
    /// [`DialogCloser::dismiss`] was called before the dialog opened or while it was open. A dismissal that lands before the answer is
    ///   read wins over a click; one that lands just after cannot recall it, so a click racing `dismiss` can still come back as a choice
    ///   — ★the caller's own atomic consume of the request is what keeps a stale answer from applying★.
    Dismissed,
    /// This process has no desktop to show a window on (Windows: session 0, or a window station that is not interactive); nothing was
    ///   shown.
    NoDesktop,
}

/// Closes one open dialog from another thread. One closer per question: once dismissed it stays dismissed, and a dialog started with a
///   dismissed closer is not shown.
#[derive(Debug, Clone, Default)]
pub struct DialogCloser {
    inner: Arc<Mutex<CloserState>>,
}

#[derive(Debug, Default)]
struct CloserState {
    /// The open dialog's window (`HWND` as an integer); 0 while none is open.
    window: isize,
    dismissed: bool,
    /// The acting buttons (Approve, the third choice) were enabled after [`APPROVE_ARM_MS`].
    approve_armed: bool,
}

impl DialogCloser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ends the question with [`ApprovalAnswer::Dismissed`]: closes the dialog if it is open, keeps it from opening if it is not yet.
    ///   `Err` = the close message could not be posted, so a dialog may stay open (the caller marks it stale); the question still ends
    ///   `Dismissed` once the dialog does end.
    pub fn dismiss(&self) -> io::Result<()> {
        let mut state = self.state();
        state.dismissed = true;
        #[cfg(windows)]
        if state.window != 0 {
            return post_close(state.window);
        }
        Ok(())
    }

    fn is_dismissed(&self) -> bool {
        self.state().dismissed
    }

    fn state(&self) -> MutexGuard<'_, CloserState> {
        // The state is two plain fields written in one step each; a panic elsewhere cannot leave it half-written.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Shows `question` and blocks until it ends — run it off any async runtime, one at a time. Topmost without taking the focus where the OS
///   allows it: the window is raised without activation, and the foreground lock keeps a background process from taking the focus (the
///   same lock can also leave it behind the window in use). `Err` = it could not be shown (a NUL inside a text is `InvalidInput`; loading
///   comctl32 v6 or the dialog itself failed). Outside Windows = `Unsupported` (module head).
#[cfg(windows)]
pub fn ask_approval(
    question: &ApprovalQuestion<'_>,
    closer: &DialogCloser,
) -> io::Result<ApprovalAnswer> {
    let prepared = Prepared::new(question)?;
    ask_with(closer, has_desktop, || show(&prepared, closer))
}
#[cfg(not(windows))]
pub fn ask_approval(
    _question: &ApprovalQuestion<'_>,
    _closer: &DialogCloser,
) -> io::Result<ApprovalAnswer> {
    Err(io::ErrorKind::Unsupported.into())
}

const APPROVE_ID: i32 = 100;
const DENY_ID: i32 = 101;
const DENY_AND_STOP_ID: i32 = 102;
/// Approve and the third choice stay disabled this long after the dialog appears — it shows topmost without taking the focus, so a
///   click meant for the window beneath must not land on a button that acts (Deny, the default, is enabled at once).
#[cfg_attr(not(windows), allow(dead_code))]
const APPROVE_ARM_MS: usize = 1000;
/// What the dialog returns when it is cancelled — Esc, the close box, Alt+F4, or the close message [`DialogCloser::dismiss`] sends
///   (Win32 `IDCANCEL`).
const CANCEL_ID: i32 = 2;

/// The order of [`ask_approval`] with the desktop check and the showing passed in — the seam the tests drive without a window.
#[cfg_attr(not(windows), allow(dead_code))]
fn ask_with(
    closer: &DialogCloser,
    has_desktop: impl FnOnce() -> io::Result<bool>,
    show: impl FnOnce() -> io::Result<i32>,
) -> io::Result<ApprovalAnswer> {
    if !has_desktop()? {
        return Ok(ApprovalAnswer::NoDesktop);
    }
    if closer.is_dismissed() {
        return Ok(ApprovalAnswer::Dismissed);
    }
    let button = show()?;
    answer_of(button, closer.is_dismissed())
}

#[cfg_attr(not(windows), allow(dead_code))]
fn answer_of(button: i32, dismissed: bool) -> io::Result<ApprovalAnswer> {
    if dismissed {
        return Ok(ApprovalAnswer::Dismissed);
    }
    match button {
        APPROVE_ID => Ok(ApprovalAnswer::Approve),
        DENY_ID => Ok(ApprovalAnswer::Deny),
        DENY_AND_STOP_ID => Ok(ApprovalAnswer::DenyAndStop),
        CANCEL_ID => Ok(ApprovalAnswer::Closed),
        other => Err(io::Error::other(format!(
            "the approval dialog returned an unknown button id {other}"
        ))),
    }
}

/// The question's texts as null-terminated wide strings, kept alive while the dialog runs.
#[cfg(windows)]
struct Prepared {
    title: Vec<u16>,
    heading: Vec<u16>,
    body: Vec<u16>,
    labels: [(i32, Vec<u16>); 3],
}

#[cfg(windows)]
impl Prepared {
    fn new(q: &ApprovalQuestion<'_>) -> io::Result<Self> {
        let w = |s: &str| to_wide(s.as_ref());
        Ok(Self {
            title: w(q.title)?,
            heading: w(q.heading)?,
            body: w(q.body)?,
            labels: [
                (APPROVE_ID, w(q.approve)?),
                (DENY_ID, w(q.deny)?),
                (DENY_AND_STOP_ID, w(q.deny_and_stop)?),
            ],
        })
    }

    fn buttons(&self) -> [TASKDIALOG_BUTTON; 3] {
        self.labels.each_ref().map(|(id, label)| TASKDIALOG_BUTTON {
            nButtonID: *id,
            pszButtonText: PCWSTR(label.as_ptr()),
        })
    }

    /// The dialog's configuration. No hyperlink flag (texts stay literal); cancellation allowed, so the user can close it and
    ///   [`DialogCloser`] can end it with a close message; the dialog does not ask to be brought to the foreground; `state` reaches
    ///   [`callback`] as its reference data.
    fn config(
        &self,
        buttons: &[TASKDIALOG_BUTTON; 3],
        state: *const Mutex<CloserState>,
    ) -> TASKDIALOGCONFIG {
        TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_NO_SET_FOREGROUND | TDF_CALLBACK_TIMER,
            pszWindowTitle: PCWSTR(self.title.as_ptr()),
            pszMainInstruction: PCWSTR(self.heading.as_ptr()),
            pszContent: PCWSTR(self.body.as_ptr()),
            cButtons: buttons.len() as u32,
            pButtons: buttons.as_ptr(),
            nDefaultButton: DENY_ID,
            pfCallback: Some(callback),
            lpCallbackData: state as isize,
            ..Default::default()
        }
    }
}

/// Session 0 never has a user's desktop (services; possibly a daemon started outside the user's session), and a window station without
///   `WSF_VISIBLE` is not interactive.
#[cfg(windows)]
fn has_desktop() -> io::Result<bool> {
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows::Win32::System::StationsAndDesktops::{
        GetProcessWindowStation, GetUserObjectInformationW, UOI_FLAGS, USEROBJECTFLAGS,
    };
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::WSF_VISIBLE;

    use crate::win::io_error;

    let mut session: u32 = 0;
    // SAFETY: this process's id and an output u32 on the stack.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }.map_err(io_error)?;
    if session == 0 {
        return Ok(false);
    }
    // SAFETY: the returned window station handle is the process's own and must not be closed; the flags struct is on the stack with its
    //   size passed.
    let flags = unsafe {
        let station = GetProcessWindowStation().map_err(io_error)?;
        let mut flags = USEROBJECTFLAGS::default();
        GetUserObjectInformationW(
            windows::Win32::Foundation::HANDLE(station.0),
            UOI_FLAGS,
            Some(std::ptr::addr_of_mut!(flags).cast()),
            std::mem::size_of::<USEROBJECTFLAGS>() as u32,
            None,
        )
        .map_err(io_error)?;
        flags
    };
    Ok(flags.dwFlags & WSF_VISIBLE as u32 != 0)
}

#[cfg(windows)]
fn show(prepared: &Prepared, closer: &DialogCloser) -> io::Result<i32> {
    let buttons = prepared.buttons();
    let config = prepared.config(&buttons, Arc::as_ptr(&closer.inner));
    let task_dialog = comctl6::TaskDialogIndirect::load()?;
    let mut button: i32 = 0;
    // SAFETY: `config` and everything it points at (`prepared`, `buttons`, the closer state behind its `Arc`) outlive the blocking call.
    let outcome = unsafe { task_dialog.call(&config, &mut button) };
    // The window is gone once the call returns, whether or not TDN_DESTROYED arrived — a later `dismiss` must not post to its handle.
    closer.state().window = 0;
    outcome.map(|()| button)
}

/// Keeps Approve and the third choice disabled for [`APPROVE_ARM_MS`] (the dialog's timer enables them), raises the window topmost without activating it once
///   it exists, and records it for [`DialogCloser::dismiss`] — closing it at once if
///   the dismissal came first. ★It runs on the dialog's thread inside the OS's message loop★, so it neither panics nor returns errors; a
///   failed raise or close is only logged (header rule 4: no way to return it).
#[cfg(windows)]
unsafe extern "system" fn callback(
    window: HWND,
    notification: TASKDIALOG_NOTIFICATIONS,
    wparam: WPARAM,
    _lparam: LPARAM,
    data: isize,
) -> HRESULT {
    // SAFETY: `data` is the `Arc`'s pointer `show` passed, alive for the whole dialog.
    let state = unsafe { &*(data as *const Mutex<CloserState>) };
    let lock = || {
        state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    };
    if notification == TDN_CREATED {
        // Every button but the default Deny starts disabled — Approve and the third choice may act (an asker can make the third
        //   one "start fresh"); the timer below enables them after `APPROVE_ARM_MS`.
        enable_acting_buttons(window, false);
        // Outside the lock: a window call can wait on other windows, and `dismiss` must not wait behind it.
        // SAFETY: the dialog's own live window.
        if let Err(e) = unsafe {
            SetWindowPos(
                window,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        } {
            tracing::warn!(error = %e, "could not raise the approval dialog topmost");
        }
        let failed_close = {
            let mut state = lock();
            state.window = window.0 as isize;
            if state.dismissed {
                post_close(state.window).err()
            } else {
                None
            }
        };
        if let Some(e) = failed_close {
            tracing::warn!(error = %e, "could not close the dismissed approval dialog");
        }
    } else if notification == TDN_TIMER {
        // `wparam` = milliseconds since the dialog was created (or the timer last reset — never here).
        let arm = wparam.0 >= APPROVE_ARM_MS && {
            let mut state = lock();
            !std::mem::replace(&mut state.approve_armed, true)
        };
        if arm {
            enable_acting_buttons(window, true);
        }
    } else if notification == TDN_DESTROYED {
        lock().window = 0;
    }
    S_OK
}

/// Enables or disables the two buttons that may act (Approve and the third choice); Deny, the default, is always enabled.
#[cfg(windows)]
fn enable_acting_buttons(window: HWND, enable: bool) {
    for id in [APPROVE_ID, DENY_AND_STOP_ID] {
        // SAFETY: the dialog's own live window (called from its callback); the message is the Task Dialog's own.
        unsafe {
            SendMessageW(
                window,
                TDM_ENABLE_BUTTON.0 as u32,
                WPARAM(id as usize),
                LPARAM(isize::from(enable)),
            )
        };
    }
}

/// Asks the dialog to close — the close message cancels it like Alt+F4. Called with the closer's lock held, so the window cannot be
///   destroyed and its handle reused in between (the dialog's thread clears it under the same lock).
#[cfg(windows)]
fn post_close(window: isize) -> io::Result<()> {
    // SAFETY: posting to a window handle is safe even if it became invalid; the lock (caller) keeps it current.
    unsafe { PostMessageW(HWND(window as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0)) }
        .map_err(crate::win::io_error)
}

/// `TaskDialogIndirect` from comctl32 v6, found at run time (module head: why not a static import).
#[cfg(windows)]
mod comctl6 {
    use std::io;

    use windows::core::{s, w, HRESULT, PCWSTR};
    use windows::Win32::Foundation::{FreeLibrary, BOOL, HANDLE, HMODULE};
    use windows::Win32::System::ApplicationInstallationAndServicing::{
        ActivateActCtx, CreateActCtxW, DeactivateActCtx, ReleaseActCtx, ACTCTXW,
    };
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::Win32::UI::Controls::TASKDIALOGCONFIG;

    use crate::win::{io_error, to_wide};

    /// Binds the process to comctl32 v6 the way an executable manifest would (the form .NET WinForms writes for the same purpose).
    const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

    type TaskDialogIndirectFn = unsafe extern "system" fn(
        *const TASKDIALOGCONFIG,
        *mut i32,
        *mut i32,
        *mut BOOL,
    ) -> HRESULT;

    /// The loaded function with everything that keeps it valid. Drop order = field order: the library is freed, then the context
    ///   deactivated (same thread — the type is not `Send`), then released.
    pub(super) struct TaskDialogIndirect {
        function: TaskDialogIndirectFn,
        _library: Library,
        _activation: Activation,
        _context: Context,
    }

    impl TaskDialogIndirect {
        pub(super) fn load() -> io::Result<Self> {
            let context = Context::create()?;
            let activation = Activation::enter(&context)?;
            // SAFETY: a constant null-terminated name, resolved under the activation context entered above → the v6 side-by-side copy.
            let library = Library(unsafe { LoadLibraryW(w!("comctl32.dll")) }.map_err(io_error)?);
            // SAFETY: a constant null-terminated name in the module just loaded.
            let found = unsafe { GetProcAddress(library.0, s!("TaskDialogIndirect")) }.ok_or_else(
                || {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "comctl32 has no TaskDialogIndirect — version 6 was not loaded",
                    )
                },
            )?;
            // SAFETY: the documented signature of TaskDialogIndirect.
            let function = unsafe {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, TaskDialogIndirectFn>(
                    found,
                )
            };
            Ok(Self {
                function,
                _library: library,
                _activation: activation,
                _context: context,
            })
        }

        /// # Safety
        /// `config` and everything it points at stay valid for the whole call.
        pub(super) unsafe fn call(
            &self,
            config: &TASKDIALOGCONFIG,
            button: &mut i32,
        ) -> io::Result<()> {
            // SAFETY: the caller's contract; the radio-button and verification outputs are not asked for.
            unsafe { (self.function)(config, button, std::ptr::null_mut(), std::ptr::null_mut()) }
                .ok()
                .map_err(io_error)
        }
    }

    struct Context(HANDLE);

    impl Context {
        /// `CreateActCtxW` reads a manifest from a file, so it is written to a fresh temp file and deleted once the context exists.
        fn create() -> io::Result<Self> {
            let file = TempManifest::write()?;
            let source = to_wide(file.0.as_os_str())?;
            let request = ACTCTXW {
                cbSize: std::mem::size_of::<ACTCTXW>() as u32,
                lpSource: PCWSTR(source.as_ptr()),
                ..Default::default()
            };
            // SAFETY: `request` and the path it points at outlive the call.
            let handle = unsafe { CreateActCtxW(&request) }.map_err(io_error)?;
            Ok(Self(handle))
        }
    }

    impl Drop for Context {
        fn drop(&mut self) {
            // SAFETY: the context created in `create`, released exactly once.
            unsafe { ReleaseActCtx(self.0) };
        }
    }

    struct Activation(usize);

    impl Activation {
        fn enter(context: &Context) -> io::Result<Self> {
            let mut cookie = 0usize;
            // SAFETY: a live context; the cookie is on the stack.
            unsafe { ActivateActCtx(context.0, &mut cookie) }.map_err(io_error)?;
            Ok(Self(cookie))
        }
    }

    impl Drop for Activation {
        fn drop(&mut self) {
            // SAFETY: the cookie `enter` got on this thread, deactivated once.
            if let Err(e) = unsafe { DeactivateActCtx(0, self.0) } {
                tracing::warn!(error = %e, "could not deactivate the comctl32 v6 activation context");
            }
        }
    }

    struct Library(HMODULE);

    impl Drop for Library {
        fn drop(&mut self) {
            // SAFETY: the module `load` loaded, freed once.
            if let Err(e) = unsafe { FreeLibrary(self.0) } {
                tracing::warn!(error = %e, "could not free comctl32");
            }
        }
    }

    struct TempManifest(std::path::PathBuf);

    impl TempManifest {
        /// Written owner-only: the temp folder can be shared, and a manifest another account could rewrite before `CreateActCtxW` reads
        ///   it would choose which DLLs this process loads.
        fn write() -> io::Result<Self> {
            use std::io::Write;
            let path = std::env::temp_dir().join(format!(
                "engram-comctl6-{}-{}.manifest",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos())
            ));
            let mut file = crate::owner_only::create_owner_only(&path)?;
            let manifest = Self(path);
            file.write_all(MANIFEST.as_bytes())?;
            Ok(manifest)
        }
    }

    impl Drop for TempManifest {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_file(&self.0) {
                tracing::warn!(path = %self.0.display(), error = %e, "could not delete the temporary comctl32 manifest");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_button_maps_to_its_answer_and_cancel_is_closed() {
        assert_eq!(
            answer_of(APPROVE_ID, false).unwrap(),
            ApprovalAnswer::Approve
        );
        assert_eq!(answer_of(DENY_ID, false).unwrap(), ApprovalAnswer::Deny);
        assert_eq!(
            answer_of(DENY_AND_STOP_ID, false).unwrap(),
            ApprovalAnswer::DenyAndStop
        );
        assert_eq!(answer_of(CANCEL_ID, false).unwrap(), ApprovalAnswer::Closed);
        assert!(
            answer_of(7, false).is_err(),
            "an unknown id is an error, not a choice"
        );
    }

    #[test]
    fn a_dismissal_wins_over_any_button() {
        for button in [APPROVE_ID, DENY_ID, DENY_AND_STOP_ID, CANCEL_ID, 7] {
            assert_eq!(
                answer_of(button, true).unwrap(),
                ApprovalAnswer::Dismissed,
                "{button}"
            );
        }
    }

    #[test]
    fn no_desktop_answers_without_showing() {
        let closer = DialogCloser::new();
        let answer = ask_with(&closer, || Ok(false), || panic!("must not show"));
        assert_eq!(answer.unwrap(), ApprovalAnswer::NoDesktop);
    }

    #[test]
    fn a_failed_desktop_check_is_an_error_without_showing() {
        let closer = DialogCloser::new();
        let answer = ask_with(
            &closer,
            || Err(io::Error::other("x")),
            || panic!("must not show"),
        );
        assert!(answer.is_err());
    }

    #[test]
    fn a_closer_dismissed_before_showing_keeps_the_dialog_from_opening() {
        let closer = DialogCloser::new();
        closer.dismiss().expect("nothing open, nothing to post");
        let answer = ask_with(&closer, || Ok(true), || panic!("must not show"));
        assert_eq!(answer.unwrap(), ApprovalAnswer::Dismissed);
    }

    #[test]
    fn a_dismissal_while_open_turns_the_click_into_dismissed() {
        let closer = DialogCloser::new();
        let during = closer.clone();
        let answer = ask_with(
            &closer,
            || Ok(true),
            || {
                during
                    .dismiss()
                    .expect("no window recorded, nothing to post");
                Ok(APPROVE_ID)
            },
        );
        assert_eq!(answer.unwrap(), ApprovalAnswer::Dismissed);
    }

    #[test]
    fn with_a_desktop_the_shown_choice_is_the_answer() {
        let closer = DialogCloser::new();
        let answer = ask_with(&closer, || Ok(true), || Ok(DENY_AND_STOP_ID));
        assert_eq!(answer.unwrap(), ApprovalAnswer::DenyAndStop);
    }

    #[cfg(windows)]
    fn question() -> ApprovalQuestion<'static> {
        ApprovalQuestion {
            title: "Engram",
            heading: "An agent asks to add a PC",
            body: "Agent \"a1b2\" asks to trust PC \"<a href=\"x\">lab</a>\".",
            approve: "Approve",
            deny: "Deny",
            deny_and_stop: "Deny and stop asking this agent",
        }
    }

    #[cfg(windows)]
    fn wide_at(p: PCWSTR) -> String {
        // SAFETY: points into a live `Prepared`.
        unsafe { p.to_string() }.expect("UTF-16")
    }

    #[cfg(windows)]
    #[test]
    fn the_config_carries_the_texts_three_buttons_in_order_and_deny_as_default() {
        use windows::Win32::UI::Controls::{TDF_ENABLE_HYPERLINKS, TDF_USE_COMMAND_LINKS};

        let q = question();
        let prepared = Prepared::new(&q).expect("prepare");
        let buttons = prepared.buttons();
        let state = Mutex::new(CloserState::default());
        let config = prepared.config(&buttons, &state);

        // The struct is packed, so fields are copied out before use.
        let (title, heading, body) = (
            config.pszWindowTitle,
            config.pszMainInstruction,
            config.pszContent,
        );
        assert_eq!(wide_at(title), q.title);
        assert_eq!(wide_at(heading), q.heading);
        assert_eq!(wide_at(body), q.body);

        let (count, default, common) = (
            config.cButtons,
            config.nDefaultButton,
            config.dwCommonButtons,
        );
        assert_eq!(count, 3);
        assert_eq!(default, DENY_ID, "deny is the default button");
        assert_eq!(common.0, 0, "no stock buttons besides the three");
        let shown: Vec<(i32, String)> = buttons
            .iter()
            .map(|b| {
                let (id, text) = (b.nButtonID, b.pszButtonText);
                (id, wide_at(text))
            })
            .collect();
        assert_eq!(
            shown,
            vec![
                (APPROVE_ID, q.approve.to_string()),
                (DENY_ID, q.deny.to_string()),
                (DENY_AND_STOP_ID, q.deny_and_stop.to_string()),
            ]
        );
        let pointer = config.pButtons;
        assert_eq!(pointer, buttons.as_ptr());

        let flags = config.dwFlags;
        assert_eq!(
            flags & TDF_NO_SET_FOREGROUND,
            TDF_NO_SET_FOREGROUND,
            "the dialog does not ask for the foreground"
        );
        assert_eq!(
            flags & TDF_ENABLE_HYPERLINKS,
            Default::default(),
            "markup stays literal"
        );
        assert_eq!(flags & TDF_USE_COMMAND_LINKS, Default::default());
        assert_eq!(
            flags & TDF_ALLOW_DIALOG_CANCELLATION,
            TDF_ALLOW_DIALOG_CANCELLATION
        );
        let (callback, data) = (config.pfCallback, config.lpCallbackData);
        assert!(callback.is_some());
        assert_eq!(data, &state as *const _ as isize);
    }

    #[cfg(windows)]
    #[test]
    fn a_nul_inside_a_text_is_refused_before_anything_is_shown() {
        let q = ApprovalQuestion {
            body: "a\0b",
            ..question()
        };
        let e = ask_approval(&q, &DialogCloser::new()).expect_err("refused");
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
    }

    /// Pops the WM_CLOSE posted to `window`, if any (posted messages go to the queue of the thread that made the window — this test's).
    #[cfg(windows)]
    fn took_close(window: HWND) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_REMOVE};
        let mut msg = MSG::default();
        // SAFETY: a live window of this thread and a message struct on the stack.
        unsafe { PeekMessageW(&mut msg, window, WM_CLOSE, WM_CLOSE, PM_REMOVE) }.as_bool()
    }

    #[cfg(windows)]
    fn is_topmost(window: HWND) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TOPMOST,
        };
        // SAFETY: a live window of this thread.
        let style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32;
        style & WS_EX_TOPMOST.0 != 0
    }

    /// Delivers one dialog notification to [`callback`] as the dialog would, with `closer`'s state as the reference data.
    #[cfg(windows)]
    fn notify(window: HWND, notification: TASKDIALOG_NOTIFICATIONS, closer: &DialogCloser) {
        let data = Arc::as_ptr(&closer.inner) as isize;
        // SAFETY: `data` points at the closer's state, alive for the call.
        let answer = unsafe { callback(window, notification, WPARAM(0), LPARAM(0), data) };
        assert_eq!(answer, S_OK);
    }

    /// The callback driven on a stand-in window: a dismissal that came before the window existed closes it as soon as it is created,
    ///   and the window is raised topmost.
    #[cfg(windows)]
    #[test]
    fn a_dismissal_before_creation_closes_the_window_once_it_exists() {
        let stand_in = crate::test_window::TestWindow::new(true);
        let closer = DialogCloser::new();
        closer.dismiss().expect("nothing open yet");
        notify(stand_in.hwnd(), TDN_CREATED, &closer);
        assert!(took_close(stand_in.hwnd()), "WM_CLOSE posted at creation");
        assert!(is_topmost(stand_in.hwnd()), "raised topmost");
    }

    /// After creation `dismiss` posts the close itself; after destruction it posts nothing (the handle may be reused).
    #[cfg(windows)]
    #[test]
    fn dismiss_closes_a_created_window_and_leaves_a_destroyed_one_alone() {
        let stand_in = crate::test_window::TestWindow::new(true);
        let closer = DialogCloser::new();
        notify(stand_in.hwnd(), TDN_CREATED, &closer);
        assert!(!took_close(stand_in.hwnd()), "not dismissed, not closed");
        closer.dismiss().expect("close posted");
        assert!(took_close(stand_in.hwnd()), "dismiss posts WM_CLOSE");

        let later = DialogCloser::new();
        notify(stand_in.hwnd(), TDN_CREATED, &later);
        notify(stand_in.hwnd(), TDN_DESTROYED, &later);
        later.dismiss().expect("nothing to post");
        assert!(!took_close(stand_in.hwnd()), "no post after destruction");
    }

    /// Loads comctl32 v6 at run time without showing anything — the part a missing manifest would break.
    #[cfg(windows)]
    #[test]
    fn task_dialog_loads_from_comctl32_v6_in_a_manifestless_executable() {
        comctl6::TaskDialogIndirect::load()
            .expect("TaskDialogIndirect found under the activation context");
    }

    // ── Manual checks — each shows a real dialog on this desktop. Run one by name with `--ignored --exact`. ──

    /// Shows one question and prints the answer — click any choice, or close it.
    #[cfg(windows)]
    #[test]
    #[ignore = "shows a real dialog and waits for a human"]
    fn manual_shows_one_approval_dialog() {
        let q = ApprovalQuestion {
            body: "An agent asks: trust PC \"lab\" (ID abcd-efgh-ijkl). Literal: <a href=\"x\">link</a> & ampersand.",
            ..question()
        };
        let answer = ask_approval(&q, &DialogCloser::new()).expect("shown");
        eprintln!("answer = {answer:?}");
    }

    /// Opens a dialog and dismisses it from another thread after a second — it must close by itself with `Dismissed`.
    #[cfg(windows)]
    #[test]
    #[ignore = "shows a real dialog for about a second"]
    fn manual_an_open_dialog_closes_when_dismissed() {
        let closer = DialogCloser::new();
        let remote = closer.clone();
        let dismisser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(1));
            remote.dismiss().expect("close posted");
        });
        let answer = ask_approval(&question(), &closer).expect("shown");
        dismisser.join().expect("dismisser thread");
        assert_eq!(answer, ApprovalAnswer::Dismissed);
    }
}
