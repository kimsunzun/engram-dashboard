//! The WMI call of [`super::spawn_outside_job`] — pairing COM initialization with its release · `Win32_Process.Create`.
//!
//! The classification of the initialization result ([`classify_com_init`]) is an OS-independent rule, so it compiles and is tested on every OS. The rest is
//! Windows only.
// ADR-0266
// ADR-0271

use std::io;
use std::path::Path;

use super::DetachedSpawnError;

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
enum ComInit {
    /// We initialized it (`S_OK` · `S_FALSE` — both carry the duty to call `CoUninitialize` once as the pair).
    Initialized,
    /// This thread was already initialized in another apartment mode (STA) (`RPC_E_CHANGED_MODE`) — we did not initialize.
    /// Call WMI into that apartment but do not release.
    AlreadyOtherMode,
    /// Any other failure — do not proceed.
    Failed(i32),
}

#[cfg_attr(not(windows), allow(dead_code))]
fn classify_com_init(hr: i32) -> ComInit {
    const S_OK: i32 = 0;
    const S_FALSE: i32 = 1;
    const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;
    match hr {
        S_OK | S_FALSE => ComInit::Initialized,
        RPC_E_CHANGED_MODE => ComInit::AlreadyOtherMode,
        other => ComInit::Failed(other),
    }
}

/// Releases the COM that we initialized exactly once on every exit path — pairing it by hand at each early `?` return of [`create`]
/// would miss one.
#[cfg(windows)]
struct ComGuard {
    needs_uninit: bool,
}

#[cfg(windows)]
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.needs_uninit {
            use windows::Win32::System::Com::CoUninitialize;
            // SAFETY: releases exactly once on the thread where we successfully initialized with CoInitializeEx.
            // The AlreadyOtherMode path has needs_uninit=false so it does not enter here.
            unsafe { CoUninitialize() };
        }
    }
}

/// Returns `ReturnValue` without raising it as an error — deciding on it is the caller's job. If the answer object or that value cannot be obtained, it is
/// `u32::MAX`. `create_flags` = `None` passes no startup information (`ProcessStartupInformation`) at all.
#[cfg(windows)]
pub(super) fn create(exe: &Path, create_flags: Option<i32>) -> Result<u32, DetachedSpawnError> {
    // Interface trait — needed for startup_inst.cast::<IUnknown>() (embedding the embedded object as a VARIANT).
    use windows::core::{Interface, BSTR, VARIANT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
    };
    use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
    use windows::Win32::System::Wmi::{
        IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_CONNECT_USE_MAX_WAIT,
    };

    let exe_str = exe.to_string_lossy();
    let command_line = format!("\"{exe_str}\"");

    // SAFETY block: the COM/WMI call sequence. It performs the same Win32_Process.Create as spike #1's PowerShell Invoke-CimMethod
    // by calling COM directly.
    unsafe {
        // SAFETY: CoInitializeEx is per-thread COM initialization. Paired by the returned HRESULT (the guard below).
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _com_guard = match classify_com_init(hr.0) {
            ComInit::Initialized => ComGuard { needs_uninit: true },
            ComInit::AlreadyOtherMode => ComGuard {
                needs_uninit: false,
            },
            ComInit::Failed(code) => {
                return Err(DetachedSpawnError::Io(io::Error::other(format!(
                    "CoInitializeEx 실패 HRESULT {:#010x}",
                    code as u32
                ))));
            }
        };

        let locator: IWbemLocator =
            CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).map_err(hresult_err)?;
        let services: IWbemServices = locator
            .ConnectServer(
                &BSTR::from("ROOT\\CIMV2"),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                WBEM_FLAG_CONNECT_USE_MAX_WAIT.0,
                &BSTR::new(),
                None,
            )
            .map_err(hresult_err)?;

        // The impersonation level needed for a local WMI call.
        CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            None,
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
        .map_err(hresult_err)?;

        let class_name = BSTR::from("Win32_Process");
        let mut class_obj: Option<IWbemClassObject> = None;
        services
            .GetObject(
                &class_name,
                Default::default(),
                None,
                Some(&mut class_obj),
                None,
            )
            .map_err(hresult_err)?;
        let class_obj = class_obj.ok_or(NO_ANSWER)?;

        let method_name = BSTR::from("Create");
        let mut in_sig: Option<IWbemClassObject> = None;
        class_obj
            .GetMethod(&method_name, 0, &mut in_sig, std::ptr::null_mut())
            .map_err(hresult_err)?;
        let in_sig = in_sig.ok_or(NO_ANSWER)?;
        let in_inst = in_sig.SpawnInstance(0).map_err(hresult_err)?;

        let cl_value = VARIANT::from(BSTR::from(command_line.as_str()));
        in_inst
            .Put(&BSTR::from("CommandLine"), 0, &cl_value, 0)
            .map_err(hresult_err)?;

        if let Some(create_flags) = create_flags {
            let startup_class_name = BSTR::from("Win32_ProcessStartup");
            let mut startup_class: Option<IWbemClassObject> = None;
            services
                .GetObject(
                    &startup_class_name,
                    Default::default(),
                    None,
                    Some(&mut startup_class),
                    None,
                )
                .map_err(hresult_err)?;
            let startup_class = startup_class.ok_or(NO_ANSWER)?;
            let startup_inst = startup_class.SpawnInstance(0).map_err(hresult_err)?;
            // CreateFlags is VT_I4 (signed 32-bit).
            let flags_value = VARIANT::from(create_flags);
            startup_inst
                .Put(&BSTR::from("CreateFlags"), 0, &flags_value, 0)
                .map_err(hresult_err)?;
            let startup_unknown: windows::core::IUnknown =
                startup_inst.cast().map_err(hresult_err)?;
            let startup_value = VARIANT::from(startup_unknown);
            in_inst
                .Put(
                    &BSTR::from("ProcessStartupInformation"),
                    0,
                    &startup_value,
                    0,
                )
                .map_err(hresult_err)?;
        }

        let mut out: Option<IWbemClassObject> = None;
        services
            .ExecMethod(
                &class_name,
                &method_name,
                Default::default(),
                None,
                &in_inst,
                Some(&mut out),
                None,
            )
            .map_err(hresult_err)?;

        let rv = match out {
            Some(out) => read_u32_prop(&out, "ReturnValue").unwrap_or(u32::MAX),
            None => u32::MAX,
        };
        Ok(rv)
    }
}

