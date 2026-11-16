# TRD — S25: remove the old priming path (`S25: refactor(agent,daemon): remove the old priming path`)

| Item | Value |
|---|---|
| **Status** | **Draft 1** (2026-11-16) — awaiting PRD+TRD review. Closes parent [`trd.md`](trd.md) §2-4 item 2. |
| **Canonical ADRs** | new ADR (§5 D1) amending ADR-0092 d1 · ADR-0327 d5 + 「영향」 · ADR-0368 d4 · ADR-0215 d2 · ADR-0328 d3 · ADR-0099 · ADR-0216 (codex Noop read) |
| **Origin** | User 2026-11-16 「기존건 지워」 on the step-8 QA finding (`trd-daemon-wiring.md` §3 row "Old priming path", §8 B2 · B3). |
| **PRD relation** | No PRD requirement changes; nothing user-visible on the screen. Operator-visible: `ENGRAM_PRIMING_FILE` and the per-spawn missing-file `WARN` go away. |
| **Reference files to fix** | `docs/reference/structure/agent-backend.md` (claude · codex argv rows, `accepts_mcp_config` row, the `provision` anchor sentence). |

## 0. Scope

Since step 8 every production claude and codex launch takes its priming from the real kits (`LibraryService`, built-in `builtin\instructions\priming.md` merged into the instructions). The old path still runs on every claude and codex MCP spawn: the control provision calls the priming provider, which stats the file (and `ENGRAM_PRIMING_FILE`) and logs a `WARN` when it is missing, and puts a path into `ControlEndpoint` that only the Noop launch branch reads.

Lands: the old path is removed end to end, together with its sibling Noop-only field, the control-channel settings fragment (§5 D2). Production launches do not change (argv, files, notices).

Not here: `NoopLaunchKits` / `LaunchPorts::noop()` themselves (they stay — manager default, `LibraryService` fallback for shell · gemini); anything in the real kit path.

## 1. What goes

| Item | Where |
|---|---|
| `PrimingProvider` trait, `FilePrimingProvider`, `NoopPrimingProvider`, `FixedPrimingProvider`, `REL_MCP_PRIMARY`, `ENV_OVERRIDE` (`ENGRAM_PRIMING_FILE`) and their unit tests | daemon `control/priming.rs` |
| The provider field and constructor argument, the `wants_priming` gate, the endpoint's `priming_file` | daemon `control/mod.rs` `DaemonControlChannel` · `provision` |
| `warn_if_priming_file_set` · `warn_if_priming_override` and their tests; the provider construction in production wiring | daemon `lib.rs` |
| `ControlEndpoint.priming_file` | agent `types.rs` |
| Noop-branch consumers: claude `--append-system-prompt-file` from the endpoint; codex `priming_text` · `developer_instructions_override` (terminal `-c developer_instructions` and app-server `developerInstructions` on the Noop branch) | agent `backend/claude/mod.rs` · `backend/codex/mod.rs` |
| Settings fragment (D2): `ControlEndpoint.settings_file`, `mcp_config::write_settings`, the Noop claude `--settings` from the endpoint | agent `types.rs` · daemon `control/mcp_config.rs` · `control/mod.rs` · agent `backend/claude/mod.rs` |
| `bin/priming_smoke.rs` and its `[[bin]]` block (D3) | daemon |
| `roundtrip_smoke --priming` and its own Noop-kit manager (D4) | daemon `bin/roundtrip_smoke.rs` |

## 2. What stays

- `control/priming.rs` keeps the mail-CLI decider (`teaches_mail_cli`, `mentions_mail_cli_surface` and helpers) and the content pins on the shipped priming (`production_priming_file_*`, `production_priming_files_*`). They guard the built-in priming, which is the kit source now. The module stays where it is, so the many doc citations of `control/priming.rs::…` stay valid.
- The `uses_mail` / `accepts_mcp_config` axis keeps driving grants and `mail_allowed`; only the priming use of it goes. Tests that cited the priming file as evidence for the mail gate keep their grant assertions.
- `control/help.rs` (`ENGRAM_HELP_FILE`, `HelpSource`) — only analogy comments that name the priming provider are reworded.
- The real kit path, `cmdline::priming_value` and its tests, `scripts/build-release.ps1` `$ExpectedBuiltins`.

## 3. Boundaries

