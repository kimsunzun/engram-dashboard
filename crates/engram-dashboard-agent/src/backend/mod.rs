//! AgentBackend — a trait producing per-backend command specs + free-function dispatch.
//!
//! The transport (PtyTransport) does not know claude/codex; only the `backend/<name>/` folder knows which program is which.
//!
//! ★This file names no backend's item★: a backend name is written only in the **registry** (`pub mod` · `pub use` ·
//! static singletons · the usage-probe table [`usage_probes`] · the settings-check table [`settings_checks`]) and the
//! **two dispatch tables** (`backend_for` · `backend_for_encoder`). Every piece of backend knowledge comes out only
//! through [`AgentBackend`] methods and the [`UsageProbe`] · [`SettingsCheck`] implementations. The gate is in the
//! `backend/claude/mod.rs` header.
//!
//! tauri import 0.

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod settings_check;
pub mod shell;

pub use claude::ClaudeBackend;
pub use codex::CodexBackend;
pub use gemini::GeminiBackend;
pub use settings_check::SettingsCheck;
pub use shell::ShellBackend;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use uuid::Uuid;

use crate::failure::AgentFailureKind;
use crate::launch::{LaunchArtifacts, LaunchFs, LaunchKit};
use crate::profile::{AgentCommand, AgentProfile, SpawnMode};
use crate::session_tracker::SessionIdSource;
use crate::transport::pty::PtyTransport;
use crate::transport::{AgentTransport, LinkSink, OutputDecoder};
use crate::turn::TurnSignal;
use crate::types::{
    AgentId, BackendCaps, CommandSpec, ControlEndpoint, DeliveryAck, MidTurnPolicy, OutputEvent,
    PtyError, CLI_EXE_ENV, CLI_EXE_NAME, TOKEN_ENV,
};
use crate::usage::UsageProbe;

// ── CLI entry injection (shared by backends) ────────────────────────────────────────────────

/// ADR-0086 step 2 (CLI entry): CLI credentials in the spawn env + PATH prepend of the directory of the control-plane CLI (`CLI_EXE_NAME`)
///   sibling.
///
/// ★Lives here because these three carry no backend knowledge (ADR-0004)★: only three env values and PATH, none a flag or file convention
///   of a specific program. A **translation** like `--mcp-config` is that program's grammar and stays in its backend folder.
/// ★[`crate::types::ControlEndpoint::config_path`] states this division as a contract★: that Option means only **whether there is an MCP
///   entry**, not whether there is CLI wiring.
///
/// ★Call condition = every spawn with a control endpoint★: control verbs are open to everyone (ADR-0132 decision 5) and one executable
///   cannot be split by family. Mail is not filtered here — enforcement is only the daemon's credential refusal (ADR-0133 decision 3).
/// ★Do not send mail availability in env★: it attaches to the agent's own erasable process, so it enforces nothing while giving the
///   outside a way to choose the CLI screen.
/// ★Why env★: the agent's shell call reads these and POSTs to the daemon control route with a Bearer token; the portable-pty
///   CommandBuilder seeds the parent env, so it is **inherited by every child process (Bash · its grandchildren)**.
/// ★So these are credentials of **auxiliary processes that program launches on its behalf** too★ — unless that program overrides the child
///   env; which programs do is recorded by measurement in that backend's folder (not counted by name here — ADR-0004).
/// ★Security★: the token is inherited only by children of the same OS user and not logged, but hard isolation is impossible (ADR-0086
///   §Invariants). ★Sub-agents the agent launched hold the same token★ — a report trusted as one identity would put the child in the
///   parent's place.
/// ★**So do not open a new report entry that establishes identity from this token alone**★ (the session id hook report was removed —
///   ADR-0216); today it opens only the control verbs and mail **the agent itself types**.
// ADR-0086 / ADR-0133 / ADR-0004 / ADR-0208 / ADR-0216
pub(crate) fn inject_cli_entrance(env: &mut Vec<(String, String)>, endpoint: &ControlEndpoint) {
    // ★ENGRAM_CONTROL_URL = base (scheme+host+port)★: endpoint.url is the MCP route (`http://127.0.0.1:<port>/mcp`); the route suffix
    //   (`/mcp`) is stripped to give the base the CLI attaches to (the CLI assembles `<base>/control/send` — route path knowledge is the CLI's).
    //   Without the suffix the url is used as the base (defensive).
    //   ★keep-in-sync (M5)★: the literal "/mcp" of strip_suffix is **hand-matched** to the daemon-side MCP_PATH
    //   (`crates/engram-dashboard-daemon/src/control/mcp_server.rs`); the build cannot enforce it — a divergence makes the CLI silently
    //   get 404. Fix both together.
    let base = endpoint
        .url
        .strip_suffix("/mcp")
        .unwrap_or(&endpoint.url)
        .to_string();
    env.push((TOKEN_ENV.to_string(), endpoint.token.clone()));
    env.push(("ENGRAM_CONTROL_URL".to_string(), base));
    // ★ENGRAM_CLI_EXE = absolute path of the CLI binary (F1)★: priming and grants teach the bare name (`CLI_EXE_NAME`, resolved by the
    //   PATH injection below); this absolute path is carried for diagnostics and manual operation.
    //   ★No priming teaches this value — do not count it as the recovery for the loud skip branch below (PATH join failure · non-UTF8)★:
    //   there the agent knows only the bare name and cannot resolve it, so it effectively cannot send; the only signal is the warn log.
    //   The None branch leaves no send entry and the daemon already cuts it fail-closed at provision — a defensive path not reached here.
    if let Some(send_exe) = &endpoint.send_exe {
        env.push((
            CLI_EXE_ENV.to_string(),
            send_exe.to_string_lossy().into_owned(),
        ));
        // ★PATH injection (ADR-0094 bare name resolution)★: grants (`Bash(<CLI_EXE_NAME>:*)`) and priming teach the bare name, so the
        //   agent's shell (and its child Bash tool) must be able to **find** it. The **parent directory** of send_exe goes at the **very
        //   front** of PATH.
        //
        // ★base = the PATH already in the env vector (profile first, FIX-1)★: rebuilding from the daemon PATH (std::env::var_os) would
        //   erase the custom PATH the profile carried, so the daemon PATH is the fallback only when the vector has no PATH.
        //   ★Key case (Windows)★: "Path" and "PATH" are the same variable.
        //   ★last-match-wins + dedupe (load-bearing)★: the transport (portable-pty) applies env **in order** with cmd.env(k,v), so with
        //   duplicates the **last** value lives (Windows `[("PATH", daemon), ("Path", profile)]`). So the last case-equivalent PATH is
        //   both base and winning entry,
        //   replaced in place with that key notation, and **all other PATH entries are removed** — a surviving later duplicate would win
        //   and **silently neutralize** the injection. Composition: `send_exe_parent + separator + base` — sibling directory **first**
        //   (shadowing defense),
        //   profile/daemon PATH **survives as the tail**.
        if let Some(parent) = send_exe.parent() {
            let is_path_key = |k: &str| engram_dashboard_platform::env::env_key_eq(k, "PATH");
            let winner_idx = env.iter().rposition(|(k, _)| is_path_key(k));
            let base_os = winner_idx
                .map(|i| std::ffi::OsString::from(env[i].1.clone()))
                .or_else(|| std::env::var_os("PATH"));
            let mut dirs = vec![parent.to_path_buf()];
            if let Some(base) = &base_os {
                dirs.extend(std::env::split_paths(base));
            }
            match std::env::join_paths(dirs)
                .ok()
                .and_then(|j| j.into_string().ok())
            {
                Some(joined) => match winner_idx {
                    Some(i) => {
                        env[i].1 = joined;
                        let mut seen = 0usize;
                        env.retain(|(k, _)| {
                            if is_path_key(k) {
                                let keep = seen == i;
                                seen += 1;
                                keep
                            } else {
                                seen += 1;
                                true
                            }
                        });
                    }
                    None => env.push(("PATH".to_string(), joined)),
                },
                // ★loud skip (FIX-2/3)★: on a join failure or non-UTF8 the injection is **skipped entirely** — a lossily converted PATH is
                //   never pushed (silently corrupting non-Unicode entries is worse than skipping). The env vector is left **exactly as it
                //   was**, so the inherited PATH is the safe fallback.
                None => {
                    tracing::warn!(
                        "CLI PATH injection skipped (PATH join failed or non-UTF8) — grants/priming promise the bare `{}` but on this installation the child may not resolve the name; keeping the inherited PATH",
                        CLI_EXE_NAME
                    );
                }
            }
        }
    }
}

// ── Session id record port ─────────────────────────────────────────────────────────

/// A **one-verb** port through which a backend hands the session id it **fetched** from the other side to the assembly point.
///
/// Called exactly once after the id is actually received, never otherwise. The argument is the string as given — ★it does not decide
///   whether it is a uuid★ (the receiver's job), so the port holds for a value that cannot be read as a uuid.
///
/// ★This shape keeps `ProfileRegistry` out of `backend/` (ADR-0004)★: visible here, the link would know the profile schema; the assembly
///   point wraps its recording means in this verb.
/// ★No backend in the name is part of the contract★ — a name "for codex" leaks the same isolation. ★One verb is part of the contract★ — no
///   second verb asking "was it recorded"; the call **order** stands in its place (see [`AgentBackend::open_spawn`]).
///
/// No return: **a failure does not go back to the caller** — the recording side swallows it into a log, so 「it was persisted」 must not be
///   inferred from this port returning.
// ADR-0004
// ADR-0185
pub type SessionIdSink = Arc<dyn Fn(&str) + Send + Sync>;

/// The one-verb port that tells the assembly point "this incarnation's conversation really has a turn now" — the
/// assembly point plugs the submission input of the session-id first-submission latch in here.
///
/// ★Only the transport of a backend whose transport carries the turn (`MidTurnPolicy::TransportOwned`) calls it★ —
///   that mode's session does not count submissions, and the transport calls this at the peer's **first user-message
///   echo**. Every other backend's session counts before sending, so it ignores this port. ★The two are a pair★: a
///   backend that declares `TransportOwned` without plugging this port into its transport never persists an id (no
///   error — every resume becomes a new conversation).
/// The argument = whether the instructions reached the model: `true` when the echoed turn carried this incarnation's
///   instructions-change context, or when there was none to deliver; `false` when a context was due and its delivery is
///   not confirmed (the retry bound dropped it, or the first echo was not of a carrying turn — a carrying turn's late
///   echo that arrives while idle also reports `false`). Session-id persistence does
///   not depend on it.
/// It may be called many times — the receiver (the latch) counts only the first call and keeps that call's argument;
///   the manager records the applied instructions hash only when that first argument is `true`.
// ADR-0226
// ADR-0231
// ADR-0337
pub type FirstTurnSink = Arc<dyn Fn(bool) + Send + Sync>;

/// Implemented as a unit struct and used as &'static — stateless.
pub trait AgentBackend: Send + Sync {
    /// Whether **we** draw the session id and hand it to this program at spawn.
    ///
    /// If true, the manager draws a uuid on every Fresh, passes it as the `session_id` of [`AgentBackend::build_spec`], and persists it to
    ///   the profile **at the first submission** (ADR-0226 — not at spawn). The sid drift observer takes it as its reference too.
    ///
    /// ★Do not read false as 「there is no session」★: the program may **issue its own** id; planting our uuid would leave a value the
    ///   program never uses in the profile and the resume decision on that fake value. Whether it can resume is the separate axis
    /// ([`AgentBackend::can_resume_stored_session`]).
    // ADR-0185
    fn assigns_session_id(&self, command: &AgentCommand) -> bool;

