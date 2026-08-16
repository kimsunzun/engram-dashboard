//! `daemon.json` — 데몬 발견(discovery) 파일이자 **단일 인스턴스 잠금 파일**(ADR-0135 결정 1).
//!
//! 여기엔 파일 이름·daemon 측 쓰기/읽기·stale 판정만 둔다. 그 파일을 **붙잡는** 쪽은
//! [`crate::instance`] 이고, crate 밖에서 오는 쓰기는 거기(guard)를 지나야 한다.
//!
//! ★[`read`] 의 로그가 토큰을 흘리지 않는지 지키는 것은 지금 이 주석뿐이다(알려진 공백)★: 파싱 실패
//! 시 serde 에러의 `Display` 를 찍는데, 그 안엔 위치·기대 타입만 담기고 값은 담기지 않는다 —
//! `token` 이 `String` 이라 `invalid type` 류의 값 에코가 나올 자리가 없기 때문이다. 필드 타입을
//! 바꾸면 그 전제가 깨진다. 같은 함정을 `ws.rs` 는 회귀 테스트로 막는데 여기엔 테스트가 없다.
//!
//! ★원자적 교체(임시 파일 + rename)를 되살리지 마라★: 데몬이 이 파일을 삭제 공유 없이 쥐고 있어
//! rename 이 거부된다(실측 `ERROR_SHARING_VIOLATION`). 그래서 [`write_in_place`] 는 보유 중인 핸들에
//! 제자리로 쓴다.
//!
//! ★그래서 읽는 쪽은 부분적으로 쓰인 내용을 볼 수 있다★: 파싱 실패는 손상이 아니라 **아직 준비 안 됨**
//! 으로 다뤄야 한다([`read`] 가 `None` 을 주는 이유). 클라이언트는 이미 50ms 로 폴링한다.
//!
//! ★사라진 파일을 주기적으로 다시 발행하지 마라(되살리지 마라)★: 실행 중인 앱이 사용자가 방금 지운
//! 파일을 말없이 다시 쓰는 동작은 조사한 선례 전부에서 버그로 신고돼 있다(ADR-0135 거부한 대안 1번).
//! 합친 뒤로는 지워지지도 않으므로 되살릴 일 자체가 없다.
//!
//! **보안:** token 은 이 파일에만 둔다(로그 금지).

use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;

pub use engram_dashboard_protocol::DaemonInfo;

/// 데이터 폴더 안 접속·잠금 파일 이름. 런타임 생성이라 배포판 압축에는 없다.
///
/// ★데몬은 이 상수로만 경로를 만들어야 한다★: 잡는 파일과 쓰는 파일이 갈리면 단일성이 조용히 깨진다.
pub const DAEMON_FILE: &str = "daemon.json";

/// 데몬이 **보유 중인 핸들**로 접속 레코드를 제자리에 쓴다.
///
/// ★crate 밖 진입점은 [`crate::instance::InstanceGuard::publish`] 하나다★ — 그래서 `pub(crate)` 다.
/// 이 함수를 `pub` 으로 열거나 경로(`&Path`)를 받는 형태로 되돌리면, 소유를 증명하지 않은 쪽이 발행할
/// 수 있게 되고 두 데몬의 쓰기가 섞인다.
///
/// ★단 타입이 막는 것은 거기까지다★: crate 안(이 파일의 테스트 포함)에서는 임의의 `File` 로 부를 수
/// 있고, **실제 강제는 보유 중인 OS 공유 모드**다 — 획득 전이나 Drop 뒤에는 아무것도 막지 않는다.
///
/// 길이를 먼저 0으로 줄이므로 그 사이 읽는 쪽은 빈 파일을 볼 수 있다 — 모듈 헤더의 "아직 준비 안 됨"
/// 계약이 그 창을 덮는다.
// ADR-0135
pub(crate) fn write_in_place(f: &mut File, info: &DaemonInfo) -> io::Result<()> {
    let json = info
        .to_json_pretty()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    f.set_len(0)?;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(&json)?;
    // 다른 프로세스의 가시성은 write_all 시점에 이미 확보된다(같은 캐시를 본다). sync_all 은 전원이
    //   끊겨도 주소가 남게 하는 내구성 몫이고, 부팅당 1회라 비용이 문제되지 않는다.
    f.sync_all()
}