#[cfg(not(windows))]
pub(super) fn create(_exe: &Path, _create_flags: Option<i32>) -> Result<u32, DetachedSpawnError> {
    Err(DetachedSpawnError::Io(io::ErrorKind::Unsupported.into()))
}

/// WMI returned an empty object — it stopped before launching, so it is reported with the same answer as not obtaining `ReturnValue`.
#[cfg(windows)]
const NO_ANSWER: DetachedSpawnError = DetachedSpawnError::Refused { rv: u32::MAX };

#[cfg(windows)]
unsafe fn read_u32_prop(
    obj: &windows::Win32::System::Wmi::IWbemClassObject,
    name: &str,
) -> Option<u32> {
    use windows::core::{BSTR, VARIANT};
    let mut value = VARIANT::default();
    obj.Get(&BSTR::from(name), 0, &mut value, None, None).ok()?;
    // ReturnValue is VT_I4 — windows-core's TryFrom<&VARIANT> for u32 handles the conversion.
    u32::try_from(&value).ok()
}

#[cfg(windows)]
fn hresult_err(e: windows::core::Error) -> DetachedSpawnError {
    DetachedSpawnError::Io(io::Error::other(format!(
        "WMI HRESULT {:#010x}",
        e.code().0 as u32
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── classify_com_init mapping (pure verification without a real CoInitialize) ────────────────

    #[test]
    fn classify_com_init_maps_hresults() {
        const S_OK: i32 = 0;
        const S_FALSE: i32 = 1;
        const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;
        assert_eq!(classify_com_init(S_OK), ComInit::Initialized);
        assert_eq!(classify_com_init(S_FALSE), ComInit::Initialized);
        assert_eq!(
            classify_com_init(RPC_E_CHANGED_MODE),
            ComInit::AlreadyOtherMode
        );
        let e_fail = 0x8000_4005u32 as i32; // An arbitrary failure like E_FAIL
        assert_eq!(classify_com_init(e_fail), ComInit::Failed(e_fail));
    }

    #[test]
    fn com_init_needs_uninit_only_when_we_initialized() {
        let needs = |hr: i32| match classify_com_init(hr) {
            ComInit::Initialized => true,
            ComInit::AlreadyOtherMode => false,
            ComInit::Failed(_) => false, // On failure the guard itself is not created
        };
        assert!(needs(0), "S_OK → uninit");
        assert!(needs(1), "S_FALSE → uninit");
        assert!(!needs(0x8001_0106u32 as i32), "CHANGED_MODE → no uninit");
    }
}
