//! AgentBackend — a trait producing per-backend command specs + free-function dispatch.
//!
//! The transport (PtyTransport) does not know claude/codex. The only place that knows which program is which
//! is the `backend/<name>/` folder.
//!
//! ★This file does not call any backend's items by name★: the places where backend names are written are only the **registry**
//! (`pub mod`·`pub use`·static singletons · the usage prober table [`usage_probes`]) and **the two dispatch tables** (`backend_for` ·
//! `backend_for_encoder`), and all per-backend knowledge comes out only through [`AgentBackend`] methods and [`UsageProbe`] implementations.
//! The gate is in the `backend/claude/mod.rs` header.
//!
//! tauri import 0.

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod shell;

pub use claude::ClaudeBackend;
pub use codex::CodexBackend;
pub use gemini::GeminiBackend;
pub use shell::ShellBackend;

use std::path::PathBuf;
use std::sync::Arc;

use uuid::Uuid;

use crate::failure::AgentFailureKind;
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
/// sibling.
///
/// ★Why it lives here = these three carry no backend knowledge at all (ADR-0004)★: what is planted is only three env values and PATH,
///   and none of them is a flag or file convention of a specific program. Conversely, a **translation** like `--mcp-config` is that
///   program's grammar, so it stays in that backend's folder — raising that to here too would give the shared place one backend's
///   vocabulary.
/// ★The doc of [`crate::types::ControlEndpoint::config_path`] already writes this division as a contract★ —
///   that Option means only **whether there is an MCP entry**, not whether there is CLI wiring. This function being shared is the
///   embodiment of that sentence.
///
/// ★Call condition = every spawn with a control endpoint★: the control verbs are open to everyone (ADR-0132 decision 5) and
///   there is only one executable, so it cannot be laid down split by family. Mail is not filtered here either — enforcement is only in one place, the daemon's
///   credential refusal (ADR-0133 decision 3).
/// ★Do not send mail availability in env★: that value attaches to the agent's own process and can be erased, so
///   it enforces nothing while creating an entry for the outside to choose the CLI screen (user decision 2026-09-23).
/// ★Why env★: when the agent calls that command through the shell, it reads this value and POSTs to the daemon control route with a Bearer
///   token. The portable-pty CommandBuilder seeds the parent env, so it is **inherited by every child process
///   (Bash · its grandchildren)**.
/// ★So these three values are credentials not only of the agent itself but also of **auxiliary processes that program launches on its
///   behalf**★ — unless that program overrides the child env. Which programs actually do so is
///   written down by measurement in that backend's folder (not counted by name here — ADR-0004).
/// ★Security★: the token is exposed through env — inherited only by children of the same OS user and not logged, but hard isolation is
///   impossible in the first place (ADR-0086 §Invariants). ★Inheritance reaching every child means **sub-agents the agent launched
///   also hold the same token**★ — if reports coming with that token are trusted as one identity, the child's value
///   sits in the parent's place. ★**So do not open a new report entry that establishes identity from this token alone**★ — the entry
///   that once existed (the session id hook report) was removed by ADR-0216, and what this credential opens today is only the control verbs and mail
///   **the agent itself types**.
// ADR-0086 / ADR-0133 / ADR-0004 / ADR-0208 / ADR-0216
pub(crate) fn inject_cli_entrance(env: &mut Vec<(String, String)>, endpoint: &ControlEndpoint) {
    // ★ENGRAM_CONTROL_URL = base (scheme+host+port)★: endpoint.url is the MCP route
    //   (`http://127.0.0.1:<port>/mcp`), so to use it as the base the CLI attaches to, the route suffix (`/mcp`) is stripped, leaving only the base
    //   — the CLI assembles `<base>/control/send` (route path knowledge is owned by the CLI). Without the suffix
    //   (a shape variant) the url is used as the base as is (defensive).
    //   ★keep-in-sync (M5)★: the literal "/mcp" of strip_suffix below is a value **matched by hand** to the daemon-side MCP_PATH constant
    //   — canonical = `crates/engram-dashboard-daemon/src/control/mcp_server.rs` (const MCP_PATH). If that
    //   path changes, this literal must be fixed together (the build cannot enforce it → if they diverge, the base derivation goes wrong and the CLI
    //   silently gets 404). Mutual anchor of the two places.
    let base = endpoint
        .url
        .strip_suffix("/mcp")
        .unwrap_or(&endpoint.url)
        .to_string();
    env.push((TOKEN_ENV.to_string(), endpoint.token.clone()));
    env.push(("ENGRAM_CONTROL_URL".to_string(), base));
    // ★ENGRAM_CLI_EXE = absolute path of the CLI binary (F1)★: priming and grants teach the bare executable name
    //   (`CLI_EXE_NAME` — resolved by the PATH injection below), but this absolute-path env is carried too —
    //   for diagnostics and manual operation (the name alignment of ADR-0094 itself is achieved through PATH).
    //   ★No priming teaches this value — so do not count it as the recovery means of the loud skip branch below (PATH join failure ·
    //     non-UTF8)★: in that branch the agent has learned only the bare name and cannot resolve it through PATH, so it
    //     effectively cannot send, and the only signal is that warn log.
    //   The None branch is a combination where no send entry remains, so the daemon already cuts it off fail-closed at provision
    //   — a defensive path that is not reached here (even if reached, it is a harmless state with credentials but no CLI to call).
    if let Some(send_exe) = &endpoint.send_exe {
        env.push((
            CLI_EXE_ENV.to_string(),
            send_exe.to_string_lossy().into_owned(),
        ));
        // ★PATH injection (ADR-0094 bare name resolution)★: grants (`Bash(<CLI_EXE_NAME>:*)`) and priming both teach the bare
        //   executable name, so the spawned agent's shell (and its child Bash tool) must actually be able to
        //   **find** that name. The **parent directory** of send_exe is put at the **very front** of PATH.
        //
        // ★base = the PATH already in the env vector (profile first, FIX-1)★: the profile env came into the vector
        //   **before** this point. Rebuilding from the daemon process PATH (std::env::var_os) would make the custom PATH the profile carried
        //   evaporate entirely, so it falls back to the daemon PATH only when the vector has no PATH.
        //   ★Key case (Windows)★: whichever notation the profile uses, "Path" or "PATH", it is the same variable.
        //   ★last-match-wins + dedupe (load-bearing)★: the transport (portable-pty) applies env **in order** with
        //   cmd.env(k,v), so if the same variable has duplicate entries the **last** value lives in the child (e.g. on Windows
        //   `[("PATH", daemon), ("Path", profile)]`). So the last case-equivalent PATH is taken as both the base and the
        //   winning entry, replaced in place with that key notation as is, and **all other PATH entries are removed** —
        //   leaving duplicates would mean fixing only the front one while the unfixed later entry wins by last-wins and the injection is **silently neutralized**
        //   (adversarial review must-fix). Composition: `send_exe_parent + separator + base` — the sibling directory is
        //   **at the very front** (shadowing defense), and the profile/daemon PATH **survives as the tail**.
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
                // ★loud skip (FIX-2/3)★: on a join failure or non-UTF8 the injection is **skipped entirely** — a lossily converted
                //   PATH is never pushed (silently corrupting non-Unicode PATH entries is worse than
                //   skipping). On skip the env vector is left **exactly as it was**, so the inherited PATH becomes the safe fallback.
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
/// It is called exactly once after the id is actually received, and never if it is not received.
/// The argument is the string the other side gave, as is — ★it does not decide whether it is a uuid★. That interpretation is the receiver's job,
/// so this port holds even when a value that cannot be read as a uuid comes.
///
/// ★It has this shape so as not to bring `ProfileRegistry` into `backend/` (ADR-0004)★ — if that type were visible here,
/// the link would know the profile schema. The assembly point wraps its own recording means in this one verb and hands it over.
/// ★Not having a backend in the name is also part of the contract★ — the moment it is called "for codex", the same isolation leaks.
/// ★Being one verb is also part of the contract★ — no second verb asking back "was it recorded". So that there is nothing to ask back, the call
/// **order** stands in its place (see the contract of [`AgentBackend::open_spawn`]).
///
/// There is no return, so **a failure does not go back to the caller** — the recording side swallows it into a log inside itself.
/// So putting 「it was persisted」 on top of the fact that this port returned would cite a guarantee that does not exist.
// ADR-0004
// ADR-0185
pub type SessionIdSink = Arc<dyn Fn(&str) + Send + Sync>;

/// A **one-verb** port telling the assembly point 「a turn actually happened in this incarnation's conversation」 — the assembly point plugs the submit input of
/// the session id first-submission latch in here.
///
/// ★The only caller is the link of a backend whose link owns the turn (`MidTurnPolicy::TransportOwned`)★ — the session of that mode
///   does not count submissions, and the link calls this on the other side's **echo of the first user message**. Other backends have the session
///   count before sending, so they ignore this port. ★The two are a pair★: reporting `TransportOwned` without plugging this port into the
///   link means no incarnation can persist its id (every resume becomes a new conversation without an error).
/// It may be called several times — the receiver (the latch) counts only the first.
// ADR-0226
// ADR-0231
pub type FirstTurnSink = Arc<dyn Fn() + Send + Sync>;

/// Implemented as a unit struct and used as &'static — stateless.
pub trait AgentBackend: Send + Sync {
    /// Whether **we** draw the session id and hand it to this program at spawn.
    ///
    /// If true, the manager draws a uuid on every Fresh and passes it as the `session_id` of [`AgentBackend::build_spec`],
    /// and that value is persisted to the profile **at the first submission** (ADR-0226 — not written at spawn). The sid drift observer also takes that value
    /// as its reference, so it hangs on this axis.
    ///
    /// ★Do not read false as 「there is no session」★: the program may be one that **issues its own** id.
    ///   Planting our uuid then would leave in the profile a value the program never uses, and the resume
    ///   decision would stand on that fake value. Whether it can resume is answered by a separate axis
    ///   ([`AgentBackend::can_resume_stored_session`]).
    // ADR-0185
    fn assigns_session_id(&self, command: &AgentCommand) -> bool;

    /// **Can this command be resumed with the backend sid stored in the profile**.
    ///
    /// Boot restore and the activation entry decide 「launch as Resume or as Fresh」 from this value together with whether a sid exists.
    /// It **does not ask who issued** that sid (ADR-0185) — it only asks whether the stored value is used for resuming
    /// at all.
    ///
    /// ★Why it receives `command`★: even for the same program, **it differs by the shape it is launched in** — one
    ///   shape may have an identifier to resume and another may not. As an argument-less predicate, the value of one of that program's
    ///   two shapes would apply as is to the other.
    // ADR-0185
    fn can_resume_stored_session(&self, command: &AgentCommand) -> bool;

    /// Whether this backend **consumes** the daemon control channel (MCP entry) (ADR-0086 F3).
    /// If true, the manager calls provision before spawn (issuing the token + mcp-config) and passes that endpoint
    /// to build_spec (claude=`--mcp-config`). If false, the manager **does not touch provision
    /// at all** — a backend that does not use the control channel, like shell, does not touch the registry, so the regression (round-2 F3) where a config-write failure
    /// aborts a spawn that never needed MCP does not happen.
    ///
    /// ★fail-closed applies only to backends that **call** provision★: for a true backend, if provision is Err
    ///   the spawn is aborted (no agent secretly running without a control channel). A false backend is unrelated to that contract.
    fn supports_control_channel(&self) -> bool;

    /// Whether this backend (program) **attaches the daemon control channel through MCP** (ADR-0099 → ADR-0209). Today claude =
    /// true (attaches the mcp-config file with `--mcp-config`) · codex = true (attaches by command-line override) ·
    /// shell · gemini stub = false. ★The name says 「does it accept a config」, but today's meaning is closer to 「does it use mail through MCP」
    /// — that mismatch is kept knowingly (the combination table below), and the field asking 「does it consume a file」 is separate★.
    /// ★backend knowledge (ADR-0004)★: "which program consumes MCP how" is backend-kind knowledge, so it is
    /// declared here — the manager does not branch directly with `matches!`.
    ///
    /// This flag drives provision's grants, **whether priming is loaded** and **mail availability** (consistency invariant =
    /// the mail channel the priming teaches **=** the mail channel that spawn can use. Teaching a channel it cannot use
    /// brings the send freeze back, and not teaching one it can use leaves an uncontrolled bypass surface). If true,
    /// `DaemonControlChannel::provision` carries the MCP bits on the endpoint and picks the MCP-only education priming
    /// (`eg_send` only — ADR-0126 decision 1) and no CLI mail; if false, **no priming injected** + CLI mail
    /// possible (ADR-0133). The control CLI wiring goes to everyone regardless of this axis.
    ///
    /// ★**This field no longer decides the mcp-config file write — do not revert this sentence**★
    ///   (ADR-0209 · the `writes_mcp_config_file` gate of `control/mod.rs`): 「does it read the file we wrote」
    ///   was split off into [`AgentBackend::writes_mcp_config_file`] below, and that write and the fail-closed `?`
    ///   hang only on that field. While 「true means write the mcp-config」 was written here, codex, just by turning this field on
    ///   to match the mail axis, got **a plaintext Bearer token JSON nobody opens** on every spawn.
    ///
    /// ★So **all four combinations of the two fields are legitimate** — especially `(true, false)`★:
    ///   - `(true, true)` = claude. Uses mail through MCP, and that entry is a file we wrote (`--mcp-config`).
    ///   - **`(true, false)` = codex. Uses mail through MCP but never opens that file** — the attachment means
    ///     is a command-line override (`-c mcp_servers.<server>=…`) and the token goes into **an env that value names**.
    ///     There is no place at all for a copy to appear on disk, and that is the worth of this combination.
    ///   - `(false, false)` = shell · gemini stub. Outside the mail plane or unmeasured.
    ///   - `(false, true)` is used by no backend today (the file is for the MCP entry, so without its pair it is waste).
    /// ★A backend that turns on only the first field and **does not look at** the second silently gets no MCP★ — the second field's default is
    ///   `false`, so if a backend that **must read** the file does not declare it, the daemon does not write the file,
    ///   the endpoint's `config_path` comes as `None`, and that backend's `--mcp-config` injection does not run a single
    ///   line. The symptom is not an error but the silence of 「no tools」. When bringing in a new backend, **answer both fields
    ///   together** — the first is 「does it use mail through MCP」, the second 「is that entry a **file** we wrote」.
    ///
    /// ★What the false side picks is not 「other priming」 but 「no priming」 — do not bring the variant axis back★:
    ///   the CLI-only copy (`prompts/agent-priming-cli.md`) was deleted in commit `2ef6902`, and the decision formula is the one
    ///   `wants_priming = uses_mail && accepts_mcp_config` (`control/mod.rs`). So a non-MCP spawn
    ///   receives not a single character of instructions, and the consistency invariant above holds by **teaching nothing** (giving a document
    ///   describing nonexistent tools would be a lie, and silence is not — ADR-0209 decision 3).
    ///
    /// ★Relation to `supports_control_channel`★: the latter is "does it **call** provision" (does it consume the control channel itself),
    ///   this one is "among the channels provision attaches, **can it plug in MCP**" — orthogonal axes. Today claude and codex
    ///   are both true (only codex's MCP attachment means is not a file — the combination table above), and shell · gemini stub are
    ///   both false. "A backend that uses the control channel only through the CLI" would be the former true, the latter false.
    ///   ★If you meet a place written as 「codex is false on both」, it is stale★ — that statement predates ADR-0209.
    // ADR-0126
    // ADR-0133
    // ADR-0209
    fn accepts_mcp_config(&self) -> bool;

    /// Whether this backend **reads the mcp-config file the daemon wrote, by path** (claude=true — `--mcp-config <path>`).
    ///
    /// ★An axis split off from the field above — do not fold it back★: the field above today effectively means 「does it use mail through MCP」
    ///   (the daemon derives `mail_allowed` from it — ADR-0133), and this field asks only 「do we write a plaintext Bearer token JSON to disk」.
    ///   While it doubled, codex, just by turning the field above on to match the mail axis,
    ///   got **a plaintext token file nobody reads** on every spawn (default ACL · deleted on revoke), and that
    ///   write is fail-closed, so **a spawn could be cut off because of a file nobody reads**.
    /// ★Default = false (the side that writes no secret)★: the direction is opposite to the fail-open of the two mail axes below, because
    ///   they measure different things — the cost of a wrong false is **a missing file** (that backend's MCP does not attach and the contract table breaks on that
    ///   row — loud), and the cost of a wrong true is **a plaintext token left on disk + the risk of aborting the
    ///   spawn** (quiet). Secrets are not written for unknown backends.
    /// ★The fail-closed of a backend that declared true stays as is★ — without this file that backend's MCP entry
    ///   physically disappears, so continuing the spawn on a write failure would make an agent running without a control channel.
    // ADR-0086
    // ADR-0099
    // ADR-0209
    fn writes_mcp_config_file(&self) -> bool {
        false
    }

    /// cwd and env are passed as values the manager normalized.
    ///
    /// `control` (ADR-0086): the control channel endpoint the daemon issued (an abstract descriptor). If present, the backend
    ///   injects it into the command line its program's way (claude=`--mcp-config <path>` — that knowledge is
    ///   `backend/claude/` alone, ADR-0004). If None, or for a backend that does not use the control channel (shell), it is ignored.
    ///
    /// `resume_session_id` = the stored backend sid this spawn **will resume**. `None` = does not resume
    ///   (launched Fresh, no stored value, or this backend's resume axis is off).
    /// ★It is **a different value from `session_id` right above — do not fold them into one field**★: that one is a value we issue and hand over
    ///   (the [`AgentBackend::assigns_session_id`] axis), and this field is a value the other side issued and we recorded
    ///   (what [`SessionIdSink`] recorded). Folding them would make 「we do not issue ids」,
    ///   which `assigns_session_id() == false` means, false — since a value we passed would be carried in the argv of a backend
    ///   whose issuing axis is off.
    /// ★It is **the same value** as the field of the same name in [`AgentBackend::open_spawn`]★ — what differs is only **what is done** with that value:
    ///   a backend that resumes by command line (codex terminal) assembles argv here, and
    ///   a backend whose link requests the resume (codex app-server) picks the first request there.
    /// ★Looking at the mode together is legitimate here — the difference from `open_spawn`★: that one does not receive the mode, so
    ///   「Fresh but with a value to resume」 cannot be expressed, but this place already receives `mode`, and
    ///   **the argv of Fresh and Resume differ in the first place** (claude's `--session-id` ↔ `--resume`). The assembly point
    ///   passes this field empty on Fresh, by the same discipline as there.
    // ADR-0185
    // ADR-0208
    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        resume_session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        control: Option<ControlEndpoint>,
    ) -> CommandSpec;

    /// Whether this spawn **can be set up** with the control endpoint the daemon issued. `Err(reason)` = fail-closed (spawn aborted).
    ///
    /// ★Why it cannot be done inside [`AgentBackend::build_spec`]★: that method only returns a `CommandSpec`, so
    ///   it has no field to tell the caller 「could not build the arguments」 — the spawn would launch as is with silently missing arguments.
    ///   So the assembly point asks this predicate **before** assembly and cuts the spawn on `Err` (the issued token is reclaimed by the assembly point's
    ///   provision guard).
    /// ★Default = `Ok(())`★ — most backends have no state in which they cannot be set up with the control endpoint. A backend like claude,
    ///   where the daemon-side `?` (mcp-config write failure) already cuts it off, has nothing to do here either. Only backends with
    ///   **failures the daemon cannot see** need this axis — codex's MCP attachment value is one
    ///   (that backend writes no file, so the daemon has no material to cut it off with).
    /// ★Do not cut 「not authorized」 here★ — a spawn the daemon gave no mail entry is **normal** (the operator
    ///   turned it off or that backend is outside the mail plane), and cutting it would kill spawns of that combination entirely. What this predicate
    ///   measures is only 「must attach but cannot」.
    // ADR-0004
    // ADR-0209
    fn precheck_control_endpoint(
        &self,
        _command: &AgentCommand,
        _control: Option<&ControlEndpoint>,
    ) -> Result<(), String> {
        Ok(())
    }

    /// The caps this backend (program) decides — session (resume) · model.
    /// Separately from the input/output/control the transport (physical channel) makes, the final Capabilities are
    /// composed by `Capabilities::compose(transport_caps, backend_caps)`.
    ///
    /// Why it receives `command` (FIX 5): even for the same program (claude), **caps differ by mode** —
    /// json (stream-json) mode does not support resume (ADR-0044 follow-up), so it must report resume=false.
    /// The backend is the source of the session caps (ADR-0030) and the mode is in command, so here it looks at command
    /// and computes honestly (type split kept — output/control are still the transport's concern).
    fn capabilities(&self, command: &AgentCommand) -> BackendCaps;

    /// The **physical link shape** this backend requires for `command`.
    ///
    /// ★Why the backend (ADR-0004)★: "through which link must this program be launched" is per-program knowledge.
    /// ★Do not bring a decision predicate back to the shared layer★: the moment the assembly point asks the `AgentCommand` payload directly, one
    ///   backend's output-format axis drives everyone's link choice, and even entries that do not use that backend pass through that
    ///   axis.
    /// ★It is not the same as the render mode axis★: what that program draws (terminal TUI ↔ structured stream) is
    ///   an axis of that backend's command payload, and this is what that choice requires of **our link**.
    ///   Even if the two are tied 1:1 inside one backend, do not merge the vocabularies — merging would make a new backend report its own render
    ///   mode under our link's name.
    /// ★Default = `Pty`★: a backend that declares nothing launches as a terminal.
    // ADR-0004
    // ADR-0044
    fn transport_shape(&self, _command: &AgentCommand) -> TransportShape {
        TransportShape::Pty
    }

    /// This backend **builds the physical link itself** to launch `command`, and hands it out at once together with the other values the session assembly
    /// needs.
    ///
    /// ★Why the backend (ADR-0191)★: "with which link implementation is this program launched" is per-program knowledge.
    ///   If the assembly point matched [`TransportShape`] again to pick a constructor, **there would be two deciding places**
    ///   (`backend_for` + that match), and that place would know backend-only link types by name (ADR-0004).
    /// ★The assembly point does not know the actual type of the returned link★ — it receives only `Box<dyn AgentTransport>`.
    /// ★Aspects are gathered and carried in one dispatch★: caps, encoder, turn classifier and mail eligibility are each owned
    ///   by their own method as is, and here they are **only gathered**. The reason for this bundle is to keep the assembly point from riding the same switch again
    ///   for each aspect.
    /// ★The link implementation hardcoding `output.structured` is still forbidden (ADR-0044/0030)★ — the backend that chose the structured
    ///   pipe injects it at that place. The rule is unchanged; only the **place** of injection is here.
    /// `cols`/`rows` are used only for terminal links — a pipe has no size concept, so they are ignored.
    ///
    /// `sid_sink` = where a backend whose other side **issues** the session id sends that value back to the assembly point
    /// ([`SessionIdSink`]). `None` = the assembly point gave no recording means → even if received there is nowhere to keep it.
    /// ★A backend that does not receive that id itself just ignores this field★ — a side that observes by watching its own file, like claude,
    ///   records through that observer path, not here.
    /// ★Order contract★: this port is called **before anything is sent through that session**. So there is no need for a second verb
    ///   asking back whether it was recorded (why the port is one verb — [`SessionIdSink`]).
    ///
    /// `first_turn_sink` = where to announce the moment a turn actually happened in this incarnation's conversation ([`FirstTurnSink`]). ★Only a backend that reports
    ///   a mode where the link owns the turn (`MidTurnPolicy::TransportOwned`) plugs it into that link — the rest
    ///   ignore it★ (the session of that mode does not count submissions — the pairing discipline is in that type's doc).
    ///
    /// `resume_session_id` = the stored backend sid this spawn **will resume**. `None` = does not resume
    ///   (launched Fresh, no stored value, or this backend's resume axis is off).
    /// ★It is **a different value from the `session_id` of [`AgentBackend::build_spec`] — do not fold them into the same thing**★:
    ///   that one is a value we issue and hand over (the [`AgentBackend::assigns_session_id`] axis), and this field is a value the other side
    ///   issued and we recorded (what [`SessionIdSink`] recorded).
    /// ★Not receiving the mode together is intended★ — the assembly point passes this field empty on Fresh. Receiving both the mode and the value
    ///   would make the combination 「Fresh but with a value to resume」 expressible, and the interpretation of that combination
    ///   would differ per backend.
    /// ★A backend that resumes by command line (claude) does not use this field★ — there `build_spec`
    ///   assembles `--resume <sid>`. This field is needed by a backend **whose link requests the resume**.
    /// ★Default = PTY + the values each aspect reports★: a backend that does not build its own link launches as a terminal
    ///   (the same place as the [`AgentBackend::transport_shape`] default), and the other fields carry the output of their own methods as
    ///   is.
    /// ★But this default **does not read** `transport_shape`★: reporting that it requires a pipe while not implementing
    ///   this method makes it silently launch as a terminal. The declaration-table tripwire (`tests::expected_codec_axis`)
    ///   measures only the reported value of `transport_shape`, so it cannot see that mismatch.
    ///
    /// `control` = **the same endpoint** [`AgentBackend::build_spec`] received. It comes for backends that must carry control-plane data **through the link
    ///   handshake** rather than the command line (codex app-server puts the priming content in
    ///   `developerInstructions` of `thread/start`).
    /// ★Do not write it again here just because it was already written there★ — if one spawn sends the same value by two means, the other side
    ///   hangs on how the other side merges the two. There must be one assembling place per mode
    ///   (the same discipline as `resume_session_id` being split between argv and the handshake).
    /// ★Most backends ignore this field★ — a backend that finishes by translating into the command line has
    ///   already done everything in `build_spec`.
    // ADR-0004
    // ADR-0191
    // ADR-0215
    /// Whether this backend's link must set up a connection (the doc of the public wrapper [`declares_link`] is canonical).
    /// ★Default = `false`★ — a backend that declares nothing gets no delivery port and takes the old decision path as is.
    // ADR-0004
    fn declares_link(&self, _command: &AgentCommand) -> bool {
        false
    }

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
    ) -> Result<SpawnParts, PtyError> {
        // This default neither fetches the session id, nor has the link own the turn, nor resumes through the link, nor puts control-plane data
        //   in the handshake — they are dropped explicitly here instead of using underscore names (the names must match what the doc above
        //   calls them: rustdoc renders the signature as is).
        let _ = (sid_sink, first_turn_sink, resume_session_id, control);
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
    /// ★Why the backend (ADR-0004 · inheriting the intent of ADR-0110 decision 4)★: "which output events are turn progress and
    ///   which are turn ends" is per-program knowledge. In particular `OutputEvent::Structured` is **a per-backend
    ///   event escape hatch**, so the meaning of its `kind` differs per backend — interpreting it in the shared layer would force one backend's
    ///   convention on everyone (a backend emitting structured meta lines would be "in a turn" forever with no end signal).
    /// ★There is only one shared signal vocabulary (`TurnSignal`)★: what differs per backend is the **mapping**, not the vocabulary.
    ///   Making a per-backend signal enum would require consumers to know the backend.
    /// ★Default = no signal (fail-open)★: a backend that declares no mapping is not observed. Unobserved is
    ///   absorbed on the consumer side as "deliver immediately" (positive-knowledge-only), but a progress signal without grounds
    ///   creates an "in a turn" that cannot be woken — so the default is silence.
    // ADR-0004
    // ADR-0113
    fn turn_classifier(&self) -> TurnClassifier {
        no_turn_signals
    }

    /// When resuming failed, **can the cause be recognized from that program's output** (ADR-0172 classification entry).
    ///
    /// `evidence` = the text that incarnation left (best-effort — may be empty). The caller passes **the two streams
    /// merged**: the console tail (PTY session) + the diagnostic stderr tail (pipe session). Depending on the transport the two fill
    /// exclusively, so the implementation does not need to know which one it came from — it only looks at the text.
    /// `None` = the kind cannot be settled from this text alone → the caller falls back to the contextual default.
    ///
    /// ★Why the backend (ADR-0004)★: "what this text means" is per-program knowledge. If the manager looked at the string
    ///   directly, that knowledge would leak into the shared layer.
    /// ★Default = unknown (fail-open)★: a backend that declares nothing settles nothing.
    /// ★It is also called for live sessions★: the caller can settle from the diagnostic stream alone without waiting for exit
    ///   (`EarlyVerdict::Diagnosed`). So declare **only "text that program can emit only when it failed"**
    ///   — declaring text that could appear mixed into the conversation body would stamp a successful activation as a failure.
    // ADR-0004
    // ADR-0172
    fn resume_failure_kind(&self, _evidence: &str) -> Option<AgentFailureKind> {
        None
    }

    /// Whether this backend is **a reader of mail** = eligibility for the recipient list of agent-to-agent mail.
    ///
    /// ★It is not the "turn observability" axis (it does not revert ADR-0116 decisions 1·7)★: terminal claude without structured output
    ///   **still receives**. What is decided here is not observability but **what the input is
    ///   interpreted as** — shell is a channel that **executes** input as commands, so an envelope reaching it is not read but
    ///   executed (the body is LLM free text, so if `&`·`|`·`;` are mixed in, what follows is parsed as a separate command).
    /// ★Default = true (fail-open)★: for a new CLI backend, reading mail is the normal side. Cutting delivery because it is unknown
    ///   would make mail silently disappear, so only a backend that is an execution channel declares false itself.
    // ADR-0004
    fn reads_messages(&self) -> bool {
        true
    }

    /// Whether this backend **is assigned a mail channel** = the sending pair of [`AgentBackend::reads_messages`].
    ///
    /// ★What happened while this field did not exist (do not bring it back)★: the daemon derived mail availability
    ///   from `!accepts_mcp_config` **alone**. So for a backend 「that uses the control channel but not mail」,
    ///   the moment it turned the control channel on, **send authorization opened along with it** — while receiving stayed false through the field above. That asymmetry,
    ///   with only sending open, was written in no declaration, and while the daemon recorded 「taught CLI mail」,
    ///   the actual spawn ended up in a state receiving not a single character of that education.
    /// ★So this axis **is stated by the backend** (ADR-0004)★: whether a program comes and goes on the mail plane is a per-program
    ///   fact. The daemon derives **one value** (`mail_allowed`) from this value and `accepts_mcp_config`, so
    ///   the single derivation point of ADR-0133 decision 2 stays as is — what changed is only that the ingredients grew from one to two.
    /// ★Default = true (fail-open)★: the same reason as the receiving field above. Cutting mail because it is unknown would make mail silently
    ///   disappear, so only a backend outside the mail plane declares false itself.
    /// ★`false` means "cannot use mail", not "cannot use control"★ — the control verbs are open to everyone
    ///   (ADR-0132 decision 5). Telling those two apart is why this field was set up.
    // ADR-0004
    // ADR-0133
    // ADR-0209
    fn uses_mail(&self) -> bool {
        true
    }

    /// The **input encoding tag** this backend uses for `command`. The session receives it once at spawn and keeps it.
    ///
    /// ★Why the backend (ADR-0004)★: "how this program reads stdin" is per-program knowledge.
    ///   If the dispatch layer picked the tag by looking at the command shape directly, that knowledge would leak into the shared layer.
    /// ★Default = `Raw`★: putting a made-up envelope on the stdin of an unknown program would make that program unable to read the input
    ///   at all — so a backend that declares nothing passes bytes through as is.
    // ADR-0004
    // ADR-0044
    fn input_encoder(&self, _command: &AgentCommand) -> InputEncoder {
        InputEncoder::Raw
    }

    /// Wraps one text turn into this backend's structured input line — the real thing behind [`InputEncoder::encode`].
    ///
    /// `msg_uuid` **must be the same** as the value passed to [`AgentBackend::input_echo_event`] (the caller generates it once
    /// and passes it to both — the dedup key).
    /// ★Default = no wrapping★: for a backend whose `input_encoder` is `Raw`, [`backend_for_encoder`] gives `None`
    ///   and this path is never taken. It is a placeholder to keep the contract total, and even if called it emits the same
    ///   bytes as `Raw`.
    fn wrap_input_turn(&self, text: &str, _msg_uuid: Uuid) -> Vec<u8> {
        text.as_bytes().to_vec()
    }

    /// The input-time user echo event the session layer emits right after input succeeds. `None` = none made.
    ///
    /// ★The event shape must be the same as what that backend's decoder makes for a replay★ — the frontend
    ///   merges the two by uuid, so if they diverge, two remain on screen. That shape is owned by the implementation.
    /// ★Default = none★: for a channel where the PTY echoes input locally right away, making a synthetic echo would duplicate it.
    // ADR-0044
    // ADR-0045
    fn input_echo_event(&self, _text: &str, _msg_uuid: Uuid) -> Option<OutputEvent> {
        None
    }

    /// The output refining decoder plugged in before pump→core. `None` = bytes pass straight through.
    ///
    /// The output-direction pair of [`AgentBackend::input_encoder`] — the side that wraps also owns the side that unwraps.
    /// ★Default = none★: the output of a backend that declares nothing flows as is without refining (terminal and plain text unchanged).
    // ADR-0004
    // ADR-0044
    fn output_decoder(&self, _command: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
        None
    }

    /// The event list restoring this command's past conversation on a resume spawn. An empty Vec = no seed.
    ///
    /// All knowledge of the transcript's file layout and format belongs to the implementation — the manager only makes this call and does not know
    /// the path rules.
    /// ★Default = empty Vec★: a backend that declares nothing does not restore the past (fresh buffer behavior unchanged).
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
    /// The sid we set at spawn can change inside that program (claude switches with `/clear`·`/resume`
    /// while the process stays the same — spike measurement). If the changed value is not followed, the next restore revives the old session.
    /// Where and how to read it (file path · JSON schema · PID workaround) all belong to the implementation, and
    /// the runner ([`crate::session_tracker`]) has only the period, the registry and the callback.
    ///
    /// `agent_id` is used only as the correlation key of the implementation's diagnostic logs — not for choosing what to observe.
    /// ★Default = `None` (no observation)★: a backend that declares nothing is not tracked. Making an observer without grounds
    ///   the default would read someone else's file and write a wrong sid into the profile.
    /// ★`None` is not a failure★: even without observation, restore works normally with the originally set sid (best-effort).
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

/// The bundle a backend hands out **in one dispatch** that is needed to assemble one spawn ([`AgentBackend::open_spawn`]).
///
/// ★The caps ownership split stays as is (ADR-0030)★: `backend_caps` is only the backend's share, and it is composed with the input/output/control caps
///   the link reports by `Capabilities::compose` in the session layer. It is not merged in advance here
///   — merging would mash the two sources into one value and lose track of which side reported what.
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
    /// Whether this incarnation can give ack notices — ★carries **the very** Arc the backend handed to its decoder and link★. Making a separate one
    ///   would split the value the session reads from the value the decoder fills, and that mismatch would show only as an eternal `Unknown`
    ///   without any error.
    // ADR-0231
    pub delivery_ack: Arc<DeliveryAck>,
}

/// The output event → turn signal mapping function (ADR-0113). The backend hands out its own function and `OutputCore` holds that pointer
/// for the session's lifetime and calls it on every event.
///
/// ★Why a function pointer★: emit is the hot path that runs on every agent output — dispatching with an `AgentCommand` on every event
///   would make the session hold its own copy of the command or retrace the manager, and `Box<dyn Fn>` causes an allocation.
///   The backend is a stateless unit struct, so the mapping falls out as a pure function; taking it once at spawn
///   and holding it as a pointer makes allocation, locking and lookup all zero.
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

/// The **backend that owns** the encoding tag. The encoder-axis pair of `backend_for` (the command axis) — the session layer holds only the tag
/// and not the command (ownership split), so it is traced back here.
///
/// `None` = that tag has no backend knowledge (bytes pass through).
/// ★Backend names are written only in this table, `backend_for` right above, [`usage_probes`] below, and the singleton declarations★ —
///   if a backend name appears outside them, the `backend/<name>/` folder isolation has leaked (ADR-0004).
fn backend_for_encoder(e: InputEncoder) -> Option<&'static dyn AgentBackend> {
    match e {
        InputEncoder::Raw => None,
        InputEncoder::ClaudeStreamJson => Some(&CLAUDE_BACKEND),
        // A tag whose envelope is made by the link, so the backend has nothing to wrap — `encode` passes through, and there is no echo.
        InputEncoder::TransportFramed => None,
    }
}

