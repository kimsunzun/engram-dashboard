//! CodexBackend — produces the codex-CLI-specific CommandSpec.
//!
//! ★The rule this folder sets = codex knowledge lives only in here (ADR-0004)★. The source of truth for the rationale, the gates and what the gates
//! cannot see is the `backend/claude/mod.rs` header, and it is not repeated here — read it with the names swapped.
//! The surfaces going outward are only two: the [`crate::backend::AgentBackend`] implementation + the usage prober singleton
//! [`CODEX_USAGE_PROBE`] (why the prober is not a trait slot is in that header too) — the polling of session id recovery
//! also runs inside [`thread_lock`], and the only place that calls that module is this folder (ADR-0218 decision 11).
//!
//! ★The codex facts written here are measured (codex-cli 0.153.4, this PC, authenticated — rechecked 2026-09-08)★.
//! ★What was measured after that **carries its version in its own place**★ — do not read this one line as binding the whole file to one
//! version. Upstream updates itself, so the measuring time differs per item.
//! `tests/backend_contract.rs` **launches the argv that this file's `build_spec` emits as is**, so if an argument is
//! changed here, that lane meets the real codex with the changed argv — ★but that lane runs only when called, since it is `#[ignore]` (not CI)★.
//!
//! ★What the test bench **does not measure again** — the old sentence that said 「all measured」 was false (review finding 2026-09-08)★:
//!   1. The **model behaviour under** the `workspace-write` · `on-request` policies — the bench measures only that it comes up with that argv and that the composer
//!      stands up (to measure it, an agent with write permission would have to actually run in the automatic lane).
//!   2. Arguments we do not use (`-m`) — they are only in this file's comments and nothing measures them.
//!      ★The MCP `-c mcp_servers.…` override is now **used by us**★ — and it was measured by hand, not by the bench
//!      (2026-09-19 · 0.155.0 · a throwaway `CODEX_HOME`): `--strict-config` accepted those keys
//!      (two fake keys died with `unknown configuration field`), the app-server's `mcpServerStatus/list`
//!      returned `engram` together with that server's two tools (send · query), and the stub server
//!      received `Authorization: Bearer <ENGRAM_TOKEN value>`. The branch through `cmd.exe /c` was the same.
//!      ★The tool names shown on screen that day were `send_message` · `messages` — those names do not exist today★:
//!      `1b84045` renamed them to `eg_send` · `eg_messages` to cut the clash with agent built-in tools. What was measured
//!      stays valid (the rename is only names) and only the names to call changed — measured again now, that list
//!      shows `eg_send` · `eg_messages`.
//!   2-1. ★`codex resume <id>` is now **used by us** — and still the bench does not measure it for real★:
//!      [`production_spec`](../../../tests/backend_contract.rs) draws argv only with `SpawnMode::Fresh`, so
//!      the resume branch is never carried in that lane at all. To measure it automatically, a way to supply a live thread id
//!      to that file is needed first.
//!      ★But **the behaviour itself was measured by hand (2026-09-19 · 0.155.0)** — the old sentence that said
//!        「nobody looks even as far as it standing up」 is no longer true★: it actually came up with that argv, and **the session id is preserved**. A new
//!        session reports id S with `source:"startup"`, and on resume **the same S** comes again with `source:"resume"`
//!        (stable across repeated resumes), and the record is appended to S's original rollout file. So the recovery of a resumed
//!        session **lands on the same value** whether it overwrites or not, and there is no branch that drifts to a stale id.
//!        ★But this is an observation of the **hook channel** that carried the `source` field, and that channel was removed (ADR-0216)★. Today's recovery
//!        path has no such field — terminal mode uses the name of the writer lock the child holds ([`thread_lock`] ·
//!        ADR-0218), app-server mode uses the `thread/start` response — **so the id behaviour on resume is left unmeasured and
//!        overwriting was chosen on top of that.** If the two look inconsistent, this paragraph is the record of the old channel.
//!        ★Terminal-mode recovery does not run at all in a resume incarnation★ — that incarnation goes out carrying the id as argv,
//!        so there is nothing to recover (ADR-0218 decision 5).
//!   3. The `%VAR%` limit of `build_spec` below (the comment at that spot is the source of truth).
//!
//! If the capability declaration disagrees with that table, the bench's **non-`#[ignore]`** items go red.
//!
//! tauri import 0.

pub(crate) mod decoder;
pub(crate) mod protocol;
// ADR-0218
pub(crate) mod thread_lock;
pub(crate) mod transport;
mod usage;
mod usage_probe;

pub(crate) use usage_probe::CODEX_USAGE_PROBE;

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_platform::shell::console_command;
use uuid::Uuid;

use self::decoder::CodexAppServerDecoder;
use self::protocol::{
    AskForApproval, SandboxMode, ThreadOpen, ThreadResumeParams, ThreadStartParams,
};
use self::transport::CodexAppServerTransport;
use crate::backend::{
    inject_cli_entrance, AgentBackend, FirstTurnSink, InputEncoder, SessionIdSink, SpawnParts,
    TransportShape, TurnClassifier,
};
use crate::failure::AgentFailureKind;
use crate::profile::{AgentCommand, AgentOutputFormat, SpawnMode};
use crate::transport::pty::PtyTransport;
use crate::transport::{AgentTransport, LinkSink, OutputDecoder};
use crate::turn::{TurnEndKind, TurnSignal};
use crate::types::{
    AgentId, BackendCaps, CommandSpec, ControlEndpoint, DeliveryAck, MidTurnPolicy, ModelCaps,
    OutputEvent, PtyError, SessionCaps, TurnOutcome, MCP_SERVER_NAME, TOKEN_ENV,
};

/// Does it launch codex as a **resident JSON server** rather than an interactive TUI = the point where the four axes inside this folder (link shape · link object ·
/// input encoding · output decoder) split together.
///
/// ★Do not lift this predicate up into `profile` (ADR-0004)★: if it were there, codex's output-format axis would directly drive the shared layer's link
///   choice. What goes outward is only the value of the neutral axis.
fn is_app_server(command: &AgentCommand) -> bool {
    matches!(
        command,
        AgentCommand::Codex {
            output_format: AgentOutputFormat::StreamJson,
            ..
        }
    )
}

/// Picks this spawn's **second handshake request** — if `resume_session_id` is present it resumes that thread,
/// otherwise it opens a new thread. Absence = nothing to resume (no stored value, or launched as Fresh).
///
/// ★The three policies (working folder · approval · sandbox) are carried **identically** on both branches★: if left out of resume, codex runs with the values
///   it stored when it created that thread — even if the profile's working folder changed in between, only the resumed session
///   quietly trusts the old folder as its workspace. That mismatch leaves no sign at all on screen.
/// ★**Write the range of what is unverified precisely — 「does it accept a different cwd」 was already measured**★
///   (`docs/reference/backend-capabilities.md` §1 「if the cwd differs」): **it resumes.** Thread identity is
///   per `CODEX_HOME`, not per workspace, so it is not refused even when the cwd differs. So 「if refused, it shows on screen as a resume
///   failure」 was a sentence preparing for a branch that does not even exist.
///   ★What is really unverified is **whether the cwd we send overrides the recorded value**★ — the same measurement writes 「the `cwd` of the response is **the original
///   cwd recorded in the thread**」. That is, our value may be ignored, and then the mismatch the paragraph just above was meant to block
///   (only the resumed session trusts the old folder) **remains as is even when this argument is sent.** To measure it, resume the same thread with two
///   different folders and see on which side the turn actually runs — not measured.
///   ★We still keep carrying the argument★: if ignored there is no loss, and if applied the intended value stands.
/// ★Turning on `excludeTurns` is a decision★ — we do not read a single field of `thread.turns` (the response type
///   has no such field), and if it came filled, one long conversation would exceed the link's line cap (`MAX_LINE_BYTES`), **only that line
///   would be dropped**, and the handshake would wait until its deadline for an answer that never comes.
///   ★**The point of this item is that that outcome cannot be told apart from a refusal**★ — the peer is alive and sent
///   the answer on stdout, but we dropped that line, so all we see is one thing: 「the handshake failed on its deadline and
///   the child is fine」. That is **the same shape** as `thread/resume` being refused with a JSON-RPC error,
///   and if that shape does not close the link's stdin, the child · reader · writer are held as a whole and it becomes a wedge nobody
///   reaps (so that close does not tell branches apart — that spot in `transport.rs`).
///   ★That is, if upstream starts ignoring `excludeTurns`, this argument blocks nothing, and the only remaining defence is that
///   close★ — do not read this argument as 「blocked」. Whether upstream actually honours it is unverified.
/// ★`developer_instructions` is carried **only on the open branch**★ — [`ThreadResumeParams`] has no such field,
///   and leaving it absent is the decision (that type's doc is the source of truth for the reason). So a resumed app-server thread
///   gets no priming — terminal mode is argv, so it is carried on resume as well, and the gap is this one branch only.
/// ★The received value is carried **untouched**★ — the JSON body has no command-line cap, no `%` substitution and no newline truncation.
///   Bringing the terminal branch's conversion ([`developer_instructions_override`]) here would block no risk
///   and only break the document the agent reads.
// ADR-0185
// ADR-0215
fn thread_open(
    spec: &CommandSpec,
    resume_session_id: Option<Uuid>,
    developer_instructions: Option<String>,
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
        }),
        None => ThreadOpen::Start(ThreadStartParams {
            cwd,
            approval_policy,
            sandbox,
            developer_instructions,
        }),
    }
}

/// Launches the name as resolved through PATH (user decision 2026-09-07 · TRD §6-H).
///
/// ★Why we do not find the real binary and launch it directly★: on Windows, `codex` on PATH is the
/// `codex.cmd → node → codex.exe` chain, and the real binary at its end lives under **a version-stamped `node_modules` vendor
/// path** (measured). codex updates itself automatically, so that path changes at times we do not know —
/// hard-coding it dies on one update, and writing a search · fallback makes us reimplement that chain.
/// ★One more layer of shim does not change kill causality — but write the range of that guarantee precisely★:
/// **once enrolled** in the Job, breakaway is blocked (neither `BREAKAWAY_OK` nor `SILENT_BREAKAWAY_OK` is turned on),
/// so grandchildren · great-grandchildren born below it cannot leave the Job, and `TerminateJobObject` ends the tree as a whole
/// (the 2 verbs of ADR-0001).
/// ★Enrolment is **after** spawn, so the window in between is outside that guarantee★ — descendants born in that window do not enter the Job.
/// All three agent links (`pty` · `stdio` · the codex link) put it in after launching, and the existing teardown tests measure only the settled
/// state, so this window **has never been measured**. Fixing it is a separate matter touching all three links together, and the precedent is the usage query
/// runner — launch suspended, put it into the Job, then wake it (`usage::process` + OS layer `spawn::prepare_tree_root` ·
/// `spawn::TreeRoot`).
const CODEX_PROGRAM: &str = "codex";

/// The peer wording that means 「there is no record of that thread」 (lower-case comparison). ★Taken verbatim from a measured response★ —
/// `-32600` + `no rollout found for thread id`(`docs/reference/backend-capabilities.md` §1).
/// ★Why this wording, not the code, is the criterion★: the same code also comes for a config error · a duplicate `initialize` · resuming a
/// child thread. Splitting by code would stamp a sound handle with 「no conversation to resume」 on an unrelated failure.
const NO_ROLLOUT_MARKER: &str = "no rollout found";

/// The flag through which codex receives the working folder. ★Separate from the process cwd★ — `CommandSpec.cwd` is where we launch the
/// process, and this value is **which folder codex trusts and edits as its workspace**. We give both the same value,
/// but they are not the same field.
const CD_FLAG: &str = "--cd";

/// Sandbox mode — allows the commands the model emits **up to writing, inside the working folder**.
const SANDBOX_FLAG: &str = "-s";
const SANDBOX_WORKSPACE_WRITE: &str = "workspace-write";

/// Approval mode — asks a person only about actions that go outside the sandbox.
const APPROVAL_FLAG: &str = "-a";
const APPROVAL_ON_REQUEST: &str = "on-request";

/// The subcommand that resumes a stored session. ★It is not a flag — `--resume` · `--session-id` do not exist★
/// (measured). What it takes is a **positional argument**, and that place is the first position of `codex resume [OPTIONS] [SESSION_ID] [PROMPT]`
/// (measured 0.155.0 `codex resume --help`).
///
/// ★The three policies (`--cd` · `-s` · `-a`) are still parsed when placed **after** this subcommand★ — those three are also declared as `resume`'s own
///   options (same measurement). For the order of the positional argument and the options, both were seen to pass
///   (`codex resume <id> --cd . -s <bad value>` and the pair with the options in front both died with a **sandbox value error**
///   = parsing got as far as that point). So we put the id right after the subcommand, so that `codex resume <id>` reads
///   as one chunk.
/// ★The second positional argument is `[PROMPT]`★ — so if a **bare word that is not a flag** comes into `extra_args` below,
///   it is consumed as the first prompt. The Fresh branch (`codex [OPTIONS] [PROMPT]`) has the same property, so it is not a risk resume
///   newly brought in — the pass-through comment below is the source of truth for why it is not filtered.
const RESUME_SUBCOMMAND: &str = "resume";

/// The flag that overrides codex settings from the command line. ★It is the only means of getting settings without creating a single file on the user's machine★
/// — we write not a single character in the user home (`$CODEX_HOME/config.toml`) or in a wrapper script.
const CONFIG_OVERRIDE_FLAG: &str = "-c";

/// The env variable that overrides, per spawn, the value codex writes verbatim into the `originator` field of the session record.
///
/// ★It is an **internal** variable upstream does not document★ — if it disappears, it is a silent ignore, not an error, and what remains then is
///   one thing: 「a record without a marker」 (ADR-0217 「three unverified items」 ③). That the value stays in the record as is was measured
///   (0.155.1 · 3 arbitrary strings · no allowlist refusal — ADR-0217 Probe B).
/// ★This value is visible to the child and its descendants — the result of measuring and leaving it as is★: what it holds is the
///   **identifier** `(agent_id, epoch)`, not a value that grants permission. What grants permission is the bearer token of
///   [`TOKEN_ENV`] already carried in the same env, and that token cannot be made from knowing this pair. Moreover this
///   value is planted **precisely to remain outside our process** (the `originator` field of the vendor record). So it was
///   not removed. ★One condition under which this reasoning dies — the day the agent id itself becomes a key that opens something★:
///   then revisit that key design, not this spot.
// ADR-0217
const ORIGINATOR_ENV: &str = "CODEX_INTERNAL_ORIGINATOR_OVERRIDE";

/// The key prefix of the config override that attaches the daemon MCP server to this spawn. The server's logical name is appended to make
/// the single key `mcp_servers.engram` (source of truth = [`MCP_SERVER_NAME`] — do not type the name again here).
///
/// ★The mechanism **differs** from claude's `--mcp-config <file>`★ — codex does not take a file and takes only this override
///   (measured 0.155.0). So the same endpoint is translated into different syntax in the two backends
///   (exactly the point ADR-0004 talks about).
const MCP_SERVER_OVERRIDE_PREFIX: &str = "mcp_servers.";

