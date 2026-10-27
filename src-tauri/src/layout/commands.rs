//! 셸이 주인인 명령 표 — `window`/`tab`/`slot` 은 **선언과 본문이 적용 서비스 옆**에 산다(ADR-0155 결정 1).
//!
//! 사람 클릭(`commands/layout.rs` 의 `#[tauri::command]`)과 중계된 LLM 호출(인바운드 수신기)이 **같은
//! 적용 서비스**(`super::apply`)에 떨어진다 — 이 파일은 그 서비스로 가는 **두 번째 껍데기**이지 두 번째
//! 제어 표면이 아니다(ADR-0081 결정 3 · 「거부한 대안」의 `ViewManager` 직접 호출을 여기서도 하지 않는다).
//!
//! ★레이아웃 밖의 셸 명령도 여기 선다★ — `window.setTheme` · `window.getTheme` · `settings.*` · `restore.*` 는 적용
//! 서비스를 안 거치지만 **셸이 주인인 이름**이라 이 표에 든다. 두 번째 선언 블록을 만들면 등록 패킷·세대 번호·중복
//! 검사가 표마다 갈린다(매크로 제약도 「모듈 하나에 블록 하나」다). 그 대신 포트는 자기 것을 따로 든다
//! ([`LayoutPorts::themes`] · [`LayoutPorts::settings`] · [`LayoutPorts::settings_events`] ·
//! [`LayoutPorts::restore`]).
//!
//! 진입점: [`make_table`](fn.make_table.html)(조립) · [`LayoutPorts`](struct.LayoutPorts.html)(주입 seam).
//!
//! ## ★선언이 서비스와 갈리는 세 자리(알고 남긴 것)★
//! 선언 매크로의 타입 알파벳이 `Uuid`·중첩 enum·재귀 타입을 못 실어서 생긴 번역이다 — 매크로 제약이
//! 원인이고 계약을 새로 만든 것이 아니다.
//! - **id 는 전부 `String`** 이고 핸들러가 파싱한다(형식 불량 = `INVALID_ARGUMENT`).
//! - **`SlotContent` 는 태그 + `agent_id` · `assign` · `show_claude` · `show_codex` 곁칸으로 펴서 받는다** — 매크로의
//!   enum 은 필드 없는 variant 만 싣는다. 조합 검사는 [`slot_content`] 가 한다.
//! - **`get_view` 는 선언에 없다** — 반환이 `ViewSnapshot`(재귀 `LayoutNode`)이고 매크로가 재귀 타입에서
//!   컴파일 에러로 멈춘다. 그래서 v1 조회는 `tab.list`·`window.list`·`slot.resolveSpatial` 셋이다.
//!
//! ## ★적용 실패의 코드★ — 레이아웃 명령만
//! `window.*Theme` · `settings.*` · `restore.answer` 는 서비스가 타입드 오류를 내므로 종류대로 옮긴다
//! ([`theme_error`] · [`settings_error`] · [`restore_error`]). 아래는 적용 서비스 몫이다.
//!
//! - **id 를 받는 동사**(적용 서비스의 id 판을 부르는 것 — `tab.create` · `split.setRatio` 등)는 실패를 `String` 으로만
//!   받아 종류를 가를 재료가 없다. 문구로 코드를 합성하는 것은 금지다(TRD §4-⑦ — `message` 로 기계 분기하지 않는다. CLI 의
//!   문자열 패턴매칭을 끝내려고 둔 계약이 바로 그것이다). 그래서 **모든 실패에 참인 코드**를 고른다: `CONFLICT` =
//!   「지금 상태로는 그 요청을 적용할 수 없다」. `NOT_FOUND` 는 main 창 거부에 거짓이고 `INTERNAL` 은 오타 난 id 에
//!   거짓이다. 사유는 문구가 그대로 나른다.
//! - **대상을 받는 도킹 동사**(`tab.move` · `tab.dock` · `tab.detach`)는 적용 서비스의 대상판이 타입드 오류를 내므로
//!   종류대로 옮긴다(TRD S28 §5-3) — 대응의 정본 = [`dock_target_error`] · [`locate_error`]. 조회 `layout.get` 은 적용
//!   실패가 없고 창 인자 오류([`window_arg_error`])만 낸다.
//! - **넓힌 칸 · 탭 동사**(`slot.*` · `layout.setSlotContent` · `tab.switch` · `tab.close` · `tab.rename` — TRD S28 §7)도
//!   대상판을 부르고 그 거절을 [`target_error`] 로 옮긴다. ★단 id 로 준 탭 · 칸이 없으면 넓히기 전 id 판이 내던 그대로다★
//!   (`CONFLICT` + 관리자 문구 — 옛 호출자가 보던 답 · [`id_miss`]). 에이전트 · 낱말로 고른 것은 [`locate_error`] 다.
//!
//! ## ★에이전트를 고르는 인자는 id 로만 들어온다★
//! 그 칸(`tab_of` · `slot_of` · `assign` 등)은 선언에 `#[agent_ref]` 로 표시한다 — 데몬이 중계 전에 자기 해석기로 이름을
//! id 로 바꾸고, 풀리지 않으면(없음 = NOT_FOUND · 겹침 = CONFLICT) 중계하지 않고 그 거절로 답한다(ADR-0308). 그래서 이
//! 표의 처리기는 그 칸에서 id 만 보고, UUID 가 아니면 데몬을 거치지 않은 호출이라 INVALID_ARGUMENT 다([`agent_arg`]).
//! 셸은 이름을 풀지 않는다 — 셸의 표시 이름 캐시는 보여 주기 몫이다(ADR-0132).
// ADR-0155
// ADR-0081
// ADR-0307
// ADR-0308

use std::sync::Arc;

use uuid::Uuid;

use engram_dashboard_base::path::normalize_spelling;
use engram_dashboard_command::{
    blocking_handler, declare_commands, CommandError, CommandFuture, CommandHandler, CommandTable,
    ErrorCode, RetryMode,
};

use super::apply::{
    self, DockOutcome, SpawnAtError, TabBarSpot, TargetDest, TargetDockError, TargetDockRequest,
    TargetError, WindowPlace,
};
use super::dock;
use super::manager::{DockError, DockMove, FocusPolicy, LayoutError};
use super::rows::{self, RowFilter};
use super::spatial::SpatialToken;
use super::target::{self, LocateError, Place, Target};
use super::tree;
use super::window_arg::{self, WindowArgError};
use super::window_ctl::{
    self, Activation, Applied, CloseOutcome, WindowAction, WindowControl, WindowCtlError,
};
use super::{
    AgentSpawner, LabelSource, LayoutEvents, LayoutState, SlotContent, SplitDir, SubscriptionSync,
    WindowHost, MAIN_WINDOW_LABEL,
};
use crate::settings::{
    reset_and_notify, set_and_notify, SettingsError, SettingsEvents, SettingsService,
};
use crate::state::restore::{AnswerError, RestoreCoordinator};
use crate::theme::{ThemeControl, ThemeError, UiTheme, WindowTheme};

