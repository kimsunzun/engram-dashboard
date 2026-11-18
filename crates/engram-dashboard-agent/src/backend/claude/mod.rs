//! ClaudeBackend — produces the CommandSpec for the claude CLI only.
//!
//! ★Claude knowledge lives only in this folder (ADR-0004)★: flags · env conventions · stream-json I/O schema · `.jsonl` transcript layout · resume-failure text · turn-signal mapping. The outside (manager · `backend/mod.rs`
//! dispatch · transport · core) carries only abstract descriptors and bytes.
//! ★A folder, not a file★ — a bare `claude.rs` cannot say 「claude ends here」, so leaks into dispatch went unread as violations; it did leak (`backend/mod.rs` called this module directly for input encoding · synthetic echo ·
//! output decoder · transcript seed).
//!
//! ★The surface that goes out = one [`crate::backend::AgentBackend`] implementation + two singletons: the usage probe
//!   [`CLAUDE_USAGE_PROBE`] ([`crate::usage::UsageProbe`]) and the settings check [`CLAUDE_SETTINGS_CHECK`]
//!   ([`crate::backend::SettingsCheck`])★: when the outside needs new knowledge, add a method to one of those traits
//!   and implement it here — the outside does not call this module's items by name. The two singletons are not trait
//!   slots because they run without an agent (no double dispatch), and the only place that may name them is the
//!   registry in `backend/mod.rs` — outside the crate `pub(crate)` lets the compiler stop it, inside the crate the
//!   isolation gate below watches.
//! ★Isolation gate (common to the four backend folders — swap the name)★:
//!   `rg -n --glob '*.rs' --glob '!**/backend/claude/**' "\bclaude::" crates/ src-tauri/`
//!   ★Do not count hits★ — judge whether each is the **registry** of `backend/mod.rs` (`pub use claude::ClaudeBackend;`); the registry and dispatch table must write the name, any other hit is a violation.
//!   ★What it cannot see★: ① **knowledge leaking without the vendor name** (a magic timeout, a bare `pid` field); ② the backend alphabet is hand-written, so **a new backend is invisible until its name is added**; ③ an alias
//!   import (`use crate::backend::claude as c;`) and `#[path]` relocation leave no `claude::`.
//!
//! tauri import 0.

mod launch;
mod leftover;
mod session_file;
mod settings;
mod usage;
mod usage_probe;

pub(crate) use settings::CLAUDE_SETTINGS_CHECK;
pub(crate) use usage_probe::CLAUDE_USAGE_PROBE;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engram_dashboard_base::panic::uncontained;
use engram_dashboard_base::time::SystemClock;
use engram_dashboard_platform::env::env_key_eq;
use engram_dashboard_platform::shell::console_command;
use uuid::Uuid;

use leftover::{Cleaner, GateCell, LogTag};

use crate::backend::{
    inject_cli_entrance, AgentBackend, FirstTurnSink, InputEncoder, SessionIdSink, SpawnParts,
    TransportShape, TurnClassifier,
};
use crate::failure::AgentFailureKind;
use crate::launch::{LaunchArtifacts, LaunchFs, LaunchKit};
use crate::profile::{AgentCommand, AgentOutputFormat, SpawnMode};
use crate::session_tracker::SessionIdSource;
use crate::transport::pty::PtyTransport;
use crate::transport::stdio::{InterruptLine, StdioTransport};
use crate::transport::{AgentTransport, LinkSink, OutputDecoder};
use crate::turn::{TurnEndKind, TurnSignal};
use crate::types::{
    AgentId, BackendCaps, CommandSpec, ControlEndpoint, DeliveryAck, DropCause, MidTurnPolicy,
    ModelCaps, OutputEvent, PtyError, QueuedInputEvent, SessionCaps, ToolCategory, ToolGrant,
    TurnOutcome,
};
use crate::usage::UsageObservation;

const CLAUDE_PROGRAM: &str = "claude";

/// env carried by every claude we launch — turns off only the background auto-update (a manual `claude update` stays). We end claude forcibly (Job Object terminate) and being cut off mid-update broke the shared
/// installation. Grounds · discarded alternatives: `docs/research/child-process-shutdown-peers-2026-10-14.md` §5 (chosen = C).
const NO_AUTOUPDATE_ENV: (&str, &str) = ("DISABLE_AUTOUPDATER", "1");

/// Whether claude launches with `--output-format stream-json` — the point where the four axes in this folder (link · input encoding · output decoder · transcript seed) split together.
///
/// ★Do not move this predicate back to `profile`★ (ADR-0004): claude's output-format axis would drive the shared layer's link choice. Only a neutral axis value such as [`AgentBackend::transport_shape`] goes out.
fn is_stream_json(command: &AgentCommand) -> bool {
    matches!(
        command,
        AgentCommand::Claude {
            output_format: AgentOutputFormat::StreamJson,
            ..
        }
    )
}

/// Mid-turn input policy — only JSON mode has the session classify and the vendor release ([`cancel_line`] is the cancel means). Terminal mode has neither a line decoder for per-entry acks nor a control channel for a cancel line.
// ADR-0231
fn mid_turn_policy(command: &AgentCommand) -> MidTurnPolicy {
    if is_stream_json(command) {
        MidTurnPolicy::SessionClassified { cancel_line }
    } else {
        MidTurnPolicy::None
    }
}

/// **Lowercase fragment** of the text claude emits when `--resume <sid>` cannot find the conversation.
///
/// ★An external program's output, not our value★ — measured: `--resume <missing uuid>` emits `No conversation found with session ID: …` with exit 1 (`docs/process/S9-session-restore/spike-results.md`).
/// ★A failed match does not lock the entry★: if claude changes the text the classification falls to 「early exit right after resume」, which is retryable (ADR-0172 fail-open).
// ADR-0172
const NO_CONVERSATION_MARKER: &str = "no conversation found";

pub struct ClaudeBackend;

impl AgentBackend for ClaudeBackend {
    /// The caller sets it with `--session-id <uuid>` (measured) — both shapes are the same.
    fn assigns_session_id(&self, _command: &AgentCommand) -> bool {
        true
    }

    /// Resumes the saved sid with `--resume <uuid>` — both shapes are the same.
    fn can_resume_stored_session(&self, _command: &AgentCommand) -> bool {
        true
    }

    fn supports_control_channel(&self) -> bool {
        true
    }

    fn accepts_mcp_config(&self) -> bool {
        // claude attaches the mcp-config with `--mcp-config` → MCP-capable.
        //
        // ★What this value decides — education and enforcement, not wiring (ADR-0133)★:
        //   - **Wiring goes to everyone.** The `engram` executable, credentials and PATH are laid down for MCP-capable spawns too (control verbs open to all — ADR-0132 decision 5; one executable cannot be split by family).
        //   - **Teaching follows this value, but this value does not load it.** The priming comes only from the launch materials
        //     (ADR-0370), and the tripwire `every_backend_with_a_settings_check_has_what_the_priming_teaches` (backend `mod.rs`) keeps
        //     every backend that gets it MCP- and mail-capable; a non-MCP spawn gets no instructions (ADR-0209 decision 3). The CLI-only
        //     copy was deleted in commit `2ef6902`.
        //   - **Enforcement is only the daemon's refusal** of mail requests carrying an MCP-capable spawn's credentials. ★The mail screen of `engram help` opens regardless★ — the spawn-env marker that chose the screen was removed;
        //     bringing it back would let a value the agent can erase decide the screen.
        // ★Teaching a channel the spawn cannot use brings back the send freeze ADR-0099 measured★ — what the priming teaches and the
        //   mail permission must agree. Mail permission splits on this value; the priming is not loaded by it (ADR-0370), so on the real
        //   path the tripwire above holds the agreement, and a non-MCP spawn agrees by **teaching nothing**.
        // ★The living invariant is channel unification (ADR-0128 decision 1)★: the mail channel is decided only by backend capability, no runtime switching or fallback; ADR-0133 changed only **how** it is enforced.
        // ADR-0099
        // ADR-0126
        // ADR-0128
        // ADR-0133
        // ADR-0209
        // ADR-0370
        true
    }

    /// ★This backend **actually opens** that file★ — `build_spec` passes `--mcp-config <path>` and claude reads it and carries the `Authorization` header (measured on 2.1.170; the daemon's `control/mcp_config.rs` header
    /// is the schema's canonical source). Only here is it true.
    /// ★Keep fail-closed on a write failure of that file★: without it the injection below runs not one line and the MCP entry vanishes — an agent without a control channel. Canonical decision: the `?` in the daemon's `control::provision`.
    // ADR-0086
    // ADR-0099
    // ADR-0209
    fn writes_mcp_config_file(&self) -> bool {
        true
    }

    fn materialize(
        &self,
        _command: &AgentCommand,
        kit: &LaunchKit,
        _cwd: &Path,
        fs: &dyn LaunchFs,
    ) -> Result<LaunchArtifacts, String> {
        launch::materialize(kit, fs)
    }

    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        _resume_session_id: Option<Uuid>,
        cwd: PathBuf,
        mut env: Vec<(String, String)>,
        control: Option<ControlEndpoint>,
        artifacts: &LaunchArtifacts,
    ) -> CommandSpec {
        match command {
            AgentCommand::Claude {
                extra_args,
                output_format,
            } => {
                let mut args = Vec::with_capacity(8 + extra_args.len());
                // ADR-0341: the folder entries go first — `--add-dir` is variadic, and the always-present
                //   `--permission-mode` right after the group ends it, so no later bare value (an `extra_args` one) joins it.
                for dir in &artifacts.add_dirs {
                    args.push("--add-dir".to_string());
                    args.push(dir.to_string_lossy().into_owned());
                }
                // ★AUTO permission mode — `--permission-mode bypassPermissions` (user decision 2026-07-22)★: the spawned
                //   claude is a headless worker with no approver — default-deny + per-tool grant wiring cannot work without a
                //   person to answer the approval prompt (CLI measured 0/38, all permission-blocked). It is a **temporary
                //   regime**; the proper replacement is a constraint layer shared by every LLM (this base flag goes then).
                //   ★Placement★: a NON-variadic pair, so it joins no absorbing group (see the extra_args comment below);
                //   it ends the `--add-dir` group above. Injected **unconditionally** — with
                //   or without a control endpoint, in every spawn mode.
                // ADR-0097
                args.push("--permission-mode".to_string());
                args.push("bypassPermissions".to_string());
                // ADR-0341: the launch-material env overrides a profile value for the same key — the stated exception to
                //   the profile-wins rule below (why the folder variable must hold: `launch.rs` `ADDED_DIRS_CLAUDE_MD_ENV`).
                for (key, value) in &artifacts.env {
                    env.retain(|(k, _)| !env_key_eq(k, key));
                    env.push((key.clone(), value.clone()));
                }
                // Skipped if the profile already set the same key — the same profile-first rule as MAX_THINKING_TOKENS below.
                let (key, value) = NO_AUTOUPDATE_ENV;
                if !env.iter().any(|(k, _)| env_key_eq(k, key)) {
                    env.push((key.to_string(), value.to_string()));
                }
                match output_format {
                    // ── Terminal (PTY interactive) — bytes/arguments frozen (no regression) ──
                    AgentOutputFormat::Terminal => {
                        if let Some(sid) = session_id {
                            let flag = match mode {
                                SpawnMode::Fresh => "--session-id",
                                SpawnMode::Resume => "--resume",
                            };
                            args.push(flag.to_string());
                            args.push(sid.to_string());
                        }
                    }
                    // ── JSON (headless stream-json) — ADR-0044 ──
                    // stream-json I/O is for claude `-p` only (measured: --help "only works with --print").
                    // --replay-user-messages echoes user turns into the output stream, so the frontend renders from the output as the single source.
                    AgentOutputFormat::StreamJson => {
                        args.push("-p".to_string());
                        args.push("--input-format".to_string());
                        args.push("stream-json".to_string());
                        args.push("--output-format".to_string());
                        args.push("stream-json".to_string());
                        args.push("--replay-user-messages".to_string());
                        // ★--verbose is required (M2 QA)★: claude 2.1.170 has no text about it in --help, but the runtime kills it at once with "When using --print, --output-format=stream-json requires --verbose" (the agent
                        //   vanishes right after spawn). Must not be removed.
                        args.push("--verbose".to_string());
                        // The vendor sends, **separately** from the `stream_event` text deltas, a complete `assistant` line per block too; the decoder's `PartialMessage` reduces the two copies to one (the frontend has no dedup).
                        // ADR-0240
                        args.push("--include-partial-messages".to_string());
                        // ADR-0044 follow-up done / ADR-0008 reuse: json (stream-json) resume enabled.
                        if let Some(sid) = session_id {
                            // ★Measured (claude 2.1.170)★: headless stream-json also supports `--resume <sid>` — it coexists with `-p`/`--input-format stream-json`/`--output-format stream-json` and resumes losslessly with no
                            //   "session already in use", so it uses the same mode split as the terminal branch (controlled-sid infrastructure, ADR-0008).
                            let flag = match mode {
                                SpawnMode::Fresh => "--session-id",
                                SpawnMode::Resume => "--resume",
                            };
                            args.push(flag.to_string());
                            args.push(sid.to_string());
                        }
                        // ADR-0049: measured (claude 2.1.170) — headless stream-json emits `"type":"thinking"` blocks only when env MAX_THINKING_TOKENS is present (off by CLI default), so 8000 is injected for json mode only;
                        //   the terminal path keeps CLI parity.
                        // ★Profile first (explicit-skip)★: skipped if the profile already set the same key — not relying on env merge order (later wins in the transport), the profile deterministically wins.
                        //   ★Key comparison follows the OS env name rules (`env_key_eq`)★: case-insensitive on Windows, so the profile's `max_thinking_tokens` counts as the same key.
                        //   Normalizing duplicate keys within the profile is out of scope (standard last-wins env semantics).
                        const MAX_THINKING_TOKENS_KEY: &str = "MAX_THINKING_TOKENS";
                        if !env
                            .iter()
                            .any(|(k, _)| env_key_eq(k, MAX_THINKING_TOKENS_KEY))
                        {
                            env.push((MAX_THINKING_TOKENS_KEY.to_string(), "8000".to_string()));
                        }
                    }
                }
                // ADR-0086: control channel injection — translates the endpoint the claude way. The CLI entry (credential env · PATH · mail marker) is laid down for **every spawn with a control endpoint**, and `--mcp-config` is
                //   **added** when there is a config_path (not exclusive — ADR-0133).
                //   ★Same injection regardless of mode★: terminal and json are both connection targets (claude 2.1.170 measured: the Authorization header is sent on every request).
                //   ★Making the two injections exclusive again would halve the control plane★: an MCP-capable spawn that cannot call `engram` cannot reach the control verbs (ADR-0132 decision 5). Mail isolation is the daemon's
                //   credential refusal, not this split.
                // ADR-0086 / ADR-0099 / ADR-0133
                if let Some(endpoint) = &control {
                    inject_cli_entrance(&mut env, endpoint);
                    if let Some(config_path) = &endpoint.config_path {
                        args.push("--mcp-config".to_string());
                        args.push(config_path.to_string_lossy().into_owned());
                    }
                }
                // Instructions — `--append-system-prompt-file <abs-path>`: claude CLI (2.1.170) reads the file itself
                //   and appends it after its default system prompt, in both modes.
                // ADR-0092
                // ADR-0316: real materials load the merged instructions here and turn the system-prompt snapshot off,
                //   so a resumed session also gets the current text.
                // ADR-0370: without real materials nothing is loaded — the control channel carries no priming file.
                if artifacts.real {
                    if let Some(instructions) = &artifacts.instructions_file {
                        args.push("--append-system-prompt-file".to_string());
                        args.push(instructions.to_string_lossy().into_owned());
                    }
                    args.push("--system-prompt-snapshot".to_string());
                    args.push("off".to_string());
                }
                // `--settings` takes a **file path**, not inline JSON: a Windows spawn goes through one more cmd.exe
                //   layer (`console_command`), whose quoting silently breaks a `{"…":[…]}` argument, and the result is
                //   "settings ignored, normal start" (ADR-0109).
                // ADR-0328: the merged raw settings (in the volatile folder) are the one `--settings`.
                // ADR-0370: without real materials there is none — the control-channel settings fragment is withdrawn.
                // ADR-0316: the skills ride in as a plugin folder here.
                if artifacts.real {
                    for root in &artifacts.skill_roots {
                        args.push("--plugin-dir".to_string());
                        args.push(root.to_string_lossy().into_owned());
                    }
                    if let Some(settings) = &artifacts.settings_file {
                        args.push("--settings".to_string());
                        args.push(settings.to_string_lossy().into_owned());
                    }
                }
                // ★Order invariant (load-bearing, ADR-0094 least privilege)★: `extra_args` (caller passthrough) is appended **before the `--allowedTools` injection**. `--allowedTools <tools...>` is **variadic** in claude (2.1.170) — it
                //   absorbs every positional argv value after it as an "allowed tool" until the next `--flag`. extra_args after the grant group starting with a bare positional (e.g. `Bash`) would be sucked in as an **extra allowed
                //   tool**, granting blanket Bash (an ADR-0094 violation). So extra_args goes first and the `--allowedTools` group is at the **very end**. Both modes feed the prompt through stdin (write_input) and
                //   `AgentCommand::Claude` has no prompt field, so argv has **no trailing positional** and absorption after the group is structurally impossible.
                args.extend(extra_args.iter().cloned());
                // ADR-0094 / ADR-0106: blocking the built-in SendMessage — `--disallowedTools SendMessage`, injected **only when there is a control endpoint**.
                //   ★Why★: the harness built-in `SendMessage` (PascalCase) collided with the **old name** of our MCP tool, `send_message` (engram, snake_case) — the spawned claude misread the priming, called the built-in, did not know the
                //   engram agent names and failed with "No agent named 'X' is reachable" (measured 2026-07-26, every spawn). Fixing only the priming let it recur, so the tool is blocked.
                //   ★The tool is now `eg_send` — the deny is deliberately kept★: the measured failure was **the harness misfiring its own built-in**, not proven to need similar spelling; removing it because the names diverged would learn
                //   the misfire is gone only at a spawn site. Not removed without a reproduction measurement.
                //   ★Scope = only with control (ADR-0106)★: only messaging-primed spawns (control endpoint) hit the collision; blocking it elsewhere would (a) lose a built-in for no reason and (b) make claude meet an unregistered tool in the deny list
                //   and pay a warning on every call.
                //   ★Order★: `--disallowedTools` is also variadic, so it goes **after** extra_args; the following `--allowedTools` ends it as a new `--flag` and keeps the allowedTools group last.
                // ADR-0094 / ADR-0106
                if control.is_some() {
                    args.push("--disallowedTools".to_string());
                    args.push("SendMessage".to_string());
                }
                // ADR-0094: send-entry pre-authorization — `--allowedTools <pattern>...`.
                //   ★Why the grant is kept (even with bypass as the spawn default)★: real authorization is the base flag's bypass; this least-privilege injection stays because (a) under bypass it is a harmless duplicate, (b) it is the deterministic
                //   policy surface when a future shared constraint layer removes bypass, and (c) it shows "which send entries this agent has". It is a policy declaration, not a gate — do not delete it.
                //   ★arg shape★: each pattern after `--allowedTools` is its own **args element** — a pattern with spaces (`Bash(C:\Program Files\…:*)`) must stay one argv element so claude does not split it (hence not one comma-joined value).
                //   The permission gate is mode-independent. ★Placed last★ — regression guard: tests `claude_allowed_tools_group_is_last_and_*`.
                // ADR-0094
                if let Some(endpoint) = &control {
                    let patterns = grants_to_allowed_tools(&endpoint.grants);
                    if !patterns.is_empty() {
                        args.push("--allowedTools".to_string());
                        args.extend(patterns);
                    }
                }
                let (program, args) = console_command(CLAUDE_PROGRAM, args);
                CommandSpec {
                    program,
                    args,
                    env,
                    cwd,
                }
            }
            // dispatch sends only the Claude variant to ClaudeBackend, but Shell is also handled defensively.
            AgentCommand::Shell { program, args } => CommandSpec {
                program: program.clone(),
                args: args.clone(),
                env,
                cwd,
            },
            AgentCommand::Codex { .. } => {
                unreachable!("ClaudeBackend does not handle the Codex variant. dispatch bug.")
            }
        }
    }

    /// Terminal and json (stream-json) both resume losslessly with `--resume <sid>`, so resume=true and `command` (the mode) is not looked at. snapshot and model options are unsupported (console CLI).
    /// The signature keeps the contract that the backend is the source of the session caps (ADR-0030 type split).
    fn capabilities(&self, _command: &AgentCommand) -> BackendCaps {
        let resume = true;
        BackendCaps {
            session: SessionCaps {
                resume,
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

    fn turn_classifier(&self) -> TurnClassifier {
        classify_turn
    }

    /// ★Do not bring back "an answer only comes in terminal mode (known and accepted)"★: a structured (stream-json) session still sends diagnostics to stderr, the caller captures it into a separate diagnostic buffer and merges it into
    ///   `evidence` (ADR-0172). stream-json is the default output format, so while that branch was silent this classifier was **effectively dead code for most agents**.
    /// ★claude emits this text only when it failed to resume★ — so a still-living session's diagnostics settle it (the trait doc's condition); adding text that could mix into the conversation body would break that.
    fn resume_failure_kind(&self, evidence: &str) -> Option<AgentFailureKind> {
        if evidence.to_lowercase().contains(NO_CONVERSATION_MARKER) {
            return Some(AgentFailureKind::NoConversationToResume);
        }
        None
    }

    /// stream-json requires a pipe — it streams line-based JSON to stdout, not a TUI.
    ///
    /// ★Only the reported value; the real thing is built by [`AgentBackend::open_spawn`]★ — both look at [`is_stream_json`], so fixing one diverges them, and the declaration-table tripwire measures only this value.
    fn transport_shape(&self, command: &AgentCommand) -> TransportShape {
        if is_stream_json(command) {
            TransportShape::StdioNdjson
        } else {
            TransportShape::Pty
        }
    }

    /// stream-json mode builds a structured pipe, terminal mode a PTY.
    ///
    /// ★The decision is [`is_stream_json`] alone — do not re-read `transport_shape` and match on it (ADR-0191)★ (a second switch on the shape value, the shape this decision removed).
    /// ★This is where `structured: true` is injected (ADR-0044/0030)★: the pipe does not know whether its bytes are line-based JSON, so [`StdioTransport`] receives the value and reports it in caps; this backend, which chose
    ///   `--output-format`, knows. The interrupt line function is plugged in for the same reason — the line shape and 「is a turn open now」 are this backend's knowledge (ADR-0238).
    /// ★`sid_sink` is unused because this backend **does not fetch** the session id★ — we issue it ([`AgentBackend::assigns_session_id`]) and later drift is observed by the file watcher ([`AgentBackend::session_id_source`]).
    // ADR-0044
    // ADR-0191
    fn open_spawn(
        &self,
        command: &AgentCommand,
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
        sid_sink: Option<SessionIdSink>,
        // ★For this backend, the session counts turns★ — there is no mode where the link owns the turn (`mid_turn_policy`), so there is no
        //   place to call this port (ADR-0226 — the session counts before sending).
        first_turn_sink: Option<FirstTurnSink>,
        resume_session_id: Option<Uuid>,
        // ★This backend has no link to set up — `declares_link()` is false, so the assembly point gives `None`
        //   in the first place. Receiving and ignoring it is the contract (the same shape as the `session_id` field).
        _link_sink: Option<LinkSink>,
        // ★The control plane is translated **entirely into the command line** for this backend ([`AgentBackend::build_spec`]) — sending it in the link handshake too would send one value by two means. Only `agent_id` is read here, as
        //   the attribution of leftover-cleanup logs.
        // ADR-0262
        control: Option<&ControlEndpoint>,
        _artifacts: &LaunchArtifacts,
    ) -> Result<SpawnParts, PtyError> {
        // Not used, as the doc above says — using an underscore name would make the signature rustdoc renders diverge from the doc.
        let _ = (sid_sink, first_turn_sink, resume_session_id);
        let delivery_ack = Arc::new(DeliveryAck::new());
        let (transport, child_pid): (Box<dyn AgentTransport>, Option<u32>) =
            if is_stream_json(command) {
                // Per incarnation, one new gate is held by the decoder (opens and closes it) and the interrupt line function (sees it open and sets up an interrupt episode).
                // The cleaner must know the link's group and root, so the gate is set up after the link is opened and the decoder receives it as a slot.
                // ADR-0238
                // ADR-0262
                let gate_slot = GateSlot::default();
                // ★The pickup decision is made here — `stream_decoder` does not receive the spawn spec (env), so it cannot decide★: keeps a non-default account's observations out of the default entry (`usage.rs`). The wrapper masks only usage
                //   pickup; decode/flush and the gate (inside the decoder) are unaffected.
                let decoder = usage::gate_decoder(
                    stream_decoder(Arc::clone(&delivery_ack), Arc::clone(&gate_slot)),
                    &spec.env,
                );
                let (t, pid) = StdioTransport::open(spec, true, Some(decoder))?;
                let gate = GateCell::new(leftover_cleaner(&t, pid, control.map(|c| c.agent_id)));
                gate_slot.get_or_init(|| Arc::clone(&gate));
                (Box::new(t.with_interrupt(interrupt_line(gate))), pid)
            } else {
                let (t, pid) = PtyTransport::open(spec, cols, rows)?;
                (Box::new(t), pid)
            };
        Ok(SpawnParts {
            transport,
            child_pid,
            backend_caps: self.capabilities(command),
            encoder: self.input_encoder(command),
            turn_classifier: self.turn_classifier(),
            reads_messages: self.reads_messages(),
            mid_turn: mid_turn_policy(command),
            // The very Arc the decoder above fills — the terminal branch has no decoder to fill it, so it stays at `Unknown`.
            delivery_ack,
        })
    }

    fn input_encoder(&self, command: &AgentCommand) -> InputEncoder {
        if is_stream_json(command) {
            InputEncoder::ClaudeStreamJson
        } else {
            InputEncoder::Raw
        }
    }

    fn wrap_input_turn(&self, text: &str, msg_uuid: Uuid) -> Vec<u8> {
        wrap_user_turn(text, msg_uuid)
    }

    /// ★Unconditionally `Some`★: called only through the tag chosen by `input_encoder` ([`crate::backend::backend_for_encoder`]); terminal mode is `Raw` and never reaches here — looking at the mode again would put one decision in two homes.
    fn input_echo_event(&self, text: &str, msg_uuid: Uuid) -> Option<OutputEvent> {
        Some(OutputEvent::Structured {
            kind: "user".to_string(),
            json: user_text_echo_json(text, msg_uuid),
        })
    }

    /// ★Production spawns do not go through this method★ — [`AgentBackend::open_spawn`] builds the decoder holding the incarnation-shared ack value and the turn-open gate
    /// directly. The decoder going out from here holds an ack value nobody reads, and its gate slot is empty.
    fn output_decoder(&self, command: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
        if is_stream_json(command) {
            Some(stream_decoder(
                Arc::new(DeliveryAck::new()),
                GateSlot::default(),
            ))
        } else {
            None
        }
    }

    /// Terminal claude is restored by the TUI through PTY repaint, so it is not seeded — doing so would show two copies on screen.
    // ADR-0079
    fn resume_transcript_events(
        &self,
        command: &AgentCommand,
        cwd: &std::path::Path,
        session_id: Uuid,
    ) -> Vec<OutputEvent> {
        if is_stream_json(command) {
            read_transcript_events(cwd, session_id)
        } else {
            Vec::new()
        }
    }

    /// Not split by mode — terminal and stream-json both write the same `sessions/<pid>.json` and the sid switch happens on both.
    // ADR-0004
    fn session_id_source(
        &self,
        agent_id: AgentId,
        child_pid: u32,
        expected_sid: Uuid,
    ) -> Option<Box<dyn SessionIdSource>> {
        session_file::ClaudeSessionIdSource::new(agent_id, child_pid, expected_sid)
            .map(|s| Box::new(s) as Box<dyn SessionIdSource>)
    }
}

/// claude stream-json events → turn signals (ADR-0113).
///
/// ★`Structured` counts as progress — load-bearing, **claude-only**★: claude emits the **user echo at input time** as this variant (`user_text_echo_json` · the decoder's user line), so turns started by the user typing into the
///   terminal and turns started by mail injection are both caught here; without it their turn starts fall outside observation.
/// ★Only one `kind` is filtered — the interrupted marker ([`INTERRUPTED_KIND`]) is not progress★: it marks the end; counted as progress after that turn's `result` it turns 「in a turn」 back on and blocks mail until the 30-minute
///   fail-open (the turn-open gate reads the same classification). Other `Structured` lines occur inside a turn, so kind is not split — if claude starts emitting structured lines outside turns, filter that kind here too.
/// ★`Usage`/`Error` are not ends★: `Usage` also comes mid-turn; `Error` is not a turn boundary (a failed turn also closes with `MessageDone` — decoder FIX-C). `TerminalBytes` has no boundary information.
/// ★Among `Error`s only a failed `result`'s is the 「turn error」 (`Failed`) — told apart by prefix ([`RESULT_FAILURE_DETAIL`])★: it comes from the same line as the `MessageDone` right after, so the table folds that end into an error end
///   (stop after error). ★Do not count other `Error`s (e.g. line-buffer overflow) as `Failed`★ — outside a turn the table holds it and folds **the next clean turn** into an error end, stopping that incarnation's mail until the user makes
///   the next turn succeed (no cap). Prefix, not a field: `Error` is a single string and a marker field would change every consumer's match (builder and reader both live in this folder).
/// ★No correlation key (known scope)★: claude's ends (`MessageDone` · interrupted `TurnEnd`) have None in the id fields, so no turn counting or fencing is done and **the last observation decides**. Whether a nested Task subagent's end line
///   leaks as the parent turn's end is unverified; if so the symptom is 「parent still in a turn but judged idle → early injection」 (timing only, no loss).
/// ★Shared with terminal mode★: no decoder there, only `TerminalBytes`, so this mapping yields no signal.
/// ★Registry events: only `Delivered` is progress★ — the 1:1 translation of the vendor `command_lifecycle` `started` line (「the vendor started running this input in the current turn」). The rest are `None`: an event **after** the turn end such
///   as `Dropped` turning 「in a turn」 back on would block that incarnation's mail until the 30-minute valve. Events the core rewrote (seal · after the ack-unavailable decision) do not pass this classifier.
/// ★`ToolResult` is neither progress nor error★ — not emitted yet, same rule as codex: an end can come after the turn end (progress would re-open 「in a turn」), and a tool failure is not a turn failure (an error would set the stop after error).
// ADR-0113
// ADR-0004
// ADR-0231
// ADR-0243
// ADR-0241
pub(crate) fn classify_turn(event: &OutputEvent) -> Option<TurnSignal> {
    match event {
        OutputEvent::Structured { kind, .. } if kind == INTERRUPTED_KIND => None,
        OutputEvent::TextDelta { .. }
        | OutputEvent::ToolCall { .. }
        | OutputEvent::Structured { .. }
        | OutputEvent::QueuedInput(QueuedInputEvent::Delivered { .. }) => {
            Some(TurnSignal::Progress)
        }
        OutputEvent::Error(detail) if detail.starts_with(RESULT_FAILURE_DETAIL) => {
            Some(TurnSignal::Failed)
        }
        // The translation of a `result` line that was not interrupted — a failed `result` also closes here (that failure is carried by the `Error` right before it).
        OutputEvent::MessageDone { .. } => Some(TurnSignal::Ended(TurnEndKind::Clean)),
        // The `TurnEnd` this decoder emits is only the `Interrupted` of an interrupted `result` (ADR-0238). ★Other outcomes are also recorded as an
        //   end★ — splitting the two end vocabularies here would make an end silently disappear the day one of them flows in.
        OutputEvent::TurnEnd { outcome, .. } => Some(TurnSignal::Ended(match outcome {
            TurnOutcome::Completed => TurnEndKind::Clean,
            TurnOutcome::Failed { .. } => TurnEndKind::Failed,
            TurnOutcome::Interrupted | TurnOutcome::Unknown => TurnEndKind::Other,
        })),
        OutputEvent::Usage { .. }
        | OutputEvent::ToolResult { .. }
        | OutputEvent::Error(_)
        | OutputEvent::TerminalBytes(_)
        | OutputEvent::QueuedInput(_) => None,
    }
}

/// ADR-0094: abstract `ToolGrant` list → list of claude `--allowedTools` pattern strings.
///
/// ★MCP pattern★: `mcp__<server>__<tool>` is claude's MCP tool naming convention.
/// ★CLI pattern syntax★: the **colon-star** of `Bash(<X>:*)` is the documented **prefix wildcard** of the Claude Code permission system — it allows only commands **starting with** that string. exe is the bare command name; the priming,
///   this grant pattern and the actual invocation all align on it.
///   ★Range of the prefix (accepted, decided not to narrow)★: the executable is `engram`, so `Bash(engram:*)` also covers siblings **starting with** it — `engram-dashboard` · `engram-dashboard-daemon` in the release folder (ADR-0100
///     co-location), also reachable by bare name via the PATH prepend below. **Whether the matcher looks at token boundaries is unverified**; narrowing needs that measured first.
///   ★Why it is no risk now — and when to look again★: spawns always run under `--permission-mode bypassPermissions` (ADR-0097), so the grant is **a policy surface, not a runtime gate**. That auto mode is a temporary regime (successor: the
///     "constraint layer shared by all LLMs" in the `ToolGrant` comment), so **when that flag is removed, review this prefix range first** — the breadth then becomes the permission itself.
///   ★Unconfirmed item on that "always" (ADR-0132 §Impact)★: the profile's `extra_args` are appended **after** our flags; if the user puts `--permission-mode` there, **whether the later value wins is unverified** — if so, that spawn alone
///     is not in auto and the broad prefix becomes an **actual permission**. The acceptance above is conditional on this.
///   ★Windows PowerShell tool (FIX-4)★: Claude Code on Windows exposes a separate **PowerShell tool** and agents sometimes use it (measured), so one `Cli{exe}` yields two patterns — two shell shapes of the same send entry; a harmless no-op
///     where the tool is absent.
///   ★The old `Bash(<abs> *)` (space-star + absolute path) is discarded★: permission-blocked 0/38 live (unmatched syntax, deployment-unfriendly); replaced with colon-star + bare name + injected PATH.
/// ★Pattern order (Cli)★: Bash first, PowerShell next — build_spec appends them in this order (test anchor).
// ADR-0098
pub(crate) fn grants_to_allowed_tools(grants: &[ToolGrant]) -> Vec<String> {
    let mut out = Vec::with_capacity(grants.len());
    for g in grants {
        match g {
            ToolGrant::Mcp { server, tool } => out.push(format!("mcp__{server}__{tool}")),
            ToolGrant::Cli { exe } => {
                out.push(format!("Bash({exe}:*)"));
                out.push(format!("PowerShell({exe}:*)"));
            }
        }
    }
    out
}

/// Builds one user-turn line (terminator `\n`) for claude stream-json stdin (ADR-0044 §4).
///
/// ★1 call = 1 complete user turn (FIX 6a)★: the whole `text` becomes one user-turn line — partial text would make the fragment a turn and break the conversation. The caller passes **the whole completed message**.
/// ★uuid dedup contract (the official VS Code extension's way)★: with the `uuid` we plant at top level, the user line claude echoes with `--replay-user-messages` **keeps this uuid** and carries `"isReplay":true` (measured). So it **must
///   equal** the uuid the same write_input used for the synthetic echo (user_text_echo_json) — the caller generates it once and passes it to both.
/// ★Exact escaping★: serde_json handles quotes, newlines and Unicode — no hand assembly (an unescaped `"` breaks the stdin JSON parser). claude parses stdin line by line, so end with `\n`.
/// ★Key order★: `serde_json::json!` (BTreeMap) reorders keys alphabetically; claude accepts any order, but to expose the schema as specified (`{"type":"user","message":{"role":"user","content":[…]},"uuid":…}`) it is a **typed struct**
///   (serde writes fields in declaration order).
pub(crate) fn wrap_user_turn(text: &str, uuid: Uuid) -> Vec<u8> {
    #[derive(serde::Serialize)]
    struct UserTurn<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        message: UserMessage<'a>,
        uuid: String,
    }
    #[derive(serde::Serialize)]
    struct UserMessage<'a> {
        role: &'static str,
        content: [ContentBlock<'a>; 1],
    }
    #[derive(serde::Serialize)]
    struct ContentBlock<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        text: &'a str,
    }

    let turn = UserTurn {
        kind: "user",
        message: UserMessage {
            role: "user",
            content: [ContentBlock { kind: "text", text }],
        },
        uuid: uuid.to_string(),
    };
    // to_string does not fail for this shape — unwrap_or_default defensively.
    let mut line = serde_json::to_string(&turn).unwrap_or_default();
    line.push('\n');
    line.into_bytes()
}

