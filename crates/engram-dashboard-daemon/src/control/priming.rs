//! PrimingProvider seam (ADR-0092 receive contract) — yields the **absolute path** of the priming file to inject
//! into the system prompt at spawn.
//!
//! ★Role★: at every spawn (provision) the daemon asks this seam for the priming file path, and if there is one sends that absolute path
//!   as `ControlEndpoint.priming_file`. This module deals with the **path only** — it does not read the file
//!   content (no hardcoding, ADR-0092: the content lives only in the external MD `prompts/agent-priming.md`).
//!
//! ★Why a seam (ADR-0092 "leave the road open")★: the current implementation (`FilePrimingProvider`) gives **one shared file
//!   regardless of agent** (a stopgap). When a per-agent prompt injection / skill registration system arrives in the future, only this
//!   trait's implementation is swapped (per-agent · per-capability priming) and it is absorbed without changing the wiring.
//!   So provision is made to depend only on the `PrimingProvider` trait.
//!
//! ★Graceful (does not block the spawn)★: if there is no resolved file it returns `None` and leaves only a warn log — the absence of
//!   priming is not a reason for spawn failure (a **different** policy from the control channel provision's fail-closed). The agent
//!   comes up without priming (the receive contract is not applied, but it works functionally).
//!
//! tauri import 0 (daemon crate).
// ADR-0092

use std::path::PathBuf;

use engram_dashboard_agent::types::{
    CLI_EXE_ENV, CLI_EXE_NAME, CLI_GROUP_MAIL, CLI_MAIL_FLAGS, CLI_MAIL_VERBS,
};

/// A character that continues a token — if this character is attached on either side of a match, it is **part of another word**.
///
/// ★Looking at a bare substring with no boundary catches ordinary prose (measured review)★: `The Engram mailbox is managed by the
///   daemon.` contains `engram mail` and `--token` contains `--to`. That false positive is not mere noise but a
///   **misdiagnosis** — the prohibition gate flags an innocent sentence, and the error message tells the operator to delete that sentence.
///   The reason `-` and `_` are included is that flags and identifiers are joined by those characters (`--to-be`·`--token`).
fn continues_token(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '-' || ch == '_'
}

/// Does `needle` appear delimited by a **word boundary**. `engram mail send` ✓ · `engram mailbox` ✗ ·
/// `--to bob` ✓ · `--token` ✗.
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