/// The field in the MCP server config that takes **not the value of the bearer token but the name of the env variable holding it**.
///
/// ★Picking this field is a decision — do not revert to an inline value (`bearer_token='…'`)★: doing so
///   **embeds the token in the command line**, and it leaks just by any process of the same user reading argv (Windows'
///   process list · WMI give argv — the very surface this repository uses for daemon discovery). With this field the token
///   is only the one copy already in the spawn env ([`inject_cli_entrance`] plants it as [`TOKEN_ENV`]), and no copy appears in argv
///   or on disk.
/// ★We saw codex actually read that env and carry it as a header (measured 2026-09-19 · 0.155.0)★ — the stub MCP server
///   received `Authorization: Bearer <that value>` on all requests from `initialize` · `tools/list` on.
const MCP_BEARER_ENV_KEY: &str = "bearer_token_env_var";

/// The field that removes the tool-call approval prompt, limited to this one MCP server entry.
///
/// The valid values are four: `auto | prompt | writes | approve` — ★`auto` does not remove the prompt.
///   Only `approve` removes it★ (measured 2026-09-20). Confusing these two once led to a wrong conclusion,
///   so both names are pinned here. It was confirmed that `approve` removes approval on all three paths:
///   the interactive TUI · `codex exec` (without this field it simply fails with
///   "MCP tool call requires approval, but approval policy is never") ·
///   `codex app-server --stdio` (without this field the gate comes as `mcpServer/elicitation/request`).
/// ★It applies only to this one server entry — approval was not turned off globally★: `approvalPolicy: on-request` ·
///   `sandbox: workspace-write` are untouched, so ordinary shell/exec approvals are still asked.
/// ★The dialog's "Always allow" does not persist to disk★ (measured by comparing before/after snapshots with an empty `CODEX_HOME`
///   — a new process asks again). So this value cannot be replaced by setting it once and
///   must be carried on every spawn.
const MCP_APPROVAL_MODE_KEY: &str = "default_tools_approval_mode";
const MCP_APPROVAL_MODE_VALUE: &str = "approve";

/// The config key that **appends** the priming brief **after** codex's default instructions.
///
/// ★Do not swap it with `model_instructions_file` — that one **replaces** rather than appends (measured 0.155.0)★:
///   it is the only instruction key that takes a path, but hanging the priming there makes codex's own operating instructions disappear
///   entirely.
/// ★This key has **no** `*_file` counterpart that takes a path★ — `developer_instructions_file` ·
///   `experimental_instructions_file` · `instructions_file` are all `unknown configuration field`
///   (same measurement). So unlike claude, this backend carries the **content**, not a path, and **reads** the brief file
///   **here**. That split is the per-backend knowledge ADR-0004 talks about.
/// ★Authority is the same layer★ — per the OpenAI docs, the `instructions` parameter and a `developer` role message are the same grade,
///   so the price of picking this field instead of a path is not 「instruction strength」.
// ADR-0004
// ADR-0215
const DEVELOPER_INSTRUCTIONS_KEY: &str = "developer_instructions";

/// The two characters (backslash + `n`) that stand in for a newline when carried on the command line.
///
/// ★If even one real LF remains, `cmd` **cuts the command line at that spot without any error**★
///   (measured) — codex comes up with the following arguments gone entirely. That outcome is not 「no priming」 but
///   「no MCP attachment and no hook registration」, so the silent cut is the most expensive failure mode of this value.
/// ★Replacing it with two characters is the measured choice★ — the model read this form as a newline as is (same measurement).
///   A TOML literal string has no escapes, so these two characters pass the parser and arrive **literally**.
// ADR-0215
const NEWLINE_REPLACEMENT: &str = "\\n";

/// The total command-line length `cmd.exe` accepts (UTF-16 code units).
///
/// ★This budget is not ours alone★ — the policy flags · pass-through · the two overrides share the same line.
///   So the decision is not a fixed-constant comparison but **the remainder after counting the already-assembled arguments**
///   ([`command_line_cost`]).
// ADR-0215
const CMD_LINE_LIMIT_UTF16: usize = 8191;

/// The length share of the wrapping (`cmd.exe /c codex`) that `console_command` adds on Windows.
const CMD_WRAPPER_COST: usize = "cmd.exe /c ".len() + CODEX_PROGRAM.len();

/// A **conservative** estimate of the command-line length the already-assembled arguments used (UTF-16 code units).
///
/// ★It deliberately counts generously instead of reproducing the quoting rules★ — for every argument it unconditionally adds one space and two
///   enclosing quotes. Counting exactly would require reimplementing **both** portable-pty's quoting rules and `cmd`'s re-parsing, and that
///   reimplementation quietly goes stale when upstream changes. Over-counting and skipping the priming is cheaper than under-counting and having
///   the command line cut — a cut erases the trailing arguments without an error.
/// ★The wrapping share is added regardless of platform★ — the cap applies only on Windows, but if the decision were split
///   by platform, the same brief would be carried on some machines and not on others. It is brought down to one value.
/// ★Counting in UTF-16 is intended★ — the Windows command line is UTF-16 and the cap is in that unit too. Counting bytes
///   overestimates for a non-ASCII brief, and counting `chars()` underestimates for characters outside the BMP.
// ADR-0215
fn command_line_cost(args: &[String]) -> usize {
    CMD_WRAPPER_COST
        + args
            .iter()
            .map(|a| a.encode_utf16().count() + 3)
            .sum::<usize>()
}

/// The **content** of the priming brief to carry on this spawn. `None` = nothing to carry (path absent · read failure · empty file).
///
/// ★A read failure is fail-open★ — only a warning is left and `None` is returned. Priming is nice to have and
///   the spawn is required (ADR-0216 inherited as a general rule 「never fail the spawn on any branch」).
/// ★That **this crate** reads the content is where it splits from the claude branch★ — that side passes a path and claude
///   reads it directly (ADR-0092). codex has no key that takes a path ([`DEVELOPER_INSTRUCTIONS_KEY`]), so that
///   option does not exist. That this read is inside the backend folder is the point of ADR-0004 — the daemon's priming
///   provider still handles **only the path**.
/// ★An empty file is `None` too★ — carrying empty instructions only spends command line and teaches nothing.
// ADR-0004
// ADR-0092
// ADR-0215
// ADR-0216
fn priming_text(control: Option<&ControlEndpoint>) -> Option<String> {
    let path = control?.priming_file.as_deref()?;
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => {
            tracing::warn!(
                "codex priming not injected — the brief file is empty: {}",
                path.display()
            );
            None
        }
        Ok(text) => Some(text),
        Err(e) => {
            tracing::warn!(
                "codex priming not injected — could not read the brief file ({}): {e}",
                path.display()
            );
            None
        }
    }
}

/// The `-c developer_instructions='…'` value to carry on a terminal-mode spawn. `None` = not carried — ★only a warning is left and
/// the spawn goes ahead★ (the same discipline as ADR-0216's 「never fail the spawn on any branch」).
///
/// `args_so_far` = the arguments already assembled before this value. Used only for the command-line budget decision.
///
/// ★Wrapped as a TOML literal string (single quotes)★ — there are no escapes, so the value crosses byte for byte, and that combination
///   was measured to survive the two layers `cmd.exe /c` + the `.cmd` shim (ADR-0210). In exchange, a single quote itself cannot
///   be held.
/// ★On meeting a character that cannot be carried, it skips instead of pushing it in with an invented escape★ — the four characters each break a different layer:
///   the single quote breaks the TOML literal; the double quote and backslash break the **command-line quoting** outside it (portable-pty
///   escapes as `\"`, but `cmd` does not know that rule and only counts quotes); `%` breaks cmd's environment variable
///   substitution (**expanded even inside quotes** — the same-named limit comment of `build_spec` is the source of truth).
/// ★Do not remove this guard because it never trips today★ — the current brief has zero of those four, but this document is
///   markdown people edit, and one English contraction (`don't`) brings in a single quote.
/// ★Control characters including tab are cut too★ — only the newline is replaced with [`NEWLINE_REPLACEMENT`] above; for the rest, we have never verified
///   `cmd`'s token splitting and quoting layers. The symptoms are all 「a silently skewed command line」, so we choose not to
///   carry it.
/// ★The guard looks at **the original before replacement**★ — doing it after would make the backslashes we inserted trip our own guard.
// ADR-0215
// ADR-0216
fn developer_instructions_override(text: &str, args_so_far: &[String]) -> Option<String> {
    if let Some(bad) = text.chars().find(|c| {
        matches!(c, '\'' | '"' | '%' | '\\') || (c.is_control() && *c != '\n' && *c != '\r')
    }) {
        tracing::warn!(
            "codex priming not injected — the brief has a character that cannot be carried on the command line ({bad:?})"
        );
        return None;
    }
    let single_line = text
        .replace("\r\n", NEWLINE_REPLACEMENT)
        .replace('\n', NEWLINE_REPLACEMENT)
        .replace('\r', NEWLINE_REPLACEMENT);
    let value = format!("{DEVELOPER_INSTRUCTIONS_KEY}='{single_line}'");
    // The share taken by `-c` and the value itself — counted with the same arithmetic as [`command_line_cost`] above.
    let cost = CONFIG_OVERRIDE_FLAG.encode_utf16().count() + 3 + value.encode_utf16().count() + 3;
    let used = command_line_cost(args_so_far);
    if used + cost > CMD_LINE_LIMIT_UTF16 {
        tracing::warn!(
            "codex priming not injected — exceeds the command-line budget (already {used}, needs {cost} more, cap {CMD_LINE_LIMIT_UTF16})"
        );
        return None;
    }
    Some(value)
}

/// The **single decision** of whether to attach the daemon MCP server to this spawn. [`AgentBackend::build_spec`] (the actual attachment) and
/// [`AgentBackend::precheck_control_endpoint`] (the fail-closed gate) read **this same value**.
///
/// ★If the two places each decided, the gate would become hollow★ — a spawn the gate passed as 「can attach」 could
///   come up without an attachment in the assembly with no signal. So there is only one decision, [`mcp_attachment`], and this
///   enum carries its result to the two reading places in the same shape.
enum McpAttachment {
    /// Attach — the value to carry on `-c`.
    Attach(String),
    /// Do not attach, and **that is the normal state**. Two branches fall here — a spawn with no control channel at all
    /// (outside the mail plane), and a spawn for which the daemon has **not authorized** the MCP send entrance (the operator turned off MCP
    /// mail — [`ControlEndpoint::grants_mcp_send`]). Neither blocks the spawn.
    NotWanted,
    /// Should attach but **cannot build it** (reason). ★Only this value produces fail-closed★ — if it simply came up in this state,
    /// the agent would sit on the receiving roster (`reads_messages`) with zero send entrances (MCP is not attached and the CLI
    /// mirror is closed by `mail_allowed=false`), and every delivered request would become a contract nobody can answer.
    Unrepresentable(String),
}

/// The only place that makes the decision above.
///
/// ★Environment variables are not read again here (ADR-0004)★: the owner of the knobs that turn off MCP mail (`ENGRAM_DISALLOW_MCP_SEND` ·
///   the harness's `ENGRAM_FORCE_CLI_ONLY_SEND`) is **the daemon**, and that decision arrives already digested
///   via [`ControlEndpoint::grants_mcp_send`]. If the backend read the same variables again there would be two authorities,
///   and on the day they split, 「claude is off but only codex is on」 would happen silently (that is the finding that
///   created this branch).
///
/// ★This branch has no plaintext token file — but this function is not what guarantees it★: that the token
///
/// ★This branch has no plaintext token file — but this function is not what guarantees it★: that the token
///   is not embedded in argv here is kept by [`MCP_BEARER_ENV_KEY`] above, and that it **is not written to disk either** is
///   kept by [`AgentBackend::writes_mcp_config_file`] being false (the daemon's `control::provision` decides
///   `mcp_config::write_config`/`write_settings` by that one field — the content and lifetime of that file are in
///   `control/mcp_config.rs`). ★This spot once said 「this branch has no such file in the first place」, and that was
///   false★ — back then the daemon looked at `accepts_mcp_config` and actually wrote that file on every codex spawn.
///   That this function does not create a file and that no file appears for that spawn are different statements.
/// ★A TOML literal string (single quotes) is used★ — there are no escapes, so the value crosses **byte for byte**, and that
///   combination was already measured to survive the two layers `cmd.exe /c` + the `.cmd` shim (ADR-0210). In exchange, a single quote
///   itself cannot be held.
/// ★On meeting a character that cannot be carried, it stops instead of pushing it in with an invented escape★ — a single quote · whitespace ·
///   control characters break the value or the TOML line itself, and `%` is expanded by cmd as an environment variable on the command line **even inside
///   quotes** (the same-named limit comment of `build_spec` below is the source of truth), making it point to a different address. All three have one
///   symptom, 「mail silently does not work」, so we make noise here.
///   ★It is normal that this check never trips today★ — this url is `http://127.0.0.1:<port>/mcp` authored by the daemon,
///   not user input. The check is for the day that form changes.
// ADR-0004
// ADR-0128
// ADR-0209
fn mcp_attachment(control: Option<&ControlEndpoint>) -> McpAttachment {
    let Some(endpoint) = control else {
        return McpAttachment::NotWanted;
    };
    // ★This one line is all that makes 「the operator turned off MCP mail」 reach this backend★ — previously it looked only at the
    //   **existence** of the control channel and attached unconditionally, so an operator who turned on the knob got a state where claude was off and codex
    //   stayed on (`eg_send` still callable with automatic approval).
    if !endpoint.grants_mcp_send() {
        return McpAttachment::NotWanted;
    }
    let url = endpoint.url.as_str();
    if let Some(bad) = url
        .chars()
        .find(|c| *c == '\'' || *c == '%' || c.is_control() || c.is_whitespace())
    {
        return McpAttachment::Unrepresentable(format!(
            "cannot attach the codex MCP server — the control channel address has a character that cannot be carried ({bad:?}): {url}"
        ));
    }
    McpAttachment::Attach(format!(
        "{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}={{url='{url}',{MCP_BEARER_ENV_KEY}='{TOKEN_ENV}',{MCP_APPROVAL_MODE_KEY}='{MCP_APPROVAL_MODE_VALUE}'}}"
    ))
}

/// The subcommand that launches it as a resident JSON server, and its transport choice (measured 0.154.0 — `--stdio` is the same as `--listen stdio://`
/// and it is the default. We state it explicitly instead of relying on the default: this link does not hold unless it is stdio,
/// and the default can be changed by upstream, and if it changes our argv stays silently the same).
const APP_SERVER_SUBCOMMAND: &str = "app-server";
const APP_SERVER_STDIO_FLAG: &str = "--stdio";

/// Does the caller pass-through set **the same config key as ours**? `true` = our override overrides it (because it is carried
/// later) — so that a value the user set on purpose is not silently erased, a warning is given at that spot.
///
/// ★Only one shape is caught = the form where the **next slot** after `-c` starts with `key`★ (`-c developer_instructions=…`).
/// ★What it does not catch — a known hole★: the form with `-c` and the value joined in one word · the same flag under a long name
///   instead of `-c` · a higher key that overrides a whole table (`-c tui=…` · `-c mcp_servers=…`) · another key that **only starts**
///   with that key. Do not try to make it 「certain」 by growing this list — it becomes a reimplementation of the codex override syntax, and that reimplementation
///   quietly goes stale every time upstream changes. What is lost by missing one is **only the warning, not behaviour** — that ours wins
///   is guaranteed separately by the order in `build_spec` below.
/// ★Taking the key as an argument is intended★ — there are several overrides (MCP server attachment · the brief), and duplicating the decision per key
///   leads to only one side getting fixed and the warning becoming half.
fn passthrough_overrides_key(extra_args: &[String], key: &str) -> bool {
    extra_args
        .windows(2)
        .any(|pair| pair[0] == CONFIG_OVERRIDE_FLAG && pair[1].starts_with(key))
}

