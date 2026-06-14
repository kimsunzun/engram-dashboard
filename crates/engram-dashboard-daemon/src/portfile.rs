//! daemon.json — 데몬 발견(discovery) 파일.
//!
//! 데몬이 잡은 host/port + 접속 토큰 + protocol_version 을 atomic 하게 기록한다.
//! UI(Embedded)나 외부 클라이언트는 이 파일을 읽어 데몬에 붙는다.
//!
//! **구조체는 `protocol::DaemonInfo`** — daemon 이 write, tauri 가 read 하는 두 프로세스의
//! 공유 계약이라 protocol crate 에 있다. 여기엔 daemon 측 IO(write_atomic/read)와 stale
//! 판정만 둔다.
//!
//! **atomic 보장(persistence/mod.rs 와 동일 패턴):** 같은 디렉토리에 tmp 를 쓰고
//! `sync_all` 후 `rename` 한다. 같은 파일시스템 내 rename 이라 교체가 원자적 —
//! 크래시가 나도 daemon.json 은 완전한 옛 내용이거나 완전한 새 내용 둘 중 하나다.
//!
//! **보안:** token 은 이 파일에만 둔다(로그 금지).

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

pub use engram_dashboard_protocol::DaemonInfo;

const TMP_NAME: &str = "daemon.json.tmp";

/// tmp → sync_all → rename. 부모 디렉토리는 호출자가 만들어 두었다고 가정하되,
/// 안전하게 create_dir_all 도 한 번 더 한다(idempotent).
pub fn write_atomic(path: &Path, info: &DaemonInfo) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent dir"))?;
    fs::create_dir_all(dir)?;

    let json = info
        .to_json_pretty()
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
    match DaemonInfo::parse(&bytes) {
        Ok(info) => Some(info),
        Err(e) => {
            tracing::warn!("daemon.json 파싱 실패: {e} — 무시");
            None
        }
    }
}

/// 기록된 데몬이 더 이상 살아있지 않은지(stale) 판정. true=죽음(무시 가능).
///
/// liveness 판정은 core 의 공유 함수(`pid_alive_with_start_time`)에 위임한다 — daemon·tauri
/// 양쪽이 같은 로직을 쓰도록(DRY). "PID 살아있음 AND creation time==기록값"일 때만 살아있다고
/// 본다. start_time==0(미상, 옛 daemon.json)이면 PID 단독 생존으로 보수 판정한다.
///
/// ★PID 재사용(M2) 방어★: 데몬이 죽고 같은 PID 를 다른 프로세스가 받았어도 creation time 이
/// 달라 dead 로 판정 → 엉뚱한 프로세스를 살아있는 데몬으로 오인하지 않는다.
pub fn is_stale(info: &DaemonInfo) -> bool {
    !engram_dashboard_core::pty::platform::pid_alive_with_start_time(info.pid, info.start_time)
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
            start_time: 0,
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
        let json = String::from_utf8(info.to_json_pretty().unwrap()).unwrap();
        let back = DaemonInfo::parse(json.as_bytes()).unwrap();
        assert_eq!(back, info);
        assert!(json.contains("\"protocol_version\""));
        assert!(json.contains("\"port\""));
    }

    #[cfg(windows)]
    #[test]
    fn pid_zero_is_stale() {
        // PID 0 은 우리 데몬일 수 없음 → stale(windows).
        let mut info = sample();
        info.pid = 0;
        assert!(is_stale(&info), "PID 0 은 stale");
    }

    #[test]
    fn current_process_with_unknown_start_time_is_not_stale() {
        // start_time==0(미상, 옛 daemon.json) → PID 생존 fallback. 자기 PID 는 살아있으므로 not stale.
        let mut info = sample();
        info.pid = std::process::id();
        info.start_time = 0;
        assert!(
            !is_stale(&info),
            "미상 start_time + 살아있는 PID → not stale"
        );
    }

    #[cfg(windows)]
    #[test]
    fn current_process_with_matching_start_time_is_not_stale() {
        // 자기 PID + 자기 creation time → not stale(정상 데몬).
        let mut info = sample();
        info.pid = std::process::id();
        info.start_time =
            engram_dashboard_core::pty::platform::current_process_start_time().unwrap();
        assert!(!is_stale(&info), "PID+creation time 일치면 not stale");
    }

    #[cfg(windows)]
    #[test]
    fn current_pid_with_mismatched_start_time_is_stale() {
        // ★PID 재사용 방어★: 같은 PID 라도 creation time 이 다르면 stale(우리 데몬 아님).
        let mut info = sample();
        info.pid = std::process::id();
        let real = engram_dashboard_core::pty::platform::current_process_start_time().unwrap();
        info.start_time = real.wrapping_add(999);
        assert!(is_stale(&info), "creation time 불일치 = 재사용 PID → stale");
    }
}