/// Builds the json payload of the input-time user echo `Structured{kind:"user"}` (ADR-0044/0045).
///
/// ★Built at input time★: in json mode input is not echoed locally like in a PTY — it appears only when claude echoes it back with `--replay-user-messages` (round-trip delay), so right after write_input succeeds the session layer emits
///   a **synthetic user event** imitating the terminal's immediate echo.
/// ★uuid dedup shape contract (load-bearing)★: this json must have **the same shape** as what the decoder builds for a replayed user text block (`{"type":"text","text":<raw>,"uuid":"X"}`) — the frontend accumulator dedups user items by
///   `uuid`, so a diverging shape or uuid leaves the synthetic and replay echoes as two.
pub(crate) fn user_text_echo_json(text: &str, uuid: Uuid) -> String {
    #[derive(serde::Serialize)]
    struct TextBlock<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        text: &'a str,
        uuid: String,
    }
    let block = TextBlock {
        kind: "text",
        text,
        uuid: uuid.to_string(),
    };
    // to_string does not fail for this shape — unwrap_or_default defensively.
    serde_json::to_string(&block).unwrap_or_default()
}

/// Prefix of the cancel request `request_id` — the uuid of that input follows. ★Carrying the uuid in the request id keeps the decoder stateless★: only the request id comes back in the response envelope, so the decoder takes the uuid off
/// there without a table of sent requests and emits the registry event ([`cancel_response_event`]).
// ADR-0231
const CANCEL_REQUEST_PREFIX: &str = "cancel:";

/// The stdin line (newline included) requesting cancellation of one queued input `id` — of [`MidTurnPolicy::SessionClassified`] `cancel_line`.
///
/// ★Vendor contract (measured M3 — claude 2.1.280)★: `control_request` `cancel_async_message` is accepted in our `-p` stream-json mode without `initialize`; the response is a `control_response` with the value nested in
///   `response.response.cancelled`. Key order = typed struct declaration order, as in [`wrap_user_turn`].
// ADR-0231
fn cancel_line(id: &str) -> Vec<u8> {
    #[derive(serde::Serialize)]
    struct ControlRequest<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        request_id: String,
        request: CancelAsyncMessage<'a>,
    }
    #[derive(serde::Serialize)]
    struct CancelAsyncMessage<'a> {
        subtype: &'static str,
        message_uuid: &'a str,
    }

    let request = ControlRequest {
        kind: "control_request",
        request_id: format!("{CANCEL_REQUEST_PREFIX}{id}"),
        request: CancelAsyncMessage {
            subtype: "cancel_async_message",
            message_uuid: id,
        },
    };
    // to_string does not fail for this shape — unwrap_or_default defensively.
    let mut line = serde_json::to_string(&request).unwrap_or_default();
    line.push('\n');
    line.into_bytes()
}

// ── ADR-0238: interrupt — the turn-open gate + control line ──────────────────────────────────────────

/// The slot receiving this incarnation's turn-open gate ([`GateCell`]) — the live decoder opens and closes the gate through it, and the interrupt line function ([`interrupt_line`]) holds the same gate directly.
///
/// ★Why a slot, not held directly★: the gate contains the leftover cleaner, which must know the root process and group the link launched, so it can only stand **after** the link is opened, while the decoder is handed to the link **before**
///   (`StdioTransport::open` argument). The pump runs only at `start`, so the slot is filled before the first line; if empty, the decoder does not touch the gate.
/// ★「In a turn」 is drawn from the same classifier as turn observation ([`classify_turn`])★ — a progress signal among one live line's events opens it, the end signal (the `result` line, the only line that ends) closes it. New input during
///   progress (live `started`'s translation `Delivered`) is announced separately: it drops that turn's interrupt episode and, if closed, opens in one section. Resume original text does not pass this gate. Measured (B2 · claude 2.1.280):
///   the opening line was always that turn's `command_lifecycle` `started`, and no line opened it outside a turn.
/// ★Not the same value as turn observation — its front end is late★: the session's input-time synthetic echo (`input_echo_event`) turns observation on without passing the decoder, so the gate opens only at the vendor `started`. An interrupt in
///   that gap is a refused no-op (TRD §9 「F2 interrupt before the turn opens」).
/// ★It inherits the classifier's 「no `result`」 hole and has no 30-minute fail-open cap★: if a `result` line is dropped by the 4 MiB resync or unreadable, or the vendor ends the turn with progress only, the gate stays open; the interrupt line then
///   reaches an idle CLI, which (B2 S6) answers one harmless response line — or interrupts input the CLI has queued at that moment. The episode that interrupt opened is not erased by a turn end; N seconds after the write the cleanup pass runs
///   (nothing is ended if no leftover satisfies the selection rules).
/// ★The vendor does not refuse an interrupt while idle (B2 S6 — always `success`)★ — only this gate can say 「no turn to interrupt」; with the gate closed the line is not sent and the link refuses with `Unsupported`.
/// ★A narrowed-not-closed race remains★: if, between the gate read and the line reaching the CLI, that turn ends and the CLI opens the next turn (queued input) itself, the line interrupts that next turn; `control_request` has no turn id
///   (B2 S7 — the interrupted next turn closes as an aborted turn, not an error).
/// ★The session, link and wire types do not know this value★ — it lives in this folder, not `types.rs` ([`DeliveryAck`] is read by the session, so it is there).
// ADR-0238
// ADR-0262
type GateSlot = Arc<OnceLock<Arc<GateCell>>>;

/// This incarnation's leftover cleaner. `None` = only opens and closes the gate — the link hands out no process group (outside Windows) or there is no root PID. The port and listening thread are not started here; the first interrupt episode starts
/// them. Log attribution is the control channel's agent id, else the root PID (ADR-0217).
// ADR-0262
fn leftover_cleaner(
    transport: &StdioTransport,
    root_pid: Option<u32>,
    agent: Option<AgentId>,
) -> Option<Arc<Cleaner>> {
    let group = transport.process_group()?;
    let root_pid = root_pid.filter(|&pid| pid != 0)?;
    Some(Arc::new(Cleaner::new(
        Arc::new(group),
        Arc::new(SystemClock),
        root_pid,
        LogTag::new(agent, root_pid),
    )))
}

/// Prefix of the interrupt request `request_id` — a new uuid v4 follows each request.
///
/// ★The response is not translated★ — a response does not mean the turn stopped (the end is that turn's `result`; B2: the response always precedes it). [`cancel_response_event`] looks only at [`CANCEL_REQUEST_PREFIX`], so a response with this prefix has no event.
// ADR-0238
const INTERRUPT_REQUEST_PREFIX: &str = "interrupt:";

/// The stdin line (newline included) asking to interrupt one running turn.
///
/// ★Vendor contract (B2 — claude 2.1.280)★: accepted in our `-p` stream-json mode without `initialize`; a running Bash tool dies and its `tool_result` comes with `is_error:true`; the response's `still_queued` = queued messages that survive and run in
///   the next turn. ★Optional fields like `cancel_queued` are not carried★ — accepted queued input is sent to the next turn, not discarded (ADR-0235 decisions 4 · 5). Key order = typed struct declaration order, as in [`wrap_user_turn`].
// ADR-0238
fn interrupt_line_bytes(request_uuid: Uuid) -> Vec<u8> {
    #[derive(serde::Serialize)]
    struct ControlRequest {
        #[serde(rename = "type")]
        kind: &'static str,
        request_id: String,
        request: Interrupt,
    }
    #[derive(serde::Serialize)]
    struct Interrupt {
        subtype: &'static str,
    }

    let request = ControlRequest {
        kind: "control_request",
        request_id: format!("{INTERRUPT_REQUEST_PREFIX}{request_uuid}"),
        request: Interrupt {
            subtype: "interrupt",
        },
    };
    // to_string does not fail for this shape — unwrap_or_default defensively.
    let mut line = serde_json::to_string(&request).unwrap_or_default();
    line.push('\n');
    line.into_bytes()
}

/// The interrupt line function plugged into the link — gives a line only while the gate is open (`None` → link `Unsupported` → bus CONFLICT). The gate decides whether to give the line and whether to carry a write-confirmation callback: an interrupt that
/// sets up or continues an episode is recorded in the gate **before** the line is returned; one that came while another interrupt is opening an episode, or whose turn changed while opening, goes out as a line only (no episode, no write confirmation).
// ADR-0238
// ADR-0262
fn interrupt_line(gate: Arc<GateCell>) -> InterruptLine {
    Arc::new(move || gate.interrupt(interrupt_line_bytes(Uuid::new_v4())))
}

// ── S15 B2: claude stream-json(NDJSON) → OutputEvent decoder (ADR-0044/0045) ────────
//
// Schema grounds = measured fixtures `backend/fixtures/claude_{text,tool}.jsonl`.

/// ★Cap on the unterminated line buffer★: with malformed/runaway output where a newline never comes the buffer would grow without bound (OOM). The link is a dumb pipe (ADR-0044), so the decoder defends — past 4MB it discards the partial line and
/// recovers from the next newline. A normal NDJSON line never exceeds 4MB (thinking/text blocks are far smaller), so exceeding it is abnormal.
const MAX_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// Maximum bytes of a message id — same value and rule as the cap of the same name in the codex translator.
///
/// ★**Filtered**, not truncated★: the id is compared for equality (dropping the complete text of a streamed block is that comparison), so truncating would read two different long ids as the same. A filtered message is not streamed and shows as one
/// complete text; without a cap one long id is duplicated for every streamed delta. The observed id is `msg_` + 26 characters.
// ADR-0240
const MAX_MESSAGE_ID_BYTES: usize = 128;

fn bounded_message_id(id: Option<&serde_json::Value>) -> Option<String> {
    id.and_then(|v| v.as_str())
        .filter(|s| s.len() <= MAX_MESSAGE_ID_BYTES)
        .map(String::from)
}

/// Prefix of the [`OutputEvent::Error`] emitted by a failed `result` — ★the turn classifier tells 「turn error」 apart by this prefix★ ([`classify_turn`]); another `Error` text (e.g. line-buffer overflow) starting with it would fold the turn into a failure.
// ADR-0231
const RESULT_FAILURE_DETAIL: &str = "claude stream-json result reported failure";

/// The ending reason (`terminal_reason`) an interrupted turn's `result` carries — while streaming text, thinking or tool input | while running a tool.
///
/// ★An interruption cannot be told apart by `subtype`·`is_error` (B2 — claude 2.1.280 · interrupted 14/14)★: it comes as `subtype:"error_during_execution"` · `is_error:true`, the same words as a real execution error; reading it as an error would
///   draw an error row on one Esc and set the stop after error, stopping mail. Normal turns were `"completed"`.
// ADR-0238
const INTERRUPTED_TERMINAL_REASONS: [&str; 2] = ["aborted_streaming", "aborted_tools"];

/// Whether it is an interrupted turn's `result` — [`INTERRUPTED_TERMINAL_REASONS`] or `subtype:"interrupted"` (not observed on this installation; kept for older CLIs).
// ADR-0238
fn is_interrupted_result(value: &serde_json::Value) -> bool {
    value.get("subtype").and_then(|v| v.as_str()) == Some("interrupted")
        || value
            .get("terminal_reason")
            .and_then(|v| v.as_str())
            .is_some_and(|r| INTERRUPTED_TERMINAL_REASONS.contains(&r))
}

/// The `Structured` kind of the interrupted-marker event — the frontend accumulator moves this word into an interrupted-marker row instead of a user bubble. `json` = `{"text": <vendor original>}`.
// ADR-0243
const INTERRUPTED_KIND: &str = "interrupted";

/// The head of the user-line text the CLI inserts before the `result` of every interrupted turn — followed by ` for tool use]` if a tool was running, else `]` (B2 — claude 2.1.280 · 14/14 · also kept in the transcript).
///
/// ★Vendor string matching★ — if the text changes, that line goes back to being a user bubble (visible, nothing lost). ★Do not recognize it by position (right before `result`)★ — a resumed transcript has no `result`.
// ADR-0243
const INTERRUPT_NOTE_PREFIX: &str = "[Request interrupted by user";

/// If one `user` line is an interrupt synthetic line, its original text — only when `message.content` is exactly one text block starting with [`INTERRUPT_NOTE_PREFIX`]; `None` for two or more blocks or a non-text block (`tool_result` etc.).
///
/// ★`isReplay:true` lines are excluded★ — user-typed text that happens to start with that head is echoed back carrying our uuid (real synthetic lines have no `isReplay` — B2), and turning it into an interrupted marker would bypass the
///   frontend's queued-row matching and uuid dedup and lose the progress signal. A plain-string `content` is not recognized (never observed; such `user` lines are not translated).
// ADR-0243
fn interrupt_note_text(value: &serde_json::Value) -> Option<&str> {
    if value.get("isReplay").and_then(|v| v.as_bool()) == Some(true) {
        return None;
    }
    let [block] = value.get("message")?.get("content")?.as_array()?.as_slice() else {
        return None;
    };
    if block.get("type").and_then(|t| t.as_str()) != Some("text") {
        return None;
    }
    block
        .get("text")?
        .as_str()
        .filter(|text| text.starts_with(INTERRUPT_NOTE_PREFIX))
}

/// The word in `system/init`'s `capabilities` meaning 「emits a per-entry lifecycle」 (measured M4 — claude 2.1.280).
// ADR-0231
const MSG_LIFECYCLE_CAPABILITY: &str = "msg_lifecycle_v1";

/// Which stream a line came from — live stdout and the resume transcript have the same envelope but carry different kinds of lines.
#[derive(Clone, Copy)]
enum LineSource<'a> {
    /// Live stdout — fills in ack availability into this value, and moves lifecycle and cancel responses into registry events.
    Live(&'a DeliveryAck),
    /// Resume transcript seed — ★emits no registry events★: the seed does not pass through the registry door (the core
    ///   rejects a `QueuedInput` coming in through `seed`). Instead, it moves the attachment lines of folded-in inputs into bubbles.
    Transcript,
}

/// Decoder for production spawns — fills the incarnation-shared ack value and opens and closes the turn-open gate.
fn stream_decoder(ack: Arc<DeliveryAck>, gate: GateSlot) -> Box<dyn OutputDecoder> {
    Box::new(ClaudeStreamDecoder::live(ack, gate))
}

/// Live partial message tracking — filled by `stream_event`, read by the complete `assistant` line.
///
/// ★Takes 「message id, block number」 as the block open now★: the complete line has no block number. The vendor contract is 「one complete message per non-empty block, before that block's `content_block_stop`」 (measured + official docs), so at a complete
///   line the open block is that line's block. If the vendor changes the order, `open` is empty and the complete text is emitted as is — erring toward overlap rather than loss.
// ADR-0240
#[derive(Debug, Default)]
struct PartialMessage {
    /// The id of the current message (`message_start`). `None` = no message being tracked — text deltas arriving then are discarded.
    id: Option<String>,
    /// Block number started and not yet stopped (`content_block_start` ~ `content_block_stop`).
    open: Option<u64>,
    /// Block numbers that streamed at least one text delta.
    streamed: BTreeSet<u64>,
}

impl PartialMessage {
    /// Whether the text of this complete `assistant` line was already streamed as deltas — if true, that line's `text` blocks are not emitted.
    fn streamed_the_open_block_of(&self, message_id: Option<&str>) -> bool {
        self.id.is_some()
            && self.id.as_deref() == message_id
            && self.open.is_some_and(|k| self.streamed.contains(&k))
    }
}

/// claude stream-json live decoder.
///
/// ★Own state = line reassembly + partial message tracking (`partial`) + usage observations the pump drains per chunk (`usage`)★. Partial tracking only reduces the vendor sending the same text twice (deltas and a complete line) to one copy —
///   ★the frontend accumulator has no dedup★ (it appends text deltas to the last text item); other merging (joining blocks of one message.id) stays the frontend's job. It does not know list entries: a registry event is a 1:1 translation of one
///   vendor line, interpreted by the registry and accumulator reduction rules.
/// ★`ack` and the turn-open gate (`gate`) are handles to incarnation-shared values, not decoder state★ — the session reads the ack value and the interrupt line function the gate.
// ADR-0240
// ADR-0238
#[derive(Debug, Default)]
pub struct ClaudeStreamDecoder {
    /// Incomplete line bytes after the last `\n` (for line-level split reassembly).
    ///
    /// ★Invariant (load-bearing)★: **never UTF-8 decode before a complete line (up to the newline) is settled.** The pump throws arbitrary byte chunks (≤4096B) ignoring line and character boundaries, so multibyte characters (Hangul · emoji) can be
    ///   cut at chunk boundaries; joining as bytes and decoding only `\n`-terminated lines absorbs that (`0x0A` cannot be a UTF-8 continuation byte, so byte-level line search is safe).
    buffer: Vec<u8>,

    /// ★Overflow resync state (FIX-A)★: whether it is **discarding the remaining tail of an overflowed polluted line up to the next `\n`** — incoming bytes are discarded until a newline, which releases it to false.
    ///
    /// ★Why★: a plain `buffer.clear()` would parse the polluted line's tail (bytes not yet arrived and bytes appended after the clear) as a "new line" up to the next `\n` and could emit **fake events** (if the tail contains a valid JSON fragment). To
    ///   lose only the one polluted line and recover from after its `\n`, a "discarding" state is needed.
    discarding: bool,

    /// Usage observations picked up from `rate_limit_event` — the pump drains them per chunk with `take_usage`.
    usage: Vec<UsageObservation>,

    /// Whether this incarnation can give ack notices — leaves `Unknown` through the capability list of `system/init` and the first `command_lifecycle` line. In production it is **the very** Arc the backend puts in `SpawnParts::delivery_ack`.
    // ADR-0231
    ack: Arc<DeliveryAck>,

    /// Used only for live lines — the resume transcript is translated without this field (`partial: None` of `consume_line`).
    // ADR-0240
    partial: PartialMessage,

    /// Opened and closed only by live lines ([`Self::consume_live_line`]) — the resume transcript has no way to reach this field.
    // ADR-0238
    gate: GateSlot,

    /// The openness this decoder last announced to the gate. This decoder is the only one writing the gate's openness, so it equals the gate's value — so
    /// it does not take the gate's lock on every progress event, only on closed → open.
    gate_open: bool,

    /// What was announced to the gate — for tests measuring 「what is announced on which event, how many times」.
    #[cfg(test)]
    gate_moves: Vec<GateMove>,

    /// The containment marker while announcing to the gate (ADR-0292) — for tests.
    #[cfg(test)]
    gate_marks: Vec<bool>,
}

/// What the decoder announces to the gate — progress (closed → open) · new input (live `started`) · turn end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateMove {
    Open,
    Deliver,
    Close,
}

impl ClaudeStreamDecoder {
    /// A decoder that fills an ack value only it holds and whose gate slot is empty — for assemblies (tests · smoke) where nobody reads the decision.
    pub fn new() -> Self {
        Self::default()
    }

    /// A decoder that fills the `ack` the test rig holds — the gate slot is empty (production assembly = [`Self::live`]). ★The decoder that wins the `Unknown → Unavailable` transition emits `AckUnavailable{delivered: []}` once★ (once even if several
    /// decoders hold the same Arc).
    // ADR-0231
    #[cfg(test)]
    fn with_delivery_ack(ack: Arc<DeliveryAck>) -> Self {
        Self {
            ack,
            ..Self::default()
        }
    }

    /// Production assembly — `ack` is read by the session, and the gate to be filled into `gate` by the interrupt line function, through the same Arc.
    // ADR-0238
    fn live(ack: Arc<DeliveryAck>, gate: GateSlot) -> Self {
        Self {
            ack,
            gate,
            ..Self::default()
        }
    }

    /// Pushes in a byte chunk and parses only the lines **completed** by this chunk, returning events.
    /// The tail (an incomplete line without a newline) is left in the buffer to join with the next chunk.
    pub fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
        let mut events = Vec::new();