/// The string if it has a shape that can be carried as the positional argument of `codex resume`, otherwise `None`.
///
/// ★Why the gate is needed — this argument **does not fail even when wrong**★ (measured 2026-09-21): when codex receives a string
///   that cannot be read as a thread id, it reinterprets it as a **session name**, **quietly creates a new session**,
///   ends with exit code `0`, and the turn is billed. That is, the price of a wrong value is not 「an error」 but 「a new conversation
///   the user believes they resumed」. The failure mode ADR-0218 set is 「not received」, not 「the wrong conversation」,
///   so for a value that cannot be carried, the subcommand itself is not emitted (it falls to new-conversation argv, and that regression
///   is reported by `opens_a_new_conversation` at the assembly point).
/// ★Can that reinterpretation path be reached today — the type already blocks half★: this field is `Uuid`, so a non-uuid
///   string cannot get this far, and profile deserialization refuses it before that too. So what this function actually
///   filters is one thing, **nil** (all zeros) — a value coming from a hand-edited profile or an initialization mistake, and that value
///   has the uuid shape, so it passes the type gate.
/// ★Do not merge it with claude★ — that side does not have this failure mode (`--resume` fails on an unknown value).
///   Lifting it to a shared place would make one backend's upstream bug an assembly rule for everyone (ADR-0004).
/// ★This function is **not** only for argv assembly — the session id recovery gate also looks at it★
///   (the terminal branch of [`AgentBackend::open_spawn`] · [`thread_lock::plan_capture`]). Fixing only one side
///   creates the combination 「argv is a new conversation but recovery is off」, and that incarnation's id is never written.
// ADR-0218
fn resume_argument(thread_id: Uuid) -> Option<String> {
    (!thread_id.is_nil()).then(|| thread_id.hyphenated().to_string())
}

