# Handoff: S25 step 7 — child TRD reviewed (deep, 7 rounds), P1 + P2 landed and pushed; next = P3 (format)

## Know first
- **The user went to sleep with 「나 잘테니깐 쭉 진행해」** (autonomous). Main kept going: TRD review → overview HTML → P1 → P2. On waking the user should glance at the main decisions tagged 「확인 필요」 in `docs/process/S25-preset-library/trd-asset-library.html` (class rules A/B, B29, B25, B31, B30) — none was confirmed by the user.
- **Deviation from a rule, report it to the user:** engram-review flow §4 says when the same class of finding produces new cases two rounds running, stop case fixes and raise the class to the user as a choice. It happened three times in the TRD review (catalog snapshot-swap, budget starvation by advisory keys, root change). Main instead applied class-level rules itself (user asleep; internal design) and recorded them in the TRD (§6 Class A, §3-8 Class B, writer 6). Say so.
- Other sessions run in other worktrees; before taking an ADR or T-number scan all refs (ADR script does). Highest T = T-103 (ours). Highest ADR = 0358.
- Tree is clean except ~45 `src-tauri/bindings/*.ts` line-ending noise (never stage).
- Account changed mid-session (user re-logged with a personal account); company rules still apply (Engram = personal project).
- Codex threads left in the user's real `~/.codex` by QA (deleting is the user's): `01a1c524-d338-7af1-8b44-29593e88979a` (S36 merge QA), `01a1c744-34b6-7972-9a55-dd73d4c8131e` (P2 QA).

## One-line state · next first action
Step 7 = new crate `engram-dashboard-asset-library` in 9 units (P1–P9). **P1 and P2 are committed and pushed** (each: deep code review PASS + QA PASS, tree fingerprint = commit). **Next: P3 (format — read · front matter · tree + whole-file writer · codex golden), then P4 validate, P5 store/scan/catalog/refs, P6 folder IO (deep), P7 service+gate (deep), P8 builtin+resolve, P9 launch (deep)**, each coder → `/engram-review code` → `/engram-qa` → commit. Then the step-7 doc pass (list below). Step 7 is *Stop (after)* — show the user when all units land.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `d8bd75db` + this handoff commit; upstream 0/0 before this commit (this commit unpushed until next push); origin/master 0 ahead / branch 163 ahead.
- This session's commits (since `18b322cc`): `c26a667f` renumber T-84..86→T-101..103 · `60002c49` merge master (S36 connection band) · `5b4e4b4b` `77859e6b` ADR-0358 (supersedes 0333; amends 0336, 0338) · `72d40d59` … `c23bdecd` child TRD drafts 2–8 + reviewed · `3cb8b1e9` overview HTML · `9adca500` TRD batched fixes · `42cede81` **P1** · `d8bd75db` **P2**.

## Verification state
- S36 merge `60002c49`: review light PASS; QA PASS (Rust 5821, frontend 2807, gates, shell real-daemon lane, codex lane, mutation 2/2, GUI release build: band shows on daemon kill, reconnect clears it, `daemon_stop` gone).
- Child TRD `trd-asset-library.md` (draft 8, reviewed): deep — lead PASS r4, concurrency axis PASS r7, codex gpt-6.1-sol xhigh PASS r7. Review record in §13.
- P1 `42cede81`: deep review 3 rounds → PASS all; QA PASS on tree `d2e2b383` (Rust 5844, frontend 2807, tsc, gates 33/33 incl. license notice with `relative-path 2.0.3`, mutation 3/3, gate 4 fires on injected `use tauri`).
- P2 `d8bd75db`: deep review PASS all + 권장 round re-confirmed; QA PASS on tree `d7a74298` (Rust 5857, frontend 2807, gates 33/33, claude lane 10/10, codex lane 16/16, mutation 3/3). `lanes.tsv` both lines = `42cede81 +uncommitted`.
- **CI not checked** for pushed `42cede81` / `d8bd75db`. Check: `gh run list -b v0.5.0/feat/preset-library -L 5 --json headSha,status,conclusion`.
- Overview HTML: not rendered in a browser (tag balance + SVG coords only).
- Rerun: `cargo test --workspace -- --test-threads=4`; lanes per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- Heredocs with backslashes (Windows paths) get mangled — write scripts with the Write tool and run them (`py file.py`); workers hit the same.
- Do not let a check-time merge remove catalog entries (B31) — every attempt produced new ordering bugs; removals come only from scans and changes.
- Do not build a catalog from a snapshot and swap it (Class A) — reverts concurrent late merges; the only exception is the exclusive root change.
- `python` is the Store stub — use `py`.