/// Normalization for token matching in the priming body. **The order is load-bearing**: ① character-level folding (lowercase · separators →
/// space · deleting meaningless characters) → ② structure removal (HTML comments) → ③ whitespace collapse. **Every decision arm sees only
/// a string that passed through this** (no raw-text comparison path) — so there is no exception like "some arm folds and some arm does not".
///
/// ★Why ① comes before ② (reversed, they neutralize each other — measured review)★: if a zero-width character or an escape gets inside the comment-closing
///   marker (`<!--x--` + zero-width + `>`), ② sees that comment as unclosed and leaves it. Then the command passes the decision while broken up
///   by the comment text. If folded first, that combination does not hold.
/// ★Why normalization is needed (the weakness of multi-word tokens)★: the decision token is several words, so compared in raw form,
///   a sentence broken by a line break, consecutive spaces, and emphasis like `**engram** mail` all read as "does not teach". A misjudgment in that direction
///   lets CLI-teaching priming pass for an MCP-capable spawn and makes ADR-0099's send freeze get
///   misattributed as a normal negative — the very class the decider was meant to block.
/// ★Two character classes are told apart★: **separators** (`*`·backtick·`~`·braces·quotes) stand **between** words so they are
///   replaced **with a space** — deleting them would **join** `` `--to`-recipient `` into `--to-recipient` and flip the decision,
///   and would break the adjacency of `"${ENGRAM_CLI_EXE}" mail send`. **Meaningless characters** (`_`·backslash·zero-width kinds) get stuck **inside**
///   a word so they are deleted — an escaped env name must fold into one token to match the real name.
/// ★An unclosed `<!--` is kept★: priming is not rendered and is injected into the system prompt **verbatim**, so
///   throwing away the whole tail by imitating a renderer would leave text visible to the agent that vanishes only for the decider.
/// ★The token we search for is also made from a value that went through the same normalization★: do not type a literal by hand; use `normalized_token()`.
fn normalize_for_token_match(content: &str) -> String {
    // ① Character-level folding.
    let mut folded = String::with_capacity(content.len());
    for ch in content.chars() {
        match ch {
            // Separators that stand between words — to a space (deleting would join the tokens on either side).
            '*' | '`' | '~' | '{' | '}' | '"' | '\'' => folded.push(' '),
            // Meaningless characters that get inside a word — delete (markdown emphasis/escape + invisible formatting characters).
            '_' | '\\' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{200E}' | '\u{200F}'
            | '\u{2060}' | '\u{FEFF}' | '\u{00AD}' => {}
            other => folded.extend(other.to_lowercase()),
        }
    }

    // ② Structure removal — closed HTML comments only.
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

    // ③ Whitespace collapse (including consecutive spaces made by ① and ②, all at once here).
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

/// Fold the token we search for by the **same rules** as the body (no hand-typing literals — they move together when normalization changes).
fn normalized_token(raw: &str) -> String {
    normalize_for_token_match(raw)
}

/// Does the body **teach mail CLI sending** (strong signal) — the decision target is **only the executable call form**:
/// the executor (executable name or env variable) + family token + the adjacency of a **verb** (`engram mail send` etc.).
///
/// ★A mere appearance of the name is not teaching★: `ENGRAM_CLI_EXE is reserved for diagnostics; do not use it to send
///   mail.` is a **sentence saying not to use it**, yet the old decision counted it as teaching. So a variable is teaching only when a family token
///   follows it, and a bare mention is counted only as `mentions_mail_cli_surface` (surface trace).
/// ★Why the variable form is not removed from teaching (it is a real teaching form)★: a call made through a variable **never
///   writes the command name** — `run \`$ENGRAM_CLI_EXE mail send --to alice\`` is the example, and the old
///   priming recorded in ADR-0098 was exactly that form (that is why the grant was unmatched). If this is removed from teaching, such a file
///   reads as "does not teach sending" and the `--cli-only` requirement gate rejects a perfectly fine experiment.
/// ★Decide by body, not by basename — do not revert★: the earlier edition looked only at a hardcoded basename list, and
///   when new CLI-instructing priming was missing from that list, the guard was quietly bypassed and the absence of the CLI binary (absence of infrastructure) was
///   misattributed as a normal negative (B_SENT=false) instead of SETUP-SKIP. Folding case, whitespace and emphasis is for the same reason.
/// ★Looking at the bare executable name is not allowed★: MCP priming legitimately uses the same word as the MCP **server name**
///   (`on the engram server`), so a bare decision catches MCP-only files as CLI-instructing and rejects every MCP run before spawn.
///   So the decision is **adjacency to the family token**, and that match is cut at word boundaries (`engram mailbox` excluded).
/// ★A verb is required as well★: `engram mail` alone is a **fragment that cannot be executed**, so "taught" would be false.
///   If that fragment were counted as teaching, the `--cli-only` requirement gate would be satisfied, and B's non-send because it does not know the verb would be graded as a normal negative
///   — the very misattribution this decider is meant to block. The verb list is agent `CLI_MAIL_VERBS` (shared with the parser).
/// ★What this gate **cannot catch** (known limits — read before trying to extend it)★:
///   - **Negation**: `Do NOT use engram mail send; use MCP instead.` is still caught as "teaching". Deciding the
///     negation of a sentence by text is a heuristic swamp, so we **deliberately do not attempt it** (user decision
///     2026-08-04, reconfirmed this round). The price is a pointless SETUP-FAIL when passing such a file without `--cli-only`,
///     and the two production primings contain no such sentence, so the real-path impact is 0. It points the same way as this gate's stance that **a loud refusal is
///     better than a quiet misattribution**.
///   - **Shell syntax in general**: this is a token-adjacency matcher, not a shell parser. Variable rebinding (`X=$ENGRAM_CLI_EXE; $X mail
///     send`), aliases and calls split across lines are not seen. Instead of growing this into a parser, if that class really appears, we then
///     fix the priming side.
///   - **Bracket-type separators**: `{}` and quotes are folded as between-word symbols but `()` and `[]` are not — this is to avoid a false positive where prose like `engram
///     (mail) 계열` (engram (mail) family) gets caught as a command (that side is worse).
/// ★Limiting the family **to mail** is also intentional (do not widen it to arbitrary families)★: the invariant this decision guards is
///   ADR-0128/0099's **mail** teaching↔wiring equality, and the misattributed observation is also a mail send (`B_SENT`). The control family
///   (`engram help` · the later `engram agent …`) is a surface **every** spawn uses (ADR-0132 decisions 4 and 5), so counting even that
///   would make ① MCP priming rejected over a single control line and ② "`--cli-only` priming that teaches no sending method at all"
///   pass the reverse gate — the misattribution now being blocked would revive on both sides.
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

/// Does the body have **a trace of the mail CLI surface** (broad signal) — strong signal ∪ CLI absolute-path env variable names ∪ CLI-only
/// flag notation.
///
/// ★Why it is kept separate from the strong signal (merge the two and one side is certain to be wrong)★: the two gates need opposite things.
///   - **Prohibit direction** (an MCP-capable spawn must not carry CLI teaching): when in doubt, blocking is safe →
///     use this broad signal. A document that writes only flags and variables without the command name has also leaked the CLI surface.
///   - **Require direction** (`--cli-only` **requires** CLI send teaching): if this broad signal were used here, a document that only brushes a flag and
///     does not teach the call form would pass, and an experiment where B runs without knowing how to send would be graded valid.
///     That side must be `teaches_mail_cli` (the strong signal).
pub fn mentions_mail_cli_surface(content: &str) -> bool {
    if teaches_mail_cli(content) {
        return true;
    }
    let normalized = normalize_for_token_match(content);
    // A verbless `engram mail` fragment is also counted as a **surface** — it cannot execute, but it is a document where the CLI surface has leaked.
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
    /// The **absolute path** of the priming MD file to inject into this spawn. `None` if it is absent (file missing) or unconfigured.
    /// ★Absolute path contract★: the agent's cwd differs from the daemon/repo (each workspace), so it must be an absolute path
    ///   for claude to find the file — a relative path would be resolved against the agent's cwd and miss.
    ///
    /// ★**Who gets the injection is not decided by this seam**★: the material for deciding the consistency invariant (the mail
    ///   channel priming **teaches** == the mail channel that spawn can actually **use** — violate it and you get the send freeze ADR-0099 measured)
    ///   is `uses_mail` and `accepts_mcp_config`, and only the daemon knows them. So provision first chooses **whether to call**,
    ///   and this seam answers only 「줄 파일이 있나」 (is there a file to give). The authority for that pairing is
    ///   `DaemonControlChannel::provision`.
    // ADR-0126
    // ADR-0133
    fn priming_file(&self) -> Option<PathBuf>;
}

/// ★Why base is taken as the exe-based root (ADR-0092, both reviewers PRIMARY)★: earlier, base was the daemon process cwd
///   (`from_cwd`) — but the production daemon is launched through WMI Win32_Process.Create and **does not inherit the parent cwd**
///   (cwd=System32), so priming was **quietly disabled**. The fix = reuse the **same exe-walk-up pattern**
///   (`data_dir::find_install_root`) that `default_data_dir` uses to anchor `.engram-dev`, and take a
///   trustworthy absolute root as base (distrust cwd).
///
/// ★Why base injection (new)★: this module does not resolve the root itself but takes base at construction — so tests can
///   swap base for a temp dir and verify deterministically without cwd/exe contamination (seam-like). The production wiring is from_install_root.
pub struct FilePrimingProvider {
    base_dir: PathBuf,
}

const REL_MCP_PRIMARY: &str = "prompts/agent-priming.md";
const ENV_OVERRIDE: &str = "ENGRAM_PRIMING_FILE";

/// ★Characters that corrupt under cmd.exe (ADR-0092, Codex #1+#5)★: on Windows, claude arguments are wrapped by `console_command`
///   (the platform crate's `shell`) as `cmd.exe /c claude …` and run — this path expands `%VAR%` **even inside quotes** and
///   interprets `& ^ | < >` as shell metacharacters, corrupting the argument. If a priming path contains these characters,
///   claude would receive a wrong/truncated path, so we do not inject at all (None).
///   ★PRE-EXISTING · separate follow-up (scope, ADR-0092)★: the cmd.exe escaping flaw of `console_command` itself is
///   not introduced by this slice but is an **existing** problem, and `--mcp-config` (config_path), carried by the same path, is
///   exposed in the same way (that one is a path the daemon makes so it is practically safe, but the principle is the same). Here we do only the
///   slice-level cheap defense (the priming path filter) and do not fix console_command — that is tracked as a separate task that
///   affects the arguments of **all backends** (avoiding scope creep).
const CMD_UNSAFE_CHARS: &[char] = &['%', '&', '^', '|', '<', '>'];

/// ★Why UTF-8 is checked too (Codex #5)★: arguments are finally stringified with `to_string_lossy()` and carried to the CLI
///   (`backend/claude/`). A non-UTF8 path is **corrupted** to U+FFFD by that lossy conversion and claude receives a nonexistent path.
///   Such a path is not injected in the first place (a corrupted path < no priming).
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

    /// If the root cannot be obtained (current_exe failure etc.), base is set to `.` (relative) — in that case absolutize fails to
    ///   make it absolute and yields None (a relative path is never injected).
    pub fn from_install_root() -> Self {
        let base = crate::data_dir::find_install_root().unwrap_or_else(|| PathBuf::from("."));
        Self::new(base)
    }

    /// ★absolute-or-None contract (ADR-0092, Codex #3)★: earlier, if base was relative it **tried** to make it absolute once more with current_dir,
    ///   and if even that failed it returned the relative PathBuf as it was.
    ///   Now the cwd fallback is removed, and if it cannot be made absolute it strictly yields None (a relative path never becomes Some).
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
                "프라이밍 경로를 절대경로로 해석 못 함(base 가 절대 아님) — 프라이밍 없이 스폰 진행(ADR-0092 graceful)"
            );
            return None;
        };
        if !path_is_cli_safe(&abs) {
            tracing::warn!(
                label,
                path = %abs.display(),
                "프라이밍 경로가 비-UTF8 이거나 cmd.exe 메타문자(% & ^ | < >)를 포함 — 부패 위험으로 미주입(ADR-0092)"
            );
            return None;
        }
        // ★TOCTOU residue (accepted, ADR-0092 Codex #4)★: if the file disappears between the moment is_file passes here and the moment claude
        //   actually opens the file (after spawn), claude comes up without priming or raises an error. The priming
        //   file is stable infrastructure (version-controlled MD), not a racing target, so the risk is low — accepted as a best-effort existence check and
        //   no lock is taken (not worth the cost). The residual risk is on par with graceful absence (the spawn still comes up).
        if abs.is_file() {
            return Some(abs);
        }
        tracing::warn!(
            label,
            path = %abs.display(),
            "프라이밍 파일을 못 찾음 — 프라이밍 없이 스폰 진행(ADR-0092 graceful)"
        );
        None
    }
}