// ★이름은 프론트 레지스트리가 **오늘 등록한 id** 를 그대로 쓴다★: `tab.create`·`slot.focus`·`slot.popout`·
//   `layout.setSlotContent`·`agent.spawnInto` 는 `src/commands/*Commands.ts` 에 실재하는 id 다(실측
//   2026-08-17). 여기서 다른 철자를 지으면 같은 동작에 이름이 둘이 되고, 화면 몫 등록(TRD §6 Step 4 —
//   그 id 를 바꾸지 않는다고 적은 자리)이 그 둘을 다 지고 간다.
// ★프론트에 짝이 없어 새로 짓는 이름도 있다★(아래는 예이고 전부가 아니다) — `tab.list`·`window.list`(조회는
//   프론트 id 가 없다) · `slot.split`(프론트는 방향을 이름에 박아 `slot.split.topBottom`/
//   `slot.split.leftRight` 둘로 두지만, 버스에서는 방향이 **인자**다 — 그래야 호출자가 방향을 값으로 고른다) ·
//   `window.setTheme`·`window.getTheme`(화면에는 테마를 바꾸는 UI 가 없다 — TRD S21-storage §10 F7) ·
//   `split.setRatio`·`split.list`(화면의 구분선 드래그는 Tauri `set_split_ratio` 를 직접 부르고 레지스트리에
//   이름을 싣지 않는다 — ADR-0227).
// ★세대 21 = LLM 레이아웃 명령의 세대★(TRD S28 §4 · §7 · ADR-0307) — 에이전트 이름 · 공간 낱말로 탭 · 칸을 고르고 바꾼
//   뒤의 배치를 답에 싣는다. 이름이 넷 늘었고(`tab.move` · `tab.dock` · `tab.detach` · `layout.get`), 칸 동사(`slot.focus` ·
//   `slot.close` · `slot.split` · `slot.popout` · `slot.assignAgent` · `layout.setSlotContent`)가 `slot_of` · `at` · `window`
//   를, 탭 동사(`tab.switch` · `tab.close` · `tab.rename`)가 `tab_of` 를, `agent.spawnInto` 가 `at` 을 받고 id 인자 · 탭
//   동사와 `agent.spawnInto` 의 `window` 가 선택이 됐다. 넣을 에이전트는 `assign` 도 받고 데몬 명부에 없으면 거절한다. 그
//   동사들의 답에 `layout` 이 붙고 오류 선언에 `NOT_FOUND` 가 들었다. 에이전트를 고르는 칸(옛 `agent_id` 포함)은
//   `#[agent_ref]` 로 표시해 데몬이 중계 전에 id 로 바꾼다(ADR-0308 — 등록 패킷의 `agent_args`). 한 커밋에 들어 세대 하나로
//   묶었다.
// ★세대 20 = 창 상태 명령 여섯(`window.minimize` · `window.maximize` · `window.unmaximize` · `window.raise` ·
//   `window.hideAll` · `window.showAll`)이 들고 `window.close` 가 main 을 받는 세대★(TRD S26-window-chrome §6-1 · §6-6) —
//   `window.close` 는 `if_empty` 를 받고 답이 `outcome` 을 싣는다(main = 모든 창 숨기기 · 지금까지의 `CONFLICT` 대신).
//   `window.list` 의 답은 `windows` 를 그대로 두고 `details` 를 더했다.
// ★세대 19 = 창 인자가 영속 id(window_id)도 받고 숫자만인 인자(창 번호)를 `INVALID_ARGUMENT` 로 거절하는 세대★(TRD
//   S26-window-chrome §4-5 · §6-6) — 이름 · 인자 타입은 그대로고 창 인자를 받는 동사의 받는 어휘 · 오류 어휘(앞 실행의
//   label = `CONFLICT`)와 summary 가 바뀌었다. `window.list` 의 summary 도 번호를 다시 쓰는 규칙을 말하게 됐다. 같은
//   세대에 `window.create` · `slot.popout` 의 답이 `window_id` · `number` 를 더했다(§3-2 · §5-3 — §6-6 의 「같은 PR 이면
//   한 번」으로 두 단위를 한 세대에 묶었다).
// ★세대 18 = `tab.list` 의 탭 칸에 `named` 가 붙고 `name` 이 보이는 이름(정한 이름 · 아니면 자동 이름)인 세대★(TRD
//   S26-window-chrome §3-2 · §6-6) — 이름은 그대로고 답 모양이 바뀌었다. 같은 세대에 `tab.create` · `tab.rename` 의
//   summary 가 자동 이름과 정한 이름의 굳음을 말하게 됐고 두 명령이 `name` 앞뒤 공백을 깎는다.
// ★세대 17 = `settings.set`·`settings.reset` 이 설정 파일을 지금 못 읽은 쓰기를 `INTERNAL` 대신 `CONFLICT` 로 답하는
//   세대★(ADR-0291 R17 · 사용자 결정 2026-10-11 — 대개 잠깐 쥔 잠김이라 조금 뒤 다시 하면 된다) — 이름 · 타입 · 오류
//   선언은 그대로고 그 답 어휘와 summary 가 바뀌었다.
// ★세대 16 = `restore.status` 의 `state_file` 이 `newer` 를 싣고 `settings.set`·`settings.reset` 이 `CONFLICT` 를
//   선언한 세대★(ADR-0291 — 새 판이 쓴 파일은 덮지 않는다) — 이름은 그대로고 답 어휘와 오류 선언이 바뀌었다.
// ★세대 15 = 창 테마 명령 둘(`window.getTheme`·`window.setTheme`)이 `agent-tree` 를 받지 않는 세대★(ADR-0225 —
//   그 label 도 모르는 창이다) — 이름도 타입도 그대로고 둘의 `window` 칸이 받는 어휘가 줄었다. summary 가 바뀐 것은
//   `window.getTheme`(`agent-tree` 를 적던 유일한 summary)과 `restore.status`(트리 창 문구가 빠졌다)다.
// ★세대 14 = 창 테마 명령 둘(`window.setTheme`·`window.getTheme`)이 들고 `ui.refresh` 가 빠진 세대★(TRD
//   S21-storage §5-6 · §5-7).
// ★세대 13 = `restore.status` 의 답에 `saves` 가 붙은 세대★(TRD S21-storage §6-5 — 가드 ⅱ 안내(ADR-0276)를 나르는
//   칸이고 칸 자체는 구현 · 세션 판단) — 이름은 그대로고 답 모양이 바뀌었다.
// ★세대 12 = `restore.status` 의 답에 `durable` · `state_file` 이 붙은 세대★(TRD S21-storage §6-5 · §6-7) — 이름은
//   그대로고 답 모양이 바뀌었다.
// ★세대 11 = 크래시 사본 명령 둘(`restore.status`·`restore.answer`)이 든 세대★(TRD S21-storage §6-7).
// ★세대 10 = 셸 설정 명령 넷(`settings.get`·`settings.set`·`settings.reset`·`settings.schema`)이 들고
//   `ui.refresh` 가 전역 테마를 파일이 아니라 설정 `theme.default` 에서 읽는 세대★(TRD S21-storage §5-4 ·
//   §5-6) — 답 모양은 그대로고 `theme` 의 출처와 summary 가 바뀌었다(그 명령은 세대 14 에서 빠졌다).
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
    catalog_version: 21;

    /// 탭 바 한 칸 — `name` = 보이는 이름 · `named` = 이름을 정했나(거짓이면 `name` 은 포커스 칸 내용을 따라 바뀐다).
    struct TabRow {
        id: String,
        name: String,
        named: bool,
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
    /// 다르고 variant 철자는 같다(`SplitDirection`↔`SplitDir` 과 같은 관계 · 맞대는 테스트가 지킨다). 변환은
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

    /// 창 하나 — window_id = 재시작을 넘어 같은 창(창 인자로 넘길 값) · window = label(그 실행 안에서만) · number = 보이는
    /// 창 번호(다시 쓰인다 — 창 인자로 받지 않는다) · main = main 창인가 · visible · minimized · maximized = 그 순간의 OS
    /// 상태 · focused = 그 창이 OS 전경 창인가. OS 창을 못 읽은 창은 넷 다 false 다.
    struct WindowRow {
        window_id: String,
        window: String,
        number: u32,
        main: bool,
        visible: bool,
        minimized: bool,
        maximized: bool,
        focused: bool,
    }

    /// window.close 의 결말 — Closed = 팝아웃을 닫았다 · HiddenAll = main 이라 닫지 않고 모든 창을 숨겼다 · Kept =
    /// if_empty 인데 그 창에 탭이 있거나 이미 닫혀 그대로 뒀다.
    enum WindowCloseOutcome {
        Closed,
        HiddenAll,
        Kept,
    }

    /// 칸에 든 것의 종류 — Unknown = 이 판이 모르는 내용(더 새 판이 저장한 칸). 셸 안쪽 쌍둥이는 `rows::SlotKind` 다.
    // ADR-0307
    enum SlotKind {
        Empty,
        Agent,
        AgentList,
        PresetPalette,
        Usage,
        Unknown,
    }

    /// 붙는 탭이 놓일 쪽 — 붙일 칸(또는 탭 전체)의 그 변에 붙는다. 셸 안쪽 쌍둥이는 `dock::DockSide` 다(매크로가 블록
    /// 밖 enum 을 못 실어 같은 철자로 하나 더 둔다).
    // ADR-0307
    enum DockSide {
        Left,
        Right,
        Top,
        Bottom,
    }

    /// 칸 하나의 줄 — 칸 뜻은 layout.get 의 summary. id 넷(window_id · tab · slot_id · agent_id)은 layout.get 의
    /// ids = true 에서만 싣는다 — 빠진 칸은 답에서 아예 빠진다(null 이 아니다).
    // ADR-0307
    struct LayoutRow {
        window: String,
        window_number: u32,
        tab_index: u32,
        tab_name: String,
        tab_active: bool,
        content: SlotKind,
        agent: Option<String>,
        focused: bool,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        window_id: Option<String>,
        tab: Option<String>,
        slot_id: Option<String>,
        agent_id: Option<String>,
    }

    /// 창에 빈 탭을 하나 더 만들고 활성화한다. name 을 빼면 자동 이름 — 그 탭의 포커스 칸(에이전트면 그 이름)을
    /// 따라 바뀐다(tab.list 의 named = false). name 을 주면 정한 이름이다(앞뒤 공백은 깎는다).
    /// window = window.list 의 window_id(재시작을 넘어 같은 창) 또는 같은 실행 안 label. 번호는 받지 않는다(다시
    /// 쓰인다 — INVALID_ARGUMENT). label 은 그 실행 안에서만 맞는다.
    #[effect(Write)]
    #[since(1)]
    "tab.create" => args TabCreateArgs {
        window: String,
        name: Option<String>,
    } -> ok TabCreateOk {
        view_id: String,
    } errors [CONFLICT];

    // ADR-0307
    /// 그 창의 보이는 탭을 바꾼다(다른 창은 그대로). 탭 = view_id | tab_of(그 에이전트가 든 탭 — 이름 또는 id) 중 하나.
    /// window 는 빼도 된다(탭에서 안다) — 주면 그 창의 탭이어야 하고 window_id 또는 같은 실행 안 label 이다(번호는 받지
    /// 않는다).
    /// 오류: tab_of 가 어느 칸에도 없거나 그 이름이 없으면 NOT_FOUND · 여러 탭에 떠 있으면 CONFLICT(문구 끝 candidates 의
    /// view_id= 값을 view_id 로 다시) · 그 이름의 에이전트가 둘 이상이면 CONFLICT(candidates = 에이전트 id — tab_of 로 다시) ·
    /// 없는 view_id 는 CONFLICT.
    #[effect(Write)]
    #[since(1)]
    "tab.switch" => args TabSwitchArgs {
        window: Option<String>,
        view_id: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
    } -> ok TabSwitchOk {} errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// 탭을 닫는다 — 창의 마지막 탭이면 그 창도 닫힌다. 탭 = view_id | tab_of(그 에이전트가 든 탭 — 이름 또는 id) 중
    /// 하나. window 는 빼도 된다(탭에서 안다) — 주면 그 창의 탭이어야 하고 window_id 또는 같은 실행 안 label 이다(번호는
    /// 받지 않는다).
    /// 답 = layout(그 창에 이제 보이는 탭의 칸 줄 — layout.get rows 모양 · id 없음 · 창째 닫혔으면 빈 목록).
    /// 오류: tab_of 가 어느 칸에도 없거나 그 이름이 없으면 NOT_FOUND · 여러 탭에 떠 있으면 CONFLICT(문구 끝 candidates 의
    /// view_id= 값을 view_id 로 다시) · 그 이름의 에이전트가 둘 이상이면 CONFLICT(candidates = 에이전트 id — tab_of 로 다시) ·
    /// 없는 view_id 는 CONFLICT.
    #[effect(Write)]
    #[since(1)]
    "tab.close" => args TabCloseArgs {
        window: Option<String>,
        view_id: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
    } -> ok TabCloseOk {
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// 탭 이름을 바꾼다(앞뒤 공백은 깎는다 · 창은 탭에서 안다). 탭 = view_id | tab_of(그 에이전트가 든 탭 — 이름 또는 id)
    /// 중 하나. 그 뒤 이름은 굳는다 — 지금 보이는 자동 이름과 같은 글자여도 포커스 칸 · 에이전트 이름을 더는 따라가지
    /// 않고, 자동 이름으로 되돌리는 명령은 없다.
    /// 오류: tab_of 가 어느 칸에도 없거나 그 이름이 없으면 NOT_FOUND · 여러 탭에 떠 있으면 CONFLICT(문구 끝 candidates 의
    /// view_id= 값을 view_id 로 다시) · 그 이름의 에이전트가 둘 이상이면 CONFLICT(candidates = 에이전트 id — tab_of 로 다시) ·
    /// 없는 view_id 는 CONFLICT.
    #[effect(Write)]
    #[since(1)]
    "tab.rename" => args TabRenameArgs {
        view_id: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
        name: String,
    } -> ok TabRenameOk {} errors [NOT_FOUND, CONFLICT];

    /// 그 창의 탭 목록 + 활성 탭 + 버전. 답의 window 는 그 창의 label 이다.
    /// window = window.list 의 window_id(재시작을 넘어 같은 창) 또는 같은 실행 안 label. 번호는 받지 않는다(다시
    /// 쓰인다 — INVALID_ARGUMENT). label 은 그 실행 안에서만 맞는다.
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

    /// 빈 탭 하나를 든 새 창을 연다. 답 = window(label — 그 실행 안에서만 맞는다) · window_id(재시작을 넘어 같은
    /// 창 — 창 인자로 넘길 값) · number(보이는 창 번호 — 닫힌 창의 번호를 다시 쓰므로 창 인자로 받지 않는다).
    #[effect(Write)]
    #[since(1)]
    "window.create" => args WindowCreateArgs {}
                    -> ok   WindowCreateOk { window: String, window_id: String, number: u32 }
                    errors [CONFLICT];

    /// 창을 닫는다 — 팝아웃은 탭째 닫고(outcome = Closed), main 은 닫지 않고 모든 창을 숨긴다(HiddenAll — window.hideAll
    /// 과 같다 · window.showAll 이나 트레이 「보이기」가 되살린다). if_empty = true 면 window 는 그 창의 window_id 여야 하고,
    /// 그 창에 탭이 들어와 있거나 이미 닫혔으면 닫지 않는다(Kept — 오류가 아니다). window 는 tab.create 와 같다(window_id
    /// 또는 같은 실행 안 label · 번호는 받지 않는다 — if_empty 여도 INVALID_ARGUMENT).
    #[effect(Write)]
    #[since(1)]
    "window.close" => args WindowCloseArgs {
        window: String,
        if_empty: Option<bool>,
    } -> ok WindowCloseOk {
        outcome: WindowCloseOutcome,
    } errors [CONFLICT];

    /// 지금 열려 있는 창. windows = label 전량 · details = 창마다 window_id · label · 번호 · main 인가 · OS 상태(번호 순 ·
    /// 칸 뜻은 행 모양). 창 번호는 비어 있는 가장 작은 수를 다시 쓴다 — 닫히는 창이 아직 없어지지 않았으면 그 번호는 비지
    /// 않는다(그 창은 목록에 없다). 창을 가리킬 때는 번호가 아니라 window_id 를 넘긴다.
    #[effect(Read)]
    #[since(1)]
    "window.list" => args WindowListArgs {}
                  -> ok   WindowListOk { windows: Vec<String>, details: Vec<WindowRow> }
                  errors [CONFLICT];

    /// 창을 최소화한다(이미 최소화면 무동작). 보이지 않는 창은 CONFLICT — 숨은 창은 먼저 window.raise 또는
    /// window.showAll, 아직 첫 화면 전인 창은 곧 보인다. window 는 tab.create 와 같다(window_id 또는 같은 실행 안 label ·
    /// 번호는 받지 않는다).
    #[effect(Write)]
    #[since(20)]
    "window.minimize" => args WindowMinimizeArgs {
        window: String,
    } -> ok WindowMinimizeOk {} errors [CONFLICT];

    /// 창을 최대화한다(이미 최대화면 무동작). 보이지 않는 창(숨김 · 아직 첫 화면 전)은 화면을 건드리지 않고 미뤄 뒀다가
    /// 그 창이 보일 때 최대화로 보인다 — deferred = true. window 는 tab.create 와 같다(window_id 또는 같은 실행 안 label ·
    /// 번호는 받지 않는다).
    #[effect(Write)]
    #[since(20)]
    "window.maximize" => args WindowMaximizeArgs {
        window: String,
    } -> ok WindowMaximizeOk {
        deferred: bool,
    } errors [CONFLICT];

    /// 최대화된 창을 원래 크기로 되돌린다(최대화가 아니면 무동작 · 보이는 최소화된 창도 무동작이다 — 먼저 window.raise). 보이지
    /// 않는 창(숨김 · 아직 첫 화면 전)은 화면을 건드리지 않고 미뤄 뒀다가 그 창이 보일 때 원래 크기로 보인다 — deferred =
    /// true. window 는 tab.create 와 같다(window_id 또는 같은 실행 안 label · 번호는 받지 않는다).
    #[effect(Write)]
    #[since(20)]
    "window.unmaximize" => args WindowUnmaximizeArgs {
        window: String,
    } -> ok WindowUnmaximizeOk {
        deferred: bool,
    } errors [CONFLICT];

    /// 창을 앞으로 낸다 — 숨어 있으면 모든 창을 보이고(shown_all = true), 최소화면 펴고, 맨 위로 올린 뒤 전경을 청한다.
    /// 빼앗지 않는다: 다른 앱이 앞이면 맨 위로만 올라 focused = false(focused 는 그 뒤 그 창이 OS 전경 창인가 그대로다).
    /// 아직 첫 화면 전인 창은 손대지 않는다(곧 보인다 — pending = true). window 는 tab.create 와 같다(window_id 또는 같은
    /// 실행 안 label · 번호는 받지 않는다).
    #[effect(Write)]
    #[since(20)]
    "window.raise" => args WindowRaiseArgs {
        window: String,
    } -> ok WindowRaiseOk {
        shown_all: bool,
        focused: bool,
        pending: bool,
    } errors [CONFLICT];

    /// 모든 창을 숨긴다(트레이로 — main 의 닫기와 같다). 되살리려면 window.showAll 또는 window.raise.
    #[effect(Write)]
    #[since(20)]
    "window.hideAll" => args WindowHideAllArgs {} -> ok WindowHideAllOk {} errors [];

    /// 숨은 창을 모두 보인다 — 포커스를 빼앗지 않는다(다른 앱이 앞이면 보이기만 한다). 아직 첫 화면 전인 창은 건너뛴다(곧
    /// 저절로 보인다). focused = 그 뒤 main 이 OS 전경 창인가.
    #[effect(Write)]
    #[since(20)]
    "window.showAll" => args WindowShowAllArgs {} -> ok WindowShowAllOk {
        focused: bool,
    } errors [];

    // ADR-0265
    /// 창 하나의 테마를 읽는다. window = window.list 의 window_id(재시작을 넘어 같은 창) 또는 같은 실행 안 label —
    /// 번호는 받지 않는다(다시 쓰인다 — INVALID_ARGUMENT).
    /// theme = 그 창에만 정한 테마(dark · light · e-ink) — null 이면 정하지 않아 전역 테마(settings 의
    /// theme.default)를 따른다. effective = 지금 그 창에 칠하는 테마(theme 가 null 이면 theme.default).
    /// 없는 창 = CONFLICT.
    #[effect(Read)]
    #[since(14)]
    "window.getTheme" => args WindowGetThemeArgs {
        window: String,
    } -> ok WindowGetThemeOk {
        theme: Option<String>,
        effective: String,
    } errors [CONFLICT];

    // ADR-0265
    /// 창 하나에만 테마를 정한다 — theme = dark · light · e-ink(대소문자 무시), 또는 null(그 창의 테마를 지워 전역
    /// theme.default 를 따르게 한다). theme 칸은 빼면 INVALID_ARGUMENT 다 — 지우려면 null 을 준다. window 는
    /// window.getTheme 과 같다(window_id 또는 같은 실행 안 label · 번호는 받지 않는다).
    /// 화면 상태에 그 창과 함께 저장돼 재시작을 넘기고(창을 닫으면 함께 사라진다 · restore.status 의 saves 가 false 인
    /// 실행은 저장하지 않는다) 그 창에 새 테마를 보낸다 — 슬롯은 다시 마운트하지 않는다(대화가 그대로다). 모든 창을
    /// 한꺼번에 바꾸려면 settings.set theme.default.
    /// 답 = 쓴 뒤의 theme · effective(window.getTheme 과 같은 모양) — 성공은 그 창에 보냈다는 뜻이지 칠한 것을 확인한
    /// 것이 아니다(아직 안 뜬 창은 뜰 때 받는다). 없는 창 = CONFLICT · 모르는 테마 = INVALID_ARGUMENT · 정했지만
    /// 그 창에 못 보냈다 = INTERNAL(값은 남아 window.getTheme 이 새 값을 답한다).
    #[effect(Write)]
    #[since(14)]
    "window.setTheme" => args WindowSetThemeArgs {
        window: String,
        theme: Option<Option<String>>,
    } -> ok WindowSetThemeOk {
        theme: Option<String>,
        effective: String,
    } errors [CONFLICT];

    // ADR-0307
    /// 칸을 둘로 나눈다 — dir = LeftRight(좌/우 두 칸) · TopBottom(위/아래 두 칸). 칸 = slot_id(view_id 를 함께 주면 그 탭의
    /// 칸이어야 한다) | slot_of(그 에이전트가 든 칸 — 이름 또는 id) | at(공간 낱말 — window 의 보이는 탭에서 · window 를
    /// 빼면 main) 중 하나. at = top-left · top-right · bottom-left · bottom-right(그 모서리 칸) · left · right · up ·
    /// down(포커스 칸의 그쪽 이웃). window 는 at 과만 쓰고 window_id 또는 같은 실행 안 label 이다(번호는 받지 않는다).
    /// 답 = slot_id(새로 생긴 칸) · layout(그 탭의 칸 줄 — layout.get rows 모양 · id 없음).
    /// 오류: slot_of 가 어느 칸에도 없거나 그 이름이 없으면 · 낱말이 칸을 못 집으면 NOT_FOUND · slot_of 가 여러 칸에 떠
    /// 있으면 CONFLICT(문구 끝 candidates 의 slot_id= 값을 slot_id 로 다시) · 그 이름의 에이전트가 둘 이상이면
    /// CONFLICT(candidates = 에이전트 id — slot_of 로 다시) · 없는 slot_id · view_id 는 CONFLICT.
    #[effect(Write)]
    #[since(1)]
    "slot.split" => args SlotSplitArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        dir: SplitDirection,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
    } -> ok SlotSplitOk {
        slot_id: String,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// 칸을 닫는다(형제가 그 자리를 물려받는다). 칸 = slot_id(view_id 를 함께 주면 그 탭의 칸이어야 한다) | slot_of(그
    /// 에이전트가 든 칸 — 이름 또는 id) | at(공간 낱말 — window 의 보이는 탭에서 · window 를 빼면 main) 중 하나. at =
    /// top-left · top-right · bottom-left · bottom-right(그 모서리 칸) · left · right · up · down(포커스 칸의 그쪽 이웃).
    /// window 는 at 과만 쓰고 window_id 또는 같은 실행 안 label 이다(번호는 받지 않는다).
    /// 답 = layout(그 탭의 남은 칸 줄 — layout.get rows 모양 · id 없음).
    /// 오류: slot_of 가 어느 칸에도 없거나 그 이름이 없으면 · 낱말이 칸을 못 집으면 NOT_FOUND · slot_of 가 여러 칸에 떠
    /// 있으면 CONFLICT(문구 끝 candidates 의 slot_id= 값을 slot_id 로 다시) · 그 이름의 에이전트가 둘 이상이면
    /// CONFLICT(candidates = 에이전트 id — slot_of 로 다시) · 없는 slot_id · view_id 는 CONFLICT.
    #[effect(Write)]
    #[since(1)]
    "slot.close" => args SlotCloseArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
    } -> ok SlotCloseOk {
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

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
    //   `{window, tab, window_id, number}`, 여기는 `{window, new_view_id, window_id, number}`. 한쪽만 맞춰 고치지 말 것 — 합류는 프론트 자체
    //   레지스트리를 은퇴시키는 후속 스텝(화면 몫 등록) 몫이고, 지금 한쪽을 바꾸면 그 스텝이 옮길 대상만
    //   늘어난다. 반대편에도 같은 메모가 붙어 있다.
    // ADR-0307
    /// 칸의 내용을 다른 창의 새 탭으로 옮긴다(원본 칸은 닫힌다) — to_window 를 빼면 새 창을 연다. 칸 하나만 옮긴다 — 탭째는
    /// tab.detach.
    /// to_window = window.list 의 window_id(재시작을 넘어 같은 창) 또는 같은 실행 안 label. 번호는 받지 않는다(다시
    /// 쓰인다 — INVALID_ARGUMENT). label 은 그 실행 안에서만 맞는다.
    /// 칸 = slot_id(view_id 를 함께 주면 그 탭의 칸이어야 한다) | slot_of(그 에이전트가 든 칸 — 이름 또는 id) | at(공간
    /// 낱말 — window 의 보이는 탭에서 · window 를 빼면 main) 중 하나. at = top-left · top-right · bottom-left ·
    /// bottom-right(그 모서리 칸) · left · right · up · down(포커스 칸의 그쪽 이웃). window 는 at 과만 쓰고 to_window 와 같은
    /// 꼴이다.
    /// 답 = 옮겨 간 창의 window · window_id · number(window.create 와 같은 뜻 — 기존 창이면 그 창의 것) + new_view_id
    /// (그 창에 생긴 탭) + layout(원본 탭 · 새 탭의 칸 줄 — layout.get rows 모양 · id 없음).
    /// 오류: slot_of 가 어느 칸에도 없거나 그 이름이 없으면 · 낱말이 칸을 못 집으면 NOT_FOUND · slot_of 가 여러 칸에 떠
    /// 있으면 CONFLICT(문구 끝 candidates 의 slot_id= 값을 slot_id 로 다시) · 그 이름의 에이전트가 둘 이상이면
    /// CONFLICT(candidates = 에이전트 id — slot_of 로 다시) · 없는 slot_id · view_id · to_window 는 CONFLICT.
    #[effect(Write)]
    #[since(2)]
    "slot.popout" => args SlotPopoutArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        to_window: Option<String>,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
    } -> ok SlotPopoutOk {
        window: String,
        new_view_id: String,
        window_id: String,
        number: u32,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// 포커스를 그 칸으로 옮긴다(출력 라우팅은 안 바뀐다). 보이는 탭은 안 바꾼다 — 그 탭을 보이려면 먼저 tab.switch tab_of.
    /// 답 = layout(그 탭의 칸 줄 — layout.get rows 모양 · id 없음).
    /// 칸 = slot_id(view_id 를 함께 주면 그 탭의 칸이어야 한다) |
    /// slot_of(그 에이전트가 든 칸 — 이름 또는 id) | at(공간 낱말 — window 의 보이는 탭에서 · window 를 빼면 main) 중 하나.
    /// at = top-left · top-right · bottom-left · bottom-right(그 모서리 칸) · left · right · up · down(포커스 칸의 그쪽 이웃).
    /// window 는 at 과만 쓰고 window_id 또는 같은 실행 안 label 이다(번호는 받지 않는다).
    /// 오류: slot_of 가 어느 칸에도 없거나 그 이름이 없으면 · 낱말이 칸을 못 집으면 NOT_FOUND · slot_of 가 여러 칸에 떠
    /// 있으면 CONFLICT(문구 끝 candidates 의 slot_id= 값을 slot_id 로 다시) · 그 이름의 에이전트가 둘 이상이면
    /// CONFLICT(candidates = 에이전트 id — slot_of 로 다시) · 없는 slot_id · view_id 는 CONFLICT.
    #[effect(Write)]
    #[since(1)]
    "slot.focus" => args SlotFocusArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
    } -> ok SlotFocusOk {
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// 이미 살아 있는 에이전트를 그 칸에 붙인다(새로 띄우지 않는다 — 띄우려면 agent.spawnInto). 붙일 에이전트 = assign(이름
    /// 또는 id) | agent_id(id) 중 하나 — 데몬 명부에 없으면 NOT_FOUND 이고 아무것도 안 바뀐다.
    /// 칸 = slot_id(view_id 를 함께 주면 그 탭의 칸이어야 한다) | slot_of(그 에이전트가 든 칸 — 이름 또는 id) | at(공간
    /// 낱말 — window 의 보이는 탭에서 · window 를 빼면 main) 중 하나. at = top-left · top-right · bottom-left ·
    /// bottom-right(그 모서리 칸) · left · right · up · down(포커스 칸의 그쪽 이웃). window 는 at 과만 쓰고 window_id 또는
    /// 같은 실행 안 label 이다(번호는 받지 않는다).
    /// 답 = layout(그 탭의 칸 줄 — layout.get rows 모양 · id 없음).
    /// 오류: 에이전트(slot_of · assign · agent_id)가 데몬 명부에 없거나 slot_of 가 어느 칸에도 없으면 · 낱말이 칸을 못 집으면
    /// NOT_FOUND · slot_of 가 여러 칸에 떠 있으면 CONFLICT(문구 끝 candidates 의 slot_id= 값을 slot_id 로 다시) · 그 이름의
    /// 에이전트가 둘 이상이면 CONFLICT(candidates = 에이전트 id — 그 인자(slot_of · assign)로 다시) · 없는 slot_id · view_id 는
    /// CONFLICT.
    #[effect(Write)]
    #[since(1)]
    "slot.assignAgent" => args SlotAssignAgentArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        #[agent_ref]
        agent_id: Option<String>,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
        #[agent_ref]
        assign: Option<String>,
    } -> ok SlotAssignAgentOk {
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// 칸이 무엇을 보여줄지 바꾼다. content=Agent 일 때만 assign(이름 또는 id) | agent_id(id) 중 하나를 준다 — 데몬 명부에
    /// 없으면 NOT_FOUND 이고 아무것도 안 바뀐다. content=Usage 일 때만 show_claude·show_codex(그 회사를 보일지)를 줄 수
    /// 있다 — 뺀 칸은 그 칸이 이미 Usage 면 지금 값을 그대로 두고, 아니면 true 다.
    /// 칸 = slot_id(view_id 를 함께 주면 그 탭의 칸이어야 한다) | slot_of(그 에이전트가 든 칸 — 이름 또는 id) | at(공간
    /// 낱말 — window 의 보이는 탭에서 · window 를 빼면 main) 중 하나. at = top-left · top-right · bottom-left ·
    /// bottom-right(그 모서리 칸) · left · right · up · down(포커스 칸의 그쪽 이웃). window 는 at 과만 쓰고 window_id 또는
    /// 같은 실행 안 label 이다(번호는 받지 않는다).
    /// 답 = layout(그 탭의 칸 줄 — layout.get rows 모양 · id 없음).
    /// 오류: 에이전트(slot_of · assign · agent_id)가 데몬 명부에 없거나 slot_of 가 어느 칸에도 없으면 · 낱말이 칸을 못 집으면
    /// NOT_FOUND · slot_of 가 여러 칸에 떠 있으면 CONFLICT(문구 끝 candidates 의 slot_id= 값을 slot_id 로 다시) · 그 이름의
    /// 에이전트가 둘 이상이면 CONFLICT(candidates = 에이전트 id — 그 인자(slot_of · assign)로 다시) · 없는 slot_id · view_id 는
    /// CONFLICT.
    #[effect(Write)]
    #[since(1)]
    "layout.setSlotContent" => args LayoutSetSlotContentArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        content: SlotContentKind,
        #[agent_ref]
        agent_id: Option<String>,
        show_claude: Option<bool>,
        show_codex: Option<bool>,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
        #[agent_ref]
        assign: Option<String>,
    } -> ok LayoutSetSlotContentOk {
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// 에이전트를 새로 띄우고 그 자리에 배치한다(스폰 + 필요하면 새 탭 + 슬롯 배정).
    /// view_id 를 빼면 새 탭을 만들어 거기 넣는다 — 그 경우 slot_id 는 줄 수 없다. at(공간 낱말 top-left · top-right ·
    /// bottom-left · bottom-right · left · right · up · down)을 주면 window 의 보이는 탭에서 그 낱말이 집는 빈 칸에 넣는다 —
    /// view_id · slot_id 와 함께 못 주고, 못 집으면 NOT_FOUND · 비어 있지 않으면 CONFLICT(둘 다 스폰 전에 거절한다).
    /// backend 는 `"claude"` 또는 `"codex"` — 둘 다 이 표면으로 만든다. 모르는 낱말은 스폰 전에
    /// 거부되고, 빼면 데몬이 거절한다(기본 백엔드는 없다).
    /// window = window_id 또는 같은 실행 안 label(번호는 받지 않는다) — 빼면 view_id 를 준 경우 그 탭의 창, 아니면 main.
    /// view_id 가 window 의 탭이 아니면 스폰 전에 CONFLICT 다.
    /// 답 = agent_id · layout(놓인 탭의 칸 줄 — layout.get rows 모양 · id 없음).
    #[effect(Write)]
    #[since(1)]
    "agent.spawnInto" => args AgentSpawnIntoArgs {
        window: Option<String>,
        cwd: String,
        view_id: Option<String>,
        slot_id: Option<String>,
        backend: Option<String>,
        at: Option<String>,
    } -> ok AgentSpawnIntoOk {
        agent_id: String,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    /// 공간/방향 낱말(top-left·right·up …)을 슬롯 id 로 푼다. view_id 를 빼면 그 창의 활성 탭이 대상이고,
    /// window 도 빼면 main 창이다. 그 방향에 슬롯이 없으면 slot_id 는 null 이다. window 는 tab.create 와 같다
    /// (window_id 또는 같은 실행 안 label · 번호는 받지 않는다).
    #[effect(Read)]
    #[since(1)]
    "slot.resolveSpatial" => args SlotResolveSpatialArgs {
        token: String,
        window: Option<String>,
        view_id: Option<String>,
    } -> ok SlotResolveSpatialOk {
        slot_id: Option<String>,
    } errors [CONFLICT];

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
    /// 모르는 키 = NOT_FOUND · 형식·범위 위반 = INVALID_ARGUMENT(문구에 기대 형식) · 설정 파일이 이 앱보다 새
    /// 버전이 쓴 것이면 CONFLICT(파일을 덮지 않는다 · 값은 그대로다 — 그 파일이 바뀌기 전에는 다시 해도 같다 · 단
    /// 유효 값이 바뀌지 않는 호출은 그때도 changed=false 로 성공한다) · 설정 파일을 지금 못 읽었으면 CONFLICT(값은
    /// 그대로다 — 대개 잠깐 쥔 잠김이라 조금 뒤 다시 하면 된다) · 디스크에 못 썼으면 INTERNAL(값은 그대로다).
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
    } errors [NOT_FOUND, CONFLICT];

    /// 키 하나, 또는 점으로 끝나는 접두가 덮는 키 전부를 기본값으로 되돌린다 — key 는 필수(전체 초기화는 없다).
    /// 답의 reset = 그 선택자가 덮은 키 전부(호출 뒤 모두 기본값). 이미 다 기본값이면 rev 가 그대로다(파일에
    /// 남은 그 키들은 지운다). 맞는 키가 없으면 NOT_FOUND. 설정 파일이 이 앱보다 새 버전이 쓴 것이면 CONFLICT ·
    /// 지금 못 읽었으면 CONFLICT(조금 뒤 다시 하면 대개 된다) · 디스크에 못 썼으면 INTERNAL 이다(settings.set 과
    /// 같다 — 이미 다 기본값인 호출은 새 판 파일이어도 rev 그대로 성공한다).
    #[effect(Write)]
    #[since(10)]
    "settings.reset" => args SettingsResetArgs {
        key: String,
    } -> ok SettingsResetOk {
        rev: u64,
        reset: Vec<String>,
    } errors [NOT_FOUND, CONFLICT];

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
    /// windows = 사본의 창 수(main + 팝아웃) · tabs = 그 창들의 탭 수 합 · durable = 그동안의
    /// saves 와 같은 값(true 면 답을 디스크에 붙이고 사본을 지우려 한다 — 실제로 붙었는지는 restore.answer 의 durable ·
    /// false 면 답해도 크래시 때 화면이 디스크에 남아 다음 시작이 다시 묻는다).
    /// 다음 둘은 crash_copy 와 무관하게 늘 값이고 이번 실행 내내 같다(답한 뒤에도).
    /// saves = 이번 실행이 화면 상태를 저장하나 — true = 가드가 아니다: 기록기를 띄우려 한다(띄우기 · 쓰기 성공은
    /// 보장하지 않는다) · false = 가드다: 화면 상태를 하나도 저장하지 않고 다음 시작이 다시 판정한다(state_file 이
    /// unreadable · newer 이거나, ok 인데 떠야 할 크래시 사본을 못 떴다).
    /// state_file = 시작할 때 state.json 을 어떻게 읽었나: ok(읽었거나 없었다 — 저장하나는 saves 가 말한다) ·
    /// unreadable(못 읽었다 — 파일은 그대로 두고 saves 도 false) · newer(이 앱보다 새 버전이 쓴 파일이라 기본 화면으로
    /// 시작했다 — 떠 두지도 덮지도 않고 saves 도 false) · corrupt_copied_aside · corrupt_not_copied(못 쓰는 파일(손상 ·
    /// 상한 초과 · UTF-8 아님)이라 기본 화면으로 시작했다 — corrupt_copied_aside 는 state.json.corrupt 로 떠 두었고
    /// corrupt_not_copied 는 떠 두지 못해 원본이 백업 없이 덮인다. 이미 있는 state.json.corrupt 는 앞선 시작이 떠
    /// 둔 것이지 이번 원본의 백업이 아니다).
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

    // ── 대상을 받는 명령(TRD S28 §4) — 에이전트 이름은 데몬 명부의 해석기 하나가 푼다(ADR-0132) ──
    // ★errors 에 데몬의 이름 풀기 거절 코드도 든다★ — `#[agent_ref]` 칸은 데몬이 중계 전에 id 로 바꾸고, 그 이름이 없으면
    //   NOT_FOUND · 둘 이상이면 CONFLICT 로 이 명령 대신 답한다(ADR-0308 · ADR-0156 발견 계약 — 실제로 나가는 코드를 다
    //   적는다).
    // ★summary 는 등록 help 상한(명부 4096 바이트) 안이어야 한다★ — 시험 `every_registered_declaration_fits_the_roster_help_cap`.

    // ADR-0307
    /// 탭 하나를 탭 줄의 다른 자리로 옮긴다(창 안 순서 · 다른 창). 옮길 탭 = tab(탭 id) | tab_of(그 에이전트가 든 탭 — 이름
    /// 또는 id) 중 하나. tab_of 의 탭에 다른 칸이 있으면 함께 간다 — 그 칸 하나만은 slot.popout.
    /// 갈 곳 = to_window(그 창의 index 자리) | after_tab(그 탭 id 바로 뒤) | after_tab_of(그 에이전트가 든 탭 바로 뒤) 중
    /// 하나 — 셋 다 없으면 지금 창의 index 자리. index 는 to_window 와 그때만(0 부터 · 빼거나 넘치면 맨 끝 · 셋 다 없으면
    /// 필수). to_window = window_id 또는 같은 실행 안 label(번호는 받지 않는다). window 인자는 INVALID_ARGUMENT(갈 곳은
    /// to_window).
    /// 보이는 탭 · 포커스를 바꾸지 않는다 — 단 보이던 탭이 떠난 창은 이웃 탭을(main 이 비면 새 빈 탭을) 보인다.
    /// 답 = changed(같은 자리면 false) · window · window_id · number(놓인 창) · tab · tab_index(그 탭 · 탭 줄 자리) ·
    /// layout(바뀐 탭의 칸 줄 — layout.get rows 모양 · id 없음).
    /// 오류: 에이전트가 어느 칸에도 없거나 그 이름이 없으면 NOT_FOUND · 여러 탭에 있거나 이름이 둘 이상이면 CONFLICT — 문구
    /// 끝 candidates 로 다시 부른다(tab= 값 → tab · after_tab= 값 → after_tab, 에이전트 id → tab_of · after_tab_of).
    /// after_tab(_of) 가 옮길 탭이면 INVALID_ARGUMENT.
    #[effect(Write)]
    #[since(21)]
    "tab.move" => args TabMoveArgs {
        tab: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
        to_window: Option<String>,
        after_tab: Option<String>,
        #[agent_ref]
        after_tab_of: Option<String>,
        index: Option<u32>,
        window: Option<String>,
    } -> ok TabMoveOk {
        changed: bool,
        window: String,
        window_id: String,
        number: u32,
        tab: String,
        tab_index: Option<u32>,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// 탭 하나를 다른 탭의 칸 옆에 붙인다(분할) — 옮긴 탭의 칸들이 그 탭 안으로 들어가고 옮긴 탭은 없어진다. 옮길 탭 =
    /// tab | tab_of(tab.move 와 같다). tab_of 의 탭에 다른 칸이 있으면 함께 간다 — 그 칸 하나만은 slot.popout.
    /// 붙일 칸 = beside_slot(칸 id) | beside(그 에이전트가 든 칸) | beside_at(공간 낱말 — window 의 보이는 탭에서 · window 를
    /// 빼면 main) 중 하나. beside_at 의 left · right · up · down = 포커스 칸의 그쪽 이웃 · top-left · top-right · bottom-left
    /// · bottom-right = 그 모서리 칸. window 는 beside_at 과만 쓰고 window_id 또는 같은 실행 안 label(번호는 받지 않는다).
    /// side = 옮긴 탭이 놓일 쪽(Left · Right · Top · Bottom). whole = true 면 그 칸이 든 탭 전체의 바깥 변. share = 붙는 쪽
    /// 몫(0.01~0.5) — 빼면 마우스와 같은 몫(창 크기를 모르면 0.5).
    /// 보이는 탭 · 포커스 · 답은 tab.move 와 같다(tab = 붙인 곳의 살아남은 탭).
    /// 오류는 tab.move 와 같다 — beside 가 여러 칸에 있으면 CONFLICT(candidates 의 beside_slot= 값 → beside_slot) · 붙일 칸이
    /// 옮길 탭 안이면 INVALID_ARGUMENT · 낱말이 칸을 못 집으면 NOT_FOUND.
    #[effect(Write)]
    #[since(21)]
    "tab.dock" => args TabDockArgs {
        tab: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
        beside_slot: Option<String>,
        #[agent_ref]
        beside: Option<String>,
        beside_at: Option<String>,
        window: Option<String>,
        side: DockSide,
        whole: Option<bool>,
        share: Option<f64>,
    } -> ok TabDockOk {
        changed: bool,
        window: String,
        window_id: String,
        number: u32,
        tab: String,
        tab_index: Option<u32>,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// 탭 하나를 새 창으로 뗀다 — 새 창은 포커스를 가져가지 않는다. 옮길 탭 = tab | tab_of(tab.move 와 같다). tab_of 의
    /// 탭에 다른 칸이 있으면 함께 간다 — 그 칸 하나만은 slot.popout. 이미 혼자인 팝아웃이면 아무것도 안 바뀐다(changed =
    /// false). 새 창 크기 = 원래 창이 팝아웃이면 그 크기 · main 이면 팝아웃 기본 크기. 답 · 오류는 tab.move 와 같다
    /// (window = 새 창).
    #[effect(Write)]
    #[since(21)]
    "tab.detach" => args TabDetachArgs {
        tab: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
    } -> ok TabDetachOk {
        changed: bool,
        window: String,
        window_id: String,
        number: u32,
        tab: String,
        tab_index: Option<u32>,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// 배치를 칸마다 한 줄로 읽는다 — 창 번호 순 → 탭 줄 순 → 탭 안 칸 순. window(window_id 또는 같은 실행 안 label ·
    /// 번호는 받지 않는다)를 주면 그 창만, agent(이름 또는 id)를 주면 그 에이전트가 든 칸만(어느 칸에도 없으면 빈 목록).
    /// 줄 = window · window_number · tab_index(그 창 탭 줄 자리 · 0 부터 — 같은 탭의 줄을 묶는 열쇠) · tab_name ·
    /// tab_active(그 창에 보이는 탭) · content(Empty · Agent · AgentList · PresetPalette · Usage · Unknown) · agent(표시
    /// 이름 — 모르면 빠진다) · focused(그 탭 안의 포커스 칸 — 창 · OS 포커스가 아니다) · x · y · w · h(탭 안 정규 좌표 0~1 —
    /// 왼쪽 위가 0,0).
    /// ids = true 면 window_id · tab(다른 명령이 view_id 라 부르는 그 탭 id) · slot_id · agent_id 도 싣는다(기본은 빠진다 —
    /// 칸은 에이전트 이름 · 공간 낱말로 부른다). 그런 에이전트 이름이 없으면 NOT_FOUND · 이름이 둘 이상이면 CONFLICT(문구 끝
    /// candidates) · 없는 창이면 CONFLICT.
    #[effect(Read)]
    #[since(21)]
    "layout.get" => args LayoutGetArgs {
        window: Option<String>,
        #[agent_ref]
        agent: Option<String>,
        ids: Option<bool>,
    } -> ok LayoutGetOk {
        rows: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];
}

/// 이 표의 핸들러들이 잡는 실물 전량 — ★조립 때 주입된다★(ADR-0155 결정 5 / 규칙 T-1).
///
/// 앞 다섯의 계약(어느 것이 락 안이고 어느 것이 락 밖인가)은 적용 서비스가, `control` 의 계약은 `super::window_ctl` 이
/// 소유한다 — 이 구조체는 그것을 **소유형으로** 들고 있을 뿐이다. `#[tauri::command]` 쪽이 빌려 쓰는 어댑터를 `Arc` 로
/// 바꾼 것이 차이의 전부이고, 그렇게 하는 이유는 표의 핸들러가 `'static` 이어야 하기 때문이다.
///
/// ★이름이 `Layout` 인데 적용 서비스 밖 포트가 넷 있다★(`themes` · `settings` · `settings_events` · `restore`) —
/// 표가 하나라 포트 묶음도 하나다(사유 = 모듈 헤더 「레이아웃 밖의 셸 명령도 여기 선다」). 그 포트들은 적용 서비스를
/// 안 거치고 자기 모듈(`crate::theme` · `crate::settings` · `crate::state::restore`)만 부르므로 락 규율도 그 모듈이
/// 진다.
pub struct LayoutPorts {
    pub state: LayoutState,
    pub subs: Arc<dyn SubscriptionSync>,
    pub events: Arc<dyn LayoutEvents>,
    pub windows: Arc<dyn WindowHost>,
    pub labels: Arc<dyn LabelSource>,
    pub spawner: Arc<dyn AgentSpawner>,
    /// 창 상태(최소화 · 최대화 · 보이기 · 숨기기 · OS 상태 읽기) — 계약은 `super::window_ctl` 이 소유한다. 실물이 메인
    /// 스레드 일을 기다리므로 이 포트를 부르는 동사는 [`offloaded_handler`] 로 꽂는다.
    pub control: Arc<dyn WindowControl>,
    /// 창 테마 읽기 · 쓰기 · 밀기 — 설정 알림 · 복원 조율자와 **같은 유효 테마 인스턴스**.
    pub themes: ThemeControl,
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
    // 탭 · 칸을 에이전트로 고를 수 있는 동사 — 적용을 블로킹 풀에서 돈다(`targeted_handler` · 아래 칸 동사 · `tab.move` 도
    //   같다).
    plug(
        &mut table,
        "tab.switch",
        targeted_handler(&ports, verb_tab_switch),
    );
    plug(
        &mut table,
        "tab.close",
        targeted_handler(&ports, verb_tab_close),
    );
    plug(
        &mut table,
        "tab.rename",
        targeted_handler(&ports, verb_tab_rename),
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
    // 창 상태 포트를 부르는 동사 — 실물이 메인 스레드 일을 기다린다(TRD S26-window-chrome §5-2). `window.close` 는
    //   main 이면 모두 숨기기를 기다리고, `window.list` 는 창마다 OS 상태를 읽는다.
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.close",
        offloaded_handler(move |args: WindowCloseArgs| verb_window_close(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.list",
        offloaded_handler(move |_: WindowListArgs| verb_window_list(&p)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.minimize",
        offloaded_handler(move |args: WindowMinimizeArgs| {
            verb_window_act(&p, &args.window, WindowAction::Minimize).map(|_| WindowMinimizeOk {})
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.maximize",
        offloaded_handler(move |args: WindowMaximizeArgs| {
            verb_window_act(&p, &args.window, WindowAction::Maximize).map(|applied| {
                WindowMaximizeOk {
                    deferred: applied.deferred,
                }
            })
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.unmaximize",
        offloaded_handler(move |args: WindowUnmaximizeArgs| {
            verb_window_act(&p, &args.window, WindowAction::Unmaximize).map(|applied| {
                WindowUnmaximizeOk {
                    deferred: applied.deferred,
                }
            })
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.raise",
        offloaded_handler(move |args: WindowRaiseArgs| {
            verb_window_act(&p, &args.window, WindowAction::Raise).map(|applied| WindowRaiseOk {
                shown_all: applied.shown_all,
                focused: applied.focused,
                pending: applied.pending,
            })
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.hideAll",
        offloaded_handler(move |_: WindowHideAllArgs| {
            p.control.hide_all();
            Ok(WindowHideAllOk {})
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.showAll",
        offloaded_handler(move |_: WindowShowAllArgs| {
            Ok(WindowShowAllOk {
                focused: p.control.show_all(Activation::NoSteal),
            })
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.getTheme",
        blocking_handler(move |args: WindowGetThemeArgs| verb_window_get_theme(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.setTheme",
        blocking_handler(move |args: WindowSetThemeArgs| verb_window_set_theme(&p, args)),
    );
    plug(
        &mut table,
        "slot.split",
        targeted_handler(&ports, verb_slot_split),
    );
    plug(
        &mut table,
        "slot.close",
        targeted_handler(&ports, verb_slot_close),
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
    plug(
        &mut table,
        "slot.popout",
        targeted_handler(&ports, verb_slot_popout),
    );
    plug(
        &mut table,
        "slot.focus",
        targeted_handler(&ports, verb_slot_focus),
    );
    plug(
        &mut table,
        "slot.assignAgent",
        targeted_handler(&ports, verb_slot_assign_agent),
    );
    plug(
        &mut table,
        "layout.setSlotContent",
        targeted_handler(&ports, verb_set_slot_content),
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
    plug(
        &mut table,
        "tab.move",
        targeted_handler(&ports, verb_tab_move),
    );
    plug(
        &mut table,
        "tab.dock",
        targeted_handler(&ports, verb_tab_dock),
    );
    plug(
        &mut table,
        "tab.detach",
        targeted_handler(&ports, verb_tab_detach),
    );
    plug(
        &mut table,
        "layout.get",
        targeted_handler(&ports, verb_layout_get),
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
            let ok = off_thread(move || run(parsed)).await?;
            serde_json::to_value(ok).map_err(|e| CommandError::internal(e.to_string()))
        })
    }
}

/// 막는 본문을 블로킹 풀에서 돌린다 — tokio 런타임 밖에서 폴링되면 그 자리에서 돈다(막을 워커가 없다). 사유는
/// [`offloaded_handler`] 와 같다. ★답보다 본문이 오래 산다★ — 기다리던 future 가 버려져도 본문은 끝까지 돈다.
async fn off_thread<T: Send + 'static>(
    body: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => runtime.spawn_blocking(body).await.map_err(|e| {
            // 버스 경로의 패닉 그물(`handler_panicked`)과 같은 문구 — 본문이 블로킹 풀에서 터지면 그 그물이 못 보고
            //   여기로 온다. 운영 셸에선 훅이 먼저 셸을 끝내 `is_panic` 갈래에 닿지 않는다 — 훅 없는 시험 빌드에서만
            //   산다(ADR-0292).
            let what = if e.is_panic() {
                "panicked"
            } else {
                "was cancelled"
            };
            CommandError::internal(format!("command handler {what}: {e}"))
        })?,
        Err(_) => body(),
    }
}

/// 탭 · 칸을 고르는 동사의 핸들러 — [`offloaded_handler`] 처럼 본문을 블로킹 풀에서 돌리고, 답은 `null` 칸을 뺀 JSON 이다
/// ([`without_nulls`]).
///
/// ★블로킹 풀인 이유★(TRD S28 §8): 새 창 도킹(`tab.detach` · 새 창 `slot.popout`)은 main 스레드 · 동기 명령에서 부르면 창
/// 짓기가 교착하고(`apply::dock_tab` 문서), 예열 창을 기다리는 동안(~1 초) tokio 일꾼을 막는다. 같은 줄의 동사가 함께 쓴다.
// ADR-0307
fn targeted_handler<A, O, F>(ports: &Arc<LayoutPorts>, run: F) -> Arc<dyn CommandHandler>
where
    F: Fn(&LayoutPorts, A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    Arc::new(Targeted {
        ports: Arc::clone(ports),
        run: Arc::new(run),
        _types: std::marker::PhantomData,
    })
}

struct Targeted<F, A, O> {
    ports: Arc<LayoutPorts>,
    run: Arc<F>,
    _types: std::marker::PhantomData<fn(A) -> O>,
}

impl<F, A, O> CommandHandler for Targeted<F, A, O>
where
    F: Fn(&LayoutPorts, A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn call(&self, args: serde_json::Value) -> CommandFuture {
        let ports = Arc::clone(&self.ports);
        let run = Arc::clone(&self.run);
        Box::pin(async move {
            let parsed: A = serde_json::from_value(args)
                .map_err(|e| CommandError::invalid_argument(e.to_string()))?;
            let ok = off_thread(move || run(&ports, parsed)).await?;
            serde_json::to_value(ok)
                .map(without_nulls)
                .map_err(|e| CommandError::internal(e.to_string()))
        })
    }
}

/// 객체의 `null` 칸을 (깊이 끝까지) 뺀다 — 대상을 받는 명령의 답이 선택 칸(줄의 id 넷 · 모르는 표시 이름)을 `null` 로
/// 싣지 않게 한다(규칙 §4 — 크기). 그 칸들은 선언 스키마에서 필수가 아니라 빠져도 계약 안이다.
// ADR-0307
fn without_nulls(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, without_nulls(v)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(without_nulls).collect())
        }
        other => other,
    }
}

// ── 동사 ────────────────────────────────────────────────────────────────────

fn verb_tab_create(ports: &LayoutPorts, args: TabCreateArgs) -> Result<TabCreateOk, CommandError> {
    let window = window_label(ports, "window", &args.window)?;
    let name = optional_text("name", args.name.as_deref())?.map(str::trim);
    let view_id = apply::create_tab(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        &window,
        name.map(str::to_string),
    )
    .map_err(not_applied)?;
    Ok(TabCreateOk {
        view_id: view_id.to_string(),
    })
}

fn verb_tab_list(ports: &LayoutPorts, args: TabListArgs) -> Result<TabListOk, CommandError> {
    let window = window_label(ports, "window", &args.window)?;
    let tabs = apply::list_tabs(&ports.state, &window).map_err(not_applied)?;
    Ok(TabListOk {
        window: tabs.label,
        tabs: tabs
            .tabs
            .into_iter()
            .map(|meta| TabRow {
                id: meta.id.to_string(),
                name: meta.name,
                named: meta.named,
            })
            .collect(),
        active: tabs.active.to_string(),
        version: tabs.version,
    })
}

fn verb_window_create(ports: &LayoutPorts) -> Result<WindowCreateOk, CommandError> {
    let created = apply::create_window(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.windows.as_ref(),
        ports.labels.as_ref(),
    )
    .map_err(not_applied)?;
    Ok(WindowCreateOk {
        window: created.window,
        window_id: created.window_id,
        number: created.number,
    })
}

// 창 인자를 해석기에 넘기기 전에 `if_empty` 를 본다 — 그 길의 창은 대개 이미 모델에 없다(차례 = `window_ctl::close`).
fn verb_window_close(
    ports: &LayoutPorts,
    args: WindowCloseArgs,
) -> Result<WindowCloseOk, CommandError> {
    let window = text("window", &args.window)?;
    let outcome = window_ctl::close(
        &ports.state,
        ports.subs.as_ref(),
        ports.windows.as_ref(),
        ports.control.as_ref(),
        window,
        args.if_empty.unwrap_or(false),
    )
    .map_err(window_ctl_error)?;
    Ok(WindowCloseOk {
        outcome: match outcome {
            CloseOutcome::Closed => WindowCloseOutcome::Closed,
            CloseOutcome::HiddenAll => WindowCloseOutcome::HiddenAll,
            CloseOutcome::Kept => WindowCloseOutcome::Kept,
        },
    })
}

fn verb_window_list(ports: &LayoutPorts) -> Result<WindowListOk, CommandError> {
    let rows = window_ctl::list(&ports.state, ports.control.as_ref()).map_err(not_applied)?;
    Ok(WindowListOk {
        windows: rows.iter().map(|row| row.window.clone()).collect(),
        details: rows
            .into_iter()
            .map(|row| WindowRow {
                window_id: row.window_id,
                window: row.window,
                number: row.number,
                main: row.main,
                visible: row.os.visible,
                minimized: row.os.minimized,
                maximized: row.os.maximized,
                focused: row.os.focused,
            })
            .collect(),
    })
}

fn verb_window_act(
    ports: &LayoutPorts,
    window: &str,
    action: WindowAction,
) -> Result<Applied, CommandError> {
    let window = text("window", window)?;
    window_ctl::act(&ports.state, ports.control.as_ref(), window, action).map_err(window_ctl_error)
}

fn window_ctl_error(error: WindowCtlError) -> CommandError {
    match error {
        WindowCtlError::Arg(error) => window_arg_error(error),
        WindowCtlError::NotApplied(detail) => not_applied(detail),
    }
}

fn verb_window_get_theme(
    ports: &LayoutPorts,
    args: WindowGetThemeArgs,
) -> Result<WindowGetThemeOk, CommandError> {
    let window = window_label(ports, "window", &args.window)?;
    let theme = ports.themes.get(&window).map_err(theme_error)?;
    let (theme, effective) = theme_reply(theme);
    Ok(WindowGetThemeOk { theme, effective })
}

fn verb_window_set_theme(
    ports: &LayoutPorts,
    args: WindowSetThemeArgs,
) -> Result<WindowSetThemeOk, CommandError> {
    let window = window_label(ports, "window", &args.window)?;
    // wire 로 들어온 부재는 여기까지 못 온다(선언 매크로가 `missing field` 로 반려한다) — 이 갈래는 코드가 인자를
    //   직접 지어 부르는 경로 몫이다(`agent.move` 의 `parent` 와 같다).
    let Some(given) = args.theme.as_ref() else {
        return Err(CommandError::invalid_argument(
            "setTheme needs theme: dark, light or e-ink — or null to clear the window's own theme",
        ));
    };
    let theme = given.as_deref().map(theme_arg).transpose()?;
    let written = ports.themes.set(&window, theme).map_err(theme_error)?;
    let (theme, effective) = theme_reply(written);
    Ok(WindowSetThemeOk { theme, effective })
}

/// 답의 `(theme, effective)` — 두 창 테마 명령이 같은 모양을 낸다.
fn theme_reply(theme: WindowTheme) -> (Option<String>, String) {
    (
        theme.own.map(|own| own.as_wire().to_string()),
        theme.effective.as_wire().to_string(),
    )
}

/// 대소문자를 가리지 않는다 — `settings.set theme.default` 의 선택지 읽기와 같은 관용이다.
fn theme_arg(raw: &str) -> Result<UiTheme, CommandError> {
    UiTheme::from_wire(&raw.to_ascii_lowercase()).ok_or_else(|| {
        CommandError::invalid_argument(format!(
            "theme must be dark, light or e-ink — or null (on the CLI: none, lowercase) to clear the window's own theme, got {raw:?}"
        ))
    })
}

/// 창 테마 서비스의 오류 종류를 코드로 — 모르는 창은 이웃 `window.*` 명령과 같은 `CONFLICT` 다(헤더 「적용 실패의
/// 코드」). 그 창에 못 보낸 쓰기는 `INTERNAL` 이다 — 값은 이미 적혀 `CONFLICT`(「지금 상태로는 적용할 수 없다」)가
/// 거짓이다.
fn theme_error(error: ThemeError) -> CommandError {
    let message = error.to_string();
    match error {
        ThemeError::UnknownWindow(_) => CommandError::of(ErrorCode::Conflict, message),
        ThemeError::Poisoned | ThemeError::Undelivered { .. } => CommandError::internal(message),
    }
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

fn verb_resolve_spatial(
    ports: &LayoutPorts,
    args: SlotResolveSpatialArgs,
) -> Result<SlotResolveSpatialOk, CommandError> {
    let token = text("token", &args.token)?;
    let window = optional_window_label(ports, "window", args.window.as_deref())?;
    let view = args
        .view_id
        .as_deref()
        .map(|raw| uuid_arg("view_id", raw))
        .transpose()?;
    let slot = apply::resolve_spatial(&ports.state, token, window.as_deref(), view)
        .map_err(not_applied)?;
    Ok(SlotResolveSpatialOk {
        slot_id: slot.map(|id| id.to_string()),
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

/// 디스크에 쓰고(`sync_all` 까지) `theme.default` 면 모든 창의 유효 테마를 민다 — 그래서 [`offloaded_handler`]
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
        // ADR-0291 R2: 새 판이 쓴 파일이라 덮지 않았다 — 「지금 상태로는 적용할 수 없다」라 다시 해도 그 파일이
        //   바뀌기 전에는 같다(`never`). `INTERNAL` 로 내면 일시 실패처럼 읽힌다.
        SettingsError::Conflict(message) => CommandError::of(ErrorCode::Conflict, message),
        // ADR-0291 R17 — 사용자 결정 2026-10-11: 쓸 때마다 다시 읽으므로 이 못 읽음은 늘 실행 중 재판정의 거절이고,
        //   대개 잠깐 쥔 잠김이라 `CONFLICT` · `after-condition` 이다(데몬 명부 저장소 · agent `store_command_error` 와
        //   같은 나눔). 오늘 그 지시는 데몬이 중계하며 `never` 로 내리므로(`restore_error` 의 같은 조항) 「조금 뒤
        //   다시」는 문구와 도움말이 함께 나른다.
        SettingsError::Unreadable(message) => {
            CommandError::with_retry(ErrorCode::Conflict, message, RetryMode::AfterCondition)
        }
        SettingsError::Internal(message) => CommandError::internal(message),
    }
}

/// `agent.spawnInto` 의 핸들러 — ★async 라 [`blocking_handler`] 를 못 쓴다★. 답은 `null` 칸을 뺀 JSON 이다
/// ([`without_nulls`] — 배치 줄의 id 칸).
///
/// 본문을 `call` 안이 아니라 future 안에서 도는 것은 계약이다(마감시각이 그 형태 위에 선다 —
/// `blocking_handler` 의 같은 조항).
/// ★[`targeted_handler`] 와 달리 적용을 블로킹 풀로 옮기지 않는다★ — 적용이 데몬 스폰을 중간에 기다리는 async 다(TRD S28
/// §5-2 · §8).
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
            serde_json::to_value(ok)
                .map(without_nulls)
                .map_err(|e| CommandError::internal(e.to_string()))
        })
    }
}

async fn verb_spawn_into(
    ports: &LayoutPorts,
    args: AgentSpawnIntoArgs,
) -> Result<AgentSpawnIntoOk, CommandError> {
    // 빼면 낱말판은 main 을, id 판은 view_id 의 창을 쓴다(적용 서비스가 스폰 전에 고른다).
    let window = optional_window_label(ports, "window", args.window.as_deref())?;
    // ★`agent.new`·`agent.spawn --cwd` 와 **같은 철자 규칙**을 받는다★ — 이 문은 그 셋의 공통 등록부
    //   (`agent::commands::register`)를 안 지나므로, 여기서 안 부르면 같은 낱말이 문마다 다르게 읽힌다.
    let cwd = normalize_spelling(text("cwd", &args.cwd)?);
    let tab = args
        .view_id
        .as_deref()
        .map(|raw| uuid_arg_from("view_id", raw, LAYOUT_IDS))
        .transpose()?;
    let slot = args
        .slot_id
        .as_deref()
        .map(|raw| uuid_arg_from("slot_id", raw, LAYOUT_IDS))
        .transpose()?;
    let at = optional_text("at", args.at.as_deref())?
        .map(|word| Ok::<_, CommandError>((spatial_token("at", word)?, word)))
        .transpose()?;
    // ★backend 를 여기서 정규화하지 않는다★ — 「명시된 backend 는 스폰 전에 거부」가 적용 서비스의 조항이라
    //   (ADR-0058) 빈 문자열을 부재로 접으면 그 거부가 이 입구에서만 느슨해진다.
    let placed = match at {
        // 낱말이 칸을 집으므로 탭 · 칸 id 와 함께 받으면 어느 쪽이 이기는지 모르게 된다.
        Some(_) if tab.is_some() || slot.is_some() => {
            return Err(CommandError::invalid_argument(
                "at picks a slot in window's shown tab — give it without view_id and slot_id",
            ))
        }
        Some((token, word)) => apply::spawn_into_at(
            &ports.state,
            ports.subs.as_ref(),
            ports.events.as_ref(),
            ports.spawner.as_ref(),
            window.as_deref().unwrap_or(MAIN_WINDOW_LABEL),
            token,
            args.backend,
            cwd,
        )
        .await
        .map_err(|error| {
            spawn_at_error(
                &ports.state,
                error,
                &Choice::spatial("at", "slot_id", None, word, token),
            )
        })?,
        None => apply::spawn_into_placed(
            &ports.state,
            ports.subs.as_ref(),
            ports.events.as_ref(),
            ports.spawner.as_ref(),
            window.as_deref(),
            tab,
            slot,
            args.backend,
            cwd,
        )
        .await
        .map_err(spawn_not_applied)?,
    };
    Ok(AgentSpawnIntoOk {
        agent_id: placed.out,
        layout: layout_of(placed.rows),
    })
}

/// 낱말판 스폰의 거절 → 버스 오류. 낱말이 칸을 못 집었으면(창 없음 포함) [`locate_error`] 의 코드 · 문구에 「띄운 것이
/// 없다」를 붙인다 · 스폰 전 그 밖의 거절(점유) = CONFLICT · 스폰 뒤 실패는 id 판과 같다([`spawn_not_applied`]).
// ADR-0307
fn spawn_at_error(state: &LayoutState, error: SpawnAtError, said: &Choice) -> CommandError {
    match error {
        SpawnAtError::Unplaced(TargetError::Locate(error)) => {
            let refused = locate_error(state, said, error);
            CommandError::of(
                refused.code(),
                format!("{} — nothing was spawned", refused.message()),
            )
        }
        unplaced @ SpawnAtError::Unplaced(_) => not_applied(unplaced.to_string()),
        SpawnAtError::Failed(detail) => spawn_not_applied(detail),
    }
}

// ── 대상을 받는 동사(TRD S28 §4 · §5 · §7) ──────────────────────────────────────
//
// 차례 = 인자 검문(락 없음) → 창 인자 풀기 + 적용 — 본문 전체가 블로킹 풀에서 돈다([`targeted_handler`]). 에이전트 칸은 데몬이
// 중계 전에 이미 id 로 바꿨다([`agent_arg`] · ADR-0308). 대상(id · 에이전트 · 낱말 → 탭 · 칸)은 적용 서비스가 **바꾸는 그 락
// 안에서** 풀고, 답의 창 · 탭 · 배치 줄도 결말을 짓는 그 락에서 뜬다(`apply::dock_target` · `DockOutcome::rows` ·
// `apply::Wrote`).
// ADR-0307

// 넓힌 칸 · 탭 동사와 `agent.spawnInto` 의 id 반려 문구가 안내하는 출처 — 탭 · 칸 id 를 함께 주는 조회.
const LAYOUT_IDS: &str = "layout.get with ids";

fn verb_tab_move(ports: &LayoutPorts, args: TabMoveArgs) -> Result<TabMoveOk, CommandError> {
    // `tab.dock` 의 `window` 와 헷갈린 호출이 말없이 제자리 순서 바꾸기가 되지 않게 받아서 거절한다.
    if args.window.is_some() {
        return Err(CommandError::invalid_argument(
            "tab.move's destination is to_window — it takes no window argument",
        ));
    }
    let subject = tab_choice(args.tab.as_deref(), args.tab_of.as_deref())?;
    let to_window = optional_window_arg("to_window", args.to_window.as_deref())?;
    let after = after_choice(args.after_tab.as_deref(), args.after_tab_of.as_deref())?;
    let index = args.index.map(|i| usize::try_from(i).unwrap_or(usize::MAX));
    match (&to_window, &after, index) {
        (Some(_), Some(after), _) => {
            return Err(CommandError::invalid_argument(format!(
                "give only one of to_window, after_tab, after_tab_of — they all say where the tab goes, got to_window and {}",
                after.arg
            )))
        }
        (None, Some(after), Some(_)) => {
            return Err(CommandError::invalid_argument(format!(
                "index does not apply with {} — the tab goes right after that tab",
                after.arg
            )))
        }
        (None, None, None) => {
            return Err(CommandError::invalid_argument(
                "say where the tab goes: to_window, after_tab, after_tab_of, or index (a place in its own window's tab bar, from 0)",
            ))
        }
        _ => {}
    }
    let tab = subject.tab_target()?;
    let spot = match &after {
        Some(after) => TabBarSpot::After(after.tab_target()?),
        None => TabBarSpot::At {
            window: to_window
                .as_deref()
                .map(|given| window_label(ports, "to_window", given))
                .transpose()?,
            index,
        },
    };
    let said = DockSaid {
        command: "tab.move",
        subject,
        dest: after,
    };
    let m = dock_by_target(ports, tab, TargetDest::TabBar(spot), &said)?;
    Ok(TabMoveOk {
        changed: m.changed,
        window: m.window,
        window_id: m.window_id,
        number: m.number,
        tab: m.tab,
        tab_index: m.tab_index,
        layout: m.layout,
    })
}

fn verb_tab_dock(ports: &LayoutPorts, args: TabDockArgs) -> Result<TabDockOk, CommandError> {
    let subject = tab_choice(args.tab.as_deref(), args.tab_of.as_deref())?;
    let beside = beside_choice(
        args.beside_slot.as_deref(),
        args.beside.as_deref(),
        args.beside_at.as_deref(),
    )?;
    let window = optional_window_arg("window", args.window.as_deref())?;
    if window.is_some() && !matches!(beside.by, ChoiceBy::At(_)) {
        return Err(CommandError::invalid_argument(
            "window only goes with beside_at — it names the window whose shown tab the word is read in",
        ));
    }
    let share = args.share.map(share_arg).transpose()?;
    let whole = args.whole.unwrap_or(false);
    let side = match args.side {
        DockSide::Left => dock::DockSide::Left,
        DockSide::Right => dock::DockSide::Right,
        DockSide::Top => dock::DockSide::Top,
        DockSide::Bottom => dock::DockSide::Bottom,
    };
    let tab = subject.tab_target()?;
    let window = window
        .as_deref()
        .map(|given| window_label(ports, "window", given))
        .transpose()?;
    let to = TargetDest::Seam {
        beside: beside.slot_target(whole, window)?,
        whole,
        side,
        share,
    };
    let said = DockSaid {
        command: "tab.dock",
        subject,
        dest: Some(beside),
    };
    let m = dock_by_target(ports, tab, to, &said)?;
    Ok(TabDockOk {
        changed: m.changed,
        window: m.window,
        window_id: m.window_id,
        number: m.number,
        tab: m.tab,
        tab_index: m.tab_index,
        layout: m.layout,
    })
}

fn verb_tab_detach(ports: &LayoutPorts, args: TabDetachArgs) -> Result<TabDetachOk, CommandError> {
    let subject = tab_choice(args.tab.as_deref(), args.tab_of.as_deref())?;
    let tab = subject.tab_target()?;
    // 자리 · 크기를 고르지 않는다 — 새 창은 label 기본 자리 · 적용 서비스가 고른 크기이고, 혼자인 팝아웃(재사용)은 받은
    //   자리로 창을 옮기므로 비워야 그 창이 제자리에 남는다(TRD S28 §4-3).
    let to = TargetDest::NewWindow {
        at: WindowPlace {
            pos: None,
            size: None,
        },
    };
    let said = DockSaid {
        command: "tab.detach",
        subject,
        dest: None,
    };
    let m = dock_by_target(ports, tab, to, &said)?;
    Ok(TabDetachOk {
        changed: m.changed,
        window: m.window,
        window_id: m.window_id,
        number: m.number,
        tab: m.tab,
        tab_index: m.tab_index,
        layout: m.layout,
    })
}

fn verb_layout_get(ports: &LayoutPorts, args: LayoutGetArgs) -> Result<LayoutGetOk, CommandError> {
    let window = optional_window_arg("window", args.window.as_deref())?;
    let agent = optional_text("agent", args.agent.as_deref())?
        .map(|given| agent_arg("agent", given))
        .transpose()?;
    let with_ids = args.ids.unwrap_or(false);
    let window = window
        .as_deref()
        .map(|given| window_label(ports, "window", given))
        .transpose()?;
    // 이 읽기의 실패는 락 오염뿐이다 — 상태 탓(CONFLICT)이 아니다.
    let rows = apply::layout_rows(&ports.state, &RowFilter { window, agent })
        .map_err(CommandError::internal)?;
    Ok(LayoutGetOk {
        rows: rows.into_iter().map(|row| row_of(row, with_ids)).collect(),
    })
}

// ── 넓힌 탭 동사(TRD S28 §7) — 탭 = view_id | tab_of · window 는 빼면 탭에서 안다 ──

fn verb_tab_switch(ports: &LayoutPorts, args: TabSwitchArgs) -> Result<TabSwitchOk, CommandError> {
    let tab = view_choice(args.view_id.as_deref(), args.tab_of.as_deref())?;
    let window = optional_window_arg("window", args.window.as_deref())?;
    let target = tab.tab_target()?;
    let window = window
        .as_deref()
        .map(|given| window_label(ports, "window", given))
        .transpose()?;
    apply::switch_tab_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        window.as_deref(),
        &target,
    )
    .map_err(|error| target_error(&ports.state, error, &tab))?;
    Ok(TabSwitchOk {})
}

fn verb_tab_close(ports: &LayoutPorts, args: TabCloseArgs) -> Result<TabCloseOk, CommandError> {
    let tab = view_choice(args.view_id.as_deref(), args.tab_of.as_deref())?;
    let window = optional_window_arg("window", args.window.as_deref())?;
    let target = tab.tab_target()?;
    let window = window
        .as_deref()
        .map(|given| window_label(ports, "window", given))
        .transpose()?;
    let closed = apply::close_tab_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.windows.as_ref(),
        window.as_deref(),
        &target,
    )
    .map_err(|error| target_error(&ports.state, error, &tab))?;
    Ok(TabCloseOk {
        layout: layout_of(closed.rows),
    })
}

fn verb_tab_rename(ports: &LayoutPorts, args: TabRenameArgs) -> Result<TabRenameOk, CommandError> {
    let tab = view_choice(args.view_id.as_deref(), args.tab_of.as_deref())?;
    let name = text("name", &args.name)?.trim().to_string();
    let target = tab.tab_target()?;
    apply::rename_tab_at(&ports.state, ports.events.as_ref(), &target, name)
        .map_err(|error| target_error(&ports.state, error, &tab))?;
    Ok(TabRenameOk {})
}

// ── 넓힌 칸 동사(TRD S28 §7) — 칸 = slot_id(+view_id) | slot_of | at(+window) ──

fn verb_slot_focus(ports: &LayoutPorts, args: SlotFocusArgs) -> Result<SlotFocusOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let target = pick.target(ports)?;
    let focused = apply::focus_slot_at(&ports.state, ports.events.as_ref(), &target)
        .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    Ok(SlotFocusOk {
        layout: layout_of(focused.rows),
    })
}

fn verb_slot_split(ports: &LayoutPorts, args: SlotSplitArgs) -> Result<SlotSplitOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let dir = match args.dir {
        SplitDirection::LeftRight => SplitDir::LeftRight,
        SplitDirection::TopBottom => SplitDir::TopBottom,
    };
    let target = pick.target(ports)?;
    let split = apply::split_slot_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        &target,
        dir,
    )
    .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    Ok(SlotSplitOk {
        slot_id: split.out.to_string(),
        layout: layout_of(split.rows),
    })
}

fn verb_slot_close(ports: &LayoutPorts, args: SlotCloseArgs) -> Result<SlotCloseOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let target = pick.target(ports)?;
    let closed = apply::close_slot_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        &target,
    )
    .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    Ok(SlotCloseOk {
        layout: layout_of(closed.rows),
    })
}

fn verb_slot_popout(
    ports: &LayoutPorts,
    args: SlotPopoutArgs,
) -> Result<SlotPopoutOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let to_window = optional_window_arg("to_window", args.to_window.as_deref())?;
    let target = pick.target(ports)?;
    let to_window = to_window
        .as_deref()
        .map(|given| window_label(ports, "to_window", given))
        .transpose()?;
    let popped = apply::move_slot_to_window_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.windows.as_ref(),
        ports.labels.as_ref(),
        &target,
        to_window,
    )
    .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    // ★`view_id` 라 부르지 않는다★ — 이 명령의 인자에 이미 `view_id`(떼어낼 **원본** 탭)가 있어서, 답의 같은
    //   이름은 반대쪽을 뜻하게 된다. 답을 그대로 되먹여 두 번 부르는 호출자는 원본 대신 방금 만든 탭을
    //   집어 엉뚱한 뷰에서 슬롯을 떼어낸다. `new_` 접두는 그 혼동을 막으면서도 값이 view id 임을 남겨
    //   `tab.rename`·`tab.switch` 의 `view_id` 에 그대로 꽂힌다. 서비스 쪽 이름은 `tab` 이다(프론트 wire).
    Ok(SlotPopoutOk {
        window: popped.out.window,
        new_view_id: popped.out.tab.to_string(),
        window_id: popped.out.window_id,
        number: popped.out.number,
        layout: layout_of(popped.rows),
    })
}

fn verb_slot_assign_agent(
    ports: &LayoutPorts,
    args: SlotAssignAgentArgs,
) -> Result<SlotAssignAgentOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let agent = Assignee::read(args.agent_id.as_deref(), args.assign.as_deref())?.ok_or_else(
        || {
            CommandError::invalid_argument(
                "the agent to put in the slot is missing — give one of assign (its name or id), agent_id",
            )
        },
    )?;
    let target = pick.target(ports)?;
    let assigned = apply::assign_agent_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        &target,
        agent.id.to_string(),
    )
    .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    Ok(SlotAssignAgentOk {
        layout: layout_of(assigned.rows),
    })
}

fn verb_set_slot_content(
    ports: &LayoutPorts,
    args: LayoutSetSlotContentArgs,
) -> Result<LayoutSetSlotContentOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let agent = Assignee::read(args.agent_id.as_deref(), args.assign.as_deref())?;
    let write = slot_content(args.content, agent, args.show_claude, args.show_codex)?;
    let target = pick.target(ports)?;
    let (state, subs, events) = (&ports.state, ports.subs.as_ref(), ports.events.as_ref());
    let written = match write {
        SlotContentWrite::Replace(content) => {
            apply::set_slot_content_at(state, subs, events, &target, content)
        }
        SlotContentWrite::Usage {
            show_claude,
            show_codex,
        } => apply::set_usage_slot_at(state, subs, events, &target, show_claude, show_codex),
    }
    .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    Ok(LayoutSetSlotContentOk {
        layout: layout_of(written.rows),
    })
}

// ── 고른 말 ──

/// 탭 · 칸을 고른 말 — 같은 것을 고르는 인자 한 줄에서 정확히 하나를 받은 것. `arg` · `word` = 받은 인자 이름과 받은
/// 그대로의 값(오류 문구가 되짚는다) · `id_arg` = 그 줄의 id 인자(모호할 때 후보를 넣을 자리 — 탭 줄이면 후보가 탭 단위다) ·
/// `agent_arg` = 그 줄에서 에이전트로 고르는 인자(낱말이 칸을 못 집었을 때 권한다 — 없으면 `None`).
struct Choice {
    arg: &'static str,
    id_arg: &'static str,
    agent_arg: Option<&'static str>,
    word: String,
    by: ChoiceBy,
}

#[derive(Clone, Copy)]
enum ChoiceBy {
    Id(Uuid),
    /// 그 에이전트가 든 탭 · 칸 — 데몬이 이름을 바꿔 넣은 에이전트 id(ADR-0308).
    Agent(Uuid),
    At(SpatialToken),
}

impl Choice {
    fn spatial(
        arg: &'static str,
        id_arg: &'static str,
        agent_arg: Option<&'static str>,
        word: &str,
        token: SpatialToken,
    ) -> Self {
        Choice {
            arg,
            id_arg,
            agent_arg,
            word: word.to_string(),
            by: ChoiceBy::At(token),
        }
    }

    // 탭을 고르는 줄인가 — 모호 후보를 탭 단위로 싣는다.
    fn names_a_tab(&self) -> bool {
        matches!(self.id_arg, "tab" | "after_tab" | "view_id")
    }

    fn tab_target(&self) -> Result<Target, CommandError> {
        match self.by {
            ChoiceBy::Id(id) => Ok(Target::Tab(id)),
            ChoiceBy::Agent(agent) => Ok(Target::TabOf(agent)),
            ChoiceBy::At(_) => Err(CommandError::internal(format!(
                "{} cannot name a tab",
                self.arg
            ))),
        }
    }

    // `window` = 낱말을 읽을 창(이미 푼 label · `None` = main).
    fn slot_target(&self, whole: bool, window: Option<String>) -> Result<Target, CommandError> {
        match self.by {
            ChoiceBy::Id(slot) => Ok(Target::Slot { view: None, slot }),
            // 탭 전체의 바깥 변에 붙이면 기준이 탭이라 그 탭 안 여러 칸에 떠 있어도 모호하지 않다.
            ChoiceBy::Agent(agent) if whole => Ok(Target::TabOf(agent)),
            ChoiceBy::Agent(agent) => Ok(Target::SlotOf(agent)),
            ChoiceBy::At(token) => Ok(Target::At { token, window }),
        }
    }
}

/// 탭을 고르는 줄 — `id_arg`(그 명령의 탭 id 인자) · `tab_of` 중 정확히 하나. `what` = 그 줄이 고르는 것(문구용) · `source` =
/// 그 id 를 돌려주는 명령(반려 문구가 안내한다).
fn tab_line(
    id_arg: &'static str,
    what: &str,
    source: &str,
    id: Option<&str>,
    tab_of: Option<&str>,
) -> Result<Choice, CommandError> {
    let (index, arg, word) = one_of(&[(id_arg, id), ("tab_of", tab_of)], what)?;
    let by = match index {
        0 => ChoiceBy::Id(id_choice(
            arg,
            word,
            source,
            "tab_of (an agent shown in that tab)",
        )?),
        _ => ChoiceBy::Agent(agent_arg(arg, word)?),
    };
    Ok(Choice {
        arg,
        id_arg,
        agent_arg: Some("tab_of"),
        word: word.to_string(),
        by,
    })
}

/// 옮길 탭 줄(`tab` · `tab_of`) — 도킹 동사.
fn tab_choice(tab: Option<&str>, tab_of: Option<&str>) -> Result<Choice, CommandError> {
    tab_line(
        "tab",
        "the tab to move",
        "tab.list / layout.get with ids",
        tab,
        tab_of,
    )
}

/// 바꿀 탭 줄(`view_id` · `tab_of`) — 넓힌 탭 동사.
fn view_choice(view_id: Option<&str>, tab_of: Option<&str>) -> Result<Choice, CommandError> {
    tab_line("view_id", "the tab", LAYOUT_IDS, view_id, tab_of)
}

/// 「그 탭 바로 뒤」 줄(`after_tab` · `after_tab_of`) — 하나도 없으면 `None`(갈 곳 줄의 나머지는 `to_window` · `index`).
fn after_choice(
    after_tab: Option<&str>,
    after_tab_of: Option<&str>,
) -> Result<Option<Choice>, CommandError> {
    let after_tab = optional_text("after_tab", after_tab)?;
    let after_tab_of = optional_text("after_tab_of", after_tab_of)?;
    let (arg, word, by) = match (after_tab, after_tab_of) {
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => {
            return Err(CommandError::invalid_argument(
                "give only one of to_window, after_tab, after_tab_of — they all say where the tab goes, got after_tab and after_tab_of",
            ))
        }
        (Some(word), None) => (
            "after_tab",
            word,
            ChoiceBy::Id(id_choice(
                "after_tab",
                word,
                "tab.list / layout.get with ids",
                "after_tab_of (an agent shown in that tab)",
            )?),
        ),
        (None, Some(word)) => (
            "after_tab_of",
            word,
            ChoiceBy::Agent(agent_arg("after_tab_of", word)?),
        ),
    };
    Ok(Some(Choice {
        arg,
        id_arg: "after_tab",
        agent_arg: Some("after_tab_of"),
        word: word.to_string(),
        by,
    }))
}

/// 붙일 칸 줄(`beside_slot` · `beside` · `beside_at`).
fn beside_choice(
    slot: Option<&str>,
    agent: Option<&str>,
    at: Option<&str>,
) -> Result<Choice, CommandError> {
    let (index, arg, word) = one_of(
        &[("beside_slot", slot), ("beside", agent), ("beside_at", at)],
        "the slot to attach beside",
    )?;
    let by = match index {
        0 => ChoiceBy::Id(id_choice(
            arg,
            word,
            "layout.get with ids",
            "beside (an agent shown in that slot) or beside_at (a spatial word)",
        )?),
        1 => ChoiceBy::Agent(agent_arg(arg, word)?),
        _ => ChoiceBy::At(spatial_token(arg, word)?),
    };
    Ok(Choice {
        arg,
        id_arg: "beside_slot",
        agent_arg: Some("beside"),
        word: word.to_string(),
        by,
    })
}

/// 칸을 고르는 줄(`slot_id` · `slot_of` · `at`)과 그 곁 인자 — 넓힌 칸 동사가 함께 쓴다(TRD S28 §7). `view` = `slot_id` 와
/// 함께 준 `view_id`(그 탭의 칸이어야 한다) · `window` = `at` 을 읽을 창(모양만 본 인자 — 모델 안 창으로는 적용 직전에 푼다).
struct SlotPick {
    said: Choice,
    view: Option<Uuid>,
    window: Option<String>,
}

impl SlotPick {
    fn read(
        view_id: Option<&str>,
        slot_id: Option<&str>,
        slot_of: Option<&str>,
        at: Option<&str>,
        window: Option<&str>,
    ) -> Result<Self, CommandError> {
        // 탭 id 를 먼저 본다 — 넓히기 전 id 호출자의 검문 차례 그대로.
        let view = optional_text("view_id", view_id)?
            .map(|raw| uuid_arg_from("view_id", raw, LAYOUT_IDS))
            .transpose()?;
        let (index, arg, word) = one_of(
            &[("slot_id", slot_id), ("slot_of", slot_of), ("at", at)],
            "the slot",
        )?;
        let by = match index {
            0 => ChoiceBy::Id(id_choice(
                arg,
                word,
                LAYOUT_IDS,
                "slot_of (an agent shown in that slot) or at (a spatial word)",
            )?),
            1 => ChoiceBy::Agent(agent_arg(arg, word)?),
            _ => ChoiceBy::At(spatial_token(arg, word)?),
        };
        if view.is_some() && !matches!(by, ChoiceBy::Id(_)) {
            return Err(CommandError::invalid_argument(
                "view_id only goes with slot_id — it names the tab that slot is in",
            ));
        }
        let window = optional_window_arg("window", window)?;
        if window.is_some() && !matches!(by, ChoiceBy::At(_)) {
            return Err(CommandError::invalid_argument(
                "window only goes with at — it names the window whose shown tab the word is read in",
            ));
        }
        Ok(SlotPick {
            said: Choice {
                arg,
                id_arg: "slot_id",
                agent_arg: Some("slot_of"),
                word: word.to_string(),
                by,
            },
            view,
            window,
        })
    }

    // 적용 직전 — 창 인자를 모델 안 창으로 풀어 대상을 짓는다.
    fn target(&self, ports: &LayoutPorts) -> Result<Target, CommandError> {
        if let ChoiceBy::Id(slot) = self.said.by {
            return Ok(Target::Slot {
                view: self.view,
                slot,
            });
        }
        let window = self
            .window
            .as_deref()
            .map(|given| window_label(ports, "window", given))
            .transpose()?;
        self.said.slot_target(false, window)
    }
}

/// 칸에 넣을 에이전트 — `assign` | `agent_id`(넓히기 전 인자) 중 하나. 둘 다 `#[agent_ref]` 라 데몬이 명부로 풀어 id 를
/// 넣고, 명부에 없으면 데몬이 거절해 여기 오지 않는다(ADR-0308). `arg` = 받은 인자 이름(문구용).
struct Assignee {
    arg: &'static str,
    id: Uuid,
}

impl Assignee {
    fn read(agent_id: Option<&str>, assign: Option<&str>) -> Result<Option<Self>, CommandError> {
        let agent_id = optional_text("agent_id", agent_id)?;
        let assign = optional_text("assign", assign)?;
        let (arg, word) = match (agent_id, assign) {
            (None, None) => return Ok(None),
            (Some(_), Some(_)) => return Err(CommandError::invalid_argument(
                "give only one of assign, agent_id — they both name the agent to put in the slot",
            )),
            (Some(word), None) => ("agent_id", word),
            (None, Some(word)) => ("assign", word),
        };
        Ok(Some(Assignee {
            arg,
            id: agent_arg(arg, word)?,
        }))
    }
}

/// `#[agent_ref]` 칸의 값 → 에이전트 id. 데몬이 중계 전에 이름을 id 로 바꿔 넣으므로(ADR-0308) UUID 가 아니면 데몬을
/// 거치지 않은 호출이다 — 셸은 이름을 풀지 않고 반려한다(해석기는 데몬 하나 — ADR-0132).
// ADR-0308
fn agent_arg(field: &str, given: &str) -> Result<Uuid, CommandError> {
    Uuid::parse_str(given.trim()).map_err(|_| {
        CommandError::invalid_argument(format!(
            "{field} must be an agent id (a UUID) here, got {given:?} — give the agent's name or id; names are turned into ids by the daemon before a command reaches this app"
        ))
    })
}

/// id 인자 — UUID 가 아니면 [`uuid_arg_from`] 의 반려 끝에 이름 · 낱말로 고르는 짝 인자(`alternative`)를 단다(이름을
/// id 칸에 넣은 호출이 다음 한 번에 고치게).
fn id_choice(
    field: &str,
    given: &str,
    source: &str,
    alternative: &str,
) -> Result<Uuid, CommandError> {
    uuid_arg_from(field, given, source).map_err(|error| {
        CommandError::invalid_argument(format!(
            "{} — to name it another way, use {alternative}",
            error.message()
        ))
    })
}

/// 공간 낱말 — 모르는 낱말이면 받는 낱말을 나열해 반려한다.
fn spatial_token(arg: &str, word: &str) -> Result<SpatialToken, CommandError> {
    SpatialToken::parse(word).ok_or_else(|| {
        CommandError::invalid_argument(format!(
            "{arg} must be one of top-left, top-right, bottom-left, bottom-right, left, right, up, down — got {word:?}"
        ))
    })
}

/// 같은 것을 고르는 인자 한 줄에서 정확히 하나 — (줄 안 자리, 인자 이름, 값). 빈 값은 [`optional_text`] 처럼 반려한다.
/// `what` = 그 줄이 고르는 것(문구용).
fn one_of<'a>(
    fields: &[(&'static str, Option<&'a str>)],
    what: &str,
) -> Result<(usize, &'static str, &'a str), CommandError> {
    let mut given = Vec::new();
    for (index, (field, value)) in fields.iter().enumerate() {
        if let Some(value) = optional_text(field, *value)? {
            given.push((index, *field, value));
        }
    }
    let all = || {
        fields
            .iter()
            .map(|(field, _)| *field)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match given.as_slice() {
        [one] => Ok(*one),
        [] => Err(CommandError::invalid_argument(format!(
            "{what} is missing — give one of {}",
            all()
        ))),
        many => Err(CommandError::invalid_argument(format!(
            "give only one of {} — they all name {what}, got {}",
            all(),
            many.iter()
                .map(|(_, field, _)| *field)
                .collect::<Vec<_>>()
                .join(" and ")
        ))),
    }
}

/// 창 인자의 모양만 먼저 본다(빈 값 · 번호 = `INVALID_ARGUMENT`) — 모델 안 창으로 푸는 것은 적용 직전의 [`window_label`]
/// 이다(그때 없는 창 = `CONFLICT`).
fn optional_window_arg(field: &str, given: Option<&str>) -> Result<Option<String>, CommandError> {
    let Some(given) = optional_text(field, given)? else {
        return Ok(None);
    };
    window_arg::reject_number(given).map_err(window_arg_error)?;
    Ok(Some(given.to_string()))
}

// 범위는 모델 계약(`tree::RATIO_MIN..=dock::SHARE_MAX`) 그대로다 — 적용 서비스도 같은 범위로 거절하지만(`InvalidShare`)
//   적용 락을 잡기 전에 반려한다.
fn share_arg(share: f64) -> Result<f64, CommandError> {
    if share.is_finite() && (tree::RATIO_MIN..=dock::SHARE_MAX).contains(&share) {
        return Ok(share);
    }
    Err(CommandError::invalid_argument(format!(
        "share is the attached side's part, from {} to {}, got {share}",
        tree::RATIO_MIN,
        dock::SHARE_MAX
    )))
}

// ── 대상판 도킹의 결말 · 거절 ──

/// 대상판 도킹 거절을 코드 · 문구로 옮길 재료 — 어느 명령이 옮길 탭과 갈 곳을 무슨 말로 골랐나. `dest` = 갈 곳을 대상으로
/// 고른 말(`after_tab` · `after_tab_of` · 붙일 칸 줄 — 창 · 새 창은 `None`).
struct DockSaid {
    command: &'static str,
    subject: Choice,
    dest: Option<Choice>,
}

impl DockSaid {
    // 호출자가 id 로 준 것.
    fn gave_id(&self, id: Uuid) -> bool {
        std::iter::once(&self.subject)
            .chain(self.dest.as_ref())
            .any(|choice| matches!(choice.by, ChoiceBy::Id(given) if given == id))
    }
}

/// 세 도킹 명령이 함께 싣는 답 칸 — 전부 결말을 짓는 그 락의 한 모습이다(`DockOutcome::rows` · `identity`).
struct Moved {
    changed: bool,
    window: String,
    window_id: String,
    number: u32,
    tab: String,
    tab_index: Option<u32>,
    layout: Vec<LayoutRow>,
}

// PRD S21-slot-docking R11b: LLM 명령은 창 활성 · 보이는 탭 · 포커스를 안 바꾼다(`Keep` — 예외는 적용 서비스의 `dock` 승계).
fn dock_by_target(
    ports: &LayoutPorts,
    tab: Target,
    to: TargetDest,
    said: &DockSaid,
) -> Result<Moved, CommandError> {
    let outcome = apply::dock_target(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.windows.as_ref(),
        ports.labels.as_ref(),
        TargetDockRequest {
            tab,
            to,
            focus: FocusPolicy::Keep,
            mv: DockMove::default(),
        },
    )
    .map_err(|error| dock_target_error(&ports.state, error, said))?;
    moved(outcome)
}

fn moved(outcome: DockOutcome) -> Result<Moved, CommandError> {
    // 대상판은 결말을 짓는 락에서 옮긴 탭을 다시 찾고, 그 창이 사라졌으면 이미 `Moved` 로 거절했다(TRD S28 §4-4) — 이
    //   갈래는 그 계약이 깨졌을 때의 그물이다.
    let Some(identity) = outcome.identity else {
        return Err(CommandError::of(
            ErrorCode::Conflict,
            TargetDockError::Moved.to_string(),
        ));
    };
    // 결말의 줄은 `tab` 을 늘 싣는다(`views` 의 첫 탭 · 같은 락) — 없으면 그 계약이 깨진 것이다. 그래도 옮기기는 이미
    //   됐으므로 오류로 답하지 않는다(적용된 쓰기를 실패로 읽히게 하지 않는다 — ADR-0159) — 그 칸만 빼고 성공이다.
    let tab_index = outcome
        .rows
        .iter()
        .find(|row| row.tab == outcome.tab)
        .map(|row| row.tab_index);
    if tab_index.is_none() {
        tracing::warn!(
            tab = %outcome.tab,
            "도킹 결말의 줄에 결과 탭이 없다 — tab_index 를 빼고 성공으로 답한다"
        );
    }
    Ok(Moved {
        changed: outcome.changed,
        window: identity.window,
        window_id: identity.window_id,
        number: identity.number,
        tab: outcome.tab.to_string(),
        tab_index,
        layout: layout_of(outcome.rows),
    })
}

/// 대상판 도킹의 거절 → 버스 오류(TRD S28 §5-3).
///
/// - 대상을 못 풀었다 = [`locate_error`].
/// - `NotASlot` · `SelfTarget` · `InvalidShare` = INVALID_ARGUMENT — 인자 조합이 스스로 어긋난다. `SelfTarget` 문구는 명령마다
///   고칠 길을 말한다(`tab.dock` = 칸 하나는 `slot.popout` 으로 먼저 뺀다).
/// - 호출자가 **id 로 준** 탭 · 칸이 없음(`TabNotFound` · `SlotNotFound` · `SlotNotInTab`) = NOT_FOUND · 에이전트 · 낱말에서
///   나온 id 면 CONFLICT(풀린 뒤 사라졌다 — 경합).
/// - `WindowNotFound` = CONFLICT(ADR-0302 결정 9 — [`window_arg_error`] 와 같다) · `Moved` 와 나머지 = CONFLICT.
// ADR-0307
fn dock_target_error(state: &LayoutState, error: TargetDockError, said: &DockSaid) -> CommandError {
    let message = error.to_string();
    match error {
        TargetDockError::Subject(e) => locate_error(state, &said.subject, e),
        TargetDockError::Dest(e) => {
            locate_error(state, said.dest.as_ref().unwrap_or(&said.subject), e)
        }
        TargetDockError::NotASlot => CommandError::invalid_argument(message),
        TargetDockError::Moved => CommandError::of(ErrorCode::Conflict, message),
        TargetDockError::Dock(e) => dock_error(state, e, said),
    }
}

fn dock_error(state: &LayoutState, error: DockError, said: &DockSaid) -> CommandError {
    let code = match &error {
        DockError::SelfTarget | DockError::InvalidShare(_) => ErrorCode::InvalidArgument,
        DockError::TabNotFound(id)
        | DockError::SlotNotFound(id)
        | DockError::SlotNotInTab { slot: id, .. }
            if said.gave_id(*id) =>
        {
            ErrorCode::NotFound
        }
        _ => ErrorCode::Conflict,
    };
    let message = match (&error, &said.dest) {
        (DockError::SelfTarget, Some(dest)) if said.command == "tab.dock" => format!(
            "{}: both are in one tab; tab.dock moves whole tabs — slot.popout that slot first, then tab.dock",
            said_head(state, dest)
        ),
        (DockError::SelfTarget, Some(dest)) => format!(
            "{}: that is the tab being moved — a tab cannot go right after itself",
            said_head(state, dest)
        ),
        _ => error.to_string(),
    };
    CommandError::of(code, message)
}

/// 대상을 못 풀었다 → 버스 오류(TRD S28 §5-2 · §5-3). 문구 = [`said_head`] + `: <사유>` + 꼬리.
///
/// - 에이전트가 어느 칸에도 없음 = NOT_FOUND + ` — candidates: <떠 있는 이름>`(없으면 `none`).
/// - 에이전트가 여러 곳 = CONFLICT + 그 줄의 id 인자로 고치라는 안내 + 후보(최대 5). 후보 머리 = 그 id 인자 이름이라 값을
///   그대로 옮겨 적는다 — 탭 줄(`tab` · `after_tab` · `view_id`)은 탭마다 한 줄 `<인자>=<id> (window=<label>, tab_index=<n>)` ·
///   칸 줄(`slot_id` · `beside_slot`)은 칸마다 `<인자>=<id> (…)`.
/// - 낱말이 칸을 못 집음 = NOT_FOUND + 모서리 낱말 · 그 줄의 에이전트 인자(`beside` · `slot_of`) 안내 · 창이 없음 =
///   CONFLICT(ADR-0302 결정 9).
/// - id 로 준 탭 · 칸이 없음 = NOT_FOUND — 이 셋은 id 대상에서만 난다(풀린 뒤 사라진 것은 적용 단계의 [`dock_error`] 몫).
// ADR-0307
fn locate_error(state: &LayoutState, said: &Choice, error: LocateError) -> CommandError {
    let head = format!("{}: {error}", said_head(state, said));
    let (code, message) = match &error {
        LocateError::AgentNotShown { .. } => (
            ErrorCode::NotFound,
            format!("{head}{}", candidates_tail(&error.candidates_as(said.id_arg))),
        ),
        LocateError::AgentAmbiguous { places, .. } => {
            let candidates = if said.names_a_tab() {
                tab_candidates(said.id_arg, places)
            } else {
                error.candidates_as(said.id_arg)
            };
            (
                ErrorCode::Conflict,
                format!(
                    "{head}; pass one of these as {}{}",
                    said.id_arg,
                    candidates_tail(&candidates)
                ),
            )
        }
        LocateError::NoSlotAt { .. } => (
            ErrorCode::NotFound,
            format!(
                "{head} — left/right/up/down are read from the focused slot; try a corner word (top-left, top-right, bottom-left, bottom-right){}",
                said.agent_arg.map(|agent| format!(" or {agent}")).unwrap_or_default()
            ),
        ),
        LocateError::WindowNotFound(_) => (ErrorCode::Conflict, head),
        LocateError::TabNotFound(_)
        | LocateError::SlotNotFound(_)
        | LocateError::SlotNotInTab { .. } => (ErrorCode::NotFound, head),
    };
    CommandError::of(code, message)
}

/// 받은 말 머리 `<인자> '<받은 말>'` — 에이전트 칸이면 데몬이 바꿔 넣은 id 대신 `<인자> '<표시 이름>' (<id>)` 로 적는다(셸 이름
/// 장부 — 보여 주기 몫이지 풀이가 아니다 · ADR-0132). 장부에 없으면 id 만.
// ★락을 쥔 채 부르지 말 것★ — 이름 장부를 읽으려고 레이아웃 락을 잡는다(적용이 돌려준 뒤의 오류 매핑에서만 부른다).
fn said_head(state: &LayoutState, said: &Choice) -> String {
    match said.by {
        ChoiceBy::Agent(id) => match apply::agent_display_name(state, id) {
            Some(name) => format!("{} '{name}' ({id})", said.arg),
            None => format!("{} '{}'", said.arg, said.word),
        },
        ChoiceBy::Id(_) | ChoiceBy::At(_) => format!("{} '{}'", said.arg, said.word),
    }
}

// 탭 단위 모호 후보 — 탭마다 한 줄(그 탭의 첫 칸 자리), 최대 [`target::MAX_CANDIDATES`]. `key` = 그 값을 넘길 인자 이름.
fn tab_candidates(key: &str, places: &[Place]) -> Vec<String> {
    let mut seen: Vec<Uuid> = Vec::new();
    places
        .iter()
        .filter(|place| {
            let first = !seen.contains(&place.view);
            seen.push(place.view);
            first
        })
        .take(target::MAX_CANDIDATES)
        .map(|place| {
            format!(
                "{key}={} (window={}, tab_index={})",
                place.view, place.window, place.tab_index
            )
        })
        .collect()
}

// 데몬 해석기 문구와 같은 꼴의 후보 꼬리 — 비면 `none`(꼬리를 빼면 「후보가 없다」와 「후보를 싣지 않는 문구」가 갈리지
//   않는다).
fn candidates_tail(candidates: &[String]) -> String {
    if candidates.is_empty() {
        return " — candidates: none".to_string();
    }
    format!(" — candidates: {}", candidates.join(", "))
}

/// 대상판 칸 · 탭 쓰기의 거절 → 버스 오류(TRD S28 §5-3 · §7).
///
/// - id 로 준 탭 · 칸이 없음 = [`id_miss`](넓히기 전 답 그대로) · 에이전트 · 낱말로 고른 대상을 못 풀었음 = [`locate_error`].
/// - 대상은 풀었으나 적용이 거절됐음 = CONFLICT + 서비스 문구(id 판과 같다 — [`not_applied`]).
/// - 칸 쓰기에 탭 대상 = INTERNAL — 칸 동사는 칸 대상만 짓는다(그 계약이 깨졌을 때의 그물).
// ADR-0307
fn target_error(state: &LayoutState, error: TargetError, said: &Choice) -> CommandError {
    match error {
        TargetError::Locate(error) if matches!(said.by, ChoiceBy::Id(_)) => id_miss(error),
        TargetError::Locate(error) => locate_error(state, said, error),
        TargetError::NotASlot => CommandError::internal(TargetError::NotASlot.to_string()),
        TargetError::Apply(detail) => not_applied(detail),
    }
}

/// id 로 준 탭 · 칸이 없다 — 넓히기 전 id 판(관리자 쓰기)이 내던 문구 · 코드 그대로다(CONFLICT — 모듈 머리 「적용 실패의
/// 코드」): 없는 탭 = `ViewNotFound` · 그 탭에 없는 칸 = `SlotNotFound`. 대상판은 그 판정을 쓰기 전에 `locate` 로 먼저 하므로
/// 문구만 관리자의 것으로 되돌린다 — 짝이 갈리면 `tests/layout_commands.rs` 의 옛 문구 대조가 빨개진다.
fn id_miss(error: LocateError) -> CommandError {
    let detail = match error {
        LocateError::TabNotFound(view) => LayoutError::ViewNotFound(view).to_string(),
        LocateError::SlotNotFound(slot) | LocateError::SlotNotInTab { slot, .. } => {
            LayoutError::SlotNotFound(slot).to_string()
        }
        other => other.to_string(),
    };
    not_applied(detail)
}

// 쓰기가 바꾼 탭의 줄 — id 없이(규칙 §4 · 칸을 이름 · 낱말로 부른다).
fn layout_of(rows: Vec<rows::LayoutRow>) -> Vec<LayoutRow> {
    rows.into_iter().map(|row| row_of(row, false)).collect()
}

// ids = false 면 id 넷을 비운다 — 답에서 그 칸은 아예 빠진다([`without_nulls`]).
fn row_of(row: rows::LayoutRow, ids: bool) -> LayoutRow {
    LayoutRow {
        window: row.window,
        window_number: row.window_number,
        tab_index: row.tab_index,
        tab_name: row.tab_name,
        tab_active: row.tab_active,
        content: match row.content {
            rows::SlotKind::Empty => SlotKind::Empty,
            rows::SlotKind::Agent => SlotKind::Agent,
            rows::SlotKind::AgentList => SlotKind::AgentList,
            rows::SlotKind::PresetPalette => SlotKind::PresetPalette,
            rows::SlotKind::Usage => SlotKind::Usage,
            rows::SlotKind::Unknown => SlotKind::Unknown,
        },
        agent: row.agent,
        focused: row.focused,
        x: row.x,
        y: row.y,
        w: row.w,
        h: row.h,
        window_id: ids.then_some(row.window_id),
        tab: ids.then(|| row.tab.to_string()),
        slot_id: ids.then(|| row.slot.to_string()),
        agent_id: row.agent_id.filter(|_| ids),
    }
}

// ── 인자 검문 ────────────────────────────────────────────────────────────────

/// 적용 서비스의 실패 문구를 그대로 실어 나른다 — id 를 받는 동사의 갈래(헤더 「적용 실패의 코드」).
fn not_applied(detail: String) -> CommandError {
    CommandError::of(ErrorCode::Conflict, detail)
}

/// `agent.spawnInto` 의 실패 — 데몬이 `CODE: 문구` 로 답한 실패(저장소 거절 = `CONFLICT` · 저장 쓰기 실패 =
/// `INTERNAL` — daemon `connection_core` 의 `store_error_text`)는 그 코드를 겉 코드로 쓰고 접두를 뗀다. 재시도 지시는
/// 그 코드의 기본값이다(WS 문구는 지시를 싣지 않는다).
/// ★겉에 `CONFLICT` 를 또 씌우지 않는다★ — 코드가 둘(`CONFLICT: INTERNAL: …`)이 되고, 쓰기 실패가 「아무것도 적용
///   안 된 반려」로 읽힌다. 이 명령이 광고하지 않는 코드나 코드 접두가 없는 문구(배치 실패 · 정책 거절 등)는
///   [`not_applied`] 다.
// ADR-0291 R17
fn spawn_not_applied(detail: String) -> CommandError {
    let advertised = AgentSpawnIntoArgs::SPEC.advertised_errors();
    if let Some((prefix, message)) = detail.split_once(": ") {
        if let Some(code) = ErrorCode::from_wire(prefix).filter(|code| advertised.contains(code)) {
            return CommandError::of(code, message);
        }
    }
    not_applied(detail)
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

/// 창 인자를 지금 모델 안 창의 label 로 푼다(TRD S26-window-chrome §4-5) — 숫자만 = `INVALID_ARGUMENT` · 모델에 없는
/// 창(앞 실행의 label 포함) = `CONFLICT`(적용 실패와 같은 코드). 레이아웃 락은 푸는 동안만 쥔다 — 그 뒤 창이 닫히면
/// 적용 서비스가 같은 `CONFLICT` 로 거절한다.
fn window_label(ports: &LayoutPorts, field: &str, given: &str) -> Result<String, CommandError> {
    let given = text(field, given)?;
    let mgr = ports
        .state
        .0
        .lock()
        .map_err(|e| not_applied(e.to_string()))?;
    window_arg::resolve(&mgr, given).map_err(window_arg_error)
}

fn optional_window_label(
    ports: &LayoutPorts,
    field: &str,
    given: Option<&str>,
) -> Result<Option<String>, CommandError> {
    optional_text(field, given)?
        .map(|given| window_label(ports, field, given))
        .transpose()
}

fn window_arg_error(error: WindowArgError) -> CommandError {
    let message = error.to_string();
    match error {
        WindowArgError::NumberGiven => CommandError::invalid_argument(message),
        WindowArgError::NotFound(_) => not_applied(message),
    }
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
    uuid_arg_from(field, given, "tab.list / layout.get with ids")
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
/// [`SlotContent`] 가 된다(`apply::set_usage_slot_at`).
enum SlotContentWrite {
    Replace(SlotContent),
    Usage {
        show_claude: Option<bool>,
        show_codex: Option<bool>,
    },
}

/// 태그 + 곁칸(넣을 에이전트 `assign` · `agent_id` · `show_claude`·`show_codex`) → 슬롯에 쓸 것.
///
/// ★어긋난 조합을 조용히 고치지 않는다★: `Agent` 인데 에이전트가 없으면 빈 슬롯이 되고, `Empty` 인데 에이전트가
/// 붙어 있으면 호출자는 그 에이전트가 붙은 줄 안다. `Usage` 가 아닌데 `show_*` 가 붙어 있으면 호출자는 그
/// 회사를 켠 줄 안다. 셋 다 반려한다.
fn slot_content(
    kind: SlotContentKind,
    agent: Option<Assignee>,
    show_claude: Option<bool>,
    show_codex: Option<bool>,
) -> Result<SlotContentWrite, CommandError> {
    let shows = show_claude.is_some() || show_codex.is_some();
    let write = match (kind, agent) {
        (SlotContentKind::Agent, Some(agent)) => SlotContentWrite::Replace(SlotContent::Agent {
            agent_id: agent.id.to_string(),
        }),
        (SlotContentKind::Agent, None) => {
            return Err(CommandError::invalid_argument(
                "content=Agent needs assign (the agent's name or id) or agent_id — use slot.assignAgent if you only want to attach a running agent",
            ))
        }
        (_, Some(agent)) => {
            return Err(CommandError::invalid_argument(format!(
                "{} only applies to content=Agent, so drop this field",
                agent.arg
            )))
        }
        (SlotContentKind::Usage, None) => {
            return Ok(SlotContentWrite::Usage {
                show_claude,
                show_codex,
            })
        }
        (SlotContentKind::Empty, None) => SlotContentWrite::Replace(SlotContent::Empty),
        (SlotContentKind::AgentList, None) => SlotContentWrite::Replace(SlotContent::AgentList),
        (SlotContentKind::PresetPalette, None) => {
            SlotContentWrite::Replace(SlotContent::PresetPalette)
        }
    };
    if shows {
        return Err(CommandError::invalid_argument(
            "show_claude/show_codex only apply to content=Usage, so drop these fields",
        ));
    }
    Ok(write)
}

// ★단언은 대부분 `tests/layout_commands.rs` 에 있다★ — 봉투가 표를 지나는 길로 잰다. 여기 단위 테스트는 그 길로
//   만들 수 없는 결말(적용 서비스 계약이 깨진 그물)만 잰다. 이 패키지의 `#[cfg(test)]` 단언은
//   `cargo test -p engram-dashboard --test lib_unit` 으로 돈다(현황 = CLAUDE.md 「빌드·검증 명령」의 그 줄 · 그 타깃을 세운
//   결정 = ADR-0174).
#[cfg(test)]
mod tests {
    use super::*;

    // 결말의 줄에 결과 탭이 없어도(적용 서비스 계약이 깨진 그물) 이미 옮긴 도킹은 성공으로 답한다 — 그 칸만 빠진다.
    #[test]
    fn a_dock_whose_rows_miss_its_tab_still_answers_success_without_tab_index() {
        let tab = Uuid::new_v4();
        let outcome = DockOutcome {
            changed: true,
            tab,
            views: vec![tab],
            window: "main".to_string(),
            slot_ids: Vec::new(),
            emptied: None,
            main_refilled: None,
            reused: false,
            prewarmed: false,
            identity: Some(apply::NewWindow {
                window: "main".to_string(),
                window_id: "main".to_string(),
                number: 1,
            }),
            rows: Vec::new(),
        };

        let m = moved(outcome).expect("성공");

        assert!(m.changed);
        assert_eq!(m.tab, tab.to_string());
        assert_eq!(m.tab_index, None);
        let answer = serde_json::to_value(TabMoveOk {
            changed: m.changed,
            window: m.window,
            window_id: m.window_id,
            number: m.number,
            tab: m.tab,
            tab_index: m.tab_index,
            layout: m.layout,
        })
        .map(without_nulls)
        .expect("JSON");
        assert!(answer.get("tab_index").is_none(), "{answer}");
    }
}
