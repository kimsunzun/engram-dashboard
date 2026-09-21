//! 「이 파일을 지금 어느 프로세스가 열고 있나」를 OS 에 묻는다.
//!
//! ★무엇을 찾으려고 묻는지는 이 파일이 모른다(ADR-0004)★ — 받는 것은 경로 하나이고 돌려주는 것은
//! 홀더 목록뿐이다. 도메인 지식이 0 이라 여기 있고, 소비자가 하나뿐이라 바닥 crate 로는 안 내려간다
//! (ADR-0175 입주 조건 ① · ADR-0218 결정 11).
//!
//! 묻는 수단은 **Windows Restart Manager** 이고 다른 OS 에는 이 질문을 할 수단을 두지 않았다 —
//! 거기서는 항상 빈 목록이라 이 위에 선 판정은 오검출 대신 「못 받음」에 머문다(ADR-0218 결정 10).
//!
//! 진입점 = [`holders_of`]. 실패는 값으로 돌려준다(panic 없음).

use std::io;
use std::path::Path;

#[cfg(windows)]
use windows::core::{PCWSTR, PWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR};
#[cfg(windows)]
use windows::Win32::System::RestartManager::{
    RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
    RM_PROCESS_INFO,
};

/// 한 파일을 열고 있는 프로세스 하나.
///
/// ★두 칸은 **함께** 대조하라고 있다★ — PID 는 OS 가 재사용하므로 PID 단독 일치는 남의 프로세스를
/// 우리 것으로 본다(ADR-0218 결정 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Holder {
    pub(crate) pid: u32,
    /// 프로세스 생성 FILETIME(1601-01-01 UTC 부터 100나노초 간격 수)의 high/low 32비트를 합친 값 —
    /// 같은 PID 에 대해 [`engram_dashboard_base::platform::process_creation_time`] 이 돌려주는 값과
    /// 같다(ADR-0218 「근거」의 실측이고, 이 파일의 테스트가 그 동일성을 잰다).
    ///
    /// Restart Manager 가 이 칸을 못 채워 0 을 주는 항목이 있는지는 미검이다 — 그런 항목은 대조에서
    /// 그냥 떨어진다(대조 상대의 시작시각은 0 이 아니므로).
    pub(crate) start_time: u64,
}

/// `path` 를 열고 있는 프로세스 전부.
///
/// - **빈 목록 = 아무도 안 쥐고 있다**. 오류가 아니라 죽은 락 파일의 정상 모습이다(ADR-0218 「근거」 —
///   소유 프로세스를 죽인 뒤 같은 파일을 물으면 0 이 나오는 것을 실측했다).
/// - **오류 = 물어보는 데 실패했다**(세션 생성·등록·열거 실패). 「홀더 없음」과 갈라 두는 이유는 호출부가
///   그 둘을 다르게 로깅할 수 있게 하려는 것뿐 — 소유 판정에서는 둘 다 「후보 아님」이다.
/// - 관리자 권한은 필요 없다(같은 실측).
/// - ★전체 경로로 준다★ — 실측·문서가 다룬 것이 전체 경로뿐이고, 상대 경로를 Restart Manager 가 무엇에
///   상대로 푸는지는 미검이다. 어긋나면 증상은 오류가 아니라 「홀더 0」이다.
#[cfg(windows)]
pub(crate) fn holders_of(path: &Path) -> io::Result<Vec<Holder>> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "경로 안에 NUL 이 있어 널종단 문자열로 넘길 수 없다",
        ));
    }
    wide.push(0);

    let session = RmSession::start()?;
    let files = [PCWSTR(wide.as_ptr())];
    // SAFETY: 살아 있는 세션 핸들 + 이 함수가 소유한 널종단 wide 경로 한 칸. 호출이 끝날 때까지
    // `wide` 가 살아 있고, 등록된 리소스는 세션이 닫힐 때 함께 정리된다.
    let rc = unsafe { RmRegisterResources(session.handle, Some(&files), None, None) };
    if rc != ERROR_SUCCESS {
        return Err(rm_err("RmRegisterResources", rc));
    }

    // 버퍼 크기는 API 에 물어서 받는다(`needed`). ★한 바퀴로 안 끝낼 수 있다★ — 두 호출 사이에 홀더가
    // 늘면 두 번째도 ERROR_MORE_DATA 로 떨어지므로 몇 바퀴 되묻고, 수렴 안 하면 무한 루프 대신 오류다.
    let mut buf: Vec<RM_PROCESS_INFO> = Vec::new();
    for _ in 0..GET_LIST_ATTEMPTS {
        let mut needed: u32 = 0;
        let mut have: u32 = buf.len() as u32;
        let mut reasons: u32 = 0;
        let out = if buf.is_empty() {
            None
        } else {
            Some(buf.as_mut_ptr())
        };
        // SAFETY: 살아 있는 세션 핸들 + 스택의 출력 포인터 셋 + `have` 칸을 가진 `buf`(비었으면 None/0).
        let rc = unsafe { RmGetList(session.handle, &mut needed, &mut have, out, &mut reasons) };
        if rc == ERROR_SUCCESS {
            buf.truncate(have as usize);
            return Ok(buf.iter().map(holder_of).collect());
        }
        if rc != ERROR_MORE_DATA {
            return Err(rm_err("RmGetList", rc));
        }
        buf = vec![RM_PROCESS_INFO::default(); needed as usize];
    }
    Err(io::Error::other(format!(
        "RmGetList 가 {GET_LIST_ATTEMPTS} 번 안에 버퍼 크기에 수렴하지 않았다"
    )))
}

