//! 셸이 주인인 명령 표 — `window`/`tab`/`slot` 은 **선언과 본문이 적용 서비스 옆**에 산다(ADR-0155 결정 1).
//!
//! 사람 클릭(`commands/layout.rs` 의 `#[tauri::command]`)과 중계된 LLM 호출(인바운드 수신기)이 **같은
//! 적용 서비스**(`super::apply`)에 떨어진다 — 이 파일은 그 서비스로 가는 **두 번째 껍데기**이지 두 번째
//! 제어 표면이 아니다(ADR-0081 결정 3 · 「거부한 대안」의 `ViewManager` 직접 호출을 여기서도 하지 않는다).
//!
//! ★레이아웃 밖의 셸 명령도 여기 선다★ — `ui.refresh` · `settings.*` · `restore.*` 는 적용 서비스를 안 거치지만
//! **셸이 주인인 이름**이라 이 표에 든다. 두 번째 선언 블록을 만들면 등록 패킷·세대 번호·중복 검사가 표마다
//! 갈린다(매크로 제약도 「모듈 하나에 블록 하나」다). 그 대신 포트는 자기 것을 따로 든다
//! ([`LayoutPorts::ui_settings`] · [`LayoutPorts::settings`] · [`LayoutPorts::settings_events`] ·
//! [`LayoutPorts::restore`]).
//!
//! 진입점: [`make_table`](fn.make_table.html)(조립) · [`LayoutPorts`](struct.LayoutPorts.html)(주입 seam).
//!
//! ## ★선언이 서비스와 갈리는 세 자리(알고 남긴 것)★
//! 선언 매크로의 타입 알파벳이 `Uuid`·중첩 enum·재귀 타입을 못 실어서 생긴 번역이다 — 매크로 제약이
//! 원인이고 계약을 새로 만든 것이 아니다.
//! - **id 는 전부 `String`** 이고 핸들러가 파싱한다(형식 불량 = `INVALID_ARGUMENT`).
//! - **`SlotContent` 는 태그 + `agent_id` · `show_claude` · `show_codex` 곁칸으로 펴서 받는다** — 매크로의
//!   enum 은 필드 없는 variant 만 싣는다. 조합 검사는 [`slot_content`] 가 한다.
//! - **`get_view` 는 선언에 없다** — 반환이 `ViewSnapshot`(재귀 `LayoutNode`)이고 매크로가 재귀 타입에서
//!   컴파일 에러로 멈춘다. 그래서 v1 조회는 `tab.list`·`window.list`·`slot.resolveSpatial` 셋이다.
//!
//! ## ★적용 실패는 코드 하나로 나간다(`CONFLICT`)★ — 레이아웃 명령만
//! `settings.*` · `restore.answer` 는 서비스가 타입드 오류를 내므로 종류대로 옮긴다([`settings_error`] ·
//! [`restore_error`]). 아래는 적용 서비스 몫이다.
//!
//! 적용 서비스가 실패를 `String` 으로만 주므로 여기서 종류를 가를 재료가 없다. 문구로 코드를 합성하는
//! 것은 금지다(TRD §4-⑦ — `message` 로 기계 분기하지 않는다. CLI 의 문자열 패턴매칭을 끝내려고 둔 계약이
//! 바로 그것이다). 그래서 **모든 실패에 참인 코드**를 고른다: `CONFLICT` = 「지금 상태로는 그 요청을 적용할
//! 수 없다」. `NOT_FOUND` 는 main 창 거부에 거짓이고 `INTERNAL` 은 오타 난 id 에 거짓이다. 사유는 문구가
//! 그대로 나른다. 종류를 가르려면 적용 서비스가 타입드 오류를 내야 하고 그건 이 파일의 결정이 아니다.
// ADR-0155
// ADR-0081

use std::sync::Arc;

use uuid::Uuid;

use engram_dashboard_agent::commands::normalize_cwd;
use engram_dashboard_command::{
    blocking_handler, declare_commands, CommandError, CommandFuture, CommandHandler, CommandTable,
    ErrorCode,
};

use super::apply;
use super::{
    AgentSpawner, LabelSource, LayoutEvents, LayoutState, SlotContent, SplitDir, SubscriptionSync,
    WindowHost,
};
use crate::settings::{
    reset_and_notify, set_and_notify, SettingsError, SettingsEvents, SettingsService,
};
use crate::state::restore::{AnswerError, RestoreCoordinator};
use crate::ui_settings::UiSettingsRefresh;

