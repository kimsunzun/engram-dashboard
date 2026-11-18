//! PrimingProvider seam (ADR-0092 receive contract) — yields the absolute path of the priming file to inject into the system prompt at spawn.
//!
//! At every spawn (provision) the daemon asks this seam for the priming file path and sends it as `ControlEndpoint.priming_file`. Path only — the file content is not read (no hardcoding, ADR-0092: content lives only in the external MD `prompts/agent-priming.md`).
//!
//! Why a seam (ADR-0092 "leave the road open"): `FilePrimingProvider` gives one shared file regardless of agent (a stopgap); a future per-agent prompt injection / skill system swaps only this trait's implementation, so provision depends only on `PrimingProvider`.
//!
//! Graceful: with no resolved file it returns `None` and leaves a warn — missing priming is not a spawn failure (unlike the control channel provision's fail-closed); the agent comes up without the receive contract but works.
// ADR-0092

use std::path::PathBuf;

use engram_dashboard_agent::types::{
    CLI_EXE_ENV, CLI_EXE_NAME, CLI_GROUP_MAIL, CLI_MAIL_FLAGS, CLI_MAIL_VERBS,
};

/// A character that continues a token — attached on either side of a match, it makes the match part of another word.
///
/// A bare substring with no boundary catches ordinary prose (measured review): `The Engram mailbox is managed by the daemon.` contains `engram mail`, `--token` contains `--to`. That false positive is a misdiagnosis — the prohibition gate flags an innocent sentence and tells the operator to delete it.
/// `-` and `_` are included because flags and identifiers are joined by them (`--to-be`·`--token`).
fn continues_token(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '-' || ch == '_'
}

/// Does `needle` appear delimited by a word boundary. `engram mail send` ✓ · `engram mailbox` ✗ · `--to bob` ✓ · `--token` ✗.
fn contains_bounded(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while let Some(rel) = haystack[from..].find(needle) {
        let start = from + rel;
        let end = start + needle.len();
        let before_ok = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !continues_token(c));
        let after_ok = haystack[end..]
            .chars()
            .next()
            .is_none_or(|c| !continues_token(c));
        if before_ok && after_ok {
            return true;
        }
        from = start + needle.chars().next().map(char::len_utf8).unwrap_or(1);
    }
    false
}

