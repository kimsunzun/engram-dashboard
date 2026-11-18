//! Backend contract test bench — asks every backend the **same question table** and fills the answers by measurement (S21 TRD §3).
//!
//! What this file owns is one question table. When backends grow to a third, [`backend_table`] **gains one row** and
//! the question functions stay as they are — it is not a structure where files grow.
//!
//! **There are two lanes.**
//! - Not ignored: the declaration checks ([`declaration_table_is_filled_for_every_backend`] and the
//!   four per-channel checks) and the trust-record checks (`trust_record_*` · `no_trust_record_*`,
//!   over fixture text). They spawn no process and run as-is in the default regression run and CI.
//! - Everything else is `#[ignore]` — it spawns the real CLI and runs only under `-- --ignored`.
//!
//! ★Why there are non-ignore items = the price of `#[ignore]`★: an `#[ignore]` suite is green even if it evaporates entirely.
//!   At least one item guarding the compile and the table's existence must remain so that silence does not arise.
//!
//! ★Inside the `--ignored` lane a missing binary is a failure, not a skip★: whoever called this lane said "I will
//!   measure", so a quiet green must not be returned. Besides, on Windows the CLI is launched wrapped as `cmd.exe /c <prog>`,
//!   so **even without the binary, `cmd.exe` always launches successfully** — "did the spawn happen" can never
//!   tell presence apart. So [`assert_program_present`] stands before the real spawn.
//!
//! ★**This file does not write the argv of real spawns — it asks the backend and uses that**★: every real spawn below
//!   launches as is the argv that [`production_spec`] draws through **the same path as operation** ([`AgentBackend::build_spec`]).
//!   The old shape wrote the arguments into this file as a string (`-s read-only -a never`), so **this test bench measured
//!   none of the arguments operation actually uses (`--cd` · `-s workspace-write` · `-a on-request`)** — what was measured
//!   and what operation launched differed, while the `backend/codex/` header said "the place that measures that again
//!   is here". Now if the operating argv changes, the real CLI launches **with that changed argv** and this
//!   lane goes through it — there is no place here where a constant is matched against its copy inside the same file.
//!
//! ★**The price of that = the test bench sets up the safety envelope itself**★: the operating argv is `-s workspace-write -a on-request`, so
//!   the commands the model issues **can write** to the working folder. If the items that put in a prompt (Q4·Q5) launched as is, this test bench
//!   would run an agent holding write permission. So [`SafetySwap`] overwrites **only the two policy value fields** in the operating argv
//!   with harmless ones (`read-only` · `never`), and does not launch at all if arguments that bypass the envelope are mixed in.
//!   ★If the flag to overwrite cannot be found in the argv, it fails right there★ — rather than just launching an argv that cannot be made harmless,
//!   it stops. The day operation changes `-s` to `--sandbox`, this lane goes red, and it runs only after a person sets up
//!   the envelope of the new argv again.
//!
//! ★**So what is still unmeasured — stated honestly**★: there is **no observation of a real
//!   model turn** under `-s workspace-write` · `-a on-request`. That would mean this harness
//!   running an agent that holds write access, and harmlessness (read-only · no approvals ·
//!   short prompts · no network work · never this repo as cwd) is this file's premise, so we
//!   do not trade it away. [`q11_production_argv_is_accepted_and_reaches_a_composer`] measures
//!   only "the real CLI accepts the production argv carrying those two values and **reaches a
//!   composer**" — the only keys it sends are the ones that pass the startup screen (the probe
//!   characters of [`PtySession::pass_startup_modal`], their erasure, and one CR); it submits
//!   no prompt (without a model turn that permission never gets a chance to be used). "What
//!   the model can do under that policy" is outside this file, and that cell is empty.
//!
//! ★**Which version was measured is left in the log**★: this CLI updates itself, so an
//!   observation means something only when tied to a version. [`record_version_once`]
//!   asks `--version` **through the production argv** and prints it (latest measurement
//!   = codex-cli 0.163.0, 2026-11-02). ★Not asserting the value is deliberate★ — pinning
//!   it would turn red on every update, and that is a calendar, not a regression.
//!
//! ★**Markers are matched ignoring whitespace layout**([`mentions`])★ — under load this TUI draws a
//!   sentence word by word with cursor moves, so the text left after stripping control sequences
//!   loses its spaces. There `flatten` + `contains` misses the marker, and **the next question then
//!   dies with "the composer never came up"** — a harness malfunction, not the question's answer,
//!   yet both are the same red. The evidence and measurements live in that function's comment.
//!
//! **§3-5 gate** — if the following go red, Phase 1 wiring does not start: **Q1 · Q4 · Q5 · Q8 · Q10**.
//! Q3 · Q6 · Q9 are observations, not gates (whichever way they come out, what differs is one argument or a record, not the *shape*
//! of the wiring). Q7 (human eyes — does it look better in our xterm) is **not covered by this test bench**: whether the bytes
//! contain an alt-screen sequence (Q6) can be measured, but "so is the screen better" cannot be measured here.
//!
//! ★A fact the TRD did not know — a first-visit folder shows a **modal**
//!   first★: a folder-trust prompt is drawn over the composer, and while
//!   it is up keystrokes go to the modal, not the composer.
//!   ★When it appears and what it says are per-version★ — measured on 0.163.0: a git-repo cwd
//!   gets the trust modal, a non-git cwd gets no modal at all, and the wording changed.
//!   (History: 0.153.4 also showed it for a non-git cwd — the old Q8 record.) So every question
//!   about the composer or the cwd first goes through [`PtySession::pass_startup_modal`] until
//!   the composer accepts keys, **modal or not** — that verb never reads the modal's wording.
//!   ★Q8 being on that list is this file's most expensive lesson★ — while a modal holds the
//!   process, "still Running after 5 s" **cannot be false**, so the old Q8, which never
//!   passed the modal, never asked about the cwd policy. That green cell was in fact empty.
//!   **Our spawn entry accepts any folder, so this is not just the harness's problem** — a
//!   codex tab started in an unseen cwd takes no input until a person passes the modal.
//!
//! ★**The real user home is never touched**★: passing the modal makes the CLI **write the folder
//!   trust into its own config file**. Left alone, every harness run would pile dead entries for
//!   deleted paths into the developer's real `~/.codex/config.toml` (it really did — that is why
//!   [`LiveProbe::home_redirect`] exists). So every real spawn in this harness redirects the
//!   CLI's state directory to **a temp home dug fresh per item**, and copies only the files
//!   needed for auth from the real home. The temp home lives ★outside the system temp dir, on a
//!   short path★ — codex requires both (evidence = [`Scratch::new_home`]).
//!   ★That guarantee is measured too★ —
//!   [`q12_working_root_comes_from_the_spawn_argument_and_env_reaches_the_process`] asserts
//!   from both sides that the item's temp path **was written to the temp home and not to the
//!   real home**. This paragraph used to be only a promise, kept by nobody but a human eye.
//!
// hangul-ok-begin
//! It launches real child processes, so it runs with `-- --test-threads=4` (TRD §7 0-2 · `.claude/skill-bindings/qa.md` 「모듈별 명령」).
// hangul-ok-end
//!   ★On a PC where a security-software DLL hooks process creation, lower it to 1★ — the same reason as that document's 「if it still
//!   dies, lower it to 2」 clause, and in this suite one item launches the real CLI several times.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::backend::{
    AgentBackend, ClaudeBackend, CodexBackend, InputEncoder, TransportShape, SUBMIT_PACING,
};
use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::profile::{AgentCommand, AgentOutputFormat, SpawnMode};
use engram_dashboard_agent::transport::pty::PtyTransport;
use engram_dashboard_agent::transport::AgentTransport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};
use engram_dashboard_base::testing::wait_until;

// ── Question table ───────────────────────────────────────────────────────────────────

/// Columns compared without running — every value the backend **reports about itself**. A new backend row compiles only if it fills all these fields
/// (that is the concrete form of "is the table filled").
///
/// ★The fields needed to fill a single row do not end with this struct★ — [`BackendRow::can_resume_stored_session`]
///   and [`BackendRow::session_resume`] also have no default and must be written too for it to compile. They are axes that differ per link, so
///   they live there, and reading it as 「this struct is filled, so the row is complete」 misses those fields.
struct Declared {
    /// ★Mode-independent, so kept as one field★ — today every backend reports the same value on both links, and
    /// [`declarations_are_the_same_across_a_backend_s_two_channels`] below actually measures that invariant. The day they differ,
    /// this field too must be split per link like [`BackendRow::can_resume_stored_session`].
    assigns_session_id: bool,
    supports_control_channel: bool,
    accepts_mcp_config: bool,
    /// Does the daemon **write an mcp-config file** for this backend — an axis split from the field above.
    ///
    /// ★Do not derive it from the field above★: the one above today means 「does it use mail via MCP」, and this field means 「does that program
    ///   read the file we wrote by path」. A row where the two differ (codex) is why this field was set up, and a row with true
    ///   receives a plaintext Bearer token JSON on disk at every spawn (that write failing is fail-closed — spawn aborted).
    // ADR-0209
    writes_mcp_config_file: bool,
    reads_messages: bool,
    /// The sending axis — the counterpart of the receiving axis above. ★A row where the two differ is the asymmetric 「can only send」, so that row's reason
    /// must be written in that backend's folder★.
    uses_mail: bool,
    session_snapshot: bool,
    /// ★This cell has real evidence — it does not stop at matching
    /// declarations★: it claims "cwd and env are used as given", so
    /// [`q12_working_root_comes_from_the_spawn_argument_and_env_reaches_the_process`]
    /// measures each half with a real process (the working root is set
    /// by `--cd` · env = the state directory passed in is really used).
    ///
    /// ★But that ground is attached **only to rows with a real probe**★ — for a row with `probe: None` (claude) this field
    ///   is still only a declaration comparison, and this file has never measured that row's cwd·env. That row's ground lies outside this test bench
    ///   (that backend is the row that chose not to launch the real CLI — the [`BackendRow::probe`] comment).
    session_cwd_env: bool,
    model_select: bool,
    model_temperature: bool,
    model_max_tokens: bool,
}

/// The rule that changes **only the policy values** in the operating argv for the test bench. With this, the argv operation actually uses
/// launches as is while the agent this test bench launches can change nothing.
///
/// ★Why this is not 「writing the arguments again」★: what is written here is not **the expected argv** but **the policy the test bench
///   allows**. Whatever arguments operation issues, the argv itself is given by [`production_spec`], and this table only overwrites
///   the values of the designated fields in it. So if operation adds or changes arguments, this file **launches them as is without
///   knowing**, and the real CLI makes the decision — that is the purpose. As this table grows the test bench diverges from operation by that much, so
///   ★before adding a field, ask 「is this the safety envelope, or an argument I do not want to measure」★.
struct SafetySwap {
    /// `(flag, value the test bench uses)`. ★If the argv lacks that flag, it fails right there★ — rather than just launching an argv that
    /// cannot be made harmless, it stops.
    values: &'static [(&'static str, &'static str)],
    /// Fragments that must not appear anywhere in the argv. Even if one value field is overwritten, **other arguments bypass that wall**
    /// (arguments that undo the sandbox itself · arguments that pass approval automatically · arguments that turn on the network), so those names are blocked separately.
    /// Checked as substrings — caught whether they come as a value or as a flag.
    forbidden: &'static [&'static str],
}

/// Columns asked of a real process. A row with `None` does not launch at all in the `--ignored` lane.
struct LiveProbe {
    /// A bare executable name resolved through PATH. ★No process is launched with this value★ — the spawn argv
    /// is all received from the backend by [`production_spec`]. The only use left here is **one PATH resolution**
    /// (the `where` of [`assert_program_present`]), and its result is the premise of Q10 that shows where the shim chain
    /// starts.
    program: &'static str,
    /// ★Interactive arguments are not written here★ — the operating argv is made by [`production_spec`] asking the backend,
    /// and this table has only **the rules that make that argv harmless**. It is not a field for writing expected arguments.
    safety: SafetySwap,
    /// Q6 — the argument that turns alt-screen off. `None` = a backend without such an argument → **Q6 skips that row**.
    ///
    /// ★Why `Option`★: as a required `&'static str`, a third backend without that argument would have to write
    ///   **a placeholder string** to fill the row, and Q6 would take it as a real argument and spawn. This file's contract
    ///   "third backend = one row" would break right there.
    no_alt_screen_arg: Option<&'static str>,
    /// Q3 — the **words** that CLI emits when it rejects a piped stdin. The exit code alone cannot tell whether it died "because stdin is not a tty"
    /// or for another reason (we give all three fds as pipes). `None` = a backend that does not know that phrase
    /// → only the exit code is looked at.
    stdin_refusal_marker: Option<&'static str>,
    /// The way to move this CLI's **per-user state directory**. `None` = a backend without such a way → the test bench would touch the real
    /// user settings, so admitting that backend row means taking on the restore responsibility separately.
    home_redirect: Option<HomeRedirect>,
    /// Q9 — the flag name with which the caller decides the session id. `None` is the claim "there is no such flag", and
    /// Q9 confirms the absence of [`SESSION_ID_CANDIDATES`] in the `--help` text.
    session_id_flag: Option<&'static str>,
    /// Q4 — a one-line body put in the composer to check the echo. ★If submitted, the model really answers★ — keep only
    /// harmless, short ones.
    submit_body: &'static str,
    /// Q5 — fragments to join with LF. If all fragments are echoed, multiple lines went in whole.
    multiline_parts: &'static [&'static str],
    /// The guidance phrase drawn in place of the composer **when it is empty**.
    ///
    /// ★Why it is needed★: submitting empties the composer — so if this phrase is **drawn again** after putting in the body, that
    ///   input was submitted. Without this signal Q5 could only decide by "did characters other than the body come", but
    ///   the TUI redraws the banner·model name·usage guidance without any input, so those characters **cannot be told apart as
    ///   a response or its own screen** (measured — that noise falsely fired the gate).
    /// `None` = a backend without that signal → Q5 only checks whether the screen
    /// stays static. ★The startup probe ([`PtySession::pass_startup_modal`]) also
    /// stands on this signal★ — with `None` that verb sends no key at all.
    composer_idle_marker: Option<&'static str>,
    /// The **byte sequence** the CLI emits once startup (loading ·
    /// first-visit modal) is over and the session is up. It is a control
    /// sequence, not screen text, so rewording does not affect it.
    /// `None` = a backend without such a signal → [`PtySession::pass_startup_modal`]
    /// decides from the probe alone and can miss a modal that appears late, after loading.
    ready_signal: Option<&'static [u8]>,
    /// Q12 — does this CLI **print its own working-root path on screen**. `false` = it does
    /// not → Q12 skips the screen-side check and looks only at the file-side evidence.
    ///
    /// ★Why yes/no instead of a marker string★: the label next to that path is
    ///   per-version (0.153.4 = a `directory:` line in the header box, 0.163.0 = the
    ///   bare path with no label — measured). The check was never about the label but
    ///   about **whether the folder on screen is the one given via `--cd`**, so Q12
    ///   deliberately runs the process in another folder and compares both leaf names.
    shows_workspace_root: bool,
    /// Q6 — does `--no-alt-screen` **change the PTY bytes**. A field Phase 0 fills by measurement, and the test
    /// measures whether the observation equals this declaration.
    ///
    /// ★Why it is not asserted as 「must differ」★: Q6 is an observation, not a gate, so either value is correct
    ///   (the reason this row is not in §3-5). Writing the value here makes this item go red the day the value changes, telling you to
    ///   re-read §4-11's recommendation — pinning it one way would hide the decision inside the test.
    no_alt_screen_changes_bytes: bool,
}

