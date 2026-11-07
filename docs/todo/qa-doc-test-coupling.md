# The QA procedure doc is a CI test input — decoupling

> **Status:** Waiting for trigger · **Canonical index:** [`../todo.md`](../todo.md)

- **Problem:** `src/util/launcherWiring.test.ts` opens `.claude/skill-bindings/qa.md` as text and asserts the build commands in its code blocks (does it call `scripts/build-client-shell.mjs` · is there no bare `cargo build -p engram-dashboard` — ADR-0137). It is a layer crossing where a product test (vitest, `src/`) depends on a procedure document for agent work (`.claude/`).
- **Cost that spread:** the CI path filter excluded `.claude/**` and so had to revive this one file at the end (ADR-0232 decision 1). The test file's location is also an exception to the colocation convention (`src/util/devConfigOverlay.test.ts` header).
- **How to untangle (candidate):** move the GUI measurement preparation commands into a script under `scripts/` and have `qa.md` only say to call that script → the test looks at the script → `qa.md` becomes a pure document and the CI revival is no longer needed.
- ★**What not to lose when untangling**★ — this assertion came from a real incident (`473e82f`: the launcher changed to a bare `cargo build` while every gate was green). The check itself remains — 「문서화된 모든 빌드 경로가 그 스크립트를 부른다」 (every documented build path calls that script). When untangled, the revival line of `ci.yml` and the revival statement of ADR-0232 decision 1 are also removed (with a new ADR).
- **Why not now:** the user is fixing the QA skill in another session, so we agreed to discuss it after that work ends (user decision 2026-09-26). **Trigger = the end of that QA skill work.**
