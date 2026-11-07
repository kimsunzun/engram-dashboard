# The convention of skipping CI for docs-only changes — mature OSS survey (2026-09-26)

- **Status:** final — reflects 2 adversarial reviews (Codex report review FIX · doc-aware document review FIX). Decision = keep ADR-0232 (settings unchanged); only the wording of its grounds was trimmed to match this report. **One user decision pending** — whether to add the two 「거부 후보」 below (third-party detection action · `[skip ci]`) to ADR-0232's rejected alternatives.
- **Method:** `/research` medium · design-decision mode. 3 collection tracks in parallel (① the workflow files of real repositories ② GitHub behavior and pitfalls ③ the rationale for choosing include/exclude lists · the case where docs are test inputs) → main checked the load-bearing quotes against the original files (grounding) → cross-family (Codex) adversarial review.
- **Trigger:** After ADR-0232 (a push or PR that changes only docs paths does not launch CI) was decided, the user asked 「보통 그렇게 하나」. It is also the survey planned by the to-do (`docs/todo/ci-scope.md`) that was deleted when that ADR was decided.
- **Confidence legend:** **certain** = checked against original files or official docs and two or more independent sources · **likely** = checked but one source, or a necessary reading of the docs · **uncertain** = could not check / inference.
- **Reference point in time:** each repository's default branch, viewed 2026-09-26. The SHA in the table is the first 12 digits of the HEAD the collector read.

## Conclusion (against our choice)

