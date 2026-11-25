//! CodexBackend — produces the codex-CLI-specific CommandSpec.
//!
//! ★The rule this folder sets = codex knowledge lives only in here (ADR-0004)★. The rationale, the gates and
//! what the gates cannot see are written once in the `backend/claude/mod.rs` header — read it with the names
//! swapped. The surfaces that leave this folder are the [`crate::backend::AgentBackend`] impl and two singletons,
//! the usage probe [`CODEX_USAGE_PROBE`] and the settings check [`CODEX_SETTINGS_CHECK`] (why they are not trait
//! slots is in that header too) — the session-id recovery polling also runs inside [`thread_lock`], and only this
//! folder calls that module (ADR-0218 decision 11).
//!
//! ★The codex facts here were measured on codex-cli 0.153.4 (this PC, authenticated, 2026-09-08)★; later measurements carry their own version at their spot.
//! `tests/backend_contract.rs` launches the argv `build_spec` emits as is — but that lane is `#[ignore]` (not CI), so it runs only when called.
//!
//! ★What the test bench does NOT measure★:
//!   1. The model behaviour under the `workspace-write` · `on-request` policies — the bench only proves it comes up with that argv.
//!   2. Arguments we do not use (`-m`) — only mentioned in comments here.
//!      The MCP `-c mcp_servers.…` override is used by us and was measured by hand (0.155.0, throwaway `CODEX_HOME`): `--strict-config` accepted the keys,
//!      `mcpServerStatus/list` returned `engram` with its send/query tools, the stub server received `Authorization: Bearer <ENGRAM_TOKEN value>`, and the
//!      `cmd.exe /c` branch behaved the same. The tools were later renamed `eg_send` · `eg_messages` (`1b84045`); the measurement stays valid.
//!   2-1. `codex resume <id>` is used by us but the bench does not measure it: [`production_spec`](../../../tests/backend_contract.rs) draws argv only with
//!      `SpawnMode::Fresh`, so the resume branch is never carried in that lane (it would need a live thread id).
//!      Measured by hand (0.155.0): it comes up with that argv and **the session id is preserved** — a new session reports id S with `source:"startup"`, a resume
//!      reports the same S with `source:"resume"` (stable across repeats), and the record is appended to S's rollout file. So recovery on a resumed
//!      session lands on the same value whether it overwrites or not.
//!      ★That observation came from the **hook channel**, which was removed (ADR-0216)★. Today terminal mode recovers via the writer-lock name the child holds
//!      ([`thread_lock`] · ADR-0218) and app-server mode via the `thread/start` response, so the id behaviour on resume is unmeasured and overwriting was
//!      chosen on top of that.
//!      ★Terminal-mode recovery does not run in a resume incarnation★ — it goes out carrying the id as argv (ADR-0218 decision 5).
//!   3. The `%VAR%` limit of `build_spec` (the comment there is the source of truth).
//!
//! If the capability declaration disagrees with that table, the bench's **non-`#[ignore]`** items go red.
//!
//! tauri import 0.

mod cmdline;
pub(crate) mod decoder;
// ADR-0420
mod hook_assembly;
// ADR-0420
mod hook_check;
mod launch;
pub(crate) mod protocol;
mod settings;
// ADR-0218
pub(crate) mod thread_lock;
pub(crate) mod transport;
mod usage;
mod usage_probe;

pub(crate) use settings::CODEX_SETTINGS_CHECK;
pub(crate) use usage_probe::CODEX_USAGE_PROBE;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use engram_dashboard_platform::shell::{console_command, text_is_cli_safe};
use uuid::Uuid;

use self::decoder::CodexAppServerDecoder;
use self::protocol::{
    AskForApproval, SandboxMode, ThreadOpen, ThreadResumeParams, ThreadStartParams,
};
use self::transport::{AppServerOpen, CodexAppServerTransport};
use crate::backend::{
    inject_cli_entrance, AgentBackend, FirstTurnSink, InputEncoder, SessionIdSink, SpawnParts,
    TransportShape, TurnClassifier,
};
use crate::failure::AgentFailureKind;
use crate::launch::{LaunchArtifacts, LaunchFs, LaunchKit};
use crate::profile::{AgentCommand, AgentOutputFormat, SpawnMode};
use crate::transport::pty::PtyTransport;
use crate::transport::{AgentTransport, LinkSink, OutputDecoder};
use crate::turn::{TurnEndKind, TurnSignal};
use crate::types::{
    AgentId, BackendCaps, CommandSpec, ControlEndpoint, DeliveryAck, MidTurnPolicy, ModelCaps,
    OutputEvent, PtyError, SessionCaps, TurnOutcome, MCP_SERVER_NAME, TOKEN_ENV,
};

/// Whether codex launches as a **resident JSON server** rather than an interactive TUI — the point where the four axes in this folder (link shape · link object ·
/// input encoding · output decoder) split together.
///
/// ★Do not lift this predicate into `profile` (ADR-0004)★: codex's output-format axis would then drive the shared layer's link choice. Only the neutral axis value goes outward.
fn is_app_server(command: &AgentCommand) -> bool {
    matches!(
        command,
        AgentCommand::Codex {
            output_format: AgentOutputFormat::StreamJson,
            ..
        }
    )
}

/// Picks this spawn's **second handshake request**: `thread/resume` if `resume_session_id` is present, otherwise open a new thread. Absence = nothing to resume
/// (no stored value, or launched as Fresh).
///
/// ★The three policies (working folder · approval · sandbox) are carried identically on both branches★: left out of resume, codex runs with the values stored at thread
///   creation, so a resumed session would quietly trust the old folder with no sign on screen.
/// ★Unverified: whether the cwd we send overrides the recorded value★ (`docs/reference/backend-capabilities.md` §1): a different cwd is not refused (**it resumes** — thread identity is per
///   `CODEX_HOME`), but the response's `cwd` is the original recorded one, so our value may be ignored and the old-folder mismatch remains. Not measured (resume the same thread with two folders).
///   We keep carrying the argument: ignored costs nothing, applied gives the intended value.
/// ★`excludeTurns` is on by decision★ — we read no field of `thread.turns`; if it came filled, one long conversation would exceed the link's line cap (`MAX_LINE_BYTES`), **only that
///   line would be dropped**, and the handshake would wait to its deadline. That outcome is indistinguishable from `thread/resume` being refused with a JSON-RPC error — the peer is alive
///   and answered, we only see 「handshake failed on its deadline, child fine」 — and if that shape does not close the link's stdin, child · reader · writer are held as a wedge nobody reaps
///   (see that close in `transport.rs`). So if upstream ignores `excludeTurns`, this argument blocks nothing and that close is the only defence. Whether upstream honours it is unverified.
/// ★`developer_instructions` goes on **both** branches★ — on a resume too, changed or not: codex holds it in the
///   resumed process and rebuilds the initial context from it at a compaction there; without it that rebuild has no
///   instructions at all (the reason and the version gate: the [`ThreadResumeParams`] field doc). It does not reach
///   the next turn on a resume — a change reaches it through the first turn's context ([`app_server_open`]).
/// ★The value goes **unmodified**★ — a JSON body has no command-line limit, no `%` expansion and no line-break cut.
///   Bringing the terminal branch's conversion ([`cmdline::priming_value`]) here would guard against nothing and only
///   damage the document the agent reads.
/// ★`config` goes on **both** branches too★ — the hook assembly's thread config (`{"hooks": …}`) is not saved with the
///   thread, so a resume without it has no hooks (measure-3 M2). `None` = the field is absent.
// ADR-0185
// ADR-0215
// ADR-0337
// ADR-0354
// ADR-0420
fn thread_open(
    spec: &CommandSpec,
    resume_session_id: Option<Uuid>,
    developer_instructions: Option<String>,
    config: Option<serde_json::Value>,
) -> ThreadOpen {
    let cwd = Some(spec.cwd.to_string_lossy().into_owned());
    let approval_policy = Some(AskForApproval::OnRequest);
    let sandbox = Some(SandboxMode::WorkspaceWrite);
    match resume_session_id {
        Some(thread_id) => ThreadOpen::Resume(ThreadResumeParams {
            thread_id: thread_id.to_string(),
            cwd,
            approval_policy,
            sandbox,
            exclude_turns: Some(true),
            developer_instructions,
            config,
        }),
        None => ThreadOpen::Start(ThreadStartParams {
            cwd,
            approval_policy,
            sandbox,
            developer_instructions,
            config,
        }),
    }
}

