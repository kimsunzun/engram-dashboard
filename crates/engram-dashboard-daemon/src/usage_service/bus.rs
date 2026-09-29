//! 버스 `usage.*` 의 데몬 실물 — agent 포트 [`UsageCommandHost`] 를 서비스에 단다(TRD S21 usage-limit-slot §1-4
//! 「버스」 · §1-6 「버스」).
//!
//! ★요청은 [`UsageService::request_blocking`] 하나로 든다★ — 버스 입구는 blocking 풀 스레드라 std 채널로 기다리고
//!   (§3 #17 — tokio 타이머를 안 쓴다), 판정은 wire ⟳ 와 같은 책·같은 요청 표에서 한다.
//! ★행은 답 한 장을 편 것뿐이다★ — 벤더 이름·정책이 없다(모듈 머리 「백엔드 확장」). 칸은 들어온 철자가 아니라
//!   조회기의 키로 찾는다([`usage_vendor_of`] — wire 입구와 같은 길).
// ADR-0004
// ADR-0155

use engram_dashboard_agent::commands::{
    usage_vendor_of, AgentBackend, UsageCommandHost, UsageServedWord, UsageStateWord,
    UsageVendorRow, UsageWindowRow, UsageWindowWord,
};
use engram_dashboard_protocol::{UsageStateDetail, UsageVendorState, UsageWindow};

use std::time::Duration;

use super::book::RequestKind;
use super::{UsageAnswer, UsageServed, UsageService, REPLY_WAIT_MAX};
use crate::command_delivery::CommandDeliveries;

// 기다리는 `usage.*` 는 버스 수거의 TIMEOUT 보다 먼저 답해야 한다 — 대기 상한에 여유 1초를 더해도 명령 마감 안.
const _: () = assert!(
    REPLY_WAIT_MAX.as_millis() + Duration::from_secs(1).as_millis()
        < CommandDeliveries::DEFAULT_DEADLINE.as_millis(),
    "사용량 답 대기 상한 + 1초가 버스 명령 마감을 넘는다 — 기다리는 usage.* 가 TIMEOUT 으로 끊긴다"
);

impl UsageCommandHost for UsageService {
    fn get(&self, backend: &AgentBackend) -> Option<UsageVendorRow> {
        self.bus_request(backend, RequestKind::Get)
    }

    fn refresh(&self, backend: &AgentBackend) -> Option<UsageVendorRow> {
        self.bus_request(backend, RequestKind::Refresh)
    }
}

impl UsageService {
    /// `None` = 그 낱말의 조회기가 없거나 이 서비스가 그 칸을 안 든다(조회기 0개 조립).
    ///
    /// ★대기 상한([`REPLY_WAIT_MAX`])은 핸들러에 든 순간부터, 버스 마감은 자리가 열린 순간부터 잰다★ — 그 사이의
    ///   blocking 풀 지연은 두 상수의 차(약 2초)가 받아 준다. 자리 마감에서 그 지연을 따로 빼 셈하지 않는다 — 풀이
    ///   그보다 밀리면 답 대신 TIMEOUT 이 나가는 것은 알고 남긴 잔여다.
    fn bus_request(&self, backend: &AgentBackend, kind: RequestKind) -> Option<UsageVendorRow> {
        let vendor = usage_vendor_of(backend)?;
        let answer = self.request_blocking(vendor, kind)?;
        Some(vendor_row(backend.clone(), answer))
    }
}

