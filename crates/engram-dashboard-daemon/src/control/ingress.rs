//! ControlIngress seam(ADR-0086 스텝 2) — 듀얼 입구(MCP + CLI)의 공통 파이프라인.
//!
//! ★역할★: 두 입구(MCP `send_message` 툴 · `/control/send` HTTP 라우트)가 각자 요청을 정규화한
//!   `ControlCommand` 로 만들어 **이 모듈의 단일 핸들러**(`handle_send`)를 부른다. 그 아래(Validator·
//!   Relay·ACK)는 어느 입구로 들어왔는지 모른다(entrance-agnostic) — 입구별 코드 중복·표류를 막는다.
//!
//! ★불변식(ADR-0086)★:
//!   - `from`(발신자 신원)은 **토큰/세션에서만 파생**된다 — 페이로드가 아니라. 두 어댑터 모두
//!     BoundIdentity(auth 미들웨어/세션 바인딩이 검증한 신원)를 넣어 ControlCommand 를 만든다(사칭 차단).
//!   - ACK/에러 JSON 은 **두 입구에서 동일 shape** 다(같은 코드가 만든다) — 자기교정 로스터(RECIPIENT_*
//!     hint)도 동일.
//!   - "enqueued" 워딩은 미래 장부(ledger)와의 forward-compat 로 유지한다 — 이 최소 버전은 즉시 배달
//!     (relay)이지만 ACK 문구는 바꾸지 않는다.
//!
//! ★범위(스텝 2-min)★: 봉투 설계·idle 게이트·장부(SQLite)·그룹(@)·스레드 필드 전부 **범위 밖**이다.
//!   메시지 래퍼는 의도적으로 최소 placeholder 이고 한 함수(`wrap_message`)에만 있다(후속 스파이크가 교체).
//!
//! tauri import 0(daemon crate).

use std::sync::Arc;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::types::{AgentId, AgentStatus};

use super::registry::{BoundIdentity, ControlRegistry};

/// ★배달-경계 관측 레코드(ADR-0088 Stage 0)★ — 제어 채널 relay 1건의 write 경계에서 남기는
///   **기계 소비용** 증거다. 배달 정확성 하네스가 이걸로 "전송 실패(바이트가 안 꽂힘)" vs "모델이
///   받고도 무시" 를 가른다 — 그 판정의 전제 계측이다.
///
/// ★왜 in-proc 레코드인가(로그 아님)★: 운영 데몬은 detached 로 돌아 로그 스크레이핑이 do-not 다
///   (ADR-0088 HARD CONSTRAINT). 그래서 이 레코드를 `ControlRegistry` 에 설치한 in-proc 싱크
///   (`DeliveryObserver`)로 흘려 통합 하네스(ADR-0012)가 직접 회수하게 한다. 같은 정보를 사람 눈용
///   tracing 으로도 남기지만(운영 forensic), 하네스는 tracing 이 아니라 이 레코드를 단언한다.
///
/// ★필드 상관(핵심)★: `msg_id`(ingress 논리 메시지 uuid — 봉투 텍스트 `id:<msg_id>`) 와
///   `msg_uuid`(session.write_input 이 만든 replay-dedup 키)를 **한 레코드에** 담아 상관시킨다.
///   하네스는 "데몬이 논리 메시지 msg_id 를 write 했다" → "claude 가 user-turn msg_uuid 를 replay 했다
///   (= 실제로 파싱함)" 를 이 쌍으로 잇는다. 실패(write 에러) 시 msg_uuid 는 없다(None).
///
/// ★보안★: body 텍스트·토큰은 절대 담지 않는다(tracing 규율과 동일 — 바이트 수만).
#[derive(Debug, Clone)]
pub struct DeliveryObservation {
    /// ingress 논리 메시지 id(봉투에 `id:<msg_id>` 로 심긴 uuid). 하네스 상관의 한 축.
    pub msg_id: String,
    /// 해석된 수신자 AgentId.
    pub to_id: AgentId,
    /// 해석된 수신자 표시 이름(profile name).
    pub to_name: String,
    /// 발신자 신원(토큰 파생 — 페이로드 아님, ADR-0086).
    pub from: BoundIdentity,
    /// 어느 입구로 들어왔나(mcp/cli) — 라벨 전용.
    pub entrance: Entrance,
    /// 넘긴 논리 메시지(`wrap_message` 로 만든 봉투 문자열)의 바이트 수 = write 요청 바이트(char 수 아님).
    /// core `WriteOutcome.bytes_requested` 와 같은 "논리 메시지 바이트" 의미다(그 계층의 논리 메시지 =
    /// 이 봉투 문자열). encoder 가 감싸는 실제 wire 바이트가 아니다.
    pub bytes_requested: usize,
    /// ★완결성 판정 레버 아님(중요)★: 배달 성공/실패는 이 값이 아니라 `error`(= 세션 write 의 Ok/Err)로
    /// 본다. write 성공 시 `Some(bytes_requested)` — core `WriteOutcome.bytes_written` 을 그대로 실은
    /// by-construction 복사값이라 `bytes_requested` 와 항상 같다(short-write 탐지 아님, 비교하면 항상 동일).
    /// write 실패 시 `None`(요청 바이트가 수용됐다는 증거 없음). `is_delivered()` 참조.
    pub bytes_written: Option<usize>,
    /// 이 유저 턴의 session-level replay-dedup 키(write 성공 시 Some). msg_id 와 상관되는 다른 한 축.
    pub msg_uuid: Option<uuid::Uuid>,
    /// ★write 가 실제로 착지한 수신자 incarnation 의 epoch(ADR-0088 Stage 1, write 성공 시 Some)★.
    /// core `WriteOutcome.epoch` 를 그대로 실은 값 = write 를 **집행한** 세션의 epoch(resolve 시점
    /// 스냅샷 epoch 이 아니다 — 그 비대칭이 핵심, 아래 성공 갈래 주석 참조). 이 필드가 오라클 5 가 남긴
    /// **관측 한계**("DeliveryObservation 이 수신자 epoch 을 안 담아 어느 incarnation 이 받았는지 레코드
    /// 만으로 단정 못 한다")를 닫는다 — mid-flight epoch race(resolve↔write 사이 재시작)에서 메시지가
    /// 새 incarnation 에 착지했음을 레코드만으로(record-self-sufficient) 직접 단언할 수 있게 한다.
    /// write 실패 시 None(꽂힌 데 없으니 착지 epoch 도 없음 — msg_uuid/bytes_written 실패 시맨틱과 정합).
    /// ★완결성 판정 레버 아님★: `is_delivered()` 는 이 값을 보지 않는다(배달 유효성 게이트가 아니라 관측 축).
    pub to_epoch: Option<u32>,
    /// write 결과 — 성공이면 None, 실패면 에러 문자열(PtyError Display). 실패를 성공으로 삼키지 않음의 증거.
    /// ★배달 완결성의 1차 증거는 이 필드다(바이트 비교 아님)★ — `None` = 세션 write_all 이 Ok.
    pub error: Option<String>,
}

