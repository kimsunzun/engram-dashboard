//! 제어 채널 토큰 레지스트리(ADR-0086) — (AgentId, epoch)별 bearer 토큰 발급·검증·폐기.
//!
//! ★역할★: 스폰 시 데몬이 (AgentId, epoch)마다 256-bit 토큰을 발급하고, 스폰된 claude 에이전트가
//!   mcp-config 의 Authorization 헤더로 그 토큰을 제시하면 여기서 검증해 신원((AgentId, epoch))을
//!   되돌린다. 화신 교체(재활성화 = 다른 표식)·kill·terminal 은 구 토큰을 폐기한다 → stale-epoch 토큰은
//!   더 이상 유효하지 않다(401). `from`(발신자 신원)은 항상 이 토큰에서 파생한다 — 페이로드가 아니라
//!   토큰이 신원의 단일 출처다(ADR-0086 §불변식 "from 은 토큰에서만 파생", 사칭 차단).
//!
//! ★보안★: 토큰 문자열은 로그에 찍지 않는다(tracing 은 AgentId/epoch 만). 토큰↔신원 역방향 조회를
//!   위해 토큰 문자열을 key 로 쓰는 맵을 두되, Debug 파생은 하지 않는다(로그 누출 방지).
//!
//! tauri import 0(daemon crate).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

use engram_dashboard_core::agent::types::AgentId;

