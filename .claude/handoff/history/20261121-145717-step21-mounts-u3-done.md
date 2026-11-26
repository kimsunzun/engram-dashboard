# Handoff: S25 step 21 (library mounts) — U1, U2a, U2b, U3 committed and pushed; next = U4 mount commands

## Know first
- The user wants: short answers, plain words, progress as a numbered list of the work by name with "now at N of M" (never bare labels). Minor review items are not shown one by one — record them (commit messages / `docs/tracking.md`).
- **Subagent cap = at most 5 running at once, all families combined (Codex counts).** The user corrected main's earlier misreading ("5 per instruction") and rewrote `global-rules` (core `6ae16b7`). Replace a worker once its context passes ~500k; spawning fresh is fine.
- **Unshipped data gets no migration (ADR-0398)** — the library never shipped (not in `v0.4.0`); do not design old-shape readers or rewrites. The user said this keeps coming up.
- Server-stage position (UI 전 18개): 1–15 done; **16 = library mounts (step 21) in progress — 3 of 5 pieces done** (1 mount model + `library.json` schema 2 ✅ · 2 virtual paths through the service ✅ (2a + 2b) · 3 launch per mount + built-in mount `engram` ✅ · 4 mount commands ⏳ · 5 help texts / reference / live scenes ⏳); then 17 = file move/copy/trash (9b) · 18 = their commands (10b) · server-stage user check · client stage.
- `src-tauri/bindings/*.ts` line-ending noise and untracked `.claude/explain/` are not ours — never stage.
- Real-claude tests (CI skip list, 6) and the claude / codex lanes were skipped per unit (same arrangement the user agreed for T-112) — run them ONCE at the end of step 21, before the freeze.

## One-line state · next first action
Step 21 TRD `docs/process/S25-preset-library/trd-library-mounts.md` is reviewed (deep, PASS); ADR-0396 (one virtual root of named mounts), ADR-0398 (no migration), ADR-0400 (step-21 rules, amends 14 ADRs) are written. U1 `ac5d7414`, U2a `faf4cfac`, U2b `151cc7e8`, U3 `fe183347` are committed and pushed; CI green through U2b, U3's CI was running at save time. **Next: U4 (mount commands) per TRD §10 U4** via the same per-unit loop (fresh coder → deep code review: lead + risk axis + Codex → QA full regression + mutation → commit), then U5, then the real-claude lanes and the freeze.

## Carried into U4 (from reviews / handovers — put these in the U4 coder brief)
- Overlap wiring (`mounts.rs` `overlap`, `Place::candidate` / `known`, `BoundedIo::identity_all` exist but are unused in production): judge reserved local folders before mounts; batch each folder's ancestor identity asks in one `identity_all`; a known side that answers TIMEOUT / NOT_CHECKED should refuse the candidate (fail closed), OS error → spelling fallback; warn-log any spelling fallback; one shared `LaunchIo` budget per judgement — size it.
- `Gate::write_mounts` is the ONLY writer of mounts / `seen` / `stamps`; remove / rename / relocate must clear `seen` and `stamps` and sync the view (`gate.rs` header rule) — otherwise `first_scan_join` can spin.
- `resolve_over` now judges states inside the resolving lock (U3) — the rename/remove race tests become possible in U4.
- `library.status` doc (`root_setting` null now also for an empty list), status reshape (per-mount `MountState`, retire `RootStatus`), `setRoot` removal, `PresetSection` lowercase words (host-checked `String`, `ProblemRow.kind` precedent; help `--section <Presets|…>`), catalog_version comment for any new declaration change.
- Remaining agent `commands.rs` wire docs from U2a: `newFolder` / `newFile` "unspecified = the root", `list` "unspecified = the root", `folders` "no root" — check what U2b/U3 already fixed.
- Rename primitive per TRD §3.2 / ADR-0400: presets → generation (root generation AND change number) → agents → re-key → registration in one write-lock hold under a Root ticket; plan with `yes` + required `root_generation`; repeat-after-failed-save rules; refuse when the mount is unusable or the profile store is `ReadOnly`; `agent.new` re-check uses `Numbers.change` (already implemented in U2a? — `agent.new` currently re-checks via the U2a / U3 paths; confirm against code).

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `fe183347`, 0 ahead / 0 behind origin at save time (this handoff commit will be ahead until pushed).
- This session's commits (oldest first): ADR-0396 `494a6452` + body `727590ae`; TRD drafts `c051da1d` `25817a70` `d8faf60a` `7aebc97c` `a6c28534` `61395f4a`; ADR-0398 `100d3f82` + body `4f19ab12`; TRD reviewed `2c617ed1`; ADR-0400 `b642ce3d` + body `5dd6e70a`; 9b hand-over note `aa61ee89`; U1 `ac5d7414`; U2a `faf4cfac`; U2b `151cc7e8`; U3 `fe183347`.
- Uncommitted: nothing of ours.
- Engram core repo `I:/Engram/core`: feedback rows this session `fc2bb24` (implement: unshipped data no migration — 미반영), `1015716` (subagent cap — since reflected by `6ae16b7`). Not pushed by main.