// ★이름은 프론트 레지스트리가 **오늘 등록한 id** 를 그대로 쓴다★: `tab.create`·`slot.focus`·`slot.popout`·
//   `layout.setSlotContent`·`agent.spawnInto` 는 `src/commands/*Commands.ts` 에 실재하는 id 다(실측
//   2026-08-17). 여기서 다른 철자를 지으면 같은 동작에 이름이 둘이 되고, 화면 몫 등록(TRD §6 Step 4 —
//   그 id 를 바꾸지 않는다고 적은 자리)이 그 둘을 다 지고 간다.
// ★프론트에 짝이 없어 새로 짓는 이름도 있다★(아래는 예이고 전부가 아니다) — `tab.list`·`window.list`(조회는
//   프론트 id 가 없다) · `slot.split`(프론트는 방향을 이름에 박아 `slot.split.topBottom`/
//   `slot.split.leftRight` 둘로 두지만, 버스에서는 방향이 **인자**다 — 그래야 호출자가 방향을 값으로 고른다) ·
//   `ui.refresh`(프론트에 대응 명령이 없다 — 화면에는 파일을 보는 명령 자체가 없다. 파일을 안 보고 테마만
//   만지던 화면 명령 둘은 ADR-0167 이 내렸다) · `split.setRatio`·`split.list`(화면의 구분선 드래그는 Tauri
//   `set_split_ratio` 를 직접 부르고 레지스트리에 이름을 싣지 않는다 — ADR-0227).
// ★세대 13 = `restore.status` 의 답에 `saves` 가 붙은 세대★(TRD S21-storage §6-5 — 사용자 결정 2026-10-06) — 이름은
//   그대로고 답 모양이 바뀌었다.
// ★세대 12 = `restore.status` 의 답에 `durable` · `state_file` 이 붙은 세대★(TRD S21-storage §6-5 · §6-7) — 이름은
//   그대로고 답 모양이 바뀌었다.
// ★세대 11 = 크래시 사본 명령 둘(`restore.status`·`restore.answer`)이 든 세대★(TRD S21-storage §6-7).
// ★세대 10 = 셸 설정 명령 넷(`settings.get`·`settings.set`·`settings.reset`·`settings.schema`)이 들고
//   `ui.refresh` 가 전역 테마를 파일이 아니라 설정 `theme.default` 에서 읽는 세대★(TRD S21-storage §5-4 ·
//   §5-6) — 답 모양은 그대로고 `theme` 의 출처와 summary 가 바뀌었다.
// ★세대 9 = `layout.setSlotContent` 가 사용량 슬롯(`content=Usage` + `show_claude`·`show_codex`)을 받는 세대★
//   (TRD S21 usage-limit-slot §1-7) — 이름은 그대로고 인자 어휘와 칸이 는 세대다.
// ★세대 8 = 분할 비율 명령 둘(`split.setRatio`·`split.list`)이 든 세대★(ADR-0227).
// ★세대 7 = 그 `backend` 칸이 `"codex"` 를 **실제로 만드는** 세대★(2026-09-22 · ADR-0219) — 인자의
//   **타입도 낱말 집합도 그대로**고(wire 어휘는 claude·codex 둘 그대로, 모르는 낱말은 여전히 모르는
//   낱말) 바뀐 것은 정책이 그 낱말을 열었다는 것과 **그것을 광고하는 summary** 다. 세대 5 가 같은 축의
//   선례다 — 거기서도 바뀐 것은 타입이 아니라 어휘와 summary 였다.
//   (세대 6 = 그 칸이 `"codex"` 를 **정책으로** 거절하던 세대 — 그 거절은 위 ADR 이 걷었다 ·
//   세대 5 = 그 칸이 실제로 고를 수 있게 된 세대 — 인자의 **타입은 그대로**고(`Option<String>`) 바뀐 것은
//   그 칸이 받는 어휘와 그것을 광고하는 summary 다 · 세대 4 = `ui.refresh` 의 답에 `source` 가 붙은 세대 ·
//   세대 3 = `ui.refresh` 자체 · 세대 2 =
//   `slot.popout`). ★이름이 늘 때만 올리는 번호가 아니다★ — **선언이 바뀌면** 올린다(답 모양도 선언이다).
//   매크로 계약 = `declare_commands!` 의 `CATALOG_VERSION` 항목. 안 올리면 codex 를 만들어 주는 셸과
//   거절하는 셸이 같은 세대를 보고해 진단이 거짓말을 한다.
//   ★분기 재료가 아니다★ — 받는 쪽이 이 번호로 거절하면 그게 틀린 것이다(`CommandEnvelope::proto_ver`).
//   ★wire 프로토콜 판(`engram_dashboard_protocol::PROTOCOL_VERSION`)과 다른 번호다★ — 그쪽은 프레임 계약이고
//   이쪽은 이 crate 의 어휘 세대다. 하나를 올린다고 다른 하나가 따라 올라가지 않는다.
declare_commands! {
    catalog_version: 13;

    /// 탭 바 한 칸.
    struct TabRow {
        id: String,
        name: String,
    }

    /// 나눌 방향 — 이름이 결과 배치를 말한다(`LeftRight` = 좌/우, `TopBottom` = 위/아래). // ADR-0140
    ///
    /// ★철자가 Tauri invoke 경로(`left_right`/`top_bottom`)와 다르다★ — 선언 매크로가 serde rename 을 못
    /// 달아 Rust variant 이름이 그대로 wire 값이 된다. 두 표면이 같은 뜻을 다른 철자로 받는 것은 알고
    /// 남긴 것이고, 변환은 핸들러가 한다.
    enum SplitDirection {
        LeftRight,
        TopBottom,
    }

    /// 창별 테마 파일을 썼나 — `File` = 읽어서 창 항목을 적용했다, `Fallback` = 없거나 못 써서 창 항목 없이
    /// 접혔다(모든 창이 전역 값).
    ///
    /// ★`theme` 만으로는 못 가르는 것을 가른다★: 창 항목이 반영된 화면과 파일이 통째로 반려돼 모든 창이 전역
    /// 값인 화면이 같은 `theme` 을 낸다. 접힌 **사유**(없음·못 읽음·깨짐·객체 아님·상한 초과)는 여기 안
    /// 싣는다 — 그건 앱 로그가 지고, 올리면 호출자가 사유별 분기를 짜 그 다섯이 계약이 된다.
    ///
    /// 셸 안쪽 쌍둥이는 `crate::ui_settings::ThemeSource` 다(`SplitDirection`↔`SplitDir` 과 같은 관계 —
    /// 선언 매크로가 남의 타입을 못 실어서 생긴 번역이지 다른 계약이 아니다).
    enum ThemeOrigin {
        File,
        Fallback,
    }

    /// 슬롯에 무엇을 담나 — `Agent` 만 `agent_id` 를, `Usage` 만 `show_claude`·`show_codex` 를 함께 받는다.
    enum SlotContentKind {
        Empty,
        Agent,
        AgentList,
        PresetPalette,
        Usage,
    }

    /// 비율 쓰기의 결말 — `Applied` = 바꿨다 · `Unchanged` = 자른 값이 지금 값과 같다(무변경) ·
    /// `TooSmall` = 손대지 않았다(분할이 두 쪽 모두 최소 칸 크기를 줄 만큼 크지 않거나, 그 값이면 어떤 칸의
    /// 폭·높이가 0 이 된다).
    ///
    /// 셸 안쪽 쌍둥이는 `super::types::SplitRatioOutcome` 이다(Tauri `set_split_ratio` 의 답) — 타입 이름은
    /// 다르고 variant 철자는 같다(`ThemeOrigin`↔`ThemeSource` 와 같은 관계 · 맞대는 테스트가 지킨다). 변환은
    /// 핸들러가 한다.
    // ADR-0227
    enum RatioOutcome {
        Applied,
        Unchanged,
        TooSmall,
    }

    /// 분할 하나 — a = 왼쪽/위, b = 오른쪽/아래(ADR-0140). `ratio` = a 쪽 칸의 몫.
    /// `a_slots`·`b_slots` = 그 쪽 서브트리에 든 슬롯 id(트리 전위 순).
    // ADR-0227
    struct SplitRow {
        split_id: String,
        dir: SplitDirection,
        ratio: f64,
        a_slots: Vec<String>,
        b_slots: Vec<String>,
    }

    /// 셸 설정 한 줄 — value 는 정규 문자열, is_default = 덮어쓴 값이 없어 기본값을 쓰고 있다.
    struct SettingRow {
        key: String,
        value: String,
        is_default: bool,
    }

    /// 셸 설정 키 하나의 모양.
    struct SettingSchemaRow {
        key: String,
        kind: String,
        default: String,
        choices: Option<Vec<String>>,
        min: Option<String>,
        max: Option<String>,
        description: String,
    }

    /// 창에 빈 탭을 하나 더 만들고 활성화한다.
    #[effect(Write)]
    #[since(1)]
    "tab.create" => args TabCreateArgs {
        window: String,
        name: Option<String>,
    } -> ok TabCreateOk {
        view_id: String,
    } errors [CONFLICT];

    /// 그 창의 활성 탭을 바꾼다(다른 창은 그대로).
    #[effect(Write)]
    #[since(1)]
    "tab.switch" => args TabSwitchArgs {
        window: String,
        view_id: String,
    } -> ok TabSwitchOk {} errors [CONFLICT];

    /// 탭을 닫는다 — 창의 마지막 탭이면 그 창도 닫힌다.
    #[effect(Write)]
    #[since(1)]
    "tab.close" => args TabCloseArgs {
        window: String,
        view_id: String,
    } -> ok TabCloseOk {} errors [CONFLICT];

    /// 탭 이름을 바꾼다(창은 탭 id 에서 파생하므로 안 받는다).
    #[effect(Write)]
    #[since(1)]
    "tab.rename" => args TabRenameArgs {
        view_id: String,
        name: String,
    } -> ok TabRenameOk {} errors [CONFLICT];

    /// 그 창의 탭 목록 + 활성 탭 + 버전.
    #[effect(Read)]
    #[since(1)]
    "tab.list" => args TabListArgs {
        window: String,
    } -> ok TabListOk {
        window: String,
        tabs: Vec<TabRow>,
        active: String,
        version: u64,
    } errors [CONFLICT];

    /// 빈 탭 하나를 든 새 창을 연다 — 성공 시 그 창 label.
    #[effect(Write)]
    #[since(1)]
    "window.create" => args WindowCreateArgs {}
                    -> ok   WindowCreateOk { window: String }
                    errors [CONFLICT];

    /// 창을 통째로 닫는다(main 창은 거부된다).
    #[effect(Write)]
    #[since(1)]
    "window.close" => args WindowCloseArgs {
        window: String,
    } -> ok WindowCloseOk {} errors [CONFLICT];

    /// 지금 열려 있는 창 label 전량.
    #[effect(Read)]
    #[since(1)]
    "window.list" => args WindowListArgs {}
                  -> ok   WindowListOk { windows: Vec<String> }
                  errors [CONFLICT];

    /// 슬롯을 둘로 나눈다 — 성공 시 새로 생긴 슬롯 id.
    #[effect(Write)]
    #[since(1)]
    "slot.split" => args SlotSplitArgs {
        view_id: String,
        slot_id: String,
        dir: SplitDirection,
    } -> ok SlotSplitOk {
        slot_id: String,
    } errors [CONFLICT];

    /// 슬롯을 닫는다(형제가 그 자리를 물려받는다).
    #[effect(Write)]
    #[since(1)]
    "slot.close" => args SlotCloseArgs {
        view_id: String,
        slot_id: String,
    } -> ok SlotCloseOk {} errors [CONFLICT];

    // ADR-0227
    // ADR-0140
    /// 분할(구분선) 하나의 비율을 정한다. ratio = a 쪽(왼쪽/위) 칸의 몫 — 창 전체가 아니라 그 분할이 나누는
    /// 영역 안의 몫이고, a 쪽에 칸이 여럿이면 그 묶음 전체의 몫이다. 0.3 이면 그 영역의 30% 를 왼쪽(위아래
    /// 분할이면 위)이 갖는다. split_id 는 split.list 가 준다.
    /// 범위 밖 값은 오류가 아니다: 셸이 0.01~0.99 로 자르고, 그 창의 크기를 알면 두 쪽이 각각 화면 최소 칸
    /// 크기 이상이 되게 한 번 더 자른 뒤 그 값을 적용하고 답의 ratio 로 돌려준다.
    /// outcome — Applied = 바꿨다 · Unchanged = 그렇게 자른 값이 지금 값과 같아 바꾼 것이 없다 ·
    /// TooSmall = 분할이 너무 작아서(또는 그 값이면 어떤 칸이 사라져서) 손대지 않았다. Applied 가 아니면
    /// ratio 는 손대지 않은 지금 값이다.
    #[effect(Write)]
    #[since(8)]
    "split.setRatio" => args SplitSetRatioArgs {
        view_id: String,
        split_id: String,
        ratio: f64,
    } -> ok SplitSetRatioOk {
        ratio: f64,
        outcome: RatioOutcome,
    } errors [CONFLICT];

    // ADR-0227
    // ADR-0140
    /// 그 탭의 분할(구분선) 전량 — 트리 전위 순. 행 = split_id · dir(LeftRight = 좌/우, TopBottom = 위/아래) ·
    /// ratio(= a 쪽(왼쪽/위) 칸의 몫 — 창 전체가 아니라 그 분할이 나누는 영역 안의 몫이고, a 쪽에 칸이
    /// 여럿이면 그 묶음 전체의 몫) · a_slots(a = 왼쪽/위 쪽에 든 슬롯 id) · b_slots(b = 오른쪽/아래 쪽).
    /// 슬롯 x 와 y 를 가르는 분할은 x 가 한쪽 목록, y 가 다른 쪽 목록에 든 행이다(하나뿐이다).
    /// 중첩은 목록의 포함으로 읽는다 — 한 행의 a_slots·b_slots 가 전부 다른 행의 한쪽 목록에 들면 그 행은
    /// 그쪽 영역을 다시 나누는 분할이다.
    /// 비율을 바꾸려면 그 split_id 로 split.setRatio 를 부른다.
    #[effect(Read)]
    #[since(8)]
    "split.list" => args SplitListArgs {
        view_id: String,
    } -> ok SplitListOk {
        splits: Vec<SplitRow>,
    } errors [CONFLICT];

    // ★알려진 과도기 분열 — 같은 id 가 두 표면에서 **받는 것도 주는 것도** 다르다★: 프론트 레지스트리에도
    //   `slot.popout` 이 있다(`src/commands/slotCommands.ts`). 받는 것 — 그쪽은 목적지 인자가 없어 **항상 새
    //   창**이고(포커스된 좌표만 쓴다), 여기는 `to_window` 로 기존 창도 고른다. 주는 것 — 그쪽은
    //   `{window, tab}`, 여기는 `{window, new_view_id}`. 한쪽만 맞춰 고치지 말 것 — 합류는 프론트 자체
    //   레지스트리를 은퇴시키는 후속 스텝(화면 몫 등록) 몫이고, 지금 한쪽을 바꾸면 그 스텝이 옮길 대상만
    //   늘어난다. 반대편에도 같은 메모가 붙어 있다.
    /// 슬롯의 내용을 다른 창의 새 탭으로 옮긴다(원본 슬롯은 닫힌다) — to_window 를 빼면 새 창을 연다.
    #[effect(Write)]
    #[since(2)]
    "slot.popout" => args SlotPopoutArgs {
        view_id: String,
        slot_id: String,
        to_window: Option<String>,
    } -> ok SlotPopoutOk {
        window: String,
        new_view_id: String,
    } errors [CONFLICT];

    /// 포커스를 그 슬롯으로 옮긴다(출력 라우팅은 안 바뀐다).
    #[effect(Write)]
    #[since(1)]
    "slot.focus" => args SlotFocusArgs {
        view_id: String,
        slot_id: String,
    } -> ok SlotFocusOk {} errors [CONFLICT];

    /// 이미 살아 있는 에이전트를 그 슬롯에 붙인다(새로 띄우지 않는다 — 띄우려면 agent.spawnInto).
    /// ★agent_id 는 UUID 다 — 표시 이름을 넘기지 말 것★: 이 층은 데몬에 실재하는지 확인하지 않고
    /// (레이아웃이 에이전트 상태를 모르는 것이 격리 규약이다 — ADR-0035) 받은 문자열을 그대로 슬롯에
    /// 넣는다. 그래서 이름을 넘기면 **거부되지 않고 슬롯만 비어 보인다** — 오류가 없어 원인을 못 찾는다.
    /// id 는 agent.list 가 준다(실발생 2026-08-20).
    #[effect(Write)]
    #[since(1)]
    "slot.assignAgent" => args SlotAssignAgentArgs {
        view_id: String,
        slot_id: String,
        agent_id: String,
    } -> ok SlotAssignAgentOk {} errors [CONFLICT];

    /// 슬롯이 무엇을 보여줄지 바꾼다. content=Agent 일 때만 agent_id 를 함께 준다.
    /// content=Usage 일 때만 show_claude·show_codex(그 회사를 보일지)를 줄 수 있다 — 뺀 칸은 그 슬롯이
    /// 이미 Usage 면 지금 값을 그대로 두고, 아니면 true 다.
    #[effect(Write)]
    #[since(1)]
    "layout.setSlotContent" => args LayoutSetSlotContentArgs {
        view_id: String,
        slot_id: String,
        content: SlotContentKind,
        agent_id: Option<String>,
        show_claude: Option<bool>,
        show_codex: Option<bool>,
    } -> ok LayoutSetSlotContentOk {} errors [CONFLICT];

    /// 에이전트를 새로 띄우고 그 자리에 배치한다(스폰 + 필요하면 새 탭 + 슬롯 배정).
    /// view_id 를 빼면 새 탭을 만들어 거기 넣는다 — 그 경우 slot_id 는 줄 수 없다.
    /// backend 는 `"claude"` 또는 `"codex"` — 둘 다 이 표면으로 만든다. 모르는 낱말은 스폰 전에
    /// 거부되고, 빼면 데몬이 거절한다(기본 백엔드는 없다).
    #[effect(Write)]
    #[since(1)]
    "agent.spawnInto" => args AgentSpawnIntoArgs {
        window: String,
        cwd: String,
        view_id: Option<String>,
        slot_id: Option<String>,
        backend: Option<String>,
    } -> ok AgentSpawnIntoOk {
        agent_id: String,
    } errors [CONFLICT];

    /// 공간/방향 낱말(top-left·right·up …)을 슬롯 id 로 푼다. view_id 를 빼면 그 창의 활성 탭이 대상이고,
    /// window 도 빼면 main 창이다. 그 방향에 슬롯이 없으면 slot_id 는 null 이다.
    #[effect(Read)]
    #[since(1)]
    "slot.resolveSpatial" => args SlotResolveSpatialArgs {
        token: String,
        window: Option<String>,
        view_id: Option<String>,
    } -> ok SlotResolveSpatialOk {
        slot_id: Option<String>,
    } errors [CONFLICT];

    /// 디스크의 창별 테마 파일(`<data_dir>/ui-settings.json`)을 다시 읽어 **창마다** 적용한다.
    /// 파일 모양 = `{"windows":{"main":"light"}}` — 값은 `dark`·`light`·`e-ink` 셋 중 하나이고 **창 label 별
    /// 덮어쓰기**다(창 label = `main`·`agent-tree`·`slot-popup-N`). 항목이 없는 창은 전역 테마를 쓴다.
    /// ★전역 테마는 이 파일이 아니라 settings.set theme.default 로 바꾼다★ — 파일의 `theme` 키는 읽지 않는다.
    /// 창 항목 하나가 못 쓸 값이면 그 창만 전역 값으로 접는다.
    /// 모르는 키는 무시한다(뒤에 키가 늘 자리). 파일을 고치는 것은 **호출자**이고 이 명령은 읽기만 한다.
    /// `<data_dir>` = 릴리스는 실행 파일 **폴더 아래 `data/`**(★exe 옆이 아니다★ — ADR-0134 결정 2 가 그
    /// 자리를 기각했다: 배포 파일과 섞이면 새 버전 압축을 덮어쓸 때 사용자 데이터가 함께 날아간다),
    /// 개발 빌드는 저장소 안 `.engram-dev`. 둘 다 `ENGRAM_DATA_DIR` 로 덮을 수 있다.
    /// 답의 theme 은 **전역** 값(theme.default)이고(창별 값은 각 창이 받는다), source 는 창 항목 파일을
    /// 썼는지(File) 없거나 못 써서 창 항목 없이 접혔는지(Fallback) 말한다 — 접힘은 오류가 아니고 사유는 앱 로그.
    /// ★오류가 되는 자리는 하나뿐이다★ — 알림을 못 보낸 창이 있는 경우(INTERNAL). 어느 창인지는 앱 로그.
    /// ★적용은 값 교체뿐이다★ — 슬롯을 다시 마운트하지 않는다(챗은 컴포넌트 상태라 리마운트 = 대화 영구
    /// 소실, ADR-0149).
    #[effect(Write)]
    #[since(3)]
    "ui.refresh" => args UiRefreshArgs {}
                 -> ok   UiRefreshOk { theme: String, source: ThemeOrigin }
                 errors [];

    // ADR-0265
    /// 셸 설정을 읽는다. key = 정확한 키(theme.default) 또는 점으로 끝나는 접두(chat.style.) — 빼면 전부.
    /// value 는 언제나 정규 문자열이다. is_default = 덮어쓴 값이 없어 기본값을 쓰고 있다.
    /// rev = 값이 실제로 바뀐 쓰기마다 1 씩 오르는 번호(셸을 띄울 때 0). 맞는 키가 없으면 NOT_FOUND.
    /// 어떤 키가 있고 무엇을 받는지는 settings.schema.
    #[effect(Read)]
    #[since(10)]
    "settings.get" => args SettingsGetArgs {
        key: Option<String>,
    } -> ok SettingsGetOk {
        rev: u64,
        items: Vec<SettingRow>,
    } errors [NOT_FOUND];

    /// 셸 설정 키 하나에 값을 쓴다 — 디스크에 남아 재시작을 넘긴다. 접두는 안 받는다.
    /// value 는 문자열 하나이고 받는 형식은 settings.schema 의 kind 를 따른다: choice = 선택지 낱말(대소문자
    /// 무시) · css-length = 수 + 그 키가 받는 단위(px, rem, em — 단위마다 범위가 따로) · css-number = 단위 없는 수.
    /// 답의 value 는 정규형이다(E-INK → e-ink, 15.0PX → 15px).
    /// 이미 그 값이면 changed=false 이고 rev · 알림이 없다(파일이 그 값과 다르게 적혀 있으면 파일만
    /// 바로잡는다). 기본값과 같은 값을 주면 덮어쓰기가 지워진다.
    /// theme.default 를 바꾸면 창별 테마가 없는 모든 창이 바로 그 테마로 바뀐다.
    /// 모르는 키 = NOT_FOUND · 형식·범위 위반 = INVALID_ARGUMENT(문구에 기대 형식) · 디스크에 못 썼으면
    /// INTERNAL(값은 그대로다).
    #[effect(Write)]
    #[since(10)]
    "settings.set" => args SettingsSetArgs {
        key: String,
        value: String,
    } -> ok SettingsSetOk {
        rev: u64,
        key: String,
        value: String,
        changed: bool,
    } errors [NOT_FOUND];

    /// 키 하나, 또는 점으로 끝나는 접두가 덮는 키 전부를 기본값으로 되돌린다 — key 는 필수(전체 초기화는 없다).
    /// 답의 reset = 그 선택자가 덮은 키 전부(호출 뒤 모두 기본값). 이미 다 기본값이면 rev 가 그대로다(파일에
    /// 남은 그 키들은 지운다). 맞는 키가 없으면 NOT_FOUND.
    #[effect(Write)]
    #[since(10)]
    "settings.reset" => args SettingsResetArgs {
        key: String,
    } -> ok SettingsResetOk {
        rev: u64,
        reset: Vec<String>,
    } errors [NOT_FOUND];

    /// 셸 설정 키의 모양 — kind(choice · css-length · css-number) · default · choices(choice 만) ·
    /// min · max(양 끝 포함 · css-length 는 받는 단위마다 한 값씩 ", " 로 잇는다 — "8px, 0.5rem, 0.5em") ·
    /// description. key 는 settings.get 과 같다.
    #[effect(Read)]
    #[since(10)]
    "settings.schema" => args SettingsSchemaArgs {
        key: Option<String>,
    } -> ok SettingsSchemaOk {
        items: Vec<SettingSchemaRow>,
    } errors [NOT_FOUND];

    /// 비정상 종료 뒤 「이전 화면을 복원할까」 상태를 읽는다 — 창 명령을 처음 부르기 전(그리고 쥔 label · view_id 가
    /// 안 맞을 때) 이것을 본다. crash_copy = none(물을 사본이 없다) · awaiting(답을 기다린다 — 다른 창 명령보다
    /// 먼저 주인이 정한 답을 restore.answer 로 낸다) · answered(이 실행에서 이미 답했다).
    /// 다음 넷은 awaiting 일 때만 값이고 아니면 null 이다 — saved_at_ms = 사본을 적은 시각(유닉스 밀리초) ·
    /// windows = 사본의 창 수(main + 팝아웃 — 트리 창은 세지 않는다) · tabs = 그 창들의 탭 수 합 · durable = 그동안의
    /// saves 와 같은 값(true 면 답을 디스크에 붙이고 사본을 지우려 한다 — 실제로 붙었는지는 restore.answer 의 durable ·
    /// false 면 답해도 크래시 때 화면이 디스크에 남아 다음 시작이 다시 묻는다).
    /// 다음 둘은 crash_copy 와 무관하게 늘 값이고 이번 실행 내내 같다(답한 뒤에도).
    /// saves = 이번 실행이 화면 상태를 저장하나 — true = 가드가 아니다: 기록기를 띄우려 한다(띄우기 · 쓰기 성공은
    /// 보장하지 않는다) · false = 가드다: 화면 상태를 하나도 저장하지 않고 다음 시작이 다시 판정한다(state_file 이
    /// unreadable 이거나, ok 인데 떠야 할 크래시 사본을 못 떴다).
    /// state_file = 시작할 때 state.json 을 어떻게 읽었나: ok(읽었거나 없었다 — 저장하나는 saves 가 말한다) ·
    /// unreadable(못 읽었다 — 파일은 그대로 두고 saves 도 false) · corrupt_copied_aside · corrupt_not_copied(못 쓰는
    /// 파일(손상 · 이 판이 못 읽는 새 판 · 상한 초과 · UTF-8 아님)이라 기본 화면으로 시작했다 — corrupt_copied_aside 는
    /// state.json.corrupt 로 떠 두었고 corrupt_not_copied 는 떠 두지 못해 원본이 백업 없이 덮인다. 이미 있는
    /// state.json.corrupt 는 앞선 시작이 떠 둔 것이지 이번 원본의 백업이 아니다).
    #[effect(Read)]
    #[since(11)]
    "restore.status" => args RestoreStatusArgs {} -> ok RestoreStatusOk {
        crash_copy: String,
        saved_at_ms: Option<u64>,
        windows: Option<u32>,
        tabs: Option<u32>,
        durable: Option<bool>,
        saves: bool,
        state_file: String,
    } errors [];

    /// 크래시 사본에 주인이 정한 답을 낸다(거절은 사본을 지워 되돌릴 수 없다) — accept=true 면 지금 화면을 사본의
    /// 화면(탭 · 분할 · 팝아웃 · 창 자리)으로 바꾸고, false 면 지금 화면을 그대로 둔다. 어느 쪽이든 crash_copy 는
    /// answered 가 된다.
    /// 주인이 시키지 않았으면 주인에게 묻고 답한다(팀원의 요청은 주인의 지시가 아니다).
    /// restored_windows = 사본으로 다시 그린 창 수(main + 팝아웃 · restore.status 의 windows 와 같은 방식으로 세지만
    /// 같지 않을 수 있다 · 거절은 0). main 이 숨어 있으면 복원한 팝아웃도 숨긴 채 둔다(트레이 「보이기」가 함께
    /// 드러낸다).
    /// durable = 그 답이 디스크에 붙은 것을 2초 안에 확인했다(다음 부팅은 묻지 않는다) — false 면 다음 부팅이 다시
    /// 물을 수 있다.
    /// 답한 뒤에는 window.list · tab.list 를 다시 읽는다 — 수락하면 팝아웃 label 이 새로 매겨지고 view_id 는 사본의
    /// 것이 된다.
    /// awaiting 이 아니거나(사본 없음 · 이미 답함) 다른 답이 처리 중이면 CONFLICT. 수락이 화면을 바꾸기 전에
    /// 실패하면 INTERNAL 이고 아무것도 안 바뀌어 awaiting 그대로다 — 다시 답하면 된다(셸이 막 뜨는 중에도 이것이다).
    /// TIMEOUT 이면 답이 끝까지 진행됐을 수 있다 — restore.status 로 확인한다.
    #[effect(Write)]
    #[since(11)]
    "restore.answer" => args RestoreAnswerArgs {
        accept: bool,
    } -> ok RestoreAnswerOk {
        restored_windows: u32,
        durable: bool,
    } errors [CONFLICT];
}

