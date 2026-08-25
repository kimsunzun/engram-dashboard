//! 웹뷰가 주인인 명령의 셸쪽 다리 — 부팅 보고를 받아 등록 패킷에 얹고, 데몬이 배달한 봉투를 그 웹뷰로
//! 내려보낸 뒤 답장을 상관시킨다(TRD §3-7 조항 2 · §3-8 의 홉 ②).
//!
//! ★두 번째 제어 표면이 아니다★: 웹뷰 안에서 실제로 도는 것은 여전히 `src/commands/registry.ts` 의
//! `run(id, args)` 하나다(ADR-0055). 이 모듈이 더하는 것은 그 입구로 가는 **바깥 경로**뿐이고, 새 전역
//! 핸들도 두 번째 레지스트리도 만들지 않는다(CLAUDE.md 「LLM-우선 제어」).
//!
//! ## ★예약 이름 — 이 필터가 등록 패킷을 지킨다★
//! 데몬은 **자기가 답하는 이름**이 하나라도 실린 등록 패킷을 통째로 반려한다
//! (`engram-dashboard-daemon` 의 `refuse_names_i_answer` — 겹친 이름만 빼 주지 않는다). 그런데 웹뷰
//! 레지스트리에는 `agent.spawn`·`agent.rename` 처럼 데몬이 답하는 것과 **철자가 같은 id** 가 있고,
//! `tab.create`·`slot.close` 처럼 셸 자기 표가 먼저 답하는 id 도 있다. 걸러내지 않으면 셸의 이름 17 개가
//! 그 한 이름 때문에 **함께** 명부에 못 오른다 — 그러면 LLM 이 창·탭·슬롯을 통째로 못 만진다.
//! 그래서 [`reserved_names`] 가 **선언에서** 그 두 집합을 뽑아 보고 시점에 뺀다(손 목록 금지 — 어휘가
//! 늘면 목록이 조용히 뒤처진다).
//!
//! ## ★배달 대상이 창 하나인 이유★
//! 이벤트를 전 창에 뿌리면 같은 명령이 창 수만큼 실행되고 답장도 그만큼 온다(한 `request_id` 에 답장
//! 하나 — TRD §4-⑤ 위반). 그래서 보고한 창 중 **하나**만 목적지로 삼고, **광고하는 명단도 그 창의
//! 것**이다 — 둘이 다른 창에서 나오면 데몬이 광고한 이름을 실행할 창이 없다([`Registered`]).
//! 고르는 규칙은 [`Registered::repick`], 죽은 창에서 회복하는 경로는 [`ViewCommandBridge::prune_dead`] 다.
//!
//! ## 진입점
//! - [`ViewCommandBridge::report_and_push`] — 부팅 보고(`report_view_commands` invoke).
//! - [`ViewCommandBridge::settle`] — 웹뷰가 낸 결말(`report_command_outcome` invoke).
//! - [`crate::daemon_client::inbound::ViewCommandPort`] 구현 — 수신기가 쓰는 세 창구.
// ADR-0155
// ADR-0156

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_command::{
    CommandDecl, CommandEnvelope, CommandError, CommandReply, ErrorCode, OwnerLookup, OwnerToken,
    RequestId,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use crate::daemon_client::inbound::ViewCommandPort;
use crate::layout::MAIN_WINDOW_LABEL;

/// 셸 → 웹뷰. 목적지 창 하나에만 간다(모듈 헤더 「배달 대상이 창 하나인 이유」).
pub const EVT_COMMAND_REQUEST: &str = "command:request";

/// 웹뷰 명령의 명부상 주인 표식.
///
/// ★데몬은 이 값을 보지 않는다★ — 명부의 주인 키는 그 패킷이 온 **연결**에서 파생되고, 웹뷰 이름도 셸
/// 토큰 아래 함께 오른다(TRD §3-7 조항 2). 이 토큰이 쓰이는 곳은 **셸 안**뿐이다: `route` 가 명부의 답을
/// 겉봉에 덮어 싣는데(그것이 2단 배달의 재료다) 실을 값이 있어야 한다.
const VIEW_OWNER: &str = "engram-dashboard-webview";

/// 웹뷰 답장을 기다리는 상한 — ★바깥 예산 **안**에 들어야 한다★.
///
/// ★없으면 창이 사라진 뒤의 봉투가 영영 안 끝난다★ — `route` 는 마감을 걸지 않는 것이 계약이라
/// (그 함수 doc: 「마감시각은 조립부의 몫」) 이 자리가 조립부다.
///
/// ★★값을 고르는 기준은 「store 조작이 얼마나 걸리나」가 **아니다**★★ — 이 왕복은 데몬이 쥔 자리 표의
/// 마감 **안쪽**에서 돈다(`engram-dashboard-daemon` 의 `CommandDeliveries::DEFAULT_DEADLINE`). 바깥보다
/// 크게 잡으면 두 가지가 동시에 깨진다:
/// - **이 마감이 도달 불가가 된다.** 데몬이 먼저 자리를 거둬 호출자에게 `TIMEOUT`/`retry: never` 로
///   답해 버리고, 뒤늦은 이쪽 결말은 자리 없는 답장(`NoSeat`)으로 버려진다 — 여기 적은 진단 문구도
///   `retry: same-request-id` 도 호출자에게 **한 번도** 닿지 않는다.
/// - **같은 명령이 두 번 돈다.** 데몬 마감 뒤 호출자가 같은 id 로 다시 부르면, 이 자리는 아직 첫 왕복을
///   붙들고 있어 두 번째 봉투가 웹뷰로 또 내려간다. 그 방어선이 [`PendingSlot::hold`] 의 거절이고,
///   부등식은 그 상황 자체가 생기지 않게 한다.
///
/// ★부등식: `VIEW_REPLY_DEADLINE + VIEW_HOP_MARGIN ≤ 데몬 자리 마감`★. 그 자리 마감은 **여기서 볼 수
/// 없다** — 데몬 crate 는 이 패키지의 dev 의존이라 운영 코드가 참조할 수 없다. 그래서 값은 여기 박고,
/// 관계는 둘 다 볼 수 있는 곳에서 문다(`tests/layout_commands.rs` 의
/// `the_webview_deadline_fits_inside_the_daemon_seat`). ★그 셈을 여기 베껴 적지 않는다★ — 데몬 쪽 doc 이
/// 정본이고, 두 사본이 갈리는 날 어느 쪽도 못 믿는다(코어 `CLI_CONTROL_READ_TIMEOUT_SECS` 의 같은 조항).
pub const VIEW_REPLY_DEADLINE: Duration = Duration::from_secs(4);

/// 이 왕복 **밖**에서 흘러가는 시간의 몫 — 데몬↔셸 두 홉의 소켓 왕복 · 적용 태스크 스케줄링 · 결말
/// 직렬화. 데몬의 `CALLER_MARGIN` 과 같은 종류의 항이고, 있어야 위 부등식이 산문이 아니라 사실이 된다.
pub const VIEW_HOP_MARGIN: Duration = Duration::from_secs(3);

// ── 웹뷰가 보내는 모양 ───────────────────────────────────────────────────────

/// 인자 한 칸의 모양 — 웹뷰가 **손으로 적는** 유일한 자리다(Rust 쪽은 선언 매크로가 채운다).
///
/// JSON Schema 조각으로 그대로 펴진다([`ViewCommandHelp::args_schema`]) — 그래서 칸 이름이 그쪽 어휘다.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewArgSchema {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// 읽기/쓰기 표식 — Rust 선언은 `#[effect(..)]` 로 컴파일 옆에서 정하지만 TypeScript 엔 그 자리가 없다.
///
/// ★그래서 웹뷰가 **실어 보내야** 한다★: 상수로 박으면 첫 조회 명령이 붙는 날 명부가 거짓 표식을 광고하고,
/// 그 값은 데몬의 쓰기 보존 회계에 그대로 먹인다(`Effect::Read` = dedup 면제 — ADR-0156). 안 실은 항목은
/// 등록에서 뺀다([`ViewCommandBridge::report`]) — 기본값을 고르는 것이 곧 그 거짓말이기 때문이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewEffect {
    Read,
    Write,
}

