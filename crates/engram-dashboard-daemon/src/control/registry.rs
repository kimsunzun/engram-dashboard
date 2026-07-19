//! 제어 채널 토큰 레지스트리(ADR-0086) — (AgentId, epoch)별 bearer 토큰 발급·검증·폐기.
//!
//! ★역할★: 스폰 시 데몬이 (AgentId, epoch)마다 256-bit 토큰을 발급하고, 스폰된 claude 에이전트가
//!   mcp-config 의 Authorization 헤더로 그 토큰을 제시하면 여기서 검증해 신원((AgentId, epoch))을
//!   되돌린다. epoch 회전(재활성화 bump)·kill·terminal 은 구 토큰을 폐기한다 → stale-epoch 토큰은
//!   더 이상 유효하지 않다(401). `from`(발신자 신원)은 항상 이 토큰에서 파생한다 — 페이로드가 아니라
//!   토큰이 신원의 단일 출처다(ADR-0086 §불변식 "from 은 토큰에서만 파생", 사칭 차단).
//!
//! ★불변식(load-bearing)★:
//!   - 토큰 → 신원 매핑은 (AgentId, epoch) 단위다. 같은 AgentId 라도 epoch 이 다르면 다른 토큰이다.
//!   - `issue` 는 그 AgentId 의 **이전 epoch 토큰을 제거**하고 새 토큰을 넣는다(한 AgentId = 산 토큰
//!     1개). 이렇게 하면 epoch 회전이 곧 구 토큰 폐기가 된다(별도 호출 불요 — 회전 자체가 폐기).
//!   - `validate` 는 토큰 문자열 → 신원. 없거나 폐기됐으면 None(호출자가 401).
//!   - 바인딩(`bind_session`)은 handshake 성공 후 Mcp-Session-Id → 신원을 기록한다(툴 호출이 세션에서
//!     신원을 되찾게). revoke 시 그 세션 바인딩도 함께 지운다.
//!
//! ★보안★: 토큰 문자열은 로그에 찍지 않는다(tracing 은 AgentId/epoch 만). 토큰↔신원 역방향 조회를
//!   위해 토큰 문자열을 key 로 쓰는 맵을 두되, Debug 파생은 하지 않는다(로그 누출 방지).
//!
//! tauri import 0(daemon crate).

use std::collections::HashMap;
use std::sync::RwLock;

use engram_dashboard_core::agent::types::AgentId;

/// 검증 성공 시 되돌리는 신원 — 토큰이 묶인 (AgentId, epoch). `from` 파생의 단일 출처(ADR-0086).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundIdentity {
    pub agent_id: AgentId,
    pub epoch: u32,
}

/// (AgentId, epoch)별 제어 채널 토큰 레지스트리. 데몬이 1개 소유(Arc 공유). 내부 RwLock —
/// 읽기(validate, 툴 호출마다)가 쓰기(issue/revoke, 스폰·종료 때)보다 훨씬 잦다.
#[derive(Default)]
pub struct ControlRegistry {
    inner: RwLock<Inner>,
}

#[derive(Default)]
struct Inner {
    /// 토큰 문자열 → 신원. validate 의 역방향 조회(에이전트가 제시한 토큰 → 누구인가).
    token_to_identity: HashMap<String, BoundIdentity>,
    /// AgentId → 현재 산 토큰. issue 가 이전 epoch 토큰을 token_to_identity 에서 제거하는 데 쓴다
    /// (한 AgentId = 산 토큰 1개 — epoch 회전 = 구 토큰 폐기).
    agent_to_token: HashMap<AgentId, String>,
    /// Mcp-Session-Id → 신원. handshake 성공 후 바인딩(bind_session) — 툴 핸들러가 세션에서 신원 복원.
    session_to_identity: HashMap<String, BoundIdentity>,
}