/// 답 한 장 → 버스 행. `backend` = 요청의 낱말 그대로 — 칸은 그 낱말의 조회기 키로 찾았으므로 같은 백엔드다.
///
/// 창 차례 = 5시간 · 주간 · 모델별(스냅숏 차례 그대로). 상태의 칸(`next_attempt_in_secs`·`retry_in_secs`·detail)은
/// 그 상태일 때만 싣는다 — 매크로 enum 이 문자열 열거뿐이라 태그 enum 을 단어 + 선택 칸으로 편다(§1-6).
pub(crate) fn vendor_row(backend: AgentBackend, answer: UsageAnswer) -> UsageVendorRow {
    let UsageAnswer { snapshot, served } = answer;
    let mut windows = Vec::with_capacity(2 + snapshot.model_scoped.len());
    if let Some(window) = snapshot.five_hour {
        windows.push(window_row(UsageWindowWord::FiveHour, None, window));
    }
    if let Some(window) = snapshot.weekly {
        windows.push(window_row(UsageWindowWord::Weekly, None, window));
    }
    for scoped in snapshot.model_scoped {
        windows.push(window_row(
            UsageWindowWord::ModelWeekly,
            Some(scoped.label),
            scoped.window,
        ));
    }
    let (state, next_attempt_in_secs, retry_in_secs, detail) = match snapshot.state {
        UsageVendorState::Ready => (UsageStateWord::Ready, None, None, None),
        UsageVendorState::NotInstalled { detail } => {
            (UsageStateWord::NotInstalled, None, None, detail)
        }
        UsageVendorState::NeedsLogin { detail } => (UsageStateWord::NeedsLogin, None, None, detail),
        UsageVendorState::Unavailable { detail } => {
            (UsageStateWord::Unavailable, None, None, detail)
        }
        UsageVendorState::Failed {
            next_attempt_in_secs,
            detail,
        } => (
            UsageStateWord::Failed,
            Some(next_attempt_in_secs),
            None,
            detail,
        ),
        UsageVendorState::Rejected {
            retry_in_secs,
            detail,
        } => (UsageStateWord::Rejected, None, Some(retry_in_secs), detail),
    };
    let (detail_kind, detail_code, upstream) = match detail {
        Some(UsageStateDetail {
            kind,
            code,
            upstream,
        }) => (Some(kind), code, upstream),
        None => (None, None, None),
    };
    UsageVendorRow {
        backend,
        account_key: snapshot.account_key,
        plan: snapshot.plan,
        windows,
        in_flight: snapshot.in_flight,
        served: match served {
            UsageServed::Fresh => UsageServedWord::Fresh,
            UsageServed::Cached => UsageServedWord::Cached,
        },
        state,
        next_attempt_in_secs,
        retry_in_secs,
        detail_kind,
        detail_code,
        upstream,
    }
}

fn window_row(
    window: UsageWindowWord,
    label: Option<String>,
    value: UsageWindow,
) -> UsageWindowRow {
    UsageWindowRow {
        window,
        label,
        used_pct: value.used_pct,
        left_pct: value.used_pct.map(left_pct),
        resets_at: value.resets_at,
        age_secs: value.age_secs,
        expired: value.expired,
    }
}

/// 남은 양 — 화면과 같은 `floor(clamp(100 − used, 0, 100))`(TRD §1-6).
///
/// ★반올림이 아니라 내림이다★ — 화면과 LLM 이 같은 수를 봐야 하고, 올리면 남은 양을 부풀리는 쪽으로 틀린다.
///   `used_pct` 는 생산자가 소수 넷째 자리로 다듬어 싣는다(§3 #32 — `100 − 55.00000000000001` 이 44 로 떨어지는
///   부동소수 꼬리를 거기서 막는다). 유한수가 아닌 값은 오지 않지만 오면 `as` 가 0 으로 접는다(패닉 없음).
fn left_pct(used_pct: f64) -> u64 {
    (100.0 - used_pct).clamp(0.0, 100.0).floor() as u64
}

#[cfg(test)]
mod tests {
    use engram_dashboard_protocol::{AgentBackendKind, UsageLimitSnapshot, UsageScopedWindow};

    use super::*;

    fn window(used_pct: Option<f64>) -> UsageWindow {
        UsageWindow {
            used_pct,
            resets_at: Some(1_900_000_000),
            age_secs: 12,
            expired: false,
        }
    }

    fn snapshot(state: UsageVendorState) -> UsageLimitSnapshot {
        UsageLimitSnapshot {
            vendor: AgentBackendKind::Claude,
            account_key: "default".to_owned(),
            five_hour: Some(window(Some(40.0))),
            weekly: None,
            model_scoped: vec![UsageScopedWindow {
                label: "opus".to_owned(),
                window: window(None),
            }],
            plan: Some("max".to_owned()),
            in_flight: true,
            state,
            revision: 9,
        }
    }

    /// ★도움말의 수가 상수와 같다★ — 호출자(LLM)가 읽는 것은 요약에 적힌 수뿐이라, 상수만 바꾸면 광고가 조용히
    /// 거짓이 된다. 요약은 agent 선언이고 상수는 이 crate 것이라 컴파일러가 둘을 잇지 않는다.
    #[test]
    fn the_usage_help_quotes_the_wait_constant() {
        use engram_dashboard_agent::commands::{UsageGetArgs, UsageRefreshArgs};
        use engram_dashboard_command::spec_item_json;

        assert_eq!(
            REPLY_WAIT_MAX.subsec_nanos(),
            0,
            "요약은 초 단위로 적는다: {REPLY_WAIT_MAX:?}"
        );
        let wait = format!("최대 {}초", REPLY_WAIT_MAX.as_secs());
        let get = spec_item_json(&UsageGetArgs::SPEC);
        let refresh = spec_item_json(&UsageRefreshArgs::SPEC);
        assert!(
            get.contains(&wait),
            "`usage.get` 도움말에 {wait} 가 없다: {get}"
        );
        assert!(
            refresh.contains(&wait),
            "`usage.refresh` 도움말에 {wait} 가 없다: {refresh}"
        );
    }