impl ViewEffect {
    /// 카탈로그 어휘 — Rust 쪽 `Effect::as_str` 와 **같은 철자**여야 한다(한 명부에 두 방언 금지).
    fn as_catalog_str(self) -> &'static str {
        match self {
            ViewEffect::Read => "Read",
            ViewEffect::Write => "Write",
        }
    }
}

/// 명령 하나의 설명 — 이름만으로는 부를 수 없으니 **인자 모양**이 함께 온다(ADR-0156).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewCommandHelp {
    pub summary: String,
    /// ★`None` = 등록 거절★. `#[serde(default)]` 이지만 기본값을 **고르지 않는다** — 사유는 [`ViewEffect`].
    /// 이 칸을 안 싣는 것은 옛 웹뷰 번들뿐인데, 그 경우 조용히 `Write` 로 광고하느니 이름이 없는 편이 낫다.
    #[serde(default)]
    pub effect: Option<ViewEffect>,
    #[serde(default)]
    pub args: BTreeMap<String, ViewArgSchema>,
    #[serde(default)]
    pub required: Vec<String>,
}

/// 웹뷰가 보고하는 항목 하나.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewCommandDecl {
    pub name: String,
    pub help: ViewCommandHelp,
}

/// 웹뷰로 내려가는 봉투 — ★`owner`·`proto_ver` 를 안 싣는다★. 그 둘은 홉 사이 라우팅 재료이고
/// (`CommandEnvelope` doc) 마지막 홉인 웹뷰는 자기 표 하나만 본다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewCommandRequest {
    pub request_id: String,
    pub name: String,
    pub args: serde_json::Value,
}

// ── 배달 seam ────────────────────────────────────────────────────────────────

/// 봉투를 창 하나로 미는 통로 — ★주입인 이유★: 실물은 `AppHandle::emit_to` 라 창 없이 못 세운다
/// (ADR-0012). 가짜를 꽂으면 필터·상관·마감 전부가 창 0으로 단언된다.
pub trait ViewDispatch: Send + Sync {
    fn deliver(&self, target: &str, request: &ViewCommandRequest) -> Result<(), String>;

    /// 그 label 의 창이 아직 있나 — ★[`ViewDispatch::deliver`] 의 실패로는 대신할 수 없다★.
    ///
    /// Tauri 의 `emit_to` 는 **없는 label 에도 `Ok` 를 준다**(맞는 리스너가 0이면 그냥 아무도 안 깨운다 —
    /// `Listeners::emit_js_filter`). 그래서 「보내 보고 실패하면 죽은 것」이라는 값싼 판정이 여기서는
    /// 성립하지 않고, 죽은 창을 목적지로 쥔 다리는 봉투마다 마감까지 기다렸다가 `TIMEOUT` 을 낸다.
    fn is_alive(&self, label: &str) -> bool;
}

/// 운영 구현 — 그 label 의 웹뷰에만 보낸다.
///
/// ★★`emit_to` 만으로는 창 하나로 좁혀지지 않는다 — 받는 쪽이 자기 label 로 구독해야 한다★★
/// Tauri 는 **`Any` 로 등록된 리스너를 필터와 무관하게 전부** 깨운다(`match_any_or_filter`), 그리고 JS
/// `listen()` 의 기본 타깃이 바로 `Any` 다. 그래서 웹뷰 쪽은 `{ target: getCurrentWindow().label }` 로
/// 건다(`src/commands/viewCommandBridge.ts`) — 그 인자를 빼면 창 수만큼 같은 명령이 실행되고 답장도
/// 그만큼 온다. **이 두 줄은 한 쌍이다.**
pub struct TauriViewDispatch(pub AppHandle);

