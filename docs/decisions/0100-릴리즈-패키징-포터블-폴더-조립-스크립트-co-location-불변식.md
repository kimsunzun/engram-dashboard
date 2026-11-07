# ADR-0100: Release packaging — a portable folder assembly script (co-location invariant)

- 상태: 확정 (2026-07-23, basis: user decision + S10 backend survey measurement) · 부분 폐기 by ADR-0134 (replacing the runtime data location with one under the run folder) · 부분 폐기 by ADR-0285 (the help body file in the Impact's description of silent degradation) · 부분 폐기 by ADR-0294 (adding the notice file to the decision 2 shipping list)
- 관련: CLAUDE.md "아키텍처 원칙" · ADR-0023(3-process topology) · ADR-0024(daemon liveness · data location) · ADR-0086(dual entrance CLI/MCP) · ADR-0092(priming file) · ADR-0099(channel capability switch · two priming files) · `scripts/build-release.ps1` · `.gitignore` · Amended by ADR-0134 (replacing the runtime data location with one under the run folder) · Amended by ADR-0285 (the help body file in the Impact's description of silent degradation) · Amended by ADR-0294 (adding the notice file to the decision 2 shipping list)

## 맥락
The release build was an area that had never been verified. Real-use confirmation ("the user commands an agent from the dashboard") is the first gate, but the current `src-tauri/tauri.conf.json` bundle settings have no `resources`, `externalBin` or `sidecar` at all, so `npm run tauri build` includes **none** of the companions the runtime needs.

The runtime layout convention (invariant) holds only if the 3 exes and the priming folder are **co-located in the same directory**:
- `engram-dashboard.exe` (the UI shell) finds the daemon exe as a sibling (`discovery::locate_daemon_exe` — current_exe sibling first).
- The daemon finds `engram-send.exe` as a sibling and injects it as `ENGRAM_SEND_EXE` (`daemon/src/lib.rs::locate_send_exe`).
- The daemon resolves `prompts/agent-priming.md` (MCP-capable) · `agent-priming-cli.md` (non-MCP) against the exe-relative install-root (`control/priming.rs` FilePrimingProvider, ADR-0092/0099). ★Correction (2026-09-19) — there is now **one** priming file. The canonical text is the correction block in 「결정」 below.★

In particular, a missing prompts file spawns an agent without priming **silently, with no error** (fail-open), and a missing engram-send silently disables the CLI entrance — both are hard for a person to notice at deployment. So a packaging procedure that guarantees the layout automatically and verifiably was needed.

## 결정
Produce the release as a **portable folder** — after the build, an assembly script (`scripts/build-release.ps1`, Windows only) does the following:
1. Build the frontend + build the 3 release binaries (`engram-dashboard.exe` · `engram-dashboard-daemon.exe` · `engram-send.exe`).
2. Copy **exactly these items only** into a clean `release/` folder (project root, `.gitignore`): the 3 exes above + `prompts/agent-priming.md` + `prompts/agent-priming-cli.md`.
3. Manifest verification — assert that the output folder has all the expected files and no other stray files (fail on mismatch).

★**Correction (2026-09-19, commit `2ef6902`) — priming is one file, not two**★: `prompts/agent-priming-cli.md` (the non-MCP variant) was deleted and the `PrimingVariant` axis was removed with it (user decision 2026-09-19 — CLI mail is a long way off, so there is no value in carrying two nearly identical instruction sheets). So the `agent-priming-cli.md` written in the list of decision 2 above and in the resolution line of 「맥락」 is **not in today's tree**, and the script's `$ExpectedPrompts` is also a single line, `@('agent-priming.md')`. ★**The decision itself of this ADR has not changed**★ — the portable folder · the assembly script · the manifest tripwire are as they were, and all that differs is the **number of files** that manifest counts. The paragraph right below handed the canonical text of "what goes in" to the script from the start, and 「영향 / 불변식」 foreshadowed a manifest update, so this change is a **factual correction**, not a reversal (the reason no new ADR is opened). The reversed decision is elsewhere — it is on the side of ADR-0099 **decision 4**, which kept priming as two static files, and the fact that a non-MCP spawn now receives not a single character of the instruction sheet (`wants_priming = uses_mail && accepts_mcp_config`) is also within its remit.

This script owns the co-location invariant and the single source of truth (SSOT) for "what goes in". The measurement bins (`roundtrip-smoke` etc.) are automatically excluded from the release graph because of `required-features=["test-harness"]`. The runtime data (`daemon.json` · `agents.json` · `sessions/`) is created at run time in `%APPDATA%\com.engram.dashboard` and is not a bundle target (ADR-0024).

## 거부한 대안
- **The official Tauri bundle (externalBin/sidecar + resources → MSI/NSIS)** — dropped for now. (a) sidecars need target-triple naming like `engram-send-x86_64-pc-windows-msvc.exe`, which is a lot of friction. (b) the output is an installer, so the form differs from the "just put the files in a folder and run" requirement. (c) it is excessive for the immediate goal of real-use confirmation. Revisit when formalizing official distribution (then supersede/amend this ADR).
- **Add only prompts to `resources` in tauri.conf.json + copy the rest (exes) by hand** — dropped. It is only half automated, so the co-location invariant gets split between the tauri settings and a manual procedure and rots. It is better to gather the single source of the layout into one assembly script.
- **Ship the `target/release/` folder as is** — dropped. Build intermediates (`deps/` · `build/` · `.pdb` · other bins) get mixed in and violate the "release files only" requirement, and `prompts/` is not in that folder in the first place.

## 근거
- User decision (2026-07-23): packaging method = portable folder only, output location = `release/`. The two alternatives (an official installer · both) are explicitly lower priority.
- The co-location invariant and the fail-open behavior were confirmed in real code in the S10 backend abstraction survey (`locate_daemon_exe` · `locate_send_exe` · `FilePrimingProvider`).
- The exclusion of the measurement bins is already guaranteed by the `required-features` gate (no extra action needed).

## 영향 / 불변식
- **Co-location invariant (do not change):** the `release/` output folder must have the 3 exes + `prompts/` (★correction 2026-09-19: 2 files → **1 file** `agent-priming.md` — the correction block in 「결정」 above★) together in one directory. If even one is missing, a silent feature degradation (a spawn without priming / the CLI entrance dead / the daemon not found) results. The script's manifest verification is the gate for this invariant. ★**The canonical source for the file count is not this line but the script's `$ExpectedPrompts`**★ — embedding a number here would drift again at the next update (it actually drifted once).
- When a new runtime companion (an extra exe or resource) appears, the script's manifest must be updated together — if not, verification fails and it is caught loudly (an intended tripwire).
- `release/` is a `.gitignore` target (regenerated on every rebuild, to keep the repo clean).
- If an official installer becomes necessary, supersede/amend this ADR and introduce the tauri bundle settings.
