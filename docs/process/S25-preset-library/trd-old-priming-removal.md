# TRD — S25: remove the old priming path (`S25: refactor(agent,daemon): remove the old priming path`)

| Item | Value |
|---|---|
| **Status** | **Draft 2** (2026-11-16) — review round 1 (full) PASS on draft 1; draft 2 applies its 권장 / batched and two user decisions (D4, D5). Closes parent [`trd.md`](trd.md) §2-4 item 2. |
| **Canonical ADRs** | new ADR (§5 D1) amending: ADR-0092 d1 · ADR-0327 d5 + 「영향」 · ADR-0368 d4 · ADR-0215 d2 · d4 · d5 (+ its rejected-alternative revival trigger) · ADR-0353 (decision on the Noop warn-and-drop + 「영향」 revival trigger) · ADR-0328 d3 + the last line of 「바뀌는 조항」 + the last 「영향」 bullet · ADR-0316 「영향」 (Noop argv byte for byte) · ADR-0209 「영향」 (the `uses_mail` split now held by agent `backend/mod.rs` `every_backend_with_a_settings_check_has_what_the_priming_teaches`) · ADR-0099 d5 (`--priming <직접 경로>`) · ADR-0093 (C0–C3 as `roundtrip_smoke` parameters) · ADR-0216 (codex Noop read) |
| **Origin** | User 2026-11-16 「기존건 지워」 on the step-8 QA finding (`trd-daemon-wiring.md` §3 row "Old priming path", §8 B2 · B3). |
| **PRD relation** | No PRD requirement changes; nothing user-visible on the screen. Operator-visible: `ENGRAM_PRIMING_FILE` (read by released v0.4.0 daemons, never in user docs) is no longer read and no longer warned about; the per-spawn missing-file `WARN` goes away. |
| **Reference files to fix** | `docs/reference/structure/agent-backend.md` — claude · codex argv rows (Noop priming / settings fragment), the `accepts_mcp_config` row, the `writes_mcp_config_file` row (the session-settings fragment), the `provision` anchor sentence (`wants_priming`). `agent-backend.html` is not redrawn (the project accepts html drift — `agent-backend.md`). |

## 0. Scope

Since step 8 every production claude and codex launch takes its priming from the real kits (`LibraryService`, built-in `builtin\instructions\priming.md` merged into the instructions). The old path still runs on every claude and codex MCP spawn: the control provision calls the priming provider, which stats the file (and `ENGRAM_PRIMING_FILE`) and logs a `WARN` when it is missing, and puts a path into `ControlEndpoint` that only the Noop launch branch reads.

Lands: the old path is removed end to end, together with its sibling Noop-only field, the control-channel settings fragment (§5 D2, written on every claude spawn). Production launches do not change (argv, files, notices).

Not here: `NoopLaunchKits` / `LaunchPorts::noop()` themselves (they stay — manager default, `LibraryService` fallback for shell · gemini); anything in the real kit path.

## 1. What goes

| Item | Where |
|---|---|
| `PrimingProvider` trait, `FilePrimingProvider`, `NoopPrimingProvider`, `FixedPrimingProvider`, `ENV_OVERRIDE` (`ENGRAM_PRIMING_FILE`) and their unit tests. `REL_MCP_PRIMARY` leaves the provider; the kept content checks (§2) keep the path for themselves | daemon `control/priming.rs` |
| The provider field and constructor argument, the `wants_priming` gate, the endpoint's `priming_file` | daemon `control/mod.rs` `DaemonControlChannel` · `provision` |
| `warn_if_priming_file_set` · `warn_if_priming_override`, `a_set_priming_override_value_warns_once`, the warn half of `the_boot_sweeps_before_the_plug_and_warns_on_the_priming_override` (§2); the provider construction in production wiring | daemon `lib.rs` |
| `ControlEndpoint.priming_file` | agent `types.rs` |
| Noop-branch consumers: claude `--append-system-prompt-file` from the endpoint; codex `priming_text` · `developer_instructions_override` (terminal `-c developer_instructions` and app-server `developerInstructions` on the Noop branch) | agent `backend/claude/mod.rs` · `backend/codex/mod.rs` |
| Settings fragment (D2): `ControlEndpoint.settings_file`, `mcp_config::write_settings`, the Noop claude `--settings` from the endpoint | agent `types.rs` · daemon `control/mcp_config.rs` · `control/mod.rs` · agent `backend/claude/mod.rs` |
| `bin/priming_smoke.rs` and its `[[bin]]` block (D3) | daemon |
| `roundtrip_smoke --priming`, `resolve_priming_path`, the `ENGRAM_PRIMING_FILE` set, its own Noop-kit manager (D4) | daemon `bin/roundtrip_smoke.rs` |

## 2. What stays

- `control/priming.rs` keeps the mail-CLI decider (`teaches_mail_cli`, `mentions_mail_cli_surface` and helpers, `pub` for `roundtrip_smoke`) and the content checks on the shipped priming (`production_priming_file_*`, `production_priming_files_*`) with the built-in priming path as a local constant. They guard the built-in priming, which is the kit source now. The module stays where it is, so the doc citations of `control/priming.rs::…` stay valid.
- The boot sweep-order test in daemon `lib.rs` keeps its sweep-before-plug assertions — it is the only check on that order (`the_production_wiring_plugs_the_library_with_the_builtin_priming` stays green if the sweep moves after the plug).
- The `uses_mail` / `accepts_mcp_config` axis keeps driving grants and `mail_allowed`; only the priming use of it goes. Tests that cited the priming file as evidence for the mail gate keep their grant assertions.
- `control/help.rs` (`ENGRAM_HELP_FILE`, `HelpSource`) — only analogy comments that name the priming provider are reworded.
- The real kit path, `cmdline::priming_value` and its tests, `scripts/build-release.ps1` `$ExpectedBuiltins`, the `ci.yml` `paths:` comment (the kept checks in `control/priming.rs` still read the built-in priming) and `OS_CFG_FILES`.