/// How to turn the CLI's state directory to a temporary home. With this, the test bench **does not touch the developer's real settings**.
struct HomeRedirect {
    /// The name of the environment variable that points at the state directory (codex = `CODEX_HOME`, measured 0.153.4 — when turned to a temporary home
    /// `codex mcp list` does not see the real home's servers).
    env: &'static str,
    /// The default location when that variable is not set (a relative path from the user home). Seed files are scooped from here.
    default_subdir: &'static str,
    /// Files that must be **copied into the temp home for the CLI to stay logged in**.
    /// Without them the TUI shows a login screen instead of the composer and everything
    /// from Q1 on collapses (measured: empty home = `Not logged in`). ★If even one cannot
    /// be copied, the harness stops before spawning★ ([`sandbox_home`]) — the login
    /// screen is a cursor-hidden list, i.e. a screen the startup verb could press CR on.
    ///
    /// ★The copies are credentials★ — [`Scratch`]'s `Drop` deletes **these copies
    ///   first**, then the folder. If a child still holds a file it retries up to a
    ///   limit, and if anything is still left it prints the path to stderr. ★Best
    ///   effort, not a guarantee★ — if the test process is killed, `Drop` never runs and
    ///   the copy stays under `%LOCALAPPDATA%\ebc\` (not a place `cargo clean` clears).
    seed_files: &'static [&'static str],
    /// Files **written fresh** into the temp home —
    /// `(file name, content)`. Applied after the copies.
    ///
    /// ★Why it is needed — measured 2026-09-07★: starting codex 0.153.4 with an empty home shows
    ///   **one more modal after** the folder-trust modal (`Set up the Codex agent sandbox …`).
    ///   Its default choice is `1. Set up default sandbox (requires Administrator permissions)`,
    ///   ★a screen an automated test must never pass with CR★ — passing it requests an
    ///   elevation. And while it is up the composer takes no keys, so Q4·Q5 get no echo and die
    ///   (they really did; that failure is this field's evidence).
    /// ★So we write "the answer the real home already has"★: the developer's real
    ///   `config.toml` already holds `[windows] sandbox = "unelevated"`, so that
    ///   modal does not appear. Production spawns meet that same home, so writing it
    ///   does not dress up the observation — it **matches production conditions**.
    ///   ★But no folder-trust entry (`[projects.*]`) is written★ — that modal is a
    ///   screen production's first-visit tab really meets, so passing it is part of
    ///   what is observed, and the entry's key is chosen by the CLI itself (measured
    ///   0.163.0: not the `C:\Users\<user>~1\…` we pass but **the lower-case long path
    ///   of the repo root**, `c:\users\<user>\…`). Pre-writing it would make this file
    ///   mimic that normalization and need fixing every time the CLI changes it.
    seed_writes: &'static [(&'static str, &'static str)],
    /// Whether the temp home holds the **folder-trust decision** for
    /// `workspace` — if so, that entry as one human-readable line.
    ///
    /// ★Why it is needed★: the startup verb ([`PtySession::pass_startup_modal`]) presses
    ///   CR without reading the modal's wording. So whether the screen passed with CR
    ///   **really was the folder-trust modal** can only be checked by its result — did a
    ///   trust entry appear. Searching the files for the path text (`find_text_under`)
    ///   does not do — it also matches when a warning screen logs that path.
    trusted_entry: fn(home: &Path, workspace: &Path) -> Option<String>,
}

struct BackendRow {
    name: &'static str,
    backend: &'static dyn AgentBackend,
    /// A command sample passed when asking the declarations.
    sample: AgentCommand,
    /// A sample of the **second link** of the same backend. `None` = a backend with only one link.
    ///
    /// ★This is where questions grow, not rows★ — the second link is not another program but the same program launched in another
    ///   shape, so writing another set of `Declared` would copy the same answers into two places. Instead
    ///   [`declarations_are_the_same_across_a_backend_s_two_channels`] asks **whether the two links' answers do not
    ///   differ** — the axes that must differ (link shape·encoding·translator) are measured separately by the questions below it.
    alt_sample: Option<AgentCommand>,
    declared: Declared,
    /// The declaration axis written **per link** = 「can it resume with the stored backend sid」. `.0` =
    /// the answer for [`BackendRow::sample`], `.1` = the answer for [`BackendRow::alt_sample`] (`None` = no second link).
    ///
    /// ★Today no row has the two links' answers differ — why it is still kept per link★: this axis is a field that **has differed
    ///   and can differ again**. The codex row was `(false, Some(true))` for a while (back when terminal mode had no resume
    ///   wiring). Folding it into one field makes such a state **not visible at all** in this table, and moving it to [`Declared`]
    ///   would make [`declarations_are_the_same_across_a_backend_s_two_channels`] false at that time, so the table would have
    ///   to be redone. The two fields being the same now is no ground for removing that preparation.
    can_resume_stored_session: (bool, Option<bool>),
    /// The **caps report field** written in the same per-link shape = `capabilities(cmd).session.resume`.
    ///
    /// ★Do not fold it into one field with the field just above★ — where it is asked differs: the one above is the axis on which the activation entry decides 「launch with
    ///   Resume?」, and this field is the value that reports that result to consumers (frontend·wire). Today
    ///   the two are the same in every row, but if they differ, **a spawn that does not resume is reported as resuming** or the reverse,
    ///   so that mismatch must be visible in this table.
    /// ★Why it is not in [`Declared`]★ — it pairs with the field just above, so it is kept in the same shape. If the two fields differ, the one placed in [`Declared`]
    ///   makes [`declarations_are_the_same_across_a_backend_s_two_channels`] false,
    ///   and that actually happened in the codex row (the doc of the field above). No row differing now is no ground for reverting
    ///   this placement — reverting it means redoing the table the next time they differ.
    session_resume: (bool, Option<bool>),
    probe: Option<LiveProbe>,
}

/// The names whose absence Q9 confirms in `--help`. If even one appears, that backend lets the caller decide the session id,
/// so [`LiveProbe::session_id_flag`] being `None` becomes false.
const SESSION_ID_CANDIDATES: &[&str] = &["--session-id", "--session", "--resume"];

static CLAUDE_BACKEND: ClaudeBackend = ClaudeBackend;
static CODEX_BACKEND: CodexBackend = CodexBackend;

fn backend_table() -> Vec<BackendRow> {
    vec![
        BackendRow {
            name: "claude",
            backend: &CLAUDE_BACKEND,
            sample: AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            alt_sample: Some(AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::StreamJson,
            }),
            declared: Declared {
                assigns_session_id: true,
                supports_control_channel: true,
                accepts_mcp_config: true,
                // Only this row **reads the file by path** (`--mcp-config <path>`) — so it is the only true one.
                writes_mcp_config_file: true,
                reads_messages: true,
                uses_mail: true,
                session_snapshot: false,
                session_cwd_env: true,
                model_select: false,
                model_temperature: false,
                model_max_tokens: false,
            },
            // Both links resume with `--resume <sid>` (measured).
            can_resume_stored_session: (true, Some(true)),
            session_resume: (true, Some(true)),
            // The real claude is not launched: this column is already filled by measurement, and the open item where tests depending on the real claude
            // are **`--skip`ped by fn name** in CI has exactly that shape (TRD §3-2). The test bench does not
            // grow that list.
            probe: None,
        },
        BackendRow {
            name: "codex",
            backend: &CODEX_BACKEND,
            sample: AgentCommand::Codex {
                extra_args: vec![],
                output_format: AgentOutputFormat::Terminal,
            },
            alt_sample: Some(AgentCommand::Codex {
                extra_args: vec![],
                output_format: AgentOutputFormat::StreamJson,
            }),
            declared: Declared {
                // ★A value stamped by measurement★ — the issuer of the session id is codex, so the caller cannot decide it
                //   (`session_id_flag: None` is its counterpart). The canonical reason is in `backend/codex/`.
                //   ★Do not read this field tied to `reads_messages` below★ — an old comment here attached 「turns cannot
                //   be observed, so it is removed from the recipient list」, but that axis was a premise ADR-0116 decision 7
                //   rejected, and now that field is true.
                assigns_session_id: false,
                supports_control_channel: true,
                // ★Read by its name, it is a wrong value — codex still cannot take our mcp-config **file**★.
                //   It is turned on because the daemon derives even the mail channel from this field, and the actual attaching is done
                //   by the single value `-c mcp_servers.engram={…}`. The canonical reason·price is in `backend/codex/`.
                accepts_mcp_config: true,
                // ★The field above is true but this field is false — that split is this field's reason to exist★: codex has no flag
                //   pointing at a file we wrote (measured 0.155.0). While it doubled up, this row received a plaintext token file nobody opens
                //   at every spawn, and that write failing could cut the spawn off. The canonical
                //   reason is that method's comment in `backend/codex/`.
                // ADR-0209
                writes_mcp_config_file: false,
                // ★Do not revert to false — ADR-0116 decision 7 rejected that reason, 「there is no turn
                //   signal」★: the terminal claude already receives in the same place (0 signals). The price (stuck in the middle of a turn
                //   · eaten by a modal widget) is something that decision explicitly accepted, and the canonical one is `backend/codex/`'s
                //   `reads_messages` doc.
                reads_messages: true,
                // ★The two fields of this row are **open together**★ — sending·receiving must stand together to keep ADR-0209 decision 4's
                //   「receive first」 order. Reverting only one side revives the asymmetry that decision meant to remove.
                uses_mail: true,
                session_snapshot: false,
                session_cwd_env: true,
                model_select: false,
                model_temperature: false,
                model_max_tokens: false,
            },
            // ★The two links give the same answer — only the **means** of resuming differ★: the app-server issues `thread/resume` with the
            //   thread id it received and wrote down, and terminal mode launches with the same id put in `codex resume <id>`
            //   (ADR-0208 — that mode's handle is returned by the hook through the control plane).
            //   The two lines splitting **together** is still the point: if only one side turns on, a new thread that was never resumed
            //   is reported as 「resumed」. The canonical reason is the comments of those two methods in `backend/codex/`.
            can_resume_stored_session: (true, Some(true)),
            session_resume: (true, Some(true)),
            probe: Some(LiveProbe {
                program: "codex",
                // Operation launches with `-s workspace-write -a on-request` (that argv is received as is).
                // What changes here is only the **values** of those two fields — `read-only` = run the commands the model issues only in a read-only
                // sandbox · `never` = do not ask a person for approval (if it asked, the automatic lane would stop).
                // The deny list is the arguments with which 「the envelope is pierced even if one value field is overwritten」: the two that undo the sandbox
                // entirely, `--approve-for-me` which automatically bypasses approval via the workspace-write sandbox, and `--search` which turns on
                // the network (this test bench does no network work). Canonical names = `codex --help` 0.153.4.
                safety: SafetySwap {
                    values: &[("-s", "read-only"), ("-a", "never")],
                    forbidden: &[
                        "danger-full-access",
                        "--dangerously-bypass-approvals-and-sandbox",
                        "--dangerously-bypass-hook-trust",
                        "--approve-for-me",
                        "--search",
                    ],
                },
                no_alt_screen_arg: Some("--no-alt-screen"),
                // That sentence of measurement M1 (TRD §2) verbatim. Q3 can assert "because stdin is not a tty" only if these words
                // appear.
                stdin_refusal_marker: Some("stdin is not a terminal"),
                home_redirect: Some(HomeRedirect {
                    env: "CODEX_HOME",
                    default_subdir: ".codex",
                    // Auth alone brings the logged-in state with it
                    // (measured: in an empty home with only this file copied,
                    // `codex login status` = `Logged in using ChatGPT`).
                    // ★`version.json` is not copied★ — if it names a newer release than the
                    // installed one, 0.163.0 shows an update modal right at start (measured: a
                    // home with `latest_version` set to 99.0.0 =
                    // `1. Update now (runs npm install -g @openai/codex)` as the default choice
                    // · cursor hidden). A CR on that screen runs a global `npm install`.
                    seed_files: &["auth.json"],
                    // `check_for_update_on_startup = false` turns off the update check and
                    // its modal (measured 0.163.0: even with a `version.json` naming 99.0.0
                    // no modal appears and the composer comes straight up · without that
                    // file none is created). `[windows] sandbox` is the real home's value
                    // as-is — without it a sandbox-setup modal asking for administrator
                    // rights follows the trust modal (measured — see the field doc).
                    seed_writes: &[(
                        "config.toml",
                        "check_for_update_on_startup = false\n\n[windows]\nsandbox = \"unelevated\"\n",
                    )],
                    trusted_entry: codex_trusted_entry,
                }),
                session_id_flag: None,
                submit_body: "say OK",
                multiline_parts: &["hi", "there"],
                composer_idle_marker: Some("Ask Codex to do anything"),
                // Measured 0.163.0 — once the session is up it announces `state=idle`
                // via OSC 7501 (both alt-screen and `--no-alt-screen`). Not while the
                // trust modal is up; it comes after the modal is accepted.
                ready_signal: Some(b"\x1b]7501;state=idle"),
                // The working-root path is printed in the header box and the bottom
                // line (measured 0.163.0). ★The TUI shortens paths to fit the width, so
                // Q12 measures with **a short leaf name + a wide PTY**★ — at 80 columns
                // the front is cut to `…` (measured on a 0.153.4 Q8 screen dump).
                shows_workspace_root: true,
                // Measured 0.163.0 — the default mode emits alt-screen entry
                // (`?1049h`) and mouse tracking (`?1003;1006h`), and `--no-alt-screen`
                // emits neither. So without the flag this version's TUI is alt-screen.
                no_alt_screen_changes_bytes: true,
            }),
        },
    ]
}

// ── Declaration items (no execution — they run in CI) ─────────────────────────────────────

#[test]
fn declaration_table_is_filled_for_every_backend() {
    let table = backend_table();
    assert!(
        !table.is_empty(),
        "the question table is empty — the test bench is measuring nothing"
    );

    for row in &table {
        assert_declared(row.name, row.backend, &row.sample, &row.declared);
    }

    // ★This also guards that the `#[ignore]` lane does not idle★: if there is not a single real probe row,
    //   Q1~Q10 all go green as an empty loop. This item catches that silence.
    assert!(
        table.iter().any(|r| r.probe.is_some()),
        "there is not a single real probe row — Q1~Q10 quietly pass as an empty loop"
    );
}

/// Compares every declaration column for one sample.
fn assert_declared(name: &str, b: &'static dyn AgentBackend, sample: &AgentCommand, d: &Declared) {
    {
        let row_sample = sample;
        assert_eq!(
            b.assigns_session_id(row_sample),
            d.assigns_session_id,
            "{name}: assigns_session_id"
        );
        assert_eq!(
            b.supports_control_channel(),
            d.supports_control_channel,
            "{name}: supports_control_channel"
        );
        assert_eq!(
            b.accepts_mcp_config(),
            d.accepts_mcp_config,
            "{name}: accepts_mcp_config"
        );
        assert_eq!(
            b.writes_mcp_config_file(),
            d.writes_mcp_config_file,
            "{name}: writes_mcp_config_file — if true, a plaintext token JSON is written to disk at every spawn"
        );
        assert_eq!(b.uses_mail(), d.uses_mail, "{name}: uses_mail");
        assert_eq!(
            b.reads_messages(),
            d.reads_messages,
            "{name}: reads_messages"
        );

        // ★`session.resume` is not measured here★ — it is a field that differs per link, so
        //   [`resume_eligibility_is_declared_per_channel`] measures it per link.
        let caps = b.capabilities(row_sample);
        assert_eq!(
            caps.session.snapshot, d.session_snapshot,
            "{name}: session.snapshot"
        );
        assert_eq!(
            caps.session.cwd_env, d.session_cwd_env,
            "{name}: session.cwd_env"
        );
        assert_eq!(caps.model.select, d.model_select, "{name}: model.select");
        assert_eq!(
            caps.model.temperature, d.model_temperature,
            "{name}: model.temperature"
        );
        assert_eq!(
            caps.model.max_tokens, d.model_max_tokens,
            "{name}: model.max_tokens"
        );
    }
}

