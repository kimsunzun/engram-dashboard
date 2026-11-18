//! GeminiBackend — CommandSpec production stub for the Gemini CLI only.
//!
//! ★The rule this folder sets = gemini knowledge lives only in here (ADR-0004)★. The canonical grounds, gates and what the gates
//! cannot see are in the `backend/claude/mod.rs` header and are not repeated here — read it with the names swapped.
//! The only surface going out is the one [`crate::backend::AgentBackend`] implementation.
//!
//! AgentCommand has no Gemini variant, so backend_for dispatch does not route to this backend.
//! This file is a stub to secure the structure; adding the AgentCommand::Gemini variant and
//! the backend_for match are settled in a separate task after the CLI spike is done.
//!
//! tauri import 0.

use std::path::PathBuf;

use uuid::Uuid;

use crate::backend::AgentBackend;
use crate::profile::{AgentCommand, SpawnMode};
use crate::types::{BackendCaps, CommandSpec, ControlEndpoint, ModelCaps, SessionCaps};

/// Gemini executable name. Resolved through PATH.
///
/// ※ best-guess: needs confirmation that the real binary name of the Google Gemini CLI is "gemini".
/// Settle it in the CLI spike with `which gemini` / `gemini --help`.
/// It may be the Google AI Studio CLI or the `gemini-cli` package name.
const GEMINI_PROGRAM: &str = "gemini";

pub struct GeminiBackend;

impl AgentBackend for GeminiBackend {
    /// ★The two axes below have never been measured, and this backend is **unreachable** today★ — `AgentCommand` has no gemini
    /// variant, so there is no way for `backend_for` to come to this implementation (which is why the declaration table in
    /// `tests/backend_contract.rs` has no row for it either). The values are best-guesses from before the CLI spike, and the task that brings in the variant replaces
    /// them with measurements.
    /// ★Having no unit tests **for these two methods only** is intended★: calling them would mean feeding another backend's command
    ///   (a pair `backend_for` cannot make), and **these two do not even read that command** (`_command`), so an assertion written that way
    ///   would only re-read a constant. ★`build_spec` in the same file is different★ — it actually
    ///   matches the command and moves the arguments, so even a synthetic sample measures something, and that is why it has tests (its limits are in the
    ///   `tests::spec` comment below).
    fn assigns_session_id(&self, _command: &AgentCommand) -> bool {
        // best-guess: true, assuming the Gemini CLI also has a conversation-session concept.
        // Change to false if it is a sessionless CLI.
        true
    }

    fn can_resume_stored_session(&self, _command: &AgentCommand) -> bool {
        // best-guess: the same assumption as the `--resume <sid>` assembly below.
        true
    }

    fn supports_control_channel(&self) -> bool {
        // Conservative stub (ADR-0086 F3) — whether the Gemini CLI supports MCP is unknown since the CLI spike has not happened.
        //   false in the same spirit as the capabilities stub being all false (an unmeasured backend does not claim to consume
        //   the control channel). Replace with measured values after the spike.
        false
    }

