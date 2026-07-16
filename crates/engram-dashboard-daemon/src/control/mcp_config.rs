//! 에이전트별 mcp-config 생성·정리(ADR-0086) — claude `--mcp-config <path>` 가 읽는 JSON 파일.
//!
//! ★역할★: provision 시 (AgentId, epoch)용 mcp-config JSON 을 데이터 디렉토리 아래에 쓰고, revoke 시
//!   지운다. 파일에는 데몬 MCP 엔드포인트 URL + Bearer 토큰(Authorization 헤더)이 담긴다 — claude 가
//!   이 파일을 읽어 initialize/tools/list/tools/call 전 요청에 헤더를 실어 보낸다(claude 2.1.170 실측).
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

/// mcp-config 파일이 사는 하위 디렉토리명(데이터 디렉토리 기준). 다른 산출물(agents.json 등)과 섞이지
/// 않게 전용 폴더로 격리한다. revoke 시 파일만 지우고 폴더는 남긴다(재사용).
const MCP_CONFIG_SUBDIR: &str = "mcp-config";

// ★서버 논리명(mcpServers 키) = `engram`★: render_config 의 `Servers.engram` 필드명이 곧 이 키다
//   (serde 는 필드 식별자를 JSON 키로 쓴다). claude 의 `system:init` 에 이 이름으로 서버가 뜬다.
//   ADR-0086 §engram-ctl 이름 재사용 금지 — 데몬 자체 브랜드로 `engram` 사용(폐기된 크레이트명 아님).

/// (AgentId, epoch)용 mcp-config 파일 경로. epoch 를 파일명에 넣어 회전 시 옛 파일과 충돌하지 않게 한다
/// (구 파일은 revoke 가 지운다). `<data_dir>/mcp-config/<agent_id>-<epoch>.json`.
pub fn config_path(data_dir: &Path, id: AgentId, epoch: u32) -> PathBuf {
    data_dir
        .join(MCP_CONFIG_SUBDIR)
        .join(format!("{id}-{epoch}.json"))
}

/// mcp-config JSON 문자열을 만든다(순수 함수 — 파일 IO 없음, 단위 테스트 대상). url·token 으로 위
/// 스키마를 조립한다. escape 는 serde_json 이 처리(손조립 금지).
///
/// ★typed struct 직렬화(키 순서 결정적)★: serde 는 struct 필드를 선언 순서대로 쓴다 — 스키마를 사양
///   그대로 드러낸다(claude 는 임의 순서 수용).
pub fn render_config(url: &str, token: &str) -> String {
    #[derive(serde::Serialize)]
    struct Root<'a> {
        #[serde(rename = "mcpServers")]
        mcp_servers: Servers<'a>,
    }
    #[derive(serde::Serialize)]
    struct Servers<'a> {
        engram: Server<'a>,
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
    let root = Root {
        mcp_servers: Servers {
            engram: Server {
                kind: "http",
                url,
                headers: Headers {
                    authorization: format!("Bearer {token}"),
                },
            },
        },
    };
    // to_string_pretty 는 이 형태에선 실패하지 않음 — 방어적 unwrap_or_default.
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

/// mcp-config 파일을 디스크에 쓴다(디렉토리 없으면 생성). 성공 시 경로를 돌려준다.
/// ★보안★: token 은 파일에만(로그 금지). 반환 경로가 backend `--mcp-config` 로 전달된다.
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

/// mcp-config 파일 삭제(revoke). 없으면 조용히 성공(idempotent — 이중 revoke 안전).
///
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

/// ★부팅 스윕(FIX 5)★: 데몬 시작 시 `<data_dir>/mcp-config/` 안의 파일을 전부 삭제한다. 데몬 크래시나
/// 세션 등록 전 실패로 살아남은 stale mcp-config 는 dead credential 이다 — 그 안의 토큰은 registry 가
/// **부팅마다 빈 상태로 시작**하므로 어떤 것도 유효하지 않다(validate None → 401). 그래도 평문 토큰
/// 파일을 디스크에 방치하지 않으려 부팅 시 일괄 청소한다. 디렉토리가 없으면 no-op(첫 부팅). 개별 파일
/// 삭제 실패는 warn 만 남기고 계속한다(다음 부팅이 재시도 — 청소 실패로 데몬 기동을 막지 않는다).
pub fn sweep_stale_configs(data_dir: &Path) {
    let dir = data_dir.join(MCP_CONFIG_SUBDIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        // 디렉토리 부재(첫 부팅) 등은 정상 — 청소할 게 없다.
        Err(_) => return,
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
                tracing::warn!(path = %path.display(), "부팅 스윕: stale mcp-config 삭제 실패(계속): {e}")
            }
        }
    }
    if removed > 0 {
        tracing::info!(
            count = removed,
            "부팅 스윕: stale mcp-config 청소(ADR-0086)"
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
        // 내용에 토큰이 담겨 있어야(claude 가 읽는다). 파일 안에만 — 로그엔 없음.
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Bearer tok-xyz"));
        remove_config(&dir, id, 0);
        assert!(!path.exists(), "revoke 시 파일이 지워져야 함");
        // 이중 remove 안전(idempotent).
        remove_config(&dir, id, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
