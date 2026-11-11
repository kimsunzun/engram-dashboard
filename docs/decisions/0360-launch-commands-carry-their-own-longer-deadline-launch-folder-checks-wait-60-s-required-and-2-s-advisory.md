# ADR-0360: Launch commands carry their own longer deadline — launch folder checks wait 60 s required and 2 s advisory

- 상태: 제안 (2026-11-11, 근거: user decision 2026-11-11 (S25 step 7, after the folder-timeout convention check) — values proposed by main, awaiting the user's confirmation)
- 관련: ADR-0341 (a folder that cannot be read blocks the launch) · ADR-0154 (command delivery) · ADR-0348 (create-and-start body) · ADR-0324 (lock waits, M47) · child TRD `docs/process/S25-preset-library/trd-asset-library.md` D1 · B25 · R15 · `crates/engram-dashboard-daemon/src/command_delivery.rs` `DEFAULT_DEADLINE`

## 맥락
Before a launch, Engram checks the folders a preset names (ADR-0341): the library root when a library preset is attached, each folder entry (exists, listing), and for codex the files Engram reads for it (`AGENTS.md`, skills). These "required" checks block the launch when a folder does not answer. "Advisory" checks (working-folder canonicalize, codex's own skill roots for the same-name notice) are skipped when they do not answer.
Their time limits came from the command bus, not from the folders: the `engram` CLI gives up after 10 s of silence (`CLI_CONTROL_READ_TIMEOUT_SECS`), so the daemon answers every command by 7 s (10 − 1 sweep − 2 margin), so the launch folder checks got 5 s required + 1 s advisory (D1 · B25).
The user pointed out that a slow or misbehaving PC, a sleeping HDD or a sleeping NAS can exceed 5 s while the folder is fine, and that an agent launched while nobody is at the PC then simply fails: 「느리거나 제대로 동작되지 않는 pc가 있는데 사람이 없을때 에이전트가 실패하면 무한 막히는거잖아.」
Convention check (2026-11-11, worker survey — issue reports and vendor docs, no source read): no app defines a pre-launch folder timeout. Claude Code waits without a limit on a dead `additionalDirectories` drive (anthropics/claude-code #48361), JetBrains and Git for Windows have no timeout, Obsidian fails after 90–100 s, and the Windows SMB client gives up on one request after 60 s (`SessTimeout`, Microsoft "SMB 2.x and SMB 3.0 timeouts"). No library cancels a blocked file call; the common pattern (tokio `spawn_blocking` + `timeout`, OpenDAL `TimeoutLayer`) abandons the call, as Engram's bounded IO already does.

## 결정
1. **Commands that run a launch get their own deadline of 65 s** — creating and starting an agent, and starting or resuming an existing one. Every other command keeps the 7 s default.
2. **For those commands the `engram` CLI waits 68 s** (65 + the 1 s sweep + the 2 s margin — the same rule `fits_caller_silence_window` enforces today). The deadline travels in the caller-set deadline field of the command envelope, which the delivery code already reserves as additive (`command_delivery.rs` `DEFAULT_DEADLINE` doc); the 7 s default stays its fallback.
3. **Launch folder checks: 60 s for the required phase, 2 s for the advisory phase**, per launch, replacing 5 s and 1 s. 60 s = the point where the Windows SMB client gives up on a request by itself, so waiting longer gains nothing. Worst case for a launch with a dead folder: 60 + 2 = 62 s of folder waiting, inside 65 s with 3 s left for preparing and spawning.
4. Folder reads outside a launch (resolving presets for a listing, other commands under the 7 s default) keep their 5 s budget.
5. The rule stays one rule for both backends: the required phase blocks, the advisory phase skips; failures are listed together in one reply.

## 거부한 대안
- **Keep 7 s / 5 s / 1 s** — a slow PC or a sleeping disk blocks a launch whose folders are fine, and an unattended agent fails. User 「느리거나 제대로 동작되지 않는 pc가 있는데 사람이 없을때 에이전트가 실패하면 무한 막히는거잖아.」 · 「시간을 늘려」. 약함 — 사용자 직접 기각.
- **Required 30 s, launch deadline 35 s (main's first proposal)** — covers a sleeping HDD and most NAS wake-ups; main raised it to the SMB ceiling because, with nobody waiting, a longer wait costs little and a failed launch costs much. 약함 — presented to the user with this record for confirmation.
- **Do not check claude folder entries; let claude load them** — claude waits without a limit on a dead folder (#48361), and codex folders are read by Engram anyway, so a generous bound is needed regardless; it would also split ADR-0341 decision 6 by backend. Presented; the user chose longer times. 약함 — 사용자 직접 기각.
- **Answer the launch at once and report folder results later** — changes the launch reply contract and every caller. Presented; not chosen.
- **Raise the global 7 s default for every command** — other commands gain nothing, and a hung command would hold its caller for 68 s; the per-command deadline field was already reserved for this.

## 근거
- User decision 2026-11-11 (quotes above).
- The deadline arithmetic in `command_delivery.rs` (`DEFAULT_DEADLINE`, `fits_caller_silence_window`) and the measured property that the CLI's limit is a silence window, not a total budget.
- The convention survey summarized in 맥락.

## 영향 / 불변식
- Child TRD: D1's 5 s and B25's 1 s become 60 s and 2 s (P6 constants `IoBudget`); the worst case per blocked launch becomes 62 s < 65 s. B28 (a success past the deadline loses its notices) now applies past 65 s. R15 (lock waits are not budgeted) keeps its meaning against the 65 s deadline.
- Step 8 adds the deadline field to the command envelope (protocol, additive), makes the daemon honour it for launch commands, and makes the CLI send it and raise its read timeout to 68 s for those commands. The compile-time check `fits_caller_silence_window` must hold for the launch value too.
- Launches that are not bus commands (webview activations, restore at boot) use the same folder budgets; how long their own callers wait is outside this decision.
- A dead required folder now costs every launch that names it up to 60 s before the block — accepted.