impl DeliveryObservation {
    /// write 가 성공(전량 수용)했나 — 하네스가 "전송 실패" vs "모델 무시" 를 가르는 1차 스위치.
    /// ★완결성의 근거는 `error.is_none()`(= 세션 write_all 이 Ok)★. 뒤의 바이트 등식은 short-write 를
    ///   잡는 게 아니라(비교하면 항상 같다 — WriteOutcome by-construction) 성공 레코드가 잘 채워졌는지의
    ///   by-construction 정합성 방어일 뿐이다(성공인데 bytes_written=None 같은 구성 버그를 거른다).
    pub fn is_delivered(&self) -> bool {
        self.error.is_none() && self.bytes_written == Some(self.bytes_requested)
    }
}

/// 배달-경계 관측 싱크(ADR-0088) — `OutputSink`/`StatusSink` 스타일의 in-proc 콜백. 통합 하네스가
///   `ControlRegistry::set_delivery_observer` 로 설치하고, `handle_send` 가 relay 마다 `observe` 를
///   호출한다. 운영 데몬은 설치하지 않아 no-op(오버헤드 0). Send+Sync — Arc 로 공유·다른 스레드 회수.
pub trait DeliveryObserver: Send + Sync {
    /// relay 1건의 배달 관측 레코드를 소비한다. 구현은 짧게(하네스는 보통 Vec 에 push) — relay 스레드가
    ///   호출하므로 블로킹 I/O 를 하지 않는다.
    fn observe(&self, obs: DeliveryObservation);
}

/// body 상한(64 KiB). 최소 버전의 방어적 상한 — 초과 시 BODY_TOO_LARGE 로 교정 에러(같은 shape).
/// (MCP 라우트의 전송 계층 상한(RequestBodyLimitLayer 1MB)과 별개 — 여기선 body **문자열** 자체의 상한.)
const MAX_BODY_BYTES: usize = 64 * 1024;

/// 어느 입구로 들어온 요청인가(ADR-0086 F6 — relay 계측 로그 필드). MCP 툴 · CLI(HTTP) 라우트 구분.
/// 파이프라인 로직은 이걸 분기하지 않는다(entrance-agnostic) — **로그 라벨 전용**이다.
#[derive(Debug, Clone, Copy)]
pub enum Entrance {
    /// MCP `send_message` 툴 경로.
    Mcp,
    /// `/control/send` 평문 HTTP 라우트(CLI `engram-send`).
    Cli,
}

impl Entrance {
    /// 구조화 로그 필드에 실을 짧은 라벨(필터 키).
    fn as_str(self) -> &'static str {
        match self {
            Entrance::Mcp => "mcp",
            Entrance::Cli => "cli",
        }
    }
}