/// What an app-server spawn opens with — the tests pin it through this function (the Noop handshake regression · TRD
/// S25 §5-2 4).
///
/// - Noop (`!artifacts.real`): no instructions, no roots, no context, no thread config, no hook check.
/// - Real: the merged instructions text ([`LaunchArtifacts::instructions_text`]); the skill roots; the thread config
///   `{"hooks": …}` when the hook assembly wrote one ([`LaunchArtifacts::hook_config`], on start and resume alike);
///   the expected trust entries for the post-open check whenever there is a thread config; and the
///   instructions-change context only on a resume whose instructions changed. A Fresh spawn never carries it, even with `instructions_changed`: a new
///   thread receives the full text through `developerInstructions` at `thread/start`.
// ADR-0337
// ADR-0370
// ADR-0420
fn app_server_open(
    spec: &CommandSpec,
    resume_session_id: Option<Uuid>,
    artifacts: &LaunchArtifacts,
) -> Result<AppServerOpen, PtyError> {
    if !artifacts.real {
        return Ok(AppServerOpen {
            thread: thread_open(spec, resume_session_id, None, None),
            extra_roots: Vec::new(),
            first_turn_context: None,
            expected_hooks: None,
        });
    }
    let extra_roots = artifacts
        .skill_roots
        .iter()
        .map(|root| {
            root.to_str().map(str::to_owned).ok_or_else(|| {
                PtyError::SpawnFailed(format!(
                    "codex skill root is not valid UTF-8: {}",
                    root.display()
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let first_turn_context = match &artifacts.instructions_text {
        Some(text) if resume_session_id.is_some() && artifacts.instructions_changed => {
            Some(format!("{INSTRUCTIONS_CHANGED_PREFACE}\n\n{text}"))
        }
        _ => None,
    };
    let config = artifacts
        .hook_config
        .clone()
        .map(|hooks| serde_json::json!({ HOOKS_CONFIG_KEY: hooks }));
    let expected_hooks = config.is_some().then(|| artifacts.hook_trust.clone());
    Ok(AppServerOpen {
        thread: thread_open(
            spec,
            resume_session_id,
            artifacts.instructions_text.clone(),
            config,
        ),
        extra_roots,
        first_turn_context,
        expected_hooks,
    })
}

/// The one key of the JSON thread config Engram sends — the whole `hooks` table (events and `state`), never a dotted
/// `"hooks.…"` key beside it (codex applies request keys in hash-map order — ADR-0420).
// ADR-0420
const HOOKS_CONFIG_KEY: &str = "hooks";

/// The line in front of the full instructions text in the instructions-change context (main's decision #3).
// ADR-0337
const INSTRUCTIONS_CHANGED_PREFACE: &str = "The instructions for this agent changed since its last launch. The current instructions follow and take precedence over earlier instructions in this conversation.";

/// Launches the name as resolved through PATH (TRD §6-H).
///
/// ★Why not find the real binary and launch it directly★: on Windows `codex` on PATH is the `codex.cmd → node → codex.exe` chain, and the real binary lives under a **version-stamped
/// `node_modules` vendor path** (measured). codex self-updates, so hard-coding it dies on one update and a search · fallback reimplements the chain.
/// ★Another shim layer does not change kill causality★: **once enrolled** in the Job, breakaway is blocked (neither `BREAKAWAY_OK` nor `SILENT_BREAKAWAY_OK`), so descendants cannot leave and
/// `TerminateJobObject` ends the tree (the 2 verbs of ADR-0001).
/// ★Enrolment is **after** spawn, so descendants born in that window never enter the Job★ — all three agent links (`pty` · `stdio` · the codex link) share this and
/// the teardown tests measure only the settled state, so the window is unmeasured. Fixing it touches all three links; the precedent is the usage query runner (launch suspended, enrol, wake:
/// `usage::process` + `spawn::prepare_tree_root` · `spawn::TreeRoot`).
const CODEX_PROGRAM: &str = "codex";

/// The peer wording that means 「there is no record of that thread」 (lower-case comparison), taken verbatim from a measured response: `-32600` + `no rollout found for thread id`
/// (`docs/reference/backend-capabilities.md` §1). ★The wording, not the code, is the criterion★: the same code also comes for a config error · a duplicate `initialize` · resuming a child
/// thread, and splitting by code would stamp a sound handle with 「no conversation to resume」 on an unrelated failure.
const NO_ROLLOUT_MARKER: &str = "no rollout found";

/// The flag through which codex receives the working folder — ★separate from the process cwd★: `CommandSpec.cwd` is where we launch the process, this is **which folder codex trusts and edits
/// as its workspace**. We give both the same value, but they are different fields.
const CD_FLAG: &str = "--cd";

/// Sandbox mode — allows the commands the model emits **up to writing, inside the working folder**.
const SANDBOX_FLAG: &str = "-s";
const SANDBOX_WORKSPACE_WRITE: &str = "workspace-write";

/// Approval mode — asks a person only about actions that go outside the sandbox.
const APPROVAL_FLAG: &str = "-a";
const APPROVAL_ON_REQUEST: &str = "on-request";

/// The subcommand that resumes a stored session. ★It is not a flag — `--resume` · `--session-id` do not exist★ (measured). The id is a **positional argument**, first position of
/// `codex resume [OPTIONS] [SESSION_ID] [PROMPT]` (0.155.0 `codex resume --help`).
///
/// ★The three policies (`--cd` · `-s` · `-a`) still parse **after** this subcommand★ (they are `resume`'s own options too; `codex resume <id> --cd . -s <bad value>` and the options-first order
///   both died with a **sandbox value error**, i.e. parsing got that far). We put the id right after the subcommand so `codex resume <id>` reads as one chunk.
/// ★The second positional argument is `[PROMPT]`★ — a bare non-flag word in `extra_args` is consumed as the first prompt. The Fresh branch has the same property, so resume adds no new risk
///   (the pass-through comment below says why it is not filtered).
const RESUME_SUBCOMMAND: &str = "resume";

/// The flag that overrides codex settings from the command line. ★It is the only means of getting settings without creating a single file on the user's machine★
/// — we write not a single character in the user home (`$CODEX_HOME/config.toml`) or in a wrapper script.
const CONFIG_OVERRIDE_FLAG: &str = "-c";

/// The env variable that overrides, per spawn, the value codex writes verbatim into the `originator` field of the session record.
///
/// ★It is an **internal** upstream variable★ — if it disappears it is a silent ignore, leaving only 「a record without a marker」 (ADR-0217 「three unverified items」 ③). That the value stays in
///   the record as is was measured (0.155.1 · 3 arbitrary strings · no allowlist refusal — ADR-0217 Probe B).
/// ★Visible to the child and its descendants — left as is on purpose★: it holds the **identifier** `(agent_id, epoch)`, not a permission; permission is the bearer token of [`TOKEN_ENV`]
///   in the same env, which cannot be made from this pair. And the value is planted precisely to remain outside our process (the vendor record's `originator`). Revisit only if the
///   agent id itself becomes a key that opens something — then fix that key design, not this spot.
// ADR-0217
const ORIGINATOR_ENV: &str = "CODEX_INTERNAL_ORIGINATOR_OVERRIDE";

/// The key prefix of the config override that attaches the daemon MCP server to this spawn; the server's logical name ([`MCP_SERVER_NAME`], do not retype it) is appended to make the single key
/// `mcp_servers.engram`.
///
/// ★The mechanism **differs** from claude's `--mcp-config <file>`★ — codex takes only this override, no file (measured 0.155.0); the same endpoint is translated into different syntax per
///   backend (ADR-0004).
const MCP_SERVER_OVERRIDE_PREFIX: &str = "mcp_servers.";

/// The MCP server config field that takes **the name of the env variable holding the bearer token**, not its value.
///
/// ★Do not revert to an inline value (`bearer_token='…'`)★: that **embeds the token in the command line**, readable by any same-user process via argv (Windows process list · WMI — the surface this repository
///   uses for daemon discovery). With this field the token is only the one copy already in the spawn env ([`inject_cli_entrance`] plants it as [`TOKEN_ENV`]); none in argv or on disk.
/// Measured 0.155.0: codex reads that env and carries it as a header — the stub MCP server received `Authorization: Bearer <that value>` on every request from `initialize` · `tools/list` on.
const MCP_BEARER_ENV_KEY: &str = "bearer_token_env_var";

/// The field that removes the tool-call approval prompt, limited to this one MCP server entry.
///
/// Valid values: `auto | prompt | writes | approve` — ★`auto` does not remove the prompt, only `approve` does★ (measured 2026-09-20), confirmed on all three paths: the interactive TUI ·
///   `codex exec` (without it: "MCP tool call requires approval, but approval policy is never") · `codex app-server --stdio` (without it the gate comes as `mcpServer/elicitation/request`).
/// ★Only this one server entry — approval is not turned off globally★: `approvalPolicy: on-request` · `sandbox: workspace-write` are untouched, so ordinary shell/exec approvals are still asked.
/// ★The dialog's "Always allow" does not persist to disk★ (before/after snapshots with an empty `CODEX_HOME`), so this value must be carried on every spawn.
const MCP_APPROVAL_MODE_KEY: &str = "default_tools_approval_mode";
const MCP_APPROVAL_MODE_VALUE: &str = "approve";

/// The config key that **appends** the priming brief **after** codex's default instructions.
///
/// ★Do not swap it with `model_instructions_file`★ — that one **replaces** instead of appending (measured 0.155.0), so codex's own operating instructions would disappear.
/// ★This key has **no** `*_file` counterpart★ — `developer_instructions_file` · `experimental_instructions_file` · `instructions_file` are all `unknown configuration field` (same measurement). So unlike
///   claude, this backend carries the **content** and **reads** the brief file here (the per-backend knowledge of ADR-0004).
/// Authority is the same layer: per the OpenAI docs the `instructions` parameter and a `developer` message are the same grade, so this field costs no 「instruction strength」.
// ADR-0004
// ADR-0215
const DEVELOPER_INSTRUCTIONS_KEY: &str = "developer_instructions";

/// The two characters (backslash + `n`) that stand in for a newline on the command line.
///
/// ★If one real LF remains, `cmd` **cuts the command line there without any error**★ (measured) — codex comes up with the following arguments gone, i.e. 「no MCP attachment and no hook registration」,
///   the most expensive failure of this value.
/// The model reads this two-character form as a newline (same measurement); a TOML literal string has no escapes, so it arrives **literally**.
// ADR-0215
const NEWLINE_REPLACEMENT: &str = "\\n";

/// The total command-line length `cmd.exe` accepts (UTF-16 code units).
///
/// ★The budget is not our value's alone★ — the policy flags, the pass-through and the overrides share one line, so
///   the check counts the whole assembled line ([`cmdline::assembled`]).
// ADR-0215
const CMD_LINE_LIMIT_UTF16: usize = 8191;

/// The **single decision** of whether to attach the daemon MCP server to this spawn; [`AgentBackend::build_spec`] (the attachment) and [`AgentBackend::precheck_control_endpoint`] (the fail-closed gate) read **this
/// same value**. ★If each decided, the gate would go hollow★ — a spawn it passed as 「can attach」 could come up unattached with no signal. The one decision is [`mcp_attachment`].
enum McpAttachment {
    /// Attach — the value to carry on `-c`.
    Attach(String),
    /// Do not attach — **the normal state**. Two branches: a spawn with no control channel (outside the mail plane), and one for which the daemon has **not authorized** the MCP send entrance
    /// (the operator turned off MCP mail — [`ControlEndpoint::grants_mcp_send`]). Neither blocks the spawn.
    NotWanted,
    /// Should attach but **cannot build it** (reason). ★Only this value produces fail-closed★ — otherwise the agent would sit on the receiving roster (`reads_messages`) with zero send entrances
    /// (MCP unattached, CLI mirror closed by `mail_allowed=false`) and every delivered request would be a contract nobody can answer.
    Unrepresentable(String),
}

/// The only place that makes the decision above.
///
/// ★Environment variables are not read again here (ADR-0004)★: the owner of the knobs that turn off MCP mail (`ENGRAM_DISALLOW_MCP_SEND` · the harness's `ENGRAM_FORCE_CLI_ONLY_SEND`) is **the daemon**, and the
///   decision arrives digested via [`ControlEndpoint::grants_mcp_send`]. A second reader would make two authorities and 「claude off, codex on」 would happen silently.
///
/// ★This branch has no plaintext token file — but this function is not what guarantees it★: the token stays out of argv via [`MCP_BEARER_ENV_KEY`], and out of disk because
///   [`AgentBackend::writes_mcp_config_file`] is false (the daemon's `control::provision` picks `mcp_config::write_config` by that one field; the file's content and lifetime:
///   `control/mcp_config.rs`). That this function creates no file and that no file appears for that spawn are different statements.
/// ★TOML literal string (single quotes)★ — no escapes, byte for byte, measured to survive `cmd.exe /c` + the `.cmd` shim (ADR-0210); a single quote itself cannot be held.
/// ★A character that cannot be carried stops it instead of inventing an escape★ — a single quote · whitespace · control characters break the value or the TOML line, and `%` is expanded by cmd **even inside
///   quotes** (`build_spec`'s same-named limit comment), pointing at a different address. All share one symptom, 「mail silently does not work」, so we make noise.
///   It never trips today (the url is daemon-authored `http://127.0.0.1:<port>/mcp`); the check is for the day that form changes.
/// ★`"` and `& ^ | < >` are refused as well★ — a `"` flips cmd's quote state for every later argument, and the
///   metacharacters act wherever that state is off (the same set as the preset `-c` guard, `cmdline::config_args`).
// ADR-0004
// ADR-0128
// ADR-0209
// ADR-0337
fn mcp_attachment(control: Option<&ControlEndpoint>) -> McpAttachment {
    let Some(endpoint) = control else {
        return McpAttachment::NotWanted;
    };
    // ★This one line is all that makes 「the operator turned off MCP mail」 reach this backend★ — looking only at the control channel's existence would attach unconditionally and leave claude off and
    //   codex on (`eg_send` still callable with automatic approval).
    if !endpoint.grants_mcp_send() {
        return McpAttachment::NotWanted;
    }
    let url = endpoint.url.as_str();
    if let Some(bad) = url.chars().find(|c| {
        *c == '\''
            || *c == '%'
            || c.is_control()
            || c.is_whitespace()
            || *c == '"'
            || !text_is_cli_safe(c.encode_utf8(&mut [0; 4]))
    }) {
        return McpAttachment::Unrepresentable(format!(
            "cannot attach the codex MCP server — the control channel address has a character that cannot be carried ({bad:?}): {url}"
        ));
    }
    // ADR-0337: the command-line count holds the URL to this bound.
    if cmdline::units(url) > MCP_URL_BUDGET {
        return McpAttachment::Unrepresentable(format!(
            "codex MCP server cannot be attached — the control channel URL is over its command-line bound ({MCP_URL_BUDGET} units): {url}"
        ));
    }
    McpAttachment::Attach(mcp_override_value(url))
}

/// The most a control channel URL may cost on the codex command line, in [`cmdline::units`]. The daemon's URL is
/// `http://127.0.0.1:<port>/mcp` (at most 26); a longer one is refused by [`mcp_attachment`] — which
/// `precheck_control_endpoint` turns into a failed spawn — so the bound the command-line count uses is never below
/// the real value.
// ADR-0337
const MCP_URL_BUDGET: usize = 128;

/// The `-c` value that attaches the daemon MCP server at `url`.
fn mcp_override_value(url: &str) -> String {
    format!(
        "{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}={{url='{url}',{MCP_BEARER_ENV_KEY}='{TOKEN_ENV}',{MCP_APPROVAL_MODE_KEY}='{MCP_APPROVAL_MODE_VALUE}'}}"
    )
}

/// The subcommand that launches it as a resident JSON server, and its transport (measured 0.154.0: `--stdio` equals `--listen stdio://` and is the default). Stated explicitly: this link
/// holds only on stdio, and an upstream default change would silently change our argv.
const APP_SERVER_SUBCOMMAND: &str = "app-server";
const APP_SERVER_STDIO_FLAG: &str = "--stdio";

/// Does the caller pass-through set **the same config key as ours**? `true` = our override wins (carried later), and a warning is given so a deliberately set value is not silently erased.
///
/// ★Catches one shape only: the **next slot** after `-c` starts with `key`★ (`-c developer_instructions=…`).
/// ★Known holes★: `-c` with the value joined in one word · the long-name flag · a higher key overriding a whole table (`-c tui=…` · `-c mcp_servers=…`) · another key that only starts with that key.
///   Do not grow the list toward 「certain」 — it becomes a reimplementation of codex's override syntax that goes stale. A miss loses **only the warning**; that ours wins is guaranteed by the order in `build_spec`.
/// Taking the key as an argument is intended: several overrides exist (MCP attachment · the brief) and per-key duplication leaves the warning half-fixed.
fn passthrough_overrides_key(extra_args: &[String], key: &str) -> bool {
    extra_args
        .windows(2)
        .any(|pair| pair[0] == CONFIG_OVERRIDE_FLAG && pair[1].starts_with(key))
}

/// The string if it has a shape that can be carried as the positional argument of `codex resume`, otherwise `None`.
///
/// ★The gate is needed because this argument **does not fail even when wrong**★ (measured 2026-09-21): a string codex cannot read as a thread id is reinterpreted as a **session name**,
///   **a new session is quietly created**, exit code `0`, and the turn is billed — 「a new conversation the user believes they resumed」. ADR-0218's failure mode is 「not received」, not 「the wrong
///   conversation」, so for an uncarriable value the subcommand is not emitted (falls to new-conversation argv; `opens_a_new_conversation` at the assembly point reports that regression).
/// ★Reachable today only for **nil**★: the field is `Uuid`, so a non-uuid string never gets here (profile deserialization refuses it earlier); nil (all zeros — hand-edited profile or init mistake)
///   has the uuid shape and passes the type gate.
/// ★Do not merge it with claude★ — `--resume` fails on an unknown value there; sharing would make one backend's upstream bug everyone's assembly rule (ADR-0004).
/// ★Not only for argv assembly — the session id recovery gate looks at it too★ (the terminal branch of [`AgentBackend::open_spawn`] · [`thread_lock::plan_capture`]); fixing only one side
///   yields 「argv is a new conversation but recovery is off」 and that incarnation's id is never written.
// ADR-0218
fn resume_argument(thread_id: Uuid) -> Option<String> {
    (!thread_id.is_nil()).then(|| thread_id.hyphenated().to_string())
}

/// The **diagnostic** marker left in the session record this incarnation creates (the value of [`ORIGINATOR_ENV`]).
///
/// ★There is no reader — do not write the contract 「recompute the same value and compare」★ (ADR-0218 decision 8): recovery is carried by the lock holder ([`thread_lock`]). The marker contains the
///   incarnation marker, so **the next incarnation computes a different value** and searching old records with it yields 0 hits without an error.
/// - Invariant within one incarnation; different when the incarnation changes (`epoch`'s job).
/// - Only `[a-z0-9-]`: any other character becomes one `-` per character.
///
/// ★Today `agent_id` is a uuid so the substitution never runs — do not remove it★: the value stays outside our process (the vendor record) and which characters that field accepts is upstream's business.
// ADR-0217
// ADR-0218
pub(crate) fn originator_marker(agent_id: AgentId, epoch: u32) -> String {
    sanitise_marker(&format!("engram-{agent_id}-{epoch}"))
}

/// The character rule of [`originator_marker`] — ★the one reason it stands separately is testability★: with production input (uuid + u32)
/// the substitution never runs, so measured through the assembly, it stays green even if the rule is erased.
fn sanitise_marker(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub struct CodexBackend;

impl AgentBackend for CodexBackend {
    /// ★The caller cannot decide the session id (measured)★ — codex has no `--session-id`-like flag and issues the id itself. If true, the manager would issue our uuid and **persist it in the profile**,
    /// codex would never use it, and the resume decision would stand on that fake value, believing a new conversation was resumed.
    /// ★Do not turn it on for app-server mode — both shapes are false★: that mode's identifier (thread id) is also issued by codex; whether it **can be resumed** is the separate axis below.
    // ADR-0185
    fn assigns_session_id(&self, _command: &AgentCommand) -> bool {
        false
    }

    /// ★Both modes resume — only the means differ★: app-server emits `thread/resume` with the stored thread id ([`AgentBackend::open_spawn`]); terminal mode launches carrying the same id as
    /// subcommand + positional argument ([`RESUME_SUBCOMMAND`] · the terminal branch of [`AgentBackend::build_spec`]).
    /// ★Do not make this look at [`is_app_server`]★: the question is **whether it can resume with the stored sid**, not through which link. Splitting by link would make terminal-mode codex launch as Fresh
    ///   even with a handle in the registry, and the recovered id ([`crate::profile::ProfileRegistry::observe_session_id`]) would never be used (ADR-0217).
    /// ★With **no** handle this field does not answer★ — [`crate::backend::can_resume_profile`] decides that together with the stored sid, and a spawn that still comes in as Resume is dropped to
    ///   new-conversation argv by [`AgentBackend::build_spec`].
    /// ★Turn it on **together with** the same predicate as `session.resume` of [`AgentBackend::capabilities`]★: either alone reports a never-resumed thread as 「resumed」 or a real resume as 「new conversation」.
    // ADR-0185
    // ADR-0208/ADR-0210
    fn can_resume_stored_session(&self, _command: &AgentCommand) -> bool {
        true
    }

    /// ★True while [`AgentBackend::accepts_mcp_config`] is false — the two axes are separate (ADR-0133)★: this one asks **does it consume the control channel** (by MCP or the CLI entrance / credential env);
    /// the field below asks only **can the mcp-config file be fed**.
    /// ★Reverting it to false makes `ENGRAM_TOKEN` · `ENGRAM_CONTROL_URL` vanish from codex spawn env★ — the assembly point (see [`crate::manager::AgentManager`]) skips provisioning, the endpoint comes as
    ///   `None`, and none of `build_spec`'s injection runs. Today's live consumers are the shell tools codex launches and MCP mail (the [`inject_cli_entrance`] doc has the credential condition); our hook
    ///   process is gone (ADR-0216), and codex overlaid none of its own env on it (0.155.0).
    /// ★This field **does not open mail**★: the daemon once derived mail-allowed from `!accepts_mcp_config` alone, so turning this on also opened send authorization (receiving closed by
    ///   [`AgentBackend::reads_messages`]). The send axis is now the separate [`AgentBackend::uses_mail`] and the daemon derives one value from both. ★Removing that field or reverting it to the default
    ///   brings the side effect back★ (it is on today **by declaration**).
    /// ★Without the `engram` executable the spawn **is not cut**★ — that fail-closed guards the pair 「CLI mail taught but no executable to call」, and a spawn outside the mail plane has no such pair (the daemon's
    ///   `control::provision` decides). Losing the control verbs remains, fail-open + warn.
    // ADR-0086
    // ADR-0132
    // ADR-0133
    // ADR-0208
    // ADR-0209
    fn supports_control_channel(&self) -> bool {
        true
    }

    /// ★Name and meaning disagree — turned on knowingly★: the name and the [`crate::types::ControlChannelNeeds`] doc ask 「can the mcp-config **file** we made be fed」, and codex's answer is **no** (no flag
    /// points to that file — measured 0.155.0). But the daemon derives the **mail channel decision** from this field (`mail_allowed = uses_mail && !accepts_mcp_config`), so false here, for a backend using MCP for
    /// mail, would **leave the CLI mirror open** — the opposite of ADR-0209's 「MCP is the official one, CLI is the mirror」.
    /// ★Turning it on once brought a file along — paid off★: the daemon wrote the mcp-config JSON (plaintext Bearer token) and the session settings fragment, neither read by codex, and that write is fail-closed,
    ///   so **a codex spawn could be cut over an unread file**. The derived axis was split: whether to write the file is decided by [`AgentBackend::writes_mcp_config_file`] alone; this field drives only
    ///   the mail channel decision. ★Removing that field or deriving it from this one brings the cost back★.
    /// ★The actual means of attachment is [`mcp_server_override`]★ — the single `-c mcp_servers.engram={…}` (measured 0.155.0, see the header).
    // ADR-0099
    // ADR-0128
    // ADR-0209
    fn accepts_mcp_config(&self) -> bool {
        true
    }

    /// ★True above, false here — that split is why this field exists★: codex has **no** flag to point to a file we wrote (0.155.0: neither `--mcp-config` nor `--settings` nor a `--config <file>` for settings).
    /// MCP attachment is only the single `-c mcp_servers.engram={…}` of [`mcp_server_override`] and the token goes to the env it names ([`MCP_BEARER_ENV_KEY`]) — nothing is copied to disk.
    /// ★False is a security decision — do not derive it from the field above★: true makes the daemon write a plaintext Bearer token JSON with the default ACL to the data directory on every spawn (nobody
    ///   opens it), and that write is **fail-closed**, cutting the codex spawn over that file.
    // ADR-0086
    // ADR-0099
    // ADR-0209
    fn writes_mcp_config_file(&self) -> bool {
        false
    }

    /// The translation and its refusals are [`launch::materialize`]'s; this reads the merged instructions file from
    /// disk.
    // ADR-0337
    fn materialize(
        &self,
        command: &AgentCommand,
        kit: &LaunchKit,
        cwd: &Path,
        fs: &dyn LaunchFs,
    ) -> Result<LaunchArtifacts, String> {
        launch::materialize(command, kit, cwd, fs, &|path| std::fs::read_to_string(path))
    }

    /// ★Once false — keep the reason it was dropped★: the old basis was 「this method does not receive `command`, so it cannot tell the two modes apart」. app-server mode emits turn signals (the link reads
    /// `turn/completed`; `turn/started` is **deliberately** not read — see that link's `TURN_COMPLETED` doc) and [`classify_turn`] emits `Ended`; terminal mode has no decoder, so the signal is 0 and
    /// true would **open that mode too**.
    /// ★That is no ground for blocking — terminal claude **already receives from exactly the same place**★ (zero turn signals, on the receiver roster). The policy is **ADR-0116 decision 7**: 「no turn signal →
    ///   inject immediately without a gate」 (the CLI's own input queue is the gate). Its rejected alternative killed 「it cannot be observed, so it cannot be delivered」 — do not go back to that premise.
    /// ★The cost is accepted knowingly★: with no busy gate an envelope can land mid-turn, and a TUI modal (approval prompt · menu · plan confirmation) can eat it (ADR-0116 decision 7; narrow it when observed).
    /// ★Do not open it like shell★ — shell's false is the axis 「**input is executed as a command**」, unaffected by this (the default implementation's doc).
    // ADR-0116
    // ADR-0209
    fn reads_messages(&self) -> bool {
        true
    }

    /// ★What this field buys, exactly★: when it is false the daemon's `build_grants` emits **nothing** (zero
    /// send-entrance grants). Closing it while opening only `accepts_mcp_config` above leaves the agent holding the MCP
    /// tool with no grant for it.
    /// ★Separate from the control verbs★ — this opens mail; the CLI entrance (token · address) and control routes are opened by `supports_control_channel` above.
    /// The receiving axis is open ([`AgentBackend::reads_messages`] is true: on the `messaging_host` delivery roster, so a `request`'s reply has somewhere to return), so ADR-0209 decision 4's 「receive first」 order holds.
    ///   ★Closing the receiving axis while leaving only this open revives the asymmetry.★
    // ADR-0133
    // ADR-0209
    fn uses_mail(&self) -> bool {
        true
    }

    /// ★Only app-server mode has a connection to set up★ — `turn/start` is allowed only after the handshake round trip; terminal mode is a PTY usable from process start (same as claude · shell).
    /// ★This declaration and the actual delivery **must not disagree**★ — true but not called: the supervisor waits until the backstop; false but called: nobody receives it. The test bench measures that pair.
    // ADR-0004
    fn declares_link(&self, command: &AgentCommand) -> bool {
        is_app_server(command)
    }

    /// Why did the resume fail — ★this backend's evidence is **the link's connection reason**★.
    ///
    /// ★Neither tail holds the reason★: a resume refusal is a JSON-RPC error on stdout, so the console tail (`terminal_tail`) does not exist on this link and the diagnostic tail (`diagnostic_tail`) holds only
    ///   stderr. So the decision takes the `reason` of [`crate::transport::LinkState::Down`] as evidence — without this declaration the classification falls to `None` and the **wrong context default** 「early exit right after
    ///   resume」 is stamped (this branch has no early exit; the process was alive).
    /// ★Do not tell apart by error code — `-32600` does not have one meaning★ (0.154.0): an unknown thread, a second `initialize`, a config error and **resuming a child thread directly** all carry it. Only the **message** is looked at.
    /// ★The declared wording is the one that was measured★ — `no rollout found for thread id` (`docs/reference/backend-capabilities.md` §1). For other reasons we have no real wording and **do not invent it** — it falls
    ///   to `None`, the caller uses the context default, and the human-readable reason goes out as the outcome's `reason` string anyway. The 「a child thread must resume its parent first」 wording is unknown too;
    ///   a guessed string would sit as dead code.
    /// ★Called on a live session too (the trait doc's condition)★ — the wording is emitted by the peer **only when refusing a request**, so it cannot mix into the conversation body.
    // ADR-0004
    // ADR-0082
    // ADR-0172
    fn resume_failure_kind(&self, evidence: &str) -> Option<AgentFailureKind> {
        if evidence.to_lowercase().contains(NO_ROLLOUT_MARKER) {
            return Some(AgentFailureKind::NoConversationToResume);
        }
        None
    }

    /// ★Fail-closed is set up in the same place as claude★ — there the daemon's `?` (mcp-config write failure) cuts it, but this backend writes no file (`writes_mcp_config_file` = false), so a failure to build the
    /// attachment value is visible **only here**.
    ///
    /// ★The cut condition is [`McpAttachment::Unrepresentable`] alone — do not widen it★: that state is 「on the delivery roster (`reads_messages`) but zero send entrances」 (MCP unattached, CLI mirror closed by
    ///   `mail_allowed=false` — always, since this backend has `accepts_mcp_config` true), so every delivered `request` becomes **a contract nobody can answer** and the symptom is eternal silence, not an error.
    /// ★Left open (both normal)★: ① no control channel — codex without mail is legitimate; ② the daemon did not authorize the MCP send entrance (operator turned MCP mail off) — that spawn expects no mail. Cutting both would
    ///   kill codex spawns in installs without `engram` and in harnesses with the knob on.
    // ADR-0004
    // ADR-0209
    fn precheck_control_endpoint(
        &self,
        _command: &AgentCommand,
        control: Option<&ControlEndpoint>,
    ) -> Result<(), String> {
        match mcp_attachment(control) {
            McpAttachment::Attach(_) | McpAttachment::NotWanted => Ok(()),
            McpAttachment::Unrepresentable(reason) => {
                tracing::warn!("codex spawn fail-closed — {reason}");
                Err(reason)
            }
        }
    }

    /// ★`session_id` is **still** not assembled — do not revive it★: `--session-id` does not exist ([`AgentBackend::assigns_session_id`] is false, so this field is always `None`); carrying it would push a
    /// uuid codex never uses onto the command line.
    /// ★What is resumed is `resume_session_id` — do not fold the two fields★: resume is **a subcommand + positional argument** ([`RESUME_SUBCOMMAND`]), a different assembly shape; folding makes
    ///   `assigns_session_id() == false` false.
    /// ★Only terminal mode is assembled★ — app-server's resume is the second handshake request (`thread/resume`, picked by [`AgentBackend::open_spawn`]): the same value by different means, not twice.
    /// ★Resume without a handle **falls to new-conversation argv**★ — `codex resume` without an id brings up the **TUI picker** (0.155.0 `--help`: "picker by default"), pinning the PTY-attached agent to the first
    ///   screen forever. So the subcommand is dropped and argv is **byte-identical** to Fresh.
    ///   ★That regression is reported elsewhere, not here★: `resume_no_fallback` of [`crate::manager::AgentManager`] decides the same condition before spawn (`opens_a_new_conversation`), warns, and emits
    ///   `Started` instead of `Resumed`; reporting here too would write one fact from two diverging sources.
    // ADR-0004
    // ADR-0185
    // ADR-0208/ADR-0210
    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        _session_id: Option<Uuid>,
        resume_session_id: Option<Uuid>,
        cwd: PathBuf,
        mut env: Vec<(String, String)>,
        control: Option<ControlEndpoint>,
        artifacts: &LaunchArtifacts,
    ) -> CommandSpec {
        match command {
            AgentCommand::Codex {
                extra_args,
                output_format,
            } => {
                let mut args = Vec::with_capacity(8 + extra_args.len());
                match output_format {
                    AgentOutputFormat::Terminal => {
                        // ★The subcommand and its positional argument must be **at the very front**★ — pushed back, clap reads `resume` as the root's `[PROMPT]` (the app-server branch's same constraint is
                        //   pinned by `app_server_extra_args_come_last`). ★Emit them as a pair★: `codex resume` without an id is the TUI picker (the doc above).
                        if let (SpawnMode::Resume, Some(thread_id)) = (mode, resume_session_id) {
                            match resume_argument(thread_id) {
                                Some(arg) => {
                                    args.push(RESUME_SUBCOMMAND.to_string());
                                    args.push(arg);
                                }
                                None => tracing::warn!(
                                    "the codex resume handle does not have a carriable shape — launching as a new conversation"
                                ),
                            }
                        }
                        args.push(CD_FLAG.to_string());
                        args.push(cwd.to_string_lossy().into_owned());
                        args.push(SANDBOX_FLAG.to_string());
                        args.push(SANDBOX_WORKSPACE_WRITE.to_string());
                        args.push(APPROVAL_FLAG.to_string());
                        args.push(APPROVAL_ON_REQUEST.to_string());
                    }
                    // ★Not carrying the policies and working folder here is intended★ — app-server takes them as `thread/start` parameters (`cwd` · `sandbox` · `approvalPolicy`, measured 0.154.0), built in
                    //   [`AgentBackend::open_spawn`]; the process cwd stays `CommandSpec.cwd` (same value, different field).
                    AgentOutputFormat::StreamJson => {
                        args.push(APP_SERVER_SUBCOMMAND.to_string());
                        args.push(APP_SERVER_STDIO_FLAG.to_string());
                    }
                }
                // Our arguments come first and the caller pass-through follows — terminal mode's three flags each take a single value, so they do not absorb following arguments (unlike claude's variadic `--allowedTools`).
                // ★The pass-through does not tell modes apart, though the two modes' option sets differ★ (0.154.0: `codex --help` and `codex app-server --help` each have flags the other lacks), so an
                //   argument written for the interactive CLI may be refused by app-server. Still not filtered per mode: upstream changes what is valid per version, and a stale list quietly erases sound arguments.
                args.extend(extra_args.iter().cloned());
                // ★Known limit — a path containing `%VAR%` is substituted while passing the shim (decided not to fix)★. `console_command` wraps it in `cmd.exe /c` on Windows and cmd expands `%NAME%` on the command
                //   line **even inside quotes**, so a real folder like `C:\x\%USERNAME%\y` makes codex see **a different folder** as its workspace. Terminal mode: the `CD_FLAG` value and `extra_args`; app-server
                //   mode: only `extra_args` (the working folder goes as a `thread/start` parameter, not through cmd).
                //   ★Why not fixed★: ① the shim cannot be removed (TRD §6-H: `codex` on PATH is a `.cmd`, which `CreateProcessW` cannot launch directly); ② the known fix is **not an ordinary escape** — rust std's
                //   `.bat` trick (`%%cd:~,%` at every `%`) presumes `cmd /e:ON` + `/c "one string"` (the RUSTSEC-2024-0006 response), while portable-pty hands an argument array to `CreateProcessW` with no `%`
                //   handling, so moving it is an unverified port; ③ that port touches `console_command`, **shared with claude**, and a mistake kills every running spawn.
                //   ★To fix it★: measure that claude and codex both come up under a `cmd` wrapped in that form.
                // On real materials the terminal working folder never holds two `%` (`materialize` refuses it); a single
                //   `%` passes, so there this limit is left to `extra_args` — a `%…%` of their own, or one `%` in them
                //   pairing with the folder's lone `%` (tracking T-102).
                // ADR-0004
                // ADR-0353

                // Real materials: the preset `-c` lines (model and effort included — there is never a `-m`), after the
                //   pass-through and before our overrides, so ours still come last and win. Empty for Noop materials.
                // ADR-0337
                args.extend(artifacts.config_args.iter().cloned());
                // The codex terminal hook lines go right after the preset lines they were folded from — still before
                //   our overrides. Empty in JSON mode (the thread config carries the hooks) and for Noop materials.
                // ADR-0420
                args.extend(artifacts.hook_args.iter().cloned());

                // ★The overrides are all **after** the pass-through — this place is the source of truth for that discipline★★ (sibling blocks point here). Measured (0.155.0): passing the same config key twice with `-c`,
                //   **the last one wins** — no merge, no duplicate-key error. In front, one line of user arguments would erase our value with no signal. ★Do not move these blocks back into the terminal branch
                //   in the name of 「tidying argument assembly」★.
                // ★If we override a user value, **we say so — silent overriding is forbidden**★ (ADR-0216 decision 1). Still not filtered (that would be us censoring user arguments); only the fact that ours won is left
                //   (also the source of truth for the sibling blocks).
                // ★MCP server attachment does not tell modes apart★: the mail entrance is needed in both. What tells modes apart is where the peer **already provides the same value by other means** (session id · priming).
                // ★This one value is the **only physical wiring** of codex mail★ — claude reads the mcp-config file, codex takes only this override (0.155.0). Without it there is no `eg_send` tool, and the daemon
                //   (`mail_allowed=false`) closes even the CLI mirror, so **send entrances become 0** — silence, not an error.
                // ADR-0128
                // ADR-0209
                // ★The two 「do not attach」 branches are **not told apart** here★ — [`McpAttachment::NotWanted`] and [`McpAttachment::Unrepresentable`] both mean the argument is not carried; cutting the
                //   failure-to-build side as a spawn abort is [`AgentBackend::precheck_control_endpoint`]'s job, this function only builds arguments.
                match mcp_attachment(control.as_ref()) {
                    McpAttachment::Attach(value) => {
                        let key = format!("{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}");
                        if passthrough_overrides_key(extra_args, &key) {
                            tracing::warn!(
                                "the codex pass-through set `{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}` directly — carrying our attachment after it for the mail entrance, overriding that value (the last `-c` wins)"
                            );
                        }
                        // ADR-0337
                        if passthrough_overrides_key(&artifacts.config_args, &key) {
                            tracing::warn!(
                                "a codex preset `-c` line sets `{key}` — our MCP attachment comes later and overrides it (the last `-c` wins)"
                            );
                        }
                        args.push(CONFIG_OVERRIDE_FLAG.to_string());
                        args.push(value);
                    }
                    McpAttachment::NotWanted | McpAttachment::Unrepresentable(_) => {}
                }

                // ★Do not revive the `SessionStart` hook here (ADR-0216/ADR-0218)★ — recovery is the writer-lock name this child holds ([`thread_lock`]) and the hook machinery was removed. Not because the hook was broken
                //   (it ran, 0.155.0): codex hashes the hook **command string** and requires human approval, and that string embeds our exe's absolute path, which changes every release (folder named after the tag),
                //   so the approval screen returns every release and **while it is up, terminal codex does not come up at all.**

                // ★Priming injection — only terminal mode is assembled here★: app-server carries the same content in the handshake JSON (`developerInstructions` of `thread/start`, [`AgentBackend::open_spawn`]) —
                //   same value, different means per mode, like resume.
                // ★Why modes are told apart★: this command line has an 8191-character cap the brief can eat whole; the JSON body has no cap (measured up to 8 MB), so carrying it on the command line in both modes
                //   would burden app-server with the cap and the `%` · newline risks for nothing.
                // ★It is **after** the pass-through★ (last `-c` wins; reason: the MCP attachment block above).
                // ★The value is the one `materialize` built from the merged instructions file★ — a value the line
                //   could not carry already failed the spawn there. Without real materials there is none.
                // ADR-0004
                // ADR-0092
                // ADR-0215
                // ADR-0337
                // ADR-0370
                if matches!(output_format, AgentOutputFormat::Terminal) {
                    if let Some(value) = artifacts.instructions_override.clone() {
                        if passthrough_overrides_key(extra_args, DEVELOPER_INSTRUCTIONS_KEY) {
                            tracing::warn!(
                                "the codex pass-through set `{DEVELOPER_INSTRUCTIONS_KEY}` directly — carrying our value after it for the receive-contract priming, overriding that value (the last `-c` wins)"
                            );
                        }
                        if passthrough_overrides_key(
                            &artifacts.config_args,
                            DEVELOPER_INSTRUCTIONS_KEY,
                        ) {
                            tracing::warn!(
                                "a codex preset `-c` line sets `{DEVELOPER_INSTRUCTIONS_KEY}` — our priming comes later and overrides it (the last `-c` wins)"
                            );
                        }
                        args.push(CONFIG_OVERRIDE_FLAG.to_string());
                        args.push(value);
                    }
                }

                // ADR-0086 step 2 (CLI entrance) — ★does not tell modes apart★: only three env values are planted, and what reads them is **the children codex launches** (shell tools · the user's own hooks) — both modes launch children.
                // ★`config_path` **does not come — the declared result**★: it is a **file claude reads by path**, and codex has no such flag (0.155.0: no `--mcp-config`, no `--settings`, no `--config <file>`
                //   for settings), so this folder declares [`AgentBackend::writes_mcp_config_file`] false and the daemon decides the write by that one field. MCP attachment is the single `-c` value above.
                //   ★Before that field was split off, **a plaintext token file nobody reads appeared** on every spawn★ (the derived axis served both 「uses MCP for mail」 and 「takes our mcp-config file」); reviving that
                //   dual role (removing the field or deriving it from `accepts_mcp_config`) brings back the waste and the fail-closed abort risk.
                //   ★The injection below ignores that field even if it comes as `Some` — keep that defence★: the control channel is not used by this backend alone, and copying claude's flags because 「it is Some」
                //   kills the launch (`the_daemon_written_files_are_still_not_translated_into_argv`).
                // ★`grants` is not used either★ — it is **pre-approval** of permission prompts, not a tool allow list (ADR-0094); translating it into `enabled_tools` changes 「approved in advance」 into 「only these exist」.
                // ADR-0086 / ADR-0092 / ADR-0133 / ADR-0209
                //
                // ★The incarnation marker is inside the same door — do not take it outside★: a spawn with no endpoint has nothing to do with our control plane, and outside the door our marker would remain in such
                //   spawns' session records (`without_an_endpoint_nothing_is_injected`). The material `(agent_id, epoch)` lives in the endpoint.
                // ★It does not tell modes apart★ — it sits in the session record codex writes, which both modes create.
                // ★Only planted — **no reader, none planned**★ (ADR-0218 decision 8): recovery is the lock holder; the marker is for diagnostics and later comparison.
                // ADR-0217
                // ADR-0218
                if let Some(endpoint) = &control {
                    inject_cli_entrance(&mut env, endpoint);
                    env.push((
                        ORIGINATOR_ENV.to_string(),
                        originator_marker(endpoint.agent_id, endpoint.epoch),
                    ));
                }
                let (program, args) = console_command(CODEX_PROGRAM, args);
                CommandSpec {
                    program,
                    args,
                    env,
                    cwd,
                }
            }
            AgentCommand::Claude { .. } | AgentCommand::Shell { .. } => {
                unreachable!("CodexBackend handles only the Codex variant. dispatch bug.")
            }
        }
    }

    /// `session.resume` is ★independent of the issuer★ — restore depends **solely** on the backend sid stored in the profile; codex receives and uses it (app-server via the `thread/start` response, terminal mode via
    /// the writer-lock name its child holds — [`thread_lock`] · ADR-0218), so both modes resume.
    /// ★**The same predicate as [`AgentBackend::can_resume_stored_session`] above — do not touch only one side**★: that axis is where the activation entrance asks 「launch as Resume?」 and this field reports the
    /// result to consumers; a split reports a new thread as 「resumed」 or the reverse.
    /// `model.select` is false even though codex has `-m` — this field reports **what this spawn uses**, and we do not expose that field.
    // ADR-0185
    // ADR-0208/ADR-0210
    fn capabilities(&self, _command: &AgentCommand) -> BackendCaps {
        BackendCaps {
            session: SessionCaps {
                resume: true,
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

    /// app-server mode requires a pipe through which envelopes flow **both ways** — the peer asks us too.
    ///
    /// ★Only the reported value; the real thing is built by [`AgentBackend::open_spawn`]★ — both look at [`is_app_server`], so fixing one side makes them disagree.
    fn transport_shape(&self, command: &AgentCommand) -> TransportShape {
        if is_app_server(command) {
            TransportShape::StdioBidiJson
        } else {
            TransportShape::Pty
        }
    }

    /// app-server mode builds a bidirectional JSON link, terminal mode builds a PTY.
    ///
    /// ★The decision is [`is_app_server`] alone — do not read back the `transport_shape` report and match on it (ADR-0191)★ (that makes a second switch on the shape value).
    /// ★This is where `structured: true` and the `thread/start` policies are injected (ADR-0044/0030)★: the link knows neither what its bytes are nor which folder to open with which sandbox; this backend, which picked the
    ///   mode and policies, does.
    /// ★`sid_sink` is used by both branches — only the means differ★: app-server **receives** the identifier in the `thread/start` response and the link hands it over as is (before anything is sent with that session —
    ///   [`AgentBackend::open_spawn`]'s ordering contract); the terminal branch has no response, so it polls the child's writer lock and puts that file name into the same verb ([`thread_lock`] · ADR-0218).
    ///   ★Do not revive a constructor that reads the PTY screen (ADR-0217)★ — recovery is decided by ownership. ★Do not create a second write path★ — the assembly point tied the incarnation guard to this verb, and
    ///   another branch touching the profile directly bypasses it (ADR-0218 decision 6).
    /// ★`thread/start` vs `thread/resume` is also picked here★ by the single `resume_session_id` the assembly point passed ([`thread_open`]); the link is not asked again (two deciding places otherwise).
    /// ★`resume_session_id` is unused in the terminal branch — **not** because there is no handle★: that mode's resume already went out in argv ([`AgentBackend::build_spec`]); using it here would resume by two means.
    /// ★`artifacts` is read **only on the app-server branch, by the same rule**★ — what comes out of it is the open
    ///   plan ([`app_server_open`]: the instructions — the merged text on a real spawn, none on a Noop one —, the skill
    ///   roots and the first-turn context). The terminal branch already put its instructions and settings on the command
    ///   line (`build_spec`), so it does not reach the PTY constructor. `control` is not read: codex carries no
    ///   control-plane data in the handshake (ADR-0370).
    // ADR-0185
    // ADR-0191
    // ADR-0215
    // ADR-0337
    fn open_spawn(
        &self,
        command: &AgentCommand,
        spec: &CommandSpec,
        cols: u16,
        rows: u16,
        sid_sink: Option<SessionIdSink>,
        first_turn_sink: Option<FirstTurnSink>,
        resume_session_id: Option<Uuid>,
        link_sink: Option<LinkSink>,
        _control: Option<&ControlEndpoint>,
        artifacts: &LaunchArtifacts,
    ) -> Result<SpawnParts, PtyError> {
        let (transport, child_pid, mid_turn, delivery_ack): (
            Box<dyn AgentTransport>,
            Option<u32>,
            MidTurnPolicy,
            Arc<DeliveryAck>,
        ) = if is_app_server(command) {
            // ★The pickup decision is made here — `output_decoder` does not receive the spawn spec (env), so it cannot decide★.
            //   Keeps observations of a profile that is not the default account from mixing into the default field (`usage.rs`).
            let decoder = self
                .output_decoder(command)
                .map(|d| usage::gate_decoder(d, &spec.env));
            let (t, pid) = CodexAppServerTransport::open(
                spec,
                true,
                decoder,
                app_server_open(spec, resume_session_id, artifacts)?,
                sid_sink,
                link_sink,
            )?;
            // ★`TransportOwned` and the first-turn port are a pair — keep both★: that mode's session does not count submissions, so without the port no incarnation can persist the thread id (no error).
            // ADR-0226 · ADR-0231: the id is persisted when it became real (the peer's first user message echo).
            let t = t.with_first_turn(first_turn_sink);
            // ADR-0231: the link carries classification · handover · cancellation. Whether receipt is possible is **the very**
            //   Arc that the link's floor decision fills — making a separate one would have the session and the link see different values.
            let ack = t.delivery_ack();
            (Box::new(t), pid, MidTurnPolicy::TransportOwned, ack)
        } else {
            // ★Terminal mode has no connection to set up★ — `declares_link()` is false, so the port is `None` too.
            // ★The first-turn port is dropped too★ — in this mode the session counts submissions (ADR-0226 decision 11 — the later of recovery and submission).
            drop(first_turn_sink);
            let (t, pid) = PtyTransport::open(spec, cols, rows)?;
            // ★Session id recovery starts **from here** — the lock appears only once the child is up★ (ADR-0218). Whether to run it and its material are decided by [`thread_lock::plan_capture`] (the conditions' source of truth);
            //   this place only hands over the child's two identity fields and **our side's** default lock folder (a different home the child received wins).
            // ADR-0218
            let child_start =
                pid.and_then(engram_dashboard_platform::process::process_creation_time);
            // ★The gate looks at 「did the resume argv actually go out」, not the **existence** of the handle★: for an uncarriable value `build_spec` emitted new-conversation argv and that incarnation is a recovery target
            //   (same predicate: [`resume_argument`]).
            let resumes_by_argv = resume_session_id.and_then(resume_argument).is_some();
            if let Some(plan) = thread_lock::plan_capture(
                &spec.env,
                resumes_by_argv,
                pid,
                child_start,
                sid_sink,
                thread_lock::lock_dir(),
            ) {
                thread_lock::spawn_capture(plan);
            }
            // Terminal is today's path as is — no list and no receipt notification.
            (
                Box::new(t),
                pid,
                MidTurnPolicy::None,
                Arc::new(DeliveryAck::new()),
            )
        };
        Ok(SpawnParts {
            transport,
            child_pid,
            backend_caps: self.capabilities(command),
            encoder: self.input_encoder(command),
            turn_classifier: self.turn_classifier(),
            reads_messages: self.reads_messages(),
            mid_turn,
            delivery_ack,
        })
    }

    /// In app-server mode **the link** builds the envelope — there is nothing for the encoder to wrap.
    fn input_encoder(&self, command: &AgentCommand) -> InputEncoder {
        if is_app_server(command) {
            InputEncoder::TransportFramed
        } else {
            InputEncoder::Raw
        }
    }

    fn turn_classifier(&self) -> TurnClassifier {
        classify_turn
    }

    // ★No synthetic input echo is declared — do not revive it★: this backend echoes user messages itself as `userMessage` items of `item/*` (measured), and the translator lets them through as 「what we sent」.
    //   In app-server mode the idle input bubble is emitted **by the link** (only it knows idle vs mid-turn; mid-turn text goes into the list with no bubble) and the bubble's uuid is our id, folding into one copy with
    //   the echo's `clientId`. The session-layer echo does not know that split, so it emits none (the link-turn verb branch of `AgentSession`).
    //   ★A link below the floor does not emit the bubble★: the echo bubble's `uuid` is `clientId` first and the codex item id only when absent; the echo below the floor has no `clientId`, so a bubble with our id
    //   could not be folded by the frontend's `uuid` dedup and the question would show twice (what ADR-0193's 「rejected alternative」 dismissed on measurement). In terminal mode the PTY echoes on its own.
    // ADR-0193
    // ADR-0231

    /// ★If not declared, the translator is not assembled and the bytes flow as is, breaking the screen — no error and no warning★.
    fn output_decoder(&self, command: &AgentCommand) -> Option<Box<dyn OutputDecoder>> {
        if is_app_server(command) {
            Some(Box::new(CodexAppServerDecoder::new()))
        } else {
            None
        }
    }
}

/// codex output event → turn signal (ADR-0113).
///
/// ★This backend's end signal is [`OutputEvent::TurnEnd`]★ — the translator does not emit `MessageDone`, yet that branch is also written as `Ended` so the end does not vanish if this backend ever emits it.
/// ★`Structured` counts as progress — load-bearing★: the only thing entering this branch is the echoed user message (`userMessage` item translation; unknown notifications are dropped — `decoder` header), the first
///   observation of 「this turn has started」. ★The window from our `turn/start` write to that first notification is unobserved★ — not filled with a synthetic echo (ADR-0193); queue release asks the link
///   implementation, and the mail gate absorbs the gap as fail-open (ADR-0104).
/// ★`Usage`/`Error` are not ends★: `Usage` (`thread/tokenUsage/updated`) comes mid-turn; `Error` is a retryable stream error. A failed turn is closed by the `TurnEnd` that `turn/completed` emits (the reason is **inside** it).
/// ★Give-up paths where `turn/completed` never comes also close with `TurnEnd`★ (write failure · response decode failure · error response · deadline expiry — the link's `end_turn_if`). Writing `Error` there left
///   **an idle agent observed as mid-turn** and mail blocked until the 30-minute fail-open valve (ADR-0127), so that side uses the boundary vocabulary instead of promoting `Error` here.
/// ★Shared with terminal mode★: that mode has no decoder, only `TerminalBytes`, so this mapping yields no signal.
/// ★Safe with no `Progress` before `Ended`★: `Ended` only writes `in_turn = false`, so an unpaired end does not turn on mid-turn (`crate::turn::TurnObservations::observe_at`), and `in_turn_snapshot` carries only `in_turn` ones;
///   no 「start signal」 is invented. ★An unpaired error end is not the state right after registration★ — it sets `last_end_failed` (the `turn/start` exit is intended: a request-stage failure is also an abnormal end), and a
///   never-established incarnation's boundary is erased by `forget`.
/// ★Error end = a `TurnEnd{Failed}` attributed to this incarnation's turn★ — a translated `turn/completed(failed)` (incl. the early-end release) and the link's give-up path (`end_turn_if`); the turn table folds it into
///   `last_end_failed` and the kernel holds mail (ADR-0231, same trigger as the link's `halted`, which only this classifier uses). ★Someone else's turn end blocked by the attribution gate and the display-only boundary
///   after a late receipt (`emit_without_turn_observation`) do not pass here★ — they neither set nor clear the halt.
/// ★Registry events up to `Delivered` are all `None` (unlike claude)★: `Delivered` comes **even after** the turn end (a late echo of text returned to the held place · revival); turning 「mid-turn」 on again would block that
///   incarnation's mail until the 30-minute valve. Turn start is already carried by the echoed user message (`Structured`).
/// ★`ToolResult` is `None` whatever its outcome★: an interrupted command keeps running and closes **after the turn end** (fixture `interrupt_fail_b5`) — counted as progress an idle incarnation would revive as 「mid-turn」;
///   and a tool failure is an ordinary agent step, not an error that sets the halt-after-error.
// ADR-0113
// ADR-0004
// ADR-0231
// ADR-0241
pub(crate) fn classify_turn(event: &OutputEvent) -> Option<TurnSignal> {
    match event {
        OutputEvent::TextDelta { .. }
        | OutputEvent::ToolCall { .. }
        | OutputEvent::Structured { .. } => Some(TurnSignal::Progress),
        OutputEvent::TurnEnd { outcome, .. } => Some(TurnSignal::Ended(match outcome {
            TurnOutcome::Completed => TurnEndKind::Clean,
            TurnOutcome::Failed { .. } => TurnEndKind::Failed,
            // An interrupted end neither sets nor clears the halt (ADR-0192 — aligned with claude). The same goes for an outcome of unknown meaning.
            TurnOutcome::Interrupted | TurnOutcome::Unknown => TurnEndKind::Other,
        })),
        // An end that carries no outcome, so it neither sets nor clears the halt-after-error.
        OutputEvent::MessageDone { .. } => Some(TurnSignal::Ended(TurnEndKind::Other)),
        OutputEvent::Usage { .. }
        | OutputEvent::ToolResult { .. }
        | OutputEvent::Error(_)
        | OutputEvent::TerminalBytes(_)
        | OutputEvent::QueuedInput(_) => None,
    }
}

#[cfg(test)]
mod tests {
    /// Polls until the condition holds — ★past the deadline it dies **saying what it was waiting for**★: a silent fallthrough once left an empty panic body that could not tell 「timeout」 from 「value error」 (v0.3.1 tag run).
    /// ★The cap is a safety net, not an assertion — do not shrink it to 「fail fast」★: the normal path holds within 1 s, a congested CI runner ran the suite **6.7× slower** (local 4.87 s ↔ CI 32.71 s); it measures
    /// 「did it happen」, not 「how fast」.
    fn wait_until(mut cond: impl FnMut() -> bool, what: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !cond() {
            if std::time::Instant::now() >= deadline {
                let message = format!(
                    "{what} — passed the deadline while waiting. The value is not wrong; **it did not happen**"
                );
                // ★Before panicking, write first to stderr, which does not go through capture★ — the panic body goes out through the panic hook,
                //   and disappears inside a quiet-hook span or if the binary aborts. This line remains in both (measured).
                let thread = std::thread::current();
                let name = thread.name().unwrap_or("<unnamed>");
                let _ = std::io::Write::write_all(
                    &mut std::io::stderr(),
                    format!("thread '{name}': {message}\n").as_bytes(),
                );
                panic!("{message}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    use super::protocol::method;
    use super::*;
    use crate::types::{ToolGrant, TurnOutcome, CLI_EXE_ENV};

    fn codex(extra_args: Vec<&str>) -> AgentCommand {
        AgentCommand::Codex {
            extra_args: extra_args.into_iter().map(String::from).collect(),
            output_format: AgentOutputFormat::Terminal,
        }
    }

    fn codex_app_server(extra_args: Vec<&str>) -> AgentCommand {
        AgentCommand::Codex {
            extra_args: extra_args.into_iter().map(String::from).collect(),
            output_format: AgentOutputFormat::StreamJson,
        }
    }

    fn spec(command: &AgentCommand, cwd: &str) -> CommandSpec {
        CodexBackend.build_spec(
            command,
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from(cwd),
            vec![],
            None,
            &LaunchArtifacts::default(),
        )
    }

    /// codex's own argv with the `console_command` wrapping stripped. On Windows it is wrapped in one layer as `cmd.exe /c codex …`
    /// (shim resolution), so to write the assertions of both platforms as one, that layer is peeled off here.
    fn codex_argv(spec: &CommandSpec) -> Vec<String> {
        #[cfg(windows)]
        {
            assert_eq!(spec.program, "cmd.exe");
            assert_eq!(spec.args[0], "/c");
            assert_eq!(spec.args[1], CODEX_PROGRAM);
            spec.args[2..].to_vec()
        }
        #[cfg(not(windows))]
        {
            assert_eq!(spec.program, CODEX_PROGRAM);
            spec.args.clone()
        }
    }

    /// The `hooks.SessionStart` line of an argv. ★With no hook output written it must **always be `None`** — Engram's own
    /// session-id hook stays removed (ADR-0216, U3)★; a hook component's line is the user's (ADR-0420), and
    /// [`tests::a_hook_components_session_start_line_carries_only_the_users_command`] measures that it holds nothing of ours.
    /// ★The key is a literal on purpose★ — production code must hold none of this string, and a revived constant would blur that.
    /// ★Do not count by the **existence** of `-c`★ — two things carry it today (MCP attachment · the brief); a flag-only count turned false-positive with each new one.
    fn hook_override_value(argv: &[String]) -> Option<String> {
        argv.windows(2)
            .find(|p| p[0] == CONFIG_OVERRIDE_FLAG && p[1].starts_with("hooks.SessionStart"))
            .map(|p| p[1].clone())
    }

    /// ★If the endpoint is `None`, not a single `-c` stands★ — the two that carry that flag today (MCP attachment · the brief)
    /// both get their material from the endpoint.
    #[test]
    fn interactive_args_are_the_measured_ones() {
        let s = spec(&codex(vec![]), "C:/workspace");
        assert_eq!(
            codex_argv(&s),
            vec![
                "--cd",
                "C:/workspace",
                "-s",
                "workspace-write",
                "-a",
                "on-request",
            ]
        );
    }

    #[test]
    fn app_server_args_are_the_measured_ones() {
        let s = spec(&codex_app_server(vec![]), "C:/workspace");
        assert_eq!(codex_argv(&s), vec!["app-server", "--stdio"]);
    }

    /// ★The policies and working folder do not go into argv★ — app-server takes those three as `thread/start` parameters.
    /// If carried here, the peer receives arguments unknown to that subcommand and the launch fails.
    #[test]
    fn app_server_argv_carries_no_policy_and_no_workspace_path() {
        let s = spec(&codex_app_server(vec![]), "C:/workspace");
        let argv = codex_argv(&s);
        for forbidden in [CD_FLAG, SANDBOX_FLAG, APPROVAL_FLAG, "C:/workspace"] {
            assert!(
                !argv.iter().any(|a| a == forbidden),
                "`{forbidden}` was carried in app-server argv: {argv:?}"
            );
        }
    }

    /// The process cwd is the same in both modes — wherever the workspace folder is carried, the process comes up in the same place.
    #[test]
    fn process_cwd_is_the_same_in_both_modes() {
        let cwd = PathBuf::from("C:/workspace");
        assert_eq!(spec(&codex(vec![]), "C:/workspace").cwd, cwd);
        assert_eq!(spec(&codex_app_server(vec![]), "C:/workspace").cwd, cwd);
    }

    /// ★「After our default arguments」, not 「at the very end」★ — the old name (`extra_args_come_last`) predates our overrides; that the overrides stand after the pass-through is the separate axis measured by the
    /// `…_follows_the_passthrough` items.
    #[test]
    fn extra_args_follow_our_base_args() {
        let s = spec(&codex(vec!["-m", "gpt-5"]), "C:/workspace");
        let argv = codex_argv(&s);
        let at = argv
            .iter()
            .position(|a| a == "-m")
            .expect("the pass-through disappeared");
        assert_eq!(&argv[at..at + 2], &["-m".to_string(), "gpt-5".to_string()]);
        assert!(
            argv[..at].iter().any(|a| a == APPROVAL_ON_REQUEST),
            "the pass-through went in front of our default arguments: {argv:?}"
        );
    }

    /// ★Session arguments must not be assembled★ — it is syntax that does not exist, so the moment it is attached codex fails to launch.
    #[test]
    fn session_id_never_reaches_the_command_line() {
        let sid = Uuid::new_v4();
        let s = CodexBackend.build_spec(
            &codex(vec![]),
            SpawnMode::Resume,
            Some(sid),
            None,
            PathBuf::from("."),
            vec![],
            None,
            &LaunchArtifacts::default(),
        );
        assert!(
            !s.args.iter().any(|a| a.contains(&sid.to_string())),
            "the sid was carried in argv: {:?}",
            s.args
        );
        assert!(
            !s.args
                .iter()
                .any(|a| a == "--session-id" || a == "--resume" || a == "--session"),
            "a session flag that does not exist was carried: {:?}",
            s.args
        );
    }

    // ── ADR-0208: terminal-mode resume (`codex resume <id>`) ──────────────────────

    /// A spec drawn with the resume field filled — the issued field (`session_id`) is left empty.
    ///
    /// ★The two fields are passed **separately** on purpose★ — folded into one, the value we issued could be carried in the argv of a backend whose [`AgentBackend::assigns_session_id`] is false, and the items below cannot see that.
    fn spec_resuming(
        command: &AgentCommand,
        mode: SpawnMode,
        resume_session_id: Option<Uuid>,
        cwd: &str,
    ) -> CommandSpec {
        CodexBackend.build_spec(
            command,
            mode,
            None,
            resume_session_id,
            PathBuf::from(cwd),
            vec![],
            None,
            &LaunchArtifacts::default(),
        )
    }

    /// ★The subcommand and positional argument go out **attached at the very front**★ — if `resume` is pushed back clap reads it as the root's `[PROMPT]` and the resume quietly becomes a new conversation; if the id separates from the
    /// subcommand it misses `[SESSION_ID]`. The three policies follow **as is** (`resume` takes them as its own options too, 0.155.0).
    #[test]
    fn resuming_with_a_stored_handle_emits_the_subcommand_and_the_positional_id() {
        let thread_id = Uuid::new_v4();
        let s = spec_resuming(
            &codex(vec![]),
            SpawnMode::Resume,
            Some(thread_id),
            "C:/workspace",
        );
        assert_eq!(
            codex_argv(&s),
            vec![
                "resume",
                &thread_id.to_string(),
                "--cd",
                "C:/workspace",
                "-s",
                "workspace-write",
                "-a",
                "on-request",
            ]
        );
    }

    /// ★Without a handle, argv must be **byte-identical to Fresh**★ — `codex resume` without an id is the TUI picker (0.155.0 `--help`: "picker by default") and the agent stops on the first screen forever; a half assembly
    /// emitting only the subcommand is forbidden.
    #[test]
    fn resuming_without_a_stored_handle_falls_back_to_the_fresh_argv() {
        let fresh = codex_argv(&spec(&codex(vec![]), "C:/workspace"));
        let fell_back = codex_argv(&spec_resuming(
            &codex(vec![]),
            SpawnMode::Resume,
            None,
            "C:/workspace",
        ));
        assert_eq!(
            fell_back, fresh,
            "a Resume without a handle emitted argv different from Fresh — a half assembly gets caught in the TUI picker"
        );
        assert!(
            !fell_back.iter().any(|a| a == RESUME_SUBCOMMAND),
            "only the subcommand was carried without an id — the picker comes up and the agent stops on the first screen: {fell_back:?}"
        );
    }

    /// ★The **reporting** of that regression is the assembly point's, not this file's★: `resume_no_fallback` of [`crate::manager::AgentManager`] decides `opens_a_new_conversation` as (resume axis ∧ no handle in the registry) and
    /// emits `Started` instead of `Resumed` (ADR-0226). **The resume axis is this file's declaration**, so it is pinned here.
    /// ★An execution assertion does not catch this regression★ — if the axis flips, a handle-less request on the resume path still falls cleanly to new-conversation argv; only **the report** turns false.
    // ADR-0208
    #[test]
    fn a_handleless_resume_still_trips_the_new_conversation_notice() {
        let terminal = codex(vec![]);
        assert!(
            CodexBackend.can_resume_stored_session(&terminal),
            "if the resume axis is off, that product is always false, so a new conversation opened without a handle is quietly \
             reported as 「resumed」"
        );
    }

    /// ★Fresh does not resume even with a handle★ — the assembly point passes this field empty on Fresh (`AgentManager::spawn_agent`), but leaning on that would put the decision in two places. A request to open a new
    /// conversation with a dead incarnation's thread is exactly that combination.
    #[test]
    fn a_fresh_spawn_ignores_a_stored_handle() {
        let s = spec_resuming(
            &codex(vec![]),
            SpawnMode::Fresh,
            Some(Uuid::new_v4()),
            "C:/workspace",
        );
        assert_eq!(
            codex_argv(&s),
            codex_argv(&spec(&codex(vec![]), "C:/workspace"))
        );
    }

    /// ★app-server mode does not emit resume as argv★ — its means is the second handshake request (`thread/resume`, [`AgentBackend::open_spawn`]); carried here, the `app-server` subcommand gets an unknown argument and the launch fails.
    #[test]
    fn the_app_server_argv_is_unchanged_by_a_stored_handle() {
        let s = spec_resuming(
            &codex_app_server(vec![]),
            SpawnMode::Resume,
            Some(Uuid::new_v4()),
            "C:/workspace",
        );
        assert_eq!(codex_argv(&s), vec!["app-server", "--stdio"]);
    }

    /// The pass-through is **after** the subcommand · policies on the resume branch too — in front it separates the subcommand and its positional argument (our overrides come even later: `…_follows_the_passthrough`).
    #[test]
    fn resume_argv_still_puts_the_passthrough_after_our_base_args() {
        let s = spec_resuming(
            &codex(vec!["-m", "gpt-5"]),
            SpawnMode::Resume,
            Some(Uuid::new_v4()),
            "C:/workspace",
        );
        let argv = codex_argv(&s);
        assert_eq!(&argv[..1], &["resume".to_string()]);
        let at = argv
            .iter()
            .position(|a| a == "-m")
            .expect("the pass-through disappeared");
        assert_eq!(&argv[at..at + 2], &["-m".to_string(), "gpt-5".to_string()]);
        assert!(
            argv[..at].iter().any(|a| a == APPROVAL_ON_REQUEST),
            "the pass-through went in front of our default arguments: {argv:?}"
        );
    }

    // ── CLI entrance injection (ADR-0086 step 2 · ADR-0133) ────────────────────────────────

    fn endpoint() -> ControlEndpoint {
        ControlEndpoint {
            agent_id: Uuid::nil(),
            epoch: 1,
            url: "http://127.0.0.1:7777/mcp".to_string(),
            token: "deadbeef".to_string(),
            // ★In production this field is `None` — the `Some` is a worst-case value on purpose★: `writes_mcp_config_file()` is false so the daemon sends both fields empty, but the test below measures 「if a path comes,
            //   does it leak into argv」; `None` would make that hollow and a collapsed daemon-side gate would go uncaught.
            config_path: Some(std::path::PathBuf::from("C:/engram/mcp-config.json")),
            send_exe: Some(std::path::PathBuf::from("C:/engram/bin/engram.exe")),
            // ★This field is the concrete form of 「the daemon authorized the MCP send entrance for this spawn」 — do not leave it empty★: an empty list is the operator-**turned-off** state ([`endpoint_without_mcp_send`]), and every
            //   MCP-attachment test would measure the 「not attaching is correct」 side and go hollow.
            //   Pinning tool names here is not retyping — the decision looks **only at the server name** (`ControlEndpoint::grants_mcp_send`); the tool names' source of truth is the daemon side.
            grants: vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: "eg_send".to_string(),
            }],
            // ★The value production carries for this backend is `false`★ — this folder declares it that way (`uses_mail`) and the daemon
            //   derives from that declaration. The sibling test below separately measures that the opposite value is followed too.
            mail_allowed: false,
        }
    }

    /// The endpoint of a spawn whose operator **turned off** MCP mail. The daemon still issues the control channel (control verbs are open to all — ADR-0132 decision 5) and only leaves out the send entrance grant.
    ///
    /// ★One fixture represents both `ENGRAM_DISALLOW_MCP_SEND` and `ENGRAM_FORCE_CLI_ONLY_SEND`★: the former removes the MCP grant in `build_grants`, the latter flips `accepts_mcp_config` so the grant goes to the CLI side —
    /// **both land on 「no MCP send grant for our server」**, the one point the backend looks at.
    fn endpoint_without_mcp_send() -> ControlEndpoint {
        ControlEndpoint {
            grants: vec![],
            ..endpoint()
        }
    }

    fn spec_with_control(command: &AgentCommand, control: Option<ControlEndpoint>) -> CommandSpec {
        CodexBackend.build_spec(
            command,
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("C:/workspace"),
            vec![],
            control,
            &LaunchArtifacts::default(),
        )
    }

    fn env_value<'a>(spec: &'a CommandSpec, key: &str) -> Option<&'a str> {
        spec.env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// ★Terminal mode and app-server mode are measured **each**★ — measuring only one mode, nobody would know if the other mode's spawn
    /// came up without credentials. The children codex launches receive this env only by inheritance.
    #[test]
    fn both_modes_carry_the_control_plane_entrance() {
        for (label, command) in [
            ("terminal", codex(vec![])),
            ("app-server", codex_app_server(vec![])),
        ] {
            let s = spec_with_control(&command, Some(endpoint()));
            assert_eq!(
                env_value(&s, "ENGRAM_TOKEN"),
                Some("deadbeef"),
                "{label}: ENGRAM_TOKEN"
            );
            assert_eq!(
                env_value(&s, "ENGRAM_CONTROL_URL"),
                Some("http://127.0.0.1:7777"),
                "{label}: ENGRAM_CONTROL_URL = the base with /mcp stripped from the MCP url"
            );
            assert_eq!(
                env_value(&s, CLI_EXE_ENV),
                Some("C:/engram/bin/engram.exe"),
                "{label}: CLI absolute path"
            );
        }
    }

    /// ★`--mcp-config` is claude's flag — if it leaks here, codex fails to launch with an unknown argument★.
    #[test]
    fn no_claude_flag_rides_along_with_the_entrance() {
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let s = spec_with_control(&command, Some(endpoint()));
            for forbidden in [
                "--mcp-config",
                "--append-system-prompt-file",
                "--settings",
                "--allowedTools",
                "--disallowedTools",
            ] {
                assert!(
                    !s.args.iter().any(|a| a == forbidden),
                    "`{forbidden}` was carried in codex argv: {:?}",
                    s.args
                );
            }
        }
    }

    /// A spawn without an endpoint (= the control channel could not be received) does **not grow env by a single line**.
    #[test]
    fn without_an_endpoint_nothing_is_injected() {
        let s = spec_with_control(&codex(vec![]), None);
        assert!(
            s.env.is_empty(),
            "env was carried although the endpoint is absent: {:?}",
            s.env
        );
    }

    // ── ADR-0217: the status-line channel was removed · the incarnation marker is planted via env ──────────────

    /// ★argv must have not a single `tui.` setting★ — half of ADR-0217 (the other half: the marker below); a return would again give up one screen line for the whole session in exchange for one recovery.
    /// ★Runs over all of endpoint presence · mode★ — the old value was carried without looking at the endpoint, so measuring only the with-endpoint side would catch half of a revival.
    #[test]
    fn no_spawn_turns_on_a_status_line() {
        for control in [None, Some(endpoint())] {
            for command in [codex(vec![]), codex_app_server(vec![])] {
                let argv = codex_argv(&spec_with_control(&command, control.clone()));
                assert!(
                    !argv.iter().any(|a| a.contains("tui.")),
                    "{command:?}: a TUI config override was carried: {argv:?}"
                );
            }
        }
    }

    /// ★An uncarriable handle emits no subcommand★ — codex reinterprets a value it cannot read as a thread id as a **session name**, quietly creates a new session and exits 0 (measured 2026-09-21): a new conversation the user believes
    /// they resumed, a different fault from ADR-0218's 「not received」. Reason: [`resume_argument`].
    #[test]
    fn a_degenerate_handle_never_reaches_the_resume_subcommand() {
        assert_eq!(resume_argument(Uuid::nil()), None);
        let live = Uuid::new_v4();
        assert_eq!(resume_argument(live), Some(live.hyphenated().to_string()));

        let s = CodexBackend.build_spec(
            &codex(vec![]),
            SpawnMode::Resume,
            None,
            Some(Uuid::nil()),
            PathBuf::from("C:/workspace"),
            vec![],
            None,
            &LaunchArtifacts::default(),
        );
        let argv = codex_argv(&s);
        assert!(
            !argv.iter().any(|a| a == RESUME_SUBCOMMAND),
            "a nil handle went out as the `resume` subcommand: {argv:?}"
        );
        assert!(
            !argv.iter().any(|a| a.contains("00000000-0000")),
            "a nil handle leaked out as a positional argument: {argv:?}"
        );
    }

    /// ★An uncarriable handle is 「a new conversation」, not 「a resume」 — so **recovery must run**★. A real defect (2026-09-21): the argv side refused the value (G7 guard) and came up as a new conversation, but the recovery gate
    /// looked only at 「is there a handle」 and was off — **nobody wrote that incarnation's thread id**. Closed by making both decisions look at [`resume_argument`].
    #[test]
    fn a_handle_we_refuse_to_resume_with_is_still_captured() {
        let s = CodexBackend.build_spec(
            &codex(vec![]),
            SpawnMode::Resume,
            None,
            Some(Uuid::nil()),
            PathBuf::from("C:/workspace"),
            vec![],
            None,
            &LaunchArtifacts::default(),
        );
        let argv = codex_argv(&s);
        assert!(
            !argv.iter().any(|a| a == RESUME_SUBCOMMAND),
            "the premise broke — this handle must not go out as argv: {argv:?}"
        );

        let sink: crate::backend::SessionIdSink = std::sync::Arc::new(|_: &str| {});
        let planned = thread_lock::plan_capture(
            &s.env,
            Some(Uuid::nil()).and_then(resume_argument).is_some(),
            Some(4242),
            Some(1),
            Some(sink),
            Some(PathBuf::from("Z:/locks")),
        );
        assert!(
            planned.is_some(),
            "it came up as a new conversation but recovery does not run — nobody writes this incarnation's thread id"
        );
    }

    /// ★Resume argv and lock recovery: **exactly one** stands★ — both: the already resumed id is found and written again; neither: **nobody writes** that incarnation's id (the real defect above, 2026-09-21).
    ///
    /// So what is measured is **equivalence** — `argv_resumes == !plans_capture`.
    /// ★`Fresh` with a handle carried in is not built here★: only the assembly point can build it and `manager::resume_handle_for` blocks it (`manager::tests::fresh_never_carries_a_resume_handle`); only
    ///   **the six cells that can actually occur** run — an assertion on a nonexistent combination leaves unclear what it guards.
    #[test]
    fn exactly_one_of_the_resume_argv_and_the_capture_plan_fires() {
        let sink: crate::backend::SessionIdSink = std::sync::Arc::new(|_: &str| {});
        let lock_dir = Some(PathBuf::from("Z:/locks"));
        // nil = a value the argv refuses to carry (`resume_argument`) — and yet it may be stored.
        for stored in [None, Some(Uuid::new_v4()), Some(Uuid::nil())] {
            for mode in [SpawnMode::Fresh, SpawnMode::Resume] {
                // Reproducing the assembly-point invariant: Fresh does not carry a handle.
                let resume = match mode {
                    SpawnMode::Resume => stored,
                    SpawnMode::Fresh => None,
                };
                let s = CodexBackend.build_spec(
                    &codex(vec![]),
                    mode,
                    None,
                    resume,
                    PathBuf::from("C:/workspace"),
                    vec![],
                    None,
                    &LaunchArtifacts::default(),
                );
                let argv_resumes = codex_argv(&s)
                    .first()
                    .is_some_and(|a| a == RESUME_SUBCOMMAND);
                // The gate is computed with **the same expression** as the production wiring (that line of `open_spawn`).
                let plans_capture = thread_lock::plan_capture(
                    &s.env,
                    resume.and_then(resume_argument).is_some(),
                    Some(4242),
                    Some(1),
                    Some(sink.clone()),
                    lock_dir.clone(),
                )
                .is_some();
                assert_eq!(
                    argv_resumes, !plans_capture,
                    "{mode:?}/{stored:?}: argv resume ({argv_resumes}) and recovery ({plans_capture}) \
                     both stood or neither stood"
                );
            }
        }
    }

    /// ★The value is pinned byte for byte — measuring that the marker is carried in the spawn env, and its shape★. ★Do not revert to 「the reader compares」★ — no comparing place exists or is planned (ADR-0218 decision 8);
    /// diagnostics lean on the shape.
    #[test]
    fn the_spawn_env_carries_the_incarnation_marker() {
        let id = Uuid::parse_str("2f1c9c0e-1111-4222-8333-444455556666").expect("fixed uuid");
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let ep = ControlEndpoint {
                agent_id: id,
                epoch: 7,
                ..endpoint()
            };
            let s = spec_with_control(&command, Some(ep));
            assert_eq!(
                env_value(&s, "CODEX_INTERNAL_ORIGINATOR_OVERRIDE"),
                Some("engram-2f1c9c0e-1111-4222-8333-444455556666-7"),
                "{command:?}: the marker disagrees"
            );
        }
    }

    /// ★When the incarnation changes, the value changes★ — if it were the same, the record a dead incarnation left would be taken as the live incarnation's.
    #[test]
    fn the_marker_changes_with_the_incarnation() {
        let id = Uuid::new_v4();
        let of = |epoch: u32| {
            let ep = ControlEndpoint {
                agent_id: id,
                epoch,
                ..endpoint()
            };
            env_value(
                &spec_with_control(&codex(vec![]), Some(ep)),
                "CODEX_INTERNAL_ORIGINATOR_OVERRIDE",
            )
            .map(str::to_string)
        };
        assert_ne!(
            of(1),
            of(2),
            "the marker is the same across different incarnations"
        );
    }

    /// ★Anything outside `[a-z0-9-]` is replaced with one `-` per character★ — the value remains outside our process (the vendor record),
    /// so which characters are accepted is upstream's business, and we close that unknown on our side.
    #[test]
    fn the_marker_sanitises_everything_outside_the_allowed_class() {
        assert_eq!(
            sanitise_marker("Engram-AB_c.d 7"),
            "engram-ab-c-d-7",
            "upper case · `_` · `.` · space are not replaced per the rule"
        );
        let live = originator_marker(Uuid::new_v4(), 0);
        assert!(
            live.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "production input, but a character outside the allowed set remained: {live}"
        );
    }

    // ── ADR-0216: the hook registration was removed ───────────────────────────────────────────────
    //
    // ★The order · conflict-warning axis is measured on the living overrides★ — `the_instructions_override_follows_the_passthrough` and `a_conflicting_mcp_passthrough_loses_to_ours_and_is_not_filtered`.
    // ★Scope since ADR-0420★: these three assert a spawn **with no hook output written** carries no hook — Engram's own
    //   session-id hook stays removed (ADR-0216, U3). Hook components and codex settings hooks lines come back as the
    //   user's own hooks, trusted per run; their line carries only the user's command (the test after these three).

    /// ★Terminal spawns with no hook output written carry no hook★ — Engram's own session-id hook stays removed
    /// (ADR-0216, U3); this one line nails down ADR-0216's replacement of the recovery path.
    /// If carried, codex's trust review screen comes back and **the agent does not come up until approval.**
    #[test]
    fn the_terminal_spawn_no_longer_registers_the_session_start_hook() {
        let argv = codex_argv(&spec_with_control(&codex(vec![]), Some(endpoint())));
        assert_eq!(hook_override_value(&argv), None, "{argv:?}");
        assert!(
            !argv.iter().any(|a| a.contains("hook session-start")),
            "a hook command string was carried in another shape: {argv:?}"
        );
    }

    /// The same for app-server mode with no hook output written. ★These two once split — back then the hook was
    /// terminal-only★ (app-server received the id directly in the `thread/start` response). Today neither mode sets it
    /// (ADR-0216, U3).
    #[test]
    fn the_app_server_spawn_registers_no_hook() {
        let s = spec_with_control(&codex_app_server(vec![]), Some(endpoint()));
        let argv = codex_argv(&s);
        assert_eq!(hook_override_value(&argv), None, "{argv:?}");
        assert_eq!(
            &argv[..2],
            &["app-server".to_string(), APP_SERVER_STDIO_FLAG.to_string()],
            "the subcommand and transport choice must still be at the very front: {argv:?}"
        );
    }

    /// The argv of a spawn with no endpoint at all — ★there must be **not a single** `-c`★: the two that carry that flag today
    /// (MCP attachment · the brief) both get their material from the endpoint. It also implies the hook is not carried.
    #[test]
    fn without_an_endpoint_no_config_override_rides() {
        let argv = codex_argv(&spec_with_control(&codex(vec![]), None));
        assert_eq!(hook_override_value(&argv), None, "{argv:?}");
        assert_eq!(
            argv.iter().filter(|a| *a == CONFIG_OVERRIDE_FLAG).count(),
            0,
            "`-c` was carried although the endpoint is absent: {argv:?}"
        );
    }

    /// ★A hook component's `SessionStart` line carries only the user's command★ (ADR-0420 on ADR-0216) — a real terminal
    /// kit with one component: the line is the assembly's (`hooks.SessionStart=[…]`, followed by the state line), its
    /// command is the component's as written (placeholder rewritten), and nothing of Engram's own past hook (the
    /// `hook session-start` subcommand, an Engram executable path) rides it.
    // ADR-0216
    // ADR-0420
    #[test]
    fn a_hook_components_session_start_line_carries_only_the_users_command() {
        use crate::backend::hooks::HookRef;
        use crate::launch::{LaunchKitSource, MemLaunchFs, NoopLaunchKits};
        use crate::profile::AgentProfile;

        let work = std::path::absolute("/fake-root/codex-0216/work").unwrap();
        let folder = std::path::absolute("/fake-root/codex-0216/library/notify").unwrap();
        let profile = AgentProfile::new(
            "adr-0216".to_string(),
            codex(vec![]),
            work.clone(),
            Vec::new(),
            false,
        );
        let plan = NoopLaunchKits.check(&profile).unwrap();
        let mut kit = NoopLaunchKits.prepare(plan, profile.id, 1).unwrap();
        kit.plan.real = true;
        let serde_json::Value::Object(definition) = serde_json::json!({"SessionStart": [{"hooks": [
            {"type": "command", "command": "node \"${CLAUDE_PLUGIN_ROOT}/notify.js\" start"}
        ]}]}) else {
            unreachable!()
        };
        kit.plan.hooks = vec![HookRef {
            path: "team/hooks/notify".to_string(),
            dir: folder.clone(),
            definition,
        }];
        let artifacts =
            crate::backend::materialize(&codex(vec![]), &kit, &work, &MemLaunchFs::default())
                .expect("materialize");
        let argv = codex_argv(&CodexBackend.build_spec(
            &codex(vec![]),
            SpawnMode::Fresh,
            None,
            None,
            work,
            vec![],
            Some(endpoint()),
            &artifacts,
        ));
        let line = hook_override_value(&argv).expect("the component's SessionStart line");
        assert_eq!(Some(&line), artifacts.hook_args.get(1), "{argv:?}");
        let folder = engram_dashboard_platform::fs::forward_slash_text(&folder);
        assert!(
            line.contains(&format!("node \"{folder}/notify.js\" start")),
            "{line}"
        );
        assert!(!line.contains("hook session-start"), "{line}");
        assert!(
            !line.contains("engram"),
            "an Engram path rode the line: {line}"
        );
        assert_eq!(
            argv.iter()
                .filter(|a| a.starts_with("hooks.SessionStart"))
                .count(),
            1,
            "{argv:?}"
        );
    }

    /// Passing another key with `-c` is not a conflict — spammed warnings go unread. ★Measured on the **living** key (the brief)★: a removed key would not tread the axis this helper drives.
    #[test]
    fn an_unrelated_config_passthrough_is_not_a_conflict() {
        assert!(!passthrough_overrides_key(
            &[CONFIG_OVERRIDE_FLAG.to_string(), "model=gpt-5".to_string()],
            DEVELOPER_INSTRUCTIONS_KEY,
        ));
        assert!(!passthrough_overrides_key(
            &[
                "--profile".to_string(),
                format!("{DEVELOPER_INSTRUCTIONS_KEY}=x")
            ],
            DEVELOPER_INSTRUCTIONS_KEY,
        ));
    }

    /// On the resume branch too, the subcommand is at the very front and our overrides are after the pass-through.
    ///
    /// ★It does not pin 「last」★ — that spot goes to the sibling override (the instructions) when real materials carry
    ///   one, and that is not a regression. The real invariant is only **after the pass-through**.
    #[test]
    fn a_resuming_spawn_still_carries_our_override_after_the_passthrough() {
        let s = CodexBackend.build_spec(
            &codex(vec!["-m", "gpt-5"]),
            SpawnMode::Resume,
            None,
            Some(Uuid::new_v4()),
            PathBuf::from("C:/workspace"),
            vec![],
            Some(endpoint()),
            &LaunchArtifacts::default(),
        );
        let argv = codex_argv(&s);
        assert_eq!(&argv[..1], &[RESUME_SUBCOMMAND.to_string()]);
        let passthrough = argv
            .iter()
            .position(|a| a == "gpt-5")
            .expect("the pass-through disappeared");
        let ours = argv
            .iter()
            .position(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX))
            .expect("the MCP attachment disappeared");
        assert!(passthrough < ours, "{argv:?}");
    }

    #[test]
    fn app_server_extra_args_come_last() {
        let s = spec(&codex_app_server(vec!["-c", "model=x"]), "C:/workspace");
        let argv = codex_argv(&s);
        assert_eq!(
            argv,
            vec!["app-server", "--stdio", "-c", "model=x"],
            "the subcommand must be at the very front — pushed back, it is read as an argument"
        );
    }

    /// The two modes report **different** link shapes, and the real thing follows that report.
    #[test]
    fn the_two_modes_declare_different_transport_shapes() {
        assert_eq!(
            CodexBackend.transport_shape(&codex(vec![])),
            TransportShape::Pty
        );
        assert_eq!(
            CodexBackend.transport_shape(&codex_app_server(vec![])),
            TransportShape::StdioBidiJson
        );
    }

    /// ★Had it been `Raw`, the session would emit CR once more after the body as a **separate call**, and a link that builds envelopes itself
    /// would read it as another body and open an empty turn★.
    #[test]
    fn app_server_encoder_passes_bytes_through_and_asks_for_no_submit_byte() {
        let enc = CodexBackend.input_encoder(&codex_app_server(vec![]));
        assert_eq!(enc, InputEncoder::TransportFramed);
        assert_eq!(enc.submit_sequence(), None);
        let body = b"hello codex";
        assert_eq!(enc.encode(body, Uuid::new_v4()), body.to_vec());
    }

    /// ★Do not revive it★ — in terminal mode the PTY echoes input itself (the session's synthetic echo imitates the json mode that lacks it — ADR-0044). The session layer holds an [`InputEncoder`] and goes through the dispatch
    /// table, so looking only at the declaration would let a revived echo pass green.
    ///
    /// app-server's bubble is emitted by the link, not this table (ADR-0231; below-floor links emit none — ADR-0193 「rejected alternative」). ★The bubble does not break **the screen-restore order** thanks to the link's idle
    /// condition (ADR-0203)★: history is received **after** the handshake and raised before the gate (`Link::Ready`), but the synthetic bubble stands only at `Ready`, so text typed in that window goes into the list without a
    /// bubble. Lifting the condition would draw the user's new words above the restored old conversation.
    #[test]
    fn the_terminal_mode_makes_no_synthetic_input_echo() {
        let enc = CodexBackend.input_encoder(&codex(vec![]));
        assert!(
            enc.input_echo_event("hello codex".as_bytes(), Uuid::new_v4())
                .is_none(),
            "{enc:?}: the synthetic echo came back"
        );
    }

    /// Only app-server mode hands the list to the link, and whether receipt is possible is **the very** Arc the link's floor decision fills. The link comes out as `dyn`, so it is measured by owner count — one share for the
    /// assembly point + one for the link's decision field (`the_transport_hands_out_the_very_ack_its_verdict_fills` measures that it is what `delivery_ack()` hands out). A separately made Arc would be 1.
    #[cfg(windows)]
    #[test]
    fn only_the_app_server_spawn_hands_the_list_to_the_transport_with_the_transport_s_own_ack() {
        let probe = CommandSpec {
            program: "cmd.exe".into(),
            args: ["/c", "ping", "-n", "30", "127.0.0.1"]
                .map(String::from)
                .to_vec(),
            env: vec![],
            cwd: PathBuf::from("."),
        };
        let open = |command: &AgentCommand| {
            crate::backend::open_spawn(
                command,
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
            .expect("open_spawn")
        };

        let json = open(&codex_app_server(vec![]));
        assert!(matches!(json.mid_turn, MidTurnPolicy::TransportOwned));
        assert_eq!(
            Arc::strong_count(&json.delivery_ack),
            2,
            "not an Arc shared with the link's decision field — the session and the link see different values"
        );
        json.transport.shutdown();

        let terminal = open(&codex(vec![]));
        assert!(matches!(terminal.mid_turn, MidTurnPolicy::None));
        assert_eq!(Arc::strong_count(&terminal.delivery_ack), 1);
        terminal.transport.shutdown();
    }

    #[test]
    fn the_turn_end_event_is_the_ended_signal() {
        let classify = CodexBackend.turn_classifier();
        assert_eq!(
            classify(&OutputEvent::TurnEnd {
                turn_id: Some("u-1".into()),
                outcome: TurnOutcome::Completed
            }),
            Some(TurnSignal::Ended(TurnEndKind::Clean))
        );
        // Whatever the outcome, the turn has ended — if failure · interruption · unknown split here, the waiting indicator of that outcome remains.
        // Failure is an error end (halt after error — ADR-0231), and interruption · unknown are 「the rest」 (they neither set nor clear the halt).
        assert_eq!(
            classify(&OutputEvent::TurnEnd {
                turn_id: None,
                outcome: TurnOutcome::Failed {
                    detail: Some("boom".into()),
                },
            }),
            Some(TurnSignal::Ended(TurnEndKind::Failed))
        );
        for outcome in [TurnOutcome::Interrupted, TurnOutcome::Unknown] {
            assert_eq!(
                classify(&OutputEvent::TurnEnd {
                    turn_id: None,
                    outcome: outcome.clone()
                }),
                Some(TurnSignal::Ended(TurnEndKind::Other)),
                "{outcome:?}"
            );
        }
        // The echoed user message — the first observation that this turn has started.
        assert_eq!(
            classify(&OutputEvent::Structured {
                kind: "user".into(),
                json: "{}".into()
            }),
            Some(TurnSignal::Progress)
        );
        assert_eq!(
            classify(&OutputEvent::TextDelta {
                text: "hi".into(),
                turn_id: None,
                message_id: None
            }),
            Some(TurnSignal::Progress)
        );
        // The `error` notification is a retryable stream error, not a turn boundary.
        assert_eq!(classify(&OutputEvent::Error("boom".into())), None);
        assert_eq!(
            classify(&OutputEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                turn_id: None
            }),
            None
        );
    }

    /// This backend's `Delivered` comes even after the turn end (late echo · revival) — none of the registry events is a turn signal.
    // ADR-0231
    #[test]
    fn no_queued_input_event_is_a_turn_signal() {
        use crate::types::{DeliveredCopy, DropCause, QueuedInputEvent};
        let classify = CodexBackend.turn_classifier();
        let id = || "c1".to_owned();
        for ev in [
            QueuedInputEvent::Queued {
                id: id(),
                text: "hi".into(),
            },
            QueuedInputEvent::CancelRequested { id: id() },
            QueuedInputEvent::CancelAnswered {
                id: id(),
                removed: false,
            },
            QueuedInputEvent::CancelFailed { id: id() },
            QueuedInputEvent::HandedOver {
                id: id(),
                sent: true,
            },
            QueuedInputEvent::Delivered { id: id() },
            QueuedInputEvent::Dropped {
                id: id(),
                cause: DropCause::Withdrawn,
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

    /// A tool end result is not a turn signal whatever its outcome (reason = [`classify_turn`] doc).
    // ADR-0241
    #[test]
    fn a_tool_result_is_never_a_turn_signal() {
        use crate::types::ToolOutcome;
        let classify = CodexBackend.turn_classifier();
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

    /// ★Measured (codex-cli 0.156.1 · fixture `interrupt_fail_b5`): the `failed` end of an interrupted command comes **after** the turn end★ — it is emitted as `ToolResult{Failed}` after `TurnEnd{Interrupted}`, and fed to the turn
    /// table it neither revives an idle incarnation as 「mid-turn」 nor sets halt-after-error.
    // ADR-0241
    #[test]
    fn a_late_tool_failure_after_an_interrupted_turn_does_not_reopen_the_turn() {
        let classify = CodexBackend.turn_classifier();
        let mut decoder = CodexAppServerDecoder::new();
        let mut events = Vec::new();
        for line in include_str!("fixtures/interrupt_fail_b5.jsonl").lines() {
            // What the link hands the translator is only notifications — response lines (carrying `id`) are filtered out.
            let v: serde_json::Value =
                serde_json::from_str(line).expect("the fixture line is not JSON");
            if v.get("method").is_some() && v.get("id").is_none() {
                events.extend(decoder.decode(format!("{line}\n").as_bytes()));
            }
        }
        let end = events
            .iter()
            .position(|e| {
                matches!(
                    e,
                    OutputEvent::TurnEnd {
                        outcome: TurnOutcome::Interrupted,
                        ..
                    }
                )
            })
            .expect("there is no interrupted turn end");
        let late = events
            .iter()
            .position(|e| matches!(e, OutputEvent::ToolResult { .. }))
            .expect("there is no late tool end result");
        assert!(
            end < late,
            "the tool end is not after the turn end: {events:?}"
        );
        assert!(
            matches!(
                &events[late],
                OutputEvent::ToolResult { id, outcome: crate::types::ToolOutcome::Failed }
                    if id == "exec-ee015669-6082-43c0-ab16-2a89375b485b"
            ),
            "{:?}",
            events[late]
        );
        assert_eq!(classify(&events[late]), None);

        let table = crate::turn::TurnObservations::new();
        let agent = uuid::Uuid::new_v4();
        table.register(agent, 1);
        for (seq, event) in events.iter().enumerate() {
            if let Some(signal) = classify(event) {
                table.observe(agent, 1, seq as u64, signal);
            }
        }
        let seen = table.get(agent, 1).expect("no observation");
        assert!(!seen.in_turn, "a late tool end reopened the turn");
        assert!(
            !seen.last_end_failed,
            "a tool failure set the halt-after-error"
        );
    }

    /// Terminal mode has no decoder, so only `TerminalBytes` flow — so even sharing the same classifier without per-mode branching
    /// yields no signal at all in that mode.
    #[test]
    fn terminal_bytes_carry_no_turn_signal() {
        assert_eq!(
            CodexBackend.turn_classifier()(&OutputEvent::TerminalBytes(b"[0m".to_vec())),
            None
        );
    }

    #[test]
    fn only_app_server_mode_gets_a_decoder() {
        assert!(CodexBackend.output_decoder(&codex(vec![])).is_none());
        assert!(CodexBackend
            .output_decoder(&codex_app_server(vec![]))
            .is_some());
    }

    /// The three declarations are one item because they are **a pair to look at together**: the issuing axis is off in both modes (codex issues the id) and the two resume fields are on in both — only **the means** (argv ↔
    /// `thread/resume`) splits, which these four fields do not ask about.
    ///
    /// ★Writing all four is the point★ — if only one side of the resume axis and the caps report splits, a never-resumed thread is reported as 「resumed」 or the reverse, and measuring one field per mode would hide it.
    /// ★Reverting terminal mode's two fields to `false` requires removing the argv assembly too★ — otherwise the activation entrance launches as Fresh while [`AgentBackend::build_spec`] stays prepared to resume: dead code.
    #[test]
    fn both_modes_declare_resume_and_neither_claims_to_issue_the_id() {
        for c in [codex(vec![]), codex_app_server(vec![])] {
            assert!(
                !CodexBackend.assigns_session_id(&c),
                "{c:?}: the issuer is codex — if we plant our uuid, that value is never used"
            );
            assert!(
                CodexBackend.can_resume_stored_session(&c),
                "{c:?}: turning off the resume axis makes the activation entrance launch as Fresh even with a handle in the registry"
            );
            assert!(
                CodexBackend.capabilities(&c).session.resume,
                "{c:?}: turning on only the axis and off the report makes a spawn that actually resumes be reported as 「new conversation」"
            );
        }
    }

    /// ★The declaration and the real thing must be a pair★ — `declares_link()` decides at the assembly point whether to **lay** the delivery port; true but uncalled: the supervisor waits until the backstop; false but called:
    /// nobody receives it.
    ///
    /// This measures the declaration side; the real side (the app-server link delivers) is measured by `a_rejected_resume_does_not_fall_back_and_ends_the_session`.
    #[test]
    fn only_app_server_declares_a_link() {
        assert!(
            CodexBackend.declares_link(&codex_app_server(vec![])),
            "app-server is usable only after finishing the handshake — declaring there is no axis keeps that outcome out of the decision, and a refusal is again reported as success"
        );
        assert!(
            !CodexBackend.declares_link(&codex(vec![])),
            "terminal mode is a PTY, usable from the moment it comes up — declaring there is an axis makes the supervisor wait until the backstop for a channel nobody delivers"
        );
    }

    /// ★This value is decided by ADR-0116 decision 7, not by whether turn signals exist★ — terminal mode having none is no reason for false (terminal claude already receives in the same place); overturn that decision first.
    // ADR-0116
    #[test]
    fn reads_messages_is_true() {
        assert!(CodexBackend.reads_messages());
    }

    // ── ADR-0185: the received thread id actually reaches the assembly point's record verb ──────────────────

    /// Removes one file of the fake app-server — `true` if removed. ★A try-once-and-give-up shape left scripts in `%TEMP%`★: the child holds the file handle a little longer even after `shutdown()` returns, so the first `remove_file`
    /// fails; the marker file goes through the same door (a third party that briefly opens new files — antivirus · indexer — can make one attempt flaky; never observed).
    /// ★It returns the result because `eprintln!` is swallowed by libtest in **passing** items★ — the failure would leave no trace and leftovers would pile up; the caller raises it as an assertion.
    #[cfg(windows)]
    #[must_use = "dropping the fact that it could not remove quietly piles up leftovers — raise it as an assertion"]
    fn remove_fake_file(path: &std::path::Path) -> bool {
        for _ in 0..40 {
            if std::fs::remove_file(path).is_ok() || !path.exists() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        false
    }

    /// What the fake answers to `thread/resume`.
    #[cfg(windows)]
    pub(super) enum FakeResume {
        /// Returns the received `threadId` with a prefix — the point is that it becomes a value **different** from the one we sent.
        EchoesTheThreadId,
        /// Refuses with `-32600`. ★That code is not exclusive to 「unknown thread」 (ADR-0082's codex-side addendum)★ — an unknown method, a duplicate `initialize` and a config error come with it too (0.154.0; this folder's `protocol` and the link
        /// test bench), so splitting by code and falling back to a new thread would overwrite a still sound handle on an unrelated failure.
        Rejects,
    }

    /// The refusal wording [`FakeResume::Rejects`] carries — items find it in the delivered reason to tell 「the refusal made the round trip」. ★The measured wording★ (`docs/reference/backend-capabilities.md` §1): with invented
    /// wording `resume_failure_kind` would not recognize it and the wiring would pass the bench unclassified.
    #[cfg(windows)]
    const FAKE_RESUME_REJECTION: &str = "no rollout found for thread id";

    /// The name of the environment variable carrying the path of the empty file the fake creates as it enters its read loop.
    #[cfg(windows)]
    const FAKE_READY_ENV: &str = "ENGRAM_FAKE_APP_SERVER_READY";

    /// The environment variable that carries the path of the fake's received-line log ([`FakeAppServer::received`]).
    #[cfg(windows)]
    const FAKE_LOG_ENV: &str = "ENGRAM_FAKE_APP_SERVER_LOG";

    /// One baked fake app-server.
    ///
    /// ★Holding the two paths together is a contract★ — each item must remove them with [`FakeAppServer::remove`] and raise a failure to remove as an assertion.
    #[cfg(windows)]
    pub(super) struct FakeAppServer {
        pub(super) spec: CommandSpec,
        script: std::path::PathBuf,
        /// The empty file the fake creates as it enters its read loop — [`wait_for_the_fake_to_listen`] waits for it.
        ready: std::path::PathBuf,
        /// Every line the fake received, in order, one per line — [`FakeAppServer::received`] reads it.
        log: std::path::PathBuf,
    }

    #[cfg(windows)]
    impl FakeAppServer {
        /// Removes the script, the marker and the method log, and returns the paths it **could not** remove — empty =
        /// all removed.
        #[must_use = "dropping the fact that it could not remove quietly piles up leftovers — raise it as an assertion"]
        pub(super) fn remove(&self) -> Vec<&std::path::Path> {
            [
                self.script.as_path(),
                self.ready.as_path(),
                self.log.as_path(),
            ]
            .into_iter()
            .filter(|path| !remove_fake_file(path))
            .collect()
        }

        /// Every line the fake received so far, parsed, in order (requests and the `initialized` notification).
        pub(super) fn received(&self) -> Vec<serde_json::Value> {
            std::fs::read_to_string(&self.log)
                .unwrap_or_default()
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str(line).expect("the fake logged a non-JSON line"))
                .collect()
        }

        /// The `method` of every received line, in order.
        pub(super) fn methods(&self) -> Vec<String> {
            self.received()
                .iter()
                .map(|v| v["method"].as_str().unwrap_or_default().to_string())
                .collect()
        }

        /// The params of the first received request of `method_name`.
        pub(super) fn params_of(&self, method_name: &str) -> Option<serde_json::Value> {
            self.received()
                .into_iter()
                .find(|v| v["method"] == method_name)
                .map(|v| v["params"].clone())
        }

        /// A **PowerShell** line that runs this fake by hand (it does not run as is in cmd or bash).
        /// ★It sets the marker and log path variables too★ — without them the script dies at the line that writes
        /// the marker or the log (`$ErrorActionPreference = 'Stop'`) and the failure is not reproduced.
        /// Every value is wrapped in single quotes with an inner `'` doubled — so spaces and `$` in a path are not
        /// interpreted.
        fn hand_run(&self) -> String {
            let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
            let args: Vec<String> = self.spec.args.iter().map(|a| quote(a)).collect();
            format!(
                "$env:{FAKE_READY_ENV}={}; $env:{FAKE_LOG_ENV}={}; {} {}",
                quote(&self.ready.to_string_lossy()),
                quote(&self.log.to_string_lossy()),
                self.spec.program,
                args.join(" ")
            )
        }
    }

    /// Bakes a fake app-server into a temp file. `start_thread_id` = the id this fake answers to `thread/start` with.
    #[cfg(windows)]
    fn bake_fake_app_server(start_thread_id: &str, resume: FakeResume) -> FakeAppServer {
        bake_fake_app_server_with(start_thread_id, resume, FakeOptions::default())
    }

    /// What else the fake answers with — [`bake_fake_app_server_with`].
    #[cfg(windows)]
    pub(super) struct FakeOptions {
        /// The `initialize` `userAgent`. The default `engram-fake/0` carries no version the transport can read.
        pub(super) user_agent: &'static str,
        pub(super) extra_roots: FakeExtraRoots,
        pub(super) hooks_list: FakeHooksList,
    }

    #[cfg(windows)]
    impl Default for FakeOptions {
        fn default() -> Self {
            FakeOptions {
                user_agent: "engram-fake/0",
                extra_roots: FakeExtraRoots::Answers,
                hooks_list: FakeHooksList::Rejects,
            }
        }
    }

    /// How the fake answers `hooks/list`.
    // ADR-0420
    #[cfg(windows)]
    #[derive(Clone, Copy)]
    pub(super) enum FakeHooksList {
        /// `-32601` — what a codex without the method would answer.
        Rejects,
        /// `{"result": <this JSON>}` — no `'` inside (it goes into a powershell single-quoted string).
        Answers(&'static str),
        /// No answer at all — a codex that accepts the request and never replies (the cap
        /// `transport::HOOKS_LIST_CAP` ends the wait).
        Silent,
    }

    /// [`FakeHooksList::Rejects`]'s message.
    #[cfg(windows)]
    const FAKE_HOOKS_LIST_REJECTION: &str = "method not found: hooks/list";

    /// How the fake answers `skills/extraRoots/set`.
    #[cfg(windows)]
    #[derive(Clone, Copy)]
    pub(super) enum FakeExtraRoots {
        /// `{}` — what codex 0.163.0 answers.
        Answers,
        /// `-32601` — what a codex without the method would answer (R16).
        Rejects,
    }

    /// [`FakeExtraRoots::Rejects`]'s message.
    #[cfg(windows)]
    const FAKE_EXTRA_ROOTS_REJECTION: &str = "method not found: skills/extraRoots/set";

    /// [`bake_fake_app_server`] with a chosen `userAgent` and `skills/extraRoots/set` answer.
    #[cfg(windows)]
    pub(super) fn bake_fake_app_server_with(
        start_thread_id: &str,
        resume: FakeResume,
        options: FakeOptions,
    ) -> FakeAppServer {
        let extra_roots_reply = match options.extra_roots {
            FakeExtraRoots::Answers => r#"'"result":{}'"#.to_string(),
            FakeExtraRoots::Rejects => {
                format!(r#"'"error":{{"code":-32601,"message":"{FAKE_EXTRA_ROOTS_REJECTION}"}}'"#)
            }
        };
        let send = |reply: String| format!(r#"Send ('{{"id":' + $rid + ',' + {reply} + '}}')"#);
        let hooks_list_answer = match options.hooks_list {
            FakeHooksList::Rejects => send(format!(
                r#"'"error":{{"code":-32601,"message":"{FAKE_HOOKS_LIST_REJECTION}"}}'"#
            )),
            FakeHooksList::Answers(result) => send(format!(r#"'"result":{result}'"#)),
            FakeHooksList::Silent => "# no answer (FakeHooksList::Silent)".to_string(),
        };
        let resume_reply = match resume {
            FakeResume::EchoesTheThreadId => {
                r#"'"result":{"thread":{"id":"resumed-' + $tid + '","cliVersion":"0.0.0-fake"}}'"#
                    .to_string()
            }
            FakeResume::Rejects => {
                format!(r#"'"error":{{"code":-32600,"message":"{FAKE_RESUME_REJECTION}"}}'"#)
            }
        };
        let script = FAKE_APP_SERVER_PS1
            .replace("THREAD_ID_PLACEHOLDER", start_thread_id)
            .replace("RESUME_REPLY_PLACEHOLDER", &resume_reply)
            .replace("EXTRA_ROOTS_REPLY_PLACEHOLDER", &extra_roots_reply)
            .replace("HOOKS_LIST_ANSWER_PLACEHOLDER", &hooks_list_answer)
            .replace("USER_AGENT_PLACEHOLDER", options.user_agent)
            .replace("READY_ENV_PLACEHOLDER", FAKE_READY_ENV)
            .replace("LOG_ENV_PLACEHOLDER", FAKE_LOG_ENV);
        let stem = format!("engram-fake-app-server-{}", Uuid::new_v4());
        let script_path = std::env::temp_dir().join(format!("{stem}.ps1"));
        let ready = std::env::temp_dir().join(format!("{stem}.ready"));
        let log = std::env::temp_dir().join(format!("{stem}.log"));
        std::fs::write(&script_path, script).expect("write the fake app-server");

        let spec = CommandSpec {
            program: "powershell.exe".into(),
            args: vec![
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                script_path.to_string_lossy().into_owned(),
            ],
            env: vec![
                (FAKE_READY_ENV.into(), ready.to_string_lossy().into_owned()),
                (FAKE_LOG_ENV.into(), log.to_string_lossy().into_owned()),
            ],
            cwd: PathBuf::from("."),
        };
        FakeAppServer {
            spec,
            script: script_path,
            ready,
            log,
        }
    }

    /// Waits until the fake enters its read loop — ★call it right before `start()`★.
    ///
    /// The handshake budget's clock (`transport::HANDSHAKE_BUDGET`) starts in the writer `start()` launches, but the fake is already up before that, in `open_spawn`; the powershell startup in between is not what these items measure.
    /// CI run 35447034272 attempt 1 failed four items together with an `initialize` deadline expiry (attempt 2 green). ★Powershell startup is the most plausible cause, not a certainty★ [unverified]: the log has one deadline line,
    /// and a 12-second delay injected before startup reproduces it. Startup is moved out of the budget and only the round trip is measured **with the production budget**.
    /// ★The budget is not raised to fit★ — that value is production behaviour; `start()` has no handle to inject a bench budget and adding one needs a new production seam (ADR-0012).
    /// ★The marker guarantees only 「the script started running」★ — the first-run cost of the per-request response path (regex · write) is still inside the budget and unmeasured on a slow runner; if large, the symptom is the same deadline expiry.
    #[cfg(windows)]
    pub(super) fn wait_for_the_fake_to_listen(fake: &FakeAppServer) {
        wait_until(
            || fake.ready.exists(),
            &format!(
                "the fake app-server entering its read loop — either powershell could not come up (execution policy) or the script died \
                 before the loop. To tell them apart, run it by hand: {}",
                fake.hand_run()
            ),
        );
    }

    /// The port that records the connection outcomes the link delivers, and that record.
    #[cfg(windows)]
    fn link_recorder() -> (
        crate::transport::LinkSink,
        std::sync::Arc<std::sync::Mutex<Vec<crate::transport::LinkResolution>>>,
    ) {
        let delivered = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink: crate::transport::LinkSink = {
            let delivered = delivered.clone();
            std::sync::Arc::new(move |r: crate::transport::LinkResolution| {
                delivered.lock().unwrap().push(r)
            })
        };
        (sink, delivered)
    }

    /// Waits until the record verb is called — ★if a connection outcome is delivered first, it stops and dies saying, with that outcome, what went wrong★.
    ///
    /// The record verb is called **only after** the handshake round trip stands, and the outcome is delivered **only after** that point, so an outcome without a record is either `Failed` (the round trip fell over) or `Ready` (**the record
    /// port did not reach the link** — without a port the link skips the record and sets up the connection: `record_session_id`), the wiring regression [`tests::an_app_server_spawn_hands_the_thread_id_to_the_sink`] catches.
    /// Waiting only for the record would spin to the deadline and leave 「it did not happen」; the reason is only in the outcome.
    #[cfg(windows)]
    fn wait_for_recording(
        recorded: impl Fn() -> bool,
        delivered: &std::sync::Mutex<Vec<crate::transport::LinkResolution>>,
        fake: &FakeAppServer,
    ) {
        wait_until(
            || recorded() || !delivered.lock().unwrap().is_empty(),
            "the record verb being called or a connection outcome being delivered",
        );
        if recorded() {
            return;
        }
        let resolution = delivered.lock().unwrap().first().cloned();
        match resolution {
            Some(crate::transport::LinkResolution::Ready { .. }) => panic!(
                "the connection stood without the record verb ever being called — the wiring is broken: the record port did not reach the link"
            ),
            Some(crate::transport::LinkResolution::Failed { reason }) => panic!(
                "the handshake fell over before the record — this item did not reach the point it meant to measure. Delivered reason: {reason}. \
                 If the reason is a deadline expiry (「no answer」), the fake could not answer within the handshake budget. The script is \
                 left in place — run it by hand: {}",
                fake.hand_run()
            ),
            None => unreachable!("the wait above returns only when either the record or the outcome stands"),
        }
    }

    /// A minimal app-server: it answers the handshake requests (`initialize` with [`FakeOptions::user_agent`],
    /// `skills/extraRoots/set` per [`FakeExtraRoots`], `thread/start`, `thread/resume`, `hooks/list` per
    /// [`FakeHooksList`]) and `turn/start` (then echoes a
    /// user message on the thread it opened), and appends every line it receives to its log, in order
    /// ([`FakeAppServer::received`]). ★Not the real codex★ — what the tests measure is our side of the wire (the thread
    /// id reaching the verb handed to [`AgentBackend::open_spawn`], what the handshake sends and in which order); the real
    /// CLI does not matter for that (ADR-0012 isolation).
    /// ★Baked into a file, not passed as an argument★ — the script contains JSON `"`, which is chewed twice by Rust `Command` escaping and powershell command-line parsing.
    /// ★stdout is written as raw bytes★ — `Write-Output` drags encoding · BOM along with host settings, and one BOM byte makes the first line not JSON.
    ///
    /// ★The `thread/resume` answer is filled in by [`FakeResume`]★ — a **powershell expression**, so an answer that appends the received `threadId` can be written.
    ///
    /// ★**This fake keeps reading stdin after answering — that is the premise of one item**★: [`tests::a_recording_failure_ends_the_session`] measures 「if the record fails we close stdin and the session ends」; a fake that ended on its own would
    /// give EOF **regardless of the close**, measure none of the wiring and go green (confirmed by mutation: removing the close made **no** state transition observed in that item). ★Turning the while loop into 「exit after
    /// responding」 quietly disarms that item★.
    ///
    /// ★The marker file is created **right before** the read loop★ — [`wait_for_the_fake_to_listen`] reads it as 「ready to answer」; moving it before the definitions used for answering (`$so` · `Send`) would falsify that.
    #[cfg(windows)]
    const FAKE_APP_SERVER_PS1: &str = r#"
$ErrorActionPreference = 'Stop'
$so = [Console]::OpenStandardOutput()
function Send([string]$s) {
  $b = [Text.Encoding]::ASCII.GetBytes($s + "`n")
  $so.Write($b, 0, $b.Length)
  $so.Flush()
}
$current = 'THREAD_ID_PLACEHOLDER'
[IO.File]::WriteAllText($env:READY_ENV_PLACEHOLDER, '')
while ($null -ne ($line = [Console]::In.ReadLine())) {
  [IO.File]::AppendAllText($env:LOG_ENV_PLACEHOLDER, $line + "`n")
  if ($line -match '"id":(-?\d+)') {
    $rid = $Matches[1]
    if ($line -match '"method":"initialize"') {
      Send ('{"id":' + $rid + ',"result":{"codexHome":"h","platformFamily":"windows","platformOs":"windows","userAgent":"USER_AGENT_PLACEHOLDER"}}')
    } elseif ($line -match '"method":"skills/extraRoots/set"') {
      Send ('{"id":' + $rid + ',' + EXTRA_ROOTS_REPLY_PLACEHOLDER + '}')
    } elseif ($line -match '"method":"thread/start"') {
      Send ('{"id":' + $rid + ',"result":{"thread":{"id":"THREAD_ID_PLACEHOLDER","cliVersion":"0.0.0-fake"}}}')
    } elseif ($line -match '"method":"thread/resume"') {
      $tid = 'MISSING'
      if ($line -match '"threadId":"([^"]+)"') { $tid = $Matches[1] }
      $current = 'resumed-' + $tid
      Send ('{"id":' + $rid + ',' + RESUME_REPLY_PLACEHOLDER + '}')
    } elseif ($line -match '"method":"hooks/list"') {
      HOOKS_LIST_ANSWER_PLACEHOLDER
    } elseif ($line -match '"method":"turn/start"') {
      Send ('{"id":' + $rid + ',"result":{"turn":{"id":"fake-turn"}}}')
      Send ('{"method":"item/started","params":{"threadId":"' + $current + '","turnId":"fake-turn","item":{"type":"userMessage","id":"fake-user-item","content":[{"type":"text","text":"echo"}]}}}')
    }
  }
}
"#;

    /// ★Back when the assembly point passed `None`, this item did not hold★ — if the wiring breaks again, it is caught here.
    /// What is measured is **whether the record verb was called**, and how that value sits in the profile is measured by the `manager.rs` side item.
    #[cfg(windows)]
    #[test]
    fn an_app_server_spawn_hands_the_thread_id_to_the_sink() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, StatusSink};
        use std::sync::{Arc, Mutex};

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: Uuid, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let thread_id = Uuid::new_v4().to_string();
        let fake = bake_fake_app_server(&thread_id, FakeResume::EchoesTheThreadId);

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: SessionIdSink = {
            let seen = seen.clone();
            Arc::new(move |id: &str| seen.lock().unwrap().push(id.to_string()))
        };
        let (link_sink, delivered) = link_recorder();

        let parts = crate::backend::open_spawn(
            &codex_app_server(vec![]),
            &fake.spec,
            80,
            24,
            Some(sink),
            None,
            None,
            Some(link_sink),
            None,
            &LaunchArtifacts::default(),
        )
        .expect("open_spawn");

        wait_for_the_fake_to_listen(&fake);
        parts.transport.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )));

        // ★Failures are split four ways★: the fake did not come up (the wait above) · the round trip fell over (the failure outcome's reason) · the connection stood without a record (wiring regression) · the fake came up but there is neither a record nor an
        //   outcome (deadline). Each dies before the removing line, so the script remains.
        wait_for_recording(|| !seen.lock().unwrap().is_empty(), &delivered, &fake);
        let got = seen.lock().unwrap().clone();

        parts.transport.shutdown();

        let leftover = fake.remove();
        assert_eq!(
            got,
            vec![thread_id],
            "the record verb received a value different from the thread id the app-server gave"
        );
        assert!(
            leftover.is_empty(),
            "could not remove the fake app-server files: {leftover:?}"
        );
    }

    /// ★If the assembly point gives a value to resume, the second request splits into `thread/resume` — and what is recorded is **the id the peer answered**★.
    ///
    /// One assertion tells three things apart (the fake answers `resumed-<the threadId we sent>`):
    ///   1. What went out is not `thread/start` (the fake would return `start_only`).
    ///   2. The `threadId` field carried our value as is (including the camelCase spelling).
    ///   3. The record verb receives **the value the peer gave** — had the implementation rewritten our value, the un-prefixed original would show; it matters because the day the peer gives a different id, that is the thread this
    ///      incarnation actually talks in.
    ///
    /// ★`send_input` is never called★ — only the handshake section is measured, and sending would mix in the branch where the queue fills to the cap.
    #[cfg(windows)]
    #[test]
    fn a_resume_target_makes_the_handshake_issue_thread_resume() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, StatusSink};
        use std::sync::{Arc, Mutex};

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: Uuid, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let resume_target = Uuid::new_v4();
        let fake = bake_fake_app_server("start_only", FakeResume::EchoesTheThreadId);

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: SessionIdSink = {
            let seen = seen.clone();
            Arc::new(move |id: &str| seen.lock().unwrap().push(id.to_string()))
        };
        let (link_sink, delivered) = link_recorder();

        let parts = crate::backend::open_spawn(
            &codex_app_server(vec![]),
            &fake.spec,
            80,
            24,
            Some(sink),
            None,
            Some(resume_target),
            Some(link_sink),
            None,
            &LaunchArtifacts::default(),
        )
        .expect("open_spawn");

        wait_for_the_fake_to_listen(&fake);
        parts.transport.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )));

        // The failure branches are the same as the item above. That the resume request did not go out is not caught here — the fake
        //   answers `thread/start` too, so the record verb is called then as well, and the value assertion below makes that split.
        wait_for_recording(|| !seen.lock().unwrap().is_empty(), &delivered, &fake);
        let got = seen.lock().unwrap().clone();

        parts.transport.shutdown();

        let leftover = fake.remove();
        assert_eq!(
            got,
            vec![format!("resumed-{resume_target}")],
            "the id of the resume response was not recorded — `start_only` means `thread/start` went out, and an original without the prefix means the value we sent was rewritten instead of the peer's answer"
        );
        // ★The value this fake gives is **not a uuid** — pinned here★. The record verb (`manager::session_id_sink`) **quietly drops** a value failing `Uuid::parse_str`, so this branch is not 「recorded」 but **a split**: the link keeps this
        //   string as `thread_id` and sends every later turn with it while the on-disk `backend_session_id` stays **old**, and the next resume opens a thread this incarnation never talked in.
        //   ★The partner measuring the split is `a_non_uuid_thread_id_leaves_the_stored_one_diverged` in `manager.rs`; this only measures that the bench creates that condition★ — otherwise the `assert_eq!` above would read as 「it got recorded」.
        assert!(
            Uuid::parse_str(&got[0]).is_err(),
            "the fake's answer reads as a uuid — the premise of this item (the assembly point drops this value) is stale: {got:?}"
        );
        assert!(
            leftover.is_empty(),
            "could not remove the fake app-server files: {leftover:?}"
        );
    }

    /// ★A refused resume **does not fall back to a new thread** (ADR-0082)★ and does not touch the handle stored in the profile.
    ///
    /// Decisive evidence: **the record verb is never called** — this fake also answers `thread/start` (`start_only`), so had the link opened a new thread that id would reach the record verb; no record means no second spawn and the stored sid
    /// stays. ★Absence is not measured by deadline alone★: it is asserted **after waiting** for the failure boundary that shows on screen, telling 「not ended yet」 from 「no fallback」.
    ///
    /// ★And **it is measured until the session reaches its end state**★ — the item's second axis; without it the item is **green on a broken implementation**. The fake keeps reading stdin after the refusal (that constant's doc), so if the link
    /// does not let go of our stdin the child · reader · writer stay alive, the reader never sees EOF, [`crate::output_core::OutputCore::finish`] never runs, no message reaches the reaper (its sole consumer — ADR-0019), and the manager's
    /// activation decision sees only `Running` for 3 seconds, **stamps 「resume succeeded」 and erases even the last failure record**. ★No other net exists★ — `tests/activation.rs` has one codex item (profile wiring) and does not measure the refusal
    /// branch, so this is the only place measuring the link letting go of stdin.
    /// ★Waiting before `shutdown()` is the point★ — killing first would drive the causality this item measures (precedent: [`tests::a_recording_failure_ends_the_session`]).
    #[cfg(windows)]
    #[test]
    fn a_rejected_resume_does_not_fall_back_and_ends_the_session() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{
            AgentInfo, AgentStatus, OutputFrame, OutputPayload, OutputSink, SinkError, SinkId,
            StatusSink, TurnOutcome,
        };
        use std::sync::{Arc, Mutex};

        /// The eye that watches the end transition — precedent · reason are the same as [`tests::a_recording_failure_ends_the_session`].
        struct RecordingStatus(Arc<Mutex<Vec<AgentStatus>>>);
        impl StatusSink for RecordingStatus {
            fn status_changed(&self, _id: Uuid, s: AgentStatus, _e: u32) {
                self.0.lock().expect("status poisoned").push(s);
            }
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        struct EventSink {
            id: SinkId,
            seen: Arc<Mutex<Vec<OutputEvent>>>,
        }
        impl OutputSink for EventSink {
            fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
                if let OutputPayload::Event(e) = frame.payload {
                    self.seen.lock().unwrap().push(e.clone());
                }
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }

        let resume_target = Uuid::new_v4();
        let fake = bake_fake_app_server("start_only", FakeResume::Rejects);

        let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: SessionIdSink = {
            let recorded = recorded.clone();
            Arc::new(move |id: &str| recorded.lock().unwrap().push(id.to_string()))
        };
        // ★Deliveries are recorded as is★ — what this item measures is 「does the link **send out** the outcome」, and this is
        //   the only place it is confirmed on a real link (the manager-side items measure only the decision logic with stand-ins).
        let (link_sink, delivered) = link_recorder();

        let parts = crate::backend::open_spawn(
            &codex_app_server(vec![]),
            &fake.spec,
            80,
            24,
            Some(sink),
            None,
            Some(resume_target),
            Some(link_sink),
            None,
            &LaunchArtifacts::default(),
        )
        .expect("open_spawn");

        let events: Arc<Mutex<Vec<OutputEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let statuses: Arc<Mutex<Vec<AgentStatus>>> = Arc::new(Mutex::new(Vec::new()));
        let core = Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(RecordingStatus(statuses.clone())),
            TurnWiring::detached(),
        ));
        core.subscribe(Arc::new(EventSink {
            id: SinkId::new_v4(),
            seen: events.clone(),
        }));
        wait_for_the_fake_to_listen(&fake);
        parts.transport.start(core);

        let failed = |es: &[OutputEvent]| {
            es.iter().any(|e| {
                matches!(
                    e,
                    OutputEvent::TurnEnd {
                        outcome: TurnOutcome::Failed { .. },
                        ..
                    }
                )
            })
        };
        wait_until(
            || failed(&events.lock().unwrap()),
            "the failure boundary (TurnEnd Failed) showing up",
        );
        let got = recorded.lock().unwrap().clone();
        // ★Does the link report 「connection could not stand + reason」 — the only real-thing measurement is here★: the manager-side items use a stand-in link, so if the real link does not fill that axis the wiring breaks with both sides green.
        //   ★Caught **before** the end state★ — afterwards the value has no reason to remain.
        let link_when_failed = match delivered.lock().unwrap().first() {
            Some(crate::transport::LinkResolution::Failed { reason }) => reason.clone(),
            other => panic!(
                "the failure boundary showed up but the link **did not deliver** the failure outcome — the activation decision has no signal \
                 to receive (delivered: {other:?})"
            ),
        };
        // ★First tell whether that reason is **the fake's refusal**★ — otherwise the whole item (no fallback · end state · classification) ran without a refusal and the classification assertion would blame an unrelated reason. It is before the
        //   end-state wait so the reason is quoted even when that wait does not finish.
        assert!(
            link_when_failed.contains(FAKE_RESUME_REJECTION),
            "the delivered reason is not the fake's refusal — this item could not measure the refusal branch. If the reason is a deadline \
             expiry (「no answer」), the fake could not answer within the handshake budget (the request did not go out or the fake \
             was late). The script is left in place — run it by hand: {}. Reason: {link_when_failed}",
            fake.hand_run()
        );

        // After the failure boundary shows up, wait for the peer to see EOF and end as a result of the link letting go of our stdin.
        //   ★Before `shutdown()`★ — the reason in the doc above.
        let terminal = || {
            statuses
                .lock()
                .expect("status poisoned")
                .iter()
                .any(|s| !s.is_live())
        };
        wait_until(|| terminal(), "the session going to its end state");
        assert_eq!(
            delivered.lock().unwrap().len(),
            1,
            "the connection outcome is not once — this port's contract is **exactly once**"
        );
        // And only if that reason goes as far as classification does the real cause, not the context default (early exit), remain in 「last failure」.
        assert_eq!(
            CodexBackend.resume_failure_kind(&link_when_failed),
            Some(crate::failure::AgentFailureKind::NoConversationToResume),
            "the connection reason was not classified — this branch has no early exit, so if the context default is stamped it is false: {link_when_failed}"
        );

        let reached_terminal = terminal();
        let seen_statuses = statuses.lock().expect("status poisoned").clone();
        // ★Neither `!is_live()` nor `Exited { .. }` is **enough**★ (the old assertion was hollow): when the reader sees EOF, `Exited { code: None }` stands regardless of child liveness, so a kind-only assertion **could not be false** once
        //   `reached_terminal` was true.
        // ★A carried code is the only evidence the child was reaped★ — `Some(_)` means `try_wait` succeeded, which only happens when the process has ended (`reader_loop` looks again, bounded — the comment there is the source of truth).
        // ★Which path it ended by is not told apart★ — the peer seeing EOF or the link ending it past the grace period are both legitimate, and telling them apart needs timing, which flakes. What this prevents is **no outcome at all**.
        let ended_with_a_reaped_child = seen_statuses
            .iter()
            .any(|s| matches!(s, AgentStatus::Exited { code: Some(_) }));

        parts.transport.shutdown();

        let leftover = fake.remove();
        assert!(
            got.is_empty(),
            "the record verb was called after a refused resume — this means it fell back to a new thread (certain if the value is `start_only`), and that value overwrites the still sound handle in the profile: {got:?}"
        );
        assert!(
            reached_terminal,
            "the resume was refused but the session did not reach an end state — the link did not let go of our stdin, so \
             the child · reader · writer are held as is, the pump cannot finish, and the message to the reaper is never \
             created. The manager reads that state as 「resume succeeded」 and erases even the last failure \
             record (the outcome ADR-0082 meant to prevent). Observed states: {seen_statuses:?}"
        );
        assert!(
            ended_with_a_reaped_child,
            "reached the end state but there is no trace of the child being reaped — `Failed` and `Exited{{code:None}}` stand even while the child is \
             alive, so by themselves they cannot be told apart from 「only the session ended and the process remained」 (observed: {seen_statuses:?})"
        );
        assert!(
            leftover.is_empty(),
            "could not remove the fake app-server files: {leftover:?}"
        );
    }

    /// ★If the record panics, the session **ends** — it neither stops silently nor stays up forever★.
    ///
    /// Three measurements, ★the third is the core★:
    ///   1. The record verb was called (= the handshake stood; otherwise the two below spin idle).
    ///   2. Input is refused — not the shape where the link stopped at `Connecting` and only the queue filled (ADR-0190).
    ///   3. ★The session **reaches an end state**★. Measuring only 2 **could not be told apart from a stuck session** (refusal works but child · reader · writer remain, the pump never ends, nothing is reaped); only the end transition splits that.
    /// ★「Record」 goes only as far as **handing it** to the port (ADR-0226)★ — behind the production port is the first-submission latch, tying persistence to submission; 「persist before the first turn」 is measured separately
    /// (`transport`'s `the_session_id_is_recorded_before_the_gate_opens` doc).
    #[cfg(windows)]
    #[test]
    fn a_recording_failure_ends_the_session() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, InputEvent, StatusSink};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};

        /// The eye that watches the end transition — a state where `is_live()` is false is the end (that predicate is the source of truth).
        struct RecordingStatus(Arc<Mutex<Vec<AgentStatus>>>);
        impl StatusSink for RecordingStatus {
            fn status_changed(&self, _id: Uuid, s: AgentStatus, _e: u32) {
                self.0.lock().expect("status poisoned").push(s);
            }
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let fake = bake_fake_app_server(&Uuid::new_v4().to_string(), FakeResume::EchoesTheThreadId);

        // It blows up leaving only the fact that it was called — 「a panic happened」 and 「it was never called」 must be told apart.
        // ★It stands until released before blowing up★ — so the quiet-hook span covers only the moment of the panic (the span comment below). If the sender disappears (the test dies first) the wait is released and it blows up, never standing forever.
        let reached = Arc::new(AtomicBool::new(false));
        let (release, released) = std::sync::mpsc::channel::<()>();
        let sink: SessionIdSink = {
            let reached = reached.clone();
            let released = Mutex::new(released);
            Arc::new(move |_: &str| {
                reached.store(true, Ordering::SeqCst);
                let _ = released.lock().expect("gate poisoned").recv();
                panic!("the record port blew up");
            })
        };
        let (link_sink, delivered) = link_recorder();

        let parts = crate::backend::open_spawn(
            &codex_app_server(vec![]),
            &fake.spec,
            80,
            24,
            Some(sink),
            None,
            None,
            Some(link_sink),
            None,
            &LaunchArtifacts::default(),
        )
        .expect("open_spawn");

        let statuses: Arc<Mutex<Vec<AgentStatus>>> = Arc::new(Mutex::new(Vec::new()));
        let terminal = {
            let statuses = statuses.clone();
            move || {
                statuses
                    .lock()
                    .expect("status poisoned")
                    .iter()
                    .any(|s| !s.is_live())
            }
        };

        wait_for_the_fake_to_listen(&fake);
        parts.transport.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(RecordingStatus(statuses.clone())),
            TurnWiring::detached(),
        )));

        // ★First wait until the record verb is called **without sending anything**★ — `send_input` before the handshake queues (ADR-0190); sending here would fill the cap (32), raise **a queue-full error** and make this item green for the wrong reason.
        wait_for_recording(|| reached.load(Ordering::SeqCst), &delivered, &fake);

        // ★The hook is not swapped bare-handed★ — it is global: other items in the binary would lose their panic output, and nesting would leave the quiet hook forever. A helper prevents both.
        // ★The quiet-hook span covers **only the moment of the panic**★ — the hook is process-global, so a longer span makes an **unrelated** item that blew up in the window lose its panic message and location, and other panicking items queue behind
        //   the helper's static mutex. The handshake · end wait stay outside; only releasing the writer to blow up and watching its result (connection outcome · input refusal) are inside — the outcome is delivered **after** the writer catches the
        //   panic, so waiting for it puts the panic inside the span.
        let refusal = engram_dashboard_command::testing::with_quiet_panic_hook(|| {
            release
                .send(())
                .expect("the record port must still be alive");

            // ★Send **only once** after the outcome is delivered★ — the link delivers the failure outcome **after** setting `Link::Down` (the failure branch of `writer_loop`), so input afterwards is certainly refused. A fixed count · interval would need the
            //   writer to wake, catch the panic and take the link down inside that window, falsely failing with 「input keeps being accepted」 on a slow runner.
            wait_until(
                || !delivered.lock().unwrap().is_empty(),
                "the connection outcome being delivered after the record failure — if it does not come, the link did not emit the record failure as an outcome \
                 (it may be the shape where the link stopped at `Connecting`)",
            );
            parts
                .transport
                .send_input(InputEvent::Raw(b"hi".to_vec()))
                .err()
        });

        // Wait for the peer to end on its own after we let go of stdin (measured 41–51ms) and for that EOF to end the pump. ★Wait without calling `shutdown()`★ — killing first would drive the causality this item measures.
        wait_until(|| terminal(), "the session going to its end state");

        let reached_terminal = terminal();
        let seen = statuses.lock().expect("status poisoned").clone();
        // ★Neither `!is_live()` nor `Exited { .. }` is **enough**★ — `Exited { code: None }` stands on reader EOF regardless of child liveness (the neighbouring item had a copy of the hollow shape, fixed together).
        // ★A carried code is the only evidence the child was reaped★ — reason: the same spot in `a_rejected_resume_does_not_fall_back_and_ends_the_session`.
        let ended_with_a_reaped_child = seen
            .iter()
            .any(|s| matches!(s, AgentStatus::Exited { code: Some(_) }));
        parts.transport.shutdown();

        let leftover = fake.remove();

        let refusal = refusal.expect(
            "even the outcome of the record failure was delivered but input is accepted — the link did not go down and the session quietly went mute",
        );
        let PtyError::WriteFailed(reason) = &refusal else {
            panic!("unexpected error kind: {refusal:?}");
        };
        // ★Only queue-full is excluded★ — only it means 「refused although the failure path did not run」. The other two reasons (link taken down by the record failure · link ended after closing stdin) are **both evidence this path ran**, and which one
        //   is caught after the outcome is a race with the reader's EOF, so pinning one would itself flake.
        assert!(
            !reason.contains("the cap"),
            "the refusal reason is queue-full — it did not measure that the failure path ran: {reason}"
        );

        assert!(
            reached_terminal,
            "the record failed but the session did not reach an end state — the child · reader · writer are held as is, and neither the pump \
             nor the reaper moves (observed states: {seen:?})"
        );
        assert!(
            ended_with_a_reaped_child,
            "reached the end state but there is no trace of the child being reaped — `Failed` and `Exited{{code:None}}` stand even while the child is \
             alive, so by themselves they cannot be told apart from 「only the session ended and the process remained」 (observed: {seen:?})"
        );
        assert!(
            leftover.is_empty(),
            "could not remove the fake app-server files: {leftover:?}"
        );
    }

    /// ★The pair for the mode where the link carries turns — the first-turn port is plugged into the real link and called not at the handshake but at the echo of the peer's user message (ADR-0226 amendment)★.
    ///
    /// The record port was called at the handshake, the first-turn port not yet → one turn makes the fake answer `turn/start` and echo the user message → the first-turn port is called once. The fake's edition is below the floor, so the echo has
    /// no `clientId` — it still counts. That the session does not count in that mode is measured by the `session.rs` test.
    #[cfg(windows)]
    #[test]
    fn an_app_server_spawn_counts_the_first_turn_at_the_echo_not_at_the_handshake() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, InputOrigin, StatusSink, TurnInput};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: Uuid, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let thread_id = Uuid::new_v4().to_string();
        let fake = bake_fake_app_server(&thread_id, FakeResume::EchoesTheThreadId);

        let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: SessionIdSink = {
            let recorded = recorded.clone();
            Arc::new(move |id: &str| recorded.lock().unwrap().push(id.to_string()))
        };
        let first_turns = Arc::new(AtomicUsize::new(0));
        let first_turn: FirstTurnSink = {
            let first_turns = first_turns.clone();
            Arc::new(move |_delivered: bool| {
                first_turns.fetch_add(1, Ordering::SeqCst);
            })
        };
        let (link_sink, delivered) = link_recorder();

        let parts = crate::backend::open_spawn(
            &codex_app_server(vec![]),
            &fake.spec,
            80,
            24,
            Some(sink),
            Some(first_turn),
            None,
            Some(link_sink),
            None,
            &LaunchArtifacts::default(),
        )
        .expect("open_spawn");
        assert!(
            matches!(parts.mid_turn, MidTurnPolicy::TransportOwned),
            "premise: this mode is the mode where the link carries turns"
        );

        wait_for_the_fake_to_listen(&fake);
        parts.transport.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )));
        wait_for_recording(|| !recorded.lock().unwrap().is_empty(), &delivered, &fake);
        wait_until(
            || !delivered.lock().unwrap().is_empty(),
            "the connection outcome being delivered",
        );
        let ready = matches!(
            delivered.lock().unwrap().first(),
            Some(crate::transport::LinkResolution::Ready { .. })
        );
        let before_any_turn = first_turns.load(Ordering::SeqCst);

        parts
            .transport
            .send_turn(TurnInput {
                id: "u-1".into(),
                body: b"hi".to_vec(),
                origin: InputOrigin::User,
            })
            .expect("accepts");
        wait_until(
            || first_turns.load(Ordering::SeqCst) > 0,
            "the first-turn port being called on the fake's user message echo",
        );
        parts.transport.shutdown();

        let leftover = fake.remove();
        assert!(
            ready,
            "the connection did not stand: {:?}",
            delivered.lock().unwrap()
        );
        assert_eq!(
            before_any_turn, 0,
            "counted the first turn from the handshake alone — the 0-turn id of an incarnation turned off without input gets persisted"
        );
        assert_eq!(first_turns.load(Ordering::SeqCst), 1);
        assert_eq!(*recorded.lock().unwrap(), vec![thread_id]);
        assert!(
            leftover.is_empty(),
            "could not remove the fake app-server files: {leftover:?}"
        );
    }

    // ── ADR-0209: MCP server attachment (`-c mcp_servers.<name>=…`) — the only physical wiring of codex mail ──────────

    /// Collects only the slots **right after** `-c` — the assertions measuring order · adjacency lean on that form.
    fn config_override_values(argv: &[String]) -> Vec<String> {
        argv.windows(2)
            .filter(|p| p[0] == CONFIG_OVERRIDE_FLAG)
            .map(|p| p[1].clone())
            .collect()
    }

    /// ★The generated value is pinned **literally**★ — a wire contract the codex config parser reads; one wrong character yields not an error but 「mail silently does not work」. Confirmed with the real codex (0.155.0: passes `--strict-config` +
    /// registered in `mcpServerStatus/list`).
    #[test]
    fn the_mcp_override_value_is_pinned_byte_for_byte() {
        let s = spec_with_control(&codex(vec![]), Some(endpoint()));
        assert!(
            config_override_values(&codex_argv(&s)).contains(
                &"mcp_servers.engram={url='http://127.0.0.1:7777/mcp',bearer_token_env_var='ENGRAM_TOKEN',default_tools_approval_mode='approve'}"
                    .to_string()
            ),
            "generated `-c` values: {:?}",
            config_override_values(&codex_argv(&s))
        );
    }

    /// ★The token must have not a single character in argv★ — this is why this backend carries the env variable **name** instead of
    /// an inline value. Reverted, the token leaks just by any process of the same user reading argv.
    #[test]
    fn the_bearer_token_never_reaches_the_command_line() {
        let ep = endpoint();
        let token = ep.token.clone();
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let s = spec_with_control(&command, Some(ep.clone()));
            assert!(
                !s.args.iter().any(|a| a.contains(&token)),
                "the token was carried in argv: {:?}",
                s.args
            );
            assert_eq!(
                env_value(&s, TOKEN_ENV),
                Some(token.as_str()),
                "the token must go only as one copy in env"
            );
        }
    }

    /// ★It does not tell modes apart★ — the mail entrance is needed in both modes. Confusing it with the hook registration being terminal-only
    /// and splitting this axis by mode too would leave codex launched as app-server with zero send entrances.
    #[test]
    fn the_mcp_override_rides_both_output_modes() {
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let s = spec_with_control(&command, Some(endpoint()));
            let values = config_override_values(&codex_argv(&s));
            assert!(
                values
                    .iter()
                    .any(|v| v
                        .starts_with(&format!("{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}="))),
                "MCP attachment is missing from {command:?}: {values:?}"
            );
        }
    }

    /// A spawn without a control channel has nothing to attach — inventing a value with an empty url makes codex come up holding a server that cannot come up.
    #[test]
    fn no_control_endpoint_means_no_mcp_override() {
        let s = spec(&codex(vec![]), "C:/workspace");
        assert!(
            !codex_argv(&s)
                .iter()
                .any(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX)),
            "MCP attachment was carried on a spawn without an endpoint: {:?}",
            codex_argv(&s)
        );
    }

    /// ★If the operator turns off MCP mail it must be off **in codex too**★ (2026-09-20): the old gate looked only at the control channel's **existence**, so `ENGRAM_DISALLOW_MCP_SEND=1` lost only claude's send entrance and codex's stayed open — and that
    /// attachment carries `default_tools_approval_mode='approve'`, i.e. **auto-approved**.
    /// ★Measuring the on row **together** is the point★ — an off-only test stays green even if the attachment is erased entirely, removing the only physical wiring of codex mail (silence, not an error).
    /// ★It also measures that the control plane is not cut★ — turning off mail and cutting control verbs differ (ADR-0132 decision 5); losing credentials would strip them from the children codex launches.
    #[test]
    fn turning_mcp_send_off_takes_the_override_off_codex_too() {
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let off = spec_with_control(&command, Some(endpoint_without_mcp_send()));
            assert!(
                !codex_argv(&off)
                    .iter()
                    .any(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX)),
                "{command:?}: there is no MCP send grant but the attachment was carried — the daemon's block bypasses only this \
                 backend: {:?}",
                codex_argv(&off)
            );
            assert_eq!(
                env_value(&off, TOKEN_ENV),
                Some("deadbeef"),
                "{command:?}: turning off mail must not make the control plane credentials disappear too"
            );
            let on = spec_with_control(&command, Some(endpoint()));
            assert!(
                codex_argv(&on)
                    .iter()
                    .any(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX)),
                "{command:?}: the attachment is missing on an authorized spawn — send entrances become 0: {:?}",
                codex_argv(&on)
            );
        }
    }

    /// ★A spawn 「on the delivery roster with zero send entrances」 is cut before it comes up★: `reads_messages` true puts this backend on the roster and `accepts_mcp_config` true makes the daemon carry `mail_allowed=false` and close the CLI mirror, so without
    /// the MCP attachment every arriving `request` is **a contract nobody can answer** — eternal silence, no trace.
    /// ★For claude the daemon's `?` (mcp-config write failure) guards this; this backend writes no file, so the daemon has nothing to cut with★ — this gate fills the asymmetry.
    /// ★Called through the public dispatch★ — hanging it on the impl only would miss the path the assembly point calls.
    #[test]
    fn an_unbuildable_attachment_fails_the_spawn_closed() {
        let bad = "http://127.0.0.1:1/it's/mcp";
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let ep = ControlEndpoint {
                url: bad.to_string(),
                ..endpoint()
            };
            let err = crate::backend::precheck_control_endpoint(&command, Some(&ep))
                .expect_err("the attachment cannot be built but the spawn proceeds as is — it comes up with zero send entrances");
            assert!(
                err.contains(bad),
                "{command:?}: the reason must carry the cause: {err}"
            );
            // Even with the gate, the assembly **still** does not invent the argument — the two walls each stand.
            let s = spec_with_control(&command, Some(ep));
            assert!(
                !codex_argv(&s)
                    .iter()
                    .any(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX)),
                "{command:?}: forced in a value that cannot be built: {:?}",
                codex_argv(&s)
            );
        }
    }

    /// ★The three legitimate absences are pinned **together** — without them the gate grows toward killing sound spawns★. ② is load-bearing: a spawn whose operator turned off MCP mail has no intent to attach, so whatever the address there is no reason to cut it.
    #[test]
    fn the_legitimate_absences_still_spawn() {
        let bad = "http://127.0.0.1:1/it's/mcp";
        for command in [codex(vec![]), codex_app_server(vec![])] {
            // ① There is no control channel at all — a codex coming up without mail is normal.
            assert!(
                crate::backend::precheck_control_endpoint(&command, None).is_ok(),
                "{command:?}: cut a spawn without a control channel"
            );
            // ② The daemon did not authorize the MCP send entrance (the operator turned it off) — it does not matter that the address cannot be carried.
            let off = ControlEndpoint {
                url: bad.to_string(),
                ..endpoint_without_mcp_send()
            };
            assert!(
                crate::backend::precheck_control_endpoint(&command, Some(&off)).is_ok(),
                "{command:?}: cut a spawn with no intent to attach as unattachable — in a harness with the knob on, codex \
                 spawns die entirely"
            );
            // ③ An ordinary production spawn.
            assert!(
                crate::backend::precheck_control_endpoint(&command, Some(&endpoint())).is_ok(),
                "{command:?}: cut the production endpoint"
            );
        }
    }

    /// ★An address that cannot be held in a TOML literal string is **not carried, and the spawn is cut**★ — pushed in with an invented escape,
    /// the value dies in parsing or points to a different address, and both have one symptom, 「mail does not work」.
    #[test]
    fn an_unrepresentable_control_url_drops_the_attachment_instead_of_mangling_it() {
        for bad in [
            "http://127.0.0.1:1/it's/mcp",
            "http://127.0.0.1:1/%USER%/mcp",
        ] {
            let mut ep = endpoint();
            ep.url = bad.to_string();
            let s = spec_with_control(&codex(vec![]), Some(ep));
            assert!(
                !codex_argv(&s)
                    .iter()
                    .any(|a| a.starts_with(MCP_SERVER_OVERRIDE_PREFIX)),
                "built the attachment value with {bad}: {:?}",
                codex_argv(&s)
            );
        }
    }

    /// The command-line count holds the control URL to [`MCP_URL_BUDGET`] (`"` and `\` counted twice) — a longer
    /// one is unrepresentable, so the spawn fails closed instead of outgrowing the count.
    #[test]
    fn a_control_url_over_its_command_line_bound_is_unrepresentable() {
        let attachment = |url: String| {
            let mut ep = endpoint();
            ep.url = url;
            mcp_attachment(Some(&ep))
        };
        let prefix = "http://127.0.0.1:1/";
        let room = MCP_URL_BUDGET - prefix.len();
        assert!(matches!(
            attachment(format!("{prefix}{}", "m".repeat(room))),
            McpAttachment::Attach(_)
        ));
        for over in [
            format!("{prefix}{}", "m".repeat(room + 1)),
            format!("{prefix}\\{}", "m".repeat(room - 1)),
        ] {
            assert!(
                matches!(attachment(over.clone()), McpAttachment::Unrepresentable(_)),
                "{over}"
            );
        }
        let mut ep = endpoint();
        ep.url = format!("{prefix}{}", "m".repeat(room + 1));
        assert!(CodexBackend
            .precheck_control_endpoint(&codex(vec![]), Some(&ep))
            .is_err());
    }

    /// A `"` or a cmd metacharacter in the control URL would change how cmd reads the rest of the line — the
    /// attachment is unrepresentable instead.
    #[test]
    fn a_control_url_with_a_quote_or_a_cmd_metachar_is_unrepresentable() {
        for bad in ['"', '&', '|', '<', '>', '^'] {
            let mut ep = endpoint();
            ep.url = format!("http://127.0.0.1:1/m{bad}cp");
            assert!(
                matches!(mcp_attachment(Some(&ep)), McpAttachment::Unrepresentable(_)),
                "{bad:?}"
            );
        }
        assert!(matches!(
            mcp_attachment(Some(&endpoint())),
            McpAttachment::Attach(_)
        ));
    }

    /// ★Even if the pass-through sets the same key, ours is carried **after** it★ — the last `-c` wins, so this order
    /// is the life or death of the mail entrance. And user arguments are not erased (filtering is censorship).
    #[test]
    fn a_conflicting_mcp_passthrough_loses_to_ours_and_is_not_filtered() {
        let key = format!("{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}");
        assert!(
            passthrough_overrides_key(
                &[
                    CONFIG_OVERRIDE_FLAG.to_string(),
                    format!("{key}={{url='http://evil/mcp'}}")
                ],
                &key,
            ),
            "did not recognize a pass-through of the same key — overrides the user setting without a warning"
        );
        let s = spec_with_control(
            &codex(vec![
                CONFIG_OVERRIDE_FLAG,
                "mcp_servers.engram={url='http://evil/mcp'}",
            ]),
            Some(endpoint()),
        );
        let values = config_override_values(&codex_argv(&s));
        let ours = values
            .iter()
            .position(|v| v.contains(MCP_BEARER_ENV_KEY))
            .expect("our attachment value is missing");
        let theirs = values
            .iter()
            .position(|v| v.contains("evil"))
            .expect("the pass-through was filtered — user arguments must not be censored");
        assert!(
            ours > theirs,
            "our value was carried in front (it is losing): {values:?}"
        );
    }

    /// ★The config path does not reach argv even when it comes as `Some`★ — production sends `None`
    /// (`writes_mcp_config_file()` = false → the daemon writes no file), and this test feeds the broken-gate state on
    /// purpose to measure the second wall. codex has no flag for that file, so copying claude's flags in 「since it is
    /// Some, use it」 kills the launch.
    #[test]
    fn the_daemon_written_files_are_still_not_translated_into_argv() {
        let ep = endpoint();
        let s = spec_with_control(&codex(vec![]), Some(ep.clone()));
        let argv = codex_argv(&s);
        for forbidden in ["--mcp-config", "--settings", "--append-system-prompt-file"] {
            assert!(
                !argv.iter().any(|a| a == forbidden),
                "claude's flag `{forbidden}` was carried in codex argv: {argv:?}"
            );
        }
        let path = ep.config_path.expect("the fixture fills the config path");
        let path = path.to_string_lossy().into_owned();
        assert!(
            !argv.iter().any(|a| a.contains(&path)),
            "the path `{path}` was carried in argv: {argv:?}"
        );
    }

    // ── ADR-0215: instructions (terminal = `-c developer_instructions=…` · app-server = handshake JSON) ──

    fn developer_instructions_value(argv: &[String]) -> Option<String> {
        let prefix = format!("{DEVELOPER_INSTRUCTIONS_KEY}=");
        config_override_values(argv)
            .into_iter()
            .find(|v| v.starts_with(&prefix))
    }

    /// ★Our instructions go **after** the pass-through★ — with the same key passed twice by `-c` the last one wins; put
    /// first, one user argument line would erase the instructions without a signal (the same rule as the MCP attachment).
    // ADR-0337
    #[test]
    fn the_instructions_override_follows_the_passthrough() {
        let artifacts = LaunchArtifacts {
            real: true,
            instructions_override: Some(
                "developer_instructions='teach the reply contract'".to_string(),
            ),
            ..LaunchArtifacts::default()
        };
        let s = CodexBackend.build_spec(
            &codex(vec![
                CONFIG_OVERRIDE_FLAG,
                "developer_instructions='theirs'",
            ]),
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("C:/workspace"),
            vec![],
            Some(endpoint()),
            &artifacts,
        );
        let values = config_override_values(&codex_argv(&s));
        let ours = values
            .iter()
            .position(|v| v.contains("teach the reply contract"))
            .expect("our value is missing");
        let theirs = values
            .iter()
            .position(|v| v.contains("theirs"))
            .expect("the pass-through was filtered — user arguments must not be censored");
        assert!(
            ours > theirs,
            "our value was carried in front (it is losing): {values:?}"
        );
    }

    /// ★The terminal hook lines come right after the preset lines★ — after the pass-through, before the MCP attachment
    /// and the priming, so our overrides still come last.
    // ADR-0420
    #[test]
    fn the_terminal_hook_lines_follow_the_preset_lines_and_precede_our_overrides() {
        let artifacts = LaunchArtifacts {
            real: true,
            config_args: vec![CONFIG_OVERRIDE_FLAG.to_string(), "model='o3'".to_string()],
            hook_args: vec![
                CONFIG_OVERRIDE_FLAG.to_string(),
                "hooks.Stop=[]".to_string(),
                CONFIG_OVERRIDE_FLAG.to_string(),
                "hooks.state={}".to_string(),
            ],
            instructions_override: Some("developer_instructions='p'".to_string()),
            ..LaunchArtifacts::default()
        };
        let s = CodexBackend.build_spec(
            &codex(vec![CONFIG_OVERRIDE_FLAG, "theirs=1"]),
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("C:/workspace"),
            vec![],
            Some(endpoint()),
            &artifacts,
        );
        let values = config_override_values(&codex_argv(&s));
        assert_eq!(
            values[..4],
            ["theirs=1", "model='o3'", "hooks.Stop=[]", "hooks.state={}"]
        );
        assert!(
            values[4].starts_with(MCP_SERVER_OVERRIDE_PREFIX),
            "{values:?}"
        );
        assert_eq!(values[5], "developer_instructions='p'");
        assert_eq!(values.len(), 6, "{values:?}");
    }

    /// ★The app-server branch does not touch the value★ — it uses none of the command line, so neither newlines nor the `'` · `%` the terminal branch cuts can break the JSON body; moving that conversion here would only break the document the agent reads.
    #[test]
    fn the_app_server_handshake_carries_the_document_unmodified() {
        let text = "line one\nyou can't skip 50% of it\n";
        let s = spec_with_control(&codex_app_server(vec![]), Some(endpoint()));
        let params = match thread_open(&s, None, Some(text.to_string()), None) {
            ThreadOpen::Start(p) => p,
            ThreadOpen::Resume(_) => panic!("resume was picked although it is not a resume"),
        };
        let v = serde_json::to_value(&params).expect("serialize");
        assert_eq!(
            v.get("developerInstructions").and_then(|v| v.as_str()),
            Some(text),
            "the instructions in the handshake JSON differ from the original: {v}"
        );
    }

    /// ★A resumed app-server thread carries the instructions too★ — unmodified, like the start — so a compaction in the
    /// resumed process rebuilds its initial context with them (M8; the reason's source = the `ThreadResumeParams` field
    /// doc). Without the field that rebuild had no instructions at all.
    // ADR-0337
    #[test]
    fn a_resuming_app_server_handshake_carries_the_instructions_field() {
        let text = "teach the reply contract\nyou can't skip 50% of it\n";
        let s = spec_with_control(&codex_app_server(vec![]), Some(endpoint()));
        let params = match thread_open(&s, Some(Uuid::new_v4()), Some(text.to_string()), None) {
            ThreadOpen::Resume(p) => p,
            ThreadOpen::Start(_) => panic!("start was picked although it is a resume"),
        };
        let v = serde_json::to_value(&params).expect("serialize");
        assert_eq!(
            v.get("developerInstructions").and_then(|v| v.as_str()),
            Some(text),
            "the resume request does not carry the document as is: {v}"
        );
    }

    /// The Noop launch-material app-server handshake — pins the values for a new thread and a resume (no instructions in
    /// either), plus the open plan's empty roots and context. A Noop spawn carries no instructions (ADR-0370).
    // ADR-0370
    #[test]
    fn the_app_server_handshake_is_pinned_for_a_spawn_without_launch_kit() {
        let spec = CommandSpec {
            program: CODEX_PROGRAM.to_string(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: PathBuf::from("C:/workspace"),
        };
        let thread = Uuid::from_u128(0x2222_2222_2222_4222_8222_2222_2222_2222);
        let handshake = |resume: Option<Uuid>| {
            let open = app_server_open(&spec, resume, &LaunchArtifacts::default())
                .expect("a Noop open plan");
            assert!(open.extra_roots.is_empty(), "a Noop spawn sent skill roots");
            assert!(
                open.first_turn_context.is_none(),
                "a Noop spawn carries a first-turn context"
            );
            match open.thread {
                ThreadOpen::Start(p) => serde_json::json!({ "start": p }),
                ThreadOpen::Resume(p) => serde_json::json!({ "resume": p }),
            }
        };
        assert_eq!(
            handshake(None),
            serde_json::json!({ "start": {
                "cwd": "C:/workspace",
                "approvalPolicy": "on-request",
                "sandbox": "workspace-write",
            }}),
            "fresh"
        );
        assert_eq!(
            handshake(Some(thread)),
            serde_json::json!({ "resume": {
                "threadId": "22222222-2222-4222-8222-222222222222",
                "cwd": "C:/workspace",
                "approvalPolicy": "on-request",
                "sandbox": "workspace-write",
                "excludeTurns": true,
            }}),
            "resume"
        );
    }

    // ── ADR-0337: the real JSON open plan (TRD S25 step 6 §4-7) ──────────────────────────────────

    const MERGED: &str = "merged instructions\nsecond line\n";

    fn real_json_artifacts(text: Option<&str>, changed: bool) -> LaunchArtifacts {
        LaunchArtifacts {
            real: true,
            instructions_text: text.map(str::to_string),
            instructions_changed: changed,
            skill_roots: vec![
                PathBuf::from("C:/vol/engram/skills"),
                PathBuf::from("C:/rules/.agents/skills"),
            ],
            ..LaunchArtifacts::default()
        }
    }

    fn open_plan(resume: Option<Uuid>, artifacts: &LaunchArtifacts) -> AppServerOpen {
        let s = spec_with_control(&codex_app_server(vec![]), Some(endpoint()));
        app_server_open(&s, resume, artifacts).expect("an open plan")
    }

    fn developer_instructions_of(plan: &AppServerOpen) -> Option<String> {
        let v = match &plan.thread {
            ThreadOpen::Start(p) => serde_json::to_value(p),
            ThreadOpen::Resume(p) => serde_json::to_value(p),
        }
        .expect("serialize");
        v.get("developerInstructions")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    /// Start · resume × changed · unchanged: the merged text goes as `developerInstructions` on both branches (M8 — on
    /// every resume, changed or not), the roots go as given, and the context = preface + text only on a changed
    /// resume — a Fresh spawn never carries it, even with `instructions_changed` (B10).
    #[test]
    fn a_real_open_plan_sends_the_merged_text_the_roots_and_a_context_only_on_a_changed_resume() {
        let thread = Uuid::new_v4();
        for (resume, changed, context) in [
            (None, false, false),
            (None, true, false),
            (Some(thread), false, false),
            (Some(thread), true, true),
        ] {
            let at = format!("resume={} changed={changed}", resume.is_some());
            let plan = open_plan(resume, &real_json_artifacts(Some(MERGED), changed));
            assert_eq!(
                matches!(plan.thread, ThreadOpen::Resume(_)),
                resume.is_some(),
                "{at}"
            );
            assert_eq!(
                developer_instructions_of(&plan).as_deref(),
                Some(MERGED),
                "{at}"
            );
            assert_eq!(
                plan.extra_roots,
                ["C:/vol/engram/skills", "C:/rules/.agents/skills"],
                "{at}"
            );
            let expected = context.then(|| format!("{INSTRUCTIONS_CHANGED_PREFACE}\n\n{MERGED}"));
            assert_eq!(plan.first_turn_context, expected, "{at}");
        }
        assert_eq!(
            INSTRUCTIONS_CHANGED_PREFACE,
            "The instructions for this agent changed since its last launch. The current instructions follow \
             and take precedence over earlier instructions in this conversation."
        );
    }

    /// A real kit with no instructions text sends no instructions field and no context; with no skill roots it sends no
    /// roots.
    #[test]
    fn a_real_open_plan_without_instructions_carries_no_instructions_field() {
        for resume in [None, Some(Uuid::new_v4())] {
            let mut artifacts = real_json_artifacts(None, false);
            artifacts.skill_roots.clear();
            let plan = open_plan(resume, &artifacts);
            assert_eq!(
                developer_instructions_of(&plan),
                None,
                "resume={}",
                resume.is_some()
            );
            assert!(plan.extra_roots.is_empty());
            assert!(plan.first_turn_context.is_none());
        }
    }

    fn hook_trust(key: &str, from: &str) -> crate::launch::HookTrust {
        crate::launch::HookTrust {
            key: key.to_string(),
            hash: format!("sha256:{key}"),
            from: from.to_string(),
        }
    }

    fn params_value(plan: &AppServerOpen) -> serde_json::Value {
        match &plan.thread {
            ThreadOpen::Start(p) => serde_json::to_value(p),
            ThreadOpen::Resume(p) => serde_json::to_value(p),
        }
        .expect("serialize")
    }

    /// ★The thread config rides start and resume alike★ — `config: {"hooks": …}` exactly as the assembly wrote it, and
    /// the expected trust entries for the post-open check; with no hook output there is no `config` field at all and no
    /// check (a Noop kit neither).
    // ADR-0420
    #[test]
    fn the_open_plan_carries_the_hook_config_on_start_and_resume_and_expects_its_trust_entries() {
        let hooks = serde_json::json!({
            "Stop": [{"hooks": [{"command": "node s.js", "type": "command"}]}],
            "state": {"k0": {"enabled": true, "trusted_hash": "sha256:k0"}},
        });
        for resume in [None, Some(Uuid::new_v4())] {
            let at = format!("resume={}", resume.is_some());
            let mut artifacts = real_json_artifacts(Some(MERGED), false);
            artifacts.hook_config = Some(hooks.clone());
            artifacts.hook_trust = vec![hook_trust("k0", "team/hooks/a")];
            let plan = open_plan(resume, &artifacts);
            assert_eq!(
                params_value(&plan)["config"],
                serde_json::json!({ "hooks": hooks }),
                "{at}"
            );
            assert_eq!(
                plan.expected_hooks,
                Some(artifacts.hook_trust.clone()),
                "{at}"
            );

            let plain = real_json_artifacts(Some(MERGED), false);
            let plan = open_plan(resume, &plain);
            assert!(params_value(&plan).get("config").is_none(), "{at}");
            assert_eq!(plan.expected_hooks, None, "{at}");

            let noop = LaunchArtifacts {
                real: false,
                ..artifacts
            };
            let plan = open_plan(resume, &noop);
            assert!(params_value(&plan).get("config").is_none(), "{at}");
            assert_eq!(plan.expected_hooks, None, "{at}");
        }
    }

    /// The check follows the thread config: one with no trust entry (only carried other-source state) still runs it
    /// (it watches for a foreign trusted session-flags hook); trust entries without a thread config (terminal mode's
    /// record) do not.
    // ADR-0420
    #[test]
    fn the_check_runs_exactly_when_a_thread_config_is_sent() {
        let mut carried_only = real_json_artifacts(None, false);
        carried_only.hook_config = Some(serde_json::json!({"state": {"u": {"enabled": true}}}));
        assert_eq!(
            open_plan(None, &carried_only).expected_hooks,
            Some(Vec::new())
        );

        let mut entries_only = real_json_artifacts(None, false);
        entries_only.hook_trust = vec![hook_trust("k0", "team/hooks/a")];
        let plan = open_plan(None, &entries_only);
        assert!(params_value(&plan).get("config").is_none());
        assert_eq!(plan.expected_hooks, None);
    }

    /// What one run against the fake app-server saw.
    #[cfg(windows)]
    struct FakeRun {
        methods: Vec<String>,
        received: Vec<serde_json::Value>,
        link: Vec<crate::transport::LinkResolution>,
        /// The first-turn reports (`FirstTurnSink` arguments), in order.
        reports: Vec<bool>,
        /// From the transport's start to the link resolution's delivery.
        resolved_after: std::time::Duration,
    }

    /// Opens a transport on `fake` with `plan`, waits for the link, sends one turn when the link is `Ready` and
    /// `send_turn` is set (and waits for the first-turn report), shuts it down and removes the fake.
    #[cfg(windows)]
    fn run_on_fake(fake: FakeAppServer, plan: AppServerOpen, send_turn: bool) -> FakeRun {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, InputOrigin, StatusSink, TurnInput};
        use std::sync::Mutex;

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: Uuid, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let reports = Arc::new(Mutex::new(Vec::new()));
        let first_turn: FirstTurnSink = {
            let reports = reports.clone();
            Arc::new(move |delivered: bool| reports.lock().unwrap().push(delivered))
        };
        let (link_sink, delivered) = link_recorder();
        let (t, _pid) =
            CodexAppServerTransport::open(&fake.spec, true, None, plan, None, Some(link_sink))
                .expect("open");
        let t = t.with_first_turn(Some(first_turn));
        wait_for_the_fake_to_listen(&fake);
        let started = std::time::Instant::now();
        t.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )));
        wait_until(
            || !delivered.lock().unwrap().is_empty(),
            "the link resolution is delivered",
        );
        let resolved_after = started.elapsed();
        let ready = matches!(
            delivered.lock().unwrap().first(),
            Some(crate::transport::LinkResolution::Ready { .. })
        );
        if ready && send_turn {
            t.send_turn(TurnInput {
                id: "u-1".into(),
                body: b"hi".to_vec(),
                origin: InputOrigin::User,
            })
            .expect("accepted");
            wait_until(
                || !reports.lock().unwrap().is_empty(),
                "the fake's echo calls the first-turn port",
            );
        }
        t.shutdown();
        let run = FakeRun {
            methods: fake.methods(),
            received: fake.received(),
            link: delivered.lock().unwrap().clone(),
            reports: reports.lock().unwrap().clone(),
            resolved_after,
        };
        let leftover = fake.remove();
        assert!(
            leftover.is_empty(),
            "could not remove the fake app-server files: {leftover:?}"
        );
        run
    }

    /// ★The post-open check★ — with expected hooks, `hooks/list {threadId}` goes out right after the thread opens (start
    /// and resume, with the id the peer answered), before the link is `Ready`, and the verdicts ride `Ready`: a key
    /// listed untrusted is named; a trusted, enabled one is not. The thread request carries the config.
    // ADR-0420
    #[cfg(windows)]
    #[test]
    fn the_post_open_check_asks_hooks_list_after_the_thread_opens_and_its_verdicts_ride_ready() {
        const LISTED: &str = r#"{"data":[{"cwd":"C:/workspace","hooks":[{"key":"k0","source":"sessionFlags","enabled":true,"trustStatus":"trusted","currentHash":"sha256:k0"},{"key":"k1","source":"sessionFlags","enabled":true,"trustStatus":"modified","currentHash":"sha256:other"}],"warnings":[],"errors":[]}]}"#;
        let hooks = serde_json::json!({"Stop": [{"hooks": [{"command": "a", "type": "command"}, {"command": "b", "type": "command"}]}]});
        for resume in [false, true] {
            let at = format!("resume={resume}");
            let fake = bake_fake_app_server_with(
                "T-start",
                FakeResume::EchoesTheThreadId,
                FakeOptions {
                    hooks_list: FakeHooksList::Answers(LISTED),
                    ..FakeOptions::default()
                },
            );
            let s = spec(&codex_app_server(vec![]), "C:/workspace");
            let sent = Uuid::new_v4();
            let plan = AppServerOpen {
                thread: thread_open(
                    &s,
                    resume.then_some(sent),
                    None,
                    Some(serde_json::json!({ "hooks": hooks })),
                ),
                extra_roots: Vec::new(),
                first_turn_context: None,
                expected_hooks: Some(vec![
                    hook_trust("k0", "team/hooks/a"),
                    hook_trust("k1", "team/hooks/b"),
                ]),
            };
            let run = run_on_fake(fake, plan, false);
            let opened = if resume {
                method::THREAD_RESUME
            } else {
                method::THREAD_START
            };
            assert_eq!(
                run.methods,
                ["initialize", "initialized", opened, method::HOOKS_LIST],
                "{at}"
            );
            assert_eq!(
                params_of(&run, opened)["config"],
                serde_json::json!({ "hooks": hooks }),
                "{at}"
            );
            let thread_id = if resume {
                format!("resumed-{sent}")
            } else {
                "T-start".to_string()
            };
            assert_eq!(
                params_of(&run, method::HOOKS_LIST),
                serde_json::json!({ "threadId": thread_id }),
                "{at}"
            );
            assert_eq!(
                run.link,
                [crate::transport::LinkResolution::Ready {
                    notices: vec![crate::launch::Reason {
                        code: crate::launch::CODEX_HOOKS_UNTRUSTED.to_string(),
                        args: serde_json::json!({ "why": "untrusted", "hooks": ["team/hooks/b"] }),
                        line: None,
                    }],
                }],
                "{at}"
            );
        }
    }

    /// ★A failed `hooks/list` keeps `Ready`★ — an older codex without the method answers `-32601`; the link still
    /// stands, with the `unchecked` run notice naming the hooks.
    // ADR-0420
    #[cfg(windows)]
    #[test]
    fn a_failed_hooks_list_keeps_ready_with_an_unchecked_notice() {
        let fake = bake_fake_app_server_with(
            "T-start",
            FakeResume::EchoesTheThreadId,
            FakeOptions::default(),
        );
        let s = spec(&codex_app_server(vec![]), "C:/workspace");
        let plan = AppServerOpen {
            thread: thread_open(&s, None, None, Some(serde_json::json!({"hooks": {}}))),
            extra_roots: Vec::new(),
            first_turn_context: None,
            expected_hooks: Some(vec![hook_trust("k0", "team/hooks/a")]),
        };
        let run = run_on_fake(fake, plan, false);
        let [crate::transport::LinkResolution::Ready { notices }] = run.link.as_slice() else {
            panic!("the link did not stand: {:?}", run.link);
        };
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!(notices[0].code, crate::launch::CODEX_HOOKS_UNTRUSTED);
        assert_eq!(notices[0].args["why"], "unchecked");
        assert_eq!(
            notices[0].args["hooks"],
            serde_json::json!(["team/hooks/a"])
        );
        assert!(
            notices[0].args["error"]
                .as_str()
                .is_some_and(|e| e.contains("hooks/list")),
            "{notices:?}"
        );
    }

    /// ★A silent `hooks/list` cannot eat the handshake budget★ — a codex that accepts the request and never answers
    /// holds the link for at most [`transport::HOOKS_LIST_CAP`] (plus one 500 ms sweep), not the rest of the 10 s
    /// handshake budget a resumed agent's history paging shares; `Ready` still comes, with the `unchecked` notice whose
    /// error states the wait in milliseconds.
    // ADR-0420
    #[cfg(windows)]
    #[test]
    fn a_silent_hooks_list_keeps_ready_within_its_cap() {
        let fake = bake_fake_app_server_with(
            "T-start",
            FakeResume::EchoesTheThreadId,
            FakeOptions {
                hooks_list: FakeHooksList::Silent,
                ..FakeOptions::default()
            },
        );
        let s = spec(&codex_app_server(vec![]), "C:/workspace");
        let plan = AppServerOpen {
            thread: thread_open(&s, None, None, Some(serde_json::json!({"hooks": {}}))),
            extra_roots: Vec::new(),
            first_turn_context: None,
            expected_hooks: Some(vec![hook_trust("k0", "team/hooks/a")]),
        };
        let run = run_on_fake(fake, plan, false);
        let [crate::transport::LinkResolution::Ready { notices }] = run.link.as_slice() else {
            panic!("the link did not stand: {:?}", run.link);
        };
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!(notices[0].args["why"], "unchecked", "{notices:?}");
        let error = notices[0].args["error"].as_str().unwrap_or_default();
        let waited: u128 = error
            .strip_prefix("hooks/list: no answer within ")
            .and_then(|rest| rest.strip_suffix(" ms"))
            .and_then(|ms| ms.parse().ok())
            .unwrap_or_else(|| panic!("the error does not state the wait in ms: {error}"));
        assert!(
            waited > 0 && waited <= transport::HOOKS_LIST_CAP.as_millis(),
            "{error}"
        );
        // Without the cap the wait is the rest of the 10 s budget; with it, the cap plus one sweep and the round trips.
        assert!(
            run.resolved_after < transport::HOOKS_LIST_CAP + std::time::Duration::from_secs(3),
            "Ready came after {:?}",
            run.resolved_after
        );
    }

    #[cfg(windows)]
    fn params_of(run: &FakeRun, method_name: &str) -> serde_json::Value {
        run.received
            .iter()
            .find(|v| v["method"] == method_name)
            .map(|v| v["params"].clone())
            .unwrap_or_else(|| panic!("{method_name} was not sent: {:?}", run.methods))
    }

    #[cfg(windows)]
    const FAKE_CODEX_0_163: &str =
        "codex_cli_rs/0.163.0 (Windows 10.0.26200; x86_64) unknown (engram-dashboard; 0.1.0)";

    /// The received order per start · resume × roots · no roots × context · no context: `initialize` (with
    /// `experimentalApi` exactly when there is a context) → `initialized` → [`skills/extraRoots/set` with the roots —
    /// on a resume too] → `thread/start` | `thread/resume`; with a context the first `turn/start` carries it and the
    /// echo reports the instructions delivered.
    #[cfg(windows)]
    #[test]
    fn the_handshake_sends_the_open_plan_in_order() {
        for resume in [false, true] {
            for roots in [false, true] {
                for context in [false, true] {
                    let at = format!("resume={resume} roots={roots} context={context}");
                    let fake = bake_fake_app_server_with(
                        "T-start",
                        FakeResume::EchoesTheThreadId,
                        FakeOptions {
                            user_agent: FAKE_CODEX_0_163,
                            ..FakeOptions::default()
                        },
                    );
                    let s = spec(&codex_app_server(vec![]), "C:/workspace");
                    let extra_roots: Vec<String> = if roots {
                        vec![
                            "C:/vol/engram/skills".into(),
                            "C:/rules/.agents/skills".into(),
                        ]
                    } else {
                        Vec::new()
                    };
                    let plan = AppServerOpen {
                        thread: thread_open(
                            &s,
                            resume.then(Uuid::new_v4),
                            Some("dev text".to_string()),
                            None,
                        ),
                        extra_roots: extra_roots.clone(),
                        first_turn_context: context.then(|| "ctx text".to_string()),
                        expected_hooks: None,
                    };
                    let run = run_on_fake(fake, plan, context);
                    assert!(
                        matches!(
                            run.link.first(),
                            Some(crate::transport::LinkResolution::Ready { .. })
                        ),
                        "{at}: {:?}",
                        run.link
                    );

                    let mut expected = vec!["initialize", "initialized"];
                    if roots {
                        expected.push(method::SKILLS_EXTRA_ROOTS_SET);
                    }
                    expected.push(if resume {
                        method::THREAD_RESUME
                    } else {
                        method::THREAD_START
                    });
                    if context {
                        expected.push(method::TURN_START);
                    }
                    assert_eq!(run.methods, expected, "{at}");

                    let init = params_of(&run, method::INITIALIZE);
                    if context {
                        assert_eq!(
                            init["capabilities"],
                            serde_json::json!({"experimentalApi": true}),
                            "{at}"
                        );
                    } else {
                        assert!(init.get("capabilities").is_none(), "{at}: {init}");
                    }
                    if roots {
                        assert_eq!(
                            params_of(&run, method::SKILLS_EXTRA_ROOTS_SET),
                            serde_json::json!({ "extraRoots": extra_roots }),
                            "{at}"
                        );
                    }
                    let opened = params_of(
                        &run,
                        if resume {
                            method::THREAD_RESUME
                        } else {
                            method::THREAD_START
                        },
                    );
                    assert_eq!(opened["developerInstructions"], "dev text", "{at}");
                    if context {
                        assert_eq!(
                            params_of(&run, method::TURN_START)["additionalContext"],
                            serde_json::json!({"engram-instructions": {"kind": "application", "value": "ctx text"}}),
                            "{at}"
                        );
                        assert_eq!(run.reports, [true], "{at}");
                    }
                }
            }
        }
    }

    /// M8 + the version gate (lead NS1): the resume carries `developerInstructions` for a codex at 0.155.0 or later read
    /// from the `userAgent`; below it, or with a `userAgent` whose version cannot be read, the field is left out.
    #[cfg(windows)]
    #[test]
    fn the_resume_carries_the_instructions_only_from_codex_0_155_0() {
        for (user_agent, carried) in [
            (FAKE_CODEX_0_163, true),
            ("codex_cli_rs/0.154.0 (Windows 10.0.26200; x86_64)", false),
            ("engram-fake/0", false),
        ] {
            let fake = bake_fake_app_server_with(
                "T-start",
                FakeResume::EchoesTheThreadId,
                FakeOptions {
                    user_agent,
                    ..FakeOptions::default()
                },
            );
            let s = spec(&codex_app_server(vec![]), "C:/workspace");
            let plan = AppServerOpen {
                thread: thread_open(&s, Some(Uuid::new_v4()), Some("dev text".to_string()), None),
                extra_roots: Vec::new(),
                first_turn_context: None,
                expected_hooks: None,
            };
            let run = run_on_fake(fake, plan, false);
            let resumed = params_of(&run, method::THREAD_RESUME);
            assert_eq!(
                resumed.get("developerInstructions").is_some(),
                carried,
                "{user_agent}: {resumed}"
            );
            assert_eq!(resumed["excludeTurns"], true, "{user_agent}");
        }
    }

    /// An error answer to `skills/extraRoots/set` fails the link (fail closed — R16), and no thread is opened.
    #[cfg(windows)]
    #[test]
    fn an_error_answer_to_the_extra_roots_fails_the_link() {
        for resume in [false, true] {
            let fake = bake_fake_app_server_with(
                "T-start",
                FakeResume::EchoesTheThreadId,
                FakeOptions {
                    user_agent: FAKE_CODEX_0_163,
                    extra_roots: FakeExtraRoots::Rejects,
                    ..FakeOptions::default()
                },
            );
            let s = spec(&codex_app_server(vec![]), "C:/workspace");
            let plan = AppServerOpen {
                thread: thread_open(&s, resume.then(Uuid::new_v4), None, None),
                extra_roots: vec!["C:/vol/engram/skills".into()],
                first_turn_context: None,
                expected_hooks: None,
            };
            let run = run_on_fake(fake, plan, false);
            match run.link.first() {
                Some(crate::transport::LinkResolution::Failed { reason }) => assert!(
                    reason.contains(FAKE_EXTRA_ROOTS_REJECTION),
                    "resume={resume}: {reason}"
                ),
                other => panic!("resume={resume}: the link did not fail: {other:?}"),
            }
            assert_eq!(
                run.methods,
                ["initialize", "initialized", method::SKILLS_EXTRA_ROOTS_SET],
                "resume={resume}"
            );
        }
    }

    /// A real resume with changed instructions, through `open_spawn`: both channels carry the same text — the resume's
    /// `developerInstructions` and the first turn's context (preface + text), with the opt-in on — plus the roots; the
    /// echo of that turn reports the instructions delivered.
    #[cfg(windows)]
    #[test]
    fn a_real_changed_resume_through_open_spawn_sends_both_channels() {
        use crate::output_core::{OutputCore, TurnWiring};
        use crate::types::{AgentInfo, AgentStatus, InputOrigin, StatusSink, TurnInput};
        use std::sync::Mutex;

        struct NoopStatus;
        impl StatusSink for NoopStatus {
            fn status_changed(&self, _id: Uuid, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        let fake = bake_fake_app_server_with(
            "T-start",
            FakeResume::EchoesTheThreadId,
            FakeOptions {
                user_agent: FAKE_CODEX_0_163,
                ..FakeOptions::default()
            },
        );
        let reports = Arc::new(Mutex::new(Vec::new()));
        let first_turn: FirstTurnSink = {
            let reports = reports.clone();
            Arc::new(move |delivered: bool| reports.lock().unwrap().push(delivered))
        };
        let (link_sink, delivered) = link_recorder();
        let parts = crate::backend::open_spawn(
            &codex_app_server(vec![]),
            &fake.spec,
            80,
            24,
            None,
            Some(first_turn),
            Some(Uuid::new_v4()),
            Some(link_sink),
            None,
            &real_json_artifacts(Some(MERGED), true),
        )
        .expect("open_spawn");
        wait_for_the_fake_to_listen(&fake);
        parts.transport.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )));
        wait_until(
            || !delivered.lock().unwrap().is_empty(),
            "the link resolution is delivered",
        );
        parts
            .transport
            .send_turn(TurnInput {
                id: "u-1".into(),
                body: b"hi".to_vec(),
                origin: InputOrigin::User,
            })
            .expect("accepted");
        wait_until(
            || !reports.lock().unwrap().is_empty(),
            "the fake's echo calls the first-turn port",
        );
        parts.transport.shutdown();
        let initialize = fake.params_of(method::INITIALIZE);
        let roots = fake.params_of(method::SKILLS_EXTRA_ROOTS_SET);
        let resumed = fake.params_of(method::THREAD_RESUME);
        let turn = fake.params_of(method::TURN_START);
        let leftover = fake.remove();

        assert_eq!(
            initialize.expect("initialize")["capabilities"],
            serde_json::json!({"experimentalApi": true})
        );
        assert_eq!(
            roots.expect("skills/extraRoots/set"),
            serde_json::json!({"extraRoots": ["C:/vol/engram/skills", "C:/rules/.agents/skills"]})
        );
        assert_eq!(
            resumed.expect("thread/resume")["developerInstructions"],
            MERGED
        );
        assert_eq!(
            turn.expect("turn/start")["additionalContext"]["engram-instructions"]["value"],
            format!("{INSTRUCTIONS_CHANGED_PREFACE}\n\n{MERGED}")
        );
        assert_eq!(*reports.lock().unwrap(), [true]);
        assert!(
            leftover.is_empty(),
            "could not remove the fake app-server files: {leftover:?}"
        );
    }

    /// A Noop spawn puts no developer instructions on the command line in either mode, control channel or not
    /// (ADR-0370). The app-server handshake side is pinned by
    /// `the_app_server_handshake_is_pinned_for_a_spawn_without_launch_kit`.
    // ADR-0370
    #[test]
    fn a_noop_spawn_has_no_developer_instructions_on_the_command_line() {
        for command in [codex(vec![]), codex_app_server(vec![])] {
            for control in [Some(endpoint()), None] {
                let s = spec_with_control(&command, control);
                assert!(
                    developer_instructions_value(&codex_argv(&s)).is_none(),
                    "{command:?}: {:?}",
                    s.args
                );
            }
        }
    }

    #[test]
    fn cwd_and_env_are_forwarded() {
        let cwd = PathBuf::from("C:/workspace");
        let env = vec![("BAR".to_string(), "baz".to_string())];
        let s = CodexBackend.build_spec(
            &codex(vec![]),
            SpawnMode::Fresh,
            None,
            None,
            cwd.clone(),
            env.clone(),
            None,
            &LaunchArtifacts::default(),
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, env);
    }
}