        // ★resync (FIX-A)★: if it is discarding the tail of a polluted line, everything before the next `\n` is discarded. If there is no
        //   newline, the whole chunk is still part of that line — return without accumulating in the buffer.
        let chunk = if self.discarding {
            match chunk.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    self.discarding = false;
                    &chunk[nl + 1..]
                }
                None => return events,
            }
        } else {
            chunk
        };

        self.buffer.extend_from_slice(chunk);

        // The remainder after the last newline stays in buffer as the tail to join with the next chunk (FIX-D: comment matched to the actual code).
        while let Some(nl) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=nl).collect();
            self.consume_live_line(&line[..line.len() - 1], &mut events);
        }

        // ★Entering resync, not a plain clear (FIX-A)★: emptying only the buffer would have the rest of this polluted line's tail
        //   parsed as a new line up to the next `\n`, emitting fake events. It loses only the 1 polluted line and recovers.
        if self.buffer.len() > MAX_BUFFER_BYTES {
            let dropped = self.buffer.len();
            self.buffer.clear();
            self.discarding = true;
            events.push(OutputEvent::Error(format!(
                "claude stream-json decoder: partial-line buffer overflow — dropping {dropped} bytes (no line terminator); resyncing to next newline"
            )));
        }
        events
    }

    /// Called at EOF — processes the last line not terminated by a newline. On a normal end the buffer is empty, so 0 events.
    // ★Invariant★: when discarding=true the buffer is always empty (clear on overflow + not loaded while discarding), so a flush then yields 0 events — do not handle a discarding remainder in flush (adding logic breaks this invariant).
    pub fn flush(&mut self) -> Vec<OutputEvent> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.consume_live_line(&line, &mut events);
        }
        events
    }

    /// Translates one live line and runs the events it emitted through the turn classifier in order, announcing to the turn-open gate — progress = open (only when closed) · new input (`Delivered` = live `started`) = drop that turn's interrupt episode and
    /// open · end = close and drop the episode. Turn errors (`Failed`) and signal-less events do not touch the gate. ★Announcing happens before the event is emitted from the pump★ (emitted only after decode returns).
    // ADR-0238
    // ADR-0262
    fn consume_live_line(&mut self, line: &[u8], events: &mut Vec<OutputEvent>) {
        let first = events.len();
        Self::consume_line(
            line,
            events,
            LineSource::Live(&self.ack),
            Some(&mut self.partial),
            Some(&mut self.usage),
        );
        // If this line did not emit a new event but was appended to the previous text event (the merging of `consume_stream_event`), it is not here — that
        //   previous event already opened the gate as progress and no closing event came after, so the gate stays open.
        for event in &events[first..] {
            // New input is progress in the classifier but announced separately — even the `started` that opens a turn does not split 「open」 and 「drop」
            //   into two announcements (one section of the gate lock).
            if matches!(
                event,
                OutputEvent::QueuedInput(QueuedInputEvent::Delivered { .. })
            ) {
                self.move_gate(GateMove::Deliver);
                continue;
            }
            match classify_turn(event) {
                Some(TurnSignal::Progress) if !self.gate_open => self.move_gate(GateMove::Open),
                Some(TurnSignal::Ended(_)) => self.move_gate(GateMove::Close),
                Some(TurnSignal::Progress | TurnSignal::Failed) | None => {}
            }
        }
    }

    /// Announces one thing to the gate (one section of the gate lock). The pump calls decode without a lock, so no other lock has an edge into the gate lock — that the gate lock is a leaf (takes nothing while held) is kept by [`GateCell`]. An empty slot does nothing.
    // ADR-0262
    fn move_gate(&mut self, to: GateMove) {
        let Some(gate) = self.gate.get() else {
            return;
        };
        // ADR-0292 decision 3: the gate is state shared with the leftover cleaner and its guard relies on abort (ADR-0262 K7) — even when called inside the pump's
        //   containment, the marker is turned off.
        uncontained(|| {
            match to {
                GateMove::Open => gate.open_turn(),
                GateMove::Deliver => gate.deliver(),
                GateMove::Close => gate.close_turn(),
            }
            #[cfg(test)]
            self.gate_marks
                .push(engram_dashboard_base::panic::is_contained());
        });
        self.gate_open = to != GateMove::Close;
        #[cfg(test)]
        self.gate_moves.push(to);
    }

    /// One complete line (bytes excluding the newline) → append 0 or more OutputEvents to events.
    ///
    /// Parse rules — failures and meta are silently skipped (no panic):
    /// - non-UTF8 / non-JSON (e.g. stderr "Warning: no stdin…") → skip.
    /// - `assistant`/`user` lines → each block of message.content[] as events in order; an interrupt synthetic `user` line becomes one `Structured{kind:"interrupted"}` (ADR-0243 — [`interrupt_note_text`]).
    /// - `result` line → MessageDone (+ Usage if result.usage exists; if is_error/subtype is of the error family, Error is also emitted **before** MessageDone — FIX-C). ※ Surfacing result errors is a **new backend policy**
    ///   (the frontend parser had none). An interrupted turn's result is one `TurnEnd{Interrupted}` after Usage — no Error or MessageDone (ADR-0238).
    /// - Live only: `command_lifecycle` → registry events · `control_response` of a `cancel:<uuid>` request → cancel response event · `system/init` → ack availability decision (ADR-0231).
    /// - Transcript only: `attachment{queued_command}` → user bubble.
    /// - `stream_event` → only when `partial` exists (live): streams text deltas as `TextDelta` and does not emit the complete `assistant` text of a streamed block. Without `partial` (transcript) skipped entirely — the complete line emits the full text (ADR-0240).
    /// - `rate_limit_event` → 0 events; if `observations` is `Some` a usage observation is added there (`usage.rs`), `None` = not picked up (reading past records).
    /// - Other `system`/`queue-operation`/unknown types → skip (0).
    // ADR-0240
    fn consume_line(
        line: &[u8],
        events: &mut Vec<OutputEvent>,
        source: LineSource<'_>,
        partial: Option<&mut PartialMessage>,
        observations: Option<&mut Vec<UsageObservation>>,
    ) {
        // ★The first UTF-8 decoding is here★ (the buffer invariant above). Not lossy but strict validation, skipping on failure —
        //   a non-UTF8 line is not normal claude output (this is not the terminal path).
        let text = match std::str::from_utf8(line) {
            Ok(t) => t.trim(), // strip surrounding whitespace and CR (\r, for CRLF)
            Err(_) => return,  // non-UTF8 → skip
        };
        if text.is_empty() {
            return; // a chunk with only an empty line or newline
        }

        let value: serde_json::Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => return, // non-JSON (stderr warnings etc.) → skip
        };

        match (value.get("type").and_then(|t| t.as_str()), source) {
            (Some(role @ ("assistant" | "user")), _) => {
                let msg = match value.get("message") {
                    Some(m) => m,
                    None => return,
                };
                // An interrupt synthetic line is an interrupted marker, not a user bubble — both live and resume.
                // ADR-0243
                if role == "user" {
                    if let Some(text) = interrupt_note_text(&value) {
                        events.push(OutputEvent::Structured {
                            kind: INTERRUPTED_KIND.to_string(),
                            json: serde_json::json!({ "text": text }).to_string(),
                        });
                        return;
                    }
                }
                let message_id = bounded_message_id(msg.get("id"));
                // ★user replay dedup key★: it is line-level, so it is extracted once outside the block loop. assistant
                //   lines do not have this concept, so it becomes None (the assistant arm of consume_block does not use it).
                let line_uuid = value.get("uuid").and_then(|v| v.as_str());
                let blocks = match msg.get("content").and_then(|c| c.as_array()) {
                    Some(arr) => arr,
                    None => return, // skip if content is not an array (schema deviation)
                };
                // ★The complete text of a streamed block is dropped — without this every claude answer appears twice★ (the frontend only appends). Only text blocks: tools and thinking are not streamed as deltas, so the complete line is their only source.
                // ADR-0240
                let already_streamed = role == "assistant"
                    && partial
                        .as_deref()
                        .is_some_and(|p| p.streamed_the_open_block_of(message_id.as_deref()));
                for block in blocks {
                    if already_streamed
                        && block.get("type").and_then(|t| t.as_str()) == Some("text")
                    {
                        continue;
                    }
                    Self::consume_block(role, block, message_id.as_deref(), line_uuid, events);
                }
            }
            (Some("result"), _) => {
                // ★Emit Usage before MessageDone★: the consumer receives the turn's final token totals before the "turn end" signal (after it they would look like a delayed arrival). result.usage.{input_tokens,output_tokens} confirmed in the measured fixture
                //   (text.jsonl line 5: input=17095, output=4).
                if let Some(usage) = value.get("usage") {
                    let input_tokens = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let output_tokens = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    // 0/0 is not meaningful usage → skip (prevents meaningless 0-token noise).
                    if input_tokens != 0 || output_tokens != 0 {
                        events.push(OutputEvent::Usage {
                            input_tokens,
                            output_tokens,
                            // stream-json lines have no turn concept of our domain (session_id is separate) → None.
                            turn_id: None,
                        });
                    }
                }
                // ★An interrupted turn is not an error — split off before the error decision★: its `result` comes with `is_error:true` and an error-family subtype (B2), so the decision below would make it a failed turn. An interruption closes only through the one
                //   `TurnEnd` branch (the classifier reads `Ended(Other)`, neither setting nor clearing the stop after error); other ends are `MessageDone` as before.
                // ADR-0238
                if is_interrupted_result(&value) {
                    events.push(OutputEvent::TurnEnd {
                        turn_id: None,
                        outcome: TurnOutcome::Interrupted,
                    });
                } else {
                    // ★Surfacing failed turns (FIX-C)★: MessageDone alone would disguise a turn failed by API error, max-turns or refusal as "completed normally". A real failure's payload is not captured (`result_error_handbuilt` was hand-built), so only
                    //   existing fields are stringified. Order Error → MessageDone (the consumer learns of the error before the end signal).
                    let is_error = value
                        .get("is_error")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let subtype = value.get("subtype").and_then(|v| v.as_str());
                    // ★error allowlist (not a denylist)★: an error only when the subtype is of the error family (measured error_max_turns · error_during_execution → `s.starts_with("error")`). Do not revert to a denylist (`s != "success"`) — future non-error
                    //   subtypes would turn into errors (the old shape read an Esc-aborted turn's `subtype:"interrupted"` as an error; the interrupt branch above now takes it first).
                    let subtype_is_error = subtype.map(|s| s.starts_with("error")).unwrap_or(false);
                    if is_error || subtype_is_error {
                        let mut detail = String::from(RESULT_FAILURE_DETAIL);
                        if let Some(s) = subtype {
                            detail.push_str(&format!(" (subtype={s})"));
                        }
                        if let Some(r) = value.get("result").and_then(|v| v.as_str()) {
                            detail.push_str(&format!(": {r}"));
                        }
                        events.push(OutputEvent::Error(detail));
                    }
                    events.push(OutputEvent::MessageDone {
                        turn_id: None,
                        message_id: None,
                    });
                }
                // ★Tracking is cleared at the turn end★ — late deltas without `message_start` are discarded, so a progress signal (`TextDelta`) after the turn end cannot turn 「in a turn」 back on (blocked until the 30-minute fail-open).
                // ADR-0240
                if let Some(partial) = partial {
                    *partial = PartialMessage::default();
                }
            }
            (Some("stream_event"), _) => {
                // The transcript (`partial: None`) is skipped — even if the record has this line, the complete line emits the full text.
                // ADR-0240
                if let Some(partial) = partial {
                    Self::consume_stream_event(&value, partial, events);
                }
            }
            (Some("rate_limit_event"), _) => {
                if let Some(observations) = observations {
                    observations.extend(usage::observation_from_rate_limit_event(&value));
                }
            }
            (Some("command_lifecycle"), LineSource::Live(ack)) => {
                // A recognized line means 「this CLI emits per-entry lifecycles」 — init comes only 0.6–0.9 s after the turn starts, the lifecycle within 1 ms after writing (M1·M4).
                // ★A line whose type matches but is not recognized does not open it (narrower than TRD §5-4 「Available on first sight」)★ — an internal surface: if the vendor changes keys or state words an opened registry row would never close. An init without the
                //   capability sends it to `Unavailable` and falls to today's path.
                // ADR-0231
                if is_recognisable_lifecycle(&value) {
                    ack.set_available();
                }
                if let Some(op) = lifecycle_event(&value) {
                    events.push(OutputEvent::QueuedInput(op));
                }
            }
            (Some("control_response"), LineSource::Live(_)) => {
                if let Some(op) = cancel_response_event(&value) {
                    events.push(OutputEvent::QueuedInput(op));
                }
            }
            (Some("system"), LineSource::Live(ack))
                if value.get("subtype").and_then(|s| s.as_str()) == Some("init") =>
            {
                if let Some(op) = judge_delivery_ack(ack, &value) {
                    events.push(OutputEvent::QueuedInput(op));
                }
            }
            (Some("attachment"), LineSource::Transcript) => {
                Self::consume_queued_command(&value, events);
            }
            // Other system (hook · thinking_tokens · task notices etc.) · queue-operation and other meta lines,
            //   lines whose stream does not match, unknown types → skip.
            _ => {}
        }
    }

    /// `attachment{queued_command}` of the resume transcript → user bubble (uuid = `source_uuid`).
    ///
    /// ★Input folded in mid-turn remains not as a `user` line but as this one line (M5 — claude 2.1.280)★ — skipping it would make the text disappear from the resumed conversation. The following `queue-operation` line has no body and is discarded; a cancelled
    ///   input leaves no body, so it is not revived.
    /// ★Only `commandMode: "prompt"` is moved★ — other modes are unexamined (may be CLI-inserted commands); only what is grounded as person-typed text becomes a bubble. ★Same bubble shape as a `user` line's text block★ (goes through `consume_block`'s user branch).
    // ADR-0231
    fn consume_queued_command(value: &serde_json::Value, events: &mut Vec<OutputEvent>) {
        let Some(attachment) = value.get("attachment") else {
            return;
        };
        let field = |key: &str| attachment.get(key).and_then(|v| v.as_str());
        if field("type") != Some("queued_command") || field("commandMode") != Some("prompt") {
            return;
        }
        let uuid = field("source_uuid");
        match attachment.get("prompt") {
            Some(serde_json::Value::Array(blocks)) => {
                for block in blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                {
                    Self::consume_block("user", block, None, uuid, events);
                }
            }
            Some(serde_json::Value::String(text)) => {
                let block = serde_json::json!({ "type": "text", "text": text });
                Self::consume_block("user", &block, None, uuid, events);
            }
            _ => {}
        }
    }

    /// One live `stream_event` line → 0–1 streamed text `TextDelta` + `partial` update.
    ///
    /// ★Emits nothing but text deltas★ — `message_stop` comes on every tool call, so an end from it would end the turn on every tool call, and auxiliary lines as `Structured` would be counted as progress by the classifier (a 30-minute block after the turn end).
    ///   The turn end is the one `result` line. Thinking and tool-input deltas are discarded too (emitted from the complete line).
    /// ★Lines with `parent_tool_use_id` are discarded entirely★ — a subagent's `message_start` would overwrite the parent's tracking and the parent's streamed block would come out once more as complete text (whether subagent deltas come at all is
    ///   unconfirmed — docs and peers disagree).
    /// ★Text deltas without `message_start` (no id) or without a block number are discarded★ — an unrecordable streamed position would leave two copies, so it shows as one complete text instead of streaming.
    // ADR-0240
    fn consume_stream_event(
        value: &serde_json::Value,
        partial: &mut PartialMessage,
        events: &mut Vec<OutputEvent>,
    ) {
        if value
            .get("parent_tool_use_id")
            .is_some_and(|p| !p.is_null())
        {
            return;
        }
        let Some(event) = value.get("event") else {
            return;
        };
        let index = event.get("index").and_then(|v| v.as_u64());
        match event.get("type").and_then(|t| t.as_str()) {
            Some("message_start") => {
                *partial = PartialMessage {
                    id: bounded_message_id(event.get("message").and_then(|m| m.get("id"))),
                    ..PartialMessage::default()
                };
            }
            Some("content_block_start") => {
                if index.is_some() {
                    partial.open = index;
                }
            }
            Some("content_block_delta") => {
                let Some(delta) = event.get("delta") else {
                    return;
                };
                if delta.get("type").and_then(|t| t.as_str()) != Some("text_delta") {
                    return;
                }
                let (Some(id), Some(index)) = (partial.id.as_ref(), index) else {
                    return;
                };
                let Some(text) = delta
                    .get("text")
                    .and_then(|v| v.as_str())
                    .filter(|t| !t.is_empty())
                else {
                    return;
                };
                partial.streamed.insert(index);
                // Ring pressure: if the event right before is text of the same message, append to it instead of a new event — `events` is the share of one decode(), so this merging reduces only when several lines came in one pump read (measured: the 519 delta
                //   lines of a 2,428-character answer came one line per read, so even merged it was 519). seq is assigned at emit so no holes appear; the accumulator appends to the last text item anyway.
                // ADR-0240
                if let Some(OutputEvent::TextDelta {
                    text: previous,
                    message_id: Some(previous_id),
                    ..
                }) = events.last_mut()
                {
                    if previous_id == id {
                        previous.push_str(text);
                        return;
                    }
                }
                events.push(OutputEvent::TextDelta {
                    text: text.to_string(),
                    turn_id: None,
                    message_id: Some(id.clone()),
                });
            }
            Some("content_block_stop") => {
                if index.is_some() && partial.open == index {
                    partial.open = None;
                }
            }
            _ => {}
        }
    }

    /// One content[] block → OutputEvent. `line_uuid`: the line-level `uuid` of a user line (replay dedup key), used only for user-role blocks.
    fn consume_block(
        role: &str,
        block: &serde_json::Value,
        message_id: Option<&str>,
        line_uuid: Option<&str>,
        events: &mut Vec<OutputEvent>,
    ) {
        // ★User line blocks are preserved whole as Structured{kind:"user"}★: OutputEvent has no role concept (only assistant-only fields), so a user replay turn has no formal variant → the original block goes through the escape hatch as is.
        if role == "user" {
            // ★No suppression (do not bring it back)★: user-role text blocks were once suppressed unconditionally (they duplicated the input-time synthetic echo), but with resume on that made all past user text disappear (even lines with no synthetic echo).
            //   They pass through carrying the uuid and dedup is left to the frontend.
            //   ★tool_result safe (HIGH FIX)★: a text echo and a tool_result can share one user line; with every block carrying the same line-level uuid, frontend dedup applies only to `type==="text"` (extractUserUuid returns null for non-text), so tool_result is always kept.
            //   Without a uuid (past lines · non-replay) the original is kept as is — not deduped.
            let json = match line_uuid {
                Some(u) => Self::user_block_with_uuid(block, u),
                None => block.to_string(),
            };
            events.push(OutputEvent::Structured {
                kind: "user".to_string(),
                json,
            });
            return;
        }

        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                // What comes here is complete text (blocks not streamed as deltas · transcript) — OutputEvent has no "complete text" variant, and TextDelta is the formal text increment.
                // ★malformed contract (FIX-B)★: without a string `text` (schema deviation) it skips instead of emitting an empty TextDelta — meaningless downstream noise, indistinguishable from a normal empty-string text block. (Skip, not Structured preservation:
                //   unlike a missing tool_use name, a missing text loses no matching information.)
                let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
                    return;
                };
                events.push(OutputEvent::TextDelta {
                    text: text.to_string(),
                    turn_id: None,
                    message_id: message_id.map(String::from),
                });
            }
            Some("tool_use") => {
                // id is tool_use.id — the key pairing it with the following tool_result.
                // ★malformed contract (FIX-B)★: without a string `name` (schema deviation) no fake ToolCall with an empty name is built (downstream it would pass as 「a nameless tool run」); the original block is preserved whole as
                //   Structured{kind:"tool_use"} (no information loss, no fake call — the render layer decides from the original).
                let Some(name) = block.get("name").and_then(|v| v.as_str()) else {
                    events.push(Self::structured("tool_use", block));
                    return;
                };
                let id = block.get("id").and_then(|v| v.as_str()).map(String::from);
                let args_json = block
                    .get("input")
                    .map(|v| v.to_string())
                    // Without input, an empty object (schema-deviation defense) — args_json is always valid JSON.
                    .unwrap_or_else(|| "{}".to_string());
                events.push(OutputEvent::ToolCall {
                    name: name.to_string(),
                    args_json,
                    id,
                    turn_id: None,
                    message_id: message_id.map(String::from),
                    category: tool_category(name),
                });
            }
            // thinking and tool_result have no formal variant → the original block goes through the Structured escape hatch (kind = the block type as is). tool_result normally comes in a user line and is eaten by the user branch above; in an assistant line it is
            // defensively absorbed here.
            Some(kind @ ("thinking" | "tool_result")) => {
                events.push(Self::structured(kind, block));
            }
            // Unknown block type → cannot be formalized, so preserved through the escape hatch (forward-compat: prevents losing new block kinds).
            Some(other) => {
                events.push(Self::structured(other, block));
            }
            None => {}
        }
    }

    fn structured(kind: &str, value: &serde_json::Value) -> OutputEvent {
        OutputEvent::Structured {
            kind: kind.to_string(),
            json: value.to_string(),
        }
    }

    /// Serializes a user replay block with the line-level `uuid` on it (attaches the dedup key).
    ///
    /// ★shape contract★: a top-level `uuid` key is added to the original block object so it has the same shape as the synthetic echo user_text_echo_json builds (`{"type":…,…,"uuid":"X"}`). A non-object original (schema deviation) has no place for
    ///   the uuid and is serialized as is (defensive). An existing block `uuid` is overwritten with the line-level one (our controlled value) — the single source of the dedup key.
    fn user_block_with_uuid(block: &serde_json::Value, uuid: &str) -> String {
        match block.as_object() {
            Some(map) => {
                let mut owned = map.clone();
                owned.insert(
                    "uuid".to_string(),
                    serde_json::Value::String(uuid.to_string()),
                );
                serde_json::Value::Object(owned).to_string()
            }
            None => block.to_string(),
        }
    }
}

/// claude tool name → neutral category. ★Exact match (case-sensitive)★ — an unknown new tool falls to [`ToolCategory::Other`] (grouping still stands, only the summary says 「other」). `Glob` is `Search` — Claude Code groups it as 「finds files」.
// ADR-0239
fn tool_category(name: &str) -> ToolCategory {
    match name {
        "Read" | "NotebookRead" => ToolCategory::Read,
        "Grep" | "Glob" => ToolCategory::Search,
        "LS" => ToolCategory::List,
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => ToolCategory::Edit,
        "Bash" | "PowerShell" | "BashOutput" | "KillShell" | "KillBash" => ToolCategory::Command,
        "WebFetch" | "WebSearch" => ToolCategory::Web,
        "Task" | "Agent" => ToolCategory::Agent,
        _ if name.starts_with("mcp__") => ToolCategory::Mcp,
        _ => ToolCategory::Other,
    }
}

// ── ADR-0231: queued input — vendor line → registry event (stateless translation) ──────────────────────

/// One `command_lifecycle` line → registry event.
///
/// ★The key is `command_uuid` (the uuid we wrote) — the line's own `uuid` is newly drawn by the CLI (M1)★.
/// ★The lifecycle of input sent right away while idle is translated too★ — an id the registry and accumulator do not know, so only a tombstone remains (reduction rule).
/// Table: `started` → accepted · `cancelled` → unknown (interrupt · failed turn · our cancel — the cancel response tells the cause) · `discarded` → agent ended · `refused` → refused. `queued` · `completed` · unknown states → none.
// ADR-0231
fn lifecycle_event(value: &serde_json::Value) -> Option<QueuedInputEvent> {
    let id = value
        .get("command_uuid")
        .and_then(|v| v.as_str())
        .filter(|id| !id.is_empty())?
        .to_owned();
    let cause = match value.get("state").and_then(|v| v.as_str())? {
        "started" => return Some(QueuedInputEvent::Delivered { id }),
        "cancelled" => DropCause::Unknown,
        "discarded" => DropCause::AgentEnded,
        "refused" => DropCause::Rejected,
        _ => return None,
    };
    Some(QueuedInputEvent::Dropped { id, cause })
}

/// The six lifecycle state words the decoder knows — the same set as the [`lifecycle_event`] translation table.
const LIFECYCLE_STATES: [&str; 6] = [
    "queued",
    "started",
    "completed",
    "cancelled",
    "discarded",
    "refused",
];

/// Whether it is a `command_lifecycle` line with both a non-empty string `command_uuid` and a known state word — the condition that opens acks as `Available`.
// ADR-0231
// ADR-0234
fn is_recognisable_lifecycle(value: &serde_json::Value) -> bool {
    let has_id = value
        .get("command_uuid")
        .and_then(|v| v.as_str())
        .is_some_and(|id| !id.is_empty());
    let known_state = value
        .get("state")
        .and_then(|v| v.as_str())
        .is_some_and(|s| LIFECYCLE_STATES.contains(&s));
    has_id && known_state
}