## Verification state
- Ran per unit: deep code review (lead · risk axis · Codex) until PASS; QA = full local regression (CI's three jobs) + bindings drift + test-name comparison + mutation. Last QA (U3, tree `95bb289e`): workspace 6625 passed, frontend 2884, gates 34/34, bindings no drift, mutation 2/2 caught (incl. the U2b survivor). CI green on U2a `faf4cfac` and U2b `151cc7e8`.
- Not verified: U3's CI (running at save); the 6 real-claude tests and the claude / codex lanes for all of step 21; live CLI scenes (U5); `cargo clippy` (not installed for toolchain 1.95.0).
- Rerun: `cargo test --workspace -- --test-threads=4` (detached per `.claude/skill-bindings/qa.md`); lanes per qa.md 「claude · codex 레인」.

## Failed approaches (do-not)
- Codex re-reviews via a "resume the same thread" prompt return `WRONG THREAD` — start a fresh Codex task and give it the earlier findings in the prompt.
- Python one-liners via `python` hit the Windows Store stub — use `py`, and write scripts to a file (heredocs swallow backslashes).
- `mutate.mjs` has no operator swap for method-call guards (`is_none` / `split_once`); pick a nearby operator or report "not mutable".
- Holding the library read lock across `agent.new`'s registry write (draft 3) put IO and the name gate under the library lock — rejected; the re-check compares the change number inside the registry insert and re-judges outside both locks.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -5 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings && gh run list --branch v0.5.0/feat/preset-library --limit 1`

## Decisions
User (2026-11-21):
- 16번 TRD 분리 · 상황: 16(라이브러리 루트 여러 개)과 17(파일 조작)을 한 TRD로 합칠지 · 따로 + 맞물림만 16에서 ★ / 합치기 · 「그냥 따로해 그럼.」 · 같음.
- 루트 모델 · 상황: 여러 독립 루트(ADR-0385)는 3~5천 줄 · 하나로 관리 ★ / 여러 루트 / (사용자 제안) 가상 루트 + 이름 붙은 마운트 · 「루트는 가상루트로 최상단 루트고(트리에서는 관리되고 실물은 없는?), 그 밑에는 특정 이름이 있고 근데 그 이름은 나중에 실제 펼칠때 대체되는거고」 · 갈림 · 조건: 「그럼 그 폴더간의 레퍼 참조 이동도 되겠네」 · 「이대로 문제는 없는거잖아. 가상이니깐. 만약에 연결 끊기면 수동으로 맞추거나 해당 폴더 되돌린다음에 어떻게 어떻게 하면 되는거고.」 → ADR-0396.
- UD1–UD9 · 상황: TRD 사용자 결정 9건 · 전부 ★ · 「ㅇㅇ 진행해.」 · 같음 (UD1 was later replaced by ADR-0398).
- 옛 경로 이전 · 상황: 자동 고쳐 쓰기에서 must 5건, 라이브러리 미배포 · 이전 안 함 ★ / 유지 · 「지금까지 저장된 경로가 없잖아. 이거 되게 자주 나오는데 배포되고 고려하라니깐 그런건.」 · 같음 → ADR-0398.
- 자율 진행 · (묻지 않음) · 선택지 없음 · 「자율모드로 진행해 나 자러간다.」 · —.
- 서브에이전트 상한 해석 · (묻지 않음) · 선택지 없음 · 「뭔소리야 새로 띄울수 있는 5개라니? 5개를 한꺼번에 띄우지 말라는 말이었는데?」 · — → global-rules rewritten (동시에 5개, family 합산).
- 마무리 · 「마무리 지으면 핸드오프 하자.」 · 「하던거는 다 마무리 짓고 핸드오프 하면될듯?」 → finished U3, then this save.
Main (reported): mount id dropped (T-72 decides identity); one library lock + one generation; 9b move/copy mechanics handed over as rules (TRD §3.6 「9b must also」 + parent trd.md 9b hand-over); U2 split into U2a / U2b; holds resolved by restoring pre-change behaviour (arrival order via lock-free `MountView`, spawn-failed stat walks) or by following decided rules (O3 / ADR-0386 d5 for unknown mounts, ADR-0400 d5 one-segment writes); relink keeps `fixed` for readable mounts and reports never-walked unusable mounts as mount-top skip rows; `engram` row unreadable when the built-in folder cannot be read; built-in reference counts documented as Default-slot only.
Open (asked, no answer yet):
- 「손으로 고친 `library.json`에서 마운트 항목 하나만 잘못돼도 목록 전체를 손상으로 본다」(ADR-0291 그대로) — 괜찮은지.
- 안 고친 권장 목록 (commit messages of U1–U3 list them): spelling fallback for an existing mount whose canonical gets no answer (U4 wiring); after a failed first start an attach skips a library preset as `not-found`; a refusal inside a change still bumps the change generation; the `ListFolderRow` / `LibraryFolderRow` `state` doc sentence; no test for "a built-in that reads but does not parse keeps the mount Ok".

## References
- TRD `docs/process/S25-preset-library/trd-library-mounts.md` (reviewed; §10 units, §9 test plan, §3.6 9b rules); ADR-0396 · ADR-0398 · ADR-0400.
- Code briefs / handovers (git-excluded): `.review-s25-d9/s21-code/coder-brief.md`, `handover-u1.md`, `handover-u2a.md`, `handover-u2b.md`, review briefs `u*-review-brief.md`, findings `u*-round*.md`.
- Reference doc `docs/reference/structure/asset-library.md` (kept current per unit).