// ── ADR-0218: session id recovery wiring of the terminal branch (real process) ────────────────────────────
//
// ★Measured: 「a terminal spawn flows the recovered id to **the sink it was handed**」★ — this wiring alone could be cut with all matcher unit tests green (2026-09-21: the comparison target was caught as the wrapper PID so a match never
//   happened), leaving no error or log.
// ★Same shape and worth as the app-server partner (`a_resume_target_makes_the_handshake_issue_thread_resume`)★ — that one measures that the id received at the handshake reaches the sink; this sets up a real process tree and measures that the id
//   found via the lock holder reaches the same sink.
#[cfg(all(test, windows))]
mod terminal_capture_wiring {
    use super::*;
    use crate::backend::SessionIdSink;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// The process holding the lock must be a **grandchild** — the PID we hold is the `cmd.exe` wrapper; `cmd.exe /c powershell …` builds that shape (powershell opens the file and holds it).
    ///
    /// ★Do not shrink this value arbitrarily — it comes from the polling schedule★: recovery scans at 0.5 · 1.5 · 3.5 · 7.5 · 15.5 · **30.5** seconds (`FIRST_DELAY` · `MAX_DELAY` of `thread_lock`). On a 4-way runner a missed first hit at 15.5 s
    ///   leaves 30.5 s as the next chance, and a holder that has left by then turns the test red **depending on load**; it must hold until **after** 30.5 s.
    const HOLD_SECONDS: u32 = 45;
    /// A generous upper bound for the time it takes for the holder to come up and the first polling round to run — not a deadline but **the test's give-up line**.
    /// It must include the 30.5-second round above, so it is larger than that.
    const WAIT_LIMIT: Duration = Duration::from_secs(50);
    const LOCK_PATH_ENV: &str = "ENGRAM_TEST_LOCK_PATH";

