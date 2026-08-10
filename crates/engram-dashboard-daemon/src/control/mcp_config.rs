//! 에이전트별 **스폰 부착 파일** 생성·정리(ADR-0086 · S18 D) — claude 가 경로로 읽는 두 JSON.
//!
//! ★역할★: provision 시 (AgentId, epoch)용 파일을 데이터 디렉토리 아래에 쓰고 revoke 시 지운다. 두 종류다:
//!   ① **mcp-config**(`--mcp-config`) — 제어 채널 엔드포인트 + Bearer 토큰.
//!   ② **세션 설정 조각**(`--settings`, S18 D) — 그 세션에만 engram MCP 서버를 허용하는 최소 조각.
//!   아래 설명은 ①을 기준으로 읽되, 생성/삭제/스윕 규율은 ②에도 그대로 적용된다. claude 는 ① 을 읽어
//!   initialize/tools/list/tools/call 전 요청에 Authorization 헤더를 실어 보낸다(claude 2.1.170 실측).
//!
//! ★스키마(claude Streamable HTTP MCP 서버)★:
//! ```json
//! { "mcpServers": { "engram": {
//!     "type": "http",
//!     "url": "http://127.0.0.1:<port>/mcp",
//!     "headers": { "Authorization": "Bearer <token>" }
//! } } }
//! ```
//!   ※ 이 스키마는 mcp-config 공통 형식이지 claude CLI **플래그** 지식이 아니다 — 플래그(`--mcp-config`)는
//!   backend/claude.rs 단독(ADR-0004). 파일 내용 생성은 데몬 관심사(토큰·엔드포인트는 데몬 소유)라
//!   여기 둔다. backend 는 이 파일 경로만 `--mcp-config` 로 가리킨다.
//!
//! ★보안(ADR-0086 §Secrets)★:
//!   - 파일은 토큰을 평문으로 담는다 → 데이터 디렉토리 아래에 두고 **revoke 시 반드시 삭제**한다.
//!   - 토큰은 로그에 절대 찍지 않는다(경로·AgentId 만).
//!
//! tauri import 0(daemon crate).

use std::path::{Path, PathBuf};

use engram_dashboard_core::agent::types::AgentId;

/// 다른 산출물(agents.json 등)과 섞이지 않게 전용 폴더로 격리한다.
/// ★이름은 `mcp-config` 로 유지★: 폴더명을 바꾸면 옛 데이터 디렉토리에 남은 파일이 스윕 대상에서 빠져
///   영원히 방치된다(마이그레이션 없는 인메모리 단계에선 이름 유지가 더 안전하다).
const MCP_CONFIG_SUBDIR: &str = "mcp-config";

/// ★서버 논리명(mcpServers 키) = `engram`★ — **단일 출처(ADR-0094)**. claude 의 `system:init` 에 이
///   이름으로 서버가 뜨고, mcp-config JSON 의 `mcpServers.<이 값>` 키도 이 값이다. ADR-0094 발신 권한
///   grant 가 `mcp__{server}__{tool}` 패턴을 만들 때 이 상수를 server 로 쓴다(DaemonControlChannel.provision).
///   ADR-0086 §engram-ctl 이름 재사용 금지 — 데몬 자체 브랜드로 `engram`(폐기된 크레이트명 아님).
pub const MCP_SERVER_NAME: &str = "engram";

/// epoch 를 파일명에 넣어 회전 시 옛 파일과 충돌하지 않게 한다.
pub fn config_path(data_dir: &Path, id: AgentId, epoch: u32) -> PathBuf {
    data_dir
        .join(MCP_CONFIG_SUBDIR)
        .join(format!("{id}-{epoch}.json"))
}

/// escape 는 serde_json 이 처리(손조립 금지).
///
/// ★typed struct 직렬화(키 순서 결정적)★: serde 는 struct 필드를 선언 순서대로 쓴다 — 스키마를 사양
///   그대로 드러낸다(claude 는 임의 순서 수용).
pub fn render_config(url: &str, token: &str) -> String {
    #[derive(serde::Serialize)]
    struct Root<'a> {
        #[serde(rename = "mcpServers")]
        mcp_servers: std::collections::BTreeMap<&'a str, Server<'a>>,
    }
    #[derive(serde::Serialize)]
    struct Server<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        url: &'a str,
        headers: Headers,
    }
    #[derive(serde::Serialize)]
    struct Headers {
        #[serde(rename = "Authorization")]
        authorization: String,
    }
    let mut mcp_servers = std::collections::BTreeMap::new();
    mcp_servers.insert(
        MCP_SERVER_NAME,
        Server {
            kind: "http",
            url,
            headers: Headers {
                authorization: format!("Bearer {token}"),
            },
        },
    );
    let root = Root { mcp_servers };
    // to_string_pretty 는 이 형태에선 실패하지 않음 — 방어적 unwrap_or_default.
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

pub fn write_config(
    data_dir: &Path,
    id: AgentId,
    epoch: u32,
    url: &str,
    token: &str,
) -> std::io::Result<PathBuf> {
    let path = config_path(data_dir, id, epoch);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, render_config(url, token))?;
    tracing::info!(agent = %id, epoch, path = %path.display(), "mcp-config 기록(ADR-0086)");
    Ok(path)
}