impl PrimingProvider for FilePrimingProvider {
    fn priming_file(&self) -> Option<PathBuf> {
        // ★An override failure does not fall back to fixed★: quietly swapping an explicit override for a different file is
        //   confusing. Whichever gate it is caught at, None (proceed without priming — resolve_checked warns).
        // ★The env override beats the fixed file (ADR-0099 test-seam)★: a harness/operator can
        //   force a specific priming (roundtrip_smoke `--priming` passes it through this env). Production leaves it unset so the fixed
        //   file below lives. ★But a spawn that provision has **decided not to call** this seam is not revived by this override either★ —
        //   that decision is one body with the mail authorization, so it must not be flipped by a single env (the trait doc is the authority).
        if let Some(v) = std::env::var_os(ENV_OVERRIDE) {
            if !v.is_empty() {
                return self.resolve_checked(PathBuf::from(v), "override");
            }
        }

        self.resolve_checked(PathBuf::from(REL_MCP_PRIMARY), "fixed")
    }
}

/// Fills the seam in tests and paths unrelated to priming while keeping `--append-system-prompt-file` from being attached.
/// Removing the dependence on a real file keeps the test deterministic.
pub struct NoopPrimingProvider;

impl PrimingProvider for NoopPrimingProvider {
    fn priming_file(&self) -> Option<PathBuf> {
        None
    }
}