/// `control_response` of a request [`cancel_line`] sent → cancel response event; `None` for a response to another request.
///
/// ★The value is nested in `response.response.cancelled` (M3)★. Success with the value missing reads as 「could not remove」 (`false`) — `false` is material for telling the cause of the vendor close that follows, so it is the side to lean to when unknown.
/// The `subtype:"error"` envelope is not captured (TRD §3-1) — only the request id is read from the same place.
// ADR-0231
fn cancel_response_event(value: &serde_json::Value) -> Option<QueuedInputEvent> {
    let response = value.get("response")?;
    let id = response
        .get("request_id")
        .and_then(|v| v.as_str())?
        .strip_prefix(CANCEL_REQUEST_PREFIX)
        .filter(|id| !id.is_empty())?
        .to_owned();
    match response.get("subtype").and_then(|v| v.as_str())? {
        "success" => Some(QueuedInputEvent::CancelAnswered {
            id,
            removed: response
                .get("response")
                .and_then(|r| r.get("cancelled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }),
        "error" => Some(QueuedInputEvent::CancelFailed { id }),
        _ => None,
    }
}

/// Decides ack availability from one `system/init` line — `Available` if [`MSG_LIFECYCLE_CAPABILITY`] is in the capability list, otherwise (including old CLIs with no list) `Unavailable`.
///
/// ★Only the call that wins the `Unavailable` transition returns `AckUnavailable{delivered: []}`★ — once per incarnation. Moving the remaining entries into bubbles is the registry and accumulator's job (the decoder does not know the entries) and the core fills
///   body copies. Once `Available`, later inits do not revert it.
// ADR-0231
fn judge_delivery_ack(ack: &DeliveryAck, value: &serde_json::Value) -> Option<QueuedInputEvent> {
    let lifecycle = value
        .get("capabilities")
        .and_then(|c| c.as_array())
        .is_some_and(|caps| {
            caps.iter()
                .any(|c| c.as_str() == Some(MSG_LIFECYCLE_CAPABILITY))
        });
    if lifecycle {
        ack.set_available();
        None
    } else if ack.try_set_unavailable() {
        Some(QueuedInputEvent::AckUnavailable {
            delivered: Vec::new(),
        })
    } else {
        None
    }
}

// ── ADR-0079: `.jsonl` transcript → OutputEvent seed on resume ──────
//
// ★Mapping reuse (one decoder)★: the transcript's `assistant`/`user` lines have **the same** envelope (top-level `type` + `message.content[]`) as live stream-json (measured, claude 2.1.170), so the live decoder's `consume_line` is reused as is. Extra top-level
//   keys (`parentUuid`/`uuid`/`timestamp`/`sessionId`/`isSidechain` …) and non-live line types (`summary`/`file-history-snapshot`/`queue-operation`/`attachment`/`ai-title`) are skipped by `consume_line`'s catch-all (`_ => {}`).
//   ★The one exception = `attachment{queued_command}`★ — the body of input folded in mid-turn remains only there, so it is moved into a bubble (the `LineSource::Transcript` branch); live-only lines (lifecycle · cancel response · init) are not translated there.
//   `stream_event` is called without partial tracking (`partial: None`) and skipped — the complete line emits one copy of the text (ADR-0240).
//   The only extra filter is `isSidechain:true` (sub-agent turns): their `type` is still user/assistant, so they are skipped at line level here.

/// Maximum bytes to parse on transcript seed (**only this much from the end of the file**). Matched to the Ring cap (2MB/4096 events): parsing a huge `.jsonl` (tens of MB) whole and scanning old parts the Ring would discard makes spawn latency explode.
/// Truncating the old past is accepted (ADR-0079; the Ring evicts oldest first anyway). 2MB + margin = 4MB (the truncated first partial line at the seek point is explicitly discarded up to the first `\n` by read_transcript_events).
const TRANSCRIPT_TAIL_BYTES: u64 = 4 * 1024 * 1024;

/// Encodes the cwd into the claude project directory slug.
///
/// ★Encoding rule (measured)★: **every non-alphanumeric character** of the cwd (`:` `\` `/` `.` `_` space etc.) becomes `-`. E.g. `C:\Users\X\AppData\Local\Temp\engram-resume-test` → `C--Users-X-AppData-Local-Temp-engram-resume-test`,
///   `I:\Engram_Workspace\a` → `I--Engram-Workspace-a`. claude makes `~/.claude/projects/<slug>/` by this rule (100% match against 45 local directories). It is lossy (different cwds can share a slug), but the goal is matching claude's behavior.
fn project_slug(cwd: &std::path::Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The directory where claude keeps its state = `CLAUDE_CONFIG_DIR` (when not empty), otherwise `~/.claude`; None if home cannot be found either.
///
/// ★Every claude file path of this folder branches off from here★ (transcript `projects/`, session file `sessions/`) — the same convention in two places would get only one side fixed and the override half-applied.
pub(super) fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    engram_dashboard_platform::env::home_dir().map(|h| h.join(".claude"))
}

/// The path `<config dir>/projects/<slug>/<sid>.jsonl`. None if home cannot be found.
fn transcript_path(cwd: &std::path::Path, sid: Uuid) -> Option<PathBuf> {
    Some(
        config_dir()?
            .join("projects")
            .join(project_slug(cwd))
            .join(format!("{sid}.jsonl")),
    )
}

/// ADR-0079: transcript original (line NDJSON) → list of past `OutputEvent`s. **A pure function (0 external dependencies)**, unit-tested with real fixtures; file I/O is `read_transcript_events` (ADR-0012 seam isolation).
///
/// - `isSidechain:true` (sub-agent turn) lines are skipped — only the original conversation is restored.
/// - result lines get the same translation as live (MessageDone · `TurnEnd` for an interrupted turn, + usage), so a turn-boundary separator appears.
/// - If it ends **without** that result line, one synthetic MessageDone is appended (reason below).
pub(crate) fn parse_transcript_events(transcript: &str) -> Vec<OutputEvent> {
    let mut events = Vec::new();
    for line in transcript.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Scanning only the top-level fields would be enough, but it is already one line, so the parse cost is small and accuracy high, hence decided
        //   with serde (non-JSON lines are skipped by consume_line anyway).
        if is_sidechain_line(trimmed) {
            continue;
        }
        // Not picked up — past limits are not current values. No partial message tracking either (`partial: None` — doc above).
        ClaudeStreamDecoder::consume_line(
            trimmed.as_bytes(),
            &mut events,
            LineSource::Transcript,
            None,
            None,
        );
    }
    // ★The restored history must always end as a "closed turn" (load-bearing)★: a real `.jsonl` **does not contain** live stream-json's `result` lines (measured 2026-08-17: 0 in all 12 recent transcripts; only user/assistant + meta such as
    //   queue-operation · attachment · last-prompt). The seed would end on progress signals with no turn end, and a structured slot reads `!turnDone && items>0` as streaming, so right after resume, **with nothing sent**, the waiting indicator (Wait …)
    //   shows permanently. The new incarnation is idle after resume and does not continue past turns, so closing with one MessageDone matches the facts.
    //
    //   ★Unrelated to observation (ADR-0113)★: this synthetic signal is **for the replay buffer only** — the seed does not go through turn observation (OutputCore::seed), so busy/idle is not bootstrapped by it. Do not bring it back.
    //
    //   If it already ends with a turn end (MessageDone · an interrupted turn's TurnEnd) (fixtures · a future claude that leaves result) nothing is appended (no duplicate boundary). 0 events (empty or meta-only transcript) stays 0 = same as fresh.
    let already_closed = matches!(
        events.last(),
        Some(OutputEvent::MessageDone { .. } | OutputEvent::TurnEnd { .. })
    );
    if !events.is_empty() && !already_closed {
        events.push(OutputEvent::MessageDone {
            turn_id: None,
            message_id: None,
        });
    }
    events
}

/// Whether the line is `isSidechain:true` (sub-agent turn). Non-JSON/missing field is false (= treated as an original turn, preserved).
fn is_sidechain_line(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get("isSidechain").and_then(|s| s.as_bool()))
        .unwrap_or(false)
}

/// ADR-0079: the entry point the daemon calls on a resume spawn. Missing file (new session) or read failure → an empty Vec (no seed = same as fresh). **The only file I/O point**; parsing is delegated to the pure function.
///
/// ★tail read (spawn latency)★: a file over the cap reads only TRANSCRIPT_TAIL_BYTES from the end. The first (partial) line at the seek point was cut mid-object, so it is **explicitly discarded** — bytes up to the first `\n` are dropped before parsing —
///   not relying on 「a partial line is non-JSON anyway」: a fragment that happens to end in a valid JSON suffix could synthesize a fake event. Seek offset 0 (file ≤ cap) keeps the intact first line.
/// ★bounded read (concurrent growth)★: `read_to_end` is unbounded if the file grows while reading; `take` hard-caps at TRANSCRIPT_TAIL_BYTES from the seek point. A seek failure returns an empty Vec explicitly instead of reading from an arbitrary position.
pub(crate) fn read_transcript_events(cwd: &std::path::Path, sid: Uuid) -> Vec<OutputEvent> {
    use std::io::{Read, Seek, SeekFrom};

    let Some(path) = transcript_path(cwd, sid) else {
        return Vec::new();
    };
    let Ok(mut file) = std::fs::File::open(&path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let seeked = len > TRANSCRIPT_TAIL_BYTES;
    if seeked {
        // On a seek failure the file pointer position is unknown → give up (empty Vec) instead of reading from an arbitrary position.
        if file
            .seek(SeekFrom::Start(len - TRANSCRIPT_TAIL_BYTES))
            .is_err()
        {
            return Vec::new();
        }
    }
    let mut buf = Vec::new();
    if file
        .take(TRANSCRIPT_TAIL_BYTES)
        .read_to_end(&mut buf)
        .is_err()
    {
        return Vec::new();
    }
    // Bytes → string is lossy — absorbs the partial multibyte character of the truncated first line (that line is discarded below).
    let text = String::from_utf8_lossy(&buf);
    // Without `\n` (= a single super-long line with no newline within the cap) there is no intact line.
    let to_parse: &str = if seeked {
        match text.find('\n') {
            Some(idx) => &text[idx + 1..],
            None => "",
        }
    } else {
        &text
    };
    parse_transcript_events(to_parse)
}

// ── S15 B3: pump→core wiring seam (ADR-0004/0044) ──────────────────────────────────
impl crate::transport::OutputDecoder for ClaudeStreamDecoder {
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent> {
        ClaudeStreamDecoder::decode(self, chunk)
    }
    fn flush(&mut self) -> Vec<OutputEvent> {
        ClaudeStreamDecoder::flush(self)
    }
    fn take_usage(&mut self) -> Vec<UsageObservation> {
        std::mem::take(&mut self.usage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DeliveredCopy, DropCause, ToolOutcome, CLI_EXE_ENV, CLI_EXE_NAME};

    // ── backend/claude/ unit tests ─────────────────────────────────────────

    /// `result` end = clean end · among error lines, the turn error is only that of a failed `result` · failure of `TurnEnd` = error end.
    // ADR-0231
    #[test]
    fn the_result_end_is_clean_and_only_a_failed_result_error_is_a_turn_error() {
        let classify = ClaudeBackend.turn_classifier();
        assert_eq!(
            classify(&OutputEvent::MessageDone {
                turn_id: None,
                message_id: None
            }),
            Some(TurnSignal::Ended(TurnEndKind::Clean))
        );
        assert_eq!(classify(&OutputEvent::Error("boom".into())), None);
        assert_eq!(
            classify(&OutputEvent::Error(format!(
                "{RESULT_FAILURE_DETAIL} (subtype=error_max_turns)"
            ))),
            Some(TurnSignal::Failed)
        );
        let end = |outcome| {
            classify(&OutputEvent::TurnEnd {
                turn_id: None,
                outcome,
            })
        };
        assert_eq!(
            end(TurnOutcome::Completed),
            Some(TurnSignal::Ended(TurnEndKind::Clean))
        );
        assert_eq!(
            end(TurnOutcome::Failed { detail: None }),
            Some(TurnSignal::Ended(TurnEndKind::Failed))
        );
        for outcome in [TurnOutcome::Interrupted, TurnOutcome::Unknown] {
            assert_eq!(
                end(outcome.clone()),
                Some(TurnSignal::Ended(TurnEndKind::Other)),
                "{outcome:?}"
            );
        }
    }

    /// A tool's end result is not a turn signal whatever its outcome — as progress an end after the turn end would turn 「in a turn」 back on, as an error one tool failure would set the stop after error.
    // ADR-0241
    #[test]
    fn a_tool_result_is_never_a_turn_signal() {
        let classify = ClaudeBackend.turn_classifier();
        for outcome in [
            ToolOutcome::Completed,
            ToolOutcome::Failed,
            ToolOutcome::Declined,
            ToolOutcome::Refused,
        ] {
            assert_eq!(
                classify(&OutputEvent::ToolResult {
                    id: "c1".into(),
                    outcome
                }),
                None,
                "{outcome:?}"
            );
        }
    }

    /// Among registry events only `Delivered` (the vendor `started`) is progress — others counted as progress would turn 「in a turn」 back on after the turn end and block mail until the 30-minute valve.
    // ADR-0231
    #[test]
    fn only_a_delivered_queued_input_is_turn_progress() {
        let classify = ClaudeBackend.turn_classifier();
        assert_eq!(
            classify(&OutputEvent::QueuedInput(QueuedInputEvent::Delivered {
                id: "u1".into()
            })),
            Some(TurnSignal::Progress)
        );
        let id = || "u1".to_owned();
        for ev in [
            QueuedInputEvent::Queued {
                id: id(),
                text: "hi".into(),
            },
            QueuedInputEvent::CancelRequested { id: id() },
            QueuedInputEvent::CancelAnswered {
                id: id(),
                removed: true,
            },
            QueuedInputEvent::CancelAnswered {
                id: id(),
                removed: false,
            },
            QueuedInputEvent::CancelFailed { id: id() },
            QueuedInputEvent::Dropped {
                id: id(),
                cause: DropCause::Rejected,
            },
            QueuedInputEvent::Dropped {
                id: id(),
                cause: DropCause::Unknown,
            },
            QueuedInputEvent::AckUnavailable {
                delivered: vec![DeliveredCopy {
                    id: id(),
                    text: "hi".into(),
                }],
            },
        ] {
            assert_eq!(
                classify(&OutputEvent::QueuedInput(ev.clone())),
                None,
                "{ev:?}"
            );
        }
    }

    fn spec(command: &AgentCommand, mode: SpawnMode, sid: Option<Uuid>) -> CommandSpec {
        ClaudeBackend.build_spec(
            command,
            mode,
            sid,
            None,
            PathBuf::from("."),
            vec![],
            None,
            &LaunchArtifacts::default(),
        )
    }

    fn spec_with_control(
        command: &AgentCommand,
        mode: SpawnMode,
        sid: Option<Uuid>,
        control: Option<ControlEndpoint>,
    ) -> CommandSpec {
        ClaudeBackend.build_spec(
            command,
            mode,
            sid,
            None,
            PathBuf::from("."),
            vec![],
            control,
            &LaunchArtifacts::default(),
        )
    }

    fn terminal(extra: Vec<&str>) -> AgentCommand {
        AgentCommand::Claude {
            extra_args: extra.into_iter().map(String::from).collect(),
            output_format: AgentOutputFormat::Terminal,
        }
    }

    #[test]
    fn claude_fresh_uses_session_id_flag() {
        let sid = Uuid::new_v4();
        let s = spec(&terminal(vec!["--verbose"]), SpawnMode::Fresh, Some(sid));
        let (p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--session-id".to_string(),
                sid.to_string(),
                "--verbose".to_string(),
            ],
        );
        assert_eq!(s.program, p);
        assert_eq!(s.args, a);
    }

    #[test]
    fn claude_resume_uses_resume_flag() {
        let sid = Uuid::new_v4();
        let s = spec(&terminal(vec![]), SpawnMode::Resume, Some(sid));
        let (_p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--resume".to_string(),
                sid.to_string(),
            ],
        );
        assert_eq!(s.args, a);
    }

    // ── ADR-0086: `--mcp-config` injection (control channel entry) ─────────────────────────────────
    fn ep() -> ControlEndpoint {
        ControlEndpoint {
            agent_id: uuid::Uuid::nil(),
            epoch: 1,
            url: "http://127.0.0.1:54321/mcp".to_string(),
            token: "deadbeef".to_string(),
            // ADR-0099: config_path is an Option — Some, since this is the MCP-capable (claude) case.
            config_path: Some(PathBuf::from("C:/data/mcp/agent-x.json")),
            // The default helper carries send_exe so the ENGRAM_CLI_EXE injection path can be verified.
            send_exe: Some(PathBuf::from("C:/app/engram.exe")),
            // ADR-0094: the base helper leaves grants empty — the grant tests fill them (so the mcp-config tests are
            //   not affected by --allowedTools).
            grants: vec![],
            // ADR-0133: the same pair as the daemon output of the MCP-capable branch (config_path=Some) — no mail.
            mail_allowed: false,
        }
    }

    /// A variant carrying send-entry grants (MCP + CLI) (ADR-0094) — for verifying `--allowedTools` injection.
    fn ep_with_grants() -> ControlEndpoint {
        ControlEndpoint {
            grants: vec![
                ToolGrant::Mcp {
                    server: "engram".to_string(),
                    tool: "eg_send".to_string(),
                },
                ToolGrant::Cli {
                    exe: CLI_EXE_NAME.to_string(),
                },
            ],
            ..ep()
        }
    }

    /// The endpoint a non-MCP backend spawn receives — `config_path=None` (no MCP entry) + mail allowed. Both variants get the CLI wiring identically (ADR-0133), so this variant alone guards **no mcp-config** and **mail marker on**.
    fn ep_cli_only() -> ControlEndpoint {
        ControlEndpoint {
            config_path: None,
            mail_allowed: true,
            ..ep()
        }
    }

    /// Non-MCP + send_exe=None (imitating a missing sibling binary).
    fn ep_cli_only_no_send() -> ControlEndpoint {
        ControlEndpoint {
            send_exe: None,
            ..ep_cli_only()
        }
    }

    #[test]
    fn claude_control_endpoint_injects_mcp_config_flag() {
        let sid = Uuid::new_v4();
        let s = spec_with_control(&terminal(vec![]), SpawnMode::Fresh, Some(sid), Some(ep()));
        let (_p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--session-id".to_string(),
                sid.to_string(),
                "--mcp-config".to_string(),
                "C:/data/mcp/agent-x.json".to_string(),
                "--disallowedTools".to_string(),
                "SendMessage".to_string(),
            ],
        );
        assert_eq!(s.args, a, "--mcp-config injected into terminal-mode claude");
    }

    #[test]
    fn claude_control_endpoint_token_never_in_args() {
        // ★Security regression guard★: the token is never carried in args (only inside the config_path file). Only the file
        //   path comes into args → no plaintext token exposure in args/process list/logs.
        let s = spec_with_control(&terminal(vec![]), SpawnMode::Fresh, None, Some(ep()));
        assert!(
            !s.args.iter().any(|a| a.contains("deadbeef")),
            "the token must not leak into args: {:?}",
            s.args
        );
        assert!(
            s.args.iter().any(|a| a == "--mcp-config"),
            "the --mcp-config flag must be present"
        );
    }

    #[test]
    fn claude_no_control_endpoint_no_mcp_config() {
        let s = spec(&terminal(vec!["--debug"]), SpawnMode::Fresh, None);
        assert!(
            !s.args.iter().any(|a| a == "--mcp-config"),
            "without control there is no --mcp-config: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_control_endpoint_none_config_path_no_mcp_config_flag() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_cli_only()),
        );
        assert!(
            !s.args.iter().any(|a| a == "--mcp-config"),
            "config_path=None → --mcp-config not injected: {:?}",
            s.args
        );
        assert!(
            s.env.iter().any(|(k, _)| k == "ENGRAM_TOKEN"),
            "config_path=None (CLI only) → ENGRAM_TOKEN injected"
        );
    }

    // ── ADR-0086 step 2: CLI credential env injection (ENGRAM_TOKEN / ENGRAM_CONTROL_URL) ──────────────
    #[test]
    fn claude_cli_only_endpoint_injects_cli_env() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_cli_only()),
        );
        let token = s
            .env
            .iter()
            .find(|(k, _)| k == "ENGRAM_TOKEN")
            .map(|(_, v)| v.as_str());
        let url = s
            .env
            .iter()
            .find(|(k, _)| k == "ENGRAM_CONTROL_URL")
            .map(|(_, v)| v.as_str());
        assert_eq!(token, Some("deadbeef"), "ENGRAM_TOKEN = endpoint token");
        assert_eq!(
            url,
            Some("http://127.0.0.1:54321"),
            "ENGRAM_CONTROL_URL = the base with /mcp stripped from the MCP url"
        );
        let send_exe = s
            .env
            .iter()
            .find(|(k, _)| k == CLI_EXE_ENV)
            .map(|(_, v)| v.as_str());
        assert_eq!(
            send_exe,
            Some("C:/app/engram.exe"),
            "ENGRAM_CLI_EXE = absolute path of endpoint.send_exe"
        );
    }

    #[test]
    fn claude_cli_only_without_send_exe_omits_send_env() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_cli_only_no_send()),
        );
        assert!(
            s.env.iter().any(|(k, _)| k == "ENGRAM_TOKEN")
                && s.env.iter().any(|(k, _)| k == "ENGRAM_CONTROL_URL"),
            "token/url are injected even without send_exe: {:?}",
            s.env
        );
        assert!(
            !s.env.iter().any(|(k, _)| k == CLI_EXE_ENV),
            "with send_exe=None, ENGRAM_CLI_EXE is omitted: {:?}",
            s.env
        );
    }

    #[test]
    fn claude_no_control_endpoint_no_cli_env() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        for key in ["ENGRAM_TOKEN", "ENGRAM_CONTROL_URL", CLI_EXE_ENV] {
            assert!(
                !s.env.iter().any(|(k, _)| k == key),
                "without control there is no CLI env ({key}): {:?}",
                s.env
            );
        }
    }

    // ── ADR-0094: PATH injection (bare executable name resolution — prepend the send_exe parent directory) ──────────
    #[test]
    fn claude_cli_only_endpoint_injects_path_with_send_exe_dir_prepended() {
        // send_exe of ep_cli_only() = C:/app/engram.exe → parent = C:/app.
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_cli_only()),
        );
        let path = s
            .env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.as_str())
            .expect("with send_exe, the PATH env is injected");
        // Asserted by split_paths decomposition, not a string prefix comparison — absorbs separator and notation differences.
        let first = std::env::split_paths(path)
            .next()
            .expect("at least one element in PATH");
        assert_eq!(
            first,
            std::path::PathBuf::from("C:/app"),
            "PATH first element = send_exe parent directory: {path}"
        );
        if let Some(orig) = std::env::var_os("PATH") {
            if let Some(orig_first) = std::env::split_paths(&orig).next() {
                let injected: Vec<std::path::PathBuf> = std::env::split_paths(path).collect();
                assert!(
                    injected.iter().skip(1).any(|c| *c == orig_first),
                    "the first component of the original PATH must be preserved after the injected parent: injected={injected:?} orig_first={orig_first:?}"
                );
            }
        }
    }

    #[test]
    fn claude_cli_only_without_send_exe_omits_path_env() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_cli_only_no_send()),
        );
        assert!(
            !s.env.iter().any(|(k, _)| k == "PATH"),
            "with send_exe=None, the PATH env is not injected: {:?}",
            s.env
        );
    }

    #[test]
    fn claude_no_control_endpoint_no_path_env() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        assert!(
            !s.env.iter().any(|(k, _)| k == "PATH"),
            "without control there is no PATH env: {:?}",
            s.env
        );
    }

    /// build_spec helper injecting a profile env (initial env vector), to verify profile PATH first (FIX-1): spec_with_control always starts with an empty env, so build_spec is called directly (imitating the spawn path pushing profile.env first).
    fn spec_with_env(
        command: &AgentCommand,
        control: Option<ControlEndpoint>,
        profile_env: Vec<(String, String)>,
    ) -> CommandSpec {
        ClaudeBackend.build_spec(
            command,
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("."),
            profile_env,
            control,
            &LaunchArtifacts::default(),
        )
    }

    fn path_entries(s: &CommandSpec) -> Vec<&str> {
        s.env
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("PATH"))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    #[test]
    fn claude_profile_path_survives_as_tail_uppercase_key() {
        let profile_env = vec![("PATH".to_string(), "C:\\custom;C:\\other".to_string())];
        let s = spec_with_env(&terminal(vec![]), Some(ep_cli_only()), profile_env);
        let entries = path_entries(&s);
        assert_eq!(
            entries.len(),
            1,
            "exactly one winning PATH (existing profile entry replaced in place): {:?}",
            s.env
        );
        let components: Vec<std::path::PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&std::path::PathBuf::from("C:/app")),
            "PATH first component = send_exe parent (C:/app): {:?}",
            components
        );
        assert!(
            components.contains(&std::path::PathBuf::from("C:\\custom"))
                && components.contains(&std::path::PathBuf::from("C:\\other")),
            "the profile PATH (C:\\custom;C:\\other) must survive as the tail: {:?}",
            components
        );
    }

    #[cfg(windows)]
    #[test]
    fn claude_profile_path_survives_with_windows_path_key_casing() {
        let profile_env = vec![("Path".to_string(), "C:\\custom;C:\\other".to_string())];
        let s = spec_with_env(&terminal(vec![]), Some(ep_cli_only()), profile_env);
        let entries = path_entries(&s);
        assert_eq!(
            entries.len(),
            1,
            "Windows: the 'Path' key is recognized as the same variable → exactly one winning PATH: {:?}",
            s.env
        );
        let components: Vec<std::path::PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&std::path::PathBuf::from("C:/app")),
            "PATH first component = send_exe parent: {:?}",
            components
        );
        assert!(
            components.contains(&std::path::PathBuf::from("C:\\custom"))
                && components.contains(&std::path::PathBuf::from("C:\\other")),
            "the profile 'Path' value survives as the tail: {:?}",
            components
        );
        assert!(
            s.env.iter().any(|(k, _)| k == "Path"),
            "replaced in place, so the profile key notation ('Path') is kept: {:?}",
            s.env
        );
    }

    #[cfg(windows)]
    #[test]
    fn claude_duplicate_case_variant_path_uses_last_value_and_dedupes() {
        let profile_env = vec![
            ("PATH".to_string(), "C:\\daemon".to_string()),
            ("Path".to_string(), "C:\\profile".to_string()),
        ];
        let s = spec_with_env(&terminal(vec![]), Some(ep_cli_only()), profile_env);
        let entries = path_entries(&s);
        assert_eq!(
            entries.len(),
            1,
            "exactly one case-equivalent PATH (duplicates removed): {:?}",
            s.env
        );
        assert!(
            s.env.iter().any(|(k, _)| k == "Path") && !s.env.iter().any(|(k, _)| k == "PATH"),
            "winning key notation = the last entry ('Path'), 'PATH' removed: {:?}",
            s.env
        );
        let components: Vec<std::path::PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&std::path::PathBuf::from("C:/app")),
            "PATH first component = send_exe parent (C:/app): {:?}",
            components
        );
        assert!(
            components.contains(&std::path::PathBuf::from("C:\\profile")),
            "tail = the last PATH value (C:\\profile) preserved: {:?}",
            components
        );
        assert!(
            !components.contains(&std::path::PathBuf::from("C:\\daemon")),
            "the first PATH value (C:\\daemon) must not be used as the base (the last wins): {:?}",
            components
        );
    }

    #[test]
    fn claude_duplicate_same_key_path_uses_last_value_and_dedupes() {
        // Exactly the same key duplicated — does not rely on case-insensitive matching, so it runs on every OS
        //   (distinguished from the cfg(windows) sibling test).
        let profile_env = vec![
            ("PATH".to_string(), "C:\\first".to_string()),
            ("PATH".to_string(), "C:\\second".to_string()),
        ];
        let s = spec_with_env(&terminal(vec![]), Some(ep_cli_only()), profile_env);
        let entries = path_entries(&s);
        assert_eq!(
            entries.len(),
            1,
            "exactly one PATH (duplicates removed): {:?}",
            s.env
        );
        let components: Vec<std::path::PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&std::path::PathBuf::from("C:/app")),
            "PATH first component = send_exe parent: {:?}",
            components
        );
        assert!(
            components.contains(&std::path::PathBuf::from("C:\\second")),
            "tail = the last PATH value (C:\\second) preserved: {:?}",
            components
        );
        assert!(
            !components.contains(&std::path::PathBuf::from("C:\\first")),
            "the first PATH value (C:\\first) must not be used as the base (the last wins): {:?}",
            components
        );
    }

    // ★The non-UTF8/join-failure skip cannot be reproduced portably★: a join_paths failure (separator in a path) or non-UTF8 PATH at the CommandSpec.env (`Vec<(String,String)>`) boundary would have to already be a valid UTF-8 String, a contradiction,
    //   and there is no OsString injection path. So this branch has no unit test (observability: the FIX-2/3 comments and a loud warn); on failure the inherited PATH is kept, a safe fallback.

    // ── ADR-0133: wiring goes to everyone, only mail is split by the marker (bidirectional assertions on both branches) ──────────────────
    fn env_value<'a>(s: &'a CommandSpec, key: &str) -> Option<&'a str> {
        s.env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// ★The profile PATH must always be carried (with an empty env the PATH assertion becomes vacuous)★: with 0 entries there is nothing
    ///   to compare tail survival against.
    #[test]
    fn claude_mcp_capable_spawn_gets_cli_wiring_with_mail_marked_off() {
        let profile_env = vec![("PATH".to_string(), "C:\\custom".to_string())];
        let s = spec_with_env(&terminal(vec![]), Some(ep()), profile_env);
        for key in ["ENGRAM_TOKEN", "ENGRAM_CONTROL_URL", CLI_EXE_ENV] {
            assert!(
                s.env.iter().any(|(k, _)| k == key),
                "an MCP-capable spawn must also receive the control CLI entry ({key}) — control is open to all: {:?}",
                s.env
            );
        }
        let entries = path_entries(&s);
        assert_eq!(entries.len(), 1, "exactly one PATH: {:?}", s.env);
        let components: Vec<PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&PathBuf::from("C:/app")),
            "front of PATH = send_exe sibling directory: {components:?}"
        );
        assert!(
            components.contains(&PathBuf::from("C:\\custom")),
            "the profile PATH survives as the tail: {components:?}"
        );
        assert!(
            s.args.iter().any(|a| a == "--mcp-config"),
            "the MCP branch must also receive the mcp-config (not exclusive with wiring): {:?}",
            s.args
        );
    }

    #[test]
    fn claude_non_mcp_spawn_gets_the_same_wiring() {
        let profile_env = vec![("PATH".to_string(), "C:\\custom".to_string())];
        let s = spec_with_env(&terminal(vec![]), Some(ep_cli_only()), profile_env);
        for key in ["ENGRAM_TOKEN", "ENGRAM_CONTROL_URL", CLI_EXE_ENV] {
            assert!(
                s.env.iter().any(|(k, _)| k == key),
                "a non-MCP spawn must receive {key}: {:?}",
                s.env
            );
        }
        let entries = path_entries(&s);
        assert_eq!(entries.len(), 1, "exactly one PATH: {:?}", s.env);
        let components: Vec<PathBuf> = std::env::split_paths(entries[0]).collect();
        assert_eq!(
            components.first(),
            Some(&PathBuf::from("C:/app")),
            "front of PATH = send_exe sibling directory: {components:?}"
        );
        assert!(
            components.contains(&PathBuf::from("C:\\custom")),
            "the profile PATH survives as the tail: {components:?}"
        );
        assert!(
            !s.args.iter().any(|a| a == "--mcp-config"),
            "no mcp-config on the non-MCP branch: {:?}",
            s.args
        );
    }

    // ── ADR-0370: a Noop launch carries no instructions and no `--settings`, control channel or not ───────────
    // ADR-0370
    #[test]
    fn claude_noop_spawn_with_a_control_endpoint_has_no_append_flag() {
        for command in [terminal(vec![]), json(vec![])] {
            let s = spec_with_control(&command, SpawnMode::Fresh, None, Some(ep()));
            assert!(
                !s.args.iter().any(|a| a == "--append-system-prompt-file"),
                "{command:?}: {:?}",
                s.args
            );
        }
    }

    #[test]
    fn claude_no_control_endpoint_no_priming_flag() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        assert!(
            !s.args.iter().any(|a| a == "--append-system-prompt-file"),
            "without control there is no priming flag: {:?}",
            s.args
        );
    }

    // ADR-0370
    #[test]
    fn claude_noop_spawn_with_a_control_endpoint_has_no_settings_flag() {
        for command in [terminal(vec![]), json(vec![])] {
            let s = spec_with_control(&command, SpawnMode::Fresh, None, Some(ep()));
            assert!(
                !s.args.iter().any(|a| a == "--settings"),
                "{command:?}: {:?}",
                s.args
            );
        }
    }

    #[test]
    fn claude_no_control_endpoint_no_settings_flag() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        assert!(
            !s.args.iter().any(|a| a == "--settings"),
            "without control there is no --settings: {:?}",
            s.args
        );
    }

    // ── ADR-0094: send permission pre-authorization (`--allowedTools`) ─────────────────────────
    #[test]
    fn grants_to_allowed_tools_mcp_pattern() {
        let out = grants_to_allowed_tools(&[ToolGrant::Mcp {
            server: "engram".to_string(),
            tool: "eg_send".to_string(),
        }]);
        assert_eq!(out, vec!["mcp__engram__eg_send".to_string()]);
    }

    #[test]
    fn grants_to_allowed_tools_cli_pattern() {
        let out = grants_to_allowed_tools(&[ToolGrant::Cli {
            exe: CLI_EXE_NAME.to_string(),
        }]);
        assert_eq!(
            out,
            vec![
                format!("Bash({CLI_EXE_NAME}:*)"),
                format!("PowerShell({CLI_EXE_NAME}:*)"),
            ]
        );
    }

    #[test]
    fn grants_to_allowed_tools_both_preserve_order() {
        let out = grants_to_allowed_tools(&[
            ToolGrant::Mcp {
                server: "engram".to_string(),
                tool: "eg_send".to_string(),
            },
            ToolGrant::Cli {
                exe: CLI_EXE_NAME.to_string(),
            },
        ]);
        assert_eq!(
            out,
            vec![
                "mcp__engram__eg_send".to_string(),
                format!("Bash({CLI_EXE_NAME}:*)"),
                format!("PowerShell({CLI_EXE_NAME}:*)"),
            ]
        );
    }

    #[test]
    fn grants_to_allowed_tools_empty_is_empty() {
        assert!(grants_to_allowed_tools(&[]).is_empty());
    }

    #[test]
    fn claude_grants_inject_allowed_tools_flag_terminal() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let pos = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("with grants, --allowedTools is injected");
        assert_eq!(
            s.args.get(pos + 1).map(|s| s.as_str()),
            Some("mcp__engram__eg_send"),
            "first pattern = MCP send entry (primary certain path): {:?}",
            s.args
        );
        assert_eq!(
            s.args.get(pos + 2).map(|s| s.as_str()),
            Some(format!("Bash({CLI_EXE_NAME}:*)").as_str()),
            "second pattern = CLI send entry, Bash shape (bare name colon-star): {:?}",
            s.args
        );
        assert_eq!(
            s.args.get(pos + 3).map(|s| s.as_str()),
            Some(format!("PowerShell({CLI_EXE_NAME}:*)").as_str()),
            "third pattern = CLI send entry, PowerShell shape (FIX-4 — two shell tools of the same entry): {:?}",
            s.args
        );
    }

    #[test]
    fn claude_grants_inject_allowed_tools_flag_json_mode() {
        let s = spec_with_control(
            &json(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        assert!(
            s.args.iter().any(|a| a == "--allowedTools"),
            "--allowedTools injected in json mode too: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_empty_grants_no_allowed_tools_flag() {
        let s = spec_with_control(&terminal(vec![]), SpawnMode::Fresh, None, Some(ep()));
        assert!(
            !s.args.iter().any(|a| a == "--allowedTools"),
            "with empty grants there is no --allowedTools: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_no_control_endpoint_no_allowed_tools_flag() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        assert!(
            !s.args.iter().any(|a| a == "--allowedTools"),
            "without control there is no --allowedTools: {:?}",
            s.args
        );
    }

    // ── ADR-0094/0106: blocking the built-in SendMessage (`--disallowedTools SendMessage`, control-scoped) ──
    #[test]
    fn claude_no_control_endpoint_no_disallowed_tools_flag() {
        // ADR-0106
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        assert!(
            !s.args.iter().any(|a| a == "--disallowedTools"),
            "without control, --disallowedTools is not injected (an ordinary spawn keeps the built-in SendMessage): {:?}",
            s.args
        );
    }

    #[test]
    fn claude_disallowed_tools_precedes_allowed_tools_group() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let disallowed = s
            .args
            .iter()
            .position(|a| a == "--disallowedTools")
            .expect("with control (including grants), --disallowedTools is injected");
        let allowed = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("with grants, --allowedTools exists");
        assert_eq!(
            s.args.get(disallowed + 1).map(|s| s.as_str()),
            Some("SendMessage"),
            "the disallowedTools value = SendMessage alone: {:?}",
            s.args
        );
        assert!(
            disallowed + 2 == allowed,
            "--allowedTools must come right after the disallowedTools group (flag+value, 2 elements) (no absorption in between): disallowed={disallowed} allowedTools={allowed} args={:?}",
            s.args
        );
    }

    #[test]
    fn claude_disallowed_tools_present_in_json_mode_too_with_control() {
        let s = spec_with_control(&json(vec![]), SpawnMode::Fresh, None, Some(ep()));
        assert!(
            s.args.iter().any(|a| a == "--disallowedTools"),
            "with control, --disallowedTools is injected in json mode too: {:?}",
            s.args
        );
    }

    fn permission_mode_pair_index(args: &[String]) -> Option<usize> {
        args.windows(2)
            .position(|w| w[0] == "--permission-mode" && w[1] == "bypassPermissions")
    }

    #[test]
    fn claude_terminal_injects_auto_permission_mode_pair() {
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, None);
        // ★First `--` flag = pair pin★: the absolute index differs per platform (the Windows console_command wrapper `cmd.exe /c claude` puts tokens in front), so it asserts "the first flag of args is this pair" — an extra_args passthrough forming the pair
        //   cannot mask a base injection regression (base always comes first).
        let first_flag = s.args.iter().position(|a| a.starts_with("--"));
        assert!(
            first_flag.is_some()
                && permission_mode_pair_index(&s.args) == first_flag,
            "the first flag of a terminal spawn must be the pair `--permission-mode bypassPermissions` (even without control): {:?}",
            s.args
        );
    }

    #[test]
    fn claude_json_injects_auto_permission_mode_pair() {
        let s = spec(&json(vec![]), SpawnMode::Fresh, None);
        let first_flag = s.args.iter().position(|a| a.starts_with("--"));
        assert!(
            first_flag.is_some()
                && permission_mode_pair_index(&s.args) == first_flag,
            "the first flag of a json spawn must be the pair `--permission-mode bypassPermissions` (even without control): {:?}",
            s.args
        );
    }

    #[test]
    fn claude_auto_permission_mode_precedes_allowed_tools_terminal() {
        let s = spec_with_control(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let pair = permission_mode_pair_index(&s.args).expect("auto permission pair present");
        let allowed = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("with grants, --allowedTools exists");
        assert!(
            pair + 1 < allowed,
            "the auto permission pair must come before the --allowedTools group: pair={pair} allowedTools={allowed} args={:?}",
            s.args
        );
    }

    #[test]
    fn claude_auto_permission_mode_precedes_allowed_tools_json() {
        let s = spec_with_control(
            &json(vec![]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let pair = permission_mode_pair_index(&s.args).expect("auto permission pair present");
        let allowed = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("with grants, --allowedTools exists");
        assert!(
            pair + 1 < allowed,
            "json mode: the auto permission pair must come before the --allowedTools group: pair={pair} allowedTools={allowed} args={:?}",
            s.args
        );
    }

    fn terminal_extra(extra: Vec<&str>) -> AgentCommand {
        terminal(extra)
    }

    fn expected_grant_patterns() -> Vec<String> {
        grants_to_allowed_tools(&ep_with_grants().grants)
    }

    #[test]
    fn claude_allowed_tools_group_is_last_and_exact_terminal() {
        // ★FIX #1 regression (prevents variadic absorption)★: puts a bare positional ("Bash") in extra_args and asserts that it
        //   does not sit adjacent after the grant value run.
        let s = spec_with_control(
            &terminal_extra(vec!["Bash", "--debug"]),
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let pos = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("with grants, --allowedTools is injected");
        let patterns = expected_grant_patterns();
        assert_eq!(
            &s.args[pos + 1..],
            &patterns[..],
            "the token run after allowedTools is exactly the grant patterns and nothing after (prevents variadic absorption): {:?}",
            s.args
        );
        let bash_pos = s
            .args
            .iter()
            .position(|a| a == "Bash")
            .expect("the bare positional 'Bash' of extra_args is present in args");
        assert!(
            bash_pos < pos,
            "the bare 'Bash' must come before --allowedTools (after it, the variadic would absorb it into a blanket Bash grant): bash={bash_pos} allowedTools={pos} args={:?}",
            s.args
        );
    }

    #[test]
    fn claude_allowed_tools_group_is_last_and_exact_json_mode() {
        let s = spec_with_control(
            &AgentCommand::Claude {
                extra_args: vec!["Bash".to_string()],
                output_format: AgentOutputFormat::StreamJson,
            },
            SpawnMode::Fresh,
            None,
            Some(ep_with_grants()),
        );
        let pos = s
            .args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("json mode grants injected");
        let patterns = expected_grant_patterns();
        assert_eq!(
            &s.args[pos + 1..],
            &patterns[..],
            "json mode: the run after allowedTools is exactly the grant patterns and nothing after: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_space_containing_bash_pattern_stays_single_argv_element() {
        // Production grants are bare executable names, but argv integrity regression is verified with a space case
        //   (grants_to_allowed_tools is name-agnostic, so it wraps a value with spaces in colon-star as is).
        let ep = ControlEndpoint {
            grants: vec![ToolGrant::Cli {
                exe: "C:\\Program Files\\eng\\engram.exe".to_string(),
            }],
            ..ep()
        };
        let s = spec_with_control(&terminal(vec![]), SpawnMode::Fresh, None, Some(ep));
        let pos = s.args.iter().position(|a| a == "--allowedTools").unwrap();
        assert_eq!(
            s.args.get(pos + 1).map(|s| s.as_str()),
            Some("Bash(C:\\Program Files\\eng\\engram.exe:*)"),
            "a Bash pattern with spaces stays one argv element (not split): {:?}",
            s.args
        );
        assert_eq!(
            s.args.get(pos + 2).map(|s| s.as_str()),
            Some("PowerShell(C:\\Program Files\\eng\\engram.exe:*)"),
            "a PowerShell pattern with spaces also stays one argv element: {:?}",
            s.args
        );
        // 1 CLI grant = 2 patterns (so pos+3 is the end).
        assert_eq!(
            pos + 3,
            s.args.len(),
            "the patterns are the last of args: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_json_mode_control_endpoint_injects_mcp_config() {
        let s = spec_with_control(&json(vec![]), SpawnMode::Fresh, None, Some(ep()));
        assert!(
            s.args.iter().any(|a| a == "--mcp-config"),
            "--mcp-config injected in json mode too: {:?}",
            s.args
        );
    }

    #[test]
    fn claude_no_session_id_produces_no_flags() {
        let s = spec(&terminal(vec!["--debug"]), SpawnMode::Fresh, None);
        let (p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--debug".to_string(),
            ],
        );
        assert_eq!(s.program, p);
        assert_eq!(s.args, a);
    }

    #[test]
    fn shell_passthrough_via_claude_backend() {
        let s = spec(
            &AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec!["/c".into(), "echo hi".into()],
            },
            SpawnMode::Fresh,
            Some(Uuid::new_v4()),
        );
        assert_eq!(s.program, "cmd.exe");
        assert_eq!(s.args, vec!["/c".to_string(), "echo hi".to_string()]);
    }

    #[test]
    fn capabilities_terminal_resume_is_true() {
        assert!(ClaudeBackend.capabilities(&terminal(vec![])).session.resume);
    }

    #[test]
    fn capabilities_json_mode_resume_is_true() {
        assert!(
            ClaudeBackend.capabilities(&json(vec![])).session.resume,
            "json-mode claude is also resume=true (--resume supported, spike-verified)"
        );
    }

    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("FOO".to_string(), "bar".to_string())];
        let s = ClaudeBackend.build_spec(
            &terminal(vec![]),
            SpawnMode::Fresh,
            None,
            None,
            cwd.clone(),
            env.clone(),
            None,
            &LaunchArtifacts::default(),
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, [env[0].clone(), owned(NO_AUTOUPDATE_ENV)]);
    }

    fn owned((k, v): (&str, &str)) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    fn autoupdate_values(s: &CommandSpec) -> Vec<&str> {
        s.env
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("DISABLE_AUTOUPDATER"))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    #[test]
    fn every_claude_spawn_disables_the_autoupdater() {
        for command in [terminal(vec![]), json(vec![])] {
            for control in [None, Some(ep())] {
                let s = spec_with_env(&command, control.clone(), vec![]);
                assert_eq!(autoupdate_values(&s), ["1"], "{command:?} / {control:?}");
            }
        }
    }

    #[test]
    fn profile_autoupdater_value_wins_over_the_default() {
        for command in [terminal(vec![]), json(vec![])] {
            // The same spelling leaves only the one profile value on any OS — removing the skip guard breaks here.
            let exact = vec![("DISABLE_AUTOUPDATER".to_string(), "0".to_string())];
            let s = spec_with_env(&command, None, exact);
            assert_eq!(autoupdate_values(&s), ["0"], "{command:?}");
            let profile = vec![("disable_autoupdater".to_string(), "0".to_string())];
            let s = spec_with_env(&command, None, profile);
            let expected: &[&str] = if env_key_eq("disable_autoupdater", "DISABLE_AUTOUPDATER") {
                &["0"]
            } else {
                &["0", "1"]
            };
            assert_eq!(autoupdate_values(&s), expected, "{command:?}");
        }
    }

    // ── ADR-0044: json (stream-json) mode build_spec golden ─────────────────────────
    fn json(extra: Vec<&str>) -> AgentCommand {
        AgentCommand::Claude {
            extra_args: extra.into_iter().map(String::from).collect(),
            output_format: AgentOutputFormat::StreamJson,
        }
    }

    #[test]
    fn json_mode_build_spec_uses_headless_stream_json_args() {
        let sid = Uuid::new_v4();
        let s = spec(
            &json(vec!["--model", "sonnet"]),
            SpawnMode::Fresh,
            Some(sid),
        );
        let (p, a) = console_command(
            CLAUDE_PROGRAM,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "-p".to_string(),
                "--input-format".to_string(),
                "stream-json".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--replay-user-messages".to_string(),
                "--verbose".to_string(),
                "--include-partial-messages".to_string(),
                "--session-id".to_string(),
                sid.to_string(),
                "--model".to_string(),
                "sonnet".to_string(),
                // ADR-0106: control=None (non-messaging spawn) → --disallowedTools not injected.
            ],
        );
        assert_eq!(s.program, p);
        assert_eq!(s.args, a, "json mode argument golden mismatch");
        assert!(s.args.iter().any(|x| x == "--verbose"));
    }

    #[test]
    fn json_mode_resume_uses_resume_flag() {
        let sid = Uuid::new_v4();
        let s = spec(&json(vec![]), SpawnMode::Resume, Some(sid));
        assert!(
            s.args.iter().any(|x| x == "--resume"),
            "json resume must go to --resume (spike-verified)"
        );
        assert!(
            !s.args.iter().any(|x| x == "--session-id"),
            "json Resume mode must not use --session-id (fresh)"
        );
        assert!(
            s.args.iter().any(|x| x == &sid.to_string()),
            "the resume sid must be carried in the arguments"
        );
    }

    #[test]
    fn terminal_mode_spec_unchanged_regression() {
        let sid = Uuid::new_v4();
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, Some(sid));
        for forbidden in ["-p", "--input-format", "--output-format", "stream-json"] {
            assert!(
                !s.args.iter().any(|x| x == forbidden),
                "json arguments leaked into terminal mode: {forbidden}"
            );
        }
    }

    // ADR-0240
    #[test]
    fn terminal_mode_has_no_partial_messages_flag() {
        for mode in [SpawnMode::Fresh, SpawnMode::Resume] {
            let s = spec(&terminal(vec![]), mode, Some(Uuid::new_v4()));
            assert!(
                !s.args.iter().any(|x| x == "--include-partial-messages"),
                "terminal mode arguments are frozen: {:?}",
                s.args
            );
        }
    }

    // ── ADR-0049: default MAX_THINKING_TOKENS injection in json mode (enables extended thinking) ──────
    #[test]
    fn json_mode_injects_default_max_thinking_tokens() {
        let s = spec(&json(vec![]), SpawnMode::Fresh, None);
        let vals: Vec<&str> = s
            .env
            .iter()
            .filter(|(k, _)| k == "MAX_THINKING_TOKENS")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            vals,
            vec!["8000"],
            "json mode injects the default MAX_THINKING_TOKENS=8000"
        );
    }

    #[test]
    fn json_mode_profile_max_thinking_tokens_wins() {
        let env = vec![("MAX_THINKING_TOKENS".to_string(), "1234".to_string())];
        let s = ClaudeBackend.build_spec(
            &json(vec![]),
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("."),
            env,
            None,
            &LaunchArtifacts::default(),
        );
        let vals: Vec<&str> = s
            .env
            .iter()
            .filter(|(k, _)| k == "MAX_THINKING_TOKENS")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            vals,
            vec!["1234"],
            "the value the profile gave must remain alone (default not injected — exactly 1)"
        );
    }

    #[test]
    fn json_mode_profile_lowercase_key_follows_the_os_key_rule() {
        let env = vec![("max_thinking_tokens".to_string(), "1234".to_string())];
        let s = ClaudeBackend.build_spec(
            &json(vec![]),
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("."),
            env,
            None,
            &LaunchArtifacts::default(),
        );
        let vals: Vec<&str> = s
            .env
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("MAX_THINKING_TOKENS"))
            .map(|(_, v)| v.as_str())
            .collect();
        let expected = if env_key_eq("max_thinking_tokens", "MAX_THINKING_TOKENS") {
            vec!["1234"]
        } else {
            vec!["1234", "8000"]
        };
        assert_eq!(
            vals, expected,
            "on an OS that treats them as the same key (Windows), a lowercase profile key also prevents duplicate injection — the value is the profile's original 1234 alone"
        );
    }

    #[test]
    fn terminal_mode_does_not_inject_max_thinking_tokens() {
        let sid = Uuid::new_v4();
        let s = spec(&terminal(vec![]), SpawnMode::Fresh, Some(sid));
        assert!(
            !s.env.iter().any(|(k, _)| k == "MAX_THINKING_TOKENS"),
            "MAX_THINKING_TOKENS must not be injected in terminal mode"
        );
    }

    // ── ADR-0044/0004: input wrapping (stdin user turn JSON) golden ─────────────────────
    #[test]
    fn wrap_user_turn_exact_line_and_newline_terminated() {
        let id = Uuid::new_v4();
        let bytes = wrap_user_turn("hello", id);
        let expected = format!(
            "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hello\"}}]}},\"uuid\":\"{id}\"}}\n"
        );
        assert_eq!(bytes, expected.into_bytes());
        assert_eq!(
            *bytes.last().unwrap(),
            b'\n',
            "line terminator \\n required"
        );
        let v: serde_json::Value =
            serde_json::from_str(String::from_utf8(bytes.clone()).unwrap().trim_end()).unwrap();
        assert_eq!(v["uuid"], id.to_string());
    }

    // ── ADR-0044/0045: input-time user echo json helper (user_text_echo_json) ─────────────
    #[test]
    fn user_text_echo_json_matches_decoder_uuid_block_shape() {
        let id = Uuid::new_v4();
        let json = user_text_echo_json("hello", id);
        let expected = format!(r#"{{"type":"text","text":"hello","uuid":"{id}"}}"#);
        assert_eq!(json, expected);
        let json2 = user_text_echo_json("a\"b\nc 한글", id); // hangul-ok
        let v: serde_json::Value = serde_json::from_str(&json2).unwrap();
        assert_eq!(v["type"], "text");
        assert_eq!(v["text"], "a\"b\nc 한글"); // hangul-ok
        assert_eq!(v["uuid"], id.to_string());
    }

    // ── ADR-0044/0045: user-role replay dedup ─────────────────────────────────────────
    #[test]
    fn user_role_text_block_passes_through_with_line_uuid() {
        let line = concat!(
            r#"{"type":"user","message":{"role":"user","content":["#,
            r#"{"type":"text","text":"a message I typed"}"#,
            r#"]},"uuid":"11111111-1111-1111-1111-111111111111","isReplay":true}"#,
            "\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:user"],
            "replay user text is not suppressed — passes through carrying the uuid: {ev:?}"
        );
        match &ev[0] {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["type"], "text");
                assert_eq!(v["text"], "a message I typed");
                assert_eq!(
                    v["uuid"], "11111111-1111-1111-1111-111111111111",
                    "the line-level uuid must be carried in the block json (dedup key)"
                );
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    #[test]
    fn user_role_past_text_block_without_uuid_is_preserved_vanish_guard() {
        let line = concat!(
            r#"{"type":"user","message":{"role":"user","content":["#,
            r#"{"type":"text","text":"a message typed in the past"}"#,
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:user"],
            "past user text without a uuid must be preserved (no vanish regression): {ev:?}"
        );
        match &ev[0] {
            OutputEvent::Structured { json, .. } => {
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["text"], "a message typed in the past");
                assert!(
                    v.get("uuid").is_none(),
                    "no uuid is attached when absent (original preserved)"
                );
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    #[test]
    fn user_role_tool_result_block_still_emitted_regression_guard() {
        let line = concat!(
            r#"{"type":"user","message":{"role":"user","content":["#,
            r#"{"tool_use_id":"toolu_1","type":"tool_result","content":"file contents"}"#,
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:user"],
            "a user-role tool_result is not subject to suppression — it must be preserved"
        );
        match &ev[0] {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["type"], "tool_result");
                assert_eq!(v["tool_use_id"], "toolu_1");
                assert_eq!(v["content"], "file contents");
            }
            other => panic!("expected Structured, got {other:?}"),
        }
    }

    #[test]
    fn user_role_mixed_blocks_all_preserved_with_line_uuid() {
        let line = concat!(
            r#"{"type":"user","message":{"role":"user","content":["#,
            r#"{"type":"text","text":"echo"},"#,
            r#"{"type":"tool_result","tool_use_id":"t1","content":"r"}"#,
            r#"]},"uuid":"22222222-2222-2222-2222-222222222222","isReplay":true}"#,
            "\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:user", "structured:user"],
            "both text and tool_result preserved (each as structured:user)"
        );
        for e in &ev {
            if let OutputEvent::Structured { json, .. } = e {
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["uuid"], "22222222-2222-2222-2222-222222222222");
            }
        }
    }

    #[test]
    fn tool_fixture_user_tool_result_survives_suppression() {
        let events = decode_all(TOOL_JSONL.as_bytes());
        assert_eq!(
            tags(&events),
            vec![
                "queued:ack-unavailable",
                "structured:thinking",
                "tool:Read",
                "structured:user",
                "structured:thinking",
                "text",
                "usage",
                "done",
            ],
            "a tool_result (user-role) must be preserved as structured:user even after suppression"
        );
    }

    #[test]
    fn wrap_user_turn_escapes_quotes_newlines_unicode() {
        let bytes = wrap_user_turn("a\"b\nc\\d 한글 😀", Uuid::new_v4()); // hangul-ok
        let line = String::from_utf8(bytes).unwrap();
        assert_eq!(
            line.matches('\n').count(),
            1,
            "an inner newline must not leak raw"
        );
        assert!(line.contains("\\\""), "quote escape");
        assert!(line.contains("\\n"), "newline escape");
        assert!(line.contains("\\\\d"), "backslash escape");
        let v: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(v["message"]["content"][0]["text"], "a\"b\nc\\d 한글 😀"); // hangul-ok
        assert_eq!(v["type"], "user");
    }

    // ── S15 B2: ClaudeStreamDecoder(stream-json → OutputEvent) ────────────────────
    //
    // Canonical = measured fixtures (claude 2.1.170 capture).

    const TEXT_JSONL: &str = include_str!("fixtures/claude_text.jsonl");
    const TOOL_JSONL: &str = include_str!("fixtures/claude_tool.jsonl");
    // ADR-0079: transcript fixture for resume seed — reproduces the measured envelope, includes 1 sidechain turn.
    const TRANSCRIPT_JSONL: &str = include_str!("fixtures/claude_transcript.jsonl");

    /// ★env serialization lock★: `CLAUDE_CONFIG_DIR` is process-global, so tests touching it race under cargo's parallel run; each takes this lock and makes set→call→remove atomic (poison ignored so another test's panic does not pollute this one).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The returned temp base directory (the value to pass as `CLAUDE_CONFIG_DIR`) is cleaned up by the caller on drop.
    ///   Call only while holding ENV_LOCK (path derivation depends on env).
    fn write_temp_transcript(cwd: &std::path::Path, sid: Uuid, content: &[u8]) -> PathBuf {
        // Unique base (pid+sid) — so file paths do not overlap even in parallel (the lock protects only the env derivation section).
        let base = std::env::temp_dir().join(format!(
            "engram-transcript-test-{}-{sid}",
            std::process::id()
        ));
        std::env::set_var("CLAUDE_CONFIG_DIR", &base);
        let path = transcript_path(cwd, sid).expect("temp transcript path");
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir temp transcript dir");
        std::fs::write(&path, content).expect("write temp transcript");
        base
    }

    fn tags(events: &[OutputEvent]) -> Vec<String> {
        events
            .iter()
            .map(|e| match e {
                OutputEvent::TerminalBytes(_) => "terminal".to_string(),
                OutputEvent::TextDelta { .. } => "text".to_string(),
                OutputEvent::ToolCall { name, .. } => format!("tool:{name}"),
                OutputEvent::ToolResult { outcome, .. } => format!("tool-result:{outcome:?}"),
                OutputEvent::Usage { .. } => "usage".to_string(),
                OutputEvent::MessageDone { .. } => "done".to_string(),
                OutputEvent::TurnEnd { .. } => "turn-end".to_string(),
                OutputEvent::Error(_) => "error".to_string(),
                OutputEvent::Structured { kind, .. } => format!("structured:{kind}"),
                OutputEvent::QueuedInput(op) => format!(
                    "queued:{}",
                    match op {
                        QueuedInputEvent::Queued { .. } => "queued",
                        QueuedInputEvent::CancelRequested { .. } => "cancel-requested",
                        QueuedInputEvent::CancelAnswered { .. } => "cancel-answered",
                        QueuedInputEvent::CancelFailed { .. } => "cancel-failed",
                        QueuedInputEvent::HandedOver { .. } => "handed-over",
                        QueuedInputEvent::Delivered { .. } => "delivered",
                        QueuedInputEvent::Dropped { .. } => "dropped",
                        QueuedInputEvent::AckUnavailable { .. } => "ack-unavailable",
                    }
                ),
            })
            .collect()
    }

    fn decode_all(bytes: &[u8]) -> Vec<OutputEvent> {
        let mut d = ClaudeStreamDecoder::new();
        let mut out = d.decode(bytes);
        out.extend(d.flush());
        out
    }

    #[test]
    fn text_fixture_maps_to_text_then_usage_done() {
        // text.jsonl: Warning (non-JSON) · rate_limit → skip / assistant[text "hello"] / result(usage).
        //   ★The system/init of 2.1.170 has no capability list → one ack-unavailable decision (ADR-0231)★.
        let events = decode_all(TEXT_JSONL.as_bytes());
        assert_eq!(
            tags(&events),
            vec!["queued:ack-unavailable", "text", "usage", "done"]
        );

        match &events[1] {
            OutputEvent::TextDelta {
                text, message_id, ..
            } => {
                assert_eq!(text, "hello");
                assert_eq!(message_id.as_deref(), Some("msg_01QDurZCCdyuXSWuV5NwWr6c"));
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
        match &events[2] {
            OutputEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(*input_tokens, 17095);
                assert_eq!(*output_tokens, 4);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn tool_fixture_maps_thinking_tooluse_toolresult_text() {
        // tool.jsonl measured sequence:
        //  9  assistant[thinking]           → structured:thinking
        //  10 assistant[tool_use Read]      → tool:Read  (same msg id as 9)
        //  11 user[tool_result]             → structured:user
        //  16 assistant[thinking]           → structured:thinking
        //  17 assistant[text]               → text
        //  18 result(usage)                 → usage, done
        //  2  system/init (no capability list — 2.1.170) → queued:ack-unavailable (ADR-0231)
        // (system/status · rate_limit · thinking_tokens meta lines are all skipped)
        let events = decode_all(TOOL_JSONL.as_bytes());
        assert_eq!(
            tags(&events),
            vec![
                "queued:ack-unavailable",
                "structured:thinking",
                "tool:Read",
                "structured:user",
                "structured:thinking",
                "text",
                "usage",
                "done",
            ]
        );

        let tool = events
            .iter()
            .find(|e| matches!(e, OutputEvent::ToolCall { .. }))
            .unwrap();
        match tool {
            OutputEvent::ToolCall {
                name,
                args_json,
                id,
                message_id,
                category,
                ..
            } => {
                assert_eq!(name, "Read");
                assert_eq!(*category, ToolCategory::Read);
                assert_eq!(id.as_deref(), Some("toolu_01LDdR9FU6CFjgEKeLPF1x1D"));
                assert_eq!(message_id.as_deref(), Some("msg_01DXXosoarwv9i1cBXa8wVXJ"));
                let v: serde_json::Value = serde_json::from_str(args_json).unwrap();
                assert_eq!(
                    v["file_path"],
                    "I:\\Engram\\apps\\engram-dashboard\\package.json"
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn chunk_boundary_invariance_arbitrary_offsets() {
        for fixture in [TEXT_JSONL, TOOL_JSONL] {
            let whole = tags(&decode_all(fixture.as_bytes()));
            for chunk_size in [1usize, 3, 7, 64, 4096] {
                let mut d = ClaudeStreamDecoder::new();
                let mut ev = Vec::new();
                for c in fixture.as_bytes().chunks(chunk_size) {
                    ev.extend(d.decode(c));
                }
                ev.extend(d.flush());
                assert_eq!(
                    tags(&ev),
                    whole,
                    "sequence mismatch at chunk_size={chunk_size}"
                );
            }
        }
    }

    #[test]
    fn utf8_multibyte_split_across_chunks_is_recovered() {
        let line = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"안녕 😀 world"}]}}"#; // hangul-ok
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');

        let whole = decode_all(&bytes);
        let whole_text = match &whole[0] {
            OutputEvent::TextDelta { text, .. } => text.clone(),
            other => panic!("expected TextDelta, got {other:?}"),
        };
        assert_eq!(whole_text, "안녕 😀 world"); // hangul-ok

        let mut d = ClaudeStreamDecoder::new();
        let mut ev = Vec::new();
        for b in &bytes {
            ev.extend(d.decode(std::slice::from_ref(b)));
        }
        ev.extend(d.flush());
        match &ev[0] {
            OutputEvent::TextDelta { text, .. } => assert_eq!(text, "안녕 😀 world"), // hangul-ok
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn non_json_and_meta_lines_are_skipped_without_panic() {
        let input = concat!(
            "Warning: no stdin data received in 3s, proceeding without it.\n",
            // An init without a capability list emits the ack-unavailable decision event (ADR-0231 — a separate entry measures it).
            "{\"type\":\"system\",\"subtype\":\"init\",\"capabilities\":[\"msg_lifecycle_v1\"]}\n",
            "{\"type\":\"rate_limit_event\",\"rate_limit_info\":{}}\n",
            "\n",
            "not json at all {{{\n",
            "{\"type\":\"unknown_future_line\"}\n",
        );
        let events = decode_all(input.as_bytes());
        assert!(
            events.is_empty(),
            "meta and non-JSON lines are all skipped: {events:?}"
        );
    }

    // ── Usage pickup (take_usage) — the interpretation rules themselves are measured by the `usage.rs` tests ──

    fn take_usage(d: &mut ClaudeStreamDecoder) -> Vec<UsageObservation> {
        crate::transport::OutputDecoder::take_usage(d)
    }

    const RATE_LIMIT_NEW_SHAPE: &str = concat!(
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour","#,
        r#""utilization":0.55,"resetsAt":1790425800,"unifiedWindows":{"five_hour":{"utilization":0.55,"#,
        r#""resetsAt":1790425800},"seven_day":{"utilization":0.83,"resetsAt":1790856000}}}}"#,
        "\n",
    );

    #[test]
    fn rate_limit_event_is_collected_without_output_events() {
        let mut d = ClaudeStreamDecoder::new();
        // Even if the chunk boundary falls in the middle of a line, pickup happens only from complete lines.
        let (head, tail) = RATE_LIMIT_NEW_SHAPE.as_bytes().split_at(40);
        assert!(d.decode(head).is_empty());
        assert!(
            take_usage(&mut d).is_empty(),
            "must not pick up from an incomplete line"
        );
        assert!(
            d.decode(tail).is_empty(),
            "usage does not go out as an output event"
        );

        let got = take_usage(&mut d);
        assert_eq!(got.len(), 1, "{got:?}");
        let obs = &got[0];
        assert_eq!(obs.source, crate::usage::UsageSource::Passive);
        let five = obs.five_hour.expect("5-hour window");
        assert_eq!(
            five.used_pct,
            Some(55.0),
            "the floating-point error of 0.55 × 100 must not remain"
        );
        assert_eq!(five.resets_at, Some(1_790_425_800));
        assert_eq!(obs.weekly.and_then(|w| w.used_pct), Some(83.0));
        assert!(take_usage(&mut d).is_empty(), "must be empty after taking");
    }

    /// The fixture's old shape (no utilization) carries only the reset, and events of other lines stay as is.
    #[test]
    fn fixture_rate_limit_line_yields_reset_only_observation() {
        let mut d = ClaudeStreamDecoder::new();
        let mut events = d.decode(TEXT_JSONL.as_bytes());
        events.extend(d.flush());
        assert_eq!(
            tags(&events),
            vec!["queued:ack-unavailable", "text", "usage", "done"]
        );

        let got = take_usage(&mut d);
        assert_eq!(got.len(), 1, "{got:?}");
        let five = got[0].five_hour.expect("5-hour window");
        assert_eq!(
            five.used_pct, None,
            "the old shape has no utilization — it is not 0"
        );
        assert_eq!(five.resets_at, Some(1_782_567_000));
        assert_eq!(got[0].weekly, None);
    }

    #[test]
    fn unterminated_rate_limit_line_is_collected_on_flush() {
        let mut d = ClaudeStreamDecoder::new();
        let line = RATE_LIMIT_NEW_SHAPE.trim_end();
        assert!(d.decode(line.as_bytes()).is_empty());
        assert!(take_usage(&mut d).is_empty());
        assert!(d.flush().is_empty());
        assert_eq!(take_usage(&mut d).len(), 1);
    }

    #[test]
    fn empty_rate_limit_info_yields_no_observation() {
        let mut d = ClaudeStreamDecoder::new();
        assert!(d
            .decode(b"{\"type\":\"rate_limit_event\",\"rate_limit_info\":{}}\n")
            .is_empty());
        assert!(take_usage(&mut d).is_empty());
    }

    /// Limits of past records are not current values — the transcript restore path does not pick them up (no events either).
    #[test]
    fn transcript_rate_limit_line_yields_no_event() {
        let events = parse_transcript_events(RATE_LIMIT_NEW_SHAPE);
        assert!(
            tags(&events).iter().all(|t| t == "done"),
            "rate_limit_event does not become an event: {:?}",
            tags(&events)
        );
    }

    /// Behind a blocked wrapper, the same line does not become an observation, and output events pass through as is.
    #[test]
    fn blocked_gate_hides_claude_usage_but_not_output() {
        use crate::transport::OutputDecoder;
        let mut gated = crate::usage::UsageGate::blocking(Box::new(ClaudeStreamDecoder::new()));
        let mut events = gated.decode(RATE_LIMIT_NEW_SHAPE.as_bytes());
        events.extend(gated.decode(TEXT_JSONL.as_bytes()));
        events.extend(gated.flush());
        assert_eq!(
            tags(&events),
            vec!["queued:ack-unavailable", "text", "usage", "done"]
        );
        assert!(gated.take_usage().is_empty());
    }

    #[test]
    fn empty_and_newline_only_chunks() {
        let mut d = ClaudeStreamDecoder::new();
        assert!(d.decode(b"").is_empty());
        assert!(d.decode(b"\n").is_empty());
        assert!(d.decode(b"\n\n\n").is_empty());
        assert!(d.flush().is_empty());
    }

    #[test]
    fn flush_processes_trailing_line_without_newline() {
        let mut d = ClaudeStreamDecoder::new();
        let line = br#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"tail"}]}}"#;
        assert!(
            d.decode(line).is_empty(),
            "nothing comes out before the newline"
        );
        let ev = d.flush();
        assert_eq!(tags(&ev), vec!["text"]);
    }

    #[test]
    fn result_without_usage_emits_only_done() {
        let ev = decode_all(b"{\"type\":\"result\",\"subtype\":\"success\"}\n");
        assert_eq!(tags(&ev), vec!["done"]);
    }

    #[test]
    fn buffer_overflow_resets_and_emits_error() {
        // FIX-A (1): when huge input without a newline (>4MB) comes, the buffer is discarded and 1 Error event is emitted (OOM prevention).
        let mut d = ClaudeStreamDecoder::new();
        let huge = vec![b'x'; MAX_BUFFER_BYTES + 1];
        let ev = d.decode(&huge);
        assert_eq!(
            tags(&ev),
            vec!["error"],
            "overflow → 1 Error + buffer reset"
        );
        assert!(d.buffer.is_empty(), "buffer reset after overflow");

        let tail_then_newline = b"garbage-tail-continues{\"type\":\"assistant\"}\n";
        let ev_tail = d.decode(tail_then_newline);
        assert!(
            ev_tail.is_empty(),
            "the polluted line's tail is discarded whole up to the newline — 0 events: {ev_tail:?}"
        );

        let line = b"{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n";
        let ev2 = d.decode(line);
        assert_eq!(
            tags(&ev2),
            vec!["text"],
            "normal line recovery after overflow"
        );
    }

    #[test]
    fn buffer_overflow_tail_with_valid_json_fragment_does_not_forge_events() {
        // FIX-A (2): a valid JSON fragment mixed into the overflow line's tail is also discarded whole up to the next newline (no fake events).
        let mut d = ClaudeStreamDecoder::new();

        let huge = vec![b'x'; MAX_BUFFER_BYTES + 1];
        let ev = d.decode(&huge);
        assert_eq!(tags(&ev), vec!["error"], "overflow → Error");

        let tail = concat!(
            "still-part-of-poisoned-line{\"type\":\"result\",\"subtype\":\"success\"}\n",
            "{\"type\":\"assistant\",\"message\":{\"id\":\"m2\",\"content\":[{\"type\":\"text\",\"text\":\"recovered\"}]}}\n",
        );
        let ev2 = d.decode(tail.as_bytes());
        assert_eq!(
            tags(&ev2),
            vec!["text"],
            "a valid JSON fragment in the polluted tail must not leak as a fake event — only the next normal line recovers"
        );
        match &ev2[0] {
            OutputEvent::TextDelta { text, .. } => assert_eq!(text, "recovered"),
            other => panic!("expected recovered TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn buffer_overflow_resync_spans_multiple_chunks_without_newline() {
        // FIX-A: even if the polluted line's tail spans several chunks (without a newline), the resync state is kept and all of it is discarded.
        let mut d = ClaudeStreamDecoder::new();
        let huge = vec![b'x'; MAX_BUFFER_BYTES + 1];
        assert_eq!(tags(&d.decode(&huge)), vec!["error"]);

        assert!(d.decode(b"tail-part-1").is_empty());
        assert!(d.decode(b"tail-part-2{\"type\":\"result\"}").is_empty());
        let ev = d.decode(b"final-tail\n{\"type\":\"result\",\"subtype\":\"success\"}\n");
        assert_eq!(
            tags(&ev),
            vec!["done"],
            "normal result recovery after resync ends"
        );
    }

    #[test]
    fn multiple_blocks_in_one_line_expand_in_order() {
        let line = concat!(
            r#"{"type":"assistant","message":{"id":"m1","content":["#,
            r#"{"type":"text","text":"first"},"#,
            r#"{"type":"tool_use","id":"t1","name":"Bash","input":{"cmd":"ls"}}"#,
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["text", "tool:Bash"]);
    }

    // ── FIX-B: malformed block contract (no fake formal events) ──────────────────────────

    #[test]
    fn tool_use_without_name_preserved_as_structured_not_empty_toolcall() {
        // FIX-B: a tool_use without a string name does not build an empty-name ToolCall but is preserved as Structured.
        let line = concat!(
            r#"{"type":"assistant","message":{"id":"m1","content":["#,
            r#"{"type":"tool_use","id":"t1","input":{"cmd":"ls"}}"#,
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["structured:tool_use"],
            "tool_use without name → no empty ToolCall, Structured preserved"
        );
        match &ev[0] {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "tool_use");
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["id"], "t1");
                assert_eq!(v["input"]["cmd"], "ls");
                assert!(
                    v.get("name").is_none(),
                    "the original's missing name is preserved"
                );
            }
            other => panic!("expected Structured, got {other:?}"),
        }
    }

    #[test]
    fn tool_use_with_non_string_name_preserved_as_structured() {
        // FIX-B: when name is not a string (schema deviation), as_str() also fails → Structured preserved.
        let line = concat!(
            r#"{"type":"assistant","message":{"id":"m1","content":["#,
            r#"{"type":"tool_use","id":"t1","name":123,"input":{}}"#,
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["structured:tool_use"]);
    }

    // ── ADR-0239: tool name → neutral category ─────────────────────────────────────────────

    /// ★The category is set by the `tool_use` branch through the name table★ — every table word · the `mcp__` prefix · unknown names (`Other`) · exact match (`Other` if the case differs) are measured through the translator path.
    // ADR-0239
    #[test]
    fn tool_use_name_maps_to_its_neutral_category() {
        for (name, want) in [
            ("Read", ToolCategory::Read),
            ("NotebookRead", ToolCategory::Read),
            ("Grep", ToolCategory::Search),
            ("Glob", ToolCategory::Search),
            ("LS", ToolCategory::List),
            ("Edit", ToolCategory::Edit),
            ("MultiEdit", ToolCategory::Edit),
            ("Write", ToolCategory::Edit),
            ("NotebookEdit", ToolCategory::Edit),
            ("Bash", ToolCategory::Command),
            ("PowerShell", ToolCategory::Command),
            ("BashOutput", ToolCategory::Command),
            ("KillShell", ToolCategory::Command),
            ("KillBash", ToolCategory::Command),
            ("WebFetch", ToolCategory::Web),
            ("WebSearch", ToolCategory::Web),
            ("Task", ToolCategory::Agent),
            ("Agent", ToolCategory::Agent),
            ("mcp__x__y", ToolCategory::Mcp),
            ("TodoWrite", ToolCategory::Other),
            ("SomeFutureTool", ToolCategory::Other),
            ("read", ToolCategory::Other),
            ("bash", ToolCategory::Other),
            ("Mcp__x__y", ToolCategory::Other),
        ] {
            let line = format!(
                "{}\n",
                serde_json::json!({"type": "assistant", "message": {"id": "m1", "content": [
                    {"type": "tool_use", "id": "t1", "name": name, "input": {}}
                ]}})
            );
            match decode_all(line.as_bytes()).as_slice() {
                [OutputEvent::ToolCall { category, .. }] => assert_eq!(*category, want, "{name}"),
                other => panic!("{name}: not a single ToolCall: {other:?}"),
            }
        }
    }

    #[test]
    fn text_block_without_text_field_is_skipped() {
        // FIX-B: a text block without a string text is skipped instead of an empty TextDelta (no information loss → dropped silently).
        let line = concat!(
            r#"{"type":"assistant","message":{"id":"m1","content":["#,
            r#"{"type":"text"},"#,
            r#"{"type":"text","text":"kept"}"#,
            "]}}\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["text"],
            "blocks without text are skipped, only normal text blocks remain"
        );
        match &ev[0] {
            OutputEvent::TextDelta { text, .. } => assert_eq!(text, "kept"),
            other => panic!("expected TextDelta 'kept', got {other:?}"),
        }
    }

    // ── FIX-C: result.is_error → Error + MessageDone (Error first) ───────────────────

    #[test]
    fn result_is_error_emits_error_before_done() {
        // FIX-C: a result line carrying is_error:true → Error is emitted before MessageDone.
        let line = concat!(
            r#"{"type":"result","subtype":"error_during_execution",""#,
            r#"is_error":true,"result":"API rate limit exceeded"}"#,
            "\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["error", "done"],
            "is_error → Error first, MessageDone later"
        );
        match &ev[0] {
            OutputEvent::Error(msg) => {
                assert!(
                    msg.contains("error_during_execution"),
                    "subtype included: {msg}"
                );
                assert!(
                    msg.contains("API rate limit exceeded"),
                    "result text included: {msg}"
                );
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn result_error_subtype_without_is_error_flag_still_emits_error() {
        // FIX-C: even without the is_error flag, if the subtype is of the error family (starts_with "error"), Error is emitted.
        let line = r#"{"type":"result","subtype":"error_max_turns"}"#.to_string() + "\n";
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["error", "done"]);
    }

    /// Whether it is the one end of an interrupted turn — `TurnEnd{turn_id: None, outcome: Interrupted}`.
    fn is_interrupted_end(event: &OutputEvent) -> bool {
        matches!(
            event,
            OutputEvent::TurnEnd {
                turn_id: None,
                outcome: TurnOutcome::Interrupted
            }
        )
    }

    // The two below are a hand-built old CLI shape (`subtype:"interrupted"`) — the measured shape is measured by the `interrupt_s1` fixture.
    // ADR-0238
    #[test]
    fn result_interrupted_subtype_ends_the_turn_as_interrupted_without_an_error() {
        // FIX-E regression: a turn the user normally aborted with Esc (subtype:"interrupted") — not an error.
        let line = r#"{"type":"result","subtype":"interrupted"}"#.to_string() + "\n";
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["turn-end"],
            "interrupted is not an error → one interrupted end without Error"
        );
        assert!(is_interrupted_end(&ev[0]), "{ev:?}");
    }

    #[test]
    fn result_interrupted_subtype_with_is_error_false_ends_the_turn_as_interrupted() {
        // FIX-E regression: an interrupted with explicit is_error:false is also one interrupted end without Error.
        let line =
            r#"{"type":"result","subtype":"interrupted","is_error":false}"#.to_string() + "\n";
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["turn-end"]);
        assert!(is_interrupted_end(&ev[0]), "{ev:?}");
    }

    #[test]
    fn result_success_subtype_emits_only_done_no_error() {
        // FIX-C regression: a normal result (subtype=success, no is_error) emits no Error.
        let line = r#"{"type":"result","subtype":"success"}"#.to_string() + "\n";
        let ev = decode_all(line.as_bytes());
        assert_eq!(
            tags(&ev),
            vec!["done"],
            "a normal result gives only done without Error"
        );
    }

    #[test]
    fn result_error_with_usage_orders_usage_error_done() {
        // FIX-C + Usage order: with usage and is_error, the order is Usage → Error → MessageDone.
        let line = concat!(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"#,
            r#""usage":{"input_tokens":10,"output_tokens":2}}"#,
            "\n",
        );
        let ev = decode_all(line.as_bytes());
        assert_eq!(tags(&ev), vec!["usage", "error", "done"]);
    }

    // ── ADR-0079: `.jsonl` transcript → past events (pure function parse_transcript_events) ────────

    #[test]
    fn transcript_maps_conversation_turns_and_skips_meta_and_sidechain() {
        // Fixture lines ↔ expected sequence:
        //     user "첫 질문"                   → structured:user // hangul-ok
        //     assistant thinking               → structured:thinking
        //     assistant tool_use Read          → tool:Read
        //     user tool_result                 → structured:user
        //     assistant [text + tool_use Write]→ text, tool:Write
        //     (sidechain assistant text        → excluded)
        //     assistant text "최종 답변"        → text // hangul-ok
        //     result(usage) success            → usage, done
        let events = parse_transcript_events(TRANSCRIPT_JSONL);
        assert_eq!(
            tags(&events),
            vec![
                "structured:user",
                "structured:thinking",
                "tool:Read",
                "structured:user",
                "text",
                "tool:Write",
                "text",
                "usage",
                "done",
            ],
            "only conversation turns mapped + both parts of the multi-block turn mapped + meta and sidechain excluded: {events:?}"
        );

        let mb_text_idx = events
            .iter()
            .position(
                |e| matches!(e, OutputEvent::TextDelta { text, .. } if text == "확인했습니다"), // hangul-ok
            )
            .expect("the text block of the multi-block turn must be mapped");
        match &events[mb_text_idx + 1] {
            OutputEvent::ToolCall { name, id, .. } => {
                assert_eq!(
                    name, "Write",
                    "the 2nd block of the same message must continue as tool_use Write"
                );
                assert_eq!(id.as_deref(), Some("toolu_2"));
            }
            other => {
                panic!("the 2nd block of the multi-block turn must be a ToolCall, got {other:?}")
            }
        }

        let tool = events
            .iter()
            .find(|e| matches!(e, OutputEvent::ToolCall { .. }))
            .unwrap();
        match tool {
            OutputEvent::ToolCall {
                name,
                id,
                args_json,
                ..
            } => {
                assert_eq!(name, "Read");
                assert_eq!(id.as_deref(), Some("toolu_1"));
                let v: serde_json::Value = serde_json::from_str(args_json).unwrap();
                assert_eq!(v["file_path"], "C:\\Users\\X\\proj\\a.txt");
            }
            _ => unreachable!(),
        }

        match &events[0] {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["text"], "첫 질문"); // hangul-ok
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    // ── FIX (2026-08-17): a transcript without result lines is closed with a synthetic MessageDone ──────────
    //
    // ★Prevents★: a real `.jsonl` has no `result` lines (0 in the 12 recent files); without closing, the structured slot reads `!turnDone && items>0` as streaming right after resume and the waiting indicator (Wait …) shows permanently.
    #[test]
    fn transcript_without_result_line_is_closed_with_synthetic_done() {
        // Measured shape, abbreviated: ends with user → assistant and has no result line (including the meta line last-prompt).
        let jsonl = concat!(
            r#"{"isSidechain":false,"type":"user","message":{"role":"user","content":[{"type":"text","text":"question"}]},"uuid":"aaaa1111-1111-1111-1111-111111111111"}"#,
            "\n",
            r#"{"isSidechain":false,"type":"assistant","message":{"id":"msg_A","role":"assistant","content":[{"type":"text","text":"answer"}]},"uuid":"bbbb2222-2222-2222-2222-222222222222"}"#,
            "\n",
            r#"{"type":"last-prompt","prompt":"question"}"#,
            "\n",
        );
        let events = parse_transcript_events(jsonl);
        assert_eq!(
            tags(&events),
            vec!["structured:user", "text", "done"],
            "a transcript without result is closed with a synthetic MessageDone: {events:?}"
        );
    }

    #[test]
    fn transcript_ending_with_result_line_is_not_double_closed() {
        // The fixture ends with a result line → it is already done, so no synthetic part is appended (prevents a duplicate turn boundary).
        let events = parse_transcript_events(TRANSCRIPT_JSONL);
        assert_eq!(
            tags(&events).iter().filter(|t| *t == "done").count(),
            1,
            "done is not appended to an already-closed transcript: {events:?}"
        );
    }

    #[test]
    fn transcript_empty_or_meta_only_yields_no_events() {
        assert!(parse_transcript_events("").is_empty());
        let meta_only = concat!(
            "{\"type\":\"summary\",\"summary\":\"x\"}\n",
            "{\"type\":\"file-history-snapshot\",\"messageId\":\"m\",\"snapshot\":{}}\n",
            "\n",
        );
        assert!(
            parse_transcript_events(meta_only).is_empty(),
            "a meta-only transcript gives 0 events"
        );
    }

    #[test]
    fn transcript_sidechain_line_is_filtered_even_with_valid_content() {
        let line = concat!(
            r#"{"isSidechain":true,"type":"assistant","message":{"id":"m","role":"assistant",""#,
            r#"content":[{"type":"text","text":"sub"}]},"uuid":"x"}"#,
            "\n",
        );
        assert!(
            parse_transcript_events(line).is_empty(),
            "sidechain turns are excluded (sub-agent output is not restored)"
        );
    }

    #[test]
    fn project_slug_matches_claude_encoding() {
        use std::path::Path;
        assert_eq!(
            project_slug(Path::new(
                r"C:\Users\X\AppData\Local\Temp\engram-resume-test"
            )),
            "C--Users-X-AppData-Local-Temp-engram-resume-test"
        );
        assert_eq!(project_slug(Path::new(r"C:\")), "C--");
        assert_eq!(
            project_slug(Path::new(r"I:\Engram_Workspace\a")),
            "I--Engram-Workspace-a"
        );
    }

    #[test]
    fn transcript_path_uses_projects_slug_and_sid() {
        let sid = Uuid::parse_str("d75b7f40-a13a-4cf3-b872-e4d5ba2cec55").unwrap();
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("CLAUDE_CONFIG_DIR", r"C:\claude-cfg");
        let p = transcript_path(std::path::Path::new(r"C:\proj\a"), sid).unwrap();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let s = p.to_string_lossy().replace('/', "\\");
        assert!(
            s.contains("projects"),
            "goes through the projects directory: {s}"
        );
        assert!(s.contains("C--proj-a"), "cwd slug: {s}");
        assert!(
            s.ends_with(&format!("{sid}.jsonl")),
            "file name = <sid>.jsonl: {s}"
        );
    }

    #[test]
    fn read_transcript_events_missing_file_is_empty() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("CLAUDE_CONFIG_DIR", r"C:\nonexistent-claude-cfg-xyz");
        let ev =
            read_transcript_events(std::path::Path::new(r"C:\no-such-proj-xyz"), Uuid::new_v4());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        assert!(ev.is_empty(), "no file gives an empty Vec (no seed)");
    }

    #[test]
    fn read_transcript_events_small_file_reads_whole() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cwd = std::path::Path::new(r"C:\proj\small");
        let sid = Uuid::new_v4();
        let base = write_temp_transcript(cwd, sid, TRANSCRIPT_JSONL.as_bytes());
        let ev = read_transcript_events(cwd, sid);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(
            tags(&ev),
            tags(&parse_transcript_events(TRANSCRIPT_JSONL)),
            "a small file is read whole, same as pure parsing (no partial line discarded)"
        );
    }

    #[test]
    fn read_transcript_events_empty_file_is_empty() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cwd = std::path::Path::new(r"C:\proj\empty");
        let sid = Uuid::new_v4();
        let base = write_temp_transcript(cwd, sid, b"");
        let ev = read_transcript_events(cwd, sid);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&base);
        assert!(ev.is_empty(), "empty file → empty Vec");
    }

    /// ★The setup in which the test actually proves the discard (FIX B)★: the seek point (= len-4MB) falls exactly on **the start byte of the inner valid JSON object inside the trap line**, so the leading fragment before the first `\n` after the seek (= the
    ///   drop target) is **a valid conversation line that parses by itself** (a phantom "text" event). Without the discard the phantom appears, with it it disappears — removing the drop fails the test (the old setup's fragment was non-JSON, skipped regardless,
    ///   proving nothing). ★Paired counter-proof★: it also asserts that parsing without the drop (`parse_transcript_events(leading)`) does produce the phantom, pinning the premise.
    #[test]
    fn read_transcript_events_tail_drops_partial_leading_line_no_phantom() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cwd = std::path::Path::new(r"C:\proj\big");
        let sid = Uuid::new_v4();

        // ★Trap line★ = [prefix junk][inner valid JSON line]. The prefix junk is before the seek point, so it does not
        //   come into the read buffer.
        let junk_prefix =
            br#"{"type":"summary","summary":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}garbage"#;
        let inner_valid = br#"{"type":"assistant","message":{"id":"phantom","content":[{"type":"text","text":"phantom"}]}}"#;
        let real = r#"{"isSidechain":false,"type":"assistant","message":{"id":"real","content":[{"type":"text","text":"the real answer"}]}}"#.as_bytes();

        // The seek point is a fixed distance (4MB) from EOF. So if the byte count "inner_valid start ~ EOF" is set to exactly 4MB,
        //   the seek point = the inner_valid start byte. inner_valid start ~ EOF = inner + \n + real + \n + trail.
        let inner_start_to_eof = TRANSCRIPT_TAIL_BYTES as usize;
        let trail_len = inner_start_to_eof - (inner_valid.len() + 1 + real.len() + 1);
        let mut content: Vec<u8> =
            Vec::with_capacity(inner_start_to_eof + junk_prefix.len() + 4096);
        content.extend_from_slice(junk_prefix);
        content.extend_from_slice(inner_valid);
        content.push(b'\n');
        content.extend_from_slice(real);
        content.push(b'\n');
        let pad_line =
            b"{\"type\":\"file-history-snapshot\",\"messageId\":\"pad\",\"snapshot\":{}}\n";
        let trail_start = content.len();
        while content.len() - trail_start < trail_len {
            content.extend_from_slice(pad_line);
        }
        content.truncate(trail_start + trail_len);

        let leading = String::from_utf8(inner_valid.to_vec()).unwrap();
        assert_eq!(
            tags(&parse_transcript_events(&leading)),
            // The tail "done" = the synthetic MessageDone closing the restored history (the FIX above). It is a constant tail unrelated to this test's subject
            //   (discarding the leading fragment) — the only concern is whether a phantom "text" appears before it.
            vec!["text", "done"],
            "premise: the leading fragment (drop target) must itself be a valid line that synthesizes a phantom text"
        );

        let base = write_temp_transcript(cwd, sid, &content);
        let ev = read_transcript_events(cwd, sid);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(
            tags(&ev),
            // The tail "done" = closing of the restored history (synthetic MessageDone). The phantom must not be before it.
            vec!["text", "done"],
            "partial first line discarded → only intact lines without a phantom: {ev:?}"
        );
        match &ev[0] {
            OutputEvent::TextDelta { text, .. } => assert_eq!(text, "the real answer"),
            other => panic!("expected TextDelta the real answer, got {other:?}"),
        }
    }

    #[test]
    fn read_transcript_events_tail_utf8_split_is_safe() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cwd = std::path::Path::new(r"C:\proj\utf8");
        let sid = Uuid::new_v4();

        // Fill the padding with Hangul (3 bytes/char) lines so the seek point (len-4MB) misses a multibyte character boundary.
        let pad_line = "{\"type\":\"summary\",\"summary\":\"가나다라마\"}\n".as_bytes(); // hangul-ok
        let mut content: Vec<u8> = Vec::with_capacity((TRANSCRIPT_TAIL_BYTES as usize) + 4096);
        while (content.len() as u64) < TRANSCRIPT_TAIL_BYTES + 1 {
            content.extend_from_slice(pad_line);
        }
        content.push(b'x'); // shift the boundary by 1 byte to induce a cut in the middle of a character.
        content.extend_from_slice(pad_line); // the fragment that becomes the truncated first line.
        content.extend_from_slice(
            r#"{"isSidechain":false,"type":"assistant","message":{"id":"real","content":[{"type":"text","text":"안녕"}]}}"#.as_bytes(), // hangul-ok
        );
        content.push(b'\n');

        let base = write_temp_transcript(cwd, sid, &content);
        let ev = read_transcript_events(cwd, sid);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(
            tags(&ev),
            // The tail "done" = closing of the restored history (synthetic MessageDone) — a constant tail unrelated to the cut-boundary subject.
            vec!["text", "done"],
            "only intact lines even at a UTF-8 cut boundary: {ev:?}"
        );
    }

    #[test]
    fn read_transcript_events_exact_boundary_no_seek() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let cwd = std::path::Path::new(r"C:\proj\boundary");
        let sid = Uuid::new_v4();

        let first = r#"{"isSidechain":false,"type":"assistant","message":{"id":"first","content":[{"type":"text","text":"boundary"}]}}"#;
        let mut content: Vec<u8> = Vec::with_capacity(TRANSCRIPT_TAIL_BYTES as usize);
        content.extend_from_slice(first.as_bytes());
        content.push(b'\n');
        let pad = b"{\"type\":\"summary\",\"summary\":\"p\"}\n";
        while (content.len() as u64) < TRANSCRIPT_TAIL_BYTES {
            content.extend_from_slice(pad);
        }
        content.truncate(TRANSCRIPT_TAIL_BYTES as usize);
        assert_eq!(content.len() as u64, TRANSCRIPT_TAIL_BYTES);

        let base = write_temp_transcript(cwd, sid, &content);
        let ev = read_transcript_events(cwd, sid);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(
            ev.first()
                .map(|e| matches!(e, OutputEvent::TextDelta { text, .. } if text == "boundary")),
            Some(true),
            "exactly at the cap, parse from the first line without seeking: {ev:?}"
        );
    }

    // ── ADR-0084: proof of the `--resume <sid>` assembly on reactivation (Resume) (dispatch level) ──────────────────
    //
    // Reinforces the weak Shell test of ADR-0083 regression ③: that reactivation passes the controlled sid as `--resume` is asserted as the backend contract through **the public dispatch `build_command_spec`** (no real claude process). manager.spawn_agent(Resume)
    // calls this dispatch, so this proves the reactivation respawn passes the resume sid losslessly (ADR-0008).
    #[test]
    fn build_command_spec_resume_emits_resume_flag_with_sid() {
        use crate::backend::build_command_spec;

        let sid = Uuid::new_v4();
        let spec = build_command_spec(
            &terminal(vec![]),
            SpawnMode::Resume,
            Some(sid),
            None,
            PathBuf::from("."),
            vec![],
            None,
            &LaunchArtifacts::default(),
        );

        let pos = spec
            .args
            .iter()
            .position(|x| x == "--resume")
            .expect("Resume-mode dispatch must assemble --resume");
        assert_eq!(
            spec.args.get(pos + 1).map(String::as_str),
            Some(sid.to_string().as_str()),
            "the controlled sid (uuid) must come right after --resume (ADR-0008 lossless resume)"
        );
        // Resume mode must not use the fresh-only --session-id (avoids sid collision — ADR-0076).
        assert!(
            !spec.args.iter().any(|x| x == "--session-id"),
            "--session-id (fresh) must not leak in Resume mode"
        );
    }

    // ── ADR-0231: queued input — lifecycle · cancel response · ack decision · turn error · attachment restore ─────────────────
    //
    // Canonical = Phase 0 measured fixtures (claude 2.1.280); only `result_error_handbuilt` was hand-built (README).

    const LIFECYCLE_M1: &str = include_str!("fixtures/lifecycle_m1.jsonl");
    const CANCEL_M3: &str = include_str!("fixtures/cancel_m3.jsonl");
    const DRAIN_M7: &str = include_str!("fixtures/drain_m7.jsonl");
    const SLASH_M13: &str = include_str!("fixtures/slash_m13.jsonl");
    const RESULT_ERROR: &str = include_str!("fixtures/result_error_handbuilt.jsonl");
    const TRANSCRIPT_QUEUED_M5: &str = include_str!("fixtures/transcript_queued_m5.jsonl");

    fn queued_ops(events: &[OutputEvent]) -> Vec<QueuedInputEvent> {
        events
            .iter()
            .filter_map(|e| match e {
                OutputEvent::QueuedInput(op) => Some(op.clone()),
                _ => None,
            })
            .collect()
    }

    fn delivered(id: &str) -> QueuedInputEvent {
        QueuedInputEvent::Delivered { id: id.into() }
    }

    fn dropped(id: &str, cause: DropCause) -> QueuedInputEvent {
        QueuedInputEvent::Dropped {
            id: id.into(),
            cause,
        }
    }

    fn answered(id: &str, removed: bool) -> QueuedInputEvent {
        QueuedInputEvent::CancelAnswered {
            id: id.into(),
            removed,
        }
    }

    /// The n-th line (1-based) of a fixture — used to feed the decoder one line at a time with a newline attached.
    fn fixture_line(fixture: &str, n: usize) -> String {
        format!("{}\n", fixture.lines().nth(n - 1).expect("fixture line"))
    }

    /// A core with turn observation wired as in production — this backend's classifier + the registered shared table.
    fn observed_core(
        table: &Arc<crate::turn::TurnObservations>,
    ) -> (crate::output_core::OutputCore, AgentId, u32) {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, StatusSink};

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let (id, epoch) = (Uuid::new_v4(), 7);
        table.register(id, epoch);
        let core = OutputCore::new(
            id,
            epoch,
            Arc::new(NoopStatus),
            TurnWiring::new(Arc::clone(table), classify_turn),
        );
        (core, id, epoch)
    }

    fn feed(core: &crate::output_core::OutputCore, d: &mut ClaudeStreamDecoder, bytes: &str) {
        for ev in d.decode(bytes.as_bytes()) {
            core.emit(ev);
        }
    }

    /// The lifecycle key is `command_uuid`, not the same line's `uuid` (newly drawn by the CLI). A, written while idle, also emits a lifecycle (M1).
    // ADR-0231
    #[test]
    fn lifecycle_lines_translate_keyed_by_command_uuid() {
        let first: serde_json::Value =
            serde_json::from_str(LIFECYCLE_M1.lines().next().unwrap()).expect("fixture json");
        assert_ne!(first["uuid"], first["command_uuid"], "fixture premise");

        assert_eq!(
            queued_ops(&decode_all(LIFECYCLE_M1.as_bytes())),
            vec![
                delivered("f53b8bb3-9dfd-4aa3-a57e-ad6659376e84"),
                delivered("44fcee51-376b-4de7-9532-e9608f9de0a0"),
            ]
        );
    }

    /// Six state translations — `queued`·`completed` have no event. Lines without an id and unknown states have none either.
    // ADR-0231
    #[test]
    fn all_six_lifecycle_states_translate() {
        let line = |state: &str| {
            format!(
                r#"{{"type":"command_lifecycle","command_uuid":"u1","state":"{state}","uuid":"x","session_id":"s"}}"#
            ) + "\n"
        };
        let cases = [
            ("queued", vec![]),
            ("started", vec![delivered("u1")]),
            ("completed", vec![]),
            ("cancelled", vec![dropped("u1", DropCause::Unknown)]),
            ("discarded", vec![dropped("u1", DropCause::AgentEnded)]),
            ("refused", vec![dropped("u1", DropCause::Rejected)]),
            ("someday_new_state", vec![]),
        ];
        for (state, want) in cases {
            assert_eq!(
                queued_ops(&decode_all(line(state).as_bytes())),
                want,
                "{state}"
            );
        }
        // A line without an id has no event and does not open acks either (`Unknown` as is).
        let no_id = "{\"type\":\"command_lifecycle\",\"state\":\"started\"}\n";
        let empty_id =
            "{\"type\":\"command_lifecycle\",\"command_uuid\":\"\",\"state\":\"started\"}\n";
        for bad in [no_id, empty_id] {
            let ack = Arc::new(DeliveryAck::new());
            let mut d = ClaudeStreamDecoder::with_delivery_ack(Arc::clone(&ack));
            assert!(queued_ops(&d.decode(bad.as_bytes())).is_empty(), "{bad}");
            assert_eq!(
                ack.state(),
                crate::types::DeliveryAckState::Unknown,
                "{bad}"
            );
        }
    }

    /// Vendor drift — a lifecycle line with a changed key (no `command_uuid`) or an unknown state word does not open acks (`Unknown`); the following init without the capability then takes today's path with one `Unavailable` + `AckUnavailable{[]}`.
    // ADR-0231
    #[test]
    fn an_unrecognisable_lifecycle_line_does_not_make_the_ack_available() {
        use crate::types::DeliveryAckState;

        let renamed_key =
            "{\"type\":\"command_lifecycle\",\"cmd_uuid\":\"u1\",\"state\":\"started\"}\n";
        let unknown_state =
            "{\"type\":\"command_lifecycle\",\"command_uuid\":\"u1\",\"state\":\"someday_new_state\"}\n";
        for drift in [renamed_key, unknown_state] {
            let ack = Arc::new(DeliveryAck::new());
            let mut d = ClaudeStreamDecoder::with_delivery_ack(Arc::clone(&ack));
            assert!(
                queued_ops(&d.decode(drift.as_bytes())).is_empty(),
                "{drift}"
            );
            assert_eq!(ack.state(), DeliveryAckState::Unknown, "{drift}");

            let bare_init = "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s\"}\n";
            assert_eq!(
                queued_ops(&d.decode(bare_init.as_bytes())),
                vec![QueuedInputEvent::AckUnavailable {
                    delivered: Vec::new()
                }],
                "{drift}"
            );
            assert_eq!(ack.state(), DeliveryAckState::Unavailable, "{drift}");
        }
    }

    /// In every measured fixture one `started` line = one `Delivered` (same `command_uuid`, same order) — folding at tool boundaries (M1) · cancel (M3) · drain turn (M7) · slash command mid-turn (M13).
    // ADR-0231
    #[test]
    fn every_started_line_in_every_fixture_is_one_delivered() {
        for (name, fixture) in [
            ("m1", LIFECYCLE_M1),
            ("m3", CANCEL_M3),
            ("m7", DRAIN_M7),
            ("m13", SLASH_M13),
            ("handbuilt", RESULT_ERROR),
        ] {
            let started: Vec<QueuedInputEvent> = fixture
                .lines()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .filter(|v| v["type"] == "command_lifecycle" && v["state"] == "started")
                .map(|v| delivered(v["command_uuid"].as_str().unwrap()))
                .collect();
            let got: Vec<QueuedInputEvent> = queued_ops(&decode_all(fixture.as_bytes()))
                .into_iter()
                .filter(|op| matches!(op, QueuedInputEvent::Delivered { .. }))
                .collect();
            assert!(!started.is_empty(), "{name}: fixture premise");
            assert_eq!(got, started, "{name}");
        }
    }

    /// M3 — (a) our cancel removed it: the `cancelled` lifecycle comes before the response `true` · (c2) the cancel arrived before the text: after the response `false` that text was folded in and delivered (`started`). The response's uuid is taken off the request id.
    // ADR-0231
    #[test]
    fn cancel_m3_translates_answers_and_lifecycle_in_line_order() {
        assert_eq!(
            queued_ops(&decode_all(CANCEL_M3.as_bytes())),
            vec![
                delivered("a7c61eaa-2e6f-4190-b4ca-33e948fd9d2f"),
                dropped("6d3edeb9-e7a6-48a6-826b-5e3e7d9a91dc", DropCause::Unknown),
                answered("6d3edeb9-e7a6-48a6-826b-5e3e7d9a91dc", true),
                delivered("154094b1-7dfd-4f71-887a-5daaa6542d5b"),
                answered("635fe486-548a-4132-bbed-c0d59ac10a85", false),
                delivered("635fe486-548a-4132-bbed-c0d59ac10a85"),
            ]
        );
    }

    /// Error response → `CancelFailed` · success without a value → 「could not remove」 · a response that is not our cancel and an empty uuid → none.
    // ADR-0231
    #[test]
    fn a_cancel_error_response_is_cancel_failed_and_foreign_responses_are_ignored() {
        let error = r#"{"type":"control_response","response":{"subtype":"error","request_id":"cancel:u1","error":"boom"}}"#;
        assert_eq!(
            queued_ops(&decode_all(format!("{error}\n").as_bytes())),
            vec![QueuedInputEvent::CancelFailed { id: "u1".into() }]
        );
        let no_value = r#"{"type":"control_response","response":{"subtype":"success","request_id":"cancel:u1","response":{}}}"#;
        assert_eq!(
            queued_ops(&decode_all(format!("{no_value}\n").as_bytes())),
            vec![answered("u1", false)]
        );
        for foreign in [
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"interrupt-1","response":{"cancelled":true}}}"#,
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"cancel:","response":{"cancelled":true}}}"#,
            r#"{"type":"control_response","response":{"subtype":"success","response":{"cancelled":true}}}"#,
        ] {
            let ev = decode_all(format!("{foreign}\n").as_bytes());
            assert!(ev.is_empty(), "{foreign} → {ev:?}");
        }
    }

    /// Cancel line byte golden (TRD §5-4 — the shape measurement M3 accepted) + the response to that request id comes back with the same uuid.
    // ADR-0231
    #[test]
    fn cancel_line_bytes_golden_and_its_answer_round_trips() {
        let id = "6d3edeb9-e7a6-48a6-826b-5e3e7d9a91dc";
        let line = cancel_line(id);
        assert_eq!(
            String::from_utf8(line.clone()).unwrap(),
            concat!(
                r#"{"type":"control_request","request_id":"cancel:6d3edeb9-e7a6-48a6-826b-5e3e7d9a91dc","#,
                r#""request":{"subtype":"cancel_async_message","message_uuid":"6d3edeb9-e7a6-48a6-826b-5e3e7d9a91dc"}}"#,
                "\n"
            )
        );

        let sent: serde_json::Value = serde_json::from_slice(&line).unwrap();
        let answer = serde_json::json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": sent["request_id"], "response": {"cancelled": true}},
        });
        assert_eq!(
            queued_ops(&decode_all(format!("{answer}\n").as_bytes())),
            vec![answered(id, true)]
        );
    }

    /// Only JSON mode reports session classification, and its cancel fragment is exactly the line of the golden above — terminal mode is today's path.
    // ADR-0231
    #[test]
    fn only_the_json_mode_reports_session_classified_with_the_cancel_line() {
        let id = "6d3edeb9-e7a6-48a6-826b-5e3e7d9a91dc";
        match mid_turn_policy(&json(vec![])) {
            MidTurnPolicy::SessionClassified { cancel_line: line } => {
                assert_eq!(line(id), cancel_line(id))
            }
            other => panic!("JSON mode must be session-classified: {other:?}"),
        }
        assert!(matches!(
            mid_turn_policy(&terminal(vec![])),
            MidTurnPolicy::None
        ));
    }

    /// The first lifecycle line opens acks as `Available` — init comes only after the turn starts, so this is first (M1·M4); a later init changes nothing.
    // ADR-0231
    #[test]
    fn the_first_lifecycle_line_makes_delivery_ack_available_before_init() {
        use crate::types::DeliveryAckState;

        let ack = Arc::new(DeliveryAck::new());
        let mut d = ClaudeStreamDecoder::with_delivery_ack(Arc::clone(&ack));
        assert!(d
            .decode(fixture_line(LIFECYCLE_M1, 1).as_bytes())
            .is_empty());
        assert_eq!(ack.state(), DeliveryAckState::Available);

        let rest: String = LIFECYCLE_M1
            .lines()
            .skip(1)
            .map(|l| format!("{l}\n"))
            .collect();
        let ops = queued_ops(&d.decode(rest.as_bytes()));
        assert!(
            !ops.iter()
                .any(|op| matches!(op, QueuedInputEvent::AckUnavailable { .. })),
            "{ops:?}"
        );
        assert_eq!(ack.state(), DeliveryAckState::Available);
    }

    /// An init with `msg_lifecycle_v1` in the capability list → `Available`, no event.
    // ADR-0231
    #[test]
    fn an_init_with_the_lifecycle_capability_is_available() {
        use crate::types::DeliveryAckState;

        let ack = Arc::new(DeliveryAck::new());
        let mut d = ClaudeStreamDecoder::with_delivery_ack(Arc::clone(&ack));
        let init = fixture_line(LIFECYCLE_M1, 3);
        assert!(init.contains("\"subtype\":\"init\""), "fixture premise");
        assert!(d.decode(init.as_bytes()).is_empty());
        assert_eq!(ack.state(), DeliveryAckState::Available);
    }

    /// An init without the capability (2.1.170) → `Unavailable` + `AckUnavailable{[]}` **once per incarnation** — the same decoder seeing it again or another decoder holding the same Arc gives no second.
    // ADR-0231
    #[test]
    fn an_init_without_the_capability_is_unavailable_with_exactly_one_ack_unavailable() {
        use crate::types::DeliveryAckState;

        let init = TEXT_JSONL
            .lines()
            .find(|l| l.contains("\"subtype\":\"init\""))
            .map(|l| format!("{l}\n"))
            .expect("2.1.170 init");
        let ack = Arc::new(DeliveryAck::new());
        let mut d = ClaudeStreamDecoder::with_delivery_ack(Arc::clone(&ack));
        let mut other = ClaudeStreamDecoder::with_delivery_ack(Arc::clone(&ack));
        let mut ops = queued_ops(&d.decode(init.as_bytes()));
        ops.extend(queued_ops(&d.decode(init.as_bytes())));
        ops.extend(queued_ops(&other.decode(init.as_bytes())));
        assert_eq!(
            ops,
            vec![QueuedInputEvent::AckUnavailable {
                delivered: Vec::new()
            }]
        );
        assert_eq!(ack.state(), DeliveryAckState::Unavailable);
    }

    /// Once `Available`, an init without the capability does not revert it and there is no event.
    // ADR-0231
    #[test]
    fn a_capability_less_init_after_available_changes_nothing() {
        use crate::types::DeliveryAckState;

        let ack = Arc::new(DeliveryAck::new());
        let mut d = ClaudeStreamDecoder::with_delivery_ack(Arc::clone(&ack));
        d.decode(fixture_line(LIFECYCLE_M1, 1).as_bytes());
        let bare_init = "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s\"}\n";
        assert!(d.decode(bare_init.as_bytes()).is_empty());
        assert_eq!(ack.state(), DeliveryAckState::Available);
    }

    /// The turn error signal is only the `Error` a failed `result` emitted right before `MessageDone` — measured with real decoder output (diverging prefixes break here). `success`·`interrupted` ends and a line-buffer overflow `Error` give 0 signals.
    // ADR-0231
    #[test]
    fn only_the_failed_result_error_is_a_turn_error_signal() {
        let classify = ClaudeBackend.turn_classifier();
        let failed_signals = |events: &[OutputEvent]| {
            events
                .iter()
                .filter(|e| classify(e) == Some(TurnSignal::Failed))
                .count()
        };

        // The two hand-built failed turns: `success`+`is_error:true` · `error_during_execution`.
        let events = decode_all(RESULT_ERROR.as_bytes());
        assert_eq!(failed_signals(&events), 2);
        for (i, e) in events.iter().enumerate() {
            if classify(e) == Some(TurnSignal::Failed) {
                assert!(
                    matches!(events.get(i + 1), Some(OutputEvent::MessageDone { .. })),
                    "after a turn error, that turn's end must come right away: {events:?}"
                );
            }
        }

        assert_eq!(failed_signals(&decode_all(TEXT_JSONL.as_bytes())), 0);
        let interrupted = "{\"type\":\"result\",\"subtype\":\"interrupted\"}\n";
        assert_eq!(failed_signals(&decode_all(interrupted.as_bytes())), 0);

        let mut d = ClaudeStreamDecoder::new();
        let overflow = d.decode(&vec![b'x'; MAX_BUFFER_BYTES + 1]);
        assert!(
            matches!(overflow.as_slice(), [OutputEvent::Error(_)]),
            "{overflow:?}"
        );
        assert_eq!(classify(&overflow[0]), None);
    }

    /// A failed `result` → core turn table `last_end_failed` true (stop after error) → the next clean turn clears it.
    // ADR-0231
    #[test]
    fn a_failed_result_halts_until_the_next_clean_turn() {
        let table = Arc::new(crate::turn::TurnObservations::new());
        let (core, id, epoch) = observed_core(&table);
        let mut d = ClaudeStreamDecoder::new();

        let first_turn: String = RESULT_ERROR
            .lines()
            .take(4)
            .map(|l| format!("{l}\n"))
            .collect();
        feed(&core, &mut d, &first_turn);
        let after_error = table.get(id, epoch).expect("observed");
        assert!(after_error.last_end_failed);
        assert!(!after_error.in_turn);

        feed(&core, &mut d, LIFECYCLE_M1);
        let after_clean = table.get(id, epoch).expect("observed");
        assert!(!after_clean.last_end_failed);
        assert!(!after_clean.in_turn);
    }

    /// ★A line-buffer overflow `Error` outside a turn does not fold the next clean turn into an error end★ — counted as a turn error the table would hold it, fold the next clean `result` into a failure and stop mail until the user succeeds once.
    // ADR-0231
    #[test]
    fn a_line_overflow_error_outside_a_turn_does_not_fail_the_next_clean_turn() {
        let table = Arc::new(crate::turn::TurnObservations::new());
        let (core, id, epoch) = observed_core(&table);
        let mut d = ClaudeStreamDecoder::new();

        for ev in d.decode(&vec![b'x'; MAX_BUFFER_BYTES + 1]) {
            core.emit(ev);
        }
        // The newline ending the overflowed line's tail — normal lines from then on.
        feed(&core, &mut d, "\n");
        assert!(!table.get(id, epoch).expect("observed").last_end_failed);

        feed(&core, &mut d, LIFECYCLE_M1);
        assert!(!table.get(id, epoch).expect("observed").last_end_failed);
    }

    /// Drain turn (M7): a core made idle by the previous `result` becomes in-turn right at the queued item's `started` (=`Delivered`), without waiting for that turn's `system/init` (0.6–0.9 s later).
    // ADR-0231
    #[test]
    fn a_drain_turn_started_puts_the_core_in_turn_before_its_init() {
        let table = Arc::new(crate::turn::TurnObservations::new());
        let (core, id, epoch) = observed_core(&table);
        let mut d = ClaudeStreamDecoder::new();
        let in_turn = || table.get(id, epoch).expect("observed").in_turn;

        for n in 1..=11 {
            feed(&core, &mut d, &fixture_line(DRAIN_M7, n));
        }
        assert!(!in_turn(), "after the previous turn's `result` = idle");
        feed(&core, &mut d, &fixture_line(DRAIN_M7, 12));
        assert!(
            !in_turn(),
            "the previous turn's `completed` is not a signal"
        );

        let started = fixture_line(DRAIN_M7, 13);
        assert!(started.contains("\"state\":\"started\""), "fixture premise");
        feed(&core, &mut d, &started);
        assert!(in_turn(), "the drain turn's `started` = in a turn");
        assert!(
            fixture_line(DRAIN_M7, 14).contains("\"subtype\":\"init\""),
            "fixture premise — that turn's init comes after `started`"
        );
    }

    /// Resume: the `attachment{queued_command}` of folded-in input → a user bubble in that place (uuid = `source_uuid`, body = the text of `prompt`) · `queue-operation` lines discarded · no registry events in the seed (M5).
    // ADR-0231
    #[test]
    fn a_queued_command_attachment_restores_as_a_user_bubble_in_place() {
        let events = parse_transcript_events(TRANSCRIPT_QUEUED_M5);
        assert_eq!(
            tags(&events),
            vec![
                "structured:user",
                "structured:thinking",
                "tool:Bash",
                "structured:user",
                "structured:user",
                "structured:thinking",
                "text",
                "done",
            ]
        );
        let OutputEvent::Structured { json, .. } = &events[4] else {
            panic!("expected the restored bubble: {:?}", events[4]);
        };
        let bubble: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(bubble["type"], "text");
        assert_eq!(
            bubble["text"],
            "When you reply, also include the word PINEAPPLE."
        );
        assert_eq!(bubble["uuid"], "44fcee51-376b-4de7-9532-e9608f9de0a0");
        assert!(queued_ops(&events).is_empty());
    }

    /// A transcript with only `queue-operation` → 0 · modes other than `prompt` and other attachments → 0 · a string `prompt` is accepted too.
    // ADR-0231
    #[test]
    fn queue_operations_and_non_prompt_attachments_are_skipped() {
        let queue_ops: String = TRANSCRIPT_QUEUED_M5
            .lines()
            .filter(|l| l.contains("\"type\":\"queue-operation\""))
            .map(|l| format!("{l}\n"))
            .collect();
        assert!(!queue_ops.is_empty(), "fixture premise");
        assert!(parse_transcript_events(&queue_ops).is_empty());

        for skipped in [
            r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":[{"type":"text","text":"x"}],"source_uuid":"u1","commandMode":"task-notification"}}"#,
            r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":[{"type":"text","text":"x"}],"source_uuid":"u1"}}"#,
            r#"{"type":"attachment","attachment":{"type":"prompt_snapshot","prompt":"x"}}"#,
        ] {
            assert!(parse_transcript_events(skipped).is_empty(), "{skipped}");
        }

        let string_prompt = r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"hi","source_uuid":"u1","commandMode":"prompt"}}"#;
        let events = parse_transcript_events(string_prompt);
        let OutputEvent::Structured { kind, json } = &events[0] else {
            panic!("{events:?}");
        };
        assert_eq!(kind, "user");
        let bubble: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(
            (bubble["text"].as_str(), bubble["uuid"].as_str()),
            (Some("hi"), Some("u1"))
        );
    }

    /// The two streams do not mix — live does not move attachment lines (live gets the echo separately; moving would make two bubbles), and the transcript does not translate lifecycles, cancel responses or init (the seed bypasses the registry door).
    // ADR-0231
    #[test]
    fn live_and_transcript_line_kinds_do_not_cross() {
        let attachment = TRANSCRIPT_QUEUED_M5
            .lines()
            .find(|l| l.contains("\"type\":\"attachment\""))
            .map(|l| format!("{l}\n"))
            .expect("fixture premise");
        assert!(decode_all(attachment.as_bytes()).is_empty());

        let live_only: String = CANCEL_M3
            .lines()
            .filter(|l| {
                l.contains("\"type\":\"command_lifecycle\"")
                    || l.contains("\"type\":\"control_response\"")
                    || l.contains("\"subtype\":\"init\"")
            })
            .map(|l| format!("{l}\n"))
            .collect();
        assert!(!live_only.is_empty(), "fixture premise");
        assert!(parse_transcript_events(&live_only).is_empty());
    }

    // ── ADR-0240: text streaming — partial messages · dropping complete text ─────────────────────────────

    /// One top-level `stream_event` line (newline included).
    fn stream_line(event: serde_json::Value) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "type": "stream_event",
                "event": event,
                "session_id": "s1",
                "parent_tool_use_id": null,
                "uuid": "u-stream"
            })
        )
    }

    fn message_start(id: &str) -> String {
        stream_line(serde_json::json!({
            "type": "message_start",
            "message": { "id": id, "type": "message", "role": "assistant", "content": [] }
        }))
    }

    fn block_start(index: u64, kind: &str) -> String {
        stream_line(serde_json::json!({
            "type": "content_block_start",
            "index": index,
            "content_block": { "type": kind }
        }))
    }

    fn text_delta(index: u64, text: &str) -> String {
        stream_line(serde_json::json!({
            "type": "content_block_delta",
            "index": index,
            "delta": { "type": "text_delta", "text": text }
        }))
    }

    fn block_stop(index: u64) -> String {
        stream_line(serde_json::json!({ "type": "content_block_stop", "index": index }))
    }

    /// A complete `assistant` line holding only one block — the vendor sends it before that block's `content_block_stop`.
    fn completed(id: &str, block: serde_json::Value) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "type": "assistant",
                "message": { "id": id, "role": "assistant", "content": [block] },
                "parent_tool_use_id": null
            })
        )
    }

    fn completed_text(id: &str, text: &str) -> String {
        completed(id, serde_json::json!({ "type": "text", "text": text }))
    }

    const RESULT_LINE: &str = "{\"type\":\"result\",\"subtype\":\"success\"}\n";

    fn text_events(events: &[OutputEvent]) -> Vec<(String, Option<String>)> {
        events
            .iter()
            .filter_map(|e| match e {
                OutputEvent::TextDelta {
                    text, message_id, ..
                } => Some((text.clone(), message_id.clone())),
                _ => None,
            })
            .collect()
    }

    fn decode_lines(lines: &[String]) -> Vec<OutputEvent> {
        decode_all(lines.concat().as_bytes())
    }

    /// One decode() per line — the shape of the real pump reading one delta line at a time (no merging of neighboring deltas).
    fn decode_each(lines: &[String]) -> Vec<OutputEvent> {
        let mut d = ClaudeStreamDecoder::new();
        let mut out: Vec<OutputEvent> = lines.iter().flat_map(|l| d.decode(l.as_bytes())).collect();
        out.extend(d.flush());
        out
    }

    /// 1: a `TextDelta` per text delta (message id = that of `message_start`) · the complete text of that block does not come out.
    // ADR-0240
    #[test]
    fn streamed_text_deltas_carry_the_message_id_and_the_completed_text_is_dropped() {
        let events = decode_each(&[
            message_start("m1"),
            block_start(0, "text"),
            text_delta(0, "Hel"),
            text_delta(0, "lo"),
            completed_text("m1", "Hello"),
            block_stop(0),
        ]);
        assert_eq!(
            text_events(&events),
            vec![
                ("Hel".to_string(), Some("m1".to_string())),
                ("lo".to_string(), Some("m1".to_string())),
            ]
        );
        assert_eq!(tags(&events), vec!["text", "text"]);
    }

    /// 2: a text block without deltas · a complete line of another message · a complete line arriving after stop → the full complete text (nothing lost).
    // ADR-0240
    #[test]
    fn a_text_block_that_was_not_streamed_falls_back_to_the_completed_text() {
        let no_deltas = decode_lines(&[
            message_start("m1"),
            block_start(0, "text"),
            completed_text("m1", "whole"),
            block_stop(0),
        ]);
        assert_eq!(
            text_events(&no_deltas),
            vec![("whole".to_string(), Some("m1".to_string()))]
        );

        let other_message = decode_lines(&[
            message_start("m1"),
            block_start(0, "text"),
            text_delta(0, "a"),
            completed_text("m2", "b"),
        ]);
        assert_eq!(
            text_events(&other_message)
                .into_iter()
                .map(|(t, _)| t)
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );

        // If the vendor changes the order (complete after stop), it shows overlapped — better than losing.
        let stopped_first = decode_lines(&[
            message_start("m1"),
            block_start(0, "text"),
            text_delta(0, "a"),
            block_stop(0),
            completed_text("m1", "a"),
        ]);
        assert_eq!(
            text_events(&stopped_first)
                .into_iter()
                .map(|(t, _)| t)
                .collect::<Vec<_>>(),
            vec!["a", "a"]
        );

        // A stream that does not go through `partial` (transcript) always emits the complete text.
        let transcript = parse_transcript_events(
            &[
                message_start("m1"),
                block_start(0, "text"),
                text_delta(0, "a"),
                completed_text("m1", "a"),
            ]
            .concat(),
        );
        assert_eq!(tags(&transcript), vec!["text", "done"]);
    }

    /// 3: `tool_use` gives one `ToolCall` from the complete line · `input_json_delta` gives 0 events.
    // ADR-0240
    #[test]
    fn a_tool_use_comes_once_from_the_completed_line_and_its_input_deltas_are_silent() {
        let input_delta = |partial_json: &str| {
            stream_line(serde_json::json!({
                "type": "content_block_delta",
                "index": 1,
                "delta": { "type": "input_json_delta", "partial_json": partial_json }
            }))
        };
        let events = decode_lines(&[
            message_start("m1"),
            block_start(0, "text"),
            text_delta(0, "Let me run it."),
            completed_text("m1", "Let me run it."),
            block_stop(0),
            block_start(1, "tool_use"),
            input_delta("{\"command\":"),
            input_delta("\"echo hi\"}"),
            completed(
                "m1",
                serde_json::json!({
                    "type": "tool_use", "id": "toolu_1", "name": "Bash",
                    "input": { "command": "echo hi" }
                }),
            ),
            block_stop(1),
        ]);
        assert_eq!(tags(&events), vec!["text", "tool:Bash"]);
    }

    /// 4: thinking and signature deltas are discarded, and the complete thinking block is one `Structured{thinking}` as today.
    // ADR-0240
    #[test]
    fn thinking_deltas_are_ignored_and_the_completed_thinking_block_is_structured() {
        let delta = |kind: &str, key: &str| {
            stream_line(serde_json::json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": kind, key: "x" }
            }))
        };
        let events = decode_lines(&[
            message_start("m1"),
            block_start(0, "thinking"),
            delta("thinking_delta", "thinking"),
            delta("signature_delta", "signature"),
            completed(
                "m1",
                serde_json::json!({ "type": "thinking", "thinking": "x", "signature": "sig" }),
            ),
            block_stop(0),
        ]);
        assert_eq!(tags(&events), vec!["structured:thinking"]);
    }

    /// 5: every `stream_event` with `parent_tool_use_id` is discarded — its `message_start` does not change the parent's tracking either.
    // ADR-0240
    #[test]
    fn sub_agent_stream_events_are_dropped_and_do_not_reset_the_parent_tracking() {
        let child = |event: serde_json::Value| {
            format!(
                "{}\n",
                serde_json::json!({
                    "type": "stream_event",
                    "event": event,
                    "parent_tool_use_id": "toolu_parent"
                })
            )
        };
        let events = decode_lines(&[
            message_start("m1"),
            block_start(0, "text"),
            text_delta(0, "parent"),
            child(serde_json::json!({
                "type": "message_start",
                "message": { "id": "m-child", "content": [] }
            })),
            child(serde_json::json!({
                "type": "content_block_start", "index": 0, "content_block": { "type": "text" }
            })),
            child(serde_json::json!({
                "type": "content_block_delta", "index": 0,
                "delta": { "type": "text_delta", "text": "child" }
            })),
            completed_text("m1", "parent"),
        ]);
        assert_eq!(
            text_events(&events),
            vec![("parent".to_string(), Some("m1".to_string()))]
        );
    }

    /// 6: message auxiliary lines · unknown `event.type` → 0 events (no `MessageDone` · `TurnEnd` · `Structured`).
    // ADR-0240
    #[test]
    fn message_bookkeeping_stream_events_emit_nothing() {
        let events = decode_lines(&[
            message_start("m1"),
            stream_line(serde_json::json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn" },
                "usage": { "output_tokens": 5 }
            })),
            stream_line(serde_json::json!({ "type": "message_stop" })),
            stream_line(serde_json::json!({ "type": "ping" })),
            stream_line(serde_json::json!({ "type": "some_future_event", "index": 0 })),
            "{\"type\":\"stream_event\"}\n".to_string(),
        ]);
        assert!(events.is_empty(), "{events:?}");
    }

    /// 7: a new `message_start` replaces the tracking — the same block number of a new message after a tool loop is counted anew.
    // ADR-0240
    #[test]
    fn a_new_message_start_replaces_the_tracking() {
        let tool_result = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_1\",\"content\":\"hi\"}]}}\n".to_string();
        let events = decode_each(&[
            message_start("m1"),
            block_start(0, "text"),
            text_delta(0, "first"),
            completed_text("m1", "first"),
            block_stop(0),
            tool_result,
            message_start("m2"),
            block_start(0, "text"),
            completed_text("m2", "not streamed"),
            block_stop(0),
            block_start(1, "text"),
            text_delta(1, "streamed"),
            completed_text("m2", "streamed"),
            block_stop(1),
            // A late complete line of an old message — its id differs from the current tracking, so the full text comes out.
            completed_text("m1", "late"),
        ]);
        assert_eq!(
            text_events(&events),
            vec![
                ("first".to_string(), Some("m1".to_string())),
                ("not streamed".to_string(), Some("m2".to_string())),
                ("streamed".to_string(), Some("m2".to_string())),
                ("late".to_string(), Some("m1".to_string())),
            ]
        );
    }

    /// 8: late deltas arriving after `result` without `message_start` → 0 events.
    // ADR-0240
    #[test]
    fn a_late_delta_after_the_result_emits_nothing() {
        let mut d = ClaudeStreamDecoder::new();
        let turn = [
            message_start("m1"),
            block_start(0, "text"),
            text_delta(0, "done"),
            completed_text("m1", "done"),
            block_stop(0),
            RESULT_LINE.to_string(),
        ]
        .concat();
        assert_eq!(tags(&d.decode(turn.as_bytes())), vec!["text", "done"]);
        let late = [
            text_delta(0, "late"),
            block_start(1, "text"),
            text_delta(1, "late"),
        ]
        .concat();
        assert!(d.decode(late.as_bytes()).is_empty());
    }

    /// 9: even if one `stream_event` line containing Hangul comes split across two chunks (in the middle of a character), there is one `TextDelta`.
    // ADR-0240
    #[test]
    fn a_stream_event_line_split_inside_a_hangul_character_yields_one_delta() {
        let mut d = ClaudeStreamDecoder::new();
        assert!(d
            .decode(
                [message_start("m1"), block_start(0, "text")]
                    .concat()
                    .as_bytes()
            )
            .is_empty());
        let line = text_delta(0, "안녕하세요"); // hangul-ok
        let bytes = line.as_bytes();
        let cut = line.find("녕").expect("hangul") + 1; // hangul-ok
        assert!(
            !line.is_char_boundary(cut),
            "test premise — cuts in the middle of a character"
        );
        let mut events = d.decode(&bytes[..cut]);
        assert!(events.is_empty());
        events.extend(d.decode(&bytes[cut..]));
        assert_eq!(
            text_events(&events),
            vec![("안녕하세요".to_string(), Some("m1".to_string()))] // hangul-ok
        );
    }

    /// 10: even if `stream_event` is mixed into the resume original, the complete text is one copy (`partial: None` skips that line).
    // ADR-0240
    #[test]
    fn stream_events_mixed_into_a_transcript_leave_one_copy_of_the_text() {
        let transcript = [
            message_start("m1"),
            block_start(0, "text"),
            text_delta(0, "Hel"),
            text_delta(0, "lo"),
            completed_text("m1", "Hello"),
            block_stop(0),
            stream_line(serde_json::json!({ "type": "message_stop" })),
        ]
        .concat();
        let events = parse_transcript_events(&transcript);
        assert_eq!(tags(&events), vec!["text", "done"]);
        assert_eq!(
            text_events(&events),
            vec![("Hello".to_string(), Some("m1".to_string()))]
        );
    }

    /// 12: a streamed `TextDelta` is progress · auxiliary lines have no events and so no signals — even if auxiliary lines come after `result`, it stays idle.
    // ADR-0240
    #[test]
    fn streamed_deltas_are_progress_and_bookkeeping_lines_leave_the_turn_closed() {
        let table = Arc::new(crate::turn::TurnObservations::new());
        let (core, id, epoch) = observed_core(&table);
        let mut d = ClaudeStreamDecoder::new();
        let in_turn = || table.get(id, epoch).expect("observed").in_turn;

        let classify = ClaudeBackend.turn_classifier();
        let streamed = d.decode(
            [
                message_start("m1"),
                block_start(0, "text"),
                text_delta(0, "x"),
            ]
            .concat()
            .as_bytes(),
        );
        assert_eq!(tags(&streamed), vec!["text"]);
        assert_eq!(classify(&streamed[0]), Some(TurnSignal::Progress));
        for ev in streamed {
            core.emit(ev);
        }
        assert!(in_turn());

        feed(
            &core,
            &mut d,
            &[block_stop(0), RESULT_LINE.to_string()].concat(),
        );
        assert!(!in_turn());

        let bookkeeping = [
            stream_line(serde_json::json!({ "type": "message_delta", "delta": {} })),
            stream_line(serde_json::json!({ "type": "message_stop" })),
            block_stop(0),
            text_delta(0, "late"),
        ]
        .concat();
        assert!(d.decode(bookkeeping.as_bytes()).is_empty());
        feed(&core, &mut d, &bookkeeping);
        assert!(
            !in_turn(),
            "auxiliary lines and late deltas after the turn end do not turn in-turn back on"
        );
    }

    const PARTIAL_STREAM_P1: &str = include_str!("fixtures/partial_stream_p1.jsonl");

    /// Blocks of the fixture's complete `assistant` lines whose `(message id, block type)` match.
    fn completed_blocks(fixture: &str, block_type: &str) -> Vec<(String, serde_json::Value)> {
        fixture
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["type"] == "assistant")
            .flat_map(|v| {
                let id = v["message"]["id"].as_str().unwrap_or_default().to_string();
                v["message"]["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|b| b["type"] == block_type)
                    .map(move |b| (id.clone(), b))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn fixture_lines(fixture: &str) -> Vec<String> {
        fixture.lines().map(|l| format!("{l}\n")).collect()
    }

    /// Folds like the accumulator — text appends to the last text item, other events one item each by their own tag.
    fn fold_like_accumulator(events: &[OutputEvent]) -> Vec<String> {
        let mut items: Vec<String> = Vec::new();
        let mut last_is_text = false;
        for (e, tag) in events.iter().zip(tags(events)) {
            match e {
                OutputEvent::TextDelta { text, .. } if last_is_text => {
                    items.last_mut().expect("text item").push_str(text)
                }
                OutputEvent::TextDelta { text, .. } => {
                    items.push(format!("text:{text}"));
                    last_is_text = true;
                }
                _ => {
                    items.push(tag);
                    last_is_text = false;
                }
            }
        }
        items
    }

    /// 1 · 3 · 4 · 7 by measurement: per message the join of streamed text = that message's complete text (one copy) · one event per text delta line · a tool once from the complete line · thinking as many as the complete blocks · a new message after folded-in input is one copy too.
    // ADR-0240
    #[test]
    fn the_partial_fixture_streams_every_text_block_exactly_once() {
        let events = decode_each(&fixture_lines(PARTIAL_STREAM_P1));

        let completed_text = completed_blocks(PARTIAL_STREAM_P1, "text");
        let ids: std::collections::BTreeSet<&str> =
            completed_text.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids.len(), 5, "fixture premise — five messages with text");
        for id in ids {
            let expected: String = completed_text
                .iter()
                .filter(|(i, _)| i == id)
                .map(|(_, b)| b["text"].as_str().expect("text"))
                .collect();
            let streamed: String = text_events(&events)
                .into_iter()
                .filter(|(_, m)| m.as_deref() == Some(id))
                .map(|(t, _)| t)
                .collect();
            assert_eq!(streamed, expected, "message {id}");
        }
        let delta_lines = PARTIAL_STREAM_P1
            .lines()
            .filter(|l| l.contains("\"type\":\"text_delta\""))
            .count();
        assert_eq!(text_events(&events).len(), delta_lines);
        assert_eq!(
            text_events(&events).last().map(|(t, _)| t.as_str()),
            Some("INEAPPLE"),
            "the end delta of the new message after folded-in input"
        );

        let non_text: Vec<String> = tags(&events).into_iter().filter(|t| t != "text").collect();
        assert_eq!(
            non_text,
            vec![
                // T1 — text only
                "queued:delivered",
                "structured:user",
                "usage",
                "done",
                // T2 — thinking · text → tool → result → text
                "queued:delivered",
                "structured:user",
                "structured:thinking",
                "tool:Bash",
                "structured:user",
                "usage",
                "done",
                // T3 — thinking · text
                "queued:delivered",
                "structured:user",
                "structured:thinking",
                "usage",
                "done",
                // T5 — B written during a tool, folded → new message_start
                "queued:delivered",
                "structured:user",
                "structured:thinking",
                "tool:Bash",
                "structured:user",
                "structured:user",
                "queued:delivered",
                "structured:thinking",
                "usage",
                "done",
            ]
        );
        assert_eq!(
            non_text
                .iter()
                .filter(|t| *t == "structured:thinking")
                .count(),
            completed_blocks(PARTIAL_STREAM_P1, "thinking").len()
        );
    }

    /// 9 · 14 by measurement: wherever the chunk boundary falls (even mid-Hangul character) the items the accumulator folds are the same, and fed at once the text is the same even if neighboring deltas merge.
    // ADR-0240
    #[test]
    fn the_partial_fixture_folds_to_the_same_items_at_any_chunk_boundary() {
        let per_line = fold_like_accumulator(&decode_each(&fixture_lines(PARTIAL_STREAM_P1)));
        for chunk_size in [1usize, 3, 7, 64, 4096, PARTIAL_STREAM_P1.len()] {
            let mut d = ClaudeStreamDecoder::new();
            let mut events = Vec::new();
            for c in PARTIAL_STREAM_P1.as_bytes().chunks(chunk_size) {
                events.extend(d.decode(c));
            }
            events.extend(d.flush());
            assert_eq!(
                fold_like_accumulator(&events),
                per_line,
                "chunk_size={chunk_size}"
            );
        }
        let whole = decode_all(PARTIAL_STREAM_P1.as_bytes());
        assert!(
            text_events(&whole).len()
                < text_events(&decode_each(&fixture_lines(PARTIAL_STREAM_P1))).len(),
            "fed at once, neighboring deltas merge"
        );
    }

    /// 13: in the captured idle intervals (after each `result` to the next input's `queued`, incl. the 32 s after T1) there is no top-level `stream_event`, those events are not progress signals and the core stays idle.
    // ADR-0240
    #[test]
    fn the_captured_idle_windows_carry_no_stream_events_and_no_progress() {
        let table = Arc::new(crate::turn::TurnObservations::new());
        let (core, id, epoch) = observed_core(&table);
        let mut d = ClaudeStreamDecoder::new();
        let classify = ClaudeBackend.turn_classifier();

        let mut in_window = false;
        let mut windows = 0;
        for line in PARTIAL_STREAM_P1.lines() {
            let value: serde_json::Value = serde_json::from_str(line).expect("fixture json");
            if value["type"] == "command_lifecycle" && value["state"] == "queued" {
                in_window = false;
            }
            let events = d.decode(format!("{line}\n").as_bytes());
            if in_window {
                assert_ne!(
                    value["type"], "stream_event",
                    "a partial line in an idle interval: {line}"
                );
                for e in &events {
                    assert_ne!(classify(e), Some(TurnSignal::Progress), "{e:?}");
                }
            }
            for e in events {
                core.emit(e);
            }
            if in_window {
                assert!(!table.get(id, epoch).expect("observed").in_turn);
            }
            if value["type"] == "result" {
                in_window = true;
                windows += 1;
                assert!(!table.get(id, epoch).expect("observed").in_turn);
            }
        }
        assert_eq!(
            windows, 4,
            "fixture premise — the ends of the four captured turns"
        );
    }

    /// 14: neighboring `TextDelta`s within one decode() merge only with the same `message_id`, and another event in between breaks it.
    // ADR-0240
    #[test]
    fn neighbouring_deltas_merge_only_within_one_decode_and_one_message() {
        let tool = completed(
            "m1",
            serde_json::json!({ "type": "tool_use", "id": "t1", "name": "Bash", "input": {} }),
        );
        let lines = [
            message_start("m1"),
            block_start(0, "text"),
            text_delta(0, "a"),
            text_delta(0, "b"),
            block_stop(0),
            block_start(1, "text"),
            text_delta(1, "c"),
            completed_text("m1", "c"),
            block_stop(1),
            tool,
            text_delta(2, "d"),
            message_start("m2"),
            block_start(0, "text"),
            text_delta(0, "e"),
            text_delta(0, "f"),
        ];
        let merged = decode_lines(&lines);
        assert_eq!(
            text_events(&merged),
            vec![
                ("abc".to_string(), Some("m1".to_string())),
                ("d".to_string(), Some("m1".to_string())),
                ("ef".to_string(), Some("m2".to_string())),
            ]
        );
        assert_eq!(tags(&merged), vec!["text", "tool:Bash", "text", "text"]);

        let separate = decode_each(&lines);
        assert_eq!(text_events(&separate).len(), 6);
        assert_eq!(
            fold_like_accumulator(&merged),
            fold_like_accumulator(&separate)
        );
    }

    /// A message id over the cap (128 bytes) is filtered, not truncated — that message is not streamed and shows as one complete text, and no id is carried in the events; an id within the cap streams as is.
    // ADR-0240
    #[test]
    fn a_message_id_over_the_bound_is_filtered_not_truncated() {
        let long = "m".repeat(MAX_MESSAGE_ID_BYTES + 1);
        let events = decode_each(&[
            message_start(&long),
            block_start(0, "text"),
            text_delta(0, "a"),
            completed_text(&long, "ab"),
            block_stop(0),
        ]);
        assert_eq!(text_events(&events), vec![("ab".to_string(), None)]);

        let at_bound = "m".repeat(MAX_MESSAGE_ID_BYTES);
        let events = decode_each(&[
            message_start(&at_bound),
            block_start(0, "text"),
            text_delta(0, "a"),
            completed_text(&at_bound, "a"),
            block_stop(0),
        ]);
        assert_eq!(
            text_events(&events),
            vec![("a".to_string(), Some(at_bound))]
        );
    }

    /// Reinforcement of 11: existing fixtures without partial lines give the same event sequence with or without partial tracking (whole Debug string compared). Passing existing fixture tests unchanged is the evidence for 11; this measures that premise
    ///   (tracking discards nothing) down to the fields.
    // ADR-0240
    #[test]
    fn fixtures_without_partial_lines_decode_the_same_with_and_without_tracking() {
        for fixture in [
            TEXT_JSONL,
            TOOL_JSONL,
            TRANSCRIPT_JSONL,
            LIFECYCLE_M1,
            CANCEL_M3,
            DRAIN_M7,
            SLASH_M13,
            TRANSCRIPT_QUEUED_M5,
            RESULT_ERROR,
        ] {
            assert!(!fixture.contains("\"stream_event\""), "fixture premise");
            let tracked = decode_all(fixture.as_bytes());
            let ack = DeliveryAck::default();
            let mut untracked = Vec::new();
            for line in fixture.lines() {
                ClaudeStreamDecoder::consume_line(
                    line.as_bytes(),
                    &mut untracked,
                    LineSource::Live(&ack),
                    None,
                    None,
                );
            }
            assert_eq!(format!("{tracked:?}"), format!("{untracked:?}"));
        }
    }

    // ── ADR-0238 · ADR-0243: interrupt — interrupt line · turn-open gate · interrupted `result` · synthetic interrupt line ──────────────────
    //
    // Canonical = `interrupt_s1` captured by the B2 spike (claude 2.1.280 · README table). Line numbers are 1-based.

    const INTERRUPT_S1: &str = include_str!("fixtures/interrupt_s1.jsonl");

    /// A decoder sharing the gate and the interrupt line function reading that gate — the production assembly (`open_spawn`) without the cleaner.
    fn gated_decoder() -> (ClaudeStreamDecoder, InterruptLine) {
        let (decoder, gate) = decoder_with_gate(GateCell::new(None));
        (decoder, interrupt_line(gate))
    }

    fn decoder_with_gate(gate: Arc<GateCell>) -> (ClaudeStreamDecoder, Arc<GateCell>) {
        let slot = Arc::new(OnceLock::from(Arc::clone(&gate)));
        let decoder = ClaudeStreamDecoder::live(Arc::new(DeliveryAck::new()), slot);
        (decoder, gate)
    }

    /// Feeds the fixture to one decoder line by line and collects per line (that line's events, whether the interrupt line function gives a line after it).
    fn replay_with_gate(fixture: &str) -> Vec<(Vec<OutputEvent>, bool)> {
        let (mut decoder, line) = gated_decoder();
        fixture_lines(fixture)
            .iter()
            .map(|l| (decoder.decode(l.as_bytes()), line().is_some()))
            .collect()
    }

    fn interrupted_note_text(event: &OutputEvent) -> Option<String> {
        match event {
            OutputEvent::Structured { kind, json } if kind == INTERRUPTED_KIND => {
                let v: serde_json::Value = serde_json::from_str(json).expect("json");
                Some(v["text"].as_str().expect("text").to_string())
            }
            _ => None,
        }
    }

    /// Interrupt line byte golden (ADR-0238 decision 1) · the response to that request id and the two captured responses have no events (a response is not 「stopped」).
    // ADR-0238
    #[test]
    fn interrupt_line_bytes_golden_and_its_answer_is_not_translated() {
        let id = Uuid::parse_str("f453a7a6-672a-4ed3-ae93-a3372f66d6f2").unwrap();
        let line = interrupt_line_bytes(id);
        assert_eq!(
            String::from_utf8(line.clone()).unwrap(),
            concat!(
                r#"{"type":"control_request","request_id":"interrupt:f453a7a6-672a-4ed3-ae93-a3372f66d6f2","#,
                r#""request":{"subtype":"interrupt"}}"#,
                "\n"
            )
        );

        let sent: serde_json::Value = serde_json::from_slice(&line).unwrap();
        let answer = serde_json::json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": sent["request_id"], "response": {"still_queued": ["u1"]}},
        });
        assert!(decode_all(format!("{answer}\n").as_bytes()).is_empty());
        for n in [135, 183] {
            let captured = fixture_line(INTERRUPT_S1, n);
            assert!(
                captured.contains("\"request_id\":\"interrupt:"),
                "fixture premise {n}"
            );
            assert!(decode_all(captured.as_bytes()).is_empty(), "{n}");
        }
    }

    /// The two interrupted `result`s (`aborted_streaming` while text streams · `aborted_tools` while a tool runs) → `[Usage?, TurnEnd{Interrupted}]` — no `Error`·`MessageDone` · classification `Ended(Other)` · no turn error signal anywhere in the fixture;
    /// the same fixture's normal end is `[Usage, done]` as today.
    // ADR-0238
    #[test]
    fn the_spike_s_interrupted_results_end_the_turn_as_interrupted_without_an_error() {
        let classify = ClaudeBackend.turn_classifier();
        let per_line = replay_with_gate(INTERRUPT_S1);
        let result_lines: Vec<usize> = INTERRUPT_S1
            .lines()
            .enumerate()
            .filter(|(_, l)| {
                serde_json::from_str::<serde_json::Value>(l).unwrap()["type"] == "result"
            })
            .map(|(i, _)| i + 1)
            .collect();
        assert_eq!(result_lines, vec![138, 187, 201], "fixture premise");

        // 138 has usage 0/0, so there is no `Usage`.
        assert_eq!(tags(&per_line[138 - 1].0), vec!["turn-end"]);
        assert_eq!(tags(&per_line[187 - 1].0), vec!["usage", "turn-end"]);
        for n in [138, 187] {
            let end = per_line[n - 1].0.last().unwrap();
            assert!(is_interrupted_end(end), "{n}: {end:?}");
            assert_eq!(
                classify(end),
                Some(TurnSignal::Ended(TurnEndKind::Other)),
                "{n}"
            );
        }
        assert_eq!(tags(&per_line[201 - 1].0), vec!["usage", "done"]);

        let failed = per_line
            .iter()
            .flat_map(|(events, _)| events)
            .filter(|e| classify(e) == Some(TurnSignal::Failed))
            .count();
        assert_eq!(failed, 0, "the interruption was read as a turn error");
    }

    /// The tails of the two interrupted turns line by line — the response gives 0 events · the truncated complete line is streamed text, so 0 (ADR-0240) · tool results are `user` as today · the synthetic line is an interrupted marker · `result` is the
    /// interrupted end · the following `cancelled` is `Dropped` (a no-op on a tombstone — the registry's job).
    // ADR-0238
    // ADR-0243
    #[test]
    fn the_interrupted_tails_translate_line_by_line() {
        let per_line = replay_with_gate(INTERRUPT_S1);
        let expected: [(usize, &[&str]); 11] = [
            (135, &[]),
            (136, &[]),
            (137, &["structured:interrupted"]),
            (138, &["turn-end"]),
            (139, &["queued:dropped"]),
            (183, &[]),
            (184, &[]),
            (185, &["structured:user"]),
            (186, &["structured:interrupted"]),
            (187, &["usage", "turn-end"]),
            (188, &["queued:dropped"]),
        ];
        for (n, want) in expected {
            assert_eq!(tags(&per_line[n - 1].0), want, "line {n}");
        }
    }

    /// S8 — gate trajectory: opening lines = each turn's `started` (2 · 141 · 190) · closing lines = each turn's `result` (138 · 187 · 201); no line opens it outside a turn (the first `queued` · the `cancelled`·`completed` after `result` · the next `queued`).
    // ADR-0238
    #[test]
    fn the_turn_gate_opens_at_each_started_line_and_closes_at_each_result() {
        let per_line = replay_with_gate(INTERRUPT_S1);
        let (mut opened, mut closed, mut was_open) = (Vec::new(), Vec::new(), false);
        for (i, (_, open)) in per_line.iter().enumerate() {
            if *open != was_open {
                if *open {
                    opened.push(i + 1)
                } else {
                    closed.push(i + 1)
                }
            }
            was_open = *open;
        }
        assert_eq!(opened, vec![2, 141, 190]);
        assert_eq!(closed, vec![138, 187, 201]);
        for n in opened {
            assert!(
                fixture_line(INTERRUPT_S1, n).contains("\"state\":\"started\""),
                "fixture premise {n}"
            );
        }
        assert!(!was_open, "the gate is closed after the last turn ends");
    }

    /// A user echo · complete `assistant` text · tool call · streamed text delta · `Delivered` line each open the gate and that turn's `result` closes it — the gate-opening events = the classifier's progress signals.
    // ADR-0238
    #[test]
    fn every_progress_line_opens_the_gate_and_the_result_closes_it() {
        let echo = fixture_line(INTERRUPT_S1, 5);
        assert!(echo.contains("\"isReplay\":true"), "fixture premise");
        let tool_use = completed(
            "m1",
            serde_json::json!({ "type": "tool_use", "id": "t1", "name": "Bash", "input": {} }),
        );
        let streamed = format!(
            "{}{}{}",
            message_start("m1"),
            block_start(0, "text"),
            text_delta(0, "hi")
        );
        for (label, opener) in [
            ("echo", echo),
            ("complete text", completed_text("m1", "hi")),
            ("tool call", tool_use),
            ("streamed delta", streamed),
            ("Delivered", fixture_line(INTERRUPT_S1, 2)),
        ] {
            let (mut decoder, line) = gated_decoder();
            decoder.decode(opener.as_bytes());
            assert!(
                line().is_some(),
                "after {label} there is a turn to interrupt"
            );
            decoder.decode(RESULT_LINE.as_bytes());
            assert!(
                line().is_none(),
                "{label}: after `result` there is no turn to interrupt"
            );
        }
    }

    /// Lines without a signal (idle `queued` · `system/init`·`status`·`task_notification` · interrupt response · cancel response · ack-unavailable init · partial message auxiliary lines · synthetic interrupt line · `cancelled` moved to `Dropped`) do not open a closed gate.
    // ADR-0238
    // ADR-0243
    #[test]
    fn the_turn_gate_starts_closed_and_signal_less_lines_do_not_open_it() {
        let (mut decoder, line) = gated_decoder();
        assert!(
            line().is_none(),
            "right after spawn there is no turn to interrupt"
        );

        let mut lines: Vec<String> = [1, 3, 4, 135, 137, 139, 183, 184, 186, 188]
            .into_iter()
            .map(|n| fixture_line(INTERRUPT_S1, n))
            .collect();
        lines.push(
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"cancel:u1","response":{"cancelled":true}}}"#.to_string() + "\n",
        );
        lines.push(
            TEXT_JSONL
                .lines()
                .find(|l| l.contains("\"subtype\":\"init\""))
                .map(|l| format!("{l}\n"))
                .expect("init without the capability"),
        );
        lines.push(message_start("m1"));
        lines.push(block_start(0, "text"));
        for l in &lines {
            decoder.decode(l.as_bytes());
            assert!(line().is_none(), "this line opened the gate: {l}");
        }
    }

    /// The last line ending without a newline also moves the gate — `flush` also takes the live line path.
    // ADR-0238
    #[test]
    fn a_trailing_line_flushed_at_eof_moves_the_gate_too() {
        let (mut decoder, line) = gated_decoder();
        let started = fixture_line(INTERRUPT_S1, 2);
        assert!(decoder.decode(started.trim_end().as_bytes()).is_empty());
        assert!(line().is_none(), "before the newline it is not a line yet");
        assert_eq!(tags(&decoder.flush()), vec!["queued:delivered"]);
        assert!(line().is_some());
    }

    /// The interrupt line function carries a new request id on every call — pressing twice while the gate is open makes two requests.
    // ADR-0238
    #[test]
    fn each_interrupt_line_carries_a_fresh_request_id() {
        let (mut decoder, line) = gated_decoder();
        decoder.decode(fixture_line(INTERRUPT_S1, 2).as_bytes());
        let ids: Vec<String> = [line(), line()]
            .into_iter()
            .map(|out| {
                let v: serde_json::Value =
                    serde_json::from_slice(&out.expect("open gate").bytes).unwrap();
                assert_eq!(v["request"]["subtype"], "interrupt");
                v["request_id"].as_str().unwrap().to_string()
            })
            .collect();
        assert!(ids
            .iter()
            .all(|id| id.starts_with(INTERRUPT_REQUEST_PREFIX)));
        assert_ne!(ids[0], ids[1]);
    }

    /// An interrupted turn neither sets nor clears the stop after error — an earlier failed turn's stop remains through two interrupted turns and the next clean turn clears it; no 「in a turn」 remains after an interrupted turn (ADR-0234 · ADR-0127).
    // ADR-0238
    #[test]
    fn an_interrupted_turn_neither_sets_nor_clears_the_error_halt() {
        let table = Arc::new(crate::turn::TurnObservations::new());
        let (core, id, epoch) = observed_core(&table);
        let mut d = ClaudeStreamDecoder::new();
        let observed = || table.get(id, epoch).expect("observed");

        let failed_turn: String = RESULT_ERROR
            .lines()
            .take(4)
            .map(|l| format!("{l}\n"))
            .collect();
        feed(&core, &mut d, &failed_turn);
        assert!(
            observed().last_end_failed,
            "premise — stop after the failed turn"
        );

        let lines = fixture_lines(INTERRUPT_S1);
        feed(&core, &mut d, &lines[..139].concat());
        assert!(
            observed().last_end_failed,
            "the first interrupted turn cleared the stop"
        );
        assert!(
            !observed().in_turn,
            "in-turn remained after an interrupted turn"
        );
        feed(&core, &mut d, &lines[139..188].concat());
        assert!(
            observed().last_end_failed,
            "the second interrupted turn cleared the stop"
        );
        assert!(!observed().in_turn);
        feed(&core, &mut d, &lines[188..].concat());
        assert!(!observed().last_end_failed, "a clean turn clears the stop");
        assert!(!observed().in_turn);

        let table = Arc::new(crate::turn::TurnObservations::new());
        let (core, id, epoch) = observed_core(&table);
        feed(
            &core,
            &mut ClaudeStreamDecoder::new(),
            &lines[..139].concat(),
        );
        assert!(
            !table.get(id, epoch).expect("observed").last_end_failed,
            "an interrupted turn set the stop"
        );
    }

    /// The two texts of the synthetic interrupt line → one interrupted marker carrying the original (not a bubble) — both live and resume. Classification = no signal.
    // ADR-0243
    #[test]
    fn the_synthetic_interrupt_lines_become_interrupted_notes_live_and_in_transcripts() {
        let classify = ClaudeBackend.turn_classifier();
        for (n, text) in [
            (137, "[Request interrupted by user]"),
            (186, "[Request interrupted by user for tool use]"),
        ] {
            let raw = fixture_line(INTERRUPT_S1, n);
            assert!(!raw.contains("isReplay"), "fixture premise {n}");

            let live = decode_all(raw.as_bytes());
            assert_eq!(live.len(), 1, "{n}: {live:?}");
            assert_eq!(
                interrupted_note_text(&live[0]).as_deref(),
                Some(text),
                "{n}"
            );
            assert_eq!(classify(&live[0]), None, "{n}");

            let seeded = parse_transcript_events(&raw);
            assert_eq!(
                tags(&seeded),
                vec!["structured:interrupted", "done"],
                "{n}: the history is closed with a synthetic end"
            );
            assert_eq!(
                interrupted_note_text(&seeded[0]).as_deref(),
                Some(text),
                "{n}"
            );
        }
    }

    /// Unrecognized `user` lines stay as today — an echo (`isReplay:true`) is a bubble even if its text starts with the head · two text blocks · text not starting with the head · a `tool_result` containing the head are `Structured{user}` · a plain-string `content` gives 0 events.
    // ADR-0243
    #[test]
    fn user_lines_that_are_not_the_synthetic_note_stay_as_today() {
        let user_line = |content: serde_json::Value, extra: serde_json::Value| {
            let mut v = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": content },
                "uuid": "11111111-1111-1111-1111-111111111111",
            });
            if let (Some(line), Some(extra)) = (v.as_object_mut(), extra.as_object()) {
                line.extend(extra.clone());
            }
            format!("{v}\n")
        };
        let text = |t: &str| serde_json::json!({ "type": "text", "text": t });
        let prefixed = "[Request interrupted by user] but I typed this";

        let cases: [(&str, String, Vec<&str>); 5] = [
            (
                "echo",
                user_line(
                    serde_json::json!([text(prefixed)]),
                    serde_json::json!({ "isReplay": true }),
                ),
                vec!["structured:user"],
            ),
            (
                "two text blocks",
                user_line(
                    serde_json::json!([text("[Request interrupted by user]"), text("more")]),
                    serde_json::json!({}),
                ),
                vec!["structured:user", "structured:user"],
            ),
            (
                "text not starting with the head",
                user_line(
                    serde_json::json!([text("text before [Request interrupted by user]")]),
                    serde_json::json!({}),
                ),
                vec!["structured:user"],
            ),
            (
                "tool_result",
                user_line(
                    serde_json::json!([{
                        "type": "tool_result",
                        "tool_use_id": "t1",
                        "content": "[Request interrupted by user]"
                    }]),
                    serde_json::json!({}),
                ),
                vec!["structured:user"],
            ),
            (
                "plain string",
                user_line(
                    serde_json::json!("[Request interrupted by user]"),
                    serde_json::json!({}),
                ),
                vec![],
            ),
        ];
        for (label, line, want) in cases {
            assert_eq!(tags(&decode_all(line.as_bytes())), want, "{label} live");
            let seeded = parse_transcript_events(&line);
            assert!(
                !tags(&seeded).contains(&"structured:interrupted".to_string()),
                "{label} resume: {seeded:?}"
            );
        }

        // An echo is a bubble carrying our uuid as is — the frontend dedup key.
        let replay = decode_all(
            user_line(
                serde_json::json!([text(prefixed)]),
                serde_json::json!({ "isReplay": true }),
            )
            .as_bytes(),
        );
        match &replay[0] {
            OutputEvent::Structured { kind, json } => {
                assert_eq!(kind, "user");
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                assert_eq!(v["text"], prefixed);
                assert_eq!(v["uuid"], "11111111-1111-1111-1111-111111111111");
            }
            other => panic!("expected Structured user, got {other:?}"),
        }
    }

    /// An interrupted marker has no signal in the turn classifier — as progress it would turn 「in a turn」 back on after `result`; other `Structured` are still progress.
    // ADR-0243
    #[test]
    fn the_interrupted_note_is_not_turn_progress_but_other_structured_events_are() {
        let classify = ClaudeBackend.turn_classifier();
        let structured = |kind: &str| OutputEvent::Structured {
            kind: kind.to_string(),
            json: "{}".to_string(),
        };
        assert_eq!(classify(&structured(INTERRUPTED_KIND)), None);
        for kind in ["user", "thinking", "tool_result", "tool_use"] {
            assert_eq!(
                classify(&structured(kind)),
                Some(TurnSignal::Progress),
                "{kind}"
            );
        }
    }

    /// If the resumed history ends with an interrupted `result`, that end is itself the turn boundary — no synthetic `MessageDone` is appended.
    // ADR-0238
    #[test]
    fn a_transcript_ending_with_an_interrupted_result_is_not_double_closed() {
        let tail = [137, 138].map(|n| fixture_line(INTERRUPT_S1, n)).concat();
        assert_eq!(
            tags(&parse_transcript_events(&tail)),
            vec!["structured:interrupted", "turn-end"]
        );
    }

    /// Production assembly — a JSON spawn's link reports the interrupt capability, refuses right after spawn (no turn to interrupt), and accepts the interrupt line once that incarnation's decoder reads a `started` line (decoder and interrupt line function share one gate).
    // ADR-0238
    #[cfg(windows)]
    #[test]
    fn the_json_spawn_shares_one_gate_between_its_decoder_and_its_interrupt_line() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, StatusSink};
        use std::time::{Duration, Instant};

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let dir = std::env::temp_dir().join(format!("engram-claude-gate-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("started.jsonl");
        std::fs::write(&file, fixture_line(INTERRUPT_S1, 2)).expect("write");
        // It streams one `started` line and lives without reading stdin — when the child ends the input queue closes too and there is nowhere to receive the interrupt line.
        let probe = CommandSpec {
            program: "cmd.exe".into(),
            args: vec![
                "/c".into(),
                "type".into(),
                file.to_string_lossy().into_owned(),
                "&".into(),
                "ping".into(),
                "-n".into(),
                "30".into(),
                "127.0.0.1".into(),
                ">nul".into(),
            ],
            env: vec![],
            cwd: PathBuf::from("."),
        };
        let parts = crate::backend::open_spawn(
            &json(vec![]),
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
        assert!(parts.transport.capabilities().control.interrupt);
        assert!(
            matches!(parts.transport.interrupt(), Err(PtyError::Unsupported(_))),
            "right after spawn there is no turn to interrupt"
        );

        let core = Arc::new(OutputCore::new(
            Uuid::new_v4(),
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        parts.transport.start(Arc::clone(&core));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match parts.transport.interrupt() {
                Ok(()) => break,
                Err(PtyError::Unsupported(_)) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                other => {
                    panic!("did not accept the interrupt line even after `started`: {other:?}")
                }
            }
        }
        parts.transport.shutdown();
        core.join_pump(Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── ADR-0262: decoder · interrupt line function ↔ interrupt episode ──────────────────────────────────────────────────

    /// The gate containing the cleaner and the decoder · interrupt line function sharing it — the `open_spawn` assembly with only the group replaced by an already-gone handle. The port cannot be used so the episode's snapshot is a failure, but the episode stands;
    /// write confirmation is not called, so neither the worker nor the listening thread starts.
    fn cleaned_decoder() -> (ClaudeStreamDecoder, InterruptLine, Arc<GateCell>) {
        use crate::transport::process_group::{ProcessGroup, RetiringSignal};
        let retiring = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let group = ProcessGroup::detached(RetiringSignal::of(&retiring));
        let cleaner = Cleaner::new(
            Arc::new(group),
            Arc::new(SystemClock),
            4242,
            LogTag::new(None, 4242),
        );
        let (decoder, gate) = decoder_with_gate(GateCell::new(Some(Arc::new(cleaner))));
        (decoder, interrupt_line(Arc::clone(&gate)), gate)
    }

    /// The episode is visible only through the Debug of `GateCell`.
    fn has_episode(gate: &GateCell) -> bool {
        let shown = format!("{gate:?}");
        assert!(shown.contains("cleaner: true"), "{shown}");
        shown.contains("episode: Some")
    }

    /// The `started` of another input — a second new input reaching an already-open turn.
    const SECOND_STARTED: &str = "{\"type\":\"command_lifecycle\",\"command_uuid\":\"u2\",\"state\":\"started\",\"uuid\":\"x\",\"session_id\":\"s\"}\n";

    /// An interrupt right after spawn · after the turn end has neither a line nor an episode · an interrupt on a gate opened by a progress line gives a line carrying write confirmation and opens an episode · same-turn progress lines do not touch the episode · `result`
    /// closes the gate and drops the episode.
    // ADR-0262
    #[test]
    fn an_interrupt_on_an_open_turn_opens_an_episode_that_the_turn_end_drops() {
        let (mut decoder, line, gate) = cleaned_decoder();
        assert!(
            line().is_none(),
            "right after spawn there is no turn to interrupt"
        );
        assert!(!has_episode(&gate));

        decoder.decode(completed_text("m1", "hi").as_bytes());
        assert!(gate.is_open());
        assert!(
            !has_episode(&gate),
            "a progress line does not create an episode"
        );

        let out = line().expect("open gate");
        assert!(
            out.on_written.is_some(),
            "an accepted interrupt carries write confirmation"
        );
        assert!(has_episode(&gate), "an accepted interrupt opens an episode");
        decoder.decode(completed_text("m1", "more").as_bytes());
        assert!(has_episode(&gate), "a progress line dropped the episode");

        decoder.decode(RESULT_LINE.as_bytes());
        assert!(!gate.is_open());
        assert!(!has_episode(&gate), "the turn end drops the episode");
        assert!(line().is_none());
        assert!(!has_episode(&gate));
        assert_eq!(decoder.gate_moves, vec![GateMove::Open, GateMove::Close]);
    }

    /// An interrupted `result` (`TurnEnd{Interrupted}`) is also a turn end — it closes the gate and drops the episode.
    // ADR-0262
    #[test]
    fn an_interrupted_result_also_drops_the_episode() {
        let (mut decoder, line, gate) = cleaned_decoder();
        let lines = fixture_lines(INTERRUPT_S1);
        for l in &lines[..137] {
            decoder.decode(l.as_bytes());
        }
        assert!(line().is_some());
        assert!(has_episode(&gate));

        let end = decoder.decode(lines[137].as_bytes());
        assert!(
            end.last().is_some_and(is_interrupted_end),
            "fixture premise: {end:?}"
        );
        assert!(!gate.is_open());
        assert!(!has_episode(&gate));
    }

    /// A second new input (`started`) reaching an open turn drops that turn's interrupt episode and keeps the gate open — an interrupt after that opens a new episode.
    // ADR-0262
    #[test]
    fn a_second_started_drops_the_episode_and_keeps_the_turn_open() {
        let (mut decoder, line, gate) = cleaned_decoder();
        decoder.decode(fixture_line(INTERRUPT_S1, 2).as_bytes());
        assert!(line().is_some());
        assert!(has_episode(&gate));

        decoder.decode(SECOND_STARTED.as_bytes());
        assert!(gate.is_open(), "the new input closed the gate");
        assert!(
            !has_episode(&gate),
            "the new input did not drop the episode"
        );

        assert!(line().is_some());
        assert!(
            has_episode(&gate),
            "the interrupt after the new input did not open an episode"
        );
        assert_eq!(
            decoder.gate_moves,
            vec![GateMove::Deliver, GateMove::Deliver]
        );
    }

    /// The `started` that opens a turn announces to the gate once (new input), 「open」 not separately; as progress lines continue the gate is called only on closed → open, and closed once per turn end — the fixture's three turns = six times.
    // ADR-0262
    #[test]
    fn the_opening_started_is_one_move_and_progress_moves_only_at_transitions() {
        let (mut decoder, _line, gate) = cleaned_decoder();
        let classify = ClaudeBackend.turn_classifier();
        let lines = fixture_lines(INTERRUPT_S1);
        let started = lines
            .iter()
            .filter(|l| l.contains("\"state\":\"started\""))
            .count();
        assert_eq!(started, 3, "fixture premise — one `started` per turn");

        decoder.decode(lines[1].as_bytes());
        assert_eq!(decoder.gate_moves, vec![GateMove::Deliver]);
        assert!(gate.is_open());

        let mut progress = 0;
        for l in &lines[2..137] {
            progress += decoder
                .decode(l.as_bytes())
                .iter()
                .filter(|e| classify(e) == Some(TurnSignal::Progress))
                .count();
        }
        assert!(
            progress > 1,
            "fixture premise — progress events of the first turn {progress}"
        );
        assert_eq!(decoder.gate_moves, vec![GateMove::Deliver]);

        for l in &lines[137..] {
            decoder.decode(l.as_bytes());
        }
        use GateMove::{Close, Deliver};
        assert_eq!(
            decoder.gate_moves,
            vec![Deliver, Close, Deliver, Close, Deliver, Close]
        );
        assert!(!gate.is_open());
    }

    /// Opening and closing the gate (open · close · new input) runs outside the containment marker — the decoder is called inside the pump's containment.
    // ADR-0292
    #[test]
    fn the_gate_moves_run_outside_the_contain_mark() {
        use engram_dashboard_base::panic::{contain, is_contained};

        let (mut decoder, _line, _gate) = cleaned_decoder();
        let after = contain(std::panic::AssertUnwindSafe(|| {
            decoder.decode(completed_text("m1", "hi").as_bytes());
            decoder.decode(RESULT_LINE.as_bytes());
            decoder.decode(SECOND_STARTED.as_bytes());
            is_contained()
        }))
        .expect("no panic");

        use GateMove::{Close, Deliver, Open};
        assert_eq!(decoder.gate_moves, vec![Open, Close, Deliver]);
        assert_eq!(decoder.gate_marks, vec![false, false, false]);
        assert!(
            after,
            "after announcing to the gate, the pump's marker did not come back"
        );
    }

    /// The resume original does not reach the gate — resuming one turn with progress and an interrupted `result` leaves a closed gate closed and an open gate and its episode as is.
    // ADR-0262
    #[test]
    fn transcript_lines_never_touch_the_gate() {
        let (mut decoder, line, gate) = cleaned_decoder();
        let turn = fixture_lines(INTERRUPT_S1)[..138].concat();
        assert!(
            !parse_transcript_events(&turn).is_empty(),
            "fixture premise"
        );
        assert!(!gate.is_open(), "the resume opened the gate");

        decoder.decode(fixture_line(INTERRUPT_S1, 2).as_bytes());
        assert!(line().is_some());
        parse_transcript_events(&turn);
        assert!(gate.is_open(), "the resume closed the gate");
        assert!(has_episode(&gate), "the resume dropped the episode");
    }

    /// The cleaner stands only when the link's process group and the root PID are both present — with no root PID or 0 it only opens and closes the gate.
    // ADR-0262
    #[cfg(windows)]
    #[test]
    fn the_cleaner_needs_the_process_group_and_a_root_pid() {
        let probe = CommandSpec {
            program: "cmd.exe".into(),
            args: ["/c", "ping", "-n", "30", "127.0.0.1", ">nul"]
                .map(String::from)
                .to_vec(),
            env: vec![],
            cwd: PathBuf::from("."),
        };
        let (t, pid) = StdioTransport::open(&probe, true, None).expect("open");
        assert!(leftover_cleaner(&t, pid, None).is_some());
        assert!(leftover_cleaner(&t, None, None).is_none());
        assert!(leftover_cleaner(&t, Some(0), None).is_none());
        t.shutdown();
    }
}
