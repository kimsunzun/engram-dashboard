//! embedded single-instance 가드 — 데이터 폴더별 named mutex.
//!
//! ## 왜 폴더별인가 (ADR-0027)
//! embedded 모드는 데몬과 달리 전역 1개가 아니다 — 사용자가 데이터 폴더를 골라 여러 독립
//! 인스턴스를 띄울 수 있다. 단일성 경계는 "프로세스 전역"이 아니라 "같은 데이터 폴더"다:
//! 같은 폴더를 공유할 2nd 인스턴스만 차단해야 한다(같은 agents.json 을 두 프로세스가 동시에
//! restore_all 하면 이중복원·경합이 난다 = 정확성 핵심). 다른 폴더 → 다른 키 → 독립 허용.
//!
//! ## daemon instance.rs 와의 관계 (sibling — 의도적 복제)
//! `crates/engram-dashboard-daemon/src/instance.rs` 가 같은 named-mutex 패턴(CreateMutexW +
//! ERROR_ALREADY_EXISTS, InstanceGuard Drop=CloseHandle, ENGRAM_INSTANCE_KEY override, non-windows
//! stub)을 쓴다. crate 가 분리돼(daemon ws_e2e 가 그 모듈에 의존) 공유 대신 **코드를 복제**한다.
//! 차이는 키 산출뿐: daemon=USERNAME 전역, 여기=data_dir 경로 해시. daemon 쪽은 건드리지 않는다.
//!
//! ## 수명 규칙
//! acquire 가 돌려준 guard 는 **프로세스가 사는 동안 계속 들고 있어야** 한다. Drop(CloseHandle)
//! 되면 mutex 가 풀려 단일성이 깨진다 → lib.rs run() 이 `.run()` 까지 살리는 스코프 변수로 보유.

/// 단일 인스턴스 mutex 식별자 override 환경변수. 설정 시 data_dir 해시 대신 이 값으로 키를
/// 만든다(테스트 격리 — daemon instance.rs 와 동일 탈출구). 미설정/빈 값이면 운영 동작 그대로.
pub const INSTANCE_KEY_ENV: &str = "ENGRAM_INSTANCE_KEY";

/// embedded single-instance 락 키(순수·테스트 대상). 데이터 폴더 경로 해시 기반.
/// 같은 데이터 폴더를 공유할 인스턴스끼리만 충돌(다른 폴더=다른 키=독립). ENGRAM_INSTANCE_KEY
/// override 가 있으면 그 값(테스트 격리 — daemon instance.rs 와 동일 탈출구).
pub fn embedded_instance_key(data_dir: &std::path::Path) -> String {
    use std::hash::{Hash, Hasher};

    // 1) 테스트 격리 override — 비어있지 않으면 그 값을 식별자로 사용(data_dir 무시).
    if let Some(key) = std::env::var_os(INSTANCE_KEY_ENV) {
        if !key.is_empty() {
            let key = key.to_string_lossy();
            return format!("Global\\Engram-{key}");
        }
    }

    // 2) 운영 기본 — data_dir 경로 해시.
    //    ★정규화★: 데이터 폴더 경로를 키로 — 같은 폴더의 다른 표기(대소문자/구분자/8.3 short-path/
    //    UNC/trailing-slash/`..`)가 다른 키로 갈리면 중복 차단이 샌다. dunce::canonicalize 로 정규화
    //    시도(8.3·UNC·`..`·중복 구분자를 흡수하고 `\\?\` prefix 는 회피), 폴더가 아직 없으면(부팅
    //    초기) 실패 → lowercase + `/`→`\` 문자열 정규화로 강등. canonicalize 성공 경로도 lowercase 로
    //    통일(Windows 는 대소문자 무관). 가드/store 가 동일 경로(M1 단일 출처)라 양쪽이 같은 정규화를
    //    거쳐 일관된 키를 얻는다.
    let normalized: String = match dunce::canonicalize(data_dir) {
        Ok(p) => p.to_string_lossy().to_lowercase().replace('/', "\\"),
        Err(_) => data_dir.to_string_lossy().to_lowercase().replace('/', "\\"),
    };

    // DefaultHasher 는 SipHash 기반이지만 고정 시드(0,0)라 같은 입력 → 같은 출력이 프로세스 간
    // 안정적이다(난수 시드 아님). 키는 사람이 읽을 필요 없으므로 16진 해시로 충분.
    // ★범위 주의★: "동일 바이너리의 프로세스 간 결정적(고정 시드)" 이지 std 버전 간 안정은 아니다.
    // 이 용도(같은 빌드끼리 mutex 충돌 판정)엔 충분 — std 버전 간 해시 안정성은 비보장이나 무관(같은
    // 빌드끼리만 비교하므로). 다음 세션이 "프로세스 간 안정"을 "버전 간 안정"으로 오독하지 말 것.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    let hash = hasher.finish();
    // `Global\` 네임스페이스 = 로그온 세션을 넘어 머신 전역에서 유일. 데이터 폴더는 user 세션을
    // 넘어 공유되는 파일경로이므로(RDP/elevated 등 다른 세션이 같은 폴더를 열 수 있음) cross-session
    // 차단이 맞다(daemon instance.rs 의 Global 논리와 동일).
    format!("Global\\Engram-{hash:016x}")
}