// ── Usage prober registry ───────────────────────────────────────────────────────

/// All active usage probers — the receiver (the daemon) gets the key list, cooldowns and time limits only from here (it does not know vendor names or policies).
///
/// ★Not being a field of [`AgentBackend`] is intended★ — the query runs without an agent. As a trait field, it would become a two-layer dispatch
///   that has to make up a command and go through `backend_for`.
/// The keys are distinct from each other (a test measures it).
// ADR-0004
// ADR-0246
pub fn usage_probes() -> [&'static dyn UsageProbe; 2] {
    [&claude::CLAUDE_USAGE_PROBE, &codex::CODEX_USAGE_PROBE]
}

/// Word → that vendor's prober. `None` = there is no prober for that word.
///
/// ASCII case-insensitive — the wire (`"claude"`) and the bus (`"Claude"`, the word of `agent.new`) reach one prober.
/// ★The entry key is made from the [`UsageProbe::key`] of the returned prober, not from the incoming word★ — making the key from the incoming spelling
///   would have the two entries hit different entries.
pub fn usage_probe_for(word: &str) -> Option<&'static dyn UsageProbe> {
    usage_probes()
        .into_iter()
        .find(|probe| probe.key().as_str().eq_ignore_ascii_case(word))
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
/// ★The place that keeps the activation entries from each writing this rule★: boot restore, the command bus and WS all call here.
/// Copying the rule would have the next entry transcribe only one of the two terms, and only that entry would silently decide differently.
/// ★This function does not know about an explicit resume request★ — whether to add it on top is decided by the calling entry.
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

