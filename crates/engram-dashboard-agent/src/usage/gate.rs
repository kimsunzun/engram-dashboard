//! 줍기 차단 — 기본 계정이 아닌 에이전트의 디코더가 내는 사용량 관측을 흘려보내지 않는다.
//!
//! ★왜 막나★: 받는 쪽의 사용량 칸은 `(벤더, 기본 계정)` 하나다. 다른 계정으로 뜬 에이전트의 관측을
//!   흘려보내면 그 계정의 값이 기본 계정 칸에 섞인다. 어느 env 가 계정을 가르는지는 벤더 지식이라 판정은
//!   각 벤더 backend 가 하고, 이 모듈은 그 판정을 적용하는 감싸개와 env 비교만 갖는다.

use std::ffi::OsStr;

use crate::transport::OutputDecoder;
use crate::types::OutputEvent;

use super::UsageObservation;

/// 사용량을 막는 디코더 감싸개 — [`OutputDecoder::take_usage`] 가 늘 빈 벡터이고, 출력 이벤트는 그대로
/// 통과한다. 줍는 에이전트의 디코더는 감싸지 않는다 — 감싸는 것은 줍지 않기로 판정된 에이전트뿐이다(판정 =
/// 각 벤더 backend 의 `gate_decoder`).
///
/// ★trait 의 메서드를 **전부** 안쪽으로 넘긴다 — 기본 몸체가 있는 것까지★: 넘기지 않은 메서드는
///   컴파일은 되지만 감싼 순간 기본 동작으로 바뀐다. 그 피해가 닿는 범위는 기본 계정이 아닌 에이전트뿐이다
///   (감싸는 것이 그쪽뿐이라). 오늘 목록 = `decode` · `flush` · `take_usage`. trait 에 메서드가 늘면 여기에도
///   넘기기를 더한다.
pub struct UsageGate {
    inner: Box<dyn OutputDecoder>,
}

impl UsageGate {
    pub fn blocking(inner: Box<dyn OutputDecoder>) -> Self {
        Self { inner }
    }
}

impl OutputDecoder for UsageGate {
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
        self.inner.decode(chunk)
    }

    fn flush(&mut self) -> Vec<OutputEvent> {
        self.inner.flush()
    }

    fn take_usage(&mut self) -> Vec<UsageObservation> {
        // 막아도 안쪽은 비운다 — 안 비우면 에이전트가 사는 내내 안쪽 벡터가 자란다.
        drop(self.inner.take_usage());
        Vec::new()
    }
}

