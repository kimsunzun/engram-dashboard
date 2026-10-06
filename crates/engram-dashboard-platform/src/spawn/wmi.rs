//! [`super::spawn_outside_job`] 의 WMI 호출 — COM 초기화와 그 해제의 짝맞춤 · `Win32_Process.Create`.
//!
//! 초기화 결과의 분류([`classify_com_init`])는 OS 와 무관한 규칙이라 모든 OS 에서 컴파일되고 시험된다. 나머지는
//! Windows 전용이다.
// ADR-0266
// ADR-0271

use std::io;
use std::path::Path;

use super::DetachedSpawnError;

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
enum ComInit {
    /// 우리가 초기화했다(`S_OK` · `S_FALSE` — 둘 다 짝으로 `CoUninitialize` 를 한 번 부를 책임이 있다).
    Initialized,
    /// 이 스레드가 이미 다른 아파트 모드(STA)로 초기화돼 있다(`RPC_E_CHANGED_MODE`) — 우리는 초기화하지 않았다.
    /// 그 아파트로 WMI 를 부르되 해제하지 않는다.
    AlreadyOtherMode,
    /// 그 밖의 실패 — 진행하지 않는다.
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

/// 우리가 초기화한 COM 을 모든 탈출 경로에서 정확히 한 번 해제한다 — [`create`] 의 `?` 조기 반환마다 손으로 짝을
/// 맞추면 하나를 빠뜨린다.
#[cfg(windows)]
struct ComGuard {
    needs_uninit: bool,
}

#[cfg(windows)]
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.needs_uninit {
            use windows::Win32::System::Com::CoUninitialize;
            // SAFETY: 우리가 CoInitializeEx 로 성공 초기화한 스레드에서 정확히 1회 해제한다.
            // AlreadyOtherMode 경로는 needs_uninit=false 라 여기 진입하지 않는다.
            unsafe { CoUninitialize() };
        }
    }
}

/// `ReturnValue` 를 오류로 올리지 않고 돌려준다 — 그 판정은 부르는 쪽 몫이다. 답 객체나 그 값을 못 얻으면
/// `u32::MAX` 다. `create_flags` = `None` 이면 시작 정보(`ProcessStartupInformation`)를 아예 넘기지 않는다.
#[cfg(windows)]
pub(super) fn create(exe: &Path, create_flags: Option<i32>) -> Result<u32, DetachedSpawnError> {
    // Interface trait — startup_inst.cast::<IUnknown>() 에 필요(임베디드 오브젝트를 VARIANT 로 박기).
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

    // SAFETY 블록: COM/WMI 호출 시퀀스. spike #1 의 PowerShell Invoke-CimMethod 와 동일한
    // Win32_Process.Create 를 COM 직접 호출로 수행한다.
    unsafe {
        // SAFETY: CoInitializeEx 는 스레드 단위 COM 초기화. 반환 HRESULT 로 짝맞춤(아래 가드).
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

        // 로컬 WMI 호출에 필요한 impersonation 레벨.
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
            // CreateFlags 는 VT_I4(부호 있는 32-bit).
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

/// WMI 가 비어 있는 객체를 돌려줬다 — 띄우기 전에 멈췄으므로 `ReturnValue` 를 못 얻은 것과 같은 답으로 낸다.
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
    // ReturnValue 는 VT_I4 — windows-core 의 TryFrom<&VARIANT> for u32 가 변환 처리.
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

    // ── classify_com_init 매핑(실제 CoInitialize 없이 순수 검증) ────────────────

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
        let e_fail = 0x8000_4005u32 as i32; // E_FAIL 류 임의 실패
        assert_eq!(classify_com_init(e_fail), ComInit::Failed(e_fail));
    }

    #[test]
    fn com_init_needs_uninit_only_when_we_initialized() {
        let needs = |hr: i32| match classify_com_init(hr) {
            ComInit::Initialized => true,
            ComInit::AlreadyOtherMode => false,
            ComInit::Failed(_) => false, // 실패면 가드 자체를 안 만듦
        };
        assert!(needs(0), "S_OK → uninit");
        assert!(needs(1), "S_FALSE → uninit");
        assert!(!needs(0x8001_0106u32 as i32), "CHANGED_MODE → no uninit");
    }
}