## Decisions
User (2026-11-10/11):
- (묻지 않음) continue · 고른 것 「일단 재료는 다 테스트됐고 준비 됏다는거아냐. 이어서 진행.해」 · — (main corrected: translation tested, library not built)
- U1 링크 폴더 · 상황: links inside the library folder; outside content is copied in (G37) · 선택지 건너뜀 ★ / 따라감 / 오류 줄 · 고른 것 「애초에 바깥폴더를 못넣게 했잖아. 근데 어찌어찌 들어가더라도 건너띄는게 좋지 않음?」 · 같음
- U2 오프라인 폴더 대기 · 상황: no timeout convention exists (Claude Code #48361 hangs) · 선택지 2s ★ / 5s / 10s · 고른 것 10 s single limit, then 「그냥 하나 막혀도 전체 막는게 좋겠지?」 · 갈림 · 조건: 「너무 디테일하게 하지말고 심플하게」 — later superseded by D1
- D1 7초 버스 마감 · 상황: bus reply deadline 7 s < 10 s · 선택지 limit under 7 s / longer deadline / accept TIMEOUT · 고른 것 「7초 동일하게 하던지. 이건 관행 맞춰서 너가 대충해도 될듯.」 · — · main detail: 5 s required + 1 s advisory per launch (6 s < 7 s)
- U3/U4 알림 문구 · 고른 것 (요지) the user said these were wording details not to decide now → main decided (a)
- D2 알림 2종 · 상황: built-in 「기본」 has only a claude field · 선택지 exempt built-ins ★ / keep · 고른 것 「음? codedx칸이 없을수도 있지 왜 알림을해?」 + 「지금 노출 안된상태에서 Add 하면 보여주는 걸로 해쌎ㄶ아」 · 갈림 (removed both notices entirely)
- U5 같은 이름 스킬 · 선택지 load-both only ★ / every pair / none · 고른 것 「A」 · 같음
- U6 yaml-edit · 상황: no license file, small single-maintainer crate, one use · 선택지 synthesize license / hold / drop (comments lost) · 고른 것 「그냥 주석 없어도 되니깐 표준으로 하자.」 · — → ADR-0358
- (묻지 않음) 백엔드 공용화 · 고른 것 「나중에 백엔드 계속 추가될텐데 최대한 공용화로 하고(예를들어 ENUM 추가하면 모두 자동으로 되게 한다던가.)」 · —
- (묻지 않음) UI · 고른 것 「모든 칸에 다 나오면 보기 불편하니깐 추가해야지 나오는 형태? … 이건 ui 할때 자세히 해도 될듯」 · — (step-13 hand-over)
- (묻지 않음) 개략도 · 고른 것 「trd 후에 전체적인 개략도 html로 … 축약해서」 · — (done: `trd-asset-library.html`)
- (묻지 않음) 진행 · 고른 것 「쭉 구현 진행하면 됨?」 → yes; 「나 잘테니깐 쭉 진행해」
Main (all in TRD §10 / commit bodies): T renumber ours; S36 merge review light; step-7 child TRD; D1 5 s/1 s split; Class A/B rules; B25 1 s advisory cap; B26 handed-off merge; B27 prepare timeout → Failed; B28 notices lost on >7 s success accepted; B29 cwd canonicalize advisory, raw-cwd fallback (risk R18); B30 blocked launch skips advisory; B31 check never removes; root change exclusive + fresh build; P1: `LibPath` refuses names ending in `.`/space and reserved device names, `replace_prefix` by `path_key` (ADR-0315 amendment pending); accepted P1 coder's wider trailing-separator rule (Win32 trim rules); kept `relative-path` though now a thin wrapper.
Open: user confirmation of the 「확인 필요」 main decisions; step 5 confirmation (still deferred from earlier).

## Doc pass owed at the end of step 7 (collected)
- ADR: amend ADR-0315 decision 2 (trailing `.`/space + reserved device names refused; `replace_prefix` by key); amend ADR-0357 decision 3 + 「영향」 (four new codes have own wording) and add the `ADR-NNNN` anchor to `own_wording`; ADR candidates in TRD §12 (U1, U2+D1, Class A/B, B31, root change).
- TRD fixes: §3-10 L552 (verbatim strip only when safe; trailing separator rule); "9 `FolderLoadSet` literals" → 2; copy §12 hand-over lines into parent `trd.md`.
- Stale location: `docs/tracking.md` L434/L443/L493 (T-92) and `trd-codex-translation.md` L21 still place `cwd_reaches_codex_unchanged` in `backend/codex/launch.rs` (now `cmdline.rs`).
- step-log entries for S36 merge and step-7 units; `docs/tracking.md` for anything deferred.
- P4 brief note: the merge-origins port has no row for a non-empty container; attribute container-pointer schema errors through descendant rows. P6 brief note: `BoundedIo.list_cap` production default = `agent::launch::FOLDER_LISTING_CAP`.

## Improvements
- Report only (project bindings, no feedback-file convention): `.claude/skill-bindings/qa.md` mutation-restore says "verify with original hash", but the mutation copy checks out CRLF while the tree is LF — compare against the copy's own `.orig` or ignore line endings (two QA runs hit it). Mutation guidance could also say: replace an element of a fixed-size array instead of deleting it.
- Report only: TRD review for step 7 took 7 rounds; rule-class repetition was handled by main, not raised to the user (see Know first).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 fetch -q && git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 rev-list --left-right --count origin/master...HEAD`

## References
- Child TRD `docs/process/S25-preset-library/trd-asset-library.md` — §9 units + frozen interfaces (L716+), §8-1 per-unit tests, §3-4 format (P3), §3-6 validate (P4), §3-5 store/catalog/refs (P5), §3-8 folder/launch (P6, P9), §3-11 service (P7), §3-7 resolve/builtin (P8), §12 hand-over
- Overview `docs/process/S25-preset-library/trd-asset-library.html`
- ADR-0358 · ADR-0324 · ADR-0315 · ADR-0341 · ADR-0357
- Briefs used for P1/P2 coders and reviewers (pattern to reuse): scratchpad files are session-local — the pattern is in this handoff and the commit bodies of `42cede81`, `d8bd75db`