/// 스폰 명세의 env 가 `key` 를 데몬 자신의 값과 **다르게** 두는가 — 계정 env 에 대한 줍기 판정의 중립 부분.
///
/// - `spec_env` = 자식에게 **더하는** env(자식은 나머지를 데몬에게서 물려받는다). 같은 키가 여럿이면 마지막이
///   이긴다(스폰이 차례로 적용한다).
/// - `daemon_value` = 데몬 프로세스 자신의 그 env 값. 호출자가 읽어 넘긴다 — 이 함수는 프로세스 env 를
///   읽지 않는다.
/// - 결과: 명세에 그 키가 없으면 `false`(물려받는다) · 있고 데몬 값과 같으면 `false` · 데몬에는 없는데 명세에
///   있거나 값이 다르면 `true`.
///
/// ★키는 ASCII 대소문자를 가리지 않고 비교하고, 값은 바이트 그대로 비교한다★ — 틀린다면 「다르다」 쪽으로
///   틀리게 골랐다. 그쪽의 대가는 그 에이전트의 관측 몇 건을 줍지 않는 것뿐이지만, 반대쪽은 다른 계정의
///   값이 기본 칸에 섞인다. 그래서 경로 정규화(끝 구분자·대소문자)도, 빈 값을 「없음」으로 접는 것도 하지
///   않는다. (Windows 는 env 이름이 대소문자를 가리지 않는다 — POSIX 에서는 이 비교가 더 넓게 잡을 뿐이다.)
pub fn env_overrides_daemon(
    spec_env: &[(String, String)],
    key: &str,
    daemon_value: Option<&OsStr>,
) -> bool {
    let Some((_, spec_value)) = spec_env
        .iter()
        .rev()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
    else {
        return false;
    };
    daemon_value != Some(OsStr::new(spec_value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{UsageSource, UsageVendorKey, WindowObs};
    use std::sync::{Arc, Mutex};

    fn observation(pct: f64) -> UsageObservation {
        UsageObservation {
            vendor: UsageVendorKey::new("test-vendor"),
            five_hour: Some(WindowObs {
                used_pct: Some(pct),
                resets_at: Some(1_900_000_000),
            }),
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Passive,
            limits_unavailable: false,
        }
    }

    /// 메서드마다 자기만의 산출을 내는 디코더 — 감싸개가 어느 하나를 안쪽으로 안 넘기면 그 산출이 사라진다.
    /// 쌓아 둔 관측(`pending`)은 밖에서도 보인다 — 감싸개가 안쪽을 비웠는지를 거기서 잰다.
    struct Distinct {
        decoded: Arc<Mutex<Vec<u8>>>,
        pending: Arc<Mutex<Vec<UsageObservation>>>,
    }

    impl OutputDecoder for Distinct {
        fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
            self.decoded.lock().unwrap().extend_from_slice(chunk);
            self.pending.lock().unwrap().push(observation(40.0));
            vec![OutputEvent::Error("from-decode".into())]
        }
        fn flush(&mut self) -> Vec<OutputEvent> {
            self.pending.lock().unwrap().push(observation(50.0));
            vec![OutputEvent::Error("from-flush".into())]
        }
        fn take_usage(&mut self) -> Vec<UsageObservation> {
            std::mem::take(&mut *self.pending.lock().unwrap())
        }
    }

    type Pending = Arc<Mutex<Vec<UsageObservation>>>;

    fn gate() -> (UsageGate, Arc<Mutex<Vec<u8>>>, Pending) {
        let decoded = Arc::new(Mutex::new(Vec::new()));
        let pending: Pending = Arc::new(Mutex::new(Vec::new()));
        let inner = Distinct {
            decoded: decoded.clone(),
            pending: pending.clone(),
        };
        (UsageGate::blocking(Box::new(inner)), decoded, pending)
    }

    fn error_texts(events: &[OutputEvent]) -> Vec<&str> {
        events
            .iter()
            .map(|e| match e {
                OutputEvent::Error(s) => s.as_str(),
                other => panic!("예상 밖 이벤트: {other:?}"),
            })
            .collect()
    }

    // ── 전달(기본 몸체가 있는 `take_usage` 포함) ──

    #[test]
    fn blocking_gate_forwards_every_method_but_never_usage() {
        let (mut g, decoded, pending) = gate();
        assert_eq!(error_texts(&g.decode(b"xyz")), vec!["from-decode"]);
        assert_eq!(decoded.lock().unwrap().as_slice(), b"xyz");
        assert!(g.take_usage().is_empty());
        assert_eq!(error_texts(&g.flush()), vec!["from-flush"]);
        assert!(g.take_usage().is_empty());
        assert!(
            pending.lock().unwrap().is_empty(),
            "take_usage 를 안쪽으로 안 넘기면 trait 기본(빈 벡터)이 되고 안쪽은 안 비워진다"
        );
    }

    /// 막는 감싸개도 안쪽을 비운다 — 아니면 안쪽 벡터가 에이전트 수명 내내 자란다.
    #[test]
    fn blocking_gate_still_drains_the_inner_decoder() {
        let (mut g, _, pending) = gate();
        for _ in 0..3 {
            g.decode(b".");
            assert!(g.take_usage().is_empty());
            assert!(
                pending.lock().unwrap().is_empty(),
                "막는 동안 버린 관측이 안쪽에 남아 있으면 안 된다"
            );
        }
    }

    // ── env 판정 ──

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn missing_key_inherits_the_daemon_value() {
        assert!(!env_overrides_daemon(&env(&[]), "ACCOUNT_DIR", None));
        assert!(!env_overrides_daemon(
            &env(&[("OTHER", "x")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("d"))
        ));
    }

    #[test]
    fn equal_value_is_not_an_override() {
        assert!(!env_overrides_daemon(
            &env(&[("ACCOUNT_DIR", "C:\\a")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("C:\\a"))
        ));
    }

    #[test]
    fn set_against_unset_or_different_is_an_override() {
        assert!(env_overrides_daemon(
            &env(&[("ACCOUNT_DIR", "C:\\a")]),
            "ACCOUNT_DIR",
            None
        ));
        assert!(env_overrides_daemon(
            &env(&[("ACCOUNT_DIR", "C:\\b")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("C:\\a"))
        ));
        assert!(
            env_overrides_daemon(&env(&[("ACCOUNT_DIR", "")]), "ACCOUNT_DIR", None),
            "빈 값도 「있다」로 본다"
        );
        assert!(
            env_overrides_daemon(
                &env(&[("ACCOUNT_DIR", "C:\\a\\")]),
                "ACCOUNT_DIR",
                Some(OsStr::new("C:\\a"))
            ),
            "경로 정규화를 하지 않는다 — 다르다 쪽으로 틀린다"
        );
    }

    #[test]
    fn key_match_ignores_ascii_case_and_last_one_wins() {
        assert!(env_overrides_daemon(
            &env(&[("account_dir", "C:\\b")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("C:\\a"))
        ));
        assert!(!env_overrides_daemon(
            &env(&[("ACCOUNT_DIR", "C:\\b"), ("Account_Dir", "C:\\a")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("C:\\a"))
        ));
        assert!(env_overrides_daemon(
            &env(&[("ACCOUNT_DIR", "C:\\a"), ("ACCOUNT_DIR", "C:\\b")]),
            "ACCOUNT_DIR",
            Some(OsStr::new("C:\\a"))
        ));
    }
}