/// ★Does the second link not quietly replace the declarations★ — it is the same program launched in another shape, so mail
/// eligibility·control channel·session capabilities must stay the same even when the link changes.
///
/// ★What this item actually measures is only the getters that **take a command**★ — a getter that takes no argument is unrelated
/// to the sample, so it only repeats what [`declaration_table_is_filled_for_every_backend`] above measured and cannot break
/// separately here. The fields that can actually differ today = `capabilities(sample)` and
/// `assigns_session_id(sample)`, the two.
///
/// ★Axes that differ per link are not here★ — this item's claim is 「the two links' answers are the same」, so putting a differing axis
///   into [`Declared`] makes this assertion false. That side is
///   measured per link by [`resume_eligibility_is_declared_per_channel`].
#[test]
fn declarations_are_the_same_across_a_backend_s_two_channels() {
    for row in &backend_table() {
        let Some(alt) = &row.alt_sample else { continue };
        assert_declared(row.name, row.backend, alt, &row.declared);
    }
}

/// Compares the axes that differ per link **per link**. ★Without this item, the second link's answer, even if written in the table, is not
/// called anywhere★ — the two items above look only at [`Declared`], and that column stands on the premise that the two links are the same.
///
/// ★It also catches mismatched pairs★: if a second link is written but its answer left empty (or the reverse), that row is quietly
///   only half measured.
/// ★It measures the two fields **together**★ — the decision axis ([`AgentBackend::can_resume_stored_session`]) and the report field
///   (`capabilities().session.resume`). Measuring only one lets a state where the two differ pass as green, and that state's symptom is
///   「or the reverse」.
#[test]
fn resume_eligibility_is_declared_per_channel() {
    for row in &backend_table() {
        assert_eq!(
            row.backend.can_resume_stored_session(&row.sample),
            row.can_resume_stored_session.0,
            "{}: resume axis mismatch on the first link",
            row.name
        );
        assert_eq!(
            row.backend.capabilities(&row.sample).session.resume,
            row.session_resume.0,
            "{}: resume report field mismatch on the first link",
            row.name
        );
        let alt_expected = (row.can_resume_stored_session.1, row.session_resume.1);
        match (&row.alt_sample, alt_expected) {
            (Some(alt), (Some(axis), Some(caps))) => {
                assert_eq!(
                    row.backend.can_resume_stored_session(alt),
                    axis,
                    "{}: resume axis mismatch on the second link — this field is the only place that mode is visible in this table",
                    row.name
                );
                assert_eq!(
                    row.backend.capabilities(alt).session.resume,
                    caps,
                    "{}: resume report field mismatch on the second link — if it differs from the axis, the report becomes false",
                    row.name
                );
            }
            (None, (None, None)) => {}
            _ => panic!(
                "{}: only one of the second link and its answer is written — if there is a link but the answer is empty, that mode is not measured",
                row.name
            ),
        }
    }
}

/// Prevents **writing down that it has a second link and then writing the same link** — then the item above asks the same sample twice
/// and measures nothing.
///
/// ★The values of the codec axes (encoder·translator·link shape) themselves are measured by the declaration table tripwire in `backend/mod.rs`★ —
/// writing those values again here would make the same fact live in two houses. What this item asks is one thing: **are the two links actually
/// different**.
#[test]
fn a_second_channel_is_actually_a_different_channel() {
    for row in &backend_table() {
        let Some(alt) = &row.alt_sample else { continue };
        let first = row.backend.transport_shape(&row.sample);
        let second = row.backend.transport_shape(alt);
        assert_ne!(
            first, second,
            "{}: the second link reports the same shape as the first ({first:?}) — the samples are the same or the declarations did not split",
            row.name
        );
    }
}

/// A link that launches with pipes **must** come with a translator.
///
/// ★Without it, the bytes go to the screen as is with no error or warning★ — the only runtime symptom is a broken screen.
#[test]
fn every_pipe_channel_declares_an_output_decoder_and_a_non_raw_encoder() {
    for row in &backend_table() {
        for sample in std::iter::once(&row.sample).chain(row.alt_sample.iter()) {
            if row.backend.transport_shape(sample) == TransportShape::Pty {
                continue;
            }
            assert!(
                row.backend.output_decoder(sample).is_some(),
                "{}: it reported pipes but has no translator — the bytes go to the screen as is",
                row.name
            );
            assert_ne!(
                row.backend.input_encoder(sample),
                InputEncoder::Raw,
                "{}: if a pipe link uses the `Raw` encoder, the session emits CR once more after the body",
                row.name
            );
        }
    }
}

// ── Trust-record check (no process — runs in CI) ─────────────────────────────

/// The workspace the trust-record fixtures below point at, in the long form the caller passes.
const TRUST_WORKSPACE: &str = r"C:\Work\Root-1234";

/// A `config.toml` with an unrelated key and table ahead of `projects`, as a real temp home has.
fn trust_config(projects: &str) -> String {
    format!(
        "check_for_update_on_startup = false\n\n[windows]\nsandbox = \"unelevated\"\n\n{projects}"
    )
}

/// The shape codex 0.163.0 writes (measured): literal-string key, lower-case long path.
#[test]
fn trust_record_is_read_from_the_table_codex_writes() {
    let literal = trust_config("[projects.'c:\\work\\root-1234']\ntrust_level = \"trusted\"\n");
    assert_eq!(
        codex_trust_record(&literal, TRUST_WORKSPACE).as_deref(),
        Some("[projects.'c:\\work\\root-1234']")
    );
}

/// The double-quoted key form TOML also allows for the same table. Never seen codex write it —
/// the check accepts it because it names the same key.
#[test]
fn trust_record_is_also_read_from_a_double_quoted_key() {
    let basic = trust_config("[projects.\"c:\\\\work\\\\root-1234\"]\ntrust_level = \"trusted\"\n");
    assert_eq!(
        codex_trust_record(&basic, TRUST_WORKSPACE).as_deref(),
        Some("[projects.\"c:\\\\work\\\\root-1234\"]")
    );
}

/// (a) the CR was accepted but no trust record appeared.
#[test]
fn no_trust_record_is_none() {
    assert_eq!(codex_trust_record(&trust_config(""), TRUST_WORKSPACE), None);
    assert_eq!(codex_trust_record("", TRUST_WORKSPACE), None);
}

/// (b) a record exists, but for another folder — a sibling, the parent or a child.
#[test]
fn trust_record_for_another_folder_is_none() {
    for other in [r"c:\work\away-1234", r"c:\work", r"c:\work\root-1234\sub"] {
        let config = trust_config(&format!(
            "[projects.'{other}']\ntrust_level = \"trusted\"\n"
        ));
        assert_eq!(
            codex_trust_record(&config, TRUST_WORKSPACE),
            None,
            "a record for `{other}` was taken as the workspace's"
        );
    }
}

/// (c) a record for the workspace exists, but with another value or in a shape this check
/// does not read.
#[test]
fn trust_record_with_another_value_or_shape_is_none() {
    // A root-level dotted key must come before any table header, so it cannot go through
    // `trust_config` (whose `[windows]` table would own it).
    let dotted = "projects.'c:\\work\\root-1234'.trust_level = \"trusted\"\n\n[windows]\nsandbox = \"unelevated\"\n";
    assert_eq!(
        codex_trust_record(dotted, TRUST_WORKSPACE),
        None,
        "taken as a trust record: {dotted:?}"
    );
    for projects in [
        "[projects.'c:\\work\\root-1234']\ntrust_level = \"untrusted\"\n",
        "[projects]\n'c:\\work\\root-1234' = { trust_level = \"trusted\" }\n",
        "[projects.'c:\\work\\root-1234']\n[other]\ntrust_level = \"trusted\"\n",
    ] {
        assert_eq!(
            codex_trust_record(&trust_config(projects), TRUST_WORKSPACE),
            None,
            "taken as a trust record: {projects:?}"
        );
    }
}

// ── Shared harness ──────────────────────────────────────────────────────────────

const FIRST_BYTES_TIMEOUT: Duration = Duration::from_secs(30);
const SETTLE_QUIET: Duration = Duration::from_millis(1500);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(25);
/// Q2 · Q8 — "it does not die for this long after the first output".
const ALIVE_WINDOW: Duration = Duration::from_secs(5);
/// Q4 — the bound for waiting for the model response to start after submitting.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(45);
/// Q4 · Q5 — the window that puts in only the body without CR and looks at "has a response already started".
const PRE_SUBMIT_WINDOW: Duration = Duration::from_secs(6);
/// Q6 — the window that collects the initial output of the two modes.
const CAPTURE_WINDOW: Duration = Duration::from_secs(8);
const PROCESS_TIMEOUT: Duration = Duration::from_secs(60);
/// Budget for getting past the startup screen (loading · modal) until the composer takes
/// keys; probes repeat within it. ★Not a hard cap★ — it is checked only at the head of
/// each round, and a round already started (probe windows · waiting for quiet · the
/// post-CR trust-record wait) runs to its end, so a run can overrun it (~54 s observed).
const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
/// Window in which one probe waits for its echo, and then for the idle
/// hint to reappear. Missing it is not a failure, just the next probe.
const PROBE_WINDOW: Duration = Duration::from_secs(2);
/// Probe characters asking whether the composer takes keys. ★Not hex characters★ — they cannot
/// collide with a temp path (uuid) on screen. ★No digits★ — in a list modal a digit selects that
/// numbered choice (trust modal `1. … 2. …`).
const COMPOSER_PROBE: &str = "xz";
/// After the startup verb sends its CR, the limit for its result (the folder-trust entry) to be
/// written into the temp home.
const TRUST_RECORD_TIMEOUT: Duration = Duration::from_secs(10);
/// How long [`Scratch`]'s `Drop` keeps retrying the removal —
/// the same value as Q10's limit for "the tree empties".
const SCRATCH_REMOVE_TIMEOUT: Duration = Duration::from_secs(10);
/// Byte limit of a Unix socket path — `sockaddr_un.sun_path`'s 108 bytes minus the trailing NUL.
const SOCKET_PATH_MAX: usize = 107;
/// The bound for waiting for the composer to be drawn after passing the modal.
const COMPOSER_TIMEOUT: Duration = Duration::from_secs(25);
/// Q4 · Q5 — the bound for waiting for the body put in to be echoed in the composer. ★Why a retry and not a fixed sleep★:
/// if a single first frame is slow, a fixed wait misjudges right there (the same reason the remaining waits are all
/// [`wait_until`]). But Q4 still always fills an interval of [`SUBMIT_PACING`] after that — that interval is the contract being measured.
const ECHO_TIMEOUT: Duration = Duration::from_secs(10);
/// The bound given to [`OutputCore::join_pump`] — ADR-0001's 5 seconds.
const JOIN_PUMP_TIMEOUT: Duration = Duration::from_secs(5);
/// The bound for reclaiming the pipe reader thread. ★No infinite join★ — even after the direct child ends, **if a grandchild holds the pipe's write end**
/// `read_to_end` never ends (the shim chain always has a grandchild).
const READER_JOIN_TIMEOUT: Duration = Duration::from_secs(5);

fn live_rows() -> Vec<BackendRow> {
    let rows: Vec<BackendRow> = backend_table()
        .into_iter()
        .filter(|r| r.probe.is_some())
        .collect();
    assert!(
        !rows.is_empty(),
        "there is no real probe row — this lane measured nothing"
    );
    rows
}

#[derive(Clone)]
struct RecordingSink {
    id: SinkId,
    output: Arc<Mutex<Vec<u8>>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            output: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn len(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    fn snapshot(&self) -> Vec<u8> {
        self.output.lock().unwrap().clone()
    }

    fn tail_from(&self, from: usize) -> Vec<u8> {
        let buf = self.output.lock().unwrap();
        if from >= buf.len() {
            Vec::new()
        } else {
            buf[from..].to_vec()
        }
    }
}

impl OutputSink for RecordingSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        if let OutputPayload::Bytes(b) = frame.payload {
            self.output.lock().unwrap().extend_from_slice(b);
        }
        Ok(())
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
}

struct NoopStatusSink;
impl StatusSink for NoopStatusSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

// ── Operating argv — this file does not write it but receives it from the backend ─────────────────────

/// A command with **caller passthrough arguments** put on `sample`. Where those arguments attach in the argv is decided by the backend —
/// this function only fills the "arguments the user wrote additionally" field.
///
/// ★The `match` being exhaustive is part of the contract★: when backends grow to a third and one more variant appears, the compiler
///   stops here — a person decides once where the new row sends the passthrough field.
fn with_extra(sample: &AgentCommand, extra: &[&str]) -> AgentCommand {
    let mut c = sample.clone();
    let add = |v: &mut Vec<String>| v.extend(extra.iter().map(|s| (*s).to_string()));
    match &mut c {
        AgentCommand::Claude { extra_args, .. } => add(extra_args),
        AgentCommand::Codex { extra_args, .. } => add(extra_args),
        AgentCommand::Shell { args, .. } => add(args),
    }
    c
}

/// **The spec operation actually launches** — drawn through the same path as operation ([`AgentBackend::build_spec`]).
///
/// ★This function is the only ground on which this file can say it 「measures operation」★. If the arguments were written in this file, the day that
///   string diverged from operation the test bench would be green while measuring **something else**, and there would be no gate anywhere catching that
///   mismatch (that actually happened — that paragraph of the header). Receiving it here leaves no place to diverge in the first place.
/// ★Bonus — the place that hand-copied the console wrapping is gone too★: that wrapping (the platform crate's
///   `shell::console_command`) is already applied **inside** `build_spec` and returned as `CommandSpec.program`/`args`.
///   So this file does not need to know that wrapping.
fn production_spec(
    row: &BackendRow,
    cwd: &Path,
    env: &[(String, String)],
    extra: &[&str],
) -> CommandSpec {
    row.backend.build_spec(
        &with_extra(&row.sample, extra),
        SpawnMode::Fresh,
        None,
        None,
        cwd.to_path_buf(),
        env.to_vec(),
        None,
    )
}

/// A spec where **only the policy values** of the operating spec are overwritten with harmless ones. ★Every item that puts in keys launches with this★ —
/// the only place that launches operation as is is [`q11_production_argv_is_accepted_and_reaches_a_composer`],
/// and that item puts in no prompt at all.
fn harnessed_spec(
    row: &BackendRow,
    cwd: &Path,
    env: &[(String, String)],
    extra: &[&str],
) -> CommandSpec {
    let probe = row.probe.as_ref().expect(
        "asked a real spec for a row without a real probe — live_rows() should have filtered it",
    );
    let mut spec = production_spec(row, cwd, env, extra);
    spec.args = safe_args(&spec.args, &probe.safety, row.name);
    spec
}

