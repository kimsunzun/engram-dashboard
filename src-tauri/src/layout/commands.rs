//! `window`/`tab`/`slot` 명령 — **선언과 본문이 적용 서비스 옆**에 산다(ADR-0140 결정 1).
//!
//! 사람 클릭(`commands/layout.rs` 의 `#[tauri::command]`)과 중계된 LLM 호출(인바운드 수신기)이 **같은
//! 적용 서비스**(`super::apply`)에 떨어진다 — 이 파일은 그 서비스로 가는 **두 번째 껍데기**이지 두 번째
//! 제어 표면이 아니다(ADR-0081 결정 3 · 「거부한 대안」의 `ViewManager` 직접 호출을 여기서도 하지 않는다).
//!
//! 진입점: [`make_table`](fn.make_table.html)(조립) · [`LayoutPorts`](struct.LayoutPorts.html)(주입 seam).
//!
//! ## ★선언이 서비스와 갈리는 세 자리(알고 남긴 것)★
//! 선언 매크로의 타입 알파벳이 `Uuid`·중첩 enum·재귀 타입을 못 실어서 생긴 번역이다 — 매크로 제약이
//! 원인이고 계약을 새로 만든 것이 아니다.
//! - **id 는 전부 `String`** 이고 핸들러가 파싱한다(형식 불량 = `INVALID_ARGUMENT`).
//! - **`SlotContent` 는 태그 + `agent_id` 두 칸으로 펴서 받는다** — 매크로의 enum 은 필드 없는 variant 만
//!   싣는다. 조합 검사는 [`slot_content`] 가 한다.
//! - **`get_view` 는 선언에 없다** — 반환이 `ViewSnapshot`(재귀 `LayoutNode`)이고 매크로가 재귀 타입에서
//!   컴파일 에러로 멈춘다. 그래서 v1 조회는 `tab.list`·`window.list`·`slot.resolveSpatial` 셋이다.
//!
//! ## ★적용 실패는 코드 하나로 나간다(`CONFLICT`)★
//! 적용 서비스가 실패를 `String` 으로만 주므로 여기서 종류를 가를 재료가 없다. 문구로 코드를 합성하는
//! 것은 금지다(TRD §4-⑦ — `message` 로 기계 분기하지 않는다. CLI 의 문자열 패턴매칭을 끝내려고 둔 계약이
//! 바로 그것이다). 그래서 **모든 실패에 참인 코드**를 고른다: `CONFLICT` = 「지금 상태로는 그 요청을 적용할
//! 수 없다」. `NOT_FOUND` 는 main 창 거부에 거짓이고 `INTERNAL` 은 오타 난 id 에 거짓이다. 사유는 문구가
//! 그대로 나른다. 종류를 가르려면 적용 서비스가 타입드 오류를 내야 하고 그건 이 파일의 결정이 아니다.
// ADR-0140
// ADR-0081

use std::sync::Arc;

use uuid::Uuid;

use engram_dashboard_command::{
    blocking_handler, declare_commands, CommandError, CommandFuture, CommandHandler, CommandTable,
    ErrorCode,
};

use super::apply;
use super::{
    AgentSpawner, LabelSource, LayoutEvents, LayoutState, SlotContent, SplitDir, SubscriptionSync,
    WindowHost,
};