    /// **Can this command be resumed with the backend sid stored in the profile**.
    ///
    /// Boot restore and the activation entry decide 「Resume or Fresh」 from this together with whether a sid exists. It **does not ask who
    ///   issued** the sid (ADR-0185), only whether the stored value is used for resuming.
    ///
    /// ★It receives `command`★: even one program **differs by the shape it is launched in** — an argument-less predicate would apply one
    ///   shape's value to the other.
    // ADR-0185
    fn can_resume_stored_session(&self, command: &AgentCommand) -> bool;

    /// Whether this backend **consumes** the daemon control channel (MCP entry) (ADR-0086 F3).
    /// If true, the manager calls provision before spawn (token + mcp-config) and passes the endpoint to build_spec
    ///   (claude=`--mcp-config`). If false, the manager **does not touch provision** — a backend like shell does not touch the registry,
    ///   so a config-write failure
    /// cannot abort a spawn that never needed MCP.
    ///
    /// ★fail-closed applies only to backends that **call** provision★: for a true backend a provision Err aborts the spawn (no agent
    ///   secretly running without a control channel).
    fn supports_control_channel(&self) -> bool;

    /// Whether this backend (program) **attaches the daemon control channel through MCP** (ADR-0099 → ADR-0209). Today claude = true
    ///   (mcp-config file via `--mcp-config`) · codex = true (command-line override) · shell · gemini stub = false.
    /// ★The name says 「does it accept a config」 but today's meaning is closer to 「does it use mail through MCP」 — kept knowingly
    ///   (combination table below); 「does it consume a file」 is a separate field★.
    /// ★Backend knowledge (ADR-0004)★: "which program consumes MCP how" is declared here — the manager does not branch with `matches!`.
    ///
    /// This flag drives the provision's grants and **mail permission** (`mail_allowed`). True →
    /// `DaemonControlChannel::provision` puts the MCP bits on the endpoint (with the MCP send grant when the backend
    /// uses mail) and refuses CLI mail;
    /// false → no MCP bits, and CLI mail is allowed when the backend uses mail (ADR-0133). It does not decide priming —
    /// the priming comes only from the launch materials (ADR-0370). Consistency invariant: the mail channel the priming
    /// teaches **=** the mail channel the spawn can use — teaching a channel it cannot use brings back the send freeze,
    /// and not teaching one it can use leaves an uncontrolled bypass surface. The control-CLI wiring goes to every spawn
    /// regardless of this axis.
    ///
    /// ★**This field no longer decides the mcp-config file write — do not revert**★ (ADR-0209 · the `writes_mcp_config_file` gate of
    ///   `control/mod.rs`): 「does it read the file we wrote」 is [`AgentBackend::writes_mcp_config_file`], and the write and its
    ///   fail-closed `?` hang only on
    /// that field; otherwise turning this on to match the mail axis gave codex **a plaintext Bearer token JSON nobody opens** on every spawn.
    ///
    /// ★All four combinations of the two fields are legitimate — especially `(true, false)`★:
    ///   - `(true, true)` = claude. Mail through MCP, and that entry is a file we wrote (`--mcp-config`).
    ///   - **`(true, false)` = codex. Mail through MCP but never opens that file** — attached by command-line override (`-c
    ///   mcp_servers.<server>=…`), token in **an env that value names**; no copy can appear on disk, which is the worth of this combination.
    ///   - `(false, false)` = shell · gemini stub. Outside the mail plane or unmeasured.
    ///   - `(false, true)` is used by none (the file is for the MCP entry).
    /// ★A backend that turns on only the first field and **does not look at** the second silently gets no MCP★: the second defaults to
    ///   `false`, so a backend that **must read** the file but does not declare it gets no file, `config_path` is `None`, its
    ///   `--mcp-config` injection runs not one
    /// line, and the symptom is the silence of 「no tools」. A new backend must **answer both together**.
    ///
    /// ★false picks 「no priming」, not 「another priming」 — do not revive a variant axis★: the CLI-only copy
    ///   (`prompts/agent-priming-cli.md`) was deleted in commit `2ef6902`. A non-MCP spawn gets no instructions, and the
    ///   consistency invariant above holds by **teaching nothing** (a document that describes a missing tool is a lie;
    ///   silence is not — ADR-0209 decision 3). The priming comes only from the launch materials, and the tripwire
    ///   `tests::every_backend_with_a_settings_check_has_what_the_priming_teaches` keeps every backend that can take it
    ///   MCP- and mail-capable (ADR-0370).
    ///
    /// ★Relation to `supports_control_channel`★: that is "does it **call** provision"; this is "among the channels provision attaches,
    ///   **can it plug in MCP**" — orthogonal. Claude and codex are both true here (codex's attachment is not a file — table above), shell
    ///   · gemini stub both false;
    /// "control channel only through the CLI" would be the former true, the latter false.
    // ADR-0126
    // ADR-0133
    // ADR-0209
    fn accepts_mcp_config(&self) -> bool;

    /// Whether this backend **reads the mcp-config file the daemon wrote, by path** (claude=true — `--mcp-config <path>`).
    ///
    /// ★Split off the field above — do not fold it back★: the field above effectively means 「does it use mail through MCP」 (the daemon
    ///   derives `mail_allowed` from it — ADR-0133); this asks only 「do we write a plaintext Bearer token JSON to disk」. Folded, codex got
    ///   **a plaintext token file
    ///   nobody reads** on every spawn (default ACL · deleted on revoke) and the fail-closed write could **cut a spawn over a file nobody
    ///   reads**.
    /// ★Default = false (writes no secret)★, opposite to the fail-open of the two mail axes below because the costs differ: a wrong false
    ///   is **a missing file** (that backend's MCP does not attach and the contract table breaks on that row — loud); a wrong true is **a
    ///   plaintext token
    ///   on disk + a spawn-abort risk** (quiet). No secrets for unknown backends.
    /// ★A backend that declared true keeps fail-closed★ — without this file its MCP entry physically disappears, so continuing on a write
    ///   failure would run an agent without a control channel.
    // ADR-0086
    // ADR-0099
    // ADR-0209
    fn writes_mcp_config_file(&self) -> bool {
        false
    }

    /// Places this incarnation's materials `kit` in the shape this backend carries — disk writes only through `fs`, only inside the
    /// volatile folder (never in the working folder). The returned value goes as `artifacts` to the same incarnation's
    /// [`AgentBackend::build_spec`] · [`AgentBackend::open_spawn`].
    ///
    /// `cwd` = the same normalized working folder [`AgentBackend::build_spec`] receives.
    /// `Err(reason)` = the spawn is aborted — what was written meanwhile is reclaimed with the volatile folder by [`crate::launch::LaunchKitSource::release`].
    /// ★Default = nothing to carry · no writes★ — only `real` is `kit.plan.real` as is; the rest is
    ///   `LaunchArtifacts::default()`. A backend that does not translate materials carries nothing even when given real ones.
    // ADR-0334
    fn materialize(
        &self,
        _command: &AgentCommand,
        kit: &LaunchKit,
        _cwd: &Path,
        _fs: &dyn LaunchFs,
    ) -> Result<LaunchArtifacts, String> {
        Ok(LaunchArtifacts {
            real: kit.plan.real,
            ..LaunchArtifacts::default()
        })
    }

