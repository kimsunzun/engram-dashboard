# Architecture discussion — notes by module

**Purpose:** decide the whole dependency picture and the module split before attaching transport (the order the user set, 2026-09-26). The user asks module by module in order, and the facts and decision candidates that come out of the answers are accumulated here.

- **Reference code:** master `a226f63`. Every fact item was checked against the code of that point in time.
- **Decisions do not end here.** A candidate whose direction is set is recorded as an ADR after the discussion is over and linked here (`../decisions/`). The facts waiting to start are carried by `../tracking.md` T-47.
- No diagrams are drawn (user decision 2026-09-26 — the `../reference/architecture-map.html` made earlier is left only as a draft).

---

## 1. base

### Facts

- **There are only two tenants** — `logging` · `platform` (`crates/engram-dashboard-base/src/lib.rs:53-54`).
- **`logging`** — global log initialization · a per-run file log · changing the log level while running · masking secrets in logs (`mask_secrets`). Initialization is called by the daemon · shell, and masking by agent · shell.
- **`platform`** — the 「이 PID 가 아직 그 프로세스인가」 decision. Since the OS reuses PIDs, identification is by the pair **PID + process creation time**. The child PID list is also here. There are two uses.
  - **Daemon duplicate prevention · discovery** — the daemon writes its own PID · start time into `daemon.json` (`crates/engram-dashboard-daemon/src/lib.rs:698`), and the one that comes later uses those two to tell whether it is a live daemon or a leftover file of a dead one (`crates/engram-dashboard-net/src/portfile.rs:86` · `crates/engram-dashboard-discovery/src/lib.rs:1037`).
  - **Recovering the codex session id** — whether the owner of the lock file is alive (`crates/engram-dashboard-agent/src/backend/codex/thread_lock.rs:279`), the process tree under the agent PID, matching the processes that have the file open.
- **Outside Windows it is a placeholder** — it answers alive as long as the PID is not 0 (`crates/engram-dashboard-base/src/platform.rs:201-228`).
- **Design intent = a leaf that any crate can use.** But today `command`·`messaging` are blocked by their own gate (workspace dependencies 0) and cannot use base either → ~~candidate 2 below solves it~~ ★candidate 2 withdrawn (user 2026-10-03) — connect only when it becomes necessary · ADR-0267★.
- **agent also has a separate `platform` module** — three things: a Job Object wrapper · finding processes that have a file open (`file_holders`) · a process tree (`process_tree`) (`crates/engram-dashboard-agent/src/platform/mod.rs:1-2`). The 「남은 `platform`은 Job Object 래퍼 하나뿐」 in the agent entry of CLAUDE.md 「백엔드 모듈 맵」 is a stale description (it is replaced along with that sentence when candidate 1 below lands).

### Decision candidate 1 — split the OS-dependent code into an independent `platform` crate

**Status: direction confirmed (user 2026-09-26) · ADR-0266 · start after the architecture discussion.**

- **What:** gather the code that varies by OS into one independent crate with zero workspace dependencies. Callers call only the interface, and the OS branching (`#[cfg]`) lives only inside that crate. Places already connected to OS code are all moved this way too.
- **User's rationale:** 「인터페이스 호출로 하고 플랫폼적인 건 다 감추고」 · 「어차피 플랫폼 전용 기능은 의존 0」 · 「애초에 초반부터 잡고 갔어야 됐어」.
- **What to move (measured current state — excluding test-only branches):**

  | What | Where it is now |
  |---|---|
  | Windows API wrappers — PID liveness · creation time · child PIDs · Job Object · finding processes that have a file open · process tree · launching outside the terminal tree via WMI | `base/src/platform.rs` · `agent/src/platform/` · `discovery/src/lib.rs:1119-1320` (`wmi_spawn`·`wmi_create_raw`) → measured correction `1060-1323` (starting from the COM classification) |
  | OS rules — attaching `.exe` · default shell · home directory · wrapping the CLI in `cmd.exe /c` · `PATH` case-insensitivity | `discovery/src/lib.rs:981` · `daemon/src/lib.rs:96,146` · `agent/src/manager.rs:75` · `agent/src/backend/claude/mod.rs:1012` · `agent/src/backend/mod.rs:44,136` → measured correction `discovery:987` · `daemon:109` **one** (CLI — there was no place that branched on the app name) · `manager.rs:77` · `claude/mod.rs:1829` · `backend/mod.rs:46-60, 138` |
  | Branches leaked to callers — carrying the Job Object handle as `#[cfg(windows)]` | `agent/src/transport/pty.rs` · `agent/src/transport/stdio.rs` · `agent/src/backend/codex/transport.rs:177` → measured correction `:183` |

  ~~The shell (`src-tauri/src`) has no OS branching.~~ → **Wrong**: shell `src-tauri/src/fsutil.rs:107` has an operational branch for atomic writes (`cfg!(windows)`) — a time-limited exception that U-W moves after storage P3 lands (ADR-0275 decision 15). ★→ A U1 closed it (2026-10-10) — the lock decision went to platform `fs::is_busy` and `fsutil.rs` was moved to base `file` in U2 and disappeared · zero operational OS branches outside platform (ADR-0291)★. The PTY itself is already hidden by `portable-pty`.

  ★The line-number corrections above are those of TRD 1-3 §2-5 (measured `0ef6292`), and the places in the table were all moved to platform in 1-3 U0~U7, so neither set of numbers points to them anymore★ — the place now = each platform module header · the list of files that still contain OS `cfg` = the `platform gate 4` step of `ci.yml` (1-3 U8).
- **What to leave:** domain knowledge — e.g. for the claude config folder, move only 「홈 디렉터리 찾기」 and keep the `.claude` path knowledge in the backend (CLAUDE.md 「백엔드 확장」). Branches that run tests only on Windows stay on the test side.
- **Interface shape (main's judgment — the user delegated):** functions + handle types, compile-time branching. No trait objects — the OS does not change while running, so runtime branching gains nothing. A Job Object is wrapped as a single 「프로세스 그룹」 handle, which removes the branching in pty · stdio · codex transport. Where a fake for tests is needed, **the using side** places a small trait (discovery's injection seam is the precedent).
- **Old decisions overturned:** the first rejected alternative of ADR-0175 decision 1 (platform as an independent crate) · ADR-0218 decision 11 (`file_holders`·`process_tree` go inside agent because there is one consumer). Both used the **number of consumers** as the criterion, and this candidate changes the criterion to **whether it is OS-dependent**.
- **Open items:**
  - ~~After the move, base keeps only `logging` — whether to leave base as is.~~ → **Closed: `logging` stays in base as is. The role of base = a leaf used from anywhere** (user 2026-09-26). The survey of new tenant candidates is in 「base 입주 후보」 below.
  - Whether to split the `windows` crate feature into per-function cargo features — `net` needs only the PID decision, yet the Job Object · Restart Manager bindings get pulled in too.
  - Implementations for other OSes do not come from this split. The empty places merely become visible in one spot.

### base tenant candidates (base 입주 후보) (survey 2026-09-26)

**Criteria:** zero domain knowledge · tenants do not reference each other. Looks at **generality** rather than the number of consumers — 「base 는 어디서든 쓰는 잎」 (user 2026-09-26). OS-specific code goes to candidate 1, so it is excluded.
**Verification:** a light worker surveyed and main checked only the definition locations. ★The behavior differences and the descriptions of private gates are worker reports and main did not re-confirm them★.