/// non-windows: 물을 수단이 없어 **항상 빈 목록**이다 — 「아무도 안 쥐고 있다」와 같은 모양으로
/// 내려간다(ADR-0218 결정 10).
#[cfg(not(windows))]
pub(crate) fn holders_of(_path: &Path) -> io::Result<Vec<Holder>> {
    Ok(Vec::new())
}

/// 홀더 목록 조회의 재시도 상한 — 크기를 묻고 받는 사이에 홀더가 느는 경우만 재시도한다.
#[cfg(windows)]
const GET_LIST_ATTEMPTS: u32 = 4;

/// Restart Manager 세션 하나. ★`Drop` 이 닫는 것이 이 타입의 전부다★ — 조회 중간에 `?` 로 빠져나가도
/// 세션이 남지 않게 하려고 값으로 쥔다.
#[cfg(windows)]
struct RmSession {
    handle: u32,
}

#[cfg(windows)]
impl RmSession {
    fn start() -> io::Result<Self> {
        let mut handle: u32 = 0;
        // 키 버퍼 크기는 API 가 정한다 — CCH_RM_SESSION_KEY 글자 + 널 하나.
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        // SAFETY: 스택의 핸들 출력 포인터 + 위 규격대로 잡은 키 버퍼. 플래그는 예약돼 있어 0 뿐이다.
        let rc = unsafe { RmStartSession(&mut handle, 0, PWSTR(key.as_mut_ptr())) };
        if rc != ERROR_SUCCESS {
            return Err(rm_err("RmStartSession", rc));
        }
        Ok(Self { handle })
    }
}

#[cfg(windows)]
impl Drop for RmSession {
    /// ★실패를 삼키지 않는다 — 닫히지 않은 세션은 폴링을 거듭할수록 쌓인다★. `Drop` 이라
    /// 돌려보낼 곳이 없으므로 남기는 것은 로그뿐이고, 그 로그가 「회수가 조용히 무거워지고
    /// 있다」를 볼 수 있는 유일한 자리다.
    fn drop(&mut self) {
        // SAFETY: start() 가 성공으로 돌려준 핸들을 한 번만 닫는다.
        let rc = unsafe { RmEndSession(self.handle) };
        if rc != ERROR_SUCCESS {
            tracing::warn!(
                handle = self.handle,
                code = rc.0,
                "RmEndSession 실패 — Restart Manager 세션이 남았을 수 있다"
            );
        }
    }
}

#[cfg(windows)]
fn holder_of(info: &RM_PROCESS_INFO) -> Holder {
    let started = info.Process.ProcessStartTime;
    Holder {
        pid: info.Process.dwProcessId,
        start_time: ((started.dwHighDateTime as u64) << 32) | (started.dwLowDateTime as u64),
    }
}

#[cfg(windows)]
fn rm_err(api: &str, code: WIN32_ERROR) -> io::Error {
    io::Error::other(format!("{api} 실패 — win32 error {}", code.0))
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    /// 자기 자신이 연 파일을 물어, PID 와 시작시각이 **둘 다** 우리 것으로 오는지 잰다. 시작시각의
    /// 기준값은 바닥 crate 의 헬퍼에서 받는다 — 두 경로가 같은 시계를 읽는다는 것이 ADR-0218 결정 2 의
    /// 전제이고, 여기가 그 전제를 지키는 자리다.
    #[cfg(windows)]
    #[test]
    fn a_file_this_process_holds_reports_this_process() {
        let path = std::env::temp_dir().join(format!(
            "engram-file-holders-{}-{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let file = std::fs::File::create(&path).expect("임시 파일 생성");

        let holders = holders_of(&path).expect("홀더 조회 성공");

        let me = std::process::id();
        let mine = holders
            .iter()
            .find(|h| h.pid == me)
            .unwrap_or_else(|| panic!("자기 PID({me}) 가 홀더로 나와야 — 받은 목록 {holders:?}"));
        let expected = engram_dashboard_base::platform::process_creation_time(me)
            .expect("자기 creation time 조회 가능");
        assert_eq!(
            mine.start_time, expected,
            "RM 의 ProcessStartTime 과 GetProcessTimes 의 creation time 이 같아야"
        );

        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    /// 아무도 안 쥔 파일(그리고 아예 없는 파일)은 오류가 아니라 빈 목록이다 — 죽은 락의 정상 모습.
    #[cfg(windows)]
    #[test]
    fn an_unheld_file_has_no_holder() {
        let path = std::env::temp_dir().join(format!(
            "engram-file-holders-{}-{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"").expect("임시 파일 생성");

        let holders = holders_of(&path).expect("홀더 조회 성공");
        assert!(
            holders.is_empty(),
            "닫아 둔 파일은 홀더 0 — 받은 것 {holders:?}"
        );

        std::fs::remove_file(&path).expect("임시 파일 삭제");
        let gone = holders_of(&path).expect("없는 파일도 오류가 아니다");
        assert!(gone.is_empty(), "없는 파일은 홀더 0 — 받은 것 {gone:?}");
    }
}
