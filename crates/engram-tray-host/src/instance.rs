//! 트레이 단일 인스턴스 가드 — named mutex.
//!
//! ★왜 필요한가(load-bearing)★: 트레이 싱글 인스턴스. 트레이를 두 번 실행하면 프로세스가
//! 여러 개 떠 시스템 트레이에 아이콘이 중복으로 쌓인다(실측: 3번 실행 → 3개 프로세스·3개 아이콘).
//! 그래서 **두 번째 실행은 기존 인스턴스에 양보하고 즉시 종료**한다(표준 싱글 인스턴스 동작).
//!
//! 데몬 `engram-dashboard-daemon/src/instance.rs` 와 **같은 named mutex 패턴**이다
//! (CreateMutexW 후 GetLastError()==ERROR_ALREADY_EXISTS 면 이미 실행 중). **다른 점은 이름뿐** —
//! 데몬은 `Global\EngramDashboardDaemon-<user>`, 트레이는 `Global\EngramTrayHost-<user>` 로
//! 서로 다른 mutex 를 잡아 **데몬과 트레이가 충돌하지 않는다**(둘은 별개 프로세스로 동시에 떠야 함).
//!
//! ★네임스페이스 선택(데몬과 동일 컨벤션)★: `Global\` 은 세션을 넘어 머신 전역에서 유일하고,
//! 사용자 식별자(USERNAME)를 이름에 넣어 "사용자당 트레이 하나" 경계를 만든다. `Local\` 은
//! 로그온 세션 한정이라 RDP/elevated 등 다른 세션에서 트레이가 중복 기동될 수 있어 쓰지 않는다.
//! data_dir 단일성 경계가 사용자 단위(.engram-data)이므로 mutex 도 사용자당 하나로 맞춘다.
//!
//! ★수명 규칙(load-bearing)★: 획득한 mutex 핸들(가드)은 **트레이 프로세스가 사는 동안 계속
//! 들고 있어야** 한다. 핸들이 Drop(CloseHandle)되면 mutex 가 풀려 다른 인스턴스가 진입 가능 =
//! 단일성 보장이 깨진다. 따라서 main 은 반환된 가드를 run() 동안 살려둔다(`_guard` 바인딩이
//! 너무 일찍 drop 되지 않게 — run 이 diverge 해도 가드가 그 스코프에 묶이게). 프로세스 종료 시
//! OS 가 mutex 를 자동 해제하므로 별도 정리 코드는 불필요.
//!
//! non-windows 는 트레이 GUI 자체가 Windows 전용이라 항상 성공하는 stub(실행은 안내만).

/// 트레이 단일 인스턴스 mutex 식별자 override 환경변수 이름. 설정 시 USERNAME 대신 이 값으로
/// mutex 이름을 만든다 — 단위테스트가 이름 분기를 검증할 때 쓰는 격리 노브다(데몬과 동일 컨벤션).
/// ★운영 회귀 0★: 미설정 시 기존 USERNAME 동작 그대로.
pub(crate) const INSTANCE_KEY_ENV: &str = "ENGRAM_TRAY_INSTANCE_KEY";

#[cfg(windows)]
mod imp {
    use super::INSTANCE_KEY_ENV;
    use std::io;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Threading::CreateMutexW;

    /// 트레이 단일 인스턴스 mutex 이름. `Global\` 네임스페이스 = 세션을 넘어 머신 전역에서 유일하므로
    /// 사용자 식별자(USERNAME)를 이름에 넣어 "사용자당 트레이 하나" 경계를 만든다.
    ///
    /// ★데몬과 이름이 달라야 한다(load-bearing)★: 데몬은 `EngramDashboardDaemon-<user>`,
    /// 트레이는 `EngramTrayHost-<user>`. 같은 이름을 쓰면 데몬이 떠 있을 때 트레이가 "이미 실행 중"
    /// 으로 오판해 종료해 버린다 — 둘은 별개 프로세스로 동시에 떠야 하므로 mutex 도 분리한다.
    ///
    /// ★ENGRAM_TRAY_INSTANCE_KEY override(테스트 격리용)★: 설정 시 USERNAME 대신 그 값을 식별자로
    ///   쓴다 → `Global\EngramTrayHost-<value>`. ★운영 회귀 0★: env 미설정 시 기존 USERNAME 동작 그대로.
    pub(crate) fn mutex_name() -> String {
        // 1) 테스트 격리 override — 비어있지 않은 값이면 그 값을 식별자로 사용.
        if let Some(key) = std::env::var_os(INSTANCE_KEY_ENV) {
            if !key.is_empty() {
                let key = key.to_string_lossy();
                return format!("Global\\EngramTrayHost-{key}");
            }
        }
        // 2) 운영 기본(회귀 0) — USERNAME 단위.
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".to_string());
        format!("Global\\EngramTrayHost-{user}")
    }