| Candidate | What it does | Where it is now | Decision |
|---|---|---|---|
| Atomic file write | write to a temporary file and rename — never leaves a half-written file | public function `src-tauri/src/ui_settings.rs:687` · two hand-made copies in agent (`persistence/mod.rs` · `persistence/presets.rs`) | **The strongest.** But the three copies are reported to behave differently (temp file name · cleanup on failure · folder fsync) — the behavior to merge into one must be decided first |
| Moving aside corrupt files | move an unreadable file to `<이름>.corrupt-<ms>` | agent `persistence/mod.rs` · `persistence/presets.rs` (two copies in the same crate) | general · it is natural as one bundle with the atomic write above (「JSON 파일 저장」 helper) |
| Cutting at a UTF-8 boundary | cut to N bytes without splitting a character | `transport/src/ws.rs:147` · `daemon/src/experiment/record.rs:178` · agent `output_core.rs:716` (a variant that cuts the front) | goes in · it is 5 lines, so the value is small |
| Path notation cleanup (`normalize_cwd`) | remove one pair of wrapping quotes · `\`→`/`. Does not look at the file system | `agent/src/commands.rs:763` → used by the shell | can go in · the contract is written as 「사람·LLM 이 친 cwd」, so it has to move **with a general name** |
| Current time (epoch ms) | wall-clock milliseconds | three copies inside agent (`profile.rs:20` · `persistence/mod.rs` · `persistence/presets.rs`) | general · merging them into one inside agent is enough |
| XML escape | `&`·`<`·`>`·quote substitution | `messaging/src/envelope.rs:126,135` | general (the function itself has no mail knowledge) · one consumer. ~~For `messaging` to use base, candidate 2 below (relaxing the gate) must come first~~ ★→ not moved now — candidate 2 withdrawn, messaging depends on base only when it becomes necessary (ADR-0267)★ |
| Bytes → hex string | lowercase hex encoding loop | three places in daemon (`experiment/record.rs:189` · `lib.rs:65` · `control/mod.rs:141`) | only the encoding loop is general · the token generator itself must not be shared (ADR-0086) · `sha256_hex` is excluded because it pulls in `sha2` |
| Cutting by character count (`clip`·`truncate`) | only the first N characters | agent codex `decoder.rs` · `transport.rs`, two inline places in daemon | **No** — on the agent side this is an intended private function that a test forces to be called only through secret masking. Pulling it out to base would open that door |
| Finding the data folder (`default_data_dir`) | decide the folder by `ENGRAM_DATA_DIR` · `.engram-data` · release layout | `discovery/src/lib.rs:84` | **No** — it is Engram-specific knowledge (ADR-0134), so it is discovery's part |
| Token generation | 32-byte random hex | two copies in daemon | **No** — the ban on sharing is explicit (ADR-0086) |
| Reconnect backoff | exponential wait + cap | `src-tauri/src/daemon_client/connection.rs:87` · `transport/src/policy.rs:81` (the richer variant) | **Not base** — it is a policy transport already has. When the shell attaches transport and absorbs it into that policy, it disappears (check at attach time) |
| `dunce::canonicalize` | normalization without the UNC prefix | direct library call | nothing to extract (a one-line call) |

- **Not seen:** whether helpers like 「비어 있지 않은 환경변수 읽기」 are duplicated across several crates was not checked (most look like `ENGRAM_*` domain reads).
- **Side fact:** the `transport` crate still has 0 consumers (only its own tests use it — manifest·source cross-check).

### Decision candidate 4 — move four general helper modules into base

**Correction (2026-10-03) — the original below is kept as a record:** ① atomic write (`write_atomic`) goes to platform, not to base `file` (ADR-0266 decision 8 — it carries the Windows-only rename retry) ② `Clock` is not 「같은 모양」 — the four copies have different shapes, and the merged shape is ADR-0269 decision 3 (for now only reading goes to base · waiting is the using side's own trait · `UsageClock` is on hold — if it fits cleanly at 1-1 it is merged, and if not the user is asked then, user 2026-10-03) ③ candidate 3, to which 「(후보 3 과 같은 결)」 on the lock-poison recovery line points, was reversed (ADR-0268 — no wrapping). That warning log calls `tracing::warn!` directly (ADR-0269 decision 4) ④ the XML escape of messaging and the lock-poison recovery at 3 places in the command harness (`testing.rs`) are not moved — candidate 2 withdrawn (ADR-0267 · ADR-0269 consequences). ⑤ 2026-10-04 — sync recovers without a warning · text keeps only hex_lower (cutting is std) · six Clock seams · UsageClock kept · transport has 3 stages (ADR-0275).

**Status: tenancy confirmed (user 2026-09-26 — 「파일 텍스트 시간 경로 등 다 옮겨. 딱 적당하네」) · the provision method is main's proposal · ADR-0269.**

- **Provision method (main's proposal):** stateless public functions. The call takes the form `engram_dashboard_base::file::write_atomic(…)`. Since there is no implementation to swap in, no trait is placed (the exception = `Clock` below).

  | Module | Functions (names are drafts) | What it absorbs |
  |---|---|---|
  | `file` | `write_atomic(path, bytes)` · `set_aside_corrupt(path, stamp_ms) -> 새 경로` | shell `ui_settings.rs:687` · agent `persistence/mod.rs` · `persistence/presets.rs` |
  | `text` | `truncate_bytes(s, max)` (keep the front) · `keep_tail_bytes(s, max)` (keep the back) · `hex_lower(bytes)` · `escape_xml_attr` / `escape_xml_text` | transport `ws.rs:147` · daemon `experiment/record.rs:178,189` · daemon `lib.rs:65` · `control/mod.rs:141` · agent `output_core.rs:716` · messaging `envelope.rs:126,135` |
  | `time` | `now_epoch_ms()` | agent `profile.rs:20` · `persistence/mod.rs` · `persistence/presets.rs` |
  | `path` | `normalize_spelling(raw)` (remove one pair of wrapping quotes · `\`→`/`) | agent `commands.rs:763` (→ shell) |

- **What to keep to:**
  - **Do not bring heavy dependencies into base** — JSON serialization stays at the call site (base does not pull in serde), and `file` takes only bytes. ★→ Changed in A: base took `serde_json` as an operational dependency — the read decision of `file` looks at a JSON document · struct serialization is still the owner's (ADR-0291 decision 3 amends ADR-0269 decision 6)★ hex is a direct loop (the `hex` crate is not used). `sha256_hex` is excluded because of `sha2`.
  - **Tenants do not reference each other (tenancy condition ③)** — the time in a corrupt file name is **received as an argument** by `file` instead of calling `time`. Add the new module names to the CI's tenant cross-reference gate (`rg "(crate|super)::(logging|platform)"`).
  - **Errors are `std::io::Result`** — no custom error type is made.
  - `file` is moved after the differences in behavior of the three copies (temp file name · cleanup on failure · folder fsync) are settled into one.
- **Additional candidates (measured 2026-09-26):**
  - **There are three copies of the `Clock` trait** — the same shape (`fn now(&self) -> Instant`) is defined separately by transport (`clock.rs:20`) · daemon (`command_delivery.rs:176`) · discovery (`lib.rs:425`). **After absorbing master (2026-10-02) a fourth appeared** — `UsageClock` of daemon `usage_service/clock.rs:9` (the real one + a hand-made fake). It is a seam for tests that swaps time for a fake → **confirmed: merge them into base `time`. Rule = 「시간은 공용이면 무조건 합친다」** (user 2026-09-26). It is the only exception to 「트레이트를 두지 않는다」 above.
  - **Test wait helper** — the general one is `wait_until(timeout, cond) -> bool` only, and **the same signature is copied into 9 files** (agent tests 7 · daemon tests 2 — e.g. `crates/engram-dashboard-agent/tests/reaper.rs:54` · `crates/engram-dashboard-daemon/tests/control_send.rs:70`). A few variants (`FnMut` variant · `poll_until` · in-module copies) also exist. All are synchronous functions, so tokio is not needed. The rest such as `wait_for_output`·`wait_for_text` know the domain and stay in place.
    → **Main's judgment: put it in base as a module behind the `testing` feature flag** (the fake `Clock` goes in the same place). It is not shipped in production builds and is turned on only in dev-dependencies. Since the general helper is a single function that uses only std, it is not worth setting up a separate dev-only crate.
  - **Lock-poison recovery is hand-repeated at 57 places** — 54 places of `.lock().unwrap_or_else(|e| e.into_inner())` + 3 places of the `PoisonError::into_inner` variant. agent has most of them, and daemon · shell · command have some too. There is no helper. → **Confirmed: gather them in a base `sync` module** (user 2026-09-26). Gathering them in one place allows control, such as **leaving a warning log when recovering** from poison that is now swallowed silently (the same lineage as candidate 3).
  - **Weak ones:** 「비어 있지 않은 환경변수 읽기」 — similarly shaped places are daemon 9 · agent 3 · protocol 1 · shell 1, but most are `ENGRAM_*` domain reads, so the part to pull out as a general helper is small. discovery's `retry_if_vanished` (`lib.rs:218`) is a discovery-specific policy.
- **Tenancy decision rule (user 2026-09-26):** 「기준을 미리 잡지 않고 지금 여러 군데 쓰는 걸로 판단」 — if it has no domain knowledge and **is used or copied in several places now**, main decides and moves it to base (reports it without asking the user). This rule replaces ADR-0175 tenancy condition ① (two or more consumer crates) — ② (zero domain knowledge · zero workspace dependencies) and ③ (tenants do not reference each other) stay as they are.
- **Later (user 2026-09-26):** update the architecture description in CLAUDE.md (or another place) to guide the main session to move any new general helper to base on its own.

### Decision candidate 2 — `command`·`messaging` also use base

**Withdrawn (user 2026-10-03 — 「필요할 때만 연결하면 되지」) → ADR-0267 rewritten: for now keep workspace dependencies at 0, and when a base helper is actually needed, that change states the need and connects it. The original below is kept as a record.**

**Status: direction confirmed (user 2026-09-26 — 「커맨드·메시지는 로깅 안 함? 당연히 해야지」) · ADR-0267.**

- **What changes:** the expected value of the dependency-cap gate for the two crates changes from 「자기 자신 1줄」 to 「자기 자신 + base」 (CLAUDE.md 「빌드·검증 명령」 · `ci.yml`). It overturns that part of ADR-0155 (command) · ADR-0110 (messaging) that wrote 「워크스페이스 의존 0」 as a property.
- **Current logging state (measured):** `messaging` already writes logs — it depends directly on `tracing` at about 34 places (`service.rs` etc.). **Printing** logs does not need base, and the printed logs go to the file through the subscriber that the process (daemon) installed via base. `command` has 0 log lines (and no `tracing` dependency either) → a target to add during the refactoring.
- **So what we gain by letting it use base:** printing logs via base (candidate 3 — ★reversed 2026-10-03, logs go directly through `tracing` · ADR-0268★) · the two crates can also use the tenant candidates above (general helpers).
- **Safety rationale:** base keeps zero domain knowledge · zero workspace dependencies through its own gate, so even if the two crates lean on base, neither a cycle nor a path for domain leakage arises.

### Decision candidate 3 — logs are printed only through base (no direct dependency on the logging library)

**Reversed (user 2026-10-03 — 「로그 감싸지 마」) → ADR-0268 rewritten: no wrapping. The original below is kept as a record.**

**Status: direction confirmed (user 2026-09-26 — 「로그 라이브러리를 왜 직접 쓰냐고. 나중에 컨트롤이 안 되잖아」) · the interface shape is main's proposal · ADR-0268.**

- **Current state (measured):** besides base there are six crates that depend directly on `tracing` — agent · daemon · discovery · messaging · net · shell. Log macro calls are about 530 places (daemon 193 · agent 129 · shell 125 · net 38 · messaging 34 · discovery 9 — a rough count including `tracing::` paths). `command` · `protocol` · `transport` are 0.
- **What is held centrally now:** the output settings — level filter · file · format are decided in one place, the subscriber that base installs.
- **What is not held:**
  - **Secret masking is not applied centrally.** The file writer writes the bytes it receives as they are (`crates/engram-dashboard-base/src/logging/mod.rs:135-141`). Masking is optional — each caller has to call `mask_secrets` itself — so if even one of the roughly 530 direct print places forgets, it leaks.
  - **Swapping the logging library or enforcing logging rules (field names · format)** becomes a job of fixing all six crates.
- **Main's proposal (interface shape):**
  - base provides the log macros **under its own path** (e.g. `engram_dashboard_base::log::warn!`). At first it thinly wraps `tracing` so call sites change only their import path. If the surface that call sites use (level macros + `키 = 값` fields) is kept small, then even if the library is swapped later, only base needs fixing.
  - **Only base is allowed a direct `tracing` dependency** — blocked by a CI gate (in the resolved dependency graph, direct `tracing` dependencies outside base = 0).
  - **Move masking into base's writer stage and apply it to every line by default** (optional → default).
- **Open items:** whether anywhere uses `tracing`-specific features such as spans (scope tracking) has not been looked at yet — if so, the surface to wrap grows. The cost of applying regex masking to every line was not measured (the default level is warn, so it is likely small).
- **The shape to build now (user 2026-10-02 — 「일단은 함수화하고 기본적인 인자만 넣도록」):** base provides the log entry point and takes only three arguments: **severity · category · message**. The category vocabulary is a free string until big section A (no checking). The message format follows the standard (Rust formatting convention) (user). Main's implementation judgment: at the call site it looks like a function but inside it is a macro — to record the call location (file:line) in the log and not to build the message string for a disabled severity.
- **The interface shape will be looked at again once 「큰 절 A」 below is decided** — because it is the place where category · severity go into the macro surface. What candidate 3 decides is only up to 「base 를 거친다」.


---

## 2. command

### Facts (light-worker survey 2026-10-02 · main cross-checked 4 key locations)

- **It is a command bus tool with 0 commands.** About 2,540 non-test lines — envelope (`envelope`) · error vocabulary (`error`) · transport hole (`link`) · declaration macro (`macros`) · registry (`roster`) · 3-stage delivery (`route`) · spec (`spec`) · command table (`table`) · argument type conversion (`coerce`) · test tools (`testing`, behind a flag).
- **The communication spec is separate.** `protocol` carries this crate's envelope · reply · declaration · owner marker in its own message fields (`protocol/src/messages.rs:4`). The actual transport is done by the daemon · shell connection code.
- **The request number is two types · one value.** Each exists in `command` and `protocol`, and the same UUID is carried over as is (`protocol/src/ids.rs:30-40`, ADR-0081 decision 4). The reason for two = `protocol` needs TS auto-generation (ts-rs) and the tool crate deliberately does not take it in.
- **Assembly is per process.** Daemon `command_delivery.rs` · shell `lib.rs:132` (+ `daemon_client/inbound.rs`) · frontend `src/commands/registry.ts`.
- **The daemon `command_roster.rs` is a wrapper (not a duplicate implementation)** — it puts connection liveness and per-connection send ports on top of the tool's registry. Because it knows `net`, it cannot be moved into the tool.
- **The daemon bypasses 1 of the 3 delivery stages** — it passes an empty table and uses only stages 2 · 3 (`daemon/src/command_delivery.rs:1833`), and stage 1 (running its own commands) runs separately through its own port. The shell is the only place where the three stages run as they are.

### Candidates (awaiting main's judgment — worker decision + rationale)

| What | Decision | Rationale |
|---|---|---|
| Splitting out the panic-net functions of `route.rs` | split (low) | `route` ↔ `table` call each other (`table.rs:339`). The same panic catching exists in three places |
| Publishing 「주인에게 넘기기」 (stages 2 · 3) separately from `route` | split (to review) | removes the bypass where the daemon passes an empty table. Needs checking against ADR-0155 decision 3 |
| Adding serialization to the read/write marker (`Effect`) | add | the shell duplicated the same thing as `ViewEffect` (`view_commands.rs:107`). The spelling is a wire decision |
| A catalog entry generator for specs made at runtime | add | the shell hand-wrote the same shape and error list (`view_commands.rs:573-633`) |
| 「재시도 안 함」 error constructor · already-finished reply helper | add (minor) | copies in daemon · shell |
| Splitting daemon `command_delivery.rs` (2.4k lines) into a folder | split (inside the daemon) | four jobs — request slot table · running the daemon's own commands · delivery · control-path bus. Handled in the daemon round |
| Two request-number types | keep | tied to the ts-rs decision |
| Moving the registry · slot table · argument-check bundle to the tool | keep | there is one place that uses it or the meaning differs · it would pull tokio into the tool |
| Frontend copies (spec shape · enum spelling matching) | keep (memo) | looked at together with `Effect` serialization in the frontend round |

- **Side fact:** the `Clock` of daemon `command_delivery.rs:176` is already in the time-interface list to be merged into base (candidate 4 above).

---

## 3. Crate composition — does the shell depend on agent

### Facts (code cross-check 2026-10-02)

- The shell uses **only three things** from the agent crate (not hosting): `COMMAND_SPECS` (`src-tauri/src/view_commands.rs:204` — reserved names the webview cannot take) · `llm_creation_refusal` (`src-tauri/src/layout/apply.rs:36` — backend policy filtering before slot spawn) · `normalize_cwd` (`src-tauri/src/layout/commands.rs:35`).
- **The daemon already blocks name collisions** — if a client registers a name that the daemon answers itself, the daemon refuses **the whole registration packet** with `CONFLICT` (`crates/engram-dashboard-daemon/src/connection_core.rs:1146`). The shell's reserved list is a second net that filters earlier than that, and it has a hole because it cannot see `mail.*`.
- **Shell commands and webview commands are registered as one packet** — if it is refused, this shell's commands become **0** until the next reconnect (the LLM cannot touch windows · tabs · slots). Today the shell leaves that refusal only as one `warn` log line (`src-tauri/src/daemon_client/connection.rs:1968-1975`).
- The canonical source of the backend policy decision is one place in agent, and the daemon's `agent.new` looks at the same table (ADR-0219). The shell side is a duplicate that filters earlier.

### Decision candidate 5 — cut the shell → agent dependency

**Status: direction confirmed (user 2026-10-02 — 「데몬에 거절만 하면 딱히 복잡하게 할 필요 없을 듯 … 실행해서 안 되면 다시 빌드하면 되니깐」) · ADR-0270.**

- Remove the reserved-name list. Put the wall in **the daemon's registration refusal alone**. No CI cross-check script is placed.
- **But the client shows the refusal as an OS message box** (user 2026-10-02 — 「빵 하고 뜨면서 클라에서 뜨면 유저도 알지」). The side that shows it = the shell (client). Since the window differs per platform, the OS branching is placed not in the shell but in the OS-dependent code place (candidate 1) or behind Tauri's dialog feature — chosen at implementation time. **No LLM notification is placed** — the cause is left in the log and appears if the agent digs through the log (user). Today's `warn` log line stays as is.
- **Principle (user 2026-10-02):** 「클라는 데몬이랑 아예 별도의 개념이라서 agent 를 모르는 게 좋음」 — the shell (client) does not depend on the agent crate.
- The backend policy filtering is left to the daemon. ★**But simply deleting the shell check opens a hole**★ — the only place that calls the policy function is the `agent.new` handling (`crates/engram-dashboard-agent/src/commands.rs:1077`), and the shell's slot spawn does not go through that command and sends the communication message `SpawnByCwd` directly (`src-tauri/src/layout/apply.rs:597`), and the daemon does not look at the policy on that path. → **Order: change slot spawn to the daemon's `agent.new` command path, then delete the shell check.** Slot spawn is in the order 「검사 → 데몬 스폰 → 배치」, so even if the daemon refuses, the layout does not change (`layout/apply.rs:585-620`). The refusal message makes one trip to the daemon, and the 「두 입구가 함께 열리나」 test of `tests/layout_apply.rs` and the two-entrance description of ADR-0219 are fixed. → The path was changed by ADR-0279 (policy wall = the daemon `SpawnByCwd` handler · the slot spawn path as is).
- `normalize_cwd` goes to base (candidate 4).
- What we gain: the shell executable does not carry the whole agent crate (PTY · Windows process management · command collection etc.). 「셸 = 데몬 클라이언트」 (ADR-0029) shows in the dependency graph too.

---

## 4. discovery

### Facts (code cross-check 2026-10-02)

- **The original reason (ADR-0024):** the daemon writes the port · token · PID into `daemon.json` of the data folder and the client reads the same file to connect · authenticate → both sides must use the **same folder rule**, so that rule (`default_data_dir`) was put in a shared crate, and the client-side finding · launching · stopping of the daemon was put in together.
- **What both sides actually use is only `default_data_dir`.** The writability check · install location are used only by the daemon (+ the `engram` CLI inside the daemon crate), and finding · launching · status · stopping · finding the executable are used only by the shell.
- **There is a small daemon client inside** — `send_stop` itself does the WebSocket connection · authentication · stop command. This is the whole reason discovery knows `net` (the authentication message) and the `protocol` command message.
- The executable-location calculation is split across two places — the daemon (`daemon/src/lib.rs:96,146`) · discovery (`lib.rs:981`). **→ Measured correction (TRD 2-2 §1-3): three places** — daemon `locate_send_exe` · discovery `locate_daemon_exe` · harness `roundtrip_smoke.rs` `sibling_send_exe`. The two old lines are stale. **Handled in 2-2 U1**: the three call platform `env::sibling_exe` in one place (ADR-0282).
- It was once closed in T-10 (2026-08-26) as 「discovery 를 없애지 않는다」. Candidate 6 below reopens it on **new grounds** (the client · daemon separation principle · preparing for remote).

### Decision candidate 6 — break discovery apart and place the parts in the daemon and the shell respectively

**Status: direction confirmed (user 2026-10-02) · ADR-0271 · T-10 reopened.**

- **User's rationale:** with a network mode only the daemon needs the folder rule · the same-PC mode can also use the **same authentication method** as remote, so there is no need to split the logic (convenience auto-auth later) · 「지금 당장은 discovery 뽀개고 데몬 셸 각각 두고 나중에 관련해서 얘기할 때 분리」.
- **How to split:**
  - To the daemon: the data folder rule (`default_data_dir` · `release_data_dir` · writability check) · install location (`find_install_root`) · executable-location calculation (two places → one).
  - To the shell: finding the daemon · launching · status · killing the process · finding the executable. The stop command (`send_stop`) goes to the shell's daemon connection code — then no small daemon client remains anywhere outside the shell.
  - The code that launches via WMI goes to `platform` (candidate 1).
- **One transitional link:** until a remote authentication path exists, the shell has to keep reading `daemon.json`, so the shell must also know **where the daemon writes that file**. Since the shell does not depend on the daemon crate (client · daemon separation), one more copy of the location rule appears on the shell side → the two copies are tied together by a test that measures whether they produce the same path (the shell already pulls in the daemon as a test-only dependency). When remote · same-PC authentication is unified, this link disappears.
- **Later (during remote work):** the client puts 「데몬 주소와 토큰을 어디서 얻나」 behind an interface, and the same PC is unified to the same authentication method · convenience auto-auth is considered. The token model stays as is: the server issues it and the client holds it as a key.

---

## 5. Cleanup principle — put the ambiguous in the daemon for now (user 2026-10-02)

- **Principle:** 「애매한 건 데몬으로 몬 다음에 위에 정리되고 다시 데몬 정리를 하는 방향」. For code whose place is ambiguous, do not set up a new crate or leave it shared, **put it inside the daemon for now.** After the boundaries (above) are all sorted out, split it again in the daemon round.
- **Order (main's judgment — the user delegated):** ① boundary refactoring (candidates 1·2·4·5·6 — but moving discovery's stop-command client goes to ②) → ② attach transport to the shell (together with the stop-command client · connection-info interface) → ③ attach transport to the daemon → ④ daemon cleanup.

## 6. transport · net

### Facts

- `transport` is an independent crate · zero workspace dependencies · 0 consumers. It has the connection machinery (message unit · request/response pairing · stream sequence · resume · hole finding · handshake · keepalive · write deadline · reconnect · N peers), and it does not know the message vocabulary (ADR-0177).
- ADR-0177 decision 1: attach by giving **each of the daemon and the shell** its own dedicated folder, and finally transport takes over the network role of `net`. Multiple daemons are just raising one more peer, so for now it is attached to the current structure (one daemon · one shell).
- `net` mixes three things — daemon-only server connection code · the authentication message types that the client also uses · daemon-only jobs (single instance · `daemon.json` writing · connection admission decision). It only barely prevents the client from coming to know the daemon-side crate, with a feature flag.

### Decision candidate 7 — after switching to transport, retire net

**Status: direction confirmed (user 2026-10-02) · ADR not written — at the transport TRD · start = step ③ above.**

- Common connection machinery → `transport` · common vocabulary (including the authentication message) → `protocol` · daemon-only jobs → **inside the daemon** (the principle of 5 above — looked at again in the daemon round).
- As a result there is almost no reason for `net` to remain as a crate. The shell no longer knows `net`.
- **Verification status:** what remains on the daemon side was estimated from ADR-0177 decisions 1·3·4 and the `net` current composition — it is before the attach TRD, so not confirmed.
- **Open question — whether to split authentication out (user 2026-10-06 「인증 쪽은 모듈이 따로 있어야 되지 않나」) · decided in the transport TRD.** Today authentication is mixed in three pieces in one place, `net`: ① the handshake frame **shape** (`net/src/auth.rs` `AuthFrame` — the shell uses it too) ② the work of **deciding** it (`net/src/ws.rs` — Origin · secret comparison · connection admission) ③ the work of **creating and distributing** the secret (the daemon writes it into `daemon.json` and the shell · CLI read and send it). The current placement in candidate 7 is ① → `protocol` · ②③ → inside the daemon, and `transport` does not know authentication (measured — zero authentication vocabulary in `transport/src/`). Main's opinion (undecided): rather than scattering ②③ inside the daemon, gather them in **one module (an authentication seam — one decision trait + the current implementation)**. Whether to split it out as a crate waits until remote preparation (a later track), when the client side also gets decision · credential logic — what the client does now is only 「파일에서 읽어 보내기」, so the crate's consumer would be the daemon alone.

---

## 7. protocol

### Facts

- `protocol` = the **packet list** that the shell and daemon exchange (+ encoding · protocol version · TS bindings for the frontend). Boundary = it leans on command, and the daemon · shell · frontend use it (discovery · net disappear through candidates 6 · 7).
- What it takes in and embeds from command = four command bus forms (envelope · reply · registration entry · owner marker — `crates/engram-dashboard-protocol/src/messages.rs:4`).
- There are already two roads that carry feature data — ① the command bus (the module decides the shape, and it is carried inside the JSON (「속」)) ② `protocol` copy types (agent list etc. — repacked at the tail end of the daemon).

### Decision candidate 8 — the packet definition does not embed other crates' structs

**Status: direction confirmed (user 2026-10-02 — 「패킷이 똥통이 되잖아」 · 「바닥 구조체가 아니라 장기적으론 모든 구조체를 품어야 되는 거잖아」) · ADR-0272 · start = when attaching transport.**

- **Principle:** `protocol` does not embed structs of other crates — the bottom tool (command) is no exception. Once it is allowed, every struct flows in whenever a feature grows.
- **Feature data:** `protocol` has communication copies and they are repacked at the shell · daemon tail end (the transport adapter).
- **Confirmed shape (user 2026-10-02 — 「다른 건 다 패킷 구조체 방식으로 하고 command 만 json 으로」):**
  - **Fixed packets** (agent list · status · subscription · profile etc.) = defined as types in `protocol` (Zed style · split into per-area modules as it grows). Conversion is at the daemon · shell tail end (agent does not know `protocol`).
  - **Command bus** = the four command fields of the packet (command · reply · registration · registration change) as `Box<RawValue>` — only the raw bytes are carried, and unpacking them into envelope structs is at the shell · daemon tail end. The same shape as the JSON-RPC envelope · Tauri commands (commands are an open set that keeps growing). → `protocol` → the command dependency disappears and the wire bytes stay as they are.
  - **Reply pairing (main's judgment):** the tail-end adapter unpacks envelopes · replies into structs as soon as it receives a packet, and the request number for pairing is read **from the unpacked struct** (natural since transport's `reply_tag` is inside the adapter). The number is not pulled out of the field, so the bytes · protocol version stay the same and there is no extra parsing. The pairing function that is inside `protocol` now (`messages.rs:1048`) moves to the adapter.
  - Cost: when unpacking the outer packet the envelope is scanned once for validation and unpacked once more at the tail end (no intermediate tree is made) — command packets are rare, so it is at a negligible level. Basis = `../research/wire-shared-types-placement-2026-10-02.md`.
- **To write anew:** the handling of the 「패킷은 풀렸는데 봉투가 깨진」 case · unpacking/repacking on the daemon relay path.
- **Basis survey:** `../research/wire-shared-types-placement-2026-10-02.md` — rust-analyzer (copies + `to_proto`/`from_proto`) · Cargo `cargo-util-schemas` (a shared schema with no behavior) · the JSON-RPC envelope (opaque field) have this shape, and there is no primary source recommending the shape where the packet crate embeds others' types.

---

## 8. Daemon boundary · engram CLI

### Facts (code cross-check 2026-10-02)

- `engram.exe` (the control CLI that an agent calls from its own terminal) is the **second bin target of the daemon package** (`crates/engram-dashboard-daemon/Cargo.toml:37-39` · source `src/bin/engram.rs` 2,791 non-test lines). It does not use a single line of the daemon lib — what it uses is only agent's command vocabulary constants (`engram.rs:117-121`) · command's request number · discovery's install location. But since it is the same package, it pulls in the entire daemon dependency set at build time (minimizing dependencies was intentional, to the point of hand-writing HTTP — the same `Cargo.toml` comment). **→ Handled in 2-3 (2026-10-08)**: moved to the independent package `crates/engram-dashboard-cli` (U2 `58c80a3` · dependencies = agent · command · serde_json). In the meantime 2-2 U2 moved the install location into the daemon lib, so the CLI called the daemon lib on one line (the help body path), and 2-3 U1 (`2de9756`) moved help to the daemon (`/control/help`) and removed that very line — 「설치 위치 규칙 한 벌 더」 of candidate 9 below no longer has a target (ADR-0285 · step-log).
- The CLI vocabulary constants (verbs · flags · executable name · status words) are also used by agent itself (`manager.rs` · `commands.rs` · backend) — since they are 「`agent.*` 명령의 어휘」, it is right for agent to own them.
- Shell tests pull in the daemon package as a test-only dependency (`src-tauri/Cargo.toml:104`) — because the test server functions (`start_test_server*`, `crates/engram-dashboard-daemon/src/lib.rs:915-929`) are exposed as public API without a test marker. **→ Handled in 2-4 (2026-10-08 · ADR-0286)**: those functions were moved behind the daemon feature `test-support`, and the shell's daemon dev dependency turns that feature on (the edge remains).

### Decision candidate 9 — engram CLI as an independent package

**Status: direction confirmed (user 2026-10-02 — 「engram 은 독립 패키지로 빼는 게 맞는데 그냥 shell 처럼 하나 분리」) · ADR-0273.**

- Set up a separate **package that produces one exe**, like the shell (`src-tauri` → `engram-dashboard.exe`) — location under `crates/`, and the package name keeps the `engram-dashboard-` prefix (the CI dependency-cap gate identifies workspace members by that name prefix — with any other name the gate just passes). **The bin name stays `engram`** (it interlocks with agent `CLI_EXE_NAME` — changing it silently stops mail).
- Dependencies = agent (command vocabulary) · command (request number) · install-location rule. The install-location rule goes to the daemon once discovery is gone (candidate 6), so one more copy is placed on the CLI side and tied by a test that checks they produce the same path (the same handling as the `daemon.json` location of the daemon · shell).
- Remaining daemon boundary (undecided): put the test server functions behind a test-only feature flag — shell integration tests start and use a real daemon, so the shell → daemon test dependency itself remains. **→ Handled in 2-4 (2026-10-08 · ADR-0286)**: moved behind the daemon feature `test-support` — this item is closed.

## 9. Shell — later (user 2026-10-02)

- 「일단 한 곳에 있다가 나중에 분리할 거니. 플러그인도 돼야 하고 좀 천지개벽이 발생될 거라서」 — the shell only does boundary cleanup (cutting agent·net·discovery) and the inner split is handled separately together with the plugin design (tied to T-27).
- **What we want to do with plugins (user 2026-10-10 — 「그냥 한번 얘기한 거야 나중에」 · not a decision)** — ① full customization of the JSON-mode chat style (e.g. mail defaults to a square box + a mail icon, but the user can change it so that it is shown only in an HTML template) ② additional development of Engram Slot ③ custom addition of local models — we do not know what will come out, so they are left out for now. Order = at shell design time (after transport).
  - What main pointed out in advance (before the survey · knowledge-based): for ①, there are three depths of swapping (theme values · HTML template · code component). ② reopens ADR-0060 (slot content = a fixed list · rationale 「플러그인 생태계가 없다」), and since layout authority is the backend (ADR-0035 · 0057) the backend has to keep slot kinds it does not know opaquely, and under 「LLM-우선 제어」 a plugin slot also has to put its own commands on the command bus. ★The biggest fork = isolation★ — the webview holds the shell · daemon IPC and the chat content is LLM output (untrustworthy), so for HTML · code customization, first decide whether to sandbox (iframe). At the start of shell design, use `/research` to look first at mature places (VS Code notebook renderers · webviews · JupyterLab MIME renderers · Obsidian etc.).

---

## 10. Work order and way of working (2026-10-02 — main's proposal · user review)

**Principle:** bottom-up (foundation first) · build green at every stage · work that touches widely is done alone · transport is a separate last stage.

- **0. ADR** — record candidates 1~9 → write ADR-0266~0273 (2026-10-02 · candidate 7 deferred to the transport TRD). What is overturned: ADR-0175 decision 1 (rejecting platform as an independent crate · tenancy condition ①) · ADR-0218 decision 11 · ~~ADR-0155/0110 「워크스페이스 의존 0」~~ (not overturned because candidate 2 is withdrawn · ADR-0267 rewritten 2026-10-03) · the two-entrance description of ADR-0219 · ADR-0024 data location sharing + T-10 closure · ADR-0155 decision 3 (carrying the envelope in `protocol`).
- **1. Foundation**
  - 1-1 base shared functions + replacement (candidate 4) — ~~command·messaging gate relaxation (candidate 2)~~ dropped (candidate 2 withdrawn — connect only when it becomes necessary · user 2026-10-03 · ADR-0267) · ★atomic write drops out here★ (it carries the Windows-only rename retry, so it goes to platform → 1-3 · main's judgment 2026-10-03 · ADR-0266/0269). Unifying the moving-aside of corrupt copies stays in 1-1 — ★but after storage P3 lands★ (way of working below). The rest (text · time · path · sync · testing) go first.
  - 1-2 → left empty (the old log switch — pulled out as side work and gone on 2026-10-03 · ADR-0268. Old records point to 1-2).
  - 1-3 platform crate (candidate 1) — Job Object as a 「프로세스 그룹」 handle · decide the behavior of the several copies of atomic write as one and move it (★after storage P3 lands — the rest of the platform move goes first★) · **the kill causality invariant is involved, so full QA**.
- **2. Splitting client · daemon**
  - 2-1 cut shell → agent (candidate 5) — prerequisite 1-1 · **① slot spawn to the `agent.new` path → ② remove the shell check, strictly in this order** · ③ registration refusal message box · GUI measurement. → The path was changed by ADR-0279 (policy wall = the daemon `SpawnByCwd` handler · the slot spawn path as is). **→ Landed 2026-10-07** (U1 `8b48386` · U2 `16f81be` · U3 `7222b90` — refusal box = ADR-0281 · step-log).
  - 2-2 split discovery (candidate 6, excluding the stop-command client) — prerequisite 1-3 · daemon startup measured. **→ Landed 2026-10-07** (U1 `a0ec7f4` · U2 `3c9bf33` · U3 `b42e139` · U4 `d2f9822` — crate deletion · the stop-command client is in the shell `daemon_client/stop.rs` as a temporary home until 3-2 · details = ADR-0282 · step-log).
  - 2-3 engram CLI independent package (candidate 9) — prerequisite 2-2. **→ Landed 2026-10-08** (U1 `2de9756` — the daemon serves help (`/control/help`) · a common CLI failure path · U2 `58c80a3` — package `engram-dashboard-cli` · U3 — gate · docs · details = ADR-0285 · step-log).
  - 2-4 test server functions behind a test-only flag. **→ Landed 2026-10-08** (details = ADR-0286 · step-log).
- **A. Two file cleanups that stage 1 deferred** (atomic write unification · corrupt-copy unification — TRD 1-1 §8 D1~D3 · TRD 1-3 U-W · bonus = three copies of agent child spawning · shell lock-poison recovery) **→ Landed 2026-10-10~11** (U1~U8 · U5a · U5b-1 · U5b-2 — the data file rule is a single base `file` · a file of a newer version is not overwritten · agents · presets save refusal = read-only mode · details = ADR-0291 · `../process/S21-crate-boundaries/trd-A-data-file-unification.md` · step-log).
- **Order after A (2026-10-11)** — ① implement the panic policy (ADR-0288 · ADR-0290 — branch `v0.3.3/feat/panic-policy`) → ② **mail address = the inner key becomes the agent id (AgentId) · the side the agent sees and writes stays as the name** (user request 2026-10-11 — included in the implementation list · state survey = `../../.claude/handoff/attachments/mail-id-survey-2026-10-11.md`: name-keyed queue · no messaging hook on rename · 「단일 id 키 안은 검토된 적 없음」) → ③ 3. transport below. The order is main's proposal (before user confirmation — written in the survey document). What the user decided earlier is 「패닉 정책을 끝내고 transport 는 새 세션에서」. ★2026-10-12 update — ① landed (ADR-0292 · ADR-0293) · the order of ② and ③ is swapped (user decision: T-29 is not urgent · the two do not overlap in files or contracts) → next = ③ transport (new session) → ② mail id unification★. Procedure = CLAUDE.md 「개발 스텝」 (TRD → review → decision → implementation).
- **3. transport** (separate TRD · separate branch) — 3-1 `protocol` cleanup (candidate 8 · moving the authentication message · zero-dependency gate) · 3-2 shell attach (adapter · stop-command client · connection-info interface) · 3-3 daemon attach + retiring net (candidate 7).
- **4. Inner cleanup** — daemon structure problems · four small command cleanups · agent `transport/` rename · whether to split agent. The shell's inside goes together with plugins.
- ~~Side work — switching logs to go via base (candidate 3 · the old 1-2)~~ — gone (user 2026-10-03 「로그 감싸지 마」 · ADR-0268 rewritten).
- **Later (separate track)** — logging system design (big section A) · preparing for remote.

**Way of working (user 2026-10-02):**
- Branch = one for stages 0~2 (`v0.3.3/refactor/<슬러그>`) · stage 3 separate.
- **Push per unit (stage)** → CI green → master merge (merge commit · integration procedure without checkout). Push · merge each time with user confirmation.
- **At every handoff, take origin updates and merge them into the working branch.**
- **After a commit, syncing the other worktrees is relayed by the user** (the session does not touch other worktrees).
- **Refactoring-related things noticed while skimming the codebase during work are accumulated in section 11 below as they come** (user 2026-10-02 — 「지나가면서 계속 메모해 놔」). Write them without asking, and raise only those that need a decision, bundled.
- **Stage 1 starts with a TRD** (user 2026-10-03) — start = next session. Implementation forks (the atomic-write behavior to merge into one · clock trait details etc.) are raised to the user as options.
  - **Stage 1 TRDs done (2026-10-03~04)** — 1-1 = `../process/S21-crate-boundaries/trd-1-1-base-helpers.md` · 1-3 = `../process/S21-crate-boundaries/trd-1-3-platform-crate.md`. The forks were delegated by the user (2026-10-04 「알아서 진행해」) → the recommendation was adopted = ADR-0275. Next = 1-1 U1 · 1-3 after the 1-1 merge.
- **File helper unification is done after storage P3 lands on master** (user 2026-10-03 「알아서」 → main applied the recommendation) — it covers atomic write `write_atomic` (→ platform · 1-3) and the corrupt-copy moving-aside `set_aside_corrupt` unification (1-1). The reason = the storage P3 of another worktree (wt1) (shell screen state `shell\state\state.json` · `../process/S21-storage/trd.md`) adds file-writing code in the shell `src-tauri/src/fsutil.rs` · settings store (`src-tauri/src/settings/`) places. The rest of stage 1 (text · time · path · sync · testing helpers · the platform move excluding `write_atomic`) goes first. ★→ Done as TRD A (`../process/S21-crate-boundaries/trd-A-data-file-unification.md`) (landed 2026-10-10~11) — the home is base `file` for both atomic write and corrupt copies (not platform · only the lock decision · retry · folder sync are in platform `fs` — ADR-0291)★

---

## 11. Things seen in passing — accumulated cleanup candidates (keep accumulating)

Observations not yet raised as decision candidates. Handle them when passing the relevant place during stage work, or hand them to the stage-4 agenda.

| What | Where | When to handle |
|---|---|---|
| ~~The 「남은 `platform` 은 Job Object 래퍼 하나뿐」 in the CLAUDE.md 「백엔드 모듈 맵」 agent entry is stale (actually three)~~ **→ Handled in 1-3 U3**: when the agent `platform` module disappeared, that sentence was changed to 「`platform` 모듈도 없다」 (marked in 1-3 U8) | `CLAUDE.md` · ~~`crates/engram-dashboard-agent/src/platform/mod.rs:1-2`~~ (deleted) | ~~Fix the whole sentence at 1-3 (platform move)~~ done |
| ~~The shell `Cargo.toml` comment 「agent 에서 `COMMAND_SPECS` 하나만 남았다」 is stale (actually three)~~ **→ Handled in 2-1 U2**: the shell's agent dependency was deleted along with that comment (before that, the comment had already been fixed to 「둘」 — TRD 2-1 §1-5) | `src-tauri/Cargo.toml:62` | ~~Disappears together with the dependency at 2-1 (cut shell → agent)~~ done |
| The daemon only declares `tracing-subscriber` and does not use it | `crates/engram-dashboard-daemon/Cargo.toml` (recorded in the CLAUDE.md 「의존성」 section) | Stage 4 (daemon cleanup) |
| agent generates and commits TS bindings but the frontend uses them in 0 places — yet it carries ts-rs as an operational dependency | `crates/engram-dashboard-agent/bindings/` | Stage 4 (agent) — first check whether there is an alias import |
| The `transport/` inside agent (PTY·stdio) and the transport crate (WebSocket) have the same name | `crates/engram-dashboard-agent/src/transport/` | Stage 4 rename |
| ~~The executable-location calculation is split across two places~~ **→ Handled in 2-2 U1**: it was actually three places — all three call platform `env::sibling_exe` (ADR-0282) | ~~`crates/engram-dashboard-daemon/src/lib.rs:96,146` · `crates/engram-dashboard-discovery/src/lib.rs:981`~~ actual = daemon `src/lib.rs` `locate_send_exe` · discovery `src/lib.rs` `locate_daemon_exe` · daemon `src/bin/roundtrip_smoke.rs` `sibling_send_exe` | ~~2-2 (split discovery)~~ done |
| The shell hand-duplicates command's read/write marker · catalog entry | `src-tauri/src/view_commands.rs:107,573-633` | Stage 4 (command cleanup 3·4) |
| The packet `CommandListEntry` is almost the same as the roster entry `RosterEntry` (`available` is always true). **Seen in 3-1 U5 (2026-11-04) → moved to stage 4**: the two stay separate — under ADR-0272 the packet is protocol's own copy and the daemon edge fills it (`control/catalog.rs` `merge`, which sets `available: true` for every row). What is left is the always-true `available` slot; dropping it changes the wire shape, so it is a protocol-version question | ~~`crates/engram-dashboard-protocol/src/messages.rs:392`~~ `crates/engram-dashboard-protocol/src/messages.rs:416` · `crates/engram-dashboard-command/src/roster.rs:10` · `crates/engram-dashboard-daemon/src/control/catalog.rs:61` | ~~To be looked at together at 3-1 (`protocol` cleanup)~~ Stage 4 (command cleanup) |
| An order trap in checking entrance arguments (`contains` → `check_args` → `call`) — if there are two entrances, make it a bundled function | `crates/engram-dashboard-daemon/src/control/commands.rs:172-182` | When the entrances increase |
| `Clock` time interface has four copies (transport · daemon `command_delivery` · discovery · daemon `usage_service`) | candidate 4 table | 1-1 |
| ~~★**It may collide with candidate 6 — look at it again with the user before starting 2-2**★~~ **→ Resolved: user 2026-10-02 — `DataLayout` is also split → ADR-0271 decision 4.** Original description: master's storage structure overhaul (ADR-0264, 2026-10-02) split the data root per component (`daemon\{state,run}` · `shell\{config,state}` · `webview\` · `logs\`) and **newly placed the single source `DataLayout` of those paths in discovery**. Candidate 6 (「discovery 를 뽀개 경로 규칙은 데몬으로, 셸은 각자」) and 「셸·데몬 경로를 한 곳에서 계산」 diverge | `crates/engram-dashboard-discovery/src/layout.rs` · ADR-0264 | ~~User decision before 2-2~~ resolved (ADR-0271 decision 4) |
| The `^\s*use tauri` anchor of the `use tauri` isolation gates (agent · base · transport · platform) cannot catch `pub use tauri::…` · `use ::tauri::…` — whether to widen it to a gate that measures the absence of tauri in the resolved dependency graph (`cargo tree -i tauri`) · source = 1-3 U0 codex review (2026-10-05) | those four gates in `.github/workflows/ci.yml` · CLAUDE.md 「빌드·검증 명령」 · `.claude/skill-bindings/qa.md` 4 · 4b · 4e | undecided (main's judgment) |
| The 「이 crate 의 패키지 이름에서 그 접두를 떼면 게이트가 조용히 눈을 감는다」 of transport header gate ① is wrong — if it renames itself, `cargo tree -p` dies or its own line loses the prefix and that gate turns red. The real holes are depending on a member without the prefix, and, if this crate drops the prefix, **other** crates' cap gates not seeing the edge into this crate (the same sentence in the platform header was fixed in the 1-3 U0 review (2026-10-05)) | `crates/engram-dashboard-transport/src/lib.rs:39-40` | Stage 3 (transport), when passing that header |
| ~~The messaging name-gate regex lacks `net` · `transport` — 「새 워크스페이스 crate 가 생기면 더한다」 was not kept for those two (zero occurrences of the two names in messaging source — adding them stays green · measured 2026-10-05)~~ **→ Handled in 2-2 U4**: the two names were added in the same edit that removes `discovery` (five copies — TRD 2-2 §8 O2) | `.github/workflows/ci.yml` messaging name gate · CLAUDE.md · `.claude/skill-bindings/qa.md` · `docs/testing-strategy.md:64` · `docs/reference/architecture-overview.md:492` | ~~The next time that gate is touched (the five copies at once)~~ done |
| ~~`.claude/skill-bindings/qa.md` has no transport isolation gate at all — only CI measures it (this conflicts with the preamble statement 「CI에 있는데 여기 없는 게이트를 발견하면 그건 드리프트다 — 이 파일을 채운다」 of that file)~~ **→ Handled in 1-3 U8**: added a 「재사용 전송 lib 격리 게이트」 block (headers ①~⑤ = CI `transport gate 1`~`4d`) to the qa standard | `.claude/skill-bindings/qa.md` · `crates/engram-dashboard-transport/src/lib.rs` 「격리 게이트」 | ~~Before stage 3 (transport)~~ done |
| ~~In operational code there are two environment-variable name comparisons that always fold case regardless of the OS — finding `CODEX_HOME` in the codex lock folder · finding `MAX_THINKING_TOKENS` in the claude spawn. (The third = `env_overrides_daemon` (`usage/gate.rs:69`) in the usage query — this one is **intentional**: the doc at that place says it was chosen to err toward 「다르다」 · pointed out by the 1-3 U8 review.) It is the same OS rule (case of environment-variable names) as platform `env::env_key_eq`, but the meaning on POSIX differs (that function treats names as equal on POSIX only if they are exactly the same). Since there is no `cfg`, it is caught neither by the `cfg` regex of TRD 1-3 §2 nor by the planned §4-4 `cfg` gate. The `TERM` · `COLORTERM` default lookup of `transport/pty.rs` is excluded because folding on every OS is intended (the comment at that place — ADR-0049) · source = 1-3 U2 review (2026-10-05) · **the 1-3 U8 reconsideration result = it is not in the OS `cfg` file list** (there is no `cfg`) — that class (runtime OS distinctions without `cfg` · environment-variable name comparisons that always fold case) was written into platform header gate ④ as a limit of the invariant gate. Changing the two to `env::env_key_eq` is a behavior change that alters the meaning on POSIX, so it is outside the scope of U8 (gate · docs)~~ **→ Handled in the 1-3 small-fix commit**: the `CODEX_HOME` · `MAX_THINKING_TOKENS` lookups were changed to `env::env_key_eq` (the Windows result is unchanged · the two tests at that place set their expected values to `env_key_eq` so that they hold on any OS). `env_overrides_daemon` stays as is because it is an intended conservative decision. The `stripped_env_keys` (`backend/claude/usage_probe.rs`) by which the usage query strips the `CLAUDE` key from the daemon env also always folds case but is intentional, so it stays as is — since it is a list to strip · a list to keep, on POSIX it merely strips more or leaves a similar-looking key that the CLI does not read | `crates/engram-dashboard-agent/src/backend/codex/thread_lock.rs` (`child_lock_dir`) · `crates/engram-dashboard-agent/src/backend/claude/mod.rs` (`MAX_THINKING_TOKENS_KEY` of `build_spec`) · `crates/engram-dashboard-agent/src/usage/gate.rs:69` (`env_overrides_daemon` — intentional) | ~~1-3 small-fix commit (user decision 2026-10-06)~~ done |
| The doc 「★왜 필요한가★」 of platform `process::child_pids` justifies its use by only one real-process isolation test — operational code (`process::subtree` — recovering the codex session id) also calls it. Rewrite that paragraph to cover the operational use as well · source = 1-3 U4 review (2026-10-06) | `crates/engram-dashboard-platform/src/process.rs` (`child_pids`) | The next time that function is passed |
| ~~net gate 2a (platform symbol allowlist) counts **only the number** — alias imports (`use engram_dashboard_platform as p;`) · braces (`use …::{spawn::x, group::Y}`) are 0 matches, a module import (`use …::spawn;`) covers that whole module with 1 line, and swapping symbols stays green as long as the count is the same. It also conflicts with the net `Cargo.toml` comment 「심볼 목록 자체를 기대값으로 못 박는다」 (CI looks at the count, not the list). Remedy candidates = compare the sorted output directly with the five names · measure the bare crate-name match count separately · source = 1-3 U6 review (2026-10-06 — codex · Claude both pointed it out as pre-existing)~~ **→ Handled in 1-3 U8**: 2a was made into two checks — ① compare the sorted symbol list directly with the five names ② zero lines outside comments for calls where `::이름글자` does not follow the crate name (alias · braces · glob · broken path). The five copies (CI · net header · CLAUDE.md · qa · testing-strategy) were fixed together, and the net `Cargo.toml` comment is now true, so it was kept. The remaining limit = if a dependency name is renamed in the manifest, both checks go blind and gate 3 does not back them either (an allowed edge, so green) — then the symbol range is the reviewer's part (written in the net header) | `.github/workflows/ci.yml` net 2a · `crates/engram-dashboard-net/src/lib.rs` header · `crates/engram-dashboard-net/Cargo.toml` | ~~1-3 U8 (invariant gate) candidate~~ done |
| ~~If channels stdio · pty fail while being put into the group after spawning (`GroupOwner::new()?` · `adopt(pid)?`), the spawned child remains outside the group — unlike the codex channel, they do not set up the guard (`ChildGuard`) first (same since HEAD) · source = 1-3 U3 review (2026-10-06)~~ **→ Handled in A U7** (ADR-0291 decision 13): the spawning of the three channels was gathered into `transport/spawn.rs` — the guard `ChildGuard` stands right after the spawn, and the next step is putting into the group (`new_group_with` — the first step that can fail after the spawn). If putting in fails, the guard kills the direct child, and if it fails after putting in, the group owner is dropped first, and after the Job ends the child that was put in and the descendants spawned after it, the guard collects the direct child (descendants are not found by number and killed — the U7 amendment of decision 13 · provisional). The placement is measured by a source test, and killing the direct child and ending even the grandchild in a failure after putting in are measured by real-process tests (std · pty). What remains = grandchildren spawned before the group put-in (both success · failure paths · out of scope — TRD A §3-7 · `docs/tracking.md` T-53) | ~~`crates/engram-dashboard-agent/src/transport/{stdio.rs, pty.rs}` (`open`)~~ actual = `crates/engram-dashboard-agent/src/transport/spawn.rs` · `transport/{stdio.rs, pty.rs}` that calls it · `backend/codex/transport.rs` (`open`) | ~~Stage 4 (agent) or the next time those channels are passed~~ done |
| The 「base 심볼 allowlist = 정확히 2」 of `docs/reference/architecture-map-notes.md:390` has been stale since U1 (now platform symbols · 5) — a date-stamped snapshot document · source = 1-3 U6 review | `docs/reference/architecture-map-notes.md` | When that map is re-extracted |
| ~~`.claude/skill-bindings/qa.md` has no command for running the real claude `#[ignore]` tests locally (it only points to the CI `--skip` list as canonical) — the QA worker guessed the command · source = 1-3 U3 QA (2026-10-06)~~ **→ Handled in 1-3 U8**: wrote the local commands under CI-uncovered ② of qa 「CI와의 분담」 — the `--skip` list (all daemon `tests/control_send.rs` · the five non-ignored ones are already run by standard step 2 · c1, which is `#[ignore]`, is `--ignored --exact`) and the other real-CLI `#[ignore]` lanes (`usage_probe_smoke` · `backend_contract`) | `.claude/skill-bindings/qa.md` | ~~1-3 U8 or the next time the binding is touched~~ done |
| ~~platform `process::kill_tree` launches `taskkill` without hiding the window — when the GUI shell calls daemon kill, a console may flash (U7 moved the behavior as is · `spawn::hide_console_window` is next to it so it is one line away, but it is a behavior change · that path is only the fallback `daemon_stop` — it runs when there is no connection or the graceful kill failed, and also when graceful was accepted but `daemon_status` answers that it is still alive or that query fails — frontend `daemonControl`) · source = 1-3 U7 review (2026-10-06)~~ **→ Handled in the 1-3 small-fix commit**: attached `spawn::hide_console_window` to `taskkill` of `kill_tree` | `crates/engram-dashboard-platform/src/process.rs` (`kill_tree`) | ~~1-3 small-fix commit (user decision 2026-10-06)~~ done |
| ~~The shell comment says 「`taskkill /F`」 but the actual argument is `/F /T` · source = 1-3 U7 coder~~ **→ Handled in 2-2 U3**: fixed that comment to 「자식 트리째 강제로 끈다(`/F /T` — platform `process::kill_tree`)」 | `src-tauri/src/commands/discovery.rs:161` | ~~2-2 (split discovery)~~ done |
| `docs/reference/architecture-map-notes.md:86` points to the WMI launch place as `discovery/src/lib.rs:932,1105-1153` — with 1-3 U7 that code went to platform `spawn/wmi.rs` (the lines were already off before that) · source = 1-3 U7 review | `docs/reference/architecture-map-notes.md` | When that map is re-extracted |
| ~~Two real WMI `#[ignore]` tests of discovery fail with `ExeNotFound` when run with `cargo test -p engram-dashboard-discovery -- --ignored real_wmi` — cargo sets the cwd to the crate folder, while the daemon exe lookup looks at the test exe folder · cwd/target/debug. Running that test binary directly with the repository root as cwd passes (measured 2026-10-06 · rv matrix None 0 · NEW_CONSOLE 0 · DETACHED 0 · NO_WINDOW 21) · source = 1-3 U7 QA~~ **→ Handled in 2-2 U3**: when the tests moved to the shell `lib_unit` they use a test-only helper (`test_daemon_exe`) that finds the daemon exe at the test exe location (`target\<profile>`) instead of the operational `locate_daemon_exe` — it does not rely on the cwd. The run command is written in the shell real-daemon lane of qa 「CI와의 분담」 | ~~`crates/engram-dashboard-discovery/src/lib.rs` (`locate_daemon_exe` · test `real_wmi_*`)~~ actual = `src-tauri/src/discovery/tests.rs` (`test_daemon_exe` · `real_wmi_*`) · `.claude/skill-bindings/qa.md` | ~~2-2 or the next time that test is run~~ done |
| CLAUDE.md 「백엔드 모듈 맵」 has no `engram-dashboard-transport` crate entry — it is a workspace member and has CI isolation gates (`transport gate 1`~`4d` of `ci.yml`) and a qa block, but only the map lacks its name (`rg engram-dashboard-transport CLAUDE.md` → 0 lines · measured 2026-10-06) · source = 2-1 survey (2026-10-06) | `CLAUDE.md` 「백엔드 모듈 맵」 · `crates/engram-dashboard-transport/src/lib.rs` (header — the canonical source the entry will point to) | Unhandled — the next time that map is touched (at the latest stage 3 transport) |
| The frontend registry's `agentlist.createCodex` (sibling `createCodexJson`) can be called by the LLM through `__engramCmd` after `humanOnly` was withdrawn (2026-09-22), and that `CreateProfile` path does not look at the LLM backend policy (`LLM_BACKEND_POLICY`) — the 2-1 daemon wall covers only `SpawnByCwd`. Zero impact today (the table closes no word). ~~Separately, the description of ③ (`humanOnly`) among the agent-side 「정책을 묻는 문 셋」 is stale because `humanOnly` was withdrawn on 2026-09-22 (not because of a bypass path)~~ **→ the 「문 ③」 text was handled in 2-1 U1 `8b48386`**: rewrote the door list as `agent.new` · daemon `SpawnByCwd` · shell `gate_backend` (~~U2 removes it~~ 2-1 U2 removed it — the door list was also fixed to two) and wrote that `humanOnly` is not a door that looks at the table — the `CreateProfile` bypass itself is unhandled · source = 2-1 TRD writing (2026-10-06 · `../process/S21-crate-boundaries/trd-2-1-shell-agent-cut.md` §8 O5) | `src/commands/agentCommands.ts` (`agentlist.createCodex` · header comment `:41`) · `crates/engram-dashboard-agent/src/commands.rs:600-614` (「문 ③」) | Unhandled — look at the path before the policy table closes even one word · ~~the 「문 ③」 text when 2-1 U1 passes that comment~~ done (`8b48386`) |
| When codex is launched in a slot, the tab title is 「Claude Code」 until a refresh — the daemon `SpawnByCwd` handler sends only `Spawned` and does not broadcast `ProfileListUpdated`, so the frontend profile list is empty (display only wrong · no effect on behavior) · source = 2-1 U2 QA full (2026-10-06) | `crates/engram-dashboard-daemon/src/connection_core.rs` (`SpawnByCwd` handler) | Unhandled — daemon cleanup (stage 4) or when passing that handler |
---