pub fn build_command_spec(
    c: &AgentCommand,
    mode: SpawnMode,
    session_id: Option<Uuid>,
    resume_session_id: Option<Uuid>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    control: Option<ControlEndpoint>,
) -> CommandSpec {
    backend_for(c).build_spec(c, mode, session_id, resume_session_id, cwd, env, control)
}

/// The fail-closed gate **right before** assembly (canonical doc = [`AgentBackend::precheck_control_endpoint`]).
///
/// ★The place where the assembly point calls this must be **after** the provision guard is armed★ — so that when cut off here,
///   the token and file already issued are reclaimed.
pub fn precheck_control_endpoint(
    c: &AgentCommand,
    control: Option<&ControlEndpoint>,
) -> Result<(), String> {
    backend_for(c).precheck_control_endpoint(c, control)
}

pub fn transport_shape(c: &AgentCommand) -> TransportShape {
    backend_for(c).transport_shape(c)
}

/// Both the real link and the aspect values carried on it are owned by [`AgentBackend::open_spawn`], and this function is
/// only dispatch — a new backend implements that method in its own folder and this function is not touched
/// (swappability). ★This call launches the child process★ — unlike the dispatches above and below it has side effects, and if it fails
/// that spawn does not hold.
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
    )
}

/// Whether this backend's link can be used only **after setting up a connection** = whether there is an axis to deliver the outcome.
///
/// ★The assembly point decides by this **whether to lay down** the delivery port★ — giving a port to a backend without the axis would make the supervisor
///   wait on a channel nobody calls. The activation decision of a `false` backend is the old path as is
///   (claude · shell · stdio · codex terminal — byte for byte the same).
/// ★Why the backend (ADR-0004)★: 「can this program be used right after it launches, or does it need a round trip」 is
///   per-program knowledge. If the manager did matches! on command directly, that knowledge would leak into the shared layer.
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
/// keeps it, and applies it on every write_input.
///
/// ★Design intent★: the transport always writes only raw bytes (a dumb pipe — ADR-0044). The knowledge of "wrapping a text turn
/// into a claude JSON line" is owned by the backend. The session holds only this enum (tag), and the actual
/// schema lives only inside the [`AgentBackend::wrap_input_turn`] implementation (ADR-0004 isolation — neither this module nor the session nor
/// the transport knows the shape).
/// The physical link shape the backend requires — a **reported value**. What builds the real link is
/// [`AgentBackend::open_spawn`], and even inside it this value is not matched again to pick a constructor
/// (ADR-0191 — the only deciding switch is `backend_for`). Today the only place reading this value is the declaration-table tripwire
/// (`tests::expected_codec_axis`).
///
/// ★Whether output is structured is implied here★: `StdioNdjson` means the bytes that pipe carries are line-based JSON,
/// and the link itself does not know that (a dumb pipe — ADR-0044). So that `structured` output cap
/// is not hardcoded but injected, and the side injecting it is the backend that chose the pipe (ADR-0030 division kept).
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
    /// ★Distinction from [`TransportShape::StdioNdjson`]★: that one is a one-way pipe where the other side reads the bytes we wrote as is
    ///   and we only read the other side's lines, so what makes the envelope is the backend encoder
    ///   ([`InputEncoder`]). This one is a protocol where the other side **also asks us and waits for the answer**, so the id, method and order of
    ///   outgoing lines are held directly by the link — so the bytes [`AgentTransport::send_input`] receives are
    ///   **the message body**, not a wire frame, and the encoder does not touch them
    ///   ([`InputEncoder::TransportFramed`]).
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
    /// ★Why `Raw` is not reused★: `Raw` is "a channel imitating a human keyboard through a PTY", so
    ///   [`InputEncoder::submit_sequence`] emits a CR, and the session emits that CR once more **as a `send_input`
    ///   call separate from the body** ([`crate::session::AgentSession::submit_input_observed`]).
    ///   If that call reached a link that builds the envelope itself, one more turn with an empty body would go out.
    TransportFramed,
}