#[cfg(windows)]
mod imp {
    use super::embedded_instance_key;
    use std::io;
    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Threading::CreateMutexW;

    /// 살아있는 동안 single-instance 를 보장하는 가드. Drop 시 mutex 핸들을 닫는다.
    pub struct InstanceGuard {
        handle: HANDLE,
    }

    // SAFETY: HANDLE 은 raw 포인터 wrapper 라 자동 Send/Sync 가 아니다. 이 핸들은 우리가 생성·소유·
    // CloseHandle 까지 단독 관리하며 생성 후 변이가 없으므로(Drop 의 단일 CloseHandle 외 접근 없음)
    // 스레드 간 이동을 허용한다(daemon instance.rs 와 동일 근거).
    unsafe impl Send for InstanceGuard {}
    unsafe impl Sync for InstanceGuard {}

    impl Drop for InstanceGuard {
        fn drop(&mut self) {
            // SAFETY: acquire 에서 생성한 유효한 mutex 핸들을 Drop 시 한 번만 닫는다.
            let r = unsafe { CloseHandle(self.handle) };
            if let Err(e) = r {
                tracing::debug!("InstanceGuard mutex CloseHandle 실패: {e}");
            }
        }
    }

    /// embedded 폴더별 single-instance 가드 획득. Ok(Some(guard))=첫 인스턴스, Ok(None)=이미 같은
    /// 폴더 실행 중(호출자가 exit 양보), Err=시스템 오류.
    pub fn acquire_embedded(data_dir: &Path) -> io::Result<Option<InstanceGuard>> {
        let name = embedded_instance_key(data_dir);
        // 이름을 UTF-16 + NUL 종단으로 변환(PCWSTR 요구).
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

        // SAFETY: CreateMutexW — 보안 속성 None, 소유 요청 false, 유효한 NUL 종단 와이드 문자열
        // 포인터. 동일 이름 mutex 가 이미 있으면 그 핸들을 반환하고 GetLastError 가
        // ERROR_ALREADY_EXISTS 를 세운다. 실패 시 Err.
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())) }
            .map_err(io::Error::other)?;

        // SAFETY: 직전 CreateMutexW 직후의 last-error 를 읽는다(GetLastError 는 인자 없음).
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already {
            // 이미 같은 폴더 인스턴스가 mutex 보유 — 받은 핸들은 닫고 None 반환.
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
    use std::path::Path;

    /// non-windows stub 가드(embedded 는 Windows 1차 — 추후 flock 등으로 대체).
    pub struct InstanceGuard;

    /// 항상 획득 성공(stub).
    pub fn acquire_embedded(_data_dir: &Path) -> io::Result<Option<InstanceGuard>> {
        Ok(Some(InstanceGuard))
    }
}