    /// 살아있는 동안 단일 인스턴스를 보장하는 가드. Drop 시 mutex 핸들을 닫는다.
    /// (운영에선 프로세스가 종료될 때까지 살아 있어 OS 가 자동 해제 — Drop 은 안전망/테스트용.)
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
                tracing::debug!("[tray-host] InstanceGuard mutex CloseHandle 실패: {e}");
            }
        }
    }

    /// mutex 획득 시도. Ok(Some(guard))=획득(우리가 첫 트레이 인스턴스), Ok(None)=이미 실행 중,
    /// Err=시스템 오류(핸들 생성 실패).
    ///
    /// ★순수 판정 분리★: 이 함수는 OS mutex 만 다루고 exit 판단은 하지 않는다 — exit 은 호출부
    /// (main)가 None→`std::process::exit(0)` 으로 결정한다. 판정/부작용 분리로 mutex_name 분기는
    /// 단위테스트, acquire 의 Some/None 의미는 호출부 한 줄로 명확히 둔다(데몬과 동일 구조).
    pub fn acquire() -> io::Result<Option<InstanceGuard>> {
        let name = mutex_name();
        // 이름을 UTF-16 + NUL 종단으로 변환(PCWSTR 요구).
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

        // SAFETY: CreateMutexW — 보안 속성 None, 소유 요청 false, 유효한 NUL 종단
        // 와이드 문자열 포인터. 동일 이름 mutex 가 이미 있으면 그 핸들을 반환하고
        // GetLastError 가 ERROR_ALREADY_EXISTS 를 세운다. 실패 시 Err.
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())) }
            .map_err(io::Error::other)?;

        // ★CreateMutexW 와 GetLastError 사이에 어떤 Win32 호출·할당도 끼우지 말 것 — last-error 오염 시 ALREADY_EXISTS 오판★
        // SAFETY: 직전 CreateMutexW 호출 직후의 last-error 를 읽는다(GetLastError 는 인자 없음).
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already {
            // 이미 다른 트레이 인스턴스가 mutex 를 보유 — 우리가 받은 핸들은 닫고 None 반환.
            // SAFETY: 방금 CreateMutexW 가 반환한 유효한 핸들을 한 번만 닫는다.
            let r = unsafe { CloseHandle(handle) };
            if let Err(e) = r {
                tracing::debug!("[tray-host] 중복 인스턴스 mutex 핸들 CloseHandle 실패: {e}");
            }
            return Ok(None);
        }

        Ok(Some(InstanceGuard { handle }))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;

    /// non-windows stub 가드. 트레이 GUI 자체가 Windows 전용이라(main 이 안내만 출력) 보유할
    /// OS 자원 없음. 항상 획득 성공으로 둬 빌드/테스트만 통과시킨다.
    pub struct InstanceGuard;

    /// 항상 획득 성공(stub).
    pub fn acquire() -> io::Result<Option<InstanceGuard>> {
        Ok(Some(InstanceGuard))
    }
}

// main 이 `let _guard: Option<instance::InstanceGuard> = ...` 로 가드를 명시 타입 바인딩하므로
// (보강 1: Err 강행 시 None 을 담아야 해 타입 추론이 안 됨) 타입 이름과 acquire 를 함께 노출한다.
pub use imp::{acquire, InstanceGuard};

#[cfg(all(test, windows))]
mod tests {
    use super::{imp::mutex_name, INSTANCE_KEY_ENV};

    /// mutex 이름의 env override / 기본(USERNAME) 분기 + 데몬 이름과 비겹침을 **한 테스트에서** 직렬 검증.
    /// ★왜 한 테스트★: ENGRAM_TRAY_INSTANCE_KEY 는 프로세스 전역 상태라 별도 테스트로 나누면 cargo 의
    ///   병렬 실행에서 두 테스트가 같은 env 를 동시에 set/remove 해 flaky 해진다. 그래서 env 를 만지는
    ///   검증을 전부 이 한 흐름으로 직렬화하고(set→확인→remove→확인), 끝에서 반드시 제거한다.
    ///   (데몬 원본도 env 테스트를 하나로 직렬화하는 규율을 따른다.)
    #[test]
    fn mutex_name_env_override_and_default() {
        // 1) override 미설정 — USERNAME 단위(운영 회귀 0).
        std::env::remove_var(INSTANCE_KEY_ENV);
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".to_string());
        let default_name = mutex_name();
        assert_eq!(
            default_name,
            format!("Global\\EngramTrayHost-{user}"),
            "override 미설정 시 USERNAME 단위 이름(운영 동작 그대로)"
        );

        // 1-b) 데몬 mutex 이름과 **겹치지 않음**을 박제한다(load-bearing 회귀 가드).
        //   만약 누군가 트레이 이름을 데몬과 같게 바꾸면 데몬이 떠 있을 때 트레이가 자살한다 —
        //   이 단언이 그 회귀를 잡는다. override 와 무관한 불변식이라 default_name 으로 검증한다.
        assert!(
            default_name.contains("EngramTrayHost"),
            "트레이 mutex 는 트레이 전용 이름이어야 함: {default_name}"
        );
        assert!(
            !default_name.contains("EngramDashboardDaemon"),
            "트레이 mutex 가 데몬 이름과 겹치면 안 됨(동시 기동 충돌): {default_name}"
        );

        // 2) override 설정 — 그 key 로 이름 생성(USERNAME 무시).
        std::env::set_var(INSTANCE_KEY_ENV, "test-key-abc123");
        assert_eq!(
            mutex_name(),
            "Global\\EngramTrayHost-test-key-abc123",
            "override 설정 시 그 key 로 mutex 이름 생성"
        );

        // 3) 빈 값은 무시하고 기본(USERNAME)으로 폴백.
        std::env::set_var(INSTANCE_KEY_ENV, "");
        assert_eq!(
            mutex_name(),
            format!("Global\\EngramTrayHost-{user}"),
            "빈 override 는 무시하고 USERNAME 기본으로 폴백"
        );

        // 정리 — 다른 테스트로 새지 않게 반드시 제거.
        std::env::remove_var(INSTANCE_KEY_ENV);
    }
}