/// 이 표의 핸들러들이 잡는 실물 전량 — ★조립 때 주입된다★(ADR-0155 결정 5 / 규칙 T-1).
///
/// 앞 다섯의 계약(어느 것이 락 안이고 어느 것이 락 밖인가)은 적용 서비스가 소유한다 — 이 구조체는 그것을
/// **소유형으로** 들고 있을 뿐이다. `#[tauri::command]` 쪽이 빌려 쓰는 어댑터를 `Arc` 로 바꾼 것이 차이의
/// 전부이고, 그렇게 하는 이유는 표의 핸들러가 `'static` 이어야 하기 때문이다.
///
/// ★이름이 `Layout` 인데 적용 서비스 밖 포트가 넷 있다★(`ui_settings` · `settings` · `settings_events` ·
/// `restore`) — 표가 하나라 포트 묶음도 하나다(사유 = 모듈 헤더 「레이아웃 밖의 셸 명령도 여기 선다」). 그
/// 포트들은 적용 서비스를 안 거치고 자기 모듈(`crate::ui_settings` · `crate::settings` · `crate::state::restore`)만
/// 부르므로 락 규율도 그 모듈이 진다.
pub struct LayoutPorts {
    pub state: LayoutState,
    pub subs: Arc<dyn SubscriptionSync>,
    pub events: Arc<dyn LayoutEvents>,
    pub windows: Arc<dyn WindowHost>,
    pub labels: Arc<dyn LabelSource>,
    pub spawner: Arc<dyn AgentSpawner>,
    pub ui_settings: Arc<dyn UiSettingsRefresh>,
    /// 셸 설정 — 사람 경로(Tauri 설정 command)와 **같은 인스턴스**.
    pub settings: Arc<SettingsService>,
    /// 실제로 바뀐 설정 쓰기의 알림(모든 웹뷰 + `theme.default` 면 유효 테마 밀기).
    pub settings_events: Arc<dyn SettingsEvents>,
    /// 크래시 사본 복원 — 사람 경로(Tauri `restore_*` command)와 **같은 인스턴스**.
    pub restore: Arc<RestoreCoordinator>,
}