impl InputEncoder {
    /// `msg_uuid`: the message uuid of this user turn (replay dedup key). ClaudeStreamJson plants it in the stdin user line
    ///   so claude echoes it back as is on replay (uuid dedup contract = `backend/claude/`).
    ///   The same write_input also passes this uuid to input_echo_event, so the synthetic echo and the replay are merged by uuid.
    ///   Raw (terminal · shell) does not use the uuid (ignored) — the byte-identical guarantee is kept.
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
    /// ★Why here (backend)★: for the terminal (Raw) the PTY echoes input locally right away, but in json mode it does not appear on screen
    ///   until claude echoes it back. To remove that round-trip delay, a synthetic user event is emitted right after write_input.
    ///   Which encoder needs this echo and what the event's json schema is are backend knowledge, so
    ///   it is decided here, not in the session (ADR-0004 — the session holds only the encoder tag and does not know the shape).
    ///   Raw (terminal) returns None so the session emits nothing (the PTY already echoes — prevents duplicates).
    ///
    /// ★Paired with the decoder uuid dedup (blunt-suppress → uuid dedup replacement)★: this event has the same shape as what the decoder makes for a replayed
    ///   user-role block (`Structured{kind:"user", json:{"type":"text",
    ///   "text":<raw>,"uuid":"X"}}`). `msg_uuid` is the same as the value planted in stdin (encode), so the replay (same uuid) claude
    ///   later echoes back is deduped by uuid in the frontend accumulator and merged into one. Previously the decoder
    ///   blunt-suppressed user text blocks and this synthetic echo "took their place", but that was a bug making past conversations disappear on resume,
    ///   so it was changed to uuid dedup (past/non-matching-uuid user text is all preserved).
    pub fn input_echo_event(
        &self,
        bytes: &[u8],
        msg_uuid: Uuid,
    ) -> Option<crate::types::OutputEvent> {
        backend_for_encoder(*self)?.input_echo_event(&String::from_utf8_lossy(bytes), msg_uuid)
    }