impl ViewDispatch for TauriViewDispatch {
    fn deliver(&self, target: &str, request: &ViewCommandRequest) -> Result<(), String> {
        self.0
            .emit_to(target, EVT_COMMAND_REQUEST, request)
            .map_err(|e| format!("웹뷰 '{target}' 로 명령을 보내지 못했다: {e}"))
    }

    /// ★`get_window` 이 아니라 `get_webview_window` 을 쓴다★ — 전자는 tauri 의 `unstable` feature 뒤에
    /// 있어 이 셸이 못 켠다(features 를 늘리는 것이 이 조각의 결정이 아니다). 이 앱의 창은 전부 웹뷰 창
    /// 하나짜리라(`tauri.conf.json` 의 정적 둘 + `WebviewWindowBuilder` 로 만드는 팝아웃) 두 label 이
    /// 같은 값이고, 그래서 `Window::label()` 로 보관한 값이 여기서 그대로 조회된다. 웹뷰가 둘인 창을
    /// 만들면 이 등식이 깨진다 — 그때는 보관하는 쪽과 조회하는 쪽을 함께 옮겨야 한다.
    fn is_alive(&self, label: &str) -> bool {
        self.0.get_webview_window(label).is_some()
    }
}

// ── 예약 이름 ────────────────────────────────────────────────────────────────

/// 웹뷰가 가져갈 수 없는 이름 = 데몬이 답하는 것 + 셸이 답하는 것.
///
/// ★손 목록이 아니라 **선언에서** 뽑는다★ — 어느 쪽 어휘가 늘어도 이 그물이 함께 자란다. 손으로 적으면
/// 그 목록만 뒤처지고, 뒤처진 순간 등록 패킷 하나가 통째로 반려된다(모듈 헤더).
/// ★셸 쪽은 **선언**을 센다(표에 꽂힌 것이 아니라)★ — 안전한 방향이다: 선언만 있고 안 꽂힌 이름을
/// 웹뷰가 가져가면 나중에 그것을 꽂는 순간 배달이 조용히 셸로 옮겨간다(`route` 는 표를 먼저 본다).
/// ★데몬 어휘를 **전부** 덮지는 못한다(알려진 한계)★ — 데몬 crate 가 자기 이름을 선언하면(TRD §6 Step 2
/// 의 `mail.*`) 셸은 그것을 컴파일에 안 들여 못 본다. 그때의 방어선은 데몬의 반려 하나뿐이다.
pub fn reserved_names() -> BTreeSet<String> {
    engram_dashboard_agent::commands::COMMAND_SPECS
        .iter()
        .chain(crate::layout::commands::COMMAND_SPECS.iter())
        .map(|spec| spec.name.to_string())
        .collect()
}

// ── 다리 ─────────────────────────────────────────────────────────────────────

/// 이번 보고가 명단을 어떻게 바꿨나 — 부르는 쪽이 차분 등록을 낼지 정하는 재료다.
#[derive(Debug, Default, PartialEq)]
pub struct ReportOutcome {
    /// 보고 뒤 웹뷰 몫 전량.
    pub accepted: Vec<CommandDecl>,
    /// 직전 명단에 없던 것.
    pub added: Vec<CommandDecl>,
    /// 직전에 있었는데 이번에 안 실린 것.
    pub removed: Vec<String>,
    /// 예약 이름이라 뺀 것(로그용 — 보고한 쪽에는 실패가 아니다).
    pub refused: Vec<String>,
}

impl ReportOutcome {
    /// 데몬에 차분을 보낼 일이 있나.
    pub fn changed(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }
}

/// 이름 → 모양(불투명 문자열 — 데몬이 열어보지 않는다, ADR-0156).
type Shapes = BTreeMap<String, String>;

/// 창별 보고를 그대로 둔다 — ★합치지 않는다★.
///
/// ★광고하는 명단과 봉투를 받는 창은 **같은 창에서 나와야 한다**★: 하나로 합쳐 두면 팝아웃의 보고가
/// 명단을 갈아치우는데 배달은 main 으로 계속 가서, 데몬은 B 를 광고하고 main 은 A 를 실행한다 — 광고된
/// 이름이 `UNKNOWN_COMMAND` 로 나가거나 실행되는 이름이 광고에 없다. 오늘은 창마다 같은 정적
/// `contributions` 를 올려 두 집합이 바이트 동일하지만, **그 우연에 계약을 얹지 않는다.**
#[derive(Default)]
struct Registered {
    /// 창 label → 그 창이 보고한 전량.
    reports: BTreeMap<String, Shapes>,
    /// 봉투를 받을 창 = **광고하는 명단의 주인**. `None` = 살아 있는 보고자가 없다.
    host: Option<String>,
}

impl Registered {
    /// 죽은 보고자를 명단에서 뺀다 — ★생사 판정은 **밖에서** 받는다★(락 규율 = [`ViewCommandBridge::prune_dead`]).
    fn forget(&mut self, dead: &[String]) {
        for label in dead {
            self.reports.remove(label);
        }
    }

