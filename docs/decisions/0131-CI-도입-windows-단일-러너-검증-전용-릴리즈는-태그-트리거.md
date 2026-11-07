# ADR-0131: Introduce CI — a single windows runner · verification only · releases triggered by tags

- 상태: 확정 (2026-08-09~11, basis: user decision + 3 rounds of adversarial review + measured runs) · 부분 폐기 by ADR-0232 (decision 2, excluding pushes and PRs that change only documentation paths from the every-push scope) · 부분 폐기 by ADR-0340 (the full regression in the impact item on the local/CI split goes to CI)
- 관련: ADR-0003(core isolation) · ADR-0110(messaging kernel isolation) · ADR-0129(network row isolation gate) · ADR-0100(release co-location) · ADR-0012(module isolation harness) · step-log S18.26 · ADR-0230(platform neutrality — this single windows runner is recorded as a known gap under that principle) · Amended by ADR-0232 (decision 2, excluding pushes and PRs that change only documentation paths from the every-push scope) · Amended by ADR-0340 (the full regression in the impact item on the local/CI split goes to CI)

## 맥락

This project had **no CI at all**. The verification commands were all defined in CLAUDE.md 「빌드·검증 명령」, but a person had to remember and type every one of them.

In particular **the isolation gates were fragile**. The ban on tauri imports in core (ADR-0003), the messaging kernel's zero dependencies (ADR-0110), the dependency cap of the network row (ADR-0129) — the compiler enforces none of the three. The syntax is fine and it runs. If a person does not count, it quietly collapses, and in fact there was an incident in which an async runtime was quietly brought into discovery (2026-08-05, the trigger for the net `default = []` decision).

Because the practice is to push to the same branch from several PCs, a surface was also needed to filter out the "it works on my PC" class.

## 결정

**Automate the whole verification gate + the tag-triggered release with GitHub Actions.** The artifacts = `.github/workflows/ci.yml` + `rust-toolchain.toml`.

### 1. Runner = `windows-latest` only

There is no Linux matrix. The Job Object code is `#[cfg(windows)]` and the PTY fixtures are `cmd.exe`/`powershell.exe`, so **whether this workspace even stands on Linux is itself unmeasured**. The only exception is the release publish job, on `ubuntu-latest` (shrinking the surface of the job that holds the write token — see 4 below).

### 2. Scope = verification only, releases only from tags

What runs on every push is only CLAUDE.md 「빌드·검증 명령」 and the isolation gates. `tauri build` runs only on tags — it is heavy (measured 634s) and not needed on every push.

### 3. The 6 tests that depend on a real claude are excluded from the CI lane

The runner has no claude. The tests' own skip guard (`spawn_json_agent` → `skip_no_claude`) **does not work on the runner** — the guard only checks ① the spawn call succeeded ② it appeared in the list within 5 seconds, and even when claude is absent, the process starts, gets caught in the list, and then dies. Which one blows up varies with the runner's timing (measured: first run `c2_live_…`, second run `mcp_send_message_…`), so they were removed by axis, not individually.

**CI coverage of this axis is 0, and it is verified only locally.** The test sources were not changed.

### 4. The release is split into a build job and a publish job — isolating the write token

- `release-build` (`contents: read`) — `npm ci` · cargo · `tauri build`. **This is where third-party code runs. No token.**
- `release-publish` (`contents: write`) — receives only the artifacts and publishes them. 3 steps, no npm, cargo, or packaging scripts.

With one job, a corrupted build dependency could plant a fake `gh` through `GITHUB_PATH`, steal the write token, and finish pretending to succeed (found as a cross-family review BLOCK). The two jobs are on different runners, so that path is cut.

### 5. The packaging logic is not reimplemented

It calls `scripts/build-release.ps1`. The single source of the ADR-0100 co-location invariant and of "what do we ship" is that script's manifest. If the expected file list were written in the workflow, there would be two sources and they would inevitably drift apart.

### 6. The version is only asserted, never rewritten