// ADR-0110
use engram_dashboard_messaging::envelope::{DeliveryObservation, DeliveryObserver, EnvelopeFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundIdentity {
    pub agent_id: AgentId,
    pub epoch: u32,
}

/// 토큰 하나에 매인 것 전부 — 신원 + **발급 시점에 확정된** 우편 가부(ADR-0133 결정 3의 판정 재료).
///
/// ★왜 발급 시점에 박나(요청마다 조회하지 않는 이유)★: 우편 채널은 백엔드 capability 로만 갈리고
///   **런타임 스위칭·폴백이 없다**(ADR-0128 결정 1) — 그래서 발급 시점의 값이 곧 진실이다. 토큰은
///   (AgentId, epoch)마다 새로 나므로 재활성화하면 새 값으로 다시 박힌다. 우편은 오가는 양이 많은
///   경로라(그게 ADR-0128 이 MCP 를 고른 이유다) 요청마다 매니저를 조회할 이유도 없다.
/// ★신원과 권한을 한 값에 담되 **동일성 비교는 신원으로만** 한다★: 세션 pinning 이 비교하는 것은
///   `identity` 이고, 이 구조체째로 비교하지 않는다 — 권한 비트가 동일성 판정에 끼면 인가 문제가
///   세션 탈취 오탐(403)으로 둔갑한다.
// ADR-0133
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBinding {
    pub identity: BoundIdentity,
    /// **HTTP/CLI 우편 입구**(`/control/send`·`/control/messages`)를 이 자격증명으로 쓸 수 있는가.
    ///
    /// ★이름보다 좁다 — "우편을 쓸 수 있는가" 가 아니다★: `false` 인 자격증명(= MCP 가능 백엔드 스폰)은
    ///   `/mcp` 의 `send_message` 로 **우편을 정상적으로 쓴다**. 채널이 백엔드 capability 로만 갈리고
    ///   런타임 폴백이 없다는 것이 설계이며(ADR-0128 결정 1), 이 값은 그 설계에서 닫혀 있어야 할 쪽 입구를
    ///   닫는 데만 쓰인다. 이 필드를 "우편 권한" 일반으로 읽고 MCP 툴 경로에까지 검사를 얹으면 그 결정을
    ///   뒤집는 것이다.
    // ADR-0133
    // ADR-0128
    pub mail_allowed: bool,
}

/// 이 impl 이 두 신원 타입을 잇는 경계의 **유일한 지점**이다(입구가 `cmd.from.into()` 로 넘긴다).
// ADR-0110
impl From<BoundIdentity> for engram_dashboard_messaging::SenderIdentity {
    fn from(b: BoundIdentity) -> Self {
        Self {
            peer_id: b.agent_id,
            epoch: b.epoch,
        }
    }
}

/// 데몬이 1개 소유(Arc 공유). 내부 RwLock — 읽기(validate, 툴 호출마다)가 쓰기(issue/revoke,
/// 스폰·종료 때)보다 훨씬 잦다.
///
/// ★ADR-0088 배달 관측 슬롯★: `delivery_observer` 는 제어 채널 relay 의 배달-경계 관측 레코드를
///   받는 **선택적 in-process 싱크**다. 왜 여기 사나 = registry 는 이미 `handle_send` 를 포함한 모든
///   제어-플레인 경로에 `Arc` 로 스레드되는 유일한 공유 객체라, 관측 싱크를 여기 매달면 `handle_send`
///   시그니처와 그 호출부를 건드리지 않고 in-proc 하네스가 관측을 설치할 수 있다(최소 풋프린트).
///   운영 데몬은 이 슬롯을 비워 둔다(observer=None) → 기존 tracing 경로만 남고 오버헤드 0. 하네스는
///   `set_delivery_observer` 로 설치해 **detached 데몬 로그 스크레이핑 없이** 레코드를 직접 회수한다
///   (ADR-0088 HARD CONSTRAINT · ADR-0012 인프로세스 하네스 결).
#[derive(Default)]
pub struct ControlRegistry {
    inner: RwLock<Inner>,
    /// ★봉투 포맷 전역 상태(ADR-0096)★ — A→B 메시지 봉투를 colon/xml 로 전환하는 **데몬 전역 상태
    ///   하나**다(수신자별/메타데이터-유무별 아님 — ADR-0096 결정 1). 왜 여기 사나 = registry 는 이미
    ///   `handle_send`(MCP·CLI 두 입구)와 WS dispatch 에 `Arc` 로 스레드되는 유일한 공유 객체라, 이
    ///   상태를 매달면 별도 상태 소유자를 새로 배선하지 않고 두 봉투 조립 경로가 같은 값을 읽는다.
    ///   ★소유 = 데몬(ADR-0029)★: registry 는 AgentManager 를 소유한 데몬 프로세스에 산다 — src-tauri
    ///   (클라이언트 셸)는 이 상태를 소유하지 않고 invoke 커맨드를 데몬으로 전달만 한다.
    ///   ★AtomicU8(락 불요)★: 단순 스칼라 토글이라 RwLock 이 과하다 — **0=Xml(기본)·1=Colon**. `Default`
    ///   가 0 이라 초기값이 Xml 로 정합한다(ADR-0103 — S18 봉투 XML 단일화로 기본 flip). ★값 매핑을 뒤집은
    ///   이유★: `#[derive(Default)]` AtomicU8=0 을 새 기본값 Xml 에 매달아야 하므로 Xml 을 0 에, Colon 을 1 에
    ///   배정한다(별도 seed 없이 파생 Default 만으로 기본 = Xml 성립). ★메모리-only★: 데몬 재시작 시 0(Xml)
    ///   으로 리셋(영속화는 백로그 — ADR-0096 결정 4). read=relay 마다(Acquire), write=set_envelope_format
    ///   커맨드(드묾, Release) — Release/Acquire 짝으로 스위치 이후 첫 메시지가 새 포맷을 확실히 보게
    ///   한다(FIX-4). 단일 스칼라라 찢김은 없으나 cross-thread 가시성을 명시적으로 성립시킨다.
    // ADR-0103
    // ADR-0096
    envelope_format: AtomicU8,
    /// RwLock: 설치는 드물고(테스트 셋업 1회) 조회는 relay 마다지만 짧다.
    delivery_observer: RwLock<Option<Arc<dyn DeliveryObserver>>>,
    /// ★mid-send yield-seam hook(ADR-0088 Stage 1 — test-harness 전용)★: `handle_send` 가 resolve↔write
    ///   갭의 **가장 늦은 지점**(수신자 주입 직전)에서 발화하는 test hook. 결정적 mid-flight
    ///   epoch race 재현용 — hook 안에서 같은 AgentId 를 새 epoch incarnation 으로 교체 주입하면 resolve 는
    ///   구 incarnation 을 봤는데 write 는 새 incarnation 에 착지한다. ★race 는 ADR-0086 §F5 가
    ///   design-accepted 로 표시★(메일은 **논리 에이전트**를 향하므로 새 incarnation 착지가 올바른 동작이다)
    ///   — 이 seam 은 그 동작을 **결정적으로 관측**하려는 것이지 epoch 를 pin 하려는 게 아니다.
    ///   운영 빌드 = feature OFF → 이 필드 자체가 존재하지 않는다(hook 발화 코드도 컴파일 안 됨,
    ///   handle_send 동작 byte-identical). RwLock: 설치 드물고 조회 짧다(observer 와 동형).
    // ADR-0088
    #[cfg(feature = "test-harness")]
    mid_send_hook: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
}

#[derive(Default)]
struct Inner {
    token_to_binding: HashMap<String, TokenBinding>,
    agent_to_token: HashMap<AgentId, String>,
    session_to_identity: HashMap<String, BoundIdentity>,
}

impl ControlRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 그 AgentId 의 **이전 epoch 토큰은 제거**한다(회전=폐기 — 한 AgentId = 산 토큰 1개).
    /// token 은 호출자(provision)가 CSPRNG 로 만들어 넘긴다 — 이 레지스트리는 난수 생성을 하지 않고
    /// 매핑만 소유한다(생성·매핑 관심사 분리).
    ///
    /// `mail_allowed` = 이 자격증명으로 우편 요청을 받을 것인가(ADR-0133). 판정 자체는 호출자
    ///   (`DaemonControlChannel::provision`) 단독이다 — 여기선 받은 값을 기록만 한다(파생 금지: 두 곳이
    ///   판정하면 갈린다).
    /// ★lock 순서(ADR-0006)★: write lock 은 이 함수 안에서만 잡고, 외부 호출을 하지 않는다(순수 맵 조작).
    pub fn issue(&self, id: AgentId, epoch: u32, token: String, mail_allowed: bool) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        if let Some(old) = inner.agent_to_token.remove(&id) {
            inner.token_to_binding.remove(&old);
            // 옛 토큰으로 붙은 세션 바인딩도 이제 stale — 함께 지운다.
            inner
                .session_to_identity
                .retain(|_, ident| ident.agent_id != id);
        }
        inner.token_to_binding.insert(
            token.clone(),
            TokenBinding {
                identity: BoundIdentity {
                    agent_id: id,
                    epoch,
                },
                mail_allowed,
            },
        );
        inner.agent_to_token.insert(id, token);
        tracing::info!(agent = %id, epoch, mail_allowed, "제어 채널 토큰 발급(ADR-0086)");
    }

    /// 없거나 폐기됐으면 None — 호출자(auth 미들웨어)가 401.
    ///
    /// ★신원과 우편 가부를 **한 번의 조회로** 함께 낸다★: 둘을 따로 조회하면 그 사이의 폐기·재발급이
    ///   "신원은 A 인데 권한은 B" 를 만들 수 있고, 그 조합엔 정의된 답이 없다.
    pub fn validate(&self, token: &str) -> Option<TokenBinding> {
        self.inner
            .read()
            .expect("control registry poisoned")
            .token_to_binding
            .get(token)
            .copied()
    }

    /// 발신자 생존 "관측"용 (게이트 아님 — 배달은 막지 않고 기록만, 사용자 결정 2026-07-19).
    ///
    /// false = kill/rotate 로 토큰이 폐기·교체됨. relay 시점엔 원본 토큰 문자열이 남아 있지 않으므로
    ///   agent_to_token → token_to_binding 으로 되짚어 epoch 일치를 본다.
    pub fn is_identity_live(&self, identity: BoundIdentity) -> bool {
        let inner = self.inner.read().expect("control registry poisoned");
        inner
            .agent_to_token
            .get(&identity.agent_id)
            .and_then(|token| inner.token_to_binding.get(token))
            .map(|bound| bound.identity.epoch == identity.epoch)
            .unwrap_or(false)
    }

    /// 넘기는 신원은 **미들웨어가 이미 검증한 것**이어야 한다(ADR-0086). auth 미들웨어가 initialize
    /// 응답에서 Mcp-Session-Id 를 발견했을 때 부른다. 이후 그 세션으로 오는 요청의 신원 확인·
    /// acceptance 관측·revoke 정리 대상이 된다.
    ///
    /// ★no-overwrite + exact-token recheck(FIX 7 + round-2 F2)★: 세션↔신원은 **initialize 때 한 번만**
    ///   고정한다(identity pinning). 이미 바인딩이 있으면 덮어쓰지 않는다 — 그래야 세션 S 를 토큰 A 로 열고
    ///   뒤에 토큰 B 로 같은 세션에 요청을 보내는 **cross-token takeover** 를 미들웨어가 감지·거부할 수
    ///   있다(바인딩이 B 로 덮이면 탈취가 성공한 것처럼 보인다). 또 바인딩 직전 **검증에 쓴 그 토큰 문자열이
    ///   아직 이 agent 의 현재 크레덴셜인지** 재확인한다(`agent_to_token[agent] == validated_token`) —
    ///   validate→bind 사이에 revoke(토큰 evict) 또는 재발급(같은 agent 새 토큰)이 끼면 바인딩을 건너뛰고
    ///   실패로 신호한다(None 반환).
    /// ★왜 identity 재확인이 아니라 **exact token** 재확인인가(round-2 F2)★: 예전엔 `agent_to_token[agent]`
    ///   가 가리키는 산 토큰의 신원이 `identity` 와 같은지만 봤다(id·epoch 일치). 그건 "화신 표식은 재활성화마다
    ///   반드시 갈린다(ADR-0007)"는 **원거리 불변식**에 기대 같은 (id,epoch) 재발급이 불가능하다는 가정
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
        let token_current = inner
            .agent_to_token
            .get(&identity.agent_id)
            .map(|cur| cur == validated_token)
            .unwrap_or(false);
        if !token_current {
            return None;
        }
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

    pub fn identity_for_session(&self, session_id: &str) -> Option<BoundIdentity> {
        self.inner
            .read()
            .expect("control registry poisoned")
            .session_to_identity
            .get(session_id)
            .copied()
    }

    /// 클라이언트가 세션을 DELETE 로 접으면 미들웨어가 부른다(FIX 8). revoke-time 정리와 별개로,
    /// 정상 teardown 경로에서 session_to_identity 가 무한 성장하지 않게 한다(반복 initialize→DELETE 가
    /// 엔트리를 쌓지 않음). 없으면 no-op.
    pub fn unbind_session(&self, session_id: &str) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        if inner.session_to_identity.remove(session_id).is_some() {
            // 호출자가 고른 헤더 값이라 로그에 실을 때는 다듬는다 — 이 모듈의 로그 필드 규율은
            //   `connection_core::sanitize_for_log` 쪽이 정본이다(`control::agent::preview` 는 **응답
            //   문구** 전용이고 제어문자를 흘린다).
            tracing::info!(
                session = %crate::connection_core::sanitize_for_log(session_id),
                "제어 채널 세션 바인딩 해제(DELETE, ADR-0086)"
            );
        }
    }

    /// (AgentId, epoch) 토큰 폐기 + 그 신원의 세션 바인딩 제거. terminal(reaper) / kill 에서 호출.
    ///
    /// ★epoch-guard★: 요청 표식이 **현재 산 토큰의 표식과 일치할 때만** 폐기한다. stale terminal 이
    ///   재활성화(새 화신 = 다른 표식)로 새로 발급된 산 토큰을 지우지 못하게 한다(ADR-0007/0084 정신을
    ///   토큰 레지스트리까지 확장). 일치하지 않으면 no-op(그 사이 새 토큰이 이미 자리를 차지).
    ///   ★대소로 바꾸지 말 것★: 표식은 화신마다 뽑은 난수라 순서에 뜻이 없다(`AgentProfile::epoch`).
    /// ★idempotent★: 이미 없으면(이중 revoke — kill 선제 + reaper) no-op. 그래서 kill_agent 와 reaper 가
    ///   둘 다 불러도 안전하다.
    pub fn revoke(&self, id: AgentId, epoch: u32) {
        let mut inner = self.inner.write().expect("control registry poisoned");
        match inner.agent_to_token.get(&id) {
            Some(token) => {
                let cur = inner.token_to_binding.get(token).map(|b| b.identity.epoch);
                if cur != Some(epoch) {
                    return;
                }
            }
            None => return,
        }
        if let Some(token) = inner.agent_to_token.remove(&id) {
            inner.token_to_binding.remove(&token);
        }
        inner
            .session_to_identity
            .retain(|_, ident| !(ident.agent_id == id && ident.epoch == epoch));
        tracing::info!(agent = %id, epoch, "제어 채널 토큰 폐기(ADR-0086)");
    }

    /// 관측용(ADR-0086 acceptance) — 통합 테스트(별도 크레이트)도 쓰므로 cfg(test) 로 감추지 않고
    /// 공개한다(순수 조회, 안전).
    pub fn live_token_count(&self) -> usize {
        self.inner
            .read()
            .expect("control registry poisoned")
            .token_to_binding
            .len()
    }

    /// 관측용(acceptance) — 통합 테스트가 "에이전트 연결 후 데몬이 세션을 붙잡았다"를 이 값으로 단언한다.
    pub fn bound_session_count(&self) -> usize {
        self.inner
            .read()
            .expect("control registry poisoned")
            .session_to_identity
            .len()
    }

    pub fn set_delivery_observer(&self, observer: Arc<dyn DeliveryObserver>) {
        *self
            .delivery_observer
            .write()
            .expect("delivery observer poisoned") = Some(observer);
    }

    /// 배달 관측 레코드 발행(ADR-0088) — `handle_send` 가 relay 성공/실패마다 부른다.
    /// ★락 규율(ADR-0006)★: observer Arc 를 clone 해 lock 을 즉시 놓은 뒤 lock 밖에서 `observe` 를
    /// 호출한다(external call 을 lock 보유 중 하지 않는다).
    ///
    /// ★관측은 배달·ACK 를 절대 교란하지 않는다(FIX-2 — 즉시 push 불변식)★: 유저 공급 `observe` 가
    ///   panic 하면 그 panic 이 `handle_send` 를 타고 올라가 relay(바이트는 이미 write 됨) 뒤 ACK 를
    ///   못 내보내고, 발신자가 재시도해 **중복 배달**이 날 수 있다 — 관측을 켰다는 이유만으로 배달
    ///   시맨틱이 바뀌면 안 된다. 그래서 `observe` 호출을 `catch_unwind` 로 격리해 panic 을 여기서
    ///   삼키고(warn 만 남김) record_delivery 는 항상 정상 반환한다. RwLock 보유 중이 아니라 clone 후
    ///   lock 밖에서 잡으므로 poison 도 전파하지 않는다(락 규율과도 정합).
    ///
    /// ★catch_unwind 는 unwind 프로파일 의존(FIX-R2-3)★: `catch_unwind` 는 unwinding panic 만 잡는다 —
    ///   workspace `[profile.release] panic = "abort"`(루트 Cargo.toml)에선 panic 이 unwind 하지 않고
    ///   즉시 abort 하므로 이 격리는 사실상 **테스트/디버그 프로파일 한정**이다. 지금은 무해하다: 운영은
    ///   observer 를 설치하지 않아(observer=None) 아래 `if let Some(sink)` 가 catch_unwind 전에 단락된다.
    ///   운영 observer 를 언젠가 붙인다면 이 보장이 release 에서 사라진다는 점을 재검토해야 한다.
    pub fn record_delivery(&self, obs: DeliveryObservation) {
        let sink = self
            .delivery_observer
            .read()
            .expect("delivery observer poisoned")
            .clone();
        if let Some(sink) = sink {
            // AssertUnwindSafe: panic 시 obs 는 소비돼 사라지고 우리는 로그만 남긴다(공유 불변식을
            //   깨진 채로 관측하지 않는다 — sink/obs 어느 것도 catch 이후 재사용하지 않음).
            let msg_id = obs.msg_id.clone();
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || sink.observe(obs)));
            if result.is_err() {
                tracing::warn!(
                    msg_id = %msg_id,
                    "ADR-0088: DeliveryObserver.observe 가 panic — 격리 삼킴(배달/ACK 는 영향 없음). 관측 싱크 버그."
                );
            }
        }
    }

    // ── 봉투 포맷 전역 상태(ADR-0096·0103) ──────────────────────────────────────────

    /// ★알 수 없는 값 방어(fold-unknown = Xml)★: 저장은 set_envelope_format 만 하므로 항상 0/1 이나,
    ///   방어적으로 **1(Colon)만 Colon, 그 외는 Xml** 로 접는다 — 기본 안전 = 운영 정상값이 xml
    ///   이므로 fold-unknown 도 Xml 로 정합시킨다(파생 Default 0 도 이 갈래로 Xml).
    // ADR-0103
    pub fn envelope_format(&self) -> EnvelopeFormat {
        match self.envelope_format.load(Ordering::Acquire) {
            1 => EnvelopeFormat::Colon,
            _ => EnvelopeFormat::Xml,
        }
    }

    pub fn set_envelope_format(&self, format: EnvelopeFormat) {
        let v = match format {
            EnvelopeFormat::Xml => 0u8,
            EnvelopeFormat::Colon => 1u8,
        };
        self.envelope_format.store(v, Ordering::Release);
        tracing::info!(?format, "봉투 포맷 전역 상태 전환(ADR-0096)");
    }

    #[cfg(feature = "test-harness")]
    pub fn set_mid_send_hook(&self, hook: Option<Arc<dyn Fn() + Send + Sync>>) {
        // ★락 규율(ADR-0006 정신)★: 새 hook 을 꽂고 옛 hook Arc 는 **lock 밖에서** drop 한다. 락 보유 중
        //   drop 하면 옛 hook 의 소멸자(캡처한 Drop)가 registry 로 재진입할 때 deadlock, panic 하면 lock
        //   poison — foreign code(Drop 포함)를 lock 아래에서 돌리지 않는다. `_old` 는 guard scope 를 벗어난
        //   뒤(락 해제 후) 이 함수 끝에서 drop 된다.
        let _old = {
            let mut guard = self.mid_send_hook.write().expect("mid-send hook poisoned");
            std::mem::replace(&mut *guard, hook)
        };
    }

    /// ★락 규율(ADR-0006)★: hook Arc 를 short read lock 밑에서 clone 해 lock 을 즉시 놓은 뒤
    ///   **lock 밖에서** 호출한다 — foreign code(hook)를 registry lock 보유 중에 절대 부르지 않는다.
    #[cfg(feature = "test-harness")]
    pub fn fire_mid_send_hook(&self) {
        let hook = self
            .mid_send_hook
            .read()
            .expect("mid-send hook poisoned")
            .clone();
        if let Some(hook) = hook {
            hook();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 우편 가부는 이 헬퍼를 쓰는 테스트들의 관심사가 아니다 — 우편이 열린 쪽(비-MCP 스폰의 값)으로 둔다.
    ///   그 축을 보는 테스트는 `issue` 를 직접 부른다.
    fn tok(reg: &ControlRegistry, id: AgentId, epoch: u32) -> String {
        // 결정적 테스트 토큰(실제 provision 은 CSPRNG). 유일성만 유지.
        let t = format!("tok-{id}-{epoch}");
        reg.issue(id, epoch, t.clone(), true);
        t
    }

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
        let bound = reg.validate(&t).expect("valid token");
        assert_eq!(bound.identity.agent_id, id);
        assert_eq!(bound.identity.epoch, 0);
    }

    /// ★우편 가부는 자격증명에 박히고 **회전 때 다시 박힌다**(ADR-0133 결정 2)★: 재활성화가 채널을
    ///   바꾸면 새 토큰이 새 판정을 들고 나온다 — 옛 판정이 남아 있으면 백엔드를 바꿔 재스폰한 에이전트가
    ///   옛 권한으로 돈다.
    #[test]
    fn issue_records_the_mail_verdict_and_rotation_replaces_it() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        reg.issue(id, 0, "mail-off".to_string(), false);
        assert!(
            !reg.validate("mail-off").expect("valid").mail_allowed,
            "발급 시 false 면 검증도 false"
        );
        reg.issue(id, 1, "mail-on".to_string(), true);
        assert!(reg.validate("mail-on").expect("valid").mail_allowed);
        assert!(
            reg.validate("mail-off").is_none(),
            "회전으로 옛 토큰(과 그 판정)은 폐기"
        );
    }

    #[test]
    fn unknown_token_is_none() {
        let reg = ControlRegistry::new();
        assert!(reg.validate("nope").is_none());
    }

    #[test]
    fn is_identity_live_tracks_revoke_and_rotation() {
        // ★F3 회귀★ — commit-point 재검증.
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        tok(&reg, id, 0);
        let ident0 = BoundIdentity {
            agent_id: id,
            epoch: 0,
        };
        assert!(reg.is_identity_live(ident0), "발급 직후 신원은 live");

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

        reg.revoke(id, 1);
        assert!(
            !reg.is_identity_live(BoundIdentity {
                agent_id: id,
                epoch: 1
            }),
            "revoke 후 신원은 dead(F3)"
        );

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
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let old = tok(&reg, id, 0);
        let new = tok(&reg, id, 1);
        assert!(
            reg.validate(&old).is_none(),
            "회전된 구 epoch 토큰은 폐기(stale) — validate None"
        );
        let bound = reg.validate(&new).expect("새 토큰 유효");
        assert_eq!(bound.identity.epoch, 1);
        assert_eq!(reg.live_token_count(), 1, "한 AgentId = 산 토큰 1개");
    }

    #[test]
    fn revoke_matching_epoch_removes_token() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let t = tok(&reg, id, 3);
        reg.revoke(id, 3);
        assert!(reg.validate(&t).is_none(), "kill revoke 후 토큰 무효");
        assert_eq!(reg.live_token_count(), 0);
    }

    #[test]
    fn revoke_is_idempotent() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        tok(&reg, id, 0);
        reg.revoke(id, 0);
        reg.revoke(id, 0);
        assert_eq!(reg.live_token_count(), 0);
    }

    #[test]
    fn stale_revoke_does_not_kill_live_token() {
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        tok(&reg, id, 0); // 구 세션 토큰
        let live = tok(&reg, id, 1); // 재활성화 — 새 산 토큰
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
        // ★identity pinning(FIX 7)★ — cross-token takeover 방지의 레지스트리 측 기반.
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
        // ★round-2 F2 회귀★: issue 는 같은 (id,epoch) 재호출이 가능하므로 여기서 epoch 를 올리지 않아도
        //   재현된다 — 이것이 "epoch-always-bumps 원거리 불변식이 깨져도 안전" 을 증명하는 핵심이다.
        let reg = ControlRegistry::new();
        let id = AgentId::new_v4();
        let stale = tok(&reg, id, 0); // 최초 발급 — 이 토큰으로 validate 됐다고 가정.
        let ident = BoundIdentity {
            agent_id: id,
            epoch: 0,
        };
        // 같은 agent·같은 epoch 로 재발급.
        reg.issue(id, 0, "reissued-token".to_string(), true);
        assert_ne!(
            current_token(&reg, id),
            stale,
            "재발급으로 현재 토큰이 바뀜"
        );
        assert_eq!(
            reg.bind_session_if_absent("sess", ident, &stale),
            None,
            "재발급된 뒤 stale 토큰의 세션은 바인딩되지 않아야(exact-token F2)"
        );
        assert!(reg.identity_for_session("sess").is_none());
        // 대조군.
        let cur = current_token(&reg, id);
        assert_eq!(
            reg.bind_session_if_absent("sess", ident, &cur),
            Some(ident),
            "현재 크레덴셜 토큰으로는 바인딩 성공"
        );
    }

    #[test]
    fn unbind_session_prunes_binding() {
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

    // ── ADR-0096·0103: 봉투 포맷 전역 상태 ────────────────────────────────────────────
    #[test]
    fn envelope_format_defaults_to_xml_and_toggles() {
        let reg = ControlRegistry::new();
        assert_eq!(
            reg.envelope_format(),
            EnvelopeFormat::Xml,
            "새 registry 기본 봉투 포맷은 xml 이어야(ADR-0103 flip)"
        );
        reg.set_envelope_format(EnvelopeFormat::Colon);
        assert_eq!(
            reg.envelope_format(),
            EnvelopeFormat::Colon,
            "set(Colon) 후 colon 으로 읽혀야(잔존 스위치)"
        );
        reg.set_envelope_format(EnvelopeFormat::Xml);
        assert_eq!(
            reg.envelope_format(),
            EnvelopeFormat::Xml,
            "set(Xml) 후 xml 로 복귀"
        );
    }

    #[test]
    fn concurrent_issue_validate_is_safe() {
        use std::sync::Arc;
        let reg = Arc::new(ControlRegistry::new());
        let mut handles = Vec::new();
        for i in 0..8u32 {
            let reg = reg.clone();
            handles.push(std::thread::spawn(move || {
                let id = AgentId::new_v4();
                let t = format!("t-{i}");
                reg.issue(id, i, t.clone(), true);
                assert_eq!(reg.validate(&t).map(|b| b.identity.epoch), Some(i));
                reg.revoke(id, i);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(reg.live_token_count(), 0);
    }
}