    /// 목적지를 다시 고른다 — ★main 우선 · 살아 있는 host 유지 · 마지막 수단은 **보이는** 창★.
    ///
    /// **왜 다시 고르나:** host 를 한 번 정하고 끝내면 그 창이 닫힌 뒤 회복 경로가 없다. 실제 경로 —
    /// main 의 단발 보고가 실패하고(그 invoke 는 재시도가 없다) 팝아웃이 host 가 된 뒤 그 팝아웃이 닫히면,
    /// 이후 모든 배달이 죽은 label 로 나가 마감까지 기다렸다가 `TIMEOUT` 이 된다. 이름은 명부에 그대로
    /// 남아 있어 「있는데 영영 안 되는」 상태가 연결 내내 굳는다.
    /// **왜 main 우선인가:** 창마다 같은 App 이 떠서 전부 보고하는데(`src/App.tsx`) 마지막 보고를 따르면
    /// 목적지가 팝아웃으로 옮겨 다니고 그 창이 닫힐 때마다 흔들린다. main 은 닫아도 숨을 뿐이라
    /// (`lib.rs` 의 `CloseRequested`) 수명이 앱과 같다.
    /// - ★**그 전제는 아직 실측되지 않았다**★: 「숨은 main 이 웹뷰 표에 남는가」를 확인하려면 자동화
    ///   하네스가 창을 숨겨 봐야 하는데 이 앱의 권한 설정이 `hide()`·`close()` 를 막아 GUI 로 못 닿았다
    ///   (2026-08-23). 아래 「보이는 창만」 규칙이 그 전제가 틀렸을 때의 **최악을 바꾼다** — 「아무도 안
    ///   보는 창에 조용히 그린다」가 아니라 「host 없음」이 된다.
    ///
    /// **왜 마지막 수단에서 숨은 창을 빼나:** 사전순 첫 생존자를 그냥 고르면 `agent-tree` 가 모든
    /// `slot-popup-N` 보다 앞선다 — 그 창은 설정이 `visible: false` 라 사람이 못 본다. 그러면 `tab.next`
    /// 는 **성공을 답하면서** 아무도 안 보는 창의 탭을 넘긴다(호출자는 자기 지시가 먹힌 줄 안다).
    /// - ★main 은 이 규칙 **밖**이다★ — `--hidden` 부팅에서 숨어 있어도 main 이 목적지다. 그 창은 트레이로
    ///   언제든 다시 보이고(사용자가 여는 그 창이다) 사람이 「메인 창」으로 인지하는 대상이라, 숨김이
    ///   **상태**이지 성질이 아니다. `agent-tree` 는 반대로 설정이 숨김이다.
    /// - ★판정 재료는 손 목록이 아니라 **앱 설정**이다★([`hidden_window_labels`]) — 설정이 숨긴 창이 더
    ///   늘어도 이 규칙이 함께 자란다.
    /// - 남는 후보가 전부 숨은 창이면 **host 는 없다** — 그 이름들은 광고에서 내려가고 `UNKNOWN_COMMAND`
    ///   가 된다. 「보이지 않는 곳에 적용됨」보다 「지금 부를 수 없음」이 호출자에게 참인 답이다.
    fn repick(&mut self, hidden: &BTreeSet<String>) {
        let keep = self
            .host
            .as_deref()
            .filter(|label| self.reports.contains_key(*label));
        if keep == Some(MAIN_WINDOW_LABEL) {
            return;
        }
        self.host = match self.reports.contains_key(MAIN_WINDOW_LABEL) {
            true => Some(MAIN_WINDOW_LABEL.to_string()),
            // 살아 있는 host 를 이유 없이 갈아치우지 않는다 — 자리를 옮기는 것은 죽었을 때뿐이다.
            false => keep.map(str::to_string).or_else(|| {
                self.reports
                    .keys()
                    .find(|label| !hidden.contains(*label))
                    .cloned()
            }),
        };
    }

    /// 지금 광고할 명단 = **host 창이 보고한 것**.
    fn advertised(&self) -> Shapes {
        self.host
            .as_ref()
            .and_then(|label| self.reports.get(label))
            .cloned()
            .unwrap_or_default()
    }
}

/// 앱 설정이 **숨김으로 선언한** 창 label 전량(`tauri.conf.json` 의 `visible: false`).
///
/// ★런타임 가시성이 아니라 **선언**을 읽는다★: `hide()` 로 숨은 창은 사용자가 다시 열 수 있는 상태지만,
/// 설정이 숨긴 창은 애초에 사람에게 보일 자리가 아니다(오늘 그것은 `agent-tree` 하나다). 런타임 상태를
/// 물으면 `--hidden` 부팅의 main 까지 걸려, 트레이로 여는 그 창이 목적지에서 빠진다.
/// 런타임에 만드는 창(팝아웃)은 설정에 없으므로 여기 안 든다 — 그것이 맞다(사람이 연 창이다).
pub fn hidden_window_labels(app: &AppHandle) -> BTreeSet<String> {
    app.config()
        .app
        .windows
        .iter()
        .filter(|window| !window.visible)
        .map(|window| window.label.clone())
        .collect()
}

/// 웹뷰 몫 등록·배달·상관을 한자리에서 쥔다.
pub struct ViewCommandBridge {
    dispatch: Arc<dyn ViewDispatch>,
    reserved: BTreeSet<String>,
    /// 설정이 숨긴 창 — 마지막 수단 목적지에서 뺀다(사유 = [`Registered::repick`]).
    hidden: BTreeSet<String>,
    deadline: Duration,
    state: Mutex<Registered>,
    /// 보고 하나의 **상태 변경과 그 차분 송신**을 한 덩이로 묶는 문(FIX: 두 보고의 뒤집힘).
    ///
    /// ★없으면 데몬과 다리가 갈린다★: 보고 A 가 이름을 더하고 보고 B 가 그것을 뺀 뒤, B 의 송신이 A 보다
    /// **먼저** 큐에 들면 데몬은 둘의 합집합을 쥔 채로 남는다(A 의 `added` 가 나중에 도착한다). 다리는 B 의
    /// 집합만 아는데 명부에는 A 의 이름이 살아 있어, 그 이름이 배달되면 웹뷰가 모른다고 답한다.
    /// 순서를 매기는 칸을 wire 에 더할 수는 없으므로(계약 무변경) **보내는 쪽을 직렬화**한다.
    outbound: tokio::sync::Mutex<()>,
    pending: Arc<Pending>,
}

/// 답장을 기다리는 자리 하나 — **누구에게 보냈나**를 함께 든다.
///
/// ★목적지를 여기 적어 두는 것이 요점이다★: 결말이 들어올 때 대조할 상대는 **그때의 host** 가 아니라
/// 이 봉투를 실제로 받은 창이다. host 는 보고 순서에 따라 바뀔 수 있어(main 이 팝아웃을 덮는다) 현재
/// host 로 대조하면 진짜 답이 반려되고, 반대로 옛 host 의 위조가 통과한다.
struct Waiting {
    /// 이 자리의 신원 — 같은 `request_id` 를 쓰는 **다음** 자리와 구별한다(사유 = [`PendingSlot::drop`]).
    seat: u64,
    target: String,
    answer: oneshot::Sender<Result<serde_json::Value, CommandError>>,
}