/// A copy with the policy fields of the operating argv overwritten. If there is no place to overwrite, it **stops with a panic**.
fn safe_args(args: &[String], swap: &SafetySwap, name: &str) -> Vec<String> {
    let mut out = args.to_vec();
    for (flag, safe) in swap.values {
        let at = out.iter().position(|a| a == flag).unwrap_or_else(|| {
            panic!(
                "{name}: the operating argv has no `{flag}` — this test bench must overwrite that field's value with `{safe}` for \
                 the safety envelope to stand, but could not find the place to overwrite. That means the operating arguments changed, so until a person sets up \
                 the envelope of the new argv again, the real CLI is not launched: {out:?}"
            )
        });
        let value_at = at + 1;
        assert!(
            value_at < out.len(),
            "{name}: no value after `{flag}` — the argv shape changed: {out:?}"
        );
        out[value_at] = (*safe).to_string();
    }
    assert_no_forbidden(&out, swap, name);
    out
}

/// Whether the argv has no arguments that bypass the envelope. ★Called both inside [`safe_args`] and where the operating argv is
/// launched as is★ — an axis that overwriting one value field cannot block, so both paths must pass the same wall.
fn assert_no_forbidden(args: &[String], swap: &SafetySwap, name: &str) {
    for bad in swap.forbidden {
        assert!(
            !args.iter().any(|a| a.contains(bad)),
            "{name}: the operating argv contains `{bad}` — even if the policy values are overwritten, this argument bypasses the safety envelope. \
             The real CLI is not launched until a person checks: {args:?}"
        );
    }
}

#[derive(Debug)]
struct Captured {
    exit_code: Option<i32>,
    timed_out: bool,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

/// Runs a child that talks only through pipes. **stdin is always a pipe** — that is the condition Q3 measures, and the remaining
/// calls too must never inherit the console and wait for human input. Past `timeout` it kills the whole tree and
/// sets `timed_out` (the return always comes).
fn run_capture(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: &Path,
    timeout: Duration,
) -> Captured {
    let started = Instant::now();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("{program} spawn failed: {e}"));

    drop(child.stdin.take());

    // ★Drain the pipes with threads★: for output over 4KB like `--help`, when the pipe buffer fills, the child
    //   stops at write and the exit polling never ends.
    //
    // ★Those threads are not reclaimed with `join()`★: even after the direct child ends, if **a grandchild that inherited the pipe's write end**
    //   is alive, `read_to_end` never sees EOF — in the shim chain (`cmd.exe → node → codex.exe`)
    //   there is always a grandchild. So the result is received over a channel and waited for only [`READER_JOIN_TIMEOUT`], and if it does not come, that thread
    //   is left behind (it disappears together when the test process ends).
    let out_reader = child.stdout.take().map(|s| drain_pipe(s));
    let err_reader = child.stderr.take().map(|s| drain_pipe(s));

    let pid = child.id();
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                status = Some(s);
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    kill_tree(pid);
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait failed: {e}"),
        }
    }

    let stdout = out_reader
        .map(|rx| rx.recv_timeout(READER_JOIN_TIMEOUT).unwrap_or_default())
        .unwrap_or_default();
    let stderr = err_reader
        .map(|rx| rx.recv_timeout(READER_JOIN_TIMEOUT).unwrap_or_default())
        .unwrap_or_default();

    Captured {
        exit_code: status.and_then(|s| s.code()),
        timed_out,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        elapsed: started.elapsed(),
    }
}

/// Drains one side of a pipe with a thread and gives the result over a channel. The caller bears the reclaim bound.
fn drain_pipe<R: std::io::Read + Send + 'static>(mut src: R) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = src.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}

/// The shim chain is deeper than the direct child, so killing just one PID leaves the grandchild.
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// ★A missing binary = a failure of this lane★. Because of the `cmd.exe /c <prog>` wrapping, a successful spawn is no evidence of presence,
/// so before spawning it asks for the PATH resolution itself. The resolved path is printed as is as evidence — where the shim chain
/// starts is Q10's premise.
///
fn assert_program_present(row: &BackendRow) {
    let probe = row
        .probe
        .as_ref()
        .expect("asked presence for a row without a real probe");
    let program = probe.program;

    #[cfg(windows)]
    let (finder, args) = (
        "cmd.exe",
        vec!["/c".to_string(), "where".to_string(), program.to_string()],
    );
    #[cfg(not(windows))]
    let (finder, args) = (
        "sh",
        vec!["-c".to_string(), format!("command -v {program}")],
    );

    let found = run_capture(finder, &args, &[], &std::env::temp_dir(), PROCESS_TIMEOUT);
    assert_eq!(
        found.exit_code,
        Some(0),
        "`{program}` is not on PATH — this lane was called to measure, so it does not pass quietly: {found:?}"
    );
    assert!(
        !found.stdout.trim().is_empty(),
        "the resolution result of `{program}` is empty: {found:?}"
    );
    println!(
        "[presence] {program} -> {}",
        found.stdout.trim().replace(['\r', '\n'], " | ")
    );

    record_version_once(row);
}

/// Leaves in the log **which version this lane measured** — asked by putting `--version` on the operating argv.
///
/// ★Why keep the version★: this CLI updates itself, so all the observations below have meaning only when tied to a version. Why it is asked with the
///   operating argv = it must be the version of exactly what we actually launch.
/// ★Not asserting the value is intended★ — pinning the version would go red at every update, and that is a calendar, not a regression.
/// ★Asked only once per process★: in this lane, **how many real child processes it creates** is itself the risk and
///   the cost.
///   Asking per item would double the codex process chains across the whole lane, but the version does not change within one process,
///   so that value **is the same even if asked once** (measured: the edition that asked per item printed the same value 11 times).
/// ★And 「does this CLI accept the operating argv」 is not this function's job★ —
///   [`q11_production_argv_is_accepted_and_reaches_a_composer`] measures it much harder with a real PTY. Measuring
///   the same thing again here would only grow the number of spawns, not the value.
fn record_version_once(row: &BackendRow) {
    static PROBED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    {
        let mut seen = PROBED.lock().unwrap();
        if seen.contains(&row.name) {
            return;
        }
        seen.push(row.name);
    }

    let probe = row
        .probe
        .as_ref()
        .expect("asked the version for a row without a real probe");
    // Asked **turned to a temporary home** — so as not to leave an update check record in the real home.
    let cwd = Scratch::new("version");
    let (_home, env) = sandbox_home(probe, "version", HomeUse::PrintAndExit);
    let spec = harnessed_spec(row, cwd.path(), &env, &["--version"]);
    let ver = run_capture(
        &spec.program,
        &spec.args,
        &spec.env,
        &spec.cwd,
        PROCESS_TIMEOUT,
    );
    let version = ver.stdout.trim().replace(['\r', '\n'], " ");
    assert!(
        ver.exit_code == Some(0) && !version.is_empty(),
        "{}: the operating argv + `--version` did not tell the version — it is unknown which version to tie this lane's \
         observations to: {ver:?}",
        row.name
    );
    println!(
        "[version] {} = {version} (asked with the operating argv)",
        row.name
    );
}

/// What the CLI will do with a temp home — decides whether [`Scratch::new_home`]
/// enforces the socket-path limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeUse {
    /// May start the TUI/app-server, which opens a Unix socket under the home. Every spawn not
    /// known to be print-only uses this.
    Session,
    /// `--version` only — assumed to print and exit without starting the app-server.
    /// ★Unmeasured★. If it does open the socket, an over-limit home path skips the clear
    /// socket-limit panic; if codex then fails, that shows up only as [`record_version_once`]'s
    /// generic "did not report a version" panic (exit code not 0 or empty output), in the first
    /// item of the run to reach it — later items skip the version probe.
    PrintAndExit,
}

/// One temporary folder. ★`Drop` deletes it★ — nothing remains even if an assertion is cut off by a panic.
///
/// ★Why one line deleting at the end of the function does not do★: the assertions in this file exist **to fail**, and failure
///   is an unwind, so it skips that line. It was a structure that left one folder each time the gate went red. Into the temporary home go
///   auth files copied from the real home, so that leak is also a credential leak.
struct Scratch {
    path: PathBuf,
    /// Credentials copied into this folder — `Drop` deletes them before the folder.
    secrets: Vec<PathBuf>,
}

impl Scratch {
    /// ★No real agent is launched with this repository as cwd★ — risks unrelated to what is measured grow. The cwd of real spawns
    /// is all made under here. ★Each item digs its own folder★ — if the same path were shared, under `-- --test-threads=4`
    /// one item's `Drop` would delete **the cwd another item is using right now**.
    fn new(tag: &str) -> Scratch {
        let path =
            std::env::temp_dir().join(format!("engram-backend-contract-{tag}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("failed to create the temporary folder");
        Scratch {
            path,
            secrets: Vec::new(),
        }
    }

    /// The test home is never placed under the system temp dir — codex 0.153.4 refuses to
    /// create its sandbox helper when its home is there and prints a warning (measured).
    /// ★The path is kept short★ — codex 0.163.0 opens a Unix socket at
    /// `app-server-control\app-server-control.sock` under its home; if that path exceeds 108 bytes
    /// (SUN_LEN) the app-server cannot start and neither does codex (measured — the old
    /// `target/tmp/backend-contract-home-<tag>-<uuid>` gave a 152-char socket path).
    /// So the home goes to `%LOCALAPPDATA%\ebc\<tag>-<8 chars>` — the parent of temp
    /// (`%LOCALAPPDATA%\Temp`), hence outside it. On an OS without that variable it is
    /// `CARGO_TARGET_TMPDIR` (= `target/tmp`).
    ///
    /// Panics when `use_` is [`HomeUse::Session`] and the socket path would exceed
    /// [`SOCKET_PATH_MAX`] — such a home would make codex fail to start.
    fn new_home(tag: &str, use_: HomeUse) -> Scratch {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(|dir| PathBuf::from(dir).join("ebc"))
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")));
        let id = Uuid::new_v4().simple().to_string();
        let path = root.join(format!("{tag}-{}", &id[..8]));
        if use_ == HomeUse::Session {
            let socket = path
                .join("app-server-control")
                .join("app-server-control.sock");
            let socket_len = socket.as_os_str().len();
            assert!(
                socket_len <= SOCKET_PATH_MAX,
                "temp home is too long — codex's app-server socket path `{}` is {socket_len} bytes, over \
                 the {SOCKET_PATH_MAX}-byte limit (SUN_LEN 108 minus the trailing NUL). codex would not \
                 start from this home",
                socket.display()
            );
        }
        std::fs::create_dir_all(&path).expect("failed to create the temporary home");
        Scratch {
            path,
            secrets: Vec::new(),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// So as not to mix the axis Q8 measures (is the cwd a git repo) into other questions, everything except Q8 runs in a temporary folder
    /// **that is a git repo**.
    fn git_init(self) -> Scratch {
        git_init_at(&self.path);
        self
    }
}

fn git_init_at(dir: &Path) {
    let init = run_capture(
        "git",
        &["init".to_string(), "-q".to_string()],
        &[],
        dir,
        PROCESS_TIMEOUT,
    );
    assert_eq!(
        init.exit_code,
        Some(0),
        "failed to initialize the temporary git repo: {init:?}"
    );
}

impl Drop for Scratch {
    /// ★Credential copies go first, and removal is retried up to a limit★: right after
    /// teardown a grandchild process still holds files in the home, so
    /// `remove_dir_all` stops midway and leaves the folder half there (measured
    /// 0.163.0: Q6's temp home stayed behind with its `auth.json`). Whatever is still
    /// left is not passed over silently — its path is printed, for a person to delete.
    fn drop(&mut self) {
        let cleared = wait_until(SCRATCH_REMOVE_TIMEOUT, || {
            for f in &self.secrets {
                let _ = std::fs::remove_file(f);
            }
            let _ = std::fs::remove_dir_all(&self.path);
            !self.path.exists()
        });
        if !cleared {
            let left: Vec<&PathBuf> = self.secrets.iter().filter(|f| f.exists()).collect();
            // Not `eprintln!` — libtest captures that and discards it when the test passes,
            // which is exactly when a leftover credential copy would go unreported.
            let _ = writeln!(
                std::io::stderr().lock(),
                "[scratch] could not remove the temp folder within {SCRATCH_REMOVE_TIMEOUT:?}: {} · \
                 credential copies left: {left:?} — delete them by hand",
                self.path.display()
            );
        }
    }
}

/// A pair of environment variables that turns this backend's state directory to **a temporary home just for this item**. For a backend without a home it is
/// an empty list, and then the real user settings are used as is (that fact is the warning of [`LiveProbe::home_redirect`]).
///
/// **The caller must hold** the returned [`Scratch`] — dropping it deletes the home right there.
/// This CLI's **real user state directory** — that one if the variable is already set, otherwise the default location
/// under the user home. ★Read only★. The place that scoops the seed files and the place that checks "is the real home not dirtied" must see the same
/// answer, so they are kept in one function.
fn real_home(redirect: &HomeRedirect) -> Option<PathBuf> {
    std::env::var_os(redirect.env)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|h| PathBuf::from(h).join(redirect.default_subdir))
        })
        .filter(|p| p.is_dir())
}

fn sandbox_home(
    probe: &LiveProbe,
    tag: &str,
    use_: HomeUse,
) -> (Option<Scratch>, Vec<(String, String)>) {
    let Some(redirect) = probe.home_redirect.as_ref() else {
        return (None, Vec::new());
    };

    let mut home = Scratch::new_home(tag, use_);

    // ★If even one seed file cannot be copied, stop before spawning★ — started without it, the CLI
    //   shows a login screen first, a screen the startup verb could press CR on
    //   ([`HomeRedirect::seed_files`]).
    let real = real_home(redirect).unwrap_or_else(|| {
        panic!(
            "the real user state directory (`{}` or ~/{}) does not exist — there is no auth to move into the temporary home",
            redirect.env, redirect.default_subdir
        )
    });
    for f in redirect.seed_files {
        let to = home.path().join(f);
        // Registered **before** the attempt, so a half-failed copy's remains are deleted too.
        home.secrets.push(to.clone());
        std::fs::copy(real.join(f), &to).unwrap_or_else(|e| {
            panic!("could not move `{f}` into the temporary home ({e}) — not launching")
        });
    }
    // This comes after the copy, so with the same name this side wins.
    for (name, body) in redirect.seed_writes {
        std::fs::write(home.path().join(name), body)
            .expect("failed to write a seed file into the temporary home");
    }

    let env = vec![(
        redirect.env.to_string(),
        home.path().to_string_lossy().into_owned(),
    )];
    (Some(home), env)
}

/// codex's folder-trust record for `workspace` in the temp home's `config.toml`
/// ([`HomeRedirect::trusted_entry`]). Reads the file and resolves `workspace`
/// to its long canonical form, then defers to [`codex_trust_record`]. `None`
/// also when the file or the folder cannot be read.
fn codex_trusted_entry(home: &Path, workspace: &Path) -> Option<String> {
    let config = std::fs::read_to_string(home.join("config.toml")).ok()?;
    let workspace = dunce::canonicalize(workspace).ok()?;
    codex_trust_record(&config, &workspace.to_string_lossy())
}

/// The header line of the `[projects.'<path>']` table in `config` whose path equals `workspace` and
/// that holds `trust_level = "trusted"`; `None` when there is no such table.
///
/// ★Paths are compared case-insensitively★ — 0.163.0 writes the lower-case long path
///   (`c:\users\<user>\…`), not the 8.3 short form we pass (`C:\Users\<user>~1\…`) (measured). The
///   caller passes the long form.
/// ★Only this one shape is read★ (the shape 0.163.0 writes — measured). A version that writes
///   another shape yields `None`, so the startup verb stops loudly instead of passing silently.
fn codex_trust_record(config: &str, workspace: &str) -> Option<String> {
    let want = workspace.to_lowercase();
    let project_key = |head: &str| -> Option<String> {
        let inner = head.strip_prefix("[projects.")?.strip_suffix(']')?;
        if let Some(literal) = inner.strip_prefix('\'').and_then(|k| k.strip_suffix('\'')) {
            return Some(literal.to_string());
        }
        let basic = inner.strip_prefix('"').and_then(|k| k.strip_suffix('"'))?;
        Some(basic.replace("\\\\", "\\"))
    };
    let mut table: Option<&str> = None;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            table = project_key(line)
                .filter(|k| k.to_lowercase() == want)
                .map(|_| line);
            continue;
        }
        if let Some(head) = table {
            if line.replace(' ', "") == "trust_level=\"trusted\"" {
                return Some(head.to_string());
            }
        }
    }
    None
}

