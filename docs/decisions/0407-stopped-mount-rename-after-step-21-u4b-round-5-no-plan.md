# ADR-0407: Stopped mount rename after step-21 U4b round 5: no plan digest in the stop answer, the repeat re-plans what is left and is confirmed again (amends ADR-0400)

- 상태: 확정 (2026-11-23, 근거: user 2026-11-23 「ㅇㅇ 어쨋든 old new 폴더 바뀌는게 최종이니깐 아직 안된게 가시적으로 보이고 중지됐으면 다시 실행 가능하다는거잖아. 그거 자체가 히스토리니깐. 진행 ㄱㄱ」 after a light convention check (Terraform 「Saved plan is stale」 after a partial apply, VS Code and IntelliJ re-trigger the rename) · step-21 U4b code review round 5 (lead and Codex, must))
- 관련: Amends ADR-0400 (decision 12 the same call finishes a stopped rename) · ADR-0405 (decision 1 yes pins what the caller saw; decisions 2 and 3 unread references and a lost mount) · ADR-0335 (relink stop rule, no progress record) · step-21 TRD `docs/process/S25-preset-library/trd-library-mounts.md` · Amends ADR-0405 (decision 3 the repeat after a lost-mount stop asks before it finishes)

## 맥락
ADR-0400 decision 12 said a stopped rename is finished by "the same call", and a stop at the agents step answers the new `root_generation` for the finishing call. Once ADR-0405 decision 1 made a confirmed rename carry `yes` + `root_generation` + `plan_digest`, review round 4 made the stop answer carry the `plan_digest` of what was left, so the repeat finished in one call. Review round 5 found that digest was computed after the rename's hold was released: a preset edit landing in between could add a file referring to the old name, and the repeat would rewrite a file the caller never saw — breaking ADR-0405 decision 1. Computing it inside the hold kept one-call finishing but needed a further rule for a mount lost during the rename.

## 결정
1. **A stop answer carries no plan digest.** A stopped rename answers what it wrote, skipped and where it stopped, with the current status (root generation); nothing in it confirms a later call.
2. **The repeat after a stop is the ordinary call.** It plans from the current files — files already rewritten no longer refer to the old name and drop out — and a plan that needs confirmation (ADR-0405 decisions 1 and 2) is confirmed with that plan's fresh tokens. A call carrying the tokens of the plan confirmed before the stop is judged like any call: it answers `mount-rename-unconfirmed` (or `root-generation-stale` when the generation moved and the plan of what is left needs confirmation) with the plan of what is left — unless that plan is the confirmed one again (every written file came to refer to the old name again; the digest pins paths, not content, as it does without a stop). There is no resume path and no stored progress (ADR-0335): the old name stays registered until the last step, so what is not done stays visible in the files and the same command can be run again in any session or after a restart.
3. ADR-0400 decision 12's "the same call finishes it" now reads: repeating the call answers the plan of what is left, and confirming that plan finishes it. A stop before any write still answers that failure's refusal.

## 거부한 대안
- **Stop answer carries the digest of what is left, computed after the hold (round-4 fix as built)** — admits a file edited between the hold and the digest into a confirmation the caller never saw. Basis: review round 5 (lead and Codex, must).
- **Compute that digest inside the hold** — keeps one-call finishing, but a mount lost during the rename then needs its own rule (the repeat could finish leaving that mount's files unread without the caller seeing a plan listing them, ADR-0405 decisions 2 and 3); one more exception for a rare path. 약함 — 사용자 직접 기각 (after the convention check: no surveyed tool carries a resume token across a failure).
- **A separate repair or resume command** — there is no progress record to repair (ADR-0335); the same rename re-plans from the files. Flyway's `repair` exists because it keeps a failed-migration history table. 약함 — 사용자에게 설명, 사용자 수용.

## 근거
- User decision 2026-11-23 quoted in 상태.
- Convention check 2026-11-23 (light): Terraform rejects a saved plan after a partial apply ("Saved plan is stale") and a plain re-run re-plans and prompts; VS Code and IntelliJ have no resume and the user re-triggers the rename with a fresh preview; git rebase `--continue` resumes without re-confirming only because the user resolves the half-done state by hand.
- Step-21 U4b review round 5 (lead and Codex, must).

## 영향 / 불변식
- `rename.rs`: `RenameAnswer` carries no digest; no digest is computed after the hold. The confirming-call checks of ADR-0405 decision 1 (at the call and inside the hold) are the only confirmation path, first call or repeat.
- Wire: the `library.renameMount` success answer (with `stopped`) has no digest field; the declaration doc and the help say the repeat answers the plan of what is left to confirm.
- A change that lets a call confirmed before a stop rewrite a file after the stop without a fresh confirmation breaks this decision and ADR-0405 decision 1.
