# ADR-0397: Cross-platform probe - a manual-only Linux and macOS build and test workflow, separate from ci.yml

- 상태: 확정 (2026-11-21, 근거: user decision)
- 관련: ADR-0131 (single windows runner — `ci.yml` is unchanged by this decision) · ADR-0230 (platform neutrality — this is the first measurement of the single-runner gap it records)

## 맥락

The product is meant to run on macOS and Linux as well as Windows (user decision 2026-11-21). The workspace has only ever been built and tested on Windows: `ci.yml` runs on `windows-latest` alone (ADR-0131 decision 1), so nobody knows whether it even compiles elsewhere, and new Windows-only code can land without anyone noticing.

macOS cannot be tested from the Windows dev PC — Apple's license allows macOS virtual machines only on Apple hardware. Linux can be tried locally through WSL2, but that covers one OS and only by hand.

The current work in progress is large, so the porting fixes themselves are to wait until it is done. What is wanted now is only a way to see the size of the gap.

## 결정

Add `.github/workflows/cross-platform.yml`, a probe that is not a gate.

- Runners = `ubuntu-latest` and `macos-latest` (GitHub-hosted, so real macOS on Apple hardware), `fail-fast: false` so one OS failing does not hide the other's results.
- Backend job: `cargo build --locked --workspace --all-targets --keep-going`, then `cargo test --locked --workspace --no-fail-fast` with the same six claude-dependent skips as `ci.yml` (ADR-0131 decision 3). Linux installs the Tauri v2 system packages first.
- Frontend job: `npm ci`, `npm test`, `npx tsc --noEmit`.
- Trigger = `workflow_dispatch` only. Run it with `gh workflow run cross-platform.yml --ref <branch>` or from the Actions tab.
- Red is the expected state until porting is done. No merge decision reads it.
- `ci.yml` stays Windows-only.
- Exit: when both OSes are green and stay green, move these jobs into `ci.yml` as a gate and retire this file.

## 거부한 대안

- **Add Linux and macOS to the `ci.yml` matrix with `continue-on-error`** — the merge check waits for the whole `ci.yml` run to reach `completed` (`docs/handbook/branch-commit.md` 「Integration」), so every merge would wait for the macOS build even though its result is ignored. The macOS build time is unmeasured.
- **A separate workflow that runs on every push** — it does not delay the merge check, but it spends runner time and produces a red run on every push while the result is known to be red. The user chose to run it only when needed (user direct rejection).
- **Emulate macOS on the Windows PC** — Apple's license permits macOS virtualization only on Apple hardware, and unofficial VMs are unstable as a test base.
- **WSL2 only** — covers Linux only, by hand, on one PC. Still useful for looking at the Linux GUI later, but not a replacement for this probe.

## 근거

- User decision 2026-11-21: macOS and Linux support is wanted; measure the gap now, fix after the current large work; run manually, not on every push.
- The repository is public, so GitHub-hosted runner minutes, macOS included, cost nothing.

## 영향 / 불변식

- This workflow never runs on `push` or `pull_request`. Adding either trigger brings back the cost the rejected alternatives avoid; that change needs a new decision.
- Its skip list mirrors `ci.yml`'s. When the `ci.yml` skip list changes, change this one too.
- It covers build and headless tests only. Window behavior, the tray and pop-outs on macOS and Linux still need a real machine (macOS) or a desktop session (Linux, e.g. WSL2 + WSLg).
- macOS code signing and notarization (Apple Developer Program) are out of scope. Distribution starts unsigned.
