# ADR-0138: Write the release log to a file synchronously — stdout goes nowhere

- 상태: 확정 (2026-08-16, grounds: measured + `tracing_appender` crate check)
- 관련: ADR-0003(core isolation) · ADR-0134 decision 4(no data folder fallback) · ADR-0135(client pre-check) · `crates/engram-dashboard-core/src/logging/mod.rs` · `crates/engram-dashboard-daemon/src/lib.rs:386` · `src-tauri/src/lib.rs:67` · `docs/reference/logging-conventions.md` · step-log S21

## 맥락

**In a release build the daemon's stdout reaches no one** (measured). There are three layers:

1. The release binary is `windows_subsystem = "windows"`, so it has no console.
2. The daemon is started through WMI `Win32_Process.Create`, and that API **cannot pass std handles** (the non-WMI path specifies null outright).
3. The daemon does not inherit the parent environment, so **`RUST_LOG` never reaches it**.

So all the user saw was one banner, `데몬 시작 대기 timeout(5s 초과)` (daemon start wait timeout, over 5s), and **there was not a single place where the reason was kept.** This is exactly why the earlier session record ruled out "check backend behavior through logs."

## 결정

1. **The daemon and the app shell each write `<데이터 폴더>/logs/<종류>-<UTC>-<pid>.log`.**
2. **Synchronous writes.** Direct `File` writes — one event line is one write.
3. **Retention = the latest 10 per kind** (the cap includes this run's file).
4. **Human-readable text, one event per line.** No ANSI color (people and LLMs read it as is).
5. **The caller passes the log folder — the core does not look for it itself.**

## 거부한 대안

- **★The non-blocking writer of `tracing_appender`★** — its worker is **asynchronous**, but the daemon ends with `std::process::exit` and skips destructors. Lines left in the buffer vanish, and **the vanishing lines are exactly the last lines before the startup failure we want to catch.** Confirmed in the crate: default buffer 128,000 lines · lossy by default · send timeout of 100ms/1000ms at shutdown. Grounds for rejection = crate source and docs check + the daemon exit path (code).
- **Show a native popup instead of a log** — **the user chose the log first**: what the LLM operator reads is the log, and a popup can be added later if it becomes necessary. Grounds for rejection = user decision (2026-08-16).
- **Add a masking/redaction layer for the log** — **rejected on purpose.** The token is already **in the same folder in plaintext** (the connection key file). Masking its copy protects nothing. The discipline adopted instead = **do not put credentials into sentences we write.** Grounds for rejection = logic (duplicate defense inside the same trust boundary).
- **Structured JSON output** — deferred. What is needed now is for a person or an LLM to read "why did the daemon die." Grounds for rejection = scope (the need does not exist yet).
- **Let the core resolve the data folder itself** — the core would depend on `discovery`, which **breaks "core isolation" (ADR-0003)** (「코어 격리」). What the core owns is only the **layout convention under** that folder (`logs/<종류>-<UTC>-<pid>.log`). Grounds for rejection = architecture invariant + isolation gate.

## 근거

- Measured: when the daemon failed in release, the only information left to the user was one banner phrase (zero cause).
- Rejecting `tracing_appender` is **a check, not an assumption** — the buffer size, the lossy default and the shutdown timeouts were read from the crate, and the code says that the daemon's `std::process::exit` exit skips destructors.
- The UTC timestamp in the file name has **a fixed digit count, so lexical order = time order**, and retention cleanup leans on that property.

## 영향 / 불변식

- **★Do not go back to an asynchronous writer★** — the rejection reasons above come back as they were (anchor: `init_subscriber` in `logging/mod.rs`).
- **If the primary folder cannot be used, it falls back to `%TEMP%/engram-dashboard/logs/`.** Otherwise this feature **fails at its own purpose** — being unable to use the data folder is a common reason for daemon startup failure, and precisely then the reason would go to stdout (which release does not have). This fallback does not violate ADR-0134 decision 4 — what that decision blocks is **state** (registry · lock) appearing in two places, and what is moved here is one stateless sheet of reason.
- **★If both the primary and the fallback fail, no sink is installed — this failure path must be handled explicitly★.** Otherwise **it disappears silently.** The owner for this case is the data folder pre-check that runs before the client starts the daemon (ADR-0135), and the user sees the reason in that window.
- **Reasons that arise before the subscriber is installed are collected and emitted all at once right after installation** (retention deletion failure · header write failure · fallback use). Until then there is nowhere to emit them.
- **The file is opened only after winning the subscriber installation.** If the order is reversed, the losing call also creates a file and even runs retention cleanup, and an empty file nobody will write to **deletes one real log** and permanently takes its place.
- **★Known residual 1 — name collision★:** If a pid is reused within the same UTC second or the clock goes backward, the names collide, and because of append mode **two runs continue in one file.** The header is printed again each time, so the boundary stays readable. Preventing it needs exclusive creation plus a retry loop, and the cost exceeds the gain.
- **★Known residual 2 — panic hook self-deadlock★:** The panic hook runs **before** unwinding, **on the thread that panicked** (daemon `install_panic_hook`). If that thread was holding the sink lock, the hook's `error!` locks the same `Mutex` again and deadlocks itself (the process hangs — worse than abort). The window exists only when it panics inside `write_all`, and it has never been measured. Eliminating it needs lock-free writes (interleaved lines) or a hook-only bypass, and the cost is higher. If it is touched, touch **the hook side**, not the sink side.
- **The sink ignores poison and keeps writing** — the standard implementation panics on poison, and then the hook's log panics again (panic-in-panic → abort) and **the very reason for the death that should be left disappears.**
- **`LogKind` is a closed set, not a free string** — if two processes pick the same prefix they share one file and lines interleave. When a new process appears, a variant is added so that a different name is chosen **consciously**.
- **Retention cleanup selects by file name grammar, not by prefix match** — going back to prefix breaks two things: if a label is a prefix of another label (`app` vs `app-updater`), cleaning one **deletes the other kind**, and a hand-placed name settling at the end of the lexical order **pushes a real log out beyond the cap.**
- **The header is the first line of the file, not a log event** — the default level is `warn`, so a normal startup leaves not a single line. Then what remains is an empty file, and you cannot even tell which binary started when. Raising the startup notice from `info` to `warn` to fill this gap would collapse the meaning of the levels.
- **No date crate is brought in** — the core's dependencies are not worth growing for one file name (the timestamp of the log **line** is already printed by tracing-subscriber).