/// 정규화된 제어 커맨드(ADR-0086) — 두 입구가 이 형태로 만들어 `handle_send` 에 넘긴다.
///
/// ★from = 토큰/세션 파생 신원★: 페이로드가 아니라 어댑터가 검증된 BoundIdentity 를 넣는다(사칭 차단).
/// 이 최소 버전은 커맨드 종류가 send 하나뿐이라 별도 cmd 태그 없이 send 전용 필드만 담는다(spawn/창이동
/// 등은 후속 additive — ADR-0086 §커맨드=의도별 전용 툴).
#[derive(Debug, Clone)]
pub struct ControlCommand {
    /// 발신자 신원 — 토큰/세션에서 파생(페이로드 아님). 사칭 차단의 단일 출처.
    pub from: BoundIdentity,
    /// 수신자 지목 — 에이전트 이름(profile name) 또는 정확한 AgentId 문자열. 미래 그룹(@) 예약.
    pub to: String,
    /// 메시지 본문(텍스트). 최소 버전은 순수 텍스트(첨부·구조화는 범위 밖).
    pub body: String,
}

/// 제어 커맨드 처리 결과 — 성공(enqueued ACK) 또는 교정 에러. 두 입구 모두 이 값을 그대로 JSON 직렬화해
/// 열린 요청에 돌려준다(동일 shape 보장). `to_json` 이 wire JSON 을 만든다.
#[derive(Debug, Clone)]
pub enum ControlResult {
    /// 배달 성공(장부 forward-compat 워딩 "enqueued") — id·해석된 수신자 이름 동봉.
    Enqueued { id: String, to: String },
    /// 교정 에러 — code + hint(자기교정용). 발신자가 이걸 보고 재시도한다.
    Error { code: &'static str, hint: String },
}

impl ControlResult {
    /// wire JSON(serde_json::Value). 두 입구가 이 값을 직렬화해 응답 body/툴 결과로 쓴다.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ControlResult::Enqueued { id, to } => serde_json::json!({
                "status": "enqueued",
                "id": id,
                "to": to,
            }),
            ControlResult::Error { code, hint } => serde_json::json!({
                "status": "error",
                "code": code,
                "hint": hint,
            }),
        }
    }

    /// 성공(enqueued)인가 — CLI 가 exit code(0/1) 매핑에 쓴다.
    pub fn is_enqueued(&self) -> bool {
        matches!(self, ControlResult::Enqueued { .. })
    }
}

/// 수신자 해석 결과(Validator 내부). 성공 시 산 세션의 (id, 표시이름), 실패 시 교정 에러.
enum Resolution {
    Ok { id: AgentId, name: String },
    Err(ControlResult),
}