    fn accepts_mcp_config(&self) -> bool {
        // Conservative stub (ADR-0099) — an unmeasured backend does not claim MCP-capable → false (non-MCP spawn).
        //   Gemini's real MCP config support is replaced with measured values after the CLI spike (ADR-0004 backend knowledge).
        // ADR-0099
        false
    }

    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        // ADR-0185 stub — this backend is not wired into dispatch, and its command-line resume syntax is also unmeasured.
        _resume_session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        // ADR-0086: stub — control channel injection is implemented when the variant is settled after the CLI spike (ignored for now).
        // TODO(ADR-0094): translate ControlEndpoint.grants to gemini permission flags
        //   (claude uses --allowedTools mcp__{s}__{t} / Bash({e}:*)+PowerShell({e}:*); the gemini dialect is settled after the CLI spike).
        _control: Option<ControlEndpoint>,
    ) -> CommandSpec {
        let mut args: Vec<String> = Vec::new();

        if let Some(sid) = session_id {
            // Unconfirmed whether the Gemini CLI's session-resume flag is --session / --resume / --conversation etc.
            // The same pattern as Claude chosen as a best-guess (assuming most AI CLIs follow a similar UX).
            let flag = match mode {
                SpawnMode::Fresh => "--session",
                SpawnMode::Resume => "--resume",
            };
            args.push(flag.to_string());
            args.push(sid.to_string());
        }

        match command {
            AgentCommand::Claude { extra_args, .. } => {
                args.extend(extra_args.iter().cloned());
            }
            AgentCommand::Shell {
                program,
                args: shell_args,
            } => {
                return CommandSpec {
                    program: GEMINI_PROGRAM.to_string(),
                    args: {
                        let _ = program;
                        shell_args.clone()
                    },
                    env,
                    cwd,
                };
            }
            // This backend is not wired into dispatch, and imitating the arguments of the wired sibling (codex) would
            //   leak that sibling's knowledge into here (ADR-0004).
            AgentCommand::Codex { .. } => {
                unreachable!("GeminiBackend does not handle the Codex variant. dispatch bug.")
            }
        }

        CommandSpec {
            program: GEMINI_PROGRAM.to_string(),
            args,
            env,
            cwd,
        }
    }

    /// Conservative stub — real resume/model capabilities unknown since the CLI spike has not happened → all false.
    /// Replace with measured values after the spike.
    fn capabilities(&self, _command: &AgentCommand) -> BackendCaps {
        BackendCaps {
            session: SessionCaps {
                resume: false,
                snapshot: false,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★The sample is synthetic — `backend_for` cannot give this backend a claude command★ (there is no gemini variant at all,
    /// so **no** command reaches here). Still, the entries using this helper measure something:
    /// `build_spec` actually matches the command, moves `extra_args` and assembles the session flags, so the assertions below
    /// measure **that assembly**.
    /// ★What is **not** measured = whether that assembly matches the real gemini★ — the program name and `--session`/`--resume` are all
    ///   best-guesses, and the real CLI has never been launched (exactly as the stub declaration in the file header says). The task that brings in the variant
    ///   turns the sample into a real command and replaces the values with measurements.
    fn spec(mode: SpawnMode, sid: Option<Uuid>) -> CommandSpec {
        GeminiBackend.build_spec(
            &AgentCommand::Claude {
                extra_args: vec![],
                output_format: crate::profile::AgentOutputFormat::Terminal,
            },
            mode,
            sid,
            None,
            PathBuf::from("."),
            vec![],
            None,
        )
    }

    #[test]
    fn gemini_program_name_is_correct() {
        let s = spec(SpawnMode::Fresh, None);
        assert_eq!(s.program, GEMINI_PROGRAM);
        assert_eq!(s.program, "gemini");
    }

    #[test]
    fn gemini_fresh_uses_session_flag_best_guess() {
        let sid = Uuid::new_v4();
        let s = spec(SpawnMode::Fresh, Some(sid));
        assert_eq!(s.program, GEMINI_PROGRAM);
        assert_eq!(s.args, vec!["--session".to_string(), sid.to_string()]);
    }

    #[test]
    fn gemini_resume_uses_resume_flag_best_guess() {
        let sid = Uuid::new_v4();
        let s = spec(SpawnMode::Resume, Some(sid));
        assert_eq!(s.args, vec!["--resume".to_string(), sid.to_string()]);
    }

    /// The canonical account of why the sample is synthetic and its limits is the [`spec`] comment above — this entry alone does not use `spec`
    /// because it has to pass cwd and env directly.
    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("GEMINI_KEY".to_string(), "dummy".to_string())];
        let s = GeminiBackend.build_spec(
            &AgentCommand::Claude {
                extra_args: vec![],
                output_format: crate::profile::AgentOutputFormat::Terminal,
            },
            SpawnMode::Fresh,
            None,
            None,
            cwd.clone(),
            env.clone(),
            None,
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, env);
    }
}
