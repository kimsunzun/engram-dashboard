# ADR-0353: Codex real launch materials fail closed on an unreadable instructions file, a priming the terminal line cannot carry, or a working folder cmd would change

- 상태: 확정 (2026-11-10, 근거: user decisions U1 2026-11-09 and 2026-11-10 · main decision #5 — TRD `docs/process/S25-preset-library/trd-codex-translation.md` §9 · landed `a3891273`, `abee6ebe`)
- 관련: Amends ADR-0215 (결정 4와 5 실재료 한정) · ADR-0351 (the claude precedent: a failed placement write fails the spawn — extended here to codex) · ADR-0337 (codex materials) · ADR-0334 (flag translation in backend `materialize`)

## 맥락
ADR-0215 made both codex instruction failures fail-open: decision 4 — in terminal mode, an instruction text that does not fit on the command line (8,191 units) or holds a refused character is dropped with a warning and the spawn proceeds; decision 5 — an unreadable priming file is skipped and the spawn proceeds. That fit the Noop launch, where the only instructions are the built-in priming and the spawn matters more. With real launch materials (S25), the merged instructions file carries what the user put in the preset; launching without it silently runs a different agent than the one the user asked for. The codex terminal command line also goes through `cmd.exe /c` and the npm `codex.cmd` shim, so a working folder holding cmd metacharacters can reach codex as a different folder or run a command.

## 결정
With real launch materials (`kit.plan.real`), the codex backend `materialize` (`backend/codex/launch.rs`) returns `Err` — which the manager turns into a failed spawn — before any write when:
1. the merged instructions file cannot be read or is not UTF-8 (both modes);
2. terminal mode: the 「기본」 priming cannot go on the command line — `cmdline::priming_value` refuses it (`' " % \`, a control character other than CR/LF, or, when the text holds no space, `& ^ | < >`) or `cmdline::fits` finds the line over the budget;
3. terminal mode: the working folder would be changed by cmd — it is not UTF-8, holds two or more `%`, a `"`, a control character, or `& ^ | < >` without a space (with a space, portable-pty quotes the value and cmd takes them literally; a single `%` was measured to reach codex intact).
The launch check (step 7) blocks the same cases earlier with `Broken`; reaching one in `materialize` is the residual case. The Noop path keeps ADR-0215's warn-and-drop for the priming.

## 거부한 대안
- **Warn and launch without the instructions (ADR-0215 decisions 4 and 5 kept for real materials)** — rejected by the user (U1, 「띄우지 말아」): the agent would run without the preset's instructions and nobody would notice.
- **Refuse every cmd metacharacter in the working folder (`path_is_cli_safe`, the first P2 rule)** — rejected by the user's 2026-11-10 rule (CLAUDE.md 「최소한만 막기」): it blocked folders that launch correctly, such as `D:\R&D Projects`. Measured: a single `%` survives both cmd passes, two `%` around a defined name are expanded.
- **No working-folder guard on the real path** — rejected: `C:\Work\R&D` reaches cmd unquoted and runs `D` (security; the Noop path still has this, tracked).

## 근거
- U1: user 2026-11-09 「띄우지 말아」; working-folder rule: user 2026-11-10 「일단 너무 막지 말고 추가로 막는건 실제로 불편하면 막자고」.
- Measurements: M5, M10a (`docs/research/codex-launch-materials-measurements-2026-11-09.md`); QA 2026-11-10 single-`%` measurement through `cmd.exe /c` + the npm shim.
- ADR-0351 precedent for claude (a failed placement write fails the spawn).

## 영향 / 불변식
- Real-path refusals are `Err` → `SpawnFailed`, never a silent drop. Tests: `codex::launch` tests (`a_priming_the_terminal_line_cannot_carry_fails_the_launch`, `a_working_folder_cmd_would_change_fails_the_real_terminal_launch_only`, `an_unreadable_instructions_file_fails_before_any_write`).
- Step 7's check must judge the same rules on the canonicalized working folder, or an affected agent fails every launch as `SpawnFailed` instead of `Broken`.
- The Noop terminal path is unchanged except the P1 safety change (a space-less priming holding `& ^ | < >` is skipped with a warning).