/// Normalization for token matching in the priming body. The order is load-bearing: 1) character-level folding (lowercase · separators → space · deleting meaningless characters) → 2) structure removal (HTML comments) → 3) whitespace collapse. Every decision arm sees only a string that passed through this (no raw-text comparison path, so no arm-specific exceptions).
///
/// 1 before 2 (reversed they neutralize each other — measured review): a zero-width character or escape inside the comment-closing marker (`<!--x--` + zero-width + `>`) makes 2 see the comment as unclosed and leave it, so the command passes the decision broken up by comment text; folded first, that combination does not hold.
/// Normalization is needed because the decision token is several words: raw comparison reads a sentence broken by a line break, consecutive spaces or emphasis like `**engram** mail` as "does not teach", letting CLI-teaching priming pass for an MCP-capable spawn and misattributing ADR-0099's send freeze as a normal negative.
/// Two character classes: separators (`*`·backtick·`~`·braces·quotes) stand between words and become a space — deleting would join `` `--to`-recipient `` into `--to-recipient` and flip the decision, and break the adjacency of `"${ENGRAM_CLI_EXE}" mail send`; meaningless characters (`_`·backslash·zero-width kinds) get stuck inside a word and are deleted — an escaped env name must fold into one token.
/// An unclosed `<!--` is kept: priming is injected into the system prompt verbatim, so discarding the tail by imitating a renderer would leave text visible to the agent that vanishes only for the decider.
/// The token we search for is also normalized the same way: never type a literal by hand; use `normalized_token()`.
fn normalize_for_token_match(content: &str) -> String {
    // 1) Character-level folding.
    let mut folded = String::with_capacity(content.len());
    for ch in content.chars() {
        match ch {
            // Separators standing between words → a space (deleting would join the tokens on either side).
            '*' | '`' | '~' | '{' | '}' | '"' | '\'' => folded.push(' '),
            // Meaningless characters inside a word → delete (markdown emphasis/escape + invisible formatting characters).
            '_' | '\\' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{200E}' | '\u{200F}'
            | '\u{2060}' | '\u{FEFF}' | '\u{00AD}' => {}
            other => folded.extend(other.to_lowercase()),
        }
    }

    // 2) Structure removal — closed HTML comments only.
    let mut stripped = String::with_capacity(folded.len());
    let mut rest = folded.as_str();
    while let Some(open) = rest.find("<!--") {
        match rest[open..].find("-->") {
            Some(close_rel) => {
                stripped.push_str(&rest[..open]);
                stripped.push(' ');
                rest = &rest[open + close_rel + 3..];
            }
            None => break,
        }
    }
    stripped.push_str(rest);

    // 3) Whitespace collapse (including consecutive spaces made by 1 and 2).
    let mut out = String::with_capacity(stripped.len());
    let mut last_was_space = false;
    for ch in stripped.chars() {
        if ch.is_whitespace() {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
                last_was_space = true;
            }
            continue;
        }
        last_was_space = false;
        out.push(ch);
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Folds the token we search for by the same rules as the body (no hand-typed literals — they move together when normalization changes).
fn normalized_token(raw: &str) -> String {
    normalize_for_token_match(raw)
}

/// Does the body teach mail CLI sending (strong signal) — the decision target is only the executable call form: the executor (executable name or env variable) + family token + the adjacency of a verb (`engram mail send` etc.).
///
/// A mere appearance of the name is not teaching: `ENGRAM_CLI_EXE is reserved for diagnostics; do not use it to send mail.` says not to use it, yet the old decision counted it. A variable teaches only when a family token follows it; a bare mention counts only as `mentions_mail_cli_surface`.
/// The variable form stays teaching (a real form): a call through a variable never writes the command name — `run \`$ENGRAM_CLI_EXE mail send --to alice\`` — and the old priming in ADR-0098 was exactly that (why the grant was unmatched); removing it would make such a file read "does not teach sending" and the `--cli-only` requirement gate reject a fine experiment.
/// Decide by body, not basename — do not revert: the earlier basename list was quietly bypassed by new CLI-instructing priming, and missing CLI binary (absent infrastructure) was misattributed as a normal negative (B_SENT=false) instead of SETUP-SKIP. Folding case, whitespace and emphasis is for the same reason.
/// The bare executable name is not enough: MCP priming legitimately uses the word as the MCP server name (`on the engram server`), and a bare decision would catch MCP-only files as CLI-instructing and reject every MCP run before spawn. So the decision is adjacency to the family token, cut at word boundaries (`engram mailbox` excluded).
/// A verb is required: `engram mail` alone is an unexecutable fragment, and counting it would satisfy the `--cli-only` requirement gate and grade B's non-send (not knowing the verb) as a normal negative — the misattribution this decider blocks. The verb list is agent `CLI_MAIL_VERBS` (shared with the parser).
/// What this gate cannot catch (known limits — read before extending):
///   - Negation: `Do NOT use engram mail send; use MCP instead.` is still caught as "teaching". Deciding sentence negation by text is a heuristic swamp, deliberately not attempted (user decision 2026-08-04, reconfirmed this round). The price is a pointless SETUP-FAIL when passing such a file without `--cli-only`;
///     the two production primings have no such sentence, so real-path impact is 0 — a loud refusal beats a quiet misattribution.
///   - Shell syntax in general: a token-adjacency matcher, not a shell parser; variable rebinding (`X=$ENGRAM_CLI_EXE; $X mail send`), aliases and calls split across lines are not seen. If that class really appears, fix the priming side rather than growing a parser.
///   - Bracket-type separators: `{}` and quotes fold as between-word symbols but `()` and `[]` do not, to avoid prose like `engram (mail) family` being caught as a command (worse).
/// Limiting the family to mail is intentional (do not widen): the guarded invariant is ADR-0128/0099's mail teaching↔wiring equality, and the misattributed observation is a mail send (`B_SENT`). The control family (`engram help` · later `engram agent …`) is a surface every spawn uses (ADR-0132 decisions 4 and 5);
/// counting it would reject MCP priming over one control line and let "`--cli-only` priming teaching no sending method" pass the reverse gate — reviving the misattribution on both sides.
// ADR-0128
// ADR-0132
pub fn teaches_mail_cli(content: &str) -> bool {
    let normalized = normalize_for_token_match(content);
    let group = normalized_token(CLI_GROUP_MAIL);
    CLI_MAIL_VERBS.iter().any(|verb| {
        let verb = normalized_token(verb);
        [CLI_EXE_NAME, CLI_EXE_ENV].iter().any(|invoker| {
            contains_bounded(
                &normalized,
                &format!("{} {group} {verb}", normalized_token(invoker)),
            )
        })
    })
}

/// Does the body have a trace of the mail CLI surface (broad signal) — strong signal ∪ CLI absolute-path env variable names ∪ CLI-only flag notation.
///
/// Kept separate from the strong signal (merged, one side is certainly wrong) because the gates need opposites:
///   - Prohibit direction (an MCP-capable spawn must not carry CLI teaching): when in doubt block → use this broad signal; a document with only flags and variables has leaked the CLI surface too.
///   - Require direction (`--cli-only` requires CLI send teaching): the broad signal would pass a document that only brushes a flag without teaching the call form, grading a run where B does not know how to send as valid; that side must be `teaches_mail_cli`.
pub fn mentions_mail_cli_surface(content: &str) -> bool {
    if teaches_mail_cli(content) {
        return true;
    }
    let normalized = normalize_for_token_match(content);
    // A verbless `engram mail` fragment also counts as a surface — it cannot execute, but the CLI surface has leaked.
    let group_form = format!(
        "{} {}",
        normalized_token(CLI_EXE_NAME),
        normalized_token(CLI_GROUP_MAIL)
    );
    if contains_bounded(&normalized, &group_form)
        || contains_bounded(&normalized, &normalized_token(CLI_EXE_ENV))
    {
        return true;
    }
    CLI_MAIL_FLAGS
        .iter()
        .any(|f| contains_bounded(&normalized, &normalized_token(f)))
}

/// Send+Sync+'static — DaemonControlChannel holds it as an Arc and calls it at every provision.
pub trait PrimingProvider: Send + Sync + 'static {
    /// The absolute path of the priming MD file to inject into this spawn. `None` if absent (file missing) or unconfigured.
    /// Absolute path contract: the agent's cwd differs from the daemon/repo (each workspace), so claude finds the file only by an absolute path — a relative one would resolve against the agent's cwd and miss.
    ///
    /// Who gets the injection is not decided by this seam: the consistency invariant (the mail channel priming teaches == the mail channel that spawn can actually use — violated, you get the send freeze ADR-0099 measured) depends on `uses_mail` and `accepts_mcp_config`, known only to the daemon. Provision chooses whether to call;
    /// this seam answers only "is there a file to give". Authority: `DaemonControlChannel::provision`.
    // ADR-0126
    // ADR-0133
    fn priming_file(&self) -> Option<PathBuf>;
}

