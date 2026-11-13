# Logging conventions (engram-dashboard)

**Status:** codifies the de-facto practice of the crates (agent/daemon) and the shell. The masking **layer** is not a *goal* — it is an **explicitly rejected direction** (see Security (보안) below — ADR-0138. A caller explicitly invoking a helper does not fall under that rejection). This document is the single source when writing or reviewing load-bearing paths.

> This covers "what to log, when, and at which level" (convention). "How to turn it on and off" (infrastructure) is in `crates/engram-dashboard-base/src/logging/mod.rs`.

## Infrastructure (summary — the canonical source is the code)

- **There are two entry points.** `logging::init_logging()` = stdout only · `logging::init_logging_with_file(logs_dir, kind)` = stdout **+ file** (`logs_dir` = the log folder itself — the caller passes it as its own `DataLayout::logs_dir()`. Canonical = the daemon's `data_dir` · the shell holds a copy and a same-path test ties them together). Both run once at boot and are idempotent, and **whoever calls first wins**. The daemon (`crates/engram-dashboard-daemon/src/lib.rs`) and the app shell (`src-tauri/src/lib.rs`) use the latter.
- `set_log_level(level)` runtime toggle (`EnvFilter` reload). ★**There are zero callers outside base — there is not yet an entry to change it at run time**★ (checked against code 2026-10-03). The argument accepts not only a level word but a whole `EnvFilter` directive (「그룹」 below).
- Default level **warn** (a release is almost silent in normal use = default OFF). `RUST_LOG` takes precedence. Debugging = `RUST_LOG=debug`. **But `RUST_LOG` does not reach a release daemon** (it does not inherit the parent environment) — that is why the file sink exists.
- **File sink = `<logs_dir>/<종류>-<UTC>-<pid>.log`** (production = `<데이터 폴더>/logs/`) (kinds = `daemon` · `app`). **Synchronous write** (one event line = one write), no ANSI. ★**Do not introduce an async writer (`tracing_appender::non_blocking`)**★ — the daemon ends with `std::process::exit`, so lines left in the buffer, that is **the last line before a startup failure**, are lost. (ADR-0138)
- **The caller passes the log folder** — resolving the data folder is the job of the daemon's `data_dir` and the shell's `discovery` module (a copy), but both depend on `base` where logging lives, so if logging called it, it would form a cycle (the leaf crate invariant — ADR-0175 decision 1. The old rationale, 「코어 격리」 ADR-0003, points in the same direction). If the primary folder is unusable it falls back to `%TEMP%/engram-dashboard/logs/`, and if both fail it starts without a file sink (the owner of that case = the client precheck, ADR-0135).
- **Retention = the latest 10 per kind** (a cap that includes this run's file). Only **names that match the grammar** of `<종류>-YYYYMMDD-HHMMSS-<pid>.log` are cleanup targets — if this is changed to a prefix match, files of other kinds and files placed by hand also become candidates.
- **First line of the file = run header** (`==== engram <종류> | <UTC> | pid <n> | <exe 경로> ====`). It is not a log event but one line outside the event plane — the default level is `warn`, so a normal startup leaves not a single line and **without the header the file would be entirely empty**. Do not raise the startup notice level to fill that gap.
- Canonical: `crates/engram-dashboard-base/src/logging/mod.rs`. Decision · rejected alternatives = **ADR-0138**.

## Levels — what goes where (de-facto)

| Level | Criterion | Actual examples |
|---|---|---|
| **error!** | **Data at risk or unrecoverable** — a person must see it (even if isolation recovers it) | agents.json · presets.json load — corrupt → empty list (copied aside just before the first save) · new edition · read failure → empty list + read-only for that whole run (agent `persistence/mod.rs` load · ADR-0291), serialization failure, panic (`daemon/lib.rs:91`, reaper isolation recovery `reaper.rs:196`), instance guard · data_dir failure |
| **warn!** | **Abnormal but falls back safely** (no data risk) | resume failure → fresh fallback (`manager.rs:324`), a corrupt agents.json copied aside to `.corrupt` just before the first save (a save of the same file), accept failure |
| **info!** | Normal lifecycle events (of interest to the operator). Default is warn so they are not seen normally, but turning it on shows the flow | agent spawn (`manager.rs:169`), restore start/result, daemon · thread start/stop, connection established (`Server::run_connection` of the daemon's `link/server.rs`) |
| **debug!** | Detailed flow · diagnostics. Only while debugging | connection accepted (`Server::run` of the daemon's `link/server.rs`), reaper/thread exit, trivial handle cleanup failure |
| **trace!** | Only ultra-high-frequency hot paths. Currently unused (0 cases) | (per-frame output chunks etc. — be careful when introducing) |

One-line reading: **data at risk/unrecoverable (error) → odd but safe fallback (warn) → normal but worth tracing (info) → internal detail (debug).** (agents.json is error for both corruption and read failure — corruption is a signal that the registry is lost, and a read failure blocks saving for that whole run · if the copy-aside afterwards succeeds it is warn — a branching example.)

## Format

- **Message = one short line in English, plain and simple so anyone can read it** (user 2026-11-13 — 「대신에 서술이 아니라 누구나 알아볼 수 있게 심플하게 구성해야겠지.」 · earlier the same day 「로그 메세지는 AI체로 하지 말아야되는거 알지?」): what happened and what the code did. Code names a developer greps for (function · type · constant · reason code) over descriptions; no narrative or explanatory prose, no literal translation of internal concepts.
  - Example: `작업 폴더 canonicalize 실패, 프로필 경로로 진행` → `cwd canonicalize failed; using profile path`.
- **Logs are English and are not localized** (user 2026-11-13 — 「로그는 그냥 영어.」, the answer to whether log text gets translated). A new or reworded log line is written in English now (the convention change — user 2026-11-13 「규약을 바꿔. … 어쨋든 지시서 바꿔 나중에 이부분 영어로 한꺼번에 변경은 할거야.」). The existing Korean log lines stay until one batch converts them (tracking T-108).
- **Identifiers and numbers go into structured fields (key=val)** — they become filter and search keys, so do not interpolate them into the message. `%`=Display, `?`=Debug.
  - Example: `tracing::info!(agent = %profile.id, epoch, ?mode, "agent spawned");`
- **Error detail (`: {e}`) may be interpolated at the end of the message** (de-facto) — "no interpolation" applies only to *identifiers and numbers* (agent · epoch · pid · conn), and `{e}` is the exception.
- **The `[component]` prefix** (`[tray]`/`[layout]`) is a practice of **only some src-tauri modules**, not a crate-wide rule. New code should prefer a **field** (`module=`) over a prefix (unification is undecided).
- **span/`#[instrument]` unused** (currently 0 cases, flat event). If introduced, update this document.

## Groups (그룹) — the unit to turn on and off (target)

Logs are not wrapped — each crate calls `tracing` directly, and the filtering is done by the `EnvFilter` that base installed (ADR-0268). A group is the **target** that filter looks at.

- **The default group = the module path of the call site** (`tracing` attaches it automatically). There is nothing to do for turning on and off per crate · module — `RUST_LOG=engram_dashboard_daemon::link=debug` · `RUST_LOG=warn,engram_dashboard_agent::backend::codex=debug`.
- **Attach `target: "<묶음>"` only when you need a grouping different from the module boundary** — when logs scattered over several modules · crates must be turned on together, or when only those logs inside one module must be turned on separately. Do not attach it without a reason.
  - ★**Once you attach a target, that log drops out of the module path filter**★ — even if you turn on the crate with `engram_dashboard_agent=debug`, the `target: "agent_stderr"` line is not turned on. Turn it on separately by the attached name (`agent_stderr=debug`).
  - Name = lowercase snake_case · 「무엇의 묶음인지」 without a crate prefix (example: `agent_stderr` = agent process stderr).
  - ★**Make sure group names are not prefixes of each other**★ — `EnvFilter` matches a target by string prefix (tracing-subscriber 0.3 `filter/env/directive.rs` — `starts_with`). `mail=debug` also turns on `mail_retry`.
  - Use the target, not a field (`module=` and the like), as the unit to turn on and off. A field directive with only the name (`[{field}]`) also filters the events that carry that field — only value matching (`[{field=값}]`) is span-only. Using the target anyway is because the directive is shorter and on the same axis as the module default.
- **Do not write the list here** — how to find it = `rg -U 'target:\s*"' -g '*.rs' crates/ src-tauri/` (`-U` = also catches a line-wrapped notation).
- **Not there yet:** the grouping vocabulary (a category system) and the entry to change at run time (the LLM control surface) are decided by the logging system design (`docs/refactoring/architecture-discussion-2026-09-26.md` big section A). Until then, new targets grow only when needed, by the rules above.

## Instrumentation duty (load-bearing paths — no instrumentation is a defect)

The following **must** leave a log at an appropriate level (do not silently drop a failure with `let _ =` — at least debug):

- **Connection/session lifetime:** establishment · failure · reconnect · end.
- **Concurrency transitions:** state changes, guard firing (discarding a stale generation · restart, etc.).
- **External boundaries:** spawn, file IO, network accept/close.

> The shell's daemon connection is instrumented to this duty (`src-tauri/src/daemon_client/{connection.rs, link/route.rs}` — S21 3-2): link up/down and state transitions (info), connect/handshake failures and refusals (warn, throttled per handle), stale items of a replaced handle (debug). The transport crate does not log (it reports events to its owner). Example of a connection/auth flow: `crates/engram-dashboard-daemon/src/link/{server,auth}.rs`.

## Security (보안)

- **Do not log tokens · credentials · passwords in plaintext.** Do not put them in errors either — measured: the daemon's `link/auth.rs` (`AuthCheck::check_text` — the token mismatch branch, the `constant_time_eq` comparison part) leaves the token value out of the log, and the `daemon_client` connection failure error carries only the url and excludes the token.
- **★A masking · redaction layer for logs was rejected (ADR-0138)★** — the token is already in **the same folder in plaintext** (the connection key file). Hiding a copy of it protects nothing. **The discipline adopted instead = do not put credentials into the sentences we write** (the first item above is that discipline). ★The `Secret` type in protocol does not reverse this rejection★ — it is not a layer that filters log output but an implementation of this discipline, where the type itself hides the value so that our sentences cannot print it.
- **The `mask_secrets` helper is not applied automatically; the caller invokes it explicitly** (`base/src/logging/mod.rs`). The callers are the places that put external process (claude · codex — including usage queries) output into a sentence (the places that put UI settings file values were withdrawn in P3d together with the `ui_settings.rs` that read that file · how to find = `rg "mask_secrets\(" -g '*.rs' crates/ src-tauri/` — the definition line, comments and test strings are caught too, so to count only calls, exclude them). If you newly log potentially sensitive output such as PTY text, **the caller must apply it explicitly** (it does not "go through" automatically). The file sink does not go through this helper.
- **★If you truncate, masking comes before truncation★** — if you truncate first, a credential that straddles the boundary is cut below the pattern's length quantifier and is logged **having slipped past masking** (`sanitize` of `agent/src/backend/codex/{decoder,transport}.rs` is fixed in that order). Truncation is a length defence and it stands as is even when applied after masking. ★**But this does not mean 「마스킹 먼저」 is safe for every input**★ — some inputs are such that masking shortens the string and pulls trailing bytes up into the leading window, so the old `src-tauri/src/ui_settings.rs` put in its place a gate that does not truncate but **filters by the shape of the value** (`describe_value` — ★that file was deleted in P3d — no living instance of the same technique was found in the shell's settings · state · theme code (2026-10-07)★ · to read it as an example, use `git show de65cae:src-tauri/src/ui_settings.rs` · decision = ADR-0166 decision 8, partly discarded by ADR-0265).

## Anti-patterns

- Swallowing a failure with no log (silence after `let _ = x;`) — at least debug.
- Interpolating identifiers · numbers into the message string (`format!`) — move them into fields (error `{e}` is the exception).
- Logging tokens · sensitive information in plaintext.
- Indiscriminate info/debug on hot paths — consider trace or sampling.