    /// Waits until some process **currently holds** that lock — `false` if not.
    ///
    /// ★Do not measure by file existence (`exists()`) — the test turns **vainly green**★: a helper that creates the file and ends leaves it behind, so 「recovery did not run」 passes without 「the holder is standing」. A dead and a live lock are told apart only
    ///   by the holder lookup (measured for real by the platform crate's `file_holders` tests).
    /// ★The measuring means overlapping the test subject is intended★ — only the holder lookup is used here, none of the decisions (which is ours · is the name an id · where to write it), so the main assertion can still break independently.
    fn wait_for_live_holder(lock_path: &std::path::Path) -> bool {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let held = engram_dashboard_platform::file_holders::holders_of(lock_path)
                .map(|holders| !holders.is_empty())
                .unwrap_or(false);
            if held {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    fn recording_sink() -> (SessionIdSink, Arc<Mutex<Vec<String>>>) {
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let write = seen.clone();
        let sink: SessionIdSink = Arc::new(move |raw: &str| {
            write.lock().expect("sink poisoned").push(raw.to_string());
        });
        (sink, seen)
    }

    /// codex's own argv with the `cmd.exe /c codex …` wrapping stripped (this module is Windows-only, so there is only one branch).
    fn codex_argv(spec: &CommandSpec) -> Vec<String> {
        assert_eq!(spec.program, "cmd.exe");
        assert_eq!(spec.args[0], "/c");
        assert_eq!(spec.args[1], CODEX_PROGRAM);
        spec.args[2..].to_vec()
    }

    /// Terminal-mode codex — the only shape in which recovery runs.
    fn terminal_codex() -> AgentCommand {
        AgentCommand::Codex {
            extra_args: Vec::new(),
            output_format: AgentOutputFormat::Terminal,
        }
    }

    #[test]
    fn a_fresh_terminal_spawn_hands_the_captured_id_to_the_sink() {
        let thread_id = Uuid::new_v4();
        let codex_home = std::env::temp_dir().join(format!(
            "engram-capture-wiring-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let lock_dir = thread_lock::lock_dir_under(&codex_home);
        std::fs::create_dir_all(&lock_dir).expect("create the lock folder");
        let lock_path = lock_dir.join(format!("{thread_id}.lock"));

        // ★The holder must be below the wrapper, not the wrapper★ — `cmd.exe` launches powershell and **powershell**
        //   opens the file. An implementation that compares only the wrapper PID finds nothing here.
        let spec = CommandSpec {
            program: "cmd.exe".to_string(),
            args: vec![
                "/c".to_string(),
                "powershell".to_string(),
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                format!(
                    "$f=[IO.File]::Create($env:{LOCK_PATH_ENV}); Start-Sleep -Seconds {HOLD_SECONDS}"
                ),
            ],
            env: vec![
                // Recovery resolves the lock folder from **the child's** home — not ours.
                ("CODEX_HOME".to_string(), codex_home.display().to_string()),
                (
                    LOCK_PATH_ENV.to_string(),
                    lock_path.display().to_string(),
                ),
            ],
            cwd: std::env::temp_dir(),
        };

        let (sink, seen) = recording_sink();
        let parts = CodexBackend
            .open_spawn(
                &terminal_codex(),
                &spec,
                80,
                24,
                Some(sink),
                None,
                // ★`None` = a fresh incarnation★ — had it been a resume, recovery must not run at all (the sibling assertion below).
                None,
                None,
                None,
                &LaunchArtifacts::default(),
            )
            .expect("start the terminal link");

        let deadline = Instant::now() + WAIT_LIMIT;
        let mut got: Vec<String> = Vec::new();
        while Instant::now() < deadline {
            got = seen.lock().expect("sink poisoned").clone();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        parts.transport.shutdown();
        let _ = std::fs::remove_dir_all(&codex_home);

        assert_eq!(
            got,
            vec![thread_id.to_string()],
            "the thread id found via the lock holder did not come out to the sink it was handed — the lock file is {} and the holder is \
             not `cmd.exe` but the powershell below it (comparing only the wrapper PID comes up empty-handed here)",
            lock_path.display()
        );
    }

    /// ★A resume incarnation **does not start recovery at all**★ — the same wiring and real thing once more, filling only `resume_session_id` (ADR-0218 decision 5).
    ///
    /// ★**What this item cannot measure**★: it launches a hand-assembled command that creates a lock-holding grandchild, so **that spec's argv has no `resume <id>`** (launching the real codex with resume argv needs a live thread id and an installed
    ///   codex, which this bench lacks — why the sibling `production_spec` lane is `#[ignore]`).
    /// ★So the two halves are **tied with the same value in one item**★: with the same handle, `build_spec(Resume, Some(id))` first measures that argv emits `resume <id>`, then that `open_spawn` **does not run** recovery from that handle. That both halves
    ///   see the same value is guaranteed by the assembly point (`manager::resume_handle_for`).
    #[test]
    fn a_resume_terminal_spawn_captures_nothing() {
        let thread_id = Uuid::new_v4();
        // Half ①: the **real** argv launched with this handle is a resume.
        let resumed = CodexBackend.build_spec(
            &terminal_codex(),
            SpawnMode::Resume,
            None,
            Some(thread_id),
            std::env::temp_dir(),
            vec![],
            None,
            &LaunchArtifacts::default(),
        );
        let argv = codex_argv(&resumed);
        assert_eq!(
            &argv[..2],
            &[RESUME_SUBCOMMAND.to_string(), thread_id.to_string()],
            "if this handle does not resume via argv, the 「no recovery」 below has no ground: {argv:?}"
        );
        let codex_home = std::env::temp_dir().join(format!(
            "engram-capture-resume-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let lock_dir = thread_lock::lock_dir_under(&codex_home);
        std::fs::create_dir_all(&lock_dir).expect("create the lock folder");
        let lock_path = lock_dir.join(format!("{thread_id}.lock"));

        let spec = CommandSpec {
            program: "cmd.exe".to_string(),
            args: vec![
                "/c".to_string(),
                "powershell".to_string(),
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                format!(
                    "$f=[IO.File]::Create($env:{LOCK_PATH_ENV}); Start-Sleep -Seconds {HOLD_SECONDS}"
                ),
            ],
            env: vec![
                ("CODEX_HOME".to_string(), codex_home.display().to_string()),
                (LOCK_PATH_ENV.to_string(), lock_path.display().to_string()),
            ],
            cwd: std::env::temp_dir(),
        };

        // Half ②: a spawn launched holding the same handle does not even start recovery.
        let (sink, seen) = recording_sink();
        let parts = CodexBackend
            .open_spawn(
                &terminal_codex(),
                &spec,
                80,
                24,
                Some(sink),
                None,
                Some(thread_id),
                None,
                None,
                &LaunchArtifacts::default(),
            )
            .expect("start the terminal link");

        // ★First confirm that the holder is **alive and holding**★ — looking only at file existence would pass even when the helper created it
        //   and ended right away, making this item green without ever setting up the state it meant to measure.
        let holder_up = wait_for_live_holder(&lock_path);
        // Look after surely passing the first two rounds (0.5 · 1.5 seconds) that the sibling item catches.
        std::thread::sleep(Duration::from_secs(4));
        let got = seen.lock().expect("sink poisoned").clone();

        parts.transport.shutdown();
        let _ = std::fs::remove_dir_all(&codex_home);

        assert!(
            holder_up,
            "a live lock holder never stood — 「no recovery」 in this state has no ground: {}",
            lock_path.display()
        );
        assert!(
            got.is_empty(),
            "the resume incarnation ran recovery — it already resumed via argv and found and wrote it again: {got:?}"
        );
    }
}