/// 없거나 파싱 불가면 None.
///
/// ★파싱 실패를 손상으로 승격하지 말 것★: 데몬이 쓰는 도중일 수 있다(모듈 헤더). 호출자는 "아직
/// 준비 안 됨"으로 보고 다시 본다.
pub fn read(path: &Path) -> Option<DaemonInfo> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return None,
    };
    match DaemonInfo::parse(&bytes) {
        Ok(info) => Some(info),
        Err(e) => {
            tracing::warn!("{DAEMON_FILE} 파싱 실패: {e} — 무시(쓰는 중일 수 있음)");
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
    !engram_dashboard_core::agent::platform::pid_alive_with_start_time(info.pid, info.start_time)
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

    fn open_held(path: &Path) -> File {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .unwrap()
    }

    #[test]
    fn write_read_roundtrip() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(DAEMON_FILE);
        let info = sample();
        write_in_place(&mut open_held(&path), &info).unwrap();

        let loaded = read(&path).expect("should read back");
        assert_eq!(loaded, info);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 제자리 쓰기라 옛 내용이 더 길면 꼬리가 남을 수 있다 — set_len 이 그것을 자른다는 단언.
    #[test]
    fn rewrite_leaves_no_tail_of_the_previous_record() {
        let dir = temp_dir("no-tail");
        let path = dir.join(DAEMON_FILE);
        let mut long = sample();
        long.token = "b".repeat(4096);
        let mut f = open_held(&path);
        write_in_place(&mut f, &long).unwrap();

        let short = sample();
        write_in_place(&mut f, &short).unwrap();
        assert_eq!(
            read(&path).expect("재파싱"),
            short,
            "옛 꼬리가 남으면 안 됨"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_missing_is_none() {
        let dir = temp_dir("missing");
        let path = dir.join(DAEMON_FILE);
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    /// ★부분 기록 = 준비 안 됨★: 제자리 쓰기라 이 상태를 클라이언트가 실제로 볼 수 있다.
    #[test]
    fn read_partial_is_none_not_an_error() {
        let dir = temp_dir("partial");
        let path = dir.join(DAEMON_FILE);
        let full = sample().to_json_pretty().unwrap();
        fs::write(&path, &full[..full.len() / 2]).unwrap();
        assert!(read(&path).is_none());
        // 빈 파일(길이 0으로 줄인 직후의 창)도 같다.
        fs::write(&path, b"").unwrap();
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_corrupt_is_none() {
        let dir = temp_dir("corrupt");
        let path = dir.join(DAEMON_FILE);
        fs::write(&path, b"{ not valid json").unwrap();
        assert!(read(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn serde_shape_is_stable() {
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
        let mut info = sample();
        info.pid = 0;
        assert!(is_stale(&info), "PID 0 은 stale");
    }

    #[test]
    fn current_process_with_unknown_start_time_is_not_stale() {
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
        let mut info = sample();
        info.pid = std::process::id();
        info.start_time =
            engram_dashboard_core::agent::platform::current_process_start_time().unwrap();
        assert!(!is_stale(&info), "PID+creation time 일치면 not stale");
    }

    #[cfg(windows)]
    #[test]
    fn current_pid_with_mismatched_start_time_is_stale() {
        let mut info = sample();
        info.pid = std::process::id();
        let real = engram_dashboard_core::agent::platform::current_process_start_time().unwrap();
        info.start_time = real.wrapping_add(999);
        assert!(is_stale(&info), "creation time 불일치 = 재사용 PID → stale");
    }
}
