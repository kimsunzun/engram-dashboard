# ADR-0273: Separate the engram CLI from the daemon package into an independent package that produces one exe

- 상태: 확정 (2026-10-02, basis: user decision 2026-10-02 (`docs/refactoring/architecture-discussion-2026-09-26.md` decision candidate 9) + code cross-check 2026-10-02) · 부분 폐기 by ADR-0285 (the install-location rule in decision 4, and decision 5) · 부분 폐기 by ADR-0286 (the remaining daemon-boundary item under Impact)
- 관련: ADR-0132 (control-plane CLI = a single executable `engram` — the package location was not decided) · ADR-0094 (bare-name grant · PATH injection — executable-name alignment) · ADR-0151 (crate decision criteria · 「개명 함정」 name prefix) · ADR-0175 decision 6 (lib weight — this package is a bin) · ADR-0271 (discovery split — the install-location rule goes to the daemon) · `crates/engram-dashboard-daemon/Cargo.toml` (`[[bin]] engram`) · `crates/engram-dashboard-daemon/src/bin/engram.rs` · `crates/engram-dashboard-agent/src/types.rs` (CLI vocabulary such as `CLI_EXE_NAME`) · step-log S21 · Amended by ADR-0285 (the install-location rule in decision 4, and decision 5) · Amended by ADR-0286 (the remaining daemon-boundary item under Impact)

## 맥락

`engram.exe` (the control CLI that an agent calls from its own terminal) is the **second bin target of the daemon package** (`[[bin]] name = "engram"` in `crates/engram-dashboard-daemon/Cargo.toml` · source `src/bin/engram.rs`, 2,791 non-test lines — measured in the memo at master `a226f63`). It does not use the daemon lib at all — the only things it uses are agent's command-vocabulary constants (`use engram_dashboard_agent::types::{…}` in `engram.rs`) · command's request number · discovery's install location (`find_install_root`). Yet because it is the same package, the build drags in the whole daemon dependency set. Minimizing dependencies was the intent, to the point of hand-building HTTP (comment in the same `Cargo.toml` — 「의존성 최소화(std TcpStream 손조립 HTTP)」).

The CLI vocabulary constants (verbs · flags · executable name · status words) are also used by agent itself (`manager.rs` · `commands.rs` · backend) — they are 「`agent.*` 명령의 어휘」, so agent owning them is right.

The user (2026-10-02): 「engram 은 독립 패키지로 빼는 게 맞는데 그냥 shell 처럼 하나 분리」.

## 결정

1. **Set up a separate package that produces one exe, like the shell (`src-tauri` → `engram-dashboard.exe`)** (user 2026-10-02). Location = under `crates/`.
2. **The package name keeps the `engram-dashboard-` prefix** — the CI dependency-ceiling gate identifies workspace members by that name prefix. With any other name the gate is simply passed over (ADR-0151 「개명 함정」).
3. **The bin name stays `engram`** — it interlocks with agent `CLI_EXE_NAME`. If it is changed, mail stops silently (comment in the daemon `Cargo.toml` · ADR-0094).
4. **Dependencies = agent (command vocabulary) · command (request number) · the install-location rule.** The CLI vocabulary stays in agent (「맥락」 above).
5. **The install-location rule (`find_install_root`) gets one more copy on the CLI side** (memo decision candidate 9) — ADR-0271 decision 2 sends that rule to the daemon, and since the CLI leaves the daemon package, it does not depend on the daemon crate. **The two copies are tied together by a test that checks they produce the same path** — the same handling as the `daemon.json` location of the daemon and the shell (ADR-0271 decision 4). Which package that test lives in is decided when 2-3 starts. Start = work order 2-3 (prerequisite 2-2 discovery split).

## 거부한 대안

- **Keep it as the second bin of the daemon package (current).** Rejected = user decision + code: the CLI does not use the daemon lib at all, yet because it is the same package it drags the whole daemon dependency set into the build (「맥락」 above — at odds with the original intent of minimizing dependencies).
- **Rename the bin.** Rejected = code: agent `CLI_EXE_NAME` and the grant · priming · PATH resolution are aligned to that name, so if they diverge mail stops silently (comment in the daemon `Cargo.toml` · ADR-0094).
- **A package name without the `engram-dashboard-` prefix.** Rejected = code: the dependency-ceiling gate identifies members by name prefix and would silently not look at that package (ADR-0151).

## 근거

- **User decision 2026-10-02** — the quotation above.
- **Code cross-check (2026-10-02)** — `[[bin]] engram` in the daemon `Cargo.toml` · the workspace imports of `engram.rs` are only the three, agent · command · discovery · the dependency-minimizing intent in the `Cargo.toml` comment · the comment on the bin name = `CLI_EXE_NAME` alignment.

## 영향 / 불변식

- **The new package registers its gates in three places (`ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md`)** (ADR-0175 Impact). Whether to add `-- --test-threads=4` to the test command is decided by whether that package's tests launch real child processes (CLAUDE.md 「병렬은 테스트 바이너리마다 걸린다」).
- ★**Dependency minimization is only partly obtained**★ — only the daemon's share falls away (tokio · WebSocket · net · messaging · protocol etc.). As long as the CLI depends on agent (decision 4), agent's runtime dependencies are built along with it — `portable-pty` · `windows` (Windows only · Job Object · Restart Manager etc.) · `ts-rs` · `chrono` · base (→ `tracing-subscriber` · `regex`) etc., and `inventory` via command (the CLI also uses command directly). Basis = cross-check of `crates/engram-dashboard-agent/Cargo.toml` · `crates/engram-dashboard-base/Cargo.toml` (2026-10-02). Moving the CLI vocabulary constants out of agent would cut more, but this decision keeps that vocabulary in agent (「맥락」 above).
- **The release build picks up the CLI from the daemon package** — `cargo build --release -p engram-dashboard-daemon --bin engram-dashboard-daemon --bin engram` in `scripts/build-release.ps1`. It is switched to the new package in the same change (the script's expected exe list entry `engram.exe` stays as it is).
- **Bundle the work into units that do not break things** (user 2026-10-02 — 「망가지지 않는 단위로 잘 그룹지어서 작업하라」). The unit that sets up the new package and moves the bin and the unit that changes the release script must go into the same commit so the distribution build does not break (release item above).
- **There are ADRs that point at the old path** — they cite `crates/engram-dashboard-daemon/src/bin/engram.rs` as a related pointer · in the body. The list is not written here — **how to find them = `rg -l "src/bin/engram.rs" docs/decisions`** (this ADR itself is hit too). After this ADR, that path points at the same file in the new package.
- **The remaining daemon boundary (separate matter · undecided)** — the test server functions (`start_test_server*`, `crates/engram-dashboard-daemon/src/lib.rs`) are exposed as public API without a test marking. Moving them behind a test-only feature flag is work order 2-4. The shell integration tests launch and use a real daemon, so the shell → daemon test dependency itself remains.
- **Code anchor = `// ADR-0273`** — the `[[bin]]` line in the new package's `Cargo.toml`.