    /// cwd and env are passed as values the manager normalized.
    ///
    /// `control` (ADR-0086): the control channel endpoint the daemon issued (an abstract descriptor). If present the backend injects it
    ///   into the command line its program's way (claude=`--mcp-config <path>` — that knowledge is `backend/claude/` alone, ADR-0004);
    ///   None, or a backend not using
    ///   the control channel (shell), ignores it.
    ///
    /// `resume_session_id` = the stored backend sid this spawn **will resume**; `None` = no resume (Fresh, no stored value, or this
    ///   backend's resume axis off).
    /// ★**Not the `session_id` above — do not fold them**★: that is a value we issue ([`AgentBackend::assigns_session_id`]); this is a
    ///   value the other side issued and we recorded ([`SessionIdSink`]). Folding would falsify `assigns_session_id() == false`, since a
    ///   value we passed would be
    ///   carried in the argv of a backend whose issuing axis is off.
    /// ★**The same value** as the field of the same name in [`AgentBackend::open_spawn`]★ — only **what is done** differs: a backend
    ///   resuming by command line (codex terminal) assembles argv here, one whose link requests the resume (codex app-server) picks the
    ///   first request there.
    /// ★Looking at the mode together is legitimate here, unlike `open_spawn`★: this place receives `mode`, and **Fresh and Resume argv
    ///   differ anyway** (claude's `--session-id` ↔ `--resume`); the assembly point passes this field empty on Fresh.
    ///
    /// `artifacts` = this incarnation's [`AgentBackend::materialize`] result. With nothing to carry (the default) the argv is
    ///   byte-identical to the golden `tests/launch_argv_golden.rs`.
    // ADR-0185
    // ADR-0208
    // ADR-0334
    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        resume_session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        control: Option<ControlEndpoint>,
        artifacts: &LaunchArtifacts,
    ) -> CommandSpec;

    /// Whether this spawn **can be set up** with the control endpoint the daemon issued. `Err(reason)` = fail-closed (spawn aborted).
    ///
    /// ★Cannot be done inside [`AgentBackend::build_spec`]★: it only returns a `CommandSpec` and has no way to say 「could not build the
    ///   arguments」 — the spawn would launch with silently missing arguments. The assembly point asks this **before** assembly and cuts
    ///   the spawn on `Err`
    ///   (the issued token is reclaimed by its provision guard).
    /// ★Default = `Ok(())`★ — most backends have no such state, and claude's daemon-side `?` (mcp-config write failure) already cuts it.
    ///   Only backends with **failures the daemon cannot see** need it — codex's MCP attachment value (no file written, so the daemon
    ///   cannot cut it).
    /// ★Do not cut 「not authorized」 here★ — a spawn the daemon gave no mail entry is **normal** (operator turned it off or the backend is
    ///   outside the mail plane); cutting it would kill that combination. This measures only 「must attach but cannot」.
    // ADR-0004
    // ADR-0209
    fn precheck_control_endpoint(
        &self,
        _command: &AgentCommand,
        _control: Option<&ControlEndpoint>,
    ) -> Result<(), String> {
        Ok(())
    }

    /// The caps this backend (program) decides — session (resume) · model. The final Capabilities are composed by
    ///   `Capabilities::compose(transport_caps, backend_caps)` with the input/output/control the transport makes.
    ///
    /// It receives `command` (FIX 5): even one program (claude) has **caps that differ by mode**, so the backend is the source of the
    ///   session caps (ADR-0030) and reports honestly from command (type split kept — output/control stay the transport's).
    fn capabilities(&self, command: &AgentCommand) -> BackendCaps;

    /// The **physical link shape** this backend requires for `command`.
    ///
    /// ★Why the backend (ADR-0004)★: "through which link must this program be launched" is per-program knowledge.
    /// ★Do not bring a decision predicate back to the shared layer★: the assembly point asking the `AgentCommand` payload would let one
    ///   backend's output-format axis drive everyone's link choice.
    /// ★Not the render mode axis★: what the program draws (terminal TUI ↔ structured stream) is an axis of its command payload; this is
    ///   what that choice requires of **our link**. Even tied 1:1 inside one backend, do not merge the vocabularies — a new backend would
    ///   report its render mode
    ///   under our link's name.
    /// ★Default = `Pty`★: a backend that declares nothing launches as a terminal.
    // ADR-0004
    // ADR-0044
    fn transport_shape(&self, _command: &AgentCommand) -> TransportShape {
        TransportShape::Pty
    }

    /// Whether this backend's link must set up a connection (the doc of the public wrapper [`declares_link`] is canonical).
    /// ★Default = `false`★ — a backend that declares nothing gets no delivery port and takes the old decision path.
    // ADR-0004
    fn declares_link(&self, _command: &AgentCommand) -> bool {
        false
    }

    /// This backend **builds the physical link itself** to launch `command` and hands it out at once with the other values session
    ///   assembly needs.
    ///
    /// ★Why the backend (ADR-0191)★: "with which link implementation is this program launched" is per-program knowledge; the assembly
    ///   point matching [`TransportShape`] to pick a constructor would make **two deciding places** (`backend_for` + that match) and know
    ///   backend-only link types (ADR-0004).
    /// ★The assembly point does not know the returned link's actual type★ (`Box<dyn AgentTransport>`).
    /// ★Aspects are gathered in one dispatch★: caps, encoder, turn classifier and mail eligibility stay owned by their own methods and are
    ///   **only gathered** here, so the assembly point does not ride the same switch per aspect.
    /// ★The link implementation hardcoding `output.structured` is still forbidden (ADR-0044/0030)★ — the backend that chose the structured
    ///   pipe injects it. `cols`/`rows` are for terminal links only (a pipe ignores them).
    ///
    /// `sid_sink` = where a backend whose other side **issues** the session id sends it back ([`SessionIdSink`]); `None` = the assembly
    ///   point gave no recording means. ★A backend that does not receive the id itself ignores it★ (claude observes its own file via the
    ///   observer path).
    /// ★Order contract★: this port is called **before anything is sent through that session**, so no second verb asking back "was it
    ///   recorded" is needed (why one verb — [`SessionIdSink`]).
    ///
    /// `first_turn_sink` = where to announce the moment a turn actually happened ([`FirstTurnSink`]). ★Only a backend reporting
    ///   `MidTurnPolicy::TransportOwned` plugs it into its link — the rest ignore it★ (pairing discipline: that type's doc).
    ///
    /// `resume_session_id` = the stored backend sid this spawn **will resume**; `None` = no resume.
    /// ★**Not the `session_id` of [`AgentBackend::build_spec`] — do not fold them**★: that is a value we issue
    ///   ([`AgentBackend::assigns_session_id`]), this one the other side issued and we recorded ([`SessionIdSink`]).
    /// ★Not receiving the mode is intended★ — the assembly point passes this field empty on Fresh; both mode and value would make 「Fresh
    ///   but with a value to resume」 expressible and interpreted differently per backend.
    /// ★A backend resuming by command line (claude) ignores it★ (`build_spec` assembles `--resume <sid>`); it is for a backend **whose
    ///   link requests the resume**.
    /// ★Default = PTY + the values each aspect reports★: a backend not building its own link launches as a terminal (as the
    ///   [`AgentBackend::transport_shape`] default); other fields carry their own methods' output.
    /// ★But this default **does not read** `transport_shape`★: reporting a pipe while not implementing this method silently launches a
    ///   terminal, and the declaration-table tripwire (`tests::expected_codec_axis`) measures only the reported value, so it cannot see
    ///   that mismatch.
    ///
    /// `control` = the **same endpoint** [`AgentBackend::build_spec`] received — for a backend whose channel needs
    ///   control-plane data when it opens (claude's leftover cleaner reads the agent id from it).
    /// ★Do not send again here what `build_spec` already put on the command line★ — when one spawn sends the same
    ///   value by two means, our behavior hangs on how the other side merges them. Each mode has one assembly point
    ///   (the same rule that splits `resume_session_id` between argv and the handshake).
    /// ★Most backends ignore this field★ — a backend that translates everything into the command line is done in
    ///   `build_spec`.
    ///
    /// `artifacts` = the **same** [`AgentBackend::materialize`] result [`AgentBackend::build_spec`] received — for a backend
    ///   with materials to carry over its channel. A backend that puts everything on the command line ignores it.
    // ADR-0004
    // ADR-0191
    // ADR-0215
    // ADR-0334
    fn open_spawn(
        &self,
        command: &AgentCommand,
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
        sid_sink: Option<SessionIdSink>,
        first_turn_sink: Option<FirstTurnSink>,
        resume_session_id: Option<Uuid>,
        // Where the connection's outcome is delivered — comes only to backends whose `declares_link()` is true.
        _link_sink: Option<LinkSink>,
        control: Option<&ControlEndpoint>,
        artifacts: &LaunchArtifacts,
    ) -> Result<SpawnParts, PtyError> {
        // This default fetches no session id, has no link-owned turn, no link resume, and no handshake control data or materials —
        //   dropped explicitly instead of underscore names (names must match the doc above: rustdoc renders the signature).
        let _ = (
            sid_sink,
            first_turn_sink,
            resume_session_id,
            control,
            artifacts,
        );
        let (transport, child_pid) = PtyTransport::open(spec, cols, rows)?;
        Ok(SpawnParts {
            transport: Box::new(transport),
            child_pid,
            backend_caps: self.capabilities(command),
            encoder: self.input_encoder(command),
            turn_classifier: self.turn_classifier(),
            reads_messages: self.reads_messages(),
            mid_turn: MidTurnPolicy::None,
            delivery_ack: Arc::new(DeliveryAck::new()),
        })
    }

    /// This backend's **turn-signal classifier** (the backend-knowledge share of the ADR-0113 fact layer).
    ///
    /// ★Why the backend (ADR-0004 · ADR-0110 decision 4)★: "which output events are turn progress and which are turn ends" is per-program.
    ///   `OutputEvent::Structured` is **a per-backend escape hatch** whose `kind` meaning differs per backend — interpreting it in the
    ///   shared layer would
    ///   force one backend's convention on all (a backend emitting structured meta lines would be "in a turn" forever with no end signal).
    /// ★One shared signal vocabulary (`TurnSignal`)★: only the **mapping** differs per backend; a per-backend signal enum would make
    ///   consumers know the backend.
    /// ★Default = no signal (fail-open)★: unobserved is absorbed by the consumer as "deliver immediately", while a progress signal without
    ///   grounds creates an "in a turn" that cannot be woken — so the default is silence.
    // ADR-0004
    // ADR-0113
    fn turn_classifier(&self) -> TurnClassifier {
        no_turn_signals
    }

    /// When resuming failed, **can the cause be recognized from that program's output** (ADR-0172 classification entry).
    ///
    /// `evidence` = the text that incarnation left (best-effort — may be empty). The caller passes **the two streams merged**: console
    ///   tail (PTY) + diagnostic stderr tail (pipe); they fill exclusively by transport, so the implementation only looks at the text.
    /// `None` = not settled from this text alone → the caller falls back to the contextual default.
    ///
    /// ★Why the backend (ADR-0004)★: "what this text means" is per-program; the manager reading the string would leak that.
    /// ★Default = unknown (fail-open)★.
    /// ★Also called for live sessions★: the caller settles from the diagnostic stream without waiting for exit
    ///   (`EarlyVerdict::Diagnosed`), so declare **only text that program can emit only when it failed** — text that could appear in the
    ///   conversation body would stamp a successful activation a failure.
    // ADR-0004
    // ADR-0172
    fn resume_failure_kind(&self, _evidence: &str) -> Option<AgentFailureKind> {
        None
    }

    /// Whether this backend **is a reader of mail** = eligibility for the recipient list of agent-to-agent mail.
    ///
    /// ★Not the "turn observability" axis (does not revert ADR-0116 decisions 1·7)★: terminal claude without structured output **still
    ///   receives**. What is decided is **what the input is interpreted as** — shell **executes** input as commands, so an envelope
    ///   reaching it is executed, not read
    ///   (the body is LLM free text; `&`·`|`·`;` would parse as a separate command).
    /// ★Default = true (fail-open)★: for a new CLI backend reading mail is the normal side, and cutting delivery on unknown would make
    ///   mail silently vanish; only an execution channel declares false.
    // ADR-0004
    fn reads_messages(&self) -> bool {
        true
    }

    /// Whether this backend **is assigned a mail channel** = the sending pair of [`AgentBackend::reads_messages`].
    ///
    /// ★Do not bring back the state without this field★: the daemon derived mail availability from `!accepts_mcp_config` **alone**, so a
    ///   backend 「using the control channel but not mail」 got **send authorization opened with it** while receiving stayed false — an
    ///   asymmetry declared nowhere.
    /// ★This axis is **stated by the backend** (ADR-0004)★: whether a program comes and goes on the mail plane is per-program fact. The
    ///   daemon derives **one value** (`mail_allowed`) from this and `accepts_mcp_config` (the single derivation point of ADR-0133 decision 2).
    /// ★Default = true (fail-open)★ as the receiving field above; only a backend outside the mail plane declares false.
    /// ★`false` means "cannot use mail", not "cannot use control"★ — control verbs are open to everyone (ADR-0132 decision 5).
    // ADR-0004
    // ADR-0133
    // ADR-0209
    fn uses_mail(&self) -> bool {
        true
    }

    /// The **input encoding tag** this backend uses for `command`; the session receives it once at spawn and keeps it.
    ///
    /// ★Why the backend (ADR-0004)★: "how this program reads stdin" is per-program; picking the tag from the command shape in the dispatch
    ///   layer would leak it.
    /// ★Default = `Raw`★: a made-up envelope on an unknown program's stdin would make it unable to read the input, so undeclared backends
    ///   pass bytes through.
    // ADR-0004
    // ADR-0044
    fn input_encoder(&self, _command: &AgentCommand) -> InputEncoder {
        InputEncoder::Raw
    }

    /// Wraps one text turn into this backend's structured input line — the real thing behind [`InputEncoder::encode`].
    ///
    /// `msg_uuid` **must be the same** as the value passed to [`AgentBackend::input_echo_event`] (generated once, passed to both — the
    ///   dedup key).
    /// ★Default = no wrapping★: for a `Raw` backend [`backend_for_encoder`] gives `None` and this path is never taken; a placeholder
    ///   keeping the contract total, emitting the same bytes as `Raw`.
    fn wrap_input_turn(&self, text: &str, _msg_uuid: Uuid) -> Vec<u8> {
        text.as_bytes().to_vec()
    }

    /// The input-time user echo event the session layer emits right after input succeeds. `None` = none made.
    ///
    /// ★The event shape must equal what that backend's decoder makes for a replay★ — the frontend merges the two by uuid, and a divergence
    ///   leaves two on screen; the implementation owns the shape.
    /// ★Default = none★: where the PTY echoes locally, a synthetic echo would duplicate.
    // ADR-0044
    // ADR-0045
    fn input_echo_event(&self, _text: &str, _msg_uuid: Uuid) -> Option<OutputEvent> {
        None
    }

    /// The output refining decoder plugged in before pump→core. `None` = bytes pass straight through.
    ///
    /// The output-direction pair of [`AgentBackend::input_encoder`] — the side that wraps also owns the side that unwraps. ★Default =
    ///   none★ (terminal and plain text unchanged).
    // ADR-0004
    // ADR-0044
    fn output_decoder(&self, _command: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
        None
    }

    /// The event list restoring this command's past conversation on a resume spawn. An empty Vec = no seed.
    ///
    /// The transcript's file layout and format belong to the implementation; the manager only calls this. ★Default = empty Vec★ (fresh
    ///   buffer behavior).
    // ADR-0004
    // ADR-0079
    fn resume_transcript_events(
        &self,
        _command: &AgentCommand,
        _cwd: &std::path::Path,
        _session_id: Uuid,
    ) -> Vec<OutputEvent> {
        Vec::new()
    }

    /// Whether **a change of this incarnation's session id can be recognized from outside** — and if so, that observer.
    ///
    /// The sid we set at spawn can change inside the program (claude switches with `/clear`·`/resume` while the process stays — spike
    ///   measurement); unfollowed, the next restore revives the old session. Where and how to read it (file path · JSON schema · PID
    ///   workaround) belongs to the
    /// implementation; the runner ([`crate::session_tracker`]) has only the period, registry and callback.
    ///
    /// `agent_id` is only the correlation key of the implementation's diagnostic logs.
    /// ★Default = `None` (no observation)★: an observer without grounds would read someone else's file and write a wrong sid into the
    ///   profile. ★`None` is not a failure★: restore works with the originally set sid (best-effort).
    // ADR-0004
    fn session_id_source(
        &self,
        _agent_id: AgentId,
        _child_pid: u32,
        _expected_sid: Uuid,
    ) -> Option<Box<dyn SessionIdSource>> {
        None
    }
}