impl ControlRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// (AgentId, epoch)에 새 토큰을 발급해 등록하고 토큰 문자열을 돌려준다. 그 AgentId 의 **이전
    /// epoch 토큰은 제거**한다(회전=폐기). token 은 호출자(provision)가 CSPRNG 로 만들어 넘긴다 —
    /// 이 레지스트리는 난수 생성을 하지 않고 매핑만 소유한다(생성·매핑 관심사 분리).
    ///
    /// ★lock 순서(ADR-0006)★: write lock 은 이 함수 안에서만 잡고, 외부 호출을 하지 않는다(순수 맵 조작).
    pub fn issue(&self, id: AgentId, epoch: u32, token: String) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        // 이전 epoch 토큰 제거(있으면) — 회전 시 구 토큰이 살아남지 않게.
        if let Some(old) = inner.agent_to_token.remove(&id) {
            inner.token_to_identity.remove(&old);
            // 옛 세션 바인딩도 무효(옛 토큰으로 붙은 세션은 이제 stale). 값 매칭으로 제거.
            inner
                .session_to_identity
                .retain(|_, ident| ident.agent_id != id);
        }
        inner.token_to_identity.insert(
            token.clone(),
            BoundIdentity {
                agent_id: id,
                epoch,
            },
        );
        inner.agent_to_token.insert(id, token);
        tracing::info!(agent = %id, epoch, "제어 채널 토큰 발급(ADR-0086)");
    }

    /// 토큰 문자열 → 신원. 없거나 폐기됐으면 None(호출자 = auth 미들웨어가 401). 읽기 전용(read lock).
    pub fn validate(&self, token: &str) -> Option<BoundIdentity> {
        self.inner
            .read()
            .expect("control registry poisoned")
            .token_to_identity
            .get(token)
            .copied()
    }

    /// 발신자 생존 "관측"용 (게이트 아님 — 배달은 막지 않고 기록만, 사용자 결정 2026-07-19).
    ///
    /// ★동작★: 그 AgentId 의 **현재** 산 토큰의 epoch 이 신원 epoch 과 같으면 true(생존), 아니면 false
    ///   (kill/rotate 로 토큰이 폐기·교체됨). relay 시점엔 원본 토큰 문자열이 남아 있지 않으므로
    ///   agent_to_token → token_to_identity 로 되짚어 epoch 일치를 본다(read lock 만, 순수 조회).
    pub fn is_identity_live(&self, identity: BoundIdentity) -> bool {
        let inner = self.inner.read().expect("control registry poisoned");
        inner
            .agent_to_token
            .get(&identity.agent_id)
            .and_then(|token| inner.token_to_identity.get(token))
            .map(|bound| bound.epoch == identity.epoch)
            .unwrap_or(false)
    }

    /// Mcp-Session-Id → **미들웨어가 검증한 신원**을 바인딩한다(ADR-0086). auth 미들웨어가 initialize
    /// 응답에서 Mcp-Session-Id 를 발견했을 때 부른다 — 그 세션 키에 이 신원을 매단다. 이후 그 세션으로
    /// 오는 요청의 신원 확인·acceptance 관측·revoke 정리 대상이 된다.
    ///
    /// ★no-overwrite + exact-token recheck(FIX 7 + round-2 F2)★: 세션↔신원은 **initialize 때 한 번만**
    ///   고정한다(identity pinning). 이미 바인딩이 있으면 덮어쓰지 않는다 — 그래야 세션 S 를 토큰 A 로 열고
    ///   뒤에 토큰 B 로 같은 세션에 요청을 보내는 **cross-token takeover** 를 미들웨어가 감지·거부할 수
    ///   있다(바인딩이 B 로 덮이면 탈취가 성공한 것처럼 보인다). 또 바인딩 직전 **검증에 쓴 그 토큰 문자열이
    ///   아직 이 agent 의 현재 크레덴셜인지** 재확인한다(`agent_to_token[agent] == validated_token`) —
    ///   validate→bind 사이에 revoke(토큰 evict) 또는 재발급(같은 agent 새 토큰)이 끼면 바인딩을 건너뛰고
    ///   실패로 신호한다(None 반환).
    /// ★왜 identity 재확인이 아니라 **exact token** 재확인인가(round-2 F2)★: 예전엔 `agent_to_token[agent]`
    ///   가 가리키는 산 토큰의 신원이 `identity` 와 같은지만 봤다(id·epoch 일치). 그건 "epoch 는 재활성화마다
    ///   반드시 bump 된다(ADR-0007)"는 **원거리 불변식**에 기대 같은 (id,epoch) 재발급이 불가능하다는 가정
    ///   위에서만 안전하다. 검사를 **국소적**으로 만들려 검증된 토큰 문자열 자체를 넘겨받아 그 문자열이
    ///   여전히 현재 크레덴셜인지 직접 비교한다 — 그러면 그 원거리 불변식이 깨지더라도(같은 id·epoch 로
    ///   토큰이 재발급돼도) stale 토큰으로 온 initialize 가 바인딩되지 않는다.
    ///   반환: 새로 바인딩했으면 Some(신원), 이미 있거나(중복 init) 토큰이 죽었거나 교체됐으면 None
    ///   (호출자가 그에 맞게 처리 — 중복은 무해, 죽음/교체는 unauthorized).
    pub fn bind_session_if_absent(
        &self,
        session_id: &str,
        identity: BoundIdentity,
        validated_token: &str,
    ) -> Option<BoundIdentity> {
        let mut inner = self.inner.write().expect("control registry poisoned");
        // ★exact-token recheck(F2)★: 검증에 쓴 **그 토큰 문자열**이 아직 이 agent 의 현재 크레덴셜이어야
        //   한다. validate→bind 사이에 revoke(evict) 또는 재발급(같은 agent 새 토큰)이 끼면 여기서 걸러진다.
        //   identity(id,epoch) 일치만 보던 예전 방식은 epoch-always-bumps 불변식에 의존했으나, 이 국소 비교는
        //   그 불변식이 깨져도(같은 id·epoch 재발급) stale 토큰의 바인딩을 막는다.
        let token_current = inner
            .agent_to_token
            .get(&identity.agent_id)
            .map(|cur| cur == validated_token)
            .unwrap_or(false);
        if !token_current {
            return None; // 토큰이 evict/교체됨 → 바인딩 안 함(호출자가 unauthorized 처리).
        }
        // no-overwrite: 이미 바인딩된 세션이면 그대로 둔다(identity pinning — 첫 init 신원 고정).
        if inner.session_to_identity.contains_key(session_id) {
            return None;
        }
        inner
            .session_to_identity
            .insert(session_id.to_string(), identity);
        tracing::info!(
            agent = %identity.agent_id,
            epoch = identity.epoch,
            "제어 채널 세션 바인딩(ADR-0086, pinned)"
        );
        Some(identity)
    }

    /// Mcp-Session-Id → 신원 조회(툴 핸들러·미들웨어 identity-pin 검사가 부른다). 없으면 None.
    pub fn identity_for_session(&self, session_id: &str) -> Option<BoundIdentity> {
        self.inner
            .read()
            .expect("control registry poisoned")
            .session_to_identity
            .get(session_id)
            .copied()
    }

    /// Mcp-Session-Id 바인딩 제거(FIX 8) — 클라이언트가 세션을 DELETE 로 접으면 미들웨어가 부른다.
    /// revoke-time 정리(revoke 가 값 매칭으로 지움)와 별개로, 정상 teardown 경로에서 session_to_identity
    /// 가 무한 성장하지 않게 한다(반복 initialize→DELETE 가 엔트리를 쌓지 않음). 없으면 no-op.
    pub fn unbind_session(&self, session_id: &str) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        if inner.session_to_identity.remove(session_id).is_some() {
            tracing::info!(session = %session_id, "제어 채널 세션 바인딩 해제(DELETE, ADR-0086)");
        }
    }

    /// (AgentId, epoch) 토큰 폐기 + 그 신원의 세션 바인딩 제거. terminal(reaper) / kill 에서 호출.
    ///
    /// ★epoch-guard★: 요청 epoch 이 **현재 산 토큰의 epoch 과 일치할 때만** 폐기한다. stale terminal 이
    ///   재활성화(epoch bump)로 새로 발급된 산 토큰을 지우지 못하게 한다(ADR-0007/0084 정신을 토큰
    ///   레지스트리까지 확장). 일치하지 않으면 no-op(그 사이 새 토큰이 이미 자리를 차지).
    /// ★idempotent★: 이미 없으면(이중 revoke — kill 선제 + reaper) no-op. 그래서 kill_agent 와 reaper 가
    ///   둘 다 불러도 안전하다.
    pub fn revoke(&self, id: AgentId, epoch: u32) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        match inner.agent_to_token.get(&id) {
            Some(token) => {
                let cur = inner.token_to_identity.get(token).map(|i| i.epoch);
                // epoch 불일치 = 그 사이 회전으로 새 토큰이 자리를 차지 → 지우지 않는다(산 토큰 보호).
                if cur != Some(epoch) {
                    return;
                }
            }
            None => return, // 이미 폐기됨(idempotent).
        }
        if let Some(token) = inner.agent_to_token.remove(&id) {
            inner.token_to_identity.remove(&token);
        }
        inner
            .session_to_identity
            .retain(|_, ident| !(ident.agent_id == id && ident.epoch == epoch));
        tracing::info!(agent = %id, epoch, "제어 채널 토큰 폐기(ADR-0086)");
    }

    /// 관측용(ADR-0086 acceptance — "queryable registry method used by tests") — 현재 산 토큰이 걸린
    /// AgentId 수. 통합 테스트(별도 크레이트)도 쓰므로 cfg(test) 로 감추지 않고 공개한다(순수 조회, 안전).
    pub fn live_token_count(&self) -> usize {
        self.inner
            .read()
            .expect("control registry poisoned")
            .token_to_identity
            .len()
    }

    /// 관측용(acceptance) — 바인딩된 MCP 세션 수(handshake 후 (AgentId,epoch) 세션 바인딩 존재 확인).
    /// 통합 테스트가 "에이전트 연결 후 데몬이 세션을 붙잡았다"를 이 값으로 단언한다.
    pub fn bound_session_count(&self) -> usize {
        self.inner
            .read()
            .expect("control registry poisoned")
            .session_to_identity
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(reg: &ControlRegistry, id: AgentId, epoch: u32) -> String {
        // 결정적 테스트 토큰(실제 provision 은 CSPRNG). 유일성만 유지.
        let t = format!("tok-{id}-{epoch}");
        reg.issue(id, epoch, t.clone());
        t
    }

    // 현재 산 토큰 문자열을 조회(exact-token bind 인자용 — 테스트가 registry 내부를 안 들여다보게 헬퍼로).
    fn current_token(reg: &ControlRegistry, id: AgentId) -> String {
        reg.inner
            .read()
            .unwrap()
            .agent_to_token
            .get(&id)
            .cloned()
            .expect("live token")
    }

    #[test]
    fn issue_then_validate_returns_identity() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 0);
        let ident = reg.validate(&t).expect("valid token");
        assert_eq!(ident.agent_id, id);
        assert_eq!(ident.epoch, 0);
    }

    #[test]
    fn unknown_token_is_none() {
        let reg = ControlRegistry::new();
        assert!(reg.validate("nope").is_none());
    }

    #[test]
    fn is_identity_live_tracks_revoke_and_rotation() {
        // ★F3 회귀★: commit-point 재검증용. 산 토큰이 있으면 live, revoke/회전 후엔 그 신원은 dead.
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        tok(&reg, id, 0);
        let ident0 = BoundIdentity {
            agent_id: id,
            epoch: 0,
        };
        assert!(reg.is_identity_live(ident0), "발급 직후 신원은 live");

        // 회전(epoch bump) → 옛 신원(epoch 0)은 dead, 새 신원(epoch 1)은 live.
        tok(&reg, id, 1);
        assert!(
            !reg.is_identity_live(ident0),
            "회전 후 옛 epoch 신원은 dead(F3)"
        );
        assert!(
            reg.is_identity_live(BoundIdentity {
                agent_id: id,
                epoch: 1
            }),
            "새 epoch 신원은 live"
        );

        // revoke → 완전히 dead.
        reg.revoke(id, 1);
        assert!(
            !reg.is_identity_live(BoundIdentity {
                agent_id: id,
                epoch: 1
            }),
            "revoke 후 신원은 dead(F3)"
        );

        // 아예 발급된 적 없는 신원도 dead.
        assert!(
            !reg.is_identity_live(BoundIdentity {
                agent_id: AgentId::new_v4(),
                epoch: 0
            }),
            "미발급 신원은 dead"
        );
    }

    #[test]
    fn epoch_rotation_revokes_old_token() {
        // 재활성화(epoch bump) 시 새 issue 가 이전 epoch 토큰을 폐기해야 한다(stale 401).
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let old = tok(&reg, id, 0);
        let new = tok(&reg, id, 1);
        assert!(
            reg.validate(&old).is_none(),
            "회전된 구 epoch 토큰은 폐기(stale) — validate None"
        );
        let ident = reg.validate(&new).expect("새 토큰 유효");
        assert_eq!(ident.epoch, 1);
        assert_eq!(reg.live_token_count(), 1, "한 AgentId = 산 토큰 1개");
    }

    #[test]
    fn revoke_matching_epoch_removes_token() {
        // kill/terminal revoke(epoch 일치) → 토큰 폐기.
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 3);
        reg.revoke(id, 3);
        assert!(reg.validate(&t).is_none(), "kill revoke 후 토큰 무효");
        assert_eq!(reg.live_token_count(), 0);
    }

    #[test]
    fn revoke_is_idempotent() {
        // kill 선제 revoke + reaper revoke 이중 호출 안전(remove-if-present).
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        tok(&reg, id, 0);
        reg.revoke(id, 0);
        reg.revoke(id, 0); // 두 번째는 no-op.
        assert_eq!(reg.live_token_count(), 0);
    }

    #[test]
    fn stale_revoke_does_not_kill_live_token() {
        // ★epoch-guard 회귀★: epoch 0 세션이 죽은 뒤 재활성화로 epoch 1 이 붙었는데, 지연된 epoch 0
        //   terminal 이 revoke(id,0)을 부르면 산 epoch 1 토큰을 지우면 안 된다.
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        tok(&reg, id, 0); // 구 세션 토큰
        let live = tok(&reg, id, 1); // 재활성화 — 새 산 토큰(구 토큰은 issue 가 이미 폐기)
        reg.revoke(id, 0); // 지연된 stale terminal
        assert!(
            reg.validate(&live).is_some(),
            "stale epoch 0 revoke 가 산 epoch 1 토큰을 지우면 안 됨(epoch-guard)"
        );
    }

    #[test]
    fn bind_and_lookup_session_identity() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 0);
        let ident = BoundIdentity {
            agent_id: id,
            epoch: 0,
        };
        assert_eq!(
            reg.bind_session_if_absent("sess-abc", ident, &t),
            Some(ident),
            "첫 바인딩은 성공"
        );
        assert_eq!(reg.identity_for_session("sess-abc"), Some(ident));
        assert!(reg.identity_for_session("other").is_none());
    }

    #[test]
    fn bind_if_absent_no_overwrite_pins_identity() {
        // ★identity pinning(FIX 7)★: 세션이 이미 신원 A 로 바인딩되면, 다른 신원 B 로 재바인딩 시도는
        //   무시된다(None 반환, 기존 A 유지) — cross-token takeover 방지의 레지스트리 측 기반.
        let reg = ControlRegistry::new();
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        let ta = tok(&reg, a, 0);
        let tb = tok(&reg, b, 0);
        let ident_a = BoundIdentity {
            agent_id: a,
            epoch: 0,
        };
        let ident_b = BoundIdentity {
            agent_id: b,
            epoch: 0,
        };
        assert_eq!(
            reg.bind_session_if_absent("sess", ident_a, &ta),
            Some(ident_a)
        );
        assert_eq!(
            reg.bind_session_if_absent("sess", ident_b, &tb),
            None,
            "이미 바인딩된 세션은 덮어쓰지 않는다(pinning)"
        );
        assert_eq!(
            reg.identity_for_session("sess"),
            Some(ident_a),
            "기존 신원 A 가 유지돼야"
        );
    }

    #[test]
    fn bind_if_absent_rejects_revoked_token() {
        // ★exact-token recheck(FIX 7 + F2)★: validate→bind 사이 revoke 가 끼어 토큰이 죽었으면 바인딩 안 함.
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 0);
        let ident = BoundIdentity {
            agent_id: id,
            epoch: 0,
        };
        reg.revoke(id, 0); // 바인딩 직전 폐기됨
        assert_eq!(
            reg.bind_session_if_absent("sess", ident, &t),
            None,
            "죽은 토큰의 세션은 바인딩되지 않아야(exact-token recheck)"
        );
        assert!(reg.identity_for_session("sess").is_none());
    }

    #[test]
    fn bind_if_absent_rejects_stale_token_after_same_agent_reissue() {
        // ★round-2 F2 — exact-token(국소) 검사 회귀★: 같은 agent 로 토큰을 재발급하면(구 토큰 evict → 새
        //   토큰), 구 토큰으로 검증됐던 뒤늦은 initialize 는 바인딩되면 안 된다. epoch-always-bumps 불변식에
        //   기대지 않고, bind 가 넘겨받은 **정확한 토큰 문자열**이 현재 크레덴셜과 다름을 국소 비교로 잡는다.
        //   (issue 는 같은 (id,epoch) 재호출이 가능하므로 여기서 epoch 를 올리지 않아도 재현된다 — 이것이
        //    "원거리 불변식이 깨져도 안전" 을 증명하는 핵심.)
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let stale = tok(&reg, id, 0); // 최초 발급 — 이 토큰으로 validate 됐다고 가정.
        let ident = BoundIdentity {
            agent_id: id,
            epoch: 0,
        };
        // 같은 agent(같은 epoch)로 재발급 — 구 토큰 evict, 새 토큰이 현재 크레덴셜.
        reg.issue(id, 0, "reissued-token".to_string());
        assert_ne!(
            current_token(&reg, id),
            stale,
            "재발급으로 현재 토큰이 바뀜"
        );
        // 구(stale) 토큰으로 온 initialize 의 bind 시도 → 현재 크레덴셜과 불일치 → 바인딩 거부.
        assert_eq!(
            reg.bind_session_if_absent("sess", ident, &stale),
            None,
            "재발급된 뒤 stale 토큰의 세션은 바인딩되지 않아야(exact-token F2)"
        );
        assert!(reg.identity_for_session("sess").is_none());
        // 대조군: 현재 토큰으로는 정상 바인딩.
        let cur = current_token(&reg, id);
        assert_eq!(
            reg.bind_session_if_absent("sess", ident, &cur),
            Some(ident),
            "현재 크레덴셜 토큰으로는 바인딩 성공"
        );
    }

    #[test]
    fn unbind_session_prunes_binding() {
        // ★FIX 8★: DELETE teardown 이 세션 바인딩을 제거한다(무한 성장 방지).
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 0);
        reg.bind_session_if_absent(
            "sess-del",
            BoundIdentity {
                agent_id: id,
                epoch: 0,
            },
            &t,
        );
        assert_eq!(reg.bound_session_count(), 1);
        reg.unbind_session("sess-del");
        assert_eq!(reg.bound_session_count(), 0, "DELETE 후 바인딩 제거");
        reg.unbind_session("sess-del"); // 없으면 no-op(이중 안전)
    }

    #[test]
    fn revoke_clears_session_binding() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 0);
        reg.bind_session_if_absent(
            "sess-x",
            BoundIdentity {
                agent_id: id,
                epoch: 0,
            },
            &t,
        );
        reg.revoke(id, 0);
        assert!(
            reg.identity_for_session("sess-x").is_none(),
            "revoke 는 세션 바인딩도 지운다"
        );
        assert_eq!(reg.bound_session_count(), 0);
    }

    #[test]
    fn concurrent_issue_validate_is_safe() {
        // 동시 접근 안전(RwLock) — 여러 스레드가 issue/validate 를 섞어도 panic·데이터 손상 없음.
        use std::sync::Arc;
        let reg = Arc::new(ControlRegistry::new());
        let mut handles = Vec::new();
        for i in 0..8u32 {
            let reg = reg.clone();
            handles.push(std::thread::spawn(move || {
                let id = AgentId::new_v4();
                let t = format!("t-{i}");
                reg.issue(id, i, t.clone());
                assert_eq!(reg.validate(&t).map(|x| x.epoch), Some(i));
                reg.revoke(id, i);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(reg.live_token_count(), 0);
    }
}
