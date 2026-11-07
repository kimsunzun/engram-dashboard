//! The daemon's real implementation of bus `usage.*` — attaches the agent port [`UsageCommandHost`] to the service (TRD S21 usage-limit-slot §1-4
//! 「버스」 (bus) · §1-6 「버스」 (bus) · ADR-0251).
//!
//! ★A request is held by [`UsageService::request_blocking`] alone★ — the bus entrance is a blocking pool thread so it waits on a std channel
//!   (§3 #17 — the tokio timer is not used), and the decision is made from the same book and the same request table as the wire ⟳.
//! ★A row is just one answer sheet unfolded★ — it has no vendor name or policy (module header 「백엔드 확장」 (backend extension)). The cell is found not by the spelling that came in but by
//!   the prober's key ([`usage_vendor_of`] — the same path as the wire entrance).
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

// A waiting `usage.*` must answer before the bus pickup's TIMEOUT — even adding a 1-second margin to the wait cap it is within the command deadline.
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
    /// `None` = there is no prober for that word or this service does not hold that cell (assembly with 0 probers).
    ///
    /// ★The wait cap ([`REPLY_WAIT_MAX`]) is measured from the moment it enters the handler, and the bus deadline from the moment the slot opens★ — the
    ///   blocking pool delay in between is absorbed by the difference of the two constants (about 2 seconds). The slot deadline does not separately subtract that delay — if the pool
    ///   lags beyond that, a TIMEOUT goes out instead of an answer, a residue we knowingly left.
    fn bus_request(&self, backend: &AgentBackend, kind: RequestKind) -> Option<UsageVendorRow> {
        let vendor = usage_vendor_of(backend)?;
        let answer = self.request_blocking(vendor, kind)?;
        Some(vendor_row(backend.clone(), answer))
    }
}

/// One answer sheet → bus row. `backend` = the request's word as is — the cell was found by that word's prober key so it is the same backend.
///
/// Window order = 5-hour · weekly · per-model (snapshot order as is). The state's fields (`next_attempt_in_secs`·`retry_in_secs`·detail)
/// are carried only in that state — the macro enum is only a string enumeration so a tag enum is unfolded into a word + optional fields (§1-6).
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

/// Amount left — the same `floor(clamp(100 − used, 0, 100))` as the screen (TRD §1-6).
///
/// ★It is rounding down, not rounding★ — the screen and the LLM must see the same number, and rounding up errs toward inflating the amount left.
///   `used_pct` is carried by the producer trimmed to the fourth decimal place (§3 #32 — it blocks there the floating-point tail where `100 − 55.00000000000001` falls to 44).
///   A non-finite value does not come, but if it does `as` folds it to 0 (no panic).
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

    /// ★The number in the help equals the constant★ — all the caller (the LLM) reads is the number in the summary, so changing only the constant makes the advertisement quietly
    /// false. The summary is an agent declaration and the constant belongs to this crate, so the compiler does not connect the two.
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

    /// ★It is rounding down★ — changing it to rounding breaks two of the first three. An out-of-range usage is clamped to 0·100.
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

    /// Window order · model names · unknown (`null`) cross over as is — the `left_pct` of a window with no `used_pct` is `null`, not 0.
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

    /// Six states map 1:1 to six words, and a state's fields are carried only in that state. detail is unfolded into three fields.
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