/// The bundle a backend hands out **in one dispatch** to assemble one spawn ([`AgentBackend::open_spawn`]).
///
/// ★The caps ownership split stays (ADR-0030)★: `backend_caps` is only the backend's share, composed with the link's input/output/control
///   caps by `Capabilities::compose` in the session layer; merging in advance would lose which side reported what.
// ADR-0191
pub struct SpawnParts {
    pub transport: Box<dyn AgentTransport>,
    pub child_pid: Option<u32>,
    pub backend_caps: BackendCaps,
    pub encoder: InputEncoder,
    pub turn_classifier: TurnClassifier,
    pub reads_messages: bool,
    /// Who classifies and releases mid-turn input — the session decides only by this value.
    // ADR-0231
    pub mid_turn: MidTurnPolicy,
    /// Whether this incarnation can give ack notices — ★carries **the very** Arc the backend handed to its decoder and link★; a separate
    ///   one would split what the session reads from what the decoder fills, showing only as an eternal `Unknown` with no error.
    // ADR-0231
    pub delivery_ack: Arc<DeliveryAck>,
}

/// The output event → turn signal mapping function (ADR-0113). The backend hands out its own function and `OutputCore` holds the pointer
///   for the session's lifetime, calling it on every event.
///
/// ★Why a function pointer★: emit is the hot path on every agent output — dispatching by `AgentCommand` per event would need a command
///   copy or manager retrace, and `Box<dyn Fn>` allocates. The backend is a stateless unit struct, so the mapping is a pure function taken
///   once at spawn:
///   zero allocation, locking and lookup.
pub type TurnClassifier = fn(&OutputEvent) -> Option<TurnSignal>;

/// Assemblies that need no observation (test harnesses) also plug this in to state "no signal" explicitly.
pub fn no_turn_signals(_event: &OutputEvent) -> Option<TurnSignal> {
    None
}

// ── Static singletons ────────────────────────────────────────────────────────────────

static CLAUDE_BACKEND: ClaudeBackend = ClaudeBackend;
static SHELL_BACKEND: ShellBackend = ShellBackend;
static CODEX_BACKEND: CodexBackend = CodexBackend;

// When wiring a new variant: tests::expected_channel_matrix (tripwire) forces a conscious capability declaration — ADR-0099
fn backend_for(c: &AgentCommand) -> &'static dyn AgentBackend {
    match c {
        AgentCommand::Claude { .. } => &CLAUDE_BACKEND,
        AgentCommand::Shell { .. } => &SHELL_BACKEND,
        AgentCommand::Codex { .. } => &CODEX_BACKEND,
    }
}

/// The **backend that owns** the encoding tag — the encoder-axis pair of `backend_for` (the command axis); the session layer holds only
///   the tag, not the command, so it is traced back here.
///
/// `None` = that tag has no backend knowledge (bytes pass through).
/// ★A backend name is written only in this table, `backend_for` right above, [`usage_probes`] ·
///   [`settings_checks`] below and the singleton declarations★ — a backend name outside them means the
///   `backend/<name>/` folder isolation leaked (ADR-0004).
fn backend_for_encoder(e: InputEncoder) -> Option<&'static dyn AgentBackend> {
    match e {
        InputEncoder::Raw => None,
        InputEncoder::ClaudeStreamJson => Some(&CLAUDE_BACKEND),
        // A tag whose envelope is made by the link, so the backend has nothing to wrap — `encode` passes through, and there is no echo.
        InputEncoder::TransportFramed => None,
    }
}

// ── Usage prober registry ───────────────────────────────────────────────────────

/// All active usage probers — the receiver (the daemon) gets the key list, cooldowns and time limits only from here (it does not know
///   vendor names or policies).
///
/// ★Not a field of [`AgentBackend`] on purpose★ — the query runs without an agent; as a trait field it would be a two-layer dispatch that
///   fabricates a command and goes through `backend_for`. Keys are distinct (a test measures it).
// ADR-0004
// ADR-0246
pub fn usage_probes() -> [&'static dyn UsageProbe; 2] {
    [&claude::CLAUDE_USAGE_PROBE, &codex::CODEX_USAGE_PROBE]
}

/// Word → that vendor's prober; `None` = no prober for that word. ASCII case-insensitive — the wire (`"claude"`) and the bus (`"Claude"`,
///   the word of `agent.new`) reach one prober.
/// ★The entry key comes from the returned prober's [`UsageProbe::key`], not the incoming word★ — a key from the incoming spelling would
///   hit different entries.
pub fn usage_probe_for(word: &str) -> Option<&'static dyn UsageProbe> {
    usage_probes()
        .into_iter()
        .find(|probe| probe.key().as_str().eq_ignore_ascii_case(word))
}

// ── Settings-check registry ────────────────────────────────────────────────────

/// Every backend settings check — the caller (asset-library) gets each backend's settings knowledge only from here.
/// A word with no entry gets no settings check — today claude and codex have one.
// ADR-0334
pub fn settings_checks() -> [&'static dyn SettingsCheck; 2] {
    [&claude::CLAUDE_SETTINGS_CHECK, &codex::CODEX_SETTINGS_CHECK]
}

/// Backend word → its settings check, ASCII case-insensitive (the same entrances as [`usage_probe_for`]).
/// `None` = that word has no settings check.
pub fn settings_check_for(word: &str) -> Option<&'static dyn SettingsCheck> {
    settings_checks()
        .into_iter()
        .find(|check| check.backend().eq_ignore_ascii_case(word))
}

// ── Free-function dispatch ─────────────────────────────────────────────────────────

pub fn assigns_session_id(c: &AgentCommand) -> bool {
    backend_for(c).assigns_session_id(c)
}

pub fn can_resume_stored_session(c: &AgentCommand) -> bool {
    backend_for(c).can_resume_stored_session(c)
}

/// Whether this profile **can be resumed with the stored sid** = the resume axis ∧ a stored sid exists.
///
/// ★Keeps the activation entries from each writing this rule★: boot restore, the command bus and WS all call here; a copy could transcribe
///   one of the two terms and only that entry would silently decide differently.
/// ★Does not know an explicit resume request★ — the calling entry decides whether to add it.
// ADR-0185
pub fn can_resume_profile(p: &AgentProfile) -> bool {
    can_resume_stored_session(&p.command) && p.backend_session_id.is_some()
}

pub fn supports_control_channel(c: &AgentCommand) -> bool {
    backend_for(c).supports_control_channel()
}

pub fn accepts_mcp_config(c: &AgentCommand) -> bool {
    backend_for(c).accepts_mcp_config()
}

pub fn writes_mcp_config_file(c: &AgentCommand) -> bool {
    backend_for(c).writes_mcp_config_file()
}

pub fn uses_mail(c: &AgentCommand) -> bool {
    backend_for(c).uses_mail()
}

/// Canonical doc = [`AgentBackend::materialize`]. ★This call may write to disk★ (through `fs`, only inside the volatile folder).
///
/// ★The backend is never handed `fs` as is★ — it gets a port confined to `kit.volatile_dir`. A write anywhere else (the working
///   folder included) fails the placement (`place_in_volatile`).
// ADR-0334
// TRD S25 §1-5 (na) 6
pub fn materialize(
    c: &AgentCommand,
    kit: &LaunchKit,
    cwd: &Path,
    fs: &dyn LaunchFs,
) -> Result<LaunchArtifacts, String> {
    crate::launch::place_in_volatile(kit, fs, |confined| {
        backend_for(c).materialize(c, kit, cwd, confined)
    })
}

pub fn build_command_spec(
    c: &AgentCommand,
    mode: SpawnMode,
    session_id: Option<Uuid>,
    resume_session_id: Option<Uuid>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    control: Option<ControlEndpoint>,
    artifacts: &LaunchArtifacts,
) -> CommandSpec {
    backend_for(c).build_spec(
        c,
        mode,
        session_id,
        resume_session_id,
        cwd,
        env,
        control,
        artifacts,
    )
}

/// The fail-closed gate **right before** assembly (canonical doc = [`AgentBackend::precheck_control_endpoint`]).
///
/// ★The assembly point must call this **after** the provision guard is armed★ — so a cut here reclaims the token and file already issued.
pub fn precheck_control_endpoint(
    c: &AgentCommand,
    control: Option<&ControlEndpoint>,
) -> Result<(), String> {
    backend_for(c).precheck_control_endpoint(c, control)
}

pub fn transport_shape(c: &AgentCommand) -> TransportShape {
    backend_for(c).transport_shape(c)
}

/// The real link and the aspect values on it are owned by [`AgentBackend::open_spawn`]; this function is only dispatch — a new backend
///   implements that method in its own folder and this function is untouched (swappability). ★This call launches the child process★ — unlike the
/// dispatches around it it has side effects, and if it fails that spawn does not hold.
// ADR-0191
pub fn open_spawn(
    c: &AgentCommand,
    spec: &CommandSpec,
    cols: u16,
    rows: u16,
    sid_sink: Option<SessionIdSink>,
    first_turn_sink: Option<FirstTurnSink>,
    resume_session_id: Option<Uuid>,
    link_sink: Option<LinkSink>,
    control: Option<&ControlEndpoint>,
    artifacts: &LaunchArtifacts,
) -> Result<SpawnParts, PtyError> {
    backend_for(c).open_spawn(
        c,
        spec,
        cols,
        rows,
        sid_sink,
        first_turn_sink,
        resume_session_id,
        link_sink,
        control,
        artifacts,
    )
}

/// Whether this backend's link can be used only **after setting up a connection** = whether there is an axis to deliver the outcome.
///
/// ★The assembly point decides by this **whether to lay down** the delivery port★ — a port to a backend without the axis would make the
///   supervisor wait on a channel nobody calls. A `false` backend (claude · shell · stdio · codex terminal) keeps the old path byte for byte.
/// ★Why the backend (ADR-0004)★: 「usable right after launch, or needs a round trip」 is per-program; the manager matching command would leak it.
pub fn declares_link(c: &AgentCommand) -> bool {
    backend_for(c).declares_link(c)
}

pub fn turn_classifier(c: &AgentCommand) -> TurnClassifier {
    backend_for(c).turn_classifier()
}

pub fn resume_failure_kind(c: &AgentCommand, evidence: &str) -> Option<AgentFailureKind> {
    backend_for(c).resume_failure_kind(evidence)
}

pub fn reads_messages(c: &AgentCommand) -> bool {
    backend_for(c).reads_messages()
}

pub fn session_id_source(
    c: &AgentCommand,
    agent_id: AgentId,
    child_pid: u32,
    expected_sid: Uuid,
) -> Option<Box<dyn SessionIdSource>> {
    backend_for(c).session_id_source(agent_id, child_pid, expected_sid)
}

// ── Input encoding (ADR-0044/0004) ────────────────────────────────────────────────

