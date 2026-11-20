# ADR-0374: A launch skips missing items and an unusable library root with a notice instead of blocking

- 상태: 확정 (2026-11-17, 근거: user decision 2026-11-17 during the S25 step-9a review — 「그냥 빠진것을 건너띄고 LLM에게 전달만하고 실행하는게 좋을듯?」, then settings included: 「그냥 B로하고 해당 내용만 호출자 클로드 코덱스가 피드백 하면 되지 않음? … 한개만 막는게 옵션이 있지 않는 이상 통일성이 좀 그런데?」) · 부분 폐기 by ADR-0387 (decision 2 unusable root)
- 관련: Amends ADR-0361 (decision 1 block 4 stands alone) · Amends ADR-0341 (the missing folder part of decision 6) · PRD `docs/process/S25-preset-library/prd.md` E4 (block reasons ① and ④) · ADR-0363 (the precedent: skip and name in a notice) · ADR-0370 (the priming reaches an agent only through a real plan) · parent TRD `docs/process/S25-preset-library/trd.md` §1-3 · step-9a TRD `trd-library-edit.md` §6 and K1 · Amended by ADR-0387 (decision 2 unusable root)

## 맥락
Since S25 step 7 a launch is refused when anything an agent's presets reach is missing: an attached instruction, skill, settings or preset file that is not there, or a folder entry whose folder is not there (block ①, PRD E4, ADR-0341 decision 6). When a library preset is attached and the library root is unset or unusable, the whole answer is one block ④ row (ADR-0361 decision 1). So one deleted or moved file, or a removed library folder, stops every agent that reaches it until a person fixes the reference. The user asked whether a launch with a broken link still runs and found that it does not; the user judged that blocking every such agent is the wrong trade (「유저가 그냥 기본 프라이밍 폴더 삭제했는데 모든 에이전트가 실행이 안되면 유저 입장에서 짜증나잖아」). Missing built-ins already never block (ADR-0361 decision 2).

## 결정
1. **A missing item does not block a launch.** A missing instruction, skill, settings or preset file, and a missing folder entry, are left out of the launch materials and the launch goes on. Settings are included: there is no per-kind exception.
2. **An unusable library root does not block a launch.** When a library preset is attached and the root is unset or unusable, every library item that agent reaches is treated as missing (decision 1); built-ins still resolve.
3. **Feedback is kept.** Each skipped item is named in a notice in the launch reply, in the daemon log, and to the launched agent (claude or codex), so the caller can act on it.
4. **Scope.** Only "missing" changes. An item that exists but cannot be read or used (broken, block ②), a conflict (block ③) and the command-line checks are not covered by this decision; their treatment, the notice code and wording, and how the agent is told are decided in the TRD of the task that implements this.
5. **Timing.** A separate task after S25 step 9a. Until it lands, the current blocks stay.

## 거부한 대안
- **Keep blocking on missing items (PRD E4 ① and ④ as they are)** — one deleted file or a removed library folder stops every agent that reaches it. Rejected by the user (사용자 직접 기각 — reason given in 맥락).
- **Skip missing instructions and skills, but keep blocking on missing settings** (main's recommendation) — settings can carry permission or sandbox limits, and the CLI enforces them, not the model; skipping a settings file can run the agent with looser permissions than intended. Rejected by the user for consistency: one kind blocking while the others skip, with no option to choose, is inconsistent (사용자 직접 기각). The risk is accepted and recorded below.
- **Hand the missing path to the CLI and let claude / codex decide** — Engram merges settings itself (claude gets one merged settings file, codex gets `-c` lines; agent `backend/claude`, `backend/codex`), so the CLI never sees a missing piece and cannot judge it; telling the model does not restore a limit the CLI no longer enforces.

## 근거
- The user's decision (상태 line) after main explained the current blocks and the settings risk.
- Precedent: ADR-0363 already skips an unreadable folder below a folder entry and names it in a notice, after a convention survey (codex skill discovery, ripgrep, find, git and walkdir all skip and warn). A convention survey for "a referenced file is missing" was not run for this decision.
- `CLAUDE.md` 「최소한만 막기」: block only what breaks execution or opens a security hole; a missing instruction or skill does not break execution.

## 영향 / 불변식
- **Accepted risk**: a missing settings file that carried a permission or sandbox limit lets the agent run without that limit. The notice is the only signal; a later option to block per kind needs a new decision.
- Code touched by the implementing task: asset-library `resolve.rs` (problem rows → notices for missing items and for ④), `launch.rs` (`launch_check`), agent `launch.rs` `BlockReason` (`Missing`, `LibraryUnavailable` stop being launch blocks), the launch reply notices (ADR-0357 wording) and the path that tells the agent.
- Screens still show a missing line as 「찾을 수 없음」 with relink (PRD R6 · R19); only the launch stops refusing.
- The step-9a TRD notes this in §6 and K1; 9a itself does not change the launch path.
