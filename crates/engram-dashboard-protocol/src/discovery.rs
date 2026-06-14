//! 데몬 발견(discovery) 공유 계약 — daemon.json 의 내용.
//!
//! 두 프로세스의 공유 계약이라 protocol 에 둔다:
//!   - daemon 이 이 구조체를 atomic 하게 **기록**한다(portfile::write_atomic).
//!   - Embedded Tauri(또는 외부 클라)가 **읽어** 데몬에 붙는다(discovery::ensure_daemon).
//!
//! ★ts-rs export 안 함★: 프론트가 직접 안 읽는 Rust 전용 IPC 파일이다(daemon.json 은
//! 백엔드 두 프로세스 사이에서만 흐른다). 그래서 serde 만 달고 TS 바인딩은 만들지 않는다.
//!
//! **보안:** `token` 은 이 파일에만 둔다(로그 금지).

use serde::{Deserialize, Serialize};

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
    /// 데몬 프로세스의 시작시각(Windows GetProcessTimes 의 creation FILETIME 을 u64 로:
    /// 1601-01-01 UTC 부터 100나노초 간격 수). PID 재사용을 구분해 liveness 를 정확히 판정한다
    /// (PID 살아있음 AND creation time 일치). 0=미상.
    ///
    /// ★wire 호환★: append 필드라 옛 reader 는 무시한다. 단, 옛 daemon.json(필드 없음)도
    /// 파싱돼야 하므로 `#[serde(default)]`(없으면 0). default=0 은 "미상"으로 취급해 liveness 가
    /// PID 단독 생존으로 보수 판정한다(살아있는 데몬을 잘못 stale 로 몰지 않게).
    #[serde(default)]
    pub start_time: u64,
}

impl DaemonInfo {
    /// daemon.json 바이트를 파싱한다. 파일 IO 와 분리한 **순수 함수** —
    /// 호출자가 읽은 bytes 를 넘겨 테스트 가능하게 한다(파일시스템 불필요).
    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// daemon.json 으로 직렬화(atomic write 가 쓸 pretty JSON).
    pub fn to_json_pretty(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DaemonInfo {
        DaemonInfo {
            pid: 4242,
            host: "127.0.0.1".into(),
            port: 51234,
            token: "b".repeat(64),
            protocol_version: 1,
            start_time: 133_000_000_000_000_000,
        }
    }

    #[test]
    fn serde_roundtrip_is_identity() {
        // 직렬화→역직렬화 가 동일 값을 복원해야 한다(공유 계약 골든).
        let info = sample();
        let bytes = info.to_json_pretty().unwrap();
        let back = DaemonInfo::parse(&bytes).unwrap();
        assert_eq!(back, info);
    }

    #[test]
    fn parse_valid_json_succeeds() {
        let json =
            br#"{"pid":7,"host":"127.0.0.1","port":9,"token":"deadbeef","protocol_version":1}"#;
        let info = DaemonInfo::parse(json).expect("valid json should parse");
        assert_eq!(info.pid, 7);
        assert_eq!(info.port, 9);
        assert_eq!(info.token, "deadbeef");
    }

    #[test]
    fn parse_corrupt_json_errors() {
        // 깨진 json → Err(파일 IO 없이 순수 파싱 실패 확인).
        assert!(DaemonInfo::parse(b"{ not valid json").is_err());
    }

    #[test]
    fn parse_missing_field_errors() {
        // 필수 필드 누락(token 없음) → Err. wire 형태 회귀 방지.
        let json = br#"{"pid":1,"host":"127.0.0.1","port":2,"protocol_version":1}"#;
        assert!(DaemonInfo::parse(json).is_err());
    }

    #[test]
    fn json_field_names_are_stable() {
        // 필드 이름 회귀 방지(daemon write ↔ tauri read 공유 wire).
        let json = String::from_utf8(sample().to_json_pretty().unwrap()).unwrap();
        for f in [
            "pid",
            "host",
            "port",
            "token",
            "protocol_version",
            "start_time",
        ] {
            assert!(json.contains(&format!("\"{f}\"")), "필드 {f} 누락");
        }
    }

    #[test]
    fn parse_old_json_without_start_time_defaults_to_zero() {
        // ★wire 호환★: start_time 필드 이전에 쓰인 옛 daemon.json(필드 없음)도 파싱되어야 한다.
        // #[serde(default)] 덕에 누락 시 0(미상)으로 채운다 — 역직렬화 실패 금지.
        let json =
            br#"{"pid":7,"host":"127.0.0.1","port":9,"token":"deadbeef","protocol_version":1}"#;
        let info = DaemonInfo::parse(json).expect("옛 파일(start_time 없음)도 파싱돼야 함");
        assert_eq!(info.start_time, 0, "누락 시 default=0(미상)");
        assert_eq!(info.pid, 7);
    }

    #[test]
    fn start_time_roundtrips() {
        // start_time 값이 직렬화→역직렬화로 보존되는지(append 필드 회귀 방지).
        let info = sample();
        let back = DaemonInfo::parse(&info.to_json_pretty().unwrap()).unwrap();
        assert_eq!(back.start_time, info.start_time);
    }
}