/// The encoding method applied **right before** session input (write_input) is sent to the transport. AgentSession receives it at spawn,
///   keeps it, and applies it on every write_input.
///
/// ★Design intent★: the transport always writes only raw bytes (a dumb pipe — ADR-0044); wrapping a text turn into a claude JSON line is
///   the backend's knowledge. The session holds only this tag and the schema lives only inside the [`AgentBackend::wrap_input_turn`]
///   implementation
/// (ADR-0004 isolation — neither this module nor the session nor the transport knows the shape).
/// The physical link shape the backend requires — a **reported value**. The real link is built by [`AgentBackend::open_spawn`], and even
///   there this value is not matched again to pick a constructor (ADR-0191 — the only deciding switch is `backend_for`); today the only
///   reader is the
/// declaration-table tripwire (`tests::expected_codec_axis`).
///
/// ★Whether output is structured is implied here★: `StdioNdjson` means line-based JSON, which the link itself does not know (a dumb pipe —
///   ADR-0044), so the `structured` cap is injected by the backend that chose the pipe (ADR-0030).
// ADR-0004
// ADR-0044
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportShape {
    /// Terminal (ConPTY/pty) — bytes as is, resize available.
    Pty,
    /// stdio pipe + line-based JSON output — no resize concept.
    StdioNdjson,
    /// stdio pipe + line-based JSON flowing **both ways** — no resize concept.
    ///
    /// ★Unlike [`TransportShape::StdioNdjson`]★: that is a one-way pipe where the other side reads our bytes as is and we only read its
    ///   lines, so the backend encoder ([`InputEncoder`]) makes the envelope. This is a protocol where the other side **also asks us and
    ///   waits for the answer**, so
    ///   the link itself holds id, method and order of outgoing lines — the bytes [`AgentTransport::send_input`] receives are **the
    ///   message body**, not a wire frame, and the encoder does not touch them ([`InputEncoder::TransportFramed`]).
    StdioBidiJson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEncoder {
    /// Bytes pass through as is (PTY/terminal · shell). **Byte-identical** to the existing behavior.
    Raw,
    /// claude stream-json: wraps one text turn into a user JSON line (`\n`-terminated) (schema = `backend/claude/`).
    ClaudeStreamJson,
    /// The envelope **is made by the link** — the encoder passes bytes through as is ([`backend_for_encoder`] is `None`).
    ///
    /// ★`Raw` is not reused★: `Raw` imitates a human keyboard through a PTY, so [`InputEncoder::submit_sequence`] emits a CR and the
    ///   session emits it **as a `send_input` call separate from the body** ([`crate::session::AgentSession::submit_input_observed`]);
    ///   reaching a link that builds
    ///   the envelope itself, it would send one more turn with an empty body.
    TransportFramed,
}

impl InputEncoder {
    /// `msg_uuid`: the message uuid of this user turn (replay dedup key). ClaudeStreamJson plants it in the stdin user line so claude
    ///   echoes it back as is on replay (dedup contract: `backend/claude/`); the same write_input passes it to input_echo_event, so the
    ///   synthetic echo and the replay merge by
    ///   uuid. Raw (terminal · shell) ignores it — byte-identical.
    pub fn encode(&self, bytes: &[u8], msg_uuid: Uuid) -> Vec<u8> {
        match backend_for_encoder(*self) {
            None => bytes.to_vec(),
            // ※from_utf8_lossy (FIX 6b): non-UTF8 input may be replaced with U+FFFD and corrupted, but structured input is
            //   a text chat message, so UTF-8 is a premise (MVP = text chat, ADR-0044) → allowed.
            Some(b) => b.wrap_input_turn(&String::from_utf8_lossy(bytes), msg_uuid),
        }
    }

    /// Builds the **input-time user echo event** the session layer core.emits right after input succeeds (ADR-0044/0045).
    ///
    /// ★Why here (backend)★: for the terminal (Raw) the PTY echoes locally at once, but in json mode input appears only when claude echoes
    ///   it back; a synthetic user event right after write_input removes that round-trip delay. Which encoder needs it and the event's
    ///   json schema are backend knowledge
    ///   (ADR-0004 — the session holds only the encoder tag). Raw returns None (the PTY already echoes — no duplicates).
    ///
    /// ★Paired with the decoder's uuid dedup★: this event has the same shape as what the decoder makes for a replayed user-role block
    ///   (`Structured{kind:"user", json:{"type":"text","text":<raw>,"uuid":"X"}}`) and `msg_uuid` equals the value planted in stdin
    ///   (encode), so the later replay (same uuid)
    ///   is deduped in the frontend accumulator and merged into one; past/non-matching-uuid user text is all preserved.
    pub fn input_echo_event(
        &self,
        bytes: &[u8],
        msg_uuid: Uuid,
    ) -> Option<crate::types::OutputEvent> {
        backend_for_encoder(*self)?.input_echo_event(&String::from_utf8_lossy(bytes), msg_uuid)
    }

    /// The submit (turn start) bytes that must be sent once more **as a separate write** after the encoded body. `None` = the one body
    ///   write is already the submit.
    ///
    /// ★Cannot be appended inside `encode` (measured 2026-08-17 — do not bring it back)★: `body + CR` in **one write** into a live claude
    ///   TUI leaves the text in the input box and the turn does not start. The submit depends on **when the receiver reads it**, which `encode`
    ///   (one byte sequence) cannot express — hence a separate question.
    /// ★Splitting the write alone is not enough★: **without a gap** the two writes bundle into one PTY chunk and one read, same as writing
    ///   together; so a paired wait is needed — [`SUBMIT_PACING`] (the two go together — do not fix one side).
    /// ★`Raw` = CR★ (terminal · shell imitate a human keyboard through a PTY; Enter = 0x0D). ★`ClaudeStreamJson` = None★: the terminating
    ///   `\n` of `encode` is that protocol's submit, and a CR would add surplus bytes. ★`TransportFramed` = None★: one body byte sequence
    ///   is one turn and the link
    ///   makes the envelope; a value here would make the session emit those bytes again **as a separate `send_input` call** and the link
    ///   would open an empty turn.
    /// ★The keystroke path does not look at this★: the only consumer is `AgentSession::submit_input_observed` ("one completed message =
    ///   one turn"); a submit inserted into terminal streaming input where a person types Enter (`write_input`) would submit a turn per
    ///   keystroke.
    // ADR-0004
    pub fn submit_sequence(&self) -> Option<&'static [u8]> {
        match self {
            InputEncoder::Raw => Some(b"\r"),
            InputEncoder::ClaudeStreamJson => None,
            InputEncoder::TransportFramed => None,
        }
    }

    /// Whether this one piece of input **submits a user turn** — the criterion the session id first-submission latch counts by.
    ///
    /// - `ClaudeStreamJson`·`TransportFramed` → always `true`: 1 call = 1 complete user turn (FIX 6a of `AgentSession::write_input` · the
    ///   codex link makes one turn per write).
    /// - `Raw` → `true` if it contains the submit bytes (CR).
    ///
    /// ★The CR decision of `Raw` is tied to the frontend's key encoding★: Enter = CR is xterm's **default** encoding; a mode sending Enter
    ///   differently (kitty keyboard · win32-input-mode) makes this **silently** false, submits are not counted and every resume of that
    ///   mode disappears.
    /// Paired with [`InputEncoder::submit_sequence`], which stands on the same assumption — do not fix only one side.
    // ADR-0226
    pub fn submits_turn(&self, bytes: &[u8]) -> bool {
        match self {
            InputEncoder::Raw => bytes.contains(&b'\r'),
            InputEncoder::ClaudeStreamJson | InputEncoder::TransportFramed => true,
        }
    }
}

/// The **wait placed between** the body write and the submit write ([`InputEncoder::submit_sequence`]).
///
/// ★Needed (measured 2026-08-17 — do not delete)★: splitting the write alone does not submit — **without a gap** the two writes bundle
///   into one PTY chunk, the claude TUI receives them in **one read** and treats the CR as part of pasted text, keeping it in the input
///   box. What makes the
///   boundary is **the receiver's read boundary**, and this wait widens it.
/// ★The observation "it submits even with 0 delay" was a measurement error (do not bring it back)★: that measurement sent control-plane
///   commands **twice separately**, so websocket round trip and task switching had already opened a few ms; the delivery path writing
///   consecutively in the same function
///   has no gap and did not submit (reconfirmed with a live agent).
/// ★The value is not arbitrary (ADR-0038)★: tmux-based multi-agent orchestrators solving the same problem use 0.5 s against Claude Code
///   for the same reason (text and Enter close together race with the input buffer). Do not change it without grounds — shrinking brings
///   this defect back.
pub const SUBMIT_PACING: std::time::Duration = std::time::Duration::from_millis(500);

pub fn input_encoder(c: &AgentCommand) -> InputEncoder {
    backend_for(c).input_encoder(c)
}

// ── Output refining (ADR-0044/0004/0045) — the symmetric pair of the input encoder ──────────────────────────

/// The output refining decoder plugged in before pump→core. None = bytes pass straight through (terminal and plain text unchanged).
///
/// ★Symmetry★: the output-direction pair of `input_encoder`. Decision and real decoder are owned by [`AgentBackend::output_decoder`]; this
///   function is only dispatch — the session knows only the encoder tag, the transport only `dyn OutputDecoder` (ADR-0004). A new backend
///   implements that method in its
/// own folder and this function is untouched.
pub fn output_decoder(c: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
    backend_for(c).output_decoder(c)
}

// ── ADR-0079: `.jsonl` transcript → past event seed on resume (backend dispatch) ──────

