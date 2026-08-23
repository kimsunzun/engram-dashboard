//! 웹뷰 몫 명령의 Tauri 어댑터 — 부팅 보고 하나와 결말 회수 하나.
//!
//! ★이 파일에 로직이 없다★: 필터·상관·마감은 `crate::view_commands` 가 소유하고, 여기 남는 것은 Tauri
//! 세계로의 번역(창 label 을 어디서 얻나 · 결말을 어떤 인자로 받나)과 **데몬에 차분을 밀 것인가** 하나뿐이다
//! (`commands/settings.rs`·`commands/layout.rs` 와 같은 분담).
//!
//! ## ★차분을 여기서 미는 이유★
//! 셸의 등록은 **핸드셰이크 직후 한 번** 나간다(`daemon_client::connection` 의 `register_own_commands`).
//! 웹뷰의 보고는 그 시점과 아무 순서 관계가 없다 — 창 스크립트가 늦게 올라오면 그 연결의 등록 패킷에는
//! 화면 이름이 **하나도 안 실린다**. 그대로 두면 다음 재연결까지 화면 명령이 통째로 `UNKNOWN_COMMAND` 다.
//! 그래서 명단이 실제로 바뀐 보고에 한해 `UpdateCommands` 를 낸다(TRD §3-7 조항 3).
// ADR-0155

use std::sync::Arc;

use engram_dashboard_protocol::{AgentCommand, RequestId};
use tauri::{AppHandle, Manager, State, Window};

use crate::daemon_client::DaemonClient;
use crate::view_commands::{ViewCommandBridge, ViewCommandDecl};
use engram_dashboard_command::{CommandError, ErrorCode};

/// 창 하나가 부팅에 자기 명령 목록을 알린다 — 창마다 한 번(`src/commands/viewCommandBridge.ts`).
///
/// ★창 label 을 인자로 받지 않는다★: 웹뷰가 스스로 밝히게 하면 잘못 적힌 label 하나가 남의 창으로 봉투를
/// 보낸다. Tauri 가 넣어 주는 [`Window`] 가 그 값의 유일한 권위다.
/// ★**창** label 이지 webview label 이 아니다★ — 웹뷰 쪽이 구독을 거는 값(`getCurrentWindow().label`)과
/// 같아야 `emit_to` 가 그 리스너에 걸린다. 둘은 오늘 같은 문자열이지만(창마다 웹뷰 하나) 같은 것을
/// **가리키는 쪽**을 골라 둔다.
/// 반환은 항상 `Ok` 다 — 예약 이름이 빠진 것은 보고한 쪽의 실패가 아니라 셸의 정책이고(사유 =
/// `view_commands` 모듈 헤더), 그 사실은 로그가 진다.
#[tauri::command]
pub async fn report_view_commands(
    window: Window,
    app: AppHandle,
    bridge: State<'_, Arc<ViewCommandBridge>>,
    commands: Vec<ViewCommandDecl>,
) -> Result<(), String> {
    let label = window.label().to_string();
    // ★상태 변경과 그 차분 송신을 **다리가 한 문 안에서** 돌린다★ — 여기서 따로 부르면 두 창의 보고가
    //   서로를 앞질러 데몬이 옛 이름을 쥔 채 남는다(사유 = `ViewCommandBridge` 의 `outbound`).
    let outcome = bridge
        .report_and_push(&label, commands, |added, removed| {
            let label = label.clone();
            let app = app.clone();
            async move { push_delta(&app, &label, added, removed).await }
        })
        .await;
    if !outcome.refused.is_empty() {
        tracing::info!(
            window = %label,
            names = ?outcome.refused,
            "웹뷰 명령 일부를 등록에서 뺐다(셸·데몬이 답하는 이름이거나 설명·표식이 없다)"
        );
    }
    if !outcome.changed() {
        tracing::debug!(window = %label, "웹뷰 명령 보고 — 광고 명단 변화 없음");
    }
    Ok(())
}

