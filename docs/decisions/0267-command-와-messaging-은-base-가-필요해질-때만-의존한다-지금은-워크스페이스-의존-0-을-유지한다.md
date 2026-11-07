# ADR-0267: command and messaging depend on base only when they need it — for now they keep zero workspace dependencies

- 상태: 확정 (rewritten 2026-10-03 · replaces the first version `ebfdafc` that was committed but not pushed (2026-10-02 · 「command 와 messaging 도 base 를 의존한다」 — it opens the dependency · the first item of 「거부한 대안」 below), basis: user decision 2026-10-03 (「필요할 때만 연결하면 되지」 — withdraws decision candidate 2 of `docs/refactoring/architecture-discussion-2026-09-26.md`) + re-measurement of the helper share 2026-10-03 + log-state measurement master `a226f63`)
- 관련: ADR-0155 (decision 2's zero workspace dependencies for command — unchanged) · ADR-0110 (decision 2's no workspace dependency for messaging and the import-0 gate in its impact — unchanged) · ADR-0175 (the dependency graph in its impact — command and messaging remain leaves) · ADR-0151 (command's reason to exist = it can be used independently and prevents cycles — unchanged) · ADR-0268 (each crate calls `tracing` directly for logs) · ADR-0269 (base generic helpers) · `crates/engram-dashboard-base/src/lib.rs` (residency conditions · gates) · `.github/workflows/ci.yml` (the messaging · command dependency ceilings · the messaging isolation regex) · CLAUDE.md 「빌드·검증 명령」 · step-log S21

## 맥락

`command` (ADR-0155 decision 2) and `messaging` (ADR-0110 decision 2) have 「워크스페이스 의존 0」 as a property, and the CI dependency-ceiling gate guards it (expected value = 1 line, itself · CLAUDE.md 「빌드·검증 명령」 · `ci.yml`). messaging also has a source-isolation regex (`rg "engram_dashboard_(agent|base|daemon|protocol|discovery|command)" crates/engram-dashboard-messaging/src/` → 0 lines), and `base` is in its forbidden list.

The first version of this ADR (2026-10-02 · `ebfdafc`) opened that property so that the two crates depend on base. The reason was logging — the user (2026-09-26): 「커맨드·메시지는 로깅 안 함? 당연히 해야지」. That reason disappeared with the ADR-0268 rewrite (2026-10-03 — logs are not wrapped and each crate calls `tracing` directly). **Printing** logs does not need base — `messaging` already depends directly on `tracing` and prints from about 34 places (`service.rs` and others), and the printed logs reach the file through the subscriber that the process (the daemon) installed with base. `command` has 0 log lines (and no `tracing` dependency — measured at master `a226f63`).

The reason was then replaced with base's generic helpers (ADR-0269) (main session's judgment 2026-10-03 · `56cb265`), but the share the two crates would receive now is small (re-measured 2026-10-03):

- **Lock-poison recovery** — there are 0 places in command's production code. There are 3 `PoisonError::into_inner` places only in the harness module `src/testing.rs` (behind the `test-support` feature flag). messaging has 0.
- **XML escaping** — the consumer is messaging alone (`escape_xml_attr` · `escape_xml_text` in `envelope.rs`).
- **Logging** — base is not needed (above · ADR-0268).

The user (2026-10-03): 「필요할 때만 연결하면 되지」.

## 결정

1. **`command` and `messaging` do not depend on base for now — they keep zero workspace dependencies** (user 2026-10-03). The two crates' dependency-ceiling gates (expected value = 1 line, itself) and the messaging source-isolation regex (`base` in the forbidden list) stay as they are (CLAUDE.md 「빌드·검증 명령」 · `ci.yml`).
2. **When a base helper is actually needed, connect it then** (user 2026-10-03). The change that connects it states that need as its basis (which helper is used where) and amends this ADR. The gates to change in that same change are in 「영향」 below.
3. **Logging is unrelated to this decision** — if `command` takes on logging, it depends directly only on `tracing` (ADR-0268). Workspace dependencies do not grow.

## 거부한 대안

- **Open the base dependency now (set the two crates' dependency ceiling to 「자기 자신 + base」).** ★This is the first version of this ADR★ (written 2026-10-02 · memo decision candidate 2 · not pushed — this rewrite replaces that version). Rejected = user decision (2026-10-03 — 「필요할 때만 연결하면 되지」). Reasons:
  - **The first version's reason (logging) is gone** — logs are printed with `tracing` without base (the ADR-0268 rewrite).
  - **The replacement reason (generic helpers) has a small share to receive now** — lock-poison recovery 0 places in command's production code (only 3 in the harness) · 0 in messaging · XML escaping has one consumer (「맥락」 above · re-measured 2026-10-03).

## 근거

- **User decision 2026-10-03** — the quote above.
- **Re-measurement 2026-10-03** — the three items in 「맥락」 above (lock-poison recovery · XML escaping · logging). The log state (messaging about 34 places · command 0 lines) is measured at master `a226f63`.
- **The shape of connecting when it becomes needed is itself not a problem** — it is not the basis of the decision but context that there is no reason to block connecting later.
  - Having most crates depend on a shared base crate is a common form in large workspaces — rust-analyzer `hir` · `vfs` → `stdx`, zed `editor` · `rope` → `util`, Materialize `adapter` · `persist` → `mz-ore`, TiKV `raftstore` → `tikv_util`, Firecracker `vmm` → `utils`, nushell `nu-protocol` → `nu-utils` (each `Cargo.toml` checked 2026-10-03).
  - base's own gates (dependency ceiling · `use tauri` 0 lines · no cross-references between residents) guard zero domain knowledge and zero workspace dependencies (`crates/engram-dashboard-base/src/lib.rs` header · CLAUDE.md 「빌드·검증 명령」). So even if the two crates lean on base, no other workspace crate gets dragged in through base.

## 영향 / 불변식

- **Invariant (unchanged): the workspace dependencies of `command` · `messaging` = 0.** The gates do not change — the two dependency ceilings (→ exactly 1 line = itself) and the messaging source-isolation regex (→ 0 lines) guard it (`ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md`).
- **ADR-0155 · ADR-0110 · ADR-0175 · ADR-0151 are not amended** — the partial-discard stamps and amendment links that the first version stamped onto three of them (0155 · 0110 · 0175) were removed (2026-10-03). The first version did not amend 0151 either (it only noted that one sentence in its impact goes stale). command and messaging remain leaves in the dependency graph of ADR-0175's impact.
- **What to change in the same change when connecting** (the list the first version wrote down — amend this ADR then):
  - The expected value of the two dependency ceilings → exactly 2 lines (itself + base). Places = `ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md`.
  - ★**Remove `base` from the forbidden list of the messaging source-isolation regex**★ — if left, it turns red the moment base starts being used (three places: `ci.yml` · CLAUDE.md · `qa.md`).
  - The command · messaging items of CLAUDE.md 「백엔드 모듈 맵」 (「워크스페이스 crate 의존 0」 · 「워크스페이스 crate 무의존(컴파일러 강제 벽)」).
- **The two crates' share drops out of ADR-0269's helper absorption** — messaging's XML escaping and the 3 lock-poison-recovery places in command's harness are not moved in work order 1-1 and stay where they are (ADR-0269 impact).
- **No code anchor is placed** — no code changes.
