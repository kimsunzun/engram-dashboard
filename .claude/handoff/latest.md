# Handoff: S25 step 9b — U1, U2a committed; U2b reviewed (PASS), QA running; U3 next (Explorer probe done)

## Know first
- Autonomous mode (user: 「쭉 진행해. 자율로. 나 퇴근」). Session stopped at the usage limit.
- Main's notes for this step: `.review-s25-d9/s9b-code/agents.md` (agent ids, snapshots, round files). Briefs/rounds in the same folder.
- Codex relay moves to background after 180 s; recover from the run's output file (works). Replace a coder past ~500k.

## One-line state · next first action
U1 `f5a8c4fc`, U2a `c4e0f5b8` committed (deep review + QA PASS each). ADR-0412 (U2a), ADR-0414 (U2b) written. U2b (delete/view/restore) is **uncommitted, review PASS (4 rounds), QA was running** (agent a`aee48ded239585700`, snapshot tree `48f287c504ab05a7b9b452c56fd522e813373f16`).
**Next:** read the U2b QA result (SendMessage the QA agent or rerun QA from `.review-s25-d9/s9b-code/u2b-qa-brief.md`); if PASS, stage exactly the files in that brief and commit U2b (commit message pattern = U2a's, `.review-s25-d9/s9b-code/u2a-commit-msg.txt`). Then U3 — first settle keep-both naming (probe: `.review-s25-d9/s9b-code/u3-explorer-probe-result.md`).

## Repo state
- Branch v0.5.0/feat/preset-library, HEAD `58a9faec` (+ this handoff commit), ahead of origin (not pushed this session). Uncommitted: U2b files (ops.rs new, catalog/folder/lib/rename/store/trash.rs, agent profile_edit.rs doc, asset-library.md). Not ours: src-tauri/bindings noise, .claude/explain/.

## Verification state
- U1/U2a: full regression + lanes (U2a) + mutation PASS — see commit messages.
- U2b: review PASS; QA **not confirmed** (gate not passed) — do not commit before QA PASS. Two asset-library timing tests flaked once in a coder run (launch::tests::a_mount_only_another_backends_entry_reaches_is_not_stat_ed_by_a_launch, tests::a_first_scan_whose_wait_for_the_load_judgement_runs_out_records_nothing_and_goes_round).

## Decisions
User: (묻지 않음) 진행 방식 · 「쭉 진행해. 자율로. 나 퇴근」.
Main: U2 split U2a/U2b; T-123 (rewrite_in recheck), T-124 (write_atomic temp) recorded; ADR-0412 (exclude std-fs job, record name, one trash take); ADR-0414 (detach writes only confirmed presets/agents via set_attachments — amends ADR-0339 M101; restore reads in hold — amends ADR-0409 M15; digest covers items; spelling/kind judged in hold).
Open (ask user): U1 unfixed 권장 (folder write-open denial skips link re-check); keep-both naming "(2)" vs " - Copy" (probe D1).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -5 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings/`