1. **Our approach is within convention — but it is the convention of 「필수 체크가 없는 저장소」 (repositories without required checks).** Path filters at the trigger level (`paths`/`paths-ignore`) are common (Node · Playwright · React · Tauri · Polars · WezTerm · Biome). On the other hand, **in the sample of large projects that appear to set required checks (branch protection · merge queue), 「잡 단위 변경 감지 + 늘 도는 집계 잡」 (job-level change detection + an aggregate job that always runs) was common** (CPython · Next.js · Electron · Zed · Ruff · uv · Vite · Servo — the population was not counted, so this is not a ratio claim). There is another shape — Ruby puts the path filter **only on push** and always runs PRs ("Do not use paths-ignore for required status checks"). The reason is one asymmetry GitHub nails down in its docs — the check of a workflow skipped by a path filter stays 「Pending」 and blocks merging, while a job skipped by a condition (`if:`) is reported as 「Success」. We decided not to set branch protection (`docs/tracking.md` T-18), so the trigger level is right, and if we do set it, as in ADR-0232 reopening trigger ①, either move to the job level (most of the sample) or drop the filter on the PR side (Ruby). **Confidence = certain** (the Pending/Success asymmetry — docs) · **likely** (which side is more common — sample).
2. **For the goal 「문서만이면 건너뛴다」 (skip when docs only), an exclude list is the convention.** Include lists are also common, but are mainly used differently — in **a split where each area has its own workflow and each looks only at its own area** (Tauri = per crate · Polars = per language · WezTerm = per platform). There is a counterexample too — Biome puts an include list on a broad PR workflow that holds lint · test · e2e · docs jobs (found by the adversarial review). So the claim is not 「포함 목록은 분할 워크플로 전용」 (include lists are only for split workflows) but only 「분할에서 많이 보인다」 (they are seen often in splits). Those that put 「문서 전용이면 건너뛴다」 (skip if docs only) as one gate use the exclude shape (Node · Ruby · Playwright · Ruff's `code` decision · Deno · Next.js's docs group · Pants · the examples in the Prow docs). We found no official guidance recommending either side — this conclusion is drawn from practice. **Confidence = likely.**
3. **If docs are test inputs, the convention is not to exclude that path or to exclude it narrowly — the goal is the same as our `qa.md` revival, and the shape differs.** Node does not exclude `doc/` from the Linux workflow because it builds addon tests from `doc/api/addons.md`, and Ruff wrote in a workflow comment 「ty 의 테스트가 Markdown 으로 쓰여 있으니 Markdown 전체를 빼지 말고 `docs/**` 처럼 구체적으로 빼라」. Node · Vite revive their own workflow file from the ignore list with `!` (it is a workflow file, not a doc, but the technique of 「빠진 경로 안의 한 파일을 되살린다」 (reviving one file inside an excluded path) is the same). There is also a real case in a small repository that read README · `docs/**` with `include_str!` while excluding them from the path filter, and missed a broken doctest (delta-arrow-reader #135 → added that path to the filter). **Confidence = certain.**
4. **Not skipping at all is also a convention.** tokio · Bevy · cargo · TypeScript · VS Code (PR workflow) do not filter by path. It is a choice that paid the cost to buy simplicity. It is on the opposite side from our user's demand (「기다림이 거슬린다」). **Confidence = certain.**
5. **Whether `[skip ci]` commit messages are used cannot be known from this investigation.** The collector counted 0 occurrences of that string in about 500 workflow files, but that feature is built into GitHub and leaves no trace in workflow files (found by the adversarial review). All we can say is that nobody wrote it into a workflow as their own rule. **Confidence = uncertain.**

### What to fix in our docs — an overstatement this investigation exposed

- ★**「`paths-ignore` 로는 되살리기를 표현할 수 없다(`!` 는 `paths` 에서만 먹는다)」 is stronger than what the docs say**★ — ADR-0232 decision 1 · rejected alternatives · rationale ⑤ and the `ci.yml` comment wrote it this way. What the docs actually say is two things: ① they **recommend** 「제외만 할 때 `paths-ignore`, 포함과 제외를 섞을 때 `!` 를 붙인 `paths`」 ② the filter pattern rules in general have "`!`: At the start of a pattern makes it negate previous positive patterns." They write neither that a `!` inside `paths-ignore` **works nor that it does not**.
  - Some use it — Node's `test-linux.yml` ignores `.github/**` inside `paths-ignore` and then revives it with `'!.github/workflows/test-linux.yml'`, and for 4 main commits that changed only files under `.github/` (`abc66cbda1` · `735a09f999` · `ffa62401d7` · `9c7c88c52c`), 「Test Linux」 ran as a push (`gh api …/actions/runs?head_sha=`, 2026-09-26).
  - ★**We do not use that run record as proof**★ — we did not confirm whether those pushes were one commit each (if several commits were pushed at once, other files may be mixed into the diff). The adversarial review also counted this inference as 「증명 아님」 (not proof) (below). **Confidence = uncertain.**
  - **Disposition:** our `paths` shape is the shape the docs recommend and we even measured the revival in our repository, so **we leave the setting as is.** Only the grounds sentence in ADR-0232 · the `ci.yml` comment is trimmed to the level of 「문서가 권한다」 (the docs recommend it).
- **A rename appears to evaluate both the old path and the new path** — it is not in the GitHub docs, and one third-party issue reports it with a run record (https://github.com/thekevinscott/willfire/issues/237 — a workflow filtered by `packages/component/**` ran on a change that moved files outside it. ★**It is a PR-event case, not a push**★ (run 35991758803) · the current file list of that PR differs from the issue's description, so it can no longer be checked again) · one community answer says the same (https://github.com/orgs/community/discussions/164673). ADR-0232's 「미확인 — 이름 바꾸기」 can be lowered to **likely: both sides evaluated** (if so, that risk is almost nonexistent). **Confidence = likely** (both sources unofficial).

## Details of the findings

### A. Real repositories — who uses what

| Project | Approach | Required-check handling (visible scope) | Basis | Confidence |
|---|---|---|---|---|
| python/cpython | Job level — `compute-changes` emits `run-docs`·`run-tests` etc. and each job has `if:` | `all-required-green` — "only used for the branch protection", `re-actors/alls-green` with **conditional `allowed-skips`** | `.github/workflows/build.yml` @ ee1bbf037ff7 | certain (main check) |
| vercel/next.js | Job level — `scripts/run-for-change.mjs --not --type docs`, the docs group holds `docs`·`errors`·`examples`·`.claude` etc. | `tests-pass` (`if: always()`) · if detection fails everything runs ("if we fail to detect the changes run the command") | `build_and_test.yml` @ abac2089cd97 | certain (main check) |
| electron/electron | Job level — its own paths-filter action · if docs only, only a separate TS compile pipeline | `gha-done` aggregate · wrote 「조건부 skip 은 success 로 친다」 in a comment | `apply-patches.yml`·`build.yml` @ 0a6c0bb09bfe | certain (main check) |
| zed-industries/zed | Job level — `git diff --name-only` + grep · everything on a main push | `tests_pass` · `merge_group` | `run_tests.yml` @ 933d8d93819c | likely |
| astral-sh/ruff | Job level — `git diff --quiet MERGE_BASE...HEAD -- ':!docs/**' ':!assets/**'` → `code` | `required-checks-passed` | `ci.yaml` @ f1ac2e7986b1 | certain (main check — Markdown comment) |
| astral-sh/uv | Job level — `plan.yml` classifies files | `required-checks-passed` | `ci.yml`·`plan.yml` @ 716f320609fe | likely |
| denoland/deno | Job level — if only under `doc/`, docs_only · build·test are skipped and lint runs · if diff fails, everything | aggregate job not seen | `ci.generated.yml` @ b157cd27e153 | likely |
| vitejs/vite | Job level — `tj-actions/changed-files` · `ci.yml` is excluded from `.github/**` | `test-passed`/`test-failed` | `ci.yml` @ bc598a6a8a6b | likely |
| rust-lang/rust-analyzer | Rust CI always runs · only the TS job uses `dorny/paths-filter` | `conclusion` job · `merge_group` | `ci.yaml` @ 1ad44dc58e65 | certain (main check) |
| servo/servo | Not by path — reduces PRs by event kind, and everything on merge queue·main | `build-result` aggregate | `main.yml` @ d05154e2b4de | likely |
| nodejs/node | Trigger-level `paths-ignore` (PR·push) · ignores `.github/**` and then revives its own file with `!` · `doc/` is not ignored | not seen | `test-linux.yml` @ 4889fb0a4373 | certain (main check + run record) |
| ruby/ruby | Trigger-level `paths-ignore` **only on push** — dropped on the PR side because required checks stalled | comment "Do not use paths-ignore for required status checks" | `ubuntu.yml` · commits 4303a02f46 · 8d4ba9d443 | certain (main check) |
| microsoft/playwright | Trigger-level `paths-ignore` (PR, `docs/**` etc.) | not seen | `tests_primary.yml` @ b9a34ac7783a | likely |
| facebook/react | Trigger level — runtime CI ignores `compiler/**`, the compiler workflow only `compiler/**` | not seen | `runtime_build_and_test.yml` @ d083ec1da1e5 | likely |
| tauri-apps/tauri | Trigger-level **include list** (PR only — `crates/**`·Cargo files·its own workflow + `!` exclusions) · `dev` push always runs | not seen | `test-core.yml` @ 15468de79772 | certain (main check) |
| wezterm/wezterm | Trigger-level **include list** (`**/*.rs`·Cargo·assets·its own workflow) — per-platform workflows | not seen | `gen_windows.yml` @ b09b56c29c1e | certain (main check) |
| pola-rs/polars | Trigger-level **include list** (PR·push) — per-area workflows | not seen | `test-rust.yml` @ 5fdcc2ffe33a | certain (main check) |
| biomejs/biome | Trigger-level include list (PR) + `merge_group` is not filtered | unclear | `pull_request.yml` @ 6964bfc8a13c | likely |
| tokio · Bevy · cargo · TypeScript · VS Code (PR) | Do not filter by path | Bevy·TypeScript = `merge_group` | each `ci.yml` etc. | likely |
| rust-lang/rust | No docs-path skipping in the YAML · the job set is computed by `citool` (not read) | bors | `ci.yml` @ 5ceaf6608eb3 | uncertain |

- **Electron's prohibition clause** — "Never add .github/workflows/** or .github/actions/** here - CI changes can affect builds" (`build.yml`). Our list also does not exclude `.github/`.
- **Everything runs on failure (fail-open)** — Deno ("An empty diff defaults to running the full CI to be safe.") · Next.js run the whole CI when detection fails. A trigger-level filter has no such option — GitHub runs it (fail-open) above 1,000 commits or on diff timeout, and **above 3,000 files it may not run (fail-closed, no notification)**. We wrote this down as an accepted exception (ADR-0232).

### B. GitHub behavior and pitfalls (official docs)

Sources = the `paths`/`paths-ignore` section and the filter pattern section of https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax, https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks, https://docs.github.com/en/actions/how-tos/manage-workflow-runs/skip-workflow-runs. All of the following are **certain** (wording checked), and what overlaps with ADR-0232 rationale ①~⑤ is not written again.

- **merge queue (`merge_group`) accepts only `branches` and cannot accept `paths`** — if a merge queue is used, trigger-level path skipping does not hold (job-level detection is needed).
- **The aggregate-job pitfall** — if a job listed in `needs:` fails or is skipped, the job that needs it is skipped too, and a skipped job counts as success. So if `if: always()` is left off the aggregate job, **a failure is disguised as green**. The Next.js comment wrote the same warning.
- **`[skip ci]` and the like** — they work only on `push`·`pull_request`, not on `pull_request_target`, and leave required checks Pending. Whether, on a multi-commit push, the markers of commits other than the head commit also work, the docs and the 2021 changelog say differently (**uncertain**).
- **`concurrency`** — cancellation happens 「새 실행이 시작될 때」 (when a new run starts). A push for which no run was created because of a path filter does not cancel the earlier run (a necessary reading of the wording — **likely**). The docs-only push on our branch (`60c1249`) had 0 runs, but ★**the earlier run had already finished then, so whether it would cancel was not tested**★.
- **`'**'` and paths starting with a dot** — GitHub does not document the matching engine or the dot-handling rules of trigger filters. The story of `actions/labeler` that 「minimatch 가 점 파일을 안 잡는다」 belongs to **a different tool** and must not be carried over. The direct evidence is one measurement on our branch (a run was created for the `be71106` push, whose only non-docs file was `.github/workflows/ci.yml`) — ★that push was a new-branch creation push, so it followed the new-branch diff rules★ (not measured with an existing-branch push · the next one, `e9abf55`, is a positive pattern that revives `qa.md` literally, so it is not a test of this question). **likely (n=1).**

### C. Security of third-party detection actions

- **The `tj-actions/changed-files` supply-chain incident — CVE-2025-30066 / GHSA-mrrh-fwg8-r2c3** (2025-03-14~15). Version tags were moved retroactively to a malicious commit that printed secrets from the runner memory to the workflow log, and in public repositories that log was public. More than 23,000 repositories were in the blast radius and it was fixed in v46.0.1 (https://github.com/advisories/GHSA-mrrh-fwg8-r2c3 · CISA alert). **Certain.**
- GitHub guidance — "Pinning an action to a full-length commit SHA is currently the only way to use an action as an immutable release." (https://docs.github.com/en/actions/reference/security/secure-use) Our workflow already pins actions by SHA (ADR-0131). When moving to the job level, **a hand-written `git diff` step** removes third-party trust altogether (Ruff · Zed · Deno do it that way) — the price is having to handle the fetch depth · the case where a new branch's `before` is 0 · commits unreachable after a force push (general knowledge — **likely**).

### D. Include list vs exclude list — grounds and incidents

- **There is no official guidance.** We found no document from GitHub or a large organization that names either side as recommended.
- Kubernetes Prow offers the two shapes as mutually exclusive fields (`run_if_changed` · `skip_if_only_changed`), and gives the example 「문서만 바꾼 PR 에서 컴파일 잡을 건너뛰기」 under the **exclude-side field** (https://docs.prow.k8s.io/docs/jobs/). **Certain** (the docs check was by the collector).
- **We found no post-mortem of a large repository where a path filter silently skipped CI and broken code entered main.** What is recorded: a near miss caught in review (Node #37999), a case where required checks stalled (Ruby), a glob mistake (Node's `*.nix` → `**.nix` — this one ran more than necessary), and a doctest omission in a small repository (delta-arrow-reader #135).

## Constraint fit (design-decision mode)

Our constraints = public repository · a single Windows runner (verification about 7 minutes) · no branch protection (T-18) · no merge queue · one workflow checks everything · actions pinned by SHA · one docs file is a test input.

| Candidate | Removes the wait on docs push | Compatible with required checks | Third-party trust | Complexity | For us |
|---|---|---|---|---|---|
| Trigger-level exclude list (current — `paths` + `!`) | ○ zero runners | ✕ (Pending) — no required checks now | none | minimal | **fits** |
| Trigger-level `paths-ignore` + `!` revival (Node shape) | ○ | ✕ | none | minimal | the docs do not guarantee the revival — nothing to gain by changing |
| Trigger-level include list | ○ | ✕ | none | list management | does not fit, since it is a whole-check workflow, not a split by area |
| Job level + hand-written `git diff` + aggregate job | △ a runner starts and only the heavy jobs are skipped | ○ | none | medium | the first candidate if we set branch protection (ADR-0232 reopening ①) |
| Job level + third-party detection action | △ | ○ | yes (CVE-2025-30066 precedent) | medium | no reason to bring it in |
| No filtering | ✕ | ○ | none | minimal | opposite of the user's demand |
| `[skip ci]` | a person must attach it | ✕ | none | minimal | no practice · easy to forget |

**Rejection candidates (candidates for ADR rejected alternatives):** third-party detection action (security precedent + replaceable with a hand-written diff) · `[skip ci]` (no practice · relies on human memory). Both are worth adding to this decision, but ADR-0232 is already final, so **whether to append them is the user's decision.**

## Adversarial review result

Codex (cross-family) once · decision **FIX** · of 8 spot checks, 7 held (GitHub Pending/Success asymmetry · CPython aggregate job · Next.js detection·aggregate · Ruff Markdown comment · Ruby push-only filter · Vite · the 3,000-file limit).

| Finding | Disposition |
|---|---|
| Asserted from the Node run record that the `!` revival inside `paths-ignore` is a 「고쳐야 할 틀린 서술」 (wrong statement to fix) | **Accepted (confidence downgraded)** — turned the 「고칠 것」 (what to fix) above into an overstatement correction and lowered it to uncertain. However, the review's counter-logic (「그 커밋들이 `test-linux.yml` 자체를 바꿨으니 분리가 안 된다」) is not a refutation — that file changing is the premise of the revival test. The ground for leaving it uncertain is a defect on our side (per-commit push not confirmed) |
| 「필수 체크를 거는 대형 프로젝트는 거의 전부 잡 단위」 — a ratio claim with no population | **Accepted** — changed to a description of the observed sample and added Ruby's other shape |
| `[skip ci]` 0 occurrences — a workflow file search cannot measure commit message usage | **Accepted** — lowered to uncertain |
| 「포함 목록은 분할 워크플로용」 — the Biome counterexample | **Accepted** — trimmed to 「분할에서 많이 보인다」 |

## Contested points · limits

- **Whether `!` inside `paths-ignore` revives — cannot be decided (uncertain)** — main leaned toward 「된다」 from the Node run record, and the adversarial review toward 「증명 아님」. It is a minor point that does not affect the conclusion (keep the setting), so we do not rule and leave it uncertain. To resolve it, find one push of a single commit that changed only its own workflow file under `.github/`.
- **Main ruled on one conflict between collectors using the original** — one track said 「대형 저장소 중 워크플로 단 포함 목록을 쓰는 곳은 없다」, and another classified Tauri·WezTerm·Polars·Biome as include lists. We opened the original files of three and confirmed they are include lists — the first track's generalization was wrong.
- **「필수 체크를 거는가」 cannot be known from public files** — the required-check column of the table was inferred from aggregate jobs·comments. Only CPython·Electron state it explicitly.
- **Not read** — CPython `compute-changes.py` · rust-lang/rust `citool` · VS Code Azure Pipelines · Bun Buildkite · the aggregate jobs of Bevy/TypeScript/rust-analyzer/Deno/Biome.
- **Outside GitHub Actions (GitLab etc.) is out of scope** — one track added the pitfall that GitLab `rules:changes` is always true on a new-branch push, but it is unrelated to us.