/// Result of trying to get past the startup screen. ★Why not a `bool`★: "there
/// was no modal" and "this backend has no way to ask" are not the same value —
/// the first is an observation, the second means there was nothing to ask.
/// ★Whether the composer came up is not here★ — [`Startup::composer`] holds that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModalOutcome {
    /// The table has no idle composer hint for this backend —
    /// no signal to build a probe on, so no key was sent.
    NotDeclared,
    /// No CR was sent — no screen covering the composer was seen.
    NotShown,
    /// A screen covered the composer, one CR picked its default choice, and
    /// as a result a folder-trust entry was written into the temp home.
    Passed,
}

/// The signal **still missing last** when the startup verb did not reach the composer in
/// time — lets the failure text say where it stopped instead of lumping it into "no composer".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupStall {
    /// The startup-done signal ([`LiveProbe::ready_signal`]) never came — loading did not finish,
    /// an unknown screen is covering, or the CLI may have stopped emitting the ready signal.
    NoReadySignal,
    /// The probe characters were not echoed — keys do not reach the composer.
    NoProbeEcho,
    /// Echoed, but after erasing them the idle composer hint was not drawn again.
    NoRedraw,
}

impl StartupStall {
    fn describe(self, idle_marker: &str) -> String {
        match self {
            StartupStall::NoReadySignal => "the startup-done signal never came (loading did not \
                 finish, an unknown screen is covering, or the CLI may have stopped emitting the \
                 ready signal)"
                .to_string(),
            StartupStall::NoProbeEcho => {
                "the probe characters were not echoed (keys do not reach the composer)".to_string()
            }
            StartupStall::NoRedraw => format!(
                "the probe characters were echoed, but after erasing them the idle composer hint \
                 (`{idle_marker}`) was not drawn again"
            ),
        }
    }
}

/// The result of [`PtySession::pass_startup_modal`].
struct Startup {
    outcome: ModalOutcome,
    /// Where the output **without** the consumed modal starts — the accumulated length right before
    /// the CR if one was sent, else 0. The modal's text also contains the workspace path (measured
    /// 0.163.0), so matching the workspace folder on screen looks only past this point (Q12).
    settled_from: usize,
    /// `Ok` = the composer took keys; the value is the accumulated length right
    /// before the passing probe wrote its erasure (after it the idle composer
    /// hint was drawn again). `Err` = not reached in time. When the table has
    /// no composer hint (`NotDeclared`) it is the end position at that moment.
    composer: Result<usize, StartupStall>,
    /// The folder-trust entry that appeared in the temp home after the CR
    /// ([`HomeRedirect::trusted_entry`]).
    trust_entry: Option<String>,
}

/// One incarnation on a real PTY. `Drop` bears the teardown — no child remains even if an assertion is cut off by a panic.
struct PtySession {
    transport: Box<dyn AgentTransport>,
    core: Arc<OutputCore>,
    sink: RecordingSink,
    child_pid: Option<u32>,
    down: bool,
}

impl PtySession {
    fn open(spec: &CommandSpec, cols: u16, rows: u16) -> PtySession {
        let (pty, child_pid) = PtyTransport::open(spec, cols, rows)
            .unwrap_or_else(|e| panic!("PTY open failed: {e:?}"));
        let status_sink: Arc<dyn StatusSink> = Arc::new(NoopStatusSink);
        let core = Arc::new(OutputCore::new(
            Uuid::new_v4(),
            0,
            status_sink,
            TurnWiring::detached(),
        ));
        let transport: Box<dyn AgentTransport> = Box::new(pty);
        transport.start(core.clone());
        let sink = RecordingSink::new();
        let _sid = core.subscribe(Arc::new(sink.clone()));
        PtySession {
            transport,
            core,
            sink,
            child_pid,
            down: false,
        }
    }

    fn write(&self, bytes: &[u8]) {
        self.transport
            .send_input(InputEvent::Raw(bytes.to_vec()))
            .expect("send_input failed");
    }

    /// ADR-0001's two verbs as they are: `shutdown()` → `join_pump(5s)`. Idempotent, so it may overlap with `Drop`.
    fn teardown(&mut self) {
        let _ = self.teardown_measured();
    }

    /// Calls the same two verbs **while timing each**. Returns `(shutdown duration, join_pump duration)`. `None` if already brought down.
    ///
    /// ★Why time it — this is the only window observing Q10's second verb★: [`OutputCore::join_pump`]
    ///   returns `()` and **discards** the result of `recv_timeout` inside (`src/output_core.rs:383-392`). So
    ///   "the pump ended" and "it just waited 5 seconds" **look the same** to the caller. Only the duration tells the two
    ///   apart — if it stuck to the bound, the pump did not end.
    fn teardown_measured(&mut self) -> Option<(Duration, Duration)> {
        if self.down {
            return None;
        }
        self.down = true;
        let t0 = Instant::now();
        self.transport.shutdown();
        let shutdown_took = t0.elapsed();
        let t1 = Instant::now();
        self.core.join_pump(JOIN_PUMP_TIMEOUT);
        Some((shutdown_took, t1.elapsed()))
    }

    fn wait_first_bytes(&self) -> bool {
        wait_until(FIRST_BYTES_TIMEOUT, || self.sink.len() > 0)
    }

    /// All the bytes that came so far, as human-readable characters.
    fn screen(&self) -> String {
        visible_text(&self.sink.snapshot())
    }

    /// Only the bytes that came after `from`, as human-readable characters.
    fn tail_text(&self, from: usize) -> String {
        visible_text(&self.sink.tail_from(from))
    }

    /// Waits until `needle` appears in the output after `from`. ★Goes through [`flatten`]★ — the TUI folds lines inside boxes
    /// and aligns them with padding, so matching against the raw text as is goes off at the folded places.
    fn wait_for_text(&self, from: usize, needle: &str, timeout: Duration) -> bool {
        wait_until(timeout, || mentions(&self.tail_text(from), needle))
    }

    /// Goes past the startup screen (loading · first-visit modal) **until the composer
    /// takes keys**. If a modal is up, it is passed once with its default choice.
    ///
    /// ★The modal's wording is never read★: it changed across versions (0.153.4 `Do you trust …`
    ///   → 0.163.0 `Trust this folder?`) — relying on it would turn this harness red on every
    ///   update and make a person fix the marker. So only three wording-free signals are used.
    ///   ① **Is startup over** = [`LiveProbe::ready_signal`]. ★The probe alone
    ///      cannot tell this★ — the composer takes keys even while loading, and
    ///      the trust modal appears **after** loading ends (measured 0.163.0: the
    ///      modal came up after the probe had passed, and Q4's body went into it).
    ///   ② **Does the composer take keys** = type the probe characters
    ///      ([`COMPOSER_PROBE`]), see the echo, and check that the idle composer hint is
    ///      drawn again **after** erasing them. A modal does not draw those characters
    ///      into the composer, so this holds only when the composer is in front.
    ///   ③ **Is something covering the composer** = the screen went quiet
    ///      with the cursor hidden. Measured 0.163.0: loading · composer =
    ///      cursor shown (`?25h`), folder-trust modal = hidden (`?25l`).
    /// When ③ holds a CR is sent — the folder-trust modal's default choice is to accept
    ///   (measured 0.153.4 `1. Yes, continue` · 0.163.0 `1. Trust and continue`). After
    ///   it, ① is looked for again **in the output after the CR**.
    ///
    /// ★What is guarded and what is not★ — the CR is pressed
    /// **before the screen is recognised**. So:
    /// - **Guarded (environment)**: the other cursor-hiding first screens we know of are kept
    ///   from appearing in the temp home at all — the update modal
    ///   (`check_for_update_on_startup = false` · `version.json` not copied), the administrator
    ///   sandbox-setup modal (`[windows] sandbox`), the login screen (if the auth copy fails,
    ///   nothing is spawned). Evidence = the two seed fields of [`HomeRedirect`].
    /// - **Guarded (post-check)**: within [`TRUST_RECORD_TIMEOUT`] after the CR
    ///   a folder-trust entry for `workspace` must appear in the temp home
    ///   ([`HomeRedirect::trusted_entry`]). If not, the run stops right there
    ///   and says so — every item that calls this verb goes through this check.
    /// - **Only one CR** — a second covering screen gets no key. The composer does not come up and
    ///   [`Startup::composer`] comes back `Err`.
    /// - **Not guarded**: a first screen the environment does not block — the CLI
    ///   showing a login screen because the copied auth expired, an update check
    ///   re-enabled under another key, or any **new** screen — gets the CR, and whatever
    ///   its default choice runs has already happened with that CR. The post-check
    ///   reports it (stops the run and prints the screen) but cannot undo it.
    /// ★Cost★: that choice leaves a folder-trust entry in the CLI's config file — so the CR is
    ///   sent only when there is a temp home (`home`). The real user config is never changed.
    ///
    /// `workspace` = the folder the CLI will use as its working root (= the folder
    /// the trust entry must point at). It can differ from the process cwd (Q12).
    ///
    /// Why the position in [`Startup::composer`] is needed: this TUI draws the modal
    /// **over** the composer, so the idle composer hint is on screen **before** the
    /// modal (measured on a 0.153.4 Q8 screen dump). Searching for the hint including
    /// that earlier stretch would mean "saw the composer hidden behind the modal",
    /// not "passed the modal and the composer came up" — a check that asks nothing.
    fn pass_startup_modal(
        &self,
        probe: &LiveProbe,
        home: Option<&Scratch>,
        workspace: &Path,
    ) -> Startup {
        let mut startup = Startup {
            outcome: ModalOutcome::NotDeclared,
            settled_from: 0,
            composer: Ok(self.sink.len()),
            trust_entry: None,
        };
        let Some(marker) = probe.composer_idle_marker else {
            return startup;
        };
        startup.outcome = ModalOutcome::NotShown;
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let mut stall = StartupStall::NoReadySignal;
        while Instant::now() < deadline {
            let ready = probe
                .ready_signal
                .is_none_or(|sig| contains_seq(&self.sink.tail_from(startup.settled_from), sig));
            if ready {
                match self.probe_composer(marker) {
                    Ok(erase_at) => {
                        startup.composer = Ok(erase_at);
                        return startup;
                    }
                    Err(missing) => stall = missing,
                }
            } else {
                stall = StartupStall::NoReadySignal;
            }
            self.wait_quiet();
            if startup.outcome == ModalOutcome::NotShown && cursor_hidden(&self.sink.snapshot()) {
                let (redirect, home) = probe.home_redirect.as_ref().zip(home).expect(
                    "cannot send CR to the start screen without a temporary home — there is no way to check the accepted screen",
                );
                startup.settled_from = self.sink.len();
                self.write(b"\r");
                startup.outcome = ModalOutcome::Passed;
                wait_until(TRUST_RECORD_TIMEOUT, || {
                    startup.trust_entry = (redirect.trusted_entry)(home.path(), workspace);
                    startup.trust_entry.is_some()
                });
                assert!(
                    startup.trust_entry.is_some(),
                    "sent a CR to a screen that hid the cursor, but within {TRUST_RECORD_TIMEOUT:?} the temp \
                     home (`{}`) got no trust record for the workspace `{}`. One of three things happened: \
                     (1) the screen was not the folder-trust modal — a person must check what its default \
                     choice ran; (2) the CLI trusted a different folder; (3) the CLI wrote the record in a \
                     shape `HomeRedirect::trusted_entry` does not read. Screen: {}",
                    home.path().display(),
                    workspace.display(),
                    excerpt(&self.tail_text(0), 1200)
                );
                self.wait_quiet();
            }
        }
        startup.composer = Err(stall);
        startup
    }

    /// One probe — the accumulated length right before the erasure if the composer
    /// took the keys, else the missing signal. ★The erasure is always sent, echo or
    /// not★: if the echo arrives after the window, the characters stay in the
    /// composer, the next probe's erasure falls short, and the hint never comes back.
    fn probe_composer(&self, marker: &str) -> Result<usize, StartupStall> {
        let typed_at = self.sink.len();
        self.write(COMPOSER_PROBE.as_bytes());
        let echoed = wait_until(PROBE_WINDOW, || {
            mentions(&self.tail_text(typed_at), COMPOSER_PROBE)
        });
        let erase_at = self.sink.len();
        self.write(&b"\x7f".repeat(COMPOSER_PROBE.len()));
        if !echoed {
            return Err(StartupStall::NoProbeEcho);
        }
        if !self.wait_for_text(erase_at, marker, PROBE_WINDOW) {
            return Err(StartupStall::NoRedraw);
        }
        Ok(erase_at)
    }

    /// The real TUI keeps redrawing for a while even after it launches. If input is put in after seeing only the first byte, the composer may not
    /// exist yet, so it waits **until the output stops** before going to the next step. Returns = the cumulative length at the moment it stopped.
    fn wait_quiet(&self) -> usize {
        let deadline = Instant::now() + SETTLE_TIMEOUT;
        let mut last = self.sink.len();
        let mut since = Instant::now();
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            let now = self.sink.len();
            if now != last {
                last = now;
                since = Instant::now();
            } else if since.elapsed() >= SETTLE_QUIET {
                break;
            }
        }
        last
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.teardown();
    }
}

/// Did the startup verb **reach the composer** — if not, stop right here, printing which signal was
/// missing and the screen. No further waiting (the startup verb's budget already spent it). Returns
/// `Some(true)` for a backend whose table has a composer hint, else `None`.
///
/// ★Before asking about the composer, ask whether there is a composer★: otherwise the key input goes to something that is not the composer (the next modal·
///   loading screen), and the "there is no echo" that comes out then is **not this question's answer but a harness malfunction**, yet both
///   are equally red. In fact codex showed one more modal in an empty home, and then Q5 died saying 「the fragments are not visible in the
///   composer」 — that is, as if it knew the answer. This gate tells the two apart.
fn assert_composer(
    s: &PtySession,
    probe: &LiveProbe,
    startup: &Startup,
    name: &str,
) -> Option<bool> {
    let marker = probe.composer_idle_marker?;
    if let Err(missing) = startup.composer {
        panic!(
            "{name}: could not get past the start screen within {STARTUP_TIMEOUT:?} (modal={:?}) — {}. What is measured here is the composer \
             contract, so if the composer does not take keys, every decision below is meaningless: {}",
            startup.outcome,
            missing.describe(marker),
            excerpt(&s.screen(), 1200)
        );
    }
    Some(true)
}