/// 답장을 기다리는 자리들 — ★`Arc` 인 이유★: 마감 타이머가 도는 future 는 `'static` 이라 다리를 빌릴 수
/// 없는데, 시간이 지나면 자기 자리를 **직접 치워야** 한다(안 치우면 죽은 창 하나가 맵을 영구히 불린다).
#[derive(Default)]
struct Pending {
    seats: Mutex<HashMap<RequestId, Waiting>>,
    /// 자리마다 새로 뽑는 번호 — 되감기지 않는다.
    next_seat: AtomicU64,
}

impl ViewCommandBridge {
    /// `hidden` = 설정이 숨긴 창 label([`hidden_window_labels`]).
    pub fn new(dispatch: Arc<dyn ViewDispatch>, hidden: BTreeSet<String>) -> Self {
        Self::with_reserved(dispatch, VIEW_REPLY_DEADLINE, reserved_names(), hidden)
    }

    /// 하네스용 — 예약 집합·마감·숨은 창을 직접 준다. ★운영에서는 [`reserved_names`]·
    /// [`hidden_window_labels`] 를 쓸 것★(손 목록은 뒤처진다).
    pub fn with_reserved(
        dispatch: Arc<dyn ViewDispatch>,
        deadline: Duration,
        reserved: impl IntoIterator<Item = String>,
        hidden: impl IntoIterator<Item = String>,
    ) -> Self {
        ViewCommandBridge {
            dispatch,
            reserved: reserved.into_iter().collect(),
            hidden: hidden.into_iter().collect(),
            deadline,
            state: Mutex::new(Registered::default()),
            outbound: tokio::sync::Mutex::new(()),
            pending: Arc::new(Pending::default()),
        }
    }

