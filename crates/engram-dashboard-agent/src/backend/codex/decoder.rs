//! codex app-server **알림** → 중립 [`OutputEvent`] 번역기.
//!
//! ★들어오는 문은 [`OutputDecoder`] 하나뿐이다★ — 임의 크기 바이트 청크를 받아 라인 재조립까지
//!   여기서 한다. 통로는 봉투를 분기하느라(요청엔 응답 의무가, 응답엔 대기표가 걸린다) 그 줄을
//!   한 번 파싱하고, 그 **원본 바이트를 그대로** 이 문으로 다시 넣는다 — 알림 한 줄당 파싱이 한 번
//!   더 든다.
//!   ★그 비용을 받아들인 이유(두 번째 문을 다시 만들고 싶어지는 자리다)★: pump 는 디코더를
//!   `Box<dyn OutputDecoder>` 로 **move** 해 간다(`transport/stdio.rs`). 그래서 trait 밖의 문은
//!   운영 경로에서 **닿을 수 없고**, 그 문을 쓰면 여기 라인 재조립·상한·resync·UTF-8 규율과 그것을
//!   지키는 테스트가 통째로 배송 경로 밖으로 나간다. 통로가 자기 인스턴스를 따로 만드는 길도
//!   같은 값을 두 벌로 쪼갠다(관측 맵과 "이름별 1 회" 가드가 갈린다). **파싱 한 번이 그보다 싸다.**
//!
//! ★번역(알림 → 이벤트) 자체는 순수하다★ — I/O 도 시계도 전역 상태도 없고 같은 알림은 언제나
//!   같은 이벤트를 낸다. ★단 **프레이밍은 호출 이력에 의존한다**★: 인스턴스가 드는 상태는 부분
//!   라인 버퍼 · resync 플래그 · 진단 계수 셋이고, 앞의 둘은 산출을 바꾼다(같은 청크라도 앞선
//!   호출에 따라 이벤트가 나기도 하고 버려지기도 한다). 골든은 그래서 **새 인스턴스**에 건다.
//!
//! ★모르는 것은 버린다 — `Structured` 로 흘리지 않는다★(TRD §6-2). 그 탈출구는 프론트에서
//!   `kind` 를 label 로 찍고 payload 를 코드블록으로 그려, 결국 **codex 메서드 이름과 프로토콜
//!   JSON 을 사용자 화면에 띄운다.** 대신 **버린 것은 전부 계수·로그로 남는다**(`observe`).
//!   그 대가 = 미지의 신호가 화면에서 로그로 옮겨 간다(TRD §4-7 이 그 회계를 진다).
//!
//! ★턴 경계를 내지 않는다★ — `turn/started`·`turn/completed` 번역과 턴 분류자는 이 단계의 범위
//!   밖이다. 「턴마다 `MessageDone` 정확히 1 회」 계약이 그 자리에서 서야 하는데, `item/completed`
//!   마다 내면 한 턴이 여러 경계로 쪼개진다(TRD §6-1).
//!
//! ★알려진 겹침 — `backend/claude/mod.rs` 의 디코더도 NDJSON 라인 재조립을 한다★. ★단 같은
//!   코드가 아니다★: 그쪽은 청크를 통째로 버퍼에 붙이고 줄마다 앞에서 drain 하며 상한을 **붙인
//!   뒤** 재고 `clear()` 로 비운다. 여기는 줄어드는 슬라이스를 앞으로만 걸으며 상한을 **붙이기
//!   전** 재고 `mem::take` 로 용량까지 반납한다. `flush` 도 갈렸다(그쪽은 꼬리를 파싱, 여기는 버림
//!   — 사유는 그 impl 주석). 그래서 "중복 제거" 는 옮겨 붙이기가 아니라 **어느 알고리즘으로
//!   통일할지 먼저 정하는 일**이고, 공용 transport 층과 claude 구현체를 함께 건드린다.

// chunk 1 은 타입과 번역기만 세우고 배선은 하나도 하지 않는다. 이 디코더를 실제로 파이프에
//   꽂는 것은 chunk 2 이고, 그때 이 allow 를 걷는다.
#![allow(dead_code)]

use std::collections::BTreeMap;

use engram_dashboard_base::logging::mask_secrets;
use serde_json::Value;

use super::protocol::{self, method, Inbound};
use crate::transport::OutputDecoder;
use crate::types::OutputEvent;

/// 로그 한 줄에 실을 상대 문자열 상한(문자 수). ★오류 본문이 4KB 에 이르는 경우가 실측됐다★
/// (모르는 메서드 오류가 유효 메서드 160 개를 전부 열거한다) — 자르지 않으면 로그가 그것으로 덮인다.
const LOG_STRING_LIMIT: usize = 512;

/// 화면으로 나가는 오류 문자열에서 **상대가 준 조각 하나마다** 걸리는 상한(문자 수).
///
/// ★합쳐 놓고 끝에서 자르지 않는 것은 의도다★ — 이어 붙인 순서상 가장 실행 가능한 정보(`steer`
/// = 다음 턴에 그대로 넣을 문장)가 맨 뒤에 오는데, 끝에서 자르면 그것부터 사라진다.
/// ★조각 여섯이 전부 이 상한을 거쳐야 총량이 유계다★ — 하나라도 빠지면 "총량 유계" 가 거짓이 된다.
/// 조각 = `message` · `codexErrorInfo` 라벨 · `additionalDetails` · `errorType`
/// · `detailedExplanation` · `steer.message`.
const ERROR_PART_LIMIT: usize = 1024;

/// 한 줄이 쓸 수 있는 최대 바이트. ★이보다 큰 정상 줄은 알려진 바 없다★ — 실측된 최대치는 4KB
/// 대의 오류 본문이다. 상한이 없으면 개행을 안 보내는 상대가 이 버퍼 하나로 메모리를 가져간다.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// 진단 맵이 보관하는 서로 다른 키의 최대 개수(드리프트·결함 포함 전체).
const MAX_UNTRANSLATED_KEYS: usize = 256;

/// 그중 **일상**(아는 이름·아는 변형) 키가 쓸 수 있는 몫.
///
/// ★이 둘로 나누는 이유 = 상한이 탐지기를 스스로 꺼 버리지 않게 하는 것이다★: 한 통이면 일상
/// 이름만으로 상한이 차서, 정작 드리프트가 도착했을 때 기록할 칸이 없다. 몫을 갈라 두면 일상 키가
/// [`MAX_ROUTINE_KEYS`] 를 넘겨 자라지 못하므로 **드리프트·결함 몫이 남는다.**
/// ★일상 천장을 여기 숫자로 적지 않는다★ — 세어 둔 값은 알림 목록·item 축이 바뀔 때마다 낡는다.
/// 그 성질을 **증명하는 것은 `routine_keys_cannot_crowd_out_drift_keys_in_any_arrival_order`** 이고,
/// 몫을 지우면 그 테스트가 빨개진다.
const MAX_ROUTINE_KEYS: usize = 128;

/// 우리가 **알면서 번역하지 않는** 서버 알림 이름(스키마 0.154.0 의 `ServerNotification` 81 종 중
/// 이 번역기가 손대는 6 종을 뺀 75 종).
///
/// ★이 목록의 목적은 단 하나 — 등급을 가르는 것이다★: 여기 있는 이름은 "아직 안 옮긴 것" 이라
/// 일상이고(debug), 여기 **없는** 이름은 상류가 새로 만든 것이라 드리프트 신호다(warn).
/// 목록이 없으면 둘이 한 통에 섞여, 정작 봐야 할 신호가 일상 소음에 묻힌다.
/// ★낡는 방향이 안전하다★ — 상류가 이름을 지우면 여기 죽은 항목이 남을 뿐이고, 상류가 이름을
/// 더하면 그것은 목록에 없으므로 의도대로 warn 이 된다.
const KNOWN_UNTRANSLATED_METHODS: &[&str] = &[
    "account/login/completed",
    "account/rateLimits/updated",
    "account/updated",
    "app/list/updated",
    "autoApprovalReview/strictReviewRequired",
    "command/exec/outputDelta",
    "configWarning",
    "externalAgentConfig/import/completed",
    "externalAgentConfig/import/progress",
    "fs/changed",
    "fuzzyFileSearch/sessionCompleted",
    "fuzzyFileSearch/sessionUpdated",
    "guardianWarning",
    "hook/completed",
    "hook/started",
    "item/autoApprovalReview/completed",
    "item/autoApprovalReview/started",
    "item/commandExecution/outputDelta",
    "item/commandExecution/terminalInteraction",
    "item/fileChange/outputDelta",
    "item/fileChange/patchUpdated",
    "item/mcpToolCall/progress",
    "item/plan/delta",
    "item/reasoning/summaryPartAdded",
    "item/reasoning/summaryTextDelta",
    "item/reasoning/textDelta",
    "mcpServer/event/stream/notification",
    "mcpServer/oauthLogin/completed",
    "mcpServer/startupStatus/updated",
    "model/rerouted",
    "model/safetyBuffering/updated",
    "model/verification",
    "modelProvider/authRecoveryCompleted",
    "modelProvider/authRecoveryStarted",
    "process/exited",
    "process/outputDelta",
    "project/changed",
    "remoteControl/status/changed",
    "serverRequest/resolved",
    "skills/changed",
    "thread/archived",
    "thread/closed",
    "thread/compacted",
    "thread/deleted",
    "thread/environment/connected",
    "thread/environment/disconnected",
    "thread/goal/cleared",
    "thread/goal/updated",
    "thread/name/updated",
    "thread/project/updated",
    "thread/queue/changed",
    "thread/realtime/closed",
    "thread/realtime/error",
    "thread/realtime/item/completed",
    "thread/realtime/item/started",
    "thread/realtime/item/transcript/delta",
    "thread/realtime/itemAdded",
    "thread/realtime/outputAudio/delta",
    "thread/realtime/sdp",
    "thread/realtime/started",
    "thread/realtime/transcript/delta",
    "thread/realtime/transcript/done",
    "thread/reverted",
    "thread/settings/updated",
    "thread/started",
    "thread/status/changed",
    "thread/unarchived",
    "turn/completed",
    "turn/diff/updated",
    "turn/moderationMetadata",
    "turn/plan/updated",
    "turn/started",
    "warning",
    "windows/worldWritableWarning",
    "windowsSandbox/setupCompleted",
];