/// Base is the exe-based root (ADR-0092, both reviewers PRIMARY): base was once the daemon process cwd (`from_cwd`), but the production daemon is launched through WMI Win32_Process.Create and does not inherit the parent cwd (cwd=System32), so priming was quietly disabled. The fix reuses the exe-walk-up pattern
/// (`data_dir::find_install_root`) that `default_data_dir` uses to anchor `.engram-dev`, taking a trustworthy absolute root (distrust cwd).
///
/// Base is injected at construction so tests can swap in a temp dir and verify deterministically without cwd/exe contamination; production wiring is from_install_root.
pub struct FilePrimingProvider {
    base_dir: PathBuf,
}

const REL_MCP_PRIMARY: &str = "prompts/agent-priming.md";
const ENV_OVERRIDE: &str = "ENGRAM_PRIMING_FILE";

/// Characters that corrupt under cmd.exe (ADR-0092, Codex #1+#5): on Windows claude arguments are wrapped by `console_command` (the platform crate's `shell`) as `cmd.exe /c claude …`, which expands `%VAR%` even inside quotes and treats `& ^ | < >` as shell metacharacters. A priming path with these would reach claude wrong/truncated, so it is not injected (None).
///   PRE-EXISTING · separate follow-up (ADR-0092): the cmd.exe escaping flaw of `console_command` is not introduced by this slice; `--mcp-config` (config_path) on the same path is exposed the same way (a daemon-made path, practically safe, same principle). This does only the cheap slice-level defense (the priming path filter) and does not fix console_command
///   — tracked separately since it affects the arguments of all backends.
const CMD_UNSAFE_CHARS: &[char] = &['%', '&', '^', '|', '<', '>'];