    /// The submit (turn start) bytes that must be sent once more **as a separate write** after the encoded body.
    /// `None` = the one body write is already the submit.
    ///
    /// ★Why it cannot be appended inside `encode` (measured 2026-08-17 — do not bring it back)★: putting `body + CR` into a live claude TUI session
    ///   in **one write** leaves the text in the input box and the turn does not start. What makes the submit
    ///   is not the byte sequence but **when the receiver reads it**, so it cannot be expressed by `encode`, which returns one byte sequence
    ///   — hence a separate question.
    /// ★Splitting the write alone is not enough★: even when split, **without a gap** the two writes are bundled into one chunk in the PTY,
    ///   the receiver gets them in one read, and in the end it is the same as writing them together. So a paired wait is needed, and
    ///   that value is [`SUBMIT_PACING`] (the two always go together — do not fix one while looking at only one side).
    /// ★`Raw` = CR★: terminal · shell are channels imitating a human keyboard through a PTY, so Enter = CR (0x0D).
    /// ★`ClaudeStreamJson` = None★: the terminating `\n` that `encode` appends is that protocol's submit. Adding a CR here
    ///   would put surplus bytes after the line boundary and pollute the payload.
    /// ★`TransportFramed` = None★: one body byte sequence is one turn as is, and putting it into an envelope and submitting it is
    ///   the link's job. With a value here, the session would emit those bytes once more **as a `send_input` call separate from the body**,
    ///   and the link would read it as one more body and open an empty turn.
    /// ★The keystroke path does not look at this value★: the only consumer is `AgentSession::submit_input_observed`, where "one completed message = one turn".
    ///   Inserting a submit into terminal streaming input where a person types Enter directly
    ///   (`write_input`) would submit a turn on every keystroke.
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
    /// - `ClaudeStreamJson`·`TransportFramed` → always `true`: 1 call = 1 complete user turn
    ///   (the FIX 6a contract of `AgentSession::write_input` · the codex link makes one turn per write).
    /// - `Raw` → `true` if it contains the submit bytes (CR).
    ///
    /// ★The CR decision of `Raw` is tied to the frontend's key encoding★: terminal Enter = CR is the byte xterm sends with its **default** key
    ///   encoding. Turning on a mode that sends Enter as a different sequence, like the kitty keyboard or win32-input-mode, makes this decision
    ///   **silently** false so submits are not counted and every resume of that mode
    ///   disappears. It is paired with [`InputEncoder::submit_sequence`], which stands on the same assumption — do not fix only one side.
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
/// ★Why it is needed (measured 2026-08-17 — do not delete)★: splitting the write in two alone does not submit. Without a gap
///   the two writes are bundled into one chunk in the PTY and the claude TUI receives them in **one read**, treating the CR as part of
///   pasted text and keeping it in the input box. What makes the boundary is not the number of writes but **the receiver's
///   read boundary**, and the means of widening it is this wait.
/// ★The old observation that "it submits even with 0 delay" was a measurement error (do not bring it back)★: that measurement sent control-plane commands **twice
///   separately**, so the websocket round trip and task switching had already opened a few milliseconds, but it was read as 0ms.
///   The delivery path that writes consecutively in the same function has no such gap and actually did not submit (reconfirmed with a live
///   agent).
/// ★The source of the value = not an arbitrary constant (ADR-0038)★: the settled value tmux-based multi-agent orchestrators solving the same problem
///   use against Claude Code is 0.5 s, for the same reason ("combining text and Enter, or sending them too close together,
///   races with the input buffer"). Do not shrink or grow it without grounds — shrinking it brings this defect right back.
pub const SUBMIT_PACING: std::time::Duration = std::time::Duration::from_millis(500);

pub fn input_encoder(c: &AgentCommand) -> InputEncoder {
    backend_for(c).input_encoder(c)
}

// ── Output refining (ADR-0044/0004/0045) — the symmetric pair of the input encoder ──────────────────────────

/// The output refining decoder plugged in before pump→core. None = bytes pass straight through (terminal and plain text unchanged).
///
/// ★Symmetry★: the output-direction pair of `input_encoder` (input direction). Both the decision and the real decoder
/// are owned by [`AgentBackend::output_decoder`], and this function is only dispatch — the session knows only the encoder tag,
/// the transport only `dyn OutputDecoder` (ADR-0004). A new backend implements that method in its own folder
/// and this function is not touched (swappability).
pub fn output_decoder(c: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
    backend_for(c).output_decoder(c)
}

// ── ADR-0079: `.jsonl` transcript → past event seed on resume (backend dispatch) ──────

/// ADR-0079: the list of `OutputEvent`s restoring this command's past conversation on a resume spawn. An empty Vec = no seed
/// (existing fresh buffer behavior unchanged).
///
/// ★Backend knowledge isolation (ADR-0004)★: the transcript's file layout, format and seed decision all belong to the
///   [`AgentBackend::resume_transcript_events`] implementation, and this function is only dispatch. The manager also
///   calls only this dispatch and does not know the path rules. The resume-direction pair of `output_decoder` (live refining).
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
    // ★ Never add a wildcard ★ — when a new AgentCommand variant appears, this match
    // gives a compile error and forces a visit to the checklist below (prevents list rot —
    // a list whose update is forced by compilation does not rot).
    //
    // Checklist when wiring a new variant:
    //   ① Do not copy-paste the stub for the new backend's two capabilities — fill them with values measured in a CLI spike.
    //   ② Wiring with supports_control_channel=false makes a standalone agent without messaging (check that it is intended).
    //   ③ If non-MCP (accepts_mcp_config=false), provision automatically picks the [Cli] grant and **does not load priming
    //      at all** (the CLI copy was deleted in commit `2ef6902` and the `roundtrip-smoke --cli-only` knob went
    //      away with it — do not try to bring them back). That branch has no means of measurement today — ADR-0209.
    //   ④ If MCP-capable, measure with the default roundtrip.
    //   ⑤ **Do not derive** the third field (writes_mcp_config_file) **from the second** — 「does it use mail through MCP」 and
    //      「does it read the mcp-config file we wrote」 are separate. true only when that program actually has a flag
    //      pointing at that file (claude's `--mcp-config`). If true, a plaintext token JSON is written to disk on every spawn and
    //      a failure of that write cuts the spawn (fail-closed) — turning it on for a backend that does not read it is pure loss.
    //   Reference: ADR-0099.
    fn expected_channel_matrix(c: &AgentCommand) -> (bool, bool, bool) {
        // (supports_control_channel, accepts_mcp_config, writes_mcp_config_file) — values measured in the CLI spike
        match c {
            AgentCommand::Claude { .. } => (true, true, true),
            AgentCommand::Shell { .. } => (false, false, false),
            // ★The second field of this row **is a wrong value if read by its name — it was turned on knowingly**★: codex still cannot consume
            //   the mcp-config file we make (it has no flag pointing at that file — measured 0.155.0).
            //   But the daemon derives even the mail channel decision from this one field (`mail_allowed`), so leaving it false
            //   would leave **the CLI mirror open** for this backend, which uses mail through MCP. The canonical reasoning is
            //   that method's comment in `backend/codex/` and is not repeated here.
            // ★So the third field is false — do not copy the second★: the cost of writing a file nobody reads (a plaintext token JSON on every
            //   spawn · spawn aborted by that write's failure) went away when this field was split. Reverting this to true
            //   brings that cost right back.
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
        // ★Regression of why `Structured` interpretation is per backend★: claude carries the input-time user echo here, so it
        //   counts as progress, but a backend that declared no mapping must be silent on the same event — otherwise
        //   structured meta lines unrelated to turns create a permanent "in a turn" with no end signal.
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
    // ★Do not add a wildcard★ — the default is true (fail-open), so it becomes a recipient even without any declaration, and
    // what this axis prevents is **LLM free text being executed as commands**, so silent inheritance becomes a safety problem.
    //
    // ★Omissions from the list are caught too (that is why slots are used)★: when it was a single hand-filled Vec, forgetting to put a new variant
    // there meant `reads_messages` was never called and fail-open passed with the test green. Adding a new variant
    // gives ① a compile error in the `variant_slot` match → ② increasing the slot number mismatches `BACKEND_VARIANTS` and
    // the sample array length, another compile error → ③ and if the sample is still not filled, the "all slots filled" assertion below
    // breaks. One of the three gates always catches it.
    //
    // Classification criterion: true if it is an agent that **reads and interprets** input (CLI coding agents etc.), false if it is a channel that
    //   **executes** input (shell · REPL kinds). If unsure, leave it false and raise it to the user.
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
            // ★It was once false, and the reason was 「turns cannot be observed, so busy times cannot be told apart」 — ADR-0116 decision 7
            //   rejected that premise★ (「cannot observe, so cannot deliver」 = an alternative that ADR
            //   rejected). Terminal claude already receives in the same place, so there is no ground to exclude only this backend, and measured by the
            //   classification criterion above, codex is on the side that **reads and interprets** input, so it is true.
            //   The canonical reasoning is that method's doc in `backend/codex/`.
            AgentCommand::Codex { .. } => true,
        }
    }

