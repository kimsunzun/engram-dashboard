# CI gate practices for flaky · timing-sensitive tests — research report

> **Status:** medium completed (3 primary-family collection tracks + main-session grounding + 1 cross-family adversarial review)
> **Date:** 2026-07-02 · **Method:** `/research` medium — collectors (sonnet)×3 in parallel (8~12 searches each) → main-session grounding (every load-bearing claim, implications judged from quotations) → Codex adversarial review (effort=medium · level 2~3 · web_search)
> **Confidence legend:** certain = independent cross-confirmation / likely = grounding supports + passed review (single source) / uncertain = unsupported · on hold / contested = counter-evidence exists
> **Question:** One passing GUI/E2E measurement cannot guarantee race-free — how does the industry's CI gate handle flaky · timing-sensitive tests?
> **Purpose (discovery chain):** the basis for the honest note "1 PASS ≠ race-free" in the qa skill redesign. Connected to `docs/reference/debugging-conventions.md` (where flaky is raised).

## Key conclusions

1. **"One pass is not grounds for trust" is the industry consensus.** A non-deterministic test, when it fails, does not tell you whether it is a bug, and when it passes, does not prove the absence of a defect (Fowler: "useless + virulent infection"). A race is the probabilistic manifestation of an interleaving, so a single run sees only one of the possible schedules. [likely]
   - A statistical nuance: "1 run = no information" is rhetoric, and the amount of information depends on the prior distribution of the failure probability — a rare flake (p≈1e-4) may need tens of thousands of runs for confident detection (Wilson interval discussion, arXiv 2512.18088). **So there is no magic number like "run it N times and it is safe" either.** [likely]