/// `ThreadItem` union 의 `type` 값 19 종(스키마 0.154.0 전량). 위 목록과 같은 목적이다 —
/// ★여기 없는 `type` 은 상류가 변형을 더한 것이고, 그것이 이 union 이 자라는 방식이다★.
///
/// TRD 의 번역 표는 이 union 을 20 변형으로 적는데 0.154.0 스키마는 19 다 — 문서 쪽 정정 대상이고
/// 이 목록은 생성기 출력이 정본이다.
const KNOWN_ITEM_TYPES: &[&str] = &[
    "userMessage",
    "hookPrompt",
    "agentMessage",
    "functionCallOutput",
    "plan",
    "reasoning",
    "commandExecution",
    "fileChange",
    "mcpToolCall",
    "dynamicToolCall",
    "collabAgentToolCall",
    "subAgentActivity",
    "webSearch",
    "imageView",
    "sleep",
    "imageGeneration",
    "enteredReviewMode",
    "exitedReviewMode",
    "contextCompaction",
];

/// 도구 호출로 옮기는 `ThreadItem` 변형. 나머지 13 종은 우리 중립 어휘에 자리가 없거나
/// (본문·추론·계획·리뷰 모드 전환) 다른 알림이 이미 나른다.
const TOOL_ITEM_TYPES: &[&str] = &[
    "mcpToolCall",
    "dynamicToolCall",
    "collabAgentToolCall",
    "commandExecution",
    "fileChange",
    "webSearch",
];

/// 옮기지 못한 관측의 등급 — 로그 레벨과 문구를 정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Observed {
    /// 아는 이름·아는 변형인데 아직 안 옮겼다. 일상이므로 debug.
    Routine,
    /// 스키마(0.154.0)에 없는 이름이다 — 상류 드리프트 의심이므로 warn.
    Drift,
    /// 아는 이름인데 모양이 다르거나 읽을 수 없다 — 결함이므로 warn.
    ///
    /// ★warn 인 것이 핵심이다★: 릴리스 데몬에는 `RUST_LOG` 가 닿지 않고 기본 레벨이 warn 이라,
    /// debug 로 두면 **화면을 비우는 실패가 배송 구성에서 아예 기록되지 않는다.**
    Malformed,
}

/// codex app-server 알림 번역기.
#[derive(Debug, Default)]
pub(crate) struct CodexAppServerDecoder {
    /// 마지막 `\n` 뒤 미완성 라인 바이트.
    ///
    /// ★불변식★: **완성 라인이 확정되기 전에는 UTF-8 디코딩하지 않는다.** pump 는 라인 경계도
    /// 문자 경계도 모르는 임의 청크로 던지므로 멀티바이트 문자가 청크 경계에서 잘릴 수 있다.
    /// 바이트로만 잇다가 `\n` 이 온 줄만 디코딩하면 그 잘림이 자연히 흡수된다(개행 `0x0A` 는
    /// UTF-8 연속 바이트로 등장할 수 없어 경계 탐색이 바이트 레벨에서 안전하다).
    /// ★길이는 [`MAX_LINE_BYTES`] 를 절대 넘지 않는다★ — 붙이기 **전에** 판정한다.
    buffer: Vec<u8>,

    /// 상한을 넘긴 줄의 **꼬리를 다음 `\n` 까지 통째 폐기**하는 중인가.
    ///
    /// ★단순히 버퍼만 비우면 안 된다★ — 그 줄의 남은 바이트가 다음 `\n` 까지 "새 줄" 로 파싱돼
    /// 가짜 이벤트를 낼 수 있다. 오염된 줄 하나만 잃고 그 줄이 끝나는 곳부터 복구한다.
    discarding: bool,

    /// 옮기지 못한 관측 → 횟수.
    ///
    /// ★키는 **축 접두사 + 나머지**이고, 상대가 만든 문자열은 언제나 접두사 **뒤**에만 놓인다★ —
    /// 그래서 상대가 어떤 이름을 지어 보내도 자기 축을 벗어나 다른 축의 칸에 떨어질 수 없다.
    /// 축 = `method:` (알림 이름) · `item:` (알림 이름 + item type) · `shape:`·`params:`
    /// ·`item-type:`·`tool:` (결함) · `line:`·`envelope:` (줄·봉투) · `deprecation:`.
    /// 접두사 뒤 내용이 어떤 글자를 담든 축은 첫 세그먼트가 정한다.
    untranslated: BTreeMap<String, u64>,

    /// 위 맵이 몫을 다 써 **기록하지 못한 관측**의 수. ★서로 다른 키의 수가 아니다★ — 같은
    /// 이름이 열 번 오면 열이 오른다. 0 이 아니면 위 맵을 전수로 읽지 말 것.
    untranslated_dropped: u64,
}