## 3. Boundaries

- **No wire or binding change.** `ControlEndpoint` is in-process between daemon and agent (`Debug, Clone` only; no serde, no ts-rs).
- **Noop launches carry no priming and no settings fragment.** A Noop claude or codex spawn gets neither instructions nor `--settings` from the control channel. Affected: tests, the harness bins (`saturation_pilot` — Noop kits plus a real control channel, claude — loses `--settings`; it measures stdin delivery only), and nothing in production (the test server has no endpoint).
- **Argv golden is regenerated once.** `tests/launch_argv_golden.rs` + `fixtures/launch_argv_golden.json`: the claude endpoint cases lose `--append-system-prompt-file` and `--settings`, the codex terminal endpoint cases lose `-c developer_instructions=…`; `fixtures/launch_golden_priming.md` is deleted; the case `endpoint-no-priming-no-settings` loses its now-meaningless name. The manager-level golden (`manager/prelaunch.rs`, `…/no-endpoint` cases) does not change. Regenerated on Windows with the `#[ignore]` `regenerate_launch_argv_golden`.
- **`ENGRAM_PRIMING_FILE` is no longer read anywhere**, and no boot warn remains (D5).

## 4. Implementation units

One unit, one commit: removing the endpoint fields breaks both crates at once, so the agent and daemon edits land together. Order inside the unit so each step compiles: (1) roundtrip_smoke onto kits (D4) and priming_smoke removal, (2) fields, provider and consumers, (3) tests and golden, (4) docs: the reference file, code comments that describe the removed path (agent `launch.rs` `include_priming` / `NoopLaunchKits` docs, agent `backend/mod.rs` tripwire doc, daemon `lib.rs` `HelpSource` comment, `control/help.rs` · `mcp_server.rs` analogies, `launch_argv_golden.rs` comment naming `priming_smoke`, daemon `Cargo.toml` bin blocks, `scripts/build-release.ps1` bin list and priming comment), parent `trd.md` §1-5 (나) 4 line on `priming_file`, and §2-4 item 2 closed with a pointer here.

## 5. Decisions

**(a) User.**
- Remove the old priming path — 「기존건 지워」 (2026-11-16); scope including D2 — 「ㅇㅇ 이대로 가면 되긴하는데」.
- **D4** priming variants in `roundtrip_smoke` = a separate copied folder passed by `--builtin-dir` — 「ㅇㅇ 별도로 복사해서해.」 (options: `--builtin-dir` ★ / edit the tracked file / keep `--priming` with an internal temp copy).
- **D5** No boot warn for a leftover `ENGRAM_PRIMING_FILE` — 「v0.4.0 이전은 필요없는데? 그냥 지우면 될듯? 앞으로 나올일 없으면」 (reverses step-8 B3 / K6's warn).

**(b) Main decides — recommendation + reason.**
- **D1** One new ADR records the withdrawal and amends the listed ADRs; written after this review, before coding.
- **D2** Remove the settings fragment in the same change. — Step-8 B2 tied it to this withdrawal ("Revisit with §2-4 2") and ADR-0328 d3 keeps it only on the Noop path "until it is withdrawn"; it is written on every claude spawn and read by no production launch, the same shape as the priming path, and doing both now regenerates the golden once instead of twice.
- **D3** Delete `priming_smoke`. — It measured whether an agent accepts a natural teammate message under the priming (ADR-0092 d4 acceptance baseline); `roundtrip_smoke`'s A→B half, built on top of it, carries that measurement on.
- **D4 shape** `roundtrip_smoke` builds `LibraryService` (template: `tests/launch_kits_live.rs`) with `LibraryValues.builtin_dir` = `--builtin-dir <dir>`, default the repo `builtin\`. Built-ins are read once when the service is built, so a variant is fixed for the run. The harness keeps reading the effective priming file itself (its CLI-surface check needs the text) and stays SETUP-FAIL when the file is missing or unreadable — the kit itself would launch unprimed with a `builtin-unavailable` notice (ADR-0361 d2), which must not pass as a primed run.

## 6. Test plan

| What | How |
|---|---|
| Production launches unchanged | claude and codex lanes per `.claude/skill-bindings/qa.md` (both lane paths are touched: `backend/claude/**`, `backend/codex/**`, `control/**`), incl. `launch_kits_live` |
| No per-spawn file read or `WARN` without `builtin\` | QA: launch with no `builtin\` — only the `builtin-unavailable` notice, no priming-file `WARN` in the daemon log |
| Mail gate still holds | `tests/mail_gate.rs` grant assertions (unchanged) |
| Boot sweep order still pinned | the kept sweep-order assertions in daemon `lib.rs` |
| Noop argv | regenerated golden; reviewed as a diff (only the listed flags disappear) |
| roundtrip_smoke still runs | build + its unit tests in the workspace run; one live run with the repo `builtin\`; `--builtin-dir` with a missing priming file → SETUP-FAIL |
| Full regression | `cargo test --workspace` per the qa binding |

## 7. Risks

| # | Risk | Mitigation |
|---|---|---|
| K1 | A test silently relied on Noop priming text to drive a model turn | grep for `priming_file` / `bake_priming` users is the coder's start check; live lanes run |
| K2 | Removing `wants_priming` drops a mail-gate assertion along with it | §2: grant assertions stay; review checks each rewritten test |
| K3 | roundtrip_smoke loses a case | none expected — today `--priming C0` already maps to the shipped priming; a no-priming run = a `--builtin-dir` copy with an empty priming file, as before with an empty file |

## 8. Review record
</content>
</invoke>