    /// The sample list that every tripwire below runs over.
    ///
    /// ★Deleting one line from this list makes **that mode drop out of those tripwires entirely while everything stays green**★
    ///   (measured — confirmed by deleting the codex app-server line and running). Variant slot coverage does not count modes, so
    ///   it cannot catch that silence. So a separate gate measuring the list itself was set up:
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

    /// The gate guarding the sample list. ★What those tripwires measure is only what this list reaches, so if the list has a hole,
    /// that hole **is visible nowhere**★ — everything stays green while that mode is never asked.
    ///
    /// ★The point is that a slot is not a variant but **(variant × output mode)**★: variant slots do not count modes,
    /// so even if the second mode is missing it looks filled. Whether a new variant or a new [`AgentOutputFormat`] value,
    /// when it grows the match below gives a compile error.
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
    // ★Do not add a wildcard★ — neither axis has a default in the trait, so **a new backend** has no choice but to write the values,
    // but **a new output mode** is invisible to the compiler (one impl receives every mode of that backend). What sets up that place
    // is the per-mode match below.
    //
    // The two axes ask different things — do not fold them into one field:
    //   ① assigns_session_id — whether **we** draw the id and hand it over at spawn. If true, the manager
    //      issues it and persists it to the profile. Turning it on for a program that issues its own id plants a uuid the program never
    //      uses, and later resume decisions stand on that fake value.
    //   ② can_resume_stored_session — whether it can resume with that stored id. **It does not ask who issued it**.
    // ★Do not fold the two fields into one★: the moment they are folded, fixing one drags the other along, and the two drive different
    // consumers (issuing = at spawn · resuming = the three activation entries). There are already rows where the values actually differ —
    // both codex modes are (false, true): codex does the issuing (we have no value to plant), and we resume with the value
    // we recorded. ★If you meet a place where the terminal mode is written as (false, false), it is stale★ —
    // that mode resumes with `codex resume <id>` (ADR-0208), and the handle is written down by the hook.
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
            // ★This row is the first row where the values of the two fields differ★ — codex still does the issuing (①), and we send
            //   `thread/resume` with the thread id we recorded (②). The canonical reasoning is the comments of those
            //   two methods in `backend/codex/`.
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
    /// resume★. Both terms are measured, so if any entry reverted to looking only at `sid.is_some()` instead of this function, that entry's
    /// test breaks along with this one.
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