impl CodexAppServerDecoder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 옮기지 못한 관측과 그 횟수(진단 전용 — 번역 결과에는 관여하지 않는다).
    ///
    /// ★키는 **저장 시점에 이미** 마스킹·절단을 거친 값이다★([`Self::observe`]) — 그래서 여기서
    /// 다시 거르지 않고 그대로 빌려준다. 마스킹을 경계가 아니라 저장 지점에 두는 것이 더 강하다:
    /// 이 맵으로 나가는 길뿐 아니라 로그로 나가는 길도 같은 한 번으로 덮인다.
    pub(crate) fn untranslated_observations(&self) -> &BTreeMap<String, u64> {
        &self.untranslated
    }

    /// 몫을 다 써 기록하지 못한 **관측의 수**(키 수가 아니다).
    pub(crate) fn untranslated_dropped(&self) -> u64 {
        self.untranslated_dropped
    }

    /// 관측 하나를 계수하고, 그 키를 **처음 본 때만** 로그한다.
    ///
    /// ★등급 판정이 맵 상태보다 **먼저** 선다★ — 맵이 얼마나 찼는지가 "이게 드리프트인가" 를
    /// 바꾸면 안 된다. 그리고 일상 키는 [`MAX_ROUTINE_KEYS`] 를 넘겨 자라지 못해 드리프트·결함의
    /// 몫을 잠식하지 못한다.
    fn observe(&mut self, key: String, what: Observed, detail: &str) {
        // ★마스킹이 절단보다 **먼저**다★: `mask_secrets` 의 패턴은 `sk-`/`sk-proj-` 뒤에 20 자
        //   이상을 요구하므로, 먼저 자르면 경계에 걸친 자격증명이 그 수량자 밑으로 잘려 **마스킹을
        //   빠져나간 채** 로그와 이 맵에 남는다. 절단은 길이 방어(상대가 이름 길이로 메모리를
        //   가져가는 것)이고, 그 방어는 마스킹 뒤에 걸어도 그대로 선다.
        let key = sanitize(&key, LOG_STRING_LIMIT);
        let budget = if what == Observed::Routine {
            MAX_ROUTINE_KEYS
        } else {
            MAX_UNTRANSLATED_KEYS
        };
        match self.untranslated.get(&key).copied() {
            Some(seen) => {
                self.untranslated.insert(key, seen.saturating_add(1));
                return; // 이미 로그한 키 — 여기서 끝난다(스팸 방지).
            }
            None if self.untranslated.len() < budget => {
                self.untranslated.insert(key.clone(), 1);
            }
            None => {
                self.untranslated_dropped = self.untranslated_dropped.saturating_add(1);
                if self.untranslated_dropped == 1 {
                    tracing::warn!(
                        cap = MAX_UNTRANSLATED_KEYS,
                        "codex app-server: 관측 이름이 몫을 넘었다 — 이후는 개수만 센다"
                    );
                }
                return;
            }
        }

        // `key` 는 위에서 이미 마스킹·절단을 거쳤다. `detail` 만 여기서 거른다.
        match what {
            Observed::Routine => {
                tracing::debug!(observed = %key, "codex app-server: 아는 이름인데 번역하지 않는다 — 버린다")
            }
            Observed::Drift => tracing::warn!(
                observed = %key,
                reason = %sanitize(detail, LOG_STRING_LIMIT),
                "codex app-server: 상류 드리프트 신호 — 버린다"
            ),
            Observed::Malformed => tracing::warn!(
                observed = %key,
                reason = %sanitize(detail, LOG_STRING_LIMIT),
                "codex app-server: 읽을 수 없는 줄 — 버린다"
            ),
        }
    }

    /// 아는 이름의 알림인데 모양이 다르면 **결함**이다 — 버리되 그 사실을 남긴다.
    ///
    /// ★로그는 키별 1 회로 눌린다★ — `item/agentMessage/delta` 는 이 프로토콜에서 가장 잦은
    /// 알림이라, 상류가 칸 하나를 바꾸면 **토큰 delta 마다** 경고가 동기 파일 sink 로 나간다.
    /// serde 오류 메시지는 문제가 된 값을 그대로 품으므로(`invalid type: string "…"`) 본문과
    /// 똑같이 마스킹·절단을 거친다.
    fn parse<T: serde::de::DeserializeOwned>(
        &mut self,
        params: Option<&Value>,
        method_name: &str,
    ) -> Option<T> {
        let Some(params) = params else {
            // 번역하는 알림은 전부 params 가 required 다 — 아예 없으면 모양이 다른 것과 같은 결함.
            self.observe(
                format!("params:{method_name}"),
                Observed::Malformed,
                "params 가 없다",
            );
            return None;
        };
        match serde::Deserialize::deserialize(params) {
            Ok(v) => Some(v),
            Err(err) => {
                self.observe(
                    format!("shape:{method_name}"),
                    Observed::Malformed,
                    &err.to_string(),
                );
                None
            }
        }
    }

    /// 알림 하나를 중립 이벤트로 옮긴다. 옮길 어휘가 없으면 **빈 벡터**다 — 오류가 아니다.
    fn translate(&mut self, method_name: &str, params: Option<&Value>) -> Vec<OutputEvent> {
        match method_name {
            method::ITEM_AGENT_MESSAGE_DELTA => self.text_delta(params),
            method::ITEM_STARTED => self.item(params, method_name, true),
            method::THREAD_TOKEN_USAGE_UPDATED => self.usage(params),
            method::ERROR => self.error(params),

            // ★아는 이름인데 일부러 안 옮긴다★ — 이 알림이 나르는 것은 도구 **결과**이고 우리
            //   중립 어휘에 그 축이 없다. `MessageDone` 으로 접으면 한 턴이 item 개수만큼의 경계로
            //   쪼개진다(TRD §6-1). 그래도 item `type` 은 들여다본다 — 새 변형은 여기로도 온다.
            method::ITEM_COMPLETED => self.item(params, method_name, false),

            // 상류 드리프트의 조기 신호라 이름만이 아니라 본문까지 남긴다(TRD §4-7 의 2).
            // ★키 하나로 눌러 담는 것은 의도다★ — 이 파일의 다른 모든 상대발(發) 경고가 키별
            //   1 회인데 여기만 매번 내면, 데몬 기본 레벨이 warn 이고 파일 sink 가 이벤트마다
            //   동기 기록이라 같은 홍수를 이 한 줄로 다시 연다. 대가 = 두 번째 이후 공지의 **본문**은
            //   남지 않는다(도착 횟수는 남는다). 0.154.0 에서 얼마나 자주 오는지는 미측정이다.
            method::DEPRECATION_NOTICE => {
                let payload = params.map(|v| v.to_string()).unwrap_or_default();
                self.observe("deprecation:notice".to_string(), Observed::Drift, &payload);
                Vec::new()
            }

            other => {
                let what = if KNOWN_UNTRANSLATED_METHODS.contains(&other) {
                    Observed::Routine
                } else {
                    Observed::Drift
                };
                self.observe(
                    format!("method:{other}"),
                    what,
                    "스키마(0.154.0) 선언에 없는 알림 이름",
                );
                Vec::new()
            }
        }
    }

    /// `item/agentMessage/delta` → [`OutputEvent::TextDelta`].
    ///
    /// ★claude 와 달리 `turn_id`/`message_id` 가 채워진다★ — codex 는 두 값을 알림에 싣는다.
    /// 잃는 것은 `threadId` 하나다 — 우리 어휘에 스레드 축이 없다.
    fn text_delta(&mut self, params: Option<&Value>) -> Vec<OutputEvent> {
        let Some(n) = self.parse::<protocol::AgentMessageDeltaNotification>(
            params,
            method::ITEM_AGENT_MESSAGE_DELTA,
        ) else {
            return Vec::new();
        };
        vec![OutputEvent::TextDelta {
            text: n.delta,
            turn_id: Some(n.turn_id),
            message_id: Some(n.item_id),
        }]
    }

    /// `item/started`·`item/completed` — 같은 `ThreadItem` union 을 나른다.
    ///
    /// `emit_tool_call` 이 true 인 쪽(`item/started`)만 도구 호출을 낸다. ★양쪽에서 내면 한 호출이
    /// 화면에 두 번 뜬다★. ★그런데 codex 가 모든 item 에 `started` 를 내는지는 미검증★ —
    /// `completed` 만 오는 item 종류가 있다면 그 호출은 사라진다(계수에는 남는다).
    fn item(
        &mut self,
        params: Option<&Value>,
        method_name: &str,
        emit_tool_call: bool,
    ) -> Vec<OutputEvent> {
        let Some(n) = self.parse::<protocol::ItemNotification>(params, method_name) else {
            return Vec::new();
        };
        let Some(kind) = n
            .item
            .get("type")
            .and_then(|v| v.as_str())
            .map(String::from)
        else {
            self.observe(
                format!("item-type:{method_name}"),
                Observed::Malformed,
                "item 에 `type` 문자열이 없다",
            );
            return Vec::new();
        };
        if emit_tool_call && TOOL_ITEM_TYPES.contains(&kind.as_str()) {
            // ★도구 변형인데 이벤트가 안 나오면 그건 "번역 안 함" 이 아니라 결함이다★ — 아래
            //   일상 계수(debug)로 흘려보내면 필수 칸이 빠진 item 이 조용한 소음이 된다.
            return match tool_call(&n.item, &kind, &n.turn_id) {
                Some(event) => vec![event],
                None => {
                    self.observe(
                        format!("tool:{method_name}#{kind}"),
                        Observed::Malformed,
                        "도구 item 에 `tool` 문자열이 없다",
                    );
                    Vec::new()
                }
            };
        }
        // ★여기가 새 변형이 들어오는 자리다★ — 아무 신호도 안 남기면, 상류가 union 을 늘렸을 때
        //   드리프트를 보여 줄 유일한 진단이 하필 드리프트가 나타나는 지점에서 눈을 감는다.
        let what = if KNOWN_ITEM_TYPES.contains(&kind.as_str()) {
            Observed::Routine
        } else {
            Observed::Drift
        };
        self.observe(format!("item:{method_name}#{kind}"), what, "");
        Vec::new()
    }

    /// `thread/tokenUsage/updated` → [`OutputEvent::Usage`].
    ///
    /// ★`total` 이 아니라 `last` 를 쓴다★ — 스키마가 두 칸의 뜻을 설명하지 않아 이름으로 읽었고,
    /// 소비자가 `Usage` 를 **누적하지 않고 이벤트마다 한 행으로 그린다**. 누적값을 반복해 실으면
    /// 같은 수가 자라는 행이 쌓이고, `last` 를 실으면 각 행이 "방금 끝난 요청이 쓴 양" 으로 홀로
    /// 읽힌다. 대가 = 한 턴의 합계는 우리가 내지 않는다(행을 더해야 나온다).
    fn usage(&mut self, params: Option<&Value>) -> Vec<OutputEvent> {
        let Some(n) = self.parse::<protocol::ThreadTokenUsageUpdatedNotification>(
            params,
            method::THREAD_TOKEN_USAGE_UPDATED,
        ) else {
            return Vec::new();
        };
        vec![OutputEvent::Usage {
            // 와이어는 `int64` 이고 우리 어휘 칸은 `u64` 다 — 음수는 토큰 수로 뜻이 없으므로 0 으로
            //   누른다. ★알림을 통째로 버리는 것보다 낫다★(그쪽을 고르면 그 턴의 사용량이 사라진다).
            input_tokens: n.token_usage.last.input_tokens.max(0) as u64,
            output_tokens: n.token_usage.last.output_tokens.max(0) as u64,
            turn_id: Some(n.turn_id),
        }]
    }

    /// `error` 알림 → [`OutputEvent::Error`].
    ///
    /// ★스키마가 가진 유일한 백프레셔 어휘가 이 경로로 온다★ — `serverOverloaded`
    /// ·`rateLimitExceeded` 는 숫자 오류 코드가 아니라 `error.codexErrorInfo` 의 문자열이다.
    /// 그래서 그 라벨을 본문에 붙인다: 붙이지 않으면 "재시도 중인 과부하" 와 "진짜 실패" 가
    /// 화면에서 같아진다.
    ///
    /// ★우리가 더하는 토큰은 전부 프로토콜의 칸 이름 그대로다★(`willRetry`·`misalignment`
    /// ·`steer`) — 이 문자열은 `src/i18n` 을 거치지 않고 화면에 그대로 그려지므로, 여기서 문장을
    /// 지어 쓰면 그것이 번역되지 않는 UI 문구가 된다.
    fn error(&mut self, params: Option<&Value>) -> Vec<OutputEvent> {
        let Some(n) = self.parse::<protocol::ErrorNotification>(params, method::ERROR) else {
            return Vec::new();
        };
        // ★길이를 상대가 정하는 조각은 **하나도 빠짐없이** 잘라야 총량이 유계다★.
        let mut message = truncate(&n.error.message, ERROR_PART_LIMIT);
        if let Some(label) = n.error.codex_error_info.as_ref().and_then(|v| v.as_str()) {
            message = format!("{message} ({})", truncate(label, ERROR_PART_LIMIT));
        }
        if n.will_retry {
            message = format!("{message} [willRetry]");
        }
        if let Some(details) = n.error.additional_details.as_deref() {
            message = format!("{message}\n{}", truncate(details, ERROR_PART_LIMIT));
        }
        // ★`message` 만으로는 왜 막혔는지도, 무엇을 하면 되는지도 알 수 없다★ — 그 둘을 나르는
        //   칸이 여기다. `steer` 는 codex 가 준 "다음 턴에 그대로 넣을 문장" 이라 우리가 짓지 않는다.
        if let Some(m) = n.error.misalignment.as_ref() {
            if let Some(kind) = m.error_type.as_deref() {
                message = format!(
                    "{message}\nmisalignment: {}",
                    truncate(kind, ERROR_PART_LIMIT)
                );
            }
            if let Some(explanation) = m.detailed_explanation.as_deref() {
                message = format!("{message}\n{}", truncate(explanation, ERROR_PART_LIMIT));
            }
            if let Some(steer) = m.steer.as_ref() {
                message = format!(
                    "{message}\nsteer: {}",
                    truncate(&steer.message, ERROR_PART_LIMIT)
                );
            }
        }
        vec![OutputEvent::Error(message)]
    }

    /// 완성 라인 1 개(개행 제외) → 0 개 이상의 이벤트.
    fn consume_line(&mut self, line: &[u8], events: &mut Vec<OutputEvent>) {
        // ★여기서 처음 UTF-8 디코딩한다★(위 `buffer` 불변식).
        let text = match std::str::from_utf8(line) {
            Ok(t) => t.trim(),
            Err(_) => {
                self.observe("line:non-utf8".to_string(), Observed::Malformed, "");
                return;
            }
        };
        if text.is_empty() {
            return;
        }

        match protocol::classify(text) {
            Ok(Inbound::Notification { method, params }) => {
                events.extend(self.translate(&method, params.as_ref()));
            }
            // ★이 문으로는 요청에 답할 수 없다 — 디코더는 쓰기 핸들을 갖지 않는다★. 그래서 이
            //   경로로 요청이 들어오면 그 에이전트는 **영구 정지한다**(상대가 답을 기다린다).
            //   통로는 반드시 봉투를 먼저 갈라 요청을 자기가 받아야 한다.
            Ok(Inbound::Request { method, .. }) => {
                self.observe("envelope:request".to_string(), Observed::Malformed, &method)
            }
            Ok(Inbound::Response { .. }) | Ok(Inbound::Error { .. }) => self.observe(
                "envelope:response".to_string(),
                Observed::Malformed,
                "통로가 응답 봉투를 번역기로 넘겼다",
            ),
            // ★사유마다 키를 나눈다★ — 한 통이면 처음 만난 깨진 줄 하나가 그 뒤의 모든 봉투
            //   결함을 영구히 침묵시킨다. 화면이 비는 실패가 배송 구성에서 기록돼야 한다는 것이
            //   이 경로의 존재 이유인데, 그 목적이 첫 줄에서 끝나 버린다.
            Err(err) => self.observe(
                format!("envelope:{}", envelope_cause(&err)),
                Observed::Malformed,
                &err.to_string(),
            ),
        }
    }
}

