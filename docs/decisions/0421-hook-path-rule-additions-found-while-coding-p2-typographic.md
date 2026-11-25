# ADR-0421: Hook path rule additions found while coding P2: typographic double quotes, a dollar ending the hook folder name, non-UTF-8 folders; cmd percent left to P3

- 상태: 확정 (2026-11-25, 근거: main decisions during step 22 P2 coding, after the P2 code review (risk axis and lead) · measurements in PowerShell 7.5.5 and Git Bash 5.2 on the exact strings the rewrite writes)
- 관련: Amends ADR-0420 (decision 3 path rule) · ADR-0359 (Engram checks only what its own assembly breaks) · TRD `docs/process/S25-preset-library/trd-hook-components.md` §2.4 · `docs/research/hook-loading-claude-codex-2026-11-24.md` (which shell each CLI runs hooks in)

## 맥락
ADR-0420 decision 3 lists the characters a hook folder path may not hold when a definition uses the `${CLAUDE_PLUGIN_ROOT}` placeholder: a backtick anywhere, a `$` that is not the last character of a path component, and off Windows also `"`, `\` or a control character (`hook-path-unsafe`). Its premise, written out in TRD §2.4, is that `$(`, `${`, `$name`, backticks and the special parameters are the only ways to run code inside double quotes in Git Bash and PowerShell 7, and that the forward-slash path works in Git Bash, PowerShell and cmd.

The P2 risk review measured that PowerShell 7.5.5 treats U+201C, U+201D and U+201E as double quotes: a folder named with one of them closes the author's quoted string early and the rest runs (the probe printed `INJECTED`). Git Bash treats them as ordinary characters. claude runs hooks in Git Bash; codex runs them in PowerShell 7 on Windows (P3), so the gap reaches every codex hook once P3 calls the rewrite. The same review found two smaller gaps in the list as ADR-0420 had it (a `$` ending the hook folder's own name, and cmd's `%NAME%`), and the lead review found that the P2 code also refuses a non-UTF-8 folder, which neither ADR-0420 nor the TRD recorded.

## 결정
1. **Typographic double quotes are refused on every OS.** The path rule also refuses U+201C, U+201D and U+201E in the hook folder path, off Windows too (pwsh also runs off Windows, through `"shell": "powershell"`).
2. **A `$` that ends the last component is refused.** A `$` ending the hook folder's own name is followed by the author's template text, not by `/`, so the next character can turn it into `$-`, `$?`, `$$` or `$(` (measured in bash and pwsh). A `$` ending any component above the hook folder (a hidden share such as `//srv/team$/x`) still passes: a hook folder always sits below a mount's top, so a share is always an ancestor.
3. **A hook folder path that is not UTF-8 is refused.** The rewrite would otherwise put lossy text into a command. It is reachable only through a non-UTF-8 mount root.
4. **cmd is left to P3.** cmd expands `%NAME%` inside double quotes, so the 「works in cmd」 claim behind ADR-0420 decision 3 (TRD §2.4) is withdrawn. Whether codex's cmd fallback needs a `%` refusal is settled in P3.

The rule still lives in the rewrite function itself (`path_rule` in agent `backend/hooks.rs`, which `rewrite` runs first), and asset-library calls the same rule at check. The rest of ADR-0420 decision 3 stands.

## 거부한 대안
- **Keep the list as ADR-0420 had it** — measured injection: a folder named with U+201C, U+201D or U+201E ran the text after it in PowerShell 7.5.5. Evidence: 실측.
- **Refuse every non-ASCII character** — blocks legitimate non-English folder names; over-blocking against CLAUDE.md 「최소한만 막기」. Evidence: 약함 (judgement; not put to the user).
- **Escape the quotes instead of refusing** — each shell has different escape rules, and refusing is the pattern the path rule already uses. Evidence: 약함 (judgement; not put to the user).

## 근거
- **P2 code review, risk axis — measurements (PowerShell 7.5.5 `Parser::ParseInput` + `Invoke-Expression`; Git Bash 5.2):**
  - `node "D:/lib/x”;Write-Output INJECTED;“y/guard.js"` tokenised as `StringExpandable "D:/lib/x”` · `Semi` · `Write-Output` `INJECTED` · `Semi` · `StringExpandable …"`, with 0 parse errors, and printed `INJECTED`. Each of U+201C, U+201D and U+201E splits the string this way; U+2018 to U+201B, U+FF02 and U+2033 do not. Git Bash treats all of them as ordinary characters.
  - The scan admits such names: every character is legal on NTFS, and the library scan refuses only names that start with a dot and temp names. The folder can be any ancestor, such as the mount path, which someone other than the hook author names; an accidental `“Final” notes` ancestor splits the argument and the hook fails silently.
  - Folder `notify$` with `"$CLAUDE_PLUGIN_ROOT-v2/x.js"`: bash turned `"D:/lib/notify$-x"` into `D:/lib/notifyhBx` and `"…notify$$"` into `…notify248610` (its PID); PowerShell turned `"D:/lib/notify$?x"` into `D:/lib/notifyTruex`; a template `…}(1)` after such a folder forms `$(1)`.
  - `//srv/team$/x` stays literal in Git Bash and PowerShell.
  - cmd: a folder `D:/lib/%TEMP%/hooks/x` makes cmd run `<user temp>/hooks/x/guard.js`; codex falls back to `%COMSPEC% /C` when it knows no session shell. claude cannot reach this, because it runs hooks only through Git Bash or PowerShell.
- **P2 code review, lead:** library item paths are UTF-8 (the catalog drops non-UTF-8 names, mount resolution returns `NotUtf8`, and mount paths in `library.json` are JSON strings), so a non-UTF-8 hook folder can come only from the default root or the built-in folder; the forward-slash conversion is lossy for non-UTF-8.

## 영향 / 불변식
- **TRD `trd-hook-components.md` §2.4** is updated in the same session to this list.
- **Help (P5)** names the refused characters.
- **One rule, two callers** — the rewrite refuses an unsafe path itself and asset-library calls the same rule at check, so check and launch cannot disagree; the added refusals reuse `hook-path-unsafe`.
- **Not decided here** — a `%` refusal for codex's cmd fallback (P3).