/// TUI output is mostly control sequences. To measure "what came newly" only in human-readable characters, strip them first.
fn visible_text(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut chars = text.chars().peekable();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            if c == '\n' || c == '\t' || !c.is_control() {
                out.push(c);
            }
            continue;
        }
        match chars.next() {
            // CSI — up to the final byte `@`..`~`.
            Some('[') => {
                for f in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&f) {
                        break;
                    }
                }
            }
            // OSC and string sequences — up to BEL or ST (ESC \).
            Some(']') | Some('P') | Some('X') | Some('^') | Some('_') => {
                let mut prev_esc = false;
                for f in chars.by_ref() {
                    if f == '\u{7}' || (prev_esc && f == '\\') {
                        break;
                    }
                    prev_esc = f == '\u{1b}';
                }
            }
            _ => {}
        }
    }
    out
}

/// The **readable characters** left after removing `needles` from `text`. If not empty, "new output other than that text"
/// came (indirect decision ② of TRD §3-3).
fn printable_residue(text: &str, needles: &[&str]) -> String {
    let mut cleaned = text.to_string();
    for n in needles {
        cleaned = cleaned.replace(n, " ");
    }
    cleaned.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Screen text as one line without line breaks·alignment spaces. The TUI folds lines inside boxes, so matching a marker against the raw text
/// as is goes off at the folded places.
fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The place where `parts` appear **one after another with nothing between them**. Returns = an excerpt with a little of what is before and after attached.
///
/// ★This is exactly what Q5 measures★: the fragments all being visible somewhere on the screen and being **joined in the composer**
///   are different facts. The former is true even if each line was submitted and scattered between responses, so that decision
///   cannot claim "the multiple lines went in whole" — if they had been submitted split, model responses·spinners·
///   prompt markers would be drawn **between** the fragments and break the adjacency. The measured conclusion (`› hi there`) is the shape of this predicate.
///
/// ★The limit, written honestly — 「is it the same screen row」 cannot be measured with this byte stream★: the TUI moves rows not with line-break
///   characters but with CSI cursor moves, so the `\n` of [`visible_text`] **does not correspond to screen rows** (measured:
///   what was drawn on one row as `› hi there` was cut into two lines in the text). Restoring screen rows needs a terminal
///   emulator. So this predicate measures not 「the same row」 but **「nothing else in between」**.
/// Does `haystack` say `needle` — ★looks at it ignoring the placement of whitespace★. Every marker comparison uses this.
///
/// ★Why [`flatten`] + `contains` is not enough — measured 2026-09-08★: this TUI draws the
///   same sentence **two ways**. When idle it writes real spaces between words; under load
///   it **moves the cursor for each word**. In the latter the text left after stripping
///   control sequences has no spaces at all (`Doyoutrustthecontents…`), so the marker
///   (`Do you trust`) misses. Back when [`PtySession::pass_startup_modal`] found the modal
///   by its wording, it then read "no modal", sent no CR, and the next question died on the
///   screen the modal still held with "the composer never came up" — ★a harness malfunction,
///   not the question's answer, yet both are the same red★. That is exactly the trap the
///   [`assert_composer`] comment warned about, and we fell in: the same lane lost 3 items to
///   it in one run and was all green in another (what varied between runs = machine load).
/// ★Removing whitespace does not blunt the marker★: what remains is in the same character order, so to match by accident it would have to be the same
///   sentence differing only in whitespace. [`contiguous_echo`] already looks at both joins `" "` and `""` for the same reason.
fn mentions(haystack: &str, needle: &str) -> bool {
    let squash = |t: &str| -> String { t.chars().filter(|c| !c.is_whitespace()).collect() };
    squash(haystack).contains(&squash(needle))
}

fn contiguous_echo(text: &str, parts: &[&str]) -> Option<String> {
    let flat = flatten(text);
    // Whether the composer joins the fragments with a space (when the line break was folded into a space) or puts them together, both are "nothing in between".
    for sep in [" ", ""] {
        let joined = parts.join(sep);
        if joined.is_empty() {
            continue;
        }
        if let Some(at) = flat.find(&joined) {
            let from = flat[..at]
                .char_indices()
                .rev()
                .nth(24)
                .map_or(0, |(i, _)| i);
            return Some(flat[from..].chars().take(80).collect());
        }
    }
    None
}

fn excerpt(text: &str, max: usize) -> String {
    let flat = flatten(text);
    if flat.chars().count() <= max {
        flat
    } else {
        let head: String = flat.chars().take(max).collect();
        format!("{head}...")
    }
}

fn contains_seq(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Did the output end with the cursor **hidden** — is the last DECTCEM (`ESC [ ? 25 h|l`) an `l`.
fn cursor_hidden(raw: &[u8]) -> bool {
    let last = |seq: &[u8]| raw.windows(seq.len()).rposition(|w| w == seq);
    match (last(b"\x1b[?25l"), last(b"\x1b[?25h")) {
        (Some(hide), Some(show)) => hide > show,
        (hidden, _) => hidden.is_some(),
    }
}

/// Every DEC private mode set/reset (`ESC [ ? <params> <h|l>`) in the output, in the order they appeared, without duplicates.
/// The evidence by which Q6 shows a person "where the bytes of the two modes differ".
fn private_modes(raw: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i + 3 < raw.len() {
        if raw[i] != 0x1b || raw[i + 1] != b'[' || raw[i + 2] != b'?' {
            i += 1;
            continue;
        }
        let mut j = i + 3;
        while j < raw.len() && (raw[j].is_ascii_digit() || raw[j] == b';') {
            j += 1;
        }
        if j < raw.len() && (raw[j] == b'h' || raw[j] == b'l') {
            let seq = format!(
                "?{}{}",
                String::from_utf8_lossy(&raw[i + 3..j]),
                raw[j] as char
            );
            if !out.contains(&seq) {
                out.push(seq);
            }
        }
        i = j.max(i + 1);
    }
    out
}

/// Every live descendant PID under `root` (excluding root itself).
fn descendants(root: u32) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    let mut frontier = vec![root];
    while let Some(p) = frontier.pop() {
        for c in engram_dashboard_platform::process::child_pids(p) {
            if !out.contains(&c) {
                out.push(c);
                frontier.push(c);
            }
        }
    }
    out
}

fn pid_alive(pid: u32) -> bool {
    engram_dashboard_platform::process::pid_alive(pid)
}

/// Everything among `known` and `root` that is still alive — ★walks again under what is alive★ to catch even descendants born **after** the
/// preceding enumeration.
///
/// ★Why re-checking the snapshot is not enough★: looking again only at the list counted once just before shutdown, a grandchild born after that
///   enumeration **survives with nobody looking**. What Q10 measures is "is the tree empty", and through that hole a non-empty
///   tree becomes green.
/// ★The remaining limit is written down★: an orphan whose parent already died cannot be found by this tree walk — this predicate sees only
///   descendants whose chain is unbroken.
fn alive_survivors(root: u32, known: &[u32]) -> Vec<u32> {
    let mut seen: Vec<u32> = std::iter::once(root).chain(known.iter().copied()).collect();
    let mut i = 0usize;
    while i < seen.len() {
        let p = seen[i];
        i += 1;
        if !pid_alive(p) {
            continue;
        }
        for c in descendants(p) {
            if !seen.contains(&c) {
                seen.push(c);
            }
        }
    }
    seen.retain(|p| pid_alive(*p));
    seen
}

// ── Q1 — does it launch under a real PTY (★gate★) ────────────────────────────────────

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q1_comes_up_under_real_pty() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);

        let cwd = Scratch::new("q1").git_init();
        let (_home, env) = sandbox_home(probe, "q1", HomeUse::Session);
        let mut s = PtySession::open(&harnessed_spec(&row, cwd.path(), &env, &[]), 80, 24);
        let started = Instant::now();
        let got = s.wait_first_bytes();
        let first_at = started.elapsed();
        let len = s.sink.len();
        let screen = s.screen();
        s.teardown();

        println!(
            "[Q1 {}] first byte {}: {len} bytes after {:?}\n  screen: {}",
            row.name,
            if got { "received" } else { "not received" },
            first_at,
            excerpt(&screen, 400)
        );
        assert!(
            got,
            "{}: no first byte within {FIRST_BYTES_TIMEOUT:?} — ★§3-5 gate★ the whole of Phase 1 collapses",
            row.name
        );
    }
}

// ── Q2 — is it alive after the first output ─────────────────────────────────────────────

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q2_stays_alive_after_first_output() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);

        let cwd = Scratch::new("q2").git_init();
        let (_home, env) = sandbox_home(probe, "q2", HomeUse::Session);
        let mut s = PtySession::open(&harnessed_spec(&row, cwd.path(), &env, &[]), 80, 24);
        let got_first = s.wait_first_bytes();
        std::thread::sleep(ALIVE_WINDOW);
        let status = s.core.status();
        let len = s.sink.len();
        s.teardown();

        println!(
            "[Q2 {}] status={status:?} at {ALIVE_WINDOW:?} after the first output · cumulative {len} bytes",
            row.name
        );
        assert!(
            got_first,
            "{}: there is no first byte, so Q2 cannot be asked",
            row.name
        );
        assert!(
            matches!(status, AgentStatus::Running),
            "{}: exited within {ALIVE_WINDOW:?} after the first output — status={status:?}",
            row.name
        );
    }
}

// ── Q3 — piped (non-TTY) stdin ───────────────────────────────────────────────

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q3_non_tty_stdin_is_refused() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);

        let cwd = Scratch::new("q3").git_init();
        let (_home, env) = sandbox_home(probe, "q3", HomeUse::Session);
        // ★Asked with the operating argv as is (only the policy values made harmless)★ — the axis of this question is stdin, but the answer must be
        //   the answer under the argv we actually launch.
        let spec = harnessed_spec(&row, cwd.path(), &env, &[]);
        let out = run_capture(
            &spec.program,
            &spec.args,
            &spec.env,
            &spec.cwd,
            PROCESS_TIMEOUT,
        );
        let said = format!("{}{}", out.stdout, out.stderr);

        println!(
            "[Q3 {}] exit={:?} timed_out={} elapsed={:?}\n  stderr: {}\n  stdout: {}",
            row.name,
            out.exit_code,
            out.timed_out,
            out.elapsed,
            excerpt(&out.stderr, 300),
            excerpt(&out.stdout, 300)
        );
        assert!(
            !out.timed_out,
            "{}: stdin is non-TTY, yet it did not end within {PROCESS_TIMEOUT:?} — meaning the interactive mode launched as is",
            row.name
        );
        assert_ne!(
            out.exit_code,
            Some(0),
            "{}: it accepted non-TTY stdin as success — the premise that the PTY is the only interactive path wavers",
            row.name
        );
        // ★The exit code alone cannot decide this question★: we give **all** three fds as pipes, so a nonzero
        //   exit could be "because stdin is not a tty", or an auth failure·configuration error·panic. In the old shape either way
        //   was equally green. The words that CLI says itself are the only evidence that tells the two apart.
        //   ★The remaining confound is written down★ — stdout/stderr are pipes too, so isolating "stdin only" needs a separate harness
        //   giving those two as a PTY. This assertion does not go that far and pins the axis by the **phrase**.
        if let Some(marker) = probe.stdin_refusal_marker {
            assert!(
                said.contains(marker),
                "{}: it exited nonzero but did not say `{marker}` — there is no evidence that this death was because of stdin \
                 (measurement M1 changed or it died for another reason): {out:?}",
                row.name
            );
        } else {
            println!(
                "[Q3 {}] ★the refusal phrase is not in the table — this row looks only at the exit code (weak decision)★",
                row.name
            );
        }
    }
}

// ── Q4 — is body + interval + CR read as a submission (★gate★) ────────────────────

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q4_body_pause_cr_reads_as_submit() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);

        let cwd = Scratch::new("q4").git_init();
        let (home, env) = sandbox_home(probe, "q4", HomeUse::Session);
        let mut s = PtySession::open(&harnessed_spec(&row, cwd.path(), &env, &[]), 80, 24);
        assert!(s.wait_first_bytes(), "{}: no first byte", row.name);
        s.wait_quiet();
        let startup = s.pass_startup_modal(probe, home.as_ref(), cwd.path());
        let modal = startup.outcome;
        let composer = assert_composer(&s, probe, &startup, row.name);

        // The TUI redraws even without input. That floor noise is measured first — **it is not an assertion criterion** (the decision below
        // stands on 「is the composer empty」). It is a number that lets the report say "how much this observation wavers".
        let idle_mark = s.sink.len();
        std::thread::sleep(PRE_SUBMIT_WINDOW);
        let idle_noise = printable_residue(&s.tail_text(idle_mark), &[]);

        let body_mark = s.sink.len();
        s.write(probe.submit_body.as_bytes());
        let wrote_at = Instant::now();
        let echoed = wait_until(ECHO_TIMEOUT, || {
            mentions(&s.tail_text(body_mark), probe.submit_body)
        });
        let echo_text = s.tail_text(body_mark);
        // ★The operating contract as is — the interval between the body write and the submit write★. If the echo wait above already
        //   exceeded that interval it sleeps no more, filling only what is short. What is measured is 「body + **interval** + CR」, so this lower bound
        //   cannot be reduced (the measurement in the `SUBMIT_PACING` comment is its ground).
        if let Some(rest) = SUBMIT_PACING.checked_sub(wrote_at.elapsed()) {
            std::thread::sleep(rest);
        }

        let cr_mark = s.sink.len();
        let submit = InputEncoder::Raw
            .submit_sequence()
            .expect("the Raw encoder has no submit bytes");
        s.write(submit);

        // ★The decision is 「is the composer empty」 — not 「was something newly drawn」★: the TUI redraws the banner·
        //   model name·usage even without any input (the `idle_noise` just above prints that value), so the latter is no evidence
        //   of a submission. A submission empties the composer, and the guidance phrase is drawn **again** in the empty composer's place — that phrase's
        //   reappearance is the fact this question actually observed, and the same thing as the recorded conclusion ("the composer became empty").
        let cleared = match probe.composer_idle_marker {
            Some(m) => s.wait_for_text(cr_mark, m, RESPONSE_TIMEOUT),
            // A backend without that signal falls back to the weak decision — did characters other than the body come. As the noise item above says,
            // this decision wavers, so that fact is not passed over quietly but written in the output below.
            None => wait_until(RESPONSE_TIMEOUT, || {
                !printable_residue(&s.tail_text(cr_mark), &[probe.submit_body]).is_empty()
            }),
        };
        let after_cr_bytes = s.sink.len().saturating_sub(cr_mark);
        let after_cr_text = s.tail_text(cr_mark);
        s.teardown();

        println!(
            "[Q4 {}] modal={modal:?}·composer={composer:?} · echo={echoed} · composer emptied after CR={cleared} \
             ({after_cr_bytes} bytes came newly, decision axis={})\n  floor noise ({PRE_SUBMIT_WINDOW:?} no input)={} \
             chars\n  echo screen: {}\n  screen after CR: {}",
            row.name,
            match probe.composer_idle_marker {
                Some(m) => format!("empty composer guidance `{m}` reappeared"),
                None => "★weak decision★ characters other than the body appeared".to_string(),
            },
            idle_noise.chars().count(),
            excerpt(&echo_text, 300),
            excerpt(&after_cr_text, 600)
        );
        assert!(
            echoed,
            "{}: the body was not echoed in the composer — decision ① failed, so ②③ cannot be asked",
            row.name
        );
        assert!(
            cleared,
            "{}: the composer did not empty within {RESPONSE_TIMEOUT:?} after CR — ★§3-5 gate★ \
             meaning the submission mechanism differs per backend, so one more InputEncoder variant becomes necessary",
            row.name
        );
    }
}