/// UTF-8 is checked too (Codex #5): arguments are stringified with `to_string_lossy()` for the CLI (`backend/claude/`); a non-UTF8 path would be corrupted to U+FFFD and claude would get a nonexistent path — not injecting beats a corrupted path.
fn path_is_cli_safe(p: &std::path::Path) -> bool {
    let Some(s) = p.to_str() else {
        return false;
    };
    !s.chars().any(|c| CMD_UNSAFE_CHARS.contains(&c))
}

impl FilePrimingProvider {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// If the root cannot be obtained (current_exe failure etc.), base is `.` (relative) — absolutize then fails and yields None (a relative path is never injected).
    pub fn from_install_root() -> Self {
        let base = crate::data_dir::find_install_root().unwrap_or_else(|| PathBuf::from("."));
        Self::new(base)
    }

    /// absolute-or-None contract (ADR-0092, Codex #3): a relative base is no longer made absolute via current_dir (and returned relative on failure); the cwd fallback is removed, so failing to absolutize strictly yields None.
    fn absolutize(&self, p: PathBuf) -> Option<PathBuf> {
        if p.is_absolute() {
            return Some(p);
        }
        let joined = self.base_dir.join(&p);
        joined.is_absolute().then_some(joined)
    }

    fn resolve_checked(&self, raw: PathBuf, label: &str) -> Option<PathBuf> {
        let Some(abs) = self.absolutize(raw) else {
            tracing::warn!(
                label,
                "cannot resolve the priming path to an absolute path (base is not absolute) — spawning without priming (ADR-0092 graceful)"
            );
            return None;
        };
        if !path_is_cli_safe(&abs) {
            tracing::warn!(
                label,
                path = %abs.display(),
                "the priming path is non-UTF8 or contains cmd.exe metacharacters (% & ^ | < >) — not injected due to corruption risk (ADR-0092)"
            );
            return None;
        }
        // TOCTOU residue (accepted, ADR-0092 Codex #4): if the file disappears between this is_file and claude opening it after spawn, claude comes up without priming or errors. The priming file is stable infrastructure (version-controlled MD), not a racing target — accepted as a best-effort existence check, no lock; residual risk equals graceful absence.
        if abs.is_file() {
            return Some(abs);
        }
        tracing::warn!(
            label,
            path = %abs.display(),
            "priming file not found — spawning without priming (ADR-0092 graceful)"
        );
        None
    }
}

impl PrimingProvider for FilePrimingProvider {
    fn priming_file(&self) -> Option<PathBuf> {
        // An override failure does not fall back to fixed: quietly swapping an explicit override for a different file is confusing; whichever gate catches it, None (proceed without priming — resolve_checked warns).
        // The env override beats the fixed file (ADR-0099 test-seam): a harness/operator can force a priming (roundtrip_smoke `--priming` passes it through this env); production leaves it unset. A spawn that provision decided not to call this seam is not revived by the override either — that decision is one body with the mail authorization and must not flip by one env (the trait doc is the authority).
        if let Some(v) = std::env::var_os(ENV_OVERRIDE) {
            if !v.is_empty() {
                return self.resolve_checked(PathBuf::from(v), "override");
            }
        }

        self.resolve_checked(PathBuf::from(REL_MCP_PRIMARY), "fixed")
    }
}

