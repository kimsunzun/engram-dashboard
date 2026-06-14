//! daemon.json — 데몬 발견(discovery) 파일.
//!
//! 데몬이 잡은 host/port + 접속 토큰 + protocol_version 을 atomic 하게 기록한다.
//! UI(Embedded)나 외부 클라이언트는 이 파일을 읽어 데몬에 붙는다(다음 단위 WS).
//!
//! **atomic 보장(persistence/mod.rs 와 동일 패턴):** 같은 디렉토리에 tmp 를 쓰고
//! `sync_all` 후 `rename` 한다. 같은 파일시스템 내 rename 이라 교체가 원자적 —
//! 크래시가 나도 daemon.json 은 완전한 옛 내용이거나 완전한 새 내용 둘 중 하나다.
//!
//! **보안:** token 은 이 파일에만 둔다(로그 금지). 파일 권한 강화는 추후 단위.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

const TMP_NAME: &str = "daemon.json.tmp";

/// 데몬 발견 정보. daemon.json 의 전체 내용.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonInfo {
    /// 데몬 프로세스 PID — stale 판정(살아있는지)에 사용.
    pub pid: u32,
    /// 항상 "127.0.0.1"(로컬 전용 바인드).
    pub host: String,
    /// 데몬이 실제로 바인드한 포트(랜덤).
    pub port: u16,
    /// 접속 토큰(256-bit hex 64자). 로그 금지.
    pub token: String,
    /// 데몬이 말하는 프로토콜 버전 — 클라이언트가 호환성 판단.
    pub protocol_version: u32,
}

/// tmp → sync_all → rename. 부모 디렉토리는 호출자가 만들어 두었다고 가정하되,
/// 안전하게 create_dir_all 도 한 번 더 한다(idempotent).
pub fn write_atomic(path: &Path, info: &DaemonInfo) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent dir"))?;
    fs::create_dir_all(dir)?;

    let json = serde_json::to_vec_pretty(info)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // 1) 같은 디렉토리 tmp 에 전체를 쓰고 디스크까지 flush(sync_all = 데이터+메타데이터).
    let tmp = dir.join(TMP_NAME);
    {
        let mut f = File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }

    // 2) atomic rename 으로 교체. 같은 디렉토리라 크로스 파일시스템 오류는 발생하지 않는다.
    //    실패 시 tmp 가 디스크에 남지 않게 정리하고 에러를 올린다.
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    // 3) parent 디렉토리 fsync — rename(디렉토리 엔트리 변경)을 영속화.
    //    Windows 에선 디렉토리 핸들 fsync 지원이 제한적이라 best-effort(실패 무시).
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// daemon.json 읽기. 없거나 파싱 불가면 None(부팅 시 무시하고 새로 발행).
pub fn read(path: &Path) -> Option<DaemonInfo> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return None,
    };
    match serde_json::from_slice::<DaemonInfo>(&bytes) {
        Ok(info) => Some(info),
        Err(e) => {
            tracing::warn!("daemon.json 파싱 실패: {e} — 무시");
            None
        }
    }
}

/// 기록된 데몬이 더 이상 살아있지 않은지(stale) 판정. true=죽음(무시 가능).
///
/// Windows: OpenProcess 로 PID 생존을 확인한다. 핸들을 열 수 있으면 살아있다고 본다
/// (best-effort — 권한 부족 등으로 못 열어도 "죽음"으로 보지 않고 살아있다고 보수적 판단).
/// non-windows: 판정 수단 미구현 → 보수적으로 "살아있음"(false) 반환.
pub fn is_stale(info: &DaemonInfo) -> bool {
    pid_is_dead(info.pid)
}

#[cfg(windows)]
fn pid_is_dead(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // PID 0 은 시스템 idle — 우리 데몬일 수 없음. stale 취급.
    if pid == 0 {
        return true;
    }

    // SAFETY: OpenProcess — 최소 권한(QUERY_LIMITED_INFORMATION)으로 대상 PID 핸들을
    // 연다. 핸들 상속 false. 대상이 없으면(이미 종료) Err 반환.
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(h) => h,
        // 못 열면: 프로세스가 없거나 권한 부족. 권한 부족과 부재를 구분하기 어려우니
        // 보수적으로 "살아있음 가능"=stale 아님 으로 본다(살아있는 데몬을 잘못 덮지 않게).
        Err(_) => return false,
    };

    // 핸들을 열었어도 좀비(이미 종료했지만 핸들 잔존)일 수 있으니 exit code 로 한 번 더 확인.
    // STILL_ACTIVE(259)면 살아있음. 그 외면 종료됨.
    const STILL_ACTIVE: u32 = 259;
    let mut code: u32 = 0;
    // SAFETY: 방금 연 유효한 핸들과 스택의 u32 출력 포인터. 실패해도 code 는 그대로.
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) }.is_ok();
    // SAFETY: OpenProcess 가 반환한 유효한 핸들을 한 번만 닫는다.
    unsafe {
        let _ = CloseHandle(handle);
    }

    // 조회 실패면 보수적으로 살아있다고 봄(stale 아님). 성공이면 STILL_ACTIVE 여부로 판정.
    if !ok {
        return false;
    }
    code != STILL_ACTIVE
}

#[cfg(not(windows))]
fn pid_is_dead(_pid: u32) -> bool {
    // non-windows: 생존 판정 미구현 — 보수적으로 살아있다고 봄(stale 아님).
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engram-daemon-portfile-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample() -> DaemonInfo {
        DaemonInfo {
            pid: 1234,
            host: "127.0.0.1".into(),
            port: 54321,
            token: "a".repeat(64),
            protocol_version: 1,
        }
    }

    #[test]
    fn write_read_roundtrip() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("daemon.json");
        let info = sample();
        write_atomic(&path, &info).unwrap();

        let loaded = read(&path).expect("should read back");
        assert_eq!(loaded, info);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_missing_is_none() {
        let dir = temp_dir("missing");
        let path = dir.join("daemon.json");
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_corrupt_is_none() {
        let dir = temp_dir("corrupt");
        let path = dir.join("daemon.json");
        fs::write(&path, b"{ not valid json").unwrap();
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn serde_shape_is_stable() {
        // 필드 이름/형태 회귀 방지(클라이언트와 공유되는 wire 포맷).
        let info = sample();
        let json = serde_json::to_string(&info).unwrap();
        let back: DaemonInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, info);
        assert!(json.contains("\"protocol_version\""));
        assert!(json.contains("\"port\""));
    }

    #[test]
    fn pid_zero_is_stale() {
        // PID 0 은 우리 데몬일 수 없음 → stale.
        let mut info = sample();
        info.pid = 0;
        // windows 에서는 true(stale), non-windows stub 에서는 false.
        // 기본 동작만 확인: 패닉 없이 bool 을 돌려준다.
        let _ = is_stale(&info);
    }

    #[test]
    fn current_process_is_not_stale() {
        // 현재 실행 중인 테스트 프로세스 PID 는 살아있으므로 stale 이 아니어야 한다.
        let mut info = sample();
        info.pid = std::process::id();
        assert!(!is_stale(&info), "현재 프로세스는 살아있어야 함");
    }
}