2. **Detection standard = rerun-based.** Rerun on the same code, and if pass/fail splits, it is flaky. Microsoft (decided if fail→retry-pass happens once), Develocity (2 axes: within a build + across builds), CircleCI (a 14-day window on the same commit). Advanced forms are statistical/ML (Atlassian Bayesian multi-signal, Fitbit autoencoder) and detection without rerunning (DeFlaker coverage differencing, 96% detection · 1.5% false positives). [likely]
3. **Gate standard = presubmit (blocking) / postsubmit (observing) split + automated quarantine.** A test judged flaky is removed from the merge gate but keeps running and being observed — Microsoft always runs it and only suppresses the result, Dropbox ignores it in presubmit and keeps running it in postsubmit, Buildkite quarantine = soft-fail. Quarantine comes with a duty to repair (Fowler: at most 8 · a 1-week cap / Microsoft: penalties such as blocking PRs when 10+ flaky bugs accumulate). [likely]
4. **Retry is a "judging tool", not a "passing tool".** Dropbox retries at most 10 times, for judging only. Retry for the purpose of passing destroys the signal and hides real bugs (Fowler). The tools' automatic retry default is 0, and even when set, a low cap of about 2~3 is given as an example (saying "2~3 is the standard recommendation" is an exaggeration — review correction). [likely]
5. **Prevention standard (timing/race) = conditional waiting + securing determinism.** No fixed sleep → Selenium explicit wait / Playwright auto-wait (actionability 5 checks: Visible·Stable·Receives Events·Enabled·Editable) / Cypress retry-ability. Wrap the system clock and replace it with a fake timer (but Promise microtasks need separate handling), remove state shared between tests, isolate external dependencies hermetically. [likely — the framework recommendations themselves were confirmed in official docs]
6. **⚠ The hidden cost of test-level quarantine.** There is research that flaky tests expose more than 1/3 of real regression defects — even with a flaky predictor precision of 99.2%, about 76% of regression defects can be missed (Chromium CI study, arXiv 2302.10594). The more accurate frame is not "flaky = always quarantine" but **classifying the failure instance** (is this failure a flake or a real one). [likely — found by the reviewer's omission probe]

## Findings in detail

### A. Detection · quantification
| Finding | Confidence | Source |
|---|---|---|
| Microsoft CloudBuild/CloudTest: judged flaky if fail→retry-pass happens once → automatic quarantine + automatic bug creation, automatic release on close. ~49,000 identified · 100+ teams | likely (read directly from the primary text) | [MS devblogs](https://devblogs.microsoft.com/engineering-at-microsoft/improving-developer-productivity-via-flaky-test-management/) |
| Google TAP: rerun a failing test 10 times, flaky if it passes even once — **but this figure is hearsay that Luo FSE 2014 cited as a personal communication from John Micco** (primary document not checked) | likely (beware hearsay) | [Luo et al. FSE 2014](https://mir.cs.illinois.edu/marinov/publications/LuoETAL14FlakyTestsAnalysis.pdf) |
| Develocity: 2-axis detection of within-build (fail→pass) + cross-build (pass/fail mixed on the same input) | likely (official docs) | [Develocity docs](https://docs.gradle.com/develocity/flaky-test-detection/) |
| CircleCI Test Insights: pass/fail mixed on the same commit within a 14-day window → flaky | likely | [CircleCI blog](https://circleci.com/blog/introducing-test-insights-with-flaky-test-detection/) |
| Atlassian Flakinator: Bayesian + multi-signal (duration variance · environment · retry frequency) 0~1 score. Of master failures, 21% (FE) / 15% (BE) were due to flaky | likely (read directly from the primary text) | [Atlassian blog](https://www.atlassian.com/blog/atlassian-engineering/taming-test-flakiness-how-we-built-a-scalable-tool-to-detect-and-manage-flaky-tests) |
| DeFlaker: detection without rerunning via coverage differencing — 96% detection vs rerun, 1.5% false positives | likely | [DeFlaker (ICSE 2018)](https://www.cs.cornell.edu/~legunsen/pubs/BellETAL18DeFlaker.pdf) |
| Chromium Findit: up to 400 reruns to trace the culprit commit (not for detection) | likely | [Findit CAT](https://sites.google.com/chromium.org/cat/findit) |

### B. Scale figures
| Finding | Confidence | Source |
|---|---|---|
| Google (2016): ~1.5% of all test *runs* had flaky results · ~16% ("1 in 7") of all *tests* are flaky to some degree. ~~30-day window~~ (not in the primary text — removed by review correction) | likely | [Google Testing Blog 2016](https://testing.googleblog.com/2016/05/flaky-tests-at-google-and-how-we.html) |
| Google (2016): 84% of pass→fail transitions are flaky (not real regressions) — at the collection stage this was suspected as "citation laundering", but **the adversarial review confirmed it exists in the primary text** | likely | same as above |
| Luo et al. FSE 2014 cause distribution (51 projects · 201 fix-commits): **raw figures async wait 74 / concurrency 32 / order dependency 19 (≈37%/16%/9% of 201)**. The widely cited 45%/20%/12% appears to be based on the classifiable portion — beware the denominator (review correction) | likely | [Luo et al.](https://mir.cs.illinois.edu/marinov/publications/LuoETAL14FlakyTestsAnalysis.pdf) |
| GitHub Actions (1,960 Java OSS): 3.2% of builds were rerun, of which 67.73% showed flaky behavior — but it is a "flaky *builds*" study (not test-level) | likely | [arXiv 2602.02307](https://arxiv.org/abs/2602.02307) |
| 75% of flaky tests are flaky from the first commit (Lam et al. OOPSLA 2020) | likely (primary text not read · summary-based) | [OOPSLA 2020](https://dl.acm.org/doi/10.1145/3428270) |

### C. Gate policy
| Finding | Confidence | Source |
|---|---|---|
| Fowler: non-deterministic test = "useless + virulent infection" · no bare sleep · quarantine at most 8 or a 1-week cap · quarantine goes outside the main pipeline | certain (primary text confirmed several times + passed review) | [martinfowler.com](https://martinfowler.com/articles/nonDeterminism.html) |
| Meta: based on PFS (Probabilistic Flakiness Score) — degraded tests are excluded from change-based testing | likely (read directly from the primary text) | [engineering.fb](https://engineering.fb.com/2020/12/10/developer-tools/probabilistic-flakiness/) |
| Dropbox Athena: 2+ postsubmit failures within a few hours → "noisy" → ignored in presubmit · kept running in postsubmit. Up to 10 retries for judging (not for passing) | likely (read directly from the primary text) | [dropbox.tech](https://dropbox.tech/infrastructure/athena-our-automated-build-health-management-system) |
| Buildkite: quarantine = soft-fail (no effect on the pipeline result) | likely (official docs) | [Buildkite docs](https://buildkite.com/docs/test-engine/test-state-and-quarantine) |
| Atlassian: flaky detection → automatic Jira ticket creation + ownership assignment, automatic re-entry once healthy for a set period | likely (read directly from the primary text) | Atlassian blog (above) |
| Spotify "Master Guardian": flaky if fail→retrigger-pass → ticket, skipped pre-merge while open | **uncertain** — secondary write-up only, primary source not checked (the reviewer also abstained) | [secondary: engineering.atspotify write-up](https://engineering.atspotify.com/2019/11/test-flakiness-methods-for-identifying-and-dealing-with-flaky-tests) |
| GitHub Actions: a native automatic flaky-isolation feature is absent in the docs (only step retry · manual re-run) — not a *proof* of absence | likely | [GH docs](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs) |

### D. Prevention (timing/race)
| Finding | Confidence | Source |
|---|---|---|
| Selenium: avoid sleep → explicit wait (state the condition), do not mix implicit+explicit ("unpredictable wait times") | certain (read directly from official docs + passed review) | [Selenium Waits](https://www.selenium.dev/documentation/webdriver/waits/) |
| Playwright: auto-wait actionability 5 checks before every action (Visible·Stable [bbox identical for 2 frames]·Receives Events·Enabled·Editable) + auto-retrying assertions | certain (official docs) | [Playwright Actionability](https://playwright.dev/docs/actionability) |
| Cypress: retry-ability — proceed as soon as the condition is met, removing hard-coded waits is the first line of defense | certain (official docs) | [Cypress Retry-ability](https://docs.cypress.io/app/core-concepts/retry-ability) |
| Wrap the system clock + fake timer (macrotask control, Promise microtasks need an async variant) · remove shared state · hermetic isolation | likely | [Jest](https://jestjs.io/docs/timer-mocks) · [Sinon](https://sinonjs.org/releases/latest/fake-timers/) · Fowler (above) · [Hermetic Servers](https://testing.googleblog.com/2012/10/hermetic-servers.html) |

## Adversarial review result (what cross-family actually caught in this run)

The reviewer (Codex, level 2~3) corrected errors in the collection and synthesis with counter-sources:
1. **Direction reversed:** we suspected the "84% transitions" figure of citation laundering (tagged uncertain) → the reviewer confirmed it exists in the Google primary text (promoted to supported).
2. **Denominator correction:** Luo 45%/20%/12% is based on the classifiable portion — the raw figures are 74/32/19 of 201 (≈37%/16%/9%).
3. **Excess precision removed:** the "30-day window" that travels with Google's 16% is not in the primary text.
4. **Source grade downgraded:** "TAP 10 runs" is not a Google primary document but a personal-communication citation in a paper (hearsay).
5. **Exaggeration softened:** "retry 2~3 = the dominant recommendation" → the tools' default is 0, and 2~3 is a configuration example/cap.

## Implications for engram (basis for the qa skill redesign)

- **A single cdp measurement PASS = proof that "the feature exists and works under this interleaving", not proof of race-free** — this report is the basis of the qa skill's honest note. There is also no magic "N passes = safe" number (it depends on the prior).
- At our scale (one person · a desktop app), big-company-style quarantine infrastructure is excessive. A realistic adoption: **① write timing-sensitive tests with conditional waiting (no fixed sleep — consistent with the ADR-0038 5ms→50ms case) ② when flaky is suspected, rerun N times on the same code to decide (no retry for passing) ③ a test judged flaky is removed from the gate but recorded, not silently deleted**.
- The failure-instance classification view (conclusion 6) is the frame for when qa later asks "is this failure a flake".

## Disputes/limits

- Spotify Master Guardian: primary source not checked → kept uncertain (both collector and reviewer abstained).
- TAP 10 runs: hearsay-based — checking Google's primary document failed (the collector repeatedly could not access the PDF).
- Link liveness: most were confirmed by actual fetches during collection · review, and a separate full pass was skipped (a medium-scope judgment).
- Uninvestigated gaps: a deeper UI-specific cause distribution (arXiv 2103.02669) · the static prediction family (Flakify/FlakeFlagger, arXiv 2112.12331) · lightweight policies for small teams · security-test quarantine exceptions · quantifying quarantine's coverage drift.
- Timing: the Google/Fowler materials are from 2011~2016 — the direction remains valid but the figures are dated.
