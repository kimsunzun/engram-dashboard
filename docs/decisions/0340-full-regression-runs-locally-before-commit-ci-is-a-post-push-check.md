# ADR-0340: Full regression runs locally before commit, CI is a post-push check

- 상태: 확정 (2026-11-03, 근거: 사용자 결정 2026-11-02 + doc review full 3 rounds on the qa binding batch)
- 관련: Amends ADR-0131 (영향 로컬/CI 분담의 전체 회귀는 CI) · `.claude/skill-bindings/qa.md` 「전체 회귀 = 로컬」(canonical) · `.claude/skill-bindings/qa-verify.md` 「전체 회귀 — CI 세 잡 로컬 실행」 · CLAUDE.md 「브랜치·커밋」

## 맥락

ADR-0131 Impact recorded the local/CI split as: local = quick + GUI, full regression = CI.

The qa binding later gained a local run of the three CI jobs as a stand-in. The user made that stand-in the default path (2026-11-02): 「그냥 테스트할때는 거기에 맞게 로컬로 돌리고 CI는 이후에 확인차 돌리면 될듯」.

The qa binding batch review then flagged that ADR-0131 still said the opposite. The user treats the ADR-0131 line as a mis-statement, not a change of direction (2026-11-03: 「새 결정까지는 아닐것같은데? 실수인듯」). This ADR is the record of that correction. Every other clause of ADR-0131 stays live.

## 결정

- **Full regression = the three CI jobs (`backend` · `gates` · `frontend`) run locally, inside qa, against the working tree.** Procedure = qa-verify 「전체 회귀 — CI 세 잡 로컬 실행」.
- **Commit comes after qa.** The committed tree must match the tree qa measured (fingerprint rule in qa.md 「전체 회귀 = 로컬」).
- **CI stays as built in ADR-0131.** Its result is a post-push integration check. It is not part of the qa verdict.
- **What CI cannot run stays local** — GUI, real claude tests, `#[ignore]` tests. Unchanged from ADR-0131 decision 3.

## 거부한 대안

- **Keep the ADR-0131 split (full regression = CI)** — the user rejected it directly (2026-11-02, quote above). 약함 · 사용자 직접 기각.
- **Edit the ADR-0131 line in place** — ADR bodies are not rewritten to change a decision. A partial supersede keeps the other ADR-0131 clauses live and leaves a two-way link plus a status stamp.

## 근거

- User decision 2026-11-02 (quote above).
- The qa binding batch passed doc review full in 3 rounds (Claude + codex). This ADR closes the hold that review raised on ADR-0131.
- The local path has run end to end: the S32 qa local full regression passed (workspace 5072 passed / 0 failed · gates 30/30 · frontend 2802).

## 영향 / 불변식

- **qa.md 「전체 회귀 = 로컬」 is canonical** for the split. ADR-0131 and ADR-0232 cite qa.md 「CI와의 분담」. That heading no longer exists. The current heading is 「전체 회귀 = 로컬」.
- **CI green still gates the merge to master** (CLAUDE.md 「브랜치·커밋」: push → CI green → `--no-ff` merge). It no longer gates the qa verdict or the commit.
- **A commit whose tree differs from the qa-measured tree is unverified.** Re-run qa, or commit the measured tree as is.