The tag is compared against the 4 places of the product version (`tauri.conf.json` · `src-tauri/Cargo.toml` · daemon `Cargo.toml` · `package.json`) + `package-lock.json`, and if they differ it stops **before the build**. CI does not align the version automatically — if the correspondence between the repository contents and the shipped artifact breaks, you cannot trace "which commit was this version".

## 거부한 대안

- **Add an `ubuntu-latest` matrix** — fast and cheap, but Linux support is unmeasured, so the first run would itself become a porting job. Windows-only code paths are not verified on Linux anyway.
- **A self-hosted runner (the dev PC)** — free and fast and it could even do GUI measurement, but **self-hosted on a public repo is taboo for security** (a fork PR would run code on the runner). The free public runner is enough.
- **Create an `ENGRAM_TEST_REQUIRE_CLAUDE=1` lane** — it is a knob the test source already provides, but since it is certain the runner has no claude, setting it makes CI red by design.
- **Inject a skip flag or `continue-on-error` into the claude-dependent tests** — it hides the failure. Exclusion by name + stating what is uncovered was chosen.
- **A single release job** — simple, but it opens the token-theft path in 4 above.
- **CI automatically bumps the version** — less manual work, but the repository code and the shipped artifact diverge.
- **Add `actions/attest-build-provenance`** — it proves "did this zip come out of our CI", but **it does not prove that the build dependencies were clean**. An artifact carrying a Trojan horse also receives a valid attestation. The placement where it would yield value (the build job) puts `id-token: write`+`attestations: write` back on exactly the job that 4 above isolated. If the threat is "swapping the assets on the release page", the placement in the publish job is right — but that is a separate matter.
- **Introduce branch protection (no merge unless green) at the same time** — if set before observing CI's stability, even fixing CI itself would have to go through a PR. Reconsider after a few days of observation (open).

## 영향

- **The callers and the source of truth are mostly separate.** The gates defined in CLAUDE.md and the net lib.rs header are only **called** by the workflow — if those are fixed, the workflow has to be fixed with them. **2 exceptions where the workflow is the only source** (ts-rs bindings sync · discovery async intake — there is no corresponding command in the local documents). Those two quietly drop out of the local fallback, so the `/qa` binding explicitly holds a pointer to them.
- **The local/CI split has changed** — local is quick + GUI measurement, and the full regression is CI. Source of truth = `.claude/skill-bindings/qa.md` 「CI와의 분담」.
- **`--locked` was attached to every cargo call.** If dependencies change and `Cargo.lock` is not committed, CI goes red (intended friction).
- **What this workflow cannot block** — it is also written in the workflow file itself. In short: the job split **protects the token, not the artifact**. A corrupted build dependency can still put a Trojan horse in the zip, and the publish job faithfully publishes it. Reproducible builds · signing · dependency audits are out of scope.
- **3 open items:** ① handling the 6 real-claude-dependent tests (leave as is / a fake CLI harness / promote skip to failure) ② branch protection ③ bringing GUI measurement into CI (there is no registered scenario, so there is nothing to automate — writing the scenarios is the real work).

## 근거 — what the adversarial review found (summary)

3 rounds, doc-aware (lead) + cross-family (non-lead) in parallel. The verdicts FIX → BLOCK → FIX/BLOCK, 15 items in total were applied. Three worth leaving in this decision document:

1. **The ts-rs binding gate was vacuous** — `cargo test -p engram-dashboard-protocol` **only regenerates** the bindings, and `tests/ts_export.rs` has 0 assertions, so wire-contract↔TS drift passed green. It was closed by appending `git diff --exit-code` after it.
2. **A public release could go out under a non-version tag** — the `tags: ['v*']` glob applies only to pushes, and `workflow_dispatch` can point at an arbitrary ref. A name guard was put directly in the job `if`.
3. **The asymmetry of pinning without checking** — rustc and ripgrep were pinned to specific versions, but there was no assertion that the pinned version actually arrived. Even if the pin is neutralized (file deleted · `RUSTUP_TOOLCHAIN` interference · a different rg on the runner), it stays green. Runtime assertions were added to both, and the rustc expected value is read from `rust-toolchain.toml` so as **not to create a second source of truth.** (The action SHA is enforced by GitHub, so there is no separate assertion — it is enough to keep the pin format.)