// ── Q5 — does a multi-line body go in whole (★gate★) ───────────────────────

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q5_multiline_body_enters_whole() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);
        let body = probe.multiline_parts.join("\n");

        let cwd = Scratch::new("q5").git_init();
        let (home, env) = sandbox_home(probe, "q5", HomeUse::Session);
        let mut s = PtySession::open(&harnessed_spec(&row, cwd.path(), &env, &[]), 80, 24);
        assert!(s.wait_first_bytes(), "{}: no first byte", row.name);
        s.wait_quiet();
        let startup = s.pass_startup_modal(probe, home.as_ref(), cwd.path());
        let modal = startup.outcome;
        let composer = assert_composer(&s, probe, &startup, row.name);

        // ★No CR is sent★: this question is whether the LF in the body alone causes a submission.
        let mark = s.sink.len();
        s.write(body.as_bytes());
        // Wait for whether the fragments **joined with nothing between them** — with a fixed sleep a single slow first frame
        // would cause a misjudgment.
        let joined = wait_until(ECHO_TIMEOUT, || {
            contiguous_echo(&s.tail_text(mark), probe.multiline_parts).is_some()
        });
        let echo_text = s.tail_text(mark);
        let composer_line = contiguous_echo(&echo_text, probe.multiline_parts);
        let missing: Vec<&str> = probe
            .multiline_parts
            .iter()
            .copied()
            .filter(|p| !mentions(&echo_text, p))
            .collect();

        // Count again **from after** the echo was confirmed. The earlier span mixes in the frames the TUI drew before the body was put in,
        // so the empty composer guidance from then is misread as "it emptied because it was submitted".
        let after_mark = s.sink.len();
        std::thread::sleep(PRE_SUBMIT_WINDOW);
        let after_bytes = s.sink.len().saturating_sub(after_mark);
        let after = s.tail_text(after_mark);
        // ★The unit of 「static」 is not bytes but **readable characters**★: measured in bytes, pure control sequences like cursor blinking
        //   are caught as "the screen moved" (measured — 48 bytes came in this window, of which 0 were characters).
        //   What is evidence of a submission is newly **drawn characters**, not the fact that the cursor blinked. The body itself is
        //   taken out as a needle — the body reappearing while the composer is redrawn is counter-evidence of a submission.
        let after_visible = printable_residue(&after, probe.multiline_parts);
        // ★The promised fallback is actually implemented★: for a backend without the empty composer guidance, the comment of
        //   [`LiveProbe::composer_idle_marker`] promises 「it looks only at whether the screen is static」, but `is_some_and` is **unconditionally
        //   false** in that case, so that backend's Q5 was permanently green. Writing down that a signal is absent, and writing that it looks
        //   at something else when absent and then looking at nothing, are different.
        let submitted = match probe.composer_idle_marker {
            Some(m) => mentions(&after, m),
            None => !after_visible.is_empty(),
        };
        s.teardown();

        println!(
            "[Q5 {}] modal={modal:?}·composer={composer:?} · unechoed fragments={missing:?} · adjacent={joined} · \
             submitted before CR={submitted}\n  adjacent place: {}\n  echo screen: {}\n  \
             screen after ({PRE_SUBMIT_WINDOW:?}, {} characters among {after_bytes} new bytes): {}",
            row.name,
            composer_line.as_deref().unwrap_or("(not found)"),
            excerpt(&echo_text, 400),
            after_visible.chars().count(),
            excerpt(&after, 500)
        );
        assert!(
            missing.is_empty(),
            "{}: fragments {missing:?} of the multi-line body are not visible in the composer — it did not go in whole",
            row.name
        );
        assert!(
            joined,
            "{}: the fragments are all on the screen, but **something else is between them** — if scattered, 'it went in \
             whole' cannot be claimed (the shape where each line was submitted and scattered between responses also shows all fragments). \
             The recorded conclusion is one composer line with the fragments joined (`› hi there`)",
            row.name
        );
        assert!(
            !submitted,
            "{}: the composer emptied before CR was sent — LF is read as a submission. ★§3-5 gate★ \
             if one envelope becomes several turns, Phase 2 mail cannot stand on it, and a separate design that brings \
             bracketed-paste markers into the injection path must come first",
            row.name
        );
        // ★The recorded conclusion is 「the screen was static」, and that is guarded too★ — leaving only the assertion just above guards only as far as "the empty composer guidance
        //   was not visible". Even if that phrase is not visible, if **other** characters were newly drawn, something
        //   happened, and then the recorded ground no longer holds. If this goes red, it is not that the gate flipped but that
        //   the ground of the conclusion changed, so a person must read the screen below and set up §3-3's indirect decision again.
        assert!(
            after_visible.is_empty(),
            "{}: without CR, {} new characters were drawn during {PRE_SUBMIT_WINDOW:?} ({after_bytes} bytes). The empty composer \
             guidance is not visible yet, but the screen is not static either — the recorded ground no longer holds: {}",
            row.name,
            after_visible.chars().count(),
            excerpt(&after, 400)
        );
    }
}

// ── Q6 — is `--no-alt-screen` different at the byte level ────────────────────────

/// The **family** of alt-screen entry sequences — xterm's 1049 · 1047, and the original 47. If any one of the three appears,
/// that mode is the one that swaps the whole screen.
const ALT_SCREEN_ENTERS: &[&[u8]] = &[b"\x1b[?1049h", b"\x1b[?1047h", b"\x1b[?47h"];

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q6_no_alt_screen_changes_the_bytes() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);

        let Some(flag) = probe.no_alt_screen_arg else {
            println!(
                "[Q6 {}] the argument that turns alt-screen off is not in the table — this row has nothing to ask (skipped)",
                row.name
            );
            continue;
        };

        // ★The two captures must differ **only by the flag**★: in the old shape the two runs used the same fixed path, so the first
        //   drew the folder trust modal and the second did not — on a cold PC that difference was recorded wholesale as the effect of
        //   `--no-alt-screen`. Now each capture has **a new cwd + a new temporary home**, so both are first visits, and if the modal
        //   results differ, the comparison itself stops below.
        //   (While a start modal is up the mode switch may not have happened yet, so it measures after both have passed.)
        let mut seen: Vec<(bool, Vec<String>, usize, ModalOutcome)> = Vec::new();
        for extra in [Vec::new(), vec![flag]] {
            let cwd = Scratch::new("q6").git_init();
            let (home, env) = sandbox_home(probe, "q6", HomeUse::Session);
            let mut s = PtySession::open(&harnessed_spec(&row, cwd.path(), &env, &extra), 80, 24);
            let got = s.wait_first_bytes();
            s.wait_quiet();
            let startup = s.pass_startup_modal(probe, home.as_ref(), cwd.path());
            let modal = startup.outcome;
            // ★Both captures must have finished startup for the comparison
            //   to hold★ — if one is captured still in loading or a modal,
            //   that difference gets recorded as the effect of `{flag}`.
            assert_composer(&s, probe, &startup, &format!("{} {extra:?}", row.name));
            std::thread::sleep(CAPTURE_WINDOW);
            let raw = s.sink.snapshot();
            s.teardown();

            assert!(
                got,
                "{}: no first byte with the {extra:?} combination",
                row.name
            );
            let alt = ALT_SCREEN_ENTERS.iter().any(|e| contains_seq(&raw, e));
            // ★Compared as sets★ — `private_modes` holds them **in the order they appeared**, so matching the `Vec` as is
            //   becomes "different" from merely a changed frame order. What this question asks is which modes it uses,
            //   not in which order it emits them.
            let mut modes = private_modes(&raw);
            modes.sort();
            modes.dedup();
            seen.push((alt, modes, raw.len(), modal));
        }

        println!(
            "[Q6 {}] default: alt_screen={} ({} bytes) modal={:?} modes={:?}\n  {flag}: alt_screen={} \
             ({} bytes) modal={:?} modes={:?}",
            row.name,
            seen[0].0,
            seen[0].2,
            seen[0].3,
            seen[0].1,
            seen[1].0,
            seen[1].2,
            seen[1].3,
            seen[1].1
        );
        assert_eq!(
            seen[0].3, seen[1].3,
            "{}: the start modal results of the two captures differ ({:?} vs {:?}) — meaning something other than the flag differed, so \
             the comparison below does not measure the effect of `{flag}`",
            row.name, seen[0].3, seen[1].3
        );
        let changed = (seen[0].0, &seen[0].1) != (seen[1].0, &seen[1].1);
        assert_eq!(
            changed, probe.no_alt_screen_changes_bytes,
            "{}: the byte effect of `{flag}` differs from the value written in the table — not a gate, but it means the ground §4-11's recommendation \
             stood on changed, so re-read that section and update this field",
            row.name
        );
    }
}

// ── Q8 — a cwd that is not a git repo (★gate★) ───────────────────────────────────

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q8_non_git_cwd_is_accepted() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);

        // ★No git init★ — that is the axis of this question. The premise is checked first rather than assumed.
        let cwd = Scratch::new("q8-nongit");
        let inside = run_capture(
            "git",
            &["rev-parse".to_string(), "--is-inside-work-tree".to_string()],
            &[],
            cwd.path(),
            PROCESS_TIMEOUT,
        );
        assert_ne!(
            inside.exit_code,
            Some(0),
            "the temporary folder is inside a git work tree — Q8's premise does not hold: {inside:?}"
        );

        let (home, env) = sandbox_home(probe, "q8", HomeUse::Session);
        let mut s = PtySession::open(&harnessed_spec(&row, cwd.path(), &env, &[]), 80, 24);
        let got_first = s.wait_first_bytes();
        s.wait_quiet();

        // ★This is the whole item★: while a modal holds the process, "still Running after 5 s"
        //   **cannot be false**. Measured in that state, "codex does not refuse a non-git cwd"
        //   becomes an unfalsifiable claim and this gate row stays green while asking nothing.
        //   Only checking **that the composer really takes keys** asks about the cwd policy.
        // ★Acceptance has two shapes★ — the composer directly with no modal, or the composer
        //   after the folder-trust modal (which version shows which: file header). A refusal or
        //   warning screen must be neither: if the screen passed with CR was not the folder-trust
        //   modal (no trust entry for this cwd appears in the temp home), the startup verb stops
        //   right there, and if the composer is not reached the gate below stops.
        let startup = s.pass_startup_modal(probe, home.as_ref(), cwd.path());
        let modal = startup.outcome;
        let composer_ready = probe.composer_idle_marker.map(|_| startup.composer);

        std::thread::sleep(ALIVE_WINDOW);
        let status = s.core.status();
        let screen = s.screen();
        s.teardown();

        println!(
            "[Q8 {}] first byte={got_first} · modal={modal:?} · composer={composer_ready:?} · trust entry={:?} · \
             status={status:?} after {ALIVE_WINDOW:?}\n  screen: {}",
            row.name,
            startup.trust_entry,
            excerpt(&screen, 1200)
        );
        assert!(
            got_first,
            "{}: no output at all in a cwd that is not a git repo — ★§3-5 gate★",
            row.name
        );
        if let (Some(Err(missing)), Some(marker)) = (composer_ready, probe.composer_idle_marker) {
            panic!(
                "{}: the composer did not take keys after the start screen (modal={modal:?} · {}) — ★§3-5 gate★ meaning it did not accept a cwd \
                 that is not a git repo (refusal·warning·unknown modal). Our spawn entry takes arbitrary \
                 folders, so the cwd policy must be decided before the wiring: {}",
                row.name,
                missing.describe(marker),
                excerpt(&screen, 800)
            );
        }
        assert!(
            matches!(status, AgentStatus::Running),
            "{}: it refused a cwd that is not a git repo (status={status:?}) — ★§3-5 gate★ our spawn entry \
             takes arbitrary folders, so the cwd policy must be decided before the wiring",
            row.name
        );
    }
}

// ── Q9 — can the caller decide the session id ─────────────────────────────────

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q9_caller_cannot_choose_the_session_id() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);

        let cwd = Scratch::new("q9");
        let (_home, env) = sandbox_home(probe, "q9", HomeUse::Session);
        // Asked by putting `--help` on the operating argv — help short-circuits at the parsing stage, so the same text comes out even with the preceding
        // arguments in place (measured 0.153.4). It is the value that keeps this file's way of calling that CLI to one.
        let spec = harnessed_spec(&row, cwd.path(), &env, &["--help"]);
        let help_run = run_capture(
            &spec.program,
            &spec.args,
            &spec.env,
            &spec.cwd,
            PROCESS_TIMEOUT,
        );

        assert_eq!(
            help_run.exit_code,
            Some(0),
            "{}: `--help` failed: {help_run:?}",
            row.name
        );
        let help = format!("{}{}", help_run.stdout, help_run.stderr);

        match probe.session_id_flag {
            Some(flag) => {
                println!("[Q9 {}] expecting the session id flag `{flag}`", row.name);
                assert!(
                    help.contains(flag),
                    "{}: `{flag}` is not in `--help` — a flag the table says exists disappeared",
                    row.name
                );
            }
            None => {
                let found: Vec<&str> = SESSION_ID_CANDIDATES
                    .iter()
                    .copied()
                    .filter(|c| help.contains(c))
                    .collect();
                println!(
                    "[Q9 {}] `--help` {} bytes · session id candidates found={found:?}",
                    row.name,
                    help.len()
                );
                assert!(
                    found.is_empty(),
                    "{}: the flag {found:?} that decides the session id actually exists — the decision that the caller cannot decide the sid \
                     is overturned",
                    row.name
                );
            }
        }
    }
}