## Big sections (큰 절) — handled separately later (only collected for now)

### A. Logging system design

**Status: only requirements collected (user 2026-09-26) · design after benchmarking other software (`/research`).**

- **Requirements (the user's original words):**
  - 「기본적으로 카테고리가 있고, 위험도가 있고」
  - 「지금은 UI 켜져 있어야 현황을 파악할 수 있는데 로그만으로 파악할 수 있게」
  - Rationale — 「나중에 다중 데몬 들어가면 결국 장애에 대한 판단을 로깅으로밖에 할 수가 없으니깐. 프론트는 괜찮은데 백엔드는 치명적」
  - Boundary — 「모든 대화 자체를 로깅하는 건 아니고」
- **Foundation:** logs are not wrapped (ADR-0268 rewritten · user 2026-10-03). Turning groups on and off is done with tracing target + base's `EnvFilter`. **Current state (code cross-check 2026-10-03):** the filter criterion is a single level — at startup `RUST_LOG` if present, otherwise `warn` (`crates/engram-dashboard-base/src/logging/mod.rs` `init_subscriber`) · the runtime change function `set_log_level` exists but has 0 callers outside base, so there is actually no entrance to change it · a separate `target:` is only `agent_stderr` (`agent/src/transport/stdio.rs` · codex transport). What this design will decide = the group (category) vocabulary · the entrance for changing at runtime (the LLM control surface — CLAUDE.md 「LLM-우선 제어」).
