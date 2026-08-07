//! 단일 인스턴스 가드 — named mutex.
//!
//! 데몬은 한 사용자당 하나만 떠야 한다(랜덤 포트·토큰을 daemon.json 으로 단일 발행).
//! Windows 는 `Global\EngramDashboardDaemon-<user>` named mutex 로 판정한다:
//! CreateMutexW 후 GetLastError()==ERROR_ALREADY_EXISTS 면 이미 다른 프로세스가
//! 같은 이름의 mutex 를 들고 있다 = 이미 실행 중.
//!
//! ★네임스페이스 선택★: `Local\` 은 **로그온 세션 한정**(RDP/elevated 등 다른 세션은
//! 별개 mutex)이라 같은 사용자의 다른 세션에서 데몬이 중복 기동돼 같은 daemon.json/
//! agents.json 을 두고 경합한다. data_dir 단일성 경계가 사용자 단위(`%APPDATA%`)이므로
//! mutex 도 `Global\` + 사용자 식별자로 맞춰 사용자당 하나를 보장한다.
//!
//! ★수명 규칙★: 획득한 mutex 핸들은 **데몬 프로세스가 사는 동안 계속 들고 있어야** 한다.
//! 핸들이 Drop(CloseHandle)되면 mutex 가 풀려 단일성 보장이 깨진다. 따라서 main 은
//! 반환된 guard 를 프로세스 종료 시점까지 살려둔다(`_guard` 바인딩).

/// 단일 인스턴스 mutex 식별자 override 환경변수 이름. 설정 시 USERNAME 대신 이 값으로 mutex 이름을
/// 만든다 — 실프로세스 격리테스트가 테스트별 고유 key 를 주입해 병렬 충돌을 없애기 위함이다.
/// ★운영 회귀 0★: 미설정 시 기존 USERNAME 동작 그대로.
pub(crate) const INSTANCE_KEY_ENV: &str = "ENGRAM_INSTANCE_KEY";

#[cfg(windows)]
mod imp {
    use super::INSTANCE_KEY_ENV;
    use std::io;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Threading::CreateMutexW;

    /// `Global\` mutex 는 같은 사용자가 일반 권한으로 생성·개방하는 데 문제없다.
    pub(crate) fn mutex_name() -> String {
        if let Some(key) = std::env::var_os(INSTANCE_KEY_ENV) {
            if !key.is_empty() {
                let key = key.to_string_lossy();
                return format!("Global\\EngramDashboardDaemon-{key}");
            }
        }
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".to_string());
        format!("Global\\EngramDashboardDaemon-{user}")
    }

    pub struct InstanceGuard {
        handle: HANDLE,
    }

    // SAFETY: HANDLE 은 raw 포인터 wrapper 라 자동 Send/Sync 가 아니다. 이 핸들은
    // 우리가 생성·소유·CloseHandle 까지 단독 관리하며 생성 후 변이가 없으므로(Drop 의
    // 단일 CloseHandle 외 접근 없음) 스레드 간 이동을 허용한다.
    unsafe impl Send for InstanceGuard {}
    unsafe impl Sync for InstanceGuard {}

    impl Drop for InstanceGuard {
        fn drop(&mut self) {
            // SAFETY: acquire()에서 생성한 유효한 mutex 핸들을 Drop 시 한 번만 닫는다.
            let r = unsafe { CloseHandle(self.handle) };
            if let Err(e) = r {
                tracing::debug!("InstanceGuard mutex CloseHandle 실패: {e}");
            }
        }
    }

    /// mutex 획득 시도. Ok(Some(guard))=획득(우리가 첫 인스턴스), Ok(None)=이미 실행 중,
    /// Err=시스템 오류(핸들 생성 실패).
    pub fn acquire() -> io::Result<Option<InstanceGuard>> {
        let name = mutex_name();
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

        // SAFETY: CreateMutexW — 보안 속성 None, 소유 요청 false, 유효한 NUL 종단
        // 와이드 문자열 포인터. 동일 이름 mutex 가 이미 있으면 그 핸들을 반환하고
        // GetLastError 가 ERROR_ALREADY_EXISTS 를 세운다. 실패 시 Err.
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())) }
            .map_err(io::Error::other)?;

        // SAFETY: 직전 CreateMutexW 호출 직후의 last-error 를 읽는다(GetLastError 는 인자 없음).
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already {
            // SAFETY: 방금 CreateMutexW 가 반환한 유효한 핸들을 한 번만 닫는다.
            let r = unsafe { CloseHandle(handle) };
            if let Err(e) = r {
                tracing::debug!("중복 인스턴스 mutex 핸들 CloseHandle 실패: {e}");
            }
            return Ok(None);
        }

        Ok(Some(InstanceGuard { handle }))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;

    /// non-windows stub 가드. 보유할 OS 자원 없음(데몬은 Windows 1차 — 추후 flock 등으로 대체).
    pub struct InstanceGuard;

    pub fn acquire() -> io::Result<Option<InstanceGuard>> {
        Ok(Some(InstanceGuard))
    }
}

// InstanceGuard 는 acquire() 반환 타입으로만 쓰여(이름 직접 참조 없음) 타입 추론으로 충분 —
// acquire 만 노출한다.
pub use imp::acquire;

#[cfg(all(test, windows))]
mod tests {
    use super::{imp::mutex_name, INSTANCE_KEY_ENV};

    /// ★왜 한 테스트★: ENGRAM_INSTANCE_KEY 는 프로세스 전역 상태라 별도 테스트로 나누면 병렬
    ///   실행 시 서로 경합한다. set→확인→remove→확인 을 한 흐름에서 직렬로 하고 끝에서 반드시 제거한다.
    #[test]
    fn mutex_name_env_override_and_default() {
        std::env::remove_var(INSTANCE_KEY_ENV);
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".to_string());
        let default_name = mutex_name();
        assert_eq!(
            default_name,
            format!("Global\\EngramDashboardDaemon-{user}"),
            "override 미설정 시 USERNAME 단위 이름(운영 동작 그대로)"
        );

        std::env::set_var(INSTANCE_KEY_ENV, "test-key-abc123");
        assert_eq!(
            mutex_name(),
            "Global\\EngramDashboardDaemon-test-key-abc123",
            "override 설정 시 그 key 로 mutex 이름 생성"
        );

        // 3) 빈 값은 무시하고 기본(USERNAME)으로 폴백(빈 env 가 의도치 않게 모두 같은 이름을 쓰지 않게).
        std::env::set_var(INSTANCE_KEY_ENV, "");
        assert_eq!(
            mutex_name(),
            format!("Global\\EngramDashboardDaemon-{user}"),
            "빈 override 는 무시하고 USERNAME 기본으로 폴백"
        );

        std::env::remove_var(INSTANCE_KEY_ENV);
    }
}
