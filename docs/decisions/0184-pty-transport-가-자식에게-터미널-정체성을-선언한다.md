# ADR-0184: The PTY transport declares a terminal identity to the child

- 상태: 확정 (2026-09-08, basis: codex TUI render A/B measurement 2026-09-08 + two-person adversarial review)
- 관련: ADR-0049 (env default injection discipline — explicit-skip · `eq_ignore_ascii_case`) · ADR-0004 (backend knowledge isolation — `CommandSpec` output) · ADR-0003 (core isolation) · `crates/engram-dashboard-agent/src/transport/pty.rs` (`// ADR-0184` anchor)

## 맥락

A PTY child inherits the parent environment wholesale, and we only layered the profile env on top of it. But **the daemon is started through WMI (`Win32_Process.Create`), so the parent is `WmiPrvSE.exe` and the environment it inherits is not a terminal environment but a service environment** — there is no `TERM` and no `COLORTERM`. portable-pty does not synthesize those two either (the only thing it synthesizes is unix `SHELL`). Across the whole repository, the number of lines that set those two names was 0.

As a result the codex TUI degraded in our xterm.js slot: the rounded header box lost its bottom border and left a vertical line in the wrong place, the `directory:` line was missing entirely, and the gray background block of the user prompt line disappeared. The same codex launched in Windows Terminal was fine — because that terminal sets those values for the child.

★**This was not a codex-only problem**★ — claude terminal mode goes through the same `TransportShape::Pty` path, so it was running without `TERM` in exactly the same way.

## 결정

**The PTY transport declares `TERM=xterm-256color` and `COLORTERM=truecolor` to the child** (`build_pty_command`).

- **The declarer = the layer that hands out the PTY.** These two values describe *which terminal we are*, not which backend. So they do not distinguish between backends.
- **The injection discipline follows ADR-0049 as is** — if the profile (`spec.env`) already has the same key, **do not inject** (explicit-skip; do not rely on last-wins merge order), and keys are compared with `eq_ignore_ascii_case`.
- ★**Inherited values are overridden**★ — only the profile is yielded to. The child draws into our xterm.js widget, not into whatever started the daemon, so an inherited `TERM` does not describe that widget.
- **Ground for the values:** our renderer is xterm.js 6.0, and it is xterm-compatible and actually draws 256 colors. 24-bit color has no entry in the terminfo system at all, so `COLORTERM` is the conventional channel. Both are true declarations — ★declaring what we cannot draw makes the child send what we cannot draw★.

## 거부한 대안

- **Inject only for the codex backend** — it is terminal knowledge, not backend knowledge, so the place is wrong (a leak in the opposite direction of ADR-0004 「백엔드 지식 격리」), and in fact it was not a codex-only problem. claude terminal mode was suffering the same lack through the same path (measured 2026-09-08). It would end up copying the same two lines into every backend.
- **Inject in `backend::build_command_spec` (`CommandSpec` must be a complete description of the child environment — a quote from ADR-0004, an alternative actually raised in this review)** — the two values **differ by transport shape** (true for PTY, false for stdio). For the backend to know that, the backend would have to know the transport, and that is a worse leak. ★**Known cost**★: with this decision `CommandSpec` is no longer a complete description of the child environment, and `env_value(&CommandSpec, key)` in `backend/claude/mod.rs` is inaccurate for these two keys only. We chose it knowing that cost.
- **Inject for stdio transport as well** — a stdio/ndjson child has no PTY. Declaring it a terminal when it is not one is a **false declaration**, which is exactly what 「값의 근거」 above forbids. That the child env differs by transport method is not a defect but the intent.
- **Fill only when absent (fill-if-absent, respecting even the inherited environment)** — if the daemon is started from a real terminal (`scripts/run-*.bat`), that terminal's `TERM` is inherited, and that value does not describe the xterm.js the child will be drawn in. Respecting it makes **the render differ by how it was started.**

## 근거

- **A/B measurement (2026-09-08, two screenshots).** The single variable = these two environment variables. When absent: header box border missing + `directory:` line gone + no prompt background. When set: identical to Windows Terminal. After the fix we reproduced it **with the profile env left empty** and confirmed that the values coming only from the code path give the same result.
- **claude terminal mode checked by eye** — normal after the fix (logo · colors · input box · status line). ★The screen from *before* the fix was not kept, so whether it improved is unmeasured★.
- **Two-person adversarial review (code/full).** cross-family (blind) = PASS. doc-aware = FIX — its findings caught and fixed ① the unit test spinning idle (the `TERM` and `COLORTERM` of the dev shell equaled the asserted values, so it passed even with the injection deleted) and ② the inconsistency with the ADR-0049 discipline. The test was shown able to fail by a mutation experiment.

## 영향 / 불변식

- **Every backend that starts through a PTY is covered.** `TransportShape::Pty` is the default, so PTY backends that are added in the future, besides codex and claude terminal, are included automatically.
- ★**The declaration must be true**★ — if the renderer is changed to something other than xterm.js or its color capability shrinks, fix these two values along with it. If not fixed, the child sends what we cannot draw.
- **The case-insensitive guard cannot fail on Windows** — because portable-pty folds env keys to lowercase on Windows, so the two spellings become one entry. CI is Windows-only, so **the CI signal for that guard is 0**, and it exists only as a unix-side safety net (stated in the test comment).
- **Unresolved — revisit when a shell backend opens.** If a child that received a capable `TERM` launches a pager (`less`) interactively, an LLM-driven shell agent can stall. For now the wire's `AgentBackendKind` is only `Claude` and `Codex`, so it is **unreachable**. The decision to put a shell backend on the wire revives this item.
- **Unmeasured — `resume_failure_kind` fragility.** `backend/claude/mod.rs` classifies a resume failure by finding a string in the tail of the output with ANSI not stripped (ADR-0172). Raising the terminal capability may change the placement of control characters in that byte sequence. It is a pre-existing weakness and we did not re-measure it this time.