/// ★듀얼 입구 공통 핸들러(ADR-0086)★: 정규화된 ControlCommand → Validator → Relay → ACK. 두 어댑터
/// (MCP 툴 · HTTP 라우트)가 유일하게 부르는 진입점이다 — 이 아래는 입구를 모른다(entrance-agnostic).
///
/// 검사 순서(첫 실패에서 교정 에러 반환 — 같은 shape 양 입구):
///   1. 그룹 주소(`@`) → GROUPS_NOT_SUPPORTED(미래 브로드캐스트 예약 슬롯).
///   2. body 상한(64 KiB) → BODY_TOO_LARGE.
///   3. 수신자 해석(AgentId 우선/이름 정확 매치, 산 에이전트) → RECIPIENT_NOT_FOUND / RECIPIENT_AMBIGUOUS.
///   4. 도달성(StreamJson + 제어 채널) → RECIPIENT_NOT_REACHABLE.
///   5. ★발신자 생존 관측(기록용만 — 게이트 아님)★ — relay 직전에 발신자가 아직 산 신원인지 registry 로
///      조회하되, 죽었어도 **거부하지 않는다**(작성 시점 인증으로 유효성 성립 — 사용자 결정 2026-07-19,
///      6번 relay 주석 참조). 죽은 발신자 배달은 forensic 로그만 남긴다.
/// 통과하면 relay(B stdin 주입) 후 Enqueued ACK.
///
/// ★self-send 허용★: to == 발신자여도 특수 처리 없이 정상 배달(테스트·자가 메시지 유용 — ADR-0086 §7).
/// ★락 규율(ADR-0006)★: manager 의 공개 API(list_agents/write_stdin)만 부른다 — 각 호출이 내부에서
///   sessions RwLock 을 Arc clone 후 즉시 해제하는 규율을 그대로 탄다. registry 조회(is_identity_live)도
///   read lock 을 잡았다 즉시 해제하는 순수 조회라, 그 lock 을 든 채 manager 를 부르지 않는다(값 반환 후 호출).
// ADR-0086
pub fn handle_send(
    manager: &Arc<AgentManager>,
    registry: &Arc<ControlRegistry>,
    entrance: Entrance,
    cmd: ControlCommand,
) -> ControlResult {
    // 1. 그룹 주소(@) — 미래 브로드캐스트 예약. 지금은 명시 교정.
    if cmd.to.starts_with('@') {
        return ControlResult::Error {
            code: "GROUPS_NOT_SUPPORTED",
            hint: "Group addresses are not available yet; send to a single agent name.".to_string(),
        };
    }

    // 2. body 상한.
    if cmd.body.len() > MAX_BODY_BYTES {
        return ControlResult::Error {
            code: "BODY_TOO_LARGE",
            hint: format!(
                "Message body exceeds the {MAX_BODY_BYTES}-byte limit; shorten it and retry."
            ),
        };
    }

    // 3+4. 수신자 해석 + 도달성. list_agents 스냅샷 1회로 판정(락 미보유 상태).
    // ★epoch 경쟁을 의도적으로 수용(F5 — ADR-0086/0007)★: 여기서 해석한 수신자가 epoch N 인데 아래
    //   write_stdin 이 도는 사이 재시작으로 epoch N+1 이 되면 메시지는 **새 incarnation** 에 꽂힌다.
    //   이건 버그가 아니라 설계 의도다 — 메일은 **논리 에이전트**(이름/AgentId 는 epoch 교체에도 유지되는
    //   안정 주소)를 향하고, 같은 이름의 재시작된 에이전트도 여전히 그 메일의 정당한 수신자다. 그래서
    //   epoch pinning 을 하지 않는다(다음 세션이 "고쳐" epoch 를 고정하면 재시작 중 유실이 생긴다).
    //   재시작 중이라 write_stdin 이 실패하면 아래 Err 갈래(RECIPIENT_NOT_REACHABLE)가 이미 덮는다.
    let agents = manager.list_agents();
    let resolved = match resolve_recipient(&cmd.to, &agents) {
        Resolution::Ok { id, name } => (id, name),
        Resolution::Err(e) => return e,
    };
    let (to_id, to_name) = resolved;

    // 4. 도달성 — StreamJson(structured 출력) 캐리어라야 stdin 주입이 유효한 user-message 라인이 된다.
    //    ADR-0086: 제어 채널은 TUI(터미널) 를 제외한다(파싱 안 되니 자연 제외). structured=true =
    //    StdioTransport(json 모드 claude) 이고, 그게 곧 제어 채널 소비 backend 다(claude).
    let reachable = agents
        .iter()
        .find(|a| a.id == to_id)
        .map(|a| a.capabilities.output.structured)
        .unwrap_or(false);
    if !reachable {
        return ControlResult::Error {
            code: "RECIPIENT_NOT_REACHABLE",
            hint: format!(
                "Agent '{to_name}' cannot receive messages; only stream-json claude agents (not TUI) are reachable via the control channel."
            ),
        };
    }

    // 5. ★발신자 생존 관측(기록용만 — 게이트 아님, 사용자 결정 2026-07-19)★
    //    메시지의 유효성은 **작성 시점 인증**(입구 auth = bearer_auth 가 발신자 토큰을 검증한 순간)으로
    //    이미 성립한다. 그 뒤 발신자가 죽거나 재시작해도(토큰 revoke/회전) 그 메시지가 무효가 되지는
    //    않는다 — "최종 결과를 보내고 종료" 는 멀티에이전트의 핵심 패턴이고(유언이 가장 중요한 메시지인
    //    경우가 많다; cf. Orca worker_done), 미래 메일박스 시맨틱도 장부 append(=커밋) 후 발신자 사망이
    //    메시지를 되돌리지 않는 방식으로 이미 이렇게 동작한다. 그래서 **발신자 생존을 배달 게이트로 쓰지
    //    않는다** — 여기서 거부하지 않고 그대로 relay 한다.
    //    다만 is_identity_live 조회는 **관측용으로 남긴다**: 발신자가 더 이상 산 신원이 아닌 채 배달되는
    //    경우를 로그로 기록한다(포렌식 / 미래 제품화 고려용 — 사용자 결정). ★레벨=warn★: 배달 자체는
    //    정상 경로지만 "죽은 발신자의 메시지 배달" 이라는 **비정상이나 안전하게 진행되는** 엣지라
    //    logging-conventions §레벨 의 warn 정의(비정상이나 안전하게 폴백/진행)에 해당한다(info=평범한 정상
    //    수명 이벤트보다 눈에 띄어야 하는 관측점). ★필드★: from·from_epoch·msg_id·entrance 만 — body
    //    텍스트·토큰은 절대 로깅 금지(보안). (msg_id 는 아래 relay 에서 만들어 함께 싣는다.)

    // 6. Relay — B stdin 에 래핑된 user-message 를 즉시 주입. msg-id 는 uuid(추적·ACK 동봉).
    let msg_id = AgentId::new_v4().to_string();

    // ★발신자 생존 관측(위 5번 — 기록용만, 배달은 그대로 진행)★: 발신자 신원이 이미 산 토큰을 잃었으면
    //   (relay 직전 revoke/회전) 배달은 막지 않되 forensic 로그를 남긴다. body/토큰은 싣지 않는다(보안).
    if !registry.is_identity_live(cmd.from) {
        tracing::warn!(
            from = %cmd.from.agent_id,
            from_epoch = cmd.from.epoch,
            msg_id = %msg_id,
            entrance = entrance.as_str(),
            "제어 채널 메시지 배달 — 발신자가 relay 시점에 더 이상 산 신원 아님(작성 시점 인증으로 유효, 게이트 아님·기록용 관측, ADR-0086·사용자 결정 2026-07-19)"
        );
    }

    let sender_name = sender_display_name(manager, cmd.from);
    let wrapped = wrap_message(&sender_name, &msg_id, &cmd.body);
    // body_bytes(순수 본문 바이트)는 기존 tracing 유지용. ★관측 레코드의 요청 바이트는 여기서 재계산하지
    //   않는다(FIX-1c)★ — 성공 경로에선 `outcome.bytes_requested`(= 세션 경계가 실제 받은 논리 메시지
    //   바이트, 단일 출처)를 쓴다. 실패 경로는 outcome 이 없으므로 `wrapped.len()` 로 대체한다(같은 값 —
    //   여기서 넘기는 바이트가 곧 세션이 받았을 논리 메시지라 정의상 일치, 아래 실패 갈래 주석 참조).
    let body_bytes = cmd.body.len();

    // ★mid-send yield-seam(ADR-0088 Stage 1 — test-harness 전용, 운영 빌드엔 컴파일 안 됨)★:
    //   resolve/reachability/wrap 를 다 끝낸 **resolve↔write 갭의 가장 늦은 지점**에서 test hook 을 발화한다.
    //   결정적 mid-flight epoch race 재현용 — hook 안에서 같은 AgentId 를 새 epoch incarnation 으로 교체
    //   주입하면 위 resolve 는 구 incarnation(epoch N)을 봤는데 아래 write 는 교체된 새 incarnation
    //   (epoch N+1)에 착지한다. ★이 race 는 ADR-0086 §F5 가 design-accepted 로 표시★: 메일은 **논리
    //   에이전트**(이름/AgentId = epoch 교체에도 유지되는 안정 주소)를 향하므로 새 incarnation 착지가 곧
    //   올바른 동작이다. 이 seam 은 그 동작을 **결정적으로 관측**할 뿐 epoch 를 pin 하지 않는다(feature OFF
    //   면 handle_send 동작은 오늘과 byte-identical — hook 발화 코드가 아예 사라진다). fire_mid_send_hook
    //   은 hook Arc 를 lock 밖에서 호출한다(ADR-0006 — record_delivery 와 동일 규율).
    // ADR-0088
    #[cfg(feature = "test-harness")]
    registry.fire_mid_send_hook();

    // ADR-0088: write 경계 계측판(write_stdin_observed)으로 논리 메시지 바이트 + 이 턴 msg_uuid 를 회수한다
    //   (완결성은 Ok/Err — 바이트 비교 아님, WriteOutcome 주석).
    //   write_stdin 은 json 모드 세션에선 encoder 가 텍스트를 claude user-message 라인으로 감싼다(ADR-0044)
    //   — 우리는 완성된 텍스트(래퍼)를 통째로 넘긴다(1 write = 완결된 유저 턴 1개 계약).
    match manager.write_stdin_observed(to_id, wrapped.as_bytes()) {
        Ok(outcome) => {
            // ★F6 계측(logging-conventions info = 정상 수명 이벤트)★: enqueue/relay 성공. from·to·msg-id·
            //   entrance·바이트수·msg_uuid 만 구조화 필드로 남긴다 — ★body 텍스트·토큰은 절대 로깅 금지★(보안).
            //   ADR-0088: bytes_written·msg_uuid 를 추가로 실어 사람 forensic 에서도 상관 가능하게.
            tracing::info!(
                from = %cmd.from.agent_id,
                to = %to_id,
                to_name = %to_name,
                msg_id = %msg_id,
                entrance = entrance.as_str(),
                body_bytes,
                bytes_requested = outcome.bytes_requested,
                bytes_written = outcome.bytes_written,
                msg_uuid = %outcome.msg_uuid,
                "제어 채널 메시지 relay(enqueued, ADR-0086·0088)"
            );
            // ADR-0088: 기계 소비용 배달 관측 레코드를 in-proc 싱크로 발행(설치 안 됐으면 no-op).
            // ADR-0006: registry.record_delivery 가 observer Arc 를 clone 후 lock 밖에서 observe 호출.
            // ★요청/실제 바이트는 재계산 없이 outcome 을 단일 출처로 쓴다(FIX-1c) — 세션 경계가 실제 받은
            //   논리 메시지 바이트. bytes_written 은 outcome 의 by-construction 복사(WriteOutcome 주석).
            // ADR-0088
            // ★to_epoch = write 가 실제 착지한 incarnation 의 epoch(outcome.epoch)이지, resolve 시점
            //   스냅샷의 epoch 이 아니다★(ADR-0088). 이 비대칭이 record-self-sufficiency 의 핵심 —
            //   resolve↔write 사이 재시작(mid-flight epoch race)으로 착지 incarnation 이 바뀌면 레코드가
            //   **실제 받은** 쪽을 담아야 한다. resolve-time epoch 을 실으면 그 race 를 레코드만으로
            //   단정할 수 없어 관측 한계가 다시 생긴다(오라클 5 가 지적한 그 한계).
            // ADR-0088
            registry.record_delivery(DeliveryObservation {
                msg_id: msg_id.clone(),
                to_id,
                to_name: to_name.clone(),
                from: cmd.from,
                entrance,
                bytes_requested: outcome.bytes_requested,
                bytes_written: Some(outcome.bytes_written),
                msg_uuid: Some(outcome.msg_uuid),
                to_epoch: Some(outcome.epoch),
                error: None,
            });
            ControlResult::Enqueued {
                id: msg_id,
                to: to_name,
            }
        }
        // 배달 실패(세션이 그 사이 사라짐·재시작 중 등) — 도달성 통과 후의 드문 경쟁. 도달 불가로 교정.
        Err(e) => {
            // ★F6 계측(warn = 비정상이나 안전 폴백)★: write_stdin 실패. 에러 디테일({e})은 메시지 끝
            //   보간 허용(식별자·수치만 필드 — logging-conventions §형식). body/토큰은 안 싣는다.
            tracing::warn!(
                to = %to_id,
                to_name = %to_name,
                msg_id = %msg_id,
                entrance = entrance.as_str(),
                "제어 채널 relay 실패(write_stdin) — 도달 불가로 교정: {e}"
            );
            // ADR-0088: 실패도 기계 소비용 레코드로 남긴다 — 하네스가 "성공으로 삼켜지지 않음"을 단언한다.
            //   bytes_written=None·msg_uuid=None·error=Some(...) = 배달 실패의 명시 증거.
            // ★요청 바이트(FIX-1c)★: 실패 경로엔 outcome 이 없으므로 `wrapped.len()`(넘기려던 논리 메시지
            //   바이트)를 쓴다. 성공 경로의 `outcome.bytes_requested` 와 값은 정의상 같다(= 세션에 넘긴
            //   `wrapped.as_bytes().len()`) — write_input_observed 가 그 len 을 그대로 요청량으로 삼기 때문.
            //   여기선 그 세션 호출이 Err 로 끝나 outcome 이 없을 뿐이라, 넘기려던 요청량으로 대체한다.
            // ADR-0088
            registry.record_delivery(DeliveryObservation {
                msg_id: msg_id.clone(),
                to_id,
                to_name: to_name.clone(),
                from: cmd.from,
                entrance,
                bytes_requested: wrapped.len(),
                bytes_written: None,
                msg_uuid: None,
                // ADR-0088: write 실패 = **완결된 write 가 없음** → attest 할 착지 incarnation 이 없다(None).
                //   0바이트 이동 주장이 아니다 — write_all 이 Err 전에 prefix 를 물리적으로 흘렸을 수 있다
                //   (core stdio_physical_pipe 부분-write 하네스가 그 축을 증명). 완결성 교리(Ok=완결/Err=실패,
                //   바이트 비교 아님)와 정합: msg_uuid/bytes_written 이 None 인 것과 같은 이유로 to_epoch 도 None.
                to_epoch: None,
                error: Some(e.to_string()),
            });
            ControlResult::Error {
                code: "RECIPIENT_NOT_REACHABLE",
                hint: format!("Delivery to '{to_name}' failed: {e}"),
            }
        }
    }
}