/// 셸의 명령 표를 조립한다 — ★핸들러 실물이 들어오는 유일한 자리★(규칙 T-1).
///
/// ★명령이 늘어도 조립부(`lib.rs`)는 안 바뀐다★ — 늘어나는 것은 선언 블록과 이 함수의 한 줄이다.
// ADR-0155
pub fn make_table(ports: LayoutPorts) -> CommandTable {
    let ports = Arc::new(ports);
    let mut table = CommandTable::new(COMMAND_SPECS);

    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "tab.create",
        blocking_handler(move |args: TabCreateArgs| verb_tab_create(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "tab.switch",
        blocking_handler(move |args: TabSwitchArgs| verb_tab_switch(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "tab.close",
        blocking_handler(move |args: TabCloseArgs| verb_tab_close(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "tab.rename",
        blocking_handler(move |args: TabRenameArgs| verb_tab_rename(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "tab.list",
        blocking_handler(move |args: TabListArgs| verb_tab_list(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.create",
        blocking_handler(move |_: WindowCreateArgs| verb_window_create(&p)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.close",
        blocking_handler(move |args: WindowCloseArgs| verb_window_close(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.list",
        blocking_handler(move |_: WindowListArgs| verb_window_list(&p)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "slot.split",
        blocking_handler(move |args: SlotSplitArgs| verb_slot_split(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "slot.close",
        blocking_handler(move |args: SlotCloseArgs| verb_slot_close(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "split.setRatio",
        blocking_handler(move |args: SplitSetRatioArgs| verb_split_set_ratio(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "split.list",
        blocking_handler(move |args: SplitListArgs| verb_split_list(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "slot.popout",
        blocking_handler(move |args: SlotPopoutArgs| verb_slot_popout(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "slot.focus",
        blocking_handler(move |args: SlotFocusArgs| verb_slot_focus(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "slot.assignAgent",
        blocking_handler(move |args: SlotAssignAgentArgs| verb_slot_assign_agent(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "layout.setSlotContent",
        blocking_handler(move |args: LayoutSetSlotContentArgs| verb_set_slot_content(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "slot.resolveSpatial",
        blocking_handler(move |args: SlotResolveSpatialArgs| verb_resolve_spatial(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "ui.refresh",
        blocking_handler(move |_: UiRefreshArgs| verb_ui_refresh(&p)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "settings.get",
        blocking_handler(move |args: SettingsGetArgs| verb_settings_get(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "settings.set",
        offloaded_handler(move |args: SettingsSetArgs| verb_settings_set(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "settings.reset",
        offloaded_handler(move |args: SettingsResetArgs| verb_settings_reset(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "settings.schema",
        blocking_handler(move |args: SettingsSchemaArgs| verb_settings_schema(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "restore.status",
        blocking_handler(move |_: RestoreStatusArgs| Ok(verb_restore_status(&p))),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "restore.answer",
        offloaded_handler(move |args: RestoreAnswerArgs| verb_restore_answer(&p, args)),
    );
    // 스폰이 데몬 왕복이라 `blocking_handler` 로 감쌀 수 없다(그 어댑터는 본문이 첫 poll 에서 끝까지 도는
    //   것을 계약으로 삼는다).
    plug(
        &mut table,
        "agent.spawnInto",
        Arc::new(SpawnInto {
            ports: Arc::clone(&ports),
        }),
    );

    table
}

/// ★조립 때 터뜨린다★: `insert` 가 반려하는 셋(선언 집합에 없는 이름 · 중복 · 선언 스키마가 JSON 아님)은
/// 전부 **빌드가 정하는 값**이라 런타임에 달라지지 않는다. 어느 것인지는 함께 실리는 `TableError` 가 말한다.
fn plug(table: &mut CommandTable, name: &'static str, handler: Arc<dyn CommandHandler>) {
    table
        .insert(name, handler)
        .unwrap_or_else(|e| panic!("{name} 를 표에 꽂지 못했다: {e}"));
}

/// 디스크를 기다리는 동기 본문을 **블로킹 풀**에서 돌리는 핸들러 — 인자 · 반환 직렬화는 `blocking_handler`
/// 와 같다.
///
/// ★`blocking_handler` 로 꽂지 않는 이유★: 그 어댑터는 「폴링하는 쪽이 `spawn_blocking` 뒤에서 부른다」를
/// 계약으로 삼는데, 이 표의 적용 태스크는 클라이언트 런타임의 async 워커에 그대로 뜬다(`RuntimeSpawner`).
/// 거기서 `sync_all` 을 기다리면 그 워커에 얹힌 다른 태스크가 함께 선다. `block_in_place` 는 current_thread
/// 런타임(`#[tokio::test]`)에서 패닉해 쓰지 않는다.
/// ★답보다 본문이 오래 산다★ — 기다리던 future 가 버려져도(런타임 종료) 본문은 끝까지 돈다. tokio 런타임
/// 밖에서 폴링되면 그 자리에서 돈다(막을 워커가 없다).
fn offloaded_handler<A, O, F>(run: F) -> Arc<dyn CommandHandler>
where
    F: Fn(A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    Arc::new(Offloaded {
        run: Arc::new(run),
        _types: std::marker::PhantomData,
    })
}

struct Offloaded<F, A, O> {
    run: Arc<F>,
    _types: std::marker::PhantomData<fn(A) -> O>,
}

impl<F, A, O> CommandHandler for Offloaded<F, A, O>
where
    F: Fn(A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn call(&self, args: serde_json::Value) -> CommandFuture {
        let run = Arc::clone(&self.run);
        Box::pin(async move {
            let parsed: A = serde_json::from_value(args)
                .map_err(|e| CommandError::invalid_argument(e.to_string()))?;
            let ok = match tokio::runtime::Handle::try_current() {
                Ok(runtime) => {
                    runtime
                        .spawn_blocking(move || run(parsed))
                        .await
                        .map_err(|e| {
                            // 버스 경로의 패닉 그물(`handler_panicked`)과 같은 문구 — 본문이 블로킹
                            //   풀에서 터지면 그 그물이 못 보고 여기로 온다.
                            let what = if e.is_panic() {
                                "panicked"
                            } else {
                                "was cancelled"
                            };
                            CommandError::internal(format!("command handler {what}: {e}"))
                        })??
                }
                Err(_) => run(parsed)?,
            };
            serde_json::to_value(ok).map_err(|e| CommandError::internal(e.to_string()))
        })
    }
}

// ── 동사 ────────────────────────────────────────────────────────────────────

fn verb_tab_create(ports: &LayoutPorts, args: TabCreateArgs) -> Result<TabCreateOk, CommandError> {
    let window = text("window", &args.window)?;
    let name = optional_text("name", args.name.as_deref())?;
    let view_id = apply::create_tab(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        window,
        name.map(str::to_string),
    )
    .map_err(not_applied)?;
    Ok(TabCreateOk {
        view_id: view_id.to_string(),
    })
}

fn verb_tab_switch(ports: &LayoutPorts, args: TabSwitchArgs) -> Result<TabSwitchOk, CommandError> {
    let window = text("window", &args.window)?;
    let view = uuid_arg("view_id", &args.view_id)?;
    apply::switch_tab(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        window,
        view,
    )
    .map_err(not_applied)?;
    Ok(TabSwitchOk {})
}

fn verb_tab_close(ports: &LayoutPorts, args: TabCloseArgs) -> Result<TabCloseOk, CommandError> {
    let window = text("window", &args.window)?;
    let view = uuid_arg("view_id", &args.view_id)?;
    apply::close_tab(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.windows.as_ref(),
        window,
        view,
    )
    .map_err(not_applied)?;
    Ok(TabCloseOk {})
}

fn verb_tab_rename(ports: &LayoutPorts, args: TabRenameArgs) -> Result<TabRenameOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let name = text("name", &args.name)?;
    apply::rename_tab(&ports.state, ports.events.as_ref(), view, name.to_string())
        .map_err(not_applied)?;
    Ok(TabRenameOk {})
}

fn verb_tab_list(ports: &LayoutPorts, args: TabListArgs) -> Result<TabListOk, CommandError> {
    let window = text("window", &args.window)?;
    let tabs = apply::list_tabs(&ports.state, window).map_err(not_applied)?;
    Ok(TabListOk {
        window: tabs.label,
        tabs: tabs
            .tabs
            .into_iter()
            .map(|meta| TabRow {
                id: meta.id.to_string(),
                name: meta.name,
            })
            .collect(),
        active: tabs.active.to_string(),
        version: tabs.version,
    })
}

fn verb_window_create(ports: &LayoutPorts) -> Result<WindowCreateOk, CommandError> {
    let window = apply::create_window(
        &ports.state,
        ports.subs.as_ref(),
        ports.windows.as_ref(),
        ports.labels.as_ref(),
    )
    .map_err(not_applied)?;
    Ok(WindowCreateOk { window })
}

fn verb_window_close(
    ports: &LayoutPorts,
    args: WindowCloseArgs,
) -> Result<WindowCloseOk, CommandError> {
    let window = text("window", &args.window)?;
    apply::close_window(
        &ports.state,
        ports.subs.as_ref(),
        ports.windows.as_ref(),
        window,
    )
    .map_err(not_applied)?;
    Ok(WindowCloseOk {})
}

fn verb_window_list(ports: &LayoutPorts) -> Result<WindowListOk, CommandError> {
    Ok(WindowListOk {
        windows: apply::list_windows(&ports.state).map_err(not_applied)?,
    })
}

fn verb_slot_split(ports: &LayoutPorts, args: SlotSplitArgs) -> Result<SlotSplitOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let slot = uuid_arg("slot_id", &args.slot_id)?;
    let dir = match args.dir {
        SplitDirection::LeftRight => SplitDir::LeftRight,
        SplitDirection::TopBottom => SplitDir::TopBottom,
    };
    let new_slot = apply::split_slot(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        view,
        slot,
        dir,
    )
    .map_err(not_applied)?;
    Ok(SlotSplitOk {
        slot_id: new_slot.to_string(),
    })
}

fn verb_slot_close(ports: &LayoutPorts, args: SlotCloseArgs) -> Result<SlotCloseOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let slot = uuid_arg("slot_id", &args.slot_id)?;
    apply::close_slot(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        view,
        slot,
    )
    .map_err(not_applied)?;
    Ok(SlotCloseOk {})
}

// ADR-0227
fn verb_split_set_ratio(
    ports: &LayoutPorts,
    args: SplitSetRatioArgs,
) -> Result<SplitSetRatioOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let split = uuid_arg_from("split_id", &args.split_id, "split.list")?;
    // JSON 은 NaN·±∞ 를 못 실어 보통은 역직렬화가 먼저 반려한다 — 이 검사는 그 전제가 깨질 때의 그물이다.
    //   여기서 거르지 않으면 관리자의 같은 거절이 `CONFLICT` 로 나가 「상태 탓」처럼 읽힌다.
    if !args.ratio.is_finite() {
        return Err(CommandError::invalid_argument(format!(
            "ratio must be a finite number, got {}",
            args.ratio
        )));
    }
    let applied =
        apply::set_split_ratio(&ports.state, ports.events.as_ref(), view, split, args.ratio)
            .map_err(not_applied)?;
    Ok(SplitSetRatioOk {
        ratio: applied.ratio,
        outcome: match applied.outcome {
            super::types::SplitRatioOutcome::Applied => RatioOutcome::Applied,
            super::types::SplitRatioOutcome::Unchanged => RatioOutcome::Unchanged,
            super::types::SplitRatioOutcome::TooSmall => RatioOutcome::TooSmall,
        },
    })
}

// ADR-0227
fn verb_split_list(ports: &LayoutPorts, args: SplitListArgs) -> Result<SplitListOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let splits = apply::list_splits(&ports.state, view).map_err(not_applied)?;
    let ids = |slots: Vec<Uuid>| -> Vec<String> { slots.iter().map(Uuid::to_string).collect() };
    Ok(SplitListOk {
        splits: splits
            .into_iter()
            .map(|s| SplitRow {
                split_id: s.id.to_string(),
                dir: match s.dir {
                    SplitDir::LeftRight => SplitDirection::LeftRight,
                    SplitDir::TopBottom => SplitDirection::TopBottom,
                },
                ratio: s.ratio,
                a_slots: ids(s.a_slots),
                b_slots: ids(s.b_slots),
            })
            .collect(),
    })
}

fn verb_slot_popout(
    ports: &LayoutPorts,
    args: SlotPopoutArgs,
) -> Result<SlotPopoutOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let slot = uuid_arg("slot_id", &args.slot_id)?;
    let target = optional_text("to_window", args.to_window.as_deref())?;
    let moved = apply::move_slot_to_window(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.windows.as_ref(),
        ports.labels.as_ref(),
        view,
        slot,
        target.map(str::to_string),
    )
    .map_err(not_applied)?;
    // ★`view_id` 라 부르지 않는다★ — 이 명령의 인자에 이미 `view_id`(떼어낼 **원본** 탭)가 있어서, 답의 같은
    //   이름은 반대쪽을 뜻하게 된다. 답을 그대로 되먹여 두 번 부르는 호출자는 원본 대신 방금 만든 탭을
    //   집어 엉뚱한 뷰에서 슬롯을 떼어낸다. `new_` 접두는 그 혼동을 막으면서도 값이 view id 임을 남겨
    //   `tab.rename`·`tab.switch` 의 `view_id` 에 그대로 꽂힌다. 서비스 쪽 이름은 `tab` 이다(프론트 wire).
    Ok(SlotPopoutOk {
        window: moved.window,
        new_view_id: moved.tab.to_string(),
    })
}

fn verb_slot_focus(ports: &LayoutPorts, args: SlotFocusArgs) -> Result<SlotFocusOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let slot = uuid_arg("slot_id", &args.slot_id)?;
    apply::focus_slot(&ports.state, ports.events.as_ref(), view, slot).map_err(not_applied)?;
    Ok(SlotFocusOk {})
}

fn verb_slot_assign_agent(
    ports: &LayoutPorts,
    args: SlotAssignAgentArgs,
) -> Result<SlotAssignAgentOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let slot = uuid_arg("slot_id", &args.slot_id)?;
    let agent = text("agent_id", &args.agent_id)?;
    apply::assign_agent(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        view,
        slot,
        agent.to_string(),
    )
    .map_err(not_applied)?;
    Ok(SlotAssignAgentOk {})
}

fn verb_set_slot_content(
    ports: &LayoutPorts,
    args: LayoutSetSlotContentArgs,
) -> Result<LayoutSetSlotContentOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let slot = uuid_arg("slot_id", &args.slot_id)?;
    let write = slot_content(
        args.content,
        args.agent_id.as_deref(),
        args.show_claude,
        args.show_codex,
    )?;
    match write {
        SlotContentWrite::Replace(content) => apply::set_slot_content(
            &ports.state,
            ports.subs.as_ref(),
            ports.events.as_ref(),
            view,
            slot,
            content,
        ),
        SlotContentWrite::Usage {
            show_claude,
            show_codex,
        } => apply::set_usage_slot(
            &ports.state,
            ports.subs.as_ref(),
            ports.events.as_ref(),
            view,
            slot,
            show_claude,
            show_codex,
        ),
    }
    .map_err(not_applied)?;
    Ok(LayoutSetSlotContentOk {})
}

fn verb_resolve_spatial(
    ports: &LayoutPorts,
    args: SlotResolveSpatialArgs,
) -> Result<SlotResolveSpatialOk, CommandError> {
    let token = text("token", &args.token)?;
    let window = optional_text("window", args.window.as_deref())?;
    let view = args
        .view_id
        .as_deref()
        .map(|raw| uuid_arg("view_id", raw))
        .transpose()?;
    let slot = apply::resolve_spatial(&ports.state, token, window, view).map_err(not_applied)?;
    Ok(SlotResolveSpatialOk {
        slot_id: slot.map(|id| id.to_string()),
    })
}

/// ★`blocking_handler` 안에서 디스크를 읽는다★ — 그 어댑터는 본문이 첫 poll 에서 끝까지 도는 것을 계약으로
/// 삼는다. 로컬 파일 한 칸을 한 번 읽는 것이라 여기 두지만, 읽을 것이 늘어 대기가 생기면 `SpawnInto` 쪽
/// (async 핸들러) 형태로 옮겨야 한다 — 그러지 않으면 연결 태스크가 아니라 **적용 태스크**가 그 시간만큼 묶인다.
fn verb_ui_refresh(ports: &LayoutPorts) -> Result<UiRefreshOk, CommandError> {
    // ★알림을 못 보냈으면 실패로 돌려준다★ — 값은 정해졌어도 화면에 안 닿았고, 이 명령이 하는 일은 그
    //   알림뿐이다. 성공으로 답하면 호출자는 자기 편집이 반영된 줄 안다(`source` 는 값의 출처를 말하지
    //   화면이 바뀌었는지를 말하지 않는다). `INTERNAL` 은 표가 자동으로 광고하므로 선언은 안 바뀐다.
    let loaded = ports
        .ui_settings
        .refresh()
        .map_err(CommandError::internal)?;
    Ok(UiRefreshOk {
        theme: loaded.theme.as_wire().to_string(),
        // 갈래를 여기서 만들지 않는다 — 읽기 쪽이 이미 정한 것을 wire 어휘로 옮기기만 한다.
        source: match loaded.source {
            crate::ui_settings::ThemeSource::File => ThemeOrigin::File,
            crate::ui_settings::ThemeSource::Fallback => ThemeOrigin::Fallback,
        },
    })
}

fn verb_settings_get(
    ports: &LayoutPorts,
    args: SettingsGetArgs,
) -> Result<SettingsGetOk, CommandError> {
    let key = optional_text("key", args.key.as_deref())?;
    let snapshot = ports.settings.get(key).map_err(settings_error)?;
    Ok(SettingsGetOk {
        rev: snapshot.rev,
        items: snapshot
            .items
            .into_iter()
            .map(|item| SettingRow {
                key: item.key,
                value: item.value,
                is_default: item.is_default,
            })
            .collect(),
    })
}

/// 디스크에 쓰고(`sync_all` 까지) `theme.default` 면 창 항목 파일을 읽어 민다 — 그래서 [`offloaded_handler`]
/// 로 꽂는다([`verb_settings_reset`] 도 같다).
fn verb_settings_set(
    ports: &LayoutPorts,
    args: SettingsSetArgs,
) -> Result<SettingsSetOk, CommandError> {
    let key = text("key", &args.key)?;
    let value = text("value", &args.value)?;
    let outcome = set_and_notify(&ports.settings, ports.settings_events.as_ref(), key, value)
        .map_err(settings_error)?;
    Ok(SettingsSetOk {
        rev: outcome.rev,
        key: outcome.key,
        value: outcome.value,
        changed: outcome.changed,
    })
}

fn verb_settings_reset(
    ports: &LayoutPorts,
    args: SettingsResetArgs,
) -> Result<SettingsResetOk, CommandError> {
    let key = text("key", &args.key)?;
    let outcome = reset_and_notify(&ports.settings, ports.settings_events.as_ref(), key)
        .map_err(settings_error)?;
    Ok(SettingsResetOk {
        rev: outcome.rev,
        reset: outcome.reset,
    })
}

fn verb_settings_schema(
    ports: &LayoutPorts,
    args: SettingsSchemaArgs,
) -> Result<SettingsSchemaOk, CommandError> {
    let key = optional_text("key", args.key.as_deref())?;
    let items = ports.settings.schema(key).map_err(settings_error)?;
    Ok(SettingsSchemaOk {
        items: items
            .into_iter()
            .map(|item| SettingSchemaRow {
                key: item.key,
                kind: item.kind.to_string(),
                default: item.default,
                choices: item.choices,
                min: item.min,
                max: item.max,
                description: item.description,
            })
            .collect(),
    })
}

fn verb_restore_status(ports: &LayoutPorts) -> RestoreStatusOk {
    let view = ports.restore.status();
    RestoreStatusOk {
        crash_copy: view.crash_copy.as_wire().to_string(),
        saved_at_ms: view.saved_at_ms,
        windows: view.windows,
        tabs: view.tabs,
        durable: view.durable,
        saves: view.saves,
        state_file: view.state_file.as_wire().to_string(),
    }
}

/// ★막는 본문이라 [`offloaded_handler`] 로 꽂는다★ — 수락은 창을 만들고 · 놓고 · 거두며 이벤트 루프를 기다리고,
/// 답은 기록기를 마감(2초)까지 기다린다(`RestoreCoordinator::answer`). 적용 태스크는 메인 스레드가 아니라 클라이언트
/// 런타임 워커에서 돌지만(`RuntimeSpawner`) 거기서 그대로 부르면 그 워커에 얹힌 소켓 태스크가 그만큼 선다.
fn verb_restore_answer(
    ports: &LayoutPorts,
    args: RestoreAnswerArgs,
) -> Result<RestoreAnswerOk, CommandError> {
    let reply = ports.restore.answer(args.accept).map_err(restore_error)?;
    Ok(RestoreAnswerOk {
        restored_windows: reply.restored_windows,
        durable: reply.durable,
    })
}

/// 복원 조율자의 오류 종류를 코드로 — `Conflict` = 지금 상태로는 답할 수 없다(사본 없음 · 이미 답함 · 다른 답이
/// 처리 중) · `NotReady` · `Internal` = 수락이 화면을 바꾸기 전에 실패했다(`awaiting` 그대로 · 다시 답할 수 있다).
/// ★재시도 지시는 싣지 않는다(코드의 기본 `never`)★ — 데몬이 중계하는 실패 답의 지시를 전부 `never` 로 내려
/// (`command_delivery` 의 `send_reply` · ADR-0159) 여기서 실어도 부르는 쪽에 닿지 않는다. 「곧 다시 답하라」는
/// `NotReady` 의 문구와 도움말(`prompts/engram-help.md`)이 나른다.
fn restore_error(error: AnswerError) -> CommandError {
    let message = error.to_string();
    match error {
        AnswerError::Conflict(_) => CommandError::of(ErrorCode::Conflict, message),
        AnswerError::NotReady | AnswerError::Internal(_) => CommandError::internal(message),
    }
}

/// 설정 서비스의 오류 종류를 같은 이름의 코드로 — 레이아웃 명령의 `CONFLICT` 한 갈래와 달리 서비스가 종류를
/// 이미 갈라 준다(TRD S21-storage §5-2).
fn settings_error(error: SettingsError) -> CommandError {
    match error {
        SettingsError::NotFound(message) => CommandError::not_found(message),
        SettingsError::InvalidArgument(message) => CommandError::invalid_argument(message),
        SettingsError::Internal(message) => CommandError::internal(message),
    }
}

/// `agent.spawnInto` 의 핸들러 — ★async 라 [`blocking_handler`] 를 못 쓴다★.
///
/// 본문을 `call` 안이 아니라 future 안에서 도는 것은 계약이다(마감시각이 그 형태 위에 선다 —
/// `blocking_handler` 의 같은 조항).
struct SpawnInto {
    ports: Arc<LayoutPorts>,
}

impl CommandHandler for SpawnInto {
    fn call(&self, args: serde_json::Value) -> CommandFuture {
        let ports = Arc::clone(&self.ports);
        Box::pin(async move {
            let args: AgentSpawnIntoArgs = serde_json::from_value(args)
                .map_err(|e| CommandError::invalid_argument(e.to_string()))?;
            let ok = verb_spawn_into(&ports, args).await?;
            serde_json::to_value(ok).map_err(|e| CommandError::internal(e.to_string()))
        })
    }
}

async fn verb_spawn_into(
    ports: &LayoutPorts,
    args: AgentSpawnIntoArgs,
) -> Result<AgentSpawnIntoOk, CommandError> {
    let window = text("window", &args.window)?;
    // ★`agent.new`·`agent.spawn --cwd` 와 **같은 철자 규칙**을 받는다★ — 이 문은 그 셋의 공통 등록부
    //   (`agent::commands::register`)를 안 지나므로, 여기서 안 부르면 같은 낱말이 문마다 다르게 읽힌다.
    let cwd = normalize_cwd(text("cwd", &args.cwd)?);
    let tab = args
        .view_id
        .as_deref()
        .map(|raw| uuid_arg("view_id", raw))
        .transpose()?;
    let slot = args
        .slot_id
        .as_deref()
        .map(|raw| uuid_arg("slot_id", raw))
        .transpose()?;
    // ★backend 를 여기서 정규화하지 않는다★ — 「명시된 backend 는 스폰 전에 거부」가 적용 서비스의 조항이라
    //   (ADR-0058) 빈 문자열을 부재로 접으면 그 거부가 이 입구에서만 느슨해진다.
    let agent_id = apply::spawn_into(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.spawner.as_ref(),
        window,
        tab,
        slot,
        args.backend,
        cwd,
    )
    .await
    .map_err(not_applied)?;
    Ok(AgentSpawnIntoOk { agent_id })
}

// ── 인자 검문 ────────────────────────────────────────────────────────────────

/// 적용 서비스의 실패 문구를 그대로 실어 나른다(헤더 「적용 실패는 코드 하나로 나간다」).
fn not_applied(detail: String) -> CommandError {
    CommandError::of(ErrorCode::Conflict, detail)
}

/// ★공백만 있는 값은 부재로 접지 않고 반려한다★ — 셸에서 미설정 변수가 빈 인자로 펼쳐지는 형태
/// (`--window "$UNSET"`)가 현실적으로 들어오고, 그것을 「안 준 것」으로 접으면 오타 하나가 다른 창을
/// 건드린다(agent 쪽 `agent.*` 의 같은 조항).
fn text<'a>(field: &str, given: &'a str) -> Result<&'a str, CommandError> {
    if given.trim().is_empty() {
        return Err(CommandError::invalid_argument(format!(
            "{field} needs a real value — an empty argument is usually an unset shell variable"
        )));
    }
    Ok(given)
}

fn optional_text<'a>(field: &str, given: Option<&'a str>) -> Result<Option<&'a str>, CommandError> {
    match given {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Err(CommandError::invalid_argument(format!(
            "{field} must either carry a value or be left out entirely — an empty argument is usually an unset shell variable"
        ))),
        Some(value) => Ok(Some(value)),
    }
}

/// ★형식 불량은 적용 전에 반려한다★ — 파싱에 실패한 id 로 적용을 부르면 「없는 id」와 「형식이 깨진 id」가
/// 같은 답을 받아, 호출자는 멀쩡한 탭을 지웠나 의심하며 목록부터 다시 뒤진다.
fn uuid_arg(field: &str, given: &str) -> Result<Uuid, CommandError> {
    uuid_arg_from(field, given, "tab.list / slot.resolveSpatial")
}

/// `source` = 그 id 를 돌려주는 명령 — 반려 문구가 호출자를 거기로 안내한다.
fn uuid_arg_from(field: &str, given: &str, source: &str) -> Result<Uuid, CommandError> {
    Uuid::parse_str(given.trim()).map_err(|_| {
        CommandError::invalid_argument(format!(
            "{field} must be a UUID (the id shown by {source}), got {given:?}"
        ))
    })
}

/// 버스 한 번이 슬롯에 쓰는 것. `Usage` 는 뺀 칸을 슬롯의 지금 값으로 채워야 해서 ViewManager 락 안에서야
/// [`SlotContent`] 가 된다(`apply::set_usage_slot`).
enum SlotContentWrite {
    Replace(SlotContent),
    Usage {
        show_claude: Option<bool>,
        show_codex: Option<bool>,
    },
}

/// 태그 + 곁칸(`agent_id` · `show_claude`·`show_codex`) → 슬롯에 쓸 것.
///
/// ★어긋난 조합을 조용히 고치지 않는다★: `Agent` 인데 id 가 없으면 빈 슬롯이 되고, `Empty` 인데 id 가
/// 붙어 있으면 호출자는 그 에이전트가 붙은 줄 안다. `Usage` 가 아닌데 `show_*` 가 붙어 있으면 호출자는 그
/// 회사를 켠 줄 안다. 셋 다 반려한다.
fn slot_content(
    kind: SlotContentKind,
    agent_id: Option<&str>,
    show_claude: Option<bool>,
    show_codex: Option<bool>,
) -> Result<SlotContentWrite, CommandError> {
    let agent_id = optional_text("agent_id", agent_id)?;
    let shows = show_claude.is_some() || show_codex.is_some();
    let replace = match (kind, agent_id) {
        (SlotContentKind::Agent, Some(id)) => SlotContent::Agent {
            agent_id: id.to_string(),
        },
        (SlotContentKind::Agent, None) => {
            return Err(CommandError::invalid_argument(
                "content=Agent needs agent_id — use slot.assignAgent if you only want to attach a running agent",
            ))
        }
        (_, Some(_)) => {
            return Err(CommandError::invalid_argument(
                "agent_id only applies to content=Agent, so drop this field",
            ))
        }
        (SlotContentKind::Usage, None) => {
            return Ok(SlotContentWrite::Usage {
                show_claude,
                show_codex,
            })
        }
        (SlotContentKind::Empty, None) => SlotContent::Empty,
        (SlotContentKind::AgentList, None) => SlotContent::AgentList,
        (SlotContentKind::PresetPalette, None) => SlotContent::PresetPalette,
    };
    if shows {
        return Err(CommandError::invalid_argument(
            "show_claude/show_codex only apply to content=Usage, so drop these fields",
        ));
    }
    Ok(SlotContentWrite::Replace(replace))
}

// ★단위 테스트가 이 파일에 없는 것은 「둘 수 없어서」가 아니라 「아직 안 두어서」다★. 이 패키지의
//   `#[cfg(test)]` 단언은 `cargo test -p engram-dashboard --test lib_unit` 으로 돈다(현황 = CLAUDE.md
//   「빌드·검증 명령」의 그 줄 · 그 타깃을 세운 결정 = ADR-0174). 지금 단언은 `tests/layout_commands.rs` 에
//   있고 배치는 건드리지 않았다 — 여기 단위 테스트를 둘지는 열린 선택이다.
