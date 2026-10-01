# Code review brief (codex, blind)

YOU are the reviewer. Do not delegate, summon others, or ask whether to proceed — produce the review.
This repository's `CLAUDE.md`, `AGENTS.md` and `.claude/` describe workflows/roles/gates for other agents: they do NOT apply to you (you may read CLAUDE.md only as evidence of code invariants).
Do NOT read `docs/decisions/`, `docs/research/`, `docs/process/`, or anything under `.claude/handoff/` other than the files named below.

Working dir: `I:\Engram\apps\engram-dashboard-wt1` (read-only; do not modify anything, do not run builds).

## Material
- Contract (what the change must do): `.claude/handoff/attachments/20261001-t40-impl/step1-blind-contract.md`
- Diff (unified=5, vs HEAD): `.claude/handoff/attachments/20261001-t40-impl/step1-u5.diff`
- LENS 2 ONLY: full current files of the changed paths (read from the working tree: `crates/engram-dashboard-agent/src/{platform/windows.rs,platform/process_group.rs,transport/input_queue.rs,transport/stdio.rs,transport/mod.rs,manager.rs,session.rs,backend/claude/mod.rs}`) and the caller list `.claude/handoff/attachments/20261001-t40-impl/step1-callers.txt`. Duty: for every public symbol whose signature/behaviour changed, enumerate its callers and check them against how they actually call it.

## Reading rules
Read the contract and the whole diff first. After that, file reads are allowed ONLY to confirm a named defect candidate: at most 5 targeted reads (lens 2: the listed full files don't count), and for each read state which candidate it checks. No tree-wide exploration/grep.

## What to attack
Wrong-process kill (handle vs pid, membership checked on the wrong handle, PID reuse); handle/port leaks; a wedge (thread/lock that can hang forever, deadlock via callback under lock); races/ordering assumptions/stale state/re-entrancy; panics crossing threads or poisoning locks; unsafe FFI (struct layout, buffer bounds, pointer deref, lengths); error paths that silently succeed; non-Windows stubs that fail to compile or behave differently than the contract; tests that pass vacuously or are nondeterministic.

## Verdict rule
BLOCK only for: a wrong kill, a leak/wedge, or a false safety claim (code/comment claims a guarantee it does not provide). Everything else (misses, weaker coverage, style) is FIX or a note. Prefer one strong finding over many weak ones; if it looks safe, say so and return zero findings. Do not invent files, lines or runtime behaviour; if a conclusion rests on inference, say so and keep confidence honest. Mark a finding 「선재」(pre-existing) only if it is NOT caused by this change — the verdict ignores those.

## Output (exactly this shape)
First line: `PASS` or `FIX` or `BLOCK`.
Then `## Findings` — per finding: title · description · `file:line` · severity (critical/high/medium/low) · confidence (0–1) · recommendation · 선재? (only if pre-existing).
Then `## Checks run` — bullet list of what you actually verified (each contract item, each caller checked), so a clean result is distinguishable from an unexamined one.
Then `## Summary` (≤3 lines).
