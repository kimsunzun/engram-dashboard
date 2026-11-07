# Third-party license notice — when to generate it and how to verify it

- **Status:** medium (3 collectors · main grounding · codex adversarial review BLOCK → findings addressed · 「적대 리뷰」 (Adversarial review) below). **Decision (user 2026-10-15) = generate at release build · do not commit** (changes the old decision 「손으로 뽑아 커밋」 (extract by hand and commit)) — ADR-0294. A follow-up to the tool-selection survey (`third-party-license-notice-survey-2026-10-15.md` — branch `v0.3.3/chore/misc`). **Trigger:** when the user changed 「손으로 뽑아 커밋」 to 「릴리즈 뽑을 때 생성」 (generate when cutting a release) (2026-10-15), the user asked 「매번 뽑을 때 잘못 뽑히는 위험은 없나 · 다른 곳은 어떻게 하나」 (is there no risk of a wrong extraction each time · how do other places do it).
- **Date:** 2026-10-15
- **Confidence legend:** certain = original text checked directly + independent evidence · likely = original text checked (single) or a collector report partly checked by main · uncertain = collector report only

## Conclusion

1. **In the Rust desktop sample we looked at, 「번들(릴리즈) 때 생성 · 커밋 안 함」 (generate at bundle (release) time · do not commit) was common — the sample is small, so we do not judge it to be the mainstream.** (likely — limited to the sample)
   - Zed — the bundle script generates first: `script/bundle-mac:88-89` 「Generate the licenses first, so they can be baked into the binaries」 · the output is `.gitignore:27` `/assets/*licenses.*` (checked by main).
   - Warp · ALVR · Graphite also run cargo-about at the packaging step (collector report · not checked by main · no revision-pinned source — uncertain).
   - Counterexample: advancedfx (a desktop project with some Rust) commits `misc/THIRDPARTY.yml` and checks it when Rust dependencies change (a counterexample raised by the adversarial review · not checked by main).
   - Other tracks: **commit + a CI up-to-date check** = Signal Desktop (`.github/workflows/ci.yml:95-99` — regenerate and then `git diff --exit-code` · checked by main) · libsignal (`--check` mode). **Commit + by hand/an in-house bot** = VS Code · PowerToys · Windows Terminal (collector report).
2. **So that it does not blow up for the first time at the release, run the same generation in a PR too and discard the result.** (certain)
   - The `check_licenses` job of Zed `run_tests.yml` (L769-792 · as of `main` 2026-10-15) runs `script/generate-licenses` · `pull_request` trigger · path filter = `Cargo.lock` · `script/*licenses*` · `LICENSE*` (L117) (checked by main).
   - ALVR `rust.yml` also has a `check-licenses` job on `pull_request` (L131-141) (checked by main).
   - Warp runs only `cargo deny check licenses` in PRs instead of generating (collector report).
3. **Pin the tool version.** (certain) Zed `script/generate-licenses:5` `CARGO_ABOUT_VERSION="0.8.2"` — if different, it installs that version (L30-32) · `--fail` + fails if anything at all is printed to stderr (`fail_on_stderr` · turned off by `ALLOW_MISSING_LICENSES`) (checked by main). libsignal has a version file `acknowledgments/cargo-about-version` and refuses if it differs (collector report).
4. **In cargo-about 0.9.2, the substitution for a 「라이선스 파일 없는 crate」 (crate without a license file) is not caught by `--fail` — the one marker is an empty `source_path` (whether in JSON or in a template).** (certain)
   - 0.9.2 `src/generate.rs:141-155` — if it cannot find the file, it only leaves a `log::debug!` and puts the SPDX standard text (`id.text()`) with `source_path: None` (the comment says 「emit a warning」, but in fact it is debug) (checked by main). Zed uses 0.8.2, so with this 0.9.2 source we cannot say whether Zed's stderr check catches this (uncertain — 0.8.2 source not checked).
   - The MIT in that standard text carries the placeholder `Copyright (c) <year> <copyright holders>` as is (observed in our output).
   - libsignal **marks this and a person fixes it** — the template appends `{{#unless source_path}} (synthesized){{/unless}}` (`acknowledgments.html.hbs:58` · checked by main), and the README says 「새 synthesized 항목이 생기면 clarify 를 더하라」 (when a new synthesized item appears, add a clarify) (collector report). CI does not reject it automatically.