/// Fixed-path provider (test only) — returns the given absolute path as is (no existence check). Used in integration tests that
/// confirm priming is carried down through the spawn wiring.
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

    /// ★env is process-global★: when tests that touch `ENGRAM_PRIMING_FILE` run in parallel, set/remove
    ///   erase each other (flaky). This mutex serializes those tests (cargo is parallel by default so it is required).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_fixture_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("engram-priming-test-{}", uuid::Uuid::new_v4()));
        let prompts = dir.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        let mut f = std::fs::File::create(prompts.join("agent-priming.md")).unwrap();
        writeln!(f, "# 테스트 프라이밍 (mcp-primary)").unwrap();
        dir
    }

    #[test]
    fn resolves_fixed_relative_to_absolute_under_base() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = make_fixture_dir();
        let provider = FilePrimingProvider::new(dir.clone());
        std::env::remove_var(ENV_OVERRIDE);
        let got = provider.priming_file().expect("고정 파일이 있으면 Some");
        assert!(got.is_absolute(), "해석 결과는 절대경로여야: {got:?}");
        assert!(
            got.ends_with("prompts/agent-priming.md") || got.ends_with("prompts\\agent-priming.md")
        );
        assert!(got.is_file(), "실제 파일을 가리켜야");
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
        assert!(provider.priming_file().is_none(), "부재 파일 → None");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_override_wins_over_fixed() {
        let _env = ENV_LOCK.lock().unwrap();
        let dir = make_fixture_dir();
        let override_file = dir.join("custom-priming.md");
        {
            let mut f = std::fs::File::create(&override_file).unwrap();
            writeln!(f, "# override 프라이밍").unwrap();
        }
        std::env::set_var(ENV_OVERRIDE, &override_file);
        let provider = FilePrimingProvider::new(dir.clone());
        let got = provider.priming_file().expect("override 파일 존재 → Some");
        assert!(got.is_absolute());
        assert!(
            got.ends_with("custom-priming.md"),
            "override 경로가 이겨야: {got:?}"
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
            "override 파일 부재 → None(fixed 폴백 안 함)"
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
            "상대 base → 절대화 불가 → None(상대경로 유출 금지)"
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
            "cmd 메타문자(% &) 포함 경로 → None(부패 위험 미주입)"
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
                "메타문자 {c:?} 포함 경로는 거부되어야"
            );
        }
        assert!(
            path_is_cli_safe(std::path::Path::new("C:/base/prompts/agent-priming.md")),
            "메타문자 없는 경로는 안전"
        );
    }

    // ── ADR-0099/0126: consistency invariant pin (content-based) — the channel the production priming file actually teaches ──────
    /// Tests always run inside the compile-time source tree, so MANIFEST_DIR is trustworthy.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")) // .../crates/engram-dashboard-daemon
            .parent()
            .and_then(|p| p.parent())
            .expect("repo 루트")
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
            assert!(teaches_mail_cli(text), "CLI 교육으로 잡혀야: {text:?}");
        }
    }

    /// If it goes back to bare-name decisions, all MCP-only priming gets caught as CLI teaching and MCP runs are
    /// rejected before spawn — a negative case that prevents that regression.
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
            assert!(!teaches_mail_cli(text), "CLI 교육이 아니어야: {text:?}");
            assert!(
                !mentions_mail_cli_surface(text),
                "CLI 표면 흔적도 없어야: {text:?}"
            );
        }
    }

    /// ★If reverted to a boundary-less substring, ordinary prose gets caught★ — that false positive is not noise but a misdiagnosis:
    ///   the prohibition gate flags an innocent sentence, and the message instructs the operator to delete that sentence.
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
            assert!(!teaches_mail_cli(text), "교육이 아니어야: {text:?}");
            assert!(
                !mentions_mail_cli_surface(text),
                "표면 흔적도 아니어야: {text:?}"
            );
        }
    }

    /// A construct that does not show in rendering but used to pass straight through normalization — it bypasses the prohibition gate.
    #[test]
    fn markdown_constructs_between_the_two_words_do_not_hide_the_command() {
        for text in [
            "Run engram<!--split--> mail pending",
            "Run engram <!-- comment --> mail pending",
            "Run engram\u{200B} mail pending",
            "Run engram\u{00AD} mail pending",
            "Run engram\\ mail pending",
            // If character folding does not run **before** structure removal, this combination passes with the comment kept alive.
            "Run engram<!--split--\u{200B}> mail pending",
            "Run engram<!--split--\\> mail pending",
            // Separators must be turned **into spaces** rather than deleted for adjacency to survive.
            "run \"${ENGRAM_CLI_EXE}\" mail send --to alice --body hi",
            "run `engram` `mail` `send`",
        ] {
            assert!(teaches_mail_cli(text), "CLI 교육으로 잡혀야: {text:?}");
        }
        // An unclosed comment does not discard what follows — priming is injected verbatim without rendering, so
        //   cutting it out by imitating a renderer would leave text visible to the agent that vanishes only for the decider.
        assert!(
            teaches_mail_cli("intro <!-- unterminated\nRun engram mail send --to bob"),
            "닫히지 않은 주석 뒤의 교육도 보여야"
        );
    }

    /// ★**Calling** a variable name and **mentioning** it are different★: a prohibition sentence must not be counted as teaching,
    ///   while a real call made through the variable (the form of the old priming — ADR-0098) must be counted as teaching.
    #[test]
    fn env_var_counts_as_teaching_only_in_an_invocation_shape() {
        let mention = "ENGRAM_CLI_EXE is reserved for diagnostics; do not use it to send mail.";
        assert!(
            !teaches_mail_cli(mention),
            "맨 언급 + 금지문은 교육이 아니다: {mention:?}"
        );
        assert!(
            mentions_mail_cli_surface(mention),
            "그래도 CLI 표면 흔적이므로 금지 방향에선 걸려야: {mention:?}"
        );

        for text in [
            "run `$ENGRAM_CLI_EXE mail send --to alice --body hi`",
            "invoke $engram_cli_exe mail pending",
            "invoke $ENGRAM\\_CLI\\_EXE mail pending",
        ] {
            assert!(teaches_mail_cli(text), "변수 호출은 교육이다: {text:?}");
        }
    }

    /// A spot that catches only the broad signal — a document that does not write an executable call (used only in the prohibit direction).
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
            assert!(mentions_mail_cli_surface(text), "표면 흔적: {text:?}");
            assert!(
                !teaches_mail_cli(text),
                "명령을 가르친 것은 아니다(요구 방향에서 이게 통과하면 B 는 발신법을 모른 채 돈다): {text:?}"
            );
        }
    }

    /// ★The known limit is **pinned by a test**★: negation is still caught as "teaching" — deciding sentence negation is
    ///   a heuristic swamp so we deliberately do not attempt it (loud refusal > quiet misattribution). If this test goes red,
    ///   someone put in negation decisions, and that is a decision matter.
    #[test]
    fn a_prohibition_sentence_still_reads_as_teaching_known_limit() {
        assert!(teaches_mail_cli(
            "Do NOT use engram mail send; use MCP instead."
        ));
    }

    /// ★The place that teaches the entry **name** is no longer this file★: priming shrank to a one-line pointer
    ///   (`engram help`), and the only place an agent reads the word `eg_send` is that tool's **description**
    ///   (rmcp puts it in tools/list). So that half moved to
    ///   `control/mcp_server.rs::the_send_message_entry_teaches_its_own_call` — the
    ///   file that owns the surface guards that surface. What remains here is the half **whose subject is still the priming file**,
    ///   that is, the absence of the CLI surface under ADR-0126 decision 1 (abolishing roundabout teaching).
    // ADR-0126
    // ADR-0128
    #[test]
    fn production_priming_file_carries_no_mail_cli_surface() {
        let root = repo_root();
        let a = std::fs::read_to_string(root.join(REL_MCP_PRIMARY)).expect("A 프라이밍 파일 존재");
        // **Total** absence of A's CLI surface — exactly the check list of the ADR-0126 impact/invariant (command notation + accompanying flags).
        //   If only the name is erased and flag notation remains, the roundabout teaching survives in half.
        // ★Do not revert to a bare `contains`★: even merely scrambling the notation with case, line breaks or markdown emphasis
        //   lets `!a.contains(...)` pass — this pin actually blocks only when it uses **the same normalization and the same word boundary** as the decider,
        //   and it also does not falsely catch ordinary words like `--token`.
        //   (`mentions_mail_cli_surface` = call form ∪ `engram mail` fragment ∪ env variable names ∪ CLI-only flags.)
        assert!(
            !mentions_mail_cli_surface(&a),
            "A(McpPrimary)에 CLI 표면(호출 형태 · `engram mail` 조각 · env 변수 이름 · CLI 전용 플래그 중 \
             하나)이 다시 들어오면 ADR-0126 결정 1(우회 교육 폐지)의 회귀 — 그 스폰의 CLI 우편은 데몬이 \
             거절하므로 가르쳐도 쓸 수 없다(ADR-0133)"
        );
    }

    /// ★Control discovery is unrelated to the mail channel (ADR-0132 decisions 4 and 5 · ADR-0133)★: control verbs are open to everyone, so
    ///   priming points to that surface with one line per family (`engram help <계열>` (family)). ★The mail family now stands in that list
    ///   unconditionally★ — the marker spawn used to plant was withdrawn by ADR-0220 decision 4.
    ///
    /// ★This line must not touch the decider★: otherwise A's CLI-surface absence pin (the test above) breaks. Do not fix it by putting the control family into the
    ///   decider to loosen that pin — control is a surface everyone uses so it cannot be a signal that separates channels, and putting it in makes the gate meaningless in both
    ///   directions (the `teaches_mail_cli` comment is the authority).
    // ADR-0133
    #[test]
    fn production_priming_files_point_at_the_control_surface() {
        let root = repo_root();
        let a = std::fs::read_to_string(root.join(REL_MCP_PRIMARY)).expect("A 프라이밍 파일 존재");
        let discovery = format!("{CLI_EXE_NAME} help");
        assert!(
            a.contains(&discovery),
            "A(McpPrimary): 제어 발견 입구(`{discovery}`)를 가리켜야"
        );
        assert!(
            !teaches_mail_cli(&discovery),
            "발견 한 줄 자체는 우편 교육이 아니어야(그렇게 읽히면 A 의 pin 이 깨진다)"
        );
        assert!(
            !mentions_mail_cli_surface(&discovery),
            "발견 한 줄은 우편 표면 흔적도 아니어야"
        );
    }

    // ★The escalation pin was here, and now it is not — the absence is noted★: a test at this spot held the "broken channel"
    //   escalation sentence that the old ADR-0126 decisions 2 and 5 required. That sentence was withdrawn **together with** the four behavior-rule
    //   lines of the priming body (user decision 2026-09-19) and will not come back.
    //   The remnant that was rewired once after that (`production_priming_file_points_at_discovery_entrypoint`) was
    //   a strict duplicate **looking at the same literal in the same file** as the
    //   `production_priming_files_point_at_the_control_surface` above (that one carries two more invariants on top), so it was deleted on 2026-09-20 — coverage
    //   did not shrink.
    // ★So there is **not a single pin** holding the escalation axis now★ — if that sentence quietly comes back to life, or conversely becomes needed again,
    //   no gate would tell us. The record of that gap is carried by the ADR-0213 「영향」 (impact) section.

    // ★There is no pin for the C3 reply contract (ADR-0103 decisions 2/3) here — do not revive it★: priming no longer carries the
    //   envelope grammar (it shrank to a pointer). The contract is split across two surfaces and each file guards its own —
    //   envelope recognition (`type="request"` · `<notice>` · reply with the received id) is covered by `the_mail_screen_teaches_the_reply_contract` in `control/help.rs`,
    //   and tool argument notation (snake_case `reply_to`·`reply_by`) by the **input schema** pin in
    //   `control/mcp_server.rs` (not the description — ADR-0220 decision 3 moved the
    //   home of the argument contract to the schema). ★Priming no longer carries behavior rules either★ — those four lines and the escalation pin were
    //   withdrawn together (the comment above). The only load-bearing property left in priming is the one discovery entry line, and
    //   dragging that grammar back here would split the same contract in three places.
}