impl OutputDecoder for CodexAppServerDecoder {
    /// 임의 크기 바이트 청크를 밀어 넣고 이번 청크로 **완성된 줄**만 옮긴다. 개행 없는 꼬리는
    /// 버퍼에 남아 다음 청크와 합쳐진다 — 한 알림이 읽기 경계에서 잘려도 잃지 않는다.
    ///
    /// 해독 못 한 줄은 기록만 남기고 건너뛴다. ★fatal 로 두지 않는 것은 방어다★: 0.154.0 의
    /// stdout 은 실측상 NDJSON 뿐이었지만(4 회 실행, 배너·비-JSON 0 줄 — 오류는 전부 stderr 로
    /// 갔다) 다른 버전이 같으리라는 보장이 없고, 파싱 실패로 스트림을 끊으면 에이전트가 죽는다.
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
        let mut events = Vec::new();
        let mut rest = chunk;

        // 상한을 넘긴 줄을 버리는 중이면 그 줄이 끝나는 개행 앞을 통째로 버린다.
        if self.discarding {
            match rest.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    self.discarding = false;
                    rest = &rest[nl + 1..];
                }
                None => return events,
            }
        }

        // ★`rest` 는 앞으로만 간다★ — 앞에서 drain 하거나 처음부터 다시 훑지 않으므로 한 청크의
        //   총 훑기 비용이 청크 길이에 선형이다(개행만 4096 개인 청크도 할당 0 회).
        while let Some(nl) = rest.iter().position(|&b| b == b'\n') {
            let head = &rest[..nl];
            rest = &rest[nl + 1..];

            if self.buffer.is_empty() {
                // 흔한 경우 — 버퍼를 거치지 않고 청크 안의 슬라이스를 그대로 읽는다.
                if head.len() > MAX_LINE_BYTES {
                    events.push(overflow_event(head.len()));
                    continue;
                }
                self.consume_line(head, &mut events);
            } else {
                // ★어느 쪽이든 버퍼를 통째로 가져와 비운다★ — 이 줄은 여기서 끝나고, 소유권이
                //   옮겨 가므로 길이만이 아니라 **용량도** 반납된다.
                let mut line = std::mem::take(&mut self.buffer);
                if line.len() + head.len() > MAX_LINE_BYTES {
                    let dropped = line.len() + head.len();
                    drop(line);
                    events.push(overflow_event(dropped));
                    continue;
                }
                line.extend_from_slice(head);
                self.consume_line(&line, &mut events);
            }
        }

        if !rest.is_empty() {
            // ★붙이기 **전에** 판정한다★ — 붙인 뒤 재면 그 순간 상한을 이미 넘긴 뒤다.
            if self.buffer.len() + rest.len() > MAX_LINE_BYTES {
                let dropped = self.buffer.len() + rest.len();
                self.buffer = Vec::new(); // 용량 반납 — `clear()` 는 길이만 0 으로 만든다.
                self.discarding = true;
                events.push(overflow_event(dropped));
            } else {
                self.buffer.extend_from_slice(rest);
            }
        }
        events
    }

    /// 스트림 종료 시 1 회.
    ///
    /// ★공용 트레이트 doc 은 "잔여 라인을 마저 처리한다" 이고 여기는 **버린다** — 의도된 갈림이다★:
    /// 개행으로 끝나지 않은 줄은 **잘린 JSON** 이라, 마저 처리한다는 것이 곧 없는 나머지를 지어내는
    /// 일이 된다. claude 쪽 구현체는 라인 단위 JSON 이 아니어도 되는 자리라 그 doc 대로 처리하므로
    /// 트레이트 문서는 건드리지 않는다. 손실 자체는 관측돼야 할 사실이라 경고로 남긴다.
    fn flush(&mut self) -> Vec<OutputEvent> {
        if !self.buffer.is_empty() {
            tracing::warn!(
                bytes = self.buffer.len(),
                "codex app-server: 개행 없이 스트림이 끝났다 — 잘린 마지막 줄을 버린다"
            );
            self.buffer = Vec::new();
        }
        self.discarding = false;
        Vec::new()
    }
}