    /// 죽은 보고자를 걷어내고 목적지를 다시 고른다.
    ///
    /// ★★생존 조회를 **내 락 밖에서** 돈다 — 이 순서를 되돌리지 말 것★★(ADR-0006 「락 보유 중 외부 호출
    /// 금지」): 조회 실물은 Tauri 의 전역 웹뷰 표 락과 창별 락을 잡는다. 지금 그 락을 쥔 채 이 다리로
    /// 되들어오는 경로는 없지만, 순서가 한 번 서면 그 사실을 이 파일 밖의 변경이 조용히 뒤집을 수 있고
    /// 그때는 교착이다. 그래서 **① 락 안에서 label 만 스냅샷 → ② 락 밖에서 조회 → ③ 다시 락 잡고 반영**
    /// 세 걸음으로 가른다.
    /// ★그 사이에 창이 죽거나 새 보고가 들어올 수 있다★ — 스냅샷에 없던 label 은 이번에 안 지워지고
    /// (다음 호출이 잡는다) 이미 사라진 label 은 지워도 무해하다. 회복이 한 박자 늦는 것이 계약이다.
    fn prune_dead(&self) {
        let labels: Vec<String> = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.reports.keys().cloned().collect()
        };
        let dead: Vec<String> = labels
            .into_iter()
            .filter(|label| !self.dispatch.is_alive(label))
            .collect();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.forget(&dead);
        state.repick(&self.hidden);
    }

    /// 창 하나가 부팅에 자기 목록을 알린다 — **그 창의 전량**이다(차분이 아니다).
    ///
    /// ★예약 이름은 조용히 빼되 조용히 넘기지는 않는다★ — 반려하면 나머지 이름까지 못 오르고, 아무 말도
    /// 안 하면 「등록했는데 안 불린다」의 원인이 어디에도 안 남는다. 그래서 빼고 [`ReportOutcome::refused`]
    /// 로 돌려준다(부르는 쪽이 로그로 남긴다).
    /// ★모양이 빈 항목도 뺀다★ — `help` 는 데몬에게 불투명 문자열이라 비어도 등록은 성공하지만, 그 이름을
    /// 발견한 호출자는 인자를 채울 재료가 없어 부를 수 없다(ADR-0156 이 왕복을 없앤 이유가 그것이다).
    /// ★차분은 **광고 집합**의 앞뒤 차이다 — 그 창의 목록이 아니다★: host 가 아닌 창이 보고해도 광고가
    /// 그대로면 보낼 것이 없다(위 [`Registered`] 의 결합 규칙이 그것을 보장한다).
    /// ★★직접 부르는 것은 하네스뿐이다★★ — 운영 경로는 [`ViewCommandBridge::report_and_push`] 이고,
    /// 그것이 상태 변경과 차분 송신을 한 문 안에 묶는다(사유 = `outbound` 필드). 여기만 부르면 그 순서
    /// 보장이 없다.
    ///
    /// ## ★남는 것 — 죽은 host 의 이름은 **재연결까지** 명부에 남는다(알려진 잔여)★
    /// 조회 경로([`ViewCommandBridge::prune_dead`])가 죽은 host 를 먼저 걷어내면, 그 뒤 이 함수가 보는
    /// `previous` 는 **이미 줄어든** 집합이다 — 그래서 사라진 이름이 `removed` 로 나가지 않는다. 데몬
    /// 명부에는 그 이름이 그대로 남아, 부르면 셸까지 왔다가 「모르는 이름」으로 되돌아간다.
    /// ★해소는 재연결의 전량 등록뿐이고, **그 재연결을 예약하는 것은 아무것도 없다**★ — 「다음 재연결이
    /// 곧 고친다」로 읽지 말 것. 고치려면 배달 경로가 차분을 낼 수 있어야 하는데, 그러려면 이 crate 가
    /// 데몬 링크를 알아야 한다(지지 않기로 한 의존이다 — 모듈 헤더).
    pub fn report(&self, label: &str, reported: Vec<ViewCommandDecl>) -> ReportOutcome {
        let mut refused = Vec::new();
        let mut next: Shapes = BTreeMap::new();
        for decl in reported {
            // 예약 이름 · 빈 설명 · **표식 없음** 셋이 같은 자리에서 빠진다(각각의 사유는 위 doc 과
            //   [`ViewEffect`]).
            let Some(effect) = decl.help.effect else {
                refused.push(decl.name);
                continue;
            };
            if self.reserved.contains(&decl.name) || decl.help.summary.trim().is_empty() {
                refused.push(decl.name);
                continue;
            }
            let help = decl.help.to_catalog_item(&decl.name, effect);
            next.insert(decl.name, help);
        }

        // 생존 조회는 **락 밖에서** 먼저 돈다(사유 = `prune_dead`) — 그래야 아래 한 락 안이 순수해진다.
        self.prune_dead();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let previous = state.advertised();
        state.reports.insert(label.to_string(), next);
        // ★보고를 넣은 **뒤에** 다시 고른다★ — 방금 뜬 창이 죽은 host 를 이어받을 수 있어야 한다.
        state.repick(&self.hidden);
        let current = state.advertised();
        drop(state);

        let added = current
            .iter()
            .filter(|(name, help)| previous.get(*name) != Some(help))
            .map(|(name, help)| CommandDecl {
                name: name.clone(),
                help: help.clone(),
            })
            .collect();
        let removed = previous
            .keys()
            .filter(|name| !current.contains_key(*name))
            .cloned()
            .collect();
        ReportOutcome {
            accepted: decls_of(&current),
            added,
            removed,
            refused,
        }
    }

    /// 운영 경로 — 보고를 반영하고, 바뀐 것이 있으면 **같은 문 안에서** 차분을 내보낸다.
    ///
    /// `push(added, removed)` 는 데몬으로 나가는 왕복이다(`commands/view_bus.rs` 가 `UpdateCommands` 를
    /// 짓는다) — 이 crate 는 그 wire 를 모르므로 호출자가 넣는다.
    /// ★문이 덮는 범위가 「변경 + 송신」 둘 다인 것이 요점이다★: 송신만 직렬화하면 두 보고가 상태를 먼저
    /// 뒤집고 차분만 순서대로 나가, 나중 보고의 차분이 옛 상태를 기준으로 계산된다(사유 = `outbound` 필드).
    pub async fn report_and_push<F, Fut>(
        &self,
        label: &str,
        reported: Vec<ViewCommandDecl>,
        push: F,
    ) -> ReportOutcome
    where
        F: FnOnce(Vec<CommandDecl>, Vec<String>) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let _order = self.outbound.lock().await;
        let outcome = self.report(label, reported);
        if outcome.changed() {
            push(outcome.added.clone(), outcome.removed.clone()).await;
        }
        outcome
    }

    /// 웹뷰가 낸 결말을 그 왕복에 붙인다 — 자리가 없으면 `Err`(늦게 온 답 · 두 번째 답 · 남의 키).
    ///
    /// `from` = 결말을 낸 창(Tauri 가 invoke 에 넣어 주는 값 — `commands/view_bus.rs`).
    /// ★봉투를 받은 창만 답할 수 있다★: 상관 키 하나로만 열면 **남의 창이 남의 왕복을 끝낼 수 있다** —
    /// 호출자는 그 위조 결말을 받고, 진짜 창은 자기 답이 반려되는 동안 부수효과는 그대로 일어난다.
    /// 대조 상대가 「지금 host」가 아니라 **그 봉투를 실제로 받은 창**인 이유는 [`Waiting`].
    /// ★대조에 실패해도 자리는 그대로 둔다★ — 빼 버리면 위조 하나가 진짜 답의 자리를 지운다.
    ///
    /// ★두 번째 답을 조용히 먹지 않는다★: 그 상황은 창 둘이 같은 봉투를 받았다는 뜻이라(배달 대상이 하나가
    /// 아니게 됐다는 신호) 부르는 쪽이 로그로 남길 수 있어야 한다.
    pub fn settle(
        &self,
        from: &str,
        request_id: &str,
        outcome: Result<serde_json::Value, CommandError>,
    ) -> Result<(), String> {
        let parsed = uuid::Uuid::parse_str(request_id.trim())
            .map(RequestId)
            .map_err(|_| format!("request_id 가 UUID 가 아니다: {request_id:?}"))?;
        let mut waiting = self.pending.seats.lock().unwrap_or_else(|e| e.into_inner());
        let Some(slot) = waiting.get(&parsed) else {
            return Err(format!(
                "기다리는 왕복이 없다(이미 답했거나 마감을 넘겼다): {parsed}"
            ));
        };
        if slot.target != from {
            return Err(format!(
                "이 왕복은 '{}' 로 내려갔는데 '{from}' 이 답했다 — 결말을 붙이지 않는다: {parsed}",
                slot.target
            ));
        }
        let slot = waiting.remove(&parsed).expect("바로 위에서 본 자리다");
        drop(waiting);
        slot.answer
            .send(outcome)
            .map_err(|_| format!("답장을 받을 쪽이 이미 사라졌다: {parsed}"))
    }

    /// 지금 봉투를 받을 창과 그 창이 광고하는 명단 — ★죽은 보고자를 걸러낸 뒤의 답이다★.
    ///
    /// 광고(등록 패킷)·조회(배달 2단계)·배달이 **전부 이 하나를 지난다** — 세 자리가 각자 상태를 읽으면
    /// 어느 하나만 옛 host 를 보는 순간 광고와 실행이 갈린다.
    fn live_host(&self) -> Option<(String, Shapes)> {
        self.prune_dead();
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let label = state.host.clone()?;
        Some((label, state.advertised()))
    }

    /// 지금 목적지로 잡혀 있는 창(진단·테스트용).
    pub fn host(&self) -> Option<String> {
        self.live_host().map(|(label, _)| label)
    }
}

fn decls_of(map: &BTreeMap<String, String>) -> Vec<CommandDecl> {
    map.iter()
        .map(|(name, help)| CommandDecl {
            name: name.clone(),
            help: help.clone(),
        })
        .collect()
}