/// 바뀐 몫만 데몬 명부에 얹는다 — ★부르는 자리는 위 한 곳뿐이다★(순서 문 안).
async fn push_delta(
    app: &AppHandle,
    label: &str,
    added: Vec<engram_dashboard_command::CommandDecl>,
    removed: Vec<String>,
) {
    tracing::info!(
        window = %label,
        added = added.len(),
        removed = removed.len(),
        "웹뷰 명령 명단 갱신"
    );
    // ★클라이언트가 없으면 조용히 끝낸다★ — 런타임 생성이 실패한 앱(`lib.rs` 의 Err 갈래)에서는 데몬
    //   명령 자체가 없다. 명단은 이미 다리에 들었으므로 다음 연결의 등록 패킷이 그것을 싣는다.
    let Some(client) = app
        .try_state::<Arc<DaemonClient>>()
        .map(|c| c.inner().clone())
    else {
        return;
    };
    let cmd = AgentCommand::UpdateCommands {
        // ★데몬은 이 칸을 쓰지 않는다★ — 명부 주인은 그 패킷이 온 연결에서 파생된다. 등록 패킷과 **같은
        //   광고 문자열**을 적어 데몬 로그에서 두 패킷이 같은 셸의 것으로 보이게 한다.
        owner: crate::daemon_client::connection::shell_owner_advert(),
        added,
        removed,
        request_id: RequestId::new(),
    };
    // ★답장을 기다리되 실패를 위로 올리지 않는다★: 보고 자체는 성공했고(다리에 들었다) 여기 실패는
    //   「이 연결에서는 아직 안 보인다」일 뿐이라 다음 재연결이 전량 등록으로 해소한다. 다만 조용히
    //   넘기지는 않는다 — 등록이 안 서면 화면 명령이 안 불리는데 그 원인이 어디에도 안 남는다.
    match client.send_command(cmd).await {
        Ok(_) => tracing::info!(window = %label, "웹뷰 명령 차분 등록 완료"),
        Err(e) => tracing::warn!(
            window = %label,
            "웹뷰 명령 차분 등록 실패(다음 재연결의 전량 등록이 해소한다): {e}"
        ),
    }
}

/// 웹뷰가 명령 하나를 끝내고 결말을 돌려준다.
///
/// ★어느 창이 답했는지를 [`Window`] 가 준다 — 인자로 받지 않는다★: 상관 키 하나로만 열면 봉투를 받지
/// 않은 창이 남의 왕복을 끝낼 수 있다(호출자는 위조 결말을 받고, 진짜 창의 부수효과는 그대로 일어나며
/// 그 답만 반려된다). 대조는 다리가 한다(`ViewCommandBridge::settle`).
///
/// ★오류 종류를 웹뷰가 못 고른다(알려진 한계)★ — 인자 실수도 진짜 실패도 `INTERNAL` + 문구로 나간다.
/// 가르려면 웹뷰 레지스트리가 타입드 코드를 던져야 하고 그건 이 어댑터의 결정이 아니다(광고하는 오류
/// 집합도 그 사실을 그대로 적는다 — `ViewCommandHelp::to_catalog_item`).
#[tauri::command]
pub fn report_command_outcome(
    window: Window,
    bridge: State<'_, Arc<ViewCommandBridge>>,
    request_id: String,
    ok: Option<serde_json::Value>,
    error: Option<String>,
) -> Result<(), String> {
    let outcome = match error {
        Some(message) => Err(CommandError::of(ErrorCode::Internal, message)),
        None => Ok(ok.unwrap_or(serde_json::Value::Null)),
    };
    if let Err(detail) = bridge.settle(window.label(), &request_id, outcome) {
        // 답장을 붙일 자리가 없다 = 마감을 넘겼거나 · 같은 봉투에 두 번째 답이 왔거나 · **봉투를 받지
        //   않은 창이 답했다**. 셋 다 사람이 봐야 하는 신호다.
        tracing::warn!(window = %window.label(), "웹뷰 명령 결말을 상관시키지 못했다: {detail}");
        return Err(detail);
    }
    Ok(())
}