// ★이름은 프론트 레지스트리가 **오늘 등록한 id** 를 그대로 쓴다★: `tab.create`·`slot.focus`·
//   `layout.setSlotContent`·`agent.spawnInto` 는 `src/commands/*Commands.ts` 에 실재하는 id 다(실측
//   2026-08-17). 여기서 다른 철자를 지으면 같은 동작에 이름이 둘이 되고, 화면 몫 등록(TRD §6 Step 4 —
//   그 id 를 바꾸지 않는다고 적은 자리)이 그 둘을 다 지고 간다.
// ★새로 짓는 이름은 셋뿐★ — `tab.list`·`window.list`(조회는 프론트 id 가 없다)와 `slot.split`(프론트는
//   방향을 이름에 박아 `slot.split.h`/`slot.split.v` 둘로 두지만, 버스에서는 방향이 **인자**다 — 그래야
//   호출자가 방향을 값으로 고른다).
declare_commands! {
    catalog_version: 1;

    /// 탭 바 한 칸.
    struct TabRow {
        id: String,
        name: String,
    }

    /// 나눌 방향.
    ///
    /// ★철자가 Tauri invoke 경로(`horizontal`/`vertical`)와 다르다★ — 선언 매크로가 serde rename 을 못
    /// 달아 Rust variant 이름이 그대로 wire 값이 된다. 두 표면이 같은 뜻을 다른 철자로 받는 것은 알고
    /// 남긴 것이고, 변환은 핸들러가 한다.
    enum SplitDirection {
        Horizontal,
        Vertical,
    }

    /// 슬롯에 무엇을 담나 — `Agent` 만 `agent_id` 를 함께 받는다.
    enum SlotContentKind {
        Empty,
        Agent,
        AgentList,
        PresetPalette,
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

    /// 포커스를 그 슬롯으로 옮긴다(출력 라우팅은 안 바뀐다).
    #[effect(Write)]
    #[since(1)]
    "slot.focus" => args SlotFocusArgs {
        view_id: String,
        slot_id: String,
    } -> ok SlotFocusOk {} errors [CONFLICT];

    /// 이미 살아 있는 에이전트를 그 슬롯에 붙인다(새로 띄우지 않는다 — 띄우려면 agent.spawnInto).
    #[effect(Write)]
    #[since(1)]
    "slot.assignAgent" => args SlotAssignAgentArgs {
        view_id: String,
        slot_id: String,
        agent_id: String,
    } -> ok SlotAssignAgentOk {} errors [CONFLICT];

    /// 슬롯이 무엇을 보여줄지 바꾼다. content=Agent 일 때만 agent_id 를 함께 준다.
    #[effect(Write)]
    #[since(1)]
    "layout.setSlotContent" => args LayoutSetSlotContentArgs {
        view_id: String,
        slot_id: String,
        content: SlotContentKind,
        agent_id: Option<String>,
    } -> ok LayoutSetSlotContentOk {} errors [CONFLICT];

    /// 에이전트를 새로 띄우고 그 자리에 배치한다(스폰 + 필요하면 새 탭 + 슬롯 배정).
    /// view_id 를 빼면 새 탭을 만들어 거기 넣는다 — 그 경우 slot_id 는 줄 수 없다.
    /// backend 는 아직 고를 수 없다(값을 적으면 스폰 전에 거부된다 — 데몬 wire 가 기본 백엔드만 띄운다).
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
}

/// 레이아웃 명령이 잡는 실물 전량 — ★조립 때 주입된다★(ADR-0140 결정 5 / 규칙 T-1).
///
/// 포트 넷의 계약(어느 것이 락 안이고 어느 것이 락 밖인가)은 적용 서비스가 소유한다 — 이 구조체는 그것을
/// **소유형으로** 들고 있을 뿐이다. `#[tauri::command]` 쪽이 빌려 쓰는 어댑터를 `Arc` 로 바꾼 것이 차이의
/// 전부이고, 그렇게 하는 이유는 표의 핸들러가 `'static` 이어야 하기 때문이다.
pub struct LayoutPorts {
    pub state: LayoutState,
    pub subs: Arc<dyn SubscriptionSync>,
    pub events: Arc<dyn LayoutEvents>,
    pub windows: Arc<dyn WindowHost>,
    pub labels: Arc<dyn LabelSource>,
    pub spawner: Arc<dyn AgentSpawner>,
}

/// 셸의 레이아웃 표를 조립한다 — ★핸들러 실물이 들어오는 유일한 자리★(규칙 T-1).
///
/// ★명령이 늘어도 조립부(`lib.rs`)는 안 바뀐다★ — 늘어나는 것은 선언 블록과 이 함수의 한 줄이다.
// ADR-0140
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
    // ★유일한 async 핸들러★ — 스폰이 데몬 왕복이라 `blocking_handler` 로 감쌀 수 없다(그 어댑터는 본문이
    //   첫 poll 에서 끝까지 도는 것을 계약으로 삼는다).
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
        SplitDirection::Horizontal => SplitDir::Horizontal,
        SplitDirection::Vertical => SplitDir::Vertical,
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
    let content = slot_content(args.content, args.agent_id.as_deref())?;
    apply::set_slot_content(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        view,
        slot,
        content,
    )
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
    let cwd = text("cwd", &args.cwd)?;
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
        cwd.to_string(),
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
/// 건드린다(core `agent.*` 의 같은 조항).
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
    Uuid::parse_str(given.trim()).map_err(|_| {
        CommandError::invalid_argument(format!(
            "{field} must be a UUID (the id shown by tab.list / slot.resolveSpatial), got {given:?}"
        ))
    })
}

/// 태그 + `agent_id` 두 칸 → 하나의 [`SlotContent`].
///
/// ★어긋난 조합을 조용히 고치지 않는다★: `Agent` 인데 id 가 없으면 빈 슬롯이 되고, `Empty` 인데 id 가
/// 붙어 있으면 호출자는 그 에이전트가 붙은 줄 안다. 둘 다 반려한다.
fn slot_content(
    kind: SlotContentKind,
    agent_id: Option<&str>,
) -> Result<SlotContent, CommandError> {
    let agent_id = optional_text("agent_id", agent_id)?;
    match (kind, agent_id) {
        (SlotContentKind::Agent, Some(id)) => Ok(SlotContent::Agent {
            agent_id: id.to_string(),
        }),
        (SlotContentKind::Agent, None) => Err(CommandError::invalid_argument(
            "content=Agent needs agent_id — use slot.assignAgent if you only want to attach a running agent",
        )),
        (_, Some(_)) => Err(CommandError::invalid_argument(
            "agent_id only applies to content=Agent, so drop this field",
        )),
        (SlotContentKind::Empty, None) => Ok(SlotContent::Empty),
        (SlotContentKind::AgentList, None) => Ok(SlotContent::AgentList),
        (SlotContentKind::PresetPalette, None) => Ok(SlotContent::PresetPalette),
    }
}

// ★단위 테스트가 이 파일에 없는 것은 의도다★ — 이 패키지의 lib 테스트 타깃은 실행 자체가 안 된다
//   (`0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`, 실측 2026-08-17). 단언은 `tests/layout_commands.rs`.