/// `to`(이름 또는 AgentId 문자열) → 산 에이전트 해석. 매치 규칙(ADR-0086 §6):
///   - ★정확한 AgentId 문자열 우선(F2)★. 이름과 별개 축이며 **이름 매치보다 먼저** 시도한다.
///   - 그 다음 이름(AgentInfo.name = profile name) 정확 일치. 여러 개면 RECIPIENT_AMBIGUOUS(후보 name+id 나열).
///   - 없으면 RECIPIENT_NOT_FOUND(산 에이전트 이름 나열 = 미니 로스터, 자기교정용).
///
/// ★왜 ID 를 먼저 보나(F2)★: 어떤 에이전트의 *이름*이 우연히 다른 에이전트의 UUID 문자열과 같으면,
///   이름 매치를 먼저 하면 ID 로 지목한 메일이 엉뚱한(이름=UUID) 에이전트에게 잡힌다. AgentId 는
///   시스템이 부여하는 안정적·유일한 주소축이므로 ID 형태의 `to` 는 항상 ID 로 먼저 해석한다(이름
///   충돌이 ID 지목을 가로채지 못하게). 그래서 exact-ID 매치가 name 매치를 **선행**한다.
///
/// ★산(live) 판정★: 종료된 세션은 reaper 가 곧 맵에서 제거하나, 스냅샷 순간에 terminal 상태가 남아 있을
///   수 있어 명시적으로 non-terminal(Running/Exiting)만 후보로 본다.
fn resolve_recipient(
    to: &str,
    agents: &[engram_dashboard_core::agent::types::AgentInfo],
) -> Resolution {
    let live: Vec<&engram_dashboard_core::agent::types::AgentInfo> =
        agents.iter().filter(|a| is_live(&a.status)).collect();

    // ★F2: AgentId 문자열 정확 일치를 이름보다 **먼저** 시도★ — 이름=UUID 충돌이 ID 지목을 가로채지 못하게.
    if let Some(a) = live.iter().find(|a| a.id.to_string() == to) {
        return Resolution::Ok {
            id: a.id,
            name: a.name.clone(),
        };
    }

    // 이름 정확 일치 후보(ID 매치 실패 후).
    let by_name: Vec<&&engram_dashboard_core::agent::types::AgentInfo> =
        live.iter().filter(|a| a.name == to).collect();

    match by_name.len() {
        1 => {
            let a = by_name[0];
            return Resolution::Ok {
                id: a.id,
                name: a.name.clone(),
            };
        }
        n if n > 1 => {
            // 동명 다수 → 후보를 name+id 쌍으로 나열해 발신자가 id 로 재지목하게 한다.
            let candidates = by_name
                .iter()
                .map(|a| format!("{}(id:{})", a.name, a.id))
                .collect::<Vec<_>>()
                .join(", ");
            return Resolution::Err(ControlResult::Error {
                code: "RECIPIENT_AMBIGUOUS",
                hint: format!(
                    "Multiple live agents named '{to}': {candidates}. Re-send using the exact agent id."
                ),
            });
        }
        _ => {}
    }

    // 아무 매치 없음 → 산 에이전트 이름 나열(미니 로스터, 자기교정).
    let roster = live
        .iter()
        .map(|a| a.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let roster = if roster.is_empty() {
        "(none)".to_string()
    } else {
        roster
    };
    Resolution::Err(ControlResult::Error {
        code: "RECIPIENT_NOT_FOUND",
        hint: format!("No live agent matches '{to}'. Live agents: {roster}."),
    })
}

/// non-terminal(산) 상태인가. Running/Exiting = 산, Exited/Failed/Killed = terminal.
fn is_live(status: &AgentStatus) -> bool {
    matches!(status, AgentStatus::Running | AgentStatus::Exiting)
}

/// 발신자 표시 이름 — profile name(단일 진실원). 없으면 id 앞 8자(agent_info fallback 과 동형).
fn sender_display_name(manager: &Arc<AgentManager>, from: BoundIdentity) -> String {
    manager
        .profiles()
        .get(from.agent_id)
        .map(|p| p.name)
        .unwrap_or_else(|| {
            let s = from.agent_id.to_string();
            s[..8.min(s.len())].to_string()
        })
}

/// ★메시지 래퍼(의도적 최소 placeholder — ADR-0086 §7)★: B stdin 에 주입할 텍스트를 만든다.
///
/// 최종 봉투 형식(발신자·id·스레드·구조화)은 **범위 밖**이고 후속 스파이크가 정한다. 그때 이 함수 하나만
/// 갈아끼우면 되도록 래퍼 조립을 **여기 한 곳에** 가둔다(호출부는 wrap_message 만 부른다). 형태:
///   `[message from <sender> id:<msg-id>] <body>`
fn wrap_message(sender: &str, msg_id: &str, body: &str) -> String {
    format!("[message from {sender} id:{msg_id}] {body}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_core::agent::types::{
        AgentInfo, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps, SessionCaps,
    };

    fn ident(id: AgentId) -> BoundIdentity {
        BoundIdentity {
            agent_id: id,
            epoch: 0,
        }
    }

    /// 테스트용 AgentInfo — 이름·structured(도달성)·상태를 지정한다.
    fn info(id: AgentId, name: &str, structured: bool, status: AgentStatus) -> AgentInfo {
        AgentInfo {
            id,
            name: name.to_string(),
            cwd: ".".to_string(),
            status,
            cols: 80,
            rows: 24,
            epoch: 0,
            capabilities: Capabilities {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: !structured,
                    structured,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
        }
    }

    // ── Validator: resolve_recipient ────────────────────────────────────────────
    #[test]
    fn resolve_by_unique_name() {
        let id = AgentId::new_v4();
        let agents = vec![info(id, "alice", true, AgentStatus::Running)];
        match resolve_recipient("alice", &agents) {
            Resolution::Ok { id: got, name } => {
                assert_eq!(got, id);
                assert_eq!(name, "alice");
            }
            Resolution::Err(_) => panic!("이름 유일 매치는 성공해야"),
        }
    }

    #[test]
    fn resolve_by_exact_agent_id() {
        let id = AgentId::new_v4();
        let agents = vec![info(id, "alice", true, AgentStatus::Running)];
        match resolve_recipient(&id.to_string(), &agents) {
            Resolution::Ok { id: got, .. } => assert_eq!(got, id),
            Resolution::Err(_) => panic!("정확한 AgentId 문자열도 수용해야"),
        }
    }

    #[test]
    fn resolve_not_found_lists_roster() {
        let agents = vec![info(AgentId::new_v4(), "alice", true, AgentStatus::Running)];
        match resolve_recipient("bob", &agents) {
            Resolution::Err(ControlResult::Error { code, hint }) => {
                assert_eq!(code, "RECIPIENT_NOT_FOUND");
                assert!(hint.contains("alice"), "미니 로스터에 산 이름 나열: {hint}");
            }
            _ => panic!("없는 수신자는 NOT_FOUND"),
        }
    }

    #[test]
    fn resolve_ambiguous_lists_candidates() {
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        let agents = vec![
            info(a, "dup", true, AgentStatus::Running),
            info(b, "dup", true, AgentStatus::Running),
        ];
        match resolve_recipient("dup", &agents) {
            Resolution::Err(ControlResult::Error { code, hint }) => {
                assert_eq!(code, "RECIPIENT_AMBIGUOUS");
                assert!(
                    hint.contains(&a.to_string()) && hint.contains(&b.to_string()),
                    "후보 name+id 쌍 나열: {hint}"
                );
            }
            _ => panic!("동명 다수는 AMBIGUOUS"),
        }
    }

    #[test]
    fn resolve_exact_id_precedes_name_match_f2() {
        // ★F2 회귀★: 에이전트 X 의 이름이 우연히 에이전트 Y 의 UUID 문자열과 같을 때, Y 의 UUID 로 지목하면
        //   ID 로 먼저 해석돼 Y 에게 가야 한다(이름=UUID 인 X 가 가로채면 안 됨).
        let y = AgentId::new_v4();
        let x = AgentId::new_v4();
        let agents = vec![
            // X 의 name = Y 의 UUID 문자열(악의/우연 충돌).
            info(x, &y.to_string(), true, AgentStatus::Running),
            info(y, "yankee", true, AgentStatus::Running),
        ];
        match resolve_recipient(&y.to_string(), &agents) {
            Resolution::Ok { id, name } => {
                assert_eq!(
                    id, y,
                    "ID 지목은 그 ID 의 에이전트(Y)로 — 이름=UUID 인 X 가 가로채면 안 됨"
                );
                assert_eq!(name, "yankee");
            }
            Resolution::Err(_) => panic!("exact-ID 매치가 이름 매치를 선행해야(F2)"),
        }
    }

    #[test]
    fn resolve_skips_terminal_agents() {
        // terminal 상태(Killed)는 산 후보에서 제외 → NOT_FOUND.
        let id = AgentId::new_v4();
        let agents = vec![info(id, "ghost", true, AgentStatus::Killed)];
        assert!(matches!(
            resolve_recipient("ghost", &agents),
            Resolution::Err(ControlResult::Error {
                code: "RECIPIENT_NOT_FOUND",
                ..
            })
        ));
    }

    // ── wrapper: 최소 placeholder shape ─────────────────────────────────────────
    #[test]
    fn wrap_message_shape() {
        let w = wrap_message("alice", "mid-1", "hello world");
        assert_eq!(w, "[message from alice id:mid-1] hello world");
    }

    // ── ControlResult wire shape(양 입구 동일) ──────────────────────────────────
    #[test]
    fn enqueued_json_shape() {
        let r = ControlResult::Enqueued {
            id: "mid".to_string(),
            to: "bob".to_string(),
        };
        let v = r.to_json();
        assert_eq!(v["status"], "enqueued");
        assert_eq!(v["id"], "mid");
        assert_eq!(v["to"], "bob");
        assert!(r.is_enqueued());
    }

    #[test]
    fn error_json_shape() {
        let r = ControlResult::Error {
            code: "GROUPS_NOT_SUPPORTED",
            hint: "h".to_string(),
        };
        let v = r.to_json();
        assert_eq!(v["status"], "error");
        assert_eq!(v["code"], "GROUPS_NOT_SUPPORTED");
        assert_eq!(v["hint"], "h");
        assert!(!r.is_enqueued());
    }

    // ── ControlCommand 정규화: from 은 값(신원)으로만 들어온다(페이로드 아님) ──────────────
    #[test]
    fn control_command_carries_identity_not_payload_from() {
        // ControlCommand 는 from 을 BoundIdentity 값으로만 담는다 — payload 에 from 필드가 없다(구조적 보장).
        let id = AgentId::new_v4();
        let cmd = ControlCommand {
            from: ident(id),
            to: "bob".to_string(),
            body: "hi".to_string(),
        };
        assert_eq!(cmd.from.agent_id, id);
    }
}
