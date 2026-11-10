# P3 handover (S25 step 6, JSON channel) — for the review-fix coder — 2026-11-10

P1+P2+P3 are uncommitted in `I:/Engram/apps/engram-dashboard-wt1` (base `7d1e0caa`). Agent tests after P3: 1805 passed /
23 ignored / 1828 listed. Paths under `crates/engram-dashboard-agent/src/`. Locate by symbol, not line.

## Structure (transport.rs unless noted)
- `AppServerOpen {thread: ThreadOpen, extra_roots: Vec<String>, first_turn_context: Option<String>}` — built only by
  `codex/mod.rs::app_server_open(spec, resume, control, artifacts)`: Noop ⇒ `priming_text(control)`, no roots/context;
  real ⇒ `artifacts.instructions_text` (priming file never read), roots as UTF-8 strings (fallible ⇒ `SpawnFailed`),
  context = `INSTRUCTIONS_CHANGED_PREFACE + "\n\n" + text` iff `resume.is_some() && instructions_changed && text`.
  `thread_open` now puts `developer_instructions` on both branches.
- `CodexAppServerTransport::open` stores the plan in `open_params: Mutex<Option<AppServerOpen>>` and builds the state
  with `State::with_first_turn_context(plan.first_turn_context.clone())`; `start()` moves the plan to `writer_loop`.
- `handshake(&AppServerOpen)` (writer thread, no state lock held): `initialize` with `capabilities` iff context →
  `initialized` → `skills/extraRoots/set` if roots (via `request_blocking`, same `HANDSHAKE_BUDGET`; error ⇒ `Err`) →
  start/resume; resume `developer_instructions` dropped (cloned params) unless
  `reaches(user_agent_version(ua), RESUME_INSTRUCTIONS_FLOOR=(0,155,0))`. Then the "codex 실행 재료 적용" info line
  (`experimental_api` on/off, `extra_roots` count, `resume_instructions`, `first_turn_context`), after "codex thread 개시".
  `floor_verdict` was refactored onto `user_agent_version` + `reaches` (same behaviour).
- `State` fields (all under the existing state mutex): `first_turn_context`, `context_turn` (seq of last carrying
  turn), `context_rejections: u8`, `instructions_delivered` (true iff no context at open).
  - `take_turn_locked` (lock held): `additional_context: s.turn_context()`; sets `context_turn = Some(seq)` when carried.
    Steer (`take_steer_locked`) never attaches.
  - `end_turn_if` (lock held): `close == Rejected && s.note_context_rejection(seq)`; at `CONTEXT_REJECTION_LIMIT=2`
    clears context; after unlock logs one warn and appends `CONTEXT_DROPPED_NOTE` (Korean screen text) to the detail.
    Both Rejected sources go through here (error reply in `Reader::resolve`, `turn_write_failed`).
  - `Reader::accepted_turn_start` (takes lock briefly): any `Ok` response to `turn/start` resets the count.
  - `Reader::note_context_echo` (lock held): runs in `handle_line` right after `note_tool_item` (before decode and
    before `witness_first_turn`); on a live userMessage echo, if context held, active seq == `context_turn` and
    `claims_locked` (refactor of `claims_our_turn`) ⇒ clear + `instructions_delivered = true`.
  - `witness_first_turn` reads `instructions_delivered` under the same lock as the thread check, calls `sink(delivered)`
    outside it. Helper `user_message_echo()` is shared by both.
- `backend/mod.rs`: `FirstTurnSink = Arc<dyn Fn(bool)>`. `session_id_latch.rs`: `first_turn_delivered: OnceLock<bool>`
  (first call wins), getter `first_turn_delivered()` dead outside tests (`allow` + TODO for step 8).
## Fake app-server (codex/mod.rs tests, cfg(windows), some items `pub(super)` for transport tests)
- `bake_fake_app_server(tid, FakeResume)` = defaults; `bake_fake_app_server_with(tid, FakeResume, FakeOptions{user_agent,
  extra_roots: FakeExtraRoots::Answers|Rejects})`. Default UA `engram-fake/0` is unreadable ⇒ resume field gated off,
  which is why the old fake tests stay unchanged.
- Script logs every received line (before answering) to `FAKE_LOG_ENV`; `received()`, `methods()`, `params_of(m)`.
  Echo after `turn/start` uses `$current` (start id or `resumed-<tid>`). Lines are logged before the answer (race-free).