5. **The means of fixing it inside cargo-about = `clarify` (`path` + checksum in `files`/`git`).** (likely — the collector cited the 0.9.2 source · not checked by main) `clarify.files` (a local file + checksum) or `clarify.git` (fetches from the original repository using the Cargo.toml `repository` + the `.cargo_vcs_info.json` commit · needs network · error if there is no `repository`). The built-in `workarounds` list (bitvec · chrono · clap · ring · rustls, etc.) has no `windows-*` or `tauri-*`.
6. **generate-license-file 4.2.5 drops a package whose license it could not determine, leaving only a warning — it does not fail because of that one package** (errors outside that `catch` still fail). (certain) `resolveNpmDependencies.ts:55-88` — in the `catch` it does `logger.warn` and does not rethrow (checked by main). There is no option in the documentation to turn it into a failure (collector report).
7. **Notices that carry a placeholder copyright exist in big companies' repositories too** — microsoft/CsWin32 `NOTICE.txt` and others (collector report · not checked by main). We make no legal judgment.
8. **Other tool tracks:** cargo-bundle-licenses leaves text it could not find as 「NOT FOUND」, the next run preserves the committed file that a person filled in, and it checks drift with `--previous --check-previous` (raised by the adversarial review · not checked by main). The `license-files` clarify of cargo-deny only checks files and does not create a notice.
9. **What we did not find:** a place that catches omissions by comparing the output against the dependency graph (`cargo metadata` · npm tree) · a place that rejects a placeholder with grep — we could not find these in the sample. The observed mechanism is 「버전 고정 + `--fail` + PR 에서 같은 생성 + (커밋형이면) diff 검사 + synthesized 항목 사람 검토」 (version pin + --fail + the same generation in PRs + (if commit type) a diff check + human review of synthesized items).

## If we carry this to Engram (session interpretation — the decision is the user's)

- The user's choice 「릴리즈 때 생성」 (generate at release) is the same as the Zed type (the sample is small, so we do not judge it to be the mainstream). If we place **a job that runs the same generation in normal CI** as its pair, it prevents the first blow-up after the tag — since we cannot retry a tag (CLAUDE.md 「검증」), this pair is needed more than by the peers.
- What our generation script already does (allowlist · identification failure · our own crate mixed in · detection of silent omissions in npm) is on the stricter side than the peers. If we add what the peers lack (omission comparison · placeholder rejection), the basis is not the peers but our own requirement.
- The **placeholder track (about 50 crates + 4 npm)** has three options — ① like libsignal, only mark `source_path == null` items as 「synthesized」 and leave them ② add a `clarify` per crate (`clarify.git` makes the release build fetch the original from the network) ③ make a placeholder a failure to force ②. → Adopted: ③ + replacement files from the upstream repository — the only synthesized one left is siphasher (ADR-0294 decision 5).

## Adversarial review

codex (effort high · web_search) verdict **BLOCK** — all 6 findings addressed (folded into the body above):
- 「주류」 (mainstream) = an overstatement that claimed dominance from a small sample → lowered to a sample-limited wording and added the counterexample (advancedfx).
- An inference that applied the 0.9.2 source behavior to Zed (0.8.2) → lowered to uncertain.
- The wording 「`source_path` 로만」 (only by source_path) looked inconsistent with the libsignal template → rewritten to mean that there is a single marker.
- A missed tool track (cargo-bundle-licenses · clarify `path`) → added in conclusion 8.
- A collector report without a revision pin → marked uncertain.
- codex says it confirmed in the tag source a defect in the `LicenseRef-*` branch of cargo-about 0.9.2 `generate.rs:146-190` (not checked by main) — we do not use a `LicenseRef-*` clarify, so it is irrelevant for now. If we come to use it, we check first.

All the BLOCK reasons were document wording · omissions, and there was no counter-evidence that overturns the basis of the decision 「릴리즈 때 생성 + 평소 CI 에서 같은 생성」 (generate at release + the same generation in normal CI) (advancedfx is a case of the 「커밋 + 검사」 (commit + check) track in the body).

## Limits

- 1 of the 3 collectors (the first collection of the safeguard track) was shallow (10 searches · summary-type fetch) — filled in by a supplementary collection.
- Alacritty · WezTerm · Lapce · RustDesk · Spacedrive · Deno could not be checked (GitHub code-search limit). The only place that wrote down the **reason** for choosing a pattern is a single comment in Zed's bundle script.
- The `LicenseRef-*` branch defect was confirmed only by codex (not checked by main).