/// 봉투를 못 읽은 사유의 안정된 이름. ★[`ParseError`] 의 Display 를 키로 쓰지 않는 이유★ =
/// 그 문자열에는 serde 오류 본문이 섞여 들어와 같은 사유가 매번 다른 키가 된다.
fn envelope_cause(err: &protocol::ParseError) -> &'static str {
    use protocol::ParseError as E;
    match err {
        E::NotJson(_) => "not-json",
        E::NotObject => "not-object",
        E::MethodNotString => "method-not-string",
        E::BadId => "bad-id",
        E::BadError(_) => "bad-error",
        E::MissingId(_) => "missing-id",
        E::ConflictingEnvelope => "conflicting",
        E::UnknownShape => "unknown-shape",
    }
}

/// 상한 초과로 줄 하나를 잃었다는 사실을 화면으로 올린다.
///
/// ★산문이 아니라 토큰이다★ — 이 문자열은 `src/i18n` 을 거치지 않고 그대로 그려지므로, 문장을
/// 지어 쓰면 번역되지 않는 UI 문구가 된다(같은 규율을 `error` 가 적는다).
fn overflow_event(seen: usize) -> OutputEvent {
    // ★`seen` 은 **그 시점까지 본 바이트**이고 줄 전체 길이가 아니다★ — 개행 없이 넘긴 경우
    //   나머지는 resync 가 조용히 삼키므로 아무도 그 총량을 세지 않는다. `dropped=` 로 적으면
    //   줄 크기로 읽힌다.
    OutputEvent::Error(format!(
        "codex-decoder: lineOverflow seen={seen}B limit={MAX_LINE_BYTES}B"
    ))
}

/// 도구 변형 item 한 개 → [`OutputEvent::ToolCall`].
///
/// `None` 은 "도구가 아니다" 가 아니라 **모양이 깨졌다**는 뜻이다 — 호출자는 [`TOOL_ITEM_TYPES`]
/// 로 먼저 거르므로, 여기 도달한 item 은 이미 도구 변형이다.
fn tool_call(item: &Value, kind: &str, turn_id: &str) -> Option<OutputEvent> {
    let name = match kind {
        // 이 셋은 `tool` 칸이 곧 도구 이름이다. `mcpToolCall` 의 `server` 는 이름에 안 섞는다 —
        //   구분자를 지어내는 것이라, 그 값은 아래 `args_json` 에 원형 그대로 남는다.
        "mcpToolCall" | "dynamicToolCall" | "collabAgentToolCall" => {
            item.get("tool")?.as_str()?.to_string()
        }
        // 이 셋은 도구 이름 칸이 없다 — item 종류 자체가 이름이다.
        _ => kind.to_string(),
    };
    Some(OutputEvent::ToolCall {
        name,
        // ★item 전체를 싣는다★ — 변형마다 "무엇이 인자인가" 가 달라(명령줄·패치·질의·arguments)
        //   한 칸을 골라 뽑으면 그 순간 나머지를 잃는다. 이 칸의 계약이 "backend 스키마 그대로" 다.
        args_json: item.to_string(),
        id: item.get("id").and_then(|v| v.as_str()).map(String::from),
        turn_id: Some(turn_id.to_string()),
        // codex item 에는 메시지 묶음 id 개념이 없다 — 호출 식별자는 위 `id` 가 진다.
        message_id: None,
    })
}

/// 상대가 만든 문자열을 로그에 실을 수 있는 모양으로 — 자격증명 마스킹 후 길이 절단.
fn sanitize(s: &str, limit: usize) -> String {
    truncate(&mask_secrets(s), limit)
}