impl ViewCommandHelp {
    /// 카탈로그 항목 하나로 편다 — ★Rust 선언이 내는 것과 **같은 칸 이름**을 쓴다★
    /// (`engram_dashboard_command::spec_item_json`). 갈리면 명부 하나에 모양이 두 방언으로 섞여, 그것을
    /// 읽는 LLM 이 명령마다 다른 독법을 써야 한다.
    ///
    /// `effect` 는 웹뷰가 실은 값을 그대로 쓴다(사유 = [`ViewEffect`]). 나머지 셋은 여기서 정하고, 각각
    /// 사실이다:
    /// - `since: 1` — 이 경로로 올라오는 이름에는 세대가 없다. 웹뷰 어휘의 세대를 실제로 세려면 그 쪽에
    ///   카탈로그 번호가 서야 하고, 없는 번호를 지어내느니 「첫 세대」로 고정한다.
    /// - `ok: {}` — 빈 스키마는 「무엇이든」이다. 웹뷰 `run` 의 반환은 명령마다 제각각이고(void·id·Promise)
    ///   계약으로 고정된 적이 없다 — 모양을 지어내는 대신 모른다고 적는다.
    /// - `errors` — 이 다리가 실제로 낼 수 있는 **전량**이다([`ViewCommandBridge`] 의 갈래들): 웹뷰가
    ///   실패했다(`INTERNAL`) · 마감을 넘겼다(`TIMEOUT`) · 보낼 창이 없다(`UNSUPPORTED`) · 같은 번호가
    ///   이미 돈다(`REQUEST_ID_CONFLICT`). ★광고에 없는 코드를 내보내지 않는다★ — 호출자는 이 목록으로
    ///   분기를 짜므로, 안 적힌 코드가 오면 그 분기의 기본 갈래(대개 「모르는 실패」)로 떨어진다.
    ///   ★웹뷰가 던진 오류는 종류를 못 가른다(알려진 한계)★ — 인자 실수도 진짜 실패도 `INTERNAL` + 문구로
    ///   나간다. 가르려면 웹뷰가 타입드 코드를 실어야 하고 그건 이 조각의 결정이 아니다.
    ///
    /// ★`effect` 를 **인자로 받는다** — `self` 의 `Option` 을 읽지 않는다★: 여기서 읽으면 「값이 없으면
    /// 무엇으로 하나」라는 질문이 되살아나고, 그 답은 어떤 것을 골라도 거짓 광고다([`ViewEffect`]).
    /// 부르는 쪽([`ViewCommandBridge::report`])이 이미 그 칸을 벗겨 냈으므로 여기엔 값만 온다.
    fn to_catalog_item(&self, name: &str, effect: ViewEffect) -> String {
        let item = serde_json::json!({
            "name": name,
            "effect": effect.as_catalog_str(),
            "since": 1,
            "summary": self.summary.trim(),
            "args": self.args_schema(),
            "ok": {},
            "errors": [
                ErrorCode::Internal.as_str(),
                ErrorCode::Timeout.as_str(),
                ErrorCode::Unsupported.as_str(),
                ErrorCode::RequestIdConflict.as_str(),
            ],
        });
        // 직렬화는 실패할 수 없다(전부 JSON 값이다). 그래도 패킷을 죽이지 않고 빈 문자열로 접는다 —
        //   빈 `help` 는 보고 필터가 이미 뺀 뒤라 여기 오면 위쪽 계약이 깨진 것이고, 그 신호는 로그가 진다.
        serde_json::to_string(&item).unwrap_or_default()
    }

    fn args_schema(&self) -> serde_json::Value {
        let properties: serde_json::Map<String, serde_json::Value> = self
            .args
            .iter()
            .map(|(field, schema)| {
                (
                    field.clone(),
                    serde_json::to_value(schema).unwrap_or_else(|_| serde_json::json!({})),
                )
            })
            .collect();
        // ★`required` 는 선언된 칸만 남긴다★ — 없는 칸을 required 에 실으면 그 스키마는 아무 인자로도
        //   만족되지 않아 호출자가 영영 못 부른다(Rust 쪽 `lint_spec` 이 같은 것을 컴파일 옆에서 막는다).
        let required: Vec<&String> = self
            .required
            .iter()
            .filter(|field| self.args.contains_key(*field))
            .collect();
        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required,
        })
    }
}

impl ViewCommandPort for ViewCommandBridge {
    /// ★광고하는 것은 **살아 있는 host 창의 목록**이다★ — 재연결마다 다시 고르므로, 죽은 창이 쥐고 있던
    /// 이름이 다음 등록 패킷에 실려 나가지 않는다.
    fn declarations(&self) -> Vec<CommandDecl> {
        self.live_host()
            .map(|(_, shapes)| decls_of(&shapes))
            .unwrap_or_default()
    }

    fn lookup(&self, name: &str) -> OwnerLookup {
        // ★목적지 창이 없으면 「모르는 이름」이다★ — 보낼 곳이 없는데 주인이 있다고 답하면 배달이
        //   `link.send` 까지 가서 오류를 만들고, 호출자는 그 둘을 구분할 수 없다.
        match self
            .live_host()
            .is_some_and(|(_, shapes)| shapes.contains_key(name))
        {
            true => OwnerLookup::Available(OwnerToken::new(VIEW_OWNER)),
            false => OwnerLookup::Unknown,
        }
    }

