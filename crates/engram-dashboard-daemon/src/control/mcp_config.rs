//! Creation and cleanup of the per-agent **spawn attachment file** (ADR-0086) — the **mcp-config** JSON claude reads by
//! path (`--mcp-config`): control channel endpoint + Bearer token.
//!
//! ★Role★: on provision, write the file for (AgentId, epoch) into [`McpDir`]; on revoke, delete it. claude reads it and
//!   sends the Authorization header on every request: initialize/tools/list/tools/call (measured on claude 2.1.170).
//! ★No session settings fragment is written here any more — do not revive it★: the launch materials carry the one
//!   `--settings` (ADR-0328). The boot sweep still clears every file in the folder, so a fragment an older daemon left
//!   goes with it.
//!
//! ★Schema (claude Streamable HTTP MCP server)★:
//! ```json
//! { "mcpServers": { "engram": {
//!     "type": "http",
//!     "url": "http://127.0.0.1:<port>/mcp",
//!     "headers": { "Authorization": "Bearer <token>" }
//! } } }
//! ```
//!   This is the common mcp-config format, not knowledge of claude CLI flags — the flag (`--mcp-config`) belongs to backend/claude/ alone (ADR-0004). The token and endpoint are owned by the daemon, so generating the content lives here; the backend only points `--mcp-config` at the file path.
//!
//! ★Security (ADR-0086 §Secrets)★:
//!   - The file holds the token in plaintext → put it under `run` in the data folder and **always delete it on revoke**.
//!   - Never print the token in logs (only the path and AgentId).
//!
//! tauri import 0 (daemon crate).
// ADR-0370

use std::path::{Path, PathBuf};

use engram_dashboard_agent::types::AgentId;

/// ★Server logical name (the mcpServers key) = `engram`★ — single source (ADR-0094). The server shows up in claude's `system:init` under this name, the mcp-config `mcpServers.<name>` key is this value, and the ADR-0094 send-permission grant builds its `mcp__{server}__{tool}` pattern from this constant (DaemonControlChannel.provision). ADR-0086: do not reuse the engram-ctl name — `engram` is the daemon's own brand.
///
/// ★Defined in the agent crate, not here — do not write the literal here again★: a second consumer exists in the backend folder (codex's `-c mcp_servers.<name>=…`) and cannot depend on the daemon. This re-export keeps existing `mcp_config::MCP_SERVER_NAME` call sites.
// ADR-0209
pub use engram_dashboard_agent::types::MCP_SERVER_NAME;

/// The spawn attachment folder itself (not the data folder) — in production `data_dir::DataLayout::mcp_config_dir` (under the daemon's `run`). Every file of this module goes directly inside it, and [`sweep_stale_configs`] deletes all files inside it.
///
/// ★Do not turn it back into a bare `Path`★: passing the data folder would compile, write the token file at the bottom of the data folder, and the boot sweep would delete that folder's files (including the registry). One constructor makes the intent explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpDir(PathBuf);