/// Fills the seam in tests and paths unrelated to priming while keeping `--append-system-prompt-file` from being attached; no real file keeps the test deterministic.
pub struct NoopPrimingProvider;

impl PrimingProvider for NoopPrimingProvider {
    fn priming_file(&self) -> Option<PathBuf> {
        None
    }
}

/// Fixed-path provider (test only) — returns the given absolute path as is (no existence check), for integration tests confirming priming is carried through the spawn wiring.
pub struct FixedPrimingProvider(pub PathBuf);

impl PrimingProvider for FixedPrimingProvider {
    fn priming_file(&self) -> Option<PathBuf> {
        Some(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_agent::types::CLI_EXE_NAME;
    use std::io::Write as _;
    use std::sync::Mutex;

    /// env is process-global: parallel tests touching `ENGRAM_PRIMING_FILE` erase each other's set/remove (flaky); this mutex serializes them (cargo is parallel by default).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_fixture_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("engram-priming-test-{}", uuid::Uuid::new_v4()));
        let prompts = dir.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        let mut f = std::fs::File::create(prompts.join("agent-priming.md")).unwrap();
        writeln!(f, "# test priming (mcp-primary)").unwrap();
        dir
    }

    #[test]
    fn resolves_fixed_relative_to_absolute_under_base() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = make_fixture_dir();
        let provider = FilePrimingProvider::new(dir.clone());
        std::env::remove_var(ENV_OVERRIDE);
        let got = provider
            .priming_file()
            .expect("Some when the fixed file exists");
        assert!(
            got.is_absolute(),
            "the resolved result must be an absolute path: {got:?}"
        );
        assert!(
            got.ends_with("prompts/agent-priming.md") || got.ends_with("prompts\\agent-priming.md")
        );
        assert!(got.is_file(), "it must point to a real file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_fixed_file_yields_none_no_panic() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir =
            std::env::temp_dir().join(format!("engram-priming-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::remove_var(ENV_OVERRIDE);
        let provider = FilePrimingProvider::new(dir.clone());
        assert!(provider.priming_file().is_none(), "missing file → None");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_override_wins_over_fixed() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = make_fixture_dir();
        let override_file = dir.join("custom-priming.md");
        {
            let mut f = std::fs::File::create(&override_file).unwrap();
            writeln!(f, "# override priming").unwrap();
        }
        std::env::set_var(ENV_OVERRIDE, &override_file);
        let provider = FilePrimingProvider::new(dir.clone());
        let got = provider
            .priming_file()
            .expect("override file exists → Some");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("custom-priming.md"),
            "the override path must win: {got:?}"
        );
        std::env::remove_var(ENV_OVERRIDE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_override_missing_file_yields_none_not_fallback() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = make_fixture_dir();
        let ghost = dir.join("does-not-exist.md");
        std::env::set_var(ENV_OVERRIDE, &ghost);
        let provider = FilePrimingProvider::new(dir.clone());
        assert!(
            provider.priming_file().is_none(),
            "override file missing → None (no fixed fallback)"
        );
        std::env::remove_var(ENV_OVERRIDE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── ADR-0092 hardening: absolute-or-None contract (Codex #3) ──────────────────────────────
    #[test]
    fn relative_base_yields_none_never_relative_path() {
        let _env = ENV_LOCK.lock().unwrap();
        std::env::remove_var(ENV_OVERRIDE);
        let provider = FilePrimingProvider::new(PathBuf::from("relative-base"));
        assert!(
            provider.priming_file().is_none(),
            "relative base → cannot be made absolute → None (no relative path leak)"
        );
    }

    // ── ADR-0092 hardening: cmd.exe metacharacter path → None (Codex #1+#5) ────────────────────
    #[test]
    fn cmd_metachar_in_path_yields_none() {
        let _env = ENV_LOCK.lock().unwrap();
        let root =
            std::env::temp_dir().join(format!("engram-priming-meta-{}", uuid::Uuid::new_v4()));
        let meta_base = root.join("we&ird%dir");
        let prompts = meta_base.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        let mut f = std::fs::File::create(prompts.join("agent-priming.md")).unwrap();
        writeln!(f, "# meta").unwrap();
        std::env::remove_var(ENV_OVERRIDE);
        let provider = FilePrimingProvider::new(meta_base.clone());
        assert!(
            provider.priming_file().is_none(),
            "path containing cmd metacharacters (% &) → None (not injected due to corruption risk)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── ADR-0092 hardening: path_is_cli_safe unit (per metacharacter) ────────────────────────────
    #[test]
    fn path_is_cli_safe_rejects_each_metachar() {
        for c in CMD_UNSAFE_CHARS {
            let p = std::path::PathBuf::from(format!("C:/base/na{c}me/agent.md"));
            assert!(
                !path_is_cli_safe(&p),
                "a path containing metacharacter {c:?} must be refused"
            );
        }
        assert!(
            path_is_cli_safe(std::path::Path::new("C:/base/prompts/agent-priming.md")),
            "a path without metacharacters is safe"
        );
    }

    // ── ADR-0099/0126: consistency invariant pin (content-based) — the channel the production priming file actually teaches ──────
    /// Tests run inside the compile-time source tree, so MANIFEST_DIR is trustworthy.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")) // .../crates/engram-dashboard-daemon
            .parent()
            .and_then(|p| p.parent())
            .expect("repo root")
            .to_path_buf()
    }

    // ── Decider normalization (the weakness of two-word tokens) ────────────────────────────────────────────────
    /// If reverted, an MCP-capable spawn passes carrying CLI-teaching priming — each case is one such bypass path.
    #[test]
    fn teaches_mail_cli_survives_whitespace_case_and_markdown_between_the_two_words() {
        for text in [
            "run engram mail send --to bob",
            "RUN ENGRAM MAIL SEND",
            "Use Engram Mail Send to deliver.",
            "run `engram mail send`",
            "run **engram** mail send",
            "run *engram* *mail* send",
            "run engram  mail send",    // two spaces
            "run engram\nmail send",    // a sentence broken by a line break
            "run engram\n   mail send", // line break + indentation
            "run engram\tmail send",
        ] {
            assert!(
                teaches_mail_cli(text),
                "must be caught as CLI teaching: {text:?}"
            );
        }
    }

    /// A negative case: reverting to bare-name decisions would catch all MCP-only priming as CLI teaching and reject MCP runs before spawn.
    #[test]
    fn mcp_only_prose_is_not_mistaken_for_cli_teaching() {
        for text in [
            "call the MCP tool `eg_send` with the recipient and body.",
            "on the `engram` server",
            "**engram** server tools are listed at startup",
            "its body opens with an [engram] marker",
            "the Engram broker daemon attaches the envelope",
            "",
        ] {
            assert!(
                !teaches_mail_cli(text),
                "must not be CLI teaching: {text:?}"
            );
            assert!(
                !mentions_mail_cli_surface(text),
                "must not be a CLI surface trace either: {text:?}"
            );
        }
    }

    /// If reverted to a boundary-less substring, ordinary prose gets caught — a misdiagnosis: the prohibition gate flags an innocent sentence and the message instructs the operator to delete it.
    #[test]
    fn ordinary_prose_that_merely_contains_the_tokens_is_not_a_match() {
        for text in [
            "The Engram mailbox is managed by the daemon.",
            "engram mailbox",
            "engram mailing list",
            "pass the --token to the server",
            "see the --tool and --topic switches",
            "the --bodyguard flag is unrelated",
        ] {
            assert!(!teaches_mail_cli(text), "must not be teaching: {text:?}");
            assert!(
                !mentions_mail_cli_surface(text),
                "must not be a surface trace either: {text:?}"
            );
        }
    }

    /// A construct invisible in rendering that used to pass straight through normalization — it bypasses the prohibition gate.
    #[test]
    fn markdown_constructs_between_the_two_words_do_not_hide_the_command() {
        for text in [
            "Run engram<!--split--> mail pending",
            "Run engram <!-- comment --> mail pending",
            "Run engram\u{200B} mail pending",
            "Run engram\u{00AD} mail pending",
            "Run engram\\ mail pending",
            // Without character folding before structure removal, this combination passes with the comment kept alive.
            "Run engram<!--split--\u{200B}> mail pending",
            "Run engram<!--split--\\> mail pending",
            // Separators must become spaces, not be deleted, for adjacency to survive.
            "run \"${ENGRAM_CLI_EXE}\" mail send --to alice --body hi",
            "run `engram` `mail` `send`",
        ] {
            assert!(
                teaches_mail_cli(text),
                "must be caught as CLI teaching: {text:?}"
            );
        }
        // An unclosed comment does not discard what follows — priming is injected verbatim, so cutting it like a renderer would leave text visible to the agent that vanishes only for the decider.
        assert!(
            teaches_mail_cli("intro <!-- unterminated\nRun engram mail send --to bob"),
            "teaching after an unclosed comment must be seen too"
        );
    }

    /// Calling a variable name and mentioning it differ: a prohibition sentence must not count as teaching, while a real call through the variable (the old priming's form — ADR-0098) must.
    #[test]
    fn env_var_counts_as_teaching_only_in_an_invocation_shape() {
        let mention = "ENGRAM_CLI_EXE is reserved for diagnostics; do not use it to send mail.";
        assert!(
            !teaches_mail_cli(mention),
            "a bare mention + a prohibition is not teaching: {mention:?}"
        );
        assert!(
            mentions_mail_cli_surface(mention),
            "still a CLI surface trace, so it must be caught in the prohibition direction: {mention:?}"
        );

        for text in [
            "run `$ENGRAM_CLI_EXE mail send --to alice --body hi`",
            "invoke $engram_cli_exe mail pending",
            "invoke $ENGRAM\\_CLI\\_EXE mail pending",
        ] {
            assert!(
                teaches_mail_cli(text),
                "a variable call is teaching: {text:?}"
            );
        }
    }

    /// Catches only the broad signal — a document with no executable call (used only in the prohibit direction).
    #[test]
    fn surface_evidence_without_a_runnable_invocation_is_not_teaching() {
        for text in [
            "pass --to <name> and --body <text> to deliver",
            // A verbless fragment — it cannot execute, so it is not "taught".
            "Run engram mail",
            "the engram mail surface exists",
            // Because separators are folded into spaces, this notation also looks like a flag.
            "the `--to`-recipient argument",
        ] {
            assert!(mentions_mail_cli_surface(text), "surface trace: {text:?}");
            assert!(
                !teaches_mail_cli(text),
                "it did not teach the command (if this passed in the requirement direction, B would run without knowing how to send): {text:?}"
            );
        }
    }

    /// The known limit is pinned by a test: negation is still caught as "teaching" — deciding sentence negation is a heuristic swamp, deliberately not attempted (loud refusal > quiet misattribution). If this goes red, someone put in negation decisions — a decision matter.
    #[test]
    fn a_prohibition_sentence_still_reads_as_teaching_known_limit() {
        assert!(teaches_mail_cli(
            "Do NOT use engram mail send; use MCP instead."
        ));
    }

    /// The place teaching the entry name is no longer this file: priming shrank to a one-line pointer (`engram help`), and the only place an agent reads `eg_send` is that tool's description (rmcp puts it in tools/list), so that half moved to
    /// `control/mcp_server.rs::the_send_message_entry_teaches_its_own_call` — the file owning the surface guards it. What remains is the half whose subject is still the priming file: the absence of the CLI surface under ADR-0126 decision 1 (abolishing roundabout teaching).
    // ADR-0126
    // ADR-0128
    #[test]
    fn production_priming_file_carries_no_mail_cli_surface() {
        let root = repo_root();
        let a = std::fs::read_to_string(root.join(REL_MCP_PRIMARY)).expect("A priming file exists");
        // Total absence of A's CLI surface — exactly the ADR-0126 impact/invariant check list (command notation + accompanying flags); erasing only the name leaves the roundabout teaching half alive.
        // Do not revert to a bare `contains`: scrambling notation with case, line breaks or markdown emphasis lets `!a.contains(...)` pass — this pin blocks only with the same normalization and word boundary as the decider, and does not falsely catch ordinary words like `--token`.
        //   (`mentions_mail_cli_surface` = call form ∪ `engram mail` fragment ∪ env variable names ∪ CLI-only flags.)
        assert!(
            !mentions_mail_cli_surface(&a),
            "if a CLI surface (a call form · an `engram mail` fragment · an env variable name · a CLI-only flag) comes back \
             into A(McpPrimary), it is a regression of ADR-0126 decision 1 (bypass teaching abolished) — the daemon refuses that spawn's \
             CLI mail, so it could not be used even if taught (ADR-0133)"
        );
    }

    /// Control discovery is unrelated to the mail channel (ADR-0132 decisions 4 and 5 · ADR-0133): control verbs are open to everyone, so priming points at that surface with one line per family (`engram help <family>`). The mail family now stands in that list unconditionally — the marker spawn used to plant was withdrawn by ADR-0220 decision 4.
    ///
    /// This line must not touch the decider, or A's CLI-surface absence pin (the test above) breaks. Do not loosen that pin by putting the control family into the decider — control is a surface everyone uses, so it cannot separate channels, and the gate would be meaningless in both directions (authority: the `teaches_mail_cli` comment).
    // ADR-0133
    #[test]
    fn production_priming_files_point_at_the_control_surface() {
        let root = repo_root();
        let a = std::fs::read_to_string(root.join(REL_MCP_PRIMARY)).expect("A priming file exists");
        let discovery = format!("{CLI_EXE_NAME} help");
        assert!(
            a.contains(&discovery),
            "A(McpPrimary): must point to the control discovery entry (`{discovery}`)"
        );
        assert!(
            !teaches_mail_cli(&discovery),
            "the discovery line itself must not be mail teaching (if it reads that way, A's pin breaks)"
        );
        assert!(
            !mentions_mail_cli_surface(&discovery),
            "the discovery line must not be a mail surface trace either"
        );
    }

    // The escalation pin was here and is gone: a test held the "broken channel" escalation sentence the old ADR-0126 decisions 2 and 5 required; it was withdrawn together with the four behavior-rule lines of the priming body (user decision 2026-09-19) and will not come back.
    //   The remnant rewired afterwards (`production_priming_file_points_at_discovery_entrypoint`) strictly duplicated `production_priming_files_point_at_the_control_surface` above (which carries two more invariants), so it was deleted 2026-09-20 with no coverage loss.
    // So no pin holds the escalation axis now — if that sentence quietly comes back, or becomes needed again, no gate tells us. The gap record: ADR-0213 Impact section.

    // No pin for the C3 reply contract (ADR-0103 decisions 2/3) here — do not revive: priming no longer carries the envelope grammar (shrank to a pointer). The contract is split across two surfaces, each file guarding its own — envelope recognition (`type="request"` · `<notice>` · reply with the received id) by `the_mail_screen_teaches_the_reply_contract` in `control/help.rs`,
    //   tool argument notation (snake_case `reply_to`·`reply_by`) by the input-schema pin in `control/mcp_server.rs` (not the description — ADR-0220 decision 3 moved the argument contract's home to the schema). Priming carries no behavior rules either (those four lines and the escalation pin were withdrawn together). The only load-bearing property left is the one discovery entry line; dragging that grammar back would split one contract in three places.
}