        // ★codex resumes through **both links** — only the means differ★: app-server with `thread/resume`,
        //   terminal with `codex resume <id>`. Measuring both is the point — measuring only one would hide here
        //   「a resuming spawn is reported as a new conversation」 when the other is silently turned off.
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
    // ★Do not add a wildcard★ — the defaults of this axis (`Raw` · no decoder) are both fail-open, so
    // it compiles without any declaration and silently goes green. But if `Raw` is attached to a program that requires structured stdin,
    // that agent **cannot read the input at all**, and at runtime there is no signal. Whether an `AgentCommand`
    // variant or an `AgentOutputFormat` value grows, the match below gives a compile error.
    //
    // How to fill it — with values measured in a CLI spike:
    //   ① input_encoder — how that program reads stdin (`Raw` if there is nothing to wrap)
    //   ② whether there is an output_decoder — whether the output is structured and needs refining
    //   ③ transport_shape — whether receiving that output needs a terminal or a pipe
    //
    // ★③ goes with ①② but is not the same axis★: a wrong link shape compiles and **even launches
    // plausibly at runtime** — receiving a TUI through a pipe breaks the screen, and receiving line-based JSON through a PTY mixes in
    // escapes. The default (`Pty`) is fail-open, so without a column here a new backend silently inherits it.
    //
    // ★What this table does not cover = the resume transcript seed★: `resume_transcript_events` has the same fail-open
    //   default (an empty Vec), but deciding it requires an actual transcript file, so "no seed" and "an empty Vec because
    //   there is no file" cannot be told apart here. That regression net is carried by the transcript unit
    //   tests of `backend/claude/` — making a third column here would only create one column that measures nothing.
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
            // ★The point of this row is that the app-server mode's encoder is not `Raw`★ — if it were `Raw`, the session would
            //   emit a CR once more **as a separate call** after the body, and the link, which builds the envelope itself, would read it as one more
            //   body and open an empty turn.
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
    // ★Why it is measured separately from the codec-axis table above (since ADR-0191)★: back when the assembly point matched `transport_shape` to pick
    //   a constructor, the third column of that table transitively asserted even **the link that actually launches**. When link creation
    //   moved into the backend's `open_spawn`, production code reading that reported value went to 0, and since then
    //   declaring a pipe while taking the default `open_spawn` (=PTY) still leaves that column green. The runtime symptom is only a broken
    //   screen, so the only thing catching that mismatch now is this test.
    // ★Why it is not split by `structured`★: that field is a value the backend injects, not the link
    //   (ADR-0030/0044), so a plain pipe also reports false — what actually tells the two implementations apart is the `terminal_bytes` and `resize`
    //   both hardcode.
    // ★No slot coverage assertion was added★: the tripwires above running the same sample list already measure it.
    // ★spec is not that backend's real CLI★: `open_spawn` takes what to launch from spec and which link to
    //   launch it with from command separately, so it launches a probe that ends immediately and looks only at the link choice (ADR-0012).
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
                // A pipe, so neither terminal bytes nor size — the same pair as the one-way pipe. `interrupt` is not
                //   in this pair: on the pipe side that field is a value the backend injects (ADR-0238), not a link report.
                //   Today both pipe shapes are true, and what `interrupt` tells apart is PTY (false · ADR-0245) and
                //   pipes.
                TransportShape::StdioBidiJson => (false, false),
            };
            let parts =
                open_spawn(c, &probe, 80, 24, None, None, None, None, None).expect("open_spawn");
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
    /// ★The assumption is baked into the name★: this decision stands on xterm's default key encoding (Enter = CR). If a key encoding mode that sends Enter
    ///   as a different sequence comes in, **the decision itself**, not this entry, must be revisited.
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

    /// ★An encoder that emits submit bytes counts those bytes as a turn★ — mail delivery counts with `submits_turn(submit bytes)` in front of the
    ///   submit bytes, so if only one side (e.g. the submit bytes of `Raw`) changes, mail is **silently**
    ///   not counted and that agent's resume disappears.
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
}