/// ADR-0079: the list of `OutputEvent`s restoring this command's past conversation on a resume spawn. An empty Vec = no seed.
///
/// ★Backend knowledge isolation (ADR-0004)★: the transcript's file layout, format and seed decision belong to
///   [`AgentBackend::resume_transcript_events`]; this function is only dispatch and the manager does not know path rules. The
///   resume-direction pair of `output_decoder` (live refining).
pub fn resume_transcript_events(
    c: &AgentCommand,
    cwd: &std::path::Path,
    session_id: Uuid,
) -> Vec<crate::types::OutputEvent> {
    backend_for(c).resume_transcript_events(c, cwd, session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::AgentOutputFormat;

    // ── ADR-0099 tripwire: forces a conscious capability declaration when wiring a new AgentCommand variant ──────
    //
    // ★ Never add a wildcard ★ — a new AgentCommand variant must give a compile error here and force a visit to the checklist below (a
    //   list whose update is forced by compilation does not rot).
    //
    // Checklist when wiring a new variant:
    //   ① Do not copy-paste the stub for the new backend's two capabilities — fill them with values measured in a CLI spike.
    //   ② supports_control_channel=false makes a standalone agent without messaging (check that it is intended).
    //   ③ Non-MCP (accepts_mcp_config=false): provision picks the [Cli] grant on its own (when the backend uses
    //      mail), and the spawn gets **no priming at all** — only a backend with a settings check gets real launch
    //      materials, and Noop kits carry none (ADR-0370). The CLI copy was deleted in commit `2ef6902` together
    //      with the `roundtrip-smoke --cli-only` knob — do not revive it. That branch has no measuring tool
    //      today — ADR-0209.
    //   ④ If MCP-capable, measure with the default roundtrip.
    //   ⑤ **Do not derive** the third field (writes_mcp_config_file) **from the second** — 「mail through MCP」 and 「reads the mcp-config
    //   file we wrote」 are separate; true only when that program has a flag pointing at that file (claude's `--mcp-config`). True writes a
    //   plaintext token JSON on
    //      every spawn and a write failure cuts the spawn (fail-closed) — pure loss for a backend that does not read it.
    //   Reference: ADR-0099.
    fn expected_channel_matrix(c: &AgentCommand) -> (bool, bool, bool) {
        // (supports_control_channel, accepts_mcp_config, writes_mcp_config_file) — values measured in the CLI spike
        match c {
            AgentCommand::Claude { .. } => (true, true, true),
            AgentCommand::Shell { .. } => (false, false, false),
            // ★The second field of this row **is a wrong value by its name — turned on knowingly**★: codex cannot consume the mcp-config
            //   file we make (no flag points at it — measured 0.155.0), but the daemon derives the mail channel decision (`mail_allowed`)
            //   from this one field, so false would
            //   **leave the CLI mirror open** for this MCP-mail backend. Canonical reasoning: that method's comment in `backend/codex/`.
            // ★So the third field is false — do not copy the second★: writing a file nobody reads (plaintext token JSON on every spawn ·
            //   spawn aborted by the write's failure) was removed by splitting this field; true brings it back.
            // ADR-0209
            AgentCommand::Codex { .. } => (true, true, false),
        }
    }

    #[test]
    fn backend_channel_matrix_is_consciously_declared() {
        let variants: Vec<AgentCommand> = vec![
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
        ];

        for c in &variants {
            let (expected_control, expected_mcp, expected_writes) = expected_channel_matrix(c);
            let actual_control = supports_control_channel(c);
            let actual_mcp = accepts_mcp_config(c);
            let actual_writes = writes_mcp_config_file(c);
            assert_eq!(
                actual_control,
                expected_control,
                "variant {:?}: supports_control_channel mismatch — declare the capability consciously following the expected_channel_matrix checklist at the top of backend mod.rs (ADR-0099)",
                c
            );
            assert_eq!(
                actual_mcp,
                expected_mcp,
                "variant {:?}: accepts_mcp_config mismatch — declare the capability consciously following the expected_channel_matrix checklist at the top of backend mod.rs (ADR-0099)",
                c
            );
            assert_eq!(
                actual_writes,
                expected_writes,
                "variant {:?}: writes_mcp_config_file mismatch — if this field is true, a plaintext token JSON is written to disk on every spawn and a failure of that write cuts the spawn. Declare it consciously following checklist ⑤ above (ADR-0209)",
                c
            );
        }
    }

    /// ★Tripwire — every backend with a settings check is MCP-capable and uses mail★. Real launch materials add the
    /// built-in priming whenever 「Default」 is attached, and that priming teaches the Engram MCP tools and mail — what
    /// priming teaches must be what the spawn has. A backend that gains a settings check without both must decide how
    /// its agents get (or skip) the priming first.
    // ADR-0126
    // ADR-0133
    #[test]
    fn every_backend_with_a_settings_check_has_what_the_priming_teaches() {
        let variants = [
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
        ];
        for check in settings_checks() {
            let command = variants
                .iter()
                .find(|c| crate::launch::backend_word(c).eq_ignore_ascii_case(check.backend()))
                .unwrap_or_else(|| {
                    panic!(
                        "settings check `{}` has no command variant here — add it to this list",
                        check.backend()
                    )
                });
            assert!(
                accepts_mcp_config(command) && uses_mail(command),
                "`{}` has a settings check, so its agents get the built-in priming, but it lacks MCP or mail",
                check.backend()
            );
        }
    }

    // ── ADR-0113/0004: turn-signal classifier dispatch (the mapping is owned by the backend, the vocabulary is shared) ──────────────

    #[test]
    fn turn_classifier_dispatch_maps_claude_events_and_silences_shell() {
        use crate::types::OutputEvent;
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let delta = OutputEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        };
        let done = OutputEvent::MessageDone {
            turn_id: None,
            message_id: None,
        };
        let claude_classify = turn_classifier(&json);
        assert_eq!(claude_classify(&delta), Some(TurnSignal::Progress));
        assert_eq!(
            claude_classify(&done),
            Some(TurnSignal::Ended(crate::turn::TurnEndKind::Clean))
        );
        // ★Why `Structured` interpretation is per backend★: claude carries the input-time user echo here so it counts as progress, but a
        //   backend with no mapping must be silent on the same event — else structured meta lines unrelated to turns create a permanent
        //   "in a turn" with no end signal.
        let meta = OutputEvent::Structured {
            kind: "session_meta".into(),
            json: "{}".into(),
        };
        assert_eq!(claude_classify(&meta), Some(TurnSignal::Progress));
        let shell_classify = turn_classifier(&shell);
        for ev in [&delta, &done, &meta] {
            assert_eq!(
                shell_classify(ev),
                None,
                "a backend that declared no mapping emits no signal for any event (default = silence)"
            );
        }
    }

    // ── ADR-0172/0004: resume-failure classification dispatch (text knowledge is owned by the backend) ──────────────────

    #[test]
    fn resume_failure_dispatch_reads_claudes_marker_and_stays_silent_elsewhere() {
        let claude = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let tail = "\u{1b}[?25hNo conversation found with session ID: 8b1c…\r\n";
        assert_eq!(
            resume_failure_kind(&claude, tail),
            Some(AgentFailureKind::NoConversationToResume)
        );
        assert_eq!(
            resume_failure_kind(&claude, "NO CONVERSATION FOUND with session ID: x"),
            Some(AgentFailureKind::NoConversationToResume),
            "case is set by the external program, so our decision must not hang on it"
        );
        assert_eq!(
            resume_failure_kind(&claude, ""),
            None,
            "an empty tail settles nothing — a structured session sends diagnostics to stderr, which does not come here"
        );
        assert_eq!(
            resume_failure_kind(&claude, "Error: EPERM"),
            None,
            "unknown text settles nothing (the caller falls back to the contextual default)"
        );
        assert_eq!(
            resume_failure_kind(&shell, tail),
            None,
            "a backend that declared nothing does not read someone else's text"
        );
    }

    #[test]
    fn input_encoder_dispatch_by_mode() {
        let term = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        };
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        assert_eq!(input_encoder(&term), InputEncoder::Raw);
        assert_eq!(input_encoder(&shell), InputEncoder::Raw);
        assert_eq!(input_encoder(&json), InputEncoder::ClaudeStreamJson);
    }

    #[test]
    fn raw_encoder_is_byte_identical() {
        let input = b"echo hi\r\n\x1b[A\x03";
        assert_eq!(
            InputEncoder::Raw.encode(input, Uuid::new_v4()),
            input.to_vec()
        );
    }

    // ── Mail eligibility tripwire: a new variant must declare its classification **consciously** ──────────────────
    //
    // ★Do not add a wildcard★ — the default is true (fail-open), so a recipient without any declaration, and this axis prevents **LLM free
    //   text being executed as commands**, so silent inheritance is a safety problem.
    //
    // ★Omissions from the list are caught too (slots)★: a single hand-filled Vec let a forgotten new variant mean `reads_messages` was
    //   never called and fail-open passed green. A new variant gives ① a compile error in the `variant_slot` match → ② a slot number
    //   mismatching `BACKEND_VARIANTS`
    // and the sample array length → ③ the "all slots filled" assertion below if the sample is not filled. One of the three always catches it.
    //
    // Criterion: true if it **reads and interprets** input (CLI coding agents etc.), false if it **executes** input (shell · REPL kinds).
    //   If unsure, leave it false and raise it to the user.
    const BACKEND_VARIANTS: usize = 3;

    fn variant_slot(c: &AgentCommand) -> usize {
        match c {
            AgentCommand::Claude { .. } => 0,
            AgentCommand::Shell { .. } => 1,
            AgentCommand::Codex { .. } => 2,
        }
    }

    fn expected_reads_messages(c: &AgentCommand) -> bool {
        match c {
            AgentCommand::Claude { .. } => true,
            AgentCommand::Shell { .. } => false,
            // ★Once false on 「turns cannot be observed, so busy times cannot be told apart」 — ADR-0116 decision 7 rejected that premise★
            //   (「cannot observe, so cannot deliver」 was a rejected alternative). Terminal claude already receives in the same place, and
            //   by the criterion above codex **reads and
            //   interprets** input, so true. Canonical reasoning: that method's doc in `backend/codex/`.
            AgentCommand::Codex { .. } => true,
        }
    }

    /// The sample list that every tripwire below runs over.
    ///
    /// ★Deleting one line makes **that mode drop out of those tripwires entirely while everything stays green**★ (measured by deleting the
    ///   codex app-server line) — variant slot coverage does not count modes. A separate gate measures the list itself:
    ///   [`the_sample_list_covers_every_mode_of_every_backend`].
    fn mail_eligibility_samples() -> Vec<AgentCommand> {
        let per_variant: [AgentCommand; BACKEND_VARIANTS] = [
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
        ];
        let mut all = per_variant.to_vec();
        all.push(AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        });
        all.push(AgentCommand::Codex {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        });
        all
    }

    /// The gate guarding the sample list. ★The tripwires measure only what this list reaches, so a hole is **visible nowhere**★ —
    ///   everything stays green while that mode is never asked.
    ///
    /// ★A slot is **(variant × output mode)**, not a variant★: variant slots do not count modes, so a missing second mode still looks
    ///   filled. A new variant or [`AgentOutputFormat`] value gives a compile error in the match below.
    const BACKEND_MODES: usize = 5;

    fn mode_slot(c: &AgentCommand) -> usize {
        match c {
            AgentCommand::Claude {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => 0,
            AgentCommand::Claude {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => 1,
            AgentCommand::Shell { .. } => 2,
            AgentCommand::Codex {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => 3,
            AgentCommand::Codex {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => 4,
        }
    }

    #[test]
    fn the_sample_list_covers_every_mode_of_every_backend() {
        let mut covered = [false; BACKEND_MODES];
        for c in &mail_eligibility_samples() {
            covered[mode_slot(c)] = true;
        }
        assert!(
            covered.iter().all(|c| *c),
            "there is a mode the sample list does not reach — that mode is caught by none of the tripwires below and everything stays green: {covered:?}"
        );
    }

    #[test]
    fn mail_eligibility_is_consciously_declared_for_every_backend() {
        let mut covered = [false; BACKEND_VARIANTS];
        for c in &mail_eligibility_samples() {
            covered[variant_slot(c)] = true;
            assert_eq!(
                reads_messages(c),
                expected_reads_messages(c),
                "variant {c:?}: mail eligibility mismatch — declare it consciously following the classification criterion of expected_reads_messages above"
            );
        }
        assert!(
            covered.iter().all(|c| *c),
            "there is a variant the sample does not reach (reads_messages is never called for that variant, so fail-open passes as is): {covered:?}"
        );
    }

    // ── Session two-axis tripwire: declare the issuing axis and the resume axis consciously per mode ────────────────────
    //
    // ★Do not add a wildcard★ — neither axis has a trait default, so **a new backend** must write the values, but **a new output mode** is
    //   invisible to the compiler (one impl receives every mode); the per-mode match below sets up that place.
    //
    // The two axes ask different things — do not fold them:
    //   ① assigns_session_id — whether **we** draw the id and hand it over at spawn (the manager issues and persists it). On for a program
    //   that issues its own id plants a uuid it never uses, and resume decisions stand on that fake value.
    //   ② can_resume_stored_session — whether it can resume with that stored id; **does not ask who issued it**.
    // ★Do not fold the two fields★: they drive different consumers (issuing = at spawn · resuming = the three activation entries), and
    //   rows already differ — both codex modes are (false, true): codex issues (we have no value to plant) and we resume with the value we
    //   recorded. ★A place
    // writing the terminal mode as (false, false) is stale★ — it resumes with `codex resume <id>` (ADR-0208).
    // ADR-0185
    fn expected_session_axes(c: &AgentCommand) -> (bool, bool) {
        // (assigns_session_id, can_resume_stored_session)
        match c {
            AgentCommand::Claude {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => (true, true),
            AgentCommand::Claude {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => (true, true),
            AgentCommand::Shell { .. } => (false, false),
            AgentCommand::Codex {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => (false, true),
            // ★The first row where the two fields differ★ — codex issues (①), and we send `thread/resume` with the thread id we recorded
            //   (②). Canonical reasoning: those two methods' comments in `backend/codex/`.
            AgentCommand::Codex {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => (false, true),
        }
    }

    /// Holes in the sample list are measured by [`the_sample_list_covers_every_mode_of_every_backend`] — slot coverage
    /// is not counted again here (same list, same slots, so the same answer would just come out twice).
    #[test]
    fn session_axes_are_consciously_declared_for_every_mode() {
        for c in &mail_eligibility_samples() {
            let (expected_assign, expected_resume) = expected_session_axes(c);
            assert_eq!(
                assigns_session_id(c),
                expected_assign,
                "{c:?}: issuing axis mismatch — declare it consciously following ① of expected_session_axes above"
            );
            assert_eq!(
                can_resume_stored_session(c),
                expected_resume,
                "{c:?}: resume axis mismatch — declare it consciously following ② of expected_session_axes above"
            );
        }
    }

    /// The regression net of the decision the three activation entries call together — ★having a stored sid alone does not mean it can
    ///   resume★. Both terms are measured, so an entry reverted to `sid.is_some()` breaks its own test and this one.
    #[test]
    fn a_stored_sid_alone_does_not_make_a_profile_resumable() {
        let sid = Some(Uuid::new_v4());
        let profile = |c: AgentCommand, s: Option<Uuid>| {
            let mut p =
                AgentProfile::new("t".into(), c, std::path::PathBuf::from("."), vec![], false);
            p.backend_session_id = s;
            p
        };

        // ★The only sample with the axis off is now shell★ — the codex terminal mode left this place (with ADR-0208 that mode also
        //   resumes with `codex resume <id>`). Shell has no resume concept at all, so this place does not become empty.
        let not_resumable = profile(
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            sid,
        );
        assert!(
            !can_resume_profile(&not_resumable),
            "a command whose resume axis is false does not resume even with a sid — turning it on would report a new conversation as resumed"
        );

        // ★codex resumes through **both links** — only the means differ★ (app-server `thread/resume`, terminal `codex resume <id>`);
        //   measuring both is the point — one alone would hide 「a resuming spawn reported as a new conversation」 when the other is
        //   silently turned off.
        for output_format in [AgentOutputFormat::StreamJson, AgentOutputFormat::Terminal] {
            let resumable_codex = profile(
                AgentCommand::Codex {
                    extra_args: vec![],
                    output_format: output_format.clone(),
                },
                sid,
            );
            assert!(
                can_resume_profile(&resumable_codex),
                "{output_format:?}: codex resumes with the recorded id — if this field is turned off, a resuming spawn is reported as a new conversation"
            );
        }

        let resumable = profile(
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            sid,
        );
        assert!(can_resume_profile(&resumable));

        let no_sid = profile(
            AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            None,
        );
        assert!(
            !can_resume_profile(&no_sid),
            "even if the axis is true, without a value to resume it is Fresh"
        );
    }

    // ── Codec-axis tripwire: a new variant or new output mode must declare its I/O codec consciously ──────────
    //
    // ★Do not add a wildcard★ — this axis's defaults (`Raw` · no decoder) are fail-open and compile without declaration; `Raw` on a
    //   program requiring structured stdin **cannot read the input at all** with no runtime signal. A new `AgentCommand` variant or
    // `AgentOutputFormat` value gives a compile error in the match below.
    //
    // How to fill it — with values measured in a CLI spike: ① input_encoder (how that program reads stdin; `Raw` if nothing to wrap) · ②
    //   output_decoder (whether output is structured and needs refining) · ③ transport_shape (terminal or pipe for that output).
    //
    // ★③ goes with ①② but is not the same axis★: a wrong link shape compiles and **launches plausibly** — a TUI through a pipe breaks the
    //   screen, line-based JSON through a PTY mixes in escapes. The default (`Pty`) is fail-open, so without a column here a new backend
    //   silently inherits it.
    //
    // ★Not covered = the resume transcript seed★: `resume_transcript_events` has the same fail-open default (empty Vec), but deciding
    //   needs an actual transcript file, so "no seed" and "empty because no file" cannot be told apart here; the transcript unit tests of
    //   `backend/claude/` carry that net.
    fn expected_codec_axis(c: &AgentCommand) -> (InputEncoder, bool, TransportShape) {
        // (input_encoder, whether there is an output_decoder, transport_shape)
        match c {
            AgentCommand::Claude {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => (InputEncoder::Raw, false, TransportShape::Pty),
            AgentCommand::Claude {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => (
                InputEncoder::ClaudeStreamJson,
                true,
                TransportShape::StdioNdjson,
            ),
            AgentCommand::Shell { .. } => (InputEncoder::Raw, false, TransportShape::Pty),
            AgentCommand::Codex {
                output_format: AgentOutputFormat::Terminal,
                ..
            } => (InputEncoder::Raw, false, TransportShape::Pty),
            // ★This row's point: the app-server mode's encoder is not `Raw`★ — `Raw` would make the session emit a CR **as a separate
            //   call** after the body, and the envelope-building link would read it as one more body and open an empty turn.
            AgentCommand::Codex {
                output_format: AgentOutputFormat::StreamJson,
                ..
            } => (
                InputEncoder::TransportFramed,
                true,
                TransportShape::StdioBidiJson,
            ),
        }
    }

    #[test]
    fn codec_axis_is_consciously_declared_for_every_backend() {
        let mut covered = [false; BACKEND_VARIANTS];
        for c in &mail_eligibility_samples() {
            covered[variant_slot(c)] = true;
            let (expected_encoder, expects_decoder, expected_shape) = expected_codec_axis(c);
            assert_eq!(
                input_encoder(c),
                expected_encoder,
                "variant {c:?}: input_encoder mismatch — declare it consciously following expected_codec_axis above"
            );
            assert_eq!(
                output_decoder(c).is_some(),
                expects_decoder,
                "variant {c:?}: output_decoder presence mismatch — declare it consciously following expected_codec_axis above"
            );
            assert_eq!(
                transport_shape(c),
                expected_shape,
                "variant {c:?}: transport_shape mismatch — declare it consciously following expected_codec_axis above"
            );
        }
        assert!(
            covered.iter().all(|c| *c),
            "there is a variant the sample does not reach (the codec axis is never called for that variant, so fail-open passes as is): {covered:?}"
        );
    }

    // ── The declared link shape and the link actually handed over ────────────────────────────────────────────
    //
    // ★Measured separately from the codec-axis table above (since ADR-0191)★: when link creation moved into the backend's `open_spawn`,
    //   production code reading the reported `transport_shape` went to 0, so declaring a pipe while taking the default `open_spawn` (=PTY)
    //   leaves that column green;
    //   the runtime symptom is only a broken screen and this test is the only catch.
    // ★Not split by `structured`★: it is a backend-injected value (ADR-0030/0044), so a plain pipe reports false too; what tells the two
    //   implementations apart is `terminal_bytes` and `resize`, both hardcoded.
    // ★No slot coverage assertion★: the tripwires above running the same sample list already measure it.
    // ★spec is not that backend's real CLI★: `open_spawn` takes what to launch from spec and which link from command, so a probe ending
    //   immediately launches and only the link choice is looked at (ADR-0012).
    // ADR-0191
    #[cfg(windows)]
    #[test]
    fn declared_transport_shape_matches_the_transport_handed_over() {
        let probe = CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo shape-probe".into()],
            env: vec![],
            cwd: std::path::PathBuf::from("."),
        };
        for c in &mail_eligibility_samples() {
            let shape = transport_shape(c);
            let expected = match shape {
                TransportShape::Pty => (true, true),
                TransportShape::StdioNdjson => (false, false),
                // A pipe: neither terminal bytes nor size — the pair of the one-way pipe. `interrupt` is not in this pair: on the pipe
                //   side it is a backend-injected value (ADR-0238), not a link report; both pipe shapes are true today and `interrupt`
                //   tells PTY (false · ADR-0245) from pipes.
                TransportShape::StdioBidiJson => (false, false),
            };
            let parts = open_spawn(
                c,
                &probe,
                80,
                24,
                None,
                None,
                None,
                None,
                None,
                &LaunchArtifacts::default(),
            )
            .expect("open_spawn");
            let caps = parts.transport.capabilities();
            let actual = (caps.output.terminal_bytes, caps.control.resize);
            parts.transport.shutdown();
            assert_eq!(
                actual, expected,
                "variant {c:?}: reported {shape:?} but the (terminal_bytes, resize) of the link open_spawn handed over differ — only one of the report and the link creation was fixed"
            );
        }
    }

    // ── Submit bytes (submit_sequence) — per backend, "does the body write alone fail to start a turn" ──────────────
    #[test]
    fn submit_sequence_is_cr_for_raw_and_absent_for_stream_json() {
        assert_eq!(
            InputEncoder::Raw.submit_sequence(),
            Some(b"\r".as_slice()),
            "terminal · shell imitate a PTY keyboard — Enter = CR"
        );
        assert_eq!(
            InputEncoder::ClaudeStreamJson.submit_sequence(),
            None,
            "the terminating \\n of encode is already the submit — adding a CR puts surplus bytes after the line"
        );
    }

    #[test]
    fn submit_sequence_is_not_baked_into_encode() {
        // ★Combined, the claude TUI does not submit (measured)★ — if submit bytes get mixed into the encode output,
        //   the first write is already polluted even if the session splits it into two writes.
        let out = InputEncoder::Raw.encode(b"<message from=\"bob\">hi</message>", Uuid::new_v4());
        assert!(
            !out.contains(&b'\r'),
            "Raw encode does not append submit bytes (the submit is a write boundary — the session's concern): {out:?}"
        );
    }

    // ── Turn submit decision (submits_turn) — the criterion of the session id first-submission latch (ADR-0226) ──────────────────
    /// ★The assumption is baked into the name★: this stands on xterm's default key encoding (Enter = CR); if a mode sending Enter
    ///   differently comes in, **the decision itself**, not this entry, must be revisited.
    #[test]
    fn raw_submission_is_cr_under_default_xterm_encoding() {
        assert!(
            !InputEncoder::Raw.submits_turn(b"hel"),
            "a keystroke fragment without CR is not a turn"
        );
        assert!(
            !InputEncoder::Raw.submits_turn(b""),
            "an empty fragment is not a turn"
        );
        assert!(
            !InputEncoder::Raw.submits_turn(b"\n"),
            "LF is not Enter in xterm's default key encoding"
        );
        assert!(InputEncoder::Raw.submits_turn(b"\r"), "Enter alone");
        assert!(
            InputEncoder::Raw.submits_turn(b"hello\r"),
            "Enter after the body came in the same fragment"
        );
        assert!(
            InputEncoder::Raw.submits_turn(b"a\rb"),
            "a fragment with a CR in the middle, like a paste, also counts as a submit"
        );
    }

    #[test]
    fn structured_encoders_always_submit_a_turn() {
        for encoder in [
            InputEncoder::ClaudeStreamJson,
            InputEncoder::TransportFramed,
        ] {
            assert!(
                encoder.submits_turn(b"hello"),
                "{encoder:?}: 1 call = 1 user turn — a turn even without CR"
            );
            assert!(
                encoder.submits_turn(b""),
                "{encoder:?}: the call itself is a turn regardless of content"
            );
        }
    }

    /// ★An encoder emitting submit bytes counts them as a turn★ — mail delivery counts with `submits_turn(submit bytes)` ahead of them, so
    ///   if only one side (e.g. `Raw`'s submit bytes) changes, mail is **silently** not counted and that agent's resume disappears.
    // ADR-0226
    #[test]
    fn every_submit_sequence_counts_as_a_turn_submission() {
        // If the variants grow, this match breaks compilation — a signal to add to the list below.
        let _exhaustive = |e: InputEncoder| match e {
            InputEncoder::Raw | InputEncoder::ClaudeStreamJson | InputEncoder::TransportFramed => {}
        };
        for encoder in [
            InputEncoder::Raw,
            InputEncoder::ClaudeStreamJson,
            InputEncoder::TransportFramed,
        ] {
            if let Some(submit) = encoder.submit_sequence() {
                assert!(
                    encoder.submits_turn(submit),
                    "{encoder:?}: the submit bytes {submit:?} are not counted as a turn submit — mail cannot call persistence"
                );
            }
        }
    }

    // ── ADR-0044/0045: input-time user echo event dispatch (input_echo_event) — uuid dedup ──────
    #[test]
    fn input_echo_event_json_mode_emits_structured_user_with_uuid() {
        use crate::types::OutputEvent;
        let id = Uuid::new_v4();
        let ev = InputEncoder::ClaudeStreamJson
            .input_echo_event(b"hi there", id)
            .expect("json mode → synthetic user echo event");
        match ev {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(&json).unwrap();
                assert_eq!(v["type"], "text");
                assert_eq!(v["text"], "hi there");
                assert_eq!(
                    v["uuid"],
                    id.to_string(),
                    "msg_uuid attached to the synthetic echo (dedup key)"
                );
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    #[test]
    fn input_echo_event_raw_is_none() {
        assert!(
            InputEncoder::Raw
                .input_echo_event(b"echo hi\r\n", Uuid::new_v4())
                .is_none(),
            "Raw must not make a synthetic user echo (prevents duplicating the PTY echo)"
        );
    }

    #[test]
    fn claude_stream_json_encoder_wraps_and_terminates() {
        let id = Uuid::new_v4();
        let out = InputEncoder::ClaudeStreamJson.encode(b"hi", id);
        assert_eq!(*out.last().unwrap(), b'\n');
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\"type\":\"user\""));
        assert!(s.contains("\"text\":\"hi\""));
        assert!(
            s.contains(&id.to_string()),
            "msg_uuid included in the stdin user line"
        );
    }

    // ── S15 B3: output_decoder dispatch (symmetric to the input encoder) ──────────────────────
    #[test]
    fn output_decoder_dispatch_by_mode() {
        let term = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        };
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let shell = AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        assert!(
            output_decoder(&term).is_none(),
            "terminal mode → no decoder (straight through)"
        );
        assert!(
            output_decoder(&shell).is_none(),
            "shell → no decoder (straight through)"
        );
        assert!(
            output_decoder(&json).is_some(),
            "json mode → ClaudeStreamDecoder injected"
        );
    }

    #[test]
    fn output_decoder_produces_structured_events_through_trait_object() {
        use crate::types::OutputEvent;
        let json = AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        };
        let mut dec = output_decoder(&json).expect("json → decoder");
        let mut ev = dec.decode(b"{\"type\":\"result\",\"subtype\":\"success\"}\n");
        ev.extend(dec.flush());
        assert!(
            ev.iter()
                .any(|e| matches!(e, OutputEvent::MessageDone { .. })),
            "the trait object decode must refine a result line into MessageDone: {ev:?}"
        );
    }

    // ── Usage prober registry ──────────────────────────────────────────────────────

    /// All backend words `agent.new` accepts (serialized spelling). ★Do not put a wildcard in the match★ — when the declaration vocabulary
    /// gains a word, compilation breaks here, making you decide whether to give that backend a usage prober.
    fn agent_new_backend_words() -> Vec<String> {
        use crate::commands::AgentBackend;
        let all = [AgentBackend::Claude, AgentBackend::Codex];
        for backend in &all {
            match backend {
                AgentBackend::Claude | AgentBackend::Codex => {}
            }
        }
        all.iter()
            .map(|backend| {
                serde_json::to_value(backend)
                    .expect("serialize")
                    .as_str()
                    .expect("a unit variant serializes as a word")
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn usage_probe_keys_are_the_agent_new_backend_words() {
        let words = agent_new_backend_words();
        assert_eq!(
            usage_probes().len(),
            words.len(),
            "probers and `agent.new` words must pair one to one: {words:?}"
        );
        for word in &words {
            let probe = usage_probe_for(word)
                .unwrap_or_else(|| panic!("no prober for the `agent.new` word {word}"));
            assert!(
                probe.key().as_str().eq_ignore_ascii_case(word),
                "{word} → {}",
                probe.key().as_str()
            );
        }
    }

    #[test]
    fn usage_probe_keys_are_unique() {
        let probes = usage_probes();
        for (i, a) in probes.iter().enumerate() {
            for b in &probes[i + 1..] {
                assert!(
                    !a.key().as_str().eq_ignore_ascii_case(b.key().as_str()),
                    "keys overlap: {}",
                    a.key().as_str()
                );
            }
        }
    }

    #[test]
    fn usage_probe_for_ignores_ascii_case_and_refuses_unknown_words() {
        for spellings in [["claude", "Claude", "CLAUDE"], ["codex", "Codex", "CODEX"]] {
            let canonical = usage_probe_for(spellings[0])
                .unwrap_or_else(|| panic!("{}", spellings[0]))
                .key();
            for word in spellings {
                let probe = usage_probe_for(word).unwrap_or_else(|| panic!("{word}"));
                assert_eq!(probe.key(), canonical, "{word}");
            }
        }
        for unknown in ["", "codx", "gemini", "shell", " claude", "claude "] {
            assert!(
                usage_probe_for(unknown).is_none(),
                "unknown word {unknown:?}"
            );
        }
    }

    #[test]
    fn usage_probe_policies_are_the_vendor_values() {
        use crate::usage::UsagePolicy;
        use std::time::Duration;

        let fifteen_minutes = Duration::from_secs(15 * 60);
        for (word, timeout) in [
            ("claude", Duration::from_secs(15)),
            ("codex", Duration::from_secs(30)),
        ] {
            let probe = usage_probe_for(word).unwrap_or_else(|| panic!("{word}"));
            assert_eq!(
                probe.policy(),
                UsagePolicy {
                    cooldown: fifteen_minutes,
                    timeout,
                },
                "{word}"
            );
        }
    }

    // ── settings-check registry (ADR-0334) ──

    #[test]
    fn each_settings_check_key_is_the_backend_word_of_its_backend() {
        let claude_word = crate::launch::backend_word(&AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        });
        let codex_word = crate::launch::backend_word(&AgentCommand::Codex {
            extra_args: vec![],
            output_format: AgentOutputFormat::Terminal,
        });
        let keys: Vec<&str> = settings_checks().iter().map(|c| c.backend()).collect();
        assert_eq!(keys, [claude_word.as_str(), codex_word.as_str()]);
        assert!(std::ptr::addr_eq(
            settings_checks()[0],
            &claude::CLAUDE_SETTINGS_CHECK as &dyn SettingsCheck,
        ));
        assert!(std::ptr::addr_eq(
            settings_checks()[1],
            &codex::CODEX_SETTINGS_CHECK as &dyn SettingsCheck,
        ));
    }

    #[test]
    fn settings_check_keys_are_unique() {
        let checks = settings_checks();
        for (i, a) in checks.iter().enumerate() {
            for b in &checks[i + 1..] {
                assert!(
                    !a.backend().eq_ignore_ascii_case(b.backend()),
                    "{}",
                    a.backend()
                );
            }
        }
    }

    #[test]
    fn settings_check_for_ignores_ascii_case_and_refuses_unknown_words() {
        for (word, backend) in [
            ("claude", "claude"),
            ("Claude", "claude"),
            ("CLAUDE", "claude"),
            ("codex", "codex"),
            ("Codex", "codex"),
            ("CODEX", "codex"),
        ] {
            let check = settings_check_for(word).unwrap_or_else(|| panic!("{word}"));
            assert_eq!(check.backend(), backend, "{word}");
        }
        for unknown in [
            "",
            "shell",
            "gemini",
            " claude",
            "claude ",
            "claud",
            "codex-cli",
        ] {
            assert!(settings_check_for(unknown).is_none(), "{unknown:?}");
        }
    }

    // ── launch-material placement (ADR-0334) ──

    /// A fake that records every spot a write through the placement port tried — measures 「no writes」.
    #[derive(Default)]
    struct RecordingFs {
        writes: std::sync::Mutex<Vec<PathBuf>>,
    }

    impl LaunchFs for RecordingFs {
        fn write_file(&self, path: &Path, _bytes: &[u8]) -> std::io::Result<()> {
            self.writes.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }

        fn link_dir(&self, link: &Path, _target: &Path) -> std::io::Result<()> {
            self.writes.lock().unwrap().push(link.to_path_buf());
            Ok(())
        }

        fn exists(&self, _path: &Path) -> bool {
            false
        }

        fn is_link(&self, _path: &Path) -> std::io::Result<bool> {
            Err(std::io::ErrorKind::NotFound.into())
        }
    }

    fn noop_kit_for(command: &AgentCommand, cwd: &Path) -> LaunchKit {
        use crate::launch::{LaunchKitSource, NoopLaunchKits};

        let profile = AgentProfile::new(
            "materialize".to_string(),
            command.clone(),
            cwd.to_path_buf(),
            Vec::new(),
            false,
        );
        let plan = NoopLaunchKits.check(&profile).expect("Noop never blocks");
        NoopLaunchKits
            .prepare(plan, profile.id, 1)
            .expect("Noop prepare")
    }

    /// Noop materials are placed as nothing to carry · no writes on every backend — the premise that the argv the golden
    /// (`tests/launch_argv_golden.rs`) measures with the default `artifacts` is the argv of a production Noop spawn.
    #[test]
    fn a_noop_kit_materializes_to_nothing_on_every_backend() {
        let cwd = std::env::temp_dir();
        for command in mail_eligibility_samples() {
            let fs = RecordingFs::default();
            assert_eq!(
                materialize(&command, &noop_kit_for(&command, &cwd), &cwd, &fs),
                Ok(LaunchArtifacts::default()),
                "{command:?}"
            );
            assert_eq!(
                *fs.writes.lock().unwrap(),
                Vec::<PathBuf>::new(),
                "{command:?}"
            );
        }

        // gemini is not wired into dispatch (its file header), so its implementation is called directly — the default implementation does not read the command.
        let stand_in = mail_eligibility_samples().remove(0);
        let fs = RecordingFs::default();
        assert_eq!(
            GeminiBackend.materialize(&stand_in, &noop_kit_for(&stand_in, &cwd), &cwd, &fs),
            Ok(LaunchArtifacts::default())
        );
        assert_eq!(*fs.writes.lock().unwrap(), Vec::<PathBuf>::new());
    }

    /// An empty real kit places nothing and writes nothing on every backend, but `real` is carried as planned — the
    /// default `materialize` and the claude and codex overrides (which have nothing to place from an empty kit) alike.
    #[test]
    fn the_default_materialize_carries_real_and_nothing_else() {
        // A fixed cmd-safe absolute path: the codex terminal override guards the working folder, and the temp folder
        //   may hold `&` or `%`. An empty kit touches no disk.
        let cwd = std::path::absolute("/engram-fake/default-materialize").unwrap();
        let real_kit = |command: &AgentCommand| {
            let mut kit = noop_kit_for(command, &cwd);
            kit.plan.real = true;
            kit
        };
        let carried = LaunchArtifacts {
            real: true,
            ..LaunchArtifacts::default()
        };
        for command in mail_eligibility_samples() {
            let fs = RecordingFs::default();
            assert_eq!(
                materialize(&command, &real_kit(&command), &cwd, &fs),
                Ok(carried.clone()),
                "{command:?}"
            );
            assert_eq!(
                *fs.writes.lock().unwrap(),
                Vec::<PathBuf>::new(),
                "{command:?}"
            );
        }

        let stand_in = mail_eligibility_samples().remove(0);
        let fs = RecordingFs::default();
        assert_eq!(
            GeminiBackend.materialize(&stand_in, &real_kit(&stand_in), &cwd, &fs),
            Ok(carried)
        );
        assert_eq!(*fs.writes.lock().unwrap(), Vec::<PathBuf>::new());
    }
}