/// The **diagnostic** marker left in the session record this incarnation creates (the value of [`ORIGINATOR_ENV`]).
///
/// ★There is no reader — do not write the contract 「recompute the same value and compare」 here again★ (ADR-0218
///   decision 8): recovery is carried by the lock holder ([`thread_lock`]) and does not lean on this marker. Writing a comparison contract
///   makes a promise that cannot be kept — the marker contains the incarnation marker, so **the next incarnation computes a different value**,
///   and searching old records with that value yields 0 hits without an error.
/// Properties:
/// - Invariant within one incarnation, and a different value when the incarnation changes (that distinction is `epoch`'s job).
/// - The returned string is only `[a-z0-9-]` — any other character is replaced with one `-` per character.
///
/// ★Today `agent_id` is a uuid, so it is normal that the substitution never runs — still, do not remove it★: this value
///   remains outside our process (the vendor record), and which characters that field accepts is upstream's business. The substitution closes that
///   unknown on our side.
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
    /// ★The caller cannot decide the session id (measured)★ — codex has no `--session-id`-like flag and the id is
    /// issued by codex itself. If set to true, the manager issues our uuid and **persists it in the profile**, but codex
    /// never once uses that value — after that, the resume decision stands on that fake value and believes an incarnation that is actually
    /// a new conversation was resumed.
    /// ★Do not turn it on for app-server mode — both shapes are false★: the issuer of that mode's identifier (thread id) is
    ///   codex too. Whether that id **can be resumed** is answered by the separate axis below.
    // ADR-0185
    fn assigns_session_id(&self, _command: &AgentCommand) -> bool {
        false
    }

    /// ★Both modes resume — only the means differ★: app-server emits `thread/resume` with the stored thread id
    /// ([`AgentBackend::open_spawn`] picks it), and terminal mode launches carrying the same id as a subcommand + positional argument
    /// ([`RESUME_SUBCOMMAND`] · the terminal branch of [`AgentBackend::build_spec`]).
    /// ★So this field does not look at [`is_app_server`] — do not revert it★: what this predicate asks is
    ///   **whether it can resume with the stored sid**, not through which link it resumes. Splitting by link would make codex launched in terminal
    ///   mode get launched as Fresh by the activation entrance even with a handle in the registry, and the id that was recovered and recorded
    ///   ([`crate::profile::ProfileRegistry::observe_session_id`]) would never be used (ADR-0217).
    /// ★When there is **no** handle, this field does not answer★ — that decision is made by [`crate::backend::can_resume_profile`],
    ///   which looks at it together with the existence of a stored sid, and a spawn that still comes in as Resume is
    ///   dropped to new-conversation argv by [`AgentBackend::build_spec`] (the doc at that spot).
    /// ★Turn it on **together with the same predicate as `session.resume` of [`AgentBackend::capabilities`] below — do not touch
    ///   only one side**★: turning either on alone reports a new thread that was never resumed as 「resumed」,
    ///   and turning it off alone reports a spawn that actually resumes as 「new conversation」.
    // ADR-0185
    // ADR-0208/ADR-0210
    fn can_resume_stored_session(&self, _command: &AgentCommand) -> bool {
        true
    }

    /// ★It is true but [`AgentBackend::accepts_mcp_config`] is false — the two axes are separate (ADR-0133)★: what this field
    /// asks is **does it consume the control channel**, and the means of that consumption is not only MCP but also the CLI entrance (credential env).
    /// What the field below asks is only one of them: **can the mcp-config file be fed**.
    /// ★Reverting it to false makes `ENGRAM_TOKEN` · `ENGRAM_CONTROL_URL` disappear entirely from the env of codex spawns★
    ///   — the assembly point looks at this field and skips provisioning itself (that spot of [`crate::manager::AgentManager`]),
    ///   so the endpoint comes as `None`, and not a single line of the injection in `build_spec` below runs.
    /// ★Once, the consumers of this env included **our hook process** — today there is none★ (ADR-0216 removed the registration).
    ///   What was measured then stays: codex **overlaid none** of its own env on the hook process (pure inheritance —
    ///   measured 2026-09-18, 0.155.0). The living consumers are the shell tools codex launches and MCP mail, and the condition the shared
    ///   injection wrote as 「it is also the credential of helper processes」 is carried by the [`inject_cli_entrance`] doc.
    /// ★This field **does not open mail — and that was not free**★: the old daemon derived whether mail is allowed
    ///   from `!accepts_mcp_config` alone, so just turning this field on **also opened send authorization**
    ///   for this backend (with receiving closed by [`AgentBackend::reads_messages`]). To remove that asymmetry, the send axis
    ///   was pulled out as a separate declaration — that is [`AgentBackend::uses_mail`] below, and the daemon derives one value from the two
    ///   axes. ★Removing that field or reverting it to the default brings this side effect straight back★.
    ///   (Today that field is on — but it is on **by declaration**, not as a side effect.)
    /// ★Even in an install without the `engram` executable, this backend's spawn **is not cut**★ — that fail-closed
    ///   guards the pair violation 「CLI mail was taught but there is no executable to call」, and a spawn outside the mail plane
    ///   has no such pair (source of truth for that decision = the daemon's `control::provision`). Losing the control verbs remains, but
    ///   that side is fail-open + warn.
    // ADR-0086
    // ADR-0132
    // ADR-0133
    // ADR-0208
    // ADR-0209
    fn supports_control_channel(&self) -> bool {
        true
    }

    /// ★The **name of this field and its meaning today disagree — turned on knowingly**★: what the name and the [`crate::types::ControlChannelNeeds`]
    /// doc ask is 「can the mcp-config **file** we made be fed」, and codex's answer is still
    /// **no** (there is no flag to point to that file — measured 0.155.0). But the daemon derives even the **mail
    /// channel decision** from this one field (`mail_allowed = uses_mail && !accepts_mcp_config`), so if this backend, which uses MCP for mail,
    /// kept false, **the CLI mirror would stay open** — the exact opposite of what ADR-0209 nailed down as 「MCP is the official one, CLI is
    /// the mirror」.
    /// ★Once, turning this field on **brought a file along too, and that cost was paid off**★: looking at this one field, the daemon actually wrote the mcp-config
    ///   JSON (plaintext Bearer token) and the session settings fragment, and codex reads neither, so on every spawn
    ///   a secret file nobody opens appeared — moreover that write is fail-closed, so **a codex spawn could be cut because of a file
    ///   that is not read**. It was resolved by splitting the daemon's derived axis in two: whether to write the file is now decided by
    ///   [`AgentBackend::writes_mcp_config_file`] below alone, and this field drives only the mail channel decision.
    ///   ★Removing that field or deriving it from the value here brings that cost straight back★.
    /// ★The actual means of attachment is [`mcp_server_override`]★ — the single value `-c mcp_servers.engram={…}`. That value was seen to actually
    ///   set up the server (measured 2026-09-19 · 0.155.0: `mcpServerStatus/list` returned `engram` together with that
    ///   server's two tools, and the stub server received `Authorization: Bearer …`).
    ///   ★The names shown that day were `send_message` · `messages`, and `1b84045` renamed them to `eg_send` · `eg_messages`★
    ///   — the same measured item in this file's header is the source of truth for the reason.
    // ADR-0099
    // ADR-0128
    // ADR-0209
    fn accepts_mcp_config(&self) -> bool {
        true
    }

    /// ★The field above is true but this field is false — that split is why this field exists★: codex has **no** flag to point to a file we wrote
    /// (measured 0.155.0 — there is neither `--mcp-config` nor `--settings`, nor a `--config <file>` that points to a settings
    /// file). MCP attachment is the single value `-c mcp_servers.engram={…}` of [`mcp_server_override`] alone,
    /// and the token goes to the env that value points to by name ([`MCP_BEARER_ENV_KEY`]) — there is no place at all to make
    /// a copy on disk.
    /// ★Keeping it false is a security decision — do not derive it from the field above★: if reverted to true, the daemon
    ///   writes a plaintext Bearer token JSON with the default ACL to the data directory on every spawn (nobody opens it), and that
    ///   write failure is **fail-closed**, so the codex spawn is cut because of that file. That was the actual state before this field
    ///   was split off.
    // ADR-0086
    // ADR-0099
    // ADR-0209
    fn writes_mcp_config_file(&self) -> bool {
        false
    }

    /// ★It was once false — leave that reason and the reason it was dropped **together** (do not remove)★: the basis of the old value was
    /// 「this method does not receive `command`, so it cannot tell the two modes apart」. app-server mode emits turn signals
    /// — the link reads `turn/completed` (`turn/started` is **deliberately** not read; the source of truth for the reason is that
    /// link's `TURN_COMPLETED` doc), the translator carries that notification over as a turn boundary, and [`classify_turn`] below emits
    /// `Ended`. But terminal mode has no decoder, so only `TerminalBytes` flow and the signal is 0, so
    /// returning true here **opens that mode too** — so it was kept closed.
    /// ★That reason is no ground for blocking — terminal claude **sits in exactly the same place and already receives**★:
    ///   claude without structured output also has zero turn signals but is on the receiver roster. The policy source of truth is **ADR-0116
    ///   decision 7**, and that sentence is 「no turn signal → inject immediately without a gate」 (that CLI's own input queue is
    ///   the gate, so there is no reason for us to observe idle). The rejected alternative of the same ADR **explicitly** killed 「it cannot be observed,
    ///   so it cannot be delivered」 — do not go back to that premise.
    /// ★So the cost is accepted **knowingly**★: terminal mode has no busy gate, so an envelope can land in the middle of a
    ///   turn, and if the TUI is in a modal state (approval prompt · menu · plan confirmation), that widget can eat the envelope.
    ///   ADR-0116 decision 7 already explicitly accepted that cost (narrow it when the problem is actually observed).
    /// ★Do not open it like shell★ — that side's false is not the observation axis but the axis 「**input is executed as a command**」, so
    ///   it stays as is regardless of this change (that paragraph of the default implementation's doc).
    // ADR-0116
    // ADR-0209
    fn reads_messages(&self) -> bool {
        true
    }

    /// ★Write precisely **what this field buys**★: the daemon's `build_grants` **short-circuits without emitting a single line** if this value is
    /// false (send entrance grants 0), and the priming variant choice is also cut by `needs.uses_mail.then_some(…)`.
    /// That is, if only `accepts_mcp_config` above is turned on with this field closed, the agent ends up **holding the MCP tools
    /// with nobody telling it they exist**.
    /// ★It is separate from the control verbs★ — what this field opens is mail; the CLI entrance (token · address) and the control routes are opened by
    ///   `supports_control_channel` above.
    /// ★This spot once said 「unresolved — only sending was opened」. It is resolved★: the first of the two grounds that paragraph
    ///   demanded (the receiving axis is opened) now stands — [`AgentBackend::reads_messages`] is true, so this backend is on the
    ///   delivery roster of the daemon's `messaging_host`, and the reply to a `request` this agent sent has somewhere to
    ///   come back to. So the 「receive first」 order of ADR-0209 decision 4 is no longer violated.
    ///   ★The rule for reverting stays the same★ — closing the receiving axis while leaving only this field open revives that asymmetry.
    // ADR-0133
    // ADR-0209
    fn uses_mail(&self) -> bool {
        true
    }

    /// ★Only app-server mode has a connection to set up★ — `turn/start` is allowed only after the handshake round trip finishes.
    /// Terminal mode is a PTY, usable from the moment the process comes up, so it has no such axis (same as claude · shell).
    /// ★This declaration and the actual delivery **must not disagree**★ — if true but not called, the supervisor waits until the backstop,
    ///   and if false but called, nobody receives that delivery. The test bench measures that pair.
    // ADR-0004
    fn declares_link(&self, command: &AgentCommand) -> bool {
        is_app_server(command)
    }

    /// Why did the resume fail — ★this backend's evidence comes as **the link's connection reason**★.
    ///
    /// ★That the reason is in neither of the two tails is why this implementation exists★: a resume refusal is a JSON-RPC error on
    ///   stdout, so the console tail (`terminal_tail`) does not exist on this link at all, and the diagnostic tail
    ///   (`diagnostic_tail`) holds only stderr. So the decision is
    ///   passes the `reason` of [`crate::transport::LinkState::Down`] as evidence — without the declaration, even if that string
    ///   comes the classification falls to `None`, and the **wrong context default** 「early exit right after resume」 is stamped
    ///   (this branch has no early exit. The process was alive and well).
    ///
    /// ★**Do not tell apart by error code — `-32600` does not have one meaning**★ (measured 0.154.0): an unknown thread,
    ///   a second `initialize`, a config error, and **an attempt to resume a child thread directly** all come with that code.
    ///   Deriving 「unknown thread」 from the code would give a sound handle the wrong classification on an unrelated failure.
    ///   So only the **message** is looked at.
    /// ★The declared wording is **the one that was measured**★ — `no rollout found for thread id`
    ///   (`docs/reference/backend-capabilities.md` §1 「when given an unknown id」). For the other reasons we do not have the actual response
    ///   wording, so **we do not invent it** — it falls to `None` and the caller uses the context
    ///   default, and the human-readable reason goes out in the outcome as the `reason` string anyway.
    ///   ★In particular the 「a child thread must resume its parent first」 branch has unknown wording★ — putting in a string guessed before
    ///   measuring the real thing leaves a non-matching match quietly as dead code.
    /// ★It is called on a live session too (the trait doc's condition)★ — the wording above is emitted by the peer **only when refusing a request**,
    ///   not words that can mix into the conversation body, so it satisfies that condition.
    // ADR-0004
    // ADR-0082
    // ADR-0172
    fn resume_failure_kind(&self, evidence: &str) -> Option<AgentFailureKind> {
        if evidence.to_lowercase().contains(NO_ROLLOUT_MARKER) {
            return Some(AgentFailureKind::NoConversationToResume);
        }
        None
    }

    /// ★Fail-closed is set up in the same place as claude★ — on that side the daemon's `?` (mcp-config write failure)
    /// cuts it, but this backend does not write the file (`writes_mcp_config_file` = false), so the daemon has nothing
    /// to cut with. Failing to build the attachment value in assembly is visible **only here**.
    ///
    /// ★The cut condition is [`McpAttachment::Unrepresentable`] alone — do not widen it★: the state at that time is
    ///   「on the delivery roster (`reads_messages` = true) but zero send entrances」. MCP is not attached, and
    ///   the daemon closed the CLI mirror with `mail_allowed=false` (this backend has `accepts_mcp_config`
    ///   true, so that derivation always falls that way) — then every delivered `request` becomes **a contract nobody can
    ///   answer**, and the symptom is not an error but eternal silence.
    /// ★The two branches left open (both are normal states)★: ① a spawn with no control channel at all — a codex coming up without mail is
    ///   legitimate ② a spawn for which the daemon did not authorize the MCP send entrance — it means the operator turned off MCP mail,
    ///   and that spawn does not expect mail in the first place. Cutting both would kill codex spawns entirely in installs running without `engram`
    ///   and in harnesses with the knob on.
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

    /// ★`session_id` is **still** not assembled — do not revive it★: `--session-id` does not exist
    /// ([`AgentBackend::assigns_session_id`] is false, so this field is always `None`), and carrying that value in argv
    /// would push a uuid codex never once uses onto the command line.
    /// ★What is resumed is not that field but `resume_session_id` — do not fold the two fields★: resume is not a flag
    ///   but **a subcommand + positional argument** ([`RESUME_SUBCOMMAND`]), so the assembly shape differs from the start. Folding them
    ///   makes the meaning of `assigns_session_id() == false` false.
    /// ★Only terminal mode is assembled★ — app-server mode's resume is not argv but the second handshake request
    ///   (`thread/resume`), and the place that picks it is [`AgentBackend::open_spawn`]. The same value goes out by different means in the two
    ///   modes, not twice.
    /// ★If it is Resume but there is no handle, **it falls to new-conversation argv**★ — emitting `codex resume` without an id
    ///   brings up the **TUI picker** (measured 0.155.0 `--help`: "picker by default"), and that screen ties the agent attached to the PTY
    ///   to the first screen forever. So here the subcommand itself is dropped and argv **byte-identical** to Fresh
    ///   is emitted.
    ///   ★That regression is not passed over silently — but the place that reports it is not here★:
    ///   `resume_no_fallback` of [`crate::manager::AgentManager`] decides the same condition before spawn
    ///   (`opens_a_new_conversation`), leaves a warning, and emits the outcome as `Started` instead of `Resumed`.
    ///   Reporting it here again would write the same fact from two sources diverging — this place only builds arguments.
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
    ) -> CommandSpec {
        match command {
            AgentCommand::Codex {
                extra_args,
                output_format,
            } => {
                let mut args = Vec::with_capacity(8 + extra_args.len());
                match output_format {
                    AgentOutputFormat::Terminal => {
                        // ★The subcommand and its positional argument must be **at the very front**★ — pushed back, clap reads
                        //   `resume` not as a subcommand but as the root's `[PROMPT]` (the same constraint of the app-server branch
                        //   is nailed down by `app_server_extra_args_come_last`).
                        // ★The two are a pair — do not emit only one★: `codex resume` without an id is the TUI picker, so
                        //   the agent stops on the first screen (that branch of the doc above).
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
                    // ★That the policies and working folder are not carried here is intended★ — app-server takes those three as
                    //   `thread/start` parameters (`cwd` · `sandbox` · `approvalPolicy`) (measured 0.154.0).
                    //   The place that builds those values is [`AgentBackend::open_spawn`] below, and the process cwd
                    //   stays as `CommandSpec.cwd` — the two receive the same value but are not the same field.
                    AgentOutputFormat::StreamJson => {
                        args.push(APP_SERVER_SUBCOMMAND.to_string());
                        args.push(APP_SERVER_STDIO_FLAG.to_string());
                    }
                }
                // Our arguments are used up first and the caller pass-through follows — the three of terminal mode all take a single
                //   value, so they do not absorb the following arguments (unlike claude's variadic `--allowedTools`).
                // ★The pass-through does not tell modes apart — and the option sets of the two modes differ (measured 0.154.0:
                //   `codex --help` and `codex app-server --help` each have flags the other lacks)★. That is, arguments written by looking at the interactive
                //   CLI may be refused by app-server. Still, they are not filtered per mode:
                //   what is valid is changed by upstream per version, so a list we hold goes stale, and a stale list quietly erases sound
                //   arguments.
                args.extend(extra_args.iter().cloned());
                // ★Known limit — a path containing `%VAR%` gets substituted while passing the shim (2026-09-08, decided not to
                //   fix)★. `console_command` below wraps it in `cmd.exe /c` on Windows, and cmd expands
                //   `%NAME%` on the command line as an environment variable **even inside quotes**. So when a real folder whose name contains `%…%`
                //   (`C:\x\%USERNAME%\y`) is received, codex sees **a different folder** as its workspace.
                //   In terminal mode the `CD_FLAG` value above and `extra_args`, and in app-server mode
                //   only `extra_args`, take that path (that mode's working folder goes not on the command line but as a `thread/start`
                //   parameter, so it does not pass cmd).
                //   ★Why it is not fixed★ — ① the shim cannot be removed (user decision · TRD §6-H: `codex` is launched by its name
                //   as is. `codex` on PATH is a `.cmd`, so `CreateProcessW` cannot launch it directly)
                //   ② the known fix is **not an ordinary escape**: the trick rust std uses when launching a `.bat` is
                //   inserting `%%cd:~,%` (a no-op substitution) at every `%`, and it presumes the form `cmd /e:ON` + `/c "one string"`
                //   (the RUSTSEC-2024-0006 response). Our path is not that form (portable-pty assembles the argument array
                //   directly and hands it to `CreateProcessW` — 0 lines of `%` handling), and moving that trick outside its
                //   premise is an unverified port ③ that port touches `console_command`, **shared with claude**,
                //   so if it is wrong, every spawn running today dies.
                //   ★What fixing it needs★: a measurement that overturns the three above (do claude and
                //   codex both come up under a `cmd` wrapped in that form). Until then this limit is left knowingly.
                // ADR-0004

                // ★★The overrides are all **after** the pass-through — this place is the source of truth for that discipline★★ (sibling blocks
                //   point here). Measured (codex-cli 0.155.0 · 2026-09-19): passing the same config key twice with `-c`
                //   **the last one wins** — no merge and no duplicate-key error. So if placed in front,
                //   one line of user arguments erases our value entirely, and that outcome leaves no signal on screen or in the
                //   log. ★So do not move these blocks back into the terminal branch above in the name of 「tidying the argument assembly」★.
                // ★★If we override a user value, **we say so — silent overriding is forbidden**★★ (ADR-0216 decision 1).
                //   ★Still, it is not filtered★: erasing the pass-through would be us censoring user
                //   arguments. Only the fact that ours won is left (this place is also the source of truth for the sibling blocks).
                // ★MCP server attachment **does not tell modes apart**★: the mail entrance is needed in both modes. What tells modes
                //   apart is the axis where the peer **already provides the same value by other means** (session id · priming), and
                //   mail is not in that axis.
                // ★This one value is the **only physical wiring** of codex mail★ — claude reads the mcp-config file but
                //   codex takes only this override (measured 0.155.0). Without it there is no `eg_send` tool at all, and
                //   the daemon decides 「it uses MCP for mail」 (`mail_allowed=false`) and closes even the CLI mirror, so
                //   **send entrances become 0**. That outcome is not an error but silence.
                // ★That ours is carried later is obtained by order★ — passing the same key twice with `-c`, the last one
                //   wins (measured · ADR-0216 inherited 「override after passthrough · warn if overriding」).
                // ADR-0128
                // ADR-0209
                // ★The two branches of 「do not attach」 are **not told apart** here★ — whether a legitimate absence
                //   ([`McpAttachment::NotWanted`]) or a failure to build ([`McpAttachment::Unrepresentable`]),
                //   what this place does is the same (it does not carry the argument). The place that cuts the failure-to-build side as a spawn abort is
                //   [`AgentBackend::precheck_control_endpoint`], and this function only builds arguments (the same discipline as the
                //   brief block below deferring the reporting place to the assembly point).
                match mcp_attachment(control.as_ref()) {
                    McpAttachment::Attach(value) => {
                        if passthrough_overrides_key(
                            extra_args,
                            &format!("{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}"),
                        ) {
                            tracing::warn!(
                                "the codex pass-through set `{MCP_SERVER_OVERRIDE_PREFIX}{MCP_SERVER_NAME}` directly — carrying our attachment after it for the mail entrance, overriding that value (the last `-c` wins)"
                            );
                        }
                        args.push(CONFIG_OVERRIDE_FLAG.to_string());
                        args.push(value);
                    }
                    McpAttachment::NotWanted | McpAttachment::Unrepresentable(_) => {}
                }

                // ★Do not revive the `SessionStart` hook here (ADR-0216/ADR-0218)★ — the recovery path is the name of the writer lock this child
                //   holds ([`thread_lock`]), and the hook machinery was removed from the repository. Not because the hook
                //   **was broken** (it ran — measured
                //   0.155.0): the one rejection reason is that codex hashes the hook **command string** and requires human approval,
                //   and that string has our exe's absolute path embedded in it. Releases unpack into a folder named after the tag,
                //   so the path changes every release, so the approval screen comes up again every release, and
                //   **while that screen is up, terminal codex does not come up at all.**

                // ★Priming injection — only terminal mode is assembled here★: app-server mode carries the same content not on the command line
                //   but in the handshake JSON (`developerInstructions` of `thread/start`)
                //   ([`AgentBackend::open_spawn`]). The same value goes out by different means in the two modes,
                //   not twice — the same shape as resume (`resume`) splitting into argv and
                //   handshake.
                // ★Why modes are told apart — unlike the hook, not 「because it is a duplicate」★: this command line has an 8191-character cap,
                //   and the brief can eat that whole budget. The JSON body has no such cap (measured to accept
                //   up to 8 MB), so the value can be sent untouched. Carrying it on the command line in both modes would make
                //   the app-server side bear that cap and the `%` · newline risks for no reason.
                // ★It is **after** the pass-through★ — passing the same key twice with `-c`, the last one wins. The source of truth for the reason is
                //   the order comment of the MCP attachment block above, and it is not repeated here.
                // ★Never fail the spawn on any branch★ — even if the file cannot be read, even if there is a character that cannot be carried,
                //   even if it exceeds the budget, only a warning is left and the argument is not carried (the discipline ADR-0216 inherited). Priming is
                //   nice to have and the spawn is required.
                // ADR-0004
                // ADR-0092
                // ADR-0215
                // ADR-0216
                if matches!(output_format, AgentOutputFormat::Terminal) {
                    if let Some(text) = priming_text(control.as_ref()) {
                        if let Some(value) = developer_instructions_override(&text, &args) {
                            if passthrough_overrides_key(extra_args, DEVELOPER_INSTRUCTIONS_KEY) {
                                tracing::warn!(
                                    "the codex pass-through set `{DEVELOPER_INSTRUCTIONS_KEY}` directly — carrying our value after it for the receive-contract priming, overriding that value (the last `-c` wins)"
                                );
                            }
                            args.push(CONFIG_OVERRIDE_FLAG.to_string());
                            args.push(value);
                        }
                    }
                }

                // ADR-0086 step 2 (CLI entrance) — ★does not tell modes apart★: what is planted is only three env values, and what
                //   reads those values is not codex but **the children codex launches** (shell tools · the user's own hooks).
                //   Both modes launch children, so there is no axis to split on.
                // ★`config_path` · `settings_file` **do not come — and that is the declared result**★: both are
                //   **files claude reads by path**, and codex has no such flag (measured 0.155.0:
                //   there is neither `--mcp-config` nor `--settings`, nor a `--config <file>` that points to a settings file),
                //   so this folder declares [`AgentBackend::writes_mcp_config_file`] false, and the daemon decides the write
                //   by that one field. This backend's MCP attachment is the single `-c` value above alone.
                //   ★Before that field was split off, **a plaintext token file nobody reads appeared** on every spawn★ —
                //   because the daemon's derived axis served both 「does it use MCP for mail」 and 「does it take our mcp-config file」 in one field.
                //   Reviving that dual role (= removing that field or deriving it from `accepts_mcp_config`)
                //   brings back both the waste and the risk of a fail-closed spawn abort.
                //   ★Still, the injection below ignores the two fields even if they come as `Some` — do not remove that defence★:
                //   the control channel is not used by this backend alone, and copying claude's flags with 「it is Some, so let's
                //   use something」 kills the launch (that assertion = `the_daemon_written_files_are_still_not_translated_into_argv`).
                // ★`grants` is not used either★ — that list is **pre-approval** of permission prompts, not a tool allow list
                //   (ADR-0094), and translating it into codex config's `enabled_tools` changes the meaning from 「approved in advance」 to
                //   「only these exist」. A value of a different meaning is not called the same value.
                // ★`priming_file` **is used — but its content, not its path**★: the only instruction key that takes a path is
                //   `model_instructions_file`, and that one **replaces** rather than appends, so hanging it there makes codex's
                //   own operating instructions disappear, and `developer_instructions_file` does not exist as a key (measured
                //   0.155.0). So this backend **reads** that file and carries it as [`DEVELOPER_INSTRUCTIONS_KEY`] above
                //   (terminal) or as `developerInstructions` of `thread/start` (app-server).
                //   ★Do not revert it with 「reading the content goes against ADR-0092」★ — the role that ADR gave the consuming
                //   program is **path resolution**, and that role is still borne by the daemon's provider alone
                //   (this folder only reads the path carried in and does not choose which file). That the reading place is the
                //   backend folder is ADR-0004 as is — 「under which key and in what shape to carry it」 has nowhere else
                //   to go.
                // ADR-0086 / ADR-0092 / ADR-0133 / ADR-0209
                //
                // ★The incarnation marker is inside the same door too — do not take it outside★: a spawn with no endpoint has nothing to do with our
                //   control plane, and if it were outside, our marker would remain in the session records of such spawns too
                //   (that assertion = `without_an_endpoint_nothing_is_injected`). That the material,
                //   `(agent_id, epoch)`, lives in the endpoint is the concrete form of that condition.
                // ★It does not tell modes apart★ — where the marker sits is not argv but the session record codex writes,
                //   and both modes create that record.
                // ★It is only planted — there is **no reader, and none is planned**★ (ADR-0218 decision 8): recovery is carried by the lock
                //   holder, so this marker remains only for diagnostics and later comparison.
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

    /// The basis for `session.resume` being on is ★independent of the issuer★ — restore depends **solely** on the backend sid stored in the profile,
    /// and who issues that sid is decided by the backend. codex is the side that receives and uses it
    /// (app-server via the `thread/start` response, terminal mode via the name of the writer lock its child holds —
    /// [`thread_lock`] · ADR-0218), and with that value both modes resume.
    /// ★So this field does not tell modes apart either — it is **the same predicate as [`AgentBackend::can_resume_stored_session`] above.
    ///   Do not touch only one side**★: that axis is where the activation entrance asks 「launch as Resume?」, and
    ///   this field is where that result is reported to consumers, so if they split, a new thread is reported as 「resumed」 (or that
    ///   other way round).
    /// `model.select` is false even though codex has `-m` — this field reports not **what the program can do** but
    /// **what this spawn uses**. We do not expose that field, so we do not report it.
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
    /// ★It is only the reported value, and the real thing is built by [`AgentBackend::open_spawn`] below★ — both look at [`is_app_server`],
    ///   so fixing only one side makes the report and the real thing disagree.
    fn transport_shape(&self, command: &AgentCommand) -> TransportShape {
        if is_app_server(command) {
            TransportShape::StdioBidiJson
        } else {
            TransportShape::Pty
        }
    }

    /// app-server mode builds a bidirectional JSON link, terminal mode builds a PTY.
    ///
    /// ★The decision is [`is_app_server`] alone — do not read back the `transport_shape` report and match on it (ADR-0191)★:
    ///   doing so creates a second switch that splits on the shape value.
    /// ★This is the place that injects `structured: true` and the `thread/start` policies (ADR-0044/0030)★: the link knows neither what the bytes
    ///   it carries are, nor which folder to open with which sandbox. The side that knows is this
    ///   backend, which picked the mode and the policies.
    /// ★`sid_sink` is **used by both branches — only the means differ**★. app-server **receives** the identifier in the `thread/start` response
    ///   and the link hands it over as is (it goes out before anything is sent with that session —
    ///   the ordering contract of [`AgentBackend::open_spawn`]). The terminal branch has no response, so it polls the writer
    ///   lock the child holds and puts that file name into the same verb ([`thread_lock`] · ADR-0218).
    ///   ★Do not revive a constructor that reads the PTY screen here (ADR-0217)★ — recovery is decided by ownership, not by
    ///   the screen.
    ///   ★Do not create a second write path★ — the assembly point tied the incarnation guard to this verb, so adding one more
    ///   branch that touches the profile directly bypasses that guard (ADR-0218 decision 6).
    /// ★The place that picks `thread/start` or `thread/resume` is also here★ — it splits by the single
    ///   `resume_session_id` the assembly point passed ([`thread_open`]). ★The link is not asked again★: if the link decided by looking at its own
    ///   state, there would be two deciding places.
    /// ★`resume_session_id` is not used in the terminal branch — but the reason is **not** 「because there is no
    ///   handle」★: that mode's resume already went out in argv (the terminal branch of [`AgentBackend::build_spec`]).
    ///   Using the same value here again would make one spawn try to resume by two means.
    /// ★`control` is also **read only in the app-server branch, by the same discipline**★ — what is taken out here is one thing, the priming content
    ///   ([`priming_text`]), and the terminal branch already sent it as `-c developer_instructions=…`.
    ///   So this field does not reach the PTY-side constructor.
    // ADR-0185
    // ADR-0191
    // ADR-0215
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
        control: Option<&ControlEndpoint>,
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
                thread_open(spec, resume_session_id, priming_text(control)),
                sid_sink,
                link_sink,
            )?;
            // ★`TransportOwned` and the first-turn port are a pair — do not keep only one★: that mode's session does not count submissions,
            //   so if the port is not plugged into the link, no incarnation can persist the thread id (no error).
            // ADR-0226 · ADR-0231: user decision — the id is persisted when it became real (the peer's first user message echo).
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
            // ★Session id recovery starts **from here** — the lock appears only once the child is already up★
            //   (ADR-0218). Whether to run it and its material are all decided by [`thread_lock::plan_capture`]
            //   (source of truth for the conditions = that doc), so all this place does is hand over the child's two identity fields and **our side's** default
            //   lock folder — if the child received a different home, that one wins.
            // ADR-0218
            let child_start =
                pid.and_then(engram_dashboard_platform::process::process_creation_time);
            // ★The fact the gate looks at = 「did the resume argv actually go out」★ — not the **existence** of the
            //   handle. For a value that cannot be carried, `build_spec` above emitted new-conversation argv, and that incarnation is
            //   a recovery target. The two places look at the same predicate ([`resume_argument`]).
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

    // ★No synthetic input echo is declared — do not revive it★: this backend echoes user messages itself as
    //   `userMessage` items of `item/*` (measured), and the translator marks them as 「what we sent」
    //   and lets them through. In app-server mode the idle input bubble is emitted **by the link** — only the link knows whether it is idle or mid-turn
    //   (text mid-turn goes into the list and has no bubble), and that bubble's uuid is our id, so it folds into one copy with the echo's
    //   `clientId`. The session-layer echo does not know that split, so this mode's session does not emit an echo
    //   (the link-turn verb branch of `AgentSession`).
    //   ★A link below the floor does not emit the bubble★ — the `uuid` of the echo bubble is `clientId` first, and the codex
    //   item id only when that is absent (the translator's echo duplicate memory always counts by item id — the user message translation of `decoder`). The echo below the floor
    //   has no `clientId`, so that `uuid` becomes the item id, and if the link emitted a bubble with our id, the frontend's
    //   `uuid` dedup could not fold the two and the same question would remain twice. That overlap is exactly what the 「rejected alternative」 of ADR-0193 dismissed
    //   based on measurement. In terminal mode the PTY echoes on its own.
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
/// ★This backend's end signal is [`OutputEvent::TurnEnd`]★ — the translator does not emit `MessageDone`.
///   Still, that branch is written as `Ended`: if two words with the same meaning were split here, the end would quietly disappear
///   on the day this backend came to emit it.
/// ★Counting `Structured` as progress is load-bearing in this backend★: the only thing that comes in through this branch is
///   the echoed user message (the `userMessage` item translation) — unknown notifications are dropped (this folder's
///   `decoder` header), so there is nothing else — and it is the first observation of 「this turn has started」.
/// ★Still, 「from the moment we wrote `turn/start` until that first notification arrives」 is unobserved★ — that window is
///   not filled with a synthetic echo (ADR-0193 rejected that alternative). For queue release, 「can it send now」 is answered by the link
///   implementation from its own state, and the unobserved part on the mail gate side is absorbed as fail-open (ADR-0104's choice).
/// ★Why `Usage`/`Error` are not ends★: `Usage` (`thread/tokenUsage/updated`) comes mid-turn, and
///   `Error` (the `error` notification) is a retryable stream error, not a turn boundary. A failed turn is also closed by the `TurnEnd` that
///   `turn/completed` emits — the failure reason is carried **inside** that event.
/// ★Give-up paths where `turn/completed` **never comes** are also closed by `TurnEnd`★ (write failure · response decode failure
///   · error response · deadline expiry — the link's `end_turn_if`). Back when `Error` was written at that spot, this table could not count the end,
///   so **an idle agent remained observed as mid-turn** and mail was blocked until the 30-minute fail-open valve
///   (ADR-0127). So rather than promoting `Error` to an end in this mapping, that side uses the boundary vocabulary.
/// ★Why it can be shared with terminal mode (no per-mode branching needed)★: that mode has no decoder, so only `TerminalBytes`
///   flow, and using this mapping as is yields no signal at all.
/// ★It is safe even with no `Progress` before `Ended` (code basis)★: the table only writes `Ended` as `in_turn = false`,
///   so an unpaired end does not turn on mid-turn (`crate::turn::TurnObservations::observe_at`), and `in_turn_snapshot` carries
///   only those that are `in_turn`. So there is no reason to invent and fill in a 「start signal」. ★An unpaired error end is not the state right after
///   registration★ — it sets `last_end_failed`: the `turn/start` exit is intended (a request-stage failure is also an abnormal end — user decision
///   N11), and the boundary of an incarnation whose connection could not stand is erased by `forget` as that incarnation soon ends.
/// ★Error end = a `TurnEnd{Failed}` attributed to this incarnation's turn★ — a translated `turn/completed(failed)` (including the early-end release) and
///   the link's give-up path (`end_turn_if`). The turn table folds it into `last_end_failed` and the kernel holds mail (ADR-0231 — the same trigger as the link's
///   `halted`, but the one that uses that field is this classifier alone). ★Someone else's turn end blocked by the attribution gate and the display-only
///   boundary after a late receipt (`emit_without_turn_observation`) do not pass here★ — they neither set nor clear the halt.
/// ★Registry events up to `Delivered` are all `None` (unlike claude)★: this backend's `Delivered` comes **even after** the turn end
///   (a late echo of text returned to the held place at turn end · revival). If that turned 「mid-turn」 on again, that incarnation's mail would be blocked until the 30-minute
///   fail-open valve. The observation of turn start is already carried by the echoed user message (`Structured`).
/// ★A tool end result (`ToolResult`) is `None` whatever its outcome★: an interrupted command keeps running and closes **after the turn end** (measured
///   fixture `interrupt_fail_b5`) — counted as progress, an idle incarnation would revive as 「mid-turn」 and mail would be blocked until the 30-minute fail-open.
///   And a tool failure is an ordinary step of the agent, so counting it as an error would make one failed command set the halt-after-error.
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
    /// Polls until the condition holds — ★past the deadline, it dies **saying what it was waiting for**★.
    ///
    /// ★Why this function exists★: the deadline used to be a **silent fallthrough**. Past it, it just fell through and the assertion
    ///   below died with 「the value is wrong」, and when it actually blew up that way in CI, **the panic body was empty, so
    ///   「timeout」 and 「value error」 could not be told apart** (2026-09-23 · two tests of this file failed that way in the v0.3.1 tag run,
    ///   and the same tree was green in the branch run just before). Only by splitting them here
    ///   does the next failure say by itself which one it is.
    /// ★The cap is a safety net, not an assertion — do not shrink it to 「fail fast」★: the normal path holds within 1 second
    ///   (measured locally 0.5 s). A congested runner ran the same suite **6.7× slower** (measured: local
    ///   4.87 s ↔ CI 32.71 s). What the cap measures is 「did it happen」, not 「how fast it happened」.
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

    /// ★This value must **always be `None`** — both the hook registration and its builder were removed (ADR-0216)★. The one remaining use
    /// is measuring that it **has not come back**.
    /// ★Writing the key as a literal is intended★ — the current invariant is that production code has not a single character of this string,
    ///   so reviving the constant and referencing it would blur that fact.
    /// ★Do not count anything by the **existence** of `-c`★ — today two things carry that flag (MCP attachment ·
    ///   the brief). The old form that counted only the flag became a false positive every time a second one came in.
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

    /// ★Not 「at the very end」 but 「**after** our default arguments」 — the old name (`extra_args_come_last`) was a fact from before our
    /// overrides existed★. That the overrides must stand after the pass-through is a separate
    /// axis, and that one is measured by the `…_follows_the_passthrough` items.
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

    /// A spec drawn with the resume field filled — the issued field (`session_id`) is left empty as is.
    ///
    /// ★Passing the two fields **separately** is the point of this helper★ — folded into one, the value we issued could be carried
    /// in the argv of a backend whose [`AgentBackend::assigns_session_id`] is false, and the items below
    /// cannot see that folding.
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
        )
    }

    /// ★The subcommand and positional argument must go out **attached at the very front**★ — if `resume` is pushed back, clap reads it as
    /// the root's `[PROMPT]` and the resume quietly becomes a new conversation, and if the id separates from the subcommand it
    /// misses the `[SESSION_ID]` slot.
    /// The three policies are carried after it **as is** — `resume` also takes those three as its own options (measured 0.155.0).
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

    /// ★Without a handle, it must be argv **byte-identical to Fresh**★ — `codex resume` without an id is the TUI
    /// picker (measured 0.155.0 `--help`: "picker by default"), and an agent attached to that screen stops on the first screen
    /// forever. So a half assembly that emits only the subcommand is forbidden.
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

    /// ★The **reporting** of that regression is not done by this file — the assembly point does it★:
    /// `resume_no_fallback` of [`crate::manager::AgentManager`] decides `opens_a_new_conversation` as the product of two terms (the resume
    /// axis ∧ no handle in the registry), hands it to the new-conversation path, and emits the outcome as `Started` instead of `Resumed`
    /// (ADR-0226). Of those, **the resume axis is this file's declaration**, so it is nailed down here.
    /// ★An execution assertion does not catch this regression★ — if that axis flips, even if a request without a handle comes up on the resume path,
    ///   the argv falls cleanly to a new conversation per the item above, and what becomes false is **only the report** (the new conversation
    ///   goes out as 「resumed」).
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

    /// ★Fresh does not resume even with a handle★ — the assembly point passing this field empty on Fresh is the discipline
    /// (`AgentManager::spawn_agent`), but if this file leaned on that discipline the decision would be in two places. A request to open
    /// a new conversation with a dead incarnation's thread is exactly that combination.
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

    /// ★app-server mode does not emit resume as argv★ — that mode's means is the second handshake request
    /// (`thread/resume`), and [`AgentBackend::open_spawn`] picks it. If carried here, the `app-server` subcommand
    /// receives an unknown argument and the launch fails.
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

    /// The pass-through is **after** the subcommand · policies on the resume branch too — coming in front, it separates the subcommand and its positional argument.
    /// (Our overrides are even further after that pass-through — the `…_follows_the_passthrough` items
    /// measure that axis.)
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
            // ★In production this field comes as `None` — the `Some` here is a worst-case value put in **on purpose**★:
            //   `writes_mcp_config_file()` is false, so the daemon writes no file for this backend and sends the two fields
            //   empty. The fixture still carries `Some` because what the test below measures is 「if a path comes, does it
            //   leak into argv」 — putting `None` makes that assertion hollow, and on the day the daemon-side gate
            //   collapses, nobody would catch the regression of this backend taking that path and using it as is.
            config_path: Some(std::path::PathBuf::from("C:/engram/mcp-config.json")),
            send_exe: Some(std::path::PathBuf::from("C:/engram/bin/engram.exe")),
            // ★This path is **a file that does not exist**, and that is intended★: this backend now reads the brief's **content** and
            //   carries it (ADR-0215), and pointing at a real file would attach one more priming override to the argv of every test using this fixture,
            //   shaking the assertions that measure adjacency · order. A read failure is fail-open, so
            //   here it lands as 「not carried」. The items that measure the injection bake a real file with [`bake_priming`].
            priming_file: Some(std::path::PathBuf::from("C:/engram/priming.md")),
            // ★This field is the concrete form of 「the daemon authorized the MCP send entrance for this spawn」 — do not leave it empty★:
            //   an empty list is the state where the operator **turned off** MCP mail (that fixture is
            //   [`endpoint_without_mcp_send`] below), and building the default fixture with that value would make every test measuring MCP attachment
            //   measure the 「not attaching is correct」 side, becoming hollow as a whole.
            //   ★Pinning the tool names here is not retyping★ — the decision looks **only at the server name**
            //   (`ControlEndpoint::grants_mcp_send`). The source of truth for tool names is the daemon side, and this crate
            //   only carries them as data.
            grants: vec![ToolGrant::Mcp {
                server: MCP_SERVER_NAME.to_string(),
                tool: "eg_send".to_string(),
            }],
            settings_file: Some(std::path::PathBuf::from("C:/engram/session.json")),
            // ★The value production carries for this backend is `false`★ — this folder declares it that way (`uses_mail`) and the daemon
            //   derives from that declaration. The sibling test below separately measures that the opposite value is followed too.
            mail_allowed: false,
        }
    }

    /// The endpoint of a spawn whose operator **turned off** MCP mail. Even then the daemon issues the control channel itself (the control verbs are
    /// open to all — ADR-0132 decision 5) and only leaves out the send entrance grant — the result is this shape.
    ///
    /// ★Why both `ENGRAM_DISALLOW_MCP_SEND` and `ENGRAM_FORCE_CLI_ONLY_SEND` are represented by this one fixture
    ///   even though they touch different places★: the former removes the MCP grant in `build_grants`, and the latter
    ///   flips `accepts_mcp_config` so the grant goes to the CLI side — **both land on 「there is no MCP send
    ///   grant for our server」.** What the backend looks at is that one landing point.
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

    /// ★argv must have not a single `tui.` setting★ — this absence is half of ADR-0217 (the other half =
    /// the marker below). If it came back, we would return to giving up one screen line for the whole session in exchange for one recovery.
    /// ★It runs over all of endpoint presence · mode★ — the old value was carried without looking at the endpoint, so measuring only the side with one
    ///   would catch only half of a revival.
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

    /// ★For a handle that cannot be carried, the subcommand itself is not emitted★ — when codex receives a value it cannot read as a thread id, it
    /// reinterprets it as a **session name**, quietly creates a new session, and ends with exit code 0 (measured 2026-09-21).
    /// Then the user gets a new conversation they believe they resumed — a different kind of fault from the failure mode ADR-0218 set (「not received」).
    /// Source of truth for the reason = [`resume_argument`].
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

    /// ★A handle that cannot be carried is not 「a resume」 but 「a new conversation」 — so **recovery must run**★.
    ///
    /// This field was a real defect (found 2026-09-21): the argv side refused that value with the G7 guard and came up as a new conversation,
    /// but the recovery gate looked only at 「is there a handle」 and was off. The result was **a session where nobody writes
    /// that incarnation's thread id**, and the next activation goes with the old handle (or none). It was closed by making the two decisions look at the same predicate
    /// ([`resume_argument`]).
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

    /// ★Resume argv and lock recovery: **exactly one** stands★ — if both stand, the already resumed id is found and written
    /// again, and if neither stands, **nobody writes** that incarnation's id. The latter was a real defect (found
    /// 2026-09-21): while the recovery gate looked at 「is there a handle」, the argv side refused a handle that cannot be carried
    /// and came up as a new conversation, but recovery was off.
    ///
    /// So what is measured is not 「both do not stand」 but **equivalence** — `argv_resumes == !plans_capture`.
    /// ★The combination of `Fresh` with a handle carried in is not built here★: the only place that can build that combination is
    ///   the assembly point, and `manager::resume_handle_for` blocks it there (that side's test =
    ///   `manager::tests::fresh_never_carries_a_resume_handle`). Here that invariant is reproduced as is and
    ///   only **the six cells that can actually occur** are run — putting an assertion on a combination that does not exist makes it
    ///   impossible to know what that assertion guards.
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

    /// ★The value is pinned byte for byte — measuring together that the marker is actually carried in the spawn env and its shape★.
    /// ★Do not revert to 「the reader compares」★ — there is no comparing place, and none is planned (ADR-0218
    ///   decision 8). The value is pinned here because diagnostics lean on that shape, not because a decision does.
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
    // ★The order · conflict-warning axis is measured on the side of the living overrides★ — `the_priming_override_follows_the_passthrough`
    //   and `a_conflicting_mcp_passthrough_loses_to_ours_and_is_not_filtered` are those places.
    //   The two hook versions that were here lost what they measured (the hook value carried in argv) and were succeeded by those.

    /// ★Terminal spawns no longer carry the hook★ — this one line nails down ADR-0216's replacement of the recovery path.
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

    /// The same for app-server mode. ★These two once split — back then the hook was terminal-only★ (app-server
    /// received the id directly in the `thread/start` response). Today neither mode sets it.
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

    /// Passing another key with `-c` is not a conflict — if warnings are spammed, nobody reads them.
    ///
    /// ★The key is measured **on the living side** (the brief)★ — measuring with a removed key would not tread the same axis as the warning
    ///   this helper actually drives.
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

    /// On the resume branch too, the subcommand is at the very front and our overrides are after the pass-through — the two do not push each other out.
    ///
    /// ★「At the very end」 is not pinned★ — that slot is taken by the sibling override (the brief) when there is a priming file,
    ///   and that is not a regression. The actual invariant is only **after the pass-through**.
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

    /// ★Do not revive it★ — in terminal mode the PTY echoes input on its own (the session's synthetic echo is a device that imitates the json
    /// mode that lacks it — ADR-0044). The session layer holds an [`InputEncoder`], not the backend, and goes through the dispatch
    /// table, so looking only at the declaration and not at that table lets a revived echo pass while green.
    ///
    /// app-server mode's bubble is emitted by the link, not by this table (ADR-0231 — the session does not emit an echo in the link-turn verb branch,
    /// and a link below the floor does not emit the bubble — ADR-0193 「rejected alternative」). ★That the bubble does not break **the order of the screen
    /// restore** is thanks to the link's idle condition (ADR-0203)★: history is received **after** the handshake and raised before the gate
    /// (`Link::Ready`), but the synthetic bubble stands only at `Ready`, so text typed in that window goes into the list without a bubble.
    /// Lifting that condition would draw the user's new words above the restored old conversation.
    #[test]
    fn the_terminal_mode_makes_no_synthetic_input_echo() {
        let enc = CodexBackend.input_encoder(&codex(vec![]));
        assert!(
            enc.input_echo_event("hello codex".as_bytes(), Uuid::new_v4())
                .is_none(),
            "{enc:?}: the synthetic echo came back"
        );
    }

    /// Only app-server mode hands the list to the link, and whether receipt is possible is **the very** Arc the link's floor decision fills.
    /// The link comes out wrapped as `dyn`, so it is measured by owner count — one share for the assembly point + one for the link's decision field (that this field
    /// is what `delivery_ack()` hands out is measured by the link-side test
    /// `the_transport_hands_out_the_very_ack_its_verdict_fills`). A separately made Arc would be 1.
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
            crate::backend::open_spawn(command, &probe, 80, 24, None, None, None, None, None)
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

    /// This backend's `Delivered` comes even after the turn end (late echo · revival) — none of the registry events is a turn
    /// signal.
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

    /// ★Measured (codex-cli 0.156.1 · fixture `interrupt_fail_b5`): the `failed` end of an interrupted command comes **after** the turn end★ — the translator
    /// emits that end as `ToolResult{Failed}` after `TurnEnd{Interrupted}`, and even when that event is fed to the turn table, an idle incarnation does not revive
    /// as 「mid-turn」 and no halt-after-error is set (counted as progress, mail would be blocked until the 30-minute fail-open).
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

    /// Why the three declarations are in one item = they are **a pair that must be looked at together**. The issuing axis is off in both modes
    /// (the issuer is codex, so there is no value for us to plant), and the two resume fields are on in both modes —
    /// what splits is only **the means** (argv ↔ `thread/resume`), and these four fields do not ask about the means.
    ///
    /// ★Writing all four fields is the point★ — if **only one side** of the resume axis and the caps report field splits, a new thread that was never
    ///   resumed is reported as 「resumed」 or the opposite, and measuring only one field per mode would leave that
    ///   mismatch invisible in this file.
    /// ★To revert the two fields of terminal mode to `false`, the argv assembly must be removed too★ — turning off only the declaration
    ///   makes the activation entrance launch as Fresh while [`AgentBackend::build_spec`] is still prepared to resume,
    ///   so that wiring becomes dead code that is never called.
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

    /// ★The declaration and the real thing must be a pair★ — `declares_link()` decides at the assembly point whether to **lay** the delivery port,
    /// so if true but the link does not call it, the supervisor waits until the backstop, and if false but it calls, nobody receives
    /// that delivery.
    ///
    /// What is measured here is the declaration side, and the real side (the app-server link actually delivers) is
    /// measured with a real link by `a_rejected_resume_does_not_fall_back_and_ends_the_session`.
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

    /// ★What decides this value is not whether there are turn signals but ADR-0116 decision 7★ — the fact that terminal mode has no signal
    /// is no reason to emit false here (terminal claude already receives in the same place). To revert it,
    /// overturn that decision first.
    // ADR-0116
    #[test]
    fn reads_messages_is_true() {
        assert!(CodexBackend.reads_messages());
    }

    // ── ADR-0185: the received thread id actually reaches the assembly point's record verb ──────────────────

    /// Removes one file of the fake app-server — `true` if removed. ★The shape that tries once and gives up left scripts in `%TEMP%`★:
    /// even after `shutdown()` returns, the child holds that file handle a little longer, so the first
    /// `remove_file` fails. The marker file goes through the same door — the fake writes it and closes it right away, but a third party that briefly
    /// opens new files (antivirus · indexer) can make a single attempt flaky (never observed).
    /// ★Why it returns the result = `eprintln!` is swallowed by libtest in **passing** items★ — written there,
    /// the fact that it could not remove leaves no trace anywhere and only leftovers pile up. The caller raises it as an assertion.
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
    enum FakeResume {
        /// Returns the received `threadId` with a prefix — the point is that it becomes a value **different** from the one we sent.
        EchoesTheThreadId,
        /// Refuses with `-32600`. ★That this code is not exclusive to 「unknown thread」 is the reason for ADR-0082's codex-side
        /// addendum★ — an unknown method, a duplicate `initialize` and a config error all come with the same code (measured
        /// 0.154.0 — the source of truth is this folder's `protocol` and the link test bench), so splitting by code and falling back to a new thread would overwrite a still
        /// sound handle on an unrelated failure.
        Rejects,
    }

    /// The refusal wording [`FakeResume::Rejects`] carries — items find it in the delivered reason to tell 「the refusal actually
    /// made the round trip」.
    /// ★The wording is the measured one★ — the purpose of this fake imitating 「unknown thread」 is to drive it all the way to classification,
    /// and with invented wording `resume_failure_kind` would not recognize it, and that wiring would simply pass the test bench
    /// (`docs/reference/backend-capabilities.md` §1 「when given an unknown id」).
    #[cfg(windows)]
    const FAKE_RESUME_REJECTION: &str = "no rollout found for thread id";

    /// The name of the environment variable carrying the path of the empty file the fake creates as it enters its read loop.
    #[cfg(windows)]
    const FAKE_READY_ENV: &str = "ENGRAM_FAKE_APP_SERVER_READY";

    /// One baked fake app-server.
    ///
    /// ★Holding the two paths together is a contract★ — each item must remove them with [`FakeAppServer::remove`], and must raise the fact
    /// that it could not remove as an assertion.
    #[cfg(windows)]
    struct FakeAppServer {
        spec: CommandSpec,
        script: std::path::PathBuf,
        /// The empty file the fake creates as it enters its read loop — [`wait_for_the_fake_to_listen`] waits for it.
        ready: std::path::PathBuf,
    }

    #[cfg(windows)]
    impl FakeAppServer {
        /// Removes the script and the marker and returns **the paths it could not remove** — if empty, both were removed.
        #[must_use = "dropping the fact that it could not remove quietly piles up leftovers — raise it as an assertion"]
        fn remove(&self) -> Vec<&std::path::Path> {
            [self.script.as_path(), self.ready.as_path()]
                .into_iter()
                .filter(|path| !remove_fake_file(path))
                .collect()
        }

        /// One **PowerShell** line to run this fake by hand (it does not run as is in cmd · bash).
        /// ★The marker path variable is written together★ — without it the script dies right at the line that writes the marker
        /// (`$ErrorActionPreference = 'Stop'`) and the failure is not reproduced.
        /// All values are wrapped in single quotes and an inner `'` is doubled — so that spaces · `$` in the path are not interpreted.
        fn hand_run(&self) -> String {
            let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
            let args: Vec<String> = self.spec.args.iter().map(|a| quote(a)).collect();
            format!(
                "$env:{FAKE_READY_ENV}={}; {} {}",
                quote(&self.ready.to_string_lossy()),
                self.spec.program,
                args.join(" ")
            )
        }
    }

    /// Bakes a fake app-server into a temp file. `start_thread_id` = the id this fake answers to `thread/start` with.
    #[cfg(windows)]
    fn bake_fake_app_server(start_thread_id: &str, resume: FakeResume) -> FakeAppServer {
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
            .replace("READY_ENV_PLACEHOLDER", FAKE_READY_ENV);
        let stem = format!("engram-fake-app-server-{}", Uuid::new_v4());
        let script_path = std::env::temp_dir().join(format!("{stem}.ps1"));
        let ready = std::env::temp_dir().join(format!("{stem}.ready"));
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
            env: vec![(FAKE_READY_ENV.into(), ready.to_string_lossy().into_owned())],
            cwd: PathBuf::from("."),
        };
        FakeAppServer {
            spec,
            script: script_path,
            ready,
        }
    }

    /// Waits until the fake enters its read loop — ★call it right before `start()`★.
    ///
    /// The clock of the handshake budget (`transport::HANDSHAKE_BUDGET`) is started by the writer `start()` launches, but the fake
    /// is already up before that, in `open_spawn`. The powershell startup in between is not what the items using this fake
    /// measure. In CI run 35447034272 attempt 1, these four items failed together with an `initialize` deadline expiry (attempt 2 of the same
    /// run was green). ★That the time went into powershell startup is the most plausible reading, not a certainty★
    /// — all the log left is one deadline-expiry line, and what backs it is that a delay injection inserting 12 seconds before startup reproduces the same
    /// failure. Startup is moved out of the budget, and only the round trip is measured **with the production budget as is**.
    /// ★The budget is not raised to fit★ — that value is production behaviour (that constant's doc). There is no handle in the `start()` path
    /// to inject a test-bench budget, and providing one would need a new seam in production code (ADR-0012 — user decision).
    /// ★What the marker guarantees reaches only 「the script started running」★ — the cost of the per-request response path (regex · write) running for the first
    /// time is still inside the budget. On a slow runner that value is unmeasured, and if large the symptom is the same deadline expiry.
    #[cfg(windows)]
    fn wait_for_the_fake_to_listen(fake: &FakeAppServer) {
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

    /// Waits until the record verb is called — ★if a connection outcome is delivered before that, it stops waiting and dies saying, with that outcome, what
    /// went wrong★.
    ///
    /// The record verb is called **only after** the handshake round trip stands, and the outcome is delivered **only after** passing that point. So
    /// an outcome that came without a record is one of two — if `Failed`, the round trip fell over, and if `Ready`, **the record port did not reach
    /// the link** (without a port, the link skips the record and sets up the connection — `record_session_id`). The latter is
    /// the wiring regression [`tests::an_app_server_spawn_hands_the_thread_id_to_the_sink`] is meant to catch. Either way,
    /// waiting only for the record spins until the deadline and leaves only 「it did not happen」 — the reason is carried only in the outcome.
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
            Some(crate::transport::LinkResolution::Ready) => panic!(
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

    /// A minimal app-server that answers only the two handshake requests. ★Not the real codex★ — what is measured is one thing, 「does the thread id the peer
    /// gave reach the verb handed to [`AgentBackend::open_spawn`]」, and the real CLI is irrelevant to that answer
    /// (ADR-0012 isolation).
    /// ★Baked into a file, not passed as an argument★ — the script contains JSON, so `"` is required, and that character
    /// gets chewed twice going through Rust `Command`'s argument escaping and powershell's command-line parsing.
    /// ★stdout is written as raw bytes★ — `Write-Output` drags encoding · BOM along with the host settings, and one BOM
    /// byte makes the first line not JSON.
    ///
    /// ★The answer of the `thread/resume` branch is filled in by [`FakeResume`]★ — what goes in that place is not a JSON fragment
    /// but **a powershell expression**, so an answer that appends the received `threadId` can be written too.
    ///
    /// ★**This fake does not die after answering and keeps reading stdin — that property is the premise of one item**★.
    /// What [`tests::a_recording_failure_ends_the_session`] measures is 「if the record fails, we close stdin and
    /// as a result the session ends」, but if this fake ended on its own, EOF would come **regardless of the close**, and that
    /// item would measure none of the wiring and go green. That this is not actually so was confirmed by mutation — removing the close
    /// makes **not a single** state transition observed in that item. ★So changing the while loop below into 「exit after
    /// responding」 quietly disarms that item★.
    ///
    /// ★The marker file is created **right before** the read loop★ — [`wait_for_the_fake_to_listen`] reads it as 「ready to
    /// answer」. Moving it before the definitions used for answering (`$so` · `Send`) would make that meaning false.
    #[cfg(windows)]
    const FAKE_APP_SERVER_PS1: &str = r#"
$ErrorActionPreference = 'Stop'
$so = [Console]::OpenStandardOutput()
function Send([string]$s) {
  $b = [Text.Encoding]::ASCII.GetBytes($s + "`n")
  $so.Write($b, 0, $b.Length)
  $so.Flush()
}
[IO.File]::WriteAllText($env:READY_ENV_PLACEHOLDER, '')
while ($null -ne ($line = [Console]::In.ReadLine())) {
  if ($line -match '"id":(-?\d+)') {
    $rid = $Matches[1]
    if ($line -match '"method":"initialize"') {
      Send ('{"id":' + $rid + ',"result":{"codexHome":"h","platformFamily":"windows","platformOs":"windows","userAgent":"engram-fake/0"}}')
    } elseif ($line -match '"method":"thread/start"') {
      Send ('{"id":' + $rid + ',"result":{"thread":{"id":"THREAD_ID_PLACEHOLDER","cliVersion":"0.0.0-fake"}}}')
    } elseif ($line -match '"method":"thread/resume"') {
      $tid = 'MISSING'
      if ($line -match '"threadId":"([^"]+)"') { $tid = $Matches[1] }
      Send ('{"id":' + $rid + ',' + RESUME_REPLY_PLACEHOLDER + '}')
    } elseif ($line -match '"method":"turn/start"') {
      Send ('{"id":' + $rid + ',"result":{"turn":{"id":"fake-turn"}}}')
      Send '{"method":"item/started","params":{"threadId":"THREAD_ID_PLACEHOLDER","turnId":"fake-turn","item":{"type":"userMessage","id":"fake-user-item","content":[{"type":"text","text":"echo"}]}}}'
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
        )
        .expect("open_spawn");

        wait_for_the_fake_to_listen(&fake);
        parts.transport.start(Arc::new(OutputCore::new(
            Uuid::new_v4(),
            1,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        )));

        // ★Failures are written split four ways★ — the fake did not come up (the wait above) · the round trip fell over (the reason of the failure outcome) ·
        //   the connection stood without a record (wiring regression) · the fake came up but there is neither a record nor an outcome (deadline). Whichever branch, it dies before the
        //   removing line, so the script remains.
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

    /// ★If the assembly point gives a value to resume, the second request splits into `thread/resume` — and what is recorded is
    /// **the id the peer answered**★.
    ///
    /// One assertion tells three things apart (because the fake's answer is `resumed-<the threadId we sent>`):
    ///   1. What went out is not `thread/start` — had it been, the fake would return `start_only`.
    ///   2. The `threadId` field carried the value we passed as is (including the camelCase spelling).
    ///   3. What the record verb receives is **the value the peer gave**. Had the implementation rewritten the value we sent, the original without the prefix
    ///      would come up and split here — that branch matters because on the day the peer gives a different id, that value is
    ///      the thread this incarnation actually talks in.
    ///
    /// ★`send_input` is never called★ — what is measured is only the handshake section, and sending here would mix in another branch where
    ///   the queue fills up to the cap.
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
        // ★The value this fake gives is **not a uuid** — that fact is pinned here★.
        //   The assembly point's record verb (`manager::session_id_sink`) **quietly drops** a value that fails `Uuid::parse_str`,
        //   so what actually happens on this branch is not 「recorded」 but **a split**: the link holds this
        //   string as `thread_id` and sends every later turn with it, but the `backend_session_id` on disk
        //   stays **the old value as is**. So the next resume opens a place that is not the thread this incarnation actually
        //   talked in.
        //   ★The partner that measures that split is `a_non_uuid_thread_id_leaves_the_stored_one_diverged` on the `manager.rs` side,
        //   and what is measured here is one thing, 「this test bench actually creates that condition」★ — without this assertion
        //   the `assert_eq!` above would be misread as 「it got as far as being recorded」.
        assert!(
            Uuid::parse_str(&got[0]).is_err(),
            "the fake's answer reads as a uuid — the premise of this item (the assembly point drops this value) is stale: {got:?}"
        );
        assert!(
            leftover.is_empty(),
            "could not remove the fake app-server files: {leftover:?}"
        );
    }

    /// ★A refused resume **does not fall back to a new thread** (ADR-0082)★ — and it does not touch the handle stored
    /// in the profile.
    ///
    /// The decisive evidence is that **the record verb is never called**: this fake answers `thread/start` too
    /// (`start_only`), so had the link seen the refusal and opened a new thread, that id would come up to the record verb. That absence
    /// is the absence of a second spawn, and with no record, the stored sid stays as is.
    /// ★Absence is not measured by deadline alone★ — absence is asserted **after waiting** for the failure boundary that shows on screen.
    ///   Only then are 「it just has not ended yet」 and 「there is no fallback」 told apart.
    ///
    /// ★And **it is measured here until the session reaches its end state**★ — this is the second axis of this item, and without it
    ///   the whole item is **green on top of a broken implementation**. This fake does not die after answering the refusal and keeps reading
    ///   stdin (that constant's doc), so if the link does not let go of our stdin, the child · reader · writer stay alive,
    ///   the reader never sees EOF, and [`crate::output_core::OutputCore::finish`] never runs. Then the message to the reaper
    ///   is never created at all (its sole consumer — ADR-0019), and the manager's activation decision sees only `Running` for the whole 3 seconds,
    ///   **stamps it 「resume succeeded」 and erases even the last failure record**.
    ///   ★This spot once said 「the end state is the manager's · reaper's job, and its regression net is `tests/activation.rs`」
    ///   — that net **does not exist** (that file has only one codex item, the profile wiring, and it does not measure the refusal
    ///   branch). So the only place that measures this link letting go of stdin is here.★
    /// ★Waiting before `shutdown()` is the point★ — if we kill it and then measure 「it ended」, we ourselves drove the causality this item meant to
    ///   measure (precedent = [`tests::a_recording_failure_ends_the_session`]).
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
        // ★Does the link actually report 「connection could not stand + reason」 — the only place measuring it on the real thing is here★:
        //   the manager-side items measure only the decision logic with a stand-in link, so if the real link does not fill that axis, that wiring
        //   breaks while both sides are green. ★It is caught **before** reaching the end state★ — after that there is no reason for this value
        //   to remain.
        let link_when_failed = match delivered.lock().unwrap().first() {
            Some(crate::transport::LinkResolution::Failed { reason }) => reason.clone(),
            other => panic!(
                "the failure boundary showed up but the link **did not deliver** the failure outcome — the activation decision has no signal \
                 to receive (delivered: {other:?})"
            ),
        };
        // ★Before anything below, first tell whether that reason is **the fake's refusal**★ — if not, this whole item (no fallback ·
        //   end state · classification) ran without a refusal, and the classification assertion would report an unrelated reason as 「could not classify」.
        //   It is even before the end-state wait so that this reason is quoted also on the branch where that wait does not finish.
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
        // ★Neither `!is_live()` nor `Exited { .. }` is **enough**★ (review finding — the old assertion was hollow):
        //   when the reader sees EOF, `Exited { code: None }` stands regardless of the child's liveness, so an assertion looking only at the kind
        //   **could not be false** when `reached_terminal` above was true — that is, it measured nothing.
        // ★A code being carried is the only evidence that the child was actually reaped★ — `Some(_)` means `try_wait`
        //   succeeded, and that succeeds only when the process has ended (so that value is not lost to a race,
        //   `reader_loop` looks again, bounded — the comment at that spot is the source of truth).
        // ★Which path it ended by is not told apart here★ — whether the peer saw EOF and ended on its own, or the link ended it directly
        //   past the grace period, both are legitimate outcomes, and telling them apart would require timing, which flakes.
        //   What this item prevents is **no outcome at all**.
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
    /// This item measures three things, and ★the third is the core★:
    ///   1. The record verb was actually called (= the handshake stood. Otherwise the two below spin idle).
    ///   2. Input is refused — not the shape where the link stopped at `Connecting` and only the queue filled (ADR-0190).
    ///   3. ★The session **reaches an end state**★. The old shape that measured only 2 **could not be told apart from a stuck session** —
    ///      a state where refusal works but the child · reader · writer remain, the pump never ends and nothing is reaped
    ///      passes 2 as is. The only thing that splits that branch is the end transition.
    /// ★「Record」 goes only as far as **handing it** to the port (ADR-0226)★ — behind the production port is the first-submission latch, so persistence is tied to
    ///   submission. A test that carries 「persist before the first turn」 exists separately (in `transport`'s
    ///   `the_session_id_is_recorded_before_the_gate_opens` doc).
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

        // It blows up leaving only the fact that it was called — because 「a panic happened」 and 「it was never called」 must be told apart.
        // ★It stands until released before blowing up★ — only then can the quiet-hook span be set only at the moment of the panic (the span
        //   comment below). If the sender disappears (the test dies first), the wait is released and it blows up as is, so it never stands forever.
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

        // ★First wait until the record verb is called **without sending anything**★ — `send_input` before the handshake
        //   normally queues (ADR-0190), so sending here would fill the cap (32), raise
        //   **a queue-full error**, and make this item green for the wrong reason.
        wait_for_recording(|| reached.load(Ordering::SeqCst), &delivered, &fake);

        // ★The hook is not swapped bare-handed★ — it is global, so other items in the same binary would lose their own panic output, and
        //   if nested, the quiet hook would remain forever. A helper that prevents both is used.
        // ★The quiet-hook span covers **only the moment of the panic**★ — that hook is process-global, so the longer the span, the more an **unrelated**
        //   item that blew up in that window loses its own panic message and location, and other panicking items line up before the helper's static mutex.
        //   So the handshake · end wait are kept outside the span, and only releasing the writer to blow it up and
        //   watching its result (connection outcome · input refusal) are inside — the outcome is delivered **after** the writer catches that panic,
        //   so waiting for it makes the panic happen inside the span for sure.
        let refusal = engram_dashboard_command::testing::with_quiet_panic_hook(|| {
            release
                .send(())
                .expect("the record port must still be alive");

            // ★Send **only once** after the outcome is delivered★ — the link delivers the failure outcome **after** setting `Link::Down`
            //   (the failure branch of `writer_loop`), so input after that is certain to be refused. Trying with a fixed count · interval
            //   would require the writer waking up, catching the panic and taking the link down to fall inside that window, so on a slow
            //   runner it falsely fails with 「input keeps being accepted」.
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

        // Wait for the peer to end on its own after we let go of stdin (measured 41–51ms) and for that EOF to end the pump.
        //   ★Waiting without calling `shutdown()` is the point★ — if we kill it and then measure 「it ended」, we ourselves drove the causality
        //   this item meant to measure.
        wait_until(|| terminal(), "the session going to its end state");

        let reached_terminal = terminal();
        let seen = statuses.lock().expect("status poisoned").clone();
        // ★Neither `!is_live()` nor `Exited { .. }` is **enough**★ — when the reader sees EOF, `Exited { code: None }` stands regardless of the child's
        //   liveness, so an assertion looking only at the kind could not be false when `reached_terminal` above was true
        //   (the old shape was hollow — review finding. A copy of it was in the neighbouring item too and was fixed together).
        // ★A code being carried is the only evidence that the child was actually reaped★ — the source of truth for the reason is
        //   the same spot in `a_rejected_resume_does_not_fall_back_and_ends_the_session`.
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
        // ★Only queue-full is excluded★ — only that one means 「refused although the failure path did not run」. The two remaining reasons
        //   (the link taken down by the record failure · the link that ended after closing stdin) are **both evidence that this path ran**, and
        //   which one is caught even after seeing the outcome is a race with the reader's EOF, so pinning it to one would itself be a flake.
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

    /// ★The pair for the mode where the link carries turns — the first-turn port is plugged all the way into the real link, and it is called not at the handshake but at the echo of the peer's user
    /// message (ADR-0226 amendment · user decision 2026-09-26)★.
    ///
    /// The record port was called at the handshake but the first-turn port not yet → handing over one turn makes the fake answer `turn/start`
    ///   and echo the user message → the first-turn port is called once. The fake's edition is below the floor, so the echo has
    ///   no `clientId` — it still counts. That the session does not count in that mode is measured by the `session.rs` test.
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
            Arc::new(move || {
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
            Some(crate::transport::LinkResolution::Ready)
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

    /// ★The generated value is pinned **literally**★ — this string is a wire contract the codex config parser reads, and
    /// if even one character is off during assembly, the symptom is not an error but 「mail silently does not work」. This shape was confirmed
    /// with the real codex (2026-09-19 · 0.155.0: passes `--strict-config` + registered in `mcpServerStatus/list`).
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

    /// ★If the operator turns off MCP mail, it must be off **in codex too** (found 2026-09-20)★: the old gate looked only at the **existence**
    /// of the control channel — so an operator who set `ENGRAM_DISALLOW_MCP_SEND=1` got a state where only the claude-side send entrance
    /// was lost and the codex side stayed open. Moreover that attachment
    /// carries `default_tools_approval_mode='approve'`, so it remains **auto-approved**.
    /// ★Measuring the on row **together** is the point★ — measuring only the off side stays green even if the attachment is erased entirely, and at that moment
    ///   the only physical wiring of codex mail disappears (the symptom is silence, not an error).
    /// ★It also measures that the control plane itself is not cut★ — turning off mail and cutting the control verbs are
    ///   different things (ADR-0132 decision 5), and if the credentials disappeared here too, the children codex launches would lose them.
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

    /// ★A spawn 「on the delivery roster but with zero send entrances」 is cut before it comes up★: this backend has
    /// `reads_messages` true so it is on the delivery roster, and `accepts_mcp_config` true so the daemon
    /// carries `mail_allowed=false` and closes the CLI mirror — in that state, if the MCP attachment is missing too, every arriving
    /// `request` becomes **a contract nobody can answer**. That outcome is not an error but eternal silence, so it
    /// leaves no trace anywhere.
    /// ★For claude, this spot is guarded by the daemon's `?` (mcp-config write failure) — this backend writes no file, so
    ///   the daemon has nothing to cut with★. This gate fills that asymmetry.
    /// ★It is called through the public dispatch★ — hanging it only on the impl would miss the path the assembly point calls.
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

    /// ★The three legitimate absences are pinned **together** — without them the gate above grows toward killing even sound spawns★.
    /// ② in particular is load-bearing: a spawn whose operator turned off MCP mail has no intent to attach in the first place, so whatever
    /// the address, there is no reason to cut it.
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

    /// ★Even if the file fields come as `Some`, they do not reach argv★ — in production they now come as `None`
    /// (`writes_mcp_config_file()` = false → the daemon does not write), but this test **deliberately** feeds the state where that gate collapsed
    /// to measure the second wall. codex has no flag to point to those two, so copying claude's flags with 「it is Some,
    /// so let's use something」 kills the launch.
    /// ★The third field (`priming_file`) being here together now means something else★ — that file **is read and carried**
    ///   (ADR-0215). Still, a **path** must not reach argv, so it stays in this comparison (the partner measuring the same fact
    ///   with a real file = `the_priming_path_never_reaches_the_command_line`).
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
        for path in [ep.config_path, ep.settings_file, ep.priming_file] {
            let path = path.expect("the fixture fills all three fields");
            let path = path.to_string_lossy().into_owned();
            assert!(
                !argv.iter().any(|a| a.contains(&path)),
                "the path `{path}` was carried in argv: {argv:?}"
            );
        }
    }

    // ── ADR-0215: priming injection (terminal = `-c developer_instructions=…` · app-server = handshake JSON) ──

    /// Bakes a brief file into a temp folder and returns an endpoint pointing to it.
    ///
    /// ★Returning the path too is a contract★ — each item must remove it. The default fixture's
    /// `C:/engram/priming.md` is **a file that does not exist**, so it falls to the read-failure branch (so the argv of other tests
    /// does not grow by this injection). To measure the injection, a real file is needed.
    fn bake_priming(text: &str) -> (ControlEndpoint, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("engram-priming-{}.md", Uuid::new_v4()));
        std::fs::write(&path, text).expect("write the priming file");
        (
            ControlEndpoint {
                priming_file: Some(path.clone()),
                ..endpoint()
            },
            path,
        )
    }

    /// A spawn the daemon did not carry priming for (the endpoint shape of the non-MCP branch).
    fn endpoint_without_priming() -> ControlEndpoint {
        ControlEndpoint {
            priming_file: None,
            ..endpoint()
        }
    }

    fn developer_instructions_value(argv: &[String]) -> Option<String> {
        let prefix = format!("{DEVELOPER_INSTRUCTIONS_KEY}=");
        config_override_values(argv)
            .into_iter()
            .find(|v| v.starts_with(&prefix))
    }

    /// Did the skipped branch **really change nothing** — is the argv byte-identical to a spawn without priming?
    ///
    /// ★Measuring only 「there is no injection」 is not enough★: even if the branch that could not build the value pushes in half an argument or shakes the order,
    ///   that assertion passes. This comparison is the concrete form of 「the spawn goes ahead as is」.
    fn assert_argv_matches_a_spawn_without_priming(argv: &[String]) {
        let bare = spec_with_control(&codex(vec![]), Some(endpoint_without_priming()));
        assert_eq!(
            argv,
            codex_argv(&bare),
            "the argv that skipped priming differs from a spawn without priming"
        );
    }

    /// ★The value is pinned **literally**★ — it is a wire contract the codex config parser reads, and the single quote (TOML literal
    /// string) and the replacement of newlines with two characters are both load-bearing.
    #[test]
    fn the_terminal_spawn_carries_the_priming_document_as_developer_instructions() {
        let (ep, path) = bake_priming("line one\nline two\n");
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let value = developer_instructions_value(&codex_argv(&s)).expect("priming was not carried");
        assert_eq!(value, "developer_instructions='line one\\nline two\\n'");
        std::fs::remove_file(&path).expect("remove the temp brief");
    }

    /// ★If even one real newline remains, cmd cuts the command line at that spot (no error)★ — the three forms
    /// (`\r\n` · `\n` · `\r`) are mixed in one document to catch an implementation that replaces only one of them.
    #[test]
    fn every_line_break_shape_collapses_into_one_line() {
        let (ep, path) = bake_priming("a\r\nb\rc\nd");
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let value = developer_instructions_value(&codex_argv(&s)).expect("priming was not carried");
        assert_eq!(value, "developer_instructions='a\\nb\\nc\\nd'");
        assert!(
            !value.contains('\n') && !value.contains('\r'),
            "a real newline remained in the value: {value:?}"
        );
        std::fs::remove_file(&path).expect("remove the temp brief");
    }

    /// ★One single quote breaks the TOML literal string — one English contraction brings it in★.
    #[test]
    fn a_quoted_priming_document_is_skipped_and_the_spawn_still_stands() {
        let (ep, path) = bake_priming("do not route around it — tell your principal you can't");
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let argv = codex_argv(&s);
        assert!(
            developer_instructions_value(&argv).is_none(),
            "a brief containing a single quote was carried as is: {argv:?}"
        );
        assert_argv_matches_a_spawn_without_priming(&argv);
        std::fs::remove_file(&path).expect("remove the temp brief");
    }

    /// ★cmd expands `%NAME%` **even inside quotes**★ — carried as is, the document the agent reads gets this machine's
    /// environment variable value embedded (substitution succeeded) or the sentence disappears entirely (undefined variable).
    #[test]
    fn a_priming_document_with_a_percent_sign_is_skipped_and_the_spawn_still_stands() {
        let (ep, path) = bake_priming("progress is 50% done");
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let argv = codex_argv(&s);
        assert!(
            developer_instructions_value(&argv).is_none(),
            "a brief containing `%` was carried as is: {argv:?}"
        );
        assert_argv_matches_a_spawn_without_priming(&argv);
        std::fs::remove_file(&path).expect("remove the temp brief");
    }

    /// ★A brief over budget is skipped **without killing the spawn**★ — a command line over the cap is punished not with an error
    /// but with a cut, so the real price of that branch is the MCP attachment and the hook registration disappearing along with it.
    #[test]
    fn an_oversized_priming_document_is_skipped_and_the_spawn_still_stands() {
        let (ep, path) = bake_priming(&"x".repeat(CMD_LINE_LIMIT_UTF16));
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let argv = codex_argv(&s);
        assert!(
            developer_instructions_value(&argv).is_none(),
            "a brief over budget was carried (length {})",
            argv.iter().map(|a| a.len()).sum::<usize>()
        );
        assert_argv_matches_a_spawn_without_priming(&argv);
        std::fs::remove_file(&path).expect("remove the temp brief");
    }

    /// ★The budget is decided by **the remainder** — it does not measure our value alone★: the same brief is skipped on a spawn
    /// with a long pass-through. Reverting to a fixed-constant comparison breaks this item.
    #[test]
    fn the_budget_counts_what_the_rest_of_the_command_line_already_spent() {
        let text = "y".repeat(CMD_LINE_LIMIT_UTF16 / 2);
        let (ep, path) = bake_priming(&text);
        let roomy = spec_with_control(&codex(vec![]), Some(ep.clone()));
        assert!(
            developer_instructions_value(&codex_argv(&roomy)).is_some(),
            "skipped on a command line with room to spare"
        );
        let filler = "z".repeat(CMD_LINE_LIMIT_UTF16 / 2);
        let crowded = spec_with_control(&codex(vec![filler.as_str()]), Some(ep));
        assert!(
            developer_instructions_value(&codex_argv(&crowded)).is_none(),
            "carried it as is on a command line whose budget the pass-through used up"
        );
        std::fs::remove_file(&path).expect("remove the temp brief");
    }

    /// ★It must be **after** the pass-through★ — passing the same key twice with `-c`, the last one wins. If placed in front, one line of user
    /// arguments erases the receive-contract priming without any signal (the same discipline as the hook · MCP attachment).
    #[test]
    fn the_priming_override_follows_the_passthrough() {
        let (ep, path) = bake_priming("teach the reply contract");
        let s = spec_with_control(
            &codex(vec![
                CONFIG_OVERRIDE_FLAG,
                "developer_instructions='theirs'",
            ]),
            Some(ep),
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
        std::fs::remove_file(&path).expect("remove the temp brief");
    }

    /// ★The app-server branch uses not a single character of the command line — and **does not touch** the value★: neither newlines
    /// nor the `'` · `%` the terminal branch cuts out can break anything in the JSON body. Moving that side's conversion here
    /// would block no risk and only break the document the agent reads.
    #[test]
    fn the_app_server_handshake_carries_the_document_unmodified() {
        let text = "line one\nyou can't skip 50% of it\n";
        let (ep, path) = bake_priming(text);
        let s = spec_with_control(&codex_app_server(vec![]), Some(ep.clone()));
        assert!(
            developer_instructions_value(&codex_argv(&s)).is_none(),
            "the app-server branch carried it on the command line too: {:?}",
            codex_argv(&s)
        );
        let params = match thread_open(&s, None, priming_text(Some(&ep))) {
            ThreadOpen::Start(p) => p,
            ThreadOpen::Resume(_) => panic!("resume was picked although it is not a resume"),
        };
        let v = serde_json::to_value(&params).expect("serialize");
        assert_eq!(
            v.get("developerInstructions").and_then(|v| v.as_str()),
            Some(text),
            "the instructions in the handshake JSON differ from the original: {v}"
        );
        std::fs::remove_file(&path).expect("remove the temp brief");
    }

    /// ★A resumed app-server thread gets no priming — a decision, not a regression★
    /// (`ThreadResumeParams`'s doc is the source of truth for the reason). On the day that field is confirmed by measurement and added, this item
    /// goes red and is fixed together with it.
    #[test]
    fn a_resuming_app_server_handshake_carries_no_instructions_field() {
        let (ep, path) = bake_priming("teach the reply contract");
        let s = spec_with_control(&codex_app_server(vec![]), Some(ep.clone()));
        let params = match thread_open(&s, Some(Uuid::new_v4()), priming_text(Some(&ep))) {
            ThreadOpen::Resume(p) => p,
            ThreadOpen::Start(_) => panic!("start was picked although it is a resume"),
        };
        let v = serde_json::to_value(&params).expect("serialize");
        assert!(
            v.get("developerInstructions").is_none(),
            "a never-measured field was carried in the resume request: {v}"
        );
        std::fs::remove_file(&path).expect("remove the temp brief");
    }

    /// A spawn the daemon did not carry priming for (the branch where `wants_priming` is false) does not grow argv by a single slot.
    #[test]
    fn without_a_priming_file_nothing_is_injected() {
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let s = spec_with_control(&command, Some(endpoint_without_priming()));
            assert!(
                developer_instructions_value(&codex_argv(&s)).is_none(),
                "{command:?}: carried although there is no path"
            );
        }
        assert!(priming_text(Some(&endpoint_without_priming())).is_none());
        assert!(priming_text(None).is_none());
    }

    /// ★A read failure is fail-open★ — only a warning is left and the spawn goes ahead. Raised as an error, codex spawns would die entirely
    /// in an install where one brief file went missing.
    #[test]
    fn an_unreadable_priming_file_is_skipped_in_both_modes() {
        let missing =
            std::env::temp_dir().join(format!("engram-priming-absent-{}.md", Uuid::new_v4()));
        let ep = ControlEndpoint {
            priming_file: Some(missing),
            ..endpoint()
        };
        assert!(priming_text(Some(&ep)).is_none(), "a missing file was read");
        for command in [codex(vec![]), codex_app_server(vec![])] {
            let s = spec_with_control(&command, Some(ep.clone()));
            let argv = codex_argv(&s);
            assert!(
                developer_instructions_value(&argv).is_none(),
                "{command:?}: carried although the file is missing"
            );
        }
        assert_argv_matches_a_spawn_without_priming(&codex_argv(&spec_with_control(
            &codex(vec![]),
            Some(ep),
        )));
    }

    /// An empty brief only spends command line and teaches nothing — there is no reason to carry it.
    #[test]
    fn an_empty_priming_file_is_not_injected() {
        let (ep, path) = bake_priming("   \n\t\n");
        assert!(
            priming_text(Some(&ep)).is_none(),
            "an empty file was carried"
        );
        std::fs::remove_file(&path).expect("remove the temp brief");
    }

    /// ★The path still does not reach argv★ — what is carried is **the content**. If the path leaked, codex would take an unknown
    /// word as the first prompt (the `[PROMPT]` positional argument).
    #[test]
    fn the_priming_path_never_reaches_the_command_line() {
        let (ep, path) = bake_priming("teach the reply contract");
        let s = spec_with_control(&codex(vec![]), Some(ep));
        let needle = path.to_string_lossy().into_owned();
        assert!(
            !s.args.iter().any(|a| a.contains(&needle)),
            "the brief path was carried in argv: {:?}",
            s.args
        );
        std::fs::remove_file(&path).expect("remove the temp brief");
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
        );
        assert_eq!(s.cwd, cwd);
        assert_eq!(s.env, env);
    }
}

// ── ADR-0218: session id recovery wiring of the terminal branch (real process) ────────────────────────────
//
// ★What is measured here = one line: 「a terminal spawn flows the recovered id to **the sink it was handed**」★ — this wiring alone
//   could be cut while all the matcher unit tests were green (found 2026-09-21: the comparison target was caught as the wrapper
//   PID so a match never happened), and that defect leaves neither an error nor a log.
// ★The same shape and the same worth as the app-server-side partner (`a_resume_target_makes_the_handshake_issue_thread_resume`)★
//   — that one launches a fake app-server and measures that the id received at the handshake reaches the sink, and this one sets up a real
//   process tree and measures that the id found via the lock holder reaches the same sink.
#[cfg(all(test, windows))]
mod terminal_capture_wiring {
    use super::*;
    use crate::backend::SessionIdSink;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// The process holding the lock must be a **grandchild** — the PID we hold is the `cmd.exe` wrapper.
    /// `cmd.exe /c powershell …` builds that shape for real (powershell opens the file and holds it).
    ///
    /// ★This value comes from the polling schedule — do not shrink it arbitrarily★: recovery scans at 0.5 · 1.5 · 3.5 · 7.5 · 15.5 ·
    ///   **30.5** seconds (that geometric series is `FIRST_DELAY` · `MAX_DELAY` of `thread_lock`). On a 4-way runner, if the first
    ///   hit misses 15.5 seconds the next chance is 30.5 seconds, and if the holder has already left by then the test goes red **depending on
    ///   load**. So it must hold until **after** 30.5 seconds.
    const HOLD_SECONDS: u32 = 45;
    /// A generous upper bound for the time it takes for the holder to come up and the first polling round to run — not a deadline but **the test's give-up line**.
    /// It must include the 30.5-second round above, so it is larger than that.
    const WAIT_LIMIT: Duration = Duration::from_secs(50);
    const LOCK_PATH_ENV: &str = "ENGRAM_TEST_LOCK_PATH";

    /// Waits until some process is in the state of **currently holding** that lock — `false` if not.
    ///
    /// ★Do not measure by file existence (`exists()`) — the test then becomes **vainly green**★: even if the helper
    ///   creates the file and ends right away, the file remains, so 「recovery did not run」 passes without ever
    ///   setting up 「the holder is standing」. A dead lock and a live lock are told apart only by the holder lookup (that split is
    ///   measured for real by the tests of the platform crate's `file_holders`).
    /// ★That the measuring means overlaps with the test subject is intended★ — what is used here is **only one thing**, the holder lookup,
    ///   and none of the decisions (which is ours · is the name an id · where to write it) are used. So even if this premise
    ///   holds, the main assertion can still break independently.
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

    /// ★A resume incarnation **does not start recovery at all**★ — it goes through the same wiring with the same real thing once more, but
    /// fills only `resume_session_id` (ADR-0218 decision 5).
    ///
    /// ★**First write what this item cannot measure**★: what is launched here is not the real codex but a command assembled by hand
    ///   to create a grandchild that holds the lock, so **the argv of that spec has no `resume <id>`.**
    ///   Launching the real codex with resume argv needs a live thread id and an installed codex at the same time,
    ///   and this test bench has neither (the same reason the sibling `production_spec` lane is `#[ignore]`).
    /// ★So the two halves are **tied with the same value inside one item**★ — below, with the same handle,
    ///   `build_spec(Resume, Some(id))` is called to first measure that the argv side actually emits `resume <id>`,
    ///   and then that the `open_spawn` side **does not run** recovery from that handle. That the two halves see the same value
    ///   is guaranteed by the assembly point (`manager::resume_handle_for`).
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