/// ★왜 mcp-config 와 **같은 폴더**인가(load-bearing — 수명 관리 단일화)★: 이 조각은 mcp-config 와 정확히
///   같은 수명이다(같은 (id,epoch)에 provision 때 생기고 revoke 때 사라진다). 전용 폴더를 새로 파면
///   ① revoke ② 부팅 스윕 ③ 데이터 디렉토리 규약이 각각 두 벌이 되고, 한쪽만 갱신되면 조각 파일이 영원히
///   쌓인다. 같은 폴더를 쓰면 **기존 부팅 스윕(`sweep_stale_configs`)이 폴더 안 파일을 전부 지우므로**
///   추가 스윕 코드 없이 청소가 따라온다. 파일명 접미(`.settings.json`)로 mcp-config(`.json`)와 구분한다.
pub fn settings_path(data_dir: &Path, id: AgentId, epoch: u32) -> PathBuf {
    data_dir
        .join(MCP_CONFIG_SUBDIR)
        .join(format!("{id}-{epoch}.settings.json"))
}

/// ★내용 = `{"allowedMcpServers":[{"serverName":"engram"}]}`(spec §6)★. 유저 전역 설정의
///   `allowedMcpServers: []`(전면 차단)가 스폰 에이전트에도 적용돼 engram MCP 서버가 툴 목록에서 사라지는
///   문제(실측 2026-07-24)를 **이 세션에만** 뒤집는다. 전역 파일은 건드리지 않는다.
/// ★최소 조각 원칙(load-bearing)★: 이 파일에 **다른 설정을 더 넣지 않는다**. `--settings` 는 설정 계층의
///   높은 우선순위 층이라(user → project → local → `--settings` → managed), 여기 들어간 키는 사용자의
///   프로젝트/로컬 설정을 조용히 덮어쓴다. 그래서 우리가 반드시 필요한 한 키만 싣는다 — 이 파일이
///   "잡다한 스폰 설정" 서랍이 되면 사용자 설정을 침범한다(설정 IR 레이어가 오면 그쪽이 정본이 된다).
// ADR-0103 (spec §6 allowedMcpServers 대책)
// ADR-0109 (--settings 조각 — 단일 키 유지·파일 주입)
pub fn render_settings() -> String {
    #[derive(serde::Serialize)]
    struct Root<'a> {
        #[serde(rename = "allowedMcpServers")]
        allowed_mcp_servers: Vec<AllowedServer<'a>>,
    }
    #[derive(serde::Serialize)]
    struct AllowedServer<'a> {
        #[serde(rename = "serverName")]
        server_name: &'a str,
    }
    let root = Root {
        allowed_mcp_servers: vec![AllowedServer {
            server_name: MCP_SERVER_NAME,
        }],
    };
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

pub fn write_settings(data_dir: &Path, id: AgentId, epoch: u32) -> std::io::Result<PathBuf> {
    let path = settings_path(data_dir, id, epoch);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, render_settings())?;
    tracing::info!(agent = %id, epoch, path = %path.display(), "세션 설정 조각 기록(spec §6)");
    Ok(path)
}

