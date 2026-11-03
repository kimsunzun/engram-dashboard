# S30 — skill bindings and project docs for the new skill skeletons

The global skills moved to new `engram-*` skeletons. This topic rewrote the project's skill bindings for them, cut CLAUDE.md down to the standing rules (the branch/commit procedure moved to `docs/handbook/branch-commit.md`), and moved qa's full regression from CI to a local run inside qa (CI stays as a post-push check). No production code changed.

- Related ADR: ADR-0340 (partial supersede of ADR-0131).
- Tracking: T-61 (step 1 done).
- Timeline: `docs/process/step-log.md` `## S30`.