// ── Q10 — is the process tree empty after shutdown (★gate★) ────────────────────

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q10_shutdown_empties_the_process_tree() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);

        let cwd = Scratch::new("q10").git_init();
        let (_home, env) = sandbox_home(probe, "q10", HomeUse::Session);
        let mut s = PtySession::open(&harnessed_spec(&row, cwd.path(), &env, &[]), 80, 24);
        assert!(s.wait_first_bytes(), "{}: no first byte", row.name);
        s.wait_quiet();

        let root = s.child_pid.expect("did not get the PTY's direct child PID");
        // What is assigned to the Job Object is the direct child (`cmd.exe`), and the real CLI is its grandchild·great-grandchild. That depth
        // must be checked to actually exist first so that "the tree is empty" does not become an empty assertion.
        // ★The seen list is **accumulated as a union**★ — holding only a snapshot taken once, a middle link born and dead in between
        //   drops out of the list, and even if that link's child remains, nobody looks.
        let mut tree_before: Vec<u32> = Vec::new();
        let deep = wait_until(Duration::from_secs(15), || {
            for p in descendants(root) {
                if !tree_before.contains(&p) {
                    tree_before.push(p);
                }
            }
            !tree_before.is_empty()
        });
        let status_before = s.core.status();

        let (shutdown_took, join_took) = s
            .teardown_measured()
            .expect("this item calls teardown for the first time");

        // ★Enumerate again — do not re-check only the old snapshot★: a grandchild born **after** the enumeration above is not in the snapshot,
        //   so looking only at the list again goes green with it surviving.
        let mut alive_after: Vec<u32> = Vec::new();
        let gone = wait_until(Duration::from_secs(10), || {
            alive_after = alive_survivors(root, &tree_before);
            alive_after.is_empty()
        });
        let status_after = s.core.status();

        println!(
            "[Q10 {}] root={root} descendants={tree_before:?} · shutdown {shutdown_took:?} → \
             join_pump {join_took:?} · alive={alive_after:?} · status {status_before:?} → {status_after:?}",
            row.name
        );
        assert!(
            deep || !cfg!(windows),
            "{}: not a single descendant is visible under the direct child — the shim chain premise wavers",
            row.name
        );
        assert!(
            gone,
            "{}: {alive_after:?} is still alive after shutdown — ★§3-5 gate★ this is not a regression but \
             a violation of the **first verb** of the kill causality (ADR-0001), so the Job Object placement must be looked at again",
            row.name
        );

        // ── From here on is the second verb ──────────────────────────────────────────────────
        //
        // ★The PID disappearing only measured `shutdown()`★. ADR-0001's causality does not end there —
        //   master drop → reader EOF → pump break → `core.finish` → done_tx follow after it, and what emits the terminal
        //   transition is **the pump alone** (ADR-0005 · `docs/reference/architecture.md` 「Runtime invariants」: finalize exactly once ·
        //   division of status notifications). So the observable result that the pump actually ended is **did status move to terminal**:
        //   if it did not, `finish` was not called = the loop did not break.
        // ★`finalized` cannot be seen directly★ (there is no public accessor). But `finish` writes status only on the winner path
        //   that passed that swap, so a terminal status is the same fact as that flag having been set.
        assert!(
            !status_after.is_live(),
            "{}: the process is gone but status is still {status_after:?} — meaning the pump did not break the loop or \
             `finish` was not called (the terminal transition is the pump alone — ADR-0005). ★§3-5 gate★ \
             the **second verb** of the kill causality (ADR-0001) does not hold",
            row.name
        );
        // ★Did it stick to the bound★ — `join_pump` discards the result of `recv_timeout` (`output_core.rs:383-392`), so
        //   "waited and failed" and "ended right away" look like the same `()` to the caller. Only the duration tells the two apart.
        //   Why the margin is large = what this assertion measures is not "is it fast" but "did it go up to the bound".
        assert!(
            join_took < JOIN_PUMP_TIMEOUT.mul_f32(0.9),
            "{}: join_pump took {join_took:?} — meaning it stuck to the bound ({JOIN_PUMP_TIMEOUT:?}), so \
             the pump did not end within it. Even if status looks terminal, it came late after the bound",
            row.name
        );
    }
}

// ── Q11 — does it accept the operating argv as is (★the place where this file says it 「measures operation」★) ──────────

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q11_production_argv_is_accepted_and_reaches_a_composer() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);

        let cwd = Scratch::new("q11").git_init();
        let (home, env) = sandbox_home(probe, "q11", HomeUse::Session);

        // ★Only this item does not apply the safety swap★ — the other questions launch with the policy fields overwritten with harmless ones, so
        //   this is the only place that asks 「does the real CLI accept those values operation uses」.
        let spec = production_spec(&row, cwd.path(), &env, &[]);
        assert_no_forbidden(&spec.args, &probe.safety, row.name);

        // ★But is that argv really different from the swapped one★: if it is the same, this item measures exactly the same thing as the questions above,
        //   and its reason to exist, 「the place that measures the operating policy」, quietly disappears. That silence is caught here.
        let swapped = safe_args(&spec.args, &probe.safety, row.name);
        assert_ne!(
            spec.args, swapped,
            "{}: the operating argv is already the same as the test-bench policy — then this item measures the same thing as the other items, \
             meaning the place that separately measured the operating policy is gone. Re-read the safety envelope and decide whether to \
             delete this item: {:?}",
            row.name, spec.args
        );

        println!(
            "[Q11 {}] operating argv: {} {:?}",
            row.name, spec.program, spec.args
        );

        // ★No prompt is ever submitted★ — the only keys sent are the probe characters,
        //   their erasure and one CR that pass the startup screen
        //   ([`PtySession::pass_startup_modal`]). Without a model turn the write access
        //   that policy grants never gets a chance to be used, and that is the only
        //   reason this item is harmless while running the production policy as-is.
        // ★That verb also carries the startup-path guarantee★ — if another first
        //   screen appears under the production policy values, its post-CR check
        //   (did a folder-trust entry for this cwd appear in the temp home) stops
        //   the run. That is why this item has no modal assertion of its own.
        let mut s = PtySession::open(&spec, 100, 24);
        let got_first = s.wait_first_bytes();
        s.wait_quiet();
        let startup = s.pass_startup_modal(probe, home.as_ref(), cwd.path());
        let modal = startup.outcome;
        let composer = probe.composer_idle_marker.map(|_| startup.composer);
        std::thread::sleep(ALIVE_WINDOW);
        let status = s.core.status();
        let screen = s.screen();
        s.teardown();

        println!(
            "[Q11 {}] first byte={got_first} · modal={modal:?} · composer={composer:?} · trust entry={:?} · \
             status={status:?} after {ALIVE_WINDOW:?}\n  screen: {}",
            row.name,
            startup.trust_entry,
            excerpt(&screen, 900)
        );

        assert!(
            got_first,
            "{}: launching with the operating argv gives no first byte — meaning the argv the test bench measured launches but the argv operation launches \
             does not. If this goes red, it is not the wiring but the operating arguments that are wrong",
            row.name
        );
        if let (Some(Err(missing)), Some(marker)) = (composer, probe.composer_idle_marker) {
            panic!(
                "{}: the operating argv did not reach the composer (modal={modal:?} · {}) — meaning they are policy values that do not reach a screen \
                 a person can use (approval waiting·sandbox install modal, etc.). The operating arguments \
                 must be looked at again: {}",
                row.name,
                missing.describe(marker),
                excerpt(&screen, 900)
            );
        }
        assert!(
            matches!(status, AgentStatus::Running),
            "{}: the process launched with the operating argv exited within {ALIVE_WINDOW:?} (status={status:?})",
            row.name
        );

        // ★This is all of this item — the remaining field is written down★: under this policy, **what the model can actually
        //   do** is not measured. Measuring it needs putting in a prompt and running a turn, and that means running an agent holding write
        //   permission in this test bench. The test bench's harmlessness weighs more than that, so they were not traded, and so
        //   that field **is empty**. Do not read this item's green as 「the whole operating policy was verified」.
    }
}

// ── Q12 — does the work root come from the argument · does env reach the process (= the ground of `session_cwd_env`) ──

/// The name tag of the folder Q12 digs. ★Keep it short★ — the TUI shortens paths to fit the width, so a long name is cut on the screen
/// and the decision cannot stand (measured 0.153.4: at 80 columns the header box cut the leaf from the front with `…`).
fn short_tag(prefix: &str) -> String {
    format!("{prefix}-{}", &Uuid::new_v4().to_string()[..8])
}

/// In which file body under `root` (recursively) does `needle` appear — returns the first place.
///
/// ★Why the file name is not pinned★: **in which file** that CLI writes the folder trust is a matter of the version, and what this
///   question asks is 「was that decision written **inside** the state directory we passed」. Pinning the name would make the observation quietly
///   an empty field the day that CLI moves the file.
fn find_text_under(root: &Path, needle: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(p),
                Ok(t) if t.is_file() => {
                    // Large files are skipped — what this question looks for is settings·records, not dumps.
                    if e.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > 4 * 1024 * 1024 {
                        continue;
                    }
                    if let Ok(bytes) = std::fs::read(&p) {
                        if String::from_utf8_lossy(&bytes).contains(needle) {
                            return Some(p);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

#[test]
#[ignore = "needs the real codex — cargo test -p engram-dashboard-agent --test backend_contract -- --ignored"]
fn q12_working_root_comes_from_the_spawn_argument_and_env_reaches_the_process() {
    for row in live_rows() {
        let probe = row.probe.as_ref().unwrap();
        assert_program_present(&row);

        // ★The declaration and the observation are tied here★ — this item runs as the ground of the `session.cwd_env` field. If that field flips to false,
        //   the claim to measure itself is gone, so what follows cannot be left as is.
        assert!(
            row.backend.capabilities(&row.sample).session.cwd_env,
            "{}: `session.cwd_env` is false, yet this item runs as the ground of that field — the table and the observation disagree",
            row.name
        );

        // Two folders are dug: the **work root** the operating spec designates by argument, and **another folder** to actually launch the process in.
        let root_box = Scratch::new("q12r");
        let away_box = Scratch::new("q12a");
        let root_leaf = short_tag("root");
        let away_leaf = short_tag("away");
        let root = root_box.path().join(&root_leaf);
        let away = away_box.path().join(&away_leaf);
        std::fs::create_dir_all(&root).expect("failed to create the work root folder");
        std::fs::create_dir_all(&away).expect("failed to create the process cwd folder");
        // ★Both folders are made git repos★ — the trust record that is the evidence for
        //   (b) below is written only by passing the trust modal, and a non-git folder
        //   gets no modal (file header; measured 0.163.0: such a folder goes straight to
        //   the composer and its path is written nowhere in the home). The process-cwd
        //   side must be a repo too, so that "a record appeared for it" is something that
        //   **can happen** — only then does that absence assertion measure anything.
        // ★A run where that side wins is usually caught first by the startup
        //   verb★ — it requires a trust entry for the **working root** after
        //   the CR, so it stops if the modal offered to trust the process cwd.
        //   The absence assertion below catches a run where both got written.
        git_init_at(&root);
        git_init_at(&away);

        let (home, env) = sandbox_home(probe, "q12", HomeUse::Session);
        let mut spec = harnessed_spec(&row, &root, &env, &[]);
        // ★Then only the process cwd is turned elsewhere★: if the two fields have the same value, 「did the work root come from the argument」
        //   cannot be asked — because even if that argument were ignored entirely, the process cwd would produce **the same answer**.
        //   This one line makes an unfalsifiable observation falsifiable, and that is all of this item.
        // ★To that extent this is not the operating shape★ — operation gives the two the same value (`build_spec`). Splitting them
        //   here is to experience 「which field decides the work root」, and the answer being **the argument side**
        //   is the ground that operation can pass the cwd as is.
        spec.cwd = away.clone();

        let root_str = root.to_string_lossy().into_owned();
        assert!(
            spec.args.iter().any(|a| a == &root_str),
            "{}: the work root ({root_str}) was not put in the operating argv — meaning this backend does not pass the work folder by argument, \
             so what this item meant to measure is not in the argv in the first place: {:?}",
            row.name,
            spec.args
        );

        println!(
            "[Q12 {}] argv={:?}\n  work root (argument)={root_str}\n  process cwd={}",
            row.name,
            spec.args,
            away.display()
        );

        // ★Launch it wide★ — this TUI shortens paths to fit the width, so at 80 columns the leaf is cut and the decision cannot stand.
        let mut s = PtySession::open(&spec, 140, 24);
        let got_first = s.wait_first_bytes();
        s.wait_quiet();
        let startup = s.pass_startup_modal(probe, home.as_ref(), &root);
        let modal = startup.outcome;
        let composer = assert_composer(&s, probe, &startup, row.name);

        // ★Only the output after the startup screen is looked at★ — the consumed
        //   trust modal's text also contains the working-root path (measured
        //   0.163.0 `Trusting will apply to the repository root: …\root-…`).
        //   Matching the whole accumulated screen would stay green thanks to
        //   that sentence even if the working-root display disappeared.
        let settled = s.tail_text(startup.settled_from);
        let says_root = mentions(&settled, &root_leaf);
        let says_away = mentions(&settled, &away_leaf);

        // The file-side evidence — after passing the modal, that decision is written into the state directory. Asynchronous, so it waits.
        let home_path = home.as_ref().map(|h| h.path().to_path_buf());
        let mut root_in_home: Option<PathBuf> = None;
        if let Some(hp) = home_path.as_ref() {
            wait_until(COMPOSER_TIMEOUT, || {
                root_in_home = find_text_under(hp, &root_leaf);
                root_in_home.is_some()
            });
        }
        let away_in_home = home_path
            .as_ref()
            .and_then(|hp| find_text_under(hp, &away_leaf));
        let screen = s.screen();
        s.teardown();

        // Is the real home not dirtied — this is also the counter-evidence that the temporary home really received that write.
        let root_in_real_home = probe
            .home_redirect
            .as_ref()
            .and_then(real_home)
            .and_then(|rh| find_text_under(&rh, &root_leaf));

        println!(
            "[Q12 {}] first byte={got_first} · modal={modal:?} · composer={composer:?} · trust entry={:?}\n  \
             folder the screen names after start: argument side (`{root_leaf}`)={says_root} · process-cwd side (`{away_leaf}`)={says_away}\n  \
             temporary home record: argument side={:?} · process-cwd side={:?}\n  real home pollution: {:?}\n  screen after start: {}\n  whole screen: {}",
            row.name,
            startup.trust_entry,
            root_in_home.as_ref().map(|p| p.display().to_string()),
            away_in_home.as_ref().map(|p| p.display().to_string()),
            root_in_real_home.as_ref().map(|p| p.display().to_string()),
            excerpt(&settled, 1000),
            excerpt(&screen, 1000)
        );

        assert!(got_first, "{}: no first byte", row.name);

        // ── (a) the cwd half — the work root is decided by the **argument** ────────────────────────────────────────
        if probe.shows_workspace_root {
            assert!(
                says_root,
                "{}: the work root given as argument (`{root_leaf}`) is not on the screen — the claim that the argument decides the work root \
                 does not stand (if it changed to not write the work root on the screen, or the screen was cut by the left width, re-measure starting from the table's \
                 `shows_workspace_root` field): {}",
                row.name,
                excerpt(&settled, 900)
            );
            assert!(
                !says_away,
                "{}: the screen names the **process cwd** (`{away_leaf}`) as the work folder — meaning the argument is ignored and \
                 the process cwd wins, so the premise that operation decides the work folder by that argument breaks: {}",
                row.name,
                excerpt(&settled, 900)
            );
        }

        // ── (b) the env half — it actually uses the state directory passed ─────────────────────────────────────
        if probe.home_redirect.is_some() {
            assert!(
                root_in_home.is_some(),
                "{}: no record of the work root (`{root_leaf}`) anywhere in the temporary home — meaning the env passed did not reach the process \
                 or that decision was written elsewhere. If the former, the env half of `session.cwd_env` \
                 is false; if the latter, this test bench may dirty the real user settings every time it runs — either way \
                 it stops",
                row.name
            );
            assert!(
                away_in_home.is_none(),
                "{}: a record of the **process cwd** (`{away_leaf}`) appeared in the temporary home ({:?}) — meaning that CLI took the process cwd, \
                 not the argument, as the work folder",
                row.name,
                away_in_home
            );
            assert!(
                root_in_real_home.is_none(),
                "{}: this item's temporary path (`{root_leaf}`) was written into the real user home ({:?}) — the home redirection \
                 is leaking. The guarantee that this test bench does not dirty the developer's settings is broken",
                row.name,
                root_in_real_home
            );
        }

        assert!(
            composer.unwrap_or(true),
            "{}: the composer did not stand",
            row.name
        );
    }
}