/// 삭제 실패는 provision/revoke 를 막지 않는다 — 이 파일엔 비밀이 없고, 다음 부팅 스윕이 어차피 쓸어낸다.
pub fn remove_settings(data_dir: &Path, id: AgentId, epoch: u32) {
    let path = settings_path(data_dir, id, epoch);
    match std::fs::remove_file(&path) {
        Ok(()) => tracing::info!(agent = %id, epoch, "세션 설정 조각 삭제"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // idempotent no-op
        Err(e) => tracing::warn!(agent = %id, epoch, "세션 설정 조각 삭제 실패(무시): {e}"),
    }
}

/// ★삭제 실패는 provision 을 막지 않는다(무해 이유 — FIX 5)★: 파일 삭제가 실패해도 warn 만 남기고
///   진행한다. 그 잔여 파일은 **inert** 하다 — 그 안의 토큰은 registry.revoke 가 이미 evict 했으므로
///   (validate 가 None → 401), 파일이 디스크에 남아도 어떤 에이전트도 그 토큰으로 인증할 수 없다.
///   즉 남은 파일은 dead credential(더 이상 유효하지 않은 문자열)일 뿐 보안 창을 열지 않는다. 다음
///   부팅의 boot sweep 이 어차피 쓸어낸다(registry 는 부팅마다 빈 상태로 시작 → 모든 기존 파일이 dead).
pub fn remove_config(data_dir: &Path, id: AgentId, epoch: u32) {
    let path = config_path(data_dir, id, epoch);
    match std::fs::remove_file(&path) {
        Ok(()) => tracing::info!(agent = %id, epoch, "mcp-config 삭제(ADR-0086)"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // idempotent no-op
        Err(e) => {
            tracing::warn!(agent = %id, epoch, "mcp-config 삭제 실패(무시 — 파일은 inert): {e}")
        }
    }
}

/// ★부팅 스윕(FIX 5)★: 데몬 크래시나 세션 등록 전 실패로 stale 파일이 살아남을 수 있다. 평문 토큰
/// 파일을 디스크에 방치하지 않으려 부팅 시 일괄 청소한다. 개별 파일 삭제 실패는 warn 만 남기고
/// 계속한다(다음 부팅이 재시도 — 청소 실패로 데몬 기동을 막지 않는다).
pub fn sweep_stale_configs(data_dir: &Path) {
    let dir = data_dir.join(MCP_CONFIG_SUBDIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        // 첫 부팅엔 폴더 자체가 없다 — 청소할 것도 없다.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            tracing::warn!(
                dir = %dir.display(),
                "부팅 스윕 미실행 — stale 평문 토큰 파일이 디스크에 남아 있을 수 있다: {e}"
            );
            return;
        }
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(e) => {
                tracing::warn!(path = %path.display(), "부팅 스윕: stale 스폰 부착 파일 삭제 실패(계속): {e}")
            }
        }
    }
    if removed > 0 {
        tracing::info!(
            count = removed,
            "부팅 스윕: stale 스폰 부착 파일 청소(mcp-config + 세션 설정 조각, ADR-0086)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_config_is_valid_json_with_correct_shape() {
        let s = render_config("http://127.0.0.1:5000/mcp", "abc123");
        let v: serde_json::Value = serde_json::from_str(&s).expect("valid JSON");
        assert_eq!(v["mcpServers"]["engram"]["type"], "http");
        assert_eq!(
            v["mcpServers"]["engram"]["url"],
            "http://127.0.0.1:5000/mcp"
        );
        assert_eq!(
            v["mcpServers"]["engram"]["headers"]["Authorization"], "Bearer abc123",
            "Authorization 헤더는 'Bearer <token>' 형식"
        );
    }

    #[test]
    fn config_path_includes_agent_and_epoch() {
        let id = AgentId::new_v4();
        let p = config_path(Path::new("C:/data"), id, 2);
        let s = p.to_string_lossy();
        assert!(s.contains(&id.to_string()), "경로에 agent id 포함");
        assert!(s.ends_with("-2.json"), "경로에 epoch 포함: {s}");
        assert!(s.contains("mcp-config"), "전용 하위 폴더 사용");
    }

    #[test]
    fn write_then_remove_roundtrip() {
        let dir = std::env::temp_dir().join(format!("engram-mcpcfg-test-{}", AgentId::new_v4()));
        let id = AgentId::new_v4();
        let path = write_config(&dir, id, 0, "http://127.0.0.1:6000/mcp", "tok-xyz")
            .expect("write config");
        assert!(path.exists(), "파일이 생성돼야 함");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Bearer tok-xyz"));
        remove_config(&dir, id, 0);
        assert!(!path.exists(), "revoke 시 파일이 지워져야 함");
        remove_config(&dir, id, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── S18 D(spec §6): 세션 한정 설정 조각 ──────────────────────────────────────────────
    #[test]
    fn render_settings_allows_exactly_the_engram_server() {
        let s = render_settings();
        let v: serde_json::Value = serde_json::from_str(&s).expect("valid JSON");
        let arr = v["allowedMcpServers"].as_array().expect("배열");
        assert_eq!(arr.len(), 1, "허용 목록은 engram 하나뿐: {s}");
        assert_eq!(
            arr[0]["serverName"], MCP_SERVER_NAME,
            "서버 이름은 mcp-config 키와 같은 단일 출처여야(갈리면 허용이 무력): {s}"
        );
        assert_eq!(
            v.as_object().map(|o| o.len()),
            Some(1),
            "조각엔 allowedMcpServers 하나만: {s}"
        );
    }

    #[test]
    fn settings_path_is_distinct_from_mcp_config_but_shares_the_swept_dir() {
        let id = AgentId::new_v4();
        let cfg = config_path(Path::new("C:/data"), id, 3);
        let set = settings_path(Path::new("C:/data"), id, 3);
        assert_ne!(cfg, set, "두 파일은 서로 다른 이름이어야(덮어쓰기 금지)");
        assert_eq!(
            cfg.parent(),
            set.parent(),
            "같은 폴더 = 기존 부팅 스윕/삭제 경로 재사용(수명 관리 단일화)"
        );
        assert!(set.to_string_lossy().ends_with("-3.settings.json"));
    }

    #[test]
    fn write_then_remove_settings_roundtrip_and_boot_sweep_cleans_it() {
        let dir = std::env::temp_dir().join(format!("engram-settings-test-{}", AgentId::new_v4()));
        let id = AgentId::new_v4();
        let path = write_settings(&dir, id, 0).expect("write settings");
        assert!(path.exists());
        remove_settings(&dir, id, 0);
        assert!(!path.exists(), "revoke 시 삭제");
        remove_settings(&dir, id, 0); // idempotent

        let path = write_settings(&dir, id, 1).expect("write settings again");
        assert!(path.exists());
        sweep_stale_configs(&dir);
        assert!(!path.exists(), "부팅 스윕이 설정 조각도 청소");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
