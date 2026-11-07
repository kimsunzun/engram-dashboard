# ADR-0286: Daemon test server functions go behind the daemon feature test-support — no production-graph gate is added

- 상태: 확정 (2026-10-08, basis: the main decision under user delegation 2026-10-07 · work order 2-4 (`docs/refactoring/architecture-discussion-2026-09-26.md` §10))
- 관련: ADR-0088 (the shape where the daemon re-declares itself as a dev dependency to turn features on) · ADR-0269 decision 5 · ADR-0275 decision 5 (the test feature `test-support` of base · platform — same name) · ADR-0275 rejected alternative N1 (the name `test-harness` was rejected) · ADR-0271 decision 4 (the shell same-path test calls the daemon `data_dir`) · memo `docs/refactoring/architecture-discussion-2026-09-26.md` §8 · §10 · `crates/engram-dashboard-daemon/src/lib.rs` · `crates/engram-dashboard-daemon/Cargo.toml` · `src-tauri/Cargo.toml` · step-log S21 · Amends ADR-0273 (the remaining daemon-boundary item of its Impact)

## 맥락

The in-process test server functions of the daemon lib (`start_test_server` · `start_test_server_with_keepalive` · `start_test_server_with_store` · the handle `TestServerHandle`) are called by none of the production `run()`, yet they were compiled and shipped as public API with no test marking. The two in-memory stores used only by these functions (`MemProfileStore` · `MemPresetStore`), and the `KeepaliveConfig` re-export kept for the public signature of the keepalive-injecting variant, were in the same position.

The map notes (`docs/reference/architecture-map-notes.md:288` — 「테스트 비계가 공개 API에 실려 나간다 … `experiment` 는 feature 게이트가 있고 테스트 서버 헬퍼는 없다」) and boundary memo §8 recorded this, and work order 2-4 of memo §10 took up 「테스트용 서버 함수를 테스트 전용 플래그 뒤로」. The callers are the daemon integration test (`tests/ws_e2e.rs`) and one shell unit test (`src-tauri/src/daemon_client/tests.rs`). The shell → daemon edge is a dev dependency.

## 결정

1. **Add an empty feature `test-support` to the daemon and put all the items above behind it** — the three test server functions · `TestServerHandle` · the internal `start_test_server_inner` · the two in-memory stores · the `KeepaliveConfig` re-export. This feature does not turn on any feature of another crate. A build with the feature off does not contain these items, so there is no unused-code warning either. The name is the same as the test feature of base · platform (ADR-0269 decision 5 · ADR-0275 decision 5).
2. **The places that turn it on are two dev dependencies** — the daemon's self re-declaration (the ADR-0088 shape) with `features = ["test-harness", "test-support"]`, and the daemon dev dependency in the shell's `src-tauri/Cargo.toml` with `features = ["test-support"]`. The call sites stay as they are.
3. **No new CI gate is added.** Every current member outside the daemon is already blocked the moment it gets a normal (production) edge to the daemon:
   - shell — `shell gate 1` turns red (daemon → agent pulls agent into the shell's normal graph).
   - cli — `cli gate 1` (direct workspace dependencies = exactly the name set agent · cli · command).
   - base · platform · command · messaging · transport · net — each one's depth-1 dependency cap gate.
   - agent · protocol — the daemon depends on both normally, so it would become a cargo dependency cycle.

   So the only paths that are not blocked are putting `test-support` into the daemon's own `default` and a new package that depends on the daemon normally, and both are edits that show up in the manifest.

## 거부한 대안

This is the main decision under user delegation 2026-10-07.

- **Reuse the existing `test-harness`** — that feature turns on the daemon's own harness share (the hook during the send in `handle_send` · the CLI force knob · the `experiment` module), plus the same-named features of agent (`insert_test_session` etc.) and messaging (harness observation seams such as `set_accept_hook_for_test`) — most of them are seams inside production code. Hooking it onto the shell dev dependency would turn them all on together inside the shell test build. The name also splits in meaning — the reason ADR-0275 rejected alternative N1 gave for rejecting `test-harness` as the test feature name is 「agent · daemon · messaging 이 「운영 코드 안의 seam 을 연다」는 다른 뜻으로 쓴다」.
- **Add a production-graph gate in the base · platform shape** — that shape works: `cargo tree -p engram-dashboard -e normal,build,features -i engram-dashboard-daemon --target all | rg 'feature "test-support"'` gives 0 lines, and the counterpart with `dev` added to `-e` gives 1 line (measured 2026-10-08 — even when the daemon is not in the shell's normal graph, the daemon is inside the shell's resolve, so `cargo tree` prints 「nothing to print」 with rc=0). But decision 3 already blocks every current member, so there is less left to cover, and adding the gate would also require adding a qa binding line (editing a binding = subject to user approval).
- **Move the test functions into a separate test-util crate** — `start_test_server_inner` calls the daemon's private assembly functions · types (`build_daemon_wiring_with_store` · `run_accept_loop` · `UsageInputs`), so moving it to an outside crate would force widening them into public API — the opposite of the goal of taking test scaffolding out of the public API. And the shell → daemon dev edge stays anyway — the shell tests bring up the real daemon assembly (memo §8) and the shell same-path test calls the daemon `data_dir` (ADR-0271 decision 4).

## 근거

- Code fact: the production `run()` calls none of these items (the shared helpers `generate_token` · `build_daemon_wiring_with_store` · `run_accept_loop` · `UsageInputs` stay outside the feature as they are). The harness bin does not call them either.
- Among the other packages, the only manifest that depends on the daemon is the shell dev dependency (`rg "engram-dashboard-daemon" --glob '**/Cargo.toml'`).
- Moving them behind the feature drops no test — the workspace regression is the same before and after the implementation (measured 2026-10-08 · result line 59 · 4233 passed · 0 failed · 31 ignored).
- Code that calls `start_test_server` · `TestServerHandle` against the daemon lib without the feature fails to compile with `E0425` (「configured out」), and `KeepaliveConfig` fails with 「private」, while the same code stands against the lib with the feature on (measured 2026-10-08 — compiled directly with rustc against the metadata of the two libs).

## 영향 / 불변식

- **The shell → daemon dev edge stays.** The shell test build still compiles the daemon and the agent under it — the reason CI `shell gate 1` removes `dev` from `-e` is unchanged.
- **The caution about feature unification within one cargo invocation still applies** — if the lib and a test target are built in one invocation (`--all-targets` etc.), the `test-support` turned on by the dev dependency is merged into the daemon lib of that invocation (the daemon `Cargo.toml` 「운영 빌드 주의」 — the same risk group as `test-harness` of the self dev dependency). Release builds are not built that way.
- **For a test of another package to use the test server, it adds `features = ["test-support"]` to its own daemon dev dependency.** It does not put it on a production dependency or the daemon `default`. **If a new package depends on the daemon as a production dependency, or `test-support` goes into the daemon `default`, this decision is reopened (at that point a gate in the base · platform shape is added, targeting the daemon itself and that new package).**
- `KeepaliveConfig` is also used by production code (the accept loop), so a build with the feature off takes in the same type as private — only the public re-export is behind the feature.
- How to find it = `rg "ADR-0286"`.