// InstanceGuard 는 acquire_embedded 반환 타입으로만 쓰여(이름 직접 참조 없음) 타입 추론으로 충분 —
// acquire_embedded 만 노출한다(daemon instance.rs 와 동일).
pub use imp::acquire_embedded;

#[cfg(test)]
mod tests {
    use super::{embedded_instance_key, INSTANCE_KEY_ENV};
    use std::path::Path;

    /// 키 산출 순수 검증을 한 테스트에서 직렬로 한다.
    /// ★왜 한 테스트★: ENGRAM_INSTANCE_KEY 는 프로세스 전역 상태라 별도 테스트로 나누면 cargo
    ///   병렬 실행 시 서로 경합한다. set→확인→remove 를 한 흐름에서 직렬로 하고 끝에서 반드시 제거.
    #[test]
    fn embedded_instance_key_cases() {
        // override 가 다른 테스트에서 새어들어오지 않게 먼저 비운다.
        std::env::remove_var(INSTANCE_KEY_ENV);

        // ① 같은 경로 → 같은 키.
        let a1 = embedded_instance_key(Path::new(r"C:\Foo\bar"));
        let a2 = embedded_instance_key(Path::new(r"C:\Foo\bar"));
        assert_eq!(a1, a2, "같은 경로는 같은 키");

        // ② 다른 경로 → 다른 키.
        let b = embedded_instance_key(Path::new(r"C:\Foo\baz"));
        assert_ne!(a1, b, "다른 경로는 다른 키");

        // ③ 대소문자/구분자 다른 같은 경로 → 같은 키(정규화 검증).
        //    C:\Foo\bar 는 보통 미존재 → dunce::canonicalize 실패 → fallback 문자열 정규화 경로를 탄다.
        //    그 경로에서도 lowercase + `/`→`\` 통일이 동작해 같은 키가 나오는지 확인.
        let c = embedded_instance_key(Path::new("c:/foo/bar"));
        assert_eq!(
            a1, c,
            "대소문자·구분자만 다른 같은 폴더는 같은 키(fallback 정규화)"
        );

        // ③' canonicalize 성공 경로(실존 폴더)에서도 표기 차이를 흡수하는지.
        //    실제 존재하는 폴더(temp dir)를 대소문자/구분자만 바꿔 넣어도 같은 키여야 한다.
        //    canonicalize 가 같은 실경로로 모으므로(성공 경로) 표기 차이가 사라진다.
        let real = std::env::temp_dir();
        if real.exists() {
            let lower = real.to_string_lossy().to_lowercase().replace('\\', "/");
            let k1 = embedded_instance_key(&real);
            let k2 = embedded_instance_key(Path::new(&lower));
            assert_eq!(k1, k2, "실존 폴더는 표기(대소문자/구분자) 달라도 같은 키");
        }

        // ④ override 설정 시 그 값을 식별자로 사용(data_dir 무시).
        std::env::set_var(INSTANCE_KEY_ENV, "test-key-xyz");
        assert_eq!(
            embedded_instance_key(Path::new(r"C:\whatever")),
            "Global\\Engram-test-key-xyz",
            "override 설정 시 그 key 로 생성"
        );
        // 빈 override 는 무시하고 해시 기본으로 폴백.
        std::env::set_var(INSTANCE_KEY_ENV, "");
        assert_eq!(
            embedded_instance_key(Path::new(r"C:\Foo\bar")),
            a1,
            "빈 override 는 무시하고 해시 기본으로 폴백"
        );

        // 정리 — 다른 테스트로 새지 않게 반드시 제거.
        std::env::remove_var(INSTANCE_KEY_ENV);
    }
}