    fn dispatch(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
        let request_id = env.request_id;
        let Some((target, _)) = self.live_host() else {
            return ready(CommandReply::err(
                request_id,
                CommandError::of(
                    ErrorCode::Unsupported,
                    "no webview has reported its commands yet, so there is nowhere to run this",
                ),
            ));
        };
        let request = ViewCommandRequest {
            request_id: request_id.to_string(),
            name: env.name,
            args: env.args,
        };

        let (answer, wait) = oneshot::channel();
        // ★보내기 **전에** 자리를 건다★ — invoke 로 오는 답이 emit 반환보다 먼저 도착할 수 있고, 그때
        //   자리가 없으면 그 답은 「기다리는 왕복이 없다」로 버려진다.
        // ★같은 번호가 이미 돌고 있으면 **안 보낸다**★ — 사유는 `PendingSlot::hold`.
        let Some(slot) = PendingSlot::hold(&self.pending, request_id, &target, answer) else {
            return ready(CommandReply::err(
                request_id,
                CommandError::of(
                    ErrorCode::RequestIdConflict,
                    "this request id is already running in the webview — wait for its answer instead of resending",
                ),
            ));
        };
        if let Err(detail) = self.dispatch.deliver(&target, &request) {
            // `slot` 이 여기서 drop 되며 자리를 되가져간다.
            return ready(CommandReply::err(
                request_id,
                CommandError::of(ErrorCode::Internal, detail),
            ));
        }

        let deadline = self.deadline;
        Box::pin(async move {
            // ★future 안으로 들고 들어간다★ — 마감이든 취소든(이 future 를 drop 하면 왕복이 그대로 버려진다,
            //   `route` 의 취소 의미) 자리를 되가져가는 경로가 `Drop` 하나로 모인다.
            let _slot = slot;
            match tokio::time::timeout(deadline, wait).await {
                Ok(Ok(outcome)) => CommandReply {
                    request_id,
                    outcome,
                },
                // 송신단이 답 없이 사라졌다 — `settle` 이 자리를 빼 갔는데 값을 못 넣은 경우뿐이다.
                Ok(Err(_)) => CommandReply::err(
                    request_id,
                    CommandError::of(
                        ErrorCode::Internal,
                        "the webview answer channel closed without an answer",
                    ),
                ),
                Err(_) => CommandReply::err(
                    request_id,
                    CommandError::of(
                        ErrorCode::Timeout,
                        format!(
                            "the webview did not answer within {}s — the window may be gone",
                            deadline.as_secs()
                        ),
                    ),
                ),
            }
        })
    }
}

/// 답장 자리 하나의 수명 — ★비우는 경로가 `Drop` 하나다★.
///
/// 자리를 손으로 지우게 두면 빠뜨린 갈래가 맵을 영구히 불린다. 실제로 갈래가 넷이다: 배달 실패 · 정상
/// 답장(`settle` 이 이미 빼 갔다) · 마감 · **이 future 자체가 drop 되는 경우**(취소 — `route` 의 그 의미).
/// 마지막 하나는 손으로는 닿을 수 없는 자리라, 없으면 런타임이 접힐 때마다 자리가 남는다.
struct PendingSlot {
    pending: Arc<Pending>,
    request_id: RequestId,
    /// 내 자리의 신원 — [`PendingSlot::drop`] 이 **남의 자리를 안 지우게** 하는 유일한 재료다.
    seat: u64,
}

impl PendingSlot {
    /// 자리를 잡는다 — ★이미 도는 번호면 `None` 이고, **살아 있는 대기자를 밀어내지 않는다**★.
    ///
    /// 조용히 덮으면 같은 명령이 두 번 돈다: 첫 왕복이 아직 웹뷰에서 도는 중에 같은 번호가 다시 오면
    /// 봉투가 한 번 더 내려가고(부수효과 2회), 첫 대기자는 답을 못 받은 채 버려지며, 먼저 도착한 **옛**
    /// 결말이 새 시도의 답으로 붙는다. 거절이 그 셋을 한꺼번에 막는다.
    /// ★그런데도 이 상황 자체가 생기지 않는 것이 정상이다★ — 재시도가 이 자리에 닿으려면 바깥 마감이
    /// 안쪽보다 먼저 끝나야 하고, 그 순서는 [`VIEW_REPLY_DEADLINE`] 의 부등식이 막는다. 이 거절은 그
    /// 부등식이 깨졌을 때의 두 번째 그물이다.
    fn hold(
        pending: &Arc<Pending>,
        request_id: RequestId,
        target: &str,
        answer: oneshot::Sender<Result<serde_json::Value, CommandError>>,
    ) -> Option<Self> {
        let mut waiting = pending.seats.lock().unwrap_or_else(|e| e.into_inner());
        match waiting.entry(request_id) {
            Entry::Occupied(_) => None,
            Entry::Vacant(vacancy) => {
                let seat = pending.next_seat.fetch_add(1, Ordering::Relaxed);
                vacancy.insert(Waiting {
                    seat,
                    target: target.to_string(),
                    answer,
                });
                drop(waiting);
                Some(PendingSlot {
                    pending: Arc::clone(pending),
                    request_id,
                    seat,
                })
            }
        }
    }
}

impl Drop for PendingSlot {
    /// ★**내 자리일 때만** 지운다★ — 번호가 같아도 자리가 다르면 남의 것이다.
    ///
    /// 막는 인터리브: `settle` 이 X 를 빼 답을 보내고 → 같은 X 로 새 봉투가 새 자리를 잡은 뒤 → 옛
    /// future 의 이 가드가 떨어지며 **새 자리**를 쫓아내면, 그 대기자는 닫힌 채널로 실패한다.
    /// ★오늘은 그 인터리브가 안 생긴다 — 그래도 신원으로 막는다★: 가드는 async 블록 끝에서 떨어지고 그
    /// 시점은 결말이 데몬에 닿기 **전**이라, 데몬이 같은 번호를 다시 열 창이 없다. 하지만 그 논증은
    /// 「가드가 언제 떨어지나」·「데몬이 언제 번호를 재사용하나」 **두 모듈의 성질**에 걸려 있어, 어느
    /// 한쪽이 바뀌면 조용히 무너진다(그 변경이 이 파일을 건드릴 이유도 없다). 신원 비교는 그 논증 자체를
    /// 필요 없게 만든다.
    fn drop(&mut self) {
        let mut waiting = self.pending.seats.lock().unwrap_or_else(|e| e.into_inner());
        if waiting
            .get(&self.request_id)
            .is_some_and(|held| held.seat == self.seat)
        {
            waiting.remove(&self.request_id);
        }
    }
}

fn ready(reply: CommandReply) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
    Box::pin(async move { reply })
}