    /// ★내림이다★ — 반올림으로 바꾸면 첫 셋 중 둘이 깨진다. 범위 밖 사용률은 0·100 에 눌린다.
    #[test]
    fn left_pct_rounds_down_and_clamps() {
        assert_eq!(left_pct(55.5), 44);
        assert_eq!(left_pct(0.4), 99);
        assert_eq!(left_pct(12.0), 88);
        assert_eq!(left_pct(0.0), 100);
        assert_eq!(left_pct(100.0), 0);
        assert_eq!(left_pct(130.0), 0);
        assert_eq!(left_pct(-5.0), 100);
    }

    /// 창 차례·모델 이름·모름(`null`)이 그대로 건너온다 — `used_pct` 가 없는 창의 `left_pct` 는 0 이 아니라 `null`.
    #[test]
    fn windows_keep_their_order_label_and_unknowns() {
        let row = vendor_row(
            AgentBackend::Claude,
            UsageAnswer {
                snapshot: snapshot(UsageVendorState::Ready),
                served: UsageServed::Fresh,
            },
        );
        assert_eq!(row.backend, AgentBackend::Claude);
        assert_eq!(row.account_key, "default");
        assert_eq!(row.plan.as_deref(), Some("max"));
        assert!(row.in_flight);
        assert_eq!(row.served, UsageServedWord::Fresh);
        assert_eq!(row.state, UsageStateWord::Ready);
        let words: Vec<_> = row.windows.iter().map(|w| w.window.clone()).collect();
        assert_eq!(
            words,
            vec![UsageWindowWord::FiveHour, UsageWindowWord::ModelWeekly]
        );
        assert_eq!(row.windows[0].label, None);
        assert_eq!(row.windows[0].left_pct, Some(60));
        assert_eq!(row.windows[0].resets_at, Some(1_900_000_000));
        assert_eq!(row.windows[0].age_secs, 12);
        assert_eq!(row.windows[1].label.as_deref(), Some("opus"));
        assert_eq!(row.windows[1].used_pct, None);
        assert_eq!(row.windows[1].left_pct, None, "모름은 0 이 아니다");
        assert_eq!(
            (row.detail_kind, row.detail_code, row.upstream),
            (None, None, None)
        );
    }

    /// 상태 여섯이 단어 여섯으로 1:1 이고, 상태의 칸은 그 상태일 때만 실린다. detail 은 세 칸으로 펴진다.
    #[test]
    fn every_wire_state_maps_to_its_word_and_carries_only_its_own_fields() {
        let detail = || {
            Some(UsageStateDetail {
                kind: "rpc_error".to_owned(),
                code: Some(-32_603),
                upstream: Some("internal error".to_owned()),
            })
        };
        let cases = [
            (UsageVendorState::Ready, UsageStateWord::Ready, None, None),
            (
                UsageVendorState::NotInstalled { detail: detail() },
                UsageStateWord::NotInstalled,
                None,
                None,
            ),
            (
                UsageVendorState::NeedsLogin { detail: detail() },
                UsageStateWord::NeedsLogin,
                None,
                None,
            ),
            (
                UsageVendorState::Unavailable { detail: detail() },
                UsageStateWord::Unavailable,
                None,
                None,
            ),
            (
                UsageVendorState::Failed {
                    next_attempt_in_secs: 70,
                    detail: detail(),
                },
                UsageStateWord::Failed,
                Some(70),
                None,
            ),
            (
                UsageVendorState::Rejected {
                    retry_in_secs: 300,
                    detail: detail(),
                },
                UsageStateWord::Rejected,
                None,
                Some(300),
            ),
        ];
        for (state, word, next, retry) in cases {
            let ready = matches!(state, UsageVendorState::Ready);
            let row = vendor_row(
                AgentBackend::Codex,
                UsageAnswer {
                    snapshot: snapshot(state),
                    served: UsageServed::Cached,
                },
            );
            assert_eq!(row.state, word);
            assert_eq!(row.served, UsageServedWord::Cached);
            assert_eq!(row.next_attempt_in_secs, next, "{word:?}");
            assert_eq!(row.retry_in_secs, retry, "{word:?}");
            if ready {
                assert_eq!(row.detail_kind, None);
            } else {
                assert_eq!(row.detail_kind.as_deref(), Some("rpc_error"), "{word:?}");
                assert_eq!(row.detail_code, Some(-32_603), "{word:?}");
                assert_eq!(row.upstream.as_deref(), Some("internal error"), "{word:?}");
            }
        }
    }
}