fn truncate(s: &str, limit: usize) -> String {
    match s.char_indices().nth(limit) {
        Some((cut, _)) => format!("{}…", &s[..cut]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★골든은 전부 배송 경로(`decode`)를 탄다★ — 프레이밍까지 함께 재기 위해서다.
    fn decode_line(line: &str) -> Vec<OutputEvent> {
        CodexAppServerDecoder::new().decode(format!("{line}\n").as_bytes())
    }

    fn notification(method: &str, params: Value) -> String {
        serde_json::json!({
            "method": method,
            "params": params,
            "emittedAtMs": 1_764_000_000_000i64,
        })
        .to_string()
    }

    fn notification_line(method: &str, params: Value) -> String {
        format!("{}\n", notification(method, params))
    }

    fn decode_notification(method: &str, params: Value) -> Vec<OutputEvent> {
        CodexAppServerDecoder::new().decode(notification_line(method, params).as_bytes())
    }

    fn delta_line(text: &str) -> String {
        notification(
            "item/agentMessage/delta",
            serde_json::json!({"threadId": "t", "turnId": "u", "itemId": "i", "delta": text}),
        )
    }

    fn text_of(events: &[OutputEvent]) -> Vec<String> {
        events
            .iter()
            .map(|e| match e {
                OutputEvent::TextDelta { text, .. } => text.clone(),
                other => panic!("TextDelta 가 아니다: {other:?}"),
            })
            .collect()
    }

    // ── 라인 재조립(트레이트 계약) ───────────────────────────────────────────

    /// ★4096B 읽기 경계에서 한 알림이 잘리는 것이 이 계약의 존재 이유다★ — 조각을 버리면
    /// 컴파일 오류도 실패 테스트도 없이 출력만 사라진다.
    #[test]
    fn one_notification_split_across_two_chunks_still_emits() {
        let line = format!("{}\n", delta_line("안녕하세요"));
        let bytes = line.as_bytes();
        let cut = bytes.len() / 2;
        let mut d = CodexAppServerDecoder::new();
        assert!(
            d.decode(&bytes[..cut]).is_empty(),
            "줄이 안 끝났는데 이벤트가 났다"
        );
        assert_eq!(text_of(&d.decode(&bytes[cut..])), vec!["안녕하세요"]);
    }

    /// 멀티바이트 문자가 청크 경계에서 잘려도 살아남아야 한다.
    #[test]
    fn a_multibyte_character_split_across_chunks_survives() {
        let line = format!("{}\n", delta_line("한글"));
        let bytes = line.as_bytes();
        // "한" 의 3 바이트 한가운데에서 자른다.
        let cut = line.find("한").expect("payload") + 1;
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(&bytes[..cut]).is_empty());
        assert_eq!(text_of(&d.decode(&bytes[cut..])), vec!["한글"]);
    }

    #[test]
    fn two_notifications_in_one_chunk_both_emit() {
        let chunk = format!("{}\n{}\n", delta_line("first"), delta_line("second"));
        let mut d = CodexAppServerDecoder::new();
        assert_eq!(
            text_of(&d.decode(chunk.as_bytes())),
            vec!["first", "second"]
        );
    }

    #[test]
    fn a_complete_line_plus_a_partial_tail_emits_only_the_complete_one() {
        let chunk = format!("{}\n{}", delta_line("done"), &delta_line("partial")[..20]);
        let mut d = CodexAppServerDecoder::new();
        assert_eq!(text_of(&d.decode(chunk.as_bytes())), vec!["done"]);
    }

    /// 개행 없이 넘어온 줄은 **보류**된다 — 개행이 오면 그때 난다.
    #[test]
    fn a_line_without_a_terminator_is_held_until_one_arrives() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(delta_line("later").as_bytes()).is_empty());
        assert_eq!(text_of(&d.decode(b"\n")), vec!["later"]);
    }

    /// 잘린 마지막 줄은 파싱하지 않고 버린다 — 반쪽 JSON 으로 가짜 이벤트를 내지 않는다.
    #[test]
    fn flush_drops_a_truncated_trailing_line() {
        let partial = &delta_line("never finished")[..30];
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(partial.as_bytes()).is_empty());
        assert!(d.flush().is_empty());
        // 버린 뒤에는 상태가 깨끗하다 — 다음 줄이 그 꼬리와 엉키지 않는다.
        let line = format!("{}\n", delta_line("after"));
        assert_eq!(text_of(&d.decode(line.as_bytes())), vec!["after"]);
    }

    #[test]
    fn flush_on_a_clean_stream_is_empty() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.flush().is_empty());
        let line = format!("{}\n", delta_line("x"));
        assert_eq!(d.decode(line.as_bytes()).len(), 1);
        assert!(d.flush().is_empty());
    }

    /// 개행만 가득한 청크에서 빈 줄은 이벤트도 관측도 만들지 않고, 그 뒤 상태가 깨끗하다.
    /// ★이 테스트는 비용(제곱 여부)을 재지 않는다★ — 재려면 할당·시간 계측이 필요하고, 그 성질은
    /// 여기서는 `decode` 본문의 "앞으로만 간다" 주석이 진다.
    #[test]
    fn a_chunk_of_only_newlines_produces_nothing_and_leaves_clean_state() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(&vec![b'\n'; 4096]).is_empty());
        let line = format!("{}\n", delta_line("after"));
        assert_eq!(text_of(&d.decode(line.as_bytes())), vec!["after"]);
    }

    /// ★개행으로 끝나는 거대한 줄도 상한에 걸려야 한다★ — 붙인 뒤에만 재면 이 경로가 통과한다.
    #[test]
    fn an_oversized_line_that_ends_in_a_newline_is_still_capped() {
        let mut d = CodexAppServerDecoder::new();
        let mut chunk = vec![b'x'; MAX_LINE_BYTES + 1];
        chunk.push(b'\n');
        let events = d.decode(&chunk);
        assert!(
            matches!(events.as_slice(), [OutputEvent::Error(m)] if m.contains("lineOverflow")),
            "{events:?}"
        );
        // 개행에서 끝난 줄이므로 resync 가 필요 없다 — 다음 줄이 바로 난다.
        let line = format!("{}\n", delta_line("recovered"));
        assert_eq!(text_of(&d.decode(line.as_bytes())), vec!["recovered"]);
    }

    #[test]
    fn an_unterminated_oversized_line_resyncs_at_the_next_newline() {
        let mut d = CodexAppServerDecoder::new();
        let events = d.decode(&vec![b'x'; MAX_LINE_BYTES + 1]);
        assert!(
            matches!(events.as_slice(), [OutputEvent::Error(m)] if m.contains("lineOverflow")),
            "{events:?}"
        );
        let tail = format!("more junk\n{}\n", delta_line("recovered"));
        assert_eq!(text_of(&d.decode(tail.as_bytes())), vec!["recovered"]);
    }

    /// 여러 청크에 걸쳐 자라는 줄도 상한을 넘기 전에 끊긴다 — 버퍼가 상한을 넘어 존재한 적이 없다.
    #[test]
    fn a_line_growing_across_chunks_is_capped_before_it_exceeds_the_limit() {
        let mut d = CodexAppServerDecoder::new();
        let mib = vec![b'x'; 1024 * 1024];
        // 4 회면 상한과 정확히 같아 아직 안 넘는다 — 넘기는 것은 다섯 번째다.
        for _ in 0..5 {
            let _ = d.decode(&mib);
            assert!(d.buffer.len() <= MAX_LINE_BYTES, "{}", d.buffer.len());
        }
        assert!(d.discarding, "상한을 넘겼는데 resync 로 들어가지 않았다");
        assert_eq!(d.buffer.capacity(), 0, "용량이 반납되지 않았다");
    }

    // ── 번역하는 알림 ────────────────────────────────────────────────────────

    #[test]
    fn agent_message_delta_becomes_a_text_delta_with_both_ids() {
        match decode_line(&delta_line("안녕")).as_slice() {
            [OutputEvent::TextDelta {
                text,
                turn_id,
                message_id,
            }] => {
                assert_eq!(text, "안녕");
                assert_eq!(turn_id.as_deref(), Some("u"));
                assert_eq!(message_id.as_deref(), Some("i"));
            }
            other => panic!("TextDelta 하나가 아니다: {other:?}"),
        }
    }

    #[test]
    fn token_usage_becomes_usage_from_the_last_breakdown() {
        let events = decode_notification(
            "thread/tokenUsage/updated",
            serde_json::json!({
                "threadId": "t-1", "turnId": "u-1",
                "tokenUsage": {
                    "last": {"inputTokens": 120, "cachedInputTokens": 100, "outputTokens": 7,
                             "reasoningOutputTokens": 3, "totalTokens": 127},
                    "total": {"inputTokens": 9999, "cachedInputTokens": 0, "outputTokens": 8888,
                              "reasoningOutputTokens": 0, "totalTokens": 18887},
                    "modelContextWindow": 272000
                }
            }),
        );
        match events.as_slice() {
            [OutputEvent::Usage {
                input_tokens,
                output_tokens,
                turn_id,
            }] => {
                assert_eq!(*input_tokens, 120);
                assert_eq!(*output_tokens, 7);
                assert_eq!(turn_id.as_deref(), Some("u-1"));
            }
            other => panic!("Usage 하나가 아니다: {other:?}"),
        }
    }

    /// 음수 하나에 그 턴의 사용량 전체가 사라지면 안 된다.
    #[test]
    fn negative_token_counts_clamp_instead_of_dropping_the_event() {
        let events = decode_notification(
            "thread/tokenUsage/updated",
            serde_json::json!({
                "threadId": "t", "turnId": "u",
                "tokenUsage": {
                    "last": {"inputTokens": -5, "cachedInputTokens": 0, "outputTokens": 3,
                             "reasoningOutputTokens": 0, "totalTokens": 0},
                    "total": {"inputTokens": 0, "cachedInputTokens": 0, "outputTokens": 0,
                              "reasoningOutputTokens": 0, "totalTokens": 0}
                }
            }),
        );
        match events.as_slice() {
            [OutputEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            }] => {
                assert_eq!(*input_tokens, 0);
                assert_eq!(*output_tokens, 3);
            }
            other => panic!("Usage 하나가 아니다: {other:?}"),
        }
    }

    #[test]
    fn error_notification_carries_the_codex_label_and_the_retry_token() {
        let events = decode_notification(
            "error",
            serde_json::json!({
                "threadId": "t-1", "turnId": "u-1", "willRetry": true,
                "error": {"message": "upstream busy", "codexErrorInfo": "serverOverloaded"}
            }),
        );
        match events.as_slice() {
            [OutputEvent::Error(message)] => {
                assert_eq!(message, "upstream busy (serverOverloaded) [willRetry]");
            }
            other => panic!("Error 하나가 아니다: {other:?}"),
        }
    }

    /// ★코어가 내는 문자열에 지역화 문장을 섞지 않는다★ — 이 값은 `src/i18n` 을 거치지 않고
    /// 화면에 그대로 그려진다. 골든으로 못 박는다(느슨한 단언은 산문을 통과시킨다).
    #[test]
    fn our_own_tokens_are_protocol_field_names_not_prose() {
        let events = decode_notification(
            "error",
            serde_json::json!({"error": {"message": "x"}, "willRetry": true}),
        );
        match events.as_slice() {
            [OutputEvent::Error(message)] => assert_eq!(message, "x [willRetry]"),
            other => panic!("Error 하나가 아니다: {other:?}"),
        }
    }

    /// 상한 초과 알림도 같은 규율을 진다 — 문장이 아니라 토큰이어야 한다.
    #[test]
    fn the_overflow_event_is_tokens_not_prose() {
        match overflow_event(7) {
            OutputEvent::Error(message) => {
                assert_eq!(
                    message,
                    format!("codex-decoder: lineOverflow seen=7B limit={MAX_LINE_BYTES}B")
                );
            }
            other => panic!("Error 가 아니다: {other:?}"),
        }
    }

    #[test]
    fn misalignment_details_and_steer_reach_the_error_event() {
        let events = decode_notification(
            "error",
            serde_json::json!({
                "error": {
                    "message": "blocked",
                    "misalignment": {
                        "errorType": "policyViolation",
                        "detailedExplanation": "그 요청은 수행할 수 없습니다",
                        "steer": {"message": "다르게 물어보세요"}
                    }
                }
            }),
        );
        match events.as_slice() {
            [OutputEvent::Error(message)] => {
                assert!(message.contains("policyViolation"), "{message}");
                assert!(
                    message.contains("그 요청은 수행할 수 없습니다"),
                    "{message}"
                );
                assert!(message.contains("steer: 다르게 물어보세요"), "{message}");
            }
            other => panic!("Error 하나가 아니다: {other:?}"),
        }
    }

    /// ★조각 여섯이 **전부** 잘려야 총량이 유계다★ — 하나라도 빠지면 한 칸으로 전체가 뚫린다.
    /// 그리고 자르는 자리가 틀리면 안 된다: 끝에서 자르면 가장 실행 가능한 `steer` 부터 사라진다.
    #[test]
    fn every_server_supplied_part_of_an_error_is_capped_and_the_steer_survives() {
        let huge = "z".repeat(50_000);
        let events = decode_notification(
            "error",
            serde_json::json!({
                "willRetry": true,
                "error": {
                    "message": huge,
                    "codexErrorInfo": huge,
                    "additionalDetails": huge,
                    "misalignment": {
                        "errorType": huge,
                        "detailedExplanation": huge,
                        "steer": {"message": "try asking differently"}
                    }
                }
            }),
        );
        match events.as_slice() {
            [OutputEvent::Error(message)] => {
                // 잘리는 조각 여섯 + 우리 토큰·구분자 약간.
                let bound = ERROR_PART_LIMIT * 6 + 200;
                assert!(
                    message.chars().count() < bound,
                    "총량이 유계가 아니다: {} (상한 {bound})",
                    message.chars().count()
                );
                assert!(
                    message.ends_with("steer: try asking differently"),
                    "steer 가 잘려 나갔다"
                );
            }
            other => panic!("Error 하나가 아니다: {other:?}"),
        }
    }

    #[test]
    fn command_execution_item_becomes_a_tool_call_named_after_the_item_kind() {
        let events = decode_notification(
            "item/started",
            serde_json::json!({
                "threadId": "t-1", "turnId": "u-1", "startedAtMs": 1764000000000i64,
                "item": {"type": "commandExecution", "id": "i-9", "command": "cargo test",
                         "cwd": "C:/w", "commandActions": [{"type": "unknown", "command": "cargo test"}],
                         "status": "inProgress"}
            }),
        );
        match events.as_slice() {
            [OutputEvent::ToolCall {
                name,
                args_json,
                id,
                turn_id,
                message_id,
            }] => {
                assert_eq!(name, "commandExecution");
                assert_eq!(id.as_deref(), Some("i-9"));
                assert_eq!(turn_id.as_deref(), Some("u-1"));
                assert!(message_id.is_none());
                let back: Value = serde_json::from_str(args_json).unwrap();
                assert_eq!(back["command"], "cargo test");
                assert_eq!(back["cwd"], "C:/w");
            }
            other => panic!("ToolCall 하나가 아니다: {other:?}"),
        }
    }

    #[test]
    fn mcp_tool_call_item_is_named_after_its_tool_field() {
        let events = decode_notification(
            "item/started",
            serde_json::json!({
                "threadId": "t-1", "turnId": "u-1", "startedAtMs": 1i64,
                "item": {"type": "mcpToolCall", "id": "i-3", "server": "fs", "tool": "read_file",
                         "arguments": {"path": "a.txt"}, "status": "inProgress"}
            }),
        );
        match events.as_slice() {
            [OutputEvent::ToolCall {
                name, args_json, ..
            }] => {
                assert_eq!(name, "read_file");
                let back: Value = serde_json::from_str(args_json).unwrap();
                assert_eq!(back["server"], "fs", "server 는 args_json 에 남아야 한다");
            }
            other => panic!("ToolCall 하나가 아니다: {other:?}"),
        }
    }

    #[test]
    fn web_search_and_file_change_items_are_tool_calls() {
        for (item, expected) in [
            (
                serde_json::json!({"type": "webSearch", "id": "i-1", "query": "rust ndjson"}),
                "webSearch",
            ),
            (
                serde_json::json!({"type": "fileChange", "id": "i-2", "status": "inProgress",
                                   "changes": [{"path": "a.rs", "diff": "-x\n+y", "kind": {"type": "update"}}]}),
                "fileChange",
            ),
        ] {
            let events = decode_notification(
                "item/started",
                serde_json::json!({"threadId": "t", "turnId": "u", "startedAtMs": 1i64, "item": item}),
            );
            match events.as_slice() {
                [OutputEvent::ToolCall { name, .. }] => assert_eq!(name, expected),
                other => panic!("{expected}: ToolCall 하나가 아니다: {other:?}"),
            }
        }
    }

    // ── 버리는 것 — 단 관측은 남긴다 ─────────────────────────────────────────

    /// ★상류가 union 을 늘리는 자리다★ — 여기서 침묵하면 드리프트 진단이 정작 드리프트가
    /// 나타나는 지점에서 눈을 감는다.
    #[test]
    fn an_unknown_item_type_is_counted_under_a_distinguishable_key() {
        let line = notification_line(
            "item/started",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                "item": {"type": "quantumEntanglement", "id": "i-1", "spookiness": 9}
            }),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("item:item/started#quantumEntanglement"),
            Some(&1)
        );
        // 메서드 이름 축과 섞이지 않는다.
        assert!(d
            .untranslated_observations()
            .get("item:item/started")
            .is_none());
    }

    /// 본문은 delta 알림이 나른다 — item 쪽에서 또 내면 같은 글이 두 번 뜬다. 그래도 관측은 남는다.
    #[test]
    fn agent_message_item_emits_nothing_but_is_observed() {
        let line = notification_line(
            "item/started",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                "item": {"type": "agentMessage", "id": "i-1", "text": "hello"}
            }),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("item:item/started#agentMessage"),
            Some(&1)
        );
    }

    #[test]
    fn item_completed_emits_nothing_but_still_inspects_the_item_type() {
        let line = notification_line(
            "item/completed",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "completedAtMs": 2i64,
                "item": {"type": "commandExecution", "id": "i-9", "command": "cargo test",
                         "cwd": "C:/w", "commandActions": [], "status": "completed"}
            }),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("item:item/completed#commandExecution"),
            Some(&1)
        );
    }

    #[test]
    fn unknown_notification_method_emits_nothing_and_is_counted_once_per_name() {
        let line = notification_line("thread/realtime/sdp", serde_json::json!({"sdp": "v=0"}));
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("method:thread/realtime/sdp"),
            Some(&2)
        );
        assert_eq!(d.untranslated_observations().len(), 1);
    }

    /// 우리 어휘에 추론 축이 없다 — 버리되, 버렸다는 사실은 남는다.
    #[test]
    fn reasoning_delta_is_dropped_but_observed() {
        let line = notification_line(
            "item/reasoning/textDelta",
            serde_json::json!({"threadId": "t", "turnId": "u", "itemId": "i", "delta": "musing"}),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("method:item/reasoning/textDelta"),
            Some(&1)
        );
    }

    /// ★상대가 보내는 경고도 키별 1 회로 눌린다★ — 이 파일의 다른 모든 상대발 warn 과 같은
    /// 규율이다. 매번 내면 데몬의 동기 파일 sink 로 같은 홍수를 연다.
    #[test]
    fn deprecation_notice_emits_nothing_and_is_throttled_like_every_other_peer_warning() {
        let line = notification_line("deprecationNotice", serde_json::json!({"message": "gone"}));
        let mut d = CodexAppServerDecoder::new();
        for _ in 0..50 {
            assert!(d.decode(line.as_bytes()).is_empty());
        }
        let seen = d.untranslated_observations();
        assert_eq!(seen.get("deprecation:notice"), Some(&50));
        assert_eq!(seen.len(), 1, "키 하나로 모여야 한다(로그는 첫 1 회뿐)");
    }

    // ── 결함은 결함으로 기록된다(일상 소음에 섞지 않는다) ────────────────────

    #[test]
    fn known_method_with_a_wrong_shape_emits_nothing_and_is_recorded_as_a_defect() {
        let line = notification_line(
            "item/agentMessage/delta",
            serde_json::json!({"threadId": "t", "turnId": "u", "itemId": "i"}),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("shape:item/agentMessage/delta"),
            Some(&1)
        );
    }

    /// ★가장 잦은 알림의 모양이 바뀌면 delta 마다 경고가 나간다★ — 키별 1 회로 눌러야 한다.
    #[test]
    fn a_repeated_shape_defect_is_counted_but_logged_once() {
        let line = notification_line(
            "item/agentMessage/delta",
            serde_json::json!({"threadId": "t", "turnId": "u", "itemId": "i"}),
        );
        let mut d = CodexAppServerDecoder::new();
        for _ in 0..100 {
            assert!(d.decode(line.as_bytes()).is_empty());
        }
        assert_eq!(
            d.untranslated_observations()
                .get("shape:item/agentMessage/delta"),
            Some(&100),
            "키 하나로 모여야 한다(로그는 첫 1 회뿐)"
        );
        assert_eq!(d.untranslated_observations().len(), 1);
    }

    #[test]
    fn known_method_without_params_is_recorded_as_a_defect() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(b"{\"method\":\"error\"}\n").is_empty());
        assert_eq!(d.untranslated_observations().get("params:error"), Some(&1));
    }

    /// ★도구 변형인데 필수 칸이 빠진 것은 결함이다★ — 일상 계수로 흘리면 조용한 소음이 된다.
    #[test]
    fn a_tool_item_missing_its_tool_field_is_recorded_as_a_defect() {
        let line = notification_line(
            "item/started",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                "item": {"type": "mcpToolCall", "id": "i-1", "server": "fs", "status": "inProgress"}
            }),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations()
                .get("tool:item/started#mcpToolCall"),
            Some(&1)
        );
        assert!(
            d.untranslated_observations()
                .get("item:item/started#mcpToolCall")
                .is_none(),
            "결함이 「아는 변형, 번역 안 함」 계수로 새어 들어갔다"
        );
    }

    #[test]
    fn an_item_without_a_type_string_is_recorded_as_a_defect() {
        let line = notification_line(
            "item/started",
            serde_json::json!({"threadId": "t", "turnId": "u", "startedAtMs": 1i64, "item": {"id": "i-1"}}),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert_eq!(
            d.untranslated_observations().get("item-type:item/started"),
            Some(&1)
        );
    }

    /// ★화면을 비우는 실패가 배송 구성에서 기록돼야 한다★ — 이 둘이 조용하면 "에이전트가
    /// 한가하다" 와 구별되지 않는다.
    #[test]
    fn unreadable_lines_are_recorded_as_defects_not_silence() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(b"Warning: something happened\n").is_empty());
        assert!(d.decode(&[0xff, 0xfe, 0x00, b'\n']).is_empty());
        let seen = d.untranslated_observations();
        assert_eq!(seen.get("envelope:not-json"), Some(&1));
        assert_eq!(seen.get("line:non-utf8"), Some(&1));
    }

    #[test]
    fn envelopes_the_transport_should_have_kept_are_recorded() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d
            .decode(b"{\"id\":7,\"method\":\"item/tool/call\",\"params\":{}}\n")
            .is_empty());
        assert!(d
            .decode(b"{\"id\":1,\"result\":{\"thread\":{\"id\":\"t\"}}}\n")
            .is_empty());
        let seen = d.untranslated_observations();
        assert_eq!(seen.get("envelope:request"), Some(&1));
        assert_eq!(seen.get("envelope:response"), Some(&1));
    }

    #[test]
    fn blank_lines_are_not_recorded() {
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(b"\n   \n\n").is_empty());
        assert!(d.untranslated_observations().is_empty());
    }

    // ── 상대가 정하는 성장의 상한 ────────────────────────────────────────────

    /// ★일상 이름이 드리프트의 몫을 잠식하면 안 된다★ — 잠식하면 탐지기가 스스로 꺼진다.
    #[test]
    fn routine_keys_cannot_crowd_out_drift_keys_in_any_arrival_order() {
        // ★서로 다른 일상 키를 전체 상한보다 **많이** 들이민다★ — 실제 알림 이름 목록으로는
        //   서로 다른 키가 75 개뿐이라 상한 근처에도 못 가고, 그러면 몫을 통째로 지워도 이
        //   테스트가 초록이다(그것이 옛 형태의 결함이었다).
        let flood = MAX_UNTRANSLATED_KEYS + 50;
        for drift_first in [false, true] {
            let mut d = CodexAppServerDecoder::new();
            if drift_first {
                d.observe("method:early/drift".to_string(), Observed::Drift, "");
            }
            for i in 0..flood {
                d.observe(format!("method:routine/{i}"), Observed::Routine, "");
            }

            let routine = d
                .untranslated_observations()
                .keys()
                .filter(|k| k.starts_with("method:routine/"))
                .count();
            assert!(
                routine <= MAX_ROUTINE_KEYS,
                "일상 키가 자기 몫을 넘겼다: {routine} (drift_first={drift_first})"
            );

            // 일상으로 아무리 밀어 넣어도 드리프트 칸이 남는다. ★기록됐다 = 그 키를 처음 본
            //   것이고, 로그(warn)가 나가는 조건이 바로 그것이다★ — 로그 자체는 여기서 캡처할
            //   수단이 없어 기록 여부로 대신 잰다.
            d.observe("method:late/drift".to_string(), Observed::Drift, "");
            assert_eq!(
                d.untranslated_observations().get("method:late/drift"),
                Some(&1),
                "일상 키가 드리프트 칸을 먹었다 (drift_first={drift_first})"
            );
            if drift_first {
                assert_eq!(
                    d.untranslated_observations().get("method:early/drift"),
                    Some(&1),
                    "먼저 온 드리프트 키가 밀려났다"
                );
            }
        }
    }

    /// ★상대가 만든 메서드 이름이 item 축 칸에 떨어지면 그 칸의 탐지가 죽는다★ — 접두사가
    /// 그것을 구조적으로 막는다.
    #[test]
    fn a_forged_method_name_cannot_land_in_the_item_axis() {
        let line = notification_line(
            "item/started",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "startedAtMs": 1i64,
                "item": {"type": "agentMessage", "id": "i-1", "text": "hello"}
            }),
        );
        let mut d = CodexAppServerDecoder::new();
        assert!(d.decode(line.as_bytes()).is_empty());
        // 상대가 그 item 축 키와 **글자 그대로 같은** 메서드 이름을 보낸다.
        d.translate("item/started#agentMessage", None);

        let seen = d.untranslated_observations();
        assert_eq!(seen.get("item:item/started#agentMessage"), Some(&1));
        assert_eq!(seen.get("method:item/started#agentMessage"), Some(&1));
    }

    #[test]
    fn distinct_names_are_capped_and_the_excess_is_counted() {
        let mut d = CodexAppServerDecoder::new();
        for i in 0..(MAX_UNTRANSLATED_KEYS + 10) {
            d.translate(&format!("drift/name/{i}"), None);
        }
        assert_eq!(d.untranslated_observations().len(), MAX_UNTRANSLATED_KEYS);
        assert_eq!(d.untranslated_dropped(), 10);
        // 상한에 닿은 뒤에도 **이미 아는 키**는 계속 센다.
        d.translate("drift/name/0", None);
        assert_eq!(
            d.untranslated_observations().get("method:drift/name/0"),
            Some(&2)
        );
    }

    /// 상대가 이름 길이로도 메모리를 가져갈 수 있다 — 개수 몫만으로는 못 막는다.
    #[test]
    fn an_absurdly_long_name_is_bounded_before_it_is_stored() {
        let mut d = CodexAppServerDecoder::new();
        d.translate(&"n".repeat(100_000), None);
        let seen = d.untranslated_observations();
        let (key, _) = seen.iter().next().unwrap();
        assert!(key.chars().count() <= LOG_STRING_LIMIT + 1, "{}", key.len());
    }

    // ── 결정성 ───────────────────────────────────────────────────────────────

    /// 같은 줄은 언제나 같은 이벤트를 낸다 — 내용까지 못 박는다(같음만 재면 늘 비어도 통과한다).
    #[test]
    fn the_same_line_always_yields_the_same_events() {
        let line = format!("{}\n", delta_line("x"));
        let mut d = CodexAppServerDecoder::new();
        let first = format!("{:?}", d.decode(line.as_bytes()));
        let second = format!("{:?}", d.decode(line.as_bytes()));
        assert_eq!(first, second);
        assert!(first.contains("TextDelta"), "{first}");
        assert!(first.contains("\"x\""), "{first}");
    }

    // ── 로그·진단 위생 ───────────────────────────────────────────────────────

    /// ★마스킹을 지우면 반드시 빨개져야 한다★ — 상한만 재던 단언은 `mask_secrets` 를 빼도 통과한다.
    #[test]
    fn sanitize_masks_credentials() {
        let secret = "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let out = sanitize(&format!("token={secret} trailing"), LOG_STRING_LIMIT);
        assert!(!out.contains(secret), "자격증명이 마스킹되지 않았다: {out}");
        assert!(out.contains("***"), "{out}");
        let bearer = sanitize("Authorization: Bearer abcdefghijklmnop", LOG_STRING_LIMIT);
        assert!(!bearer.contains("abcdefghijklmnop"), "{bearer}");
    }

    /// 진단 맵은 호출자가 어디로든 찍는다 — 경계에서 마스킹되어 나가야 한다.
    #[test]
    fn the_observation_map_hands_back_masked_keys() {
        let secret = "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut d = CodexAppServerDecoder::new();
        d.translate(&format!("drift/{secret}"), None);
        let seen = d.untranslated_observations();
        let (key, _) = seen.iter().next().unwrap();
        assert!(
            !key.contains(secret),
            "마스킹되지 않은 키가 새어 나갔다: {key}"
        );
    }

    /// ★자르고 마스킹하면 경계에 걸친 자격증명이 살아남는다★ — `mask_secrets` 의 패턴이
    /// `sk-proj-` 뒤에 20 자 이상을 요구하므로, 먼저 자른 조각은 그 수량자에 안 걸린다.
    #[test]
    fn a_credential_straddling_the_truncation_boundary_is_still_masked() {
        // ★키를 `observe` 에 **직접** 넘긴다★ — `translate` 를 거치면 `method:` 접두사가 절단
        //   지점을 밀어, 자격증명이 경계에 걸치지 못하고 통째로 잘려 나간다(그러면 이 테스트가
        //   아무것도 재지 않는다).
        // 절단 지점이 `sk-proj-` 8 자 + 꼬리 4 자 뒤에 오도록 채운다 — 그 꼬리 4 자는
        //   `mask_secrets` 가 요구하는 20 자 수량자에 못 미친다.
        let padding = "a".repeat(LOG_STRING_LIMIT - 12);
        let key = format!("{padding}sk-proj-{}", "B".repeat(40));
        let mut d = CodexAppServerDecoder::new();
        d.observe(key, Observed::Drift, "");

        let seen = d.untranslated_observations();
        let (key, _) = seen.iter().next().unwrap();
        assert!(
            !key.contains("sk-proj-"),
            "경계에 걸친 자격증명이 마스킹을 빠져나갔다: {key}"
        );
        assert!(key.chars().count() <= LOG_STRING_LIMIT + 1, "{}", key.len());
    }

    #[test]
    fn sanitize_bounds_the_length() {
        let out = sanitize(&"a".repeat(LOG_STRING_LIMIT * 4), LOG_STRING_LIMIT);
        assert!(out.chars().count() <= LOG_STRING_LIMIT + 1, "{}", out.len());
    }

    #[test]
    fn truncate_cuts_on_character_boundaries() {
        assert_eq!(truncate(&"한".repeat(10), 3), "한한한…");
    }
}