## Tests (what pins what)
- protocol.rs: capabilities absent/present, resume field, extraRoots params, `additionalContext` shape, no-id envelope
  has exactly 2 fields. Usage-probe initialize pinned by the existing `the_dialogue_is_initialize_then_…`.
- codex/mod.rs: Noop pin `the_app_server_handshake_is_pinned_for_a_spawn_without_launch_kit` (only `resume/priming` row
  changed); flipped `a_resuming_app_server_handshake_carries_the_instructions_field`; plan tests `a_real_open_plan_*`;
  fake tests `the_handshake_sends_the_open_plan_in_order` (8 combos, `run_on_fake` helper),
  `the_resume_carries_the_instructions_only_from_codex_0_155_0`, `an_error_answer_to_the_extra_roots_fails_the_link`,
  `a_real_changed_resume_through_open_spawn_sends_both_channels`.
- transport.rs (end of tests module): helpers `harness_with_context`, `carries_context` (asserts exact shape),
  `open_user_turn`, `report_recorder`; reuse existing `End` enum + `finish_turn` for every no-echo end. Rows: carry until
  echo, kept after 8 ends, steer, below floor, no-port clear, late echo while Idle, retry bound (+LogCapture warn,
  failure line), reset/unanswered, no-context ⇒ true, e2e with real latch + real `launch::materialize` +
  `app_server_open`, notifications, UA gate unit, log line via `handshake_on_fake` (direct `handshake()` on test thread
  with `spawn_piped` + `reader_loop`, so a thread-local `LogCapture` sees it).
## Live file `tests/codex_launch_live.rs` (all `#[ignore]`, not run by me)
- L1–L3 one test: real `AgentManager::new_with_control` + `LiveControl` (endpoint URL with no server) + `LiveKits`
  (writes `<root>/vol-<id>-<epoch>/instructions.md`; release is a no-op, `remove_root` unlinks junctions then deletes).
  Global `AppliedCapture` subscriber keeps the §4-7 line. Stored hash written by hand (stands in for step 8).
  Approximations: hydration = L1 marker in replayed events' debug text; "context not in bubble" scans `Structured` only.
- L4: readiness = OSC 7501 `state=idle` bytes; argv order via `Get-CimInstance Win32_Process` on the cmd.exe whose line
  holds the scene root leaf. Assumes no startup modal in real `~/.codex`.
- L5 (cfg(windows)): `pty_quoted` copy of portable-pty `append_quoted`, `cmd.exe` + `raw_arg`, `config/read` sessionFlags
  under temp `CODEX_HOME`. (ii) asserts the odd-`"` line is NOT intact. (iii) cmd.exe found on the test's own PATH
  (approximation), shim lines sized assuming std quotes argv0 as `"cmd.exe"`; 8150 must fail, 7891 must succeed.
## Interpretations (flag to review if challenged)
- Rejection-count reset: any `Ok` turn/start response, incl. unreadable success and oversized turn id.
- Which echo clears: any attributed userMessage echo while active seq == `context_turn` — a late echo of an earlier
  carrying turn landing during the next carrying turn's response wait also clears (that turn did carry it).
- Known overclaim outside P3 files: `launch.rs` `instructions_text` doc says "every `thread/resume`" (gate not mentioned).
## Tooling and times
- Bash heredoc/py-in-heredoc folds `\\n` into a newline (hit once in the live file) — put `\` strings via Edit/Write.
- rustfmt only touched files: `rustfmt --edition 2021 src/backend/codex/mod.rs src/session_id_latch.rs
  tests/codex_launch_live.rs` (mod.rs recurses into codex children; they were clean). backend/mod.rs, manager.rs,
  session.rs were already clean — do not run rustfmt on backend/mod.rs blindly (it recurses into claude).
- Lib codex filter `cargo test -p engram-dashboard-agent --lib -- --test-threads=4 backend::codex` ≈ 9 s; full agent
  tests ≈ 2 min incl. build; workspace build ≈ 45 s. Baseline names: `-- --list | grep ': test' | sort`.
## Failed / rejected approaches
- Capturing the writer-thread log with a thread-local subscriber through the full transport — impossible; hence the
  direct-`handshake()` test. Global subscriber only in the live binary.