impl McpDir {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self(dir.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// The epoch is in the file name so a rotation does not collide with the old file.
pub fn config_path(mcp_dir: &McpDir, id: AgentId, epoch: u32) -> PathBuf {
    mcp_dir.0.join(format!("{id}-{epoch}.json"))
}

/// Escaping is handled by serde_json (no hand assembly).
///
/// ★Typed struct serialization★: serde writes fields in declaration order, exposing the schema as the spec has it (claude accepts any order).
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
    // to_string_pretty does not fail on this shape — defensive unwrap_or_default.
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

/// (A new data root has no folder — the atomic write creates it.)
// ADR-0291: the atomic write's temp file is in the same folder, so the token is briefly there — same ACL; leftovers from a power-off or failed delete are swept by the boot sweep.
fn write_attached(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    engram_dashboard_base::file::write_atomic(path, bytes, crate::file_hooks::OS_HOOKS)
}

pub fn write_config(
    mcp_dir: &McpDir,
    id: AgentId,
    epoch: u32,
    url: &str,
    token: &str,
) -> std::io::Result<PathBuf> {
    let path = config_path(mcp_dir, id, epoch);
    write_attached(&path, render_config(url, token).as_bytes())?;
    tracing::info!(agent = %id, epoch, path = %path.display(), "mcp-config written (ADR-0086)");
    Ok(path)
}

/// ★A delete failure does not block provision (why it is harmless — FIX 5)★: even if deleting the file fails, only a warn is left and
///   we proceed. That leftover file is **inert** — the token inside it was already evicted by registry.revoke
///   (validate returns None → 401), so even if the file stays on disk no agent can authenticate with that token.
///   That is, the remaining file is merely a dead credential (a string that is no longer valid) and opens no security window. The next
///   boot's boot sweep sweeps it up anyway (the registry starts empty on every boot → every existing file is dead).
pub fn remove_config(mcp_dir: &McpDir, id: AgentId, epoch: u32) {
    let path = config_path(mcp_dir, id, epoch);
    match std::fs::remove_file(&path) {
        Ok(()) => tracing::info!(agent = %id, epoch, "mcp-config deleted (ADR-0086)"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // idempotent no-op
        Err(e) => {
            tracing::warn!(agent = %id, epoch, "mcp-config delete failed (ignored — the file is inert): {e}")
        }
    }
}

/// ★Boot sweep (FIX 5)★: a daemon crash or a failure before session registration can leave stale plaintext-token files, so clean up in bulk at boot. An individual delete failure only warns and continues (the next boot retries — cleanup failure does not block startup).
pub fn sweep_stale_configs(mcp_dir: &McpDir) {
    let dir = mcp_dir.as_path();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // On the first boot the folder does not exist — nothing to clean.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            tracing::warn!(
                dir = %dir.display(),
                "boot sweep not run — stale plaintext token files may remain on disk: {e}"
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
                tracing::warn!(path = %path.display(), "boot sweep: failed to delete a stale spawn attachment file (continuing): {e}")
            }
        }
    }
    if removed > 0 {
        tracing::info!(
            count = removed,
            "boot sweep: removed stale spawn attachment files"
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
            "the Authorization header has the form 'Bearer <token>'"
        );
    }

    #[test]
    fn config_path_includes_agent_and_epoch() {
        let id = AgentId::new_v4();
        let p = config_path(&McpDir::new("C:/data/mcp"), id, 2);
        let s = p.to_string_lossy();
        assert!(
            s.contains(&id.to_string()),
            "the path contains the agent id"
        );
        assert!(s.ends_with("-2.json"), "the path contains the epoch: {s}");
        assert_eq!(
            p.parent(),
            Some(Path::new("C:/data/mcp")),
            "directly inside the given folder"
        );
    }

    #[test]
    fn write_then_remove_roundtrip() {
        let root = std::env::temp_dir().join(format!("engram-mcpcfg-test-{}", AgentId::new_v4()));
        let dir = McpDir::new(root.join("mcp"));
        let id = AgentId::new_v4();
        let path = write_config(&dir, id, 0, "http://127.0.0.1:6000/mcp", "tok-xyz")
            .expect("write config");
        assert!(path.exists(), "the file must be created");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Bearer tok-xyz"));
        remove_config(&dir, id, 0);
        assert!(!path.exists(), "revoke must delete the file");
        remove_config(&dir, id, 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A new data root has no folder yet — the write must create up to the root for the first spawn (ADR-0291 R11).
    #[test]
    fn writes_into_a_folder_that_does_not_exist_yet() {
        let root = std::env::temp_dir().join(format!("engram-mcpcfg-fresh-{}", AgentId::new_v4()));
        let dir = McpDir::new(root.join("run").join("mcp"));
        let id = AgentId::new_v4();
        let cfg = write_config(&dir, id, 0, "http://127.0.0.1:6000/mcp", "tok").expect("config");
        assert!(cfg.is_file());
        let names: Vec<_> = std::fs::read_dir(dir.as_path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "a temp file was left: {names:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Atomic-write temp files left by a power-off (a token may be in them) are also swept by the boot sweep.
    #[test]
    fn boot_sweep_collects_a_leftover_atomic_write_temp() {
        let root = std::env::temp_dir().join(format!("engram-mcpcfg-tmp-{}", AgentId::new_v4()));
        let dir = McpDir::new(root.join("mcp"));
        std::fs::create_dir_all(dir.as_path()).unwrap();
        let id = AgentId::new_v4();
        let temp = dir
            .as_path()
            .join(format!("{id}-0.json.tmp{}.0", std::process::id()));
        std::fs::write(&temp, "Bearer tok-leftover").unwrap();
        sweep_stale_configs(&dir);
        assert!(
            !temp.exists(),
            "a temp file carrying the token was left behind"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