- **No wire or binding change.** `ControlEndpoint` is in-process between daemon and agent (`Debug, Clone` only; no serde, no ts-rs).
- **Noop launches carry no priming and no settings fragment.** A Noop claude or codex spawn (tests, harness setups, the test server — which has no endpoint anyway) gets neither instructions nor `--settings` from the control channel.
- **Argv golden is regenerated once.** `tests/launch_argv_golden.rs` + `fixtures/launch_argv_golden.json`: the claude endpoint cases lose `--append-system-prompt-file` and `--settings`, the codex terminal endpoint cases lose `-c developer_instructions=…`; `fixtures/launch_golden_priming.md` is deleted; the case `endpoint-no-priming-no-settings` loses its now-meaningless name. The manager-level golden (`manager/prelaunch.rs`, `…/no-endpoint` cases) does not change. Regenerated on Windows with the `#[ignore]` `regenerate_launch_argv_golden`.
- **`ENGRAM_PRIMING_FILE` is no longer read anywhere**, and no boot warn remains (D5).
- **CI:** `ci.yml` `OS_CFG_FILES` keeps `roundtrip_smoke.rs` as long as the file keeps its `#[cfg]`; the `paths:` comment naming `control/priming.rs` as the reader of `builtin/instructions/priming.md` is reworded.

## 4. Implementation units

One unit, one commit: removing the endpoint fields breaks both crates at once, so the agent and daemon edits land together. Order inside the unit so each step compiles: (1) roundtrip_smoke onto kits (D4) and priming_smoke removal, (2) fields, provider and consumers, (3) tests and golden, (4) docs, ADR anchors, reference file, CI comment.

## 5. Decisions

**(a) User.**
- Remove the old priming path — 「기존건 지워」 (2026-11-16).

**(b) Main decides — recommendation + reason.**
- **D1** One new ADR records the withdrawal and amends the listed ADRs; written after this review, before coding.
- **D2** Remove the settings fragment in the same change. — Step-8 B2 tied it to this withdrawal ("Revisit with §2-4 2") and ADR-0328 d3 keeps it only on the Noop path "until it is withdrawn"; it is written on every claude spawn and read by no production launch, the same shape as the priming path, and doing both now regenerates the golden once instead of twice.
- **D3** Delete `priming_smoke`. — Its purpose (ADR-0092 baseline: is the priming file read and applied) is covered live by `launch_kits_live` L1–L4 on the real path.
- **D4** `roundtrip_smoke` moves onto `LibraryService` with the repo `builtin\` (template: `tests/launch_kits_live.rs`), and `--priming` is dropped. — A priming variant is measured by editing `builtin\instructions\priming.md` in the working tree, which the kit reads at launch; keeping `--priming` would need a temp copy of `builtin\` with one file swapped, extra code for the same ability. The mail-CLI decider stays `pub` for its use.
- **D5** No boot warn for a leftover `ENGRAM_PRIMING_FILE`. — The variable only ever reached dev and harness setups, v0.5.0 is unreleased, and a warn would keep the name alive in code.

## 6. Test plan

| What | How |
|---|---|
| Production launches unchanged | claude and codex lanes per `.claude/skill-bindings/qa.md` (both lane paths are touched: `backend/claude/**`, `backend/codex/**`, `control/**`), incl. `launch_kits_live` |
| No per-spawn file read or `WARN` without `builtin\` | QA: launch with no `builtin\` — only the `builtin-unavailable` notice, no priming-file `WARN` in the daemon log |
| Mail gate still holds | `tests/mail_gate.rs` grant assertions (unchanged) |
| Noop argv | regenerated golden; reviewed as a diff (only the listed flags disappear) |
| roundtrip_smoke still runs | build + its unit tests in the workspace run; one live run with the repo `builtin\` |
| Full regression | `cargo test --workspace` per the qa binding |

## 7. Risks

| # | Risk | Mitigation |
|---|---|---|
| K1 | A test silently relied on Noop priming text to drive a model turn | grep for `priming_file` / `bake_priming` users is the coder's start check; live lanes run |
| K2 | Removing `wants_priming` drops a mail-gate assertion along with it | §2: grant assertions stay; review checks each rewritten test |
| K3 | roundtrip_smoke loses its C0 (no priming) case | accepted — C0 is history (ADR-0093); a no-priming run = an empty `builtin\instructions\priming.md` in the working tree |

## 8. Review record
</content>
</invoke>
